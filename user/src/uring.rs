//! Userspace half of `jring` (a ~60-line liburing).
//!
//! The kernel maps one shared page holding a submission queue (we produce,
//! kernel consumes) and a completion queue (kernel produces, we consume).
//! [`sqe`] writes are plain memory stores; [`submit`] is the only syscall, and
//! a parked `READ` completes later from the kernel's timer interrupt — the CQE
//! appears in [`wait`]'s spin with no syscall at all.

use core::arch::asm;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

const SYS_RING_SETUP: u64 = 5;
const SYS_RING_ENTER: u64 = 6;

pub const OP_NOP: u64 = 0;
pub const OP_PRINT: u64 = 1;
pub const OP_READ: u64 = 2;

const ENTRIES: u32 = 16;
const MASK: u32 = ENTRIES - 1;
const SQ_TAIL: usize = 0x04;
const CQ_HEAD: usize = 0x08;
const CQ_TAIL: usize = 0x0c;
const FLAGS: usize = 0x10;
const SQ_OFF: usize = 0x40;
const CQ_OFF: usize = 0x280;

/// Flags-word bit: the kernel's SQ poller went to sleep; one `enter` revives it.
const NEED_WAKEUP: u32 = 1;

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

fn ring() -> usize {
    RING_VA.load(Ordering::Relaxed)
}

fn index(off: usize) -> &'static AtomicU32 {
    // SAFETY: the four ring indices are aligned u32s in the mapped ring page.
    unsafe { &*((ring() + off) as *const AtomicU32) }
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
    let tail = index(SQ_TAIL).load(Ordering::Relaxed); // we are the only producer
    let p = (ring() + SQ_OFF + (tail & MASK) as usize * 32) as *mut u64;
    // SAFETY: in-bounds SQE slot in the mapped ring page.
    unsafe {
        p.write_volatile(op);
        p.add(1).write_volatile(a0);
        p.add(2).write_volatile(a1);
        p.add(3).write_volatile(tag);
    }
    index(SQ_TAIL).store(tail.wrapping_add(1), Ordering::Release);
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
    if index(FLAGS).load(Ordering::Acquire) & NEED_WAKEUP != 0 {
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
    loop {
        let head = index(CQ_HEAD).load(Ordering::Relaxed); // we are the only consumer
        let tail = index(CQ_TAIL).load(Ordering::Acquire);
        if head == tail {
            if block {
                enter(1); // sleep until at least one completion is available
            } else {
                core::hint::spin_loop();
            }
            continue;
        }
        let p = (ring() + CQ_OFF + (head & MASK) as usize * 16) as *const u64;
        // SAFETY: in-bounds CQE slot, published by the release store of cq_tail.
        let (got, res) = unsafe { (p.read_volatile(), p.add(1).read_volatile() as i64) };
        index(CQ_HEAD).store(head.wrapping_add(1), Ordering::Release);
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
