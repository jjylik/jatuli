//! `jring`: an io_uring-lite over a shared ring page, now per process.
//!
//! Each process owns a ring page (a RAM frame). Userspace maps it at
//! `USER_RING_VA` in its own address space; the kernel reaches the *same frame*
//! at its physical address via the identity map (`ring_pa`), so it can post
//! completions to any process's ring under any `TTBR0` — this is what lets an
//! IRQ complete a parked read for a process that isn't current.
//!
//! Reading an SQE's *payload* (a print string, a read buffer) is different: those
//! are user VAs that only resolve in the owner's address space. So the SQPOLL
//! poller activates a process's space before draining its submissions, and a
//! syscall's `enter` drains under the trapping process's own (already live) space.

use core::sync::atomic::Ordering;

use abi::{
    Cqe, RingPage, Sqe, NEED_WAKEUP, OP_NOP, OP_PRINT, OP_READ, OP_SPAWN, OP_WAIT, RING_ENTRIES,
    RING_MASK, USER_RING_VA,
};

use crate::frames::alloc_frame;
use crate::mmu::{self, PAGE_USER_RW};
use crate::process::{self, Pending};
use crate::sched;
use crate::sync::Locked;
use crate::uart;

/// Ticks without work before the SQPOLL task goes to sleep.
const SQ_IDLE_TICKS: u64 = 5;

/// CQE result for an invalid pointer, opcode, or a full pending table.
const ERR: i64 = -1;

/// Global SQPOLL state — only the poller task index; everything else is per
/// process now.
struct RingState {
    /// The SQPOLL task, woken by `enter` when a ring's `NEED_WAKEUP` was set.
    poller: Option<usize>,
}

static RING: Locked<RingState> = Locked::new(RingState { poller: None });

/// A process's ring page, typed, reached at its physical address (identity map).
///
/// # Safety
/// `ring_pa` must be a live ring frame (a process's `ring_pa`, non-zero). All
/// fields are atomics, so shared mutation from EL0/IRQ context is sound.
fn ring(ring_pa: usize) -> &'static RingPage {
    // SAFETY: `ring_pa` is a 4 KiB ring frame, identity-mapped and live for the
    // owning process's life; the typed view is all-atomic.
    unsafe { &*(ring_pa as *const RingPage) }
}

/// Slot for a free-running index.
fn slot(index: u32) -> usize {
    (index & RING_MASK) as usize
}

