//! Per-process state: the address space plus everything the kernel owns on a
//! user program's behalf.
//!
//! Replaces the former singletons (`LOADED`, `USER_FRAMES`, and the ring page +
//! its waiter). A [`Process`] is referenced by the scheduler's `Task` (kernel
//! tasks reference none); the process table is just where the `Process`es live.

use alloc::vec::Vec;

use crate::elf;
use crate::frames::{alloc_frame, Frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;

/// Virtual base of the user stack: 1 MiB into the user window (`abi::USER_BASE`).
pub const USER_STACK_VA: usize = abi::USER_BASE + 0x10_0000;
/// User stack size (one page).
pub const USER_STACK_SIZE: usize = FRAME_SIZE;

/// Maximum simultaneously parked reads per process.
pub const MAX_PENDING: usize = 8;

/// A parked `READ` awaiting console input.
#[derive(Clone, Copy)]
pub struct Pending {
    /// User-space destination buffer (virtual address), validated at accept.
    pub buf: usize,
    /// Capacity of that buffer in bytes.
    pub len: usize,
    /// The request's completion tag, echoed in its CQE.
    pub user_data: u64,
}

/// One user program's kernel-side state.
pub struct Process {
    /// Address-space root (`TTBR0` value). The scheduler installs it when a task
    /// bound to this process runs.
    pub ttbr0: u64,
    /// The loaded image's mapped ranges, for syscall-pointer validation.
    pub loaded: elf::Loaded,
    /// Every `(va, frame)` the program owns (segments + stack + ring), for teardown.
    pub frames: Vec<(usize, Frame)>,
    /// Physical address of this process's ring page (`0` until `SYS_RING_SETUP`).
    /// The kernel reaches the ring here, via the identity map, under any `TTBR0`.
    pub ring_pa: usize,
    /// Parked `READ`s awaiting input.
    pub pending: [Option<Pending>; MAX_PENDING],
    /// Task blocked in `enter` waiting for completions, woken by `poll_pending`.
    pub ring_waiter: Option<usize>,
    /// Whether this process owns the keyboard (its parked reads receive input).
    pub foreground: bool,
}

static PROCESSES: Locked<Vec<Process>> = Locked::new(Vec::new());

/// Create a user process: build (or reuse) its address space, load the image
/// into it, and map a stack. Returns the new process id (index into the table).
///
/// `ttbr0` is the address-space root to load into — a fresh space from
/// [`mmu::new_address_space`], so the program's segments/stack/ring live in this
/// process's private L0[1] while the kernel stays shared via L0[0].
pub fn create(image: &[u8], ttbr0: u64) -> usize {
    let mut frames = Vec::new();
    let loaded = elf::load(image, ttbr0, &mut frames);

    let stack = alloc_frame().expect("out of frames for the user stack");
    // SAFETY: `ttbr0` is a valid address-space root being populated before use.
    unsafe { mmu::map_page_in(mmu::l0_ptr(ttbr0), USER_STACK_VA, stack.addr(), PAGE_USER_RW) };
    frames.push((USER_STACK_VA, stack));

    let mut procs = PROCESSES.lock();
    procs.push(Process {
        ttbr0,
        loaded,
        frames,
        ring_pa: 0,
        pending: [None; MAX_PENDING],
        ring_waiter: None,
        foreground: false,
    });
    procs.len() - 1
}

/// Number of processes in the table (never shrinks; teardown leaves a husk).
pub fn count() -> usize {
    PROCESSES.lock().len()
}

/// The address-space root of process `pid`.
pub fn ttbr0(pid: usize) -> u64 {
    PROCESSES.lock()[pid].ttbr0
}

/// The entry virtual address of process `pid`.
pub fn entry(pid: usize) -> usize {
    PROCESSES.lock()[pid].loaded.entry
}

/// Physical address of process `pid`'s ring page (`0` if not yet set up).
pub fn ring_pa(pid: usize) -> usize {
    PROCESSES.lock()[pid].ring_pa
}

/// Record `pid`'s ring frame: store its PA and add it to the owned set so
/// teardown reclaims it.
pub fn set_ring(pid: usize, frame: Frame) {
    let mut procs = PROCESSES.lock();
    procs[pid].ring_pa = frame.addr();
    procs[pid].frames.push((abi::USER_RING_VA, frame));
}

/// Mark whether process `pid` owns the keyboard.
pub fn set_foreground(pid: usize, on: bool) {
    PROCESSES.lock()[pid].foreground = on;
}

/// The foreground process (the one that receives console input), if any.
pub fn foreground_pid() -> Option<usize> {
    PROCESSES.lock().iter().position(|p| p.foreground)
}

/// Park a `READ` for process `pid`. Returns false if its pending table is full.
pub fn park_read(pid: usize, p: Pending) -> bool {
    let mut procs = PROCESSES.lock();
    match procs[pid].pending.iter_mut().find(|s| s.is_none()) {
        Some(slot) => {
            *slot = Some(p);
            true
        }
        None => false,
    }
}

/// A copy of process `pid`'s parked-read table, for the completer to scan.
pub fn pending_snapshot(pid: usize) -> [Option<Pending>; MAX_PENDING] {
    PROCESSES.lock()[pid].pending
}

/// Clear parked-read slot `idx` of process `pid` (it has been completed).
pub fn clear_pending(pid: usize, idx: usize) {
    PROCESSES.lock()[pid].pending[idx] = None;
}

/// Record the task blocked in `enter` for process `pid`.
pub fn set_waiter(pid: usize, task: usize) {
    PROCESSES.lock()[pid].ring_waiter = Some(task);
}

/// Take (and clear) process `pid`'s blocked waiter, if any.
pub fn take_waiter(pid: usize) -> Option<usize> {
    PROCESSES.lock()[pid].ring_waiter.take()
}

/// Drop process `pid`'s ring-related state at teardown: parked reads point at
/// buffers about to be unmapped, and the waiter is exiting.
pub fn clear_ring(pid: usize) {
    let mut procs = PROCESSES.lock();
    procs[pid].pending = [None; MAX_PENDING];
    procs[pid].ring_waiter = None;
}

/// Take ownership of process `pid`'s `(va, frame)` list, leaving it empty. The
/// caller unmaps and frees them (teardown).
pub fn take_frames(pid: usize) -> Vec<(usize, Frame)> {
    core::mem::take(&mut PROCESSES.lock()[pid].frames)
}

/// Mark process `pid` exited: zero its `ring_pa` (so the poller skips it) and
/// drop its foreground claim. The husk stays in the table so indices are stable.
pub fn mark_exited(pid: usize) {
    let mut procs = PROCESSES.lock();
    procs[pid].ring_pa = 0;
    procs[pid].foreground = false;
}

/// Whether `[ptr, ptr + len)` lies entirely within process `pid`'s mapped user
/// memory. With `need_write`, the range must additionally be writable (the
/// stack, or a `PAGE_USER_RW` segment). The single gate for validating
/// user-supplied syscall pointers, now per-process.
pub fn is_user_range(pid: usize, ptr: usize, len: usize, need_write: bool) -> bool {
    let end = match ptr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    // The stack is always read/write.
    if ptr >= USER_STACK_VA && end <= USER_STACK_VA + USER_STACK_SIZE {
        return true;
    }
    let procs = PROCESSES.lock();
    let loaded = &procs[pid].loaded;
    for r in &loaded.ranges[..loaded.count] {
        if ptr >= r.start && end <= r.end {
            return !need_write || r.writable;
        }
    }
    false
}
