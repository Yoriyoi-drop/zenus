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

