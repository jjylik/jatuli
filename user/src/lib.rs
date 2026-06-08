//! The jos userspace runtime ("libu"): the shared library every program in this
//! package links against.
//!
//! Holds the jring liburing (`uring`), the print/line helpers, the program-exit
//! syscall, and the single `#[panic_handler]` (a `no_std` lib may define it; each
//! linked binary picks it up). Each program is a binary under `src/bin/` that
//! provides its own `_start` and calls into here.

#![no_std]

pub mod uring;

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

// All "action" I/O flows through the jring (see uring.rs); the only plain syscall
// left is exit, which by nature cannot complete via the ring.
use abi::SYS_EXIT;

/// Maximum input line length for [`read_line`].
pub const MAX_LINE: usize = 128;

/// Monotonic completion tag (0 is reserved for "empty stash slot").
static NEXT_TAG: AtomicU64 = AtomicU64::new(1);

/// Next unique completion tag.
pub fn tag() -> u64 {
    NEXT_TAG.fetch_add(1, Ordering::Relaxed)
}

/// Print a string via the ring.
pub fn print(s: &str) {
    print_bytes(s.as_bytes());
}

/// Print bytes via the ring: one PRINT submission, one completion.
pub fn print_bytes(bytes: &[u8]) {
    let t = tag();
    uring::sqe(uring::OP_PRINT, bytes.as_ptr() as u64, bytes.len() as u64, t);
    uring::submit();
    uring::wait(t);
}

/// Read one line into `line`, echoing as we go (the terminal does not local-echo).
/// Returns the line length; handles Enter (`\r`/`\n`) and backspace.
pub fn read_line(line: &mut [u8; MAX_LINE]) -> usize {
    let mut len = 0;
    loop {
        let byte = read_byte();
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

/// Fetch one byte via an async READ: submit, then spin on the completion queue.
/// If no key is waiting, the kernel parks the op and the CQE is posted later from
/// the UART interrupt — zero syscalls on the wakeup path.
fn read_byte() -> u8 {
    let mut byte = 0u8;
    let t = tag();
    uring::sqe(uring::OP_READ, &mut byte as *mut u8 as u64, 1, t);
    uring::submit();
    uring::wait(t);
    byte
}

/// Terminate the program with `code` via `SYS_EXIT`; never returns. Always sound
/// to call — the kernel never returns control to EL0 afterward.
pub fn exit(code: i32) -> ! {
    // SAFETY: SYS_EXIT does not return; `noreturn` matches that.
    unsafe {
        asm!(
            "svc #0",
            in("x8") SYS_EXIT,
            in("x0") code as u64,
            options(nostack, noreturn),
        );
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit(1)
}
