Layer 2: Invisible Micro Perimeter (IMP)

"Jika identitas belum terbukti, maka sistem bahkan tidak boleh terlihat."

Layer 1 memastikan siapa yang mencoba mengakses sistem.

Layer 2 menentukan apa yang boleh dilihat dan diakses oleh identitas tersebut.

Inilah perbedaan terbesar dengan firewall tradisional.

Firewall berkata:

"Port 443 terbuka."

Invisible Micro Perimeter berkata:

"Tidak ada port untukmu karena aku bahkan tidak mengakui keberadaan layanan ini."

Filosofi

Internet saat ini bekerja seperti kota.

Semua bangunan terlihat.

Walaupun pintunya terkunci, orang masih bisa melihat:

alamat
ukuran
jenis bangunan
jumlah lantai

Penyerang dapat melakukan:

Port Scan
Banner Grabbing
OS Fingerprinting
Service Enumeration

AETHER SHIELD menghilangkan konsep itu.

Layanan hanya "muncul" untuk identitas yang telah dipercaya.

Tujuan Layer 2

Layer ini mempunyai beberapa tujuan:

Menyembunyikan seluruh layanan.
Membuat setiap koneksi bersifat sementara.
Memverifikasi identitas sebelum koneksi dibuat.
Mengisolasi setiap layanan.
Menghilangkan kepercayaan terhadap jaringan.
Konsep "Invisible Service"

Server sebenarnya tetap berjalan.

Tetapi bagi pihak yang tidak berwenang:

Tidak ada IP yang merespons.

Tidak ada port terbuka.

Tidak ada banner.

Tidak ada TLS handshake.

Tidak ada service discovery.

Bagi mereka, target tampak tidak ada.

Service Identity

Setiap layanan memiliki identitas sendiri.

Contoh

Service ID

Owner

Certificate

Public Key

Trust Score

Allowed Client

Allowed Version

Allowed Role

Health Score

Risk Score


Jadi bukan hanya pengguna yang memiliki identitas.

API pun memilikinya.

Database pun memilikinya.

Container pun memilikinya.

Identity Handshake

Sebelum koneksi dibuat terjadi proses seperti ini.

Client

↓

Identity Passport

↓

Identity Verification

↓

Trust Evaluation

↓

Permission Validation

↓

Temporary Session Key

↓

Encrypted Tunnel

↓

Service

Jika salah satu gagal

maka koneksi tidak pernah dibuat.

Dynamic Session

Tidak ada koneksi permanen.

Misalnya

Firewall biasa

Port 443

Selalu terbuka

AETHER SHIELD

Port Virtual

Hanya muncul

3 detik

untuk

Client A


Setelah selesai

port virtual menghilang.

Sidecar Guardian

Setiap layanan ditemani satu penjaga kecil.

Misalnya

Container

↓

Sidecar Guardian

↓

Application

Tugas Sidecar:

memverifikasi identitas,
memeriksa kebijakan,
membuat tunnel terenkripsi,
memantau perilaku sesi,
menghentikan koneksi bila risiko meningkat.

Aplikasi utama tidak perlu menangani semua logika keamanan.

Dynamic Tunnel

Semua komunikasi dilakukan melalui tunnel sementara.

Contoh

Client

↓

Session Key

↓

Tunnel A

↓

Database

Client lain

↓

Tunnel B

Tunnel A dan Tunnel B berbeda.

Tidak ada tunnel yang dipakai bersama.

Service Cloaking

Ini fitur penting.

Misalnya server memiliki:

SSH

Database

Redis

RabbitMQ

API


Bagi Administrator

Semuanya terlihat.

Bagi Developer

API

Git


Bagi User

Frontend saja.

Bagi Penyerang

Tidak ada apa pun.
Adaptive Network Topology

Topologi jaringan tidak statis.

Misalnya

Jam 10.00

API

↓

Database A

Jam 10.05

API

↓

Database B

Jam 10.10

API

↓

Replica

Perubahan ini transparan bagi aplikasi, tetapi mempersulit pemetaan oleh pihak yang tidak berwenang.

Micro Segmentation

Setiap layanan hidup di ruang sendiri.

Contoh

Payment

Inventory

User

Auth

Analytics

Jika salah satu mengalami kompromi,

layanan lain tetap tidak dapat diakses secara bebas karena komunikasi antarsegmen dikontrol oleh identitas dan kebijakan.

Least Privilege Routing

Tidak semua layanan boleh saling berbicara.

Misalnya

Payment

↓

Database

Diizinkan.

Tetapi

Payment

↓

HR Database

Ditolak.

Bukan karena firewall,

melainkan karena identitas layanan tidak memiliki hak tersebut.

Continuous Session Verification

Layer 1 memverifikasi identitas.

Layer 2 memverifikasi sesi.

Misalnya

Menit pertama

Trust

97

Menit kedua

Trust

95

Menit ketiga

Trust

52

Karena pola akses berubah drastis.

Layer 2 dapat:

menurunkan hak akses,
meminta autentikasi ulang,
atau memutus koneksi.
Quantum-Ready Secure Tunnel

Tunnel dapat menggunakan kombinasi:

TLS 1.3

mTLS

Ed25519

X25519

Kyber

Dilithium

Sehingga desain siap beradaptasi dengan algoritma kriptografi modern maupun pasca-kuantum ketika diperlukan.

Service Mesh Intelligence

Setiap Sidecar Guardian berbagi informasi.

Misalnya

Service A

↓

Trust turun

Informasi langsung disebarkan.

Service lain mengetahui bahwa identitas tersebut sedang berisiko tinggi dan dapat menyesuaikan kebijakannya tanpa menunggu pembaruan manual.

Dynamic Risk Response

Keputusan tidak hanya "boleh" atau "tidak".

Contohnya:

Trust Score	Tindakan
95-100	Akses penuh
80-94	Akses normal dengan pemantauan
60-79	Akses terbatas dan logging diperketat
40-59	Isolasi sebagian, autentikasi tambahan
20-39	Hanya layanan yang sangat terbatas
0-19	Koneksi ditolak
Output Layer 2

Setelah Layer 2 selesai, setiap sesi menghasilkan Secure Session Context yang akan dikirim ke Layer 3.

Session ID      : S-7A91E
Entity ID       : DEV-LAPTOP-001
Service         : Payment API
Tunnel ID       : TNL-4C18
Trust Score     : 96
Risk Score      : 4
Permission      : Read/Write
Encryption      : TLS 1.3 + mTLS
Session TTL     : 300 detik
Monitoring Mode : Active

