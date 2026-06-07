//! `jring`: an io_uring-lite over a shared ring page.
//!
//! One 4 KiB page, mapped EL0-RW at [`USER_RING_VA`], holds a submission queue
//! (user produces, kernel consumes) and a completion queue (kernel produces,
//! user consumes). The user batches SQEs and makes one `SYS_RING_ENTER` call;
//! `NOP`/`PRINT` complete immediately, while a `READ` with no input waiting is
//! parked in a pending table and completed later **from the timer interrupt**,
//! while user code runs — asynchronous completion with no syscall involved.
//!
//! Index discipline (mirrors io_uring): head/tail are free-running `u32`s
//! masked by `ENTRIES - 1`; a producer publishes entries with a release store
//! of its tail, a consumer reads the tail with acquire before the entries.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::frames::alloc_frame;
use crate::mmu::{self, PAGE_USER_RW};
use crate::sync::Locked;
use crate::uart;

/// Virtual address of the shared ring page (above the user stack).
pub const USER_RING_VA: usize = 0x2_0030_0000;

/// Entries in each ring (power of two).
const ENTRIES: u32 = 16;
const MASK: u32 = ENTRIES - 1;

// Byte offsets within the ring page.
const SQ_HEAD: usize = 0x00;
const SQ_TAIL: usize = 0x04;
const CQ_HEAD: usize = 0x08;
const CQ_TAIL: usize = 0x0c;
const SQ_OFF: usize = 0x40; // 16 x 32-byte SQEs
const CQ_OFF: usize = 0x280; // 16 x 16-byte CQEs

// SQE opcodes.
pub const OP_NOP: u64 = 0;
pub const OP_PRINT: u64 = 1;
pub const OP_READ: u64 = 2;

/// CQE result for an invalid pointer, opcode, or a full pending table.
const ERR: i64 = -1;

/// A parked `READ` awaiting console input.
#[derive(Clone, Copy)]
struct Pending {
    buf: usize,
    len: usize,
    user_data: u64,
}

/// Maximum simultaneously parked reads.
const MAX_PENDING: usize = 8;

struct RingState {
    /// Whether the ring page has been mapped (setup is idempotent).
    mapped: bool,
    pending: [Option<Pending>; MAX_PENDING],
}

static RING: Locked<RingState> = Locked::new(RingState {
    mapped: false,
    pending: [None; MAX_PENDING],
});

/// A head/tail index in the shared page, viewed as an atomic.
fn index(off: usize) -> &'static AtomicU32 {
    // SAFETY: the ring page is mapped and the four indices are aligned u32s;
    // AtomicU32 gives both sides ordered access to the same memory.
    unsafe { &*((USER_RING_VA + off) as *const AtomicU32) }
}

/// Read field `n` (of 4) of the SQE at ring slot `slot`.
fn sqe_field(slot: u32, n: usize) -> u64 {
    let p = (USER_RING_VA + SQ_OFF + (slot & MASK) as usize * 32 + n * 8) as *const u64;
    // SAFETY: in-bounds within the mapped ring page.
    unsafe { p.read_volatile() }
}

/// Post a completion: write the CQE, then publish with a release store of the
/// tail so the user sees the entry before the index moves.
fn complete(user_data: u64, result: i64) {
    let tail = index(CQ_TAIL).load(Ordering::Relaxed);
    let head = index(CQ_HEAD).load(Ordering::Acquire);
    if tail.wrapping_sub(head) >= ENTRIES {
        crate::kprintln!("jring: completion queue overflow, dropping CQE");
        return;
    }
    let p = (USER_RING_VA + CQ_OFF + (tail & MASK) as usize * 16) as *mut u64;
    // SAFETY: in-bounds within the mapped ring page.
    unsafe {
        p.write_volatile(user_data);
        p.add(1).write_volatile(result as u64);
    }
    index(CQ_TAIL).store(tail.wrapping_add(1), Ordering::Release);
}

