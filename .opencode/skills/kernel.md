# Kernel Review Skill

This skill specializes in reviewing and improving Zenus OS kernel architecture, design, and implementation. It focuses on:

## Core Responsibilities
- Analyze kernel architecture and design patterns
- Review system layering and component interactions
- Assess resource management and isolation mechanisms
- Evaluate hardware abstraction and driver models
- Optimize performance and scalability

## Review Focus

### System Architecture
- Layer 1: Core Infrastructure (arch, console, sync)
- Layer 2: Resource Management (memory, scheduler)
- Layer 3: Storage and File Systems
- Layer 4: Networking (TCP/IP, UDP, ICMP)
- Layer 5: User Interface and Services (syscalls, user-mode execution)

### Design Patterns
- Component modularity and separation of concerns
- Resource isolation and protection
- Performance optimization strategies
- Hardware abstraction layers
- Interrupt handling and synchronization

### Technical Review
- Memory management (4-level paging, space isolation)
- Scheduler algorithms (preemptive round-robin, SMP support)
- Filesystem architecture (ext2, tmpfs, devfs, VFS)
- Networking stack (TCP, UDP, ICMP, routing)
- User-mode task execution (SYSCALL/SYSRET, ring 3)

## Output Format
- Architecture assessment (Pass/Fail with evidence)
- Design pattern evaluation
- Performance optimization opportunities
- Hardware compatibility assessment
- Upgrade path recommendations

## Integration
Connects with:
- Production review skill for holistic assessment
- Architecture documentation (ARCHITECTURE.md, SUMMARY.md)
- Build configuration (Cargo.toml, Makefile)
- Hardware-specific considerations (x86_64, ACPI, APIC)
