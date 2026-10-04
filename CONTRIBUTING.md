# Zenus OS Contributing Guide

## Welcome to Zenus OS

Thank you for your interest in contributing to Zenus OS! This document provides guidelines for developers who want to contribute to this educational operating system kernel written in Rust.

## Project Overview

Zenus OS is a pre-alpha kernel project designed to teach modern kernel development concepts. We're looking for contributors who understand:

- Rust systems programming
- Operating system concepts
- Low-level x86_64 architecture
- Concurrency and synchronization

## Project Status

Pre-alpha. The previous number here (18.5 %) came with a claim that Phases 2
and 3 were complete — including a capability system, KPTI and encryption, none
of which exist. See `ROADMAP.md` for what is actually built.

**Critical remaining work**, in order:
- SMAP/SMEP: implemented, disabled at boot, blocking on a PML4 U/S bug
- Privilege model: no capabilities, `prctl` is a no-op
- Container runtime: namespaces exist, cgroups are a read-only view

## How to Contribute

### 1. Understanding the Project

#### Key Characteristics
- **Educational Focus**: Code is intentionally simple and understandable
- **Minimal Dependencies**: Only `x86_64` crate as external dependency
- **Rust-Based**: memory safety by ownership where possible — 465 `unsafe` blocks remain (see `SECURITY.md`)
- **Pre-Alpha**: Not production-ready, expect frequent changes

#### Development Philosophy
- **Layered Architecture**: Clear separation of concerns
- **Modular Design**: Each subsystem in its own crate
- **Bare-Metal**: No standard library (std::os::windows/unix)
- **Learning First**: Comments explain "why" not just "what"

### 2. Running Tests

#### Prerequisites
```bash
cargo install cargo-watch # Optional, for watching changes
rustup target add x86_64-unknown-none
```

#### Building Tests
```bash
make test-host      # cargo test --workspace --target x86_64-unknown-linux-gnu
make test           # in-kernel suite inside QEMU
make fuzz-smoke     # in-kernel fuzzing campaign
```

#### Targets
There is deliberately **no default cargo target**. `cargo build` and
`cargo test` build for the host; anything producing kernel code passes
`--target x86_64-unknown-none` (the Makefile does).

`cargo test` therefore works out of the box. Running it against the bare-metal
triple cannot work: that target has no `std`, so there is no test harness to
link ("can't find crate for `test`", plus a missing `#[panic_handler]`).

#### Test Structure
Two layers, and they are not interchangeable:

- **Host unit tests** — `#[cfg(test)] mod host_tests` inside the kernel
  crates. Pure logic: VMA arithmetic, packet parsing, permission bits, syscall
  numbering, journal replay, fuzzing bookkeeping. They must never touch
  `cli`/`sti`, port I/O, CR0/CR3/CR8, MSRs or MMIO — all of those fault in
  ring 3. Where a bare-metal path needs one, it is
  `#[cfg(target_os = "none")]`-gated with a host twin.
- **In-kernel tests** — `#[cfg(feature = "testing")] pub mod tests` returning
  `Result<(), &'static str>`, registered in `apps/src/test_runner.rs`. The only
  way to test MMIO, the IDT and the APIC. 25 tests today.

**Current coverage**: 151 host tests across 11 crates, 25 in-kernel tests.

#### Writing Tests
- Global kernel state is shared by every test in the process (block cache,
  devfs, VFS, sysctl, procfs, journal, route table, firewall, corpus,
  coverage, lockdep). Guard those tests with a `static SERIAL: SpinLock<()>`
  guard.
- If state leaks between tests, add a reset API (`TmpFs::reset`,
  `procfs::reset_sources`, `bc_invalidate_all`, …) instead of writing a test
  that depends on ordering.
- Assert the requirement, not the implementation. A test that asserts the
  current value of something wrong passes until someone fixes the bug.
- If a test finds a bug, fix the bug and keep the test as the regression.

### 3. Development Workflow

#### Workflow for New Features

##### Phase 1: Design
1. **Identify Requirements**: Determine what needs to be added
2. **Check Architecture**: See if it fits existing layers
3. **Propose Solution**: Document design decisions
4. **Get Approval**: Discuss with maintainers

##### Phase 2: Implementation
1. **New Crate**: Start with new crate if needed
2. **Integration Points**: Identify existing integration points
3. **Safety First**: Focus on memory safety
4. **Testing Early**: Write unit tests before integration

##### Phase 3: Integration
1. **Boot Test**: Verify functionality survives boot
2. **End-to-End Test**: Test with real use cases
3. **Performance**: Check for performance regressions
4. **Documentation**: Update relevant docs

#### Workflow for Bug Fixes

1. **Reproduce**: Create a minimal reproduction case
2. **Analyze**: Use panic messages or logs
3. **Fix**: Make minimal, safe changes
4. **Test**: Add regression tests
5. **Document**: Update issue tracker

