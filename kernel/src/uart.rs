//! Minimal PL011 UART driver (transmit + polled receive) for the QEMU `virt`
//! machine.

use core::ptr::{read_volatile, write_volatile};

/// Base MMIO address of PL011 UART0 on QEMU `virt`.
const UART0_BASE: usize = 0x0900_0000;
/// Data register offset.
const UARTDR: usize = 0x00;
/// Flag register offset.
const UARTFR: usize = 0x18;
/// Transmit-FIFO-full flag (UARTFR bit 5).
const TXFF: u32 = 1 << 5;
/// Receive-FIFO-empty flag (UARTFR bit 4).
const RXFE: u32 = 1 << 4;
/// Interrupt-mask register offset.
const UARTIMSC: usize = 0x38;
/// Receive-interrupt mask bit (UARTIMSC bit 4).
const RXIM: u32 = 1 << 4;

/// INTID of the PL011 UART on the QEMU `virt` machine (SPI 1).
pub const UART_INTID: u32 = 33;

/// Mask or unmask the receive interrupt. The RX condition is level-asserted
/// while unread data sits in the receiver (and cleared by reading `UARTDR`),
/// so the interrupt is enabled only while someone is waiting for input —
/// otherwise it would re-fire endlessly with nobody to consume the byte.
pub fn set_rx_irq(enabled: bool) {
    // SAFETY: valid PL011 MMIO register on the virt machine; volatile write.
    unsafe {
        let imsc = (UART0_BASE + UARTIMSC) as *mut u32;
        write_volatile(imsc, if enabled { RXIM } else { 0 });
    }
}

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

/// Read a single byte if one is waiting, without blocking. The PL011 stays in
/// its reset (1-byte FIFO) mode; QEMU's chardev layer applies backpressure, so
/// pasted or piped input queues host-side rather than being dropped.
pub fn try_getc() -> Option<u8> {
    // SAFETY: valid PL011 MMIO registers on the virt machine; volatile reads.
    unsafe {
        let fr = (UART0_BASE + UARTFR) as *const u32;
        if read_volatile(fr) & RXFE != 0 {
            return None;
        }
        let dr = (UART0_BASE + UARTDR) as *const u32;
        Some(read_volatile(dr) as u8)
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

/// A zero-sized handle implementing [`core::fmt::Write`] over the UART, so the
/// `kprint!`/`kprintln!` macros can render `format_args!` without a heap.
pub struct Uart;

impl core::fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write_str(s);
        Ok(())
    }
}

/// Backing function for the `kprint!`/`kprintln!` macros.
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    // Writing to the UART is infallible here; ignore the formatter Result.
    let _ = Uart.write_fmt(args);
}

/// Print formatted text to the UART (no trailing newline).
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::uart::_print(format_args!($($arg)*)) };
}

/// Print formatted text to the UART followed by a newline.
#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => { $crate::kprint!("{}\n", format_args!($($arg)*)) };
}
