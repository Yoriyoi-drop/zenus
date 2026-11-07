## Roadmap to Production Ready

### Phase 1: Foundation (6-12 months) → Alpha
**Goal:** Sistem bisa boot, load user programs, menyimpan data.

1. **User mode execution** — ✅ **VERIFIED.** Ring 3 switching via IRETQ (`user.rs`), `create_user_task()` sets up USER_CODE/USER_DATA selectors. SYSCALL entry (EFER.SCE + STAR MSR) working end-to-end. **"Hello from user mode!"** printed to serial console from Ring 3. **Remaining issue:** GPF(0x10) at iretq in `apic_timer_isr_stub` when timer fires during user execution — ISR frame layout mismatch (3 vs 5 items).
2. **Per-process address space** — ✅ `cr3` + `heap_brk` di `Task` struct. CR3 save/restore di `yield_now()` dan `schedule_tick()`. `create_address_space()` memclone kernel page table + clear user half.
3. **Fix ELF loader** — ✅ **VERIFIED.** ELF loader rewritten di `elf.rs` dengan manual page table allocation (bypasses `OffsetPageTable` flush issues). User binary (`user.s`) loaded at 0x400000, stack at 0x7FFFFF00000. Verified via debug page table walk.
4. **Page table management** — ✅ `OffsetPageTable` di-instantiate via `with_mapper()`. `map_page()`, `unmap_page()`, `map_user_page_raw()`, `create_address_space()` all functional. Verified no GPF.
5. **Basic disk filesystem** — ✅ ext2 read-only: superblock/BGDT/inode parsing, directory iteration, file read via block cache. **Verified**: `read_dir("/")` returns `lost+found(11), hello.txt(12), subdir(13)`; `read(hello.txt)` returns `"Hello from ext2 filesystem on Zenus!"`.
6. **Block cache** — ✅ 64-entry LRU write-back cache di `block_cache.rs`. `bc_read`, `bc_write`, `bc_flush`, `bc_stats` via global `BLOCK_CACHE` SpinLock. Boot-time stats: 4 misses, 0 hits.
7. **Block I/O scheduler** — ✅ Minimal noop scheduler via block cache layer. Synchronous ATA PIO — I/O queuing tidak diperlukan.
8. **Testing framework** — ✅ **27 unit tests** across block_cache, VFS, ext2, paging. `make test` build + run via QEMU. `testing` feature flag di workspace crates. Test runner prints results over serial and halts.

### Phase 2: Networking & Security (6-12 months) → Beta
**Goal:** Jaringan berfungsi, security model dasar.

9. ✅ **TCP implementation** — 11/11 RFC 793 states wired, retransmission (poll-based, 5 retries), ACK processing (send_una tracking), ISN randomization (RDRAND+LCG), active open (connect/SYN sent), passive open (listen/accept), graceful close (FIN_WAIT1/2, CLOSING, TIME_WAIT with timeout, LAST_ACK), multi-connection per port, slot recycling, echo server verified
10. ✅ **UDP implementation** — Port binding via socket API, ring buffer (8 entries × 1500 bytes), send/sendto/recv, generic port dispatch, echo server verified
11. ✅ **BSD socket API** — socket/bind/listen/accept/connect/send/sendto/recv/close, accept-loop based echo server
12. ✅ ~~**DHCP client**~~ — Dynamic IP via DISCOVER→OFFER→REQUEST→ACK, verified on QEMU
13. ✅ ~~**DNS resolver**~~ — Basic stub resolver. A-record queries, response parsing, verified with example.com + google.com.
14. ✅ ~~**IP routing table**~~ — Static routes, default gateway, longest-prefix match lookup, DHCP integration.
15. ✅ **User/group model** — uid/gid/euid/egid, syscalls 100-105, `id`/`whoami` commands
16. ✅ **File permissions** — owner/group/other read/write/execute, mode bits, chmod, `ls -l`, `access_check` on open
17. ✅ **ASLR** — Per-process userspace stack/heap randomization via RDRAND-based RNG, RTC+PIT seeded fallback, kernel at fixed base
18. ✅ **NIC IRQ support** — I/O APIC driver, IRQ 11 routed to vector 43, ISR handler ack + EOI
19. ✅ **DHCP server** — IP pool 10.0.2.100-10.0.2.115, lease table, DISCOVER→OFFER→REQUEST→ACK, subnet/gateway/dns/lease-time options

