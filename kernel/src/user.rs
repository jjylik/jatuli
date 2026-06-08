//! Drop to EL0 by loading the embedded userspace ELF.
//!
//! Loads every `PT_LOAD` segment (see `elf.rs`), maps a user stack, and `ERET`s
//! to the program's entry point. After that the kernel is re-entered only via
//! syscalls or interrupts. User-supplied syscall pointers are validated against
//! the loaded segment ranges plus the stack.

use core::arch::asm;

use alloc::vec::Vec;

use crate::elf;
use crate::frames::{alloc_frame, free_frame, Frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;

/// Virtual base of the user stack: 1 MiB into the user window, above the
/// program's segments (which start at `abi::USER_BASE`).
pub const USER_STACK_VA: usize = abi::USER_BASE + 0x10_0000;
/// User stack size (one page).
const USER_STACK_SIZE: usize = FRAME_SIZE;

/// The loaded image's mapped ranges, recorded for pointer validation.
static LOADED: Locked<Option<elf::Loaded>> = Locked::new(None);

/// Every `(va, frame)` the program owns (segments + stack), recorded at load
/// time so [`teardown`] can unmap and free them.
static USER_FRAMES: Locked<Vec<(usize, Frame)>> = Locked::new(Vec::new());

/// Whether `[ptr, ptr + len)` lies entirely within a mapped user segment or the
/// user stack. Used to validate syscall pointers from EL0.
pub fn is_user_range(ptr: usize, len: usize) -> bool {
    check_user_range(ptr, len, false)
}

/// Like [`is_user_range`], but additionally requires the range to be writable:
/// the stack, or a segment the loader mapped `PAGE_USER_RW`. The kernel must
/// check this before storing into user memory (e.g. `SYS_READ`) — a store into
/// the R-X code segment would permission-fault the kernel itself.
pub fn is_user_range_writable(ptr: usize, len: usize) -> bool {
    check_user_range(ptr, len, true)
}

/// Copy kernel bytes into user memory, validating first. Returns false (and
/// writes nothing) if the destination is not writable user memory.
///
/// This is the single, named gate for kernel→user data movement — the jos
/// analog of Linux's `copy_to_user` (`access_ok` + the copy). The copy uses
/// `STTRB`, AArch64's *unprivileged store*: executed at EL1 it performs the
/// MMU permission check with EL0 rules, exactly as Linux's
/// `__arch_copy_to_user` does. So even if validation were buggy, the hardware
/// re-checks every byte — a destination in kernel memory (EL0 no-access) or a
/// user R-X segment (EL0 read-only) faults loudly (same-EL data abort →
/// `report_and_halt`) instead of being silently corrupted. On PAN-enabled
/// kernels (ARMv8.1+; our A72 predates it) the same instructions are also the
/// only lawful channel to user memory, making this gate hardware-enforced in
/// both directions. (Linux additionally recovers from such faults via
/// exception fixup tables rather than halting — a possible later refinement.)
pub fn copy_to_user(dst: usize, src: &[u8]) -> bool {
    if !is_user_range_writable(dst, src.len()) {
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

fn check_user_range(ptr: usize, len: usize, need_write: bool) -> bool {
    let end = match ptr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    // The stack is always read/write.
    if ptr >= USER_STACK_VA && end <= USER_STACK_VA + USER_STACK_SIZE {
        return true;
    }
    let guard = LOADED.lock();
    if let Some(loaded) = guard.as_ref() {
        for r in &loaded.ranges[..loaded.count] {
            if ptr >= r.start && end <= r.end {
                return !need_write || r.writable;
            }
        }
    }
    false
}

/// Load the embedded user ELF, map a user stack, and drop to EL0 at its entry.
/// Does not return: the kernel runs only via syscalls/IRQs afterward.
pub fn enter_user() -> ! {
    let mut owned = Vec::new();
    let loaded = elf::load(elf::USER_ELF, &mut owned);
    let entry = loaded.entry;
    *LOADED.lock() = Some(loaded);

    // Map a user stack page as EL0 read/write.
    let stack = alloc_frame().expect("out of frames for the user stack");
    mmu::map_page(USER_STACK_VA, stack.addr(), PAGE_USER_RW);
    owned.push((USER_STACK_VA, stack));
    *USER_FRAMES.lock() = owned;
    let user_sp = USER_STACK_VA + USER_STACK_SIZE;

    // SAFETY: segments + stack are mapped EL0-accessible; SPSR selects EL0t with
    // interrupts enabled; ERET transfers to unprivileged execution at `entry`.
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

/// Reclaim the user program's memory: unmap every page it owned (segments +
/// stack) and return the frames to the pool. Pointer validation fails closed
/// afterwards (`LOADED` cleared), and parked ring reads are dropped — their
/// buffers no longer exist. Shared with exit and fault-kill; the ring page
/// itself stays (kernel-owned infrastructure, see `ring::setup`).
pub fn teardown() {
    crate::ring::abort_user();
    *LOADED.lock() = None;
    let owned = core::mem::take(&mut *USER_FRAMES.lock());
    let count = owned.len();
    for (va, frame) in owned {
        mmu::unmap_page(va);
        free_frame(frame);
    }
    crate::kprintln!("[user] freed {} frames", count);
}
