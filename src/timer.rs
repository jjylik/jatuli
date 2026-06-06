//! ARM generic timer (EL1 physical timer, `CNTP_*`).
//!
//! Fires a periodic interrupt — a PPI, INTID 30 on the `virt` machine — at
//! ~100 Hz. The IRQ handler calls [`on_tick`], which counts the tick and reloads
//! the countdown (writing `CNTP_TVAL_EL0` re-arms the timer and clears the
//! pending condition).

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

/// INTID of the EL1 physical timer on the `virt` machine (PPI 14).
pub const TIMER_INTID: u32 = 30;

/// Target interrupt rate (ticks per second).
const TICK_HZ: u64 = 100;

/// Timer input frequency in Hz, read from `CNTFRQ_EL0` at init.
static FREQ: AtomicU64 = AtomicU64::new(0);
/// Number of timer ticks observed since boot.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Number of timer ticks since boot.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Countdown value (in timer cycles) for one tick interval.
fn interval() -> u64 {
    FREQ.load(Ordering::Relaxed) / TICK_HZ
}

/// Read the timer frequency and start the periodic timer. Call once, after the
/// GIC has enabled [`TIMER_INTID`].
pub fn init() {
    let freq: u64;
    // SAFETY: reading CNTFRQ_EL0 has no memory effects.
    unsafe { asm!("mrs {0}, cntfrq_el0", out(reg) freq, options(nomem, nostack, preserves_flags)) };
    FREQ.store(freq, Ordering::Relaxed);

    let tval = freq / TICK_HZ;
    // SAFETY: programming this core's physical timer.
    unsafe {
        asm!("msr cntp_tval_el0, {0}", in(reg) tval, options(nostack, preserves_flags));
        asm!("msr cntp_ctl_el0, {0}", in(reg) 1u64, options(nostack, preserves_flags)); // ENABLE
    }
}

/// Handle one timer tick: count it and reload the countdown (which also clears
/// the pending timer condition, so it must happen before the GIC EOI).
pub fn on_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    let tval = interval();
    // SAFETY: reloading this core's physical timer countdown.
    unsafe { asm!("msr cntp_tval_el0, {0}", in(reg) tval, options(nostack, preserves_flags)) };
}
