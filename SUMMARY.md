# Zenus OS — Current State

> Snapshot: 2026-10-04. Kept for orientation; the source of truth is the code
> and the other documents in this directory.

## Where things stand

Zenus boots via Limine on BIOS and UEFI, brings up four CPUs, mounts an ext2
filesystem read-write with journalling, runs a BSD-socket TCP/IP stack, executes
ring-3 programs through a 109-syscall interface, and gives you a shell with
about 90 builtins. An init system (PID 1) supervises services.

It is **pre-alpha**. The core is real; the security model is not finished.

## Facts worth knowing

| | |
|---|---|
| Syscalls | 109 implemented in a 256-slot table |
| Tasks | 128 max, 8 CPUs, work stealing |
| Time slice | 5 ticks (~50 ms) |
| TCP connections | 256 |
| Block cache | 512 sectors, 4-way, write-back |
| Kernel heap | 8 MiB static arena |
| Namespaces | 16 each: PID, UTS, mount, net, user, IPC |
| Host unit tests | 289 |
| In-kernel tests | 27 |
| `unsafe` blocks | 465 in the kernel, 472 tree-wide, plus 11 `unsafe impl` |

## The three things that would change the risk picture most

1. **SMAP/SMEP are enabled at boot** (fixed, see DEVLOG BUG-033). Kernel code
   may no longer read or write user pages without `stac`/`clac`.
2. **Privilege model is partial.** One capability set per task exists
   (`CAP_SYS_ADMIN`, `CAP_KILL`, `CAP_SETUID`/`CAP_SETGID`, `CAP_DAC_OVERRIDE`,
   `CAP_DAC_READ_SEARCH` enforced), but nothing can add a capability back, and
   `root` (euid 0) still bypasses file checks unless the capability is dropped.
   `prctl(PR_CAPBSET_READ/DROP)` works; `PR_SET_SECCOMP` still `-EINVAL`.
3. **No cryptography.** No TLS, no disk encryption, no secure boot. The
   `ZENUS_SSH/1.0` service is a homegrown line protocol with a weak keystream
   and a default password.

## Testing

```bash
make test-host   # 289 host unit tests, no VM
make test        # 27 in-kernel tests in QEMU
make fuzz-smoke  # in-kernel fuzzing campaign
```

Host tests cover pure logic and are where new features should get coverage.
The in-kernel suite is for hardware paths.

## Things that are views, not implementations

- `cgroup2`: `pids`/`memory` limits are enforced (a fork or `mmap` is refused
  over them); `cpu`/`io` are advertised but not, and the hierarchy is flat.
- `ZENUS_SSH/1.0`: not SSH.
- `userspace/`: hand-built ring-3 programs, no libc.
- Storage: PIO only, no AHCI/NVMe.

See `SECURITY.md` for the full picture and `ROADMAP.md` for what is planned.
