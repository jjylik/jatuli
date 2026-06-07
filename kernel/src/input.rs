//! Kernel console-input buffer.
//!
//! The UART RX interrupt drains the device into this fixed byte ring the
//! moment data arrives — whether or not anyone asked for it yet (type-ahead;
//! on real hardware this is what prevents FIFO-overrun data loss). `READ`
//! completions consume from here, never from the device. The buffer lives in
//! kernel `.bss`: EL0 cannot reach it (identity-map AP bits), so "kernel-only"
//! is enforced by the same permission system the user pages opt out of.
//!
//! Flow control: when the ring fills, the RX interrupt is masked — further
//! input waits host-side (QEMU chardev backpressure) — and unmasked again
//! once a consumer frees space. The Phase 13 on-demand masking pattern, moved
//! down a layer and made honest.

use crate::sync::Locked;
use crate::uart;

/// Capacity of the input ring. Must exceed any burst we expect before a
/// consumer runs (test.sh's whole scripted session is ~20 bytes).
const INPUT_BUF_SIZE: usize = 256;

struct InputBuf {
    buf: [u8; INPUT_BUF_SIZE],
    /// Read position; `head == tail` means empty.
    head: usize,
    /// Write position (one slot is sacrificed to distinguish full from empty).
    tail: usize,
}

static INPUT: Locked<InputBuf> = Locked::new(InputBuf {
    buf: [0; INPUT_BUF_SIZE],
    head: 0,
    tail: 0,
});

impl InputBuf {
    fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    fn is_full(&self) -> bool {
        (self.tail + 1) % INPUT_BUF_SIZE == self.head
    }
}

/// Move everything the UART holds into the ring. Called from the RX interrupt
/// handler (IRQs masked). Masks the RX interrupt if the ring fills — [`pop`]
/// unmasks it once space frees.
pub fn drain_uart() {
    let mut input = INPUT.lock();
    loop {
        if input.is_full() {
            uart::set_rx_irq(false);
            return;
        }
        match uart::try_getc() {
            Some(b) => {
                let tail = input.tail;
                input.buf[tail] = b;
                input.tail = (tail + 1) % INPUT_BUF_SIZE;
            }
            None => return,
        }
    }
}

/// Take one buffered byte, if any. Unmasks the RX interrupt when consuming
/// from a previously full ring (there is space again). IRQs are masked for the
/// duration: the RX interrupt also takes the lock, and a spinlock taken from
/// both thread and IRQ context deadlocks a single core without this.
pub fn pop() -> Option<u8> {
    let d = crate::irq::disable();
    let b = {
        let mut input = INPUT.lock();
        if input.is_empty() {
            None
        } else {
            let was_full = input.is_full();
            let b = input.buf[input.head];
            input.head = (input.head + 1) % INPUT_BUF_SIZE;
            if was_full {
                uart::set_rx_irq(true);
            }
            Some(b)
        }
    };
    crate::irq::restore(d);
    b
}
