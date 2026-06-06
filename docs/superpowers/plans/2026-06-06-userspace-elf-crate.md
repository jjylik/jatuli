# Separately-Compiled EL0 Userspace Crate + ELF Loader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-written in-kernel `user.s` EL0 blob with a real, separately-compiled userspace crate that the kernel loads via a minimal ELF loader; the program prints "Hello, world" from EL0 and exits cleanly via a new `SYS_EXIT`.

**Architecture:** Convert the single-crate repo into a Cargo workspace (`kernel/` + `user/`). The kernel's `build.rs` compiles the `user` crate into an isolated target dir and embeds its ELF with `include_bytes!`. A new `kernel/src/elf.rs` validates the ELF header and maps each `PT_LOAD` segment at its `p_vaddr` with EL0 permissions (populating frames via their identity-mapped physical address, then doing cache maintenance, because RX pages are read-only at EL1 and AArch64 I/D caches are not coherent). `enter_user()` loads the embedded ELF and `ERET`s to its entry; `SYS_EXIT` returns control to an EL1 idle loop.

**Tech Stack:** Rust `no_std`/`no_main`, AArch64 (`aarch64-unknown-none`), QEMU `virt`, ld.lld linker scripts, Cargo workspace + build scripts.

**Spec:** `docs/superpowers/specs/2026-06-06-userspace-elf-crate-design.md`

**Testing model:** This is a bare-metal `no_std` kernel with no unit-test harness; the project tests by booting under QEMU and grepping serial-output markers (`test.sh`). Each task's "test" is therefore a boot marker observed via `./test.sh` or `cargo run`. "Verify it fails" means the marker is absent (or the old behavior still shows) before the change; "verify it passes" means the new marker appears.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` (root) | Workspace manifest; `default-members = ["kernel"]`; shared `panic = "abort"` profiles. |
| `kernel/Cargo.toml` | The kernel package (`name = "jos"`), formerly the root manifest. |
| `kernel/build.rs` | Emit the kernel linker script `-T` arg; build the `user` crate (isolated target dir) and expose its ELF via `USER_ELF`. |
| `kernel/linker.ld` | Kernel image layout (moved from root; `.user_text` section removed in Task 5). |
| `kernel/src/elf.rs` | Embed the user ELF; validate the header; map `PT_LOAD` segments; record loaded ranges. |
| `kernel/src/user.rs` | Load the embedded ELF, map a user stack, `ERET` to EL0; validate user pointers against loaded ranges. |
| `kernel/src/mmu.rs` | Add `sync_instruction_cache` (clean D-cache / invalidate I-cache for freshly-written code). |
| `kernel/src/syscall.rs` | Add `SYS_EXIT` and its dispatch arm. |
| `user/Cargo.toml` | The EL0 program crate. |
| `user/build.rs` | Emit the user linker script `-T` arg and `max-page-size=4096`. |
| `user/user.ld` | EL0 VA layout; explicit `PHDRS` for separate R-X / R-W `PT_LOAD` segments. |
| `user/src/main.rs` | `_start`, `svc` syscall stubs, hello + exit, panic handler. |
| `.cargo/config.toml` | Drop the global `-Tlinker.ld` rustflag (now per-crate); keep `target` + `runner`. |
| `test.sh` | Updated serial-output markers. |
| `README.md` | Updated layout section. |

---

## Task 1: Restructure into a Cargo workspace

Move the kernel into `kernel/` and make the root a workspace, with the kernel linker script selected per-crate via `build.rs` instead of a global rustflag. Behavior is unchanged — the old `user.s` blob still runs — so the existing `test.sh` markers must all still pass.

**Files:**
- Create: `kernel/Cargo.toml`, `kernel/build.rs`
- Move: `src/` → `kernel/src/`, `linker.ld` → `kernel/linker.ld`
- Modify: `Cargo.toml` (root), `.cargo/config.toml`

- [ ] **Step 1: Move the kernel sources and linker script (preserve git history)**

```bash
git mv src kernel/src
git mv linker.ld kernel/linker.ld
mkdir -p kernel   # already exists after the moves; no-op safeguard
```

