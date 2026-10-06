# Zenus OS Roadmap

## Where we are

**Pre-alpha.** The kernel boots, runs user programs, mounts filesystems, serves
the network and runs an init system with supervision. SMEP/SMAP are on. The
security model is still not finished, and a few subsystems are views onto data
rather than the real thing (see "Not real yet").

The previous version of this file claimed Phases 2 and 3 were 100 % complete,
including a capability system, KPTI, encryption, NAT and incremental backups.
None of those exist. This version lists what is built, what is partially built,
and what is missing, checked against the tree rather than against intention.

## Done

**Boot and hardware** — Limine (BIOS + UEFI), HHDM, SMP bring-up, PCI
enumeration, keyboard, RTC, LAPIC/IOAPIC/PIT, crash dump. ACPI exists only as
the shutdown path: `acpi::init()` is never called at boot.

**Memory** — 4-level paging, per-process address spaces, frame allocator,
free-list heap, VMA tracking for mmap/munmap/mprotect.

**Scheduling** — preemptive round-robin with work stealing across CPUs,
signals, PID 1 with service supervision and restart policies.

**System calls** — 109 syscalls in a 256-slot table, ELF loader, ring-3
execution, file descriptors, namespaces (`uname_ns`, `getpid_ns`).

**Filesystem** — ext2 read/write with write-ahead journalling and fsck, block
cache, tmpfs, devfs, tarfs initrd, procfs, sysctl, a `.zpk` package manager
with path confinement.

**Network** — ARP, IPv4, ICMP, TCP (congestion control, retransmission), UDP,
DHCP client and server, DNS, static routing, a stateful firewall, BSD sockets,
RTL8139, virtio-net.

**Virtualisation** — virtio-net, virtio-blk, virtio-console, virtio-balloon.

**Namespaces** — PID, UTS, mount, net, user, IPC.

**Security** — SMEP and SMAP enabled at boot, per-process address spaces,
`STACK_GUARD`, `USER_SPACE_LIMIT`, NX on user mappings.

**Observability** — structured error-code catalog, dmesg ring, syslog, lockdep,
watchdog, `zbench`/`zdiag`/`zdoctor`/`ztrace`.

**Testing** — 217 host unit tests (`make test-host`), 25 in-kernel tests
(`make test`), in-kernel fuzzing campaigns (`make fuzz-*`), CI running both.

## Partially done

- **User-mode hardening** — SMAP/SMEP are **enabled**; KPTI is not, so a
  user CR3 still has the whole kernel mapped.
- **Container story** — the six namespace types exist and work, but there is no
  OCI runtime, no cgroup enforcement (the cgroup2 tree is read-only) and no
  overlayfs.
- **Service management** — the init system supervises and restarts services, but
  there is no dependency ordering, no health checking beyond "still running",
  and no declarative manifests.
- **Shell** — ~90 builtins, no job control, no pipelines, no redirection.

## Known broken

- **`meminfo`'s frame lines do not add up.** Recycling is fixed (BUG-036), so
  "Free stack" now moves from 0 to 24 after a user program runs and is reaped.
  "Used" stays 0, which is correct at the two points it is sampled — before
  anything is mapped, and after everything is freed — but nothing samples it
  while frames are actually outstanding, so the line is untested rather than
  wrong.
- **`mmap` is capped at 16384 pages.** `sys_mmap` budgets against
  `free_frames_count()`, which still returns the recycled stack's depth rather
  than what `alloc_frame` can serve, so large mappings are refused on a machine
  with 2 GiB spare. An availability limit, not a safety one.
- **The frame allocator's second pass advances `regions[i].base` without
  shrinking `length`,** so a region's claimed end grows by a page every time it
  serves one. The repair (a per-region cursor and a fixed floor) was attempted
  three times and reverted each time; BUG-038 records the mutation results. The
  blocker is `reserve_region`'s reshape branches being untested, which is
  ordinary missing work rather than a hard limit.
- **`reap_terminated_stacks()` is unreachable**, and `TERMINATED_STACKS` is only
  filled by `task_exit()`, which no syscall reaches. `kill_task` and `reap_task`
  each free task stacks on their own path, with nothing keeping those paths
  consistent.

## Not real yet

Do not build on these without reading the code:

| Thing | Reality |
|---|---|
| cgroups | Read-only view of the v2 layout; create/write fail |
| SSH | `ZENUS_SSH/1.0`, a homegrown line protocol with a weak keystream and a default password |
| Capabilities / seccomp | `prctl` returns success and does nothing |
| Container runtime / OCI | Does not exist |
| Cryptography | No library at all |
| Userspace | Ring-3 programs are built from this workspace by hand; no libc |
| Drivers | Monolithic, PIO-only, no hotplug, no AHCI/NVMe/USB |

## Next, in order

1. **Close the parser holes.** `make fuzz-coverage` now runs clean at 50 000
   cases (`crashes=0`), and the harness reports its verdict correctly (BUG-037).
   The remaining work is audit findings, not fuzzer findings — `DEVLOG.md`
   tracks them, ranked by impact.
2. **Real cgroup enforcement** — at least `memory` and `pids` — so the existing
   namespace work has teeth.
3. **KPTI**, so a user CR3 does not have the kernel half mapped. SMEP/SMEP
   being on (done) is not the same thing.
4. **A driver model** so storage stops being PIO-only: AHCI first.
5. **Capabilities and a real `prctl`**, replacing the current no-ops.
6. **Boot testing**: a QEMU smoke test in CI that boots to the shell prompt and
   fails the build if it does not.

## Explicitly not planned

Live migration, Kubernetes support, and the enterprise-certification items from
the previous version of this file. They are not credible goals for a project
this size and listing them made the roadmap useless.
