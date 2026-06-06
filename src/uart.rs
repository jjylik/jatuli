//! Minimal PL011 UART output driver for the QEMU `virt` machine.

use core::ptr::{read_volatile, write_volatile};

/// Base MMIO address of PL011 UART0 on QEMU `virt`.
const UART0_BASE: usize = 0x0900_0000;
/// Data register offset.
const UARTDR: usize = 0x00;
/// Flag register offset.
const UARTFR: usize = 0x18;
/// Transmit-FIFO-full flag (UARTFR bit 5).
const TXFF: u32 = 1 << 5;

/// Write a single byte, spinning until the TX FIFO has room.
fn write_byte(b: u8) {
    // SAFETY: these are valid PL011 MMIO registers on the virt machine; volatile
    // access prevents the compiler from reordering or eliding the hardware writes.
    unsafe {
        let fr = (UART0_BASE + UARTFR) as *const u32;
        while read_volatile(fr) & TXFF != 0 {}
        let dr = (UART0_BASE + UARTDR) as *mut u32;
        write_volatile(dr, b as u32);
    }
}

/// Write a string, translating `\n` into `\r\n` for terminal-friendly output.
pub fn write_str(s: &str) {
    for &b in s.as_bytes() {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
}