Expected: `kernel/src/main.rs`, `kernel/linker.ld` exist; top-level `src/` and `linker.ld` are gone.

- [ ] **Step 2: Create `kernel/Cargo.toml`**

```toml
[package]
name = "jos"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "jos"
path = "src/main.rs"
```

(Profiles are intentionally NOT here — Cargo only honors `[profile.*]` in the workspace root manifest.)

- [ ] **Step 3: Rewrite the root `Cargo.toml` as a workspace**

```toml
[workspace]
resolver = "2"
members = ["kernel"]
default-members = ["kernel"]

[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
```

(`user` is added to `members` in Task 2.)

- [ ] **Step 4: Create `kernel/build.rs` to select the linker script with an absolute path**

```rust
use std::path::PathBuf;

fn main() {
    // Absolute path so the linker resolves it regardless of its working directory.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ld = manifest.join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rerun-if-changed={}", ld.display());
}
```

- [ ] **Step 5: Drop the global linker rustflag from `.cargo/config.toml`**

Remove the `[target.aarch64-unknown-none] rustflags = [...]` linker line; keep `build.target` and the `runner`. The file becomes:

```toml
[build]
target = "aarch64-unknown-none"

[target.aarch64-unknown-none]
runner = "qemu-system-aarch64 -machine virt,gic-version=3 -cpu cortex-a72 -nographic -kernel"
```

- [ ] **Step 6: Verify the restructure builds and behaves identically**

Run: `./test.sh`
Expected: `PASS: boot + heap self-check` (all existing markers, including `Hello from EL0!` and `rejected out-of-range user pointer`, still present — the old `user.s` is untouched).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Restructure kernel into a Cargo workspace member"
```

---

## Task 2: Add the `user` crate (hello + exit), built standalone

Create the userspace program as its own crate with its own entry point and linker layout. It is not yet referenced by the kernel; this task only proves it compiles to a valid AArch64 ELF executable.

**Files:**
- Create: `user/Cargo.toml`, `user/build.rs`, `user/user.ld`, `user/src/main.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `user` to the workspace members**

In the root `Cargo.toml`, change `members = ["kernel"]` to:

```toml
members = ["kernel", "user"]
```

(Leave `default-members = ["kernel"]` so plain `cargo build` still builds only the kernel.)

- [ ] **Step 2: Create `user/Cargo.toml`**

```toml
[package]
name = "user"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "user"
path = "src/main.rs"
```

- [ ] **Step 3: Create `user/user.ld`**

```text
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5);   /* R + X */
    data PT_LOAD FLAGS(6);   /* R + W */
}

SECTIONS
{
    /* User VA region, matching the kernel's user window (Phase 9). */
    . = 0x200000000;

    .text   : { *(.text .text.*) }              :text
    .rodata : { *(.rodata .rodata.*) }           :text

    /* Page-align so data/bss start a fresh page -> its own R+W PT_LOAD. */
    . = ALIGN(0x1000);

    .data   : { *(.data .data.*) }               :data
    .bss    : { *(.bss .bss.*) *(COMMON) }       :data
}
```

- [ ] **Step 4: Create `user/build.rs`**

```rust
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ld = manifest.join("user.ld");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    // Keep segments 4 KiB-granular (not the 64 KiB AArch64 default).
    println!("cargo:rustc-link-arg=-z");
    println!("cargo:rustc-link-arg=max-page-size=4096");
    println!("cargo:rerun-if-changed={}", ld.display());
}
```

- [ ] **Step 5: Create `user/src/main.rs`**

