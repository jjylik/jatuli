ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5);   /* R + X */
    data PT_LOAD FLAGS(6);   /* R + W */
}

SECTIONS
{
    /* User VA region: L0 slot 1, must equal abi::USER_BASE (0x80_0000_0000).
       Linker scripts can't read Rust, so this literal is the one duplicate of
       that constant; the kernel asserts loaded segments land in this L0 slot. */
    . = 0x8000000000;

    .text   : { *(.text .text.*) }              :text
    .rodata : { *(.rodata .rodata.*) }           :text

    /* Page-align so data/bss start a fresh page -> its own R+W PT_LOAD. */
    . = ALIGN(0x1000);

    .data   : { *(.data .data.*) }               :data
    .bss    : { *(.bss .bss.*) *(COMMON) }       :data
}