/// Post a completion to the ring at `ring_pa`: write the CQE, then publish with a
/// release store of the tail so the user sees the entry before the index moves.
fn complete(ring_pa: usize, user_data: u64, result: i64) {
    let p = ring(ring_pa);
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

/// `SYS_RING_SETUP`: map the calling process's ring page into its address space
/// (idempotent) and record the frame's PA for kernel-side access. Returns the
/// ring VA.
pub fn setup() -> u64 {
    let pid = match sched::current_process() {
        Some(p) => p,
        None => return u64::MAX,
    };
    if process::ring_pa(pid) != 0 {
        return USER_RING_VA as u64; // already set up
    }
    let frame = alloc_frame().expect("out of frames for the ring page");
    // SAFETY: a fresh identity-mapped frame; zero it before publishing.
    unsafe { core::ptr::write_bytes(frame.addr() as *mut u8, 0, 4096) };
    // Map it EL0-RW into the calling process's space (current = that process).
    // SAFETY: `ttbr0(pid)` is the live process's root; this user VA is unmapped.
    unsafe {
        mmu::map_page_in(mmu::l0_ptr(process::ttbr0(pid)), USER_RING_VA, frame.addr(), PAGE_USER_RW);
    }
    process::set_ring(pid, frame);
    USER_RING_VA as u64
}

/// `SYS_RING_ENTER`: consume the calling process's published SQEs, then block
/// until its completion queue holds at least `min_complete` unreaped entries
/// (`0` = submit-only). Runs in syscall context, so the process's own space is
/// live and `drain` can read SQE payloads directly.
pub fn enter(min_complete: u64) -> u64 {
    let pid = match sched::current_process() {
        Some(p) => p,
        None => return u64::MAX,
    };
    let ring_pa = process::ring_pa(pid);
    if ring_pa == 0 {
        return u64::MAX;
    }

    // Revive the poller if it slept and we're the submitter that tripped the flag.
    let pg = ring(ring_pa);
    if pg.flags.load(Ordering::Acquire) & NEED_WAKEUP != 0 {
        pg.flags.store(0, Ordering::Release);
        let poller = RING.lock().poller;
        if let Some(p) = poller {
            sched::wake(p);
        }
    }

    drain(pid);

    // Both CQ indices live in the user-writable page: a hostile user can block
    // itself forever or not at all — liveness, never safety.
    loop {
        let unreaped = pg
            .cq_tail
            .load(Ordering::Relaxed)
            .wrapping_sub(pg.cq_head.load(Ordering::Relaxed));
        if u64::from(unreaped) >= min_complete {
            return 0;
        }
        process::set_waiter(pid, sched::current());
        // Recheck after waking, never trust a wake.
        sched::block_current();
    }
}

/// Consume all of process `pid`'s published SQEs; returns whether any work was
/// done. IRQs disabled so per-SQE processing doesn't interleave with the timer.
/// The caller must ensure `pid`'s address space is live (SQE payloads are user
/// VAs): `enter` runs in the owner's syscall context; the poller activates it.
fn drain(pid: usize) -> bool {
    let d = crate::irq::disable();
    let ring_pa = process::ring_pa(pid);
    if ring_pa == 0 {
        crate::irq::restore(d);
        return false;
    }
    let p = ring(ring_pa);
    let mut head = p.sq_head.load(Ordering::Relaxed);
    let tail = p.sq_tail.load(Ordering::Acquire);
    let worked = head != tail;
    while head != tail {
        process_sqe(pid, &p.sq[slot(head)]);
        head = head.wrapping_add(1);
        p.sq_head.store(head, Ordering::Release);
    }
    crate::irq::restore(d);
    worked
}

/// The SQPOLL task: round-robin over every process, draining any with published
/// submissions so they're consumed with no syscall. Activates a process's
/// address space before draining it (its SQEs point into that space). After
/// [`SQ_IDLE_TICKS`] idle it raises `NEED_WAKEUP` on every ring and sleeps;
/// a submitter seeing the flag traps once via `enter` to revive it.
pub extern "C" fn sqpoll_main(_arg: usize) {
    RING.lock().poller = Some(sched::current());
    let mut announced = false;
    let mut last_work = crate::timer::ticks();
    loop {
        let mut worked = false;
        for pid in 0..process::count() {
            if has_submissions(pid) {
                mmu::activate(process::ttbr0(pid));
                drain(pid);
                worked = true;
            }
        }

        if worked {
            if !announced {
                crate::kprintln!("[sqpoll] picked up work");
                announced = true;
            }
            set_all_wakeup(false);
            last_work = crate::timer::ticks();
        } else if crate::timer::ticks().wrapping_sub(last_work) >= SQ_IDLE_TICKS {
            // Going to sleep. Raise the flag on every ring FIRST, then re-scan:
            // any SQE published just before catches here; any published after
            // sees NEED_WAKEUP and traps to wake us.
            set_all_wakeup(true);
            if (0..process::count()).any(has_submissions) {
                set_all_wakeup(false);
            } else {
                sched::block_current();
                // enter() cleared the waking ring's flag; clear the rest.
                set_all_wakeup(false);
            }
            last_work = crate::timer::ticks();
        }
        // Single core: give the producers their turn between polls.
        sched::yield_now();
    }
}

/// Whether process `pid` has a non-empty submission queue (peeked via the
/// identity-mapped ring page, no address-space switch needed to read indices).
fn has_submissions(pid: usize) -> bool {
    let ring_pa = process::ring_pa(pid);
    if ring_pa == 0 {
        return false;
    }
    let p = ring(ring_pa);
    p.sq_head.load(Ordering::Relaxed) != p.sq_tail.load(Ordering::Acquire)
}

/// Set or clear `NEED_WAKEUP` on every process's ring.
fn set_all_wakeup(on: bool) {
    let flag = if on { NEED_WAKEUP } else { 0 };
    for pid in 0..process::count() {
        let ring_pa = process::ring_pa(pid);
        if ring_pa != 0 {
            ring(ring_pa).flags.store(flag, Ordering::Release);
        }
    }
}

/// Execute one of process `pid`'s submissions, posting its completion (now, or
/// for a parked `READ`, later from the UART receive interrupt). The process's
/// address space is live, so user pointers resolve.
fn process_sqe(pid: usize, sqe: &Sqe) {
    let opcode = sqe.opcode.load(Ordering::Relaxed);
    let arg0 = sqe.arg0.load(Ordering::Relaxed) as usize;
    let arg1 = sqe.arg1.load(Ordering::Relaxed) as usize;
    let user_data = sqe.user_data.load(Ordering::Relaxed);
    let ring_pa = process::ring_pa(pid);

    match opcode {
        OP_NOP => complete(ring_pa, user_data, 0),
        OP_PRINT => {
            if !process::is_user_range(pid, arg0, arg1, false) {
                complete(ring_pa, user_data, ERR);
                return;
            }
            // SAFETY: validated to lie in this process's mapped, live user memory.
            let bytes = unsafe { core::slice::from_raw_parts(arg0 as *const u8, arg1) };
            if let Ok(s) = core::str::from_utf8(bytes) {
                uart::write_str(s);
            }
            complete(ring_pa, user_data, 0);
        }
        OP_READ => {
            if !process::is_user_range(pid, arg0, arg1, true) {
                complete(ring_pa, user_data, ERR);
                return;
            }
            if arg1 == 0 {
                complete(ring_pa, user_data, 0);
                return;
            }
            // Buffered input completes immediately, but only for the foreground
            // process; a background process's read parks (and, today, waits
            // indefinitely — only the foreground owns the keyboard).
            let count = if process::foreground_pid() == Some(pid) {
                drain_input(pid, arg0, arg1)
            } else {
                0
            };
            if count > 0 {
                complete(ring_pa, user_data, count as i64);
                return;
            }
            if !process::park_read(pid, Pending { buf: arg0, len: arg1, user_data }) {
                complete(ring_pa, user_data, ERR);
            }
        }
        OP_SPAWN => spawn(pid, arg0, arg1, user_data),
        OP_WAIT => wait(pid, arg0, user_data),
        _ => complete(ring_pa, user_data, ERR),
    }
}

/// `OP_SPAWN`: create a fresh process running the named program and run it as a
/// background task. The caller's address space is live (drain runs in its context
/// or the poller activated it), so the name pointer resolves. Completes with the
/// child's handle (pid), or [`ERR`] for a bad pointer or unknown program.
fn spawn(pid: usize, name_ptr: usize, name_len: usize, user_data: u64) {
    let ring_pa = process::ring_pa(pid);
    if !process::is_user_range(pid, name_ptr, name_len, false) {
        complete(ring_pa, user_data, ERR);
        return;
    }
    // SAFETY: validated to lie in the caller's mapped, live user memory.
    let bytes = unsafe { core::slice::from_raw_parts(name_ptr as *const u8, name_len) };
    let name = core::str::from_utf8(bytes).ok();
    let image = match name.and_then(crate::programs::get) {
        Some(image) => image,
        None => {
            complete(ring_pa, user_data, ERR);
            return;
        }
    };

    let ttbr0 = mmu::new_address_space();
    let child = process::create(image, ttbr0);
    process::set_parent(child, pid);
    debug_assert!(
        mmu::translate(ttbr0, abi::USER_BASE) != mmu::translate(process::ttbr0(pid), abi::USER_BASE),
        "spawned child shares the parent's frame at USER_BASE",
    );
    crate::kprintln!("[spawn] {} -> pid {}", name.unwrap_or("?"), child);
    // Background task: the parent keeps the foreground.
    sched::spawn_user(crate::user_task, 0, child, ttbr0);
    complete(ring_pa, user_data, child as i64);
}

/// `OP_WAIT`: complete with a child's exit code. If the child already exited,
/// complete now; otherwise park the wait — the child's exit fires it. Rejects a
/// handle that is not one of the caller's children.
fn wait(pid: usize, handle: usize, user_data: u64) {
    let ring_pa = process::ring_pa(pid);
    if handle >= process::count() || process::parent(handle) != Some(pid) {
        complete(ring_pa, user_data, ERR);
        return;
    }
    match process::exit_code(handle) {
        Some(code) => complete(ring_pa, user_data, code),
        None => process::register_wait(handle, user_data),
    }
}

/// A process exited (or was killed) with `code`: record it and, if a parent is
/// parked in `OP_WAIT`, post the completion to the parent's ring (reached by
/// identity PA, so it works even though the parent is not current) and wake it.
pub fn notify_exit(pid: usize, code: i64) {
    if let Some((parent, tag)) = process::on_exit(pid, code) {
        let parent_ring = process::ring_pa(parent);
        if parent_ring != 0 {
            complete(parent_ring, tag, code);
        }
        if let Some(waiter) = process::take_waiter(parent) {
            sched::wake(waiter);
        }
    }
}

/// Complete the foreground process's parked reads from the kernel input buffer.
/// Called from the UART RX interrupt (after it drained the device into the
/// buffer) — CQEs land the moment a key arrives, while user code runs.
///
/// `copy_to_user` writes through the live `TTBR0`, but the interrupted task may
/// be in any address space (idle, the poller, or the *background* process). So we
/// temporarily install the foreground space for the copy and restore the prior
/// one before returning — IRQs are masked here, so nothing preempts between the
/// two switches. (A page-table-walk copy would avoid the switch entirely; that's
/// deferred with the rest of I/O multiplexing.)
pub fn poll_pending() {
    let pid = match process::foreground_pid() {
        Some(p) => p,
        None => return,
    };
    let ring_pa = process::ring_pa(pid);
    if ring_pa == 0 {
        return;
    }

    let prev = mmu::current_ttbr0();
    let fg = process::ttbr0(pid);
    if prev != fg {
        mmu::activate(fg);
    }

    let mut completed = false;
    for (i, slot) in process::pending_snapshot(pid).iter().enumerate() {
        if let Some(p) = *slot {
            let count = drain_input(pid, p.buf, p.len);
            if count > 0 {
                complete(ring_pa, p.user_data, count as i64);
                process::clear_pending(pid, i);
                completed = true;
            }
        }
    }

    if prev != fg {
        mmu::activate(prev);
    }

    // New completions may satisfy the task blocked in `enter`: wake it.
    if completed {
        if let Some(waiter) = process::take_waiter(pid) {
            sched::wake(waiter);
        }
    }
}

/// Drop process `pid`'s parked reads and waiter at teardown — their buffers are
/// about to be unmapped, and the waiter task is exiting. Their CQEs are simply
/// never posted; nobody is left to reap them.
pub fn abort_user(pid: usize) {
    let d = crate::irq::disable();
    process::clear_ring(pid);
    crate::irq::restore(d);
}

/// Deliver buffered console input into process `pid`'s user range `[buf, buf+len)`
/// via `copy_to_user`. The caller ensures `pid`'s space is live.
fn drain_input(pid: usize, buf: usize, len: usize) -> usize {
    let mut count = 0;
    while count < len {
        match crate::input::pop() {
            Some(b) => {
                // Validated at SQE-accept time; copy_to_user re-checks (the Linux
                // access_ok-and-copy shape). A failure here would drop the byte.
                if !crate::user::copy_to_user(pid, buf + count, &[b]) {
                    break;
                }
                count += 1;
            }
            None => break,
        }
    }
    count
}
