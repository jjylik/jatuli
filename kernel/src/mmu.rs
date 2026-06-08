//! Enable the AArch64 MMU with an identity map.
//!
//! Builds a three-level page table (L0 -> L1 -> L2) in frames from the frame
//! allocator, identity-mapping the device region (Device memory) and the 128 MiB
//! of RAM (Normal cacheable), then programs MAIR/TCR/TTBR0 and sets SCTLR_EL1.M.
//! Because the map is identity (virtual == physical), the kernel keeps running at
//! the same addresses once translation is on.

use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::frames::{alloc_frame, free_frame_at};

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

/// Physical address of the kernel's L1 table — the one the boot L0[0] points at,
/// covering the device MMIO, the RAM identity map, and the kernel heap. Captured
/// at [`init_mmu`]; every per-process address space aliases it at *its* L0[0], so
/// the kernel stays mapped under any `TTBR0`. Zero until the MMU is initialized.
static KERNEL_L1: AtomicUsize = AtomicUsize::new(0);

/// The boot address-space root: a kernel-only space (slot 1 empty). Used as a
/// neutral space to switch onto before freeing a dying process's tables.
static KERNEL_TTBR0: AtomicUsize = AtomicUsize::new(0);

/// The boot/kernel address-space root (`TTBR0` value).
pub fn kernel_ttbr0() -> u64 {
    KERNEL_TTBR0.load(Ordering::Relaxed) as u64
}

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

    // Remember the kernel L1 so per-process address spaces can alias it.
    KERNEL_L1.store(l1 as usize, Ordering::Relaxed);

    let ttbr0 = l0 as u64;
    KERNEL_TTBR0.store(ttbr0 as usize, Ordering::Relaxed);

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

