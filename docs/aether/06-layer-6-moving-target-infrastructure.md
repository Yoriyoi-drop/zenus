Layer 6: Moving Target Infrastructure (MTI)
"Jika musuh sudah mengetahui peta sistemmu, maka ubahlah petanya sebelum ia sempat menggunakannya."

Layer 1 menjawab

Siapa kamu?

Layer 2 menjawab

Apa yang boleh kamu akses?

Layer 3 menjawab

Apakah perilakumu normal?

Layer 4 menjawab

Apa tujuanmu?

Layer 5 menjawab

Apa yang kemungkinan akan terjadi?

Layer 6 menjawab

Bagaimana cara mengubah sistem agar prediksi serangan itu gagal?

Layer ini adalah mekanisme pertahanan dinamis AETHER SHIELD.

Filosofi

Sebagian besar sistem keamanan bersifat statis.

Misalnya

Server

IP
192.168.1.10

Port
443

Hostname
api.company.local

Semua tetap sama selama berbulan-bulan.

Penyerang memiliki waktu tanpa batas untuk:

melakukan reconnaissance,
memetakan jaringan,
menemukan celah,
menyusun strategi.

Moving Target Infrastructure menghilangkan keuntungan tersebut.

Tujuan Layer 6

Layer ini bertugas:

Mengubah permukaan serangan secara dinamis.
Membuat informasi hasil reconnaissance cepat usang.
Mengurangi peluang keberhasilan eksploitasi.
Menyesuaikan infrastruktur berdasarkan tingkat risiko.
Menyiapkan lingkungan untuk Layer 7.
Dynamic Infrastructure

Tidak ada komponen yang benar-benar permanen.

Misalnya

Pagi

API

↓

Server A

Siang

API

↓

Server C

Malam

API

↓

Server F

Perubahan dilakukan tanpa mengganggu pengguna.

Dynamic IP Rotation

Alamat jaringan bersifat virtual.

Misalnya

10.0.0.15

5 menit kemudian

10.0.8.44

10 menit kemudian

10.0.2.91

Aplikasi tetap dapat berkomunikasi karena menggunakan Service Identity, bukan alamat IP secara langsung.

Dynamic Host Identity

Hostname juga berubah.

Misalnya

api-prod-01

↓

service-x19

↓

node-f84

Identitas logis tetap sama.

Nama fisik berubah.

Adaptive Port Virtualization

Firewall klasik

HTTPS

443

Selalu.

AETHER SHIELD

443

↓

51822

↓

32214

↓

60110

Nomor port virtual dapat berubah sesuai kebijakan dan sesi komunikasi.

Dynamic Service Discovery

Tidak ada daftar layanan permanen.

Client bertanya

Saya ingin Payment API.

Identity Mesh menjawab

Payment API

↓

Node

X-94

↓

Tunnel

812

Client tidak pernah mengetahui lokasi sebenarnya.

Live Workload Migration

Jika AI memprediksi ancaman

workload dapat dipindahkan.

Misalnya

Server A

↓

Server D

Pengguna tetap terhubung.

Penyerang kehilangan target.

Dynamic Container Relocation

Container tidak selalu hidup di node yang sama.

Misalnya

Node 5

↓

Node 2

↓

Node 14

↓

Node 8

Perpindahan dilakukan sesuai kebijakan dan kondisi sistem.

Adaptive Memory Randomization

Selain ASLR standar

AETHER SHIELD memiliki

Continuous Memory Randomization

Misalnya

Library

0x7F23

↓

0x8A52

↓

0x91D1

Dilakukan saat aplikasi masih berjalan jika platform mendukungnya.

Tujuannya memperkecil peluang eksploitasi yang bergantung pada tata letak memori yang dapat diprediksi.

Dynamic API Mapping

Misalnya

Hari ini

/api/v2/payment

↓

Besok

/service/payment

↓

Lusa

/endpoint/x91

Klien resmi memperoleh pemetaan terbaru melalui Identity Mesh.

Klien tidak sah tidak mendapatkan informasi tersebut.

Runtime Binary Rotation

