# jatuli — Phase 6: Exception Vectors (Report-and-Halt) (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Install the `VBAR_EL1` exception vector table and a handler that decodes and
prints any exception (`ESR`/`ELR`/`FAR`/`SPSR` + cause) then halts. Adds a minimal
`core::fmt::Write` for the UART. After this, no fault is ever silent again.

## Decisions (from brainstorming)

- **Report-and-halt**: read the syndrome registers, print a diagnostic, park the CPU. No
  general-register saving (the handler never returns). Full trap-frame save/restore + ERET
  comes with the syscall phase.
- **Install early** — first thing in `kmain`, so any later init fault is reported.
- Folds in **`core::fmt::Write`** for the UART (needed to print hex). IRQ/FIQ/timer/GIC and
  EL0 are out of scope.

## Background

`VBAR_EL1` points at a 2 KiB-aligned table of 16 entries, 0x80 bytes apart, in four
groups: current-EL/SP0, current-EL/SPx, lower-EL/AArch64, lower-EL/AArch32. We run at EL1
with `SPSel=1` (SP_EL1), so our own faults land in the **current-EL/SPx** group (vectors
4–7; synchronous = vector 4). On entry, hardware sets `ELR_EL1` (return addr), `SPSR_EL1`
(saved state), `ESR_EL1` (syndrome; `EC = ESR[31:26]`), and `FAR_EL1` (fault address).

## Components / files

| File | Change |
|---|---|
| `src/uart.rs` | Add a `Uart` unit struct impl'ing `core::fmt::Write` (delegates to `write_str`), a `_print(args)` helper, and `kprint!`/`kprintln!` macros (`format_args!`, no heap). `write_str` stays. |
| `src/exceptions.s` (new) | Vector table: `.balign 0x800` base, 16 entries `.balign 0x80` apart, each `mov x0, #<index>; b common_exception`; `common_exception: b exception_dispatch`. |
| `src/exceptions.rs` (new) | `global_asm!(include_str!("exceptions.s"))`, `init_exceptions()` (set `VBAR_EL1` from the table symbol via `addr_of!`), `exception_dispatch(kind) -> !`, `ec_name`/`vector_name`. |
| `src/main.rs` | `mod exceptions;`, call `init_exceptions()` first in `kmain`, add `exception_self_check()` last. |
| `test.sh` | Grep for the exception diagnostic. |

## The handler

`exception_dispatch(kind: u64) -> !` reads `ESR/ELR/FAR/SPSR` via `mrs` (first thing, before
any printing, so they aren't clobbered), computes `EC = (esr >> 26) & 0x3F`, prints:
```
*** EXCEPTION (vector 4: current EL, SPx) ***
  ESR_EL1  = 0x0000000096000004  (EC = 0x25: data abort (same EL))
  ELR_EL1  = 0x....
  FAR_EL1  = 0x00000000dead0000
  SPSR_EL1 = 0x....
halting.
```
then `wfe`-loops. `ec_name` decodes common classes: `0x00` unknown, `0x07` SIMD/FP trap,
`0x15` SVC, `0x20/0x21` instruction abort, `0x24/0x25` data abort, `0x3C` BRK. No GPRs are
saved (never returns). `VBAR_EL1` is loaded from `extern "C" { static exception_vector_table: u8; }`
+ `addr_of!` (avoids the `fn`-to-int cast).

## Verification

`exception_self_check()` prints a banner, then **deliberately reads the unmapped address
`0xDEAD_0000`** (invalid `L1[3]`) → data abort → handler prints the diagnostic (with
`FAR=0x...dead0000`, EC `0x25`, vector 4) and halts. `test.sh` greps `exception vectors
installed`, `EXCEPTION (vector 4`, and `data abort (same EL)`.

Note: the deliberate fault becomes the end of boot (the kernel halts in the handler instead
of the idle loop). It's a temporary self-test, removed when the syscall phase gives the
kernel real work.

## Out of scope

Full trap-frame save/restore + ERET return, IRQ/FIQ/timer/GIC, EL0/userspace, recovering
from faults.
