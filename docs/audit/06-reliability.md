## 6. Reliability

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| Stress testing | NOT IMPLEMENTED | HIGH |
| Long uptime capability | NOT IMPLEMENTED | HIGH |
| Memory leak resistance | NOT IMPLEMENTED | MEDIUM |
| Deadlock detection | ✅ FUNCTIONAL | LOW |
| Watchdog system | ✅ FUNCTIONAL | LOW |
| Fault tolerance | NOT IMPLEMENTED | HIGH |
| Crash diagnostics | ✅ FUNCTIONAL | LOW |

### Detail
- **Stress testing:** Tidak ada framework atau hasil stress test.
- **Uptime:** Belum pernah diuji. Kernel heap 4MB fixed — kemungkinan fragmentasi dan OOM pada uptime panjang.
- **Memory leaks:** Free-list allocator tanpa garbage collection. Tidak ada kmemleak atau alat deteksi.
- **Deadlock:** ✅ **FUNCTIONAL** — Lockdep dengan lock ordering tracking, circular dependency detection, per-CPU acquisition stack (depth 8), 64 lock classes, 256 dependency edges. `crates/zenus-sync/src/lockdep.rs`
- **Watchdog:** ✅ **FUNCTIONAL** — Software watchdog dengan APIC timer integration, 30s default timeout, pet mechanism (shell loop + scheduler tick), auto-reboot on expiry, stop/status API. `crates/zenus-arch/src/watchdog.rs`
- **Fault tolerance:** Tidak ada. Single point of failure di semua layer.
- **Crash diagnostics:** ✅ **FUNCTIONAL** — Crash dump: full CPU register save (16 GP + RIP/RFLAGS/CS/SS), CR3 capture, 16-entry backtrace via frame pointer walk, panic message recording, serial dump + disk save support. `crates/zenus-arch/src/crash.rs`
- **Testing:** ✅ **27 unit tests** across block_cache, VFS, ext2, paging. `make test` untuk build + run QEMU.
- **Missing:** CI/CD, kmemleak, kdump/kexec, fault injection, fuzzing, stress testing framework.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Watchdog | softdog/hwdog | softdog/hwdog | softdog/hwdog | softdog | Software watchdog |
| Crash dump | kdump | kdump | kdump | kdump | Crash dump (reg+bt) |
| Lock checking | lockdep | lockdep | lockdep | lockdep | Lockdep (basic) |
| Test framework | LTP+ktest | LTP+ktest | LTP+ktest | LTP | 27 unit tests |

---

