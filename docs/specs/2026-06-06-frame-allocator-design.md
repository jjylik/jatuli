# jos — Phase 3: Physical Frame Allocator (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Hand out and reclaim 4 KiB physical page frames of real RAM, ready for the
MMU phase to consume (page tables live in frames). Independent of the existing 1 MiB
static heap; rebasing the heap onto frames comes after the MMU.

## Decisions (from brainstorming)

- **Frame allocator only**; MMU is a separate later phase (page tables need frames anyway).
- **RAM hardcoded**: base `0x4000_0000`, size 128 MiB (QEMU `virt` default), top `0x4800_0000`.
  DTB parsing deferred to its own future mini-phase.
- **Frame size 4 KiB** (the standard AArch64 translation granule). Large-page benefits
  come later from 2 MiB/1 GiB *block* descriptors in the MMU phase, not from a bigger granule.
- **Intrusive free-list (stack)** data structure: real O(1) alloc/free, zero metadata array.

## Memory map it manages

```
0x4000_0000  RAM base    low region (QEMU/DTB scratch) — skipped
0x4008_0000  kernel image: code, rodata, data, .bss (incl. 1 MiB heap)
             …256 KiB stack…
_kernel_end  (new linker symbol, 4 KiB-aligned)
[ free frame pool ]   ← allocator owns exactly this
0x4800_0000  RAM top
```

Pool = `[_kernel_end, RAM_TOP)`. Starting above `_kernel_end` protects the kernel image and
the live stack. The DTB is not preserved because the kernel never reads it.

## Data structure

A single `head: usize` (`0` = empty; safe since RAM starts at `0x4000_0000`). Each free
frame stores the next free frame's address in its own first 8 bytes (valid pre-MMU because
physical RAM is flat/identity-accessible).

```
alloc: if head==0 -> None; f=head; head=*(f as *const usize); count-=1; Frame(f)
free:  *(f as *mut usize)=head; head=f; count+=1
```

Wrapped in the existing `sync::Locked` spinlock for `Sync`.

## Components / files

| File | Responsibility |
|---|---|
| `src/frames.rs` (new) | `Frame` newtype (4 KiB-aligned phys addr), `FrameAllocator`, the `Locked` global, `init_frames`/`alloc_frame`/`free_frame`/`free_frame_count`. `init_frames` walks the pool pushing every frame. |
| `linker.ld` (modified) | Add `_kernel_end = .;` (4 KiB-aligned) at the end; read from Rust via `extern "C" { static _kernel_end: u8; }`. |
| `src/main.rs` (modified) | `mod frames;`, call `frames::init_frames()`, run a frame self-check. |

## Constants

`RAM_BASE=0x4000_0000`, `RAM_SIZE=128 MiB`, `RAM_TOP=0x4800_0000`, `FRAME_SIZE=4096`.

## Error handling

`alloc_frame()` returns `None` when the pool is empty (no `GlobalAlloc` involvement).
Frames are handed out **un-zeroed**; callers that need zeroed memory clear them.

## Verification (smoke test, no formatting needed)

`kmain` self-check that `panic!`s (→ halt, no marker) on failure:
- `free_frame_count()` is large (> 1000)
- two `alloc_frame()` → distinct, 4 KiB-aligned addresses
- count drops by 2, then returns to initial after freeing both
- alloc again returns the just-freed frame (LIFO reuse proves `free` works)
- print `frame self-check passed`; `test.sh` greps for it.

## Out of scope

MMU/page tables/translation, zeroing-on-alloc, contiguous multi-frame allocation,
DTB parsing, formatted printing, rebasing the heap onto frames.
