ENTRY(_start)

PHDRS {
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0x400000;

    .text : ALIGN(4K) {
        *(.text .text.*)
        . = ALIGN(4K);
    } :text

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
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
    } :data

    /DISCARD/ : {
        *(.note*)
        *(.comment*)
        *(.eh_frame*)
        *(.got)
        *(.got.plt)
    }
}
