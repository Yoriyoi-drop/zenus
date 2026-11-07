## 5. Server Features

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| SSH | ✅ FUNCTIONAL | MEDIUM |
| Service management | ✅ FUNCTIONAL | MEDIUM |
| Daemon supervision | ✅ FUNCTIONAL | MEDIUM |
| Logging | ✅ FUNCTIONAL | LOW |
| Monitoring | NOT IMPLEMENTED | HIGH |
| Resource control | NOT IMPLEMENTED | HIGH |
| Cron scheduling | NOT IMPLEMENTED | HIGH |
| Backup support | NOT IMPLEMENTED | HIGH |

### Detail
- **SSH:** ✅ **FUNCTIONAL** — ZENUS_SSH/1.0 encrypted remote shell on port 22, XOR stream cipher, password auth, up to 4 concurrent connections, shell command execution (help/echo/ls/cat/ps/ifconfig/meminfo/dmesg/id/uname/exit). `crates/zenus-net/src/ssh.rs`
- **Service management:** ✅ **FUNCTIONAL** — Init system (PID 1) with service lifecycle: register/start/stop/restart, graceful shutdown. 16 services max, per-service stop/restart commands. `crates/zenus-sched/src/init.rs`
- **Init system:** ✅ **FUNCTIONAL** — PID 1 process manager. Initrd startup script (`/initrd/init/startup.sh`) executed at boot. `crates/zenus-sched/src/init.rs`
- **Daemon supervision:** ✅ **FUNCTIONAL** — `service_supervise()` periodic health check, auto-restart with max_restarts limit, crash detection via task list scan. `crates/zenus-sched/src/init.rs`
- **Logging:** ✅ **FUNCTIONAL** — Syslog: 4096 entries (vs 32-entry dmesg), RDTSC timestamps, structured entries (level/module/msg), output to file support. Dmesg retains 256-entry ring. `crates/zenus-console/src/syslog.rs`
- **Monitoring:** Belum ada metrics, health checks, atau alerting.
- **Resource control:** Tidak ada ulimit, cgroups, rlimits, atau memory limits per task.
- **Cron:** Tidak ada scheduler pekerjaan.
- **Backup:** Tidak ada tool atau API.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Init system | systemd | systemd | systemd | OpenRC | Init (PID 1) |
| Service mgmt | systemctl | systemctl | systemctl | rc-service | service cmd |
| SSH | OpenSSH | OpenSSH | OpenSSH | Dropbear | ZENUS_SSH/1.0 |
| Logging | journald | journald | journald | syslogd | 4096-entry syslog |
| Monitoring | prom/node | prom/node | cockpit | None | None |

---

