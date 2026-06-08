// AArch64 exception vectors with full trap-frame save/restore.
//
// 16 entries, 0x80 bytes apart, table 2 KiB-aligned. Each entry allocates a
// TrapFrame on the stack, saves x0/x1, records its index in x0, and branches to
// the common routine, which saves the rest, calls Rust, restores, and ERETs.
//
// TrapFrame layout (272 bytes): x0..x30 (0..240), ELR (248), SPSR (256), pad (264).

.macro VENTRY index
    .balign 0x80
    sub     sp, sp, #272
    stp     x0, x1, [sp, #0]
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
    VENTRY 4      // Current EL with SPx: Synchronous   <- our EL1 faults/SVC land here
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
    // x0 = kind; x0/x1 originals already saved at [sp,#0]/[sp,#8].
    stp     x2, x3,   [sp, #16]
    stp     x4, x5,   [sp, #32]
    stp     x6, x7,   [sp, #48]
    stp     x8, x9,   [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    str     x30,      [sp, #240]
    mrs     x1, elr_el1
    mrs     x2, spsr_el1
    stp     x1, x2,   [sp, #248]
    mrs     x3, sp_el0              // EL0 stack pointer: per-task, must survive a
    str     x3, [sp, #264]          // switch between two EL0 tasks (x3 already saved)

    mov     x1, sp                  // x1 = &TrapFrame
    bl      exception_dispatch

    ldp     x1, x2,   [sp, #248]
    msr     elr_el1, x1
    msr     spsr_el1, x2
    ldr     x3, [sp, #264]          // restore EL0 stack pointer (x3 not yet reloaded)
    msr     sp_el0, x3
    ldp     x0, x1,   [sp, #0]
    ldp     x2, x3,   [sp, #16]
    ldp     x4, x5,   [sp, #32]
    ldp     x6, x7,   [sp, #48]
    ldp     x8, x9,   [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    ldr     x30,      [sp, #240]
    add     sp, sp, #272
    eret
