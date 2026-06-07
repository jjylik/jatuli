//! Drop to EL0 by loading the embedded userspace ELF.
//!
//! Loads every `PT_LOAD` segment (see `elf.rs`), maps a user stack, and `ERET`s
//! to the program's entry point. After that the kernel is re-entered only via
//! syscalls or interrupts. User-supplied syscall pointers are validated against
//! the loaded segment ranges plus the stack.

use core::arch::asm;

use crate::elf;
use crate::frames::{alloc_frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;

/// Virtual base of the user stack (above the program's segments at 0x2_0000_0000).
pub const USER_STACK_VA: usize = 0x2_0010_0000;
/// User stack size (one page).
const USER_STACK_SIZE: usize = FRAME_SIZE;

/// The loaded image's mapped ranges, recorded for pointer validation.
static LOADED: Locked<Option<elf::Loaded>> = Locked::new(None);

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
/// analog of Linux's `copy_to_user` (`access_ok` + the copy). On hardened
/// real-world kernels this function is where the deliberate-access machinery
/// lives: ARM's PAN is toggled off (or unprivileged `sttr` stores are used)
/// only inside it, so any *other* kernel dereference of a user pointer faults
/// instead of becoming an exploit primitive. jos doesn't enable PAN, so the
/// gate is convention — but every kernel write into user memory goes through
/// here, which is the structure PAN would enforce.
pub fn copy_to_user(dst: usize, src: &[u8]) -> bool {
    if !is_user_range_writable(dst, src.len()) {
        return false;
    }
    let p = dst as *mut u8;
    for (i, &b) in src.iter().enumerate() {
        // SAFETY: just validated as writable user memory; same address space,
        // and PAGE_USER_RW is EL1-writable.
        unsafe { p.add(i).write_volatile(b) };
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
    let loaded = elf::load(elf::USER_ELF);
    let entry = loaded.entry;
    *LOADED.lock() = Some(loaded);

    // Map a user stack page as EL0 read/write.
    let stack = alloc_frame().expect("out of frames for the user stack");
    mmu::map_page(USER_STACK_VA, stack.addr(), PAGE_USER_RW);
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
