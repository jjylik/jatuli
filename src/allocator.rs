//! The kernel's global heap allocator: a bump allocator over a fixed region.
//!
//! Each allocation rounds the `next` pointer up to the requested alignment and
//! advances it past the requested size. Memory is reclaimed only once every
//! allocation has been freed (the allocation count returns to zero), at which
//! point `next` resets to the start of the heap. This is the simplest allocator
//! that exercises the full `GlobalAlloc` path; a free-list comes in a later phase.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::sync::Locked;

/// Size of the kernel heap, in bytes (1 MiB).
const HEAP_SIZE: usize = 1024 * 1024;

/// Backing storage for the heap, reserved in `.bss`.
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// The kernel's global allocator.
#[global_allocator]
static ALLOCATOR: Locked<BumpAllocator> = Locked::new(BumpAllocator::new());

/// Initialize the kernel heap. Must be called exactly once, from `kmain`,
/// before the first allocation.
pub fn init_heap() {
    let start = ptr::addr_of_mut!(HEAP) as usize;
    // SAFETY: called once before any allocation; HEAP is a valid, writable
    // region of exactly HEAP_SIZE bytes that lives for the rest of the program.
    unsafe {
        ALLOCATOR.lock().init(start, HEAP_SIZE);
    }
}

/// Round `addr` up to the nearest multiple of `align` (a power of two).
fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// A bump allocator over a single contiguous heap region.
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

    /// Initialize the allocator with the bounds of its heap region.
    ///
    /// # Safety
    /// The caller must ensure `[start, start + size)` is a valid, unused, writable
    /// memory region that outlives every allocation made from it.
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
