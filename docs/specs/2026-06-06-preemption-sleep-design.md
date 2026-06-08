# jos — Phase 11: Preemptive Scheduling + Sleep (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Make the scheduler preemptive (the timer IRQ drives context switches) and add
sleep/blocking (threads block on a wake-tick, woken by the timer). Builds on Phase 10.

## Decisions (from brainstorming)

- Preemption **reuses `context_switch`** called from the timer IRQ (xv6-style); `yield_now`
  and the timer both funnel into one `schedule()`.
- Scheduler critical sections run with **IRQs disabled** (single-core mutual exclusion) — the
  fix for the lock-across-switch/timer-race problem flagged in Phase 10.
- Sleep via a `Blocked` state + `wake_at`; `kmain` (task 0) is the always-runnable idle.
- This **replaces** the Phase 10 `ABCABCABC` cooperative demo (no longer deterministic once
  preemption is live).

## IRQ helpers (`irq.rs`, new)

`disable() -> u64` (save DAIF, set I), `restore(daif)`, `enable()`.

## Scheduler rework (`sched.rs`)

```rust
enum State { Runnable, Blocked, Exited }
struct Task { sp: usize, state: State, wake_at: u64 }

fn schedule()              // caller holds IRQs off: round-robin pick + context_switch
pub fn yield_now()         // irq::disable(); schedule(); irq::restore()
pub fn sleep_ticks(n)      // disable; mark Blocked, wake_at=ticks()+n; schedule(); restore
fn wake_sleepers(now)      // Blocked && wake_at<=now -> Runnable
pub fn tick()              // from timer IRQ (IRQs already off): wake_sleepers + schedule
```
`spawn`/`init`/`any_worker_alive` also bracket their scheduler access with `irq::disable/
restore`. `schedule()` skips `Blocked`/`Exited`; `kmain` is always `Runnable`, so there is
always a thread to run.

## Preemption path (`exceptions.rs`)

For the timer INTID, `handle_irq`:
```
timer::on_tick();        // count + reload (clears the timer line)
gic::eoi(intid);         // EOI before switching away
sched::tick();           // wake sleepers + preempt (IRQs already masked by hw)
return;
```
The preempted thread's trap frame stays on its stack; when rescheduled it resumes inside
`schedule()`, unwinds through the handler, and the stub `ERET`s it back.

## New-thread IRQ enable (`switch.s`)

When `schedule()` starts a brand-new thread from inside the IRQ, IRQs are still masked. So
`task_trampoline` enables them first, or a non-yielding new thread would never be preempted:
```asm
task_trampoline:
    msr daifclr, #2          // enable IRQs for the new thread
    mov x0, x20 ; blr x19 ; b task_exit
```

## Demo (`main.rs`)

```rust
extern "C" fn sleeper(_:usize) { for i in 1..=3 { kprintln!("[sleeper] woke {}",i); sched::sleep_ticks(3);} }
extern "C" fn busy(_:usize)    { for r in 1..=3 { kprintln!("[busy] round {}",r); spin_ticks(3);} uart::write_str("busy thread done\n"); }
fn spin_ticks(n) { let t = timer::ticks()+n; while timer::ticks() < t { core::hint::spin_loop(); } } // CPU-bound poll, never yields
```
`sched_self_check`: `init`, `spawn(sleeper)`, `spawn(busy)`, idle `while any_worker_alive() {
yield_now() }`, then `preempt+sleep self-check passed`.

`spin_ticks` is **tick-deterministic** (not iteration-count/TCG-speed dependent) and never
yields, so the timer must *preempt* it for the sleeper to run — the interleaved `[busy]` /
`[sleeper]` lines in the serial output are the visible proof of preemption. `sleeper woke 3`
proves sleep + timer-driven wakeup. `test.sh` asserts completion: greps `[sleeper] woke 3`,
`busy thread done`, `preempt+sleep self-check passed`. (Smoke window bumped to 2 s.)

## Files

| File | Change |
|---|---|
| `src/irq.rs` (new) | `disable`/`restore`/`enable`. |
| `src/sched.rs` | `Blocked`/`wake_at`, `schedule`, IRQ-bracketed `yield_now`/`sleep_ticks`/`spawn`/`init`, `tick`, `wake_sleepers`, `any_worker_alive`. |
| `src/switch.s` | `task_trampoline` enables IRQs. |
| `src/exceptions.rs` | `handle_irq` calls `sched::tick()` for the timer. |
| `src/main.rs` | `mod irq;`, `irq::enable()`, new `sleeper`/`busy`/`spin_ticks` demo. |
| `test.sh` | new markers; 2 s window. |

## Out of scope

Priorities/fairness, wait queues for non-timer events, sub-tick sleep, reaping, SMP,
per-thread FP save.
