# AGENTS.md - Zenus OS

## Overview

Zenus is an x86_64 kernel written in Rust (`no_std`). It boots via Limine
(BIOS + UEFI), runs under QEMU, and has an interactive shell, an in-kernel
fuzzing framework and a host-side unit-test suite.

State: **pre-alpha**. Core subsystems work; the security model is incomplete
(see `SECURITY.md`) and several subsystems are deliberately shallow (see
"Known shallow spots" below).

## Build and run

```bash
make build          # kernel -> build/zenus (cargo build --target x86_64-unknown-none)
make iso            # bootable ISO
make run-gui        # QEMU with a window
make run-serial     # QEMU headless, serial on stdio
make run-tcp        # serial over TCP (nc localhost 45678)
```

**Target discipline matters.** The repo has *no* default cargo target on
purpose. `cargo build`/`cargo test` therefore build for the host, and anything
producing kernel code passes `--target x86_64-unknown-none` explicitly (the
Makefile does). `x86_64-unknown-none` has no `std`, so `cargo test` there
fails with "can't find crate for `test`" and a missing `#[panic_handler]`.
Do not "fix" this by adding `[build] target` back, and do not add a cargo
alias for it either: cargo silently ignores an alias that shadows a built-in
command.

## Testing

Two layers:

```bash
make test-host      # host unit tests: cargo test --workspace
make test           # in-kernel suite inside QEMU (apps/src/test_runner.rs)
make fuzz-smoke     # in-kernel fuzzing campaign
```

- **Host tests** are `#[cfg(test)] mod host_tests` modules inside the kernel
  crates, run for the host triple. They cover pure logic and must never touch
  privileged instructions: `cli`/`sti`, `in`/`out` port I/O, CR0/CR3/CR8, MSRs
  and MMIO all fault in ring 3. Where a bare-metal path needs one of those, it
  is `#[cfg(target_os = "none")]`-gated with a host twin.
- **In-kernel tests** are `#[cfg(feature = "testing")] pub mod tests` modules
  returning `Result<(), &'static str>`, registered in
  `apps/src/test_runner.rs`. Only this layer can test MMIO/IDT/APIC.

Conventions:

- Host tests use `#[test]` and `assert!`. Kernel tests use `test!()` and return
  `Result`. The two are named differently (`host_tests` vs `tests`) on purpose.
- Module-global kernel state (block cache, devfs, VFS, sysctl, procfs, journal,
  route table, firewall, corpus, coverage, lockdep) is shared by every test in
  the process. Guard such tests with a `static SERIAL: SpinLock<()>`.
- Cross-process state that leaks between tests needs a reset API
  (`TmpFs::reset`, `procfs::reset_sources`, `bc_invalidate_all`, …); add one
  rather than writing a test that depends on ordering.
- A new feature comes with a test. If a test finds a bug, fix the bug and keep
  the test as the regression.

## Architecture

Layered crates, bottom to top: `zenus-sync` → `zenus-console`/`zenus-mem` →
`zenus-arch` → `zenus-fs`/`zenus-net` → `zenus-sched` → `zenus-syscall`, with
`apps` as the entry point and `zenus-fuzz` as an alternate entry point.

Boot order in `apps::entry`: Limine/HHDM → frame allocator → paging → IDT →
APIC (timer) → keyboard + serial IRQ → scheduler → VFS mounts → namespaces →
PCI → virtio → ATA → ext2 (`/mnt`, `/virtio`) → journal replay → network →
SMP bring-up → init system → shell task → `loop { scheduler::idle() }`.

Three things worth knowing before touching the scheduler:

- `TSS.RSP0` points at `kernel_rsp_top`, so the timer ISR descends from the top
  of each task stack. Task frames live `STACK_GUARD` bytes below it
  (`scheduler::frame_base`); every constructor must use that helper.
- `SpinLock::lock()` masks interrupts on bare metal. Anything called from an
  ISR must not take a lock that a task can hold.
- Interrupt vectors are shared constants in `zenus_arch::interrupts`
  (`TIMER_VECTOR`, `SERIAL_VECTOR`, `NIC_VECTOR`, `SPURIOUS_VECTOR`,
  `RESERVED_VECTORS`). A driver that routes to a vector not in that list gets
  its interrupt acknowledged and dropped.

## Known shallow spots

Do not assume these work; read the code first.

- **SMAP/SMEP are implemented but disabled** at boot (`apps/src/lib.rs`, the
  commented-out `enable_smep_smap`). They fault in userspace programs because
  writing the user stack through HHDM with `stac` is unreliable; the suspected
  cause is PML4 U/S handling in `create_address_space` / `map_user_page_raw`.
  Until that is fixed the kernel can touch user memory freely.
- No KPTI, no capabilities, no secure boot, no crypto library.
- `zenus-fs/src/cgroup.rs` is a **read-only view** of the cgroup v2 layout:
  create/unlink/write all fail, no controller is enforced.
- `zenus-net/src/ssh.rs` speaks a homegrown `ZENUS_SSH/1.0` line protocol with a
  hand-rolled keystream and a default password of `zenus`. It is not SSH.
- `zenus-fuzz`'s `minimizer::minimize` uses a predicate that cannot reproduce a
  crash (it only refuses to shrink to nothing); use `minimize_with` with a real
  oracle.
- `io_scheduler::io_stats()` returns a total and two hardcoded zeros.
- The ZENUS_SSH keystream has ~2 bits of entropy per byte (pinned by a test).

## Docs

`ARCHITECTURE.md` (layers), `SECURITY.md` (threats and gaps), `ROADMAP.md`
(phases), `CONTRIBUTING.md` (workflow), `doc/fuzzing.md` (campaign modes),
`DESIGN.md` / `AETHER.md` (proposed executable formats and CLI design — design
documents, not descriptions of the current code).

## Style

- Comments explain *why*, especially when the obvious implementation is wrong.
  Several invariants here were learned the hard way and are documented at the
  code that enforces them; keep that up.
- Prefer a pure helper over logic that can only be tested in a VM. That is how
  `frame_base`, `stack_size_is_valid`, `install_path_for`, `minimize_with` and
  `error::catalog` came to exist.
- Keep `unsafe` blocks local and justified; several exist to talk to hardware
  and say so in a comment.
