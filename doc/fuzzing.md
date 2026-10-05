# Zenus OS Fuzzing Framework

## Overview

Fuzzing framework untuk Zenus OS. Stateful + coverage-guided + snapshot-based. QEMU menangani isolasi, kernel menyediakan instrumentation, fuzz engine mengelola corpus/mutasi/coverage/minimization/reproducer.

## Arsitektur

```
┌───────────────────────────────────────────────────────────────┐
│                         FUZZ ORCHESTRATOR                     │
│                                                               │
│  Corpus Manager ── Mutator ── Scheduler ── Coverage DB       │
│        │                │            │              │          │
│        └────────────────┴────────────┴──────────────┘          │
│                              │                                │
│                    Crash / Hang Classifier                    │
└──────────────────────────────┬────────────────────────────────┘
                               │
                    deterministic test case
                               │
                    ┌──────────▼──────────┐
                    │    QEMU SNAPSHOT    │
                    │                    │
                    │  reset → execute   │
                    │  → collect → reset │
                    └──────────┬─────────┘
                               │
             ┌─────────────────┼──────────────────┐
             │                 │                  │
             ▼                 ▼                  ▼
       Kernel Fuzz       Device Fuzz        Userspace Fuzz
             │                 │                  │
     ┌───────┼───────┐    ┌────┼─────┐      ┌────┼─────┐
     │       │       │    │    │     │      │    │     │
   syscall  MM     IPC   PS/2  disk  net   shell FS   parser
     │       │       │
     └───────┴───────┘
             │
             ▼
       COVERAGE / CRASH
             │
      ┌──────┴──────┐
      ▼             ▼
   new path       failure
      │             │
   corpus       minimize
                    │
                    ▼
              regression test
```

## Domain Fuzzing

| Domain | Target | Prioritas |
|---|---|---|
| Syscall | argument, pointer, length, invalid handle | P0 |
| Memory | page fault, allocator, mapping, heap | P0 |
| Kernel parser | command/config/protocol parser | P0 |
| Driver | keyboard, disk, timer, PCI, serial | P1 |
| Filesystem | path, inode, directory, corruption | P1 |
| Network | packet/protocol handling | P2 |

## Layer Fuzzing

### Layer 0: Pure Rust/C Unit Fuzzing

Sebelum QEMU. Target: lexer, parser, allocator, bitmap, path parser, ELF loader, filesystem parser, protocol parser, syscall decoder.

```
fuzz_input → parse() → valid → AST
                     → invalid → error
```

Tujuan: cari bug deterministik dengan sangat murah.

### Layer 1: Syscall Fuzzing (Core)

ABI fuzz input:

```rust
SYSCALL_CASE {
    syscall_id: u8,
    arg0..arg5: u64,
    memory_regions: bytes,
}
```

Mutasi syscall_id: 0, max, max+1, random.
Mutasi pointer: NULL, kernel address, unmapped, misaligned, userspace, boundary.
Mutasi length: 0, 1, PAGE_SIZE-1, PAGE_SIZE, PAGE_SIZE+1, UINT_MAX.

Yang dicari: double free, use-after-free, invalid mapping, privilege escalation, kernel memory leak, deadlock, infinite loop, invalid state transition.

### Layer 2: Memory Manager Fuzzer

```
ALLOC → alloc(size), free(ptr), realloc(ptr,size), map(addr,size), unmap(addr,size), protect(addr,size,flags)
```

Sequence fuzzing: alloc(4096) → alloc(8192) → free(A) → map(...) → free(A) → unmap(...) → access(A)

State machine:
```
UNALLOCATED → alloc → ALLOCATED → free → FREED/INVALID
                    → realloc → ALLOCATED
```

### Layer 3: Driver Fuzzing

```
FUZZER → Virtual Device → QEMU → IRQ → Zenus Driver → Input Buffer → Shell
```

Keyboard: scancode → IRQ1 → keyboard handler → ring buffer → decoder → key event → shell.

Mutasi: normal key, release, press/release mismatch, extended key, invalid scancode, rapid sequence, buffer overflow/empty/full.

