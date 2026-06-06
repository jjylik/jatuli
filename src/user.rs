//! Minimal EL0 user mode.
//!
//! Maps a tiny user routine (the `.user_text` section) and a user stack into a
//! fresh EL0-accessible virtual range, then `ERET`s to EL0. After that the kernel
//! is re-entered only via syscalls or interrupts.

use core::arch::{asm, global_asm};

use crate::frames::{alloc_frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW, PAGE_USER_RX};

global_asm!(include_str!("user.s"));

extern "C" {
    /// Entry point of the EL0 routine (defined in `user.s`).
    static user_entry: u8;
    /// Bounds of the `.user_text` section (defined in `linker.ld`).
    static __user_start: u8;
    static __user_end: u8;
}

/// Virtual base where the user code is mapped (outside the identity and heap regions).
pub const USER_CODE_VA: usize = 0x2_0000_0000;
/// Virtual base of the user stack.
pub const USER_STACK_VA: usize = 0x2_0010_0000;
/// User stack size (one page).
const USER_STACK_SIZE: usize = FRAME_SIZE;

fn user_code_phys() -> usize {
    core::ptr::addr_of!(__user_start) as usize
}

fn user_code_size() -> usize {
    core::ptr::addr_of!(__user_end) as usize - user_code_phys()
}

/// Whether `[ptr, ptr + len)` lies entirely within the user-accessible region
/// (the mapped code page(s) or the user stack). Used to validate syscall pointers.
pub fn is_user_range(ptr: usize, len: usize) -> bool {
    let end = match ptr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    let in_code = ptr >= USER_CODE_VA && end <= USER_CODE_VA + user_code_size();
    let in_stack = ptr >= USER_STACK_VA && end <= USER_STACK_VA + USER_STACK_SIZE;
    in_code || in_stack
}

/// Map the user code + stack with EL0 permissions and drop to EL0 at `user_entry`.
/// Does not return: the kernel runs only via syscalls/IRQs after this.
pub fn enter_user() -> ! {
    let code_phys = user_code_phys();
    let code_size = user_code_size();

    // Map the user code (in place) as EL0 read/execute.
    let mut off = 0;
    while off < code_size {
        mmu::map_page(USER_CODE_VA + off, code_phys + off, PAGE_USER_RX);
        off += FRAME_SIZE;
    }

    // Map a user stack page as EL0 read/write.
    let stack = alloc_frame().expect("out of frames for the user stack");
    mmu::map_page(USER_STACK_VA, stack.addr(), PAGE_USER_RW);
    let user_sp = USER_STACK_VA + USER_STACK_SIZE;

    // Virtual address of the entry point within the mapped user code region.
    let entry = USER_CODE_VA + (core::ptr::addr_of!(user_entry) as usize - code_phys);

    // SAFETY: the code and stack are mapped EL0-accessible; SPSR selects EL0t with
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