```rust
#![no_std]
#![no_main]

use core::arch::asm;

// Syscall numbers — must match the kernel's `syscall.rs` ABI.
const SYS_PRINT: u64 = 2;
const SYS_EXIT: u64 = 3;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello, world from EL0!\n";
    // SAFETY: `msg` is a live byte slice in our mapped .rodata; SYS_PRINT reads (ptr, len).
    unsafe {
        sys_print(msg.as_ptr(), msg.len());
        sys_exit(0);
    }
}

/// SYS_PRINT(ptr, len): ask the kernel to print a UTF-8 string.
///
/// # Safety
/// `ptr`/`len` must describe a readable byte range in this program's memory.
unsafe fn sys_print(ptr: *const u8, len: usize) {
    asm!(
        "svc #0",
        in("x8") SYS_PRINT,
        in("x0") ptr as u64,
        in("x1") len as u64,
        options(nostack),
    );
}

/// SYS_EXIT(code): terminate the program; never returns.
///
/// # Safety
/// Always sound to call; the kernel never returns control to EL0 afterward.
unsafe fn sys_exit(code: i32) -> ! {
    asm!(
        "svc #0",
        in("x8") SYS_EXIT,
        in("x0") code as u64,
        options(nostack, noreturn),
    );
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // SAFETY: SYS_EXIT never returns.
    unsafe { sys_exit(1) }
}
```

- [ ] **Step 6: Build the user crate and verify it is an AArch64 ELF executable**

Run: `cargo build -p user`
Then: `file target/aarch64-unknown-none/debug/user`
Expected: output mentions `ELF 64-bit LSB executable, ARM aarch64`.

Troubleshooting: if `file` reports a *shared object* (`ET_DYN`) instead of *executable* (`ET_EXEC`), add `println!("cargo:rustc-link-arg=-no-pie");` to `user/build.rs` and rebuild — the ELF loader requires `ET_EXEC`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "Add userspace crate: EL0 hello-world program with syscall stubs"
```

---

## Task 3: Build and embed the user ELF; validate its header at boot

Wire the kernel's `build.rs` to compile the `user` crate into an isolated target dir and embed the resulting ELF. Add `elf.rs` with header validation and a boot self-check that proves the embedded image is a well-formed AArch64 executable. The old `user.s` still runs (cutover is Task 5).

**Files:**
- Modify: `kernel/build.rs`
- Create: `kernel/src/elf.rs`
- Modify: `kernel/src/main.rs`, `test.sh`

- [ ] **Step 1: Extend `kernel/build.rs` to build and locate the user ELF**

Replace `kernel/build.rs` with:

```rust
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // Kernel linker script (absolute path).
    let ld = manifest.join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rerun-if-changed={}", ld.display());

    // Build the userspace crate into an ISOLATED target dir so this nested
    // `cargo` invocation does not deadlock on the parent build's locked target dir.
    let workspace = manifest.parent().unwrap().to_path_buf();
    let user_dir = workspace.join("user");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let user_target = out_dir.join("user-target");
    let profile = std::env::var("PROFILE").unwrap(); // "debug" or "release"

    let mut cmd = Command::new(std::env::var("CARGO").unwrap());
    cmd.current_dir(&workspace)
        .arg("build")
        .arg("-p")
        .arg("user")
        .arg("--target")
        .arg("aarch64-unknown-none")
        .env("CARGO_TARGET_DIR", &user_target);
    if profile == "release" {
        cmd.arg("--release");
    }
    let status = cmd.status().expect("failed to spawn cargo for the user crate");
    assert!(status.success(), "building the user crate failed");

    let elf = user_target
        .join("aarch64-unknown-none")
        .join(&profile)
        .join("user");
    println!("cargo:rustc-env=USER_ELF={}", elf.display());

    println!("cargo:rerun-if-changed={}", user_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("user.ld").display());
    println!("cargo:rerun-if-changed={}", user_dir.join("build.rs").display());
}
```

- [ ] **Step 2: Create `kernel/src/elf.rs` with the embedded image + header validation**

```rust
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
```

(`read_u32`, `E_PHOFF`, `E_PHENTSIZE`, `E_PHNUM` are unused until Task 4. Add `#[allow(dead_code)]` on each if the build warns, or accept the warnings — they're resolved in Task 4.)

- [ ] **Step 3: Register the module and add a boot self-check in `kernel/src/main.rs`**

Add the module declaration next to the others (after `mod allocator;`):

```rust
mod elf;
```

Add this function (place it near the other `*_self_check` functions):

