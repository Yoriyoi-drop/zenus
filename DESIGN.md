Kalau `.zns` menjadi **format executable resmi Zenus OS**, maka jangan hanya dianggap sebagai "file yang bisa dijalankan". Ia sebaiknya menjadi fondasi seluruh ekosistem aplikasi Zenus, setara dengan **PE (.exe)** di Windows atau **ELF** di Linux. Manusia memang gemar membuat format baru, lalu menghabiskan bertahun-tahun memastikan format itu tidak meledak saat membaca satu byte yang salah.

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

# 2. `.znl` (Zenus Native Library)

## Tujuan

`.znl` adalah **Zenus Native Library**, yaitu kumpulan fungsi, kelas, API, atau layanan yang dapat digunakan oleh banyak aplikasi tanpa harus disalin ke setiap executable.

Contoh:

```text
libgraphics.znl
libnetwork.znl
libcrypto.znl
libaudio.znl
libfilesystem.znl
libgui.znl
```

---

# Fungsi Utama

Satu library dapat dipakai oleh banyak aplikasi.

Contoh:

```text
terminal.zns
browser.zns
editor.zns
music.zns
game.zns
```

semuanya menggunakan

```text
libgraphics.znl
```

Sehingga hanya ada satu salinan library di memori yang dibagikan (shared) ke semua aplikasi.

---

# Tujuan Desain

`.znl` dirancang untuk:

* Shared library.
* Dynamic loading.
* Hot update (opsional).
* Aman terhadap modifikasi.
* Mendukung multi-thread.
* Mendukung versioning.
* Mendukung digital signature.
* Mendukung lazy loading.
* Kompatibel dengan AEX32/AEX64.
* Mendukung plugin dan ekstensi.

---

# Struktur File

```text
+--------------------------------------------+
| Magic Number                               |
+--------------------------------------------+
| Library Header                             |
+--------------------------------------------+
| Export Table                               |
+--------------------------------------------+
| Import Table                               |
+--------------------------------------------+
| Symbol Table                               |
+--------------------------------------------+
| Relocation Table                           |
+--------------------------------------------+
| Exception Table                            |
+--------------------------------------------+
| Resource Table                             |
+--------------------------------------------+
| Metadata                                   |
+--------------------------------------------+
| Signature                                  |
+--------------------------------------------+
| Code (.text)                               |
+--------------------------------------------+
| Read Only Data (.rodata)                   |
+--------------------------------------------+
| Writable Data (.data)                      |
+--------------------------------------------+
| TLS                                        |
+--------------------------------------------+
```

---

# 1. Magic Number

Misalnya:

```text
ZNLB
```

Dalam hexadecimal:

```text
5A 4E 4C 42
```

Kernel atau loader langsung mengenali bahwa file ini adalah library Zenus.

---

# 2. Library Header

Berisi informasi utama.

Contoh:

| Field          | Fungsi                 |
| -------------- | ---------------------- |
| Format Version | Versi format `.znl`    |
| Library Name   | Nama library           |
| ABI Version    | Versi ABI              |
| Architecture   | AEX32 / AEX64          |
| Build Type     | Debug / Release        |
| Flags          | Shared, Plugin, System |

---

# 3. Export Table

Bagian paling penting.

Semua fungsi yang boleh dipanggil aplikasi lain dicatat di sini.

Contoh:

```text
CreateWindow()

DrawImage()

RenderFrame()

LoadTexture()

CreateButton()

CloseWindow()
```

Loader dapat menemukan alamat fungsi tanpa harus membaca seluruh file.

---

# 4. Import Table

Jika library membutuhkan library lain.

Misalnya:

```text
libgraphics.znl
```

menggunakan

```text
libgpu.znl

libmemory.znl
```

Maka loader akan memuat keduanya terlebih dahulu.

---

# 5. Symbol Table

Berisi simbol internal.

Misalnya:

```text
Renderer

WindowManager

FontEngine

TextureCache

GPUDriver
```

Dipakai debugger maupun linker.

---

# 6. Code Section

Berisi instruksi mesin AetherX.

Misalnya:

```text
DrawTriangle()

CopyMemory()

OpenSocket()

EncryptAES()
```

---

# 7. Read Only Data

Berisi:

```text
String

Konstanta

Shader

Template

Lookup Table
```

---

# 8. Data Section

Variabel global library.

Misalnya:

```text
CurrentWindow

GPUState

NetworkStatus
```

---

# 9. Thread Local Storage

Data khusus setiap thread.

Misalnya:

```text
LastError

TLS Cache

Worker Buffer
```

Thread A dan Thread B memiliki salinan datanya masing-masing sehingga tidak saling mengganggu.

---

# 10. Relocation Table

Digunakan jika alamat library berubah.

Misalnya karena:

* ASLR
* Loader
* Virtual Memory

Loader akan memperbaiki semua alamat secara otomatis.

---

# 11. Exception Table

Berisi informasi penanganan error.

Contoh:

```text
Stack Unwind

Catch Block

Runtime Exception
```

Debugger dapat menghasilkan stack trace yang lebih jelas.

---

# 12. Resource Table

Library juga dapat membawa aset.

Misalnya:

```text
Default Icon

Theme

Shader

Font

Language

Animation
```

---

# 13. Metadata

Contoh:

