//! `jring`: an io_uring-lite over a shared ring page.
//!
//! The page layout and all shared constants live in the `abi` crate — the
//! kernel/userspace contract. This module is the kernel half: it consumes
//! SQEs (submission queue entries), posts CQEs (completion queue entries),
//! parks reads until input arrives, hosts the SQPOLL submission-poller task,
//! and blocks `enter(min_complete)` callers until completions exist.
//!
//! `NOP`/`PRINT` complete immediately; a `READ` with no buffered input is
//! parked in a pending table and completed later from the UART receive
//! interrupt, while user code runs — asynchronous completion, no syscall.

use core::sync::atomic::Ordering;

use abi::{Cqe, RingPage, Sqe, NEED_WAKEUP, OP_NOP, OP_PRINT, OP_READ, RING_ENTRIES, RING_MASK, USER_RING_VA};

use crate::frames::alloc_frame;
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;
use crate::uart;

/// Ticks without work before the SQPOLL task goes to sleep (the
/// `sq_thread_idle` analog).
const SQ_IDLE_TICKS: u64 = 5;

/// CQE result for an invalid pointer, opcode, or a full pending table.
const ERR: i64 = -1;

/// A parked `READ` awaiting console input.
#[derive(Clone, Copy)]
struct Pending {
    /// User-space destination buffer (virtual address), validated at accept.
    buf: usize,
    /// Capacity of that buffer in bytes.
    len: usize,
    /// The request's completion tag, echoed in its CQE.
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

/// The shared ring page, typed. The ONE place in the kernel where the ring's
/// raw address becomes a reference; sound only after [`setup`] mapped it —
/// every public entry point is gated on `RingState::mapped`.
///
/// HAZARD (per-process address spaces): the kernel dereferences the ring at a
/// *user* VA (`USER_RING_VA`, in the user L0 slot). This is sound only while a
/// single global TTBR0 maps that VA for both EL1 and EL0. Once TTBR0 is switched
/// per process, this breaks: when process B is current, an IRQ completing process
/// A's parked `READ` cannot reach A's ring through A's user VA. The fix — a
/// kernel-side alias of the ring in the kernel's L0 slot, plus deciding how
/// per-process rings are created and mapped — belongs to the per-process step.
fn page() -> &'static RingPage {
    // SAFETY: the page is mapped (callers check `mapped`), 4 KiB, and lives
    // for the rest of the kernel's life; all fields are atomics, so shared
    // mutation from EL0/IRQ context through &-references is sound.
    unsafe { &*(USER_RING_VA as *const RingPage) }
}

/// Slot for a free-running index.
fn slot(index: u32) -> usize {
    (index & RING_MASK) as usize
}

/// Post a completion: write the CQE, then publish with a release store of the
/// tail so the user sees the entry before the index moves.
fn complete(user_data: u64, result: i64) {
    let p = page();
    let tail = p.cq_tail.load(Ordering::Relaxed);
    let head = p.cq_head.load(Ordering::Acquire);
    if tail.wrapping_sub(head) >= RING_ENTRIES {
        crate::kprintln!("jring: completion queue overflow, dropping CQE");
        return;
    }
    let cqe: &Cqe = &p.cq[slot(tail)];
    cqe.user_data.store(user_data, Ordering::Relaxed);
    cqe.result.store(result, Ordering::Relaxed);
    p.cq_tail.store(tail.wrapping_add(1), Ordering::Release);
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
    if page().flags.load(Ordering::Acquire) & NEED_WAKEUP != 0 {
        page().flags.store(0, Ordering::Release);
        let poller = RING.lock().poller;
        if let Some(p) = poller {
            crate::sched::wake(p);
        }
    }

    drain(from_user);

    // Both indices live in the user-writable page: a hostile user can block
    // themselves forever or not at all — liveness, never safety.
    loop {
        let unreaped = page()
            .cq_tail
            .load(Ordering::Relaxed)
            .wrapping_sub(page().cq_head.load(Ordering::Relaxed));
        if u64::from(unreaped) >= min_complete {
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
    let p = page();
    let mut head = p.sq_head.load(Ordering::Relaxed);
    let tail = p.sq_tail.load(Ordering::Acquire);
    let worked = head != tail;
    while head != tail {
        process_sqe(&p.sq[slot(head)], from_user);
        head = head.wrapping_add(1);
        p.sq_head.store(head, Ordering::Release);
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
            page().flags.store(NEED_WAKEUP, Ordering::Release);
            if drain(true) {
                page().flags.store(0, Ordering::Release);
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
/// `READ`, later from the UART receive interrupt).
fn process_sqe(sqe: &Sqe, from_user: bool) {
    let opcode = sqe.opcode.load(Ordering::Relaxed);
    let arg0 = sqe.arg0.load(Ordering::Relaxed) as usize;
    let arg1 = sqe.arg1.load(Ordering::Relaxed) as usize;
    let user_data = sqe.user_data.load(Ordering::Relaxed);

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
    if !ring.mapped {
        return; // input can arrive before the ring exists
    }
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

/// Drop all state referencing the (dying) user program: parked reads point at
/// buffers that are about to be unmapped, and the waiter task is exiting.
/// Their CQEs are simply never posted — nobody is left to reap them.
pub fn abort_user() {
    let d = crate::irq::disable();
    {
        let mut ring = RING.lock();
        ring.pending = [None; MAX_PENDING];
        ring.waiter = None;
    }
    crate::irq::restore(d);
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
