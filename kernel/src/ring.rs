//! `jring`: an io_uring-lite over a shared ring page.
//!
//! One 4 KiB page, mapped EL0-RW at [`USER_RING_VA`], holds a submission queue
//! (user produces, kernel consumes) and a completion queue (kernel produces,
//! user consumes). The user batches SQEs and makes one `SYS_RING_ENTER` call;
//! `NOP`/`PRINT` complete immediately, while a `READ` with no input waiting is
//! parked in a pending table and completed later **from the timer interrupt**,
//! while user code runs — asynchronous completion with no syscall involved.
//!
//! Index discipline (mirrors io_uring): head/tail are free-running `u32`s
//! masked by `ENTRIES - 1`; a producer publishes entries with a release store
//! of its tail, a consumer reads the tail with acquire before the entries.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::frames::alloc_frame;
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;
use crate::uart;

/// Virtual address of the shared ring page (above the user stack).
pub const USER_RING_VA: usize = 0x2_0030_0000;

/// Entries in each ring (power of two).
const ENTRIES: u32 = 16;
const MASK: u32 = ENTRIES - 1;

// Byte offsets within the ring page.
const SQ_HEAD: usize = 0x00;
const SQ_TAIL: usize = 0x04;
const CQ_HEAD: usize = 0x08;
const CQ_TAIL: usize = 0x0c;
const FLAGS: usize = 0x10;
const SQ_OFF: usize = 0x40; // 16 x 32-byte SQEs
const CQ_OFF: usize = 0x280; // 16 x 16-byte CQEs

/// Flags-word bit: the poller went to sleep; submitters must call
/// `SYS_RING_ENTER` once to revive it (Linux's `IORING_SQ_NEED_WAKEUP`).
pub const NEED_WAKEUP: u32 = 1;

/// Ticks without work before the poller goes to sleep (`sq_thread_idle`).
const SQ_IDLE_TICKS: u64 = 5;

// SQE opcodes.
pub const OP_NOP: u64 = 0;
pub const OP_PRINT: u64 = 1;
pub const OP_READ: u64 = 2;

/// CQE result for an invalid pointer, opcode, or a full pending table.
const ERR: i64 = -1;

/// A parked `READ` awaiting console input.
#[derive(Clone, Copy)]
struct Pending {
    buf: usize,
    len: usize,
    user_data: u64,
}

/// Maximum simultaneously parked reads.
const MAX_PENDING: usize = 8;

struct RingState {
    /// Whether the ring page has been mapped (setup is idempotent).
    mapped: bool,
    pending: [Option<Pending>; MAX_PENDING],
    /// Task blocked in `enter` waiting for completions, woken by
    /// [`poll_pending`]. Kernel-side state on purpose: the user must not be
    /// able to forge a wake target through the shared page.
    waiter: Option<usize>,
    /// The SQPOLL task, woken by `enter` when `NEED_WAKEUP` was set.
    poller: Option<usize>,
}

static RING: Locked<RingState> = Locked::new(RingState {
    mapped: false,
    pending: [None; MAX_PENDING],
    waiter: None,
    poller: None,
});

/// A head/tail index in the shared page, viewed as an atomic.
fn index(off: usize) -> &'static AtomicU32 {
    // SAFETY: the ring page is mapped and the four indices are aligned u32s;
    // AtomicU32 gives both sides ordered access to the same memory.
    unsafe { &*((USER_RING_VA + off) as *const AtomicU32) }
}

/// Read field `n` (of 4) of the SQE at ring slot `slot`.
fn sqe_field(slot: u32, n: usize) -> u64 {
    let p = (USER_RING_VA + SQ_OFF + (slot & MASK) as usize * 32 + n * 8) as *const u64;
    // SAFETY: in-bounds within the mapped ring page.
    unsafe { p.read_volatile() }
}

