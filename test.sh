#!/usr/bin/env bash
set -euo pipefail

cargo build -q
KERNEL=target/aarch64-unknown-none/debug/jos

OUT="$(mktemp)"
# Pipe a scripted shell session into the serial console: a typo, help, exit.
# (\r is what a real terminal sends for Enter.)
printf 'hellp\rhelp\rspam\rexit\r' | qemu-system-aarch64 -machine virt,gic-version=3 \
    -cpu cortex-a72 -nographic -kernel "$KERNEL" >"$OUT" 2>&1 &
QPID=$!

sleep 2
kill "$QPID" 2>/dev/null || true
wait "$QPID" 2>/dev/null || true

fail=0
for needle in "Hello, World!" "Hello from the heap!" "heap self-check passed" "frame self-check passed" "mmu enabled" "mmu self-check passed" "syscall self-check passed" "irq self-check passed" "elf self-check passed" "Hello from the ring!" "ring self-check passed" "entering user mode (EL0)" "jsh: type 'help'" "unknown command: hellp" "commands: help spam exit" "spam 3" "[sqpoll] picked up work" "[user] exited with code 0" "[sleeper] woke 3" "busy thread done" "preempt+sleep self-check passed"; do
    if ! grep -qF "$needle" "$OUT"; then
        echo "FAIL: expected '$needle' in serial output."
        fail=1
    fi
done

if [ "$fail" -eq 0 ]; then
    echo "PASS: boot + heap self-check"
    rm -f "$OUT"
    exit 0
else
    echo "--- serial output was: ---"
    cat "$OUT"
    rm -f "$OUT"
    exit 1
fi
