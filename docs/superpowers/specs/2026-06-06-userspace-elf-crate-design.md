# jos — Phase 10: Separately-Compiled EL0 Userspace Crate + ELF Loader (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Replace the in-kernel hand-written `user.s` blob with a real, separately-compiled
userspace program — its own Rust crate, its own entry point and linker layout — that the
kernel loads via a minimal ELF loader. Mirrors how a real OS treats userspace: a wholly
separate compilation unit the kernel only ever sees as a built ELF. The program prints
"Hello, world" from EL0 and exits cleanly via a new `SYS_EXIT`.

## Reuses

- `enter_user()` / `ERET` to EL0 and the trap-frame syscall path (Phase 9).
- `map_page` + `PAGE_USER_RX` / `PAGE_USER_RW` (Phases 5, 9) — the loader maps each segment
  with these flags per `p_flags`.
- `is_user_range` pointer validation (Phase 9) — still gates `SYS_PRINT`, now backed by the
  loader-recorded segment ranges instead of a fixed window.

## Repo layout — Cargo workspace

```
jos/
├── Cargo.toml          # [workspace], default-members = ["kernel"]
├── kernel/             # everything in src/ today moves here
│   ├── Cargo.toml
│   ├── build.rs        # builds the user crate, exposes its ELF path
│   ├── linker.ld       # kernel image @ 0x40080000 (unchanged)
│   └── src/            # gains elf.rs; user.s deleted, user.rs slimmed
└── user/
    ├── Cargo.toml
    ├── build.rs        # emits -Tuser.ld
    ├── user.ld         # EL0 VA layout, separate RX / RW PT_LOAD segments
    └── src/main.rs     # no_std/no_main: _start, syscall stubs, hello + exit
```

The global `-Tlinker.ld` rustflag in `.cargo/config.toml` is **removed**; each crate emits its
own linker script via `build.rs` (`cargo:rustc-link-arg=-T...`). The shared
`target = "aarch64-unknown-none"` and the QEMU `runner` stay.

## Build flow (`kernel/build.rs`)

The kernel `build.rs` compiles the user crate and hands its ELF to `include_bytes!`:

```text
cargo build -p user            # with CARGO_TARGET_DIR = <OUT_DIR>/user-target
                               #   (isolated dir avoids the workspace target-dir lock)
cargo:rerun-if-changed=../user/src
cargo:rerun-if-changed=../user/Cargo.toml
cargo:rerun-if-changed=../user/user.ld
cargo:rustc-env=USER_ELF=<OUT_DIR>/user-target/aarch64-unknown-none/<profile>/user
```

