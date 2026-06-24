ENTRY(entry)

PHDRS {
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
    bss PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0xffffffff80000000 + SIZEOF_HEADERS;

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

    /DISCARD/ : {
        *(.note.gnu.build-id)
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
