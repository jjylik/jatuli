//! Drop to EL0 and run a loaded user process.
//!
//! Process creation (building the address space, loading the ELF, mapping a
//! stack) lives in `process.rs`; this module ERETs into the *current* process
//! and houses the kernel↔user data gate (`copy_to_user`) and syscall-pointer
//! validation, both of which resolve against the current process.

use core::arch::asm;

use crate::process;
use crate::sched;

/// Copy kernel bytes into process `pid`'s user memory, validating first. Returns
/// false (and writes nothing) if the destination is not writable user memory.
///
/// This is the single, named gate for kernel→user data movement — the jos analog
/// of Linux's `copy_to_user` (`access_ok` + the copy). The copy uses `STTRB`,
/// AArch64's *unprivileged store*: executed at EL1 it performs the MMU permission
/// check with EL0 rules, exactly as Linux's `__arch_copy_to_user` does. So even
/// if validation were buggy, the hardware re-checks every byte — a destination in
/// kernel memory (EL0 no-access) or a user R-X segment (EL0 read-only) faults
/// loudly (same-EL data abort → `report_and_halt`) instead of being silently
/// corrupted. (Linux additionally recovers from such faults via exception fixup
/// tables rather than halting — a possible later refinement.)
///
/// `STTRB` resolves the destination in the live `TTBR0`, so the caller must
/// ensure `pid`'s address space is installed (it is, in its syscall context, or
/// after the poller/`poll_pending` made it live).
pub fn copy_to_user(pid: usize, dst: usize, src: &[u8]) -> bool {
    if !process::is_user_range(pid, dst, src.len(), true) {
        return false;
    }
    for (i, &b) in src.iter().enumerate() {
        // SAFETY: validated above, and STTRB stores with EL0 permissions —
        // it can only ever succeed on EL0-writable memory.
        unsafe {
            asm!(
                "sttrb {b:w}, [{p}]",
                b = in(reg) b,
                p = in(reg) dst + i,
                options(nostack, preserves_flags),
            );
        }
    }
    true
}

/// ERET into the current process at its entry point. The scheduler has already
/// installed the process's address space, and `process::create` mapped its
/// segments and stack. Does not return: the kernel runs only via syscalls/IRQs
/// afterward, all on this task's kernel stack.
pub fn enter_user() -> ! {
    let pid = sched::current_process().expect("enter_user without a process");
    let entry = process::entry(pid);
    let user_sp = process::USER_STACK_VA + process::USER_STACK_SIZE;

    // SAFETY: segments + stack are mapped EL0-accessible in the live address
    // space; SPSR selects EL0t with interrupts enabled; ERET transfers to
    // unprivileged execution at `entry`.
    unsafe {
        asm!(
            "msr spsr_el1, {spsr}",
            "msr elr_el1, {entry}",
            "msr sp_el0, {sp}",
            "eret",
            spsr = in(reg) 0u64,
            entry = in(reg) entry,
            sp = in(reg) user_sp,
            options(noreturn),
        );
    }
}

/// Reclaim the current process's memory: unmap every page it owned (segments +
/// stack + ring) and return the frames to the pool. Parked ring reads are
/// dropped — their buffers no longer exist. Shared with exit and fault-kill.
pub fn teardown(code: i64) {
    let pid = sched::current_process().expect("teardown without a process");
    crate::ring::abort_user(pid);

    // Record our exit and wake any parent parked in OP_WAIT on us (the completion
    // posts to the parent's ring via its identity PA, so it works even though the
    // parent is not the current process).
    crate::ring::notify_exit(pid, code);

    let dead_ttbr0 = process::ttbr0(pid);

    // Unmap and free the program's frames from its own (currently live) space.
    let owned = process::take_frames(pid);
    let count = owned.len();
    for (va, frame) in owned {
        crate::mmu::unmap_page(va);
        crate::frames::free_frame(frame);
    }

    // Switch onto the kernel space before reclaiming the dying space's page
    // tables — we must not free the L0 while it is the live `TTBR0`.
    crate::mmu::activate(crate::mmu::kernel_ttbr0());
    // SAFETY: `dead_ttbr0` is this process's space, no longer live and never used
    // again (the task is about to exit; the husk's `ring_pa` is cleared below).
    unsafe { crate::mmu::free_address_space(dead_ttbr0) };
    process::mark_exited(pid);

    crate::kprintln!("[user] freed {} frames", count);
}
