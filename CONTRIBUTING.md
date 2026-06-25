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

Current Production Readiness: **18.5%**
- **Phase 1 (Foundation)**: 100% complete
- **Phase 2 (Networking & Security)**: 100% complete  
- **Phase 3 (Production)**: 10% complete (virtio + namespaces)

**Critical Remaining Work**:
- User/kernel isolation (security)
- Complete syscall set (fork, exec, pipe, signals)
- Dynamic memory management
- Container runtime support

## How to Contribute

### 1. Understanding the Project

#### Key Characteristics
- **Educational Focus**: Code is intentionally simple and understandable
- **Minimal Dependencies**: Only `x86_64` crate as external dependency
- **Rust-Based**: Full memory safety via Rust's ownership system
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
# Build with testing feature
cargo build --features testing

# Run unit tests
make test

# Run in QEMU with test overlay
make test
```

#### Test Structure
Tests use Rust's built-in test framework. Most tests run automatically during boot.

**Current Test Coverage**:
- Block cache LRU logic: ✅
- VFS path resolution: ✅
- ext2 filesystem constants: ✅
- Paging operations: ✅
- **25 total unit tests**

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

# Run tests
make test

# Build and run in QEMU
make run
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
Add tests in `tests/` directory or within modules using `#[cfg(test)]`.

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

# Run unit tests in QEMU
make test

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
// In panic handler (apps/src/lib.rs)
use zenus_arch::crash::panic_dump;

// Add better panic handling
syscall_dispatch() should catch panics and dump to serial
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

### Short Term (0-6 months)
- Complete missing syscalls (fork, exec, pipe, signals)
- Implement dynamic memory management
- Add proper user/kernel isolation
- Improve filesystem performance

### Medium Term (6-18 months)
- Implement cgroups and resource controls
- Add container runtime support
- Implement live migration
- Add performance monitoring

### Long Term (18+ months)
- Full virtualization support
- Cloud orchestration integration
- Production-grade networking
- Comprehensive security hardening

## Code of Conduct

This project follows:
- **Respectful Communication**: No harassment or intimidation
- **Constructive Feedback**: Technical, not personal
- **Inclusion**: Welcome diverse perspectives
- **Learning**: Everyone learns together

## License

Apache License 2.0 - See LICENSE file for details

## Acknowledgements

Thank you to all contributors, especially:
- The Rust community for the language and tooling
- Limine bootloader team for open-source Limine
- QEMU team for excellent emulation
- The documentation for inspiring this guide

*This contributing guide is a work in progress. Please suggest improvements!*