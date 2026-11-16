#!/usr/bin/env python3
"""Regression smoke test for BUG-083 (syscall return value clobbered by KPTI).

The bug made every syscall return the user CR3 in RAX instead of its real
result.  `cat` (which does `mov rdx, rax` after `read`) then called
`write(1, buf, 0x62000)` and flooded the serial with ~401 KB of zeroes.

This test drives the ordinary kernel over its serial TCP console, runs `cat`,
feeds it one short line, and fails if either:

  * the line is not echoed back (read's return value never reached user), or
  * the serial output is flooded (the old 401 KB zero-spew came back).

It needs no in-kernel instrumentation: the byte count on the wire is the signal.
"""

import os
import socket
import subprocess
import sys
import time

PORT = int(os.environ.get("ABI_SMOKE_PORT", "45679"))
ISO = os.environ.get("ABI_SMOKE_ISO", "build/zenus.iso")
QEMU = os.environ.get("QEMU", "qemu-system-x86_64")
TIMEOUT = int(os.environ.get("ABI_SMOKE_TIMEOUT", "120"))
MARKER = b"ABCRETVALMARKER"
LOG = os.environ.get("ABI_SMOKE_LOG", "build/abi-smoke.log")


def connect():
    deadline = time.time() + TIMEOUT
    while time.time() < deadline:
        try:
            return socket.create_connection(("localhost", PORT), timeout=2)
        except OSError:
            time.sleep(0.25)
    raise RuntimeError("serial console never came up")


def recv_for(s, seconds):
    """Drain the socket for a fixed window; never block past it."""
    s.settimeout(0.5)
    buf = bytearray()
    end = time.time() + seconds
    while time.time() < end:
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            continue
        if not chunk:
            break
        buf += chunk
    return bytes(buf)


def main():
    os.makedirs(os.path.dirname(LOG) or ".", exist_ok=True)
    proc = subprocess.Popen(
        [
            QEMU, "-display", "none", "-serial",
            f"tcp:localhost:{PORT},server,nowait", "-m", "2G", "-smp", "2",
            "-cdrom", ISO, "-no-reboot", "-cpu", "max", "-net", "none",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        s = connect()
        boot = recv_for(s, min(TIMEOUT, 150))
        if b"zenus$ " not in boot:
            print("[ABI-SMOKE] FAIL: kernel never reached the shell prompt")
            return 1

        # `cat` reads stdin (fd 0) and writes the exact byte count back.  With a
        # working syscall ABI the marker comes back once; with BUG-083 it is
        # followed by ~401 KB of zeroes.
        before = len(boot)
        s.sendall(b"run /initrd/bin/cat\r")
        time.sleep(1.0)
        s.sendall(MARKER + b"\r")
        out = recv_for(s, 4.0)
        total = boot + out
        open(LOG, "wb").write(total)

        segment = total[before:]
        echoed = MARKER in segment
        nulls = segment.count(b"\x00")
        # 2 KB is generous for the shell's own log lines around `run cat`.
        flooded = len(segment) > 2048 or nulls > 50

        if echoed and not flooded:
            print(
                f"[ABI-SMOKE] OK: read() return value reached userspace "
                f"({len(segment)} bytes, {nulls} nulls)"
            )
            return 0

        print(
            f"[ABI-SMOKE] FAIL: echoed={echoed} nulls={nulls} "
            f"bytes={len(segment)} (serial tail in {LOG})"
        )
        return 1
    finally:
        proc.kill()
        proc.wait()


if __name__ == "__main__":
    sys.exit(main())
