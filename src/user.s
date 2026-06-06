// Minimal EL0 user routine, placed in its own page so it can be mapped EL0-RX.
// Position-independent: runs correctly at whatever virtual address it is mapped to.

.section .user_text, "ax"
.global user_entry
user_entry:
    // Good syscall: SYS_PRINT(message, length).
    adr     x0, user_msg
    mov     x1, #(user_msg_end - user_msg)
    mov     x8, #2                      // SYS_PRINT
    svc     #0

    // Bad syscall: SYS_PRINT(<kernel address>, 16) -> the kernel must reject it.
    movz    x0, #0x0000
    movk    x0, #0x4008, lsl #16        // x0 = 0x4008_0000 (a kernel address)
    mov     x1, #16
    mov     x8, #2                      // SYS_PRINT
    svc     #0

1:  wfe
    b       1b

user_msg:
    .ascii  "Hello from EL0!\n"
user_msg_end:
