# jatuli

A hobby AArch64 kernel for learning kernel basics, run under QEMU.

Monolithic and Linux-idiomatic on the inside, but **ring-native** rather than
Unix-like on the outside: it borrows Linux's internal mechanisms (the EL0/EL1
split, ELF loading, the AArch64 syscall convention, preemptive tasks,
`copy_to_user`, W^X) without Unix's external API. All "action" I/O flows through an io_uring-style shared
ring (`jring`), and process creation is planned as a ring operation, not a
syscall. Inspired by SerenityOS and FlexSC.

## Prerequisites

- Rust (`rustup target add aarch64-unknown-none`)
- QEMU (`qemu-system-aarch64`)

## Build

    cargo build

## Run

    cargo run

Boots under QEMU (`-machine virt,gic-version=3`), runs its self-checks, and drops
into `jsh`, a minimal userspace shell — type at the `jsh> ` prompt (`help`, `exit`).

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
the QEMU command line, then attach with `lldb target/aarch64-unknown-none/debug/jatuli`
and `gdb-remote localhost:1234` (or `gdb` + `target remote :1234`) and read any
address with the kernel's symbol names.

## Layout

- `abi/` — the kernel/userspace ABI contract: syscall numbers, jring page layout (`#[repr(C)]` + compile-time layout asserts), opcodes, flags.
- `kernel/` — the kernel crate (`jatuli`).
- `user/` — the EL0 userspace crate (separately compiled): a runtime lib plus one binary per program.
 
See `docs/specs/` for per-phase design documents.

## Quiz

Check out the quiz.md in the docs folder for a quiz over what this kernel actually does. Easy way to check your understanding.

## TODO

- Program arguments for `OP_SPAWN` (today it takes a name only).
- Finish I/O multiplexing: foreground owns the keyboard today; decide fair
  keystroke routing and whose SQ the poller prioritizes.
- Graceful spawn failure on frame exhaustion (kernel currently panics).
- Tickless idle: program `CNTP_CVAL` one-shot for the next `wake_at` instead of ticking through idle.
- Wake-time preemption: need-resched on IRQ return (single-core analog of the reschedule IPI).
- Exception fixup tables so kernel-side `copy_to_user` faults recover instead of halting.
- Reap exited tasks' kernel stacks (`spawn` currently leaks them).
- ASIDs to avoid a full TLB flush on address-space switch.