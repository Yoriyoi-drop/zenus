## 7. Production Operations

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| Package manager | ✅ FUNCTIONAL | MEDIUM |
| Dependency management | NOT IMPLEMENTED | CRITICAL |
| Update mechanism | NOT IMPLEMENTED | CRITICAL |
| Rollback mechanism | NOT IMPLEMENTED | CRITICAL |
| Repository infrastructure | NOT IMPLEMENTED | CRITICAL |
| Configuration management | ✅ PARTIAL | MEDIUM |
| Automation support | NOT IMPLEMENTED | HIGH |

### Detail
- **Package manager:** ✅ **FUNCTIONAL** — .zpk format (ZPK1 header), install/remove/list/info commands, manifest tracking di `/var/db/zpk/`. Package file extraction ke `/usr/local/`. `crates/zenus-fs/src/pkg.rs`
- **Updates:** Tidak ada mekanisme untuk update kernel atau system files. Satu-satunya cara: rebuild ISO dan reboot.
- **Rollback:** Tidak ada. Tidak ada konsep versioned packages.
- **Repository:** Tidak ada infrastructure. Packages must be manually copied to disk.
- **Configuration management:** ✅ **PARTIAL** — Sysctl kernel parameter interface, 8 default sysctls (hostname, log_level, version, uptime, max_tasks, watchdog_timeout, ip_forward, dns.server). Type-safe get/set, read-only protection. `crates/zenus-fs/src/sysctl.rs`. **Masih kurang:** `/etc` persistence across reboot.
- **Userland:** Tidak ada userland sama sekali. Yang ada hanya kernel + shell. Tidak ada coreutils, libc, compiler, interpreter.
- **Missing:** Update mechanism, rollback, repo infrastructure, atomic updates, A/B partitioning, config persistence on disk, automation API, provisioning tools, orchestration support.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Package mgr | apt | apt | dnf | apk | zpk |
| Packages | 60,000+ | 60,000+ | 40,000+ | 10,000+ | 0 |
| Atomic update | No | No | No | No | No |
| Userland | Full | Full | Full | Busybox | None |

---

