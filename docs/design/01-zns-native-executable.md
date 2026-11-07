# 1. `.zns` (Zenus Native Executable)

## Tujuan

`.zns` adalah format biner yang dirancang khusus untuk menjalankan aplikasi di Zenus OS.

Contohnya:

```
terminal.zns
browser.zns
editor.zns
compiler.zns
game.zns
installer.zns
```

Kernel Zenus hanya perlu memahami satu format executable utama sehingga proses loading menjadi lebih sederhana dan konsisten.

---

# Tujuan Desain

Format `.zns` dirancang agar:

* Cepat di-load.
* Aman.
* Mendukung multi-core.
* Mendukung AetherX (AEX) ISA.
* Mendukung digital signature.
* Mendukung ASLR.
* Mendukung DEP/NX.
* Mendukung sandbox.
* Mudah di-debug.
* Siap untuk update di masa depan tanpa memecahkan kompatibilitas.

---

# Struktur File

```
+------------------------------------------------+
| Magic Number                                   |
+------------------------------------------------+
| Format Version                                 |
+------------------------------------------------+
| Executable Header                              |
+------------------------------------------------+
| Program Header Table                           |
+------------------------------------------------+
| Section Table                                  |
+------------------------------------------------+
| Import Table                                   |
+------------------------------------------------+
| Export Table                                   |
+------------------------------------------------+
| Resource Table                                 |
+------------------------------------------------+
| Symbol Table                                   |
+------------------------------------------------+
| Debug Information                              |
+------------------------------------------------+
| Relocation Table                               |
+------------------------------------------------+
| Exception Table                                |
+------------------------------------------------+
| Signature Block                                |
+------------------------------------------------+
| Compressed Sections (optional)                 |
+------------------------------------------------+
| Executable Code (.text)                        |
+------------------------------------------------+
| Read Only Data (.rodata)                       |
+------------------------------------------------+
| Writable Data (.data)                          |
+------------------------------------------------+
| Uninitialized Data (.bss)                      |
+------------------------------------------------+
| Thread Local Storage                           |
+------------------------------------------------+
| Metadata                                       |
+------------------------------------------------+
```

---

# 1. Magic Number

Byte pertama file.

Misalnya:

```
5A 4E 53 58
```

ASCII:

```
Z N S X
```

Kernel langsung mengetahui:

> "Ini executable Zenus."

Jika magic salah:

```
Error:
Invalid ZNS executable
```

---

# 2. Header

Header berisi informasi dasar.

Contoh:

| Field          | Fungsi               |
| -------------- | -------------------- |
| Format Version | versi format         |
| Architecture   | AEX32 / AEX64        |
| Entry Point    | alamat awal eksekusi |
| Program Offset | lokasi program       |
| Section Count  | jumlah section       |
| Image Size     | ukuran executable    |
| Flags          | atribut executable   |

---

# 3. Architecture

Contoh:

```
Architecture:
AEX64
```

atau

```
AEX32
```

Kernel mengetahui executable dibuat untuk ISA mana.

---

# 4. Entry Point

Alamat instruksi pertama.

Misalnya:

```
Entry Point

0x0000000010004000
```

Kernel akan melompat ke alamat ini setelah proses loading selesai.

---

# 5. Program Header

Berisi daftar segmen yang harus dimuat ke RAM.

Misalnya:

```
Segment 1

.text

Offset

0x1000

Virtual

0x400000

Size

65536
```

Kernel tidak perlu membaca seluruh file.

---

# 6. Section Table

Berisi semua section.

Misalnya:

```
.text
.rodata
.data
.bss
.tls
.resources
.debug
```

---

# 7. Code Section (.text)

Berisi instruksi mesin AetherX.

Misalnya:

```
MOV R1,R2
ADD R3,R4
CALL main
```

Section ini bersifat:

```
Read
Execute
```

Tidak boleh ditulis.

---

# 8. Read Only Data

Berisi:

```
String

Font

Constant

Lookup Table
```

Misalnya:

```
"Hello Zenus"
```

---

# 9. Data Section

Variabel global.

Misalnya:

```
int counter=10;
```

---

# 10. BSS

Variabel yang belum diinisialisasi.

Misalnya:

```
int buffer[100000];
```

Tidak disimpan dalam file.

Kernel hanya mengalokasikan memori saat program dijalankan.

