## 2. Security

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| User privilege model | PARTIAL | CRITICAL |
| Permissions | ✅ FUNCTIONAL | MEDIUM |
| Authentication | NOT IMPLEMENTED | CRITICAL |
| Authorization | NOT IMPLEMENTED | CRITICAL |
| Capability system | ✅ PARTIAL | HIGH |
| Secure boot | NOT IMPLEMENTED | MEDIUM |
| Kernel hardening | PARTIAL | HIGH |
| ASLR | PARTIAL | HIGH |
| Stack protection | PARTIAL | MEDIUM |
| Memory safety | PARTIAL | MEDIUM |
| Sandboxing | ✅ PARTIAL (PID + UTS namespaces) | HIGH |
| Audit logging | NOT IMPLEMENTED | HIGH |
| Encryption support | NOT IMPLEMENTED | HIGH |

### Detail
- **Privilege model:** Ring 3 berfungsi (SYSCALL/IRETQ) dengan per-process CR3. SMEP/SMAP aktif. KPTI masih parsial (stage 1-2 dibangun, belum aktif). Semua task default root; capability set per task ada, tetapi belum ada permitted/effective/inheritable split.
- **Permissions:** ✅ **FUNCTIONAL — uid/gid/euid/egid di Task struct.** File permissions dengan owner/group/other mode bits. Syscalls getuid(100), geteuid(101), getgid(102), getegid(103), setuid(104), setgid(105). `chmod` command dan `ls -l` menunjukkan permissions/owner. `vfs::access_check()` **dipanggil di `fd_open()`** (fd.rs) — permissions ditegakkan saat file dibuka. **Masih kurang:** setuid/setgid executable bits, ACL, sticky bit, capability system.
- **ASLR:** ✅ **PARTIAL — RNG infrastructure + per-process userspace ASLR.** RDRAND instruction (with LCG fallback seeded from RTC+PIT) provides entropy. Userspace ELF loader randomizes stack bottom (~8GB range) and heap base (~32MB range). `sys_brk` uses per-task `heap_brk`. `create_user_task()` juga menerapkan ASLR: heap_brk dirandomize per task, user stack position dirandomize jika tidak diberikan. Kernel itself remains at fixed base `0xFFFFFFFF80000000` (full KASLR would require PIE linking + relocation, not yet implemented).
- **Stack protection:** Rust memberikan beberapa proteksi (bounds checking), tapi kernel stack overflow tidak terdeteksi — tidak ada guard pages.
- **Memory safety:** Rust safety di kode aman, tapi `unsafe` blocks ekstensif di MMIO, port I/O, assembly, dan manipulasi memory rendah. Belum ada audit `unsafe` blocks.
- **Encryption:** Tidak ada crypto API, tidak ada disk encryption, tidak ada TLS.
- **Audit:** Tidak ada audit subsystem.
- **Secure boot:** Tidak ada implementasi.
- **Sandboxing:** ✅ **PARTIAL — PID + UTS namespaces implemented.** PID namespace with local PID remapping (`crates/zenus-ns/src/pid.rs`), UTS namespace with per-namespace hostname (`crates/zenus-ns/src/uts.rs`). Clone flags `CLONE_NEWPID` and `CLONE_NEWUTS` supported via internal `clone_task()`. Task struct tracks `pid_ns`, `uts_ns`, `mnt_ns` IDs. Task exit properly unregisters from PID namespace. Missing: NET, USER, IPC, MOUNT namespace isolation, setns() syscall, /proc/PID/ns integration.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| User/kernel isolation | Full | Full | Full | Full | None |
| ASLR | Full | Full | Full | Full | None |
| SELinux/AppArmor | AppArmor | AppArmor | SELinux | None | None |
| Secure boot | Yes | Yes | Yes | Yes | No |
| Crypto API | Yes | Yes | Yes | Yes | No |

---

