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

    /* EL0 user code, page-aligned and isolated so it can be mapped EL0-RX. */
    . = ALIGN(0x1000);
    __user_start = .;
    .user_text : { KEEP(*(.user_text)) }
    . = ALIGN(0x1000);
    __user_end = .;

    .data   : { *(.data .data.*) }
    .bss    : { *(.bss .bss.*) *(COMMON) }

    /* Reserve a 256 KiB stack; _stack_top is the top (stack grows down). */
    . = ALIGN(16);
    . = . + 0x40000;
    _stack_top = .;

    /* End of everything the kernel image occupies. The frame allocator hands
       out physical frames only from above here, rounded up to a 4 KiB boundary. */
    . = ALIGN(0x1000);
    _kernel_end = .;
}
