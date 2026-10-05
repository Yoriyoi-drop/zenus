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
