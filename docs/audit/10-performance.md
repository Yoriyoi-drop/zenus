## 10. Performance

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| Boot speed | FUNCTIONAL | LOW |
| Context switch latency | PARTIAL | MEDIUM |
| Network throughput | PARTIAL | HIGH |
| Disk throughput | PARTIAL | HIGH |
| Memory efficiency | PARTIAL | MEDIUM |
| CPU scalability | PARTIAL | MEDIUM |

### Detail
- **Boot speed:** Boot serial + VGA output. Proses inisialisasi sekitar 100-200ms di emulasi. Tidak ada measurement.
- **Context switch:** ~100 APIC timer ticks (~10 detik) untuk round-robin. Belum diukur.
- **Network:** RTL8139 PIO — maksimum ~10Mbps (terbatas hardware emulasi dan PIO). Tidak ada throughput measurement.
- **Disk:** ATA PIO — ~16MB/s theoretical max. Satu sector per interrupt. No DMA, no NCQ.
- **Memory:** 512MB QEMU default. Frame allocator bump. Heap 4MB fixed. Tidak ada slab allocator.
- **CPU scalability:** Per-CPU tracking (max 8) tapi semua task lahir di CPU 0. Tidak ada load balancing. APs idle 100%.
- **Missing:** Benchmark suite, perf monitoring, profiling, DMA engine, slab allocator, TLB optimization, prefetching, CPU frequency scaling, ACPI P-states, C-states.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Boot time | ~5s | ~5s | ~5s | ~2s | ~1s* |
| Net throughput | 40Gbps+ | 40Gbps+ | 40Gbps+ | 40Gbps+ | ~10Mbps |
| Disk IOPS | 1M+ | 1M+ | 1M+ | 1M+ | ~100 |
| Memory mgmt | Slab+Buddy | Slab+Buddy | Slab+Buddy | Slab+Buddy | Bump+FreeList |

*\* Di emulasi tanpa beban berarti*

---

