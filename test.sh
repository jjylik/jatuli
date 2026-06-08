#!/usr/bin/env bash
set -euo pipefail

cargo build -q
KERNEL=target/aarch64-unknown-none/debug/jos

# Boot the kernel with `$1` piped into the serial console; serial output lands
# in the file named by `$2`.
boot() {
    printf '%b' "$1" | qemu-system-aarch64 -machine virt,gic-version=3 \
        -cpu cortex-a72 -nographic -kernel "$KERNEL" >"$2" 2>&1 &
    QPID=$!
    sleep 2
    kill "$QPID" 2>/dev/null || true
    wait "$QPID" 2>/dev/null || true
}

fail=0
expect() { # expect <file> <needle>
    if ! grep -qF "$2" "$1"; then
        echo "FAIL: expected '$2' in serial output."
        fail=1
    fi
}
reject() { # reject <file> <needle>
    if grep -qF "$2" "$1"; then
        echo "FAIL: did NOT expect '$2' in serial output."
        fail=1
    fi
}

# Run 1 — a normal session: a typo, help, the SQPOLL demo, clean exit.
# (\r is what a real terminal sends for Enter.)
OUT="$(mktemp)"
boot 'hellp\rhelp\rspam\rexit\r' "$OUT"
for needle in "Hello, World!" "Hello from the heap!" "heap self-check passed" "frame self-check passed" "mmu enabled" "mmu self-check passed" "syscall self-check passed" "irq self-check passed" "elf self-check passed" "process isolation self-check passed" "echo: hello from a second program" "entering user mode (EL0)" "jsh: type 'help'" "unknown command: hellp" "commands: help spam crash exit" "spam 3" "[sqpoll] picked up work" "[user] exited with code 0" "[user] freed" "[sleeper] woke 3" "busy thread done" "preempt+sleep self-check passed"; do
    expect "$OUT" "$needle"
done

# Run 2 — the program faults (write to its own R-X code segment): the kernel
# must kill it, reclaim its memory, and survive (no kernel panic).
OUT2="$(mktemp)"
boot 'crash\r' "$OUT2"
expect "$OUT2" "[user] killed: data abort (lower EL)"
expect "$OUT2" "[user] freed"
reject "$OUT2" "*** EXCEPTION"

if [ "$fail" -eq 0 ]; then
    echo "PASS: boot + heap self-check"
    rm -f "$OUT" "$OUT2"
    exit 0
else
    echo "--- serial output (run 1): ---"
    cat "$OUT"
    echo "--- serial output (run 2): ---"
    cat "$OUT2"
    rm -f "$OUT" "$OUT2"
    exit 1
fi
