ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5);   /* R + X */
    data PT_LOAD FLAGS(6);   /* R + W */
}

SECTIONS
{
    /* User VA region, matching the kernel's user window (Phase 9). */
    . = 0x200000000;

    .text   : { *(.text .text.*) }              :text
    .rodata : { *(.rodata .rodata.*) }           :text

    /* Page-align so data/bss start a fresh page -> its own R+W PT_LOAD. */
    . = ALIGN(0x1000);

    .data   : { *(.data .data.*) }               :data
    .bss    : { *(.bss .bss.*) *(COMMON) }       :data
}
