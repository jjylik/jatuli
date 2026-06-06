# jos

A hobby AArch64 kernel for learning kernel basics, run under QEMU.

## Prerequisites

- Rust (`rustup target add aarch64-unknown-none`)
- QEMU (`qemu-system-aarch64`)

## Build

    cargo build

## Run

    cargo run

Prints `Hello, World!` to the serial console, then parks the CPU.
Exit QEMU with `Ctrl-A` then `X`.

## Test

    ./test.sh

Boots the kernel under QEMU and checks that it prints `Hello, World!`.

## Layout

- `src/boot.s` — `_start`: enables FP/SIMD, sets up the stack, branches to `kmain`.
- `src/main.rs` — `kmain` entry point and panic handler.
- `src/uart.rs` — minimal PL011 UART output driver.
- `linker.ld` — places the image at `0x40080000` and reserves the stack.

See `docs/superpowers/specs/` for design and `docs/superpowers/plans/` for the plan.