Beberapa layanan dapat memiliki beberapa build yang setara secara fungsional.

Misalnya

Binary A

↓

Binary B

↓

Binary C

Semuanya memberikan layanan yang sama.

Tetapi tata letak internal berbeda.

Hal ini dapat meningkatkan ketahanan terhadap eksploitasi yang sangat spesifik pada implementasi tertentu.

Moving Trust Zone

Zona kepercayaan tidak tetap.

Contoh

Jam 09.00

Developer

↓

Production

Read Only

Jam 10.00

Risiko naik.

Developer

↓

Production

No Access

Jam 11.00

Risiko turun.

Developer

↓

Production

Read Only

Hak akses berubah mengikuti evaluasi risiko.

Dynamic Encryption

Kunci sesi diperbarui secara berkala.

Misalnya

Session Key

↓

K1

↓

K2

↓

K3

↓

K4

Jika satu kunci bocor,

kunci berikutnya tetap aman.

Autonomous Topology Mutation

AI dapat mengubah topologi.

Misalnya

Awalnya

Web

↓

API

↓

Database

Saat ancaman meningkat

Web

↓

Proxy

↓

Replica

↓

Database

Atau menambahkan lapisan tambahan untuk membatasi jalur akses.

Dynamic Service Replica

AI dapat membuat replika layanan.

Misalnya

API

↓

Node A

Menjadi

API

↓

Node A

↓

Node B

↓

Node C

Kemudian lalu lintas dialihkan sesuai kebutuhan.

Shadow Infrastructure

Layer ini dapat membuat lingkungan bayangan.

Misalnya

Database Asli

↓

Database Shadow

Aktivitas yang dianggap berisiko dapat diarahkan ke lingkungan yang diawasi lebih ketat untuk analisis tanpa langsung mengganggu layanan utama.

Adaptive Network Mutation

AI mengubah jalur komunikasi.

Misalnya

Normal

Client

↓

API

↓

Database

Saat ancaman meningkat

Client

↓

Gateway

↓

Inspection

↓

Proxy

↓

Database

Lapisan tambahan diaktifkan hanya ketika diperlukan.

Infrastructure Entropy Engine

Layer ini menghitung Infrastructure Entropy Score.

Semakin tinggi nilainya,

semakin sulit bagi penyerang membangun peta yang akurat.

Contoh

Entropy	Kondisi
10	Infrastruktur hampir statis
40	Perubahan ringan
70	Perubahan rutin
90	Sangat dinamis
100	Berubah hampir terus-menerus
Mutation Policy Engine

Tidak semua perubahan dilakukan terus-menerus.

AI menentukan kapan perubahan dilakukan.

Contoh

Threat Level

↓

Low

Perubahan

30 menit
Threat Level

↓

Medium

Perubahan

10 menit
Threat Level

↓

Critical

Perubahan

30 detik

Frekuensi disesuaikan agar keamanan meningkat tanpa mengorbankan stabilitas.

Cost Optimization Engine

Perubahan infrastruktur memerlukan sumber daya.

Layer ini menghitung:

CPU

RAM

Bandwidth

Storage

Latency

Availability

Jika biaya perubahan lebih besar daripada manfaatnya,

AI memilih strategi lain.

Autonomous Mutation Planner

AI membuat rencana perubahan.

Misalnya

Step 1

Rotasi Tunnel

↓

Step 2

Migrasi Container

↓

Step 3

Perbarui Identity Route

↓

Step 4

Rotasi Session Key

↓

Step 5

Hapus Jalur Lama

Semua dilakukan secara bertahap agar layanan tetap tersedia.

Output Layer 6

Layer ini menghasilkan Infrastructure Mutation Plan.

Plan ID              : MTI-40182
Threat Level         : High
Mutation Strategy    : Adaptive Rotation
IP Rotation          : Enabled
Port Virtualization  : Enabled
Service Relocation   : Payment API → Node-17
Tunnel Rotation      : Every 60 seconds
Key Rotation         : Every 30 seconds
Expected Downtime    : 0%
Infrastructure Score : 97