```text
Library Name:
Zenus Graphics

Version:
3.2.1

ABI:
5

Developer:
Zenus Foundation

License:
MIT

Minimum Kernel:
2.0

Compatible:
AEX64

Build:
Release
```

---

# 14. Digital Signature

Berisi:

```text
Certificate

Developer ID

Hash

Signature
```

Loader dapat memastikan library belum dimodifikasi.

---

# Cara Kerja Loader

Saat pengguna menjalankan:

```text
browser.zns
```

Header executable berisi:

```text
Import:

libgraphics.znl

libnetwork.znl

libcrypto.znl
```

Loader kemudian:

```text
1. Membaca daftar library
        │
        ▼
2. Mencari file .znl
        │
        ▼
3. Memverifikasi tanda tangan digital
        │
        ▼
4. Memuat library ke memori
        │
        ▼
5. Menyelesaikan alamat fungsi (symbol resolution)
        │
        ▼
6. Menghubungkan executable dengan library
        │
        ▼
7. Menjalankan aplikasi
```

---

# Shared Memory

Misalnya ada tiga aplikasi:

```text
browser.zns

editor.zns

terminal.zns
```

Ketiganya menggunakan:

```text
libgraphics.znl
```

Di RAM hanya ada **satu** salinan `libgraphics.znl`.

```text
RAM

Browser
        │
        ├────► libgraphics.znl
        │
Editor
        │
        ├────► libgraphics.znl
        │
Terminal
        │
        └────► libgraphics.znl
```

Ini menghemat memori dan mempercepat pemuatan aplikasi.

---

# Dukungan Plugin

Karena format `.znl` adalah library dinamis, ia juga dapat dipakai sebagai sistem plugin.

Contoh:

```text
plugins/

pdf_viewer.znl

markdown.znl

theme_dark.znl

git_extension.znl

python_runtime.znl
```

Aplikasi dapat memuat plugin hanya saat diperlukan, tanpa harus dibangun ulang.

---

# Keunggulan `.znl`

| Fitur                  | `.znl` |
| ---------------------- | ------ |
| Shared library         | ✅      |
| Dynamic loading        | ✅      |
| Lazy loading           | ✅      |
| Digital signature      | ✅      |
| Versioning & ABI       | ✅      |
| Dukungan plugin        | ✅      |
| Thread Local Storage   | ✅      |
| ASLR & relokasi        | ✅      |
| Resource terintegrasi  | ✅      |
| Kompatibel AEX32/AEX64 | ✅      |
| Hot update (opsional)  | ✅      |
| Dukungan debugger      | ✅      |

Dengan rancangan ini, `.znl` menjadi fondasi seluruh API Zenus OS. Semua komponen, mulai dari GUI, jaringan, audio, grafis, hingga layanan sistem dapat disediakan sebagai library modular yang dapat dipakai ulang, diperbarui secara independen, dan dibagikan ke banyak aplikasi tanpa pemborosan memori.


`.znd` adalah salah satu format paling penting dalam sebuah sistem operasi. Kalau `.zns` menjalankan aplikasi dan `.znl` menyediakan library, maka `.znd` adalah jembatan antara kernel dan perangkat keras. Tanpa driver, sistem operasi hanya melihat "benda misterius yang terhubung". Komputer memang cepat, tetapi tidak memiliki bakat telepati.

# 3. `.znd` (Zenus Native Driver)

## Tujuan

`.znd` adalah **Zenus Native Driver**, yaitu modul perangkat lunak yang memungkinkan kernel Zenus berkomunikasi dengan perangkat keras (hardware).

Contoh:

```text
nvme.znd
usb.znd
wifi.znd
bluetooth.znd
audio.znd
gpu.znd
ethernet.znd
camera.znd
touchscreen.znd
```

---

# Fungsi Utama

Driver bertugas menerjemahkan perintah kernel menjadi operasi yang dimengerti perangkat keras.

Contoh alur:

```text
Aplikasi
     │
     ▼
Kernel Zenus
     │
     ▼
gpu.znd
     │
     ▼
GPU
```

Tanpa `gpu.znd`, aplikasi tidak bisa menggambar ke layar meskipun GPU terpasang.

---

# Filosofi Desain

`.znd` dirancang untuk:

* Modular.
* Aman.
* Mudah diperbarui.
* Mendukung hot-plug (USB, Thunderbolt, dll.).
* Mendukung multi-core.
* Mendukung virtualisasi.
* Mendukung power management.
* Mendukung sandbox driver (opsional).
* Mendukung rollback jika pembaruan gagal.

---

# Struktur File

```text
+--------------------------------------------+
| Magic Number                               |
+--------------------------------------------+
| Driver Header                              |
+--------------------------------------------+
| Hardware ID Table                          |
+--------------------------------------------+
| Driver Capability Table                    |
+--------------------------------------------+
| Initialization Table                       |
+--------------------------------------------+
| Interrupt Handler Table                    |
+--------------------------------------------+
| Power Management Table                     |
+--------------------------------------------+
| DMA Configuration                          |
+--------------------------------------------+
| Resource Table                             |
+--------------------------------------------+
| Relocation Table                           |
+--------------------------------------------+
| Exception Table                            |
+--------------------------------------------+
| Signature                                  |
+--------------------------------------------+
| Driver Code (.text)                        |
+--------------------------------------------+
| Driver Data (.data)                        |
+--------------------------------------------+
| Metadata                                   |
+--------------------------------------------+
```