/// `SYS_RING_SETUP`: map and zero the ring page (idempotent). Returns its VA.
pub fn setup() -> u64 {
    let mut ring = RING.lock();
    if !ring.mapped {
        let frame = alloc_frame().expect("out of frames for the ring page");
        mmu::map_page(USER_RING_VA, frame.addr(), PAGE_USER_RW);
        // SAFETY: freshly mapped EL1-writable page; zero it before publishing.
        unsafe { core::ptr::write_bytes(USER_RING_VA as *mut u8, 0, 4096) };
        ring.mapped = true;
    }
    USER_RING_VA as u64
}

/// `SYS_RING_ENTER`: consume all published SQEs. Runs with IRQs masked (we are
/// in a syscall), so it never races [`poll_pending`].
pub fn enter(from_user: bool) -> u64 {
    if !RING.lock().mapped {
        return u64::MAX;
    }
    let mut head = index(SQ_HEAD).load(Ordering::Relaxed);
    let tail = index(SQ_TAIL).load(Ordering::Acquire);
    while head != tail {
        process_sqe(head, from_user);
        head = head.wrapping_add(1);
        index(SQ_HEAD).store(head, Ordering::Release);
    }
    0
}

/// Execute one submission, posting its completion (now, or for a parked
/// `READ`, later from the timer IRQ).
fn process_sqe(slot: u32, from_user: bool) {
    let opcode = sqe_field(slot, 0);
    let arg0 = sqe_field(slot, 1) as usize;
    let arg1 = sqe_field(slot, 2) as usize;
    let user_data = sqe_field(slot, 3);

    match opcode {
        OP_NOP => complete(user_data, 0),
        OP_PRINT => {
            if from_user && !crate::user::is_user_range(arg0, arg1) {
                complete(user_data, ERR);
                return;
            }
            // SAFETY: kernel-trusted, or validated to lie in mapped user memory.
            let bytes = unsafe { core::slice::from_raw_parts(arg0 as *const u8, arg1) };
            if let Ok(s) = core::str::from_utf8(bytes) {
                uart::write_str(s);
            }
            complete(user_data, 0);
        }
        OP_READ => {
            if from_user && !crate::user::is_user_range_writable(arg0, arg1) {
                complete(user_data, ERR);
                return;
            }
            if arg1 == 0 {
                complete(user_data, 0);
                return;
            }
            // Input already waiting completes immediately; otherwise park.
            let count = drain_uart(arg0, arg1);
            if count > 0 {
                complete(user_data, count as i64);
                return;
            }
            let mut ring = RING.lock();
            match ring.pending.iter_mut().find(|s| s.is_none()) {
                Some(slot) => {
                    *slot = Some(Pending {
                        buf: arg0,
                        len: arg1,
                        user_data,
                    })
                }
                None => complete(user_data, ERR),
            }
        }
        _ => complete(user_data, ERR),
    }
}

/// Complete parked reads whose input has arrived. Called from the timer IRQ
/// handler each tick — CQEs land while user code runs, no syscall involved.
pub fn poll_pending() {
    let mut ring = RING.lock();
    for slot in ring.pending.iter_mut() {
        if let Some(p) = *slot {
            let count = drain_uart(p.buf, p.len);
            if count > 0 {
                complete(p.user_data, count as i64);
                *slot = None;
            }
        }
    }
}

/// Copy whatever console input is immediately available into `[buf, buf+len)`.
/// The buffer was validated (or is kernel-trusted) when the SQE was accepted.
fn drain_uart(buf: usize, len: usize) -> usize {
    let dst = buf as *mut u8;
    let mut count = 0;
    while count < len {
        match uart::try_getc() {
            // SAFETY: validated writable user memory (or kernel-trusted).
            Some(b) => unsafe {
                dst.add(count).write_volatile(b);
                count += 1;
            },
            None => break,
        }
    }
    count
}
