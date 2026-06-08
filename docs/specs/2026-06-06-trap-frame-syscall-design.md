# jatuli — Phase 7: Trap Frame + SVC Syscall Dispatch (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Upgrade the exception handler from "report and halt" to a full trap-frame
save/restore that can `ERET` back, and dispatch `SVC` as a syscall. Demonstrate a syscall
round-trip from EL1 (kernel-to-kernel; EL0 comes later). Boot now completes and idles.

## Decisions (from brainstorming)

- **Linux-like ABI**: syscall number in `x8`, args in `x0..x5`, return value in `x0`.
- Faults (data abort, etc.) still **report-and-halt** (not recoverable); only `SVC` resumes.
- Drop Phase 6's deliberate data-abort trigger — boot completes to the idle loop.
- Trap frame on the current kernel stack (EL1->EL1; no stack switch yet).

## Trap frame (matches `exceptions.s`)

```rust
#[repr(C)]
pub struct TrapFrame {
    pub x: [u64; 31],  // x0..x30   offsets 0..240
    pub elr: u64,      // ELR_EL1   offset 248
    pub spsr: u64,     // SPSR_EL1  offset 256
    pub _pad: u64,     // -> 272 bytes, 16-byte aligned
}
```

## Vector stub + common save/restore (`exceptions.s`)

Each of the 16 entries (still 0x80 apart, table 2 KiB-aligned) saves registers before
clobbering any:
```asm
sub  sp, sp, #272
stp  x0, x1, [sp, #0]     // save x0,x1 first
mov  x0, #<index>         // kind
b    common_exception
```
`common_exception` saves `x2..x30`, `ELR_EL1`, `SPSR_EL1`, calls
`exception_dispatch(kind=x0, frame=x1=sp)`, then restores all registers + `ELR`/`SPSR` and
`ERET`s. SP stays 16-aligned (frame = 272). The handler may modify `frame.x[0]` (syscall
return) or `frame.elr`; the restore picks it up.

## Dispatch (`exceptions.rs`)

```rust
extern "C" fn exception_dispatch(kind: u64, frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    let ec = (read_esr() >> 26) & 0x3F;
    match ec {
        0x15 /* SVC */ => syscall::dispatch(frame),     // handle, ERET resumes
        _              => report_and_halt(kind, esr, frame), // fault -> print + halt
    }
}
```
`report_and_halt` keeps the Phase 6 diagnostic (ESR/ELR/FAR/SPSR + decoded EC) and parks
the CPU. `exception_dispatch` returns normally only for SVC.

## Syscall layer (`src/syscall.rs`, new)

```rust
pub const SYS_ADD: u64 = 1;     // x0 + x1 -> x0
pub const SYS_PRINT: u64 = 2;   // x0 = ptr, x1 = len -> 0

pub fn dispatch(frame: &mut TrapFrame) {
    let ret = match frame.x[8] {
        SYS_ADD   => frame.x[0].wrapping_add(frame.x[1]),
        SYS_PRINT => { sys_print(frame.x[0], frame.x[1]); 0 }
        n => { kprintln!("unknown syscall {}", n); u64::MAX }
    };
    frame.x[0] = ret;
}
```
`sys_print` builds a `&str` from `(ptr, len)` and writes it. Trusted kernel caller for now;
user-pointer validation arrives with EL0.

## Demo (`main.rs`)

`syscall(num, a, b)` wrapper issues `svc #0` (inline asm: `x8`=num, `x0`/`x1`=args, `x0`=ret).
`syscall_self_check`:
- `syscall(SYS_ADD, 3, 4)` → assert `== 7`,
- `syscall(SYS_PRINT, msg.as_ptr(), msg.len())` prints `Hello from a syscall!`,
- prints `syscall self-check passed`, then boot falls through to the idle loop.

## Files

| File | Change |
|---|---|
| `src/exceptions.s` | Rewrite: trap-frame save in stubs + common save/restore/`eret`. |
| `src/exceptions.rs` | `TrapFrame`, dispatch-on-EC, keep `report_and_halt`/`ec_name`/`vector_name`. |
| `src/syscall.rs` (new) | ABI constants + `dispatch` + `sys_print`. |
| `src/main.rs` | `mod syscall;`, replace the deliberate-fault check with the syscall demo + `svc` wrapper. |
| `test.sh` | Drop exception-trigger greps; add `Hello from a syscall!`, `syscall self-check passed`. |

## Verification

Boot prints all prior self-checks, then `Hello from a syscall!` and `syscall self-check
passed`, then idles. Printing *after* the `svc` proves the full round-trip (trap → save →
dispatch → restore → `ERET`) and that execution resumed. `assert_eq!(sum, 7)` proves
args-in/return-out. Debug hangs with `qemu ... -d int`.

## Out of scope

EL0/userspace, user-pointer validation, IRQ/timer, per-process state, syscalls beyond the
two demos.
