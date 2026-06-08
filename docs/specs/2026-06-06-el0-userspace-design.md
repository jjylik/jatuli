# jatuli — Phase 9: Minimal EL0 Userspace + Pointer Validation (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Drop to EL0 (unprivileged), run a tiny user routine that makes a syscall serviced
by the kernel and returns to EL0, and validate user-supplied pointers so a malicious user
address is rejected. Turns the `SVC` path into a real user→kernel boundary.

## Reuses

- `ERET` + trap frame (Phase 7). Syscall dispatch is EL-agnostic: an EL0 `svc` lands in
  vector 8 (lower-EL sync) vs vector 4 (EL1), both `kind % 4 == 0`, `EC == 0x15`.
- `map_page` (Phase 5), now with EL0 permission flags.

## New permission flags (`mmu.rs`)

```rust
const AP_EL0_RW: u64 = 0b01 << 6;   // EL1 RW, EL0 RW
const AP_EL0_RO: u64 = 0b11 << 6;   // EL1 RO, EL0 RO
pub const PAGE_USER_RW: u64 = DESC_PAGE | AP_EL0_RW | SH_INNER | AF | UXN | PXN; // stack/data
pub const PAGE_USER_RX: u64 = DESC_PAGE | AP_EL0_RO | SH_INNER | AF | PXN;       // code, UXN=0 (EL0 exec), W^X
```

## User memory layout

Fresh VA range, separate from identity (0–2 GiB) and heap (`0x1_0000_0000`), so no 2 MiB
block splitting (`map_page` builds new L1[8]→L2→L3):
- `USER_CODE_VA = 0x2_0000_0000` → `.user_text` section's physical pages, `PAGE_USER_RX`.
- `USER_STACK_VA = 0x2_0010_0000` → a fresh frame, `PAGE_USER_RW`.

The code page stays EL1-only at its identity address and EL0-RX at the user VA (aliased).

## User routine (`user.s`, `.user_text` section)

Position-independent (`adr` for the message → user VA):
```asm
user_entry:
    adr x0, user_msg ; mov x1, #(user_msg_end-user_msg) ; mov x8, #2 ; svc #0   // good
    movz x0,#0 ; movk x0,#0x4008,lsl#16 ; mov x1,#16 ; mov x8,#2 ; svc #0        // bad: kernel addr
1:  wfe ; b 1b
user_msg: .ascii "Hello from EL0!\n"
```

## Drop to EL0 (`user.rs`)

`enter_user() -> !`: `map_page` the code (EL0-RX, in place) and a stack (EL0-RW), then
```
msr SPSR_EL1, #0          // M=EL0t, interrupts enabled
msr ELR_EL1, <user code VA> ; msr SP_EL0, <user stack top> ; eret
```
The kernel is re-entered only via syscalls/IRQs afterward. Symbol addresses via
`extern "C" { static ... }` + `addr_of!` (no `fn`-to-int cast).

## Pointer validation (`syscall.rs`)

Validation applies only to untrusted (EL0) callers, so the kernel's own Phase 7
`SYS_PRINT` (kernel pointer) still works. The dispatcher knows the source: `from_user = kind >= 8`.
```rust
pub fn dispatch(frame: &mut TrapFrame, from_user: bool) { ... }
fn sys_print(ptr, len, from_user) -> u64 {
    if from_user && !user::is_user_range(ptr, len) {
        kprintln!("rejected out-of-range user pointer {:#x}", ptr);
        return u64::MAX;
    }
    /* read & print */ 0
}
```
`user::is_user_range(ptr, len)` checks `[ptr, ptr+len)` lies entirely within the mapped
user code or stack window.

## Demo / verification

After the IRQ check, `kmain` prints `entering user mode (EL0)...` and calls `enter_user()`.
The EL0 routine: (1) good `SYS_PRINT` → kernel prints `Hello from EL0!`; (2) bad
`SYS_PRINT(0x40080000)` → kernel prints `rejected out-of-range user pointer 0x40080000`;
(3) loops. `test.sh` greps those three markers. The rejection only fires for `from_user`
(SVC via the lower-EL vector), so it is itself evidence the call came from EL0.

## Files

| File | Change |
|---|---|
| `src/user.s` (new) | EL0 routine in `.user_text`. |
| `src/user.rs` (new) | user VA layout, `is_user_range`, `enter_user`. |
| `src/mmu.rs` | `PAGE_USER_RW` / `PAGE_USER_RX`. |
| `src/syscall.rs` | `dispatch(frame, from_user)` + validated `sys_print`. |
| `src/exceptions.rs` | SVC arm passes `kind >= 8`. |
| `src/main.rs` | `mod user;`, enter EL0 at the end (replaces idle loop). |
| `linker.ld` | `.user_text` + `__user_start`/`__user_end`. |
| `test.sh` | new markers. |

## Out of scope

Separate per-process address spaces / multiple processes, ELF loader, higher-half kernel,
`brk`/`mmap`, copy-to/from-user beyond the range check, returning from EL0 to the kernel.
