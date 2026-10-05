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

## Kandidat berikutnya

Semua kandidat dari daftar `bug hunt` sudah dikerjakan. Yang tersisa bukan bug
yang tercatat, tapi yang **tidak** bisa diselesaikan di host:

| Item | Kenapa belum selesai |
|---|---|
| SMAP/SMEP | Root cause-nya belum dipahami (interaksi U/S PML4 antara `ensure_kernel_pages_supervisor`, `create_address_space`, `map_user_page_raw`). Semua test userspace gagal saat diaktifkan. Butuh board QEMU + alasan yang benar, bukan tebakan. Ini item #1 di `ROADMAP.md` |
| `mapped_at` satu alamat per segmen SHM | `shmat` kedua untuk segmen yang sama menimpa yang pertama, jadi pemetaan pertama tidak bisa di-detach. Perbaikannya butuh daftar attachment per segmen, bukan skalar — perubahan struktural, di luar-fix yang bisa diverifikasi |
| SHM end-to-end | Butuh page table sungguhan. Yang bisa diuji di host (bagian indeks) sudah ada test-nya |
| `io_scheduler::io_stats()` | Mengembalikan total + dua nol hardcoded (`ARCHITECTURE.md`) |
| lockdep | Graf dan pengecekan reverse-edge sudah ada, tapi tidak ada jalur produksi yang mendaftarkan kelas lock |
| CI lint | `cargo clippy -- -D warnings` sudah gagal di HEAD (288 warning), bukan karena perubahan di sini |