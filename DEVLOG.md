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
`TmpFs::write` memakainya. Helper-nya murni agar bisa diuji di host tanpa VM —
sesuai `AGENTS.md` ("prefer a pure helper over logic that can only be tested in
a VM").

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

### BUG-002 — `s_inodes_per_group == 0` → `#DE` di `fsck` dan di setiap lookup inode

**Status:** sudah di-fix
**Keparahan:** tinggi — satu `#DE` menghentikan kernel; reachable lewat `fsck`
di shell, atau lewat mount image crafted tanpa perlu `fsck` sama sekali
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::ext2_group_count_refuses_a_zero_divisor_instead_of_faulting`

**Di mana:**
- `crates/zenus-fs/src/ext2_fsck.rs` — `fsck()`
- `crates/zenus-fs/src/ext2.rs` — `mount()`, `read_inode_raw()`, `alloc_block()`

**Yang salah:** `s_inodes_per_group` dan `s_blocks_per_group` adalah `u32`
biasa di superblock, dan tidak ada yang menolak nilai nol. Semua pemakai
kemudian membaginya sebagai `(count + per_group - 1) / per_group`.

Yang paling buruk ada di `fsck()`: ia memanggil
`add_msg("inodes_per_group is zero")` lalu menjalankan pembagIANnya pada
pernyataan berikutnya. Jadi alat yang tugasnya melaporkan filesystem rusak
mati persis pada masukan yang ia laporkan — dan `#DE` di kernel berarti
`panic` → `abort`.

`Ext2Fs::mount` sudah memvalidasi `s_inode_size` dan `s_log_block_size`, tapi
tidak dua field ini. Jadi `read_inode_raw` (dipakai oleh stat/read) dan
`alloc_block` (dipakai oleh write) ikut membagi dengan nol pada filesystem
yang sudah ter-mount, tanpa perlu `fsck` di mana pun.

**Fix:** helper murni `ext2::group_count(count, per_group) -> Option<u64>` yang
menolak pembagi nol. Dipakai di keempat tempat. `mount` kini menolak mount saat
salah satu field nol, bukan menunggu lookup pertama. Langitnya dari `u64` ke
`u32` di `fsck` dijaga dengan komentar, karena `group_count` menghasilkan
ceiling dari dua `u32` dengan pembagi `u32` tak nol.

**Bonus yang ketemu saat menulis fix:** bentuk lama `(count + per_group - 1)`
juga bisa *overflow* kalau `count` mendekati `u64::MAX`, dan diam-diam melaporkan
jumlah group yang lebih sedikit dari seharusnya. Sekarang ceiling ditulis
`count / per_group + (count % per_group != 0)`. Kedua sisi ada di test.

---

### BUG-003 — `fsck` menghitung ukuran group terakhir dengan pengurangan yang bisa underflow

**Status:** sudah di-fix (ditemukan saat menulis fix BUG-002)
**Keparahan:** sedang-tinggi — `u32` underflow; dev build panic, release build
wrap ke ~4 miliar
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::ext2_fsck_last_group_survives_a_mismatched_group_count`

**Di mana:** `crates/zenus-fs/src/ext2_fsck.rs`, `fsck()`

```rust
// sebelum
let num_groups = core::cmp::max(num_groups, blocks_groups);
let last_group_blocks = blocks_count as u64 - (num_groups - 1) as u64 * blocks_per_group as u64;
let last_group_inodes = inodes_count - (num_groups - 1) * inodes_per_group;
```

**Yang salah:** `num_groups` adalah nilai **terbesar** dari hitungan group yang
berasal dari inode dan yang berasal dari block. Filesystem yang kedua
hitungannya berbeda adalah filesystem *yang valid*, bukan yang rusak —
`inodes_per_group = 8192` dengan 8193 inode memberi 2 group inode, sedangkan
`blocks_per_group = 1024` dengan 8192 block memberi 8 group block.

fsck melaporkan ketidakcocokan itu sebagai warning 11, lalu berjalan 8 group,
lalu menghitung `8193 - 7 * 8192` = **-50111**. Pada `u32` itu wrap menjadi
`4294912185` — jumlah inode bebas di group terakhir yang diklaim filesystem
hampir empat miliar.

Jadi `fsck` mati pada masukan yang baru saja ia laporkan sendiri.

**Fix:** helper murni `ext2_fsck::last_group_size(total, per_group, num_groups)`
dengan `saturating_sub` di seluruh rantainya. Saturating, bukan wrapping:
filesystem yang terlalu rusak untuk mendeskripsikan group terakhir sendiri
bukan alasan untuk berhenti memeriksa sisa groupnya.

---

### BUG-004 — `fsck` menulis melewati stack buffer karena `s_inode_size` tidak divalidasi

**Status:** sudah di-fix
**Keparahan:** tinggi — 512 byte melewati array stack, dari satu field superblock.
`fsck` adalah perintah shell, jadi image crafted sudah cukup untuk memicu
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::ext2_fsck_refuses_to_read_an_inode_that_does_not_fit_its_buffer`

**Di mana:** `crates/zenus-fs/src/ext2_fsck.rs`, `read_inode()`

```rust
// sebelum
let mut buf = [0u8; 1024];
let needed_sectors = (offset_in_sector + inode_size as usize + 511) / 512;
for i in 0..needed_sectors {
    let base = i * 512;
    bc_read(dev_id, sector + i as u64, &mut buf[base..base + 512]);
}
```

**Yang salah:** `needed_sectors` dihitung dari `s_inode_size` — field `u16`
biasa di superblock — lalu dipakai untuk menulis ke `buf` yang besarnya tetap
1024 byte. Tidak ada cek bahwa hitungannya masih di dalam buffer.

Superblock dengan `s_inode_size = 0xFFFF` dan inode yang mulai 511 byte di
dalam sector meminta 130 sector, lalu menulis `buf[1024..1536]`: 512 byte
melewati akhir array stack.

Yang membuatnyaironis: `Ext2Fs::mount` **sudah** menolak `inode_size` di luar
128..=256 sejak dulu (`ext2.rs:181`), tapi `fsck` hanya mencatat
`"inode_size < 128"` sebagai pesan lalu berjalan dengan nilai whatever yang
ada di field. Menjalankan fsck pada filesystem yang dicurigai persis alasan
kenapa validasi batas tidak boleh dilewati.

**Fix:** helper murni `ext2_fsck::inode_read_sectors(offset_in_sector, inode_size,
buf_len) -> Option<usize>` memakai `checked_add` dan membandingkan terhadap
`buf_len` sebelum slicing. `None` berarti tidak muat, dan pemanggil melaporkan
kegagalan baca inode root alih-alih melampaui frame-nya sendiri.
`read_inode` juga menolak `inodes_per_group == 0` dan `inode_no == 0` sebagai
lapisan kedua.

---

### BUG-005 — journal menulis redo image ke blok ext2 yang hidup

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — korupsi data. 15 blok journal, 123 entri diterima
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::journal_write_refuses_to_leave_its_own_block_range`
- `host_tests::journal_replay_clamps_a_num_entries_field_from_the_disk`
- `host_tests::journal_write_stops_at_the_end_of_the_journal`

**Di mana:** `crates/zenus-fs/src/journal.rs` — `journal_write`,
`journal_commit`, `journal_replay`

```rust
// sebelum
if hdr.num_entries as usize >= MAX_ENTRIES { return false; }
let max_data_block = start_block + 1 + MAX_ENTRIES as u64 - 1;
let data_block = start_block + 1 + idx as u64;
if data_block > max_data_block { return false; }
```

**Yang salah:** batas entri memakai `MAX_ENTRIES` (123) — ukuran array
`targets[]` di memori — padahal yang terbatas adalah ukuran journal di
device. Boot mengonfigurasi `journal_init(0, 3000, 16)`: 1 blok header + 15
blok redo. Entri #15 menulis redo image ke blok `3000 + 1 + 15` = **3016**,
yaitu blok ext2 yang masih hidup. Tidak ada `#DE`, tidak ada panic: journal
yang seharusnya melindungi filesystem justru menulis ke dalam filesystem.

`JNL_NUM_BLOCKS` sudah disimpan oleh `journal_init` tapi tidak pernah
dibaca di mana pun — di HEAD, satu-satunya kemunculannya adalah penulisan
di `journal_init` sendiri.

Bug kedua di jalur yang sama: `num_entries` dibaca dari disk lalu dipakai
sebagai batas loop di `journal_commit` dan `journal_replay`. Field itu
attacker-reachable, dan `MAX_ENTRIES` (123) tidak menggambarkan ukuran
journal, sehingga header yang sudah dimodifikasi bisa membuat replay
mengambil `targets[]` dan blok redo di luar jangkauan journal.

**Fix:** tiga helper murni, semuanya tanpa device:
- `capacity_for_blocks(num_blocks) -> usize` = `min(MAX_ENTRIES, num_blocks - 1)`
- `data_block_for(start, num_blocks, idx) -> Option<u64>` — `None` berarti di luar
- `replay_entry_limit(num_entries, num_blocks) -> usize` — clamp ke array **dan**
  ke ukuran device

`journal_write` dan `journal_commit` memakai `data_block_for`;
`journal_replay` memakai `replay_entry_limit`.

**Kenapa `journal_replay` dapat parameter baru:** ukuran journal tidak
direkam di header, jadi caller harus menyediakannya. Jadi
`journal_replay(dev, start, num_blocks)` — nilai yang sama dengan
`journal_init`. Menebaknya dari `MAX_ENTRIES` akan mengembalikan bug yang
sama. Kedua call site production sudah diperbarui
(`apps/src/lib.rs`, `apps/src/shell.rs`).

**Bug bonus yang diperbaiki sekalian:** `journal_replay` mengeset
`JNL_NUM_BLOCKS` ke 0 di akhir, jadi `journal_init` berikutnya yang gagal
tidak meninggalkan batas yang benar.

---

### BUG-006 — mount di `/tmp` juga menangkap `/tmp.evil/x`

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — path confusion / confinement bypass
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::a_mount_prefix_must_end_on_a_path_boundary`
- `host_tests::mount_points_are_normalised_on_the_way_in`
- `host_tests::a_mounted_filesystem_does_not_capture_a_sibling_directory`
- `host_tests::a_mount_point_with_a_trailing_slash_still_resolves`

**Di mana:** `crates/zenus-fs/src/vfs.rs` — `find_mount_in_table`,
`find_mount_to_pair`, `mount_in_ns`

```rust
// sebelum, tiga kali di tree
if path.starts_with(m.path) && m.path.len() > best_len { ... }
```

**Yang salah:** pencocokan mount tidak memeriksa batas komponen path.
Mount di `/tmp` ikut mencakup `/tmp.evil/x`, `/tmpx`, `/tmp2`. Ini bypass
konfinement: caller yang dibatasi ke `/tmp` bisa menjangkau direktori
tetangga hanya dengan memilih nama yang diawali karakter yang sama. Tiga
tempat mengulangi logika yang sama — persis masalah yang
`ARCHITECTURE.md` mencatat satu salinan VMA di `zenus-mem` sebagai
persis masalah bentuk ini.

Bug kedua di tempat yang sama: `Mount.path` disimpan apa adanya, termasuk
slash di akhir, sementara `open_in_ns` memotong dengan
`&path[prefix.len()..]`. Mount yang diregistrasi sebagai `/tmp/` membuat
potongan itu mendarat satu byte terlalu awal — atau, untuk path mount point
sendiri, gagal total dan jatuh ke lookup seluruh path.

**Fix:** dua helper murni.

- `mount_covers(path, mount) -> bool` — prefix harus diakhiri `/` atau
  berakhirnya string. Mengembalikan `0` (tidak cocok) kalau tidak.
- `normalise_mount_path(path) -> &'static str` — tanpa slash di akhir, dengan
  slash di depan. Dipanggil sekali di `mount_in_ns`, di titik mount
  direkam, sehingga semua keputusan berikutnya bisa berasumsi prefix sudah
  bersih. `mount_prefix_len` memakai `mount_covers` untuk longest-prefix.

Ketiga loop sekarang memakai `mount_prefix_len`, jadi hanya ada satu
implementasi aturan tersebut.

**Test pakai filesystem penanda** (`MarkerFs` dengan `root_inode = 0xF00D`)
karena tmpfs dan procfs sama-sama memakai inode 0 untuk root — inode saja
tidak cukup untuk membuktikan filesystem mana yang menjawab. Yang diperiksa
tambahan adalah `file_type`: `/tmp.evil` dibuat sebagai file di tmpfs,
sedangkan `MarkerFs` melaporkan semuanya sebagai directory.

Ketiga test sudah diverifikasi gagal sebelum fix (reverted `mount_covers`
dan `normalise_mount_path`, lalu `cargo test` → 3 FAILED).

---

### BUG-007 — `pkg_remove` = arbitrary-path delete

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — uninstall lebih berkuasa dari install
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::pkg_remove_refuses_to_delete_outside_the_install_dir`
- `host_tests::dir_confinement_checks_a_component_boundary`
- `host_tests::every_installable_path_is_recorded_inside_the_install_dir`

**Di mana:** `crates/zenus-fs/src/pkg.rs` — `pkg_remove`

```rust
// sebelum
for f in &info.files {
    vfs::remove(f);
}
```

**Yang salah:** `pkg_install` memvalidasi setiap path entri lewat
`install_path_for` (menolak `..` dan NUL, meng-prefix `/usr/local`), tapi
`pkg_remove` tidak menjalankan validasi itu lagi. Manifesto adalah file teks
biasa di `/var/db/zpk/<pkg>/manifest`. Siapa pun yang bisa menulis satu baris
ke manifesto — atau memasang paket lalu menyuntingnya — mendapat
**penghapusan path sembarang**. Install divalidasi, uninstall tidak, jadi
uninstall adalah operasi yang lebih berbahaya daripada yang memasang.

Yang hilang bukan proteksi di lapisan VFS, melainkan pemanggilan validasi
yang sudah ada di `pkg.rs` dan hanya dipakai di satu sisi.

**Fix:** dua helper murni.

- `is_inside_dir(candidate, dir) -> bool` — `starts_with` saja tidak cukup:
  `/usr/localevil` bukan di dalam `/usr/local`, jadi prefix harus berakhir
  pada batas komponen.
- `owned_path_for(recorded) -> Option<String>` — manifesto menyimpan path
  yang **sudah ter-resolve** (output `install_path_for`), jadi helper ini
  tidak boleh memberi prefix kedua kali; ia hanya reinstate cek
  confinement.

`pkg_remove` sekarang menolak baris di luar install dir, mencatat
`FS_METADATA_CORRUPT`, **dan** mengembalikan `false` — "uninstall sukses"
padahal ada file yang tidak dihapus adalah laporan sukses yang salah. Baris
yang sah tetap dihapus: penolakan berlaku pada baris buruknya, bukan pada
pembatalan seluruh uninstall.

**Bug yang ketemu saat menulis test:** `owned_path_for` versi pertama
memanggil `install_path_for` pada baris manifesto. Karena baris itu sudah
meresolve path, hasilnya `/usr/local/usr/local/bin/demo`, dan test
`pkg_install_list_and_remove` yang sudah ada langsung gagal. Itu bukti bahwa
uninstall sebelumnya memang sudah salah: ia menghapus path yang tidak ada,
dan kegagalannya bisu.

`is_inside_dir(x, "")` mengembalikan `false`, bukan `true` — direktori kosong
bukan induk dari semua path.

Test end-to-end sudah diverifikasi gagal sebelum fix (guard
`owned_path_for` diganti unconditional → 1 FAILED).

---

## Kandidat berikutnya (dari bug hunt, belum dikerjakan)

Prioritas menurut dampak × kemudahan diuji:

| # | Lokasi | Bug | Uji |
|---|---|---|---|
| 2 | `zenus-net/src/nic.rs:118-121` | Balasan ARP dihitung lalu dibuang (`arp::handle` return-nya diabaikan). Virtio NIC tidak pernah menjawab ARP → semua IPv4 outbound gagal | in-kernel |
| 3 | `zenus-net/src/nic.rs:120` | `our_mac` yang dikirim adalah MAC peminta, dan IP di-hardcode `10.0.2.15`. Kalau #2 diperbaiki, hasilnya ARP poisoning yang sticky (`arp.rs:64-69` menolak mengubah MAC untuk IP yang sudah ada) | host (arg builder) |
| 4 | `zenus-net/src/tcp.rs:681` | Window rx dihitung di ruang 65535 padahal buffer 4096 byte → zero-window tak pernah diumumkan, payload masuk sack_blocks lalu hilang. `recv_window` juga dipakai untuk *window yang kita umumkan*, diisi dari window yang di-peer-advertise | host (helper) |
| 5 | `zenus-net/src/tcp.rs:446` | `seq + payload.len()` overflow u32 → panic dari satu paket tak terautentikasi ke port tertutup. Sama di `:768`, `:790` | host (helper) |
| 6 | `zenus-net/src/udp.rs:42` | Field `length` UDP dibaca lalu tidak pernah dipakai; checksum diverifikasi atas panjang yang salah → data di luar datagram masuk ke DHCP/DNS | host |
| 7 | `zenus-syscall/src/syscall.rs:2681` | `8 * nfds` overflow → ukuran tervalidasi 0, ukuran terpakai 2^61 | host (helper) |
| 8 | `zenus-syscall/src/syscall.rs:2459` | `sys_shmdt` `invlpg` tanpa menulis PTE → frame di-free sementara PTE masih hidup → UAF antar task. `shmat` juga tidak menaikkan `attached` | in-kernel |
| 9 | `zenus-syscall/src/syscall.rs:1346` | `recv` alokasi `len` yang hanya dibatasi `USER_SPACE_LIMIT` (128 TiB), tidak seperti `sys_read`'s `MAX_READ` | host (helper) |
| 10 | `zenus-syscall/src/syscall/fd.rs:545` | `vfs_access` mengabaikan mode dan melewati `access_check` → `access("/etc/shadow", W_OK)` = 0 untuk file mode 000 | host |
| 11 | `zenus-syscall/src/syscall/fd.rs:537` | `vfs_chown` resolve node lalu membuangnya, selalu `true`. `FileSystem::chown` sudah diimplementasi tapi tidak pernah dipanggil | host |
| 12 | `zenus-fs/src/vfs.rs:335-380` | Semua jalur mutating VFS (mkdir/unlink/chmod/chown) tanpa permission check; hanya `fd_open` yang memanggil `access_check` | host |
| 13 | `zenus-syscall/src/syscall.rs:2034` | `sys_mount` menerima `MS_RDONLY\|MS_NOSUID\|MS_NODEV\|MS_NOEXEC` lalu mengabaikan semuanya, return 0. `vfs::Mount` tidak punya field flag sama sekali. Tidak ada cek `euid == 0` | host |
| 14 | `zenus-syscall/src/syscall.rs:708` | `brk(small)` menjalankan `unmap_heap_pages` yang menelusuri *semua* halaman di `[addr, heap_brk)` lalu `free_frame` tiap yang ter-map → program membebaskan frame ELF-nya sendiri, dan menelusuri 6.4e9 entri page table (hang). `heap_brk = loaded.heap_base ≈ 0x6000_0000_0000` | host |
| 18 | `zenus-net/src/tcp.rs:894` | `KEEPALIVE_PROBE_INTERVAL` dihitung lalu dibuang (`let _probe_interval = ...`) → probe 96× lebih lambat dari yang didokumentasikan | host |