Layer 4: Intent & Behavior Graph (IBG)
"Jangan hanya melihat apa yang dilakukan. Pahami tujuan akhirnya."

Layer 1 menjawab:

Siapa kamu?

Layer 2 menjawab:

Apa yang boleh kamu akses?

Layer 3 menjawab:

Apakah perilakumu masih normal?

Layer 4 menjawab:

Apa sebenarnya tujuan dari semua aktivitas ini?

Inilah otak analisis AETHER SHIELD.

Firewall melihat paket.

EDR melihat event.

SIEM melihat log.

Layer 4 melihat cerita lengkap.

Filosofi

Satu aktivitas sering kali terlihat normal.

Contoh

Word membuka file.

Normal.

PowerShell berjalan.

Masih normal.

Chrome mengunduh file.

Masih normal.

Tetapi jika semuanya digabung:

Word

↓

PowerShell

↓

Download EXE

↓

Disable Defender

↓

Encrypt Files

Maka itu bukan lagi lima aktivitas.

Itu adalah satu niat.

Tujuan Layer 4

Layer ini bertugas:

Menghubungkan seluruh aktivitas.
Membentuk Behavior Graph.
Mengidentifikasi tujuan.
Memprediksi langkah berikutnya.
Memberikan keputusan ke Layer 5.
Behavior Graph

Semua aktivitas menjadi node.

Misalnya

Chrome

↓

Download

↓

Save File

↓

Run EXE

↓

Inject DLL

↓

Open Socket

↓

Beacon

↓

Persistence

AI melihat keseluruhan graph.

Apa itu Graph?

Graph terdiri dari

Node

↓

Edge

↓

Relationship

Node

Process

File

Registry

Memory

Socket

User

Service

Container

API

Edge

Open

Read

Write

Inject

Spawn

Delete

Encrypt

Connect

Fork

Contoh Graph
Explorer

↓

Word

↓

PowerShell

↓

CMD

↓

Rundll32

↓

Credential Dump

↓

Exfiltration

↓

Persistence

Semua node saling terhubung.

Intent Engine

Inilah inti Layer 4.

AI mencoba menjawab

"Apa tujuan akhirnya?"

Misalnya

Aktivitas

Chrome

↓

Download ZIP

↓

Extract

↓

Run

Bisa saja

Install Software

Tetapi

Chrome

↓

Download ZIP

↓

Extract

↓

PowerShell

↓

Disable Security

↓

Inject

Kemungkinan besar merupakan rantai serangan.

Intent Library

Layer ini memiliki perpustakaan pola tujuan.

Misalnya

Credential Theft
Browser

↓

LSASS

↓

Memory Read

↓

Dump

↓

Compress

↓

Network
Ransomware
Read

↓

Encrypt

↓

Rename

↓

Delete Backup

↓

Shadow Copy

↓

Network
Worm
Network Scan

↓

Exploit

↓

Copy

↓

Execute

↓

Repeat
Supply Chain Attack
IDE

↓

Compiler

↓

Inject

↓

Build

↓

Sign

↓

Deploy
Insider Threat
Employee

↓

Database

↓

Mass Query

↓

Compress

↓

USB

↓

Cloud Upload
Multi-Dimensional Graph

Graph bukan hanya berdasarkan proses.

Tetapi juga:

User

↓

Device

↓

Container

↓

API

↓

Database

↓

Cloud

↓

Network

↓

Identity

Semuanya saling terhubung.

Time Graph

Layer ini juga memahami waktu.

Misalnya

09:00

Login

↓

09:01

Download

↓

09:02

Disable Security

↓

09:03

Encrypt

Jika semua terjadi dalam beberapa menit,

risiko meningkat drastis.

Spatial Graph

AI memahami lokasi.

Misalnya

Laptop Jakarta

↓

Server Tokyo

↓

Database Frankfurt

Jika tiba-tiba

Login

↓

Russia

↓

Brazil

↓

Indonesia

↓

5 Menit

Ada indikasi anomali yang memerlukan verifikasi lebih lanjut.

Relationship Weight

Tidak semua hubungan sama.

Misalnya

Chrome

↓

HTTPS

Bobot rendah.

Tetapi

Chrome

↓

LSASS

Bobot sangat tinggi.

AI memberi nilai berbeda.

Chain Risk Engine

Misalnya

PowerShell

↓

Download

Risiko

20

Lalu

↓

Memory Injection

Risiko

60

Lalu

↓

Persistence

Risiko

95

Yang dihitung bukan hanya event, tetapi akumulasi rantai.

Context Engine

Layer ini memahami konteks.

Contoh

Developer

Compile

Git

Docker

SSH

Normal.

Tetapi

Accounting

Compile

Docker

Kernel Debugger

Tidak lazim untuk peran tersebut sehingga skor risiko meningkat.

Digital Story Builder

Semua event dijadikan cerita.

Misalnya

User Login

↓

Open Email

↓

Open Attachment

↓

Macro

↓

PowerShell

↓

Download

↓

Memory Injection

↓

Persistence

↓

Beacon

↓

Lateral Movement

AI tidak melihatnya sebagai 10 log.

AI melihatnya sebagai satu insiden.

Predictive Attack Engine

Setelah graph terbentuk

AI memprediksi langkah berikutnya.

Misalnya

Saat ini

PowerShell

↓

Download Payload

Prediksi

Credential Dump

82%
Persistence

75%
Lateral Movement

69%

Prediksi ini digunakan untuk meningkatkan kesiapsiagaan dan memilih respons yang sesuai, bukan sebagai bukti pasti bahwa langkah tersebut akan terjadi.

Threat DNA

Setiap serangan memiliki DNA.

Contoh

APT

Recon

↓

Persistence

↓

Credential

↓

Lateral

↓

Exfiltration

Ransomware

Encrypt

↓

Delete Backup

↓

Rename

↓

Ransom Note

Layer 4 membandingkan graph yang sedang terbentuk dengan pola-pola tersebut.

Graph Compression Engine

Dalam perusahaan besar

bisa muncul miliaran event setiap hari.

Layer ini mengompresi event menjadi graph yang lebih kecil.

Misalnya

2 Miliar Event

↓

5 Juta Graph

↓

12 Ribu Threat Graph

↓

40 Critical Graph

Analis tidak lagi membaca miliaran log.

Mereka melihat beberapa puluh graph yang paling penting.

Causal Graph

Ini adalah peningkatan dari sekadar Behavior Graph.

Behavior Graph menjawab

Apa yang terjadi?

Causal Graph menjawab

Mengapa hal itu terjadi?

Contoh

Update Service

↓

Restart

↓

CPU Tinggi

Normal.

Tetapi

Email Attachment

↓

Macro

↓

PowerShell

↓

Credential Dump

Hubungan sebab-akibat menunjukkan pola yang jauh lebih mengkhawatirkan.

Decision Engine

Setelah graph selesai dianalisis

Layer 4 memberi keputusan.

Misalnya

Intent Score	Keputusan
0-20	Normal
21-40	Monitoring
41-60	Enhanced Monitoring
61-80	Partial Isolation
81-100	Critical Threat
Output Layer 4

Layer ini menghasilkan Intent Intelligence Report.

Incident ID        : INT-20481
Primary Intent     : Credential Theft
Confidence         : 94%
Attack Stage       : Initial Access
Behavior Score     : 92
Intent Score       : 95
Predicted Next     : Lateral Movement
Affected Processes : 17
Affected Devices   : 3
Risk Level         : Critical
Recommended Action : Isolate Session & Escalate

