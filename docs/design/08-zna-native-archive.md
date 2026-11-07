# 8. `.zna` (Zenus Native Archive)

## Tujuan

`.zna` adalah format arsip universal Zenus OS yang digunakan untuk:

* Menggabungkan banyak file menjadi satu.
* Mengompresi data.
* Membuat backup.
* Menyimpan aset aplikasi.
* Menyimpan proyek.
* Menyimpan resource game.
* Menyimpan tema dan ikon.
* Menjadi format pertukaran data antar aplikasi Zenus.

Contoh:

```text
assets.zna
backup.zna
project.zna
icons.zna
themes.zna
game-data.zna
```

---

# Filosofi Desain

`.zna` dirancang agar:

* Cepat dibuka.
* Mendukung kompresi modern.
* Mendukung enkripsi.
* Mendukung deduplikasi data.
* Mendukung streaming.
* Mendukung file sangat besar (>16 EB secara teoritis dengan indeks 128-bit).
* Mendukung multi-thread compression.
* Mendukung checksum per file.
* Mendukung digital signature.

---

# Struktur File

```text
+------------------------------------------------+
| Magic Number                                   |
+------------------------------------------------+
| Archive Header                                 |
+------------------------------------------------+
| Compression Information                        |
+------------------------------------------------+
| File Index Table                               |
+------------------------------------------------+
| Folder Tree                                    |
+------------------------------------------------+
| Metadata Table                                 |
+------------------------------------------------+
| File Data                                      |
+------------------------------------------------+
| Checksum Table                                 |
+------------------------------------------------+
| Digital Signature                              |
+------------------------------------------------+
| Footer                                         |
+------------------------------------------------+
```

---

# 1. Magic Number

Contoh:

```text
ZNAR
```

Hexadecimal:

```text
5A 4E 41 52
```

Kernel atau aplikasi arsip Zenus langsung mengenali file sebagai arsip `.zna`.

---

# 2. Archive Header

Berisi informasi utama.

| Field           | Fungsi                  |
| --------------- | ----------------------- |
| Archive Version | Versi format            |
| Compression     | Algoritma kompresi      |
| Encryption      | Jenis enkripsi          |
| Total Files     | Jumlah file             |
| Total Size      | Ukuran sebelum kompresi |
| Compressed Size | Ukuran setelah kompresi |
| Creation Date   | Waktu pembuatan         |

---

# 3. Compression Information

`.zna` dapat mendukung beberapa algoritma.

| Algoritma        | Karakteristik         |
| ---------------- | --------------------- |
| LZ4              | Sangat cepat          |
| Zstandard (Zstd) | Seimbang              |
| Brotli           | Bagus untuk data teks |
| XZ               | Kompresi maksimum     |
| None             | Tanpa kompresi        |

Setiap file bahkan dapat menggunakan algoritma yang berbeda jika diperlukan.

---

# 4. File Index Table

Berisi daftar seluruh file.

Contoh:

```text
Offset
Ukuran
Ukuran Terkompresi
Metode Kompresi
Checksum
Permission
```

Karena ada indeks, aplikasi dapat langsung membuka file tertentu tanpa membaca seluruh arsip.

---

# 5. Folder Tree

Menyimpan struktur direktori.

Contoh:

```text
assets/

icons/

images/

fonts/

audio/

video/
```

Struktur ini dipertahankan saat diekstrak.

---

# 6. Metadata Table

Setiap file dapat memiliki metadata.

Contoh:

```text
Nama File

Pemilik

Permission

Tanggal Dibuat

Tanggal Diubah

Tanggal Diakses

Label

Komentar
```

---

# 7. File Data

Berisi isi file yang sebenarnya.

Misalnya:

```text
logo.png

theme.json

background.jpg

music.ogg

font.ttf
```

---

# 8. Checksum Table

Setiap file memiliki checksum sendiri.

Misalnya:

```text
SHA-256

SHA-512

BLAKE3
```

Jika hanya satu file rusak, file lain tetap dapat digunakan.

---

# 9. Digital Signature

Opsional namun sangat berguna.

Berisi:

```text
Developer ID

Certificate

Hash

Signature
```

Zenus dapat memastikan arsip berasal dari sumber tepercaya.

---

