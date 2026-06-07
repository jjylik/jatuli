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
const DESC_VALID: u64 = 0b01; // bit 0 set = entry is valid
const DESC_PAGE: u64 = 0b11; // at L3, 0b11 means a page (not a table)

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

/// Mask selecting a descriptor's output-address bits [47:12].
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

/// L3 page-descriptor flags for kernel read/write data: Normal cacheable, EL1
/// read/write, non-executable. (Type bits 0b11 = a valid page at L3.)
pub const PAGE_KERNEL_RW: u64 = DESC_PAGE | SH_INNER | AF | UXN | PXN;

/// Access permission: EL1 read/write, EL0 read/write.
const AP_EL0_RW: u64 = 0b01 << 6;
/// Access permission: EL1 read-only, EL0 read-only.
const AP_EL0_RO: u64 = 0b11 << 6;

/// L3 page flags for user read/write data (EL0 RW, non-executable).
pub const PAGE_USER_RW: u64 = DESC_PAGE | AP_EL0_RW | SH_INNER | AF | UXN | PXN;
/// L3 page flags for user code (EL0 read + execute; kernel cannot execute it).
pub const PAGE_USER_RX: u64 = DESC_PAGE | AP_EL0_RO | SH_INNER | AF | PXN;

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

/// Follow the table descriptor at `table[index]`, allocating and installing a
/// fresh next-level table if the entry is empty.
///
/// # Safety
/// `table` must point at a valid 512-entry page table.
unsafe fn get_or_create(table: *mut u64, index: usize) -> *mut u64 {
    let entry = table.add(index).read_volatile();
    if entry & DESC_VALID != 0 {
        assert!(
            entry & DESC_TABLE == DESC_TABLE,
            "expected a table descriptor while walking, found a block"
        );
        (entry & ADDR_MASK) as *mut u64
    } else {
        let next = alloc_table();
        table.add(index).write_volatile((next as u64) | DESC_TABLE);
        next
    }
}

/// Map one 4 KiB page: virtual `va` -> physical `pa` with the given descriptor
/// `flags`, creating any missing intermediate tables. `va` and `pa` must be
/// 4 KiB-aligned. Requires the MMU to be on (it walks the live TTBR0 tables).
pub fn map_page(va: usize, pa: usize, flags: u64) {
    let ttbr0: u64;
    // SAFETY: reading a system register has no memory effects.
    unsafe {
        asm!("mrs {0}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack, preserves_flags));
    }
    let l0 = (ttbr0 & ADDR_MASK) as *mut u64;

    let i0 = (va >> 39) & 0x1FF;
    let i1 = (va >> 30) & 0x1FF;
    let i2 = (va >> 21) & 0x1FF;
    let i3 = (va >> 12) & 0x1FF;

    // SAFETY: l0 is the live top-level table; get_or_create keeps every level a
    // valid table, and each index is < 512.
    unsafe {
        let l1 = get_or_create(l0, i0);
        let l2 = get_or_create(l1, i1);
        let l3 = get_or_create(l2, i2);
        l3.add(i3).write_volatile((pa as u64 & ADDR_MASK) | flags);

        asm!(
            "dsb ishst",            // table writes visible to the walker
            "tlbi vaae1, {page}",   // drop any stale entry for this VA
            "dsb ish",
            "isb",
            page = in(reg) (va >> 12) as u64,
            options(nostack, preserves_flags),
        );
    }
}

/// Remove the 4 KiB mapping at `va`: clear its L3 descriptor and invalidate the
/// TLB entry. No-op if the address was never mapped (any walk level missing).
pub fn unmap_page(va: usize) {
    let ttbr0: u64;
    // SAFETY: reading a system register has no memory effects.
    unsafe {
        asm!("mrs {0}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack, preserves_flags));
    }
    let mut table = (ttbr0 & ADDR_MASK) as *mut u64;

    // Walk L0 -> L2 without creating anything; bail if a level is absent.
    for shift in [39usize, 30, 21] {
        let index = (va >> shift) & 0x1FF;
        // SAFETY: `table` is a live page table of 512 entries; index < 512.
        let entry = unsafe { table.add(index).read_volatile() };
        if entry & DESC_TABLE != DESC_TABLE {
            return;
        }
        table = (entry & ADDR_MASK) as *mut u64;
    }

    let i3 = (va >> 12) & 0x1FF;
    // SAFETY: `table` is the live L3 table; clearing an entry plus TLB
    // invalidation is the architectural unmap sequence.
    unsafe {
        table.add(i3).write_volatile(0);
        asm!(
            "dsb ishst",            // descriptor clear visible to the walker
            "tlbi vaae1, {page}",   // drop the cached translation
            "dsb ish",
            "isb",
            page = in(reg) (va >> 12) as u64,
            options(nostack, preserves_flags),
        );
    }
}

// See B2.6.5 Concurrent modification and execution of instructions in the ARMv8 Architecture Reference Manual
pub fn sync_instruction_cache(pa: usize, len: usize) {
    const LINE: usize = 64; // Cortex-A72 cache-line size.
    let start = pa & !(LINE - 1);
    let end = pa + len;

    // SAFETY: cache maintenance over identity-mapped Normal memory we just wrote.
    unsafe {
        let mut p = start;
        while p < end {
            asm!("dc cvau, {0}", in(reg) p, options(nostack, preserves_flags));
            p += LINE;
        }
        asm!("dsb ish", options(nostack, preserves_flags));
        let mut q = start;
        while q < end {
            asm!("ic ivau, {0}", in(reg) q, options(nostack, preserves_flags));
            q += LINE;
        }
        asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}
