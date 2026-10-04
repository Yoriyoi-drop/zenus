# Zenus OS Architecture

A 64-bit x86 kernel written in Rust. Layered so that each crate only depends on
the ones below it, and so that the pure logic of each layer can be unit-tested
on the host without booting anything.

```
zenus-sync          spinlocks, IRQ guard, lockdep
     │
zenus-console  zenus-mem          output/logging        paging, heap, VMAs
     │                              │
zenus-arch ─────────────────────────┘   CPU, IDT/GDT, APIC, PCI, ACPI, SMP,
     │                                     ATA, keyboard, RTC, crash dump
zenus-fs   zenus-net   zenus-ns   zenus-virtio   zenus-sched
     │                                    │               │
     └──────────── zenus-syscall ─────────┴───────────────┘
                              │
                           apps  (entry, boot sequence, init, shell)
        zenus-fuzz   fuzzing engine, called from apps::entry
```

`apps` is the only entry point (`#![no_main]`, one `_start` → `entry()`).
`zenus-fuzz` is a library it calls: a fuzzing build is still `zenus`, compiled
with `--features fuzz-<mode>`, and `apps::entry` replaces the shell with the
campaign. Both link through `apps/src/linker.ld`.

## Layer 1 — Synchronization (`zenus-sync`)

| Component | Notes |
|---|---|
| `SpinLock<T>` | Atomic CAS with exponential backoff; masks interrupts on bare metal |
| `IrqGuard` | Scoped IF masking |
| `lockdep` | Lock-order graph; 64 classes, 256 edges, per-CPU depth 8 |

`lockdep` class IDs are **1-based**: `0` is the "not registered" sentinel, and
the first lock must not be handed it. `LockdepSnapshot::classes` is indexed by
class ID, so slot 0 is always empty. No production path registers classes yet —
lockdep is wired up and reported by `lockdep-status`, but nothing feeds it.

Everything is host-safe: interrupt masking, the CR8 read in `current_cpu` and
the UART diagnostics are `cfg(target_os = "none")`-gated with host twins, so
`cargo test` can exercise locks without a VM.

## Layer 2 — Memory (`zenus-mem`)

| Component | Notes |
|---|---|
| Frame allocator | Free-stack allocator, 16384 frames; regions come from Limine |
| Paging | 4-level, per-process address spaces via CR3, HHDM for physical access |
| Heap | Free-list allocator over an 8 MiB static arena |
| VMA table | 64 regions per process, mmap/munmap/mprotect bookkeeping |

The VMA table is the single copy of that logic (`zenus_mem::vma`); it used to
exist twice, and `mmap` used the copy in `zenus-sched`, so every fix existed in
two places. `find_free` is iterative with a bounded scan: the previous
recursive version looped forever on `size == 0` and could hand back a range that
straddled an existing mapping.

## Layer 3 — Scheduling (`zenus-sched`)

| Component | Notes |
|---|---|
| Scheduler | Preemptive round-robin, `TIME_SLICE` = 5 ticks (~50 ms at 100 Hz) |
| Tasks | 128 max, 8 CPUs, 64 zombies |
| SMP | Work stealing; a task runs on the CPU that created it |
| Signals | 64 signals, dispositions, handler frames |
| Init | PID 1: service registry, supervision, restart policies |

Stack layout is an invariant, not a detail. `TSS.RSP0` is `kernel_rsp_top`, so
the timer ISR pushes at the top of the task stack and descends ~160 bytes. Task
frames therefore live at `frame_base(stack_top)` = `stack_top - STACK_GUARD`
(32 KiB below the top); every constructor uses that helper, and
`stack_size_is_valid()` rejects stacks too small for it (below the guard the
subtraction underflows).

Tick path (`schedule_tick`): BSP-only PIC EOI → LAPIC EOI → `SYS_TICKS++`,
`pit::tick()`, `sysctl::sysctl_tick()` → early-out for APs and for single-task
systems → context switch. The timer source is the LAPIC timer; the PIT is kept
running because `get_ticks()` feeds uptime.

## Layer 4 — Filesystem (`zenus-fs`)

| Component | Notes |
|---|---|
| VFS | Mount table (per mount namespace), path resolution, Unix permission bits |
| ext2 | Read/write, journalling (123 entries), fsck |
| Block cache | 512 sectors, 4-way associative, write-back |
| tmpfs | 128 nodes, 1 KiB per file, `reset()` |
| devfs | Device nodes + block devices behind a `fn`-pointer vtable |
| tarfs | initrd (CPIO-style tar), procfs, cgroup2 view, sysctl, `.zpk` packages |