/// Post a completion: write the CQE, then publish with a release store of the
/// tail so the user sees the entry before the index moves.
fn complete(user_data: u64, result: i64) {
    let tail = index(CQ_TAIL).load(Ordering::Relaxed);
    let head = index(CQ_HEAD).load(Ordering::Acquire);
    if tail.wrapping_sub(head) >= ENTRIES {
        crate::kprintln!("jring: completion queue overflow, dropping CQE");
        return;
    }
    let p = (USER_RING_VA + CQ_OFF + (tail & MASK) as usize * 16) as *mut u64;
    // SAFETY: in-bounds within the mapped ring page.
    unsafe {
        p.write_volatile(user_data);
        p.add(1).write_volatile(result as u64);
    }
    index(CQ_TAIL).store(tail.wrapping_add(1), Ordering::Release);
}

/// `SYS_RING_SETUP`: map and zero the ring page (idempotent). Returns its VA.
pub fn setup() -> u64 {
    let mut ring = RING.lock();
    if !ring.mapped {
        let frame = alloc_frame().expect("out of frames for the ring page");
        mmu::map_page(USER_RING_VA, frame.addr(), PAGE_USER_RW);
        // SAFETY: freshly mapped EL1-writable page; zero it before publishing.
        unsafe { core::ptr::write_bytes(USER_RING_VA as *mut u8, 0, 4096) };
        ring.mapped = true;
    }
    USER_RING_VA as u64
}

/// `SYS_RING_ENTER`: consume all published SQEs, then — like Linux's
/// `io_uring_enter(min_complete)` — block until the completion queue holds at
/// least `min_complete` unreaped entries (`0` = submit-only).
///
/// Runs with IRQs masked (we are in a syscall), so the drain never races
/// [`poll_pending`]; IRQs stay masked until the block's `context_switch` lands
/// in a task that re-enables them, so no completion can slip in between the
/// recheck and the block (no lost wakeup, single core).
pub fn enter(from_user: bool, min_complete: u64) -> u64 {
    if !RING.lock().mapped {
        return u64::MAX;
    }

    // The poller went to sleep and a submitter trapped in to revive it: wake
    // it and clear the flag HERE, not when the poller runs — otherwise every
    // submit until its next timeslice would see the flag and trap too.
    if index(FLAGS).load(Ordering::Acquire) & NEED_WAKEUP != 0 {
        index(FLAGS).store(0, Ordering::Release);
        let poller = RING.lock().poller;
        if let Some(p) = poller {
            crate::sched::wake(p);
        }
    }

    drain(from_user);

    // Both indices live in the user-writable page: a hostile user can block
    // themselves forever or not at all — liveness, never safety.
    loop {
        let unreaped = index(CQ_TAIL)
            .load(Ordering::Relaxed)
            .wrapping_sub(index(CQ_HEAD).load(Ordering::Relaxed));
        if unreaped as u64 >= min_complete {
            return 0;
        }
        RING.lock().waiter = Some(crate::sched::current());
        // Lock dropped before blocking: the waking IRQ path needs it.
        crate::sched::block_current();
        // Woken: recheck the condition, never trust a wake.
    }
}

/// Consume all published SQEs; returns whether any work was done. Runs with
/// IRQs disabled: it has two callers — the `enter` syscall and the
/// (preemptible) SQPOLL task — and per-SQE processing must not interleave.
fn drain(from_user: bool) -> bool {
    let d = crate::irq::disable();
    let mut head = index(SQ_HEAD).load(Ordering::Relaxed);
    let tail = index(SQ_TAIL).load(Ordering::Acquire);
    let worked = head != tail;
    while head != tail {
        process_sqe(head, from_user);
        head = head.wrapping_add(1);
        index(SQ_HEAD).store(head, Ordering::Release);
    }
    crate::irq::restore(d);
    worked
}

