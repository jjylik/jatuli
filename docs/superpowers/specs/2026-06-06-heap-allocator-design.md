# jos — Phase 2: Kernel Heap Allocator (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Provide a `#[global_allocator]` so the `alloc` crate works in the kernel
(`Box`, `Vec`, `String`), backed by a hand-rolled bump allocator over a fixed heap.
No userspace, no MMU, no syscalls — pure in-kernel memory allocation.

## Why a bump allocator

It is the simplest allocator that exercises the full `GlobalAlloc` path (alignment,
out-of-memory, allocation tracking). Memory is reclaimed only when every allocation has
been freed. A real free-list allocator is deferred to a later phase; the `GlobalAlloc`
interface stays the same, so only the internals change later.

## Components

| File | Responsibility |
|---|---|
| `src/sync.rs` (new) | `Locked<A>`: a minimal `AtomicBool` spinlock. Needed because `GlobalAlloc` takes `&self`, so the allocator needs interior mutability + `Sync`. Single-core/no-interrupts means it never contends, but it is the smallest sound primitive. |
| `src/allocator.rs` (new) | `BumpAllocator`, its `GlobalAlloc` impl, `align_up`, the 1 MiB `HEAP` static (in `.bss`), the `#[global_allocator]` static, and `init_heap()`. |
| `src/mem.rs` (new) | `memcpy`/`memset`/`memmove`/`memcmp` — see "freestanding mem functions" below. |
| `src/main.rs` (modified) | `extern crate alloc;`, declare modules, call `init_heap()`, run heap self-check. |
| `linker.ld` | Unchanged — the heap is a `static` array, no linker work needed. |

## Bump allocator algorithm

```
alloc(layout):
    start = align_up(next, layout.align())
    end   = start + layout.size()   (checked_add; None -> null)
    if end > heap_end: return null        // OOM
    next = end; allocations += 1
    return start

dealloc(_):
    allocations -= 1
    if allocations == 0: next = heap_start   // reset only when fully drained
```

`align_up(addr, align) = (addr + align - 1) & !(align - 1)` (align is a power of two).

## Freestanding mem functions (the non-obvious part)

Using `Vec`/`String` makes the compiler emit calls to `memcpy`/`memset`/`memmove`/`memcmp`.
On `aarch64-unknown-none` (stable, no `libc`, no `compiler_builtins` `mem` feature) these
symbols are otherwise undefined, so we implement them ourselves — the same four functions
`background.md` shows and warns not to remove. They must use **manual byte loops**, not
`core::ptr::copy*` (which would lower back into `memcpy` and recurse). They are ordinary
EL1 functions, not syscalls.

## Error handling

`alloc` returns null on OOM → Rust's stable default `handle_alloc_error` → our
`#[panic_handler]` → CPU halts. Nothing extra to implement.

## Verification

`kmain` runs an in-kernel self-check that `panic!`s (→ halt, no marker) on failure:
- `Vec<u32>` push 1,2,3 → assert sum == 6
- heap-backed `String` "Hello from the heap!" → print via `uart::write_str`
- on success print `heap self-check passed`

`test.sh` greps for both `Hello, World!` and `heap self-check passed`.

## Out of scope

Free-list/reuse, MMU/virtual memory, frame allocator, formatted printing, userspace/syscalls.
