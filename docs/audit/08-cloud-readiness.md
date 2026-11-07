## 8. Cloud Readiness

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| KVM | NOT IMPLEMENTED | MEDIUM |
| QEMU | PARTIAL | MEDIUM |
| Docker compatibility | NOT IMPLEMENTED | CRITICAL |
| OCI support | NOT IMPLEMENTED | CRITICAL |
| Kubernetes readiness | NOT IMPLEMENTED | CRITICAL |
| Virtualization support | NOT IMPLEMENTED | HIGH |

### Detail
- **KVM/QEMU:** Boots di QEMU dengan emulasi penuh (`-cpu max`). Hanya emulasi — tidak ada KVM paravirt support atau PVH entry.
- **Virtio:** ✅ **DONE** — virtio-net (multi-queue via `VIRTIO_NET_F_MQ`, polling RX/TX, MAC from device config), virtio-blk (sector read/write, registered via devfs, ext2 mount), virtio-balloon (inflate/deflate queue siap), virtio-console (TX queue siap). Transport: modern virtio-over-PCI v1.0 common config MMIO, split virtqueue. Init sequence lengkap (reset→ack→driver→FEATURES_OK→queue setup→DRIVER_OK). **Notify address fix:** `notify_off_multiplier` dibaca dari PCI config capability, per-queue notify address computed dengan `queue_notify_off * notify_off_multiplier`. Lihat `crates/zenus-virtio/`.
- **Docker:** Belum mungkin. ✅ **PID + UTS namespaces** implemented (clone_task with CLONE_NEWPID/CLONE_NEWUTS flags, UTS hostname isolation). Masih kurang: NET, USER, IPC, MOUNT namespaces, cgroups, overlayfs, seccomp, capabilities.
- **OCI:** Tidak ada support.
- **Kubernetes:** Tidak ada. Kublet, container runtime (containerd/CRI-O), dan network plugin (CNI) tidak bisa berjalan.
- **Cloud-init:** Tidak ada.
- **Missing:** KVM paravirt, PVH entry, container namespaces, cgroups v2, overlayfs, seccomp, OCI runtime spec, CNI, cloud-init, ACPI hotplug.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Docker | Full | Full | Full | Full | None |
| K8s | Full | Full | Full | Full | None |
| Virtio | Yes | Yes | Yes | Yes | ✅ DONE |
| Cloud-init | Yes | Yes | Yes | No | None |

---

