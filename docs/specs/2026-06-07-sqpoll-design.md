# jatuli — Phase 15: SQPOLL (Kernel Submission Poller) (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** A kernel task that polls the submission queue, picking up SQEs without any
syscall — Linux's `IORING_SETUP_SQPOLL` in miniature, including the genuinely clever
part: the `NEED_WAKEUP` handshake through shared memory, so the trap reappears only
when the poller stopped watching. Demonstrated by a new jsh builtin that performs
prints with no syscall in the submission path.

## Reuses

- `block_current`/`wake`/scheduler tasks (Phase 14) — the poller is just another task
  that sleeps when idle.
- The jring drain logic (Phase 12) — now callable from two contexts, so it gains
  mutual exclusion.

## Honest single-core framing

Linux SQPOLL assumes a spare core to dedicate; jatuli has one CPU, so the poller
time-shares with the user task (poll → `yield_now` → user runs until the next tick).
Pickup latency is therefore up to one 10 ms tick, and SQPOLL here is *architectural*
demonstration, not a performance win. Also: jsh's normal submit-then-wait pattern
drains the SQ in the user's own `enter(1)` before the poller can — the poller only
sees work published by code that keeps running. Both facts are stated in the docs and
shape the demo (below).

## Shared-page addition: the flags word

`u32` at offset `0x10` (spare header space). Bit 0 = `NEED_WAKEUP`: set by the poller
before it blocks, cleared when it wakes. Kernel-written, user-read — same
release/acquire discipline as the indices.

## Kernel (`ring.rs`, `main.rs`)

- **Drain extraction + mutual exclusion**: the SQ-consuming loop moves into
  `drain(from_user) -> bool` (did any work), executed with IRQs disabled — it now has
  two callers (syscall `enter`, preemptible poller task) and per-SQE processing must
  not interleave. `enter` keeps its blocking `min_complete` logic on top.
- **Poller task** (`ring::sqpoll_main`, spawned from `kmain` before `user_task`):
  records its task id, then loops: `drain(true)`; if work was found, reset the idle
  clock and `yield_now()`; if idle for `SQ_IDLE_TICKS = 5` ticks (50 ms, the
  `sq_thread_idle` analog), set `NEED_WAKEUP`, `block_current()`; on wake clear the
  flag and resume polling. On its first-ever nonempty drain it prints
  `"[sqpoll] picked up work"` once — the test marker proving the poller (not `enter`)
  consumed submissions.
- **Waking the poller**: `enter` checks `NEED_WAKEUP` at entry; if set, wakes the
  poller (id in kernel state) and clears the flag. (The user task's own ring-waiter
  wake from Phase 14 is unchanged and separate.)
- The poller drains with `from_user = true`: SQEs it consumes came from EL0, so
  pointer validation applies exactly as in a user `enter`.

## User (`uring.rs`, `main.rs`)

- `submit()` becomes flag-aware: publish the tail, then read the flags word — only if
  `NEED_WAKEUP` is set call `enter(0)`. Poller awake → **zero syscalls to submit**.
- New jsh builtin **`spam`**: publishes three PRINT SQEs (`"spam 1\n"`..`"spam 3\n"`)
  via flag-aware submits, then waits for the last tag by **spinning on the CQ**
  (deliberately not `enter`, so the poller is the only possible consumer; the spin is
  confined to this demo). `help` output becomes `commands: help spam exit`.

## Demo / verification

`test.sh` session becomes `hellp\rhelp\rspam\rexit\r`; new markers:
`"commands: help spam exit"` (replaces the old help line), `"spam 3"`, and
`"[sqpoll] picked up work"`. The last one is the structural proof: it prints only when
the poller task itself consumed SQEs. Existing markers unchanged. `dump.sh` unaffected.

## Files

| File | Change |
|---|---|
| `kernel/src/ring.rs` | flags word, `drain()` with IRQ-off exclusion, `sqpoll_main`, poller wake in `enter`. |
| `kernel/src/main.rs` | spawn the poller task before the user task. |
| `user/src/uring.rs` | flag-aware `submit()`; expose a CQ-spin reap for the demo. |
| `user/src/main.rs` | `spam` builtin; help text. |
| `test.sh` | spam exchange + new markers. |
| `README.md` | layout-line updates. |

## Out of scope (named later phases)

IOPOLL-style device polling · multiple rings / per-process pollers · poller CPU
affinity (single core) · kernel input buffer (still the next tty-shaped phase) ·
auto-tuned idle timeouts.