/// The SQPOLL task: poll the submission queue so published SQEs are consumed
/// with no syscall at all. After [`SQ_IDLE_TICKS`] without work it raises
/// `NEED_WAKEUP` and sleeps; submitters seeing that flag trap once to revive
/// it (the handshake itself runs through shared memory).
pub extern "C" fn sqpoll_main(_arg: usize) {
    RING.lock().poller = Some(crate::sched::current());
    let mut announced = false;
    let mut last_work = crate::timer::ticks();
    loop {
        if drain(true) {
            // SQEs always come from EL0 publishes, hence from_user = true.
            if !announced {
                crate::kprintln!("[sqpoll] picked up work");
                announced = true;
            }
            last_work = crate::timer::ticks();
        } else if crate::timer::ticks().wrapping_sub(last_work) >= SQ_IDLE_TICKS {
            // Going to sleep. Order matters (the lost-wakeup window): raise
            // the flag FIRST, then drain once more — any SQE published just
            // before the flag went up is caught here; any published after it
            // sees NEED_WAKEUP and traps to wake us.
            index(FLAGS).store(NEED_WAKEUP, Ordering::Release);
            if drain(true) {
                index(FLAGS).store(0, Ordering::Release);
            } else {
                crate::sched::block_current();
                // enter() cleared the flag when it woke us.
            }
            last_work = crate::timer::ticks();
        }
        // Single core: give the producer its turn between polls.
        crate::sched::yield_now();
    }
}

/// Execute one submission, posting its completion (now, or for a parked
/// `READ`, later from the timer IRQ).
fn process_sqe(slot: u32, from_user: bool) {
    let opcode = sqe_field(slot, 0);
    let arg0 = sqe_field(slot, 1) as usize;
    let arg1 = sqe_field(slot, 2) as usize;
    let user_data = sqe_field(slot, 3);

    match opcode {
        OP_NOP => complete(user_data, 0),
        OP_PRINT => {
            if from_user && !crate::user::is_user_range(arg0, arg1) {
                complete(user_data, ERR);
                return;
            }
            // SAFETY: kernel-trusted, or validated to lie in mapped user memory.
            let bytes = unsafe { core::slice::from_raw_parts(arg0 as *const u8, arg1) };
            if let Ok(s) = core::str::from_utf8(bytes) {
                uart::write_str(s);
            }
            complete(user_data, 0);
        }
        OP_READ => {
            if from_user && !crate::user::is_user_range_writable(arg0, arg1) {
                complete(user_data, ERR);
                return;
            }
            if arg1 == 0 {
                complete(user_data, 0);
                return;
            }
            // Buffered input completes immediately; otherwise park.
            let count = drain_input(arg0, arg1);
            if count > 0 {
                complete(user_data, count as i64);
                return;
            }
            let mut ring = RING.lock();
            match ring.pending.iter_mut().find(|s| s.is_none()) {
                Some(slot) => {
                    *slot = Some(Pending {
                        buf: arg0,
                        len: arg1,
                        user_data,
                    })
                }
                None => complete(user_data, ERR),
            }
        }
        _ => complete(user_data, ERR),
    }
}

/// Complete parked reads from the kernel input buffer. Called from the UART RX
/// interrupt handler (after it drained the device into the buffer) — CQEs land
/// the moment a key arrives, while user code runs, no syscall involved.
pub fn poll_pending() {
    let mut ring = RING.lock();
    let mut completed = false;
    for slot in ring.pending.iter_mut() {
        if let Some(p) = *slot {
            let count = drain_input(p.buf, p.len);
            if count > 0 {
                complete(p.user_data, count as i64);
                *slot = None;
                completed = true;
            }
        }
    }
    // New completions may satisfy a task blocked in `enter`: wake it.
    if completed {
        if let Some(waiter) = ring.waiter.take() {
            crate::sched::wake(waiter);
        }
    }
}

/// Deliver buffered console input into the user range `[buf, buf+len)` via
/// `copy_to_user`. The ring layer no longer touches the UART: the driver
/// (`input.rs`) owns the device, this layer owns the user-facing interface.
fn drain_input(buf: usize, len: usize) -> usize {
    let mut count = 0;
    while count < len {
        match crate::input::pop() {
            Some(b) => {
                // Validated at SQE-accept time; copy_to_user re-checks (the
                // Linux access_ok-and-copy shape). A failure here would drop
                // the byte — defense in depth, not an expected path.
                if !crate::user::copy_to_user(buf + count, &[b]) {
                    break;
                }
                count += 1;
            }
            None => break,
        }
    }
    count
}
