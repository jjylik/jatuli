# jos — Phase 22: Ring-Native `OP_SPAWN` / `OP_WAIT` (Design)

**Date:** 2026-06-08
**Status:** Implemented
**Goal:** Let a running program start and reap a child program entirely over the
ring — no `fork`, no `exec`, and (for spawn/wait) no syscall. A program submits an
`OP_SPAWN` naming a program and gets back a handle; an `OP_WAIT` on that handle
completes with the child's exit code. The kernel creates a *pristine* process from
scratch (fresh address space, loaded program, fresh stack); nothing is copied.

This is the modern "create a pristine process, then run it" model rather than
"copy the parent, then throw the copy away." jos has no `fork` to retire — spawn
*is* the primitive.

## Builds on Phases 20–21

- Per-process address spaces (Phase 20): a child gets its own `TTBR0`; the kernel
  reaches any process's ring at its identity-mapped physical address, which is how
  a child's exit posts a completion to the *parent's* ring while some other process
  is current.
- The program registry (Phase 21): `programs::get(name)` resolves a name to an
  embedded ELF — exactly what `OP_SPAWN` needs.

## Deliberately not done

No `fork`, no `exec`/`execve`, no `posix_spawn`-style builder. `OP_SPAWN` takes a
name and nothing else — no `argv`, `envp`, file-descriptor actions, or flags. jos
has no environment, no fd table, and no signals, so those Linux concepts have
nothing to map to; if `argv` ever earns its place it will be added then. The SQE
carries the parameters directly (no args struct).

`SYS_EXIT` stays a syscall: a process exiting cannot go through its own ring (a
ring op is asynchronous, and the process would keep running in EL0 awaiting a
completion that means "you are dead"). Dying needs a synchronous trap that never
returns. Spawn and wait — requests *about* other processes, serviceable
asynchronously — are ring-native; exit is the one principled `svc`.

## ABI additions (`abi`)

Two ring opcodes; no new syscalls:

```rust
/// Opcode: spawn a program by name (arg0 = name ptr, arg1 = name len). Completes
/// with the child's handle in `result`, or -1 (unknown program / bad pointer).
pub const OP_SPAWN: u64 = 3;
/// Opcode: wait for a child to exit (arg0 = its handle). Completes with the
/// child's exit code in `result`, or -1 (not the caller's child / bad handle).
pub const OP_WAIT: u64 = 4;
```

The handle is the child's pid. The process table never recycles slots (an exited
process leaves a husk), so a handle stays valid for the husk's life — `OP_WAIT`
works even long after the child exited.

## Process model (`process.rs`)

Three fields added to `Process`:

- `parent: Option<usize>` — set at spawn. `OP_WAIT` succeeds only for the caller's
  own children.
- `exit_code: Option<i64>` — recorded at teardown; readable from the husk forever.
- `exit_waiter: Option<u64>` — a parked wait's completion tag, set when a parent
  waits before the child has exited.

New helpers (each a brief `PROCESSES` lock): `set_parent`, `parent`, `exit_code`,
`register_wait(child, tag)`, and `on_exit(pid, code) -> Option<(parent, tag)>`
(records the exit code and returns a pending waiter to fire, if any).

## Kernel ring handling (`ring.rs`)

`process_sqe` gains two arms (the caller's address space is live during drain — the
`enter` path runs in its own syscall context, the SQPOLL path activates it):

- **`OP_SPAWN`** (caller = `pid`):
  1. Validate `[arg0, arg1)` against the caller's mapped memory; read the name
     (EL1 reads the caller's EL0-readable page directly, as `OP_PRINT` does).
  2. `programs::get(name)` → image, else complete `-1`.
  3. `let ttbr0 = mmu::new_address_space(); let child = process::create(image, ttbr0);`
     `process::set_parent(child, pid);`
  4. `debug_assert!` the child's `USER_BASE` translates to a different frame than
     the caller's (isolation, verified at the spawn event). `kprintln!("[spawn]
     {name} -> pid {child}")`.
  5. `sched::spawn_user(user_task, 0, child, ttbr0)` — a background task (the
     parent keeps the foreground).
  6. Complete with `child as i64`.
- **`OP_WAIT`** (caller = `pid`, `arg0` = handle):
  - Reject (`-1`) if the handle is out of range or not a child of `pid`.
  - If the child already exited (`exit_code` is `Some`) → complete now with the
    code.
  - Else `process::register_wait(handle, user_data)` and let the caller block in
    `enter`. No completion yet — it fires from the child's exit.

**Child exit** (`teardown(code)`): after recording the code via `process::on_exit`,
if a waiter is registered, post its CQE to the **parent's** ring (`ring_pa`,
reached by identity PA) and wake the parent's `ring_waiter`. The CQE is posted
regardless of whether the parent is currently blocked, so there is no lost wakeup —
the same discipline as parked reads. A *killed* child fires its waiter too, with a
fault sentinel code, so a parent never hangs on a child that crashed.

`teardown` gains a `code: i64` parameter: `SYS_EXIT` passes `x0`; `kill_user`
passes a sentinel (e.g. `-1`).

### Cost note

`OP_SPAWN` loads the child's ELF (hundreds of KiB copied into fresh frames) inside
`drain`'s IRQ-masked section — a one-time latency blip while spawning. Acceptable
for now; moving the heavy load out of the critical section (e.g. a loader helper
task) is later work, not part of this phase.

## Userspace (`user/src/lib.rs`, `user/src/bin/jsh.rs`)

- libu gains thin wrappers: `spawn(name: &str) -> i64` (`OP_SPAWN` + submit + wait)
  and `wait_child(handle: i64) -> i64` (`OP_WAIT` + submit + wait), plus a small
  `print_dec(i64)` for printing the exit code.
- jsh gains a `spawn` builtin: `let h = spawn("echo"); if h < 0 { print("spawn
  failed\n") } else { let code = wait_child(h); print("echo exited: ");
  print_dec(code); print("\n") }`.

## Boot

`kmain` creates **only** jsh (foreground). The Phase 20/21 boot-time `echo` and the
two-process isolation self-check are removed — there is no kernel-hardcoded second
process anymore; children are created on demand by `OP_SPAWN`, and isolation is
asserted inline at each spawn. `elf_self_check` still validates *both* embedded
program headers (jsh and echo) at boot.

## Testing

`./test.sh` run 1 types `help` / `spam` / `spawn` / `exit`. Asserts:

- `[spawn] echo -> pid` — the kernel created a child on a user request,
- `echo: hello from a second program` — the spawned child ran (its own ring,
  drained by SQPOLL),
- `echo exited: 0` — jsh reaped the child's exit code over the ring,
- clean exits and frees for both echo (on its own) and jsh (via `exit`).

Run 2 (`crash`) is unchanged. Together these prove a *user* program created another
and reaped its exit, entirely through shared-memory ring ops.

## Out of scope

`argv`/program arguments, spawning by arbitrary path, foreground hand-off to a
child, graceful out-of-frames handling (the kernel still panics on frame
exhaustion, as today), and optimizing the in-`drain` ELF load. Each is noted; none
blocks the milestone.
