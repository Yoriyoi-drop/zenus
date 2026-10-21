ENTRY(entry)

PHDRS {
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
    bss PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0xffffffff80000000 + SIZEOF_HEADERS;

    __kernel_start = .;

    __text_start = .;
    .text : ALIGN(4K) {
        KEEP(*(.limine_reqs))
        /* The KPTI trampoline gets its own page range so the user view can map
         * exactly it and nothing else. It has to come before the .text
         * wildcard, which would otherwise swallow the section and leave the
         * range covering whatever happened to be linked next to it. */
        . = ALIGN(4K);
        __kpti_text_start = .;
        KEEP(*(.text.kpti))
        . = ALIGN(4K);
        __kpti_text_end = .;
        *(.text .text.*)
        . = ALIGN(4K);
    } :text
    __text_end = .;

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.got)
        *(.got.plt)
        . = ALIGN(4K);
    } :rodata

    .data : ALIGN(4K) {
        /* The trampoline's data page. It is mapped into the user view because
         * the entry stub has to read the kernel CR3 before it can switch to it,
         * and before the switch nothing outside the trampoline is reachable. */
        . = ALIGN(4K);
        __kpti_data_start = .;
        KEEP(*(.kpti_data))
        . = ALIGN(4K);
        __kpti_data_end = .;
        *(.data .data.*)
        . = ALIGN(4K);
    } :data

    .bss : ALIGN(4K) {
        *(COMMON)
        *(.bss .bss.*)
        . = ALIGN(4K);
    } :bss

    /* One past the last byte the kernel owns, including `.bss`.
     *
     * Limine knows which pages it loaded the kernel into and leaves them out of
     * the usable memory map — but `.bss` is not in the file Limine reads (it is
     * zeroed at load time), so nothing in the tree had a way to reserve the
     * kernel's own image. These symbols give the frame allocator something to
     * reserve without depending on the bootloader's bookkeeping being right. */
    __kernel_end = .;

    /DISCARD/ : {
        *(.note.gnu.build-id)
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