The block layer is exercised in host tests through a fake block device
registered in devfs, which covers the cache, the I/O scheduler and the journal
without any hardware.

The journal writes its header **through** the block cache (a direct device
write leaves the previous image cached, and the next `journal_begin` then
revives a stale header), and `journal_replay` flushes the redo data before
retiring the header — otherwise a crash in that window loses the transaction
with no journal left.

`cgroup.rs` is a read-only view of the cgroup v2 layout: create, unlink and
write all fail, because no controller is enforced behind it.

## Layer 5 — Networking (`zenus-net`)

| Component | Notes |
|---|---|
| Protocols | ARP, IPv4, ICMP, TCP, UDP, DHCP client + server, DNS |
| Sockets | BSD-style, 256 TCP connections |
| Routing | Static, longest-prefix match, 8 entries |
| Firewall | 32 rules, 64 connection-tracking entries, protocol/port/established matching |
| Driver | RTL8139 (PIO) |

TCP does its own congestion control and retransmission; the firewall's
`firewall_clear_connections` is an *age-based* evictor despite the name.

The NIC interrupt is routed to a fixed vector (`interrupts::NIC_VECTOR`).
Computing it as `32 + irq_line` put QEMU's rtl8139 (IRQ 10 → vector 42) on a
slot with no handler, so every RX interrupt was acknowledged and dropped.

## Layer 6 — System calls (`zenus-syscall`)

256 dispatch slots, 109 implemented: file I/O and descriptors, process
(creation/exec/exit/wait/clone), signals, memory (mmap/mprotect/munmap/brk),
time, sockets, filesystem extras, scheduling, IPC (shm/futex), namespaces
(`uname_ns`, `getpid_ns`) and resource limits.

The numbers were renumbered twice to remove collisions (pipe 22 → 111, dup 32 →
113, nanosleep 35 → 114, dup2 37 → 33, …). `userspace/` hard-codes some of
them; a host test pins those numbers against the kernel's list. It does not
read `userspace/`, so it catches kernel-side renumbering, not drift in the
userspace workspace.

`prctl` accepts most options and returns success without doing anything —
including `PR_SET_SECCOMP` and `PR_SET_NO_NEW_PRIVS`. That is a security gap,
not a bug to be papered over.

## Layer 7 — Console and diagnostics (`zenus-console`)

Serial (interrupt-driven), VGA text, framebuffer, a 256-entry dmesg ring, a
1024-entry syslog, and a catalog of 21 structured error codes (`ZN-XXX-NNNN`)
with severity, cause, actions and suggestion. Counters are per (module, number):
hashing only the digits merged every module's `*-0001` into one slot.

## Layer 8 — Namespaces (`zenus-ns`)

PID (local↔global mapping, 64 entries per namespace), UTS (hostname), mount
(mount table per namespace), net (interface bitmap), user (uid maps, one per
inner uid), IPC. 16 namespaces per kind. `mnt`, `net`, `user` and `ipc` can be
destroyed and refuse to destroy the root; `uts` and `pid` have no destroy path
at all.

## Layer 9 — Virtio (`zenus-virtio`)

virtio-net (up to 2 queue pairs, 64 buffers each), virtio-blk (with a real
FLUSH barrier, which the journal requires for durability), virtio-console,
virtio-balloon. 256-entry split virtqueues.

## Testing architecture

| Layer | Where | Runs |
|---|---|---|
| Host unit tests | `#[cfg(test)] mod host_tests` in each crate | `make test-host` |
| In-kernel tests | `#[cfg(feature = "testing")] mod tests`, registered in `apps/src/test_runner.rs` | `make test` (QEMU) |

| Fuzzing | `zenus-fuzz`, fault containment via `zenus_arch::fuzz_guard` | `make fuzz-*` |

Host tests are the ones that grow with new features: they run in milliseconds
and they can assert on things a QEMU run cannot (a syscall table has no
duplicate numbers; a path with 33 components is refused). The in-kernel suite
is for hardware paths.

## Where the invariants live

The bugs this kernel has had were mostly in three places, and each now has the
invariant documented next to the code that enforces it:

1. **Stack/frame layout** — `scheduler::frame_base`, `STACK_GUARD`.
2. **Interrupt vectors** — `zenus_arch::interrupts::{TIMER,SERIAL,NIC,SPURIOUS}_VECTOR`
   and `RESERVED_VECTORS`, used by the IDT and by the drivers — though
   `apps::entry` still passes the timer vector to `enable_tick_source` as a
   literal `32`, which is worth fixing.
3. **Bounds on parsed input** — every packet/structure decoder returns `None`
   rather than indexing past the buffer.
