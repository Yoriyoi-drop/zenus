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