---

# 1. Magic Number

Contoh:

```text
ZNDR
```

Hexadecimal:

```text
5A 4E 44 52
```

Kernel langsung mengenali file sebagai driver.

---

# 2. Driver Header

Berisi informasi dasar.

| Field          | Fungsi                         |
| -------------- | ------------------------------ |
| Driver Name    | Nama driver                    |
| Version        | Versi driver                   |
| Architecture   | AEX32 / AEX64                  |
| Driver Type    | Storage, GPU, USB, Audio, dll. |
| Build          | Debug / Release                |
| Minimum Kernel | Versi kernel minimum           |
| Flags          | Boot, System, Hotplug, Secure  |

---

# 3. Hardware ID Table

Berisi daftar perangkat yang didukung.

Contoh:

```text
Vendor ID : 8086
Device ID : 1234

Vendor ID : 10DE
Device ID : 2684

Vendor ID : 1002
Device ID : 744C
```

Saat boot, kernel mencocokkan perangkat yang ditemukan dengan daftar ini.

---

# 4. Driver Capability Table

Menjelaskan kemampuan driver.

Contoh:

```text
DMA
MSI-X
PCI Express
Power Saving
Hot Plug
Suspend
Resume
Virtualization
```

Kernel mengetahui fitur apa saja yang tersedia.

---

# 5. Initialization Table

Fungsi yang dipanggil saat driver dimuat.

Contoh:

```text
DriverInit()

DeviceProbe()

AllocateDMA()

CreateDevice()

RegisterInterrupt()

StartDriver()
```

---

# 6. Interrupt Handler Table

Perangkat keras dapat mengirim interrupt.

Contoh:

```text
GPU Interrupt

USB Interrupt

NVMe Interrupt

Network Interrupt
```

Kernel akan memanggil handler yang sesuai saat interrupt terjadi.

---

# 7. DMA Configuration

Jika perangkat menggunakan Direct Memory Access (DMA), konfigurasi disimpan di sini.

Contoh:

```text
DMA Buffer

DMA Alignment

DMA Size

DMA Channel
```

Ini mempercepat transfer data tanpa membebani CPU.

---

# 8. Power Management Table

Mendukung penghematan daya.

State yang umum:

```text
ON

Idle

Sleep

Suspend

Hibernate

Off
```

Driver dapat mengurangi konsumsi daya saat perangkat tidak aktif.

---

# 9. Resource Table

Berisi sumber daya yang dibutuhkan driver.

Contoh:

```text
Firmware

Microcode

Shader

Calibration Data

Configuration
```

---

# 10. Driver Code

Berisi instruksi mesin AEX.

Contoh fungsi:

```text
ReadSector()

WriteSector()

SendPacket()

ReceivePacket()

RenderFrame()

PlayAudio()
```

---

# 11. Driver Data

Variabel internal driver.

Misalnya:

```text
CurrentDevice

CurrentPowerState

DMAAddress

InterruptCount
```

---

# 12. Relocation Table

Digunakan jika alamat memori berubah akibat ASLR atau tata letak memori kernel.

Loader akan memperbaiki referensi alamat secara otomatis.

---

# 13. Exception Table

Berisi mekanisme penanganan kesalahan.

Contoh:

```text
Device Timeout

DMA Error

PCI Error

Reset Device
```

Jika perangkat gagal merespons, driver dapat mencoba pemulihan tanpa menyebabkan kernel panik.

---

# 14. Digital Signature

Berisi:

```text
Developer ID

Certificate

Hash

Signature
```

Kernel hanya memuat driver yang valid jika Secure Boot Zenus diaktifkan.

---

# 15. Metadata

Contoh:

```text
Driver:
Zenus NVMe Driver

Version:
2.3.0

Developer:
Zenus Foundation

License:
MIT

Minimum Kernel:
2.1

Architecture:
AEX64

Device Class:
Storage

Supports:
PCIe Gen3
PCIe Gen4
PCIe Gen5
```

---

# Proses Pemuatan Driver

Misalnya komputer memiliki SSD NVMe.

```text
Boot Zenus
      │
      ▼
Kernel mendeteksi PCIe
      │
      ▼
Vendor ID = 144D
Device ID = A808
      │
      ▼
Mencari driver yang cocok
      │
      ▼
Memuat nvme.znd
      │
      ▼
Verifikasi tanda tangan digital
      │
      ▼
Menjalankan DriverInit()
      │
      ▼
Mendaftarkan perangkat ke kernel
      │
      ▼
SSD siap digunakan
```

---

# Lokasi Driver

Contoh struktur direktori:

```text
/system/drivers/

gpu.znd
audio.znd
usb.znd
nvme.znd
wifi.znd
ethernet.znd
camera.znd
```

Driver pihak ketiga dapat ditempatkan di:

```text
/usr/drivers/

/vendor/drivers/
```

---

# Keamanan

Zenus dapat menerapkan beberapa lapisan keamanan untuk `.znd`:

* Isolasi driver di ruang pengguna (user-space) untuk perangkat tertentu.
* Driver kernel hanya boleh mengakses memori yang diberikan kernel.
* Validasi parameter dari aplikasi.
* Pembatasan akses DMA melalui IOMMU.
* Verifikasi tanda tangan digital sebelum dimuat.
* Dukungan rollback otomatis jika driver baru menyebabkan kegagalan boot.

