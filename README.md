# Zenus — Operating System Kernel in Rust

x86_64 OS kernel written in Rust. Bootable via Limine bootloader with QEMU.

## Architecture

Modular workspace with 8 crates:

| Crate | Description |
|---|---|
| `zenus-arch` | x86_64 architecture: APIC, ACPI, PCI, SMP, GDT, IDT, interrupts, keyboard, ATA, RTC |
| `zenus-mem` | Memory management: paging, frame allocator, heap allocator |
| `zenus-sched` | Scheduler & task management |
| `zenus-fs` | Filesystem: ext2, tmpfs, tarfs, devfs, VFS, block cache, journal, I/O scheduler |
| `zenus-net` | Networking: TCP, UDP, IP, ARP, DHCP, DNS, ICMP, RTL8139 driver |
| `zenus-syscall` | Syscall interface, ELF loader, file descriptors |
| `zenus-sync` | Synchronization: spinlock, IRQ guard |
| `zenus-console` | Console: VGA text mode, serial, logging |

## Quick Start

```bash
make qemu          # Run in QEMU
make run           # Build + run
make build         # Build only
make clean         # Clean artifacts
```

## Features

- [x] SMP boot (multi-core)
- [x] Preemptive scheduler
- [x] ext2 filesystem with journalling
- [x] TCP/IP stack (ARP, IP, ICMP, TCP, UDP, DHCP, DNS)
- [x] RTL8139 network driver
- [x] ELF user-space loader
- [x] System calls
- [x] ATA disk driver
- [x] PCI enumeration
- [x] ACPI power management
- [x] Memory paging (4-level)
- [x] Shell
