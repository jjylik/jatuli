# jos — Phase 8: GICv3 + Generic Timer Interrupts (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Deliver and handle a periodic timer interrupt: bring up the GICv3 interrupt
controller, program the EL1 physical timer (~100 Hz), unmask IRQs, and handle the timer
PPI through the IRQ vector (reusing the Phase 7 trap frame). Demonstrate with a tick
counter driven entirely by interrupts.

## Decisions (from brainstorming)

- **GICv3** (the modern, harder path): pin `-machine virt,gic-version=3`. CPU interface is
  `ICC_*` system registers; per-CPU redistributor must be woken; affinity routing enabled.
- EL1 **physical timer** (`CNTP_*`), PPI **INTID 30**.
- Timer-first as the GIC's first interrupt source (deterministic, smoke-testable). Keyboard
  / SPIs and scheduling are later.
- **Risk:** `cortex-a72` is GICv2-era; if `ICC_SRE_EL1` traps, the report-and-halt handler
  will surface it and we fall back to `-cpu cortex-a76` (or `max`).

## Memory map (GICv3 on `virt`)

- **GICD** (Distributor) @ `0x0800_0000` — global control + SPIs.
- **GICR** (Redistributor, CPU0) @ `0x080A_0000` (RD frame); SGI/PPI frame at `0x080B_0000`.
  PPIs (INTID 0–31, incl. the timer) live here, not in the distributor.
- **CPU interface** — `ICC_*` system registers (no MMIO).

All GIC MMIO is inside the existing identity-mapped device region.

## GICv3 bring-up (`src/gic.rs`)

```
1. ICC_SRE_EL1.SRE = 1 ; isb                       // enable sysreg CPU interface
2. GICD_CTLR = ARE(4) | EnableGrp1(1) | EnableGrp0(0)   // = 0x13
3. GICR_WAKER: clear ProcessorSleep(1); spin until ChildrenAsleep(2) == 0
4. PPI in SGI frame (0x080B_0000):
     GICR_IGROUPR0   |= 1<<INTID        // group 1
     GICR_IPRIORITYR[INTID] = 0x00      // highest priority
     GICR_ISENABLER0 = 1<<INTID         // enable
5. ICC_PMR_EL1 = 0xFF ; ICC_IGRPEN1_EL1 = 1 ; isb  // allow all priorities, enable Grp1
```
Plus `acknowledge() -> ICC_IAR1_EL1` and `eoi(intid) -> ICC_EOIR1_EL1`. (RWP polling on
GICD_CTLR skipped — QEMU completes writes immediately; noted as a real-HW concern.)

## Timer (`src/timer.rs`)

EL1 physical timer, INTID 30:
```
freq = CNTFRQ_EL0                    (stored in a static)
CNTP_TVAL_EL0 = freq / 100           (~10 ms -> ~100 Hz)
CNTP_CTL_EL0  = 1                    (ENABLE, IMASK=0)
on_tick(): TICKS += 1; CNTP_TVAL_EL0 = freq/100   (reload clears the timer condition)
```
`TICKS: AtomicU64` (handler writes, `kmain` reads, `Relaxed`). `pub const TIMER_INTID = 30`.

## IRQ dispatch (`exceptions.rs`)

Vectors come in groups of four; `kind % 4 == 1` is the IRQ entry (vector 5 at EL1/SPx):
```rust
match kind % 4 {
    1 => handle_irq(),
    0 => match ec { EC_SVC => syscall::dispatch(frame), _ => report_and_halt(..) },
    _ => report_and_halt(..),
}
fn handle_irq() {
    let intid = gic::acknowledge();
    if intid >= 1020 { return; }                 // spurious
    if intid == timer::TIMER_INTID { timer::on_tick(); }  // reload BEFORE eoi
    gic::eoi(intid);
}
```
The Phase 7 trap frame saves/restores/`ERET`s around this, so interrupted code resumes.

## Enabling + demo (`main.rs`)

After the syscall check: `gic::init(timer::TIMER_INTID); timer::init();` then unmask IRQs
(`msr daifclr, #2`). `irq_self_check`:
```
while timer::ticks() < 5 { wfi }     // sleep; each timer IRQ wakes us
kprintln!("timer fired {} times", timer::ticks());
uart::write_str("irq self-check passed\n");
```
Idle loop becomes `wfi`. `wfi` (not busy-wait) is the point: the CPU sleeps and the timer
interrupt wakes it.

## Files

| File | Change |
|---|---|
| `src/gic.rs` (new) | GICv3 init, `acknowledge`, `eoi`, register constants. |
| `src/timer.rs` (new) | timer init/reload, `TICKS`, `on_tick`, `ticks`, `TIMER_INTID`. |
| `src/exceptions.rs` | IRQ arm in the dispatcher + `handle_irq`. |
| `src/main.rs` | `mod gic; mod timer;`, init + `enable_irqs` + `irq_self_check`; idle `wfi`. |
| `.cargo/config.toml`, `test.sh`, `README.md` | add `gic-version=3`. |

## Verification

Boot prints the prior self-checks, then (after sleeping on `wfi`, woken by timer IRQs)
`timer fired 5 times` and `irq self-check passed`. That proves GICv3 routes the PPI, the
IRQ vector + trap frame work, the ISR acknowledges/reloads/EOIs, and `wfi`/wake works.
At ~100 Hz, 5 ticks ≈ 50 ms, inside the 1 s smoke window. Debug with `qemu ... -d int`.

## Out of scope

SPIs / interrupt-driven UART (keyboard), scheduling/preemption, GICD RWP polling,
GICv2 fallback unless `a72` rejects `ICC_*`, FIQ, multi-core, EOImode split.
