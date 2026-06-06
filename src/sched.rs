//! Cooperative kernel threads and a round-robin scheduler.
//!
//! A thread is a stack plus a saved register context (see `switch.s`). The saved
//! stack pointer is the thread's handle: `context_switch` pushes the callee-saved
//! registers, stores SP into the previous thread, loads the next thread's SP, and
//! restores. A freshly spawned thread gets a fabricated initial context so the
//! first switch "returns" into `task_trampoline`, which calls its entry function.

use core::arch::global_asm;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::sync::Locked;

global_asm!(include_str!("switch.s"));

/// Per-thread stack size (16 KiB), from the contiguous kernel heap.
const STACK_SIZE: usize = 16 * 1024;
/// Callee-saved registers saved by `context_switch` (x19..x30).
const CONTEXT_REGS: usize = 12;

extern "C" {
    fn context_switch(prev_sp: *mut usize, next_sp: usize);
    /// Trampoline for new threads; we only take its address (never call it from Rust).
    static task_trampoline: u8;
}

#[derive(PartialEq, Eq)]
enum State {
    Runnable,
    Exited,
}

struct Task {
    /// Saved stack pointer (the context lives at/above it). 0 for the bootstrap
    /// task until its first switch-out.
    sp: usize,
    state: State,
}

struct Scheduler {
    tasks: Vec<Task>,
    current: usize,
}

static SCHEDULER: Locked<Scheduler> = Locked::new(Scheduler {
    tasks: Vec::new(),
    current: 0,
});

/// Register the currently running code (`kmain`) as task 0. Call once before spawning.
pub fn init() {
    SCHEDULER.lock().tasks.push(Task {
        sp: 0,
        state: State::Runnable,
    });
}

/// Spawn a new kernel thread that runs `entry(arg)`.
pub fn spawn(entry: extern "C" fn(usize), arg: usize) {
    // Leaked: this phase never reaps tasks, so the stack lives forever.
    let stack: &'static mut [u8] = Box::leak(vec![0u8; STACK_SIZE].into_boxed_slice());
    let top = (stack.as_mut_ptr() as usize + STACK_SIZE) & !0xF; // 16-byte aligned
    let ctx = top - CONTEXT_REGS * 8;

    // SAFETY: ctx..top lies within the fresh stack; we fabricate the saved context
    // that `context_switch` restores (layout: x29,x30,x27,x28,...,x19,x20).
    unsafe {
        let p = ctx as *mut u64;
        for i in 0..CONTEXT_REGS {
            p.add(i).write(0);
        }
        p.add(1).write(core::ptr::addr_of!(task_trampoline) as u64); // x30 = trampoline
        p.add(10).write(entry as usize as u64); // x19 = entry
        p.add(11).write(arg as u64); // x20 = arg
    }

    SCHEDULER.lock().tasks.push(Task {
        sp: ctx,
        state: State::Runnable,
    });
}

/// Whether any spawned worker (task index >= 1) is still runnable.
pub fn any_worker_runnable() -> bool {
    let s = SCHEDULER.lock();
    s.tasks.iter().skip(1).any(|t| t.state == State::Runnable)
}

/// Voluntarily yield to the next runnable thread (round-robin). Returns when this
/// thread runs again, or immediately if it is the only runnable task.
pub fn yield_now() {
    let (prev_sp, next_sp) = {
        let mut s = SCHEDULER.lock();
        let cur = s.current;
        let n = s.tasks.len();
        let mut next = cur;
        for i in 1..=n {
            let cand = (cur + i) % n;
            if s.tasks[cand].state == State::Runnable {
                next = cand;
                break;
            }
        }
        if next == cur {
            return; // no other runnable task
        }
        s.current = next;
        let prev_sp = core::ptr::addr_of_mut!(s.tasks[cur].sp);
        let next_sp = s.tasks[next].sp;
        (prev_sp, next_sp)
    }; // lock released before switching (never hold a lock across a context switch)

    // SAFETY: single-core cooperative; prev_sp/next_sp are valid task contexts and
    // the scheduler Vec does not move while we hold these raw pointers.
    unsafe { context_switch(prev_sp, next_sp) };
}

/// Mark the current thread exited and yield away permanently. Reached from
/// `task_trampoline` when a thread's entry function returns.
#[no_mangle]
extern "C" fn task_exit() -> ! {
    {
        let mut s = SCHEDULER.lock();
        let cur = s.current;
        s.tasks[cur].state = State::Exited;
    }
    yield_now();
    // The scheduler skips Exited tasks, so we are never scheduled again.
    loop {
        core::hint::spin_loop();
    }
}
