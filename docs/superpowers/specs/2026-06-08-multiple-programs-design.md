# jos — Phase 21: Multiple Userspace Programs in One Package (Design)

**Date:** 2026-06-08
**Status:** Approved design
**Goal:** Run *distinct* user programs, not two instances of one image. Keep a
single `user` Cargo package, but hold several programs inside it as separate
binaries (`jsh`, `echo`), each its own ELF, sharing a runtime library. The kernel
embeds them all and boots `jsh` (foreground) alongside `echo` (background).

This replaces the Phase 20 demo (two copies of the same image) with two genuinely
different programs, and lays the registry groundwork for a later `OP_SPAWN`
(spawn-by-name).

## Builds on Phase 20

Per-process address spaces are what make this simple. Every program links at the
**same** `USER_BASE` and uses the **same** `user.ld`, because each runs in its own
address space — identical VAs, different physical frames. No per-program linker
scripts are needed; the isolation self-check already proves two spaces map
`USER_BASE` to different frames.

## Userspace crate: shared lib + per-program binaries

`user/` restructures from one binary into a runtime library plus one binary per
program:

- **`user/src/lib.rs`** (new) — `#![no_std]`, the shared runtime ("libu"). Holds:
  - `pub mod uring;` (the liburing, moved unchanged from `src/uring.rs`),
  - the single `#[panic_handler]` (exactly one in any linked binary; the lib
    provides it, the bins do not),
  - the helpers currently in `main.rs`: `print`, `print_bytes`, `read_line`,
    `read_byte`, `tag`/`NEXT_TAG`, `sys_exit`, and `MAX_LINE`.
- **`user/src/uring.rs`** — unchanged, now a module of the lib.
- **`user/src/bin/jsh.rs`** (new) — the shell. Its own `_start`, plus the
  `dispatch` / `spam` / prompt loop moved from today's `main.rs`. Uses the lib
  (`use user::{uring, print, read_line, …}`) and `abi::USER_BASE` (the `crash`
  builtin).
- **`user/src/bin/echo.rs`** (new) — `_start` → `uring::setup()` →
  `print("echo: hello from a second program\n")` → `sys_exit(0)`. (A baked-in
  message stands in for an argument until args/`OP_SPAWN` exist; it cannot read,
  since the foreground process owns the keyboard.)
- **Delete** `user/src/main.rs`. `Cargo.toml` swaps `[[bin]] user` for `[lib]`;
  Cargo auto-discovers `src/bin/*.rs` as binaries. Bins reach the lib by the
  package name (`user::`) and keep access to `abi::` (a package dependency).

`user/user.ld` and `user/build.rs` are unchanged — the link args (`-T user.ld`,
4 KiB max-page-size) apply to every binary the package build produces, so all link
at `USER_BASE` with `ENTRY(_start)`.

Each bin is `#![no_std] #![no_main]` and defines its own `#[no_mangle] _start`.
Lang-item/panic settings are unchanged from today (the panic handler simply moves
from the old `main.rs` into the lib).

## Build: auto-discovery + codegen (`kernel/build.rs`)

`kernel/build.rs` already builds the `user` package into an isolated target dir.
The change: instead of grabbing one `user` ELF, discover and embed them all.

1. Build `-p user` as today (compiles every binary).
2. Scan `user/src/bin/*.rs`; each `<name>.rs` is a binary whose ELF is at
   `user-target/aarch64-unknown-none/<profile>/<name>`.
3. Generate `OUT_DIR/programs.rs`:
   ```rust
   pub static PROGRAMS: &[(&str, &[u8])] = &[
       ("echo", include_bytes!("…/user-target/aarch64-unknown-none/<profile>/echo")),
       ("jsh",  include_bytes!("…/user-target/aarch64-unknown-none/<profile>/jsh")),
   ];
   ```
4. Add `cargo:rerun-if-changed=user/src/bin` (plus the existing `user/src`,
   `abi/src`, `Cargo.toml`, `user.ld` triggers).

Adding a program is then just dropping a file in `user/src/bin/` — the program
list lives in exactly one place (the directory), discovered at build time.

## Kernel: program registry + bring-up

- **`kernel/src/programs.rs`** (new) — `include!(concat!(env!("OUT_DIR"),
  "/programs.rs"))` to pull in `PROGRAMS`, plus `pub fn get(name: &str) ->
  Option<&'static [u8]>` (linear scan; the table is tiny). This is the seam a
  future `OP_SPAWN` resolves a name against.
- **`elf.rs`** — `USER_ELF` (the single embedded image) is removed.
  `elf_self_check` iterates `PROGRAMS`, validating each header and that each
  entry virtual address lands in the user L0 slot (`abi::USER_L0_IDX`).
- **`main.rs`** bring-up: process 0 = `programs::get("jsh").unwrap()`
  (foreground), process 1 = `programs::get("echo").unwrap()` (background), each via
  `process::create(image, mmu::new_address_space())`. `process::create` already
  takes an image, so it just receives different bytes. The isolation self-check is
  unchanged and still holds (distinct programs → distinct frames at `USER_BASE`).

The kernel otherwise does not change: loading, the ring, scheduling, teardown all
operate per-process exactly as in Phase 20.

## Testing

`./test.sh`:

- Run 1 additionally asserts `echo: hello from a second program` (echo's distinct
  output) and that echo exits cleanly. Both echo and jsh now produce
  `[user] exited with code 0` and `[user] freed …` (echo on its own,  jsh via the
  `exit` builtin). jsh stays interactive: `help`, `spam`, `exit` unchanged.
- Run 2 (`crash`) is unchanged: the foreground jsh faults, is killed, its space is
  freed, and the kernel plus the other process survive.

Together these prove two *distinct* programs load from one package, run isolated
in their own address spaces and rings, and tear down independently.

## Out of scope

`OP_SPAWN` / `OP_WAIT` (user-initiated spawn-by-name — the registry is the
groundwork), program arguments, foreground switching, and any change to which two
programs the kernel boots (still hardcoded in `main.rs`). Distinct *behaviors*
beyond print/echo are left to those later phases.
