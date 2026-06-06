ENTRY(_start)

SECTIONS
{
    /* QEMU loads an AArch64 -kernel image here (RAM base is 0x40000000). */
    . = 0x40080000;

    .text : {
        KEEP(*(.text.boot))   /* _start must come first */
        *(.text .text.*)
    }

    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : { *(.bss .bss.*) *(COMMON) }

    /* Reserve a 256 KiB stack; _stack_top is the top (stack grows down). */
    . = ALIGN(16);
    . = . + 0x40000;
    _stack_top = .;
}
