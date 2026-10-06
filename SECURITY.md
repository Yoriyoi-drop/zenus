# Zenus OS Security Guide

## Posture in one paragraph

Zenus is an educational kernel. It has real memory-safety discipline in the
Rust sense (ownership, no `unsafe` without a comment explaining it) and real
effort in its parsers, but its **security model is not complete**. SMEP and
SMAP are enabled at boot, which closes the largest hole in the list below, but
there is still no KPTI, no capability system and no driver isolation. Do not
deploy this on anything you care about.

## What is actually enforced

| Control | State |
|---|---|
| User/kernel address separation | Yes — per-process CR3, `USER_SPACE_LIMIT = 0x0000_8000_0000_0000` |
| SMEP / SMAP | **Enabled at boot**, in `apps::entry`. Needs a CPU (or QEMU `-cpu max`) that has them |
| KPTI | Not implemented |
| Unix permission bits (uid/gid/mode) | Yes, in `vfs::access_check` |
| Path confinement | Components are bounded (`MAX_PATH_SEGMENTS`), over-long paths are refused, `..` cannot escape a mount prefix |
| Package install confinement | `.zpk` entry paths are validated before anything is written |
| Syscall table | Fixed 256 slots, no number registered twice, unknown numbers return -1 |
| Pointer validation on copy-in | `validate_user_range` + per-page `virt_to_phys_raw` revalidation |
| NX | Set on user mappings without `PROT_EXEC` |
| Heap/stack guard | `STACK_GUARD` at the top of every task stack; `stack_size_is_valid` refuses undersized stacks |

## The gaps that matter

### 1. No KPTI

SMEP and SMAP are on. What is *not* on is page-table isolation from user
mappings: a user CR3 keeps the kernel half mapped, and a `%cr3` write or a
kernel bug that lands in ring 0 while a user CR3 is loaded still finds the
whole kernel image mapped. KPTI (or PCID-based user/entry trampolines) is the
usual answer and is not implemented.

Two supporting rules the kernel relies on, both enforced by the source rather
than by the hardware:

* **Supervisor code pages are supervisor.** `ensure_kernel_pages_supervisor`
  clears the U/S bit at all four levels of every present entry in the boot
  address space before SMEP is switched on, so no ring-0 instruction fetch can
  come from a user page.
* **Supervisor access to user pages goes through `stac`/`clac`.** See
  `zenus_arch::cpu::stac` for why that pair is memory-opaque and must never be
  marked `nomem`, and `zenus_syscall::userstack::write_initial_user_stack` for
  the one place allowed to run with a foreign CR3 loaded.

### 2. No privilege separation beyond uid/gid

- No capabilities. `prctl` answers `-EINVAL` for `PR_SET_SECCOMP`,
  `PR_SET_NO_NEW_PRIVS`, `PR_SET_MEMBARRIER` and friends — it used to return
  success and do nothing, which let a program believe it was sandboxed.
  `PR_SET_NAME`/`PR_GET_NAME` are implemented; `PR_SET_PDEATHSIG` and
  `PR_SET_KEEPCAPS` are accepted as documented no-ops.
- `root` (euid 0) bypasses every permission check.
- No LSM, no audit log of security decisions. `syslog` and the error-code
  counters record *events*, not *decisions*.

### 3. No cryptography, and one protocol that looks like it has some

There is no crypto library. Consequences:

- No disk encryption, no TLS, no secure boot, no signed modules.
- `zenus-net/src/ssh.rs` implements `ZENUS_SSH/1.0`: a line protocol with a
  homegrown XOR keystream, a password comparison with `constant_time_eq`, and a
  **default password of `zenus`** (overridable at build time with the
  `ssh_password` feature). It is not SSH, it is not authenticated in any
  meaningful sense, and its keystream carries roughly 2 bits of entropy per
  byte. A test pins that weakness deliberately: changing it would break the
  wire format, and hiding it would be worse.

## Input handling

This is where most of the real work has gone, and where the tests live:

- Every packet parser (ARP, IPv4, TCP, UDP, ICMP, DNS, DHCP) is bounds-checked
  and returns `None` rather than indexing past the buffer. DNS response parsing
  in particular used to read one byte past the end on a truncated compression
  pointer.
- `ipv4::parse` rejects non-IPv4 versions and validates the checksum over the
  *whole* header, including options.
- ext2 structure decoders refuse short buffers instead of reading past them.
- The `.zpk` reader copies fields at explicit offsets with no alignment
  assumption, and validates the entire image before touching the filesystem, so
  a hostile package cannot create anything at all.
- The VFS refuses paths with more than `MAX_PATH_SEGMENTS` components rather
  than truncating them (truncation would resolve a *different* file).

## Concurrency

- `SpinLock` masks interrupts on bare metal, so a lock held across an
  interrupt handler that takes the same lock deadlocks the CPU. New code called
  from the ISR path must be checked against that.
- `lockdep` can detect lock-order inversions (it has the graph and the
  reverse-edge check) but nothing registers lock classes yet.
- Global state that used to be unsynchronised is now atomic or locked: journal
  state, the route table, tmpfs nodes, error counters.

## Memory safety

465 `unsafe` blocks remain in the kernel crates and `apps` (472 tree-wide),
plus 11 `unsafe impl`. Most of them are unavoidable: MMIO, port I/O,
IDT descriptors, inline assembly for context switching and the syscall return
path, and ELF loading. The parsers that used to be unsafe have been converted
to bounds-checked code. Run `cargo clippy --all-targets --all-features
--target x86_64-unknown-linux-gnu` before submitting.

## Before you call a change secure

- [ ] User/kernel separation still holds; no new unchecked user pointer
- [ ] Any new parser bounds-checks every index, including one-past-the-end
- [ ] Anything reachable from an interrupt handler does not take a lock a task
      can hold, and does not allocate
- [ ] Any new syscall number is not already in use (`syscall_count()` /
      `registered_syscalls()` in `zenus-syscall` assert this)
- [ ] Path and archive handling refuses `..`, over-long paths and absolute
      paths where they are not wanted
- [ ] `make test-host` (`cargo test --workspace --target x86_64-unknown-linux-gnu`)
      passes, and a test exists for the new behaviour
- [ ] `make test` (QEMU) still passes if you touched MMIO, the IDT or paging
- [ ] `make fuzz-smoke` if you touched a parser

## Reporting a vulnerability

Open a private security advisory on the repository rather than a public issue.
Include the reproduction, the QEMU command line, and any serial log. Fix
priority is driven by exploitability, not by how interesting the bug is.
