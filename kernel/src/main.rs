#![no_std]
#![no_main]

extern crate alloc;

use core::arch::global_asm;
use core::panic::PanicInfo;

mod allocator;
mod elf;
mod exceptions;
mod frames;
mod gic;
mod irq;
mod mem;
mod mmu;
mod ring;
mod sched;
mod syscall;
mod sync;
mod timer;
mod uart;
mod user;

global_asm!(include_str!("boot.s"));

/// Kernel entry point. Called from `_start` (see `boot.s`) once the stack is set up.
#[no_mangle]
pub extern "C" fn kmain() -> ! {
    uart::write_str("Hello, World!\n");

    exceptions::init_exceptions();

    frames::init_frames();
    frame_self_check();

    mmu::init_mmu();
    mmu_self_check();

    allocator::init_heap();
    heap_self_check();

    syscall_self_check();

    gic::init(timer::TIMER_INTID);
    timer::init();
    irq::enable();
    irq_self_check();

    sched_self_check();

    elf_self_check();
    ring_self_check();

    uart::write_str("entering user mode (EL0)...\n");
    user::enter_user();
}

/// Exercise the jring path from EL1, acting as the ring's user: set it up via
/// syscall, publish a NOP + PRINT + bad-opcode batch, submit with one
/// `SYS_RING_ENTER`, and check the three completions (incl. tag matching).
fn ring_self_check() {
    // SAFETY: ring syscalls take no pointers from us beyond the SQEs below.
    let va = unsafe { syscall(syscall::SYS_RING_SETUP, 0, 0) } as usize;
    assert_eq!(va, ring::USER_RING_VA, "ring page mapped at the wrong VA");

    // Publish three SQEs the way userspace will: write entries, release the tail.
    let msg = "Hello from the ring!\n";
    push_sqe(va, 0, ring::OP_NOP, 0, 0, 101);
    push_sqe(va, 1, ring::OP_PRINT, msg.as_ptr() as u64, msg.len() as u64, 102);
    push_sqe(va, 2, 99, 0, 0, 103); // invalid opcode -> error completion
    let sq_tail = (va + 0x04) as *mut u32;
    // SAFETY: publishing the SQ tail in the mapped ring page.
    unsafe { core::ptr::write_volatile(sq_tail, 3) };

    // One submission syscall for the whole batch.
    // SAFETY: the SQEs above are fully written before the tail was published.
    unsafe { syscall(syscall::SYS_RING_ENTER, 0, 0) };

    // All three ops complete synchronously: expect CQEs (101,0) (102,0) (103,-1).
    let cq_tail = (va + 0x0c) as *const u32;
    // SAFETY: reading the published CQ tail.
    let produced = unsafe { core::ptr::read_volatile(cq_tail) };
    assert_eq!(produced, 3, "expected three completions");
    for (i, want) in [(101u64, 0i64), (102, 0), (103, -1)].iter().enumerate() {
        let p = (va + 0x280 + i * 16) as *const u64;
        // SAFETY: in-bounds CQE reads in the mapped ring page.
        let (tag, res) = unsafe { (core::ptr::read_volatile(p), core::ptr::read_volatile(p.add(1)) as i64) };
        assert_eq!(tag, want.0, "completion tag mismatch");
        assert_eq!(res, want.1, "completion result mismatch");
    }

    uart::write_str("ring self-check passed\n");
}

/// Write one SQE into ring slot `slot` (ring page at `va`).
fn push_sqe(va: usize, slot: usize, op: u64, a0: u64, a1: u64, tag: u64) {
    let p = (va + 0x40 + slot * 32) as *mut u64;
    // SAFETY: in-bounds SQE writes in the mapped ring page.
    unsafe {
        p.write_volatile(op);
        p.add(1).write_volatile(a0);
        p.add(2).write_volatile(a1);
        p.add(3).write_volatile(tag);
    }
}

/// Validate the embedded userspace ELF header before we try to run it.
fn elf_self_check() {
    let entry = elf::validate(elf::USER_ELF);
    assert!(entry >= 0x2_0000_0000, "user entry VA not in the user window");
    kprintln!("user elf: {} bytes, entry {:#x}", elf::USER_ELF.len(), entry);
    uart::write_str("elf self-check passed\n");
}

/// Exercise the global allocator. Panics (and so halts) if anything is wrong.
// Pushing one element at a time is intentional: it forces the Vec to reallocate
// as it grows, which exercises our freestanding `memcpy` (see `mem.rs`).
#[expect(
    clippy::vec_init_then_push,
    reason = "growth-by-push deliberately exercises Vec reallocation and memcpy"
)]
fn heap_self_check() {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;

    let mut numbers: Vec<u32> = Vec::new();
    numbers.push(1);
    numbers.push(2);
    numbers.push(3);
    let sum: u32 = numbers.iter().sum();
    assert_eq!(sum, 6, "heap-backed Vec produced the wrong sum");

    let mut greeting = String::new();
    greeting.push_str("Hello from the heap!");
    uart::write_str(&greeting);
    uart::write_str("\n");

    // The allocation must land in the frame-backed virtual window, proving the
    // heap is no longer on a static array.
    let boxed = Box::new(0xCAFEu32);
    assert_eq!(*boxed, 0xCAFE, "heap read-back wrong");
    let addr = &*boxed as *const u32 as usize;
    assert!(
        (allocator::HEAP_VBASE..allocator::HEAP_VBASE + allocator::HEAP_SIZE).contains(&addr),
        "heap allocation is not in the mapped virtual window"
    );

    uart::write_str("heap self-check passed\n");
}

