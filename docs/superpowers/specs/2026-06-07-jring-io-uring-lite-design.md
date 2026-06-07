# jos — Phase 12: `jring`, an io_uring-lite (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** Replace the per-call I/O syscalls with an io_uring-style interface: submission
and completion rings in shared memory, batched submission via one syscall, opaque
completion tags, and genuinely asynchronous completion — a parked `READ` is completed
from the timer interrupt while user code runs, with no syscall involved. `SYS_PRINT`
and `SYS_READ` are deleted; all "action" I/O flows through the ring.

Context: inspired by LWN's "moving beyond fork() + exec()" discussion. jos never had
fork, so the spawn-related ideas wait for per-process address spaces; this phase builds
the io_uring mechanism itself, in its honest reduced form (see "What this is not").

## Reuses

- Shared user/kernel address space + `PAGE_USER_RW` (Phases 5/9): the rings are just a
  mapped page both sides dereference.
- Pointer validation (`is_user_range`, `is_user_range_writable`, Phases 9/11), now
  applied per-SQE with failures reported as negative CQE results.
- The 100 Hz timer IRQ (Phase 8) as the completion engine for parked reads.
- `uart::try_getc` (Phase 11).

## Shared ring page (ABI)

One 4 KiB frame mapped `PAGE_USER_RW` at `USER_RING_VA = 0x2_0030_0000`. Layout
(offsets within the page):

```
0x000  sq_head  u32   kernel advances after consuming
0x004  sq_tail  u32   user advances after publishing SQEs
0x008  cq_head  u32   user advances after reaping
0x00c  cq_tail  u32   kernel advances after posting
0x040  SQ: 16 x SQE { opcode u64, arg0 u64, arg1 u64, user_data u64 }   (32 B)
0x280  CQ: 16 x CQE { user_data u64, result i64 }                       (16 B)
```

Indices free-run and are masked with `& 15`. Producer publishes entries *then* bumps its
tail (release); consumer reads the tail (acquire) before entries. `user_data` is an
opaque tag echoed in the CQE — completions may arrive out of order; the tag is how the
user matches them.

Opcodes: `NOP = 0` (completes with 0), `PRINT = 1` (`arg0` ptr, `arg1` len),
`READ = 2` (`arg0` buf, `arg1` len; completes with bytes-read once input arrives).
CQE result is `>= 0` for success or a negative error (`-1` invalid pointer/opcode).

## Syscall surface

`SYS_PRINT (2)` and `SYS_READ (4)` are **deleted**. New:

```rust
pub const SYS_RING_SETUP: u64 = 5;  // map + zero the ring page; returns USER_RING_VA
pub const SYS_RING_ENTER: u64 = 6;  // process new submissions; returns 0
```

Final table: `ADD` (kept as the raw-SVC demo), `EXIT`, `RING_SETUP`, `RING_ENTER`.
`EXIT` stays a plain syscall on principle: it destroys the context that would reap its
completion (Linux's io_uring has no exit op either). `RING_SETUP` is idempotent.
`kmain`'s `syscall_self_check` drops its `SYS_PRINT` call (`"Hello from a syscall!"`
marker removed); a new `ring_self_check` exercises setup + a NOP/PRINT/READ-error batch
from EL1 before user entry.

## Kernel: `ring.rs`

Owns the ring state and a fixed **pending table** (8 slots: `{buf, len, user_data}`).

- `setup()`: allocate a frame, `map_page` at `USER_RING_VA`, zero it, remember it.
- `enter(from_user)`: drain `sq_head..sq_tail`. Per SQE: validate pointers (gated on
  `from_user`, like the old syscalls); `NOP`/`PRINT` execute and complete immediately;
  `READ` tries `uart::try_getc` now — completes if data is waiting, otherwise parks in
  the pending table (table full → complete with `-1`). Bad opcode → `-1`.
- `poll_pending()`: called from the timer IRQ handler each tick. For each pending slot
  with input available: fill the user buffer, post the CQE, free the slot.
- CQ overflow is impossible by construction: in-flight ops (16 SQ slots + 8 pending)
  never exceed completions the user hasn't reaped... conservatively, the kernel posts at
  most 16 CQEs per drain and the user reaps before submitting more; the shell's usage
  keeps depth ≤ 3. Document rather than engineer around.

Concurrency: syscall handlers run with IRQs masked, so `enter` and `poll_pending` never
interleave; user↔IRQ ring sharing is safe via the release/acquire index discipline
(single core). Kernel touches the ring through `USER_RING_VA` (same address space).

## User: `user/src/uring.rs` (a ~60-line liburing)

```rust
pub fn setup();                                      // SYS_RING_SETUP once
pub fn sqe(op: u64, a0: u64, a1: u64, tag: u64);     // write entry, bump sq_tail
pub fn submit();                                     // SYS_RING_ENTER (one svc)
pub fn wait(tag: u64) -> i64;                        // spin-reap CQ until tag appears
```

`wait` busy-polls `cq_head..cq_tail` (the user program is not schedulable yet — there is
nothing to sleep on), reaping entries in order and returning when the tag matches;
non-matching completions before it are remembered (small reaped-tags cache) so no CQE is
lost. jsh changes: `print` = `sqe(PRINT) + submit + wait`; `read_line`'s byte fetch =
`sqe(READ) + submit + wait` — after the ENTER that parks the READ, the keystroke path is
interrupt → CQE → user reap, **zero syscalls**. The banner is submitted as a
NOP + PRINT batch: one syscall, two tag-matched completions (batching demo).

## What this is not (honest gaps vs Linux io_uring)

- **Polled, not event-driven completion**: parked reads complete on the next 100 Hz tick
  (≤ 10 ms latency), not on a UART RX interrupt. Named refinement: enable PL011 RX IRQ
  (INTID 33) and complete instantly.
- **No general async engine**: one hardcoded pendable op; no io-wq workers, no linked
  SQEs, no other opcodes. Generalizing requires the scheduler-integration phase.
- **No sleeping wait**: `wait` spins; blocking `enter(min_complete)` needs the user
  program to be a schedulable task. Same root cause defers SQPOLL.
- No registered buffers/files, multishot, CQ-overflow handling, ring resizing.

## Demo / verification

Same piped session (`hellp\rhelp\rexit\r`); every jsh line now flows through ring
PRINTs, so existing markers already prove the ring path. Marker changes: remove
`"Hello from a syscall!"`; add `"ring self-check passed"`. Latency: imperceptible
(≤ 10 ms/keystroke at 100 Hz).

## Files

| File | Change |
|---|---|
| `kernel/src/ring.rs` (new) | ring state, `setup`/`enter`/`poll_pending`, pending table. |
| `kernel/src/syscall.rs` | delete `SYS_PRINT`/`SYS_READ` + handlers; add `RING_SETUP`/`RING_ENTER`. |
| `kernel/src/exceptions.rs` | timer-IRQ arm also calls `ring::poll_pending()`. |
| `kernel/src/main.rs` | `mod ring;`, `syscall_self_check` without print, new `ring_self_check`. |
| `user/src/uring.rs` (new) | setup/sqe/submit/wait. |
| `user/src/main.rs` | jsh I/O via the ring; banner batch demo. |
| `test.sh` | marker updates. |
| `README.md` | layout + syscall description updates. |

## Out of scope (named later phases)

UART RX interrupt completion · shell as a scheduler task (then: sleeping waits, SQPOLL,
io-wq-style workers) · linked SQEs / more opcodes · `SPAWN` op (needs per-process
address spaces) · registered buffers.
