//! Local IRQ masking helpers (PSTATE.I).
//!
//! On a single core, disabling IRQs is the mutual-exclusion mechanism for
//! scheduler critical sections: the timer can't fire mid-update, so shared
//! scheduler state can be touched safely.

use core::arch::asm;

/// Disable IRQs, returning the previous DAIF so it can be restored.
pub fn disable() -> u64 {
    let daif: u64;
    // SAFETY: reading DAIF and setting the I mask have no memory effects.
    unsafe {
        asm!("mrs {0}, daif", out(reg) daif, options(nomem, nostack, preserves_flags));
        asm!("msr daifset, #2", options(nomem, nostack, preserves_flags));
    }
    daif
}

/// Restore a DAIF value previously returned by [`disable`].
pub fn restore(daif: u64) {
    // SAFETY: restoring the interrupt-mask state has no memory effects.
    unsafe { asm!("msr daif, {0}", in(reg) daif, options(nomem, nostack, preserves_flags)) };
}

/// Unmask IRQs.
pub fn enable() {
    // SAFETY: enabling IRQ delivery has no memory effects of its own.
    unsafe { asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags)) };
}
