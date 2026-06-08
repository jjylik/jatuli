//! Minimal ELF64 loader for the embedded EL0 userspace image.
//!
//! Supports exactly what jos needs: a little-endian AArch64 `ET_EXEC` image with
//! `PT_LOAD` segments at fixed virtual addresses. No relocations, no dynamic
//! linking, no demand paging.

use alloc::vec::Vec;

use crate::frames::{alloc_frame, Frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW, PAGE_USER_RX};

/// The userspace program's ELF image, built by `build.rs` and embedded here.
pub static USER_ELF: &[u8] = include_bytes!(env!("USER_ELF"));

// ELF64 header field offsets (little-endian).
const E_TYPE: usize = 16; // u16; 2 = ET_EXEC
const E_MACHINE: usize = 18; // u16; 0xB7 = EM_AARCH64
const E_ENTRY: usize = 24; // u64
const E_PHOFF: usize = 32; // u64
const E_PHENTSIZE: usize = 54; // u16
const E_PHNUM: usize = 56; // u16

fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

#[allow(dead_code)] // used by the segment loader (Task 4).
fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn read_u64(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

/// Validate that `image` is a 64-bit little-endian AArch64 `ET_EXEC` ELF and
/// return its entry virtual address. Panics on a malformed header.
pub fn validate(image: &[u8]) -> usize {
    assert!(image.len() >= 64, "ELF image too small for a header");
    assert_eq!(&image[0..4], b"\x7fELF", "bad ELF magic");
    assert_eq!(image[4], 2, "not ELFCLASS64");
    assert_eq!(read_u16(image, E_TYPE), 2, "not ET_EXEC");
    assert_eq!(read_u16(image, E_MACHINE), 0xB7, "not EM_AARCH64");
    read_u64(image, E_ENTRY) as usize
}

// Program-header field offsets (within one entry).
const P_TYPE: usize = 0; // u32; 1 = PT_LOAD
const P_FLAGS: usize = 4; // u32; bit0 = X, bit1 = W, bit2 = R
const P_OFFSET: usize = 8; // u64
const P_VADDR: usize = 16; // u64
const P_FILESZ: usize = 32; // u64
const P_MEMSZ: usize = 40; // u64

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

/// Maximum number of `PT_LOAD` segments we track.
const MAX_SEGMENTS: usize = 8;

/// A virtual-address range occupied by a loaded segment.
#[derive(Clone, Copy)]
pub struct Range {
    pub start: usize,
    pub end: usize,
    /// Whether the segment was mapped writable (`PAGE_USER_RW`; everything the
    /// loader maps is either RX or RW). Gates the kernel writing into it on the
    /// user's behalf (e.g. `SYS_READ`).
    pub writable: bool,
}

/// The result of loading an image: where to begin execution, and the mapped
/// virtual ranges (used to validate user-supplied syscall pointers).
#[derive(Clone, Copy)]
pub struct Loaded {
    pub entry: usize,
    pub ranges: [Range; MAX_SEGMENTS],
    pub count: usize,
}

fn align_up(x: usize) -> usize {
    (x + FRAME_SIZE - 1) & !(FRAME_SIZE - 1)
}

/// Map every `PT_LOAD` segment of `image` at its `p_vaddr` with EL0 permissions,
/// copying file contents in and zero-filling the BSS tail. Returns the entry
/// point and the mapped ranges; every `(va, frame)` pair mapped is recorded in
/// `owned` so teardown can unmap and free the program's memory. Panics on
/// anything malformed or unsupported.
pub fn load(image: &[u8], ttbr0: u64, owned: &mut Vec<(usize, Frame)>) -> Loaded {
    let entry = validate(image);
    let l0 = mmu::l0_ptr(ttbr0);
    let phoff = read_u64(image, E_PHOFF) as usize;
    let phentsize = read_u16(image, E_PHENTSIZE) as usize;
    let phnum = read_u16(image, E_PHNUM) as usize;

    let mut ranges = [Range {
        start: 0,
        end: 0,
        writable: false,
    }; MAX_SEGMENTS];
    let mut count = 0;

    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if read_u32(image, ph + P_TYPE) != PT_LOAD {
            continue;
        }
        let memsz = read_u64(image, ph + P_MEMSZ) as usize;
        if memsz == 0 {
            continue;
        }
        let filesz = read_u64(image, ph + P_FILESZ) as usize;
        let vaddr = read_u64(image, ph + P_VADDR) as usize;
        let offset = read_u64(image, ph + P_OFFSET) as usize;
        let flags = read_u32(image, ph + P_FLAGS);

        assert_eq!(vaddr % FRAME_SIZE, 0, "segment vaddr not page-aligned");
        let executable = flags & PF_X != 0;
        let perms = if executable {
            assert_eq!(flags & PF_W, 0, "W^X violated: segment is both W and X");
            PAGE_USER_RX
        } else {
            PAGE_USER_RW
        };

        // Map page by page. Populate each frame through its identity-mapped
        // physical address (writable at EL1) BEFORE installing the user mapping,
        // because PAGE_USER_RX is read-only even to the kernel.
        let mut page = 0;
        while page < align_up(memsz) {
            let frame = alloc_frame().expect("out of frames loading a user segment");
            let dst = frame.addr() as *mut u8;

            for b in 0..FRAME_SIZE {
                let seg_off = page + b;
                let byte = if seg_off < filesz {
                    image[offset + seg_off]
                } else {
                    0 // BSS tail (memsz > filesz) or padding.
                };
                // SAFETY: `dst` is a fresh identity-mapped frame, writable at EL1.
                unsafe { dst.add(b).write_volatile(byte) };
            }

            // Code must reach the instruction fetch path before we execute it.
            if executable {
                mmu::sync_instruction_cache(frame.addr(), FRAME_SIZE);
            }

            // SAFETY: `l0` is the target process's address-space root, being
            // populated before it runs; this VA in the user window is unmapped.
            unsafe { mmu::map_page_in(l0, vaddr + page, frame.addr(), perms) };
            owned.push((vaddr + page, frame));
            page += FRAME_SIZE;
        }

        assert!(count < MAX_SEGMENTS, "too many PT_LOAD segments");
        ranges[count] = Range {
            start: vaddr,
            end: vaddr + memsz,
            writable: !executable,
        };
        count += 1;
    }

    Loaded {
        entry,
        ranges,
        count,
    }
}
