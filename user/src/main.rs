#![no_std]
#![no_main]

mod uring;

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

// All "action" I/O flows through the jring (see uring.rs); the only plain
// syscall left for us is exit, which by nature cannot complete via the ring.
use abi::SYS_EXIT;

const MAX_LINE: usize = 128;

/// Monotonic completion tag (0 is reserved for "empty stash slot").
static NEXT_TAG: AtomicU64 = AtomicU64::new(1);

fn tag() -> u64 {
    NEXT_TAG.fetch_add(1, Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    uring::setup();

    // Banner as a batch: two ops published, ONE syscall, tag-matched reaping.
    let banner = "jsh: type 'help'\n";
    let (t1, t2) = (tag(), tag());
    uring::sqe(uring::OP_NOP, 0, 0, t1);
    uring::sqe(uring::OP_PRINT, banner.as_ptr() as u64, banner.len() as u64, t2);
    uring::submit();
    uring::wait(t1);
    uring::wait(t2);

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

/// Fetch one byte via an async READ: submit, then spin on the completion
/// queue. If no key is waiting, the kernel parks the op and the CQE is posted
/// later from the timer interrupt — zero syscalls on the wakeup path.
fn read_byte() -> u8 {
    let mut byte = 0u8;
    let t = tag();
    uring::sqe(uring::OP_READ, &mut byte as *mut u8 as u64, 1, t);
    uring::submit();
    uring::wait(t);
    byte
}

/// Run one entered line.
fn dispatch(line: &[u8]) {
    match line {
        b"" => {}
        b"exit" => {
            // SAFETY: SYS_EXIT never returns.
            unsafe { sys_exit(0) }
        }
        b"help" => print("commands: help spam crash exit\n"),
        b"spam" => spam(),
        b"crash" => {
            // Write to our own R-X code segment: the MMU's W^X enforcement
            // turns this into a data abort, and the kernel kills us.
            // SAFETY: deliberately not safe — that's the demo.
            unsafe { (0x2_0000_0000 as *mut u8).write_volatile(0) }
        }
        other => {
            print("unknown command: ");
            print_bytes(other);
            print("\n");
        }
    }
}

/// SQPOLL demo: publish three prints with flag-aware submits (zero syscalls
/// while the kernel's SQ poller is awake), then spin on the CQ — deliberately
/// never sleeping in the kernel, so the poller is the only thing that can
/// consume the submissions. Pure shared-memory I/O.
fn spam() {
    let lines = [b"spam 1\n", b"spam 2\n", b"spam 3\n"];
    let mut tags = [0u64; 3];
    for (i, line) in lines.iter().enumerate() {
        tags[i] = tag();
        uring::sqe(uring::OP_PRINT, line.as_ptr() as u64, line.len() as u64, tags[i]);
        uring::submit(); // traps only if the poller raised NEED_WAKEUP
    }
    for t in tags {
        uring::wait_spin(t);
    }
}

fn print(s: &str) {
    print_bytes(s.as_bytes());
}

/// Print via the ring: one PRINT submission, one completion.
fn print_bytes(bytes: &[u8]) {
    let t = tag();
    uring::sqe(uring::OP_PRINT, bytes.as_ptr() as u64, bytes.len() as u64, t);
    uring::submit();
    uring::wait(t);
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