/// The live `TTBR0_EL1` value (the L0 table physical address).
pub fn current_ttbr0() -> u64 {
    let ttbr0: u64;
    // SAFETY: reading a system register has no memory effects.
    unsafe {
        asm!("mrs {0}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack, preserves_flags));
    }
    ttbr0
}

/// The L0 table pointer for an address-space root (`TTBR0` value).
pub fn l0_ptr(ttbr0: u64) -> *mut u64 {
    (ttbr0 & ADDR_MASK) as *mut u64
}

/// Physical address of the live `TTBR0_EL1` L0 table.
fn live_l0() -> *mut u64 {
    l0_ptr(current_ttbr0())
}

/// Map one 4 KiB page in the live address space: virtual `va` -> physical `pa`
/// with the given descriptor `flags`, creating any missing intermediate tables.
/// `va` and `pa` must be 4 KiB-aligned. Used for kernel mappings (heap), which
/// live under the shared L0[0] and so resolve the same in every address space.
pub fn map_page(va: usize, pa: usize, flags: u64) {
    // SAFETY: `live_l0()` is the live top-level table.
    unsafe { map_page_in(live_l0(), va, pa, flags) }
}

/// Map one 4 KiB page into the address space rooted at `l0` (which need not be
/// the live one — this is how a process's tables are built before it runs).
///
/// # Safety
/// `l0` must point at a valid 512-entry L0 table.
pub unsafe fn map_page_in(l0: *mut u64, va: usize, pa: usize, flags: u64) {
    let i0 = (va >> 39) & 0x1FF;
    let i1 = (va >> 30) & 0x1FF;
    let i2 = (va >> 21) & 0x1FF;
    let i3 = (va >> 12) & 0x1FF;

    // get_or_create keeps every level a valid table, and each index is < 512.
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

/// Remove the 4 KiB mapping at `va` in the live address space.
pub fn unmap_page(va: usize) {
    // SAFETY: `live_l0()` is the live top-level table.
    unsafe { unmap_page_in(live_l0(), va) }
}

/// Remove the 4 KiB mapping at `va` in the address space rooted at `l0`: clear
/// its L3 descriptor and invalidate the TLB entry. No-op if the address was
/// never mapped (any walk level missing).
///
/// # Safety
/// `l0` must point at a valid 512-entry L0 table.
pub unsafe fn unmap_page_in(l0: *mut u64, va: usize) {
    let mut table = l0;

    // Walk L0 -> L2 without creating anything; bail if a level is absent.
    for shift in [39usize, 30, 21] {
        let index = (va >> shift) & 0x1FF;
        let entry = table.add(index).read_volatile();
        if entry & DESC_TABLE != DESC_TABLE {
            return;
        }
        table = (entry & ADDR_MASK) as *mut u64;
    }

    let i3 = (va >> 12) & 0x1FF;
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

/// Translate `va` in the address space rooted at `ttbr0` to a physical address,
/// or `None` if unmapped. Walks the tables read-only — used by the isolation
/// self-check to prove two processes map the same VA to different frames.
pub fn translate(ttbr0: u64, va: usize) -> Option<usize> {
    let mut table = l0_ptr(ttbr0);
    for shift in [39usize, 30, 21] {
        let index = (va >> shift) & 0x1FF;
        // SAFETY: `table` is a valid 512-entry page table; index < 512.
        let entry = unsafe { table.add(index).read_volatile() };
        if entry & DESC_TABLE != DESC_TABLE {
            return None;
        }
        table = (entry & ADDR_MASK) as *mut u64;
    }
    let i3 = (va >> 12) & 0x1FF;
    // SAFETY: `table` is the live L3 table; index < 512.
    let entry = unsafe { table.add(i3).read_volatile() };
    if entry & DESC_VALID == 0 {
        return None;
    }
    Some(((entry & ADDR_MASK) as usize) | (va & 0xFFF))
}

/// Create a fresh address space: an L0 table whose slot 0 aliases the shared
/// kernel L1 (so the kernel stays mapped under this `TTBR0`) and whose slot 1
/// (the user window) starts empty, filled lazily by [`map_page_in`]. Returns the
/// L0's physical address, i.e. the `TTBR0` value for the new space.
pub fn new_address_space() -> u64 {
    let l0 = alloc_table();
    let kernel_l1 = KERNEL_L1.load(Ordering::Relaxed);
    debug_assert!(kernel_l1 != 0, "new_address_space before init_mmu");
    // SAFETY: `l0` is a fresh zeroed 512-entry table; index 0 < 512. Slot 0
    // points at the kernel L1, shared by reference with every other space.
    unsafe { l0.add(0).write_volatile((kernel_l1 as u64) | DESC_TABLE) };
    l0 as u64
}

/// The address-space root currently installed in `TTBR0`. Lets [`activate`] skip
/// a redundant switch+flush when the target space is already live. `0` until the
/// first activation.
static INSTALLED: AtomicUsize = AtomicUsize::new(0);

/// Install `ttbr0` as the live address space and flush the whole TLB (no ASIDs
/// yet, so stale entries from the previous space cannot leak). Idempotent: a
/// no-op if `ttbr0` is already live. Self-tracking, so the scheduler, the SQPOLL
/// poller, and IRQ completion can all call it; runs with IRQs masked so the
/// load/store/`msr` sequence can't be split by preemption.
pub fn activate(ttbr0: u64) {
    let d = crate::irq::disable();
    if INSTALLED.load(Ordering::Relaxed) != ttbr0 as usize {
        INSTALLED.store(ttbr0 as usize, Ordering::Relaxed);
        // SAFETY: `ttbr0` is an L0 table PA from `new_address_space` (or the boot
        // L0). Slot 0 of every such table aliases the kernel L1, so kernel code
        // and the current stack remain mapped across the switch.
        unsafe {
            asm!(
                "msr ttbr0_el1, {t}",
                "isb",
                "tlbi vmalle1",     // drop every TLB entry (no ASIDs)
                "dsb ish",
                "isb",
                t = in(reg) ttbr0,
                options(nostack, preserves_flags),
            );
        }
    }
    crate::irq::restore(d);
}

/// Free the address space rooted at `l0`: reclaim its slot-1 (user) page-table
/// frames and the L0 frame itself. Slot 0 (the shared kernel L1) is never
/// followed. `l0` must not be the live `TTBR0` when this is called.
///
/// # Safety
/// `l0` must be an L0 PA from [`new_address_space`] that is no longer in use.
pub unsafe fn free_address_space(l0: u64) {
    let l0 = (l0 & ADDR_MASK) as *mut u64;
    // Only slot 1 (the user window) holds process-private tables; free its
    // subtree depth-first (L3s, then L2s, then the L1), then the L0.
    let e1 = l0.add(USER_L0_IDX).read_volatile();
    if e1 & DESC_TABLE == DESC_TABLE {
        let l1 = (e1 & ADDR_MASK) as *mut u64;
        free_subtree(l1, 2); // L1 has L2 children (level depth 2 below it)
    }
    free_frame_at(l0 as usize);
}

/// Recursively free a page-table subtree. `levels_below` is how many table
/// levels sit beneath `table` (L1 -> 2: L2 then L3). At `levels_below == 0` the
/// children are page descriptors, not tables, so we stop after freeing `table`.
///
/// # Safety
/// `table` must be a valid 512-entry page table owned by a dead address space.
unsafe fn free_subtree(table: *mut u64, levels_below: u32) {
    if levels_below > 0 {
        for i in 0..ENTRIES {
            let entry = table.add(i).read_volatile();
            if entry & DESC_TABLE == DESC_TABLE {
                free_subtree((entry & ADDR_MASK) as *mut u64, levels_below - 1);
            }
        }
    }
    free_frame_at(table as usize);
}

/// L0 index of the user window — kept in sync with `abi::USER_L0_IDX` (= 1).
const USER_L0_IDX: usize = abi::USER_L0_IDX;

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