# 10. Footer

Berisi informasi akhir arsip.

Contoh:

```text
Archive End

Footer Version

Global Checksum

Reserved Space
```

Footer memudahkan pembacaan arsip dari belakang, misalnya untuk mendapatkan indeks dengan cepat.

---

# Proses Membuat Arsip

Misalnya pengguna ingin mengarsipkan proyek.

```text
project/

main.cpp

readme.md

assets/

textures/

audio/
```

Menjadi:

```text
project.zna
```

Prosesnya:

```text
Pilih folder
      │
      ▼
Membuat indeks file
      │
      ▼
Mengompresi setiap file
      │
      ▼
Menghitung checksum
      │
      ▼
Menambahkan metadata
      │
      ▼
Menandatangani arsip (opsional)
      │
      ▼
Menyimpan project.zna
```

---

# Proses Membuka Arsip

```text
User membuka project.zna
        │
        ▼
Magic Number diperiksa
        │
        ▼
Header dibaca
        │
        ▼
Checksum diverifikasi
        │
        ▼
Digital signature diverifikasi (opsional)
        │
        ▼
Indeks dimuat
        │
        ▼
Pengguna dapat melihat isi arsip
```

Karena memiliki indeks, aplikasi tidak perlu mengekstrak seluruh arsip hanya untuk membaca satu file.

---

# Dukungan Fitur Modern

`.zna` dapat mendukung:

* **Solid compression** untuk kompresi maksimal pada kumpulan file serupa.
* **Random access** sehingga file tertentu dapat dibaca langsung.
* **Incremental archive** untuk hanya menyimpan perubahan.
* **Deduplikasi** agar data identik tidak disimpan berulang.
* **Streaming archive** sehingga arsip dapat dibaca saat masih diunduh.
* **Multi-volume archive** (`backup.part1.zna`, `backup.part2.zna`, dan seterusnya).
* **Password protection** dengan enkripsi AES-256 atau algoritma modern lainnya.

---

# Contoh Penggunaan di Zenus

| Nama Arsip      | Isi                 |
| --------------- | ------------------- |
| `assets.zna`    | Gambar, audio, font |
| `backup.zna`    | Cadangan sistem     |
| `theme.zna`     | Tema desktop        |
| `icons.zna`     | Paket ikon          |
| `game-data.zna` | Aset game           |
| `workspace.zna` | Proyek pengembangan |

---

# Keunggulan `.zna`

| Fitur                           | `.zna` |
| ------------------------------- | ------ |
| Multi-thread compression        | ✅      |
| Random access                   | ✅      |
| Solid compression               | ✅      |
| Streaming support               | ✅      |
| Deduplikasi                     | ✅      |
| Checksum per file               | ✅      |
| Digital signature               | ✅      |
| Enkripsi                        | ✅      |
| Arsip multi-volume              | ✅      |
| Metadata lengkap                | ✅      |
| Mendukung file sangat besar     | ✅      |
| Integrasi penuh dengan Zenus OS | ✅      |

## Integrasi dalam Ekosistem Zenus

```text
.zns    → Executable
.znl    → Shared Library
.znd    → Driver
.znm    → Kernel Module
.znpkg  → Paket Instalasi
.zni    → System Image
.znf    → Firmware
.zna    → Universal Archive
```

Dengan posisi ini, `.zna` menjadi format arsip serbaguna Zenus OS. Berbeda dari `.znpkg` yang berfokus pada instalasi perangkat lunak, `.zna` ditujukan sebagai format penyimpanan umum yang cepat, aman, dan fleksibel untuk segala jenis data di dalam ekosistem Zenus.


`.znc` adalah **Zenus Native Configuration**, yaitu format konfigurasi standar untuk seluruh komponen Zenus OS. Kalau `.zns` menentukan **apa yang dijalankan**, maka `.znc` menentukan **bagaimana aplikasi atau sistem tersebut berjalan**. Banyak sistem operasi memakai campuran `.ini`, `.conf`, `.cfg`, `.xml`, `.json`, `.yaml`, yang akhirnya membuat pengembang harus mengingat lima sintaks berbeda hanya untuk mengubah angka `timeout`. Zenus dapat menghindari kekacauan itu dengan satu format konfigurasi resmi.