---

# 11. Thread Local Storage

Data khusus setiap thread.

Misalnya:

```
errno

thread id

thread cache
```

---

# 12. Import Table

Daftar fungsi yang dibutuhkan.

Misalnya:

```
printf()

malloc()

CreateWindow()

SocketOpen()
```

Loader akan mencarinya di library `.znl`.

---

# 13. Export Table

Daftar fungsi yang bisa digunakan program lain.

Misalnya:

```
CreateRenderer()

OpenFile()

EncryptAES()
```

---

# 14. Resource Table

Semua aset aplikasi.

Contoh:

```
Icon

Cursor

Image

Audio

Theme

Language

Manifest

License
```

Sehingga:

```
browser.zns
```

cukup satu file tanpa aset terpisah jika diinginkan.

---

# 15. Relocation Table

Digunakan jika program dipindahkan karena ASLR.

Loader akan memperbaiki alamat memori secara otomatis.

---

# 16. Exception Table

Berisi informasi penanganan error.

Misalnya:

```
try

catch

stack unwind
```

Crash menjadi lebih mudah dianalisis.

---

# 17. Debug Information

Tidak digunakan saat release.

Digunakan oleh debugger Zenus.

Berisi:

```
Nama fungsi

Nomor baris

Nama file

Variabel lokal
```

---

# 18. Digital Signature

Bagian keamanan.

Berisi:

```
Developer ID

Certificate

Signature

Hash
```

Kernel dapat memverifikasi:

* aplikasi asli,
* belum dimodifikasi,
* berasal dari pengembang tepercaya.

---

# 19. Compression

Section tertentu dapat dikompresi.

Misalnya:

```
.text

LZ4
```

atau

```
Zstandard
```

Saat dijalankan, loader akan mendekompresi secara otomatis.

---

# 20. Metadata

Informasi tambahan, misalnya:

```
Nama Aplikasi:
Zenus Browser

Versi:
2.5.0

Developer:
Zenus Foundation

Build:
Release

Target OS:
Zenus 2.0+

Minimum Kernel:
2.0.1

Permission:
Filesystem
Network
Camera
GPU
Audio
```

Kernel dapat memeriksa izin sebelum aplikasi dijalankan, sehingga model keamanan lebih modern dibanding executable tradisional.

## Alur Eksekusi `.zns`

```
User menjalankan browser.zns
        │
        ▼
Kernel membaca Magic Number
        │
        ▼
Memeriksa Header dan versi
        │
        ▼
Memverifikasi tanda tangan digital
        │
        ▼
Memetakan segmen ke memori
        │
        ▼
Memuat library (.znl) yang dibutuhkan
        │
        ▼
Menerapkan ASLR dan relokasi
        │
        ▼
Menyiapkan Thread Local Storage
        │
        ▼
Melompat ke Entry Point
        │
        ▼
Aplikasi mulai berjalan
```

## Keunggulan dibanding format executable lama

| Fitur                          | `.zns` |
| ------------------------------ | ------ |
| Mendukung AEX32/AEX64          | ✅      |
| Digital signature bawaan       | ✅      |
| Metadata aplikasi terintegrasi | ✅      |
| Permission aplikasi            | ✅      |
| ASLR & DEP/NX                  | ✅      |
| Kompresi section opsional      | ✅      |
| Resource terintegrasi          | ✅      |
| Multi-thread aware             | ✅      |
| Dukungan debugger              | ✅      |
| Siap untuk sandbox             | ✅      |

Dengan desain seperti ini, `.zns` bukan sekadar "file yang bisa dijalankan", tetapi menjadi **format executable modern** yang menggabungkan kemampuan format seperti ELF, PE, dan Mach-O, sambil menambahkan fitur keamanan, metadata, serta manajemen izin yang dirancang sejak awal untuk Zenus OS.


Kalau `.znl` menjadi library resmi Zenus OS, maka perannya setara dengan **DLL** di Windows atau **shared object (.so)** di Linux. Bedanya, format ini bisa dirancang sejak awal agar lebih aman, lebih cepat dimuat, dan lebih mudah dikelola. Karena jika setiap aplikasi membawa salinan fungsi yang sama, RAM akan bekerja lembur hanya untuk mengulang pekerjaan yang identik. Komputer memang sabar, tetapi bukan berarti harus diperlakukan begitu.

