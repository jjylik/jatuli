# jos

A hobby AArch64 kernel for learning kernel basics, run under QEMU.

## Prerequisites

- Rust (`rustup target add aarch64-unknown-none`)
- QEMU (`qemu-system-aarch64`)

## Build

    cargo build

## Run

    cargo run

Boots under QEMU (`-machine virt,gic-version=3`), runs its self-checks, and drops
into `jsh`, a minimal userspace shell — type at the `jsh> ` prompt (`help`, `exit`).
Exit QEMU with `Ctrl-A` then `X`.

## Test

    ./test.sh

Boots the kernel under QEMU and checks the expected self-check output.

## Inspecting memory

    ./dump.sh

Boots the kernel, waits until the user program has exited (kernel parked in
`wfi`, MMU on, user mappings live), then snapshots memory via the QEMU monitor
into `dumps/`: all 128 MiB of physical RAM (`ram.bin`), each user `PT_LOAD`
segment and the user stack (read through the live page tables), plus the serial
transcript and a `MAP.txt` legend mapping file offsets to addresses.

Explore the dumps with any hex viewer (`xxd`, [ImHex](https://imhex.werwolv.net/),
Hex Fiend). For example, the kernel image starts at `ram.bin` offset `0x80000`
(physical `0x40080000`):

    xxd -s 0x80000 -l 64 dumps/ram.bin

For *live* exploration with symbols, use QEMU's GDB stub instead: add `-s` to
the QEMU command line, then attach with `lldb target/aarch64-unknown-none/debug/jos`
and `gdb-remote localhost:1234` (or `gdb` + `target remote :1234`) and read any
address with the kernel's symbol names.

## What it does

Brings itself up in stages, each with a self-check printed to the serial console:
boot → heap → physical frames → MMU → frame-backed heap → exception vectors →
trap frame + `SVC` syscalls → GICv3 + timer interrupts.

## Layout

- `kernel/` — the kernel crate (`jos`).
  - `kernel/src/boot.s` — `_start`: enables FP/SIMD, sets up the stack, branches to `kmain`.
  - `kernel/src/main.rs` — `kmain` entry point, self-checks, panic handler.
  - `kernel/src/uart.rs` — PL011 UART driver (TX + RX with on-demand RX interrupt) + `kprint!`/`kprintln!`.
  - `kernel/src/mem.rs` — freestanding `memcpy`/`memset`/`memmove`/`memcmp`.
  - `kernel/src/sync.rs` — `Locked<A>` spinlock.
  - `kernel/src/allocator.rs` — bump heap over a frame-backed virtual window.
  - `kernel/src/frames.rs` — physical 4 KiB frame allocator (intrusive free-list).
  - `kernel/src/mmu.rs` — page tables, MMU enable, `map_page`, cache maintenance.
  - `kernel/src/exceptions.rs` / `kernel/src/exceptions.s` — vector table, trap frame, dispatch.
  - `kernel/src/syscall.rs` — `SVC` syscall dispatch (Linux-like ABI): add, exit, ring setup/enter.
  - `kernel/src/gic.rs` — GICv3 interrupt controller (PPIs + distributor-routed SPIs).
  - `kernel/src/timer.rs` — generic timer (periodic interrupt).
  - `kernel/src/sched.rs` / `kernel/src/switch.s` — preemptive scheduler with block/wake; hosts the user task.
  - `kernel/src/ring.rs` — `jring`, an io_uring-lite: shared SQ/CQ rings, IRQ-completed reads, blocking `min_complete` waits.
  - `kernel/src/elf.rs` — minimal ELF64 loader for the embedded user image.
  - `kernel/src/user.rs` — load the user ELF, map a stack, drop to EL0; pointer validation.
  - `kernel/build.rs` — builds the `user` crate and embeds its ELF.
  - `kernel/linker.ld` — kernel image at `0x40080000`, stack, `_kernel_end`.
- `user/` — the EL0 userspace program crate (separately compiled).
  - `user/src/main.rs` — `jsh`: prompt, line editing, `help`/`exit` builtins (I/O via the ring).
  - `user/src/uring.rs` — userspace half of `jring`: setup/sqe/submit/wait.
  - `user/user.ld` — EL0 VA layout with separate R-X / R-W segments.

See `docs/superpowers/specs/` for per-phase design and `docs/superpowers/plans/` for plans.
