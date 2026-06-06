# jos

A hobby AArch64 kernel for learning kernel basics, run under QEMU.

## Prerequisites

- Rust (`rustup target add aarch64-unknown-none`)
- QEMU (`qemu-system-aarch64`)

## Build

    cargo build

## Run

    cargo run

Boots under QEMU (`-machine virt,gic-version=3`), runs its self-checks, and idles.
Exit QEMU with `Ctrl-A` then `X`.

## Test

    ./test.sh

Boots the kernel under QEMU and checks the expected self-check output.

## What it does

Brings itself up in stages, each with a self-check printed to the serial console:
boot → heap → physical frames → MMU → frame-backed heap → exception vectors →
trap frame + `SVC` syscalls → GICv3 + timer interrupts.

## Layout

- `src/boot.s` — `_start`: enables FP/SIMD, sets up the stack, branches to `kmain`.
- `src/main.rs` — `kmain` entry point, self-checks, panic handler.
- `src/uart.rs` — PL011 UART driver + `core::fmt::Write` (`kprint!`/`kprintln!`).
- `src/mem.rs` — freestanding `memcpy`/`memset`/`memmove`/`memcmp`.
- `src/sync.rs` — `Locked<A>` spinlock.
- `src/allocator.rs` — bump heap over a frame-backed virtual window.
- `src/frames.rs` — physical 4 KiB frame allocator (intrusive free-list).
- `src/mmu.rs` — page tables, MMU enable, `map_page`.
- `src/exceptions.rs` / `src/exceptions.s` — vector table, trap frame, dispatch.
- `src/syscall.rs` — `SVC` syscall dispatch (Linux-like ABI).
- `src/gic.rs` — GICv3 interrupt controller.
- `src/timer.rs` — generic timer (periodic interrupt).
- `linker.ld` — image at `0x40080000`, stack, `_kernel_end`.

See `docs/superpowers/specs/` for per-phase design and `docs/superpowers/plans/` for plans.
