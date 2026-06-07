//! Kernel syscall dispatch.
//!
//! ABI (Linux-like AArch64): syscall number in `x8`, arguments in `x0..x5`,
//! return value in `x0`. The vector stub has already saved everything into the
//! [`TrapFrame`]; we read the number and args from it and write the result back
//! to `x[0]`, which the stub restores before `ERET`.

use crate::exceptions::TrapFrame;
use crate::kprintln;

/// Add two arguments: `x0 + x1 -> x0`. (Demo syscall.)
pub const SYS_ADD: u64 = 1;
/// Terminate the user program: `x0` = exit code. Does not return to EL0.
pub const SYS_EXIT: u64 = 3;
/// Map (idempotently) the shared jring page; returns its virtual address.
pub const SYS_RING_SETUP: u64 = 5;
/// Process all published jring submissions; returns 0.
pub const SYS_RING_ENTER: u64 = 6;
// (2 and 4 were SYS_PRINT and SYS_READ; all action I/O now flows through the
// jring, so the numbers are retired rather than reused.)

/// Dispatch the syscall described by `frame` (number in `x8`, args in `x0..`).
/// `from_user` is true when the `SVC` came from EL0, which gates pointer validation.
pub fn dispatch(frame: &mut TrapFrame, from_user: bool) {
    let ret = match frame.x[8] {
        SYS_ADD => frame.x[0].wrapping_add(frame.x[1]),
        SYS_RING_SETUP => crate::ring::setup(),
        SYS_RING_ENTER => crate::ring::enter(from_user),
        SYS_EXIT => {
            kprintln!("[user] exited with code {}", frame.x[0] as i64);
            // The process is done: retire its task and switch away for good.
            // We never ERET back to EL0; the idle task runs from here on.
            // (Freeing the user's frames is a later phase.)
            crate::sched::exit_current()
        }
        other => {
            kprintln!("unknown syscall {}", other);
            u64::MAX
        }
    };
    frame.x[0] = ret;
}

