# jos — Phase 18: EL0 Fault Handling + Process Teardown (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** A user-program bug stops being a kernel emergency. A fault at EL0 (data abort,
instruction abort, anything that isn't an `SVC`) kills the user task with a diagnostic —
the jos analog of SIGSEGV — and the kernel carries on. Exit and death both reclaim the
program's memory: segments and stack are unmapped and their frames returned to the pool.
Completes the *death* half of the process lifecycle before the multi-process arc.

## Reuses

- Lower-EL exception decoding (Phase 7): EL0 faults already arrive distinguishably
  (vectors 8–11, `ESR_EL1.EC`); they just currently fall into `report_and_halt`.
- `sched::exit_current` (Phase 14): retiring the user task is one call.
- `free_frame` (Phase 4) and the loader's knowledge of every frame it allocated.

## Fault path (`exceptions.rs`)

In `exception_dispatch`, a synchronous exception with `kind >= 8` (lower EL) whose `EC`
is not `SVC` no longer halts. New handler:

```
[user] killed: <ec name>, ELR=<addr>, FAR=<addr>
```

then `user::teardown()` and `sched::exit_current()` — the idle task (and the SQPOLL
poller) keep running; the machine survives. Same-EL faults (kernel bugs) still
`report_and_halt`: a kernel that killed *itself* on its own fault would hide real bugs.

## Teardown (`user.rs`, `mmu.rs`, `elf.rs`, `ring.rs`)

New mechanisms:

- **`mmu::unmap_page(va)`**: walk to the L3 entry, clear it, `tlbi vaae1` + barriers
  (the existing `map_page` sequence, pointed at invalidation). First unmapping in jos.
- **Frame bookkeeping**: `elf::load` records every `(va, frame)` it maps into a
  caller-provided `Vec` (kernel has `alloc`); `enter_user` adds the stack frame and
  stores the list in kernel-side state (`USER_FRAMES`).
- **`user::teardown()`**: unmap each recorded page, `free_frame` it, clear the recorded
  ranges in `LOADED` (so `is_user_range*` fails closed afterwards), and clear the
  jring pending table + waiter (`ring::abort_user()` — parked READs reference buffers
  that no longer exist; nobody will reap their CQEs, so the slots are simply dropped).
  Prints `[user] freed <N> frames` — the observable teardown marker.
- The **ring page stays mapped**: it is kernel-owned infrastructure (`setup` is
  idempotent, the poller references it); per-process rings arrive with per-process
  address spaces.

`SYS_EXIT` calls `teardown()` before `exit_current()` — exit and death share the
reclamation path. The dead task's 16 KiB kernel stack is still leaked (`spawn` never
reaps; noted out of scope, unchanged).

## Demo: the `crash` builtin (`user/src/main.rs`)

`crash` writes to the program's own code segment — the W^X demonstration cut from
Phase 10, finally deliverable:

```rust
b"crash" => unsafe { (0x2_0000_0000 as *mut u8).write_volatile(0) },
```

EL0 store to a `PAGE_USER_RX` page (`AP = 0b11`, read-only) → permission fault →
data abort from EL0 (EC `0x24`) → killed. `help` becomes `commands: help spam crash exit`.

## Demo / verification (`test.sh`)

Two boots:

1. **Normal session** (existing): `hellp\rhelp\rspam\rexit\r` — markers as today, plus
   `"[user] freed"` (teardown on exit) and the updated help line.
2. **Crash session** (new): `crash\r` — assert `"[user] killed"` *and* that the kernel
   survives it: the kill line must be followed by continued silence (no `*** EXCEPTION`
   panic output), checked by asserting the absence of `"*** EXCEPTION"`.

## Files

| File | Change |
|---|---|
| `kernel/src/exceptions.rs` | lower-EL non-SVC sync → kill path instead of halt. |
| `kernel/src/mmu.rs` | `unmap_page`. |
| `kernel/src/elf.rs` | `load` records `(va, frame)` pairs. |
| `kernel/src/user.rs` | `USER_FRAMES`, `teardown()`. |
| `kernel/src/ring.rs` | `abort_user()` (clear pending + waiter). |
| `kernel/src/syscall.rs` | `SYS_EXIT` arm calls `teardown()`. |
| `user/src/main.rs` | `crash` builtin; help text. |
| `test.sh` | second (crash) boot + markers. |
| `README.md` | layout-line updates. |

## Out of scope (named later phases)

Signals/fault delivery *to* the program (we kill, never notify) · reaping kernel stacks ·
freeing/recreating the ring page (per-process address spaces) · respawn/restart of the
program · exception fixup tables for kernel-side uaccess faults.
