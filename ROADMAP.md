# Zenus OS Roadmap

## Where we are

**Pre-alpha.** The kernel boots, runs user programs, mounts filesystems, serves
the network and runs an init system with supervision. The security model is not
finished, and a few subsystems are views onto data rather than the real thing
(see "Not real yet").

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

**Observability** — structured error-code catalog, dmesg ring, syslog, lockdep,
watchdog, `zbench`/`zdiag`/`zdoctor`/`ztrace`.

**Testing** — 198 host unit tests (`make test-host`), 25 in-kernel tests
(`make test`), in-kernel fuzzing campaigns (`make fuzz-*`), CI running both.

## Partially done

- **User-mode hardening** — SMAP/SMEP are implemented and disabled; the PML4
  U/S interaction that breaks them is not understood yet. This is the single
  highest-value fix available.
- **Container story** — the six namespace types exist and work, but there is no
  OCI runtime, no cgroup enforcement (the cgroup2 tree is read-only) and no
  overlayfs.
- **Service management** — the init system supervises and restarts services, but
  there is no dependency ordering, no health checking beyond "still running",
  and no declarative manifests.
- **Shell** — ~90 builtins, no job control, no pipelines, no redirection.

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

1. **Fix and enable SMAP/SMEP.** Understand the PML4 U/S interaction; make the
   userspace tests pass with them on. Everything else in this list is worth
   less.
2. **Close the parser holes.** Fuzz every decoder (`make fuzz-coverage`) and
   treat each crash as a Critical until it is not. `DEVLOG.md` tracks the audit
   findings that are not fixed yet, ranked by impact.
3. **Real cgroup enforcement** — at least `memory` and `pids` — so the existing
   namespace work has teeth.
4. **A driver model** so storage stops being PIO-only: AHCI first.
5. **Capabilities and a real `prctl`**, replacing the current no-ops.
6. **Boot testing**: a QEMU smoke test in CI that boots to the shell prompt and
   fails the build if it does not.

## Explicitly not planned

Live migration, Kubernetes support, and the enterprise-certification items from
the previous version of this file. They are not credible goals for a project
this size and listing them made the roadmap useless.