### Phase 3: Server Infrastructure (6-12 months) → Release Candidate
**Goal:** Sistem bisa dioperasikan sebagai server.

20. ✅ **SSH server** — ZENUS_SSH/1.0 encrypted remote shell on port 22, XOR stream cipher, password auth, up to 4 concurrent connections, command execution. `crates/zenus-net/src/ssh.rs`
21. ✅ **Init system** — PID 1 process manager, service lifecycle (register/start/stop/restart), graceful shutdown. `crates/zenus-sched/src/init.rs`
22. ✅ **Package manager** — .zpk format (ZPK1 header), install/remove/list/info, manifest tracking at `/var/db/zpk/`. `crates/zenus-fs/src/pkg.rs`
23. ✅ **Initrd execution** — `/initrd/init/startup.sh` parsed and executed as init script (echo/cat/ls/mkdir/touch/sleep). `crates/zenus-sched/src/init.rs`
24. ✅ **Persistent config** — Sysctl kernel parameter interface, 8 default sysctls, type-safe get/set, read-only protection. `crates/zenus-fs/src/sysctl.rs`
25. ✅ **Service supervision** — Periodic health check via `service_supervise()`, auto-restart with max_restarts limit, crash detection. `crates/zenus-sched/src/init.rs`
26. ✅ **Reliable logging** — 4096-entry syslog buffer, RDTSC timestamps, structured entries (level/module/msg), file output support. `crates/zenus-console/src/syslog.rs`
27. ✅ **Watchdog** — Software watchdog with APIC timer integration, 30s default timeout, auto-reboot on expiry. `crates/zenus-arch/src/watchdog.rs`
28. ✅ **Crash dump** — Full CPU register save, 16-entry backtrace, CR3 capture, panic message, serial/disk output. `crates/zenus-arch/src/crash.rs`
29. ✅ **Lockdep** — Lock ordering tracking, circular dependency detection, per-CPU acquisition stack, 64 lock classes. `crates/zenus-sync/src/lockdep.rs`
30. ✅ ~~**Filesystem journaling**~~ — Write-ahead log, crash-safe flush, verified on disk
31. ~~**fsck**~~ ✅ **DONE** — `ext2_fsck::fsck(dev_id)` checks superblock, BGDT, bitmaps, root inode; verified PASSED on test image

### Phase 4: Cloud & Production (12-18 months) → Production Ready
**Goal:** Sistem cloud-ready, enterprise grade.

32. ✅ **Virtio drivers** — virtio-net, virtio-blk, virtio-console, virtio-balloon + multi-queue NIC (`VIRTIO_NET_F_MQ`, up to 2 queue pairs, per-queue notify addr, round-robin TX).`crates/zenus-virtio/`
33. **Container namespaces** — pid, net, mount, user, uts, ipc namespaces.
34. **Cgroups v2** — CPU, memory, I/O, PID controllers.
35. **Overlayfs** — Container image layer support.
36. **Seccomp** — Syscall filtering for containers.
37. **OCI runtime** — Basic container runtime (runz-like).
38. **Docker/containerd compatibility** — OCI spec compliance.
39. **NFS client** — Network filesystem access.
40. **IPv6** — Full IPv6 stack.
41. **Firewall** — Packet filtering, stateful inspection, NAT.
42. **Crypto API** — AES, SHA, RSA, ECC in-kernel. Hardware acceleration via AES-NI.
43. **Disk encryption** — LUKS-style full disk encryption.
44. **Volume management** — LVM-like logical volumes.
45. **RAID** — Software RAID (md-like).
46. **Performance monitoring** — Perf-like profiling, pmu counters.
47. **Cloud-init** — First-boot provisioning.
48. **NUMA awareness** — Memory policy, CPU pinning.
49. **Load balancing** — Cross-CPU task migration, IPI-based scheduling.
50. **Production docs** — Admin guide, API reference, tuning guide.

---

