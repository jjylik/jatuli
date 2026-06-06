# jos Phase 1 — Hello World Boot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Boot a hand-rolled Rust kernel under QEMU on emulated AArch64 (`virt`) and print `Hello, World!` to the serial console.

**Architecture:** A freestanding `#![no_std]`/`#![no_main]` Rust binary. An AArch64 assembly stub (`_start`) sets up the stack and branches into `kmain`. `kmain` writes bytes to the PL011 UART (MMIO at `0x0900_0000`) and then parks the CPU. A custom linker script places the image at QEMU's kernel load address `0x4008_0000`. QEMU loads the ELF directly via `-kernel` — no bootloader.

**Tech Stack:** Rust (target `aarch64-unknown-none`, bundled `rust-lld`), AArch64 inline/global asm, QEMU `qemu-system-aarch64`.

---

## Testing approach (read first)

This is bare-metal code — it cannot run on the host, so there are no host unit tests. The
verification harness is an **automated QEMU smoke test** (`test.sh`, built in Task 4): it
boots the kernel, captures serial output, and greps for `Hello, World!`. We follow TDD at
the integration level — the smoke test is written to fail first (no output), then made to
pass once the UART print works.

Until Task 4 exists, intermediate verification is `cargo build` succeeding (the image links)
and a manual QEMU run.

## File structure

```
jos/
├── Cargo.toml              # bin crate "jos", panic="abort"
├── .cargo/config.toml      # target, -Tlinker.ld, qemu runner
├── linker.ld               # load 0x40080000, sections, _stack_top
├── test.sh                 # automated boot-and-grep smoke test
├── README.md               # build & run instructions
└── src/
    ├── main.rs             # no_std/no_main, global_asm!(boot.s), kmain, panic handler
    ├── boot.s              # _start: set sp, bl kmain, wfe-park
    └── uart.rs             # PL011 write_byte + write_str (volatile MMIO)
```

---

### Task 1: Install the bare-metal Rust target

**Files:** none (environment setup).

- [ ] **Step 1: Add the target**

Run:
```bash
rustup target add aarch64-unknown-none
```

- [ ] **Step 2: Verify it is installed**

Run:
```bash
rustup target list --installed | grep aarch64-unknown-none
```
Expected output: `aarch64-unknown-none`

- [ ] **Step 3: Verify QEMU is present**

Run:
```bash
qemu-system-aarch64 --version | head -1
```
Expected: a line like `QEMU emulator version 9.1.0` (any 7.x+ is fine).

No commit (no files changed).

---

### Task 2: Project skeleton that compiles and links to an ELF

This task produces a buildable image whose `kmain` just parks the CPU (no output yet).
The checkpoint is that it **links** — proving the target, linker script, and boot stub
all fit together.

**Files:**
- Create: `Cargo.toml`
- Create: `.cargo/config.toml`
- Create: `linker.ld`
- Create: `src/boot.s`
- Create: `src/main.rs`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "jos"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "jos"
path = "src/main.rs"

