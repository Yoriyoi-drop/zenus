Layer 7: Memory Guardian (MG)
"Banyak serangan modern tidak pernah menyentuh hard disk. Mereka hidup, bersembunyi, dan mati di dalam memori."

Layer 1 menjawab

Siapa kamu?

Layer 2 menjawab

Apa yang boleh kamu akses?

Layer 3 menjawab

Apakah perilakumu normal?

Layer 4 menjawab

Apa tujuanmu?

Layer 5 menjawab

Apa yang akan terjadi?

Layer 6 menjawab

Bagaimana mengubah infrastruktur?

Layer 7 menjawab

Apa yang sebenarnya sedang terjadi di dalam memori komputer saat ini?

Ini adalah lapisan yang menjaga RAM, karena banyak teknik serangan modern beroperasi sepenuhnya di memori dan tidak meninggalkan jejak di penyimpanan.

Filosofi

Antivirus klasik memindai file.

Memory Guardian memantau eksekusi di memori.

Misalnya

Malware

↓

Load

↓

Inject

↓

Execute

↓

Delete File

File hilang.

Tetapi kode masih berjalan di RAM.

Layer 7 tetap dapat melihat aktivitas tersebut.

Tujuan Layer 7

Layer ini bertugas:

Memantau seluruh memori proses.
Mendeteksi injeksi kode.
Mengawasi perubahan struktur memori.
Menilai integritas memori.
Mengirim hasil ke Layer 8.
Memory Architecture

Layer ini membangun peta seluruh memori.

Physical Memory

↓

Kernel Memory

↓

User Memory

↓

Shared Memory

↓

GPU Memory

↓

DMA Region

Seluruh wilayah dipantau.

Memory DNA

Setiap proses memiliki DNA memori.

Misalnya Chrome

Heap

320 MB

Stack

12 MB

DLL

34

Shared Memory

18

Memory Region

81

Jika tiba-tiba

DLL

78

atau muncul wilayah memori yang tidak sesuai,

AI meningkatkan skor risiko.

Memory Map Engine

Setiap proses mempunyai peta.

Misalnya

Text Segment

↓

Data Segment

↓

Heap

↓

Thread Stack

↓

Mapped Library

↓

Shared Memory

Perubahan yang tidak sesuai dengan pola normal dapat menjadi indikator kompromi.

Memory Integrity Verification

Setiap wilayah memori diverifikasi.

Contoh

Executable

↓

Hash

↓

Expected

↓

Current

Jika berbeda

Integrity Score turun.

Injection Detection

Layer ini memantau berbagai teknik injeksi memori.

Misalnya

DLL Injection

Remote Thread

Reflective DLL

Process Hollowing

Module Stomping

Thread Hijacking

APC Injection

Memory Mapping Abuse

Alih-alih hanya mencari pola tetap, sistem mengevaluasi perilaku dan konteks teknik tersebut.

Shellcode Detector

AI mencari ciri khas shellcode.

Misalnya

Executable Memory

↓

NOP Pattern

↓

Decoder Loop

↓

API Resolve

↓

Execution

Namun keputusan tidak didasarkan pada satu indikator saja untuk mengurangi false positive.

Memory Permission Engine

Setiap halaman memori memiliki hak akses.

Misalnya

Read

Write

Execute

Jika halaman yang semula hanya Read/Write tiba-tiba menjadi Executable, sistem mengevaluasi apakah perubahan itu sah.

Runtime Memory Mutation

AI memantau perubahan.

Contoh

Awal

Heap

↓

320 MB

↓

5 detik kemudian

1.8 GB

↓

1 detik kemudian

Executable Memory

↓

Thread Baru

↓

Socket Baru

Gabungan perubahan ini meningkatkan risiko.

Stack Guardian

Layer ini memantau stack.

Misalnya

Return Address

↓

Function Call

↓

Call Depth

↓

Frame Integrity

Jika pola berubah tidak wajar,

AI menandai anomali untuk dianalisis lebih lanjut.

Heap Guardian

Memantau

Allocation

Free

Fragmentation

Reuse

Double Allocation

Abnormal Growth

Perubahan ekstrem bisa menunjukkan adanya eksploitasi atau kerusakan memori.

Shared Memory Guardian