```rust
/// Validate the embedded userspace ELF header before we try to run it.
fn elf_self_check() {
    let entry = elf::validate(elf::USER_ELF);
    assert!(entry >= 0x2_0000_0000, "user entry VA not in the user window");
    kprintln!("user elf: {} bytes, entry {:#x}", elf::USER_ELF.len(), entry);
    uart::write_str("elf self-check passed\n");
}
```

Call it in `kmain`, immediately before the "entering user mode" line:

```rust
    sched_self_check();

    elf_self_check();

    uart::write_str("entering user mode (EL0)...\n");
    user::enter_user();
```

- [ ] **Step 4: Add the new marker to `test.sh`**

In the `for needle in ...` list, add `"elf self-check passed"` (the old `user.s` markers stay for now). Insert it just before `"entering user mode (EL0)"`:

```bash
... "irq self-check passed" "elf self-check passed" "entering user mode (EL0)" "Hello from EL0!" ...
```

- [ ] **Step 5: Verify the embedded ELF validates at boot**

Run: `./test.sh`
Expected: `PASS: boot + heap self-check`; the serial output now includes `elf self-check passed` and a `user elf: <N> bytes, entry 0x2...` line, with all prior markers still present.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "Build and embed the userspace ELF; validate its header at boot"
```

---

## Task 4: Map `PT_LOAD` segments (the ELF loader core)

Add `elf::load`, which maps each `PT_LOAD` segment at its `p_vaddr` with EL0 permissions. It populates each frame through its identity-mapped physical address (writable at EL1) before installing the user mapping, then performs cache maintenance for executable pages. A temporary self-check exercises it (removed at cutover in Task 5).

**Files:**
- Modify: `kernel/src/mmu.rs`, `kernel/src/elf.rs`, `kernel/src/main.rs`

- [ ] **Step 1: Add instruction-cache maintenance to `kernel/src/mmu.rs`**

Append:

```rust
/// Make freshly-written code at the identity-mapped physical range `[pa, pa+len)`
/// visible to instruction fetch: clean it from the D-cache to the point of
/// unification and invalidate the I-cache. AArch64 I/D caches are not coherent,
/// so writing instructions then executing them requires this. (QEMU's TCG would
/// not catch its absence, but real hardware would.)
pub fn sync_instruction_cache(pa: usize, len: usize) {
    const LINE: usize = 64; // Cortex-A72 cache-line size.
    let start = pa & !(LINE - 1);
    let end = pa + len;

    // SAFETY: cache maintenance over identity-mapped Normal memory we just wrote.
    unsafe {
        let mut p = start;
        while p < end {
            asm!("dc cvau, {0}", in(reg) p, options(nostack, preserves_flags));
            p += LINE;
        }
        asm!("dsb ish", options(nostack, preserves_flags));
        let mut q = start;
        while q < end {
            asm!("ic ivau, {0}", in(reg) q, options(nostack, preserves_flags));
            q += LINE;
        }
        asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}
```

- [ ] **Step 2: Add the loader to `kernel/src/elf.rs`**

Add imports at the top of the file:

```rust
use crate::frames::{alloc_frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW, PAGE_USER_RX};
```

Append the loader:

```rust
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
/// point and the mapped ranges. Panics on anything malformed or unsupported.
pub fn load(image: &[u8]) -> Loaded {
    let entry = validate(image);
    let phoff = read_u64(image, E_PHOFF) as usize;
    let phentsize = read_u16(image, E_PHENTSIZE) as usize;
    let phnum = read_u16(image, E_PHNUM) as usize;

    let mut ranges = [Range { start: 0, end: 0 }; MAX_SEGMENTS];
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

            mmu::map_page(vaddr + page, frame.addr(), perms);
            page += FRAME_SIZE;
        }

        assert!(count < MAX_SEGMENTS, "too many PT_LOAD segments");
        ranges[count] = Range {
            start: vaddr,
            end: vaddr + memsz,
        };
        count += 1;
    }

    Loaded {
        entry,
        ranges,
        count,
    }
}
```

- [ ] **Step 3: Add a temporary load self-check in `kernel/src/main.rs`**

Replace the body of `elf_self_check` with a version that also loads and reads back the first instruction word at the entry VA (RX pages are EL1-readable):

```rust
/// Validate and load the embedded userspace ELF, then read back its entry word.
/// (The load self-check is temporary; Task 5 moves loading into `enter_user`.)
fn elf_self_check() {
    let loaded = elf::load(elf::USER_ELF);
    assert!(loaded.count >= 1, "no PT_LOAD segments mapped");
    assert!(loaded.entry >= 0x2_0000_0000, "user entry VA not in the user window");

    // SAFETY: the entry VA was just mapped EL0-RX, which is readable at EL1.
    let first = unsafe { (loaded.entry as *const u32).read_volatile() };
    assert_ne!(first, 0, "entry instruction is zero — segment not populated");

    kprintln!(
        "user elf: {} segs, entry {:#x}, first insn {:#010x}",
        loaded.count,
        loaded.entry,
        first
    );
    uart::write_str("elf load self-check passed\n");
}
```

- [ ] **Step 4: Point `test.sh` at the new marker**

In `test.sh`, change `"elf self-check passed"` to `"elf load self-check passed"`.

- [ ] **Step 5: Verify segments map and the entry word reads back**

Run: `./test.sh`
Expected: `PASS: boot + heap self-check`; serial output includes `elf load self-check passed` and a `user elf: <N> segs, entry 0x2..., first insn 0x...` line (nonzero instruction). The old `user.s` still runs afterward (its `Hello from EL0!` / `rejected ...` markers still pass).

Troubleshooting: a hang or fault here usually means a missing cache maintenance step (Step 1) or a non-page-aligned `p_vaddr` (revisit `user.ld` / `max-page-size`).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "Add ELF PT_LOAD segment loader with EL0 mapping and cache maintenance"
```

