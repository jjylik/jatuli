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

/// Dispatch the syscall described by `frame` (number in `x8`, args in `x0..`).
/// `from_user` is true when the `SVC` came from EL0, which gates pointer validation.
pub fn dispatch(frame: &mut TrapFrame, from_user: bool) {
    let ret = match frame.x[8] {
        SYS_ADD => frame.x[0].wrapping_add(frame.x[1]),
        SYS_PRINT => sys_print(frame.x[0], frame.x[1], from_user),
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
