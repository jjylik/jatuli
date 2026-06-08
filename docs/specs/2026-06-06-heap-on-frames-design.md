# jos — Phase 5: Rebase the Heap onto Frames (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Replace the heap's `static [u8; 1MiB]` backing with physical frames mapped into
a contiguous virtual window, retiring the static array. Adds a reusable `map_page`
primitive. The bump allocator algorithm is unchanged — only its backing store moves.

## Decisions (from brainstorming)

- **Virtual window + 4 KiB paging**: add `map_page` (creates L2/L3 tables on demand),
  reserve a virtual window, map scattered frames into it. Virtually contiguous,
  physically scattered.
- **Map all up front at init**: fixed 2 MiB window (= 512 pages = one L3 table).
- Heap window at **`0x1_0000_0000`** (4 GiB), outside the 0–2 GiB identity region, so heap
  addresses are genuinely virtual != physical and can't collide with the identity blocks.

## New MMU primitive: `map_page(va, pa, flags)`

Walks the live tables (via `TTBR0_EL1`), creating intermediate tables as needed, and
installs an L3 page descriptor.

```
map_page(va, pa, flags):
    l0 = TTBR0_EL1 & ADDR_MASK
    l1 = get_or_create(l0, idx0(va))
    l2 = get_or_create(l1, idx1(va))
    l3 = get_or_create(l2, idx2(va))
    l3[idx3(va)] = (pa & ADDR_MASK) | flags
    dsb ishst ; tlbi vaae1, va>>12 ; dsb ish ; isb
```

- Indices: `idx0=(va>>39)&0x1FF`, `idx1=(va>>30)&0x1FF`, `idx2=(va>>21)&0x1FF`,
  `idx3=(va>>12)&0x1FF`.
- `ADDR_MASK = 0x0000_FFFF_FFFF_F000` (descriptor output bits [47:12]).
- `get_or_create(table, i)`: if `table[i]` valid, follow it (assert it's a table, not a
  block); else allocate a zeroed table frame and install `frame | DESC_TABLE (0x3)`.
- **`PAGE_KERNEL_RW`** (L3 page, kernel data) = `DESC_PAGE(0b11) | SH_INNER | AF | UXN | PXN`
  = `0x703 | (3<<53)`: Normal cacheable, EL1 read/write, non-executable.

## Heap window

- `HEAP_VBASE = 0x1_0000_0000`, `HEAP_SIZE = 2 MiB`.
- Walk: `L0[0] → L1[4] → (new L2)[0] → (new L3)[0..512]`. Reuses the existing L1 from
  Phase 4 (entry 4 was invalid); `map_page` allocates one L2 + one L3 + 512 data frames.

## Changes by file

| File | Change |
|---|---|
| `src/mmu.rs` | Add `ADDR_MASK`, `DESC_VALID`, `DESC_PAGE`, `PAGE_KERNEL_RW`, `get_or_create`, `map_page`. `init_mmu` unchanged. |
| `src/allocator.rs` | Delete `static mut HEAP`. `init_heap()` maps the window onto frames via `map_page`, then `init(HEAP_VBASE, HEAP_SIZE)`. Expose `HEAP_VBASE`/`HEAP_SIZE`. `BumpAllocator` untouched. |
| `src/main.rs` | Reorder `kmain`: frames → MMU → heap (heap now depends on both). Drop `Box` from `mmu_self_check`. `heap_self_check` adds a `Box` whose address must fall in `[HEAP_VBASE, HEAP_VBASE+HEAP_SIZE)`. |
| `test.sh` | Unchanged (greps by presence; line order differs). |

## Verification

`heap_self_check` (`Vec`/`String`/`Box`) now runs against virtual addresses ~`0x1_0000_0000`,
translated through the new L3 mappings to scattered frames. Surviving plus the
address-range assertion proves the full chain. A bug hangs silently (no vector table) →
debug with `qemu ... -d mmu,int`.

## Out of scope

Lazy/demand growth, freeing heap pages, unmapping, higher-half, per-allocation
permissions, userspace, exception vectors.
