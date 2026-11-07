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

