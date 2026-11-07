Layer 1: Adaptive Identity Mesh (AIM)

"Jangan percaya siapa pun. Verifikasi semuanya, setiap saat."

Layer 1 adalah fondasi seluruh AETHER SHIELD. Semua layer lain bergantung pada keputusan yang dibuat di sini. Jika identitas suatu entitas tidak dapat dibuktikan, ia tidak akan pernah mencapai Layer 2.

Firewall tradisional biasanya menjawab pertanyaan:

"Apakah paket ini berasal dari IP yang diizinkan?"

Adaptive Identity Mesh menjawab:

"Apakah identitas ini benar-benar asli, masih sah saat ini, dan apakah perilakunya sesuai dengan identitas tersebut?"

Filosofi

Di dunia modern, IP Address bukan identitas.

IP bisa berubah.

MAC Address bisa dipalsukan.

Hostname bisa diganti.

VPN bisa menyamarkan lokasi.

Bahkan sertifikat bisa dicuri.

Maka identitas dibangun dari banyak faktor sekaligus.

Tujuan Layer 1

Layer ini mempunyai lima tujuan utama:

Mengenali seluruh entitas.
Memverifikasi identitas.
Memberi tingkat kepercayaan (Trust Score).
Memantau perubahan identitas secara real-time.
Menolak semua yang tidak bisa dibuktikan.
Apa yang Disebut Entitas?

Bukan hanya manusia.

Dalam AETHER SHIELD semua memiliki identitas.

Misalnya

User

Laptop

Virtual Machine

Container

Docker

Browser

Chrome

VSCode

Database

API

Service

Kernel Module

USB

Printer

Robot

AI Agent

Cloud Function

IoT

Smart Camera

Semuanya mempunyai identitas digital.

Struktur Identitas

Setiap entitas mempunyai Identity Card.

Contoh

Entity ID

Human Readable Name

Owner

Public Key

Private Key (Secure Storage)

Hardware Fingerprint

Software Fingerprint

Version

Role

Trust Score

Permission

Risk Score

Behavior History

Location History

Session History

Certificate

Token

Creation Time

Expiration

Hardware Identity

Jika perangkat memiliki TPM atau Secure Enclave:

CPU ID

TPM Identity

Secure Boot Hash

Firmware Hash

BIOS Measurement

Motherboard Fingerprint


Kalau salah satu berubah drastis, sistem menurunkan tingkat kepercayaan dan meminta verifikasi ulang.

Software Identity

Bukan hanya perangkat.

Program juga memiliki identitas.

Misalnya

Chrome

Executable Hash

Developer Signature

Expected DLL

Expected Syscall

Expected Memory Layout

Expected Network Pattern

Jika ada perubahan yang tidak sah, identitas dianggap rusak.

Identity DNA

Inilah bagian baru.

Setiap identitas mempunyai DNA.

Misalnya

Normal CPU Usage

Memory Pattern

Thread Pattern

Network Pattern

Disk Pattern

GPU Pattern

Typical Schedule

Typical Access

AI mempelajari semuanya.

Jika pola berubah:

Trust Score turun.
Multi-Factor Identity

Identitas tidak berasal dari satu sumber.

Melainkan gabungan.

Password

Passkey

TPM

Face

Fingerprint

FIDO2

Certificate

Device Health

Network Health

Behavior

Semua digabung.

Continuous Authentication

Biasanya login hanya sekali.

Di sini verifikasi berlangsung terus-menerus.

Contoh

08.00

Login

Trust = 98

09.00

Masih normal

Trust = 97

11.00

CPU Pattern berubah

Trust = 81

11.02

Browser mencoba akses kernel

Trust = 40

11.03

Session dibekukan.

Tidak perlu menunggu pengguna logout.

Trust Engine

Trust Score dihitung dari banyak parameter.

Misalnya

Identity Authenticity

Hardware Health

Certificate

Behavior

Location

Risk Intelligence

Device History

Software Integrity

Threat Feed


Hasil akhirnya

Trust Score = 0-100
Contoh

Laptop milik admin.

Identity

100

Hardware

98

Behavior

99

Location

98

Certificate

100

Total

Trust = 99

Laptop lain

Identity

95

Behavior

30

Unknown Country

40

New Browser

35

Unknown USB

20

Trust

34

Layer berikutnya langsung diberi tahu bahwa risiko tinggi.

Dynamic Permission

Hak akses berubah otomatis.

Trust 100

Full Access

Trust 80

Developer Access

Trust 60

Read Only

Trust 40

Limited Network

Trust 10

Isolation

Trust 0

Rejected
Session DNA

Bukan hanya perangkat.

Setiap sesi memiliki DNA.

Misalnya

Average Request Rate

Mouse Movement

Keyboard Pattern

Navigation

API Pattern

Network Flow

Latency


Jika berubah ekstrem, sistem tidak langsung menganggap akun dicuri, tetapi meningkatkan risiko dan meminta bukti tambahan sesuai kebijakan.

AI Identity Engine

Model AI membangun profil kebiasaan.

Contoh

Developer

Jam kerja

09-18

Editor

VSCode

Bahasa

Rust

Git

Aktif

Docker

Aktif

Besok

Jam

03.00

PowerShell

Ratusan proses

Credential Dump

Network Scan

Meskipun kata sandi benar, kombinasi perilaku ini meningkatkan risiko dan dapat memicu autentikasi ulang atau isolasi.

Cryptographic Identity

Semua komunikasi ditandatangani.

ECDSA

Ed25519

Dilithium (post-quantum)

Kyber (post-quantum key exchange)

TLS 1.3

mTLS

Dengan begitu, identitas tidak hanya berdasarkan "siapa yang mengaku", tetapi juga dapat dibuktikan secara kriptografis.

Identity Mesh

Mengapa disebut Mesh?

Karena tidak ada server identitas tunggal yang menjadi pusat kegagalan.

Laptop

↓

Identity Node

↓

Cloud

↓

Container

↓

Database

↓

API

↓

IoT

↓

Robot

Semuanya saling memverifikasi melalui jaringan kepercayaan yang terdistribusi. Jika satu node gagal, node lain tetap dapat memvalidasi identitas sesuai kebijakan yang ditetapkan.

Hasil Akhir Layer 1

Pada akhirnya, Layer 1 menghasilkan sebuah Identity Passport yang diteruskan ke Layer 2. Contohnya:

Entity ID      : DEV-LAPTOP-001
Owner          : Alice
Trust Score    : 97
Risk Score     : 3
Device Health  : Healthy
Behavior Match : 98%
Certificate    : Valid
Session Status : Active
Permission     : Developer
Expires        : 14:35 UTC


