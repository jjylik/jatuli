.section .text.boot
.global _start
_start:
    adrp    x9, _stack_top
    add     x9, x9, :lo12:_stack_top
    mov     sp, x9
    bl      kmain
1:  wfe
    b       1b
