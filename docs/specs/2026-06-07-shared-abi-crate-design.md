# jatuli — Phase 17: Shared ABI Crate + Typed Ring Page (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** A third workspace crate, `abi/`, holding the single source of truth for
everything kernel and userspace must agree on: syscall numbers, the jring shared-page
layout as a `#[repr(C)]` struct, opcodes, and flags. The ring page stops being offset
arithmetic + casts on both sides and becomes typed field access through one audited
VA→reference conversion per side. No behavior change — `test.sh` green throughout.

## Why

- `uring.rs` hand-mirrors every constant and offset in `ring.rs`; agreement is by
  vigilance. The uapi-header pattern (one definition, both sides) eliminates the class.
- Offset arithmetic (`0x280 + (tail & MASK) * 16`) becomes compiler-checked fields,
  with `offset_of!`/`size_of` const asserts pinning the layout — drift is a build
  error, not a protocol corruption.
- Correctness upgrade: entry fields become relaxed atomics (the Linux
  `READ_ONCE`/`WRITE_ONCE` discipline) instead of `volatile`, which is the wrong tool
  for memory concurrently mutated by another agent. Same codegen (plain `ldr`/`str`).
- House rule, applied from here on: structures may keep short field names, but every
  acronym used by a structure is expanded in its doc comments.

## The `abi` crate

`#![no_std]` library; both `kernel` and `user` depend on it by path. Contents:

- Syscall numbers (`SYS_ADD`, `SYS_EXIT`, `SYS_RING_SETUP`, `SYS_RING_ENTER`) with the
  calling convention documented once.
- `USER_RING_VA`, `RING_ENTRIES`, `RING_MASK`, `NEED_WAKEUP`, `OP_NOP/PRINT/READ`.
- `Sqe` (32 B), `Cqe` (16 B), `RingPage` (header at fixed offsets, `sq` at `0x40`,
  `cq` at `0x280`) — all fields atomics, `#[repr(C)]`, layout pinned by const asserts.

Each side keeps exactly one cast: `&*(va as *const RingPage)` — the audited line where
a hardware-decided address becomes a type. Soundness gate unchanged: only after
`setup()` mapped the page (`poll_pending` gains an explicit early-return for clarity).

## Rewiring

- `kernel/src/ring.rs`: drop `index()`/`sqe_field()`/offset constants; all access via
  `page()`. `kernel/src/syscall.rs` re-exports the numbers from `abi`.
- `kernel/src/main.rs`: `ring_self_check` builds its batch through the `abi` struct
  (mimicking userspace) instead of raw offset pokes.
- `user/src/uring.rs`: same treatment; opcodes re-exported from `abi`.
- `kernel/build.rs`: `rerun-if-changed` for `../abi` (the embedded user ELF depends on it).

## Files

| File | Change |
|---|---|
| `abi/Cargo.toml`, `abi/src/lib.rs` (new) | the contract crate. |
| `Cargo.toml` (root) | workspace member `abi`. |
| `kernel/Cargo.toml`, `user/Cargo.toml` | path dependency on `abi`. |
| `kernel/build.rs` | rerun-if-changed `../abi`. |
| `kernel/src/ring.rs`, `kernel/src/syscall.rs`, `kernel/src/main.rs` | typed access, re-exports. |
| `user/src/uring.rs`, `user/src/main.rs` | typed access, re-exports. |
| `README.md` | layout. |

## Out of scope

New ops or behavior · strict-provenance sweep of the other modules (mmu/uart/gic) ·
versioning the ABI (single in-tree consumer).