`kernel/src` then does `include_bytes!(env!("USER_ELF"))`. `cargo run` and `test.sh` keep
working as a single command. The isolated `CARGO_TARGET_DIR` is the key detail that makes
the recursive cargo invocation safe (no deadlock on the parent build's locked target dir).

## The user crate (`user/src/main.rs`)

`#![no_std]` / `#![no_main]`. A `#[no_mangle] pub extern "C" fn _start() -> !` is the ELF
`e_entry`. Thin syscall stubs via inline `svc #0` (Linux-like ABI: `x8` = number, args in
`x0..`, return in `x0`):

```rust
unsafe fn sys_print(ptr: *const u8, len: usize) { syscall2(SYS_PRINT, ptr as u64, len as u64); }
unsafe fn sys_exit(code: i32) -> ! { syscall1(SYS_EXIT, code as u64); loop {} }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello, world from EL0!\n";
    unsafe { sys_print(msg.as_ptr(), msg.len()); sys_exit(0); }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { unsafe { sys_exit(1) } }
```

The string lives in `.rodata` (R-X segment). No heap, no libc.

### `user.ld`

Non-PIE (`ET_EXEC`), fixed link base at the existing user region (`0x2_0000_0000`). Explicit
`PHDRS` produce a clean two-segment split so the loader sees real per-segment perms:

```text
PHDRS { text PT_LOAD FLAGS(5); data PT_LOAD FLAGS(6); }   /* 5 = R+X, 6 = R+W */
ENTRY(_start)
. = 0x200000000;
.text   : { *(.text .text.*) }    :text
.rodata : { *(.rodata .rodata.*) } :text
. = ALIGN(0x1000);
.data   : { *(.data .data.*) }    :data
.bss    : { *(.bss .bss.*) *(COMMON) } :data
```

`-z max-page-size=4096` so segments are page-granular.

## Kernel ELF loader (`kernel/src/elf.rs`)

Minimal ET_EXEC AArch64 ELF64 loader over the embedded image bytes:

1. Validate header: magic `\x7fELF`, `EI_CLASS = ELFCLASS64`, `e_machine = EM_AARCH64 (0xB7)`,
   `e_type = ET_EXEC`.
2. Walk program headers (`e_phoff`, `e_phnum`, `e_phentsize`). For each `PT_LOAD` (`p_type == 1`):
   - Allocate frames covering `[p_vaddr, p_vaddr + p_memsz)`, page-aligned.
   - Map at `p_vaddr` with perms from `p_flags`: `PF_X → PAGE_USER_RX`, else `PAGE_USER_RW`
     (W^X holds — no segment is both W and X).
   - `memcpy` `p_filesz` bytes from `image[p_offset..]`; zero-fill the `p_memsz - p_filesz`
     BSS tail.
   - Record `[p_vaddr, p_vaddr + p_memsz)` in a small fixed-size table of loaded ranges.
3. Return `e_entry`.

No demand paging (eager copy), no relocations/PIE, no dynamic linking, no argv/auxv.

## `enter_user()` and pointer validation (`kernel/src/user.rs`)

Slimmed: instead of mapping a fixed in-place blob, it calls `elf::load(USER_ELF_BYTES)` to
map the segments, maps a user stack page (`PAGE_USER_RW`), and `ERET`s to the returned
`e_entry`. `is_user_range(ptr, len)` now checks `[ptr, ptr+len)` against the loader-recorded
segment ranges plus the stack window — so the `SYS_PRINT` validation stays meaningful (the
hello-string pointer must fall inside a mapped user segment). `user.s` and its `.user_text`
linker section / `__user_start`/`__user_end` symbols are deleted.

## `SYS_EXIT` (`kernel/src/syscall.rs`)

```rust
pub const SYS_EXIT: u64 = 3;   // x0 = exit code
```

The dispatcher handles `SYS_EXIT` by printing `"[user] exited with code <n>"` and **not**
returning to the EL0 trap-return path — control stays at EL1 and the kernel enters a `wfi`
idle loop (`loop { wfi }`, noreturn). This is a first, minimal cut of process termination:
the process is done and the kernel regains control, rather than EL0 spinning forever. Timer
IRQs still fire during idle. Full teardown (free the user's frames, return to a
scheduler/shell) is a later phase.

## Demo / verification

`kmain` reaches `entering user mode (EL0)...` and calls `enter_user()` as today. The EL0
program prints `Hello, world from EL0!` then calls `SYS_EXIT(0)`; the kernel prints
`[user] exited with code 0` and idles.

`test.sh` changes:
- **Remove** `"rejected out-of-range user pointer"` (the bad-pointer demo is gone).
- **Replace** `"Hello from EL0!"` with `"Hello, world from EL0!"`.
- **Add** `"[user] exited with code 0"`.
- Keep `"entering user mode (EL0)"`.

## Files

| File | Change |
|---|---|
| `Cargo.toml` (root) | becomes `[workspace]`, `default-members = ["kernel"]`. |
| `kernel/Cargo.toml` (new) | the package formerly at root (`name = "jos"`). |
| `kernel/build.rs` (new) | build user crate (isolated target dir), emit `USER_ELF`, `-Tlinker.ld`. |
| `kernel/linker.ld` | moved from root, unchanged. |
| `kernel/src/*` | moved from `src/`. |
| `kernel/src/elf.rs` (new) | minimal ET_EXEC AArch64 ELF64 loader. |
| `kernel/src/user.rs` | load via `elf::load`, `is_user_range` over loaded ranges, embed `USER_ELF`. |
| `kernel/src/user.s` | **deleted**. |
| `kernel/src/syscall.rs` | `SYS_EXIT` + dispatch arm (print + idle, noreturn). |
| `kernel/src/main.rs` | drop `mod`-level references to the old user blob as needed. |
| `user/Cargo.toml` (new) | the EL0 program crate. |
| `user/build.rs` (new) | emit `-Tuser.ld`. |
| `user/user.ld` (new) | EL0 layout, two `PT_LOAD` segments. |
| `user/src/main.rs` (new) | `_start`, syscall stubs, hello + exit, panic handler. |
| `.cargo/config.toml` | drop the global `-Tlinker.ld` rustflag (now per-crate). |
| `test.sh` | updated markers (see above). |
| `README.md` | layout section updated for the workspace + new files. |

## Out of scope (named later phases)

Demand paging · PIE / ASLR / relocations · dynamic linking · `argv`/`envp`/`auxv` ·
filesystem & a real `exec` · running the user program as a scheduler task · graceful EL0
fault handling (write-to-`.text` W^X demo) · freeing the user's frames on exit.
