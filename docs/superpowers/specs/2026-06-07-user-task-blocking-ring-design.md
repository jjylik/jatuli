# jos — Phase 14: User Program as a Scheduler Task + Blocking Ring Waits (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** Make the user program a schedulable task so it can genuinely *block*: jsh
waiting for a keystroke sleeps (kernel idles in `wfi`) instead of spinning on the CQ,
and the UART interrupt *wakes* it. `SYS_RING_ENTER` gains `min_complete` (Linux's
`io_uring_enter` shape). No spinning remains anywhere in the input path.

## Reuses

- The existing scheduler wholesale (Phase 8/9 machinery): per-task 16 KiB kernel
  stacks, `context_switch`, preemption from the timer IRQ, `Blocked` + `wake_at`.
  Key enabling facts, verified in `sched.rs`/`switch.s`/`exceptions.s`: preemption
  already context-switches from inside an IRQ handler on the interrupted task's own
  stack, and every exception saves/restores `ELR`/`SPSR` in its trap frame, so trap
  state survives a switch.
- jring + UART RX interrupt (Phases 12/13): `poll_pending()` becomes a *wake* source.

## The one structural change

Today `kmain` calls `enter_user()` directly, so every EL0 trap lands on the boot stack
(task 0's). Instead:

- `kmain` (task 0) becomes the **idle task**: after the self-checks,
  `sched::spawn(user_task, 0)`, then `loop { wfi; yield_now() }`.
- `user_task` (a normal spawned kernel thread) calls `user::enter_user()` — the ERET
  happens on its own stack, so every subsequent EL0 trap (syscall or IRQ) pushes its
  frame there. Blocking inside a syscall handler is then ordinary `schedule()` — the
  same move `sleep_ticks` already makes.

Single-user-task assumption, stated not engineered around: only one task ever ERETs to
EL0, so `SP_EL0` and the user page mappings are never clobbered by another task. This
holds until per-process address spaces.

## Scheduler additions (`sched.rs`)

```rust
pub fn current() -> usize;       // index of the running task
pub fn block_current();          // state = Blocked, wake_at = u64::MAX, schedule()
pub fn wake(task: usize);        // Blocked -> Runnable (no-op otherwise)
pub fn exit_current() -> !;      // public version of task_exit's marking + schedule
```

`wake_sleepers` only wakes `wake_at <= now`, so `u64::MAX` means "woken only by an
explicit `wake()`" — sleep-by-time and block-on-event share the `Blocked` state.

## Blocking `SYS_RING_ENTER`

ABI: `x0 = min_complete` (0 = submit-only, today's behavior). After draining
submissions, `ring::enter` loops:

1. `unreaped = cq_tail - cq_head` (both read from the shared page; a hostile user can
   lie and block themselves forever or never — liveness-only, consistent with the
   Phase 12 trust analysis).
2. If `unreaped >= min_complete`: return.
3. Record `waiter = Some(sched::current())` in kernel-side ring state, then
   `sched::block_current()`. On wake, loop back to 1 (recheck, never trust the wake).

**Lost-wakeup analysis (why this is sound):** the entire syscall handler runs with IRQs
masked, and they stay masked until `context_switch` lands in a task that re-enables
them. So no CQE can be posted between the `unreaped` check and the block — the wake
source (UART IRQ) physically cannot run in that window. Single core makes this
airtight. The kernel-side `waiter` field lives in `RingState` (kernel `.bss`, not the
shared page — the user must not be able to forge a wake target), and the ring lock is
**not** held across `block_current()` (the IRQ path needs it).

## Wake path

`ring::poll_pending()` (UART IRQ): after completing any CQE, if a waiter is recorded,
`sched::wake(waiter)` and clear it. Sequence while jsh sleeps: idle task sits in `wfi`
→ key arrives → UART IRQ (handled on idle's stack) → CQE posted + waiter woken → handler
returns to idle → `yield_now()` switches to the user task → it resumes inside
`ring::enter`'s recheck loop → returns through the trap frame → ERET to EL0 with the
completion already reaped-able. Wake latency is one `wfi` wakeup, effectively immediate.

## `SYS_EXIT` via the scheduler

The wfi-park in the `SYS_EXIT` arm would now freeze the machine (it would never let the
idle task run again). Replace it: print the exit message, then `sched::exit_current()` —
the user task is marked `Exited` and the idle task runs forever. Observable behavior
unchanged (`"[user] exited with code 0"` then quiet ticking).

## User side (`uring.rs`)

`submit()` = `enter(min_complete = 0)`. `wait(tag)`:

1. Check the stash; reap whatever the CQ holds (stashing non-matching tags). Found → return.
2. Nothing reaped: `enter(min_complete = 1)` — *sleeps in the kernel* until a CQE
   exists — then loop back to 1.

The user-side spin (`spin_loop` in `wait`) is deleted. jsh logic is unchanged — it just
stops burning CPU between keystrokes.

## Demo / verification

`test.sh` unchanged and must stay green at every stage. Staged implementation, each
independently tested:

1. Scheduler additions (no behavior change).
2. User-as-task + `SYS_EXIT` via scheduler (shell still spin-waits; now preemption
   round-robins idle ↔ user, exercising switch-from-interrupted-EL0).
3. Blocking enter + IRQ wake + non-spinning `wait()`.

The structural proof at stage 3: the only remaining wake source for a parked READ is
the UART IRQ → `wake()` chain; if the piped session still completes, blocking works.

## Files

| File | Change |
|---|---|
| `kernel/src/sched.rs` | `current`, `block_current`, `wake`, `exit_current`. |
| `kernel/src/ring.rs` | `enter(from_user, min_complete)` with block/recheck; waiter + wake in `poll_pending`. |
| `kernel/src/syscall.rs` | pass `x0` as `min_complete`; `SYS_EXIT` arm uses `exit_current`. |
| `kernel/src/main.rs` | spawn `user_task`; `kmain` becomes the idle loop. |
| `user/src/uring.rs` | `wait` sleeps via `enter(1)` instead of spinning. |
| `README.md` | layout/description updates. |

## Out of scope (named later phases)

Kernel input buffer + `copy_to_user` (tty shape) · SQPOLL poller task · a general
waitqueue type (one waiter suffices until there are more sleepers) · multiple user
tasks / per-process address spaces · reaping exited tasks' stacks.
