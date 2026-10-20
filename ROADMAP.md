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

**Testing** — 261 host unit tests (`make test-host`), 25 in-kernel tests
(`make test`), in-kernel fuzzing campaigns (`make fuzz-*`), a headless boot
smoke test that requires the *ordinary* kernel to reach the shell prompt
(`make smoke`), CI running all of them.

## Partially done

- **User-mode hardening** — SMAP/SMEP are **enabled**; KPTI is not, so a
  user CR3 still has the whole kernel mapped.
- **Container story** — the six namespace types exist and work, and cgroup
  `pids` and `memory` limits are now enforced; there is still no OCI runtime, no
  `cpu`/`io` enforcement, and no overlayfs.
- **Service management** — the init system supervises and restarts services and
  now starts them in dependency order (`service_register_with_deps`); there is
  still no health checking beyond "still running" and no declarative manifests.
- **Shell** — ~90 builtins, no job control, no pipelines, no redirection.

## Known broken

- **`meminfo`'s `Used` line is untested rather than wrong.** Recycling is fixed
  (BUG-036) and the free count is now exact (BUG-039), so the memory lines add
  up: `Total` is what survived the reservations and `Free` is every frame
  `alloc_frame` can still serve. `Used` stays 0, which is correct at the two
  points it is sampled — before anything is mapped, and after everything is
  freed. A host test in `zenus-mem/src/used_memory_tests.rs` now exercises the
  `used_memory` accounting across alloc/free, so the line has a test behind it
  (the number itself was always correct when sampled live).
- **The frame allocator's second pass is fixed.** It used to advance
  `regions[i].base` without shrinking `length`, so a region's claimed end grew by
  a page every time it served one and the first pass eventually handed out memory
  that does not exist. It now keeps `regions` read-only and tracks a `floor` plus
  a per-region `low_cursor`; and the seeding decision is the pure `plan_regions`,
  which `global_init` and `FrameAllocator::new` both call. `DEVLOG.md` BUG-038 has
  the account, including the mutation check (15 of 16 caught, the survivor a dead
  store).
- **`reap_terminated_stacks()` is unreachable**, and `TERMINATED_STACKS` is only
  filled by `task_exit()`, which no syscall reaches. `kill_task` and `reap_task`
  each free task stacks on their own path, with nothing keeping those paths
  consistent.

  **Fixed (BUG-042):** `task_exit()`, `TERMINATED_STACKS`, and
  `reap_terminated_stacks()` were dead code — `sys_exit` uses
  `exit_current_task()` and `reap_task()` frees the kernel stack directly. The
  unreachable path and its deferred-free machinery were removed; the teardown
  paths are now `exit_current_task` → `reap_task` and `kill_task` only.

## Not real yet

Do not build on these without reading the code:

| Thing | Reality |
|---|---|
| SSH | `ZENUS_SSH/1.0`, a homegrown line protocol with a weak keystream and a default password |
| Capabilities / seccomp | `PR_SET_NO_NEW_PRIVS` is tracked and inherited; `PR_SET_SECCOMP` fails with `-EINVAL`; no capability set yet |
| Container runtime / OCI | Does not exist |
| Cryptography | No library at all |
| Userspace | Ring-3 programs are built from this workspace by hand; no libc |
| Drivers | Monolithic, PIO-only, no hotplug, no AHCI/NVMe/USB |

## Next, in order

1. **Close the parser holes.** `make fuzz-coverage` now runs clean at 50 000
   cases (`crashes=0`), and the harness reports its verdict correctly (BUG-037).
   The remaining work is audit findings, not fuzzer findings — `DEVLOG.md`
   tracks them, ranked by impact.

**Real cgroup enforcement is done for `pids` and `memory`** (BUG-041):
`pids.max` refuses a task creation and `memory.max` refuses an `mmap`, both
checked against a pure, host-tested controller table. `cpu`/`io` remain
advertised but unenforced, and the hierarchy is flat. Next:

2. **KPTI**, so a user CR3 does not have the kernel half mapped. SMEP/SMEP
   being on (done) is not the same thing.
3. **A driver model** so storage stops being PIO-only: AHCI first.
4. **Capabilities and a real `prctl`**, replacing the current no-ops.

Boot testing is done: `make smoke` boots the ordinary kernel headless and fails
unless the shell prompt appears, and `make test-quiet` now fails when the
in-kernel suite fails and returns instead of hanging (it did neither before).

## Explicitly not planned

Live migration, Kubernetes support, and the enterprise-certification items from
the previous version of this file. They are not credible goals for a project
this size and listing them made the roadmap useless.
