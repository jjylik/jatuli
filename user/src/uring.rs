//! Userspace half of `jring` (a ~60-line liburing).
//!
//! The page layout, opcodes, and syscall numbers come from the shared `abi`
//! crate — the same definitions the kernel compiles against, so the two sides
//! cannot drift. [`sqe`] writes are plain memory stores; [`submit`] traps only
//! when the kernel's SQ poller raised `NEED_WAKEUP`, and a parked `READ`
//! completes from the kernel's UART interrupt — the CQE simply appears in
//! shared memory.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use abi::{RingPage, NEED_WAKEUP, RING_MASK, SYS_RING_ENTER, SYS_RING_SETUP};

pub use abi::{OP_NOP, OP_PRINT, OP_READ};

/// Ring page VA, returned by `SYS_RING_SETUP`. (Lives in our .bss — which also
/// gives the program a writable PT_LOAD segment, exercising the loader's
/// per-segment permissions.)
static RING_VA: AtomicUsize = AtomicUsize::new(0);

/// Completions reaped while waiting for a different tag (tag 0 = empty slot;
/// max in-flight is small — the shell never exceeds a banner batch).
static STASH: [(AtomicU64, AtomicU64); 4] = [
    (AtomicU64::new(0), AtomicU64::new(0)),
    (AtomicU64::new(0), AtomicU64::new(0)),
    (AtomicU64::new(0), AtomicU64::new(0)),
    (AtomicU64::new(0), AtomicU64::new(0)),
];

/// The shared ring page, typed. The ONE place in userspace where the ring's
/// raw address becomes a reference; sound only after [`setup`] stored the VA.
fn page() -> &'static RingPage {
    // SAFETY: setup() mapped the page and recorded its address; all fields
    // are atomics, so sharing with the kernel through &-references is sound.
    unsafe { &*(RING_VA.load(Ordering::Relaxed) as *const RingPage) }
}

/// Map the ring (idempotent) and remember where it lives. Call once at startup.
pub fn setup() {
    let va: u64;
    // SAFETY: SYS_RING_SETUP takes no arguments and returns the ring page VA.
    unsafe {
        asm!("svc #0", in("x8") SYS_RING_SETUP, out("x0") va, in("x1") 0u64, options(nostack));
    }
    RING_VA.store(va as usize, Ordering::Relaxed);
}

/// Queue one submission: write the SQE, then publish it with a release store
/// of the tail (the kernel acquires the tail before reading entries).
pub fn sqe(op: u64, a0: u64, a1: u64, tag: u64) {
    let p = page();
    let tail = p.sq_tail.load(Ordering::Relaxed); // we are the only producer
    let e = &p.sq[(tail & RING_MASK) as usize];
    e.opcode.store(op, Ordering::Relaxed);
    e.arg0.store(a0, Ordering::Relaxed);
    e.arg1.store(a1, Ordering::Relaxed);
    e.user_data.store(tag, Ordering::Relaxed);
    p.sq_tail.store(tail.wrapping_add(1), Ordering::Release);
}

/// `SYS_RING_ENTER(min_complete)`: process published submissions, then block
/// in the kernel until the CQ holds at least `min_complete` unreaped entries.
fn enter(min_complete: u64) {
    // SAFETY: SYS_RING_ENTER reads only the (already published) ring.
    unsafe {
        asm!("svc #0", in("x8") SYS_RING_ENTER, in("x0") min_complete, in("x1") 0u64, options(nostack));
    }
}

/// Hand published submissions to the kernel. With the SQ poller awake this is
/// **zero syscalls** — the entries are already visible in shared memory and
/// the poller will consume them; we only trap if it raised `NEED_WAKEUP`.
pub fn submit() {
    if page().flags.load(Ordering::Acquire) & NEED_WAKEUP != 0 {
        enter(0);
    }
}

/// Wait for the completion tagged `tag`; returns its result. When the CQ is
/// empty we *sleep* in the kernel (`enter(min_complete = 1)`) until a CQE
/// exists — no spinning. Completions for other tags reaped along the way are
/// stashed, not lost.
pub fn wait(tag: u64) -> i64 {
    reap(tag, true)
}

/// Like [`wait`], but busy-poll the CQ instead of sleeping — no syscall on
/// this path at all. Demo use only (see jsh's `spam`): with the SQ poller
/// consuming submissions, publish + spin proves I/O with zero traps.
pub fn wait_spin(tag: u64) -> i64 {
    reap(tag, false)
}

fn reap(tag: u64, block: bool) -> i64 {
    // Did an earlier wait already reap it?
    for (t, r) in STASH.iter() {
        if tag != 0 && t.load(Ordering::Relaxed) == tag {
            t.store(0, Ordering::Relaxed);
            return r.load(Ordering::Relaxed) as i64;
        }
    }
    let p = page();
    loop {
        let head = p.cq_head.load(Ordering::Relaxed); // we are the only consumer
        let tail = p.cq_tail.load(Ordering::Acquire);
        if head == tail {
            if block {
                enter(1); // sleep until at least one completion is available
            } else {
                core::hint::spin_loop();
            }
            continue;
        }
        let cqe = &p.cq[(head & RING_MASK) as usize];
        let (got, res) = (
            cqe.user_data.load(Ordering::Relaxed),
            cqe.result.load(Ordering::Relaxed),
        );
        p.cq_head.store(head.wrapping_add(1), Ordering::Release);
        if got == tag {
            return res;
        }
        for (t, r) in STASH.iter() {
            if t.load(Ordering::Relaxed) == 0 {
                t.store(got, Ordering::Relaxed);
                r.store(res as u64, Ordering::Relaxed);
                break;
            }
        }
    }
}
