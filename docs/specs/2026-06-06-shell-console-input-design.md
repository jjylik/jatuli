# jos — Phase 11: Console Input + `jsh` Userspace Shell (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Give the kernel console input (PL011 receive + a blocking `SYS_READ` that writes
into validated user memory) and turn the userspace program into `jsh`, an ultra-simple
shell: prompt, echo, line editing, and two builtins (`help`, `exit`). First phase where
data flows kernel→user and where write permission (not just "is it user memory") matters.

## Reuses

- The `user` crate, ELF loader, and loaded-range table (Phase 10). `SYS_READ` validation
  extends the same range table `SYS_PRINT` already checks.
- `SYS_EXIT` (Phase 10) — `exit` ends the shell cleanly, keeping the existing test marker.
- PL011 driver (Phase 1), gaining a receive path.

## Kernel: UART receive (`uart.rs`)

Non-blocking read: if `UARTFR` (offset `0x18`) bit 4 (`RXFE`, RX FIFO empty) is clear,
return the low byte of `UARTDR` (offset `0x00`).

```rust
pub fn try_getc() -> Option<u8>
```

The PL011 stays in reset (1-byte FIFO) mode; QEMU's chardev layer applies backpressure,
so pasted or piped input queues host-side rather than being dropped.

## Kernel: writable-range tracking (`elf.rs`, `user.rs`)

`elf::Range` gains `writable: bool`, set from the segment's `PF_W` flag while loading.
`user.rs` adds:

```rust
pub fn is_user_range_writable(ptr: usize, len: usize) -> bool
```

True if the range lies within the user stack page, or within a loaded segment whose
`writable` is set. The read-side `is_user_range` is unchanged. Rationale: the kernel is
about to *write* into user memory; aiming `SYS_READ` at the R-X code segment must be
rejected up front (a plain store there would permission-fault the kernel — `PAGE_USER_RX`
is read-only at EL1 too).

## Kernel: `SYS_READ` (`syscall.rs`)

```rust
pub const SYS_READ: u64 = 4;   // x0 = buf, x1 = len -> bytes read (u64::MAX on bad buf)
```

- Validate `[buf, buf+len)` with `is_user_range_writable` when `from_user`.
- `len == 0` → return 0.
- Block until input: poll `uart::try_getc()` with `core::hint::spin_loop()`. (Not `wfi`:
  IRQs are masked during the syscall, so the pending timer interrupt would make `wfi`
  return immediately anyway — an honest spin is clearer.)
- After the first byte, greedily drain whatever is immediately available, up to `len`.
- Return the count (≥ 1). Bytes are stored directly through the user VA (same address
  space; `PAGE_USER_RW` is EL1-writable). No kernel-side buffer exists in this design —
  the data path is device register → CPU register → user buffer.

## User: the shell (`user/src/main.rs`)

The hello-world program becomes `jsh`. All state on the stack (the ELF keeps its single
R-X segment; the stack is deliberately the only valid `SYS_READ` target).

- Banner: `jsh: type 'help'`, then loop:
- Prompt `jsh> `, read one byte at a time (`sys_read(&mut byte, 1)`), echo as we go
  (QEMU's terminal does not local-echo):
  - `\r` / `\n` → echo `\r\n`, dispatch the line
  - backspace `0x7f`/`0x08` → if the line is non-empty, drop one char and echo `\x08 \x08`
  - printable `0x20..=0x7e` → append to a 128-byte line buffer (ignore when full), echo
  - anything else → ignore
- Dispatch: `exit` → `sys_exit(0)`; `help` → `commands: help exit`; empty line → new
  prompt; otherwise `unknown command: <line>`.
- New stub: `sys_read(ptr: *mut u8, len: usize) -> usize` (`svc #0`, number 4).

## Demo / verification

`test.sh` pipes a scripted session into QEMU's stdin (the shell accepts `\r`, which is
what a real terminal sends for Enter):

```bash
printf 'hellp\rhelp\rexit\r' | qemu-system-aarch64 ... >"$OUT" 2>&1 &
```

Markers: `"Hello, world from EL0!"` is replaced by `"jsh: type 'help'"`; add
`"unknown command: hellp"` and `"commands: help exit"`; keep
`"[user] exited with code 0"`. Interactive use is just `cargo run` — type at the prompt.

## Files

| File | Change |
|---|---|
| `kernel/src/uart.rs` | `try_getc()` (PL011 RX). |
| `kernel/src/elf.rs` | `Range.writable` from `PF_W`. |
| `kernel/src/user.rs` | `is_user_range_writable`. |
| `kernel/src/syscall.rs` | `SYS_READ` + blocking poll + copy-to-user store. |
| `user/src/main.rs` | becomes `jsh`: read loop, line editing, builtins. |
| `test.sh` | scripted stdin session + new markers. |
| `README.md` | user crate description, interactive usage note. |

## Out of scope (named later phases)

IRQ-driven RX ring buffer (real `copy_to_user` from a kernel buffer) · shell as a
scheduler task · `echo`/argument parsing · spawning programs · Ctrl-C / job control ·
PAN-style hardened user-memory access.
