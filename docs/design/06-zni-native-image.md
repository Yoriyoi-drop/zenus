# 6. `.zni` (Zenus Native Image)

## Tujuan

`.zni` adalah format image resmi Zenus OS untuk menyimpan salinan lengkap sistem atau komponen sistem.

Digunakan untuk:

* Boot image
* Recovery image
* Installer image
* System image
* Virtual machine image
* Container base image
* Firmware bundle
* Backup image

Contoh:

```text
boot.zni
recovery.zni
installer.zni
server-base.zni
desktop.zni
minimal.zni
```

---

# Filosofi Desain

`.zni` dirancang agar:

* Cepat di-boot
* Mudah diverifikasi
* Mendukung kompresi
* Mendukung enkripsi
* Mendukung snapshot
* Mendukung incremental update
* Mendukung secure boot
* Mendukung rollback
* Mendukung boot dari USB, SSD, atau jaringan

---

# Struktur File

```text
+---------------------------------------------+
| Magic Number                                |
+---------------------------------------------+
| Image Header                                |
+---------------------------------------------+
| Partition Table                             |
+---------------------------------------------+
| Boot Configuration                          |
+---------------------------------------------+
| Kernel Image                                |
+---------------------------------------------+
| Init System                                 |
+---------------------------------------------+
| Root Filesystem                             |
+---------------------------------------------+
| Recovery Environment                        |
+---------------------------------------------+
| Package Index                               |
+---------------------------------------------+
| Metadata                                    |
+---------------------------------------------+
| Digital Signature                           |
+---------------------------------------------+
| Checksum                                    |
+---------------------------------------------+
```

---

# 1. Magic Number

Misalnya:

```text
ZNIM
```

Hexadecimal:

```text
5A 4E 49 4D
```

Bootloader langsung mengetahui bahwa file tersebut adalah image Zenus.

---

# 2. Image Header

Berisi informasi utama.

| Field        | Fungsi                        |
| ------------ | ----------------------------- |
| Image Name   | Nama image                    |
| Version      | Versi image                   |
| Image Type   | Boot, Recovery, Installer, VM |
| Architecture | AEX32 / AEX64                 |
| Compression  | LZ4, Zstd, None               |
| Encryption   | Opsional                      |
| Image Size   | Ukuran image                  |

---

# 3. Partition Table

Menjelaskan isi image.

Contoh:

```text
EFI
Kernel
RootFS
Recovery
UserData
```

Jika image digunakan untuk instalasi, partisi akan dibuat sesuai tabel ini.

---

# 4. Boot Configuration

Berisi konfigurasi boot.

Contoh:

```text
Default Kernel

Boot Timeout

Kernel Parameter

Secure Boot

Safe Mode
```

---

# 5. Kernel Image

Berisi kernel Zenus.

Contoh:

```text
zenus.kernel
```

Saat boot, bootloader memuat bagian ini ke memori.

---

# 6. Init System

Program pertama yang dijalankan setelah kernel selesai inisialisasi.

Contoh:

```text
/init
```

Tugasnya:

* Memasang filesystem.
* Memulai layanan inti.
* Menjalankan service manager.
* Menyiapkan lingkungan pengguna.

---

# 7. Root Filesystem

Berisi isi sistem operasi.

Contoh:

```text
/system/
/usr/
/bin/
/lib/
/home/
/etc/
```

Bagian ini biasanya menjadi bagian terbesar dalam image.

---

# 8. Recovery Environment

Image dapat menyertakan lingkungan pemulihan.

Fitur yang dapat tersedia:

```text
Disk Repair

Package Repair

Kernel Repair

Boot Repair

Filesystem Check

Backup Restore
```

Jika sistem gagal boot, recovery dapat dijalankan tanpa media eksternal.

---

# 9. Package Index

Daftar paket yang sudah ada di image.

Contoh:

```text
terminal.zns
browser.zns
editor.zns
libgraphics.znl
python.znpkg
```

