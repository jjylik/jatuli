#![no_std]
#![no_main]

use core::arch::asm;

// Syscall numbers — must match the kernel's `syscall.rs` ABI.
const SYS_PRINT: u64 = 2;
const SYS_EXIT: u64 = 3;
const SYS_READ: u64 = 4;

const MAX_LINE: usize = 128;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    print("jsh: type 'help'\n");
    let mut line = [0u8; MAX_LINE];
    loop {
        print("jsh> ");
        let len = read_line(&mut line);
        dispatch(&line[..len]);
    }
}

/// Read one line, echoing as we go (the terminal does not local-echo).
/// Returns the line length; handles Enter (`\r`/`\n`) and backspace.
fn read_line(line: &mut [u8; MAX_LINE]) -> usize {
    let mut len = 0;
    loop {
        let mut byte = 0u8;
        // SAFETY: `byte` is a writable stack byte; SYS_READ blocks for >= 1.
        unsafe { sys_read(&mut byte, 1) };
        match byte {
            b'\r' | b'\n' => {
                print("\n");
                return len;
            }
            0x7f | 0x08 => {
                if len > 0 {
                    len -= 1;
                    print("\x08 \x08"); // rub out the last glyph
                }
            }
            0x20..=0x7e => {
                if len < MAX_LINE {
                    line[len] = byte;
                    len += 1;
                    print_bytes(core::slice::from_ref(&byte));
                }
            }
            _ => {} // ignore other control bytes
        }
    }
}

/// Run one entered line.
fn dispatch(line: &[u8]) {
    match line {
        b"" => {}
        b"exit" => {
            // SAFETY: SYS_EXIT never returns.
            unsafe { sys_exit(0) }
        }
        b"help" => print("commands: help exit\n"),
        other => {
            print("unknown command: ");
            print_bytes(other);
            print("\n");
        }
    }
}

fn print(s: &str) {
    print_bytes(s.as_bytes());
}

fn print_bytes(bytes: &[u8]) {
    // SAFETY: `bytes` is a live slice in our mapped memory; SYS_PRINT reads (ptr, len).
    unsafe { sys_print(bytes.as_ptr(), bytes.len()) };
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

/// SYS_READ(buf, len): block until console input; returns bytes read.
///
/// # Safety
/// `buf` must point to `len` writable bytes in this program's memory.
unsafe fn sys_read(buf: *mut u8, len: usize) -> usize {
    let ret: u64;
    asm!(
        "svc #0",
        in("x8") SYS_READ,
        inout("x0") buf as u64 => ret,
        in("x1") len as u64,
        options(nostack),
    );
    ret as usize
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
