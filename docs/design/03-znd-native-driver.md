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

