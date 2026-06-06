//! AArch64 exception vectors: install `VBAR_EL1` and report any exception.
//!
//! This phase reports and halts: the handler decodes ESR/ELR/FAR/SPSR, prints a
//! diagnostic, and parks the CPU. No general registers are saved because the
//! handler never returns. Full trap-frame save/restore (for syscalls and IRQs)
//! comes in a later phase.

use core::arch::{asm, global_asm};

use crate::kprintln;

global_asm!(include_str!("exceptions.s"));

extern "C" {
    /// The exception vector table, defined in `exceptions.s`.
    static exception_vector_table: u8;
}

/// Install the exception vector table. Call once, early in `kmain`, so any later
/// fault produces a diagnostic instead of a silent hang.
pub fn init_exceptions() {
    let vbar = core::ptr::addr_of!(exception_vector_table) as u64;
    // SAFETY: `vbar` is the address of the correctly 2 KiB-aligned vector table.
    unsafe {
        asm!("msr vbar_el1, {0}", "isb", in(reg) vbar, options(nostack, preserves_flags));
    }
}

/// Human-readable name for an exception class (`ESR_EL1.EC`).
fn ec_name(ec: u64) -> &'static str {
    match ec {
        0x00 => "unknown",
        0x07 => "SIMD/FP access trapped",
        0x15 => "SVC (AArch64 syscall)",
        0x20 => "instruction abort (lower EL)",
        0x21 => "instruction abort (same EL)",
        0x24 => "data abort (lower EL)",
        0x25 => "data abort (same EL)",
        0x3C => "BRK (AArch64 breakpoint)",
        _ => "other",
    }
}

/// Which of the 16 vector-table groups an entry index belongs to.
fn vector_name(kind: u64) -> &'static str {
    match kind {
        0..=3 => "current EL, SP0",
        4..=7 => "current EL, SPx",
        8..=11 => "lower EL, AArch64",
        _ => "lower EL, AArch32",
    }
}

/// Common exception handler: decode, report, and halt. Reached from every vector
/// entry via `common_exception` in `exceptions.s`; `kind` is the entry index.
#[no_mangle]
extern "C" fn exception_dispatch(kind: u64) -> ! {
    let esr: u64;
    let elr: u64;
    let far: u64;
    let spsr: u64;
    // SAFETY: reading these system registers has no memory effects. Read them
    // before any other work so they aren't clobbered by a later exception.
    unsafe {
        asm!("mrs {0}, esr_el1", out(reg) esr, options(nomem, nostack, preserves_flags));
        asm!("mrs {0}, elr_el1", out(reg) elr, options(nomem, nostack, preserves_flags));
        asm!("mrs {0}, far_el1", out(reg) far, options(nomem, nostack, preserves_flags));
        asm!("mrs {0}, spsr_el1", out(reg) spsr, options(nomem, nostack, preserves_flags));
    }
    let ec = (esr >> 26) & 0x3F;

    kprintln!();
    kprintln!("*** EXCEPTION (vector {}: {}) ***", kind, vector_name(kind));
    kprintln!("  ESR_EL1  = {:#018x}  (EC = {:#04x}: {})", esr, ec, ec_name(ec));
    kprintln!("  ELR_EL1  = {:#018x}", elr);
    kprintln!("  FAR_EL1  = {:#018x}", far);
    kprintln!("  SPSR_EL1 = {:#018x}", spsr);
    kprintln!("halting.");

    loop {
        // SAFETY: `wfe` just waits for an event; no memory effects.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}