[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
```

- [ ] **Step 2: Write `.cargo/config.toml`**

The `runner` makes `cargo run` boot the image; `link-arg` feeds our linker script to `rust-lld`.

```toml
[build]
target = "aarch64-unknown-none"

[target.aarch64-unknown-none]
rustflags = ["-C", "link-arg=-Tlinker.ld"]
runner = "qemu-system-aarch64 -machine virt -cpu cortex-a72 -nographic -kernel"
```

- [ ] **Step 3: Write `linker.ld`**

```ld
ENTRY(_start)

SECTIONS
{
    /* QEMU loads an AArch64 -kernel image here (RAM base is 0x40000000). */
    . = 0x40080000;

    .text : {
        KEEP(*(.text.boot))   /* _start must come first */
        *(.text .text.*)
    }

    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : { *(.bss .bss.*) *(COMMON) }

    /* Reserve a 256 KiB stack; _stack_top is the top (stack grows down). */
    . = ALIGN(16);
    . = . + 0x40000;
    _stack_top = .;
}
```

- [ ] **Step 4: Write `src/boot.s`**

Uses `adrp`/`add` (the canonical PC-relative way to load a symbol address; supported by
LLVM's integrated assembler). Also enables FP/SIMD: AArch64 resets with FP/SIMD access
trapped (`CPACR_EL1.FPEN = 0`), and the Rust compiler may emit NEON instructions, so we
must enable it before calling into Rust or the first NEON instruction faults.

```asm
.section .text.boot
.global _start
_start:
    // AArch64 resets with FP/SIMD access trapped (CPACR_EL1.FPEN = 0b00).
    // The compiler may emit NEON/FP instructions, so enable full FP/SIMD
    // access (FPEN = 0b11) before running any Rust code. ISB so the change
    // takes effect before the next instructions.
    mov     x0, #(3 << 20)
    msr     cpacr_el1, x0
    isb

    // Set up the stack (grows down from _stack_top) and jump to Rust.
    adrp    x9, _stack_top
    add     x9, x9, :lo12:_stack_top
    mov     sp, x9
    bl      kmain
1:  wfe
    b       1b
```

- [ ] **Step 5: Write `src/main.rs`** (minimal — parks, no output yet)

```rust
#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(include_str!("boot.s"));

/// Kernel entry point. Called from `_start` (see `boot.s`) once the stack is set up.
#[no_mangle]
pub extern "C" fn kmain() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
```

- [ ] **Step 6: Build and verify it links**

Run:
```bash
cargo build
```
Expected: `Finished ... target(s)`, with the ELF at `target/aarch64-unknown-none/debug/jos`. No errors.

- [ ] **Step 7: Sanity-check the ELF**

Run:
```bash
file target/aarch64-unknown-none/debug/jos
```
Expected: contains `ELF 64-bit ... ARM aarch64`.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml .cargo/config.toml linker.ld src/boot.s src/main.rs
git commit -m "Add buildable AArch64 kernel skeleton that boots and parks"
```

---

### Task 3: UART driver and print `Hello, World!`

**Files:**
- Create: `src/uart.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write `src/uart.rs`**

```rust
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
```

- [ ] **Step 2: Modify `src/main.rs` to declare the module and print**

Add `mod uart;` after the `use` lines, and replace the body of `kmain` so it prints before parking. The full file becomes:

```rust
#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

mod uart;

global_asm!(include_str!("boot.s"));

/// Kernel entry point. Called from `_start` (see `boot.s`) once the stack is set up.
#[no_mangle]
pub extern "C" fn kmain() -> ! {
    uart::write_str("Hello, World!\n");
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
```

- [ ] **Step 3: Build**

Run:
```bash
cargo build
```
Expected: `Finished` with no errors or warnings.

- [ ] **Step 4: Run manually and observe output**

Run:
```bash
cargo run
```
Expected: the terminal prints `Hello, World!`, then QEMU hangs (CPU parked). Exit QEMU with `Ctrl-A` then `X`.

- [ ] **Step 5: Commit**

```bash
git add src/uart.rs src/main.rs
git commit -m "Add PL011 UART driver and print Hello, World on boot"
```

---

### Task 4: Automated smoke test and README

Adds a repeatable test so success isn't a manual eyeball check, plus run docs.

**Files:**
- Create: `test.sh`
- Create: `README.md`

- [ ] **Step 1: Write `test.sh`**

`timeout` is not on stock macOS, so we background QEMU, capture serial, then kill it.

```bash
#!/usr/bin/env bash
set -euo pipefail

cargo build -q
KERNEL=target/aarch64-unknown-none/debug/jos

OUT="$(mktemp)"
qemu-system-aarch64 -machine virt -cpu cortex-a72 -nographic \
    -kernel "$KERNEL" >"$OUT" 2>&1 &
QPID=$!

sleep 1
kill "$QPID" 2>/dev/null || true
wait "$QPID" 2>/dev/null || true

if grep -q "Hello, World!" "$OUT"; then
    echo "PASS: kernel printed 'Hello, World!'"
    rm -f "$OUT"
    exit 0
else
    echo "FAIL: expected 'Hello, World!' in serial output. Got:"
    cat "$OUT"
    rm -f "$OUT"
    exit 1
fi
```

- [ ] **Step 2: Make it executable**

Run:
```bash
chmod +x test.sh
```

- [ ] **Step 3: Run the smoke test**

Run:
```bash
./test.sh
```
Expected output: `PASS: kernel printed 'Hello, World!'` and exit code 0.

- [ ] **Step 4: Verify it actually fails when output is wrong (red check)**

Temporarily change the grep string to something absent, confirm FAIL, then revert.

Run:
```bash
sed -i '' 's/Hello, World!/NOT_PRESENT_XYZ/' test.sh && ./test.sh; echo "exit=$?"
git checkout test.sh
```
Expected: prints `FAIL:` with the captured output and `exit=1`. After `git checkout`, `test.sh` is restored. (If `test.sh` isn't committed yet, instead re-apply Step 1's content manually.)

- [ ] **Step 5: Write `README.md`**

```markdown
# jos

A hobby AArch64 kernel for learning kernel basics, run under QEMU.

## Prerequisites

- Rust (`rustup target add aarch64-unknown-none`)
- QEMU (`qemu-system-aarch64`)

## Build

    cargo build

## Run

    cargo run

Prints `Hello, World!` to the serial console, then parks the CPU.
Exit QEMU with `Ctrl-A` then `X`.

## Test

    ./test.sh

Boots the kernel under QEMU and checks that it prints `Hello, World!`.

## Layout

- `src/boot.s` — `_start`: sets up the stack, branches to `kmain`.
- `src/main.rs` — `kmain` entry point and panic handler.
- `src/uart.rs` — minimal PL011 UART output driver.
- `linker.ld` — places the image at `0x40080000` and reserves the stack.

See `docs/superpowers/specs/` for design and `docs/superpowers/plans/` for the plan.
```

- [ ] **Step 6: Commit**

```bash
git add test.sh README.md
git commit -m "Add automated QEMU smoke test and README"
```

---

## Done criteria

- `cargo run` prints `Hello, World!` under QEMU.
- `./test.sh` exits 0 with `PASS`.
- All work committed.

This completes Phase 1. Phase 2 (proper PL011 init + UART input) gets its own spec.