Installer dapat mengetahui isi image tanpa mengekstraknya terlebih dahulu.

---

# 10. Metadata

Contoh:

```text
Image:
Zenus Desktop

Version:
3.0.0

Developer:
Zenus Foundation

Architecture:
AEX64

Build:
Release

Minimum RAM:
4 GB

Recommended RAM:
8 GB
```

---

# 11. Digital Signature

Berisi:

```text
Developer ID

Certificate

Hash

Signature
```

Bootloader dapat memastikan image belum dimodifikasi.

---

# 12. Checksum

Misalnya menggunakan:

```text
SHA-256

SHA-512

BLAKE3
```

Digunakan untuk memverifikasi integritas image.

---

# Proses Boot dari `.zni`

Misalnya pengguna melakukan boot dari USB.

```text
UEFI
      │
      ▼
Zenus Bootloader
      │
      ▼
Membaca boot.zni
      │
      ▼
Memverifikasi tanda tangan digital
      │
      ▼
Memuat kernel
      │
      ▼
Memuat init system
      │
      ▼
Memasang RootFS
      │
      ▼
Menjalankan layanan sistem
      │
      ▼
Desktop Zenus muncul
```

---

# Jenis `.zni`

| Jenis                | Fungsi                            |
| -------------------- | --------------------------------- |
| Boot Image           | Boot sistem                       |
| Recovery Image       | Pemulihan sistem                  |
| Installer Image      | Instalasi Zenus                   |
| Live Image           | Menjalankan Zenus tanpa instalasi |
| VM Image             | Mesin virtual                     |
| Server Image         | Deployment server                 |
| Embedded Image       | Perangkat IoT                     |
| Container Base Image | Dasar untuk container             |

---

# Kompresi

`.zni` dapat menggunakan beberapa algoritma:

| Algoritma        | Kelebihan                            |
| ---------------- | ------------------------------------ |
| LZ4              | Sangat cepat saat boot               |
| Zstandard (Zstd) | Seimbang antara ukuran dan kecepatan |
| XZ               | Ukuran paling kecil, lebih lambat    |
| None             | Tanpa kompresi                       |

---

# Keamanan

Fitur keamanan yang dapat diterapkan:

* Secure Boot.
* Verifikasi tanda tangan digital.
* Enkripsi image (misalnya AES-256).
* Anti-tamper.
* Verifikasi checksum saat boot.
* Dukungan Trusted Platform Module (TPM).
* Measured Boot untuk memastikan rantai kepercayaan sejak proses boot.

---

# Keunggulan `.zni`

| Fitur                   | `.zni` |
| ----------------------- | ------ |
| Bootable                | ✅      |
| Recovery bawaan         | ✅      |
| Installer terintegrasi  | ✅      |
| Snapshot                | ✅      |
| Incremental update      | ✅      |
| Kompresi                | ✅      |
| Enkripsi                | ✅      |
| Digital signature       | ✅      |
| Multi-architecture      | ✅      |
| Secure Boot             | ✅      |
| Dukungan VM & container | ✅      |
| Rollback sistem         | ✅      |

Dengan rancangan ini, `.zni` menjadi format image serbaguna untuk Zenus OS. Satu format yang sama dapat dipakai mulai dari instalasi desktop, deployment server, mesin virtual, hingga perangkat embedded, sehingga seluruh ekosistem menggunakan mekanisme distribusi dan boot yang konsisten.


`.znf` adalah **Zenus Native Firmware**, yaitu format firmware resmi untuk Zenus OS. Jika `.znd` mengendalikan perangkat keras melalui driver, maka `.znf` adalah perangkat lunak tingkat rendah yang berjalan **di dalam perangkat keras itu sendiri**. Driver memberi perintah, firmware yang menerjemahkan perintah itu menjadi aksi pada perangkat. Tanpa firmware, banyak perangkat modern hanyalah sekumpulan transistor yang menunggu instruksi dengan sabar.

