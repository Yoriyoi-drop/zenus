## 3. Filesystem

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| Journaling | ✅ FUNCTIONAL | MEDIUM |
| Crash recovery | ✅ FUNCTIONAL | MEDIUM |
| Mount management | FUNCTIONAL | LOW |
| Permissions | ✅ FUNCTIONAL | MEDIUM |
| Symbolic links | ✅ FUNCTIONAL | LOW |
| Hard links | NOT IMPLEMENTED | LOW |
| File locking | NOT IMPLEMENTED | MEDIUM |
| Storage scalability | NOT IMPLEMENTED | HIGH |

### Detail
- **VFS:** Trait `FileSystem` dengan mount table (8 entries). Path resolution support `/`, `..`, `.`.
- **Tmpfs:** In-memory. 128 node max, 4KB file size max, 64-byte names. Linked-list directory. Fully functional read/write/create/delete.
- **Devfs:** Static char devices (null, zero, console, serial). Block device registration via function pointers. ATA drives registered sebagai `/dev/sda`-`/dev/sdd`.
- **Tarfs:** Read-only ustar parser. 64 entries max. Mounted di `/initrd`.
- **Ext2fs:** ✅ **FUNCTIONAL (read-write).** Superblock/BGDT/inode/directory parsing via block cache. **Write support**: data writes, block allocation from bitmap, inode size/mtime update, single indirect block support. Mounted otomatis di `/mnt` jika ATA drive 0 ext2.
- **Journaling:** ✅ **FUNCTIONAL — write-ahead journal (blocks 3000-3015 on device 0).** `journal_init`/`journal_begin`/`journal_write`/`journal_commit`/`journal_replay` — full cycle works. Verifikasi crash recovery: replay applies committed entries. **Crash-safe**: journal data + header flushed to disk before commit mark.
- **Crash recovery:** ✅ **Dual protection.** `ext2_fsck::fsck(dev_id)` checks superblock, BGDT, bitmaps, root inode. Journal replay recovers committed metadata writes.
- **Disk filesystem:** ✅ **FUNCTIONAL (ext2 read-write).** ATA driver + block cache + ext2 driver. Bisa mount ext2 filesystem, read directory, baca file, **write file** (data + block allocation).
- **Block layer:** ✅ **FUNCTIONAL.** 64-entry LRU write-back cache via `BlockCache`. `bc_read`/`bc_write` sebagai wrapper. Synchronous PIO. **I/O scheduler**: noop FIFO queue via `io_scheduler.rs` — request tracking, per-device pending I/O stats.
- **Missing:** Journaling FS, disk FS writethrough, TRIM/discard, volume management (LVM), RAID, encryption, NFS, FUSE.
- **Scalability:** ATA PIO satu sector per transfer. Limitasi hardware.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| Journaling FS | ext4/xfs | ext4/xfs | xfs/ext4 | ext4 | None |
| Volume mgmt | LVM | LVM | LVM+stratisd | None | None |
| RAID | mdadm | mdadm | mdadm | None | None |
| Encryption | LUKS | LUKS | LUKS | None | None |
| Max file size | 16TB+ | 16TB+ | 16TB+ | 16TB+ | 4KB (tmpfs) / 2TB (ext2) |

---

