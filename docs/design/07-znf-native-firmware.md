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

