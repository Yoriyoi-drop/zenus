# Zenus OS Architecture

## Overview
Zenus OS is a 64-bit x86 operating system kernel written in Rust, designed as an educational platform for understanding modern kernel development. It follows a modular, layered architecture that separates concerns across multiple crates while maintaining Rust's memory safety guarantees.

## Project Structure

### Workspace Components
The entire project is organized as a Cargo workspace with the following key crates:

- `apps/` - Entry point, boot sequence, shell, user mode, and main integration
- `crates/zenus-arch/` - x86 architecture and hardware abstraction
- `crates/zenus-mem/` - Memory management and paging
- `crates/zenus-sched/` - Task scheduling and management
- `crates/zenus-fs/` - Filesystem implementation and virtual filesystem
- `crates/zenus-net/` - Networking stack
- `crates/zenus-syscall/` - System call interface
- `crates/zenus-sync/` - Synchronization primitives
- `crates/zenus-console/` - Console and logging
- `crates/zenus-virtio/` - VirtIO virtualization drivers
- `crates/zenus-ns/` - Namespace implementation for containers
- `zutils/crates/` - User-space utilities (coreutils-style commands)

### Architecture Layers

#### Layer 1: Core Infrastructure
**Crate: `zenus-arch`**

| Component | Purpose |
|-----------|---------|
| CPU & Peripherals | x86_64 instruction set, CPUID, APIC, PCI, ACPI |
| Interrupt Handling | IDT, IRQ routing, exception handling |
| Memory Management Unit | GDT, TSS, descriptor tables |
| Hardware Drivers | Keyboard, ATA disk, RTC, timer |
| Boot Protocol | Limine integration for BIOS/UEFI boot |

**Key Features:**
- Implements complete x86_64 privileged instruction requirements
- Hardware initialization and detection
- Basic device driver model (driver-specific, not abstracted)
- ACPI and power management hooks

#### Layer 2: Memory Management
**Crate: `zenus-mem`**

| Component | Purpose |
|-----------|---------|
| Frame Allocator | Physical memory allocation, buddy algorithm (internal) |
| Virtual Memory | 4-level page tables, address space management |
| Kernel Heap | Free-list allocator with 16MB fixed size |
| Paging | Page mapping, user/kernel space isolation |

**Current State:**
- **Physical Memory**: Bump allocator with free stack (4096 frames capacity)
- **Virtual Memory**: 4-level paging with PDE/PTE separation
- **User Space**: Separate address spaces via CR3 switching
- **Limitations**: No page reclaim, swapping, or huge pages

#### Layer 3: Process Scheduling
**Crate: `zenus-sched`**

| Component | Purpose |
|-----------|---------|
| Task Management | Process control block, state management |
| Scheduler | Preemptive round-robin with 50-tick quantum |
| SMP Support | Per-CPU data, basic multi-core support |
| Context Switching | Register save/restore, stack validation |

**Current State:**
- **Tasks**: Max 128 concurrent tasks
- **Scheduler**: Simple round-robin, no priorities
- **SMP**: No load balancing, tasks born on CPU 0
- **Context Switch**: ~100 APIC ticks per switch

#### Layer 4: Filesystem
**Crate: `zenus-fs`**

| Component | Purpose |
|-----------|---------|
| Virtual Filesystem | Mount table, path resolution, permissions |
| ext2 | Read-write ext2 filesystem with journaling |
| tmpfs | In-memory filesystem (128 nodes, 4KB max file) |
| devfs | Device filesystem interface |
| tarfs | Initrd extraction |

**Current State:**
- **ext2**: Complete implementation with journaling and fsck
- **Journal**: Write-ahead journaling (16 blocks = 8KB)
- **Block Cache**: 64-entry LRU write-back cache
- **Permissions**: Unix mode bits with UID/GID checking

#### Layer 5: Networking
**Crate: `zenus-net`**

| Component | Purpose |
|-----------|---------|
| Network Stack | TCP, UDP, IP, ICMP, routing, DHCP |
| Hardware Drivers | RTL8139 PIO driver, VirtIO drivers |
| Application Protocols | SSH, DNS, DHCP client/server |
| Socket API | BSD socket interface |

**Current State:**
- **TCP**: Full RFC 793 implementation (11 states), 16-connection limit
- **UDP**: Datagram handling with basic socket API
- **ICMP**: Echo reply support
- **DHCP**: Client and server implementations
- **Routing**: Static routing with longest-prefix match
- **Drivers**: Single RTL8139 PIO driver, VirtIO implementation

#### Layer 6: System Calls
**Crate: `zenus-syscall`**

| Component | Purpose |
|-----------|---------|
| Syscall Dispatch | 128-slot interrupt-based dispatch |
| System Call Interface | User mode interaction via SYSCALL/SYSRET |
| ELF Loader | User program loading and memory mapping |
| File Descriptors | POSIX file descriptor management |

**Current State:**
- **Implemented**: 22 system calls (out of ~300 typical Linux)
- **Missing**: `fork`, `exec`, `pipe`, `signal` handling
- **Security**: User pointer validation, ASLR support

#### Layer 7: Synchronization
**Crate: `zenus-sync`**

| Component | Purpose |
|-----------|---------|
| Spinlock | IRQ-safe atomic operations with exponential backoff |
| IRQ Guard | Interrupt context protection |
| Lockdep | Deadlock detection and prevention |
| Priority Inversion | Basic protection mechanisms |

#### Layer 8: Console & Logging
**Crate: `zenus-console`**

| Component | Purpose |
|-----------|---------|
| Serial Port | NS16550A UART driver (COM1)
| VGA Console | Text mode display |
| Syslog | Structured logging with 4096-entry buffer |
| DMESG | Circular debug log (256 entries × 128 bytes) |

## Development Model

### Code Quality
- **Memory Safety**: Rust's ownership system with targeted unsafe blocks
- **Testing**: Unit tests (25 total) with testing feature flag
- **Build**: Custom Makefile over Cargo
- **Documentation**: Minimal inline comments only

### Key Design Decisions

#### 1. Single Address Space Kernel
All kernel code runs at CPL0 (Ring 0). User/kernel isolation exists at the page table level but not at the privilege level. This simplifies the architecture while providing basic separation.

#### 2. User Space via SYSCALL/SYSRET
Rather than using traditional sysret-based user mode, Zenus uses explicit `SYSCALL`/`SYSRET` instructions via MSR interface. This allows fine-grained control but requires careful user/exceptions handling.

#### 3. Bare-Metal Approach
Minimal dependencies: only `x86_64` crate for architecture primitives. This maximizes control but increases development complexity.

### Current Limitations

#### Architecture
- No KPTI (Kernel Page Table Isolation)
- Fixed 16MB kernel heap
- No huge pages for performance
- Single-core scheduler (no load balancing)

#### Security
- No SMAP/SMEP (memory access control)
- Limited privilege separation
- No address space layouts (KASLR)
- Basic filesystem permissions only

#### Performance
- PIO-only storage driver (ATA, RTL8139)
- Small block cache (32KB total)
- Round-robin scheduling (5-second quantum)
- Limited concurrency (128 tasks, 16 TCP connections)

## Development Guidelines

### Coding Standards
1. **Memory Safety**: Prefer safe Rust, audit all unsafe blocks
2. **Error Handling**: Minimal error types, focus on kernel correctness
3. **Separation**: Strict module boundaries, no cross-contamination
4. **Testing**: Comprehensive unit tests for critical paths

### Adding New Features
1. **Modular Design**: New functionality in dedicated modules
2. **Layer Adherence**: Respect existing abstraction layers
3. **Integration**: Minimal changes to core systems
4. **Testing**: Unit tests before integration

## Future Architecture Improvements

### Phase 1: Foundation
- User/kernel privilege separation (SMAP/SMEP)
- Proper user-mode syscall handling
- Dynamic memory management (swap, OOM)
- Advanced filesystem journaling (ext4-style)

### Phase 2: Networking & Security
- Full TLS/SSL support
- Firewall and packet filtering
- Secure RPC mechanisms
- Memory protection keys (MPK)

### Phase 3: Production
- Container runtime support
- Live migration capabilities
- Fault tolerance and recovery
- Performance monitoring and optimization

## Contributing to Architecture

This architecture serves as a living document. Contributors are encouraged to:
- Update this document with new components
- Document design decisions and trade-offs
- Track implementation progress against architecture
- Maintain consistency across all layers