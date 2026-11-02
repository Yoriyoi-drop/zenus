<!--
DESIGN DOCUMENT — a proposal for the `.zns` executable format and `.znl`
libraries. None of this is implemented: the kernel loads ELF, not `.zns`. Keep
this as a design record, not as a description of the system.
-->

# Zenus OS — Format File Native & Arsitektur Desain

Kalau format `.zn*` menjadi **fondasi ekosistem file resmi Zenus OS**, format-format ini dirancang untuk mencakup seluruh kebutuhan sistem: executable, library, driver, modul kernel, paket aplikasi, disk image, firmware, arsip, konfigurasi, hingga audit log.

Dokumen desain lengkap telah dipecah menjadi file per bagian di dalam direktori [`docs/design/`](file:///home/whale-d/zenus/docs/design/):

## Daftar Isi Bagian Format

1. [`.zns` — Zenus Native Executable](file:///home/whale-d/zenus/docs/design/01-zns-native-executable.md)
   - Format executable aplikasi Zenus OS pengganti ELF/PE untuk user-space.
2. [`.znl` — Zenus Native Library](file:///home/whale-d/zenus/docs/design/02-znl-native-library.md)
   - Dynamic shared library format dengan shared memory dan plugin system.
3. [`.znd` — Zenus Native Driver](file:///home/whale-d/zenus/docs/design/03-znd-native-driver.md)
   - Driver hardware mandiri dengan Hardware ID, DMA config, dan isolation.
4. [`.znm` — Zenus Native Module](file:///home/whale-d/zenus/docs/design/04-znm-native-module.md)
   - Kernel extension module dengan permission table dan granular dependencies.
5. [`.znpkg` — Zenus Native Package](file:///home/whale-d/zenus/docs/design/05-znpkg-native-package.md)
   - Paket distribusi perangkat lunak terpadu untuk ekosistem Zenus.
6. [`.zni` — Zenus Native Image](file:///home/whale-d/zenus/docs/design/06-zni-native-image.md)
   - Sistem image bootable, partition layout, dan snapshot deployment.
7. [`.znf` — Zenus Native Firmware](file:///home/whale-d/zenus/docs/design/07-znf-native-firmware.md)
   - Firmware image, calibration data, device recovery, dan update mechanism.
8. [`.zna` — Zenus Native Archive](file:///home/whale-d/zenus/docs/design/08-zna-native-archive.md)
   - Container arsip berkas berkinerja tinggi dengan metadata dan checksum terintegrasi.
9. [`.znc` — Zenus Native Configuration](file:///home/whale-d/zenus/docs/design/09-znc-native-configuration.md)
   - Konfigurasi terstruktur (Text & Binary) dengan validation schema, inheritance, dan live reload.
10. [`.znlog` — Zenus Native Log](file:///home/whale-d/zenus/docs/design/10-znlog-native-log.md)
    - Structured binary/text audit log dengan rotation, encryption, dan verifikasi tanda tangan digital.
