//! Preemptive kernel threads with sleep/blocking.
//!
//! Both voluntary `yield_now` and the timer IRQ funnel into `schedule()`, which
//! picks the next runnable thread and calls `context_switch` (see `switch.s`).
//! Scheduler state is protected by disabling IRQs (single-core mutual exclusion):
//! the timer can't fire mid-update, so the spinlock never contends. A new thread's
//! `task_trampoline` enables IRQs before running, so every thread is preemptible.

use core::arch::global_asm;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::irq;
use crate::sync::Locked;
use crate::timer;

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
    Blocked,
    Exited,
}

struct Task {
    /// Saved stack pointer (the context lives at/above it). 0 for the bootstrap
    /// task until its first switch-out.
    sp: usize,
    state: State,
    /// Tick at which a `Blocked` task should be woken.
    wake_at: u64,
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
    let d = irq::disable();
    SCHEDULER.lock().tasks.push(Task {
        sp: 0,
        state: State::Runnable,
        wake_at: 0,
    });
    irq::restore(d);
}

/// Spawn a new kernel thread that runs `entry(arg)`.
pub fn spawn(entry: extern "C" fn(usize), arg: usize) {
    // Leaked: this phase never reaps tasks, so the stack lives forever.
    let stack: &'static mut [u8] = Box::leak(vec![0u8; STACK_SIZE].into_boxed_slice());
    let top = (stack.as_mut_ptr() as usize + STACK_SIZE) & !0xF;
    let ctx = top - CONTEXT_REGS * 8;

    // SAFETY: ctx..top lies within the fresh stack; we fabricate the saved context
    // restored by `context_switch` (layout: x29,x30,x27,x28,...,x19,x20).
    unsafe {
        let p = ctx as *mut u64;
        for i in 0..CONTEXT_REGS {
            p.add(i).write(0);
        }
        p.add(1).write(core::ptr::addr_of!(task_trampoline) as u64); // x30 = trampoline
        p.add(10).write(entry as usize as u64); // x19 = entry
        p.add(11).write(arg as u64); // x20 = arg
    }

    let d = irq::disable();
    SCHEDULER.lock().tasks.push(Task {
        sp: ctx,
        state: State::Runnable,
        wake_at: 0,
    });
    irq::restore(d);
}

/// Whether any spawned worker (task index >= 1) has not yet exited.
pub fn any_worker_alive() -> bool {
    let d = irq::disable();
    let alive = SCHEDULER.lock().tasks.iter().skip(1).any(|t| t.state != State::Exited);
    irq::restore(d);
    alive
}

/// Pick the next runnable thread and switch to it. The caller MUST hold IRQs
/// disabled. Returns (in the calling thread) once it is scheduled again.
fn schedule() {
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
            return; // nothing else runnable; keep running
        }
        s.current = next;
        let prev_sp = core::ptr::addr_of_mut!(s.tasks[cur].sp);
        let next_sp = s.tasks[next].sp;
        (prev_sp, next_sp)
    };
    // SAFETY: IRQs are disabled; prev_sp/next_sp are valid task contexts and the
    // scheduler Vec does not move while we hold these raw pointers.
    unsafe { context_switch(prev_sp, next_sp) };
}

/// Voluntarily yield to the next runnable thread.
pub fn yield_now() {
    let d = irq::disable();
    schedule();
    irq::restore(d);
}

/// Index of the currently running task.
pub fn current() -> usize {
    let d = irq::disable();
    let cur = SCHEDULER.lock().current;
    irq::restore(d);
    cur
}

/// Block the current task until an explicit [`wake`]. (`wake_at = u64::MAX`
/// keeps the time-based waker's hands off it.) Returns once woken; the caller
/// must re-check its wait condition — never trust a wake.
pub fn block_current() {
    let d = irq::disable();
    {
        let mut s = SCHEDULER.lock();
        let cur = s.current;
        s.tasks[cur].wake_at = u64::MAX;
        s.tasks[cur].state = State::Blocked;
    }
    schedule();
    irq::restore(d);
}

/// Make a blocked task runnable again (no-op for any other state). Safe to
/// call from IRQ context.
pub fn wake(task: usize) {
    let d = irq::disable();
    {
        let mut s = SCHEDULER.lock();
        if s.tasks[task].state == State::Blocked {
            s.tasks[task].state = State::Runnable;
        }
    }
    irq::restore(d);
}

/// Mark the current task exited and switch away permanently.
pub fn exit_current() -> ! {
    task_exit()
}

/// Block the current thread for `n` timer ticks.
pub fn sleep_ticks(n: u64) {
    let d = irq::disable();
    {
        let mut s = SCHEDULER.lock();
        let cur = s.current;
        s.tasks[cur].wake_at = timer::ticks() + n;
        s.tasks[cur].state = State::Blocked;
    }
    schedule();
    irq::restore(d);
}

/// Wake any blocked thread whose wake time has arrived.
fn wake_sleepers(now: u64) {
    let mut s = SCHEDULER.lock();
    for t in s.tasks.iter_mut() {
        if t.state == State::Blocked && t.wake_at <= now {
            t.state = State::Runnable;
        }
    }
}

/// Called from the timer IRQ (IRQs already masked by hardware): wake due sleepers
/// and preempt the current thread.
pub fn tick() {
    wake_sleepers(timer::ticks());
    schedule();
}

/// Mark the current thread exited and switch away permanently. Reached from
/// `task_trampoline` when a thread's entry function returns.
#[no_mangle]
extern "C" fn task_exit() -> ! {
    let _ = irq::disable();
    {
        let mut s = SCHEDULER.lock();
        let cur = s.current;
        s.tasks[cur].state = State::Exited;
    }
    schedule();
    // The scheduler skips Exited tasks, so we are never scheduled again.
    loop {
        core::hint::spin_loop();
    }
}