---

# Keunggulan `.znd`

| Fitur                     | `.znd` |
| ------------------------- | ------ |
| Dukungan AEX32/AEX64      | ✅      |
| Hardware ID bawaan        | ✅      |
| Hot-plug                  | ✅      |
| Multi-core aware          | ✅      |
| DMA support               | ✅      |
| Power management          | ✅      |
| Interrupt handling        | ✅      |
| Digital signature         | ✅      |
| ASLR & relokasi           | ✅      |
| Sandbox driver (opsional) | ✅      |
| Rollback pembaruan        | ✅      |
| Virtualization-ready      | ✅      |

Dengan rancangan seperti ini, `.znd` tidak hanya menjadi "file driver", tetapi sebuah format modular yang memungkinkan Zenus OS mengelola perangkat keras secara aman, efisien, dan mudah diperbarui. Desain ini juga memberi ruang untuk fitur modern seperti isolasi driver, hot-plug, dan dukungan perangkat generasi baru tanpa harus mengubah format dasarnya.


`.znm` berada satu tingkat lebih dekat ke inti sistem dibanding `.znd`. Kalau **`.znd`** berbicara dengan perangkat keras, maka **`.znm` (Zenus Native Module)** memperluas kemampuan **kernel** itu sendiri. Anggap saja seperti memasang organ baru ke tubuh yang masih hidup. Karena manusia tampaknya menyukai tantangan semacam itu.

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
`.zni` adalah **image sistem** Zenus OS. Jika `.zns` adalah satu aplikasi dan `.znpkg` adalah paket instalasi, maka `.zni` adalah gambaran lengkap sebuah sistem atau partisi yang siap digunakan. Format ini bisa dipakai untuk boot, recovery, installer, mesin virtual, bahkan deployment server. Karena memasang ulang sistem dari nol setiap kali ada masalah memang terdengar heroik, tetapi jauh lebih efisien jika cukup memuat sebuah image.

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

# 7. `.znf` (Zenus Native Firmware)

## Tujuan

`.znf` adalah format firmware standar Zenus yang digunakan untuk memperbarui atau memuat firmware ke berbagai perangkat keras.

Digunakan untuk:

* SSD/NVMe
* GPU
* Wi-Fi
* Bluetooth
* Keyboard
* Touchpad
* Embedded Controller (EC)
* BIOS/UEFI (jika didukung vendor)
* Microcontroller
* IoT Device
* FPGA bitstream (opsional)

Contoh:

```text
nvme.znf
gpu.znf
wifi.znf
ec.znf
touchpad.znf
bios-update.znf
```

---

# Filosofi Desain

`.znf` dirancang agar:

* Aman diperbarui.
* Memiliki mekanisme rollback.
* Mendukung update parsial.
* Mendukung verifikasi digital.
* Mendukung multi-versi hardware.
* Mendukung pembaruan OTA (Over-the-Air).
* Mendukung enkripsi.
* Tahan terhadap kegagalan daya saat update.

---

# Struktur File

```text
+---------------------------------------------+
| Magic Number                                |
+---------------------------------------------+
| Firmware Header                             |
+---------------------------------------------+
| Device Compatibility Table                  |
+---------------------------------------------+
| Firmware Metadata                           |
+---------------------------------------------+
| Bootloader Section                          |
+---------------------------------------------+
| Firmware Code                               |
+---------------------------------------------+
| Configuration Section                       |
+---------------------------------------------+
| Calibration Data                            |
+---------------------------------------------+
| Recovery Firmware                           |
+---------------------------------------------+
| Update Script                               |
+---------------------------------------------+
| Checksum                                    |
+---------------------------------------------+
| Digital Signature                           |
+---------------------------------------------+
```

---

# 1. Magic Number

Contoh:

```text
ZNFW
```

Hexadecimal:

```text
5A 4E 46 57
```

Bootloader atau utilitas pembaruan langsung mengenali file sebagai firmware Zenus.

---

# 2. Firmware Header

Berisi informasi dasar.

| Field         | Fungsi                        |
| ------------- | ----------------------------- |
| Firmware Name | Nama firmware                 |
| Version       | Versi                         |
| Device Type   | GPU, SSD, Wi-Fi, dll.         |
| Vendor        | Produsen perangkat            |
| Build Date    | Tanggal build                 |
| Architecture  | ARM, RISC-V, AEX-M, x86, dll. |
| Image Size    | Ukuran firmware               |

---

# 3. Device Compatibility Table

Menentukan perangkat mana yang boleh menggunakan firmware ini.

Contoh:

```text
Vendor ID : 8086
Device ID : A123

Vendor ID : 10EC
Device ID : 8852
```

Jika perangkat tidak cocok, proses update dibatalkan.

---

# 4. Firmware Metadata

Contoh:

```text
Firmware:
Zenus Wi-Fi Firmware

Version:
5.1.0

Release:
Stable

Minimum Driver:
wifi.znd 2.0

Supported Kernel:
Zenus 3.0+
```

Metadata membantu sistem memastikan kompatibilitas antara firmware dan driver.

---

# 5. Bootloader Section

Beberapa perangkat memiliki bootloader internal.

Bagian ini dapat berisi:

```text
Recovery Bootloader

Secure Boot Key

Boot Configuration
```

Biasanya hanya diperbarui jika benar-benar diperlukan.

---

# 6. Firmware Code

Berisi instruksi yang dijalankan langsung oleh prosesor di dalam perangkat.

Contoh fungsi:

```text
InitializeHardware()

HandleDMA()

ManagePower()

ReceivePackets()

ProcessCommands()
```

Kode ini berbeda dari `.zns` karena tidak dijalankan oleh CPU utama sistem, melainkan oleh mikrokontroler atau prosesor di perangkat tersebut.

---

# 7. Configuration Section

Berisi konfigurasi bawaan.

Contoh:

```text
Power Limit

Clock Speed

Buffer Size

Operating Mode

Region Code
```

---

# 8. Calibration Data

Penting untuk perangkat tertentu.

Misalnya:

```text
Wi-Fi Antenna Calibration

Touchscreen Sensitivity

Battery Calibration

Camera White Balance
```

Data ini sering kali unik untuk model atau bahkan unit perangkat tertentu.

---

# 9. Recovery Firmware

Salinan firmware cadangan yang dapat digunakan jika pembaruan utama gagal.

Alur:

```text
Update gagal
      │
      ▼
Recovery Firmware aktif
      │
      ▼
Perangkat kembali ke versi aman
```

---

# 10. Update Script

Berisi urutan pembaruan.

Contoh:

```text
Verify Device

Backup Current Firmware

Erase Flash

Write New Firmware

Verify Checksum

Reboot Device
```

---

# 11. Checksum

Digunakan untuk memastikan data tidak rusak.

Contoh algoritma:

```text
SHA-256

SHA-512

BLAKE3

CRC32 (untuk pengecekan cepat)
```

---

# 12. Digital Signature

Berisi:

```text
Developer ID

Vendor Certificate

Signature

Hash
```

Perangkat hanya menerima firmware yang ditandatangani oleh pihak yang berwenang.

---

# Proses Update Firmware

Contoh pembaruan firmware SSD:

```text
User memilih nvme.znf
        │
        ▼
Driver nvme.znd memverifikasi perangkat
        │
        ▼
Sistem memeriksa tanda tangan digital
        │
        ▼
Firmware lama dicadangkan
        │
        ▼
Firmware baru ditulis ke flash
        │
        ▼
Checksum diverifikasi
        │
        ▼
SSD di-restart
        │
        ▼
Firmware baru aktif
```

---

# Lokasi Firmware

Contoh struktur direktori:

```text
/system/firmware/

gpu.znf
wifi.znf
bluetooth.znf
nvme.znf
touchpad.znf
camera.znf
```

Firmware pihak ketiga dapat ditempatkan di:

```text
/vendor/firmware/
```

---

# Keamanan

Zenus dapat menerapkan beberapa lapisan keamanan:

* Verifikasi tanda tangan digital sebelum update.
* Pengecekan kecocokan Vendor ID dan Device ID.
* Anti-downgrade untuk mencegah pemasangan firmware yang rentan.
* Backup otomatis firmware lama.
* Rollback jika update gagal.
* Dukungan TPM untuk menyimpan bukti integritas firmware.
* Log audit setiap proses pembaruan.

---

# Keunggulan `.znf`

| Fitur                        | `.znf` |
| ---------------------------- | ------ |
| Multi-device support         | ✅      |
| Device compatibility table   | ✅      |
| Recovery firmware            | ✅      |
| Incremental update           | ✅      |
| OTA update                   | ✅      |
| Digital signature            | ✅      |
| Checksum verification        | ✅      |
| Rollback                     | ✅      |
| Anti-downgrade               | ✅      |
| Enkripsi opsional            | ✅      |
| Dukungan multi-arsitektur    | ✅      |
| Aman terhadap kegagalan daya | ✅      |

## Integrasi dengan Zenus OS

Dalam ekosistem Zenus, hubungan antar format dapat dirancang seperti ini:

```text
.zns   → Aplikasi
.znl   → Library bersama
.znd   → Driver perangkat keras
.znm   → Modul kernel
.znpkg → Paket instalasi
.zni   → Image sistem
.znf   → Firmware perangkat
```

Dengan pembagian tanggung jawab yang jelas, setiap lapisan sistem memiliki format file khusus. Hal ini memudahkan pemeliharaan, meningkatkan keamanan, dan membuat Zenus OS memiliki identitas teknis yang konsisten dari aplikasi tingkat pengguna hingga firmware perangkat keras.


`.zna` adalah **Zenus Native Archive**, yaitu format arsip resmi Zenus OS. Jika `.znpkg` adalah paket instalasi yang memiliki logika pemasangan, maka `.zna` adalah wadah umum untuk menyimpan dan menggabungkan berbagai berkas menjadi satu file. Bayangkan seperti ZIP, TAR, atau 7Z, tetapi dirancang sejak awal untuk kebutuhan Zenus. Dunia memang sudah punya banyak format arsip, tetapi memiliki format internal yang terintegrasi memberi keuntungan dalam keamanan, metadata, dan performa.

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

# 9. `.znc` (Zenus Native Configuration)

## Tujuan

`.znc` adalah format konfigurasi universal yang digunakan oleh:

* Kernel
* Driver
* Modul Kernel
* Aplikasi
* Desktop Environment
* Network Manager
* Package Manager
* Service Manager
* Bootloader
* Virtual Machine
* AI Agent (opsional)

Contoh:

