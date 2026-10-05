# Zenus — Operating System Kernel in Rust

x86_64 OS kernel written in Rust (`no_std`, `#![no_main]`). Boots via the
**Limine** bootloader (BIOS + UEFI), built with `cargo` + `ld.lld` and a custom
linker script, and run under QEMU.

## Quick Start

```bash
make build          # kernel -> build/zenus   (cargo build --target x86_64-unknown-none)
make iso            # bootable ISO (BIOS + UEFI)
make run-gui        # QEMU with a window
make run-serial     # QEMU headless, serial on stdio
make run-tcp        # serial over TCP, then: nc localhost 45678

make test-host      # cargo test --workspace --target x86_64-unknown-linux-gnu
make test           # in-kernel test suite inside QEMU
```

## Workspace Layout

| Path | What it is |
|---|---|
| `apps/` | Kernel entry point, boot sequence, init system (PID 1), shell |
| `crates/zenus-arch/` | Limine, CPU, GDT/IDT, APIC/IOAPIC/PIT, PCI, ACPI, SMP, ATA, RTC, keyboard, crash dump |
| `crates/zenus-mem/` | Frame allocator, 4-level paging, VMA tracking, kernel heap |
| `crates/zenus-sched/` | Preemptive round-robin scheduler, signals, init/service supervision |
| `crates/zenus-fs/` | VFS, ext2 (R/W + journal + fsck), tmpfs, devfs, tarfs, procfs, cgroup2 view, sysctl, package manager, block cache |
| `crates/zenus-net/` | ARP, IPv4, ICMP, TCP, UDP, DHCP client+server, DNS, routing, firewall, RTL8139 |
| `crates/zenus-syscall/` | Syscall dispatch (109 syscalls) and the ELF loader |
| `crates/zenus-sync/` | Spinlock, IRQ guard, lockdep |
| `crates/zenus-console/` | Serial, VGA, framebuffer, logging, dmesg, syslog, structured error codes |
| `crates/zenus-virtio/` | virtio-net / -blk / -console / -balloon |
| `crates/zenus-ns/` | PID, UTS, mount, net, user and IPC namespaces |
| `crates/zenus-fuzz/` | In-kernel fuzzing campaigns (mutation, corpus, coverage, minimiser) |
| `zutils/crates/` | 41 `coreutils`-style shell builtins (`ls`, `grep`, `df`, …) |
| `userspace/` | Ring-3 test programs — a **separate** cargo workspace, not covered by `cargo test --workspace` |

## Feature Status

Implemented and exercised:

- Boot via Limine, SMP (AP bring-up, per-CPU data), PCI enumeration
  (ACPI shutdown/power-off exists; `acpi::init()` is never called)
- 4-level paging with per-process address spaces; preemption via LAPIC timer
- 109-syscall interface with ELF loading and ring-3 execution
- ext2 read/write with journalling, fsck, and a write-back block cache
- TCP/UDP/ICMP/DHCP/DNS with static routing, a netfilter-style firewall and
  BSD sockets
- procfs, a read-only cgroup v2 view, sysctl, a `.zpk` package manager
- PID/UTS/mount/net/user/IPC namespaces
- Init system (PID 1) with service supervision and restart policies
- A ZENUS_SSH/1.0 line protocol (**not** SSH, and not cryptographically sound —
  see `SECURITY.md`)

Known gaps are listed in `ROADMAP.md` and `SECURITY.md`. The important ones:
SMAP/SMEP are implemented but **disabled** (see below), there is no KPTI, no
capability system, no driver isolation or hotplug, and storage is PIO-only.

## Testing

Two layers, both runnable:

```bash
make test-host   # == cargo test --workspace --target x86_64-unknown-linux-gnu
```

Host unit tests are `#[cfg(test)]` modules inside the kernel crates. They cover
the pure logic — VMA arithmetic, packet parsing, permission bits, syscall
numbering, journal replay, the fuzzing bookkeeping — and need no VM. 192 tests
across 11 crates.

```bash
make test        # QEMU: the in-kernel suite (apps/src/test_runner.rs)
```

The in-kernel suite is the only layer that *can* test real MMIO, the IDT and the
APIC. Today its 25 cases are still pure assertions (block-cache fields, VFS
path helpers, ext2 struct sizes, paging constants) — a second host-test layer
that happens to run inside QEMU. It is invoked directly from `entry()` on the
boot CPU, before any task is created, and reports over the serial line.

**Targets matter.** `cargo test` defaults to the *host* triple on purpose:
`x86_64-unknown-none` has no `std`, so a bare `cargo test` there fails with
"can't find crate for `test`" plus a missing `#[panic_handler]`. Anything that
produces kernel code must say `--target x86_64-unknown-none` (the Makefile
does; `make build` is the shortcut). There is deliberately no cargo alias for
this: cargo ignores an alias that shadows a built-in command.

### Fuzzing

```bash
make fuzz-smoke        # 2 000 cases, every commit
make fuzz-coverage     # 50 000 cases, hunting for new paths
make fuzz-regression   # replay recorded crashes (see the caveat)
```

`fuzz-regression` currently replays nothing: the crash log lives only in kernel
memory and `zenus_fuzz::init()` clears it, so the corpus is always empty at
boot. The run now says so explicitly (`[FUZZ] NO-CORPUS`) and exits `2` rather
than reporting a clean regression. It needs an on-disk crash corpus first.

The campaign runs *inside* the kernel: it replaces the shell, contains each
fault through `zenus_arch::fuzz_guard`, and prints a machine-readable report
(`[FUZZ] EXIT code=0|1|2`). Details in `doc/fuzzing.md`.

## Shell

86 builtins: 41 from `zutils/crates/` (`ls`, `cat`, `cp`, `grep`, `df`, `top`,
plus the `z*` diagnostics in `zutils/crates/native/`) and 45 kernel-side commands
(`tcp-*`, `udp-*`, `dhcp`, `resolve`, `fsck`, `journal-*`, `bcache`, `ns-*`,
`firewall-*`, `pkg-*`, `syslog`, `watchdog-*`, `lockdep-status`, …).

## License

MIT (see `Cargo.toml`).
