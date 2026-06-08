# jatuli — Phase 13: Event-Driven READ Completion (PL011 RX Interrupt) (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** Complete parked jring `READ`s the moment a byte arrives, from the UART's own
interrupt, instead of polling on the 100 Hz timer tick. Closes the "polled, not
event-driven" gap named in the Phase 12 spec: after this, I/O completion is driven by
the device, and the timer is once again purely a scheduler concern.

## Reuses

- jring pending table + `poll_pending()` (Phase 12) — unchanged logic, new trigger.
- GICv3 bring-up (Phase 8); this phase adds the *distributor*-side enable path.
- `uart::try_getc` (Phase 11) — reading `UARTDR` also clears the RX condition.

## The design problem: IRQ storms

The PL011 RX interrupt is **level**-asserted while unread data sits in the receiver.
`test.sh` pipes its whole script at boot, before jsh parks any `READ`: an
unconditionally-enabled RX interrupt would fire, find no parked read to complete,
return without reading `UARTDR` — and re-fire immediately. Livelock.

Resolution: **interrupt-on-demand masking.** `UARTIMSC.RXIM` is set only while parked
reads exist; unconsumed input simply waits in the UART (QEMU's chardev applies
backpressure, as established in Phase 11). The mask is the flow control — the same
pattern as Linux's NAPI (disable the interrupt while there is nothing ready to consume
its data). The alternative — a kernel-side input ring buffer drained unconditionally —
is the "real tty driver" shape and lands next phase, where waitqueues give the buffer
consumers that can sleep.

## `uart.rs`

```rust
pub const UART_INTID: u32 = 33;          // SPI 1 on the virt machine
pub fn set_rx_irq(enabled: bool)         // write UARTIMSC (0x38) bit 4 (RXIM)
```

No FIFO-mode change; in the reset (1-byte) mode the RX interrupt asserts while the
holding register is full and clears when `UARTDR` is read.

## `gic.rs`

```rust
pub fn enable_spi(intid: u32)            // distributor-side enable, route to CPU 0
```

PPIs/SGIs (INTID < 32) are per-core and live in the redistributor SGI frame (the
Phase 8 path); SPIs are shared bus interrupts configured in the **distributor**:
`GICD_IGROUPR` (group 1), `GICD_IPRIORITYR` (priority 0), `GICD_IROUTER`
(affinity-route to CPU 0; offset `0x6100 + 8 * (intid - 32)`), `GICD_ISENABLER`.

## `ring.rs`

- Parking a `READ` (pending table insert) → `uart::set_rx_irq(true)`.
- `poll_pending()`: after the sweep, if the pending table is empty →
  `uart::set_rx_irq(false)`.
- Both run with IRQs masked (syscall or IRQ context), so mask state never races.

## `exceptions.rs`

`handle_irq` gains an arm: `uart::UART_INTID` → `ring::poll_pending()` → EOI.
The `poll_pending()` call in the **timer arm is deleted** — the structural proof that
I/O completion no longer depends on the tick.

## `main.rs`

`gic::enable_spi(uart::UART_INTID)` after `gic::init(...)`. No new self-check: with the
timer poll gone, the only way the existing piped shell session can work is via the RX
interrupt — the whole existing test *is* the check.

## Demo / verification

`test.sh` unchanged. Typing latency drops from ≤ 10 ms (tick) to effectively immediate;
the observable proof is structural (timer poll removed, shell still works).

## Files

| File | Change |
|---|---|
| `kernel/src/uart.rs` | `UART_INTID`, `set_rx_irq`. |
| `kernel/src/gic.rs` | `enable_spi` (distributor path). |
| `kernel/src/ring.rs` | unmask on park, mask when pending table empties. |
| `kernel/src/exceptions.rs` | UART IRQ arm; timer arm no longer polls the ring. |
| `kernel/src/main.rs` | enable the UART SPI at boot. |
| `README.md` | layout-line updates. |

## Out of scope (named later phases)

Kernel input buffer + real `copy_to_user` (tty-driver shape) · waitqueues / blocking
`RING_ENTER(min_complete)` · SQPOLL · shell as a scheduler task.