/// Exercise the physical frame allocator. Panics (and so halts) on any failure.
fn frame_self_check() {
    use frames::{alloc_frame, free_frame, free_frame_count, FRAME_SIZE};

    let initial = free_frame_count();
    assert!(initial > 1000, "expected a large free frame pool");

    let f1 = alloc_frame().expect("frame pool unexpectedly empty");
    let f2 = alloc_frame().expect("frame pool unexpectedly empty");
    assert_ne!(f1.addr(), f2.addr(), "allocated the same frame twice");
    assert_eq!(f1.addr() % FRAME_SIZE, 0, "frame f1 is not 4 KiB aligned");
    assert_eq!(f2.addr() % FRAME_SIZE, 0, "frame f2 is not 4 KiB aligned");
    assert_eq!(free_frame_count(), initial - 2, "free count wrong after alloc");

    free_frame(f1);
    free_frame(f2);
    assert_eq!(free_frame_count(), initial, "free count wrong after free");

    // LIFO: the most recently freed frame (f2) is handed back first.
    let f3 = alloc_frame().expect("frame pool unexpectedly empty");
    assert_eq!(f3.addr(), f2.addr(), "free/alloc did not reuse the freed frame");
    free_frame(f3);

    uart::write_str("frame self-check passed\n");
}

/// Verify the MMU is on and the identity map works. Panics (halts) on failure.
fn mmu_self_check() {
    use frames::{alloc_frame, free_frame};

    // Reaching here at all means instruction fetch survived enabling translation;
    // this print proves the Device mapping (UART) is correct.
    uart::write_str("mmu enabled\n");

    // A freshly allocated frame is reachable via its identity-mapped address.
    let f = alloc_frame().expect("frame pool unexpectedly empty");
    // SAFETY: f is a valid 4 KiB frame, identity-mapped as Normal RAM.
    unsafe {
        let p = f.addr() as *mut u32;
        p.write_volatile(0x1234_5678);
        assert_eq!(
            p.read_volatile(),
            0x1234_5678,
            "frame read-back wrong after MMU enable"
        );
    }
    free_frame(f);

    uart::write_str("mmu self-check passed\n");
}

/// Exercise the syscall path via `SVC` from EL1: prove args-in/return-out and a
/// side-effecting syscall, then let boot continue (which proves `ERET` resumed).
fn syscall_self_check() {
    // SAFETY: issuing supervisor calls with the kernel's own syscall ABI.
    let sum = unsafe { syscall(syscall::SYS_ADD, 3, 4) };
    assert_eq!(sum, 7, "syscall add returned the wrong value");

    let msg = "Hello from a syscall!";
    // SAFETY: msg is a live UTF-8 string; SYS_PRINT reads (ptr, len).
    unsafe { syscall(syscall::SYS_PRINT, msg.as_ptr() as u64, msg.len() as u64) };
    uart::write_str("\n");

    uart::write_str("syscall self-check passed\n");
}

/// Issue a syscall via `SVC` (Linux-like AArch64 ABI: x8 = number, x0.. = args,
/// x0 = return value).
///
/// # Safety
/// Performs a supervisor call; the caller must pass arguments valid for the
/// requested syscall.
unsafe fn syscall(num: u64, arg0: u64, arg1: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "svc #0",
        in("x8") num,
        inout("x0") arg0 => ret,
        in("x1") arg1,
    );
    ret
}

/// Prove the interrupt path works: sleep on `wfi` until the timer has fired
/// several times (each IRQ wakes the CPU), then report.
fn irq_self_check() {
    while timer::ticks() < 5 {
        // SAFETY: wait for an interrupt; the timer IRQ wakes us.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack, preserves_flags)) };
    }
    kprintln!("timer fired {} times", timer::ticks());
    uart::write_str("irq self-check passed\n");
}

/// Spawn a blocking thread and a CPU-bound thread to exercise sleep and
/// preemption; `kmain` idles until both exit.
fn sched_self_check() {
    sched::init();
    sched::spawn(sleeper, 0);
    sched::spawn(busy, 0);

    // kmain is the idle task: yield until every worker has exited.
    while sched::any_worker_alive() {
        sched::yield_now();
    }

    uart::write_str("preempt+sleep self-check passed\n");
}

/// A thread that blocks: prints on each wake, sleeping between.
extern "C" fn sleeper(_arg: usize) {
    for i in 1..=3 {
        kprintln!("[sleeper] woke {}", i);
        sched::sleep_ticks(3);
    }
}

/// A CPU-bound thread that never yields or sleeps. Only preemption lets the
/// sleeper run while this is spinning; the interleaved output is the proof.
extern "C" fn busy(_arg: usize) {
    for round in 1..=3 {
        kprintln!("[busy] round {}", round);
        spin_ticks(3);
    }
    uart::write_str("busy thread done\n");
}

/// Busy-poll (without yielding) until `n` timer ticks have elapsed. Tick-bounded
/// so the duration doesn't depend on emulation speed.
fn spin_ticks(n: u64) {
    let target = timer::ticks() + n;
    while timer::ticks() < target {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") }
    }
}
