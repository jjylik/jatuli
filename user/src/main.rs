#![no_std]
#![no_main]

use core::arch::asm;

// Syscall numbers — must match the kernel's `syscall.rs` ABI.
const SYS_PRINT: u64 = 2;
const SYS_EXIT: u64 = 3;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello, world from EL0!\n";
    // SAFETY: `msg` is a live byte slice in our mapped .rodata; SYS_PRINT reads (ptr, len).
    unsafe {
        sys_print(msg.as_ptr(), msg.len());
        sys_exit(0);
    }
}

/// SYS_PRINT(ptr, len): ask the kernel to print a UTF-8 string.
///
/// # Safety
/// `ptr`/`len` must describe a readable byte range in this program's memory.
unsafe fn sys_print(ptr: *const u8, len: usize) {
    asm!(
        "svc #0",
        in("x8") SYS_PRINT,
        in("x0") ptr as u64,
        in("x1") len as u64,
        options(nostack),
    );
}

/// SYS_EXIT(code): terminate the program; never returns.
///
/// # Safety
/// Always sound to call; the kernel never returns control to EL0 afterward.
unsafe fn sys_exit(code: i32) -> ! {
    asm!(
        "svc #0",
        in("x8") SYS_EXIT,
        in("x0") code as u64,
        options(nostack, noreturn),
    );
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // SAFETY: SYS_EXIT never returns.
    unsafe { sys_exit(1) }
}
