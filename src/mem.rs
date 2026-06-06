//! Freestanding implementations of the memory primitives the compiler may emit
//! calls to (struct/slice copies, zero-init, `Vec` growth). Without `libc` or the
//! `compiler_builtins` `mem` feature, these symbols are otherwise undefined.
//!
//! These are ordinary EL1 functions, not syscalls. They MUST use manual byte
//! loops: `core::ptr::copy*` would lower back into `memcpy` and recurse forever.
//! Do not remove or rename them.

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = *src.add(i);
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut u8, value: i32, n: usize) -> *mut u8 {
    let byte = value as u8;
    let mut i = 0;
    while i < n {
        *dest.add(i) = byte;
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (src as usize) < (dest as usize) {
        // Regions may overlap with src before dest: copy backwards.
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dest.add(i) = *src.add(i);
        }
    } else {
        let mut i = 0;
        while i < n {
            *dest.add(i) = *src.add(i);
            i += 1;
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let x = *a.add(i);
        let y = *b.add(i);
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}
