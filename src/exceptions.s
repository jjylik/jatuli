// AArch64 exception vector table. 16 entries, 0x80 bytes apart, table 2 KiB-aligned.
// Each entry records its index in x0 and branches to the common dispatcher.

.macro VENTRY index
    .balign 0x80
    mov     x0, #\index
    b       common_exception
.endm

.section .text
.balign 0x800
.global exception_vector_table
exception_vector_table:
    VENTRY 0      // Current EL with SP0: Synchronous
    VENTRY 1      // Current EL with SP0: IRQ
    VENTRY 2      // Current EL with SP0: FIQ
    VENTRY 3      // Current EL with SP0: SError
    VENTRY 4      // Current EL with SPx: Synchronous   <- our EL1 faults land here
    VENTRY 5      // Current EL with SPx: IRQ
    VENTRY 6      // Current EL with SPx: FIQ
    VENTRY 7      // Current EL with SPx: SError
    VENTRY 8      // Lower EL (AArch64): Synchronous
    VENTRY 9      // Lower EL (AArch64): IRQ
    VENTRY 10     // Lower EL (AArch64): FIQ
    VENTRY 11     // Lower EL (AArch64): SError
    VENTRY 12     // Lower EL (AArch32): Synchronous
    VENTRY 13     // Lower EL (AArch32): IRQ
    VENTRY 14     // Lower EL (AArch32): FIQ
    VENTRY 15     // Lower EL (AArch32): SError

common_exception:
    b       exception_dispatch