---

## Task 5: Cut over to the loaded ELF; add `SYS_EXIT`; delete `user.s`

Make `enter_user()` run the loaded ELF, validate user pointers against the recorded ranges, add the `SYS_EXIT` syscall, and delete the obsolete `user.s` blob and its linker section. After this the EL0 program is the separately-compiled crate, which prints hello and exits cleanly.

**Files:**
- Modify: `kernel/src/syscall.rs`, `kernel/src/user.rs`, `kernel/src/main.rs`, `kernel/linker.ld`, `test.sh`, `README.md`
- Delete: `kernel/src/user.s`

- [ ] **Step 1: Add `SYS_EXIT` to `kernel/src/syscall.rs`**

Add the constant beside the others:

```rust
/// Terminate the user program: `x0` = exit code. Does not return to EL0.
pub const SYS_EXIT: u64 = 3;
```

Add a dispatch arm. Because exit must not resume EL0, the arm diverges into an EL1 idle loop instead of returning a value:

```rust
pub fn dispatch(frame: &mut TrapFrame, from_user: bool) {
    let ret = match frame.x[8] {
        SYS_ADD => frame.x[0].wrapping_add(frame.x[1]),
        SYS_PRINT => sys_print(frame.x[0], frame.x[1], from_user),
        SYS_EXIT => {
            kprintln!("[user] exited with code {}", frame.x[0] as i64);
            // The process is done; control stays at EL1. Park the CPU here —
            // we never ERET back to EL0. (Full teardown is a later phase.)
            loop {
                // SAFETY: idle until an interrupt; nothing left to run.
                unsafe { core::arch::asm!("wfi", options(nomem, nostack, preserves_flags)) };
            }
        }
        other => {
            kprintln!("unknown syscall {}", other);
            u64::MAX
        }
    };
    frame.x[0] = ret;
}
```

- [ ] **Step 2: Rewrite `kernel/src/user.rs` to load the ELF and validate against its ranges**

Replace the entire file with:

