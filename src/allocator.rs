//! The kernel's global heap allocator: a bump allocator over a contiguous
//! virtual window backed by physical frames.
//!
//! The heap lives at a fixed virtual range (`HEAP_VBASE`) that `init_heap` maps
//! onto scattered 4 KiB frames from the frame allocator. The bump allocator only
//! ever sees the contiguous virtual window: each allocation rounds `next` up to
//! the requested alignment and advances past the size. Memory is reclaimed only
//! once every allocation has been freed; then `next` resets to the start. A
//! free-list comes in a later phase.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::frames::{alloc_frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_KERNEL_RW};
use crate::sync::Locked;

/// Base virtual address of the kernel heap window. Outside the 0–2 GiB identity
/// region, so heap addresses are genuinely virtual != physical.
pub const HEAP_VBASE: usize = 0x1_0000_0000;
/// Size of the kernel heap window in bytes (2 MiB = one full L3 table).
pub const HEAP_SIZE: usize = 2 * 1024 * 1024;

/// The kernel's global allocator.
#[global_allocator]
static ALLOCATOR: Locked<BumpAllocator> = Locked::new(BumpAllocator::new());

/// Initialize the kernel heap: map the heap window onto frames, then point the
/// bump allocator at it. Requires the frame allocator and MMU to be initialized;
/// call exactly once, before the first allocation.
pub fn init_heap() {
    let mut off = 0;
    while off < HEAP_SIZE {
        let frame = alloc_frame().expect("out of frames while mapping the heap");
        mmu::map_page(HEAP_VBASE + off, frame.addr(), PAGE_KERNEL_RW);
        off += FRAME_SIZE;
    }
    // SAFETY: [HEAP_VBASE, HEAP_VBASE + HEAP_SIZE) is now mapped and writable,
    // used only by the allocator; called once before any allocation.
    unsafe {
        ALLOCATOR.lock().init(HEAP_VBASE, HEAP_SIZE);
    }
}

/// Round `addr` up to the nearest multiple of `align` (a power of two).
fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// A bump allocator over a single contiguous (virtual) region.
pub struct BumpAllocator {
    heap_start: usize,
    heap_end: usize,
    next: usize,
    allocations: usize,
}

impl BumpAllocator {
    /// Create an uninitialized bump allocator. Call [`BumpAllocator::init`] before use.
    pub const fn new() -> Self {
        Self {
            heap_start: 0,
            heap_end: 0,
            next: 0,
            allocations: 0,
        }
    }

    /// Initialize the allocator with the bounds of its (mapped) heap region.
    ///
    /// # Safety
    /// `[start, start + size)` must be valid, mapped, writable memory that
    /// outlives every allocation made from it.
    pub unsafe fn init(&mut self, start: usize, size: usize) {
        self.heap_start = start;
        self.heap_end = start + size;
        self.next = start;
        self.allocations = 0;
    }
}

unsafe impl GlobalAlloc for Locked<BumpAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut bump = self.lock();

        let alloc_start = align_up(bump.next, layout.align());
        let alloc_end = match alloc_start.checked_add(layout.size()) {
            Some(end) => end,
            None => return ptr::null_mut(),
        };

        if alloc_end > bump.heap_end {
            ptr::null_mut() // out of memory
        } else {
            bump.next = alloc_end;
            bump.allocations += 1;
            alloc_start as *mut u8
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        let mut bump = self.lock();
        bump.allocations -= 1;
        if bump.allocations == 0 {
            bump.next = bump.heap_start;
        }
    }
}
