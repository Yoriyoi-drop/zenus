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

### BUG-008 — `chmod`/`chown`/`access` tanpa cek, dan `chown` yang tidak pernah jalan

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — kerentanan pada semua jalur mutating VFS
**Test:** `crates/zenus-fs/src/lib.rs`
- `host_tests::mutating_paths_are_refused_without_permission_on_the_parent`
- `host_tests::the_parent_directory_is_what_gets_checked_not_the_target`
- `host_tests::chmod_and_chown_are_owner_or_root_only`
- `host_tests::access_honours_its_mode_argument`
- `host_tests::access_mode_bits_are_decoded_or_refused`
- `host_tests::ownership_is_root_or_the_owner`

**Di mana:** `crates/zenus-syscall/src/syscall/fd.rs` — `vfs_mkdir`,
`vfs_unlink`, `vfs_rmdir`, `vfs_chmod`, `vfs_chown`, `vfs_access`;
`crates/zenus-fs/src/vfs.rs`

**Yang salah:** `fd_open` adalah satu-satunya pemanggil `access_check`.
Empat jalur lain masuk ke filesystem tanpa cek apa pun.

- `vfs_mkdir`/`vfs_unlink` → `vfs::create_dir`/`vfs::remove` langsung
- `vfs_chmod` → `node.fs.chmod` tanpa cek kepemilikan
- `vfs_chown` → me-*resolve* node, membuangnya, `return true` tanpa syarat
- `vfs_access` → membuang argumen `mode`, hanya menanyakan "path ini
  resolve?" sehingga `access("/etc/shadow", W_OK)` sukses untuk file mode 000

`FileSystem::chown` sudah diimplementasi di tmpfs sejak awal dan tidak pernah
dipanggil dari mana pun. Jadi `chown` melaporkan sukses sambil tidak melakukan
apa-apa — lebih buruk dari gagal, karena program tidak tahu harus mencari
niat lain.

Karena kernel menjalankan shell-nya sebagai satu task dan `current_euid()`
adalah 0 untuk semua task yang tidak menetapkannya, dampaknya praktis: setiap
program mendapat hak filesystem root.

**Fix:** permission check pindah ke `zenus-fs` supaya bisa diuji di host.

- `Credentials` — identitas dilewatkan masuk, bukan dibaca dari scheduler,
  supaya setiap keputusan bisa diuji tanpa VM
- `decode_access_mode(u32) -> Option<(read, write, exec)>` — `None` untuk bit
  di luar `R_OK|W_OK|X_OK`, ditolak bukan direduksi diam-diam
- `access_check_bits(...)` — bentuk umum dengan ketiga bit berdiri sendiri.
  `access_check` lama hanya bisa "baca atau tulis", jadi tidak bisa
  menyatakan `X_OK` alone maupun "baca tanpa tulis" — persis yang ditanyakan
  `access(2)`. `access_check` tetap ada sebagai shorthand untuk `fd_open`
- `owns(uid, euid, stat)` — root atau owner
- `may_modify_parent(parent_stat, cred)` — untuk `mkdir`/`unlink`/`rmdir`

Empat entry point namespace-aware: `create_dir_permitted`,
`remove_permitted`, `chmod_permitted`, `chown_permitted`, plus
`access_permitted`. `sys_access` sekarang meneruskan `mode`-nya.

** object's yang dicek adalah parent, bukan target.** `mkdir d` memodifikasi
**Objek yang dicek adalah parent, bukan target.** `mkdir d` memodifikasi
`parentof(d)`, bukan `d`. Bit pada file target tidak berperan apa pun untuk
unlink: file read-only di dalam direktori yang bisa ditulis tetap boleh
dihapus — itu POSIX. Test
`the_parent_directory_is_what_gets_checked_not_the_target` yang mengunci arah
ini, sekaligus menahan versi yang terlalu ketat (yang akan menolak
menghapus file read-only sendiri).

**`chown` memakai `i64`, bukan `u32`.** ABI-nya `-1` berarti "tidak diubah",
dan selama ini diteruskan sebagai `u32::MAX` — yang akan ditulis ke `uid`/
`gid` sebagai 4294967295. `fd::vfs_chown` yang menerjemahkan `u32::MAX` → `-1`.

**Bug kedua yang ketemu: race di test suite.** Setelah check masuk,
`cargo test -p zenus-fs` gagal 1 dari ~200 jalan:
`tmpfs_write_with_a_wrapping_offset_returns_none` gagal di
`tmpfs.rs:441` ("create a file"). Penyebabnya `tmpfs::host_tests` membuat node
di tabel tmpfs global tanpa lock yang sama dengan `fresh_fs()` di `lib.rs`
yang memanggil `TmpFs::reset()` — reset dari thread lain menghapus inode-nya.
`SERIAL` sekarang `pub(crate)` dan dipakai kedua modul. Diuji 200× tanpa lock
(1 gagal) vs 200× dengan lock (0 gagal).

Keempat test lain diverifikasi gagal sebelum fix (guard di `vfs.rs`
dikembalikan ke "tidak ada cek" → 4 FAILED).

---

### BUG-009 — TCP: window yang diumumkan salah ruang, dan overflow sequence number

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — kehilangan data TCP diam-diam; panic di debug build
**Test:** `crates/zenus-net/src/tcp.rs` → `tcp::host_tests`
- `the_advertised_window_is_the_free_space_in_the_receive_buffer`
- `sequence_numbers_advance_modulo_two_to_the_32`
- `keepalive_probes_are_spaced_by_the_documented_interval` (BUG-010, sekalian)

**Di mana:** `crates/zenus-net/src/tcp.rs`

### BUG-009a — window rx dihitung di ruang 65535, bukan 4096

```rust
// sebelum
let window = 65535u16.saturating_sub(tcb.rx_data_len as u16);
```

Field window itu `u16`, jadi 65535 adalah nilai tertingginya — **bukan**
kapasitas. Buffer receive 4096 byte. Dengan buffer penuh, window yang
diumumkan adalah `65535 - 4096 = 61439`: buffer penuh tapi peer diberi tahu
masih ada 61 KiB ruang. Akibatnya:

1. Peer terus mengirim
2. `copy_len = min(payload.len(), rx_data.len() - rx_data_len)` = **0**
3. Payload dibuang
4. `recv_nxt` tetap maju melewati payload itu
5. Kita mengirim ACK yang menyatakan data itu diterima

Data hilang permanen, dan peer believes we've got it. Tidak ada timeout yang
pernah memicunya:Sequence number sudah lewat, jadi retransmitor peer menganggap
sudah sampai.

**Fix:** `advertised_window(rx_data_len)` = ruang kosong di buffer, dan
`Tcb::win()` memanggilnya setiap kali segment dibangun. Field `recv_window`
dihapus total — penyimpanannya berarti harus menyegarkan di setiap site yang
mengisi atau mengosongkan buffer, dan satu yang terlewat berarti
mengumumkan ruang yang sudah terpakai.

### BUG-009b — `recv_window` diisi dari window milik peer

Satu field dipakai untuk dua hal: window yang **kita** umumkan, dan window yang
**peer** umumkan. Tiga site menulis `tcb.recv_window = window` dari header
ACK peer. Jadi window yang kita kirim =whatever peer terakhir bilang. Dua
field dipisah: field yang dihapus, dan `peer_window` yang baru — sekarang
dipakai sungguhan untuk membatasi `limit` di jalur kirim, bersama `cwnd`.

### BUG-009c — `seq + payload.len()` overflow

`seq` datang dari header paket, bebas dipilih peer. Empat site melakukan
`seq + payload.len() as u32 (+1)` tanpa cek. Untuk `seq` dekat `u32::MAX`,
build debug **panic** — dari satu segmen tak terautentikasi ke port tertutup.
Release wrap diam-diam, dan itu kebetulan *benar*: aritmetika sequence TCP
didefinisikan modulo 2^32 (RFC 793). Jadi ini hanya crash, pernah nilai salah.

**Fix:** `seq_advance(seq, len, extra)` memakai `wrapping_add` dan menyatakan
niatnya. `len as u32` juga benar untuk tujuan ini: truncation ke u32 adalah
mod 2^32 yang sama.

### BUG-010 — `KEEPALIVE_PROBE_INTERVAL` dihitung lalu dibuang

**Status:** sudah di-fix (commit ini)
**Keparahan:** sedang — keepalive tidak berfungsi seperti terdokumentasi
**Test:** `keepalive_probes_are_spaced_by_the_documented_interval`

```rust
// sebelum
tcb.keepalive_time += 1;
if tcb.keepalive_time >= KEEPALIVE_IDLE { ... tcb.keepalive_probes += 1;
let _probe_interval = KEEPALIVE_PROBE_INTERVAL;   // dibuang
```

**Yang salah:** dua hal. Interval probe **dihitung lalu dibuang**, jadi
sembilan probe keluar beruntun pada call berturut-turut, bukan 75 detik
apartinya. Dan `keepalive_time` menghitung **call ke
`poll_retransmit`**, bukan detik — `poll_retransmit` dipanggil dari jalur
accept/connect socket, bukan dari timer. Jadi "2 jam idle" sebenarnya
"7200 kali seseorang menerima koneksi".

**Fix:** keepalive dijadwalkan dalam PIT tick (`get_ticks()`, 100 Hz sesuai
`uptime`). `keepalive_action(ticks, next_due, probes_sent) -> Keepalive`
murni, plus `keepalive_idle_deadline` / `keepalive_probe_deadline`. Field
`keepalive_time` diganti `keepalive_due` (tick absolut). Trafik apa pun
menyetel ulang ke `keepalive_idle_deadline(now)`; probe yang tidak dijawab
menyetel ke `keepalive_probe_deadline(now)` — nilai yang dulu dibuang.

Ketiga test diverifikasi gagal sebelum fix (helper dikembalikan ke
`65535 - len`, `+` biasa, dan gating dimatikan → 3 FAILED).

---

### BUG-011 — field `length` UDP dibaca lalu tidak pernah dipakai

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — data di luar datagram masuk ke DHCP/DNS; checksum diverifikasi atas panjang yang salah
**Test:** `crates/zenus-net/src/udp.rs` → `udp::host_tests`
- `the_length_field_bounds_the_payload`
- `an_impossible_length_field_is_refused`
- `short_buffers_are_refused`

**Di mana:** `crates/zenus-net/src/udp.rs` — `parse`, `handle_receive`

```rust
// sebelum
let header = UdpHeader { /* … */ length, checksum };
let udp_payload = &packet[8..];      // packet adalah hasil parse IPv4
```

**Yang salah:** `packet` di sini adalah payload IPv4, yang bisa **lebih
panjang** dari datagram UDP — padding, atau apa pun yang mengikuti di frame. Field
`length` diparse ke `UdpHeader` lalu tidak pernah dibaca lagi. Dua konsekuensi
dari satu field:

1. `udp_payload` = `&packet[8..]`, jadi byte setelah batas datagram ikut
   diteruskan. `dhcp::handle_receive`, `dhcp_server::handle_receive` dan
   `dns::handle_receive` semuanya mengindeks header DHCP/DNS di offset tetap
   dari payload itu — bytes tambahan itu jadi input untuk parser.
2. `handle_receive` memverifikasi checksum atas `packet` utuh, sementara
   pseudo-header UDP menyertakan `udp_len` di dalamnya. Dua panjang itu tidak
   sama, jadi checksum **tidak pernah cocok** untuk datagram yang punya junk
   setelahnya... atau lebih buruk: kebetulan cocok untuk data yang korup
   tepat ketika ada trailing junk. Kelima parser DHCP/DNS/socket bergantung
   pada hasil `handle_receive`.

`dhcp.rs:150` sudah punya komentar bahwa ia re-parse `resp_buf` dengan
`udp::parse()` dan mengindeks header DHCP di offset konstan — jadi jalur
tersebut sekarang benar-benar dilindungi oleh batas length.

**Fix:** `parse` memotong ke `&packet[..length as usize]` dan menolak length
di bawah 8 (header) atau di atas `packet.len()`. Payload dikembalikan dari
potongan itu, dan checksum diverifikasi atas potongan yang sama.

Tiga test diverifikasi gagal sebelum fix (bound dikembalikan ke
`&packet[8..]` → 3 FAILED).

---

### BUG-012 — Virtio NIC tidak pernah menjawab ARP, dan balancerannya akan jadi ARP poisoning

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — semua IPv4 outbound lewat virtio gagal; kalau balancerannya dikirim, itu poisoning sticky
**Test:** `crates/zenus-net/src/arp.rs` → `arp::host_tests`
- `the_reply_carries_our_identity_and_their_addresses`
- `the_answering_interface_is_chosen_by_address_not_hardcoded`
- `a_conflicting_mac_is_reported_rather_than_silently_ignored`
- `the_target_address_needs_a_long_enough_payload`

**Di mana:** `crates/zenus-net/src/nic.rs` — `net_poll`, `poll_packet`;
`crates/zenus-net/src/arp.rs`