```text
system.znc
network.znc
display.znc
boot.znc
audio.znc
firewall.znc
desktop.znc
browser.znc
```

---

# Filosofi Desain

`.znc` dirancang agar:

* Mudah dibaca manusia.
* Cepat diproses mesin.
* Mendukung komentar.
* Mendukung validasi skema.
* Mendukung enkripsi sebagian nilai.
* Mendukung pewarisan konfigurasi (inheritance).
* Mendukung profil.
* Mendukung live reload.
* Mendukung versioning.

---

# Struktur File

```text
+----------------------------------------+
| Magic Number (opsional)                |
+----------------------------------------+
| Header                                 |
+----------------------------------------+
| Metadata                               |
+----------------------------------------+
| Configuration Tree                     |
+----------------------------------------+
| Validation Schema                      |
+----------------------------------------+
| Signature (opsional)                   |
+----------------------------------------+
```

Untuk file teks biasa, magic number dapat dihilangkan. Jika `.znc` disimpan dalam format biner, magic number membantu identifikasi.

---

# Sintaks Dasar

Contoh:

```znc
system {
    hostname = "Zenus-PC"

    timezone = "Asia/Jakarta"

    language = "id-ID"

    auto_update = true

    max_threads = 32
}
```

Sintaks dibuat sederhana:

* `{}` untuk blok
* `=` untuk assignment
* Mendukung string, angka, boolean, array, object

---

# Header

Bagian awal menjelaskan informasi konfigurasi.

Contoh:

```znc
header {

    version = "1.0"

    schema = "desktop"

    encoding = "UTF-8"

}
```

---

# Metadata

Berisi informasi tambahan.

```znc
metadata {

    author = "Zenus Foundation"

    created = "2026-07-03"

    modified = "2026-07-03"

    description = "Desktop configuration"

}
```

---

# Configuration Tree

Bagian utama.

Contoh:

```znc
desktop {

    wallpaper = "/system/wallpaper/default.jpg"

    theme = "Dark"

    icon_size = 48

    animations = true

}
```

---

# Mendukung Nested Object

```znc
network {

    ethernet {

        dhcp = true

    }

    wifi {

        ssid = "Zenus"

        auto_connect = true

    }

}
```

---

# Array

```znc
repositories = [

    "https://repo.zenus.org",

    "https://mirror1.zenus.org",

    "https://mirror2.zenus.org"

]
```

---

# Komentar

```znc
# konfigurasi jaringan

network {

    dhcp = true

}
```

atau

```znc
// konfigurasi firewall
```

---

# Environment Variable

```znc
home = "${HOME}"

cache = "${HOME}/.cache"
```

Loader akan mengganti variabel saat runtime.

---

# Include File

```znc
include "/system/config/network.znc"

include "/system/config/audio.znc"
```

Konfigurasi besar dapat dipecah menjadi beberapa file.

---

# Inheritance

```znc
profile Desktop {

    theme = "Dark"

}

profile Gaming : Desktop {

    fps_limit = 240

    game_mode = true

}
```

Profil `Gaming` otomatis mewarisi pengaturan dari `Desktop`.

---

# Validation Schema

Schema memastikan nilai yang diberikan valid.

Contoh:

```znc
validation {

    max_threads {

        type = integer

        minimum = 1

        maximum = 1024

    }

}
```

Jika pengguna menulis:

```znc
max_threads = -10
```

Sistem langsung menolak konfigurasi tersebut.

---

# Enkripsi Nilai

Data sensitif dapat dienkripsi.

```znc
wifi {

    password = encrypt("AES256")

}
```

Saat dibaca:

```text
************
```

Nilai asli hanya bisa diakses oleh proses yang berwenang.

---

# Live Reload

Beberapa aplikasi dapat memuat ulang konfigurasi tanpa restart.

Misalnya:

```text
display.znc
```

diubah menjadi:

```znc
brightness = 70
```

Desktop langsung menerapkan perubahan tanpa logout.

---

# Digital Signature

Konfigurasi sistem penting dapat ditandatangani.

Contoh:

```text
Signature

Developer ID

Certificate

Hash
```

Kernel dapat memastikan konfigurasi boot atau keamanan belum dimodifikasi.

---

# Contoh Konfigurasi Lengkap

```znc
header {

    version = "1.0"

}

system {

    hostname = "Zenus"

    language = "id-ID"

    timezone = "Asia/Jakarta"

}

network {

    dhcp = true

    hostname = "zenus"

}

display {

    resolution = "2560x1440"

    refresh_rate = 144

    theme = "Dark"

}
```

---

# Lokasi Konfigurasi

```text
/system/config/

boot.znc

network.znc

display.znc

audio.znc

security.znc

desktop.znc
```

Konfigurasi aplikasi:

```text
/home/user/.config/

browser.znc

editor.znc

terminal.znc
```

---

# Keunggulan `.znc`

| Fitur                   | `.znc` |
| ----------------------- | ------ |
| Mudah dibaca manusia    | ✅      |
| Cepat diparse           | ✅      |
| Komentar                | ✅      |
| Include file            | ✅      |
| Environment variable    | ✅      |
| Validation schema       | ✅      |
| Inheritance             | ✅      |
| Live reload             | ✅      |
| Versioning              | ✅      |
| Enkripsi nilai sensitif | ✅      |
| Digital signature       | ✅      |
| UTF-8 native            | ✅      |

