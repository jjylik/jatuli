.section .text.boot
.global _start
_start:
    // AArch64 resets with FP/SIMD access trapped (CPACR_EL1.FPEN = 0b00).
    // The compiler may emit NEON/FP instructions, so enable full FP/SIMD
    // access (FPEN = 0b11) before running any Rust code. ISB so the change
    // takes effect before the next instructions.
    mov     x0, #(3 << 20)
    msr     cpacr_el1, x0
    isb

    // Set up the stack (grows down from _stack_top) and jump to Rust.
    adrp    x9, _stack_top
    add     x9, x9, :lo12:_stack_top
    mov     sp, x9
    bl      kmain
1:  wfe
    b       1b
