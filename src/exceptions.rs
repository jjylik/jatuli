//! AArch64 exception vectors with a full trap frame.
//!
//! The vector stubs (see `exceptions.s`) save every general register plus ELR
//! and SPSR into a [`TrapFrame`] on the kernel stack, call [`exception_dispatch`],
//! then restore and `ERET`. `SVC` is dispatched as a syscall (the handler may
//! modify the frame — e.g. the return value — then resume). Real faults are
//! reported and the CPU is halted.

use core::arch::{asm, global_asm};

use crate::kprintln;
use crate::syscall;

global_asm!(include_str!("exceptions.s"));

extern "C" {
    /// The exception vector table, defined in `exceptions.s`.
    static exception_vector_table: u8;
}

/// Saved processor state on exception entry. Layout matches `exceptions.s`.
#[repr(C)]
pub struct TrapFrame {
    /// General registers x0..x30.
    pub x: [u64; 31],
    /// Exception Link Register (the return address).
    pub elr: u64,
    /// Saved Program Status Register.
    pub spsr: u64,
    /// Padding to keep the frame 16-byte aligned.
    pub _pad: u64,
}

/// Exception class (`ESR_EL1.EC`) for an AArch64 `SVC`.
const EC_SVC: u64 = 0x15;

/// Install the exception vector table. Call once, early in `kmain`, so any later
/// fault produces a diagnostic instead of a silent hang.
pub fn init_exceptions() {
    let vbar = core::ptr::addr_of!(exception_vector_table) as u64;
    // SAFETY: `vbar` is the address of the correctly 2 KiB-aligned vector table.
    unsafe {
        asm!("msr vbar_el1, {0}", "isb", in(reg) vbar, options(nostack, preserves_flags));
    }
}

/// Read `ESR_EL1` (the exception syndrome register).
fn read_esr() -> u64 {
    let esr: u64;
    // SAFETY: reading a system register has no memory effects.
    unsafe { asm!("mrs {0}, esr_el1", out(reg) esr, options(nomem, nostack, preserves_flags)) };
    esr
}

/// Common exception handler. Reached from every vector via `common_exception`;
/// `kind` is the vector index and `frame` is the saved state (mutable, so a
/// syscall can write its return value or a handler can adjust `elr`).
#[no_mangle]
extern "C" fn exception_dispatch(kind: u64, frame: *mut TrapFrame) {
    // SAFETY: the asm stub passes a pointer to a valid TrapFrame on the stack.
    let frame = unsafe { &mut *frame };
    let esr = read_esr();

    // The 16 vectors come in groups of four; index % 4 == 1 is the IRQ entry
    // (vector 5 at our current EL with SPx).
    match kind % 4 {
        1 => handle_irq(),
        0 => {
            let ec = (esr >> 26) & 0x3F;
            match ec {
                // kind >= 8 means the SVC came from a lower EL (EL0 userspace).
                EC_SVC => syscall::dispatch(frame, kind >= 8),
                _ => report_and_halt(kind, esr, frame),
            }
        }
        _ => report_and_halt(kind, esr, frame),
    }
}

/// Service an IRQ: acknowledge it at the GIC, dispatch by INTID, then EOI.
fn handle_irq() {
    let intid = crate::gic::acknowledge();
    if crate::gic::is_spurious(intid) {
        return;
    }
    if intid == crate::timer::TIMER_INTID {
        crate::timer::on_tick();
        crate::gic::eoi(intid); // EOI before switching away
        crate::sched::tick(); // wake sleepers + preempt (IRQs already masked)
        return;
    }
    crate::gic::eoi(intid);
}

/// Report an unexpected exception and halt (faults are not recoverable here).
fn report_and_halt(kind: u64, esr: u64, frame: &TrapFrame) -> ! {
    let ec = (esr >> 26) & 0x3F;
    let far: u64;
    // SAFETY: reading a system register has no memory effects.
    unsafe { asm!("mrs {0}, far_el1", out(reg) far, options(nomem, nostack, preserves_flags)) };

    kprintln!();
    kprintln!("*** EXCEPTION (vector {}: {}) ***", kind, vector_name(kind));
    kprintln!("  ESR_EL1  = {:#018x}  (EC = {:#04x}: {})", esr, ec, ec_name(ec));
    kprintln!("  ELR_EL1  = {:#018x}", frame.elr);
    kprintln!("  FAR_EL1  = {:#018x}", far);
    kprintln!("  SPSR_EL1 = {:#018x}", frame.spsr);
    kprintln!("halting.");

    loop {
        // SAFETY: `wfe` just waits for an event; no memory effects.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
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
