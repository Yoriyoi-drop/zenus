#!/usr/bin/env bash
#
# Boot the kernel under QEMU and fail unless it reaches the interactive shell.
#
# This is ROADMAP item 6: a smoke test that boots to the shell prompt and fails
# the build if it does not. Everything else in the tree is unit-tested — 235 host
# tests, 25 in QEMU — but before this nothing asserted the *normal* kernel boots
# at all. `make test` builds the `testing` kernel and runs the in-kernel suite;
# the ordinary kernel, the one that ships in the ISO, had no one checking it.
#
# It is deliberately thin: no input is sent, so it does not drive the shell. It
# waits for the prompt and stops. A kernel that panics, triple-faults, or comes
# up into the wrong image never prints the prompt and the target fails.
#
# Usage:  boot-smoke.sh <iso> [more qemu args...]
#
# Environment:
#   SMOKE_TIMEOUT   seconds to wait for the prompt          (default 90)
#   SMOKE_LOG       where the serial output is captured     (default build/smoke.log)
#   QEMU            the QEMU binary                          (default qemu-system-x86_64)
#   SMP             CPU count, so the boot path matches `make run`  (default 4)
set -uo pipefail

iso=${1:?usage: boot-smoke.sh <iso> [qemu args...]}
shift

timeout=${SMOKE_TIMEOUT:-90}
log=${SMOKE_LOG:-build/smoke.log}
qemu=${QEMU:-qemu-system-x86_64}
smp=${SMP:-4}

# The prompt `Shell::run` prints, with no `0x`/escape decoration. `grep -F` so the
# `$` is a literal, not an anchor.
prompt='zenus$ '

mkdir -p "$(dirname "$log")"
: > "$log"

# `-cpu max` for the same reason `make test` uses it: the qemu64 default has no
# SMEP and no SMAP, so a boot that exercises them needs a model that has them.
# `-display none` because there is no console to draw on.
"$qemu" -display none -serial "file:$log" -m 2G -smp "$smp" -cdrom "$iso" \
    -no-reboot -cpu max -netdev user,id=net0 -device rtl8139,netdev=net0 "$@" &
pid=$!

deadline=$(( $(date +%s) + timeout ))
while kill -0 "$pid" 2>/dev/null; do
    if grep -aqF "$prompt" "$log"; then
        break
    fi
    if [ "$(date +%s)" -ge "$deadline" ]; then
        break
    fi
    sleep 1
done

if grep -aqF "$prompt" "$log"; then
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    echo "[SMOKE] OK: reached the shell prompt ($(wc -l < "$log") serial lines)"
    exit 0
fi

kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
echo "[SMOKE] FAIL: no shell prompt within ${timeout}s. Serial tail:"
tail -40 "$log"
exit 1
