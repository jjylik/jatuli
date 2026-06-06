// Cooperative context switch and new-thread trampoline.

.section .text

// context_switch(prev_sp: *mut usize /*x0*/, next_sp: usize /*x1*/)
//
// Save the callee-saved registers (x19..x30) on the current stack, store the
// resulting SP into *prev_sp, switch to next_sp, restore x19..x30, and `ret`
// into the next thread's saved LR.
.global context_switch
context_switch:
    stp     x19, x20, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x29, x30, [sp, #-16]!
    mov     x9, sp
    str     x9, [x0]            // *prev_sp = sp
    mov     sp, x1              // sp = next_sp
    ldp     x29, x30, [sp], #16
    ldp     x27, x28, [sp], #16
    ldp     x25, x26, [sp], #16
    ldp     x23, x24, [sp], #16
    ldp     x21, x22, [sp], #16
    ldp     x19, x20, [sp], #16
    ret

// First-run trampoline for a freshly spawned thread. The fabricated context puts
// the entry function in x19, its argument in x20, and this routine's address in
// the saved x30, so the first context_switch returns here.
.global task_trampoline
task_trampoline:
    msr     daifclr, #2         // enable IRQs so the new thread is preemptible
    mov     x0, x20             // arg
    blr     x19                 // entry(arg)
    b       task_exit           // entry returned -> exit (never returns)
