## 1. Kernel Architecture

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| Scheduler | FUNCTIONAL | MEDIUM |
| Process management | ✅ FUNCTIONAL | MEDIUM |
| Memory management | PARTIAL | HIGH |
| Virtual memory | PARTIAL | HIGH |
| Paging | PARTIAL | HIGH |
| Context switching | FUNCTIONAL | LOW |
| SMP support | FUNCTIONAL | MEDIUM |
| NUMA awareness | NOT IMPLEMENTED | LOW |
| Interrupt handling | FUNCTIONAL | MEDIUM |
| APIC support | FUNCTIONAL | LOW |
| Kernel panic recovery | ✅ FUNCTIONAL | MEDIUM |

### Detail
- **Scheduler:** Preemptive round-robin per-CPU dengan APIC timer (~100ms tick). Time slice 50 ticks (~5 detik). Cooperative yield juga didukung. Bekerja tetapi tanpa load balancing antar CPU — semua task terlahir di CPU 0.
- **Process management:** ✅ **FUNCTIONAL.** User mode (Ring 3) task berfungsi — user demo binary di-load via ELF loader, di-map ke user address space, dan dijalankan sebagai task terpisah dengan `create_user_task()`. **User demo verified**: menulis "Hello from user mode!" ke serial console via SYSCALL (SYS_WRITE=1). **GPF(0x10) di `iretq` saat APIC timer interrupt pertama** — interrupt fires during Ring 3 execution, ISR stubs don't handle non-ring-transition frame (3 items instead of 5). Kernel belum punya dedicated kernel stack untuk syscall entry (SYSCALL doesn't switch stacks, kernel code runs on user stack until interrupt). Shell masih berjalan di Ring 0 untuk keperluan interaktif.
- **Memory management:** Frame allocator bump + free stack (256 entri). Heap allocator 4MB free-list. Tidak ada page reclaim, swapping, COW.
- **Paging:** ✅ **FUNCTIONAL.** PGE flag di CR4. `OffsetPageTable` di-instantiate via `with_mapper()`. `map_page`/`unmap_page`/`map_user_page_raw`/`create_address_space` semua aktif. CR3 switching di scheduler (yield + preempt).
- **Context switching:** Working — menyimpan 15 GP registers + iretq frame.
- **SMP:** Mendeteksi CPU via Limine MP, APs boot, menginisialisasi APIC, masuk idle loop. Tidak ada IPI-based scheduling.
- **NUMA:** Tidak dideteksi atau digunakan.
- **Interrupts:** IDT lengkap (0-31 exception handlers, semua vector). Double fault dengan IST. Page fault infinite loop (debug).
- **Panic recovery:** ✅ **FUNCTIONAL** — Crash dump registers 16 GP regs + RIP/RFLAGS/CS/SS + CR3 + 16-entry backtrace + panic message, dumps to serial + disk. Watchdog auto-reboots on expiry. `crates/zenus-arch/src/crash.rs`, `crates/zenus-arch/src/watchdog.rs`.
- **Missing:** COW, swapping, page reclaim, load balancing, I/O priorities, real-time scheduling classes, cgroups/CPUSET.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Scheduler | CFS+EEVDF | CFS+EEVDF | CFS+EEVDF | CFS+EEVDF | Round-robin |
| Preempt | Full | Full | Full | Full | Preemptible |
| NUMA | Yes | Yes | Yes | Yes | No |
| SMP | Yes | Yes | Yes | Yes | Basic |
| User mode | Full | Full | Full | Full | Partial (Ring 3 via SYSCALL) |

---

