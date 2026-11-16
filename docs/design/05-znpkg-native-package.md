# 5. `.znpkg` (Zenus Native Package)

## Tujuan

`.znpkg` adalah format paket instalasi resmi Zenus OS.

Digunakan untuk:

* Menginstal aplikasi
* Memperbarui aplikasi
* Menghapus aplikasi
* Menginstal library
* Menginstal driver
* Menginstal module kernel
* Menginstal plugin
* Menginstal tema

Contoh:

```text
firefox.znpkg
vscode.znpkg
zenus-office.znpkg
docker.znpkg
python.znpkg
```

---

# Filosofi Desain

`.znpkg` dirancang agar:

* Aman
* Mudah diverifikasi
* Mendukung update delta
* Mendukung rollback
* Mendukung dependency
* Mendukung repository resmi
* Mendukung offline install
* Mendukung multi-architecture
* Mendukung sandbox
* Mendukung digital signature

---

# Hubungan dengan File Zenus

Misalnya:

```text
firefox.znpkg
│
├── browser.zns
├── libbrowser.znl
├── theme.zna
├── icon.png
├── manifest.znmf
└── install.script
```

Saat diinstal:

```text
browser.z
```

---

## Status implementasi

Format `.znpkg` sudah diimplementasi sebagai **stak parser stub** di `crates/zenus-formats` (bersama `.zns`, `.znl`, `.znd`, dan `.znm`):

- `ZNPKG_MAGIC = 0x4B504E5A` ("ZNPK", little-endian), `ZNPKG_VERSION = 1`
- `ZnpkgHeader`: versi, jumlah berkas, offset/ukuran manifest dan skrip instalasi
- `ZnpkgEntry`: nama berkas, offset, ukuran, dan tipe (executable/library/driver/module/archive/firmware/configuration/log)
- `ZnpkgFile`: daftar entri, blob manifest, dan skrip instalasi
- `ZnpkgFile::parse(data)`: parser murni (`no_std`, host-testable) yang memvalidasi magic/versi, lalu membaca tabel entri, manifest, dan skrip
- `ZnpkgFile::entry_data(name)`: ambil isi berkas tertutup dalam paket berdasarkan nama
- Host tests: `crates/zenus-formats/src/lib.rs` (`host_tests`) — 25 tes (6 `.zns` + 5 `.znl` + 5 `.znd` + 4 `.znm` + 5 `.znpkg`), semua lulus (`cargo test -p zenus-formats`)
- Bare-metal build: lolos `cargo build -p zenus-formats --target x86_64-unknown-none`

Belum diimplementasi (jadwal berikutnya): verifikasi tanda tangan digital paket, penanganan dependensi antar paket, ekstraksi ke filesystem, dan skrip instalasi yang dieksekusi.


`.zni` adalah **image sistem** Zenus OS. Jika `.zns` adalah satu aplikasi dan `.znpkg` adalah paket instalasi, maka `.zni` adalah gambaran lengkap sebuah sistem atau partisi yang siap digunakan. Format ini bisa dipakai untuk boot, recovery, installer, mesin virtual, bahkan deployment server. Karena memasang ulang sistem dari nol setiap kali ada masalah memang terdengar heroik, tetapi jauh lebih efisien jika cukup memuat sebuah image.