Shared Memory sering dipakai aplikasi normal.

Tetapi juga dapat disalahgunakan.

AI memeriksa

Owner

↓

Permission

↓

Consumer

↓

Lifetime

↓

Data Pattern
Thread Guardian

Setiap thread dipantau.

Misalnya

Thread

↓

Origin

↓

Target

↓

Execution Path

↓

CPU Time

Thread yang muncul dari sumber tidak lazim akan mendapat evaluasi risiko lebih tinggi.

Memory Relationship Graph

Semua wilayah memori dibuat graph.

Process

↓

Heap

↓

Thread

↓

DLL

↓

Shared Memory

↓

Socket

Layer berikutnya dapat melihat hubungan seluruh komponen.

Kernel Memory Guardian

Tidak hanya user memory.

Tetapi juga

Kernel Object

Driver

Page Table

Interrupt Table

Handle Table

Security Object

Perubahan yang tidak sesuai kebijakan akan memicu pemeriksaan lebih lanjut.

GPU Memory Guardian

AI juga memperhatikan GPU.

Karena komputasi modern banyak memanfaatkan GPU.

Misalnya

CUDA

OpenCL

Vulkan

DirectX

Perubahan pola penggunaan GPU yang tidak sesuai karakteristik aplikasi dapat menjadi indikator tambahan.

DMA Guardian

Perangkat tertentu dapat mengakses RAM melalui DMA.

Layer ini memantau

PCIe Device

↓

DMA Request

↓

Target Region

↓

Transfer Pattern

Jika perangkat mencoba mengakses wilayah yang tidak semestinya, tindakan mitigasi dapat diterapkan.

Memory Time Machine

Ini salah satu fitur unik.

Layer ini membuat snapshot ringan metadata memori secara berkala.

Misalnya

T0

↓

T1

↓

T2

↓

T3

Jika terjadi insiden,

AI dapat membandingkan perubahan antar snapshot untuk mempercepat analisis forensik.

Memory Digital Twin

AI memiliki model virtual memori.

Misalnya

Expected Layout

↓

Current Layout

↓

Similarity

98%

Normal.

↓

61%

Perlu investigasi.

↓

25%

Risiko tinggi.

Memory Risk Engine

Setiap indikator memiliki bobot.

Perubahan	Risiko
Heap Growth	+3
Thread Baru	+5
DLL Tidak Lazim	+15
Perubahan Permission Halaman	+20
Injeksi Memori Terindikasi	+40
Shellcode Terindikasi	+50
Kernel Memory Anomaly	+60

Skor akhir dihitung dari kombinasi indikator, bukan satu peristiwa tunggal.

AI Memory Predictor

AI memprediksi evolusi.

Misalnya

Saat ini

Heap

↓

Executable Page

↓

Prediksi

Thread Baru

81%

↓

Network Beacon

74%

↓

Persistence

69%

Prediksi membantu memilih respons yang paling tepat.

Autonomous Memory Response

Jika ancaman meningkat,

Layer 7 dapat memilih respons sesuai kebijakan.

Misalnya

Risiko	Respons
Rendah	Monitoring
Sedang	Pembatasan izin memori
Tinggi	Suspend proses sementara
Sangat Tinggi	Isolasi proses
Kritis	Isolasi perangkat dan eskalasi ke lapisan berikutnya

Keputusan tetap mempertimbangkan tingkat keyakinan AI dan dampak terhadap operasi.

Explainable Memory Analysis

Semua keputusan dapat diaudit.

Contoh

Mengapa proses diberi skor tinggi?

• Memory Layout berubah
• Executable Page baru muncul
• Thread baru berasal dari wilayah memori tidak lazim
• Memory Similarity turun menjadi 48%
• Prediksi tahap berikutnya: Network Beacon (79%)

Administrator dapat melihat alasan teknis tanpa harus menebak.

Output Layer 7

Layer ini menghasilkan Memory Integrity Report.

Report ID             : MEM-10284
Process               : chrome.exe
Memory Similarity     : 95%
Integrity Score       : 97
Injection Risk        : Low
Shellcode Indicator   : None
Thread Health         : Healthy
Kernel Interaction    : Normal
Overall Memory Risk   : 6
Recommended Action    : Continue Monitoring


