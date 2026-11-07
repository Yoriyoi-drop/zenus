# 4. `.znm` (Zenus Native Module)

## Tujuan

`.znm` adalah **Kernel Module** untuk Zenus OS, yaitu komponen yang dapat dimuat (load) atau dilepas (unload) tanpa perlu mengompilasi ulang atau me-reboot kernel.

Berbeda dengan `.znd`, modul kernel tidak selalu berhubungan dengan perangkat keras. Modul ini dapat menambahkan fitur baru ke kernel.

Contoh:

```text
filesystem.znm
scheduler.znm
security.znm
network.znm
container.znm
virtualization.znm
crypto.znm
memory.znm
```

---

# Fungsi Utama

Modul kernel memungkinkan Zenus menambah kemampuan secara dinamis.

Contoh:

```
Kernel
│
├── memory.znm
├── scheduler.znm
├── network.znm
├── filesystem.znm
├── crypto.znm
└── virtualization.znm
```

Kernel inti tetap kecil (microkernel atau hybrid), sedangkan fitur tambahan dimuat saat diperlukan.

---

# Tujuan Desain

`.znm` dirancang untuk:

* Modular.
* Dapat dimuat saat sistem berjalan.
* Aman.
* Mendukung multi-core.
* Mendukung hot loading.
* Mendukung hot unloading.
* Mendukung dependency antar modul.
* Mendukung versioning.
* Mendukung rollback.
* Mendukung digital signature.

---

# Struktur File

```text
+-------------------------------------------+
| Magic Number                              |
+-------------------------------------------+
| Module Header                             |
+-------------------------------------------+
| Dependency Table                          |
+-------------------------------------------+
| Export Symbol Table                       |
+-------------------------------------------+
| Import Symbol Table                       |
+-------------------------------------------+
| Initialization Table                      |
+-------------------------------------------+
| Cleanup Table                             |
+-------------------------------------------+
| Permission Table                          |
+-------------------------------------------+
| Relocation Table                          |
+-------------------------------------------+
| Exception Table                           |
+-------------------------------------------+
| Module Code (.text)                       |
+-------------------------------------------+
| Read Only Data (.rodata)                  |
+-------------------------------------------+
| Writable Data (.data)                     |
+-------------------------------------------+
| Thread Local Storage                      |
+-------------------------------------------+
| Metadata                                  |
+-------------------------------------------+
| Digital Signature                         |
+-------------------------------------------+
```

---

# 1. Magic Number

Misalnya:

```text
ZNKM
```

Hexadecimal:

```text
5A 4E 4B 4D
```

Loader kernel langsung mengetahui bahwa file tersebut adalah modul kernel.

---

# 2. Module Header

Berisi informasi dasar.

| Field          | Fungsi                       |
| -------------- | ---------------------------- |
| Module Name    | Nama modul                   |
| Version        | Versi                        |
| ABI Version    | Versi ABI kernel             |
| Architecture   | AEX32 / AEX64                |
| Minimum Kernel | Kernel minimum               |
| Build Type     | Debug / Release              |
| Flags          | Core, Optional, Experimental |

---

# 3. Dependency Table

Jika modul membutuhkan modul lain.

Contoh:

```text
filesystem.znm
```

membutuhkan

```text
memory.znm

security.znm
```

Kernel akan memuat modul yang dibutuhkan terlebih dahulu.

---

# 4. Export Symbol Table

Daftar fungsi yang disediakan modul.

Contoh:

```text
AllocatePage()

FreePage()

MapVirtualMemory()

CreateFilesystem()

EncryptBlock()
```

Modul lain dapat menggunakan fungsi ini.

---

# 5. Import Symbol Table

Daftar fungsi yang dipakai dari modul lain.

Misalnya:

```text
AllocatePage()

RegisterInterrupt()

CreateMutex()

HashSHA512()
```

Loader akan menghubungkan simbol-simbol tersebut saat modul dimuat.

---

# 6. Initialization Table

Fungsi yang dijalankan saat modul dimuat.

Contoh:

```text
ModuleInit()

RegisterSubsystem()

CreateObjects()

InitializeCache()
```

---

# 7. Cleanup Table

Fungsi yang dipanggil saat modul dilepas.

Contoh:

```text
ModuleExit()

FreeMemory()

CloseHandle()

UnregisterSubsystem()
```

Ini memastikan tidak ada kebocoran memori atau resource.

---

# 8. Permission Table

Modul menyatakan hak akses yang diperlukan.

Contoh:

```text
Memory Manager

Scheduler

PCI Access

Filesystem

Networking

Interrupt Controller
```

