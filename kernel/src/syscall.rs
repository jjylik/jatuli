//! Kernel syscall dispatch.
//!
//! ABI (Linux-like AArch64): syscall number in `x8`, arguments in `x0..x5`,
//! return value in `x0`. The vector stub has already saved everything into the
//! [`TrapFrame`]; we read the number and args from it and write the result back
//! to `x[0]`, which the stub restores before `ERET`.

use crate::exceptions::TrapFrame;
use crate::kprintln;

// The numbers (and their docs) live in the shared `abi` crate — the
// kernel/userspace contract; re-exported so kernel code keeps reading
// naturally as `syscall::SYS_*`.
pub use abi::{SYS_ADD, SYS_EXIT, SYS_RING_ENTER, SYS_RING_SETUP};

/// Dispatch the syscall described by `frame` (number in `x8`, args in `x0..`).
/// `_from_user` (true when the `SVC` came from EL0) is no longer needed here —
/// pointer validation is now per-process inside the ring layer.
pub fn dispatch(frame: &mut TrapFrame, _from_user: bool) {
    let ret = match frame.x[8] {
        SYS_ADD => frame.x[0].wrapping_add(frame.x[1]),
        SYS_RING_SETUP => crate::ring::setup(),
        SYS_RING_ENTER => crate::ring::enter(frame.x[0]),
        SYS_EXIT => {
            kprintln!("[user] exited with code {}", frame.x[0] as i64);
            // The process is done: reclaim its memory, retire its task, and
            // switch away for good. We never ERET back to EL0; the idle task
            // runs from here on.
            crate::user::teardown();
            crate::sched::exit_current()
        }
        other => {
            kprintln!("unknown syscall {}", other);
            u64::MAX
        }
    };
    frame.x[0] = ret;
}

