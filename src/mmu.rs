//! Enable the AArch64 MMU with an identity map.
//!
//! Builds a three-level page table (L0 -> L1 -> L2) in frames from the frame
//! allocator, identity-mapping the device region (Device memory) and the 128 MiB
//! of RAM (Normal cacheable), then programs MAIR/TCR/TTBR0 and sets SCTLR_EL1.M.
//! Because the map is identity (virtual == physical), the kernel keeps running at
//! the same addresses once translation is on.

use core::arch::asm;

use crate::frames::alloc_frame;

/// Entries per 4 KiB table (4096 / 8).
const ENTRIES: usize = 512;

// Descriptor type bits [1:0].
const DESC_BLOCK: u64 = 0b01;
const DESC_TABLE: u64 = 0b11;

// Lower attributes (zero-valued fields — AttrIndx=0 for Normal, AP=0b00 for EL1
// read/write — are simply left out of the OR).
const ATTR_IDX_DEVICE: u64 = 1 << 2; // MAIR index 1
const SH_INNER: u64 = 0b11 << 8; // inner shareable (Normal memory)
const AF: u64 = 1 << 10; // access flag (must be set or first access faults)

// Upper attributes.
const PXN: u64 = 1 << 53; // privileged execute-never
const UXN: u64 = 1 << 54; // unprivileged execute-never

/// Block flags for Normal cacheable RAM (AttrIndx=0, AP=EL1 RW, kernel-executable).
const NORMAL_BLOCK: u64 = DESC_BLOCK | SH_INNER | AF | UXN;
/// Block flags for Device MMIO (AttrIndx=1, never executable).
const DEVICE_BLOCK: u64 = DESC_BLOCK | ATTR_IDX_DEVICE | AF | PXN | UXN;

// Identity-mapped physical layout.
const RAM_BASE: usize = 0x4000_0000;
const RAM_SIZE: usize = 128 * 1024 * 1024;
const BLOCK_2MIB: usize = 2 * 1024 * 1024;
const RAM_BLOCKS: usize = RAM_SIZE / BLOCK_2MIB; // 64

/// MAIR: attr0 = Normal write-back (0xFF), attr1 = Device-nGnRE (0x04).
const MAIR: u64 = (0x04 << 8) | 0xFF;
/// TCR: T0SZ=16 (48-bit VA), 4 KiB granule, inner-shareable WB walks, TTBR1
/// disabled, TG1=4 KiB, IPS=36-bit.
const TCR: u64 = 0x1_8080_3510;

/// Allocate a frame and zero it for use as a page table.
fn alloc_table() -> *mut u64 {
    let frame = alloc_frame().expect("out of frames while building page tables");
    let table = frame.addr() as *mut u64;
    for i in 0..ENTRIES {
        // SAFETY: a frame is 4 KiB (>= 512 u64 slots) and flat-mapped before the MMU is on.
        unsafe { table.add(i).write_volatile(0) };
    }
    table
}

/// Build the identity-map page tables and enable the MMU. Call once, after the
/// frame allocator is initialized.
pub fn init_mmu() {
    let l0 = alloc_table();
    let l1 = alloc_table();
    let l2 = alloc_table();

    // SAFETY: all three are valid zeroed tables; every index below is < 512.
    unsafe {
        // L2: 64 Normal 2 MiB blocks covering exactly the 128 MiB of RAM.
        for i in 0..RAM_BLOCKS {
            let pa = (RAM_BASE + i * BLOCK_2MIB) as u64;
            l2.add(i).write_volatile(pa | NORMAL_BLOCK);
        }

        // L1[0]: Device 1 GiB block at physical 0 (UART, GIC, ...).
        l1.add(0).write_volatile(DEVICE_BLOCK);
        // L1[1]: table -> L2 (covers 0x4000_0000 .. 0x8000_0000).
        l1.add(1).write_volatile((l2 as u64) | DESC_TABLE);

        // L0[0]: table -> L1 (covers 0 .. 512 GiB).
        l0.add(0).write_volatile((l1 as u64) | DESC_TABLE);
    }

    let ttbr0 = l0 as u64;

    // SAFETY: the tables are fully built above; this programs and enables translation.
    unsafe {
        asm!(
            "dsb ishst",        // ensure table writes are visible to the table walker
            "tlbi vmalle1",     // flush any stale TLB entries
            "dsb ish",
            "isb",
            "msr mair_el1, {mair}",
            "msr tcr_el1, {tcr}",
            "msr ttbr0_el1, {ttbr0}",
            "isb",
            mair = in(reg) MAIR,
            tcr = in(reg) TCR,
            ttbr0 = in(reg) ttbr0,
            options(nostack, preserves_flags),
        );

        // Enable MMU (M) plus data/instruction caches (C, I), consistent with the
        // write-back attributes above.
        let mut sctlr: u64;
        asm!("mrs {0}, sctlr_el1", out(reg) sctlr, options(nomem, nostack, preserves_flags));
        sctlr |= (1 << 0) | (1 << 2) | (1 << 12); // M | C | I
        asm!(
            "msr sctlr_el1, {0}",
            "isb",
            in(reg) sctlr,
            options(nostack, preserves_flags),
        );
    }
}
