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
| Host unit tests | 151 |
| In-kernel tests | 25 |
| `unsafe` blocks | 465 in the kernel, 472 tree-wide, plus 11 `unsafe impl` |

## The three things that would change the risk picture most

1. **SMAP/SMEP are implemented and switched off.** They fault in userspace
   programs, and the PML4 U/S interaction behind that is not understood. Until
   it is fixed, kernel code can read and write user memory freely.
2. **No privilege model.** No capabilities, `prctl` is a no-op, `root` bypasses
   everything.
3. **No cryptography.** No TLS, no disk encryption, no secure boot. The
   `ZENUS_SSH/1.0` service is a homegrown line protocol with a weak keystream
   and a default password.

## Testing

```bash
make test-host   # 151 host unit tests, no VM
make test        # 25 in-kernel tests in QEMU
make fuzz-smoke  # in-kernel fuzzing campaign
```

Host tests cover pure logic and are where new features should get coverage.
The in-kernel suite is for hardware paths.

## Things that are views, not implementations

- `cgroup2`: read-only layout, create/write fail, no enforcement.
- `ZENUS_SSH/1.0`: not SSH.
- `userspace/`: hand-built ring-3 programs, no libc.
- Storage: PIO only, no AHCI/NVMe.

See `SECURITY.md` for the full picture and `ROADMAP.md` for what is planned.