### Layer 4: Filesystem Fuzzing

Dua jalur:
- API fuzzing: create/open/read, write/rename, unlink, mount
- Image fuzzing: corrupted image, invalid inode, bad directory, malformed metadata

Sequence: create("/a") → write("/a", data) → rename("/a", "/b") → unlink("/b") → open("/b")

Target bug: out-of-bounds, integer overflow, path traversal, dangling inode, double free, corrupted metadata, infinite traversal.

### Layer 5: Shell Fuzzing

```
zenus$ <FUZZ>
```

Corpus awal: help, ls, cd /, pwd, echo test, cat file, mkdir test, rm test.

Mutasi: "", " ", "////", "../../..", "$(...)", "; ; ;", "&&&&", "||||", "aaaa....aaaa".

### Layer 6: Scheduler Fuzzing

Thread A dan Thread B concurrent syscall. Fuzzer kontrol: thread creation, context switch, interrupt timing, lock acquisition, sleep/wakeup, IPC, scheduler quantum.

Target: deadlock, race, lost wakeup, priority inversion, double unlock, scheduler corruption.

## Coverage

Tingkat coverage:
- Edge coverage
- Block coverage
- Syscall coverage

```
case #001 → 13 edges
case #002 → 21 edges
case #003 → 21 edges  ← discard
case #004 → 27 edges  ← KEEP
```

## QEMU Snapshot

Boot sekali → snapshot → loop { restore → execute → collect }.

## Struktur Direktori

```
zenus-fuzz/
├── corpus/
│   ├── syscall/
│   ├── memory/
│   ├── keyboard/
│   ├── filesystem/
│   ├── shell/
│   └── network/
├── crashes/
│   ├── syscall/
│   ├── memory/
│   ├── keyboard/
│   └── filesystem/
├── hangs/
├── minimized/
├── snapshots/
├── coverage/
└── metadata/
    ├── corpus.db
    ├── coverage.db
    └── crashes.db
```

## Crash Pipeline

```
CRASH → Capture registers → Capture stack → Capture coverage → Deduplicate
  → duplicate: ignore
  → unique: minimize → regression test
```

Format crash ID: `ZENUS-FUZZ-000127`

## Mode Operasi

| Mode | Cases (default) | Kapan | Perintah |
|---|---|---|---|
| smoke | 2 000 | Setiap commit | `make fuzz-smoke` |
| coverage | 50 000 | Cari jalur baru | `make fuzz-coverage` |
| stress | — | Overnight | belum diimplementasikan |
| regression | semua crash tercatat | Setiap build | `make fuzz-regression` (belum berfungsi — lihat batasan) |

Default-nya ada di `zenus_fuzz::Mode::default_cases()`.

### Batasan yang diketahui

Jujur soal apa yang benar-benar bekerja:

- **Minimisasi belum reproduktif.** `minimizer::minimize` memakai predikat
  yang tidak bisa menjalankan input (hanya menolak hasil kosong), jadi hasilnya
  tidak berguna untuk debugging. Yang bisa dipakai: `minimize_with`, dengan
  oracle yang sebenarnya. Ada test yang mengunci perilaku ini.
- **Konten minimalisasi tidak persisten.** Corpus dan crash disimpan di memori
  kernel saja; `build/fuzz/fuzz.log` adalah satu-satunya jejak setelah VM mati.
- **Fault containment hanya di BSP.** `fuzz_guard` menyimpan satu pasangan
  (rsp, rip), jadi hanya CPU yang memasang checkpoint yang bisa memulih; AP
  diambil jalur panic biasa.
- **Snapshot QEMU belum dipakai.** `snapshot.rs` masih kerangka; yang bekerja
  adalah containment in-kernel, bukan save/restore dari luar.
- **Coverage berbasis edge counter**, bukan instrumentasi LLVM: nilainya kasar, dan
  "jalur baru" berarti counter edge yang belum pernah naik.