**Yang salah (#12a):** `arp::handle` mengembalikan frame balasan, dan return-nya
diabaikan:

```rust
// sebelum
crate::arp::handle(&eth_hdr, eth_payload, &[10, 0, 2, 15], &eth_hdr.src_mac);
```

Jadi virtio NIC tidak pernah menjawab ARP. Setiap `send_packet` memanggil
`arp::resolve`, yang gagal → `None` → `return false`. Tidak ada IPv4 outbound
yang bisa keluar. Jalur RTL8139 memanggil `send_raw` dengan benar, jadi
gejalanya hanya pada virtio — yang persis device yang dipakai CI QEMU dan
container.

**Yang salah (#12b):** dua argumen terakhir salah. `our_mac` diisi
`eth_hdr.src_mac` — **MAC peminta** — dan IP di-hardcode `10.0.2.15`. Kalau
balancerannya dikirim, ethereum src-nya adalah MAC peminta, dan `arp_insert`
menolak mengganti MAC yang sudah ada (`arp.rs`). Balasan pertama yang pernah
diterima untuk IP kita akan tercatat permanen. Itu ARP poisoning yang sticky,
dilakukan kernel terhadap dirinya sendiri, dan tidak ada yang bisa
memperbaikinya karena cache tidak pernah berubah.

**Bug ketiga yang ketemu di file yang sama:** indeks interface di-hardcode `1`.
`1` hanya benar kalau probe RTL8139 keluar pertama; kalau hanya virtio yang
ada, `1` adalah loopback. `tcp`/`udp::handle_receive` juga menerima `1`
terasuk apa pun NIC asalnya.

**Bug keempat:** jalur virtio tidak memeriksa `dst_ip`, dan tidak menangani
ICMP sama sekali — jadi `ping` tidak pernah dijawab di NIC itu, dan setiap
host di segmen diteruskan ke firewall/TCP/UDP.

**Fix:**

- `build_reply(our_mac, our_ip, requester_mac, requester_ip)` — helper murni,
  dipanggil dengan `iface.ip`/`iface.mac`
- `answering_identity(target_ip, ifaces)` — memilih interface berdasarkan
  alamat, bukan konstanta
- `virtio_iface_index()` — mencari `NicType::Virtio` di tabel
- `poll_packet(iface_idx, iface, data, reply) -> Option<usize>` —
  mengembalikan frame yang harus dikirim; `net_poll` mengirimkannya dengan
  `v.send_raw` **langsung**, bukan lewat `send_frame`, karena kita sudah
  berada di dalam `with_nic` dan keluar lagi darinya akan mengambil
  `NET_LOCK` dua kali — self-deadlock yang persis sama seperti yang
  didokumentasikan pada `Rtl8139::poll`
- `classify_insert` + `arp_insert` sekarang mengembalikan `ArpInsert`, jadi
  percobaan poisoning bisa dilaporkan (`ARP: x already maps to another MAC`)
  alih-alih diabaikan diam-diam. `add_static` dipisah: entri statis
  bersifat otoritatif dan tidak tunduk pada aturan "jangan ganti MAC"
- `target_address(payload)` — pembacaan offset 24 sekarang punya batas
  eksplisit

Dua test diverifikasi gagal sebelum fix (`build_reply` dikembalikan ke MAC
peminta dan `answering_identity` ke `ifaces.first()` → 2 FAILED).

---

### BUG-013 — `brk(small)` membebaskan frame ELF program itu sendiri

**Status:** sudah di-fix (commit ini)
**Keparahan:** sangat tinggi — program bisa membebaskan kode yang sedang
dijalankan; plus hang
**Test:** `crates/zenus-syscall/src/syscall.rs` → `syscall::host_tests`
- `brk_refuses_to_shrink_below_the_heap_floor`

**Di mana:** `crates/zenus-syscall/src/syscall.rs` — `sys_brk`,
`unmap_heap_pages`; `crates/zenus-sched/` — `Task::heap_floor`

```rust
// sebelum
} else if addr < heap_start {
    unmap_heap_pages(cr3, addr, heap_start);
}
```

**Yang salah:** `heap_start` adalah `heap_brk`, yang **dimulai** di
`heap_base` dari ELF loader — sekitar `0x6000_0000_0000`. `brk(0x1000)`
menyebabkan `unmap_heap_pages(cr3, 0x1000, 0x6000_0000_0000)`, dan loop
itu menghitung:

```
(end + 0xFFF) & !0xFFF - (start & !0xFFF)  /  0x1000
= 0x6000_0000_0000 - 0x1000  /  0x1000
= 6.442.450.944 halaman
```

Dua kerusakan sekaligus:

1. **Program membebaskan dirinya sendiri.** Setiap halaman yang ter-map diberi
   `free_frame`. ELF image, stack, dan heap semua berada di antara `0x1000`
   dan heap base — jadi program mengembalikan frame kode yang sedang
   dieksekusinya ke frame allocator, yang akan diberikan ke frame lain.
2. **Hang.** 6.4 miliar iterasi, masing-masing menelusuri empat level page
   table.

Satu syscall dari ring 3, tanpa hak akses apa pun (`current_euid()` = 0 untuk
hampir semua task). Tidak ada auth, tidak ada flag.

**Fix:** `Task` punya `heap_floor` — break awal, yaitu tempat loader
menaruh heap. `brk_action(addr, current, floor, limit)` adalah helper murni
yang mengembalikan `Query` / `Grow` / `Shrink` / `Refuse`; `addr < floor`
ditolak. Jadi shrink hanya bisa pernah menutupi heap itu sendiri.

`get_task_heap_floor` mengembalikan fallback yang sama dengan
`get_task_heap_brk` (`0x6000_0000_0000`), **bukan 0** — floor 0 membuat
setiap shrink legal, dan itu arah gagal yang tidak aman.

`exec` memuat image lain, jadi floor ikut di-reset lewat
`reset_task_heap_floor`; kalau tidak, heap image baru bisa dikecilkan
kembali ke apa pun yang dipetakan program sebelumnya.

### BUG-014 — `8 * nfds` overflow, dan `recv` yang bisa meminta 128 TiB

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — validasi dilewati; alokasi kernel tak terbatas
**Test:**
- `the_pollfd_span_is_checked_before_it_is_used_as_a_length`
- `a_zero_length_span_is_refused`
- `a_single_transfer_cannot_be_unbounded`

**Di mana:** `crates/zenus-syscall/src/syscall.rs` — `sys_poll`, `sys_recv`,
`sys_recvfrom`, `sys_read`

**BUG-014a — overflow perkalian:**

```rust
// sebelum
let total_size = core::mem::size_of::<Pollfd>() as u64 * nfds;
if !validate_user_range(fds_ptr, total_size) { return -1; }
```

`size_of::<Pollfd>()` = 8, jadi `8 * nfds` wrap untuk `nfds > 2^61`. Yang
wrap adalah nilai yang **lebih kecil**, jadi ia lolos `validate_user_range` —
lalu loop di bawahnya menelusuri `nfds` entri dari buffer yang hanya
divalidasi sepanjang panjang yang sudah wrap.

**Fix:** `checked_array_len::<T>(count) -> Option<u64>`, `checked_mul` plus
filter `n > 0` (span nol berarti pemindaian dengan stride 0, juga tidak
berguna). Dipakai di `sys_poll`.

**BUG-014b — `recv` tanpa batas transfer:**

`sys_read` punya `MAX_READ = 65536`; `sys_recv` dan `sys_recvfrom` tidak punya
apa pun selain `USER_SPACE_LIMIT` = 128 TiB, lalu langsung
`alloc::vec![0u8; len as usize]`. Jadi alokasi *adalah* batasnya, dan
`recv(fd, buf, 1<<40)` tidak gagal — ia mencoba, dan kernel kehabisan memori.
Dari ring 3, tanpa auth. Sekarang keduanya memakai `MAX_XFER` yang sama
dengan `sys_read`.

Empat test diverifikasi gagal sebelum fix (floor check dihapus,
`checked_mul` diganti `*`, batas `MAX_XFER` pada recv dihapus → 3 FAILED).

---

### BUG-015 — `shmdt`: `invlpg` tanpa menulis PTE → use-after-free antar task

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — UAF; page table masih mengklaim page ter-map
**Test:** `crates/zenus-mem/src/lib.rs`
- `page_table_indices_are_split_at_the_right_levels`

**Di mana:** `crates/zenus-syscall/src/syscall.rs` — `sys_shmdt`, `sys_shmat`;
`crates/zenus-mem/src/paging.rs` — `unmap_page_raw_keep_frame`, `split_indices`

```rust
// sebelum, per page
if let Some(phys) = zenus_mem::paging::virt_to_phys_raw(cr3, virt) {
    // Drop the PTEs. The frames themselves stay owned by the SHM segment...
    unsafe {
        zenus_arch::cpu::stac();
        core::arch::asm!("invlpg [{virt}]", ...);
        zenus_arch::cpu::clac();
    }
}
```

**Yang salah (#15a):** `invlpg` **bukan** unmap. Ia hanya membatalkan entri
TLB; PTE-nya tetap PRESENT dan tetap menunjuk frame yang sama. Jadi:

1. PTE masih hidup dan menunjuk frame SHM
2. `detach` menurunkan `attached`; di nol, frame di-`free_frame`
3. Frame diberikan ke frame lain — page table kernel, buffer, task lain
4. Akses berikutnya ke alamat virtual itu membaca **memori orang lain**

Page table masih bilang page itu ter-map. Tidak ada page fault, tidak ada
`#PF`, tidak ada jejak. Dua task berbeda bisa membaca halaman yang sama lewat
"shared memory" yang sudah di-detach.

Yang membuat bug ini bertahan lama: `virt_to_phys_raw` dipanggil, jadi
terlihat seperti pemetaan memang disentuh, dan `stac`/`clac` terlihat seperti
perhatian sungguhan terhadap user page. Yang tidak ada adalah satu-satunya
instruksi yang penting: menulis 0 ke PTE.

**Yang salah (#15b):** `shmat` tidak menaikkan `attached`. `shmget` menaikkan
satu, `shmdt` menurunkan satu — jadi `shmget` + `shmat` + `shmdt` pertama sudah
membawa hitungan ke nol dan membebaskan frame **sementara PTE task ini masih
hidup**. Kedua sisi dari UAF yang sama.

**Fix:**

- `zenus_mem::paging::unmap_page_raw_keep_frame(cr3, virt)` — berjalan empat
  level, **menulis 0 ke PTE leaf**, lalu `invlpg`. Frame tidak dibebaskan:
  kepemilikannya ada di segmen SHM. `fence(SeqCst)` antara store dan
  invalidasi, karena tanpa itu CPU boleh melayani akses yang sedang kita
  matikan dari entri TBL lama.
- `shmat` menaikkan `attached` untuk pemetaannya sendiri, jadi `shmdt`
  pertama hanya melepas **pemetaan**-nya, bukan segmennya.
- `split_indices(virt)` — helper murni untuk empat shift. Tiga walk di
  `paging.rs` memakai shift yang sama dan salah satu berarti membaca tabel
  yang salah; test memverifikasi terhadap `0x3000_0000_0000` (alamat yang
  dipakai `shmat`), 0x1000, dan batas 2 MiB.

Dicatat: `mapped_at` menyimpan **satu** alamat per segmen, jadi `shmat`
kedua untuk segmen yang sama menimpa yang pertama dan pemetaan pertama
menjadi tidak bisa di-detach. Itu batas yang sudah ada sebelumnya dan tidak
disentuh di sini; perbaikannya butuh daftar attachment per segmen, bukan satu
skalar.

Test diverifikasi gagal sebelum fix (shift PML4 diganti ke level yang salah →
1 FAILED). Test full end-to-end SHM tetap butuh QEMU karena butuh page table
sungguhan; yang bisa diuji di host adalah bagian indeksnya.

---

### BUG-016 — `MS_RDONLY` diterima lalu dibuang, dan `mount` tanpa cek root

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — filesystem read-only bisa ditulis; mount/umount tanpa hak
**Test:**
- `crates/zenus-syscall/src/syscall.rs` → `ms_rdonly_survives_the_translation_into_the_vfs`
- `crates/zenus-fs/src/lib.rs` → `a_read_only_mount_produces_read_only_nodes`,
  `a_read_only_mount_does_not_cover_a_sibling`

**Di mana:** `crates/zenus-syscall/src/syscall.rs` — `sys_mount`,
`sys_umount2`, `mount_flags_from_syscall`; `crates/zenus-fs/src/vfs.rs` —
`Mount`, `VfsNode`, `mount_with_flags`, `mount_is_read_only`;
`crates/zenus-syscall/src/syscall/fd.rs` — `FdEntry::read_only`, `fd_write`

**Yang salah (#16a — flag dibuang):** `sys_mount` sudah menolak flag di luar
`MS_SUPPORTED` (perbaikan BUG-005), tapi `MS_RDONLY` **sendiri** tidak pernah
dipakai. `vfs::Mount` juga tidak punya field flag sama sekali — tidak ada tempat
untuk menyimpannya. Jadi `mount(..., MS_RDONLY)` mengembalikan 0 dan caller
mendapat view yang bisa ditulis. Tidak ada yang bisa memeriksa apa pun di
`sys_write`, karena informasinya tidak pernah ada.

**Yang salah (#16b — tanpa cek root):** `sys_mount` dan `sys_umount2` tidak
pernah memanggil `current_euid()`. Karena `current_euid()` adalah 0 untuk
hampir semua task (lihat BUG-008), setiap program bisa memasang ext2 untuk
device mana pun yang bisa ia sebut, dan melepas filesystem mana pun dari tree.

**Fix:**

- `vfs::Mount.flags: u32`, diisi lewat `mount_with_flags`
- `VfsNode.read_only: bool`, di-set dari flag mount yang cocok saat lookup.
  Dipakai sebagai field node, bukan dicari ulang saat write, supaya tidak ada
  jalur yang lupa mengecek.
- `FdEntry.read_only`, diisi dari node saat `open`, dan **dipertahankan** oleh
  `dup` dan `dup2` — kalau tidak, `dup2` jadi cara-tutorial mengubah deskriptor
  read-only menjadi bisa tulis.
- `fd_write` mengembalikan `None` (EROFS) untuk deskriptor read-only
- `mount_flags_from_syscall(flags) -> u32` murni. Hanya `MS_RDONLY` yang
  diteruskan; `MS_NOSUID`/`MS_NODEV`/`MS_NOEXEC` tetap diterima
  (`MS_SUPPORTED` tidak berubah) tapi dipetakan ke nol, karena tidak ada
  enforced suid/dev/exec di VFS. Menerima bit lalu berpura-pura menghornya
  adalah kebohongan yang sama dengan mengabaikannya, jadi apa yang mereka petakan
  dinyatakan eksplisit di sini.
- `sys_mount` dan `sys_umount2` menolak `euid != 0`
- `mount_is_read_only(path, mount, flags)` murni, dibangun di atas
  `mount_covers` supaya aturan read-only tidak bisa menyimpang dari aturan
  prefix mount

Test read-only diverifikasi gagal sebelum fix (`read_only` dipaksa `false`
dan `mount_flags_from_syscall` dikembalikan ke `0` → 1 FAILED).

---

### BUG-017 — `make fuzz-smoke` tidak bisa dikompilasi sama sekali

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — salah satu dari tiga lapisan verifikasi tidak jalan
**Test:** tidak ada test; ini compile error, jadi `cargo build` yang menjaganya

**Di mana:** `crates/zenus-sched/src/scheduler.rs` — `idle_until`;
`apps/src/fuzz_runner.rs` — `run_and_exit`

```rust
// sebelum
pub fn idle_until(cond: fn() -> bool, finished: fn() -> !) { … }

pub fn run_and_exit(mode: Mode, cases: u64) -> ! {
    …
    zenus_sched::scheduler::idle_until(watchdog, || abort("watchdog"));
}
```

**Yang salah:** `idle_until` tidak pernah kembali — `finished` adalah
`fn() -> !` dan blok asm-nya diakhiri `ud2` — tapi tipenya `()`. Inline asm
tidak dianggap divergen oleh compiler, jadi `idle_until(...)` sebagai
**tail expression** dari fungsi `-> !` tidak lolos typecheck:

```
error[E0308]: mismatched types
  --> apps/src/fuzz_runner.rs:126:60
   |
126 | pub fn run_and_exit(mode: Mode, cases: u64) -> ! {
   |        ------------                                        ^ expected `!`, found `()`
```

Artinya `make fuzz-smoke`, `make fuzz-coverage`, dan `make fuzz-regression`
**gagal build** di HEAD. Tidak ada fuzzing yang pernah berjalan di repo ini,
dan karena `doc/fuzzing.md` dan `README.md` memperlakukannya sebagai lapisan
verifikasi, ini satu lapisan yang hilang tanpa terlihat.

**Fix:** `idle_until` dideklarasikan `-> !` dengan `unreachable!()` setelah
blok asm (asm-nya sudah tidak bisa keluar). Satu-satunya caller adalah
`run_and_exit`, jadi tidak ada efek samping.

**Verifikasi:** `make fuzz-smoke` sekarang build dan **jalan** — banner,
campaign task dibuat, framework terinisialisasi. denounced di bawah bahwa
kampanye masih belum menyelesaikan diri.

### BUG-018 — entry `testing` kehilangan inisialisasi APIC

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — `make test` crash sebelum test pertama
**Test:** tidak ada test; ini crash di hardware-ish, butuh QEMU

**Di mana:** `apps/src/lib.rs` — `entry()` yang `#[cfg(feature = "testing")]`

**Yang salah:** entry testing menyalin urutan boot dari entry normal, tapi
salinannya sudah berbeda: tidak ada `apic::init_with_virt`. Padahal
`keyboard::init()` — dipanggil beberapa baris kemudian — me-route IRQ1 lewat
IOAPIC dan butuh APIC id, dan `current_apic_id()` adalah **read MMIO LAPIC**:

```
TYPE: supervisor-read-nonpresent
ADDR=0x0000000000000020 RIP=0xFFFFFFFF8003ED5C
#2 zenus_arch::interrupts::apic::lapic_read (reg=32) at crates/zenus-arch/src/interrupts/apic.rs:19
#3 zenus_arch::interrupts::apic::current_apic_id () at crates/zenus-arch/src/interrupts/apic.rs:61
#4 zenus_arch::keyboard::init () at crates/zenus-arch/src/keyboard.rs:96
#5 zenus::entry () at apps/src/lib.rs:724
```

`LAPIC_VIRT_BASE` masih 0, jadi alamatnya `0 + 0x20`. `#PF` tepat sebelum test
pertama jalan. Dikonfirmasi pre-existing: worktree di `c8b7fce` (sebelum
seluruh commit di sini)WYSIWYG crash identik.

**Fix:** blok APIC + PIT + tick source disalin ke entry testing, persis seperti
entry normal. Sekalian `enable_tick_source(32)` literal diganti
`interrupts::TIMER_VECTOR` — konstanta yang sama, tapi sekarang kalau vektor
timer berubah hanya ada satu tempat yang perlu diubah (`ARCHITECTURE.md`
menyebut ini sebagai yang layak dibetulkan).

### BUG-019 — hasil test in-kernel tidak pernah masuk ke serial

**Status:** sudah di-fix (commit ini)
**Keparateh:** sedang — `make test` melaporkan "terminated" tanpa output
**Test:** tidak ada test; ini masalah output, bukan logika

**Di mana:** `apps/src/test_runner.rs` — `run_tests`

**Yang salah:** `SerialPort::write_str` tidak menulis ke UART, ia menumpuk ke
`OUTPUT_BUF`, dan yang mengosongkan buffer itu adalah scheduler atau
`flush_output_blocking()` eksplisit. Tapi suite test dijalankan **di boot CPU**,
tanpa task, tanpa scheduler — jadi tidak ada yang pernah flush. Mesin lalu
`hlt` selamanya dengan hasil test masih di buffer.

Akibatnya `make test-quiet` (grep `\[TEST\]`) tidak pernah menemukan apa pun dan
make melaporkan `Error`, yang identik dengan gejala suite yang hang. Dua
lapisan yang menutupi satu sama lain.

**Fix:** `flush_output_blocking()` setelah tiap test dan setelah ringkasan.

**Belum selesai / diketahui sekarang (lihat bagian di bawah).**

---

### BUG-020 — `idle_until` memanggil pointer fungsi basi di putaran kedua

**Status:** sudah di-fix (commit ini)
**Keparahan:** sangat tinggi — lompat ke `.bss`, instruction-fetch fault
**Test:** tidak bisa jadi host test — ini level register dalam inline asm.
Buktinya Serial output sebelum/sesudah, ada di bawah

**Di mana:** `crates/zenus-sched/src/scheduler.rs` — `idle_until`

```asm
; sebelum — rsi dan rdi dipilih compiler untuk check dan finished
3:      call *%rdi          ; cond
        test %al,%al
        jnz  4f
        sti
        hlt
        jmp  3b             ; ← kembali ke call *%rdi
```

**Yang salah:** `check` dan `finished` diletakkan di register pilihan compiler
(`rdi` dan `rsi`), lalu loop memanggilnya lagi. Tapi `call *%rdi` memanggil
fungsi Rust yang **boleh** mengubah register caller-clobbered — dan `rdi`/`rsi`
termasuk. Jadi:

1. Putaran pertama `call *%rdi` → `watchdog()` jalan
2. `watchdog()` memanggil `kinfo!` → format dan cetak → **`%rdi` berubah**
3. `hlt`, timer tick, `jmp 3b`
4. Putaran kedua `call *%rdi` → memanggil **isi register itu**, yaitu
   `fuzz_runner::LAST_REPORT`, sebuah `AtomicU64` di `.bss`

Hasilnya persis yang tercatat di serial:

```
!!! PAGE FAULT !!!
TYPE: supervisor-write-nonpresent [IF]
ADDR=0xFFFFFFFF805B9008 RIP=0xFFFFFFFF805B9008 CAUSE=instruction-fetch
```

`ADDR == RIP`: control sudah pindah ke sana, lalu CPU mencoba mengeksekusi
data. `nm` mengonfirmasi `0xffffffff805b9008` adalah
`zenus::fuzz_runner::LAST_REPORT + 0x0`.

Yang membuatnya bertahan: pointer masuk dengan benar. Di breakpoint entry
`idle_until`, gdb melaporkan
`cond=0xffffffff80004d90 <zenus::fuzz_runner::watchdog>` — jadi tidak ada yang
salah di sisi pemanggil. Kerusakan baru terjadi *di dalam* loop, di iterasi
kedua, yang tidak terlihat dari breakpoint mana pun.

`idle()` tidak terkena: pointer-nya `sym idle_yield_bridge`, jadi assembler
yang me-resolve ke alamat tetap, dan tidak ada register yang harus bertahan
melewati panggilan.

**Fix:** pointer masuk ke `static UNTIL_CHECK` / `UNTIL_FINISHED`, dan loop
memuat ulang dari slot itu **tepat sebelum** tiap `call`. Register yang
di-clobber tidak bisa berpengaruh karena nilainya dibaca ulang dari memori
setiap putaran. `sym` tidak bisa dipakai karena `cond`/`finished` adalah nilai
runtime, bukan fungsi tetap.

**Bukti (before → after), `make fuzz-smoke`:**

| | Sebelum | Sesudah |
|---|---|---|
| Fault | `instruction-fetch` di `0xffffffff805b9008` | tidak ada |
| `[FUZZ] SUMMARY` | tidak pernah tercetak | `cases=313 crashes=6 hangs=0 new_paths=66 corpus=1151 edges=69 faults=0 unrecovered=0` |
| `[FUZZ] EXIT` | tidak pernah tercetak | `code=3` (watchdog — lihat BUG-021) |
| Crash ditemukan | 0 | **6** |

Efek sampingnya: campaign sekarang **menemukan 6 crash asli** yang selama ini
tidak pernah terlihat, karena fuzzing tidak pernah berjalan sama sekali.

Tidak ada regression test untuk ini. `idle_until` berisi `cli`/`sti`/`hlt`,
yang di ring 3 fault — jadi hanya lapisan in-kernel yang bisa mengujinya, dan
suite itu sendiri belum selesai (BUG-023). Yang bisa dijadikan bukti adalah
output serial di atas, dan itu direkam.

---

### BUG-022 — enam syscall menulis ke user pointer tanpa cek halaman ter-map

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — page fault di ring 0, bukan `EFAULT`
**Ditemukan oleh:** `make fuzz-smoke` (BUG-020 membuatnya bisa jalan)
**Test:** `crates/zenus-syscall/src/syscall.rs` → `syscall::host_tests`
- `no_syscall_writes_to_user_space_through_a_raw_pointer`
- `a_scalar_copy_helper_and_the_range_check_agree_on_what_is_addressable`

**Di mana:** `sys_wait4` (status), `sys_getsockname` (addrlen),
`sys_rt_sigprocmask` (oldset), `sys_getsockopt` (optval + optlen),
`sys_socketpair` (fds), `sys_pipe2` (fds)

```rust
// bentuk yang dipakai keenamnya
if validate_user_range(ptr, 8) {
    *(ptr as *mut u64) = value;
}
```

**Yang salah:** `validate_user_range` hanya memeriksa bahwa alamatnya **di
dalam rentang user** — `ptr >= 0x1000` dan ujungnya `< USER_SPACE_LIMIT`.
Ia tidak memeriksa bahwa halamannya ter-map. Setelah itu, store langsung
menulis lewat pointer.

Jadi pointer in-range tapi tidak ter-map → `#PF` di ring 0, bukan `EFAULT`.
Dan karena SMAP **dimatikan** (`SECURITY.md` gap #1), tidak ada jalur fixup
`stac` yang bisa mengubahnya jadi error syscall. Program yang mengirim
pointer seperti itu menjatuhkan kernel, bukan menerima `EFAULT`.

`copy_kernel_to_user` sudah benar sejak awal: ia `checked_add` rentangnya,
mengambil CR3, lalu **revalidasi per halaman** dengan `virt_to_phys_raw`
sebelum setiap chunk. `SECURITY.md` bahkan menyebutnya sebagai kontrol yang
dipakai. Enam call site itu tidak memakainya — mereka hanya memakai
`validate_user_range` yang hanya setengah-setengah.

**Bukti dari fuzzer:**

```
[FUZZ] CRASH ZENUS-FUZZ-000002 subsystem=0 type=PAGE_FAULT vector=14
  rip=0xffffffff80029b8f addr=0x7075 err=0x2
  args=[0, 4, 2f, 6d, 6e, 74, 0, 0, ...]
```

`addr=0x7075` in-range, tidak ter-map. `err=0x2` = write dari user. `rip`
mengarah ke `sys_rt_sigprocmask` (`addr2line` →
`crates/zenus-syscall/src/syscall.rs:1799`, yang persis baris
`*(oldset_ptr as *mut u64) = old_mask;`). `args` mulai `[0, 4, '/', 'm',
'n', 't']` = path "/mnt" — input fuzzer yang memancingnya.

**Fix:** tiga helper — `copy_u64_to_user`, `copy_u32_to_user`,
`copy_u64_pair_to_user` — semuanya menyeberang ke `copy_kernel_to_user`, jadi
mendapat revalidasi per halaman. `rt_sigprocmask`, `getsockopt` dan `pipe2`/
`socketpair` sekarang mengembalikan `-1` kalau copy-nya gagal (dan
`socketpair`/`pipe2` menutup fd yang sudah dibuat, supaya tidak bocor).

**Yang sengaja tidak diubah:** empat store di `execve` yang menulis stack awal
user milik address space yang baru dibuat loader. Alamatnya dari
`loaded.stack_top`, bukan dari caller.

**Test:** store-nya sendiri butuh page table hidup, jadi yang dikunci di sini
adalah **invariant**-nya — tidak ada syscall yang boleh menembak lewat store
pointer mentah ke user space. `include_str!("syscall.rs")` + pemindaian
menangkap site baru pada hari yang sama. Needle-nya dirakit dari
`concat!` supaya test ini tidak mendeteksi dirinya sendiri. Diverifikasi gagal
sebelum fix (satu site dikembalikan ke bentuk lama → 1 FAILED).

Bug kedua yang ketemu di file yang sama: `execve` punya **dua** loop yang
menulis argv pointer array. Loop pertama ("Actually let me rewrite more
carefully" mengikutinya) langsung ditimpa, dan `str_cur2` di loop pertama
menghitung indeks dari pointer yang sudah digeser — logika yang salah tapi
tidak terlihat. Tidak disentuh di commit ini; layak cleanup tersendiri.

---

### BUG-023 — `sys_mprotect` menjalankan loop halaman tanpa batas

**Status:** sudah di-fix (commit ini)
**Keparahan:** sangat tinggi — **bukan** bug khusus fuzz. `mprotect` adalah syscall biasa
**Ditemukan oleh:** `make fuzz-smoke` (BUG-020 membuatnya bisa jalan)
**Test:** `mprotect_refuses_a_range_it_would_have_to_walk_forever`

**Di mana:** `crates/zenus-syscall/src/syscall.rs` — `sys_mprotect`

```rust
// sebelum
let end_page = ((addr + length) + 0xFFF) & !0xFFF;
let mut page = start_page;
while page < end_page {
    zenus_mem::paging::protect_page_raw(cr3, page, writable, executable);
    page += 0x1000;
}
```

**Yang salah:** tidak ada cek overflow **dan** tidak ada batas atas.
`addr + length` wrap bisu, dan tidak ada yang membatasi rentangnya.

Yang fuzzer berikan:

```
[FUZZ] TIMEOUT … stuck_in_case=313 subsystem=0 syscall=10
[FUZZ] stuck args=[7665002f706d742f, 6c6900001000, 100000000000000, 0, 0, 0]
```

`syscall=10` adalah `SYS_MPROTECT`. `args[0] = 0x7665002f706d742f`,
`args[1] = 0x6c6900001000` — rentangnya **3,4 GB**, jadi ~870 000 iterasi,
masing-masing memanggil `protect_page_raw` yang menyusuri empat level page
table lalu `invlpg`. Tidak ada apa pun di loop itu yang bisa menghentikannya.

**Ini bukan temuan eksotis.** `mprotect` dipanggil program mana pun; cukup
satu `length` yang salah dan mesinnya menggantung. `map_heap_pages` sudah punya
keluar `page >= USER_SPACE_LIMIT`; `mprotect` tidak pernah mendapatkannya.

Bug ini adalah **pemblokir campaign**: sebelum diperbaiki, `make fuzz-smoke`
berhenti di kasus 313 dari 2000. Sesudah diperbaiki, campaign menyelesaikan
2000 kasus dalam ~10 detik.

**Fix:** `mprotect_range(addr, length, limit) -> Option<(start_page, end_page)>`
murni yang menolak `length == 0`, overflow, dan apa pun yang menyentuh
`USER_SPACE_LIMIT`.

Bug keempat yang ketemu di sini: `end_page = (end + 0xFFF) & !0xFFF` untuk
`end` yang sudah page-aligned **tidak benar** — `0x3000` membulatkan ke
`0x3000`, sehingga halaman yang memuat byte `0x3000` sendiri hilang. Bentuk
benarnya `(end | 0xFFF) + 1`. Ini sedikit di luar bug utama, tapi test
menangkapnya sebelum di-commit.

### BUG-024 — `sys_nanosleep` membandingkan milidetik dengan tick

**Status:** sudah di-fix (commit ini)
**Keparahan:** sedang — semua sleep 100× terlalu lama; input fuzzer bisa memblokir permanen
**Ditemukan oleh:** `make fuzz-smoke`
**Test:** `a_sleep_deadline_is_in_ticks_and_cannot_wrap`

```rust
// sebelum
let total_ms = sec.saturating_mul(1000).saturating_add(nsec / 1_000_000);
…
if elapsed >= total_ms { break; }     // elapsed adalah TICK
```

**Yang salah:** `pit::get_ticks` menghitung **tick** (10 ms), tapi
perbandingannya memakai **milidetik**. `nanosleep(1, 0)` tidur **10 detik**.
`nanosleep(0, 10_000_000)` tidur 100 ms, bukan 10 ms.

Selain itu `nsec` tidak pernah diperiksa terhadap batas 1e9 — POSIX bilang
`EINVAL` — dan kode ini sama sekali tidak memeriksanya. `ticks` juga tidak
disaturasi secara bermakna:
1,8e19 tick. Task itu lalu `yield_now()` selamanya — persis kondisi yang
membuat watchdog melapor `TIMEOUT` dengan `hangs=0`.

**Fix:** `nanosleep_deadline(now, sec, nsec)` murni, membulatkan **ke atas**
(permintaan tidak boleh kembali lebih awal), menolak `nsec >= 1e9`, dan
menolak deadline yang akan melewati `u64::MAX`.

### BUG-025 — 17 syscall membentuk referensi user space tanpa cek halaman

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — `#PF` di ring 0, bukan `EFAULT`, dari 17 syscall
**Ditemukan oleh:** `make fuzz-coverage`
**Test:** `no_syscall_reaches_user_space_through_a_raw_pointer`

**Ini kelanjutan langsung dari BUG-022.** Setelah enam site *scalar* diperbaiki,
`make fuzz-coverage` menemukan bentuk yang sama dalam bentuk lain:

```
[FUZZ] CRASH ZENUS-FUZZ-000001 type=PAGE_FAULT
  rip=0xffffffff8002e0be addr=0x10000000000 err=0x2
  input=[06, 09, 2f, 74, 6d, 70, 38, 66, 75, 7a, 7a, ...]   ← "/tmp8fuzz"
```

`addr2line` → `sys_stat`, dan barisnya:

```rust
let stat_buf = stat_ptr as *mut StatBuf;
unsafe { (*stat_buf).st_size = stat.size; … }
```

Tujuh belas syscall melakukan ini lebih atau kurang sama: mereka menerima
alamat dari caller lalu langsung membentuk `&mut *(ptr as *mut T)` atau
`let p = ptr as *mut T` lalu menulis lewat `(*p).field`. Semuanya hanya
melewati `validate_user_range`, yang **hanya** membatasi rentang alamat — bukan
mengecek apakah halamannya ada. Dengan SMAP mati, tidak ada jalur fixup yang
bisa mengubahnya jadi `EFAULT`.

Bentuk yang berbeda dari BUG-022 (`as *mut u64) =`), jadi test yang mengunci
kelas itu saja tidak menangkapnya — needle-nya diperluas ke `&mut *(` dan
`&*(`.

**Fix:** `unsafe fn user_ptr<T>(ptr) -> Option<*mut T>` +
`fn user_pages_mapped(ptr, len) -> bool`. `user_ptr` memvalidasi rentangnya
**dan** mengecek tiap halaman dengan `virt_to_phys_raw` sebelum/Reference-nya
dibentuk. Ketujuh belas site sekarang lewat situ, dan yang gagal mengembalikan
`-1` (`EFAULT`) alih-alih fault.

**Test diperluas** untuk menutup bentuk yang terlewat, dan memindai hanya
bagian file sebelum `mod host_tests` supaya test tidak mendeteksi dirinya
sendiri. Diverifikasi gagal sebelum fix (satu `sys_poll` dikembalikan ke
`&mut *(offset as *mut Pollfd)` → 1 FAILED).

### BUG-026 — verdict yang benar ditimpa `TIMEOUT` palsu

**Status:** sudah di-fix (commit ini)
**Keparahan:** sedang — laporan hasil berbohong
**Test:** tidak ada test; ini perilaku harness

**Di mana:** `Makefile` (`FUZZ_TIMEOUT`), `apps/src/fuzz_runner.rs`

**Yang salah:** `make fuzz-smoke` menjalankan QEMU dengan `-no-shutdown`, jadi
permintaan S5 dari `poweroff()` **selalu diabaikan**. Kalau campaign selesai,
`finish()` mencetak SUMMARY dan EXIT code yang benar, lalu memanggil
`poweroff()`, yang tidak pernah menyala → terjebak di loop `hlt`
`shutdown_via_acpi`. Boot task ternyata masih jalan, watchdog melihat
`CAMPAIGN_DONE`, memanggil `finished`, dan **mencetak `TIMEOUT` di atas verdict
yang benar**, dengan SUMMARY kedua dan `EXIT code=3` kedua. Run yang lulus
terbaca seperti run yang hang.

**Fix:** dua bagian.
1. `-no-shutdown` dihapus dari `fuzz_run`, jadi `poweroff()` benar-benar
   bekerja dan QEMU keluar dengan kode yang benar.
2. `EXITING` di-set di awal `finish()`, `abort()`, dan
   `run_regression_and_exit()`. Selama flag itu menyala, watchdog hanya
   menunggu — jadi kalau poweroff gagal, verdict yang sudah terbit
   tetap berdiri sendiri.

### Diagnostik harness: sekarang hang bisa dinamai

Sebelum BUG-023 bisa ditemukan, campaign tidak bisa bilang **input mana** yang
membuatnya macet. `CASE_COUNTER` hanya memberi tahu seberapa jauh campaign
sudah berjalan, dan angkanya sama untuk "berhenti bersih di kasus 313" dan
"macet di dalam kasus 313".

Ditambahkan:
- `zenus_fuzz::CURRENT_CASE` — indeks kasus yang sedang jalan, dipublikasikan
  **sebelum** kasus itu jalan
- `syscall_fuzz::CURRENT_SYSCALL` dan `CURRENT_ARGS[6]` — dipublikasikan
  sebelum dispatch

Baris `TIMEOUT` sekarang mencantumkan semuanya, dan baris `waiting`
mencantumkan kasus yang sedang jalan. Karena seed-nya tetap, indeks kasus
sudah cukup untuk mereplikasinya — itulah yang membuat BUG-023 bisa
ditemukan dalam satu putaran, bukan dalam satu minggu.

---

## Hasil: ketiga target fuzz sekarang jalan

| Target | Sebelum | Sesudah |
|---|---|---|
| `make fuzz-smoke` | **tidak bisa dikompilasi** | 2000 kasus, `crashes=0`, `EXIT code=0`, ~10 s |
| `make fuzz-regression` | tidak bisa dikompilasi | jalan; melaporkan `NO-CORPUS` dengan benar (`EXIT code=3` masih salah — lihat catatan) |
| `make fuzz-coverage` | tidak bisa dikompilasi | 5506 kasus, `crashes=4`, `EXIT code=3` |

`fuzz-smoke` discovering → fixing → clean dalam satu sesi:

| Run | Hasil |
|---|---|
| pertama (setelah BUG-020) | `cases=313 crashes=6` |
| setelah BUG-022 | `cases=313 crashes=0` |
| setelah BUG-023 | `cases=2000 crashes=0` `EXIT code=0` |

### Yang masih ditemukan `fuzz-coverage` — pekerjaan berikutnya

Tidak diperbaiki di commit ini; invoice-nya sudah sempit:

| Site | Bentuk |
|---|---|
| `sys_stat` (`syscall.rs:613`) | `let stat_buf = stat_ptr as *mut StatBuf;` lalu `(*stat_buf).field = …` |
| `sys_write`-jalur (`syscall.rs:639`) | `let dst = buf as *mut u8;` |
| `sys_pipe` (`syscall.rs:925`) | `let dst = pipefd_ptr as *mut u32;` |
| `sys_execve` (`syscall.rs:1044`) | `*(arg_ptr_ptr as *const u64)` |
| `sys_sethostname`/`gethostname` (`:1184`, `:1254`) | `let uts = buf as *mut UtsName;` |
| `sys_sendto`/`recvfrom` (`:1435`, `:1487`) | `from_raw_parts(buf_ptr as *const u8, len)` — slice user space |
| `sys_rt_sigaction` (`:1805`, `:1817`) | `oldact_ptr as *mut KernelSigAction` |
| `sys_rt_sigprocmask` set (`:1937`) | `*(set_ptr as *const u64)` |
| (`:3337`) | `*(buf_ptr as *mut u8).add(n) = 0` |

Semuanya bentuk yang sama: pointer mentah ke user space tanpa `user_ptr()`.
Penargetannya jelas sekarang, dan `user_ptr()` sudah ada.

### Catatan lain dari campaign

- `EXIT code=3` untuk `fuzz-regression` belum benar. README bilang harusnya
  `2` ("corpus kosong = run ini tidak membuktikan apa-apa"). Jalur
  `regression_verdict` sudah mengembalikan verdict yang benar, tetapi
  `run_regression_and_exit` mengabaikannya dan selalu `emit_exit(3)`.
- Record crash `type=UNKNOWN rip=0x0 input=[00]` adalah noise harness:
  `run_case` mencatat fault untuk kasus yang sebenarnya tidak fault, karena
  `fuzz_guard::arm` me-*reset* `LAST_VECTOR`/`LAST_RIP`, dan tidak ada yang
 mengisinya lagi untuk kasus yang tidak fault. Bukan bug kernel.
- `doc/fuzzing.md` dan `README.md` menyebut smoke sebagai "2 000 cases". Itu
  sekarang benar untuk pertama kalinya.

---

---

### BUG-027 — 11 site pointer mentah ke user space, plus `send` tanpa batas panjang

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — `#PF` di ring 0; `send` bisa meminta 128 TiB
**Ditemukan oleh:** `make fuzz-coverage`
**Test:** `no_syscall_reaches_user_space_through_a_raw_pointer` (diperluas),
`every_transfer_shares_one_length_bound`

**Ini BUG-025, putaran kedua.** `fuzz-coverage` menemukan bentuk yang
sekarang tidak lolos dari scan yang sama:

```
[FUZZ] CRASH ZENUS-FUZZ-000001 type=PAGE_FAULT
  rip=0xffffffff8002e0be addr=0x10000000000 err=0x2
  input=[06, 09, 2f, 74, 6d, 70, 38, 66, 75, 7a, 7a, ...]   ← "/tmp8fuzz"
```

`addr2line` → `sys_stat`. Dan `syscall.rs` ternyata punya **tujuh** bentuk
tertulis dari bug yang sama — masing-masing ditemukan pada putaran fuzzer yang
berbeda:

| Bentuk | Contoh | Site |
|---|---|---|
| scalar store | `*(ptr as *mut u32) = v` | (BUG-022, sudah) |
| struct reference | `&mut *(ptr as *mut Pollfd)` | (BUG-025, sudah) |
| `let` lalu deref | `let p = ptr as *mut StatBuf; (*p).st_size = …` | `sys_stat` |
| struct, nama lain | `let uts = buf as *mut UtsName;` | `sys_uname`, `sys_uname_ns` |
| struct | `let rl = &mut *(rlim_ptr as *mut Rlimit)` | `sys_getrlimit` |
| slice | `from_raw_parts(buf_ptr as *const u8, len)` | `sys_send`, `sys_sendto` |
| pointer lalu tulis | `let dst = pipefd_ptr as *mut u32; dst.write(..)` | `sys_pipe` |

Sisanya (`sys_execve` jalannya argv, `sys_rt_sigaction`, `sys_rt_sigprocmask`
untuk `set`, `sys_getcwd`, `sys_readdir`, `sys_poll`) sudah ikut BUG-022/025
atau diperbaiki di sini.

**Fix:** semuanya lewat `user_ptr()` (struct), `user_bytes()` (slice), atau
`copy_bytes_to_user` / `copy_u32_pair_to_user` (byte). Test invariant-nya
diperluas dari lima needle ke tujuh, danpemindaiannya dipangkas mulai
`mod host_tests` supaya test tidak mendeteksi dirinya sendiri. Diverifikasi
gagal sebelum fix dengan tiga site dikembalikan ke bentuk lamanya → 1 FAILED.

**Bug kedua yang ketemu sekalian: `send` dan `sendto` tidak punya batas
panjang sama sekali.** `recv` sudah dapat `MAX_XFER` di commit BUG-014; dua
ini terlewat, jadi `sys_send` meminta socket layer membaca 128 TiB dari user
space. Sekarang ketiganya berbagi `MAX_XFER`, dan satu test mengunci bahwa
memang mereka berbagi.

### BUG-028 — `sys_mmap` overflow, tanpa batas, dan bocorkan frame

**Status:** sudah di-fix (commit ini)
**Keparahan:** tinggi — satu `mmap` besar bisa menghabiskan seluruh memori
**Ditemukan oleh:** `make fuzz-coverage`
**Test:** `an_mmap_extent_rounds_up_without_wrapping`

Setelah BUG-025, `fuzz-coverage` berhenti di kasus 5506:

```
[FUZZ] TIMEOUT … stuck_in_case=5506 subsystem=0 syscall=9
[FUZZ] stuck args=[706d742f, 100000010, 0, 0, 0, 0]
```

`syscall=9` adalah `SYS_MMAP`, `length = 0x100000010` — 4 GiB di mesin 2 GiB.

**Tiga bug di satu fungsi:**

1. `((length + 0xFFF) & !0xFFF) / 0x1000` memakai `+` biasa. Length dekat
   `u64::MAX` wrap ke page count **kecil**, sementara VMA tetap dicatat dengan
   ukuran yang berbeda dari halaman yang dipetakan.
2. **Tidak ada batas atas milik sendiri.** `mmap` di sini memetakan
   *eager*, satu halaman demi satu, jadi ukuran permintaan **adalah** biayanya.
   Loop berjalan 1 048 577 kali — tiap iterasi mengambil lock frame allocator
   dan menyusuri page table — sebelum gagal di frame ke ~500 000. Itu menit
   pekerjaan emulasi untuk mapping yang mustahil berhasil.
3. **Semua frame yang sudah dipetakan dibocorkan** ketika allocator habis:
   `None => return -1i64 as u64` tanpa membersihkan apa pun.

**Fix:** `mmap_extent(length, max_size, free_frames)` murni, yang memakai
`checked_next_multiple_of` (membulatkan ke atas **dan** melaporkan overflow),
menolak ukuran yang tidak bisa di-back oleh jumlah frame bebas, dan
mengembalikan `(page_count, size)` yang konsisten. Di `sys_mmap`, ketika gagal
di tengah, halaman yang sudah dipetakan sekarang di-unmap dan frame-nya
dilepas — lewat `zenus_mem::paging::unmap_page_raw`, pasangan baru dari
`unmap_page_raw_keep_frame` yang sudah ada sejak BUG-015.

### Yang **tidak** ketemu, dan alasannya

`fuzz-coverage` masih tidak selesai, tapi sekarang macet dengan cara yang
**berbeda** — dan itu sendiri informasinya:

```
[FUZZ] progress cases=5500 crashes=0 edges=465 corpus=1454
[FUZZ] waiting cases=2561 stuck_in_case=2561 elapsed=4s
(nol output lagi selama 200 detik; tidak ada baris TIMEOUT)
```

Kasus yang macet berubah dari run ke run (corpus ikut tumbuh), dan yang penting
begini: **watchdog ikut mati**. Baris `waiting` terakhir ada di `elapsed=4s`,
lalu tidak apa-apa selama 200 detik lagi. Kalau hanya campaign task yang macet,
boot task masih akan menjadwalkan dan watchdog akan mencetak `TIMEOUT` di
detik ke-120 — persis yang terjadi di BUG-023 dan BUG-024.

Jadi ini bukan "campaign task sedang looping". **Seluruh CPU tersendat**:
kemungkinan spinlock yang dipegang task selagi loop tak berujung, lalu timer
ISR mencoba lock yang sama dan menggantung dengan IF=0. Persis mode
kegagalan yang `AGENTS.md` peringatkan ("Anything called from an ISR must not
take a lock that a task can hold") dan yang `SpinLock::long_spin_report`
seharusnya laporkan — tapi tidak ada baris `SPIN:` di log, jadi laporkanannya
sendiri tidak pernah sampai ke UART.

`crashes=0` sepanjang perjalanan: tidak ada `#PF` yang lolos. Yang belum
diketahui adalah **site** mana. Itu butuh breakpoint di `SpinLock::lock`, dan
simbol generik `SpinLock<T>::lock` tidak bisa di-breakpoint lewat nama karena
yang ada di symbol table adalah bentuk mangled per-tipe — perlu address tiap
instansiasi, atau `rbreak` setelah symbol statiknya ditemukan.

---

---

### BUG-029 — `heap_floor` tidak pernah di-set untuk task kernel, jadi check `brk` jadi tak berguna

**Status:** sudah di-fix (commit ini)
**Keparahan:** sangat tinggi — satu syscallwedges seluruh CPU
**Ditemukan oleh:** `make fuzz-coverage`, dengan gdb
**Test:** `crates/zenus-sched/src/lib.rs` → `a_kernel_task_gets_a_heap_floor_too`

**Ini complementos langsung BUG-013.** Check `brk` yang saya tambahkan di sana
hanya berlaku untuk task yang dibuat lewat `create_user_task` dan
`clone_task`. `create_task_named` — yang dipakai untuk **setiap** task kernel,
termasuk init dan task campaign fuzzing — **tidak pernah mengisi `heap_brk`
maupun `heap_floor` sama sekali**. Keduanya tetap 0.

Dan dua accessor itu tidak memperlakukan 0 secara sama:

```rust
// get_task_heap_brk:  brk == 0  →  pakai default  (aman)
pub fn get_task_heap_brk(id: u64) -> u64 {
    … if brk == 0 { return 0x6000_0000_0000u64; } …

// get_task_heap_floor: floor == 0  →  dikembalikan apa adanya (BERBAHAYA)
pub fn get_task_heap_floor(id: u64) -> u64 {
    … return task.heap_floor; …        // 0 lolos
}
```

Jadi untuk task kernel: `current = 0x6000_0000_0000`, `floor = 0`.
`brk_action(0x61706e69, 0x6000_0000_0000, 0, USER_SPACE_LIMIT)` →
`addr >= floor` (karena `0x61706e69 >= 0`) → **Shrink**. Dan `Shrink`
menjalankan `unmap_heap_pages(cr3, 0x61706e69, 0x6000_0000_0000)`: 6,4 miliar
halaman, tiap iterasi mengambil lock frame allocator.

**Cara saya memastikan.** `gdb` tidak bisa attach ke QEMU yang sedang berjalan
dengan `interrupt` setelah `continue &` — QEMU gdbstub menolaknya ("Selected
thread is running"). Yang berhasil: QEMU dijalankan dengan monitor socket
(`-monitor unix:…`), CPU di-`stop` lewat monitor, lalu gdb di-attach (all-stop
menghentikan CPU saat connect). Backtrace-nya langsung:

```
#0 zenus_mem::paging::virt_to_phys_raw (…) at crates/zenus-mem/src/paging.rs:238
#1 zenus_syscall::syscall::unmap_heap_pages (cr3=…, start=1633771873,
      end=105553116266496)            at crates/zenus-syscall/src/syscall.rs:775
#2 zenus_syscall::syscall::sys_brk (addr=1633771873, …) at …:898
#3 zenus_syscall::syscall::syscall_dispatch6 (num=12, arg1=1633771873, …)
#4 zenus_fuzz::syscall_fuzz::execute (input=…)
#7 zenus_fuzz::run_campaign (mode=Coverage, cases=50000, seed=…)
```

`1633771873 = 0x61706e69` — empat byte ASCII dari path fuzzer. `end =
105553116266496 = 0x6000_0000_0000` — default break. Dan `unmap_heap_pages`
tidak pernah melakukan `yield_now()`, jadi campaign task tidak pernah
menyerahkan CPU:Inilah kenapa watchdog ikut mati (BUG-028) — bukan karena
lock, tapi karena loop-nya tidak pernah preemptible.

Sebelum fix, **RIP identik 8 dari 8 sampel** (`Atomic<u64>::load` yang
dari dalam loop), dan `RSP` berada di dalam loop — bukan di lock. Jadi ini
bukan deadlock spinlock seperti yang saya duga; itu loop murni yang tidak pernah
preemptible.

**Fix:**
1. `DEFAULT_HEAP_BRK` jadi konstanta tunggal, dipakai oleh kedua accessor —
   supaya tidak bisa menyimpang lagi.
2. `get_task_heap_floor` memperlakukan `floor == 0` sebagai "belum
   diinisialisasi" dan memakai default, sama seperti `get_task_heap_brk`
   sudah memperlakukan `brk == 0`.
3. `create_task_named` mengisi `heap_brk` **dan** `heap_floor` dengan default.

**Test:** membuat task bukan host-safe (`create_task_named` membaca CR8 dan
mengalokasikan stack asli), jadi test-nya memeriksa **invariant**-nya: body
`create_task_named` harus berisi kedua assignment, accessor harus mengecek
nol, dan alamat brk yang di-fuzz harus jatuh di bawah default. Diverifikasi
gagal sebelum fix (dua assignment dihapus → 1 FAILED).

### BUG-021 Closed: anggaran watchdog ternyata sudah cukup

Saya sempat mencatat throughput campaign sebagai "2,6 kasus/detik, satu kasus
~380 ms, tidak jelas mengapa". **Ternyata itu `brk`**: setiap kasus fuzzer yang
menyentuh task kernel menjalankan loop 6,4 miliar halaman itu, dan campaign
hanya bisa sedalam 313 kasus dalam 120 detik karena itu. Setelah BUG-029, kedua
mode menyelesaikan_cases-nya di bawah watchdog:

| | Sebelum | Sesudah |
|---|---|---|
| `make fuzz-smoke` | tidak bisa dikompilasi | 2000 kasus, `crashes=0`, `EXIT code=0` |
| `make fuzz-coverage` | tidak bisa dikompilasi | **50 000 kasus**, `crashes=0`, `EXIT code=0` |
| `make fuzz-regression` | tidak bisa dikompilasi | `NO-CORPUS`, `EXIT code=2` |

Ketiganya menjalankan Verdikt yang benar untuk pertama kalinya.

### BUG-030 — satu set artefak dipakai semua mode fuzzing

**Status:** sudah di-fix (commit ini)
**Keparahan:** sedang — laporan hasil bisa dibaca salah
**Test:** tidak ada test; ini konfigurasi build

`fuzz_build` menulis `build/fuzz/zenus-fuzz`, `zenus-fuzz.iso` dan
`fuzz.log` — nama yang sama untuk smoke, coverage **dan** regression. Mode yang
dibangun terakhir menang.

Jadi `make fuzz-regression` menimpa ISO coverage, dan `make fuzz-coverage`
berikutnya membaca log yang isinya campaign berbeda. Saya tersesat dua kali
karena ini dan sempat hampir melaporkan `EXIT code=3` yang salah sebagai bug.

**Fix:** `FUZZ_KERNEL`/`FUZZ_ISO`/`FUZZ_LOG` jadi fungsi dari nama mode
(`zenus-fuzz-coverage.iso`, `fuzz-coverage.log`, dst), dan setiap grep memakai
log yang cocok dengan mode-nya.

**Catatan:** `EXIT code=3` yang saya laporkan untuk regression di tick
sebelumnya **salah baca** — exit code-nya memang sudah benar (`2`). Yang
benar-benar ada bugs-nya artefak bersama ini.

---

### BUG-031 — in-kernel test menaruh 270 KiB frame di boot stack

**Status:** diperbaiki dalam dua langkah — lihat "BUG-031 (bagian 2)" di bawah,
yang merupakan penjelasan yang benar dan final
**Keparahan:** tinggi — satu frame 270 KiB di stack yang paling sempit di kernel

> **Koreksi.** Entri ini pernah ditulis sebagai "sudah di-fix, tapi bukan
> penyebabnya". Itu **salah**. Arahnya benar, mechanismenya salah: `Box::new`
> tidak mengurangi frame stack sama sedikit pun. Dan BUG-032 di sebelahnya
> **tidak pernah diuji tanpa bug yang sebenarnya**, jadi klaim "ditolak"
> atasnya juga tidak sah. Kedua koreksi itu ditulis di bagian 2.

---

### BUG-031 (bagian 2) — `Box::new` tidak menolong; ini yang akhirnya berhasil

**Status:** sudah di-fix (commit ini) — dan inilah penyebab sebenarnya `make test`
**Keparahan:** tinggi — satu frame 270 KiB di boot stack, satu-satunya stack yang dipakai suite in-kernel
**Test:** tidak ada test; ini bentuk, bukan logika. Ukuran cache sudah dipatok
`test_cache_size_constant`

**Di mana:** `crates/zenus-fs/src/block_cache.rs` — modul `tests`
(`#[cfg(feature = "testing")]`)

`BlockCache` adalah 512 entri × 512 byte ≈ **270 KiB**. Kelima test
block-cache membuatnya sebagai `let`, jadi masing-masing mendorong frame
quarter-megabyte ke boot stack.

**Bagian pertama (sudah di-fix di commit sebelumnya)** memindahkannya ke heap
lewat `Box::new(BlockCache::new())`. Itu **tidak menolong**, dan saya sempat
menyimpulkan teorinya salah karena gejalanya tetap. Itu keliru.

**Kenapa `Box::new` tidak menolong:** `BlockCache::new()` mengembalikan nilai
(bukan `Box`), jadi compiler membangun **sementara 270 KiB di stack** lebih
dulu, lalu menyalinnya ke dalam box. Alokasi heap-nya benar; frame stack-nya
tetap ada.

Yang membuktikannya adalah breakpoint, bukan penalaran:

```
Breakpoint 1, zenus::test_runner::run_tests      test_runner.rs:124   ← tercapai
Breakpoint 2, zenus_fs::block_cache::BlockCache::new   block_cache.rs:37 ← masuk
Breakpoint 3, zenus_arch::...::page_fault_handler      idt.rs:395        ← fault
```

Fault terjadi **tepat setelah** `BlockCache::new` — persis di frame sementara
itu. Watchpoint di alamat fault (`0xffffffff8054a9e8`) tidak pernah fire,
dan `cr2` **berbeda antar-build** (`0xffffffff8054a9e8` vs
`0xffffffff80575488`), yang mengindikasikan dengan benar bahwa alamat itu bukan
alamat statis tetap melainkan pointer liar yang reread dari memori yang sudah
rusak.

**Fix:** karena `BlockCache::new()` adalah `const fn`, satu `static mut
TEST_CACHE: BlockCache = BlockCache::new();` menaruhnya di `.bss` — nol byte di
stack, nol alokasi heap. `test_cache()` mengembalikan `&'static mut` ke
salah satunya lewat `addr_of_mut!`.

Satu cache bersama cukup: tidak ada test yang menyisipkan entri, jadi tidak
satu pun bisa mengganggu state yang lain.

**Hasil:** `make test` → **`=== Results: 25 passed, 0 failed, 25 total ===`**.
Empat lapisan verifikasi sekarang hijau untuk pertama kalinya.

### Koreksi atas catatan BUG-031 dan BUG-032 sebelumnya

Saya menulis di `DEVLOG.md` bahwa BUG-031 dan BUG-032 "ditolak". Itu **salah
untuk BUG-031** dan **tidak terbukti untuk BUG-032**:

- **BUG-031 arahnya benar, mechanismenya salah.** Memindahkan ke heap tidak
  cukup karena `Box::new(f())` tetap membuat sementara di stack. Gejala tetap
  muncul, jadi saya menyimpulkan harusnya "ditolak". Pelajaran: uji
  mekanismenya, bukan hanya gejalanya — `Box::new` kelihatan seperti alokasi,
  padahal yang dialokasikan adalah yang sudah selesai di stack.
- **BUG-032 tidak teruji bersih.** Waktu saya mengujinya, BUG-031 versi heap
  masih ada. Jadi BUG-032 tidak pernah diuji tanpa bug yang sebenarnya. Itu
  masih hardening yang benar (mekanismenya nyata: reservasi setelah
  `global_init` memang tidak melindungi apa pun karena free stack sudah
  terisi), tapi apakah ia memperbaiki sesuatu **tidak diketahui**. Saya tidak
  akan mengklaim apa pun soal itu.

---

### `make test` — **SELESAI**

Rinciannya ada di "BUG-031 (bagian 2)" di atas. Hasilnya:

```
[TEST] bc/new_cache_empty... OK          [TEST] ext2/magic_constant... OK
… 25 baris …
=== Results: 25 passed, 0 failed, 25 total ===
```

### Yang sudah diketahui

| | |
|---|---|
| `run_tests` tercapai? | Ya — `run_tests` → `BlockCache::new` → fault, urutan itu yang membuktikan mekanismenya |
| Alamat fault | Berbeda antar-build (`0xffffffff8054a9e8`, `0xffffffff80575488`) — jadi pointer liar, bukan alamat statis tetap |
| Watchpoint di alamat itu | Tidak pernah fire, konsisten: yang written bukan address itu, tapi nilai pointer di suatu static |
| Gejalanya | Nol baris `[TEST]`, nol baris `PAGE FAULT`, stack boot tidak terbaca sehingga handler tidak bisa melaporkan |

### BUG-032 — image kernel tidak pernah di-reserve dari frame allocator

**Status:** sudah di-fix (commit ini) — hardening yang benar. **Apakah ia
memperbaiki sesuatu tidak diketahui**: waktu diuji, BUG-031 versi heap masih
ada, jadi pengujiannya tidak bersih
**Keparahan:** tinggi — frame yang ditulis di atas static yang masih dipakai
**Test:** tidak ada test; ini urutan pemanggilan, dan `global_init` tidak
host-safe (terpakai `rsp`)

**Di mana:** `crates/zenus-mem/src/frame_allocator.rs` — `global_init`,
`reserve_boot_stack`; `apps/src/linker.ld`; `apps/src/lib.rs`

Tidak ada apa pun di tree yang mengembalikan image kernel dari frame
allocator. `linker.ld` tidak punya simbol batas sama sekali, jadi tidak ada
yang bisa mengembalikannya. Limine memang mengetahui di mana ia memuat kernel dan
meninggalkan halaman itu di luar peta *usable* — tapi itu bookkeeping bootloader,
dan `.bss` **tidak ada di berkas yang dibaca Limine** (di-nol-kan saat load),
sehingga setiap static di `.bss` bergantung sepenuhnya pada Limine mereservasi
rentang yang benar.

Dua cacat yang lebih halus, keduanya soal **urutan**:

1. `reserve_boot_stack()` dipanggil **setelah** `global_init`. Reservasi hanya
   mengecilkan daftar region; free stack sudah diisi dari region itu di dalam
   `global_init`. Frame yang sudah masuk free stack tetap keluar nanti —
   reservasi belakangan melindungi **tidak apa-apa**.
2. Pemanggilan reservasi gambar sendiri harus berada di dalam `global_init`,
   sebelum free stack diisi, bukan sebagai `reserve_region` sesudahnya.

**Fix:**
- `linker.ld` menandai `__kernel_start` dan `__kernel_end` (mencakup `.bss`,
  yang di-align ke 4 KiB)
- `global_init(memory_map, hhdm_offset)` mencadangkan gambar kernel **dan**
  boot stack sebelum free stack diisi
- Both `entry()` call sites memperbarui argumennya

**Bug yang sama berlaku untuk boot stack**, dan sekarang ikut diperbaiki di
tempat yang benar.

### Teori yang diuji dan ditolak untuk `make test`

Dua-duanya saya terapkan, bangun ulang, dan jalankan. Keduanya **tidak**
menyembuhkan gejala. Dicatat supaya tidak diulang.

| Teori | Status | Hasil |
|---|---|---|
| BUG-031: frame 270 KiB di boot stack | **benar**, tapi versi `Box::new`-nya tidak | Versi heap tetap gagal; versi `.bss` berhasil |
| BUG-032: image kernel tidak di-reserve | **tidak teruji bersih** | Diuji saat BUG-031 masih ada. Mekanismenya nyata, tapi sebelum/sesudah belum terisolasi |

Yang **tetap** terverifikasi:

- `run_tests` tercapai (breakpoint di `test_runner.rs:124`, dari `apps/src/lib.rs:780`)
- fault deterministik: `cr2 = 0xffffffff8054a9e8`
- `cr2` itu `console::error::ERR_BUF + 0x3e10`, yaitu **~1 KiB di luar**
  `ERR_BUF`, di halaman yang tidak ter-map, dan **tidak ada simbol** di sana
- `ErrorBuf::push` sudah bounded di semua cabangnya (`min(15)`, `min(23)`,
  `min(127)`, `min(47)`), jadi ini **bukan** overflow `ERR_BUF` — ini pointer
  liar yang nilainya diambil dari static yang sudah rusak
- `CR3` adalah CR3 kernel; stack boot tidak terbaca saat fault
- nol baris `[TEST]`, nol baris `PAGE FAULT` di log

### Yang belum terverifikasi

BUG-032 belum diuji dalam bentuk yang bersih (tanpa BUG-031). Untuk
memastikannya: kembalikan keempat test ke `Box::new(BlockCache::new())` dalam
satu commit terpisah, jalankan `make test`, bandingkan — supaya satu
perubahan saja yang diukur.

### Catatan metodologi

Tiga dari empat kesimpulan yang salah pada tick-tick terakhir berasal dari satu
hal yang sama: menguji **gejala** alih-alih **mekanismenya**.

1. `-smp 4` membuat gdb melaporkan stop di thread AP yang duduk di state
   reset-nya, dengan `CR2` yang menyesatkan.
2. Breakpoint beberapa baris di dalam page-fault handler sudah melihat
   `CR2` milik fault berikutnya.
3. `Box::new(f())` terlihat seperti "dipindahkan ke heap", padahal `f()`
   sudah selesai membangun 270 KiB di stack lebih dulu. Gejalanya tetap,
   jadi teorinya saya nyatakan salah — padahal mekanismenya yang salah, bukan
   teorinya.

### BUG-032 — image kernel tidak pernah di-reserve dari frame allocator

**Status:** sudah di-fix (commit ini) — hardening yang benar. **Apakah ia
memperbaiki sesuatu tidak diketahui**: waktu diuji, BUG-031 versi heap masih
ada, jadi pengujiannya tidak bersih
**Keparahan:** tinggi — frame yang ditulis di atas static yang masih dipakai
**Test:** tidak ada test; ini urutan pemanggilan, dan `global_init` tidak
host-safe (terpakai `rsp`)

**Di mana:** `crates/zenus-mem/src/frame_allocator.rs` — `global_init`,
`reserve_boot_stack`; `apps/src/linker.ld`; `apps/src/lib.rs`

Tidak ada apa pun di tree yang mengembalikan image kernel dari frame
allocator. `linker.ld` tidak punya simbol batas sama sekali, jadi tidak ada
yang bisa mengembalikannya. Limine memang mengetahui di mana ia memuat kernel dan
meninggalkan halaman itu di luar peta *usable* — tapi itu bookkeeping bootloader,
dan `.bss` **tidak ada di berkas yang dibaca Limine** (di-nol-kan saat load),
sehingga setiap static di `.bss` bergantung sepenuhnya pada Limine mereservasi
rentang yang benar.

Dua cacat yang lebih halus, keduanya soal **urutan**:

1. `reserve_boot_stack()` dipanggil **setelah** `global_init`. Reservasi hanya
   mengecilkan daftar region; free stack sudah diisi dari region itu di dalam
   `global_init`. Frame yang sudah masuk free stack tetap keluar nanti —
   reservasi belakangan melindungi **tidak apa-apa**.
2. Pemanggilan reservasi gambar sendiri harus berada di dalam `global_init`,
   sebelum free stack diisi, bukan sebagai `reserve_region` sesudahnya.

**Fix:**
- `linker.ld` menandai `__kernel_start` dan `__kernel_end` (mencakup `.bss`,
  yang di-align ke 4 KiB)
- `global_init(memory_map, hhdm_offset)` mencadangkan gambar kernel **dan**
  boot stack sebelum free stack diisi
- Both `entry()` call sites memperbarui argumennya

**Bug yang sama berlaku untuk boot stack**, dan sekarang ikut diperbaiki di
tempat yang benar.

### Teori yang diuji dan ditolak untuk `make test`

Dua-duanya saya terapkan, bangun ulang, dan jalankan. Keduanya **tidak**
menyembuhkan gejala. Dicatat supaya tidak diulang.

| Teori | Status | Hasil |
|---|---|---|
| BUG-031: frame 270 KiB di boot stack | **benar**, tapi versi `Box::new`-nya tidak | Versi heap tetap gagal; versi `.bss` berhasil |
| BUG-032: image kernel tidak di-reserve | **tidak teruji bersih** | Diuji saat BUG-031 masih ada. Mekanismenya nyata, tapi sebelum/sesudah belum terisolasi |

Yang **tetap** terverifikasi:

- `run_tests` tercapai (breakpoint di `test_runner.rs:124`, dari `apps/src/lib.rs:780`)
- fault deterministik: `cr2 = 0xffffffff8054a9e8`
- `cr2` itu `console::error::ERR_BUF + 0x3e10`, yaitu **~1 KiB di luar**
  `ERR_BUF`, di halaman yang tidak ter-map, dan **tidak ada simbol** di sana
- `ErrorBuf::push` sudah bounded di semua cabangnya (`min(15)`, `min(23)`,
  `min(127)`, `min(47)`), jadi ini **bukan** overflow `ERR_BUF` — ini pointer
  liar yang nilainya diambil dari static yang sudah rusak
- `CR3` adalah CR3 kernel; stack boot tidak terbaca saat fault
- nol baris `[TEST]`, nol baris `PAGE FAULT` di log

### Yang harus dilakukan berikutnya

`cr2` diambil dari **static yang nilainya sudah rusak** — itu yang membuat saya menduga frame menimpa `.bss`. Tapi BUG-032 menunjukkan mekanisme yang paling
wajar untuk itu tidak cukup untuk menjelaskan gejala. Yang belum dicoba:

1. Hardware **watchpoint** di `0xffffffff8054a9e8` lewat gdb — gdb akan
   memberi tahu instruksi mana yang menulis ke sana.Alamat itu tidak
   ter-map, jadi `awatch` mungkin harus dipasang lewat alamat fisik, atau
   lewat HHDM (`hhdm + 0x8054a9e8`) yang ter-map.
2. Breakpoint di `alloc_frame` dengan kondisi — cari frame pertama yang
   jatuh di `[__kernel_start, __kernel_end)` atau di boot stack. Jika
   tidak pernah ada, BUG-032 menutup jalur itu sepenuhnya dan jawabannya
   ada di tempat lain.
3. Watchpoint di beberapa static `.bss` yang nilainya dipakai sebagai pointer
   (`OUTPUT_BUF`, `ERR_BUF`, `TCP_STATE`, `TASKS`) untuk menangkap saat
   nilainya berubah menjadi alamat liar.

Catatan metodologi yang sudah terngi untuk semua ini: **QEMU harus `-smp 1`**
dan **`-S`**, dan breakpoint harus di **entry** page-fault handler. Dengan
`-smp 4` gdb melaporkan stop di thread AP; dengan breakpoint beberapa baris
dalam, `CR2` yang dibaca sudah milik fault berikutnya.

---

## Lapisan verifikasi: apa yang benar-benar jalan

| Lapisan | Status | Catatan |
|---|---|---|
| `make test-host` | **hijau**, 199 test | Sepanjang sesi ini |
| `cargo build` (default / testing / fuzz-smoke / fuzz-coverage / fuzz-regression) | **hijau** | Semua kombinasi feature |
| `make fuzz-smoke` | **hijau** | 2000 kasus, `crashes=0`, `EXIT code=0` |
| `make fuzz-coverage` | **hijau** | **50 000 kasus**, `crashes=0`, `EXIT code=0`, `new_paths=978` |
| `make fuzz-regression` | **hijau** | `NO-CORPUS`, `EXIT code=2` — benar, dan exit code-nya memang sudah benar sejak awal |
| `make test` | **hijau** | **25 dari 25 lulus** di QEMU. Lihat BUG-031 bagian 2 |

### Yang belum selesai, dan kenapa saya tidak menebaknya

Dua hal di atas memerlukan diagnosis yang benar, bukan tebakan. Menebak akan
berarti menambahkan kode yang *terlihat* seperti perbaikan tanpa bukti — persis
yang `AGENTS.md` larang ("kalau tidak bisa dibuktikan gagal dulu, itu bukan
regression test — catatan saja").

1. **Fault di dalam `make fuzz-smoke`.** Belum ada diagnostics yang cukup. Yang
   diketahui: `CAUSE=instruction-fetch`, `ADDR == RIP`, `RSP == IDLE_RSP - 8`.
   Hipotesis yang harus diuji, berurutan: (a) kedalaman frame
   `run_campaign` vs ukuran stack task, (b) `fuzz_guard` checkpoint, (c)
   `idle_until` yang menimpa RSP tanpa menyimpannya.
2. **Page fault berulang di `make test`.** Handler-nya sendiri fault membaca
   stack, jadi dump tidak pernahCetak. Yang diketahui: test pertama
   (`test_new_cache_empty`) selesai, jadi bukan boot lagi.
3. **SMAP/SMEP.** Tidak berubah; masih item #1 di `ROADMAP.md`.

Kalau ada yang mau diambil berikutnya, urutannya:

Ketiga target fuzz sekarang hijau, jadi tidak ada lagi item yang perlu
dikejar dari sisi itu. Yang tersisa:

1. **`make test`** — masih page fault setelah test pertama, dan handler
   page-fault-nya sendiri ikut fault saat membaca stack sehingga tidak ada
   jejak. Butuh QEMU + gdb; mekanismenya sudah diketahui (attach gdb
   menghentikan CPU saat connect, monitor socket untuk menghentikan VM yang
   sedang berjalan) tapi belum dijalankan terhadap build `testing`.
2. **`make test`** — butuh QEMU + gdb, dan jejak yang sudah ada tidak
   cukup untuk menemukan penyebabnya sendiri.
3. **SMAP/SMEP** — selesai, lihat BUG-033. Item #1 di `ROADMAP.md` sudah
   tertutup; tidak ada lagi yang perlu diambil dari sisi itu.

---

## BUG-033 — SMAP/SMEP: the "PML4 U/S interaction" was three unrelated bugs

`ROADMAP.md` item 1, the top gap in `SECURITY.md`, and the top of the
"Kandidat berikutnya" list in this file for several sessions. The note said the
root cause was "the PML4 U/S interaction between `ensure_kernel_pages_supervisor`,
`create_address_space` and `map_user_page_raw`", and refused to guess further.

**It was not the page tables.** Nothing in `create_address_space` or
`map_user_page_raw` was wrong. There were three independent defects, and the
"does not reliably write the expected values" wording in the original comment is
what pointed at the first one.

### 1. `stac`/`clac` were `nomem`

```rust
core::arch::asm!("stac", options(nostack, nomem));
```

`nomem` tells LLVM the asm neither reads nor writes memory. Every call site does

```rust
stac();
let b = core::ptr::read_volatile(addr as *const u8);
clac();
```

A `read_volatile` cannot be *deleted* or moved relative to other volatile
operations, but `nomem` asm is not a volatile operation, so the load was free to
be scheduled after the `clac`. "Does not reliably write the expected values" is
exactly what a reordering bug looks like from the outside.

The `mov cr4` guard the pair had was wrong twice over: it put a second
memory-opaque asm inside the window, and per Intel SDM Vol 2 `STAC` sets AC
whether or not SMAP is enabled, and AC has no effect while SMAP is clear — so
the guard was testing something that could not matter.

Fixed in `crates/zenus-arch/src/cpu.rs`: `options(nostack, preserves_flags)`, no
CR4 read, with the reason written down at the function.

### 2. `sys_write` never called `stac` at all

`crates/zenus-syscall/src/syscall.rs`, the fd 1/2 fast path:

```rust
// stdout/stderr (fd 1, 2): tulis langsung dari user buffer byte-by-byte
// tanpa heap allocation. SMAP disabled, jadi kernel bisa akses user memory
// langsung via raw pointer.
let byte = unsafe { core::ptr::read_volatile((buf + i as u64) as *const u8) };
```

The comment is the bug report. This is a bug with SMAP off too: `validate_user_range`
bounds the address but does not translate it, so `write(1, 0x400000, 1)` — a
*present* page — took a ring-0 page fault and killed the machine.

This is the fault the boot log actually showed, once the handler stopped eating
its own `#PF`:

```
TYPE: supervisor-read-protection [SMAP: supervisor touched a user page without stac]
ADDR=0x000000000040002A RIP=0xFFFFFFFF8008219C CS=0x01 CAUSE=read CODE=0x1
```

`0x40002a` is 42 bytes into the user's text segment — the program's own `buf`.
Replaced with a fixed-size chunked stream through the direct map
(`copy_user_chunk_hhdm`), which needs no `stac` because the HHDM is
supervisor-only, and turns an unmapped buffer into `EFAULT`.

### 3. The page-fault handler faulted itself

`try_read_u8`/`try_read_u64` in `crates/zenus-arch/src/interrupts/idt.rs` walked
the interrupted frame's `RSP` — which for a ring-3 task is a *user* address —
with a bare `read_volatile`. With SMAP off that works; with SMAP on it is a
nested `#PF` inside the handler, the handler re-enters itself, and the machine
wedges printing one header and nothing else. That is the exact symptom in the
previous entry of this log ("the handler itself faults reading the stack, so the
dump never finishes"), previously blamed on the test runner.

Now: walk the tables first, then read the *physical* address through the HHDM.
The direct map is supervisor, so no `stac` is needed and the read cannot fault
on the U/S bit at all. Every frame the allocator hands out comes out of a Limine
usable region, which is what the HHDM covers.

### Why it took so long to find

The fault-type table in the same handler indexed the error code as
present?/user?/write?. Bit 1 is W/R and bit 2 is U/S, so `0x1` — a supervisor
read of a **present** page, i.e. exactly what SMAP raises — printed as
"supervisor-write-nonpresent". Five of the eight encodings were mislabelled. A
SMAP violation was being reported as a write to a missing page, which sends you
looking for a mapping bug instead of a missing `stac`.

The table is fixed, and the handler now prints `[SMAP: supervisor touched a user
page without stac]` when the error code, the address and `CR4.SMAP` all agree.

### Found along the way, not part of the above

* `sys_execve` wrote the new image's argv into user addresses while the **old**
  CR3 was still loaded. `load_elf` randomises `stack_top`, so those addresses
  were essentially never mapped in the address space that was installed. It also
  carried a dead, off-by-one duplicate of its own pointer-array loop, marked
  `// Actually let me rewrite more carefully`.
* The shell's `run` did switch CR3, but left interrupts enabled across the
  window, so a timer tick could run the ISR on the half-built user address space.
* `signal::setup_signal_frame`/`restore_signal_frame` pushed and popped the
  frame through raw user pointers with no mapping check — from the timer
  handler, so a bad pointer is a wedge.
* `make test` did not pass `-cpu max`, and QEMU's default `qemu64` model has
  neither SMEP nor SMAP. The in-kernel suite was not testing them and could not
  have.

### Verification

* `make test-host` — 205 passed, 0 failed (was 199; +6 for the stack layout).
* `make test` — 25/25 in-kernel, **with SMEP+SMAP actually on**.
* `run /initrd/bin/hello` prints `Hello from userspace!` and exits 0 under
  `-cpu max` with both features enabled.

### What the layout helper is for

`crates/zenus-syscall/src/userstack.rs` holds `layout_user_stack`, which is pure
arithmetic over `stack_top` and the argument lengths, and
`write_initial_user_stack`, which is the one place allowed to run with a
foreign CR3 loaded. The two old implementations disagreed about the layout —
the shell packed strings down from `stack_top`, `execve` grew them up from
`stack_top - total` — so the same program saw argv at different addresses
depending on how it was started. Both now call the same helper, which is
host-tested; see `a_null_argument_still_costs_a_byte` and
`an_argv_that_does_not_fit_is_refused_not_wrapped` for the two arithmetic traps
that were in it.

---

## BUG-034 — heap free list is corrupted by `alloc_stack`, writer not yet identified

Pre-existing and **not** related to BUG-033. It reproduces with SMAP disabled
(verified by rebuilding with `enable_smep_smap` commented out), so it is not a
consequence of that work.

### Symptom

One user program runs and exits cleanly, then:

```
[ERROR][zenus_mem::allocator] Heap header corrupted at 0xffffffff80841400
  (magic=0x0 canary=0x0) ptr=0xffffffff80841420
```

repeated four times, and every subsequent `run` fails with:

```
[CRIT ][zenus_mem::allocator] Heap exhausted! free_head=0xffffffff80840230, size=65536
run: failed to create task
```

So: run one program, and the shell can never run another. `meminfo` at boot is
clean and reports 8061 KB free, and a boot with no `run` at all never reports
corruption — it needs one `create_user_task`.

### What is established

Probing the free list at successive points in `cmd_run` narrowed it a long way:

| after | free list |
|---|---|
| `vfs` read | clean |
| `create_address_space` | clean |
| `load_elf_raw` | clean |
| `write_initial_user_stack` | clean |
| **`alloc_stack`** (64 KiB, inside `create_user_task`)** | **broken** |

The corruption is a single 4 KiB-aligned region inside the heap whose free-block
*headers* have been zeroed (`magic=0`, `canary=0`) while surrounding headers stay
intact. The block at the head of the list has `size=0`, which no path in the
allocator can produce — `alloc_mut` only creates free blocks with
`size >= MIN_BLOCK` (32) and `dealloc_mut` only coalesces upward.

The write is not a plain zero-fill of the whole page: the list shows intact
blocks *interleaved* with dead ones at 0x30-byte strides, which is the stride of
a `BlockHeader` (32) plus a `u64`. That looks like a sequence of 8-byte stores
at header+8 offsets, not a `write_bytes`.

### What is *not* established

**Who writes.** The frame allocator cannot hand out a frame inside the kernel
image — `global_init` reserves `__kernel_start..__kernel_end` (`.text`, `.rodata`,
`.data` **and** `.bss`, which is where the heap lives) *inside* the function,
before the free stack is filled, precisely so a late `reserve_region` cannot
leave a stale frame queued. `ensure_initialized` writes the first header, and
`alloc_mut`'s split path writes each new header once. So the writer is something
else entirely, and it is reached from `alloc_stack`.

One methodological note, because it cost real time and would cost it again: the
first version of the probe printed unconditionally. Every message goes through
the formatter, which **allocates** — so the probe replaced a clean free list with
a corrupt one and the bisect pointed at `alloc_stack` when the real transition
was earlier. The check now reports only on failure, and the reasoning is recorded
at `debug_check_free_list` so the next attempt does not repeat it.

### What was fixed here anyway

`clone_user_address_space` zeroed each new page table with
`write_bytes(ptr, 0, 512 * 8)` against a `*mut u64`. `write_bytes` counts
**elements**, so that is 4096 elements = **32 KiB** per table instead of 4 KiB —
three tables per `fork`, so every fork zeroed 24 KiB past the end of a page table
it had just taken from the frame allocator. This is the same bug class as the
comment on the idle task's frame ("writing top-down from the stack base
underflowed into the heap block in front of the allocation"), one level up.

Not the cause of the symptom above — `run /initrd/bin/hello` does not fork, so
the corrupted bytes are not explained by it — but it is a real heap-clobbering
overrun on a path that userspace can reach, and it was found while looking.

`page_table_bytes()` now exists so the zero-fills are written in bytes against a
`*mut u8`, with `a_page_table_is_exactly_one_page` pinning the arithmetic.

### Also added

`FreeListAllocator::debug_check_free_list` walks the list on every `alloc` and
`dealloc` and reports the first thing wrong (bad magic, non-ascending chain, a
block outside the arena, a `size` below `MIN_BLOCK`). It reports nothing when
the list is fine. This is the check whose absence made this bug silent: `dealloc`
only refuses a pointer whose header has the wrong magic, and a *partially*
overwritten header that still says `MAGIC_FREE` sails straight through.

Verified: `make test-host` 206 passed, `make test` 25/25.

### A hardware watchpoint was built, and it does not work here

The obvious instrument for "who wrote this byte" is a data breakpoint, and one
was written: `zenus_mem::watchpoint`, DR0, 8-byte write watch, `#DB` routed
through the existing `debug_handler` to report the faulting RIP. It was armed
from inside `alloc_mut` on the header of the block about to be split — which is
the moment the address is known, and the reason every external GDB attempt
missed.

It never fired. 227 arms, zero hits. A `self_test()` was then added that arms on
a private stack slot, writes it, and checks the handler ran, and **that failed
too** — which is the useful part. It distinguishes "nothing wrote there" from
"the mechanism is broken", and it was broken. Six plausible DR7 encodings were
swept (enable bit at 16 per the SDM, at 8 as some emulators document, local vs
global, 4-byte vs 8-byte length); every one read back as written and none raised
`#DB`. QEMU 8.2.2 TCG does not implement DR0 data watchpoints.

The instrument was removed rather than left in dead, because a self-test that
prints FAILED on every boot is noise, not a tool. The finding that matters is
recorded here and in the `debug_check_free_list` doc comment: on this platform
the only way to catch a specific write is `-s` + GDB, and GDB cannot be used for
a heap address because the block is only picked inside `alloc_mut`.

Two bugs were found *in the instrument* on the way, both of which would have
produced confident wrong answers:

* The first version armed at the block address, which is `magic`. `magic` and
  `canary` survive this corruption — only `size` is clobbered — so it watched a
  field that is never written and reported nothing.
* The first DR7 used bit 7 for the enable flag. Bit 7 is `LEN1`, the length of
  DR1; the enable bits are 16-23. It read back `dr7=0x489`, which is the tell.

### What the raw dump settled

`report_list_fault` now dumps 24 qwords around the bad header on the first
fault. That is what showed the arena is full of **legitimately tiny** free
blocks: `COLBERF` (`MAGIC_FREE` backwards) interleaved with `dev` and `tmp`,
`COLBDESU` (`MAGIC_USED` backwards), sizes 2, 3, 3, 3, 4, 8. Those are
`devfs::readdir`'s per-entry `String::from(name)` allocations. The blocks are
fine; the walk just ends at a zeroed header.

It also caught a **false positive in the new check itself**. The check required
every free block to have `size >= MIN_BLOCK`, reasoning that `alloc_mut` only
creates remainders that large. That is wrong: only the *remainder* of a split has
to reach `MIN_BLOCK`, and the block handed out keeps the caller's size, so a
freed 3-byte `String` is a legal 3-byte free block. The check reported a
perfectly healthy list as broken hundreds of times per `run` and buried the real
fault. Removed, with the reasoning written down so it is not re-added.

### The one measurement that is actually new

`alloc_mut` records what it decided about each split into a static array
(`record_split_trace`: block, size, region_end, prev, aligned_data, requested)
and the fault report prints it. `alloc_mut` cannot print on the happy path —
the formatter allocates — so recording is free and printing is deferred.

The recorded split for the failing run:

```
block=0xFFFFFFFF80856A58 size=0x7DB278
region_end=0xFFFFFFFF81031CF0 prev=0xFFFFFFFF80844840
aligned_data=0xFFFFFFFF80856A80 req=0x10000
```

`region_end` is **past the end of the arena**. The arena is
`[0xFFFFFFFF80830F70, 0xFFFFFFFF81030F70)`, so that block claims 0xD80 bytes
more than exists. A block header with a valid magic, a valid canary and an
inflated `size` passes every check that only looks at the header, and then
`alloc_mut` computes a `region_end` outside the heap and will eventually hand out
a payload pointer past the end. That is a new invariant to check, and it is now
checked:

```rust
if cur + HEADER_SIZE + size > arena_hi {
    self.report_list_fault(where_, n, cur, "block extends past the arena");
}
```

This did not fire in the run above, because the faulting walk ended on a zeroed
header before reaching the inflated block. Both faults are present; the zeroed
header is found first.

### Where this leaves BUG-034

Two distinct problems in one symptom, both now characterised:

1. **A zeroed 32-byte header** terminating the free list, found first, so it is
   what every report shows. Its neighbours are intact.
2. **A block whose `size` is inflated past the arena end.** Found by the split
   trace, not by the list walk. How a `size` grows is the open question —
   `dealloc_mut`'s coalescing writes `(*p).size += HEADER_SIZE + block_size`
   and `(*n).size` into a neighbour, and a coalesce that runs against a header
   that is not really a free block would inflate exactly like this.

Next step, and it is a specific one: `dealloc_mut`'s coalescing is the only
place `size` is ever increased, so log every coalesce (prev, block, next, and
the three sizes before and after) into the same static trace and read back which
coalesce inflated the block. That is a bounded, purely local question, and it
does not need a write watchpoint.

---

## Kandidat berikutnya

Semua kandidat dari daftar `bug hunt` sudah dikerjakan. Yang tersisa bukan bug
yang tercatat, tapi yang **tidak** bisa diselesaikan di host:

| Item | Kenapa belum selesai |
|---|---|
| SMAP/SMEP | **SELESAI** — lihat BUG-033. Root cause-nya bukan PML4 U/S; ada tiga bug terpisah (`stac`/`clac` `nomem`, `sys_write` tanpa `stac`, page-fault handler yang fault dirinya sendiri). Sekarang aktif dan `run /initrd/bin/hello` lulus. |
| Heap free list korup setelah satu user task | **BUG-034, belum selesai, tapi oracle baru.** Dua masalah dalam satu gejala: (a) header 32-byte yang di-zero mengakhiri free list — ini yang selalu ditemukan pertama; (b) sebuah block yang `size`-nya **melewati ujung arena** (`region_end=0x81031CF0` vs arena berakhir `0x81030F70`), ditemukan lewat split-trace, bukan list walk. Watchpoint hardware sudah dicoba dan **tidak berfungsi di QEMU 8.2.2 TCG** (0 dari 6 encoding DR7 menghasilkan `#DB`), jadi jangan coba lagi. Langkah berikutnya sudah spesifik: `dealloc_mut` coalescing adalah satu-satunya tempat `size` pernah bertambah, jadi log setiap coalesce. Detail di BUG-034 |
| `mapped_at` satu alamat per segmen SHM | `shmat` kedua untuk segmen yang sama menimpa yang pertama, jadi pemetaan pertama tidak bisa di-detach. Perbaikannya butuh daftar attachment per segmen, bukan skalar — perubahan struktural, di luar-fix yang bisa diverifikasi |
| SHM end-to-end | Butuh page table sungguhan. Yang bisa diuji di host (bagian indeks) sudah ada test-nya |
| `io_scheduler::io_stats()` | Mengembalikan total + dua nol hardcoded (`ARCHITECTURE.md`) |
| lockdep | Graf dan pengecekan reverse-edge sudah ada, tapi tidak ada jalur produksi yang mendaftarkan kelas lock |
| CI lint | `cargo clippy -- -D warnings` sudah gagal di HEAD (288 warning), bukan karena perubahan di sini |