# jos — Knowledge Self-Check

A quiz over what this kernel actually does. Each question ties an OS concept to a concrete
detail in the code. Answer in your head (or out loud), then expand the answer to check.

Files referenced live in `kernel/src/`, `user/src/`, and `abi/src/`. Phases map to the
specs in `docs/superpowers/specs/`.

---

## 1. Boot & toolchain

**1.1** The kernel is linked to load at `0x4008_0000`. Where does that address come from, and
how does QEMU know to start executing there (we pass no bootloader)?

<details><summary>Answer</summary>
`0x4008_0000` is where QEMU's `virt` machine places an AArch64 `-kernel` image (RAM base is
`0x4000_0000`). We produce an **ELF**, so QEMU loads it by its program headers and jumps to
the ELF entry point — which our `linker.ld` sets to `_start` via `ENTRY(_start)`, placed
first in `.text` at `0x4008_0000`. No bootloader needed.
</details>

**1.2** Our very first "Hello World" hung with no output. What was the root cause, and how did
we diagnose it?

<details><summary>Answer</summary>
AArch64 **resets with FP/SIMD access trapped** (`CPACR_EL1.FPEN = 0`). The compiler
auto-vectorized `write_str`'s byte loop into a NEON instruction, which trapped (`ESR.EC =
0x7`) before any output. With no exception vector table yet, the fault looped forever. We
diagnosed it with `qemu-system-aarch64 ... -d in_asm,int` — the log showed execution
reaching `kmain`, then "Taking exception ... ESR 0x7" (SIMD/FP trap).
</details>

**1.3** What does `boot.s` do to fix that, and why must it run before any Rust code?

<details><summary>Answer</summary>
It writes `CPACR_EL1 = 3 << 20` (FPEN = 0b11, stop trapping FP/SIMD) and `isb`, before
`bl kmain`. It must precede Rust because the compiler may emit NEON anywhere; if FP isn't
enabled first, the first such instruction faults.
</details>

**1.4** `boot.s` sets `sp` from `_stack_top` using `adrp`/`add` rather than `ldr =sym`. What is
`adrp`/`add` doing conceptually?

<details><summary>Answer</summary>
Computing a symbol's address **PC-relatively**: `adrp` loads the 4 KiB page base relative to
the current PC, `add :lo12:` adds the low 12 bits. It's the canonical position-independent
way to materialize an address on AArch64.
</details>

---

## 2. UART & the freestanding runtime

**2.1** `write_byte` spins while `UARTFR & (1<<5)` is set before writing `UARTDR`. What is that
bit, and what is the MMIO base it's reading?

<details><summary>Answer</summary>
The PL011 UART is at `0x0900_0000`. `UARTFR` (offset `0x18`) bit 5 is **TXFF** (transmit FIFO
full); we wait until there's room, then write the byte to `UARTDR` (offset `0x00`).
</details>

**2.2** We had to implement `memcpy`/`memset`/`memmove`/`memcmp` ourselves in `mem.rs`. Why
were they needed only once we used `Vec`/`String`, and not in phase 1?

<details><summary>Answer</summary>
The compiler lowers bulk memory operations (slice copies, `Vec` growth, zero-init) into
calls to symbols named `memcpy`/`memset`/etc. Phase 1 never triggered any. With no `libc`
and the `compiler_builtins` `mem` feature off on this bare-metal target, those symbols are
undefined unless we provide them.
</details>

**2.3** Why must `mem.rs`'s `memcpy` use a manual byte loop instead of `core::ptr::copy_*`?

<details><summary>Answer</summary>
`core::ptr::copy*` itself lowers to a `memcpy` call — using it inside `memcpy` would recurse
infinitely. A hand-written byte loop is the actual primitive.
</details>

**2.4** These functions are `#[no_mangle] extern "C"`. Are they C code? What do those two
attributes do?

<details><summary>Answer</summary>
No — they're Rust. `extern "C"` selects the C **calling convention/ABI** (not the language);
`#[no_mangle]` exports the symbol under its literal name (`memcpy`) so the linker/compiler
can resolve calls to it.
</details>

---

## 3. Heap allocator

**3.1** The bump allocator is wrapped in `Locked<BumpAllocator>`. Why is the lock necessary for
something registered as `#[global_allocator]`?

<details><summary>Answer</summary>
`GlobalAlloc`'s methods take `&self`, so the allocator needs **interior mutability**, and a
`static` must be `Sync`. `Locked` provides both — a spinlock (`AtomicBool`) guarding an
`UnsafeCell`. On our single core it never actually contends, but it's the smallest sound
primitive.
</details>

**3.2** What does the bump allocator do on `dealloc`, and what's the consequence?

<details><summary>Answer</summary>
It decrements an allocation counter and only resets `next` to the start when the count hits
zero. Consequence: freed memory mid-stream isn't reused until *everything* is freed — the
known limitation of a bump allocator (a free-list would fix it).
</details>

**3.3** When a `Vec` grows, `memcpy` runs. Does `memcpy` call the allocator? Walk the sequence.

<details><summary>Answer</summary>
No. `Vec::push` (when full) → `realloc` → **alloc** a new block (the allocator) → **memcpy**
the old contents into it (no allocator, just byte copies) → **dealloc** the old block. The
allocator reserves; `memcpy` fills already-reserved memory. They're siblings, not caller/callee.
</details>

---

## 4. Frame allocator

**4.1** Frames are tracked with an *intrusive* free-list. Where is the "next" pointer stored,
and why does that cost no extra memory?

<details><summary>Answer</summary>
In the **first 8 bytes of each free frame itself**. A free frame is empty anyway, so it
holds the list link; the instant it's allocated, real data overwrites the link. Zero
metadata array.
</details>

**4.2** `head == 0` means the pool is empty. Why is `0` a safe sentinel?

<details><summary>Answer</summary>
Usable RAM starts at `0x4000_0000`, so address `0` is never a valid frame — it can't collide
with a real free-frame address.
</details>

**4.3** The pool starts at `_kernel_end` (a linker symbol), not at RAM base. Why?

<details><summary>Answer</summary>
The kernel image, its `.bss`, and the stack occupy the bottom of RAM. Handing out frames
from above `_kernel_end` guarantees the allocator never returns memory that overlaps the
running kernel.
</details>

**4.4** Reading the linker symbol used `addr_of!(_kernel_end)` with no `unsafe`. Why is taking
the address safe even though it's an `extern static`?

<details><summary>Answer</summary>
`addr_of!` computes an address without *reading* the static — no load happens, so it's safe.
Only dereferencing/reading an extern static is `unsafe`. (The compiler caught our needless
`unsafe` block.)
</details>

---

## 5. MMU & paging

**5.1** Name the four registers we program to turn on translation, and which one flips it live.

<details><summary>Answer</summary>
`MAIR_EL1` (memory attribute table), `TCR_EL1` (granule + address-space sizes), `TTBR0_EL1`
(top-level table address), then `SCTLR_EL1.M = 1` flips the MMU on. (We also set C/I for caches.)
</details>

**5.2** What does `MAIR_EL1 = 0x04FF` encode, and why do we need two entries?

<details><summary>Answer</summary>
Two attribute indices: index 0 = `0xFF` (Normal write-back cacheable, for RAM), index 1 =
`0x04` (Device-nGnRE, for MMIO like the UART/GIC). Descriptors carry an `AttrIndx` selecting
one — RAM is cacheable, device memory must not be.
</details>

**5.3** We map RAM with 2 MiB blocks and the device region with a 1 GiB block, through L0→L1→L2.
What descriptor bits distinguish a *block* from a *table*, and at which levels can blocks appear?

<details><summary>Answer</summary>
Bits `[1:0]`: `0b01` = block, `0b11` = table (and `0b11` = page at L3). With a 4 KiB granule,
block descriptors are valid at **L1** (1 GiB) and **L2** (2 MiB); L3 has pages, not blocks.
</details>

**5.4** Because it's an identity map, the kernel kept running unchanged when the MMU came on.
Why didn't it need relocating, and what does `TTBR0_EL1` point at?

<details><summary>Answer</summary>
Identity map means virtual == physical, so every address (code at `0x4008…`, UART at
`0x0900…`) still resolves to itself — nothing moves. `TTBR0_EL1` points at the physical
address of the top-level (L0) table.
</details>

**5.5** `map_page` writes an L3 descriptor with low bits `0b11`. But `0b11` also means "table"
at L0–L2. Why isn't this ambiguous?

<details><summary>Answer</summary>
The meaning is **level-dependent**. The walker knows which level it's at: at L0–L2, `0b11`
means "table descriptor, continue walking"; at L3 (the last level), `0b11` means "page". So
the same bits mean "page" only because it's the final level.
</details>

**5.6** Why did `map_page` invalidate the TLB (`tlbi`) after writing the descriptor, and what's
the `dsb` before it for?

<details><summary>Answer</summary>
`dsb ishst` ensures the descriptor write has actually reached memory before the walker could
use it; `tlbi vaae1` drops any stale cached translation for that VA; the trailing `dsb ish` +
`isb` make the invalidation take effect before continuing. Without it, the CPU could keep
using an old (or absent) translation.
</details>

---

## 6. Frame-backed heap

**6.1** The heap lives at virtual `0x1_0000_0000` but its bytes are in scattered physical
frames. What makes them appear contiguous, and why couldn't the bump allocator just consume
frames directly?

<details><summary>Answer</summary>
`map_page` maps scattered physical frames into a **contiguous virtual window**, so the bump
allocator sees one contiguous arena. It can't use frames directly because the intrusive
free-list hands them out in no particular order (not contiguous), and a bump allocator
requires a contiguous range.
</details>

**6.2** We chose the heap VA `0x1_0000_0000` deliberately. Why not somewhere in the
identity-mapped 0–2 GiB region?

<details><summary>Answer</summary>
That region is mapped with 2 MiB blocks (EL1-only). Reusing it would require splitting a
block to set 4 KiB-page permissions. A fresh VA range (`L1[4]`) gets its own L2/L3 tables
with no conflict — and makes the heap genuinely virtual ≠ physical.
</details>

---

## 7. Exceptions

**7.1** When an exception fires, hardware sets four registers. Name them and what each holds.

<details><summary>Answer</summary>
`ELR_EL1` = return address (faulting/next instruction); `SPSR_EL1` = saved PSTATE (incl.
the EL we came from); `ESR_EL1` = syndrome (cause; `EC = ESR[31:26]`); `FAR_EL1` = faulting
virtual address (for aborts).
</details>

**7.2** The vector table has 16 entries. A data abort at EL1 lands in vector 4; an SVC from EL0
lands in vector 8. Why the difference?

<details><summary>Answer</summary>
The 16 vectors are 4 groups of 4 (sync/IRQ/FIQ/SError): current-EL/SP0, current-EL/SPx,
lower-EL/AArch64, lower-EL/AArch32. We run at EL1 with SP_EL1 → **current-EL/SPx** (vectors
4–7), so EL1 sync = vector 4. An exception from EL0 → **lower-EL/AArch64** (vectors 8–11),
so EL0 sync = vector 8.
</details>

**7.3** How does the dispatcher tell an IRQ from a synchronous exception, and a syscall from a
fault?

<details><summary>Answer</summary>
By `kind % 4`: `== 1` is the IRQ entry, `== 0` is synchronous. For synchronous it reads
`ESR.EC`: `0x15` = SVC (→ syscall), anything else → report-and-halt (fault).
</details>

**7.4** Before phase 6, what was the symptom of *any* fault, and why?

<details><summary>Answer</summary>
A silent hang / infinite exception loop: `VBAR_EL1` was unset, so a fault jumped to a garbage
vector address, faulting again forever. Installing the vector table turned faults into
printed diagnostics.
</details>

---

## 8. Trap frame & syscalls

**8.1** Each vector stub does `sub sp,#272; stp x0,x1,[sp]; mov x0,#index` *before* branching to
common code. Why save `x0/x1` first?

<details><summary>Answer</summary>
The stub needs a scratch register to record the vector index (`mov x0,#index`), which would
destroy the interrupted code's `x0`. Saving `x0`/`x1` to the trap frame first preserves them;
the rest are saved in the common routine.
</details>

**8.2** The trap frame is 272 bytes for x0–x30 (31), ELR, SPSR — that's 33×8 = 264. Why 272?

<details><summary>Answer</summary>
16-byte stack alignment. 264 isn't a multiple of 16, so we pad to 272 (one extra slot) to
keep `SP` 16-aligned, which AArch64 requires at function calls.
</details>

**8.3** A syscall returns a value to the caller. Where does the handler put it, and how does it
reach EL0/EL1 code after `ERET`?

<details><summary>Answer</summary>
The handler writes `frame.x[0]`. On the way out, the stub reloads `x0` from the frame, so
after `ERET` the caller's `x0` holds the return value (Linux-like ABI: return in `x0`).
</details>

**8.4** The `syscall` inline-asm wrapper lists only `x8`, `x0`, `x1`. Why doesn't it need to
mark `x2`–`x7` (or `x9`+) as clobbered?

<details><summary>Answer</summary>
The trap stub saves and restores **all** of x0–x30 around the handler, so every register is
preserved across the `svc`. The compiler can safely assume unlisted registers survive.
</details>

---

## 9. Interrupts (GICv3 + timer)

**9.1** GICv3's CPU interface is reached through `ICC_*` system registers, not MMIO. What did we
set *first*, and why?

<details><summary>Answer</summary>
`ICC_SRE_EL1.SRE = 1` — enable the system-register interface. Until that's set, the other
`ICC_*` registers (PMR, IGRPEN1, IAR1, EOIR1) aren't usable.
</details>

**9.2** Distributor vs redistributor: the timer is INTID 30. Is it a PPI or an SPI, and where
did we enable it?

<details><summary>Answer</summary>
A **PPI** (per-CPU, INTID 16–31). PPIs/SGIs live in the per-CPU **redistributor**, so we
enabled it via `GICR_ISENABLER0` (in the SGI frame), not the distributor. (The distributor
handles global SPIs and overall control.) We also had to *wake* the redistributor
(`GICR_WAKER`).
</details>

**9.3** `on_tick` both counts the tick and reloads `CNTP_TVAL_EL0`. Why must the reload happen
*before* the GIC EOI?

<details><summary>Answer</summary>
Writing `CNTP_TVAL_EL0` re-arms the timer and clears its pending condition (the interrupt
line de-asserts). If we EOI'd first, the timer line would still be asserted and the interrupt
would immediately re-fire.
</details>

**9.4** The demo sleeps with `wfi` and the timer wakes it. If we'd left `PSTATE.I` masked, what
would happen?

<details><summary>Answer</summary>
The interrupt would never be *taken* (delivery is masked), so the handler never runs, `TICKS`
never advances, and the `while ticks() < 5` loop spins forever — a hang. We unmask with
`msr daifclr, #2`.
</details>

---

## 10. EL0 userspace

*Note: 10.4–10.5 describe the Phase 9 design (a `user.s` blob baked into the kernel image).
Phase 10 replaced it with a separately compiled ELF — see section 14.*

**10.1** Which AP encoding makes a page EL0 read/write, and which bit makes a page executable at
EL0? What did `PAGE_USER_RX` use?