# Integrasi dalam Ekosistem Zenus

```text
.zns    → Executable
.znl    → Shared Library
.znd    → Driver
.znm    → Kernel Module
.znpkg  → Paket Instalasi
.zni    → System Image
.znf    → Firmware
.zna    → Universal Archive
.znc    → Universal Configuration
```

## Saran Penyempurnaan

Agar `.znc` benar-benar menjadi format konfigurasi modern, saya menyarankan dua lapisan:

1. **ZNC-T (Text)**: Format teks seperti contoh di atas. Mudah diedit dengan editor apa pun dan ideal untuk pengembangan.
2. **ZNC-B (Binary)**: Representasi biner dari konfigurasi yang sama untuk produksi atau sistem embedded. Lebih cepat dibaca, lebih sulit dimodifikasi secara sembarangan, dan dapat menyimpan tanda tangan digital serta data terenkripsi dengan lebih efisien.

Pendekatan ini mirip dengan bagaimana beberapa ekosistem memiliki format yang mudah ditulis manusia sekaligus representasi biner yang dioptimalkan untuk performa, sehingga Zenus mendapatkan kemudahan penggunaan tanpa mengorbankan efisiensi.


`.znlog` adalah **Zenus Native Log**, yaitu format log standar di seluruh Zenus OS. Jika `.znc` mengatur konfigurasi sistem, maka `.znlog` mencatat **semua peristiwa (events)** yang terjadi di sistem. Daripada setiap aplikasi membuat format log sendiri yang harus dibaca dengan doa dan keberanian, Zenus dapat memiliki satu standar log yang konsisten.

# 10. `.znlog` (Zenus Native Log)

## Tujuan

`.znlog` adalah format log universal Zenus OS yang digunakan untuk mencatat aktivitas sistem secara terstruktur.

Digunakan oleh:

* Kernel
* Driver (`.znd`)
* Kernel Module (`.znm`)
* Aplikasi (`.zns`)
* Package Manager
* Bootloader
* Network Manager
* Security Service
* AI Agent
* Hypervisor
* Container Runtime

Contoh:

```text
kernel.znlog
boot.znlog
network.znlog
browser.znlog
security.znlog
installer.znlog
```

---

# Filosofi Desain

`.znlog` dirancang agar:

* Cepat ditulis.
* Aman.
* Mudah dicari.
* Mudah difilter.
* Mendukung streaming.
* Mendukung rotasi otomatis.
* Mendukung kompresi.
* Mendukung enkripsi.
* Mendukung tanda tangan digital.
* Mendukung analisis AI.

---

# Struktur File

```text
+---------------------------------------------+
| Magic Number                                |
+---------------------------------------------+
| Log Header                                  |
+---------------------------------------------+
| Session Information                         |
+---------------------------------------------+
| Log Entries                                 |
+---------------------------------------------+
| Event Index                                 |
+---------------------------------------------+
| Checksum                                    |
+---------------------------------------------+
| Digital Signature                           |
+---------------------------------------------+
```

---

# 1. Magic Number

Contoh:

```text
ZNLG
```

Hexadecimal:

```text
5A 4E 4C 47
```

Zenus mengetahui bahwa file tersebut adalah log resmi.

---

# 2. Log Header

Berisi informasi dasar.

| Field        | Fungsi              |
| ------------ | ------------------- |
| Log Version  | Versi format        |
| Source       | Sumber log          |
| Machine ID   | Identitas perangkat |
| Boot ID      | ID sesi boot        |
| Compression  | Jenis kompresi      |
| Encryption   | Jenis enkripsi      |
| Created Time | Waktu dibuat        |

---

# 3. Session Information

Setiap boot memiliki Session ID.

Contoh:

```text
Session ID:
4F8C-99AB-1D55

Boot Count:
325

Kernel Version:
3.1

Architecture:
AEX64
```

Dengan Session ID, log dari satu proses boot dapat dipisahkan dari boot lainnya.

---

# 4. Log Entry

Setiap kejadian dicatat sebagai satu entri.

Contoh:

```text
Timestamp:
2026-07-03 08:15:20.123

Level:
INFO

Component:
Kernel

Message:
Memory Manager initialized.
```

---

# Format Internal

Secara konseptual:

```text
Timestamp

Level

Subsystem

Module

Thread ID

CPU ID

Event ID

Message

Metadata
```

---

# Level Log

Zenus dapat memiliki level berikut:

| Level    | Arti                           |
| -------- | ------------------------------ |
| TRACE    | Sangat detail                  |
| DEBUG    | Untuk pengembang               |
| INFO     | Informasi umum                 |
| NOTICE   | Perubahan penting              |
| WARNING  | Peringatan                     |
| ERROR    | Kesalahan                      |
| CRITICAL | Gangguan serius                |
| FATAL    | Sistem tidak dapat melanjutkan |

---

# Event ID

Setiap event memiliki ID unik.

Contoh:

```text
BOOT001

NET103

GPU245

AUTH021

FS900
```

Keuntungan:

* Mudah dicari.
* Tidak bergantung pada bahasa.
* Cocok untuk dokumentasi dan troubleshooting.

---

# Metadata

Setiap log dapat menyimpan metadata.

Contoh:

```text
PID = 142

Thread = 8

CPU = 3

Memory = 25 MB

User = root
```

---

# Structured Logging

