Layer 3: Living Process DNA (LPD)

"Jangan hanya mengenali programnya. Kenali kehidupan normalnya."

Layer 1 menjawab:

Siapa kamu?

Layer 2 menjawab:

Apa yang boleh kamu akses?

Layer 3 menjawab:

Apakah perilakumu masih sesuai dengan dirimu sendiri?

Inilah lapisan yang mengubah AETHER SHIELD dari sekadar sistem autentikasi menjadi sistem yang memahami perilaku setiap proses.

Filosofi

Firewall melihat paket.

Antivirus melihat file.

EDR melihat aktivitas.

Living Process DNA melihat kepribadian digital setiap proses.

Misalnya:

Chrome.

Firewall hanya tahu:

chrome.exe

Living Process DNA mengetahui:

Chrome biasanya:

- memakai 30-80 thread
- membuka HTTP/HTTPS
- memakai GPU
- membuka cache
- memakai sandbox
- tidak pernah membaca LSASS
- tidak pernah memanggil driver kernel
- tidak pernah mengakses raw disk

Jadi saat Chrome tiba-tiba melakukan hal yang tidak sesuai, sistem segera menilai bahwa ada penyimpangan.

Tujuan Layer 3

Layer ini bertugas untuk:

Membuat DNA setiap proses.
Memantau perubahan DNA.
Menghitung tingkat penyimpangan.
Memberikan skor risiko.
Mengirim hasil ke Layer 4.
Apa itu DNA?

DNA bukan hash file.

DNA adalah keseluruhan karakteristik hidup suatu proses.

Misalnya:

Executable

↓

Memory

↓

Thread

↓

CPU

↓

GPU

↓

Syscall

↓

Registry

↓

Network

↓

Disk

↓

Behavior

Semuanya menjadi satu profil.

Struktur DNA

Setiap proses memiliki DNA seperti berikut.

Process ID

Executable Hash

Parent Process

Child Process

Thread Count

Memory Layout

CPU Pattern

GPU Pattern

Expected DLL

Expected Syscall

Expected API

Expected Registry

Expected File Access

Expected Network Flow

Privilege

Environment

Digital Signature

Risk History

DNA Learning

Saat pertama kali aplikasi dijalankan

AI belum mengetahui karakteristiknya.

Misalnya

Visual Studio Code

Selama beberapa hari

AI mempelajari

Jam penggunaan

CPU

RAM

Extension

Git

Compiler

Debugger

Network

Plugin

Kemudian dibuat DNA permanen.

DNA Evolution

DNA tidak statis.

Misalnya

Hari pertama

VSCode

CPU

15%

Setelah update

20%

Masih normal.

Tetapi

300%

secara tiba-tiba

langsung dianggap anomali.

Komponen DNA
1. Memory DNA

Memahami pola memori.

Contoh

Heap

Stack

Virtual Memory

Shared Memory

Mapped File

Allocation Pattern


Jika ada:

Shellcode
Reflective DLL
Memory Injection

DNA berubah drastis.

2. Thread DNA

Normal

40 Thread

Hari ini

600 Thread

atau

Thread

↓

Remote Process

Anomali.

3. Syscall DNA

Contoh Chrome

Biasanya

Read

Write

Socket

Window

GPU

Tiba-tiba

NtWriteVirtualMemory

NtCreateThreadEx

NtMapViewOfSection

DNA berubah.

4. Network DNA

Normal

HTTPS

HTTP/2

QUIC

DNS

Tiba-tiba

Raw Socket

SMB Scan

Port Sweep

ICMP Flood

Risiko meningkat.

5. Disk DNA

Normal

Read Cache

Download

History

Tiba-tiba

Encrypt

Rename

Delete

Shadow Copy

DNA berubah drastis.

6. Registry DNA

Normal

Browser Setting

Hari ini

RunOnce

Services

LSA

Credential

Security Policy

Tidak sesuai.

7. Parent-Child DNA

Normal

Explorer

↓

Chrome

Normal.

Tetapi

Word

↓

PowerShell

↓

CMD

↓

Rundll32

↓

Chrome

Hubungan ini jauh lebih mencurigakan.

Behavioral Fingerprint

Selain DNA

AI membuat sidik jari perilaku.

Misalnya

CPU Burst

↓

Memory Burst

↓

Disk Burst

↓

Network Burst

Urutan aktivitas juga penting.

DNA Similarity Engine

AI menghitung

Current DNA

VS

Expected DNA

Hasil

Similarity

99%

Normal.

78%

Perlu dipantau.

41%

Berisiko tinggi.

Mutation Detection

Layaknya biologi,

DNA dapat mengalami mutasi.

Misalnya

Chrome

↓

Injected DLL

↓

Thread Baru

↓

Privilege Naik

AI mencatat perubahan tersebut sebagai mutasi dan mengevaluasi apakah masih wajar, misalnya karena ekstensi resmi atau pembaruan, atau justru mengindikasikan kompromi.

Digital Twin Comparison

Layer ini menggunakan Digital Twin.

Misalnya

AI memiliki model virtual

Notepad

Versi nyata

dibandingkan dengan

versi virtual.

Jika terlalu berbeda

risiko naik.

Risk Engine

Setiap perubahan diberi bobot.

Contoh

Perubahan	Skor Risiko
Thread bertambah sedikit	+2
DLL baru yang valid	+5
DLL tanpa tanda tangan	+20
Memory Injection	+45
Privilege Escalation	+35
Shellcode	+50
Raw Disk Access	+30

Skor ini diteruskan ke mesin keputusan.

AI Mutation Predictor

Ini fitur baru.

AI tidak hanya melihat kondisi saat ini.

Tetapi memprediksi

langkah berikutnya.

Misalnya

Word

↓

PowerShell

AI memprediksi kemungkinan berikut:

↓

Download Payload

↓

Credential Dump

↓

Persistence

Belum tentu benar, tetapi probabilitas tersebut membantu menentukan tingkat kewaspadaan dan tindakan yang proporsional.

DNA Vault

Semua DNA disimpan.

Chrome

v136

↓

Chrome

v137

↓

Chrome

v138

Dengan demikian perubahan karena pembaruan resmi dapat dibedakan dari perubahan akibat kompromi.

Output Layer 3

Layer 3 menghasilkan Process DNA Profile yang dikirim ke Layer 4.

Process ID        : 2841
Application       : chrome.exe
DNA Similarity    : 96%
Mutation Score    : 4
Behavior Score    : 98
Risk Score        : 6
Threat Prediction : Low
Parent Process    : explorer.exe
Status            : Healthy

