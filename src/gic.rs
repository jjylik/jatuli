//! GICv3 interrupt controller (QEMU `virt`).
//!
//! The Distributor (GICD) and per-CPU Redistributor (GICR) are MMIO; the CPU
//! interface is the `ICC_*` system registers. This brings up a single CPU at EL1
//! and enables one PPI (the timer). All GIC MMIO sits in the identity-mapped
//! device region.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};

/// Distributor base.
const GICD_BASE: usize = 0x0800_0000;
/// Redistributor base for CPU0 (RD frame).
const GICR_BASE: usize = 0x080A_0000;
/// Redistributor SGI/PPI frame for CPU0 (RD base + 64 KiB).
const GICR_SGI_BASE: usize = GICR_BASE + 0x1_0000;

// Distributor registers (offset from GICD_BASE).
const GICD_CTLR: usize = 0x0000;

// Redistributor RD-frame registers (offset from GICR_BASE).
const GICR_WAKER: usize = 0x0014;

// Redistributor SGI-frame registers (offset from GICR_SGI_BASE).
const GICR_IGROUPR0: usize = 0x0080;
const GICR_ISENABLER0: usize = 0x0100;
const GICR_IPRIORITYR: usize = 0x0400;

// GICR_WAKER bits.
const WAKER_PROCESSOR_SLEEP: u32 = 1 << 1;
const WAKER_CHILDREN_ASLEEP: u32 = 1 << 2;

// GICD_CTLR bits (single security state, DS=1 on QEMU `virt`).
const CTLR_ENABLE_GRP0: u32 = 1 << 0;
const CTLR_ENABLE_GRP1: u32 = 1 << 1;
const CTLR_ARE: u32 = 1 << 4;

/// INTIDs >= this value returned by the IAR are spurious (no real interrupt).
const SPURIOUS_MIN: u32 = 1020;

unsafe fn w32(base: usize, off: usize, val: u32) {
    write_volatile((base + off) as *mut u32, val);
}

unsafe fn r32(base: usize, off: usize) -> u32 {
    read_volatile((base + off) as *const u32)
}

/// Bring up the GICv3 for a single CPU at EL1 and enable the given PPI INTID
/// (must be < 32). Call once.
pub fn init(ppi_intid: u32) {
    // SAFETY: GIC MMIO and ICC_* registers are valid on the virt machine; this
    // runs once during early boot.
    unsafe {
        // 1. Enable the system-register CPU interface.
        let mut sre: u64;
        asm!("mrs {0}, ICC_SRE_EL1", out(reg) sre, options(nomem, nostack, preserves_flags));
        sre |= 1; // SRE
        asm!("msr ICC_SRE_EL1, {0}", "isb", in(reg) sre, options(nostack, preserves_flags));

        // 2. Distributor: affinity routing + groups enabled.
        w32(GICD_BASE, GICD_CTLR, CTLR_ARE | CTLR_ENABLE_GRP1 | CTLR_ENABLE_GRP0);

        // 3. Wake this CPU's redistributor.
        let waker = r32(GICR_BASE, GICR_WAKER) & !WAKER_PROCESSOR_SLEEP;
        w32(GICR_BASE, GICR_WAKER, waker);
        while r32(GICR_BASE, GICR_WAKER) & WAKER_CHILDREN_ASLEEP != 0 {}

        // 4. Configure the PPI in the redistributor SGI frame.
        let bit = 1u32 << ppi_intid;
        w32(GICR_SGI_BASE, GICR_IGROUPR0, r32(GICR_SGI_BASE, GICR_IGROUPR0) | bit); // group 1
        write_volatile((GICR_SGI_BASE + GICR_IPRIORITYR + ppi_intid as usize) as *mut u8, 0x00);
        w32(GICR_SGI_BASE, GICR_ISENABLER0, bit); // enable

        // 5. CPU interface: allow all priorities, enable group 1.
        asm!("msr ICC_PMR_EL1, {0}", in(reg) 0xFFu64, options(nostack, preserves_flags));
        asm!("msr ICC_IGRPEN1_EL1, {0}", "isb", in(reg) 1u64, options(nostack, preserves_flags));
    }
}

/// Acknowledge the highest-priority pending interrupt and return its INTID.
/// A value >= [`SPURIOUS_MIN`] means there was no real interrupt to handle.
pub fn acknowledge() -> u32 {
    let iar: u64;
    // SAFETY: reading ICC_IAR1_EL1 acknowledges the pending Group 1 interrupt.
    unsafe { asm!("mrs {0}, ICC_IAR1_EL1", out(reg) iar, options(nomem, nostack, preserves_flags)) };
    iar as u32
}

/// Returns whether `intid` (as returned by [`acknowledge`]) is spurious.
pub fn is_spurious(intid: u32) -> bool {
    intid >= SPURIOUS_MIN
}

/// Signal end-of-interrupt for a previously acknowledged `intid`.
pub fn eoi(intid: u32) {
    // SAFETY: writing ICC_EOIR1_EL1 completes handling of the acknowledged interrupt.
    unsafe { asm!("msr ICC_EOIR1_EL1, {0}", in(reg) intid as u64, options(nostack, preserves_flags)) };
}