<details><summary>Answer</summary>
`AP = 0b01` → EL0 (and EL1) read/write. Executable at EL0 requires `UXN = 0`. `PAGE_USER_RX`
used `AP = 0b11` (EL0 read-only — code shouldn't be writable, W^X) with `UXN = 0` (executable)
and `PXN = 1` (kernel won't execute it).
</details>

**10.2** To drop to EL0 we `ERET` after setting three registers. Which three, and what mode
value goes in SPSR?

<details><summary>Answer</summary>
`ELR_EL1` = user entry VA, `SP_EL0` = user stack top, `SPSR_EL1` = target state. SPSR mode
field `M[3:0] = 0b0000` (EL0t); we used `0` overall (EL0t, interrupts unmasked). Then `eret`.
</details>

**10.3** A user passed pointer `0x40080000` and the kernel rejected it, but the kernel's own
syscalls aren't validated. How does the kernel know to validate this one?

<details><summary>Answer</summary>
`from_user = kind >= 8`: an EL0 `svc` arrives via the lower-EL vectors (8–11), an EL1 `svc`
via vector 4. Only `from_user` calls run `is_user_range`, which checks the pointer lies in the
mapped user code/stack window. `0x40080000` (kernel) fails, so it's rejected.
</details>

**10.4** The user routine is mapped at `0x2_0000_0000` but linked into the kernel image at
`0x4008…`. Why must it be position-independent, and what would break otherwise?

<details><summary>Answer</summary>
It executes at the user VA (`0x2_0000_0000`), not its linked address. PC-relative code (`adr`)
computes correct runtime addresses; absolute references would point back to the linked
kernel address — EL1-only memory — and fault. (This is exactly why a Rust user routine needs
PIC + co-located data.)
</details>

**10.5** The user code page exists at *two* virtual addresses. What are they and how do their
permissions differ?

<details><summary>Answer</summary>
At its **identity address** (`0x4008…`, inside the 2 MiB RAM block) it's EL1-only; at
`0x2_0000_0000` it's mapped EL0-RX. Same physical frame, two mappings, different permissions —
the kernel can't be reached from EL0, and EL0 runs only via the user mapping.
</details>

---

## 11. Cooperative scheduling

**11.1** A "thread" is made of two things in this kernel. What are they, and which single
value serves as the thread's handle?

<details><summary>Answer</summary>
A **stack** and a **saved register context**. The handle is the saved **stack pointer**
(`Task.sp`) — the saved context lives on that stack at/above the SP, so the SP alone is
enough to resume the thread.
</details>

**11.2** `context_switch` saves only x19–x30 (callee-saved) + SP, not all 31 GPRs. Why is
that sufficient for cooperative switching?

<details><summary>Answer</summary>
`yield_now` is a normal function call, and per the AAPCS the caller-saved registers
(x0–x18) are already considered clobbered across any call — the compiler has spilled
anything it still needs. So only the callee-saved registers, LR, and SP must be preserved to
resume correctly. (Preemption is different — there the *full* trap frame is saved, because it
interrupts at an arbitrary instruction.)
</details>

**11.3** A brand-new thread has no saved context. How does `spawn` make the first switch land
in the thread's entry function?

<details><summary>Answer</summary>
It **fabricates** a context block on the new stack matching what `context_switch` restores:
`x30 = task_trampoline`, `x19 = entry`, `x20 = arg`, the rest 0. The first `context_switch`
restores those and `ret`s into `task_trampoline`, which does `entry(arg)` (and `b task_exit`
if it returns).
</details>

**11.4** `yield_now` extracts the next SP under the scheduler lock, then **drops the lock
before** calling `context_switch`. Why must the lock not be held across the switch?

<details><summary>Answer</summary>
The thread you switch *to* (and any later resume) would try to take the same lock → deadlock;
and a guard's lifetime can't sensibly span a stack switch. The rule is general: never hold a
lock across a context switch. Each thread re-acquires the lock fresh when it runs.
</details>

**11.5** `kmain` is registered as task 0 with `sp = 0`, and that 0 is never used. Why doesn't
its context need fabricating?

<details><summary>Answer</summary>
`kmain` is the *currently running* context. Its real SP is captured on the first switch
**out** of it (written through `prev_sp`). We only ever switch *to* task 0 after having
switched *from* it once, so the placeholder 0 is overwritten before it's used.
</details>

---

## 12. Preemptive scheduling & sleep

**12.1** Cooperative `yield_now` and the timer IRQ both funnel into the same `schedule()`.
What does that unification buy, and what does routing the timer through it add?

<details><summary>Answer</summary>
One switch mechanism for both voluntary and forced switches (less code, one thing to get
right). Routing the timer through it makes scheduling **preemptive**: a thread that never
calls `yield`/`sleep` still gets switched away on a tick.
</details>

**12.2** Once the timer IRQ also touches scheduler state, a tick firing mid-`yield_now` could
corrupt it. What's the single-core fix, and why does it make the spinlock effectively safe?

<details><summary>Answer</summary>
Run scheduler critical sections with **IRQs disabled** (`irq::disable`/`restore`). On one
core, IRQs-off means the timer can't fire, so there's no concurrent access — the spinlock
never actually contends. Disabling interrupts *is* the mutual-exclusion mechanism here.
</details>

**12.3** In the timer IRQ path we call `gic::eoi(intid)` **before** `sched::tick()` (which may
switch away). Why before, not after?

<details><summary>Answer</summary>
`sched::tick()` may `schedule()` to another thread and not return for a long time. The EOI
must happen first so the GIC drops this interrupt's priority and can deliver further
interrupts; otherwise the priority would stay raised while we're off running another thread,
blocking future IRQs.
</details>

**12.4** `task_trampoline` runs `msr daifclr, #2` before calling the entry. Why is that
essential *specifically* once scheduling is preemptive?

<details><summary>Answer</summary>
A new thread is first entered from inside the timer IRQ handler, where IRQs are masked
(hardware masks them on exception entry, and we switched away instead of `ERET`-ing). Without
re-enabling them, a new non-yielding thread would run with interrupts masked → never get
preempted → monopolize the CPU. The trampoline restores the "threads run with IRQs on"
invariant.
</details>

**12.5** `sleep_ticks` marks the thread `Blocked` and `schedule()`s away. What makes it
`Runnable` again, and what guarantees a thread is always available to run while everyone
sleeps?

<details><summary>Answer</summary>
`wake_sleepers` (called from the timer tick) flips `Blocked → Runnable` when `wake_at <= now`.
`kmain` (task 0) is always `Runnable` — the idle task — so `schedule()` always finds someone
to run even when every worker is blocked.
</details>

**12.6** The first preemption smoke test "failed" even though the serial output looked
correct. What was actually wrong, and what's the lesson?

<details><summary>Answer</summary>
The test's `grep` read the literal needle `[sleeper]` as a **regex character class**, so it
never matched — the kernel was correct. Fixed with `grep -F` (fixed strings). Lesson: read the
actual output and localize the failure; don't "fix" working code because a PASS/FAIL flag
says so.
</details>

---

## 13. Integrative

**13.1** Trace what happens, end to end, when the EL0 routine executes `svc #0` for `SYS_PRINT`.

<details><summary>Answer</summary>
EL0 `svc` → synchronous exception to EL1 via **vector 8** → stub allocates a trap frame on
SP_EL1, saves x0–x30/ELR/SPSR, calls `exception_dispatch(kind=8, frame)` → `kind%4==0`,
`ESR.EC==0x15` → `syscall::dispatch(frame, from_user=true)` → `SYS_PRINT`: validate `x0` is
in user range, build a `&str`, write to the UART, set `frame.x[0]=0` → stub restores
registers + `ERET` → resumes EL0 after the `svc` with the return value in `x0`.
</details>

**13.2** List the layers that cooperate when the heap grows by one page (post-userspace), from
allocation request down to bytes landing in RAM.

<details><summary>Answer</summary>
Heap (bump) needs space → frame allocator hands out a physical 4 KiB frame → `map_page`
installs an L3 page descriptor mapping a virtual page to that frame (TLB invalidated) → the
MMU translates the heap's virtual address through the page tables → `memcpy`/writes place
bytes at the now-mapped physical frame.
</details>

**13.3** Name three distinct things that, misconfigured, would have caused a *silent* hang at
different phases — and what we now have that turns each into a visible diagnostic.

<details><summary>Answer</summary>
Examples: the FP/SIMD trap (phase 1), a wrong page-table descriptor (phase 4), a bad TLB/
barrier sequence (phase 5). Before phase 6 each hung silently (no `VBAR_EL1`). The exception
vector table + `report_and_halt` now prints `ESR`/`ELR`/`FAR`/`SPSR` with a decoded cause for
any such fault.
</details>

**13.4** Trace what happens when the timer fires while the non-yielding `busy` thread is
spinning and the `sleeper` is due to wake.

<details><summary>Answer</summary>
Timer IRQ → vector 5 stub saves `busy`'s **full trap frame** on `busy`'s stack → `handle_irq`:
`on_tick` (count + reload), `eoi`, `sched::tick` → `wake_sleepers` flips the sleeper to
`Runnable` → `schedule()` round-robins to it → `context_switch` saves `busy`'s handler context
(into `busy`'s `Task.sp`) and loads the sleeper's → the sleeper runs, prints, then
sleeps/yields → eventually `schedule()` returns to `busy` (resuming inside its handler) → the
handler returns → `ERET` restores `busy`'s trap frame → `busy` resumes spinning exactly where
it was, none the wiser.
</details>

---

## 14. A real userspace program (ELF loader & workspace)

**14.1** Phase 10 chose an embedded ELF + in-kernel loader over a flat binary at a fixed VA.
What makes the ELF path "how a real OS does it", and what is our `include_bytes!` the moral
equivalent of?

<details><summary>Answer</summary>
A real kernel (`binfmt_elf`) loads programs by walking **`PT_LOAD` program headers** and
mapping each segment at its `p_vaddr` with that segment's own permissions (R-X text, RW
data, zero-filled BSS tail). A flat blob only appears in early-boot stubs/RTOSes. Our
`include_bytes!(USER_ELF)` is the **initramfs** analog — the one place real OSes also bundle
userspace with the kernel image, to solve "run programs before a filesystem exists".
</details>

**14.2** The loader populates each frame through its *identity-mapped physical address* before
installing the user mapping. Why can't it just map first and then write the code in?

<details><summary>Answer</summary>
`PAGE_USER_RX` uses `AP = 0b11` — read-only **at EL1 too**. Once the RX mapping is installed,
the kernel itself can't write through it; a store would permission-fault the kernel. So:
fill the frame via its identity address (EL1-writable), then map it RX.
</details>

**14.3** After copying an executable segment, the loader runs `dc cvau` / `ic ivau` over it.
What problem does that solve, and why do our tests pass even if you delete it?

<details><summary>Answer</summary>
AArch64's I-cache and D-cache are **not coherent**: freshly written code sits dirty in the
D-cache while instruction fetch reads stale memory/I-cache. The sequence cleans the D-cache
to the point of unification, invalidates the I-cache, then `isb`. QEMU's TCG doesn't model
separate I/D caches (stores are immediately visible to fetch), so omitting it works under
QEMU — and would fail intermittently on real hardware. (Apple Silicon wouldn't show it
either: `CTR_EL0.DIC/IDC = 1`.)
</details>

**14.4** How does one `cargo run` build two programs for two privilege levels?

<details><summary>Answer</summary>
The workspace has `kernel` and `user` crates with separate linker scripts. `kernel/build.rs`
runs `cargo build -p user` into an **isolated `CARGO_TARGET_DIR`** (avoiding a deadlock on
the parent build's locked target dir), then exposes the produced ELF path via
`cargo:rustc-env=USER_ELF` for `include_bytes!`.
</details>

**14.5** The user ELF's RW segment has `filesz 8, memsz 0x50`. What does the loader do with
the difference, and what produced those 8 bytes?

<details><summary>Answer</summary>
`memsz - filesz` is the **BSS tail**: zero-initialized statics occupy no file bytes; the
loader zero-fills them. The 8 file bytes are `.data` — `NEXT_TAG: AtomicU64 = 1`, the only
static with a nonzero initializer.
</details>

---

## 15. jring (io_uring-lite)

**15.1** "The point of io_uring is avoiding copies" — correct that statement precisely.

<details><summary>Answer</summary>
io_uring eliminates **control-plane crossings**, not data copies: SQEs/CQEs are syscall
arguments and return values turned into shared-memory data structures, so requests and
completions transfer without traps. The data plane still copies (page cache → user buffer
etc.). Zero-copy is a separate opt-in built on **pinning** (registered buffers, `SEND_ZC`).
</details>

**15.2** How is "I have work for you" actually signaled through the ring page?

<details><summary>Answer</summary>
By **index inequality, not flags**: the producer writes entries, then *publishes* with a
release store of its tail; `tail != head` *is* the notification. The consumer acquires the
tail before reading entries. The only true flag is `NEED_WAKEUP`, which exists solely for
the wake-up negotiation.
</details>

**15.3** The entire ring page is user-writable, including the "kernel-owned" indices. Why is
that not a security hole?

<details><summary>Answer</summary>
The kernel treats everything read from the page as **untrusted input**: indices are masked
(`& RING_MASK`) before use, per-SQE pointers are validated (`is_user_range*`), and
kernel-private state (pending table, waiter, poller id) lives in kernel `.bss`, not the
page. A hostile user can only hurt **liveness** (block themselves, drop their own
completions, burn their own CPU) — never kernel memory safety. Same posture as Linux's
mmap'd rings.
</details>

**15.4** Why does `user_data` exist, and why is tag 0 special in our userspace library?

<details><summary>Answer</summary>
Completions can arrive **out of order** (a parked READ finishes after later-submitted
PRINTs); `user_data` is the opaque tag matching a CQE to its SQE. In `uring.rs`, reaped
non-matching completions are stashed, with tag 0 marking an empty stash slot — so tags
start at 1.
</details>

**15.5** Why is `exit` still a plain syscall instead of a ring opcode?

<details><summary>Answer</summary>
An exit completion could never be reaped — the op destroys the context that would consume
its CQE. Even Linux's io_uring has no exit op. The syscall table is thus `ADD`, `EXIT`,
`RING_SETUP`, `RING_ENTER`; `SYS_PRINT`/`SYS_READ` (numbers 2 and 4) were deleted and the
numbers retired.
</details>

---

## 16. Interrupt-driven input

**16.1** The timer is INTID 30 and the UART is INTID 33. Why did enabling the UART interrupt
require touching a different GIC block than the timer did?

<details><summary>Answer</summary>
INTID 30 is a **PPI** (per-CPU private interrupt, 16–31) — configured in the per-CPU
**redistributor**. INTID 33 is an **SPI** (shared peripheral interrupt, 32+) — configured in
the **distributor**: group, priority, `GICD_IROUTER` (affinity-route to CPU 0), then
`GICD_ISENABLER`.
</details>

**16.2** With the RX interrupt enabled and nobody consuming bytes, the kernel livelocks. Why,
and what are the two honest fixes?

<details><summary>Answer</summary>
The PL011 RX interrupt is **level-asserted while unread data sits in the receiver**: a
handler that returns without reading `UARTDR` re-fires immediately, forever. Fixes: (1)
mask the interrupt when nobody wants input (Phase 13's on-demand masking; the mask *is* the
flow control — NAPI's pattern), or (2) drain unconditionally into a kernel buffer (Phase
16's tty shape). We did 1, then graduated to 2 — where masking survives as buffer-full
backpressure.
</details>

**16.3** What's the *structural* proof in the test suite that completion became event-driven
in Phase 13?

<details><summary>Answer</summary>
The `poll_pending()` call was **deleted from the timer tick**. After that, the only path
that can complete a parked READ is UART IRQ → `poll_pending()` — so the piped session
passing at all proves the interrupt path works. (Latency also dropped from ≤10 ms to
effectively immediate, but the deletion is the assertable proof.)
</details>

---

## 17. Blocking & wake (the user program as a task)

**17.1** Phase 14's entire structural change is one sentence. What is it, and what does it
unlock?

<details><summary>Answer</summary>
**The ERET to EL0 happens inside a spawned task, so every EL0 trap lands on that task's own
kernel stack.** Blocking inside a syscall then becomes ordinary `schedule()` (the same move
`sleep_ticks` makes), unlocking `RING_ENTER(min_complete)` sleeps, IRQ wakes, and later
SQPOLL. `kmain` stays behind as the idle task (`wfi` + yield).
</details>

**17.2** State the lost-wakeup argument for the blocking `enter` loop.

<details><summary>Answer</summary>
The check ("unreaped < min_complete") and the block (`block_current`) could race a CQE
posted in between — except the whole syscall runs with **IRQs masked, and they stay masked
until `context_switch` lands in a task that re-enables them**. The wake source (UART IRQ)
physically cannot run in the window. Single core makes it airtight; it's the first thing to
revisit for SMP. And the loop *re-checks after every wake* — never trust a wake.
</details>

**17.3** Why did `SYS_EXIT`'s old implementation (park in `wfi`) become an active bug in
Phase 14?

<details><summary>Answer</summary>
Parking inside the syscall handler never returns to the scheduler — with the user program
now a task, that freezes the whole machine (the idle task would never run again). Exit now
marks the task `Exited` and `schedule()`s away (`sched::exit_current`).
</details>

**17.4** What did the phase do to QEMU's host CPU usage at an idle `jsh> ` prompt, and why?

<details><summary>Answer</summary>
~100% → **0.7%**. Before: `wait()` busy-spun on the CQ at EL0. After: the user task blocks
in the kernel, the idle task sits in `wfi`, and the UART interrupt wakes the chain — nobody
spins anywhere on the input path.
</details>

---

## 18. SQPOLL

**18.1** "How does the kernel poller wake up without a syscall?" — untangle the trick.

<details><summary>Answer</summary>
While polling it's not asleep at all: it's a `Runnable` task that the **timer tick**
round-robins onto the CPU, where it *looks* at `sq_tail`. Memory writes notify nobody; the
two ways to get attention are "someone was already looking" (polling) or a trap. SQPOLL
moves submission into the first category, with the standing 100 Hz tick paying the
context-switch cost. Once genuinely asleep (`NEED_WAKEUP` raised), only a trap can revive
it — one `enter` call.
</details>

**18.2** The poller raises `NEED_WAKEUP`, then drains **once more** before blocking. What bug
does that final drain prevent?

<details><summary>Answer</summary>
A SQE published *after* the poller's last drain but *before* the flag went up: its submitter
saw the flag clear (no wake syscall), and the poller would sleep with work queued — with a
CQ-spinning user, a deadlock. The order "raise flag → final drain → block" guarantees every
SQE is either caught by that drain or published late enough to see the flag and trap.
Linux's SQPOLL does the same dance.
</details>

**18.3** Why does `enter` clear `NEED_WAKEUP` itself when waking the poller, instead of
letting the poller clear it when it runs?

<details><summary>Answer</summary>
The woken poller doesn't *run* until its next timeslice. If the flag stayed set meanwhile,
every subsequent submit would see it and trap too — in the `spam` demo, all three submits
would syscall and the poller would find an empty SQ (and never print its pickup line).
Clear-at-wake makes exactly one trap pay for the revival.
</details>

**18.4** Why is jos's SQPOLL "architectural demonstration, not a performance win", where
Linux's can be both?

<details><summary>Answer</summary>
Linux dedicates a **spare core** to the poller — submission becomes literally trap-free on
the submitting core (cache coherency carries the store across). jos has one CPU: the poller
time-shares with the producer, pickup latency is tick-bounded, and the poller's polling
steals cycles from the task producing the work. The `NEED_WAKEUP` protocol is identical;
only the economics differ.
</details>

---

## 19. Kernel input buffer & `copy_to_user`

**19.1** The input buffer decouples data arrival from user requests "in both directions".
Name both.

<details><summary>Answer</summary>
**Data before request**: keystrokes are captured (type-ahead) even with no READ parked —
previously they survived only by QEMU's chardev backpressure; real hardware would overrun.
**Request before data**: a parked READ completes on arrival — as before, but the bytes now
come from kernel memory via a named copy. (`test.sh` exercises type-ahead every run: the
whole piped script arrives during boot, before jsh exists.)
</details>

**19.2** Why must the in-between owner of the bytes be the *kernel*, fundamentally?

<details><summary>Answer</summary>
Timing (devices deliver on their schedule; unconsumed bytes need a home or are lost), trust
and lifetime (in a real multi-process OS the kernel can't write a user buffer at arbitrary
interrupt time — wrong address space, possibly swapped out, possibly freed; interrupt
context can't take page faults), and multiplexing/policy (line discipline, many consumers).
jos's earlier direct-to-user-buffer IRQ write was sound only under toy conditions: single
address space, no swapping, a provably live spinning waiter.
</details>

**19.3** `copy_to_user` stores with `STTRB`. What does that instruction do, and what bug
class does it close *today* given we have no PAN?

<details><summary>Answer</summary>
`STTRB` is an **unprivileged store**: executed at EL1, the MMU checks it with EL0
permissions. So even if range validation were buggy, a destination in kernel memory (EL0
no-access) or the user's R-X segment (EL0 read-only) **faults loudly** instead of being
silently corrupted — hardware defense-in-depth against our own bugs. (With PAN, ARMv8.1+,
`LDTR`/`STTR` are also the only lawful channel to user memory; Linux's
`__arch_copy_to_user` is exactly this. Linux additionally recovers via exception fixup
tables where we halt.)
</details>

---

## 20. The ABI crate & shared-memory typing

**20.1** What class of bug does moving `RingPage`/`Sqe`/`Cqe` into the `abi` crate eliminate,
and what enforces the layout now?

<details><summary>Answer</summary>
Kernel and user previously each hand-maintained the offsets (`0x280`, entry sizes) —
agreement by vigilance; drift meant silent protocol corruption. Now one `#[repr(C)]`
definition serves both sides (the uapi-header pattern), and `const` asserts over
`offset_of!`/`size_of` make any drift a **compile error**.
</details>

**20.2** The refactor replaced `read_volatile`/`write_volatile` on ring entries with relaxed
atomics. Why is that a *correctness* change, not just style?

<details><summary>Answer</summary>
`volatile` is for MMIO; for ordinary memory concurrently mutated by another agent (EL0 vs
EL1), racing non-atomic/volatile accesses are formally UB in the Rust/C11 memory model.
Relaxed atomics are the correct tool (Linux's `READ_ONCE`/`WRITE_ONCE` discipline) and
compile to the same plain `ldr`/`str` on AArch64 — zero cost. Acquire/release stays on the
indices, which is what publishes entries.
</details>

**20.3** How many integer↔pointer casts does each side need for the whole ring protocol now,
and why can't it be zero?

<details><summary>Answer</summary>
**One**: `&*(VA as *const RingPage)` — the audited line where a hardware-decided address
becomes a type. It can't be zero because conjuring a reference from a raw address is
inherently the unsafe boundary; the win is concentrating it (everything downstream is
compiler-checked field access).
</details>

---

## 21. Tools & integrative (round 2)

**21.1** `dump.sh` saves user-space memory with `memsave` *after* the program exited. Why do
the user virtual addresses still translate?

<details><summary>Answer</summary>
`SYS_EXIT` retires the task but tears nothing down: the kernel parks with the **MMU on and
the user mappings still installed** (freeing the user's frames is future work). `memsave`
reads through the live EL1 translation, so `0x2_0000_0000` still resolves. (`pmemsave` reads
physical RAM and needs no translation at all.)
</details>

**21.2** The first `dump.sh` run failed with `invalid char 't' in expression`. What was wrong?

<details><summary>Answer</summary>
QEMU's HMP monitor parses `pmemsave 0x40000000 4096 /tmp/x` with an expression parser that
greedily reads `4096 /tmp/...` as a **division**. Filenames starting with `/` must be
double-quoted in the command string.
</details>

**21.3** Trace one keystroke at an idle `jsh> ` prompt, end to end, in the final architecture.

<details><summary>Answer</summary>
jsh is blocked in `enter(min_complete=1)`; idle task sits in `wfi`. Key → UART asserts
INTID 33 → GIC → IRQ on the idle task's stack → `input::drain_uart()` (device → kernel ring,
type-ahead safe) → `ring::poll_pending()`: pops the byte, `copy_to_user` (STTRB) into the
parked READ's buffer, posts the CQE (release of `cq_tail`), `sched::wake(waiter)` → handler
returns, idle yields → context switch into the user task, which resumes in `enter`'s recheck
loop, returns through its trap frame, `ERET` → jsh reaps the CQE from shared memory by tag.
Zero syscalls, zero polling, zero copies beyond the one named kernel→user copy.
</details>

**21.4** Across Phases 12–15, jring gained all three of real io_uring's operating modes. Name
them and what each demonstrated.

<details><summary>Answer</summary>
(1) **Enter-driven batching** (Phase 12): N ops published, one syscall, tag-matched
completions. (2) **Async completion** (Phases 12–13): parked ops completed from interrupt
context while user code runs — first timer-polled, then event-driven via the UART IRQ.
(3) **SQPOLL** (Phase 15): submission with no syscall, the kernel poller consuming published
SQEs, with the `NEED_WAKEUP` shared-memory handshake bounding the trap cost to wake-ups.
Phase 14's block/wake is what made sleeping (instead of spinning) possible in modes 1–3.
</details>

---

*Built across 17 phases — see `docs/superpowers/specs/` for the design of each.*
