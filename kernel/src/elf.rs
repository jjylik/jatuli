//! Minimal ELF64 loader for the embedded EL0 userspace image.
//!
//! Supports exactly what jos needs: a little-endian AArch64 `ET_EXEC` image with
//! `PT_LOAD` segments at fixed virtual addresses. No relocations, no dynamic
//! linking, no demand paging.

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
