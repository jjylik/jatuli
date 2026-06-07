# jos — Phase 16: Kernel Input Buffer + `copy_to_user` (Design)

**Date:** 2026-06-07
**Status:** Approved design
**Goal:** Give console input the real driver shape: the UART interrupt unconditionally
drains the device into a kernel-only byte buffer (type-ahead works; on real hardware
this is what prevents FIFO overrun data loss), and `READ` completions become a genuine
kernel-memory → user-memory copy through a named, validated `copy_to_user`. The jring
layer stops knowing the UART exists.

## Reuses

- The jring pending/wake machinery (Phases 12–14) — only its byte *source* changes.
- The RX interrupt path (Phase 13) — the on-demand masking is retired upward: it
  becomes buffer-full flow control instead of works-around-having-no-buffer.
- `is_user_range_writable` (Phase 11) — now also enforced inside `copy_to_user`.

## Why a kernel buffer (recap from design discussion)

The buffer decouples data arrival from user requests in both directions:
- **Data before request**: keystrokes are captured even when no `READ` is parked.
  (Until now QEMU's chardev backpressure hid this gap; real hardware would drop bytes.)
- **Request before data**: parked `READ`s complete on arrival, as before — but the
  completing copy now moves bytes from kernel memory, which is the step that needs a
  named gate (`copy_to_user`) in any real OS.

## `kernel/src/input.rs` (new)

A fixed ring of `INPUT_BUF_SIZE = 256` raw bytes in kernel `.bss` (EL0-inaccessible by
the identity map's AP bits — "kernel-only" is enforced by the same permission system
the user pages opt out of). API:

```rust
pub fn push(b: u8) -> bool;     // from IRQ context; false if full
pub fn pop() -> Option<u8>;     // from syscall/poller context
```

`Locked` + IRQs-off discipline, same as the jring state.

**Flow control:** when `push` finds the buffer full, the caller masks the RX interrupt
(`uart::set_rx_irq(false)`) — further input waits host-side in QEMU's chardev
backpressure. `pop` unmasks again once space frees. The Phase 13 masking pattern,
moved down a layer and made honest.

## `copy_to_user` (`kernel/src/user.rs`)

```rust
pub fn copy_to_user(dst: usize, src: &[u8]) -> bool
```

Validates `is_user_range_writable(dst, src.len())`, then performs the volatile store
loop through the user VA. False (no partial write) on validation failure. Doc comment
carries the real-world framing: this is the single auditable gate where the kernel
writes user memory — the function where hardened kernels toggle PAN or use `sttr`.
SQE-accept-time validation stays as well; re-checking here is cheap and is the
correct (Linux `access_ok` + copy) shape.

## Rewiring

- **UART IRQ handler** (`exceptions.rs` arm): `input::drain_uart()` — move bytes
  device → kernel buffer — then `ring::poll_pending()` completes parked `READ`s *from
  the buffer* and wakes the waiter.
- **`ring.rs`**: `drain_uart(buf, len)` becomes `drain_input(buf, len)`: pop bytes
  from `input`, deliver via `copy_to_user` (kernel-trusted completions for the EL1
  self-check path keep a direct store). The ring layer no longer touches the UART:
  driver (input.rs) vs I/O interface (ring.rs) layering.
- **Boot** (`main.rs`): `uart::set_rx_irq(true)` once, unconditionally, after the GIC
  SPI enable. The park-time unmask / empty-table mask in `ring.rs` is deleted.

## Demo / verification

`test.sh` unchanged — and strengthened for free: the piped 19-byte script arrives
during boot, before jsh runs, so every run now exercises type-ahead through the kernel
buffer end-to-end. The buffer (256 B) comfortably exceeds the script.

## Files

| File | Change |
|---|---|
| `kernel/src/input.rs` (new) | byte ring, `push`/`pop`/`drain_uart`, full/space flow control. |
| `kernel/src/user.rs` | `copy_to_user`. |
| `kernel/src/ring.rs` | consume from `input`, deliver via `copy_to_user`; masking removed. |
| `kernel/src/exceptions.rs` | UART arm: drain to buffer first. |
| `kernel/src/main.rs` | `mod input;` + always-on RX IRQ. |
| `README.md` | layout updates. |

## Out of scope (named later phases)

Line discipline (canonical mode, echo, ^C) · PAN/`sttr` hardened uaccess ·
`copy_from_user` (no write-style ops consume user data buffers yet) · multiple input
consumers · resizable buffers.
