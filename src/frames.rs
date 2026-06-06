//! Physical frame allocator: hands out and reclaims 4 KiB physical page frames.
//!
//! Free frames are tracked with an intrusive singly-linked stack: each free
//! frame stores, in its own first 8 bytes, the physical address of the next
//! free frame. `head` is the first free frame (`0` means empty, which is safe
//! because usable RAM starts at `0x4000_0000`). `alloc` pops the head; `free`
//! pushes. Both are O(1) and need no separate metadata array.

use crate::sync::Locked;

/// Size of a physical frame (the 4 KiB AArch64 translation granule).
pub const FRAME_SIZE: usize = 4096;

/// Base of usable RAM on the QEMU `virt` machine.
const RAM_BASE: usize = 0x4000_0000;
/// Size of RAM (hardcoded to the QEMU `virt` default of 128 MiB).
const RAM_SIZE: usize = 128 * 1024 * 1024;
/// One past the last usable RAM address.
const RAM_TOP: usize = RAM_BASE + RAM_SIZE;

extern "C" {
    /// End of the kernel image, defined by `linker.ld`. Frames are handed out
    /// only from above this address, so the kernel and its stack are never
    /// overwritten.
    static _kernel_end: u8;
}

/// A 4 KiB-aligned physical frame, identified by its base address. A newtype so
/// physical frames can't be confused with plain integers or (later) virtual
/// addresses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame(usize);

impl Frame {
    /// The physical base address of this frame.
    pub fn addr(self) -> usize {
        self.0
    }
}

/// The kernel's global physical frame allocator.
static FRAME_ALLOCATOR: Locked<FrameAllocator> = Locked::new(FrameAllocator::new());

/// Round `addr` up to the next multiple of [`FRAME_SIZE`].
fn frame_align_up(addr: usize) -> usize {
    (addr + FRAME_SIZE - 1) & !(FRAME_SIZE - 1)
}

/// An intrusive free-list ("stack") of physical frames.
pub struct FrameAllocator {
    /// Physical address of the first free frame, or `0` when the pool is empty.
    head: usize,
    /// Number of frames currently free.
    free_count: usize,
}

impl FrameAllocator {
    const fn new() -> Self {
        Self {
            head: 0,
            free_count: 0,
        }
    }

    /// Push every whole 4 KiB frame in `[start, end)` onto the free list.
    ///
    /// # Safety
    /// `[start, end)` must be valid, writable RAM not used for anything else.
    unsafe fn add_region(&mut self, start: usize, end: usize) {
        let mut frame = frame_align_up(start);
        while frame + FRAME_SIZE <= end {
            self.push(frame);
            frame += FRAME_SIZE;
        }
    }

    /// Push a frame onto the free list, storing the old head inside it.
    ///
    /// # Safety
    /// `frame` must be a valid, writable 4 KiB frame.
    unsafe fn push(&mut self, frame: usize) {
        *(frame as *mut usize) = self.head;
        self.head = frame;
        self.free_count += 1;
    }

    /// Pop a frame from the free list, reading the next head out of it.
    fn pop(&mut self) -> Option<Frame> {
        if self.head == 0 {
            return None;
        }
        let frame = self.head;
        // SAFETY: `frame` is on the free list, so its first 8 bytes hold the
        // address of the next free frame (or 0).
        self.head = unsafe { *(frame as *const usize) };
        self.free_count -= 1;
        Some(Frame(frame))
    }
}

/// Initialize the frame allocator. Call once from `kmain`, before any frame use.
pub fn init_frames() {
    // Only computes the address of a linker-provided symbol; no read occurs.
    let kernel_end = core::ptr::addr_of!(_kernel_end) as usize;
    let start = frame_align_up(kernel_end);
    // SAFETY: `[start, RAM_TOP)` is RAM above the kernel image and stack, so it
    // is ours to manage; called once before any allocation.
    unsafe {
        FRAME_ALLOCATOR.lock().add_region(start, RAM_TOP);
    }
}

/// Allocate one physical frame, or `None` if the pool is empty.
pub fn alloc_frame() -> Option<Frame> {
    FRAME_ALLOCATOR.lock().pop()
}

/// Return a previously allocated frame to the pool.
pub fn free_frame(frame: Frame) {
    // SAFETY: `frame` was produced by `alloc_frame`, so it is a valid writable frame.
    unsafe {
        FRAME_ALLOCATOR.lock().push(frame.addr());
    }
}

/// Number of frames currently free.
pub fn free_frame_count() -> usize {
    FRAME_ALLOCATOR.lock().free_count
}
