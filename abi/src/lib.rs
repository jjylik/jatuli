//! The jos ABI (Application Binary Interface): the contract between kernel
//! and userspace, in one place.
//!
//! Everything both sides must agree on byte-for-byte lives here — syscall
//! numbers, the jring shared-page layout, opcodes, and flags. The kernel and
//! the user program are separately compiled crates; this crate is their only
//! shared vocabulary (the moral equivalent of Linux's uapi headers).
//!
//! Concurrency model of the shared page: all fields are atomics. A producer
//! writes entries with relaxed stores, then *publishes* them with a release
//! store of its tail index; the consumer reads the tail with acquire before
//! touching entries. Relaxed atomic loads/stores compile to plain `ldr`/`str`
//! on AArch64 — this is the Linux `READ_ONCE`/`WRITE_ONCE` discipline, which
//! (unlike `volatile`) is the correct tool for memory another agent mutates.

#![no_std]

use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};

// ---------------------------------------------------------------------------
// Syscalls
// ---------------------------------------------------------------------------
// Calling convention (Linux-like AArch64): syscall number in `x8`, arguments
// in `x0..x5`, return value in `x0`, invoked with `svc #0`.

/// Add two arguments: `x0 + x1 -> x0`. (Demo syscall.)
pub const SYS_ADD: u64 = 1;
/// Terminate the user program: `x0` = exit code. Does not return to EL0.
pub const SYS_EXIT: u64 = 3;
/// Map (idempotently) the shared jring page; returns its virtual address.
pub const SYS_RING_SETUP: u64 = 5;
/// Process all published jring submissions, then block until the completion
/// queue holds at least `x0` unreaped entries (0 = submit-only); returns 0.
pub const SYS_RING_ENTER: u64 = 6;
// (2 and 4 were SYS_PRINT and SYS_READ; all action I/O now flows through the
// jring, so the numbers are retired rather than reused.)

// ---------------------------------------------------------------------------
// jring: shared submission/completion rings
// ---------------------------------------------------------------------------

/// Base of the EL0 virtual-address window. This is L0 slot 1 (512 GiB),
/// deliberately separate from the kernel's L0 slot 0, so that per-process page
/// tables can split kernel and user mappings on a clean top-level boundary.
/// The `user.ld` linker script hardcodes this same value (it cannot read Rust);
/// the kernel asserts loaded segments land in this slot, catching any drift.
pub const USER_BASE: usize = 0x80_0000_0000;
/// L0 index of the user window (= 1). The kernel checks loaded user VAs against it.
pub const USER_L0_IDX: usize = USER_BASE >> 39;

/// Virtual address where the kernel maps the shared ring page (3 MiB into the
/// user window, above the user stack). Fixed by convention; `SYS_RING_SETUP`
/// also returns it.
pub const USER_RING_VA: usize = USER_BASE + 0x30_0000;

/// Entries in each ring (power of two, so indices wrap with [`RING_MASK`]).
pub const RING_ENTRIES: u32 = 16;
/// Mask turning a free-running index into a ring slot.
pub const RING_MASK: u32 = RING_ENTRIES - 1;

/// Flags-word bit: the kernel's SQ poller went to sleep; submitters must call
/// `SYS_RING_ENTER` once to revive it (Linux's `IORING_SQ_NEED_WAKEUP`).
pub const NEED_WAKEUP: u32 = 1;

/// Opcode: NOP ("no operation") — completes immediately with result 0.
pub const OP_NOP: u64 = 0;
/// Opcode: print a UTF-8 string (`arg0` = pointer, `arg1` = length).
pub const OP_PRINT: u64 = 1;
/// Opcode: read console input (`arg0` = buffer, `arg1` = length); completes
/// with the number of bytes read once input is available.
pub const OP_READ: u64 = 2;
/// Opcode: spawn a program by name (`arg0` = name pointer, `arg1` = name length).
/// Completes with the child's handle (its pid) in `result`, or negative on error
/// (unknown program, or a bad/invalid name pointer). Ring-native — no `fork`/`exec`
/// and no syscall: a fresh process is created from scratch and run.
pub const OP_SPAWN: u64 = 3;
/// Opcode: wait for a child to exit (`arg0` = a handle from `OP_SPAWN`). Completes
/// with the child's exit code in `result`, or negative if the handle is not a
/// child of the caller. Completes immediately if the child already exited.
pub const OP_WAIT: u64 = 4;

