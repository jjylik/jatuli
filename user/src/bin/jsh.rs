//! `jsh`: the jos shell. Prompt, line editing, and the `help`/`spam`/`crash`/
//! `exit` builtins. All I/O flows through the ring (see the `user` runtime lib).

#![no_std]
#![no_main]

use user::{exit, print, print_bytes, read_line, tag, uring, MAX_LINE};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    uring::setup();

    // Banner as a batch: two ops published, ONE syscall, tag-matched reaping.
    let banner = "jsh: type 'help'\n";
    let (t1, t2) = (tag(), tag());
    uring::sqe(uring::OP_NOP, 0, 0, t1);
    uring::sqe(uring::OP_PRINT, banner.as_ptr() as u64, banner.len() as u64, t2);
    uring::submit();
    uring::wait(t1);
    uring::wait(t2);

    let mut line = [0u8; MAX_LINE];
    loop {
        print("jsh> ");
        let len = read_line(&mut line);
        dispatch(&line[..len]);
    }
}

/// Run one entered line.
fn dispatch(line: &[u8]) {
    match line {
        b"" => {}
        b"exit" => exit(0),
        b"help" => print("commands: help spam crash exit\n"),
        b"spam" => spam(),
        b"crash" => {
            // Write to our own R-X code segment: the MMU's W^X enforcement turns
            // this into a data abort, and the kernel kills us.
            // SAFETY: deliberately not safe — that's the demo.
            unsafe { (abi::USER_BASE as *mut u8).write_volatile(0) }
        }
        other => {
            print("unknown command: ");
            print_bytes(other);
            print("\n");
        }
    }
}

/// SQPOLL demo: publish three prints with flag-aware submits (zero syscalls while
/// the kernel's SQ poller is awake), then spin on the CQ — deliberately never
/// sleeping in the kernel, so the poller is the only thing that can consume the
/// submissions. Pure shared-memory I/O.
fn spam() {
    let lines = [b"spam 1\n", b"spam 2\n", b"spam 3\n"];
    let mut tags = [0u64; 3];
    for (i, line) in lines.iter().enumerate() {
        tags[i] = tag();
        uring::sqe(uring::OP_PRINT, line.as_ptr() as u64, line.len() as u64, tags[i]);
        uring::submit(); // traps only if the poller raised NEED_WAKEUP
    }
    for t in tags {
        uring::wait_spin(t);
    }
}