```rust
//! Drop to EL0 by loading the embedded userspace ELF.
//!
//! Loads every `PT_LOAD` segment (see `elf.rs`), maps a user stack, and `ERET`s
//! to the program's entry point. After that the kernel is re-entered only via
//! syscalls or interrupts. User-supplied syscall pointers are validated against
//! the loaded segment ranges plus the stack.

use core::arch::asm;

use crate::elf;
use crate::frames::{alloc_frame, FRAME_SIZE};
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;

/// Virtual base of the user stack (above the program's segments at 0x2_0000_0000).
pub const USER_STACK_VA: usize = 0x2_0010_0000;
/// User stack size (one page).
const USER_STACK_SIZE: usize = FRAME_SIZE;

/// The loaded image's mapped ranges, recorded for pointer validation.
static LOADED: Locked<Option<elf::Loaded>> = Locked::new(None);

/// Whether `[ptr, ptr + len)` lies entirely within a mapped user segment or the
/// user stack. Used to validate syscall pointers from EL0.
pub fn is_user_range(ptr: usize, len: usize) -> bool {
    let end = match ptr.checked_add(len) {
        Some(e) => e,
        None => return false,
    };
    if ptr >= USER_STACK_VA && end <= USER_STACK_VA + USER_STACK_SIZE {
        return true;
    }
    let guard = LOADED.lock();
    if let Some(loaded) = guard.as_ref() {
        for r in &loaded.ranges[..loaded.count] {
            if ptr >= r.start && end <= r.end {
                return true;
            }
        }
    }
    false
}

/// Load the embedded user ELF, map a user stack, and drop to EL0 at its entry.
/// Does not return: the kernel runs only via syscalls/IRQs afterward.
pub fn enter_user() -> ! {
    let loaded = elf::load(elf::USER_ELF);
    let entry = loaded.entry;
    *LOADED.lock() = Some(loaded);

    // Map a user stack page as EL0 read/write.
    let stack = alloc_frame().expect("out of frames for the user stack");
    mmu::map_page(USER_STACK_VA, stack.addr(), PAGE_USER_RW);
    let user_sp = USER_STACK_VA + USER_STACK_SIZE;

    // SAFETY: segments + stack are mapped EL0-accessible; SPSR selects EL0t with
    // interrupts enabled; ERET transfers to unprivileged execution at `entry`.
    unsafe {
        asm!(
            "msr spsr_el1, {spsr}",
            "msr elr_el1, {entry}",
            "msr sp_el0, {sp}",
            "eret",
            spsr = in(reg) 0u64,
            entry = in(reg) entry,
            sp = in(reg) user_sp,
            options(noreturn),
        );
    }
}
```

- [ ] **Step 3: Restore `elf_self_check` to header-only validation in `kernel/src/main.rs`**

The load now happens in `enter_user`, so the self-check must not also load (that would map the segments twice). Replace `elf_self_check` with:

```rust
/// Validate the embedded userspace ELF header before we try to run it.
fn elf_self_check() {
    let entry = elf::validate(elf::USER_ELF);
    assert!(entry >= 0x2_0000_0000, "user entry VA not in the user window");
    kprintln!("user elf: {} bytes, entry {:#x}", elf::USER_ELF.len(), entry);
    uart::write_str("elf self-check passed\n");
}
```

- [ ] **Step 4: Delete the obsolete assembly user program**

```bash
git rm kernel/src/user.s
```

- [ ] **Step 5: Remove the `.user_text` section from `kernel/linker.ld`**

Delete these lines (the EL0 code is no longer baked into the kernel image):

```text
    /* EL0 user code, page-aligned and isolated so it can be mapped EL0-RX. */
    . = ALIGN(0x1000);
    __user_start = .;
    .user_text : { KEEP(*(.user_text)) }
    . = ALIGN(0x1000);
    __user_end = .;
```

- [ ] **Step 6: Update `test.sh` markers for the new behavior**

In the `for needle in ...` list:
- Change `"elf load self-check passed"` back to `"elf self-check passed"`.
- Remove `"Hello from EL0!"` and `"rejected out-of-range user pointer"`.
- After `"entering user mode (EL0)"`, add `"Hello, world from EL0!"` and `"[user] exited with code 0"`.

The relevant span of the list should read:

