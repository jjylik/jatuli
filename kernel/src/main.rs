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
mod input;
mod irq;
mod mem;
mod mmu;
mod process;
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
    gic::enable_spi(uart::UART_INTID);
    uart::set_rx_irq(true); // always on: input drains to the kernel buffer
    timer::init();
    irq::enable();
    irq_self_check();

    sched_self_check();

    elf_self_check();

    // SQPOLL: a kernel task that polls the submission queue, so published
    // SQEs are consumed without any syscall (it sleeps via NEED_WAKEUP when idle).
    sched::spawn(ring::sqpoll_main, 0);

    // Two processes from the same image, each in its own address space: identical
    // user VAs backed by different frames and different TTBR0 — isolation by
    // construction. Process 0 owns the keyboard (foreground); process 1 runs in
    // the background (its reads park). Each runs as a schedulable task: the ERET
    // to EL0 is on its own kernel stack, so its traps land there. kmain stays
    // behind as the idle task.
    let p0 = process::create(elf::USER_ELF, mmu::new_address_space());
    process::set_foreground(p0, true);
    let p1 = process::create(elf::USER_ELF, mmu::new_address_space());
    process_isolation_self_check(p0, p1);
    sched::spawn_user(user_task, 0, p0, process::ttbr0(p0));
    sched::spawn_user(user_task, 0, p1, process::ttbr0(p1));
    loop {
        // SAFETY: wait for an interrupt; any IRQ (timer, UART) wakes us.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack, preserves_flags)) };
        sched::yield_now();
    }
}

/// The task that hosts the user program. Never returns: after the ERET the
/// kernel re-enters only via traps, which use this task's stack.
extern "C" fn user_task(_arg: usize) {
    uart::write_str("entering user mode (EL0)...\n");
    user::enter_user();
}

/// Prove the two processes are isolated: distinct address-space roots, and the
/// same user VA (`USER_BASE`, each program's text base) backed by *different*
/// physical frames. Only separate page tables can do that.
fn process_isolation_self_check(p0: usize, p1: usize) {
    let (t0, t1) = (process::ttbr0(p0), process::ttbr0(p1));
    assert_ne!(t0, t1, "processes share an address-space root");
    let pa0 = mmu::translate(t0, abi::USER_BASE).expect("p0 USER_BASE unmapped");
    let pa1 = mmu::translate(t1, abi::USER_BASE).expect("p1 USER_BASE unmapped");
    assert_ne!(pa0, pa1, "processes share a frame at USER_BASE");
    kprintln!("process isolation: USER_BASE -> {:#x} (p0) vs {:#x} (p1)", pa0, pa1);
    uart::write_str("process isolation self-check passed\n");
}

/// Validate the embedded userspace ELF header before we try to run it.
fn elf_self_check() {
    let entry = elf::validate(elf::USER_ELF);
    assert!((entry >> 39) == abi::USER_L0_IDX, "user entry VA not in the user L0 slot");
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

/// Exercise the syscall path via `SVC` from EL1: prove args-in/return-out and
/// that `ERET` resumed us. (Side-effecting I/O syscalls are gone — all action
/// I/O flows through the jring, which `ring_self_check` covers.)
fn syscall_self_check() {
    // SAFETY: issuing supervisor calls with the kernel's own syscall ABI.
    let sum = unsafe { syscall(syscall::SYS_ADD, 3, 4) };
    assert_eq!(sum, 7, "syscall add returned the wrong value");

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