Kernel dapat menolak modul yang meminta izin di luar kebijakannya.

---

# 9. Relocation Table

Jika modul dimuat pada alamat memori yang berbeda, loader memperbaiki semua referensi alamat secara otomatis.

---

# 10. Exception Table

Berisi informasi untuk menangani kesalahan.

Contoh:

```text
Kernel Exception

Stack Unwind

Recovery Handler

Module Panic Callback
```

---

# 11. Module Code

Berisi instruksi mesin AEX yang mengimplementasikan fitur modul.

Contoh:

```text
Page Allocation

Thread Scheduler

Virtual Memory

Filesystem Cache

Packet Routing
```

---

# 12. Read Only Data

Berisi data yang tidak berubah.

Misalnya:

```text
String

Lookup Table

Compression Table

Hash Constant
```

---

# 13. Writable Data

Variabel internal modul.

Contoh:

```text
PageCounter

CurrentThread

CacheSize

NetworkQueue
```

---

# 14. Thread Local Storage

Data khusus untuk setiap thread kernel.

Misalnya:

```text
CPU Local Cache

Worker Context

Scheduler Buffer
```

---

# 15. Metadata

Contoh:

```text
Module:
Zenus Scheduler

Version:
4.0

Developer:
Zenus Foundation

License:
MIT

Category:
Kernel Scheduler

Minimum Kernel:
2.1

Architecture:
AEX64

Status:
Stable
```

---

# 16. Digital Signature

Berisi:

```text
Developer ID

Certificate

Hash

Signature
```

Kernel dapat menolak modul yang tidak sah atau telah dimodifikasi.

---

# Proses Pemuatan Modul

Sebagai contoh, saat sistem membutuhkan dukungan sistem berkas baru:

```text
Kernel berjalan
      │
      ▼
User memasang disk ZFS
      │
      ▼
Kernel mencari zfs.znm
      │
      ▼
Memeriksa dependency
      │
      ▼
Verifikasi tanda tangan digital
      │
      ▼
Memetakan modul ke memori
      │
      ▼
Menjalankan ModuleInit()
      │
      ▼
Mendaftarkan driver dan API baru
      │
      ▼
Filesystem siap digunakan
```

---

# Lokasi Modul

Contoh struktur direktori:

```text
/system/modules/

memory.znm
scheduler.znm
filesystem.znm
security.znm
network.znm
crypto.znm
virtualization.znm
container.znm
```

---

# Perbedaan `.znm` dan `.znd`

| Aspek                             | `.znm`                          | `.znd`                        |
| --------------------------------- | ------------------------------- | ----------------------------- |
| Fungsi                            | Menambah kemampuan kernel       | Mengendalikan perangkat keras |
| Berhubungan dengan hardware       | Tidak selalu                    | Ya                            |
| Dapat menyediakan API kernel      | ✅                               | ❌                             |
| Dependency antar modul            | ✅                               | Terbatas                      |
| Dapat dimuat/dilepas saat runtime | ✅                               | ✅                             |
| Contoh                            | Scheduler, Filesystem, Security | GPU, NVMe, USB, Audio         |

---

# Keunggulan `.znm`

| Fitur                      | `.znm` |
| -------------------------- | ------ |
| Hot load & unload          | ✅      |
| Dependency management      | ✅      |
| Symbol export/import       | ✅      |
| Multi-core aware           | ✅      |
| Digital signature          | ✅      |
| Version & ABI checking     | ✅      |
| Permission table           | ✅      |
| ASLR & relokasi            | ✅      |
| Rollback jika gagal dimuat | ✅      |
| Dukungan debugger kernel   | ✅      |

Dengan desain ini, `.znm` menjadi fondasi arsitektur kernel Zenus yang modular. Fitur seperti sistem berkas baru, penjadwal CPU alternatif, subsistem keamanan, virtualisasi, atau dukungan container dapat ditambahkan atau diperbarui sebagai modul tanpa harus membangun ulang seluruh kernel, sehingga Zenus OS lebih mudah dikembangkan dan dipelihara.


`.znpkg` adalah format paket aplikasi resmi Zenus OS. Kalau `.zns` adalah aplikasi yang sudah siap dijalankan, maka `.znpkg` adalah wadah distribusinya. Anggap seperti kotak pengiriman yang berisi aplikasi, metadata, ikon, dependensi, dan instruksi instalasi. Jauh lebih rapi daripada tradisi lama yang menyuruh pengguna mengunduh 14 file dari forum lalu berharap semuanya berada di folder yang benar.

