# AGENTS.md - Zenus OS Production Review

## Overview

This file contains the comprehensive production review configuration for Zenus OS, an x86_64 server kernel written in Rust. The project is currently at **pre-alpha** stage (approximately 18.5% production readiness) with core infrastructure like a scheduler, memory manager, filesystem, networking stack, and user mode support already functional.

## Architecture

### Kernel Architecture Overview

#### Layer 1: Core Infrastructure
- **Architecture** (`zenus-arch`): x86_64 details, APIC, ACPI, PCI, SMP, GDT, IDT, interrupts, drivers (keyboard, ATA, RTC)
- **Console** (`zenus-console`): VGA text mode, serial, logging, kernel messages
- **Synchronization** (`zenus-sync`): Spinlock, IRQ guard, lockdep for deadlock detection

#### Layer 2: Resource Management
- **Memory** (`zenus-mem`): Paging, frame allocator, heap allocator, virtual memory
- **Scheduler** (`zenus-sched`): Preemptive round-robin, task management, SMP support

#### Layer 3: Storage and File Systems
- **Filesystem** (`zenus-fs`): ext2 (read-write), tmpfs, devfs, tarfs, VFS
- **Block Cache**: 64-entry LRU write-back cache

#### Layer 4: Networking
- **Network Stack** (`zenus-net`): IPv4/TCP/UDP/ICMP, DHCP, DNS, routing, RTL8139 driver
- **VirtIO**: virtio-net, virtio-blk, virtio-balloon drivers

#### Layer 5: User Interface and Services
- **Syscall** (`zenus-syscall`): Basic Linux-compatible system call interface
- **Userspace**: ELF loader, user-mode task execution via SYSCALL/SYSRET
- **Init System** (`zenus-sched/init.rs`): PID 1 process manager, service supervision

### Key Features Implemented

#### Core Infrastructure ✅
- **Boot**: Limine bootloader (BIOS + UEFI support)
- **SMP**: Multi-core support with APIC timer, per-CPU data
- **Memory**: 4-level paging with user/kernel space isolation
- **Interrupts**: IDT with all 32 exceptions + IRQ support

#### Storage ✅
- **Filesystems**: ext2 (read-write), journaling, fsck, block cache
- **Device Model**: devfs with ATA driver for disk access

#### Networking ✅
- **Stack**: Full TCP (3-way handshake, retransmissions), UDP, ICMP, DHCP server/client
- **Routing**: Static routing with longest-prefix match
- **Network Interface**: RTL8139 PIO driver + virtio networking

#### User Space ✅
- **Process Model**: User-mode tasks via SYSCALL/SYSRET, ring 3 support
- **System Calls**: 22 implemented, 22 missing (fork, exec, pipe, signals etc.)
- **Services**: Init system with supervision, crash recovery

### Critical Missing Components ⚠️

#### Security (Critical Issues)
- User/kernel isolation: No SMAP/SMEP, no KPTI
- Capability system: Absent
- Authentication/Authorization: Not implemented
- Secure boot: Not supported
- Memory safety: Extensive unsafe blocks without audit

#### Process Management (Critical)
- No fork/exec system calls
- Missing pipe IPC
- No signal handling
- No shared libraries (no libc)

#### Storage (Critical)
- Driver model: Monolithic drivers, no hotplug
- ATA implementation: PIO-only (performance issues)
- No virtualization drivers beyond virtio

#### Cloud & Production (Critical)
- Container namespaces: Partial (PID + UTS only)
- Cgroups v2: Missing
- OCI runtime: Not implemented
- Docker compatibility: None

#### Developer Experience (High)
- Documentation: Minimal (this audit.md is primary documentation)
- Build system: Custom Makefile (no standardized CI/CD)
- Testing: 25 unit tests (limited coverage)
- API stability: Not versioned

### Target Audience

**Primary Purpose**: Educational operating system for understanding kernel development in Rust
**Not suitable for**: Production workloads, cloud infrastructure, mission-critical systems

## Notes

### Current Strengths
- Rust-based memory safety
- Modular crate architecture
- Detailed TCP/IP implementation
- ext2 journaling and crash recovery
- BSD socket API familiarity

### Current Limitations
- User/kernel isolation model incomplete
- Syscall interface very limited
- No modern networking security features
- Development tools not standardized
- Performance limited by PIO-only drivers

### Recommended Next Steps

1. **Phase 1 Complete** (6-12 months): User/kernel isolation, complete syscall set
2. **Phase 2 Focus** (12-18 months): Container support, cgroups, secure boot
3. **Phase 3 Vision** (18-24 months): Cloud readiness, virtualization integration