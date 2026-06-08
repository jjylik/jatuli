# jatuli — Phase 1: Hello World Boot (Design)

**Date:** 2026-06-06
**Status:** Approved design, pre-implementation
**Goal of this phase:** Boot a hand-rolled Rust kernel under QEMU on emulated ARM and print `Hello, World!` to the serial console. Nothing more.

## Why this scope

Learning kernel basics. No userspace. Start with the absolute minimum that proves the
whole toolchain → boot → run loop works end-to-end, with every byte hand-written so the
boot path is understood, not borrowed.

## Target platform (fixed for now)

| Choice | Value | Reason |
|---|---|---|
| Architecture | AArch64 (64-bit ARM) | Modern, clean Rust target, well-documented on QEMU |
| Machine | QEMU `virt` | Emulated, no real-hardware quirks while learning |
| CPU | `cortex-a72` | Common, fully supported by `virt` |
| Language | Rust (`#![no_std]`) + a little AArch64 asm | Cleanest bare-metal setup on macOS |
| Rust target | `aarch64-unknown-none` | Bare-metal, uses bundled `rust-lld` (no external linker needed) |
| Boot | QEMU `-kernel <elf>` | No bootloader/ISO/UEFI — QEMU loads the ELF and jumps to entry |

## How hello-world works here (4 moving parts)

1. **Linker script (`linker.ld`)** — place code at `0x4008_0000` (QEMU's AArch64 kernel
   load address; RAM base is `0x4000_0000`), lay out `.text/.rodata/.data/.bss`, and
   reserve a stack with a `_stack_top` symbol.
2. **Boot stub (`boot.s`, included via `global_asm!`)** — the first code that runs:
   set `sp = _stack_top`, branch to `kmain`. If it ever returns, park the CPU in a
   `wfe` loop. (~6 instructions.)
3. **UART driver (`uart.rs`)** — the PL011 UART is MMIO-mapped at `0x0900_0000` on `virt`.
   Output = volatile write of each byte to the data register (`UARTDR`, offset `0x00`),
   spinning on the TX-full flag (`UARTFR` bit 5, offset `0x18`) first. QEMU needs no
   UART init for output; we spin on the flag anyway as correct practice.
4. **Kernel entry + panic handler (`main.rs`)** — `#![no_std] #![no_main]`, a
   `#[no_mangle] extern "C" fn kmain() -> !` that prints the string then `wfe`-loops,
   and a `#[panic_handler]` that halts the CPU.

## File layout

```
jatuli/
├── Cargo.toml              # name=jatuli, panic="abort" (no unwinder)
├── .cargo/config.toml      # target, link-arg=-Tlinker.ld, qemu runner
├── linker.ld               # load 0x40080000, sections, _stack_top
├── src/
│   ├── boot.s              # _start: set sp, bl kmain, wfe-park
│   ├── main.rs             # no_std/no_main, global_asm!(boot.s), kmain, panic handler
│   └── uart.rs             # PL011 write_byte + write_str (volatile MMIO)
└── docs/superpowers/specs/ # this spec
```

## Key config

- **`Cargo.toml`**: `panic = "abort"` in both dev/release profiles (no stack unwinding
  in a freestanding binary).
- **`.cargo/config.toml`**:
  - `[build] target = "aarch64-unknown-none"`
  - `rustflags = ["-C", "link-arg=-Tlinker.ld"]`
  - `runner = "qemu-system-aarch64 -machine virt -cpu cortex-a72 -nographic -kernel"`
    so `cargo run` boots it.
- **Prereq**: `rustup target add aarch64-unknown-none`.

## Run / success criteria

```
cargo run
# or explicitly:
qemu-system-aarch64 -machine virt -cpu cortex-a72 -nographic \
  -kernel target/aarch64-unknown-none/debug/jatuli
```

**Success = `Hello, World!` appears in the terminal, then QEMU sits idle (CPU parked).**
Exit QEMU with `Ctrl-A` then `X`.

## Rust best-practices notes (no_std caveats)

The `rust-best-practices` skill assumes `std`; this is a freestanding kernel, so:

- **Applies directly:** raw-pointer/volatile MMIO care and `Send`/`Sync` reasoning
  (Ch.9); borrowing over cloning, iterators (Ch.1); `cargo clippy -- -D warnings`
  (Ch.2); doc style — `//` = why, `///` = what (Ch.8).
- **Does NOT apply as written:** `anyhow` needs `std` (unused); `thiserror` only
  recently supports `no_std` (skip for now). The "never `panic!`/`unwrap`" rule bends
  — a `#[panic_handler]` is required, and "panic = halt the CPU" is correct early-boot
  behavior. Phase-1 error handling is minimal (mostly infallible MMIO writes).

## Out of scope (explicitly NOT in phase 1)

No interrupts, no exception vectors, no MMU/paging, no heap/allocator, no multitasking,
no UART input, no multi-core (assume `-smp 1`; secondary-CPU parking deferred).

## Roadmap (non-binding, intentionally minimal)

1. **Phase 1 — Hello World boot** *(this spec)*: boot + serial print.
2. **Phase 2 — Real UART + input**: proper PL011 init, read bytes, tiny echo.
3. **Phase 3 — Exceptions & timer**: AArch64 exception vectors, generic timer IRQ,
   panic handler that prints.
4. **Phase 4 — Memory**: MMU/page tables, then a heap allocator.
5. **Phase 5 — Multitasking**: switch between a couple of kernel threads.

Each phase gets its own spec when we reach it.
