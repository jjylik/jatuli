# jos

A hobby AArch64 kernel for learning kernel basics, run under QEMU.

Monolithic and Linux-idiomatic on the inside, but **ring-native** rather than
Unix-like on the outside: it borrows Linux's internal mechanisms (the EL0/EL1
split, ELF loading, the AArch64 syscall convention, preemptive tasks,
`copy_to_user`, W^X) without Unix's external API — there are no files, no
signals, and no `fork`. All "action" I/O flows through an io_uring-style shared
ring (`jring`), and process creation is planned as a ring operation, not a
syscall. The closest relatives are research systems that kept Unix's structure
but replaced the syscall surface with shared-memory queues (FlexSC's
exception-less syscalls; dataplane OSes like Arrakis/IX) — post-Unix, not
pre-Unix.

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
trap frame + `SVC` syscalls → GICv3 + timer interrupts → preemptive scheduler →
EL0 userspace via an ELF loader → the `jring` io_uring-lite → `jsh`, a shell
whose every keystroke and print flows through the ring.

Then it drops into `jsh` (see *Run*). Console input is interrupt-driven into a
kernel buffer and delivered to userspace via `copy_to_user`; the shell blocks
in the kernel between keystrokes (no busy-spin), a kernel SQPOLL task can
consume submissions with no syscall, and a faulting program is killed and its
memory reclaimed while the kernel keeps running.

## Layout

- `abi/` — the kernel/userspace ABI contract: syscall numbers, jring page layout (`#[repr(C)]` + compile-time layout asserts), opcodes, flags.
- `kernel/` — the kernel crate (`jos`).
  - `kernel/src/boot.s` — `_start`: enables FP/SIMD, sets up the stack, branches to `kmain`.
  - `kernel/src/main.rs` — `kmain` entry point, self-checks, panic handler.
  - `kernel/src/uart.rs` — PL011 UART driver (TX + RX with on-demand RX interrupt) + `kprint!`/`kprintln!`.
  - `kernel/src/mem.rs` — freestanding `memcpy`/`memset`/`memmove`/`memcmp`.
  - `kernel/src/sync.rs` — `Locked<A>` spinlock.
  - `kernel/src/allocator.rs` — bump heap over a frame-backed virtual window.
  - `kernel/src/frames.rs` — physical 4 KiB frame allocator (intrusive free-list).
  - `kernel/src/mmu.rs` — page tables, MMU enable, `map_page`/`unmap_page`, cache maintenance.
  - `kernel/src/exceptions.rs` / `kernel/src/exceptions.s` — vector table, trap frame, dispatch; EL0 faults kill the program.
  - `kernel/src/syscall.rs` — `SVC` syscall dispatch (Linux-like ABI): add, exit, ring setup/enter.
  - `kernel/src/gic.rs` — GICv3 interrupt controller (PPIs + distributor-routed SPIs).
  - `kernel/src/timer.rs` — generic timer (periodic interrupt).
  - `kernel/src/sched.rs` / `kernel/src/switch.s` — preemptive scheduler with block/wake; hosts the user task.
  - `kernel/src/input.rs` — kernel console-input buffer (IRQ-drained byte ring, flow control).
  - `kernel/src/ring.rs` — `jring`, an io_uring-lite: shared SQ/CQ rings, IRQ-completed reads, blocking `min_complete` waits, SQPOLL task.
  - `kernel/src/elf.rs` — minimal ELF64 loader for the embedded user image.
  - `kernel/src/user.rs` — load the user ELF, map a stack, drop to EL0; pointer validation, `copy_to_user`.
  - `kernel/build.rs` — builds the `user` crate and embeds its ELF.
  - `kernel/linker.ld` — kernel image at `0x40080000`, stack, `_kernel_end`.
- `user/` — the EL0 userspace program crate (separately compiled).
  - `user/src/main.rs` — `jsh`: prompt, line editing, `help`/`spam`/`crash`/`exit` builtins (I/O via the ring).
  - `user/src/uring.rs` — userspace half of `jring`: setup/sqe/submit/wait.
  - `user/user.ld` — EL0 VA layout with separate R-X / R-W segments.

See `docs/superpowers/specs/` for per-phase design and `docs/superpowers/plans/` for plans.

## TODO

The process arc (toward ring-native `spawn`, no `fork`):
- Per-process address spaces: a `Process` owning its page tables, TTBR0-switched on context switch.
- Process table + per-process state (today's `LOADED`/`USER_FRAMES`/ring waiter are singletons).
- Multiple userspace programs.
- `OP_SPAWN` ring op returning a process handle; `OP_WAIT` completing on child exit.
- Decide I/O multiplexing

Independent polish:

- Tickless idle: program `CNTP_CVAL` one-shot for the next `wake_at` instead of ticking through idle.
- Wake-time preemption: need-resched on IRQ return (single-core analog of the reschedule IPI).
- Exception fixup tables so kernel-side `copy_to_user` faults recover instead of halting.
- Reap exited tasks' kernel stacks (`spawn` currently leaks them).
- ASIDs to avoid a full TLB flush on address-space switch.
