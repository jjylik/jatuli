# jos — Phase 19: User VAs in Their Own L0 Slot (Design)

**Date:** 2026-06-08
**Status:** Approved design
**Goal:** Relocate every EL0-accessible virtual address into its own top-level
page-table slot (L0[1]) so that kernel mappings and user mappings no longer share
a table. This is the prerequisite for per-process address spaces: once the boundary
is clean, a process's private mappings live entirely under L0[1] and can be swapped
wholesale on a TTBR0 switch while the kernel (L0[0]) stays mapped through every
syscall and IRQ.

## Why now

This is the first item of the process arc (toward ring-native `spawn`). The next
steps — per-process page tables, TTBR0-switched on context switch — only work
cleanly if user mappings never share a table with kernel mappings. Today they do.

## Current state (the problem)

With `T0SZ=16` (48-bit VA), each L0 entry covers 512 GiB. Everything currently
lives in **L0[0]**:

| Region | VA | L0 | L1 |
|---|---|----|----|
| Device MMIO | `0x0` | 0 | 0 |
| RAM (identity, kernel) | `0x4000_0000` | 0 | 1 |
| User segments | `0x2_0000_0000` (8 GiB) | **0** | 8 |
| User stack | `0x2_0010_0000` | **0** | 8 |
| Ring page | `0x2_0030_0000` | **0** | 8 |

`init_mmu` builds `L0[0] → L1` with `L1[0]` = device block and `L1[1]` = RAM table.
`map_page` then lazily hangs the user subtree off the *same* L1 table at `L1[8]`.
Kernel and user are intertwined one level below the root.

## Target state

`USER_BASE = 0x80_0000_0000` (512 GiB, **L0[1]**). The three EL0 regions rebase
there with offsets unchanged, so they land in L0[1]/L1[0]:

| Region | New VA | Expression |
|---|---|---|
| User segments | `0x80_0000_0000` | `USER_BASE` |
| User stack | `0x80_0010_0000` | `USER_BASE + 1 MiB` |
| Ring page | `0x80_0030_0000` | `USER_BASE + 3 MiB` |

The clean boundary becomes a *verifiable property*: `init_mmu` only ever touches
L0[0]; every EL0 mapping is created (lazily, by `map_page`) under L0[1]; the two
never share a table.

## Approach: lazy split, kernel-only `init_mmu`

`init_mmu` is unchanged — it builds the kernel subtree (L0[0]) and never touches
L0[1]. The user subtree under L0[1] is created lazily by `map_page` at user-load
time, exactly as it is created under L0[0]/L1[8] today; only the base VA moves.
`map_page(0x80_0000_0000)` computes `i0 = 1`, so `get_or_create(l0, 1)` allocates a
fresh L1 table under the (empty) L0[1] entry and the rest of the walk proceeds as
before.

Rejected alternatives:

- **Pre-build the user L1 root at boot.** Would make `init_mmu` know about
  userspace and allocate a user table before any program exists, for marginal
  benefit — the per-process step allocates per-process L0[1] subtrees anyway and
  would discard a boot-time global one.
- **TTBR1 split now** (kernel high, user low). Bigger than this step asks: rewrites
  the identity map, linker, and boot. The TODO specifies "own L0 slot" then
  "TTBR0-switched", i.e. stay single-TTBR0. Deferred.

## Single source of truth

Collapse the five scattered literals into one anchored definition.

In `abi/src/lib.rs` (the contract both crates compile against):

```rust
/// Base of the EL0 virtual-address window: L0 slot 1, kept separate from the
/// kernel's L0 slot 0 so per-process tables can split on this boundary.
pub const USER_BASE: usize = 0x80_0000_0000;
/// L0 index of the user window (= 1). The kernel asserts loaded segments land here.
pub const USER_L0_IDX: usize = USER_BASE >> 39;
/// Shared ring page, 3 MiB into the user window.
pub const USER_RING_VA: usize = USER_BASE + 0x30_0000;
```

In `kernel/src/user.rs` (kernel owns stack placement, anchored to the shared base):

```rust
pub const USER_STACK_VA: usize = abi::USER_BASE + 0x10_0000;
```

The `user/src/main.rs` `crash` builtin pokes `abi::USER_BASE` instead of the
`0x2_0000_0000` literal (it still targets the R-X `.text` base, so the store still
faults as intended).

**The one unavoidable duplicate:** `user/user.ld`'s base is a literal
(`. = 0x8000000000;`) — linker scripts cannot read Rust constants. It gets a comment
naming `abi::USER_BASE`, and the kernel's boot assertion (below) is the safety net
if the two ever drift.

## Enforced invariant

`elf_self_check` in `kernel/src/main.rs` changes from the weak lower-bound check:

```rust
assert!(entry >= 0x2_0000_0000, "user entry VA not in the user window");
```

to a slot check:

```rust
assert!((entry >> 39) == abi::USER_L0_IDX, "user entry VA not in the user L0 slot");
```

This fails the boot loudly in exactly the drift case (e.g. `user.ld` updated but
`abi` not, or vice versa). The existing ring-VA assert (`main.rs:88`,
`assert_eq!(va, abi::USER_RING_VA, …)`) already tracks the constant automatically.

## Ring-page hazard (documented, not fixed here)

The kernel dereferences the ring page at a *user* VA (`ring.rs:69`,
`&*(USER_RING_VA as *const RingPage)`). This is sound only while there is a single
global TTBR0: the ring is mapped in the one address space the kernel and user share.
Moving it into L0[1] keeps that true for this step.

It stops being true once TTBR0 is switched per-process: when process B is current,
an IRQ completing process A's parked `READ` cannot reach A's ring through A's user
VA. The fix — a kernel-side alias of the ring in L0[0], plus deciding how
per-process rings are created and mapped — belongs to the per-process step. A
comment at `ring.rs` records this so the next phase has the hazard in front of it.

## Sites touched

1. `abi/src/lib.rs` — add `USER_BASE`, `USER_L0_IDX`; redefine `USER_RING_VA`.
2. `kernel/src/user.rs` — `USER_STACK_VA` as `abi::USER_BASE + 0x10_0000`; update the
   line-17 doc comment ("above the program's segments at …").
3. `kernel/src/main.rs` — strengthen the `elf_self_check` assert to the slot check.
4. `user/user.ld` — base `. = 0x8000000000;` + coupling comment naming `abi::USER_BASE`.
5. `user/src/main.rs` — `crash` builtin pokes `abi::USER_BASE`.
6. `dump.sh` — `USER_STACK_VA=0x800100000` (currently `0x200100000`).

Plus the ring-hazard comment in `kernel/src/ring.rs`.

## Verification

A pure relocation: success = identical behavior at the new addresses.

- `./test.sh` — both golden runs must still pass. Run 1 (`hellp`, `help`, `spam`,
  `exit`) exercises segment mapping, the user stack, ring I/O (the SQPOLL `spam`
  demo), pointer validation, and clean teardown (`[user] freed`). Run 2 (`crash`)
  must still produce `[user] killed: data abort (lower EL)` then `[user] freed`,
  with no kernel panic (`*** EXCEPTION` rejected). This confirms segment mapping,
  stack, ring path, `copy_to_user` validation, and fault teardown all work at L0[1].
- Optional: `./dump.sh` — confirm the stack snapshot resolves at `0x800100000`.

## Out of scope

Per-process L0 tables, TTBR0 switching, the kernel-side ring alias, and a TTBR1
split. This step only relocates the user window and centralizes its definition.