Selain pesan biasa:

```text
Network Connected
```

Zenus dapat menyimpan struktur:

```text
Event:
Network Connected

Interface:
eth0

IP:
192.168.1.25

Gateway:
192.168.1.1

Speed:
1 Gbps
```

AI maupun alat analisis tidak perlu menebak isi teks.

---

# Index

File log memiliki indeks.

Misalnya:

```text
BOOT

NETWORK

GPU

FILESYSTEM

SECURITY

APPLICATION
```

Viewer dapat langsung melompat ke bagian yang diinginkan tanpa membaca seluruh file.

---

# Kompresi

Log lama dapat dikompresi.

Contoh:

| Algoritma | Fungsi               |
| --------- | -------------------- |
| LZ4       | Sangat cepat         |
| Zstd      | Seimbang             |
| XZ        | Arsip jangka panjang |

---

# Rotasi Log

Misalnya:

```text
kernel.znlog

kernel.1.znlog

kernel.2.znlog

kernel.3.znlog
```

Atau berdasarkan tanggal:

```text
2026-07-01.znlog

2026-07-02.znlog

2026-07-03.znlog
```

---

# Enkripsi

Log sensitif dapat dienkripsi.

Misalnya:

```text
Security Log

Authentication Log

TPM Log

Kernel Panic
```

Menggunakan:

```text
AES-256

ChaCha20-Poly1305
```

---

# Digital Signature

Berisi:

```text
Developer ID

Machine Certificate

Hash

Signature
```

Mencegah log dimodifikasi tanpa terdeteksi.

---

# Contoh Isi Log

```text
[2026-07-03 08:00:01.120]

INFO

BOOT001

Bootloader initialized

----------------------------------

[2026-07-03 08:00:01.340]

INFO

KERNEL015

Memory Manager initialized

----------------------------------

[2026-07-03 08:00:02.100]

NOTICE

NET001

Ethernet Connected

----------------------------------

[2026-07-03 08:00:03.520]

WARNING

GPU211

GPU Temperature High

----------------------------------

[2026-07-03 08:00:04.910]

ERROR

APP510

Browser crashed
```

---

# Lokasi Penyimpanan

```text
/system/log/

kernel.znlog

boot.znlog

security.znlog

network.znlog
```

Log pengguna:

```text
/home/user/logs/

browser.znlog

editor.znlog

terminal.znlog
```

---

# AI Analysis

Karena formatnya terstruktur, AI dapat langsung melakukan:

* Mendeteksi crash.
* Menemukan pola kegagalan.
* Memprediksi kerusakan perangkat.
* Menganalisis performa.
* Menemukan aktivitas mencurigakan.
* Menghasilkan ringkasan log otomatis.

Contoh:

```text
AI Summary

Boot Time:
3.2 detik

Warning:
2

Error:
1

Critical:
0

Most Active Component:
Network
```

---

# Zenus Log Viewer

Zenus dapat menyediakan aplikasi bawaan:

```text
ZenLog Viewer
```

Fitur:

* Live log streaming.
* Filter berdasarkan level.
* Filter berdasarkan proses.
* Filter berdasarkan Event ID.
* Timeline boot.
* Grafik penggunaan CPU dan RAM.
* Ekspor ke `.zna`.
* Analisis AI sekali klik.

---

# Keunggulan `.znlog`

| Fitur                        | `.znlog` |
| ---------------------------- | -------- |
| Structured logging           | ✅        |
| Event ID                     | ✅        |
| Session ID                   | ✅        |
| Multi-thread aware           | ✅        |
| CPU ID                       | ✅        |
| Metadata                     | ✅        |
| Rotasi log                   | ✅        |
| Kompresi                     | ✅        |
| Enkripsi                     | ✅        |
| Digital signature            | ✅        |
| AI-ready                     | ✅        |
| Indeks untuk pencarian cepat | ✅        |

# Integrasi Lengkap Ekosistem Zenus

| Ekstensi | Fungsi            |
| -------- | ----------------- |
| `.zns`   | Executable        |
| `.znl`   | Shared Library    |
| `.znd`   | Driver            |
| `.znm`   | Kernel Module     |
| `.znpkg` | Paket Instalasi   |
| `.zni`   | System Image      |
| `.znf`   | Firmware          |
| `.zna`   | Universal Archive |
| `.znc`   | Configuration     |
| `.znlog` | Structured Log    |

## Saran untuk Zenus OS

Kalau tujuan Zenus adalah menjadi OS modern yang cocok untuk desktop, server, AI, dan embedded, saya akan menambahkan satu konsep lagi di atas `.znlog`, yaitu **Zenus Event Framework (ZEF)**.

Alih-alih aplikasi menulis teks bebas ke log, setiap komponen mengirim **event terstruktur** ke ZEF. Framework inilah yang kemudian:

* Menyimpan ke `.znlog`.
* Mengirim log real-time ke dashboard administrator.
* Meneruskan event keamanan ke sistem audit.
* Memberi umpan ke modul AI untuk deteksi anomali.
* Menghasilkan metrik performa secara otomatis.

Dengan pendekatan ini, `.znlog` bukan sekadar file log, tetapi menjadi bagian dari sistem observabilitas Zenus, mirip konsep yang digunakan pada platform modern untuk logging, monitoring, dan tracing, namun disatukan dalam satu standar sejak awal desain OS.