- **Mode regression kosong.** Log crash hanya ada di memori kernel dan
  `zenus_fuzz::init()` memanggil `crash::clear()`, jadi saat boot
  `get_crash_count()` selalu 0 dan loop tidak pernah jalan. Runner sekarang
  mencetak `[FUZZ] NO-CORPUS` dan keluar dengan kode 2 ("tidak bisa menarik
  kesimpulan"), bukan melaporkan `[FUZZ] EXIT code=0` untuk run yang tidak
  menguji apa pun. Butuh corpus crash di disk lebih dulu.

## Urutan Implementasi

1. Phase 1: syscall + parser
2. Phase 2: memory allocator/MM
3. Phase 3: keyboard + IRQ + timer
4. Phase 4: filesystem
5. Phase 5: scheduler/concurrency
6. Phase 6: full-system fuzzing

## Integrasi Build

```bash
# Build dengan fuzzing support
make build-fuzz

# Run fuzzing smoke test
make fuzz-smoke

# Run fuzzing coverage
make fuzz-coverage

# Run regression test
make fuzz-regression
```

## Bug Ditemukan

### BUG-001: Syscall Table Overflow

**Lokasi**: `crates/zenus-syscall/src/syscall.rs:232`

**Masalah**: Syscall table berukuran 256 entry. Beberapa syscall ID konflik (duplikat):
- `SYS_DUP3 = 24` dan `SYS_SCHED_YIELD = 24`
- `SYS_GETPPID = 39` dan `SYS_GETPID = 39`
- `SYS_SOCKET = 41` dan `SYS_NICE = 41`
- `SYS_ACCEPT = 43` dan `SYS_SCHED_SETSCHEDULER = 43`
- `SYS_SENDTO = 44` dan `SYS_SCHED_GETPARAM = 44`
- `SYS_RECVFROM = 45` dan `SYS_SCHED_SETPARAM = 45`
- `SYS_GETSOCKOPT = 55` dan `SYS_GETRUSAGE = 55`
- `SYS_FORK = 57` dan `SYS_EVENTFD2 = 57`
- `SYS_CLONE = 56` dan `SYS_TIMES = 56`

**Dampak**: Syscall yang salah dipanggil. Potensi security vulnerability.

**Fix**: Assign unique syscall IDs untuk setiap entry.

### BUG-002: Integer Overflow di sys_read

**Lokasi**: `crates/zenus-syscall/src/syscall.rs:436-443`

**Masalah**: `count` tidak divalidasi sebelum `resize`. Jika `count > 1048576` return -1, tapi `count` antara 1MB-8MB tetap di-alloc.

**Dampak**: Memory exhaustion.

**Fix**: Turunkan limit atau gunakan checked allocation.

### BUG-003: Use-After-Free di sys_shmat

**Lokasi**: `crates/zenus-syscall/src/syscall.rs:2211-2244`

**Masalah**: `table.segments[shmid as usize]` diakses tanpa bounds check. Jika `shmid >= MAX_SHMSEG`, out-of-bounds read.

**Dampak**: Kernel crash atau information leak.

**Fix**: Tambah bounds check.

### BUG-004: Buffer Overflow di sys_readdir

**Lokasi**: `crates/zenus-syscall/src/syscall.rs:573-612`

**Masalah**: `buf_size` tidak divalidasi dengan benar. Loop `for entry in entries` bisa menulis melebihi `buf_size`.

**Dampak**: Kernel stack corruption.

**Fix**: Validasi `buf_size` sebelum loop.

### BUG-005: Missing Validation di sys_mount

**Lokasi**: `crates/zenus-syscall/src/syscall.rs:2014-2086`

**Masalah**: `source` dan `fstype` tidak digunakan setelah di-parse. `target` di-leak via `leak_string` tanpa validasi.

**Dampak**: Memory leak.

**Fix**: Validasi source dan fstype.

## Kesimpulan

Fuzzing Zenus harus stateful + coverage-guided + snapshot-based, bukan random-input-only. QEMU menangani isolasi, kernel menyediakan instrumentation, fuzz engine mengelola corpus/mutasi/coverage/minimization/reproducer.
