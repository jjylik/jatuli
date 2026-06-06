#![no_std]
#![no_main]

extern crate alloc;

use core::arch::global_asm;
use core::panic::PanicInfo;

mod allocator;
mod mem;
mod sync;
mod uart;

global_asm!(include_str!("boot.s"));

/// Kernel entry point. Called from `_start` (see `boot.s`) once the stack is set up.
#[no_mangle]
pub extern "C" fn kmain() -> ! {
    uart::write_str("Hello, World!\n");

    allocator::init_heap();
    heap_self_check();

    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

/// Exercise the global allocator. Panics (and so halts) if anything is wrong.
// Pushing one element at a time is intentional: it forces the Vec to reallocate
// as it grows, which exercises our freestanding `memcpy` (see `mem.rs`).
#[expect(
    clippy::vec_init_then_push,
    reason = "growth-by-push deliberately exercises Vec reallocation and memcpy"
)]
fn heap_self_check() {
    use alloc::string::String;
    use alloc::vec::Vec;

    let mut numbers: Vec<u32> = Vec::new();
    numbers.push(1);
    numbers.push(2);
    numbers.push(3);
    let sum: u32 = numbers.iter().sum();
    assert_eq!(sum, 6, "heap-backed Vec produced the wrong sum");

    let mut greeting = String::new();
    greeting.push_str("Hello from the heap!");
    uart::write_str(&greeting);
    uart::write_str("\n");

    uart::write_str("heap self-check passed\n");
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
