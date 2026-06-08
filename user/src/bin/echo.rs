//! `echo`: a second, distinct program. Prints a baked-in line and exits.
//!
//! It cannot read input yet — the foreground process owns the keyboard — so the
//! message stands in for an argument until args/spawn exist. Its purpose this
//! phase is to prove a *different* program loads and runs isolated in its own
//! address space and ring, then tears down cleanly.

#![no_std]
#![no_main]

use user::{exit, print, uring};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    uring::setup();
    print("echo: hello from a second program\n");
    exit(0)
}