```bash
... "irq self-check passed" "elf self-check passed" "entering user mode (EL0)" "Hello, world from EL0!" "[user] exited with code 0" "[sleeper] woke 3" ...
```

- [ ] **Step 7: Verify the full new behavior end-to-end**

Run: `./test.sh`
Expected: `PASS: boot + heap self-check`. Serial output shows `elf self-check passed`, `entering user mode (EL0)...`, `Hello, world from EL0!`, then `[user] exited with code 0`. The old `Hello from EL0!` / `rejected out-of-range user pointer` lines are gone.

- [ ] **Step 8: Update the `README.md` layout section**

Replace the `## Layout` bullets to reflect the workspace. Replace the existing `- src/...` lines with kernel-prefixed paths and add the new entries:

```markdown
## Layout

- `kernel/` — the kernel crate (`jos`).
  - `kernel/src/boot.s` — `_start`: enables FP/SIMD, sets up the stack, branches to `kmain`.
  - `kernel/src/main.rs` — `kmain` entry point, self-checks, panic handler.
  - `kernel/src/uart.rs` — PL011 UART driver + `core::fmt::Write` (`kprint!`/`kprintln!`).
  - `kernel/src/mem.rs` — freestanding `memcpy`/`memset`/`memmove`/`memcmp`.
  - `kernel/src/sync.rs` — `Locked<A>` spinlock.
  - `kernel/src/allocator.rs` — bump heap over a frame-backed virtual window.
  - `kernel/src/frames.rs` — physical 4 KiB frame allocator (intrusive free-list).
  - `kernel/src/mmu.rs` — page tables, MMU enable, `map_page`, cache maintenance.
  - `kernel/src/exceptions.rs` / `kernel/src/exceptions.s` — vector table, trap frame, dispatch.
  - `kernel/src/syscall.rs` — `SVC` syscall dispatch (Linux-like ABI), incl. `SYS_EXIT`.
  - `kernel/src/gic.rs` — GICv3 interrupt controller.
  - `kernel/src/timer.rs` — generic timer (periodic interrupt).
  - `kernel/src/sched.rs` / `kernel/src/switch.s` — cooperative + preemptive scheduler.
  - `kernel/src/elf.rs` — minimal ELF64 loader for the embedded user image.
  - `kernel/src/user.rs` — load the user ELF, map a stack, drop to EL0; pointer validation.
  - `kernel/build.rs` — builds the `user` crate and embeds its ELF.
  - `kernel/linker.ld` — kernel image at `0x40080000`, stack, `_kernel_end`.
- `user/` — the EL0 userspace program crate (separately compiled).
  - `user/src/main.rs` — `_start`, `svc` syscall stubs, hello + exit.
  - `user/user.ld` — EL0 VA layout with separate R-X / R-W segments.
```

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "Run separately-compiled EL0 program via ELF loader; add SYS_EXIT"
```

---

## Self-Review notes (addressed)

- **Spec coverage:** workspace layout (Task 1), build.rs orchestration with isolated target dir (Task 3), user crate hello+exit (Task 2), `user.ld` PHDRS split (Task 2), ELF loader mapping PT_LOAD with per-segment perms + BSS zero-fill (Task 4), `enter_user`/`is_user_range` cutover (Task 5), `SYS_EXIT` print+idle (Task 5), `user.s`/`.user_text` deletion (Task 5), `test.sh` + `README` updates (Tasks 3–5). All spec sections map to a task.
- **Correctness additions beyond the spec prose:** RX pages are EL1-read-only, so frames are populated via their identity PA before mapping (Task 4); AArch64 I/D caches are incoherent, so `sync_instruction_cache` runs for executable segments (Task 4). Both are required for the loader to actually work, not optional.
- **Type consistency:** `elf::Loaded { entry, ranges, count }`, `elf::Range { start, end }`, `elf::validate`, `elf::load`, `mmu::sync_instruction_cache`, and `user::is_user_range` are used consistently across Tasks 4–5. Syscall numbers (`SYS_PRINT = 2`, `SYS_EXIT = 3`) match between `user/src/main.rs` and `kernel/src/syscall.rs`.
```
