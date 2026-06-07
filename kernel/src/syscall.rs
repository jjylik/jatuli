//! Kernel syscall dispatch.
//!
//! ABI (Linux-like AArch64): syscall number in `x8`, arguments in `x0..x5`,
//! return value in `x0`. The vector stub has already saved everything into the
//! [`TrapFrame`]; we read the number and args from it and write the result back
//! to `x[0]`, which the stub restores before `ERET`.

use crate::exceptions::TrapFrame;
use crate::kprintln;
use crate::uart;

/// Add two arguments: `x0 + x1 -> x0`. (Demo syscall.)
pub const SYS_ADD: u64 = 1;
/// Print a string: `x0 = ptr`, `x1 = len`, returns 0. (Demo syscall.)
pub const SYS_PRINT: u64 = 2;
/// Terminate the user program: `x0` = exit code. Does not return to EL0.
pub const SYS_EXIT: u64 = 3;
/// Read from the console: `x0` = buf, `x1` = len. Blocks until at least one
/// byte is available; returns the number of bytes read.
pub const SYS_READ: u64 = 4;
/// Map (idempotently) the shared jring page; returns its virtual address.
pub const SYS_RING_SETUP: u64 = 5;
/// Process all published jring submissions; returns 0.
pub const SYS_RING_ENTER: u64 = 6;

/// Dispatch the syscall described by `frame` (number in `x8`, args in `x0..`).
/// `from_user` is true when the `SVC` came from EL0, which gates pointer validation.
pub fn dispatch(frame: &mut TrapFrame, from_user: bool) {
    let ret = match frame.x[8] {
        SYS_ADD => frame.x[0].wrapping_add(frame.x[1]),
        SYS_PRINT => sys_print(frame.x[0], frame.x[1], from_user),
        SYS_READ => sys_read(frame.x[0], frame.x[1], from_user),
        SYS_RING_SETUP => crate::ring::setup(),
        SYS_RING_ENTER => crate::ring::enter(from_user),
        SYS_EXIT => {
            kprintln!("[user] exited with code {}", frame.x[0] as i64);
            // The process is done; control stays at EL1. Park the CPU here —
            // we never ERET back to EL0. (Full teardown is a later phase.)
            loop {
                // SAFETY: idle until an interrupt; nothing left to run.
                unsafe { core::arch::asm!("wfi", options(nomem, nostack, preserves_flags)) };
            }
        }
        other => {
            kprintln!("unknown syscall {}", other);
            u64::MAX
        }
    };
    frame.x[0] = ret;
}

/// Read console bytes into `[buf, buf+len)`. Blocks (polling the UART) until at
/// least one byte arrives, then greedily drains whatever else is immediately
/// available. Buffers from user mode must lie in *writable* user memory — the
/// kernel is about to store through this pointer, and a store into the R-X code
/// segment would permission-fault at EL1 too.
///
/// No kernel buffer is involved: the data path is UART data register → CPU
/// register → user memory (same address space, `PAGE_USER_RW` is EL1-writable).
fn sys_read(buf: u64, len: u64, from_user: bool) -> u64 {
    if from_user && !crate::user::is_user_range_writable(buf as usize, len as usize) {
        kprintln!("rejected non-writable user buffer {:#x}", buf);
        return u64::MAX;
    }
    if len == 0 {
        return 0;
    }
    let dst = buf as *mut u8;
    let mut count: usize = 0;
    while count < len as usize {
        match uart::try_getc() {
            // SAFETY: kernel-trusted, or validated writable user memory.
            Some(b) => unsafe {
                dst.add(count).write_volatile(b);
                count += 1;
            },
            // Nothing waiting: block for the first byte, return once we have
            // some. A plain spin, not `wfi`: IRQs are masked while we handle
            // the syscall, so the pending timer interrupt would make `wfi`
            // return immediately anyway.
            None if count == 0 => core::hint::spin_loop(),
            None => break,
        }
    }
    count as u64
}

/// Print a UTF-8 string given a pointer and length. Pointers from user mode are
/// validated to lie within the user's address range before being dereferenced;
/// kernel callers are trusted.
fn sys_print(ptr: u64, len: u64, from_user: bool) -> u64 {
    if from_user && !crate::user::is_user_range(ptr as usize, len as usize) {
        kprintln!("rejected out-of-range user pointer {:#x}", ptr);
        return u64::MAX;
    }
    // SAFETY: kernel-trusted, or validated to lie within mapped user memory.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    if let Ok(s) = core::str::from_utf8(bytes) {
        uart::write_str(s);
    }
    0
}