/// SQE — **Submission Queue Entry**: one I/O request, written by userspace
/// into the SQ (submission queue) and consumed by the kernel.
///
/// Acronyms: SQE = Submission Queue Entry; an SQE's `user_data` is an opaque
/// tag echoed back in the matching [`Cqe`], which is how completions (possibly
/// out of order) are matched to requests.
#[repr(C)]
pub struct Sqe {
    /// Operation code (`OP_NOP` / `OP_PRINT` / `OP_READ` / `OP_SPAWN` / `OP_WAIT`).
    pub opcode: AtomicU64,
    /// First argument (typically a pointer, as a virtual address).
    pub arg0: AtomicU64,
    /// Second argument (typically a length in bytes).
    pub arg1: AtomicU64,
    /// Opaque completion tag, echoed in the matching CQE.
    pub user_data: AtomicU64,
}

/// CQE — **Completion Queue Entry**: one finished request, written by the
/// kernel into the CQ (completion queue) and reaped by userspace.
///
/// Acronyms: CQE = Completion Queue Entry.
#[repr(C)]
pub struct Cqe {
    /// The originating SQE's `user_data` tag.
    pub user_data: AtomicU64,
    /// Operation result: `>= 0` on success, negative on error.
    pub result: AtomicI64,
}

/// The shared ring page: one 4 KiB page, mapped user-read/write, holding the
/// SQ (submission queue: userspace produces, kernel consumes) and the CQ
/// (completion queue: kernel produces, userspace consumes).
///
/// Acronyms used by the fields: SQ = Submission Queue, CQ = Completion Queue;
/// `head` is the consumer's position, `tail` the producer's. Indices
/// free-run as `u32`s and are masked with [`RING_MASK`] to select a slot;
/// `tail - head` is the number of unconsumed entries.
#[repr(C)]
pub struct RingPage {
    /// SQ consumer index (kernel advances after consuming submissions).
    pub sq_head: AtomicU32,
    /// SQ producer index (userspace advances to publish submissions).
    pub sq_tail: AtomicU32,
    /// CQ consumer index (userspace advances after reaping completions).
    pub cq_head: AtomicU32,
    /// CQ producer index (kernel advances after posting completions).
    pub cq_tail: AtomicU32,
    /// Status flags ([`NEED_WAKEUP`]); kernel-written, user-read.
    pub flags: AtomicU32,
    _pad0: [u8; 0x40 - 0x14],
    /// The submission queue entries.
    pub sq: [Sqe; RING_ENTRIES as usize],
    _pad1: [u8; 0x280 - 0x240],
    /// The completion queue entries.
    pub cq: [Cqe; RING_ENTRIES as usize],
}

// The layout IS the contract: pin it at compile time so drift between the
// sides is a build error, never a silent protocol corruption.
const _: () = {
    assert!(size_of::<Sqe>() == 32);
    assert!(size_of::<Cqe>() == 16);
    assert!(offset_of!(RingPage, sq_head) == 0x00);
    assert!(offset_of!(RingPage, sq_tail) == 0x04);
    assert!(offset_of!(RingPage, cq_head) == 0x08);
    assert!(offset_of!(RingPage, cq_tail) == 0x0c);
    assert!(offset_of!(RingPage, flags) == 0x10);
    assert!(offset_of!(RingPage, sq) == 0x40);
    assert!(offset_of!(RingPage, cq) == 0x280);
    assert!(size_of::<RingPage>() <= 4096);
};
