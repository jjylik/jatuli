#!/usr/bin/env bash
set -euo pipefail

cargo build -q
KERNEL=target/aarch64-unknown-none/debug/jos

OUT="$(mktemp)"
qemu-system-aarch64 -machine virt -cpu cortex-a72 -nographic \
    -kernel "$KERNEL" >"$OUT" 2>&1 &
QPID=$!

sleep 1
kill "$QPID" 2>/dev/null || true
wait "$QPID" 2>/dev/null || true

if grep -q "Hello, World!" "$OUT"; then
    echo "PASS: kernel printed 'Hello, World!'"
    rm -f "$OUT"
    exit 0
else
    echo "FAIL: expected 'Hello, World!' in serial output. Got:"
    cat "$OUT"
    rm -f "$OUT"
    exit 1
fi
