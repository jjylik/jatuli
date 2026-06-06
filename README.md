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

- `kernel/` — the kernel crate (`jos`).
  - `kernel/src/boot.s` — `_start`: enables FP/SIMD, sets up the stack, branches to `kmain`.
  - `kernel/src/main.rs` — `kmain` entry point, self-checks, panic handler.
  - `kernel/src/uart.rs` — PL011 UART driver + `core::fmt::Write` (`kprint!`/`kprintln!`).
  - `kernel/src/mem.rs` — freestanding `memcpy`/`memset`/`memmove`/`memcmp`.
  - `kernel/src/sync.rs` — `Locked<A>` spinlock.
  - `kernel/src/allocator.rs` — bump heap over a frame-backed virtual window.
  - `kernel/src/frames.rs` — physical 4 KiB frame allocator (intrusive free-list).
  - `kernel/src/mmu.rs` — page tables, MMU enable, `map_page`, cache maintenance.
  - `kernel/src/exceptions.rs` / `kernel/src/exceptions.s` — vector table, trap frame, dispatch.
  - `kernel/src/syscall.rs` — `SVC` syscall dispatch (Linux-like ABI), incl. `SYS_EXIT`.
  - `kernel/src/gic.rs` — GICv3 interrupt controller.
  - `kernel/src/timer.rs` — generic timer (periodic interrupt).
  - `kernel/src/sched.rs` / `kernel/src/switch.s` — cooperative + preemptive scheduler.
  - `kernel/src/elf.rs` — minimal ELF64 loader for the embedded user image.
  - `kernel/src/user.rs` — load the user ELF, map a stack, drop to EL0; pointer validation.
  - `kernel/build.rs` — builds the `user` crate and embeds its ELF.
  - `kernel/linker.ld` — kernel image at `0x40080000`, stack, `_kernel_end`.
- `user/` — the EL0 userspace program crate (separately compiled).
  - `user/src/main.rs` — `_start`, `svc` syscall stubs, hello + exit.
  - `user/user.ld` — EL0 VA layout with separate R-X / R-W segments.

See `docs/superpowers/specs/` for per-phase design and `docs/superpowers/plans/` for plans.
