# Zenus OS — Dev Log

> Log kerjarngga: satu bug = satu regression test.
> Format tiap entri: **BUG-NNN** → di mana → apa yang salah → test mana yang
> mengunci perilaku itu.
> Sumber kebenaran tetap kode + `ARCHITECTURE.md` / `SECURITY.md` / `ROADMAP.md`.

## Aturan

- Satu bug ditemukan → **tepat satu** test baru (`#[test]` host, atau `test!()`
  in-kernel kalau butuh MMIO/IDT/paging).
- Test harus gagal sebelum fix dan lulus sesudahnya. Kalau tidak bisa
  dibuktikan gagal dulu, itu bukan regression test — catatan saja.
- Test named setelah perilaku yang dijaga, bukan setelah nomor bug
  (`vma_find_free_rejects_zero_size`, bukan `test_bug_007`).
- Bug di parser → `make fuzz-smoke` ikut dijalankan kalau relevan.
- Selesai satu bug → `make test-host` hijau, lalu commit.

---

## Bug diperbaiki

### BUG-001 — `tmpfs::write` bisa dipanggil dengan offset yang wrap → kernel abort

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — reachable dari ring 3, `open` + `lseek` + `write`
**Test:** `crates/zenus-fs/src/tmpfs.rs`
- `host_tests::a_write_offset_that_wraps_is_refused_instead_of_panicking`
- `host_tests::tmpfs_write_with_a_wrapping_offset_returns_none`

**Di mana:** `crates/zenus-fs/src/tmpfs.rs`, `TmpFs::write`

```rust
// sebelum
let end = offset as usize + buf.len();
if end > MAX_FILE_SIZE { return None; }        // cek terlalu lambat
nodes[idx].data[offset as usize..end].copy_from_slice(buf);
```

**Yang salah:** `offset` datang langsung dari `lseek` tanpa dipanggil juga ke
`ext2::write` (yang memang menolak `offset > file_size`). Penambahan
`offset as usize + buf.len()` tidak dicek overflow-nya, dan batas
`MAX_FILE_SIZE` diperiksa *sesudah* penambahan, bukan sebelum slicing.

`lseek(fd, -1, SEEK_SET)` menghasilkan `offset = 0xFFFF_FFFF_FFFF_FFFF`
(`SEEK_SET` dengan offset negatif di-cast ke `u64`). Tulis 2 byte:

```
end = 0xFFFF_FFFF_FFFF_FFFF + 2  →  wrap ke 1
1 > MAX_FILE_SIZE (1024)?         →  false, guard lolos
data[0xFFFF_FFFF_FFFF_FFFF..1]    →  panic: index mulai melewati akhir array
```

Di build release `add` membungkus diam-diam; di build dev (overflow-checks on)
panic di baris penambahan. Hasilnya: satu `lseek` + satu `write` menghentikan
kernel.

**Fix:** helper murni `tmpfs::write_end(offset, buf_len) -> Option<usize>` yang
menggunakan `checked_add` dan membandingkan batas *sebelum* slicing.
`TmpFs::write` memakainya. Helper-nya murni supaya bisa diuji di host tanpa VM —
membuat helper-nya murni agar bisa diuji di host tanpa VM — sesuai `AGENTS.md`
("prefer a pure helper over logic that can only be tested in a VM").

**Kenapa dua test, satu bug:** yang pertama mengunci helper (termasuk kasus
`MAX_FILE_SIZE + 1` dengan `buf_len = 0`, yang tidak bisa dihasilkan oleh
`checked_add` saja karena end-nya masih dalam batas — harus ditolak karena
*start*-nya sudah di luar). Yang kedua memverifikasi jalur `FileSystem::write`
sungguhan, supaya guard tidak bisa dihapus dari `write` sementara helper-nya
dibiarkan benar.

**Catatan test:** saat pertama ditulis, test-nya sendiri yang salah — ia menolak
`write_end(MAX_FILE_SIZE, 0)`, padahal itu tulis kosong tepat di EOF dan
`data[1024..1024]` valid. Assertion-nya yang dikoreksi, bukan fix-nya.

---

## Kandidat berikutnya (dari bug hunt, belum dikerjakan)

Prioritas menurut dampak × kemudahan diuji:

| # | Lokasi | Bug | Uji |
|---|---|---|---|
| 2 | `zenus-fs/src/ext2_fsck.rs:148-172` | `inodes_per_group == 0` → pesan error lalu langsung `÷0` → `#DE`. `mount` tidak cek field ini juga, jadi jalur panic tanpa perlu fsck | host (helper geometri) |
| 3 | `zenus-fs/src/ext2_fsck.rs:466` | `inode_size` dari superblock menentukan loop sector; `buf` cuma 1024 byte → tulis melewati stack buffer. `fsck` di shell reachable | host |
| 4 | `zenus-net/src/nic.rs:118-121` | Balasan ARP dihitung lalu dibuang (`arp::handle` return-nya diabaikan). Virtio NIC tidak pernah menjawab ARP → semua IPv4 outbound gagal | in-kernel |
| 5 | `zenus-net/src/nic.rs:120` | `our_mac` yang dikirim adalah MAC peminta, dan IP di-hardcode `10.0.2.15`. Kalau #4 diperbaiki, hasilnya ARP poisoning yang sticky | host (arg builder) |
| 6 | `zenus-net/src/tcp.rs:681` | Window rx dihitung di ruang 65535 padahal buffer 4096 byte → zero-window tak pernah diumumkan, payload dropout | host (helper) |
| 7 | `zenus-net/src/tcp.rs:446` | `seq + payload.len()` overflow u32 → panic dari satu paket tak terautentikasi ke port tertutup | host (helper) |
| 8 | `zenus-net/src/udp.rs:42` | Field `length` UDP dibaca lalu tidak pernah dipakai; checksum diverifikasi atas panjang yang salah → data di luar datagram masuk ke DHCP/DNS | host |
| 9 | `zenus-syscall/src/syscall.rs:2681` | `8 * nfds` overflow → ukuran tervalidasi 0, ukuran terpakai 2^61 | host (helper) |
| 10 | `zenus-syscall/src/syscall.rs:2459` | `sys_shmdt` `invlpg` tanpa menulis PTE → frame di-free sementara PTE masih hidup → UAF antar task | in-kernel |
| 11 | `zenus-syscall/src/syscall.rs:1346` | `recv` alokasi `len` yang hanya dibatasi `USER_SPACE_LIMIT` (128 TiB), tidak seperti `sys_read`'s `MAX_READ` | host (helper) |
| 12 | `zenus-syscall/src/syscall/fd.rs:545` | `vfs_access` mengabaikan mode dan melewati `access_check` → `access("/etc/shadow", W_OK)` = 0 untuk file mode 000 | host |
| 13 | `zenus-syscall/src/syscall/fd.rs:537` | `vfs_chown` resolve node lalu membuangnya, selalu `true`. `FileSystem::chown` sudah diimplementasi tapi tidak pernah dipanggil | host |
| 14 | `zenus-fs/src/vfs.rs:335-380` | Semua jalur mutating VFS (mkdir/unlink/chmod/chown) tanpa permission check; hanya `fd_open` yang memanggil `access_check` | host |
| 15 | `zenus-syscall/src/syscall.rs:2034` | `sys_mount` menerima `MS_RDONLY\|MS_NOSUID\|MS_NODEV\|MS_NOEXEC` lalu mengabaikan semuanya, return 0. `vfs::Mount` tidak punya field flag sama sekali | host |