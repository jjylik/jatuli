# jos — Phase 10: Cooperative Kernel Scheduler (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Multiple kernel threads (EL1) that round-robin via a voluntary `yield()`, with a
real context switch. Deterministic demo: three threads print `A`/`B`/`C` → `ABCABCABC`, then
the kernel prints a pass marker and continues (into the EL0 phase, unchanged).

## Decisions (from brainstorming)

- **Cooperative** (yield-based), not preemptive — preemption is the next phase.
- Context switch saves only **callee-saved** registers (x19–x30) + SP — it's a normal call.
- Per-thread **stacks from the heap** (`Box<[u8; 16 KiB]>`, contiguous), leaked (tasks aren't
  reaped this phase).
- Round-robin, skipping exited tasks; `kmain` is task 0 and acts as the idle task.

## Context switch (`switch.s`)

`context_switch(prev_sp: *mut usize /*x0*/, next_sp: usize /*x1*/)`:
```asm
stp x19,x20,[sp,#-16]! ; ... ; stp x29,x30,[sp,#-16]!   // push x19..x30 (12 regs)
mov x9, sp ; str x9, [x0]   // *prev_sp = sp
mov sp, x1                  // switch stacks
ldp x29,x30,[sp],#16 ; ... ; ldp x19,x20,[sp],#16        // pop x19..x30
ret                         // return into next thread's restored LR
```
The saved SP is the thread handle. Saved block layout (low→high): `x29,x30,x27,x28,x25,x26,
x23,x24,x21,x22,x19,x20`.

## New-thread bootstrap

`spawn` fabricates that 12-register block on a fresh stack so the first switch "returns"
into the trampoline: `x30 = task_trampoline`, `x19 = entry`, `x20 = arg`, rest 0.
```asm
task_trampoline:
    mov x0, x20 ; blr x19    // entry(arg)
    b   task_exit            // entry returned -> exit
```

## Tasks & scheduler (`sched.rs`)

```rust
enum State { Runnable, Exited }
struct Task { sp: usize, state: State }     // stack is leaked, not owned here
struct Scheduler { tasks: Vec<Task>, current: usize }
static SCHEDULER: Locked<Scheduler> = Locked::new(Scheduler { tasks: Vec::new(), current: 0 });
```
- `init()` — push `kmain` as task 0 (`sp = 0`, filled on first switch-out; never fabricated).
- `spawn(entry, arg)` — leak a 16 KiB heap stack, fabricate the initial block, push a
  `Runnable` task. (`task_trampoline` address via `extern static` + `addr_of!`; `entry` is a
  fn pointer, so `entry as usize` is fine.)
- `yield_now()` — round-robin from `current+1` to the next `Runnable`; if found, drop the
  lock and `context_switch(&mut tasks[cur].sp, tasks[next].sp)`. If the caller is the only
  runnable, return.
- `task_exit() -> !` (`#[no_mangle]`) — mark current `Exited`, `yield_now()`; exited tasks
  are skipped so it never returns.
- `any_worker_runnable()` — any task index ≥ 1 still `Runnable`.

## Demo (`main.rs`, before `enter_user`)

```rust
fn sched_self_check() {
    sched::init();
    sched::spawn(worker, b'A' as usize);
    sched::spawn(worker, b'B' as usize);
    sched::spawn(worker, b'C' as usize);
    while sched::any_worker_runnable() { sched::yield_now(); }   // kmain idles, yielding
    uart::write_str("\nscheduler self-check passed\n");
}
extern "C" fn worker(letter: usize) {
    for _ in 0..3 { kprint!("{}", letter as u8 as char); sched::yield_now(); }
}
```
Output: `ABCABCABC` then `scheduler self-check passed`. `test.sh` greps both.

## Design note (load-bearing)

A scheduler must **not hold a lock across a context switch**: `yield_now` extracts
`(prev_sp, next_sp)` under the lock, **drops it**, then switches. Sound here only because
single-core + cooperative (the timer IRQ doesn't touch scheduler state). Preemption will
require reworking this. Timer IRQs during `context_switch` are harmless (they push their
trap frame below the current SP and restore it).

## Files

| File | Change |
|---|---|
| `src/switch.s` (new) | `context_switch`, `task_trampoline`. |
| `src/sched.rs` (new) | `Task`/`Scheduler`/`init`/`spawn`/`yield_now`/`task_exit`/`any_worker_runnable`. |
| `src/main.rs` | `mod sched;`, `sched_self_check` + `worker`, call before `enter_user`. |
| `test.sh` | `ABCABCABC`, `scheduler self-check passed`. |

## Out of scope

Preemption (timer-driven switching), priorities/sleep/blocking, thread join, per-thread FP
(`d8–d15`) save, IRQ-safe scheduler locking, reaping exited tasks, SMP.