### 4. Code Review Process

#### Pull Request Guidelines
- **Small PRs**: One clear change per PR
- **Tests First**: Always include tests for new code
- **Safety Checks**: No breaking changes to public API
- **Documentation**: Update relevant documentation

#### Review Criteria
- **Memory Safety**: No use-after-free, buffer overflows
- **Logic Correctness**: Handles edge cases properly
- **Performance**: No significant regressions
- **Integration**: Works smoothly with existing code

### 5. Development Environment Setup

#### Local Development
```bash
# Clone and enter project
cd /path/to/zenus

# Quick test build
cargo build --target x86_64-unknown-none

# Run the host unit tests
make test-host

# Run the in-kernel suite (needs QEMU)
make test

# Build and run in QEMU
make run-gui
```

#### Cross-Compilation
```bash
# Target is x86_64-unknown-none
cargo build --target x86_64-unknown-none

# With testing feature for unit tests
cargo build --target x86_64-unknown-none --features testing
```

### 6. Testing Your Changes

#### Unit Tests
Add them next to the code as `#[cfg(test)] mod host_tests` (pure logic) or
`#[cfg(feature = "testing")] pub mod tests` (needs the kernel), and register
kernel tests in `apps/src/test_runner.rs`.

#### Integration Tests
Build your changes into the kernel and test in QEMU:
```bash
make build  # Build kernel
make run    # Run in QEMU
```

#### Manual Testing
Use the shell commands (ls, cat, ps, etc.) to verify functionality.

### 7. Build System

#### Makefile
```bash
# Build only the kernel
make build

# Build ISO for QEMU
make iso

# Build HDD image
make img

# Run in QEMU (BIOS)
make run

# Run in QEMU (UEFI)
make run-uefi

# Run the in-kernel suite in QEMU
make test

# Run the host unit tests
make test-host

# Run a fuzzing campaign
make fuzz-smoke

# Clean everything
make clean
```

#### Custom Build System Details
- **Rust Nightly**: Requires nightly-2026-05-01
- **Linker**: ld.lld with custom linker script
- **Bootloader**: Limine (BIOS + UEFI support)
- **Target**: x86_64-unknown-none (bare metal)

### 8. Common Development Tasks

#### Fixing Common Bugs

##### Kernel Panic Recovery
```rust
// The panic handler lives in apps/src/lib.rs and already dumps via
// zenus_console::kpanic_code!. The crash-dump API it uses is:
use zenus_arch::crash::{crash_dump_init, crash_dump_save, crash_dump_print};

crash_dump_init();   // called once from apps::entry
crash_dump_save();   // on a fatal fault
crash_dump_print();  // to the serial console
```

##### User Mode Crashes
```rust
// User stack overflow protection
// Add guard pages in ELF loader (zenus-syscall/src/elf.rs)
// Implement proper user page fault handling (zenus-arch/src/interrupts/idt.rs)
```

#### Performance Optimizations

##### Memory Management
- Implement slab allocator for frequent small allocations
- Add huge page support for memory-intensive tasks
- Implement file-based swap for OOM scenarios

##### Scheduling
- Implement priority queues for better task ordering
- Add load balancing for SMP systems
- Implement real-time scheduling classes

### 9. Getting Help

#### Asking Questions
1. **Include Context**: What you're trying to achieve
2. **Show Code**: Paste relevant code snippets
3. **Reproduction Steps**: How to reproduce the problem
4. **Expected vs Actual**: Clear description of results

#### Reporting Issues
1. **Issue Template**: Use GitHub issue templates
2. **Bug Reports**: Include stack traces, reproduction steps
3. **Feature Requests**: Explain use case and design
4. **Security Issues**: Handle via private channels

## Development Roadmap

Do not keep a second roadmap here. `ROADMAP.md` is the one, and it is checked
against the tree: what is done, what is partial, what is a view rather than an
implementation, and the next six things in priority order. The list that used
to live in this file predated the work (it called fork/exec/pipe/signals and
dynamic memory management missing — all shipped — and promised live migration
and cloud orchestration, which `ROADMAP.md` now lists as explicitly not
planned).

## Code of Conduct

There is no `CODE_OF_CONDUCT.md` in the tree yet. Until one exists, the short
version: technical critique of the code is welcome and expected, personal
attacks are not.

## License

MIT — see the `license` field in `Cargo.toml`. There is no `LICENSE` file in
the tree yet; adding one is a small open task.

## Acknowledgements

Thank you to all contributors, especially:
- The Rust community for the language and tooling
- Limine bootloader team for open-source Limine
- QEMU team for excellent emulation
- everyone who filed a bug that turned out to be real

*This guide is a work in progress. Corrections welcome — especially the parts
that contradict the code.*