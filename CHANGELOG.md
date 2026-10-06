# Zenus OS Changelog

## Unreleased

### Security
- **SMAP and SMEP are enabled at boot.** This was item 1 of `ROADMAP.md` and
  the top gap in `SECURITY.md`; it is done. Five separate defects had to be
  fixed first, and each one was a real bug on its own, independent of SMAP:
  - `zenus_arch::cpu::stac`/`clac` were declared `nomem`, which told LLVM the
    inline asm touches no memory and therefore permitted it to hoist a
    `read_volatile`/`copy_nonoverlapping` across the pair. Every user-memory
    copy in the syscall layer was inside that window and could be scheduled
    after the `clac`, so with SMAP on it faulted. The `mov cr4` guard around
    them was also pointless: `STAC` sets AC whether or not SMAP is enabled.
  - `sys_write`'s stdout/stderr path read the user buffer byte-by-byte through a
    raw pointer with **no `stac` at all** — its comment said as much ("SMAP
    disabled, so the kernel can access user memory directly"). It now streams in
    fixed chunks through the direct map, which also means an unmapped buffer is
    `EFAULT` instead of a ring-0 page fault.
  - `signal::setup_signal_frame`/`restore_signal_frame` pushed and popped the
    `ucontext` frame through raw user pointers, and never checked the frame was
    mapped. A signal handler could wedge the machine from the timer handler.
  - The page-fault handler read faulting stacks with a bare `read_volatile` and
    no translation check, so it faulted *itself*: a nested `#PF` re-entered the
    handler and the machine wedged in a silent loop printing one header. It now
    walks the tables and reads through the HHDM, which is supervisor-only and so
    needs no `stac` at all.
  - `sys_execve` built the new image's initial user stack while the **old** CR3
    was still loaded. `load_elf` randomises `stack_top`, so those writes went to
    an address the loaded CR3 did not map. The shell's `run` command did switch
    CR3 but left interrupts enabled across the window, so a timer tick could
    run the ISR on the half-built address space. Both now go through
    `zenus_syscall::userstack::write_initial_user_stack`.

### Fixed
- `clone_user_address_space` zeroed each new page table with
  `write_bytes(ptr, 0, 512 * 8)` against a `*mut u64`. `write_bytes` counts
  *elements*, so that is 4096 elements = 32 KiB per table instead of 4 KiB —
  three tables per `fork`, so every fork zeroed 24 KiB past the end of a page
  table it had just taken from the frame allocator. `page_table_bytes()` makes
  the count a byte count, and `a_page_table_is_exactly_one_page` pins it.
- The page-fault dump mislabelled five of its eight fault-type encodings. It
  indexed the error code as present?/user?/write?, when bit 1 is W/R and bit 2
  is U/S — so `0x1`, a supervisor read of a *present* page, printed as
  "supervisor-write-nonpresent". That is what a SMAP violation looks like, and
  it is why this took as long as it did to find. It now also prints
  `[SMAP: supervisor touched a user page without stac]` when that is the case.
- `make test` and `make test-quiet` did not pass `-cpu max`. QEMU's default
  `qemu64` model has no SMEP and no SMAP, so the in-kernel suite was silently
  not testing them at all.

### Observability
- The heap allocator verifies its own free list on every `alloc` and `dealloc`
  and names the first thing wrong (bad magic, non-ascending chain, a block
  outside the arena, a `size` below `MIN_BLOCK`). The corruption report now also
  says whether the pointer is inside the arena and which free block encloses it.
  Both were added for BUG-034, which is still open: one user program runs, then
  the free list is corrupt and no further program can start. `DEVLOG.md` records
  what is ruled out (the frame allocator, `alloc_mut`'s own writes) and what the
  next step is.

### Testing
- Added a host unit-test layer: `#[cfg(test)] mod host_tests` inside the
  kernel crates, run with `make test-host` / `cargo test --workspace`. 151
  tests across 11 crates, covering VMA arithmetic, packet parsing, permission
  bits, syscall numbering, the journal, the fuzzer's bookkeeping, namespaces,
  the error-code catalog and the initial user-stack layout.
- Removed the workspace-wide default cargo target. `cargo test` could not run
  against `x86_64-unknown-none` (no `std`, therefore no test harness); kernel
  builds now always pass `--target x86_64-unknown-none` explicitly. CI runs
  the host suite and lints the kernel target separately.
- `make build` now exists (the README documented it; the target did not).

### Fixed
- Task frames were built at the top of the stack in `create_user_task`, inside
  the area `TSS.RSP0` points at, so the first timer tick overwrote the saved
  user context. All constructors share `frame_base()` now, and
  `stack_size_is_valid()` rejects stacks too small for it (below the guard the
  subtraction underflowed to ~2^64).
- NIC interrupts were routed to `32 + irq_line` while the handler is installed
  at a fixed vector, so every RX interrupt was acknowledged and dropped.
  Vectors are now shared constants used by both the IDT and the drivers.
- `VmaTable::find_free` recursed forever on `size == 0`, overflowed on large
  inputs, and could return a range overlapping an existing mapping.
- ext2 structure decoders read past short buffers.
- `.zpk` entry paths could escape the install prefix via `..`, and a rejected
  image still created a package directory. Records were also read by casting
  byte offsets to `#[repr(C)]` pointers, which is undefined behaviour when a
  payload length is not a multiple of 4.
- VFS silently truncated paths longer than 32 components, resolving a
  *different* file than requested.
- `access_check` did not require the execute bit for "other" on directories.
- `dns::parse_response` read one byte past the buffer on a truncated
  compression pointer; `ipv4::parse` accepted other protocol versions and
  ignored header options when validating the checksum.
- `lockdep` gave class id 0 to the first lock, which is also the "unregistered"
  sentinel, so it never checked it.
- Error-code counters merged every module's `*-0001` into one slot, and the
  code parser dropped all two-letter `ZN-FS-*` codes.
- `journal_replay` retired the header before flushing the redo data; a crash in
  that window lost the transaction with no journal left.
- `journal_init`/`journal_replay` wrote the header straight to the device,
  leaving a stale cached copy that the next `journal_begin` wrote back.
- `virtio-blk`'s flush reported failure when no virtio-blk was present, which
  wedged `journal_commit` and every later `journal_begin`; its completion poll
  used a bare `hlt`, which never wakes with interrupts disabled.
- The fuzzer's corpus cursor advanced twice per call, so a campaign replayed
  the first input forever; the minimiser looped forever on inputs containing a
  zero byte.
- `idle_until` could never return to its caller's frame, making the fuzzing
  watchdog's abort unreachable — a timeout could not be reported.
- `sysctl_init` appended a duplicate copy of every default on each call;
  `kernel.uptime` divided 100 Hz ticks by 1000 and was never incremented.
- `tmpfs` removed non-empty directories, orphaning their children, and handed
  out `&'static mut` for its node table.
- PIC EOI was issued from application processors, re-aiming the interrupt line.
- `userspace/{syscall_core,pipe_test}` called `SYS_PIPE = 22`, which is
  `SYS_ACCESS` after the syscall renumbering.
- The cgroup2 filesystem reported success for create/unlink/write on a
  read-only view.

### Changed
- Unsynchronised globals are now atomics or locked: journal state, the route
  table, tmpfs nodes, error counters.
- `block_cache::evict_one` no longer overwrites the hash slot when a window is
  full; it prefers an empty slot, then a clean one.
- Removed dead code: the duplicated VMA table in `zenus-sched`, and
  `zenus-fs/{cred,initrd}.rs` plus `zenus-net/checksum.rs`, none of which were
  ever compiled.

### Docs
- Rewritten against the tree: `README.md`, `AGENTS.md`, `ARCHITECTURE.md`,
  `SECURITY.md`, `ROADMAP.md`, `CONTRIBUTING.md`, `SUMMARY.md`,
  `doc/fuzzing.md`, plus status banners on `audit.md`, `DESIGN.md` and
  `AETHER.md`.
- The previous `ROADMAP.md` claimed Phases 2 and 3 were 100 % complete,
  including a capability system, KPTI, encryption, NAT and incremental
  backups — none of which exist in this tree — and counted the 25 in-kernel
  tests as the whole suite.
- `ROADMAP.md` now separates what is done, what is partial, and what is a view
  rather than an implementation (the cgroup2 tree, ZENUS_SSH, the container
  story), and lists the next six items in priority order.
- `audit.md`, `DESIGN.md` and `AETHER.md` are now labelled as what they are: a
  dated audit snapshot, and two design proposals for things that do not exist.

## Version 0.1.0 - Pre-Alpha (2026-06-25)

### Overview
First public release of Zenus OS. Pre-Alpha quality with core foundation complete (100% Phase 1, 100% Phase 2, 100% Phase 3, 10% Phase 4).

### Major Changes
- Initial kernel release with full foundation (Phase 1, 2, 3 complete)
- Excellent architecture foundation with excellent modularity and educational value
- Core infrastructure (boot, memory, scheduler, filesystem, networking) complete
- User mode execution working (Ring 3 via SYSCALL/SYSRET)
- Production-grade server infrastructure complete (SSH, services, supervision)
- virtio drivers and multi-queue NIC support
- Container namespaces (PID + UTS) ready for network control
- Comprehensive networking stack (TCP, UDP, DHCP, DNS, routing)
- ext2 filesystem with journaling and crash recovery
- 25 unit tests across critical systems

### Breaking Changes
- None (first public release)

### New Features
#### Core Infrastructure
- Full Limine bootloader support (BIOS + UEFI)
- SMP boot with per-CPU data structures
- Preemptive round-robin scheduler
- 4-level paging with user/kernel isolation
- Hardware drivers: ATA, keyboard, RTC

#### Storage
- ext2 filesystem (read-write with journaling)
- Block cache (64-entry LRU write-back)
- fsck for crash recovery
- Virtual filesystem (VFS) with mount table

#### Networking
- Complete TCP/IP stack (RFC 793 compliant)
- 11/11 TCP states implemented
- UDP, ICMP, DHCP client/server
- IPv4 routing with longest-prefix match
- BSD socket API (22 syscalls implemented)

#### User Space
- User mode task execution (Ring 3)
- ELF loader with ASLR support
- File descriptor management
- Shell with 30+ commands

#### Security
- Unix permission model (UID/GID, mode bits)
- Address space layout randomization
- User pointer validation
- Access control lists for files

#### Services
- Init system (PID 1 process manager)
- Service supervision with auto-restart
- SSH server (ZENUS_SSH/1.0)
- Package manager (.zpk format)
- Sysctl kernel parameter interface

#### Reliability
- Watchdog system (30s timeout)
- Crash dump functionality
- Deadlock detection (lockdep)
- Syslog with 4096-entry buffer

### Known Issues
- Missing syscalls (fork, exec, pipe, signal handling)
- No user/kernel isolation (no SMAP/SMEP, no KPTI)
- ATA PIO-only driver (poor performance)
- Networking driver PIO-only (poor performance)
- Limited concurrency (128 tasks, 16 TCP connections)
- No dynamic memory management (swap, OOM)
- Shell runs in kernel space (security risk)
- No package manager in user space
- No SSH server in production
- No firewall support
- No secure boot
- No capability system
- Minimal security features

### Highlights from Audit
**Phase 1 (Foundation): ✅ 100% Complete**
- User mode execution verified with "Hello from user mode!" message
- Per-process address spaces implemented with CR3 switching
- Verified ELF loader with proper user page table setup
- ext2 filesystem functional: root, hello.txt, subdir read/write
- Block cache working with proper LRU logic
- 25 unit tests across block_cache, VFS, ext2, paging

**Phase 2 (Networking & Security): ✅ 100% Complete**
- TCP stack 11/11 RFC 793 states
- Full BSD socket API
- DHCP client/server with IP allocation
- DNS stub resolver
- IP routing table with longest-prefix match
- User/group model (UID/GID, syscalls 100-105)

**Phase 3 (Server Infrastructure): ✅ 100% Complete**
- SSH server (ZENUS_SSH/1.0)
- Init system with service lifecycle
- Package manager (.zpk format)
- Initrd script execution with startup.sh
- Sysctl interface for 8 kernel parameters
- Service supervision with health checks
- Reliable logging (4096-entry syslog)
- Watchdog system (30s timeout)
- Crash dump with backtrace (16 frames)

**Phase 4 (Cloud & Production): ✅ 10% Complete**
- Virtio drivers (net, blk, balloon, console)
- Multi-queue NIC support (virtio-net VIRTIO_NET_F_MQ)
- Container namespaces (PID + UTS isolation)

**Production Score: 41% (+15% Phase 3 complete, +1.5% Virtio drivers, +1% namespaces)**

## Installation and Usage

### Quick Start
1. Install Rust (nightly-2026-05-01)
2. Build with `make run` (QEMU)
3. Test with `make test` (QEMU with ext2 image)

### Building
```bash
# Build kernel only
make build

# Build ISO for QEMU (BIOS + UEFI)
make iso

# Build HDD image
make img

# Run in QEMU
make run

# Run with GDB debugging
make run-qemu-gdb

# Run unit tests
make test

# Clean build artifacts
make clean
```

### Testing
```bash
# Automated tests (requires QEMU)
make test

# Verbose test output
make test-quiet
```

### SSH Access
Once running in QEMU, SSH server listens on port 22 within QEMU user networking.

## System Administration

### Shell Commands
Basic shell with 30+ commands:

#### File System
- `ls` - List directory contents
- `cat <file>` - Display file contents
- `mkdir <dir>` - Create directory
- `rm <file>` - Remove file/directory
- `touch <file>` - Create empty file
- `mount` - List mounted filesystems

#### Process Management
- `ps` - List running processes
- `kill <pid>` - Terminate process
- `whoami` - Show current user
- `id` - Show user/group ID
- `uname` - Show system information
- `uptime` - Show system uptime
- `meminfo` - Show memory information

#### Networking
- `ifconfig` - Show network interfaces
- `dmesg` - Show kernel messages
- `netstat` - Network statistics

### Configuration

#### Kernel Parameters
- `zenus-sysctl get <param>` - Get kernel parameter
- `zenus-sysctl set <param> <value>` - Set kernel parameter

#### Available Parameters
- `hostname` - System hostname
- `log_level` - Logging verbosity
- `version` - Kernel version
- `uptime` - System uptime
- `max_tasks` - Maximum tasks
- `watchdog_timeout` - Watchdog timeout
- `ip_forward` - Enable IP forwarding
- `dns.server` - DNS server address

## Technical Details

### Kernel Architecture
- **Language**: Rust (edition 2021, nightly-2026-05-01)
- **Target**: x86_64-unknown-none (bare metal)
- **Bootloader**: Limine (BIOS + UEFI)
- **Scheduler**: Preemptive round-robin (50-tick quantum)
- **Paging**: 4-level with higher-half mapping
- **User Space**: ELF loader with ASLR

### System Call Interface
```c
// 22 system calls implemented (out of ~300 typical)
0: SYS_READ      - Read from file descriptor
1: SYS_WRITE     - Write to file descriptor
2: SYS_OPEN      - Open file
3: SYS_CLOSE     - Close file descriptor
4: SYS_STAT      - Get file status
5: SYS_READDIR   - Read directory entries
8: SYS_LSEEK     - Change file offset
16: SYS_IOCTL    - Device-specific commands
32: SYS_DUP      - Duplicate file descriptor
35: SYS_NANOSLEEP - Sleep with nanosecond precision
39: SYS_GETPID    - Get process ID
45: SYS_BRK       - Manage heap
60: SYS_EXIT     - Exit process
63: SYS_UNAME    - Get system information
100-105: SYS_GETUID...SYS_SETGID - User/group ID management
```

### Filesystem
- **Primary**: ext2 (read-write with journaling)
- **In-memory**: tmpfs (128 nodes, 4KB files)
- **Devices**: devfs (null, zero, console, serial)
- **Initrd**: tarfs (read-only)

### Networking
- **Stack**: IPv4/TCP/UDP/ICMP with DHCP/DNS
- **Transport**: RTL8139 PIO driver
- **Virtual**: Loopback interface
- **Socket API**: BSD-style

### Security
- **Memory**: Rust-based with targeted unsafe blocks
- **Access**: Unix permissions with UID/GID checking
- **Isolation**: Separate address spaces per process
- **Randomization**: ASLR for heap and stack

## Development

### Building from Source
```bash
# Install dependencies
cargo install cargo-watch
rustup target add x86_64-unknown-none

# Build kernel
cargo build --target x86_64-unknown-none

# Build with testing feature for unit tests
cargo build --target x86_64-unknown-none --features testing
```

### Running Tests
```bash
# Test with QEMU (default)
make test

# Test with verbose output
make test-quiet

# Custom test configuration
make test SMP=8
```

### Contributing
1. Fork the repository
2. Create a feature branch
3. Implement changes with tests
4. Submit a pull request
5. Ensure code passes CI checks

### Testing Your Changes
Run unit tests with:
```bash
cargo test --features testing
```

Integration tests require QEMU:
```bash
make test
```

## Performance

### Boot Performance
- **Initial Boot**: ~1-2 seconds in QEMU
- **With Tests**: ~5-10 seconds
- **Serial**: Kernel messages through serial port

### Memory Usage
- **Kernel Heap**: 16MB fixed size
- **Tasks**: Max 128 concurrent tasks
- **TCP Connections**: Max 16 connections
- **Block Cache**: 32KB (64 entries × 512 bytes)

### Scalability Limitations
- **Single Core**: Basic scheduler
- **Multi-core**: Per-CPU data, no load balancing
- **I/O**: PIO-only drivers
- **Storage**: No DMA, poor throughput

## Known Issues and Limitations

### Security
- No user/kernel memory access controls (SMAP/SMEP)
- No KPTI (Kernel Page Table Isolation)
- Shell runs in kernel space
- Limited process isolation

### Performance
- ATA PIO driver (~3-5 MB/s)
- RTL8139 PIO driver (~10 Mbps)
- 5-second context switch quantum
- No huge pages support

### Functionality
- Missing: fork, exec, pipe, signal syscalls
- Missing: shared libraries (no libc)
- Missing: cgroups v2
- Missing: overlayfs
- Missing: full container support

## Future Plans

### Phase 1 (Foundation) ✅ COMPLETE
- User mode execution
- Per-process address spaces
- ELF loader
- Basic disk filesystem

### Phase 2 (Networking & Security) ✅ COMPLETE
- Enhanced TCP/IP stack
- Security hardening
- Platform security

### Phase 3 (Server Infrastructure) ✅ COMPLETE
- Server services
- System management
- Storage & persistence

### Phase 4 (Cloud & Production) ✅ IN PROGRESS
- VirtIO drivers
- Container namespaces
- Cloud integration

### Phase 5 (Enterprise Production) 🔄 PLANNING
- High-availability clusters
- Enterprise security
- Cloud orchestration

## Legal

### License
Apache License 2.0

### Dependencies
- Rust x86_64 crate (0.15) - https://crates.io/crates/x86_64
- Limine bootloader - https://github.com/limine-bootloader/limine

### Trademark
Zenus OS is a work in progress. All rights reserved.

## Contact
- GitHub: https://github.com/whale-d/zenus
- Issues: Report bugs and feature requests
- Discussions: Community support

## Acknowledgements
Thanks to:
- The Rust community for the language and tooling
- Limine bootloader team for open-source Limine
- QEMU team for excellent emulation
- All contributors and testers

*This changelog is a work in progress. Contributions are welcome!*