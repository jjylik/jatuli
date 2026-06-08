# jatuli — Phase 4: Enable the MMU (Identity Map) (Design)

**Date:** 2026-06-06
**Status:** Approved design
**Goal:** Turn on AArch64 address translation at EL1 with an identity map (virtual ==
physical), so the kernel keeps running unchanged but all accesses now go through page
tables with proper cache/permission attributes. No higher-half, no heap rebasing, no
fine-grained permissions.

## Decisions (from brainstorming)

- **Identity map** via `TTBR0_EL1` (kernel runs at the same addresses; nothing relocated).
- **Enable MMU only** — rebasing the heap onto frames is a later phase.
- **2 MiB blocks for RAM, 1 GiB block for the device region**; tables L0 → L1 → L2.
- Page-table frames come from the Phase 3 frame allocator.

## Page tables (4 KiB granule, 48-bit VA)

VA indices: L0 `[47:39]` → L1 `[38:30]` → L2 `[29:21]` → L3 `[20:12]` → offset `[11:0]`.

```
L0[0] ─▶ L1 table                              (L0[0] covers 0–512 GiB)
  L1[0] = BLOCK 1 GiB @ 0x0000_0000  Device     ← UART (0x0900_0000), GIC, etc.
  L1[1] ─▶ L2 table                             (covers 0x4000_0000–0x8000_0000)
      L2[0..64] = BLOCK 2 MiB @ 0x4000_0000 + i*2MiB  Normal  ← 128 MiB RAM
      L2[64..512] = 0 (invalid)
```

Three frames (L0, L1, L2), allocated and zeroed (invalid entry == 0).

## System registers

- **`MAIR_EL1` = `0x04FF`** — attr index 0 = Normal write-back cacheable (`0xFF`),
  index 1 = Device-nGnRE (`0x04`).
- **`TCR_EL1` = `0x1_8080_3510`** — T0SZ=16 (48-bit VA), TG0=4 KiB, inner-shareable WB
  table walks, EPD1=1 (TTBR1 disabled), TG1=4 KiB (valid encoding), IPS=36-bit.
- **`TTBR0_EL1`** = physical address of the L0 table.
- **`SCTLR_EL1`** — read-modify-write to set M (bit 0) | C (bit 2) | I (bit 12).

## Descriptor encoding

- **Block** (L1/L2): `output_addr | flags`.
  - Normal RAM: `DESC_BLOCK | SH_INNER | AF | UXN` (AttrIndx=0, AP=0b00 EL1 RW,
    PXN=0 so kernel code is executable) → `addr | (1<<54) | 0x701`.
  - Device: `DESC_BLOCK | ATTR_IDX_DEVICE | AF | PXN | UXN` → `addr | (3<<53) | 0x405`.
- **Table** (L0→L1, L1→L2): `next_table_phys | 0x3`.

## Enable sequence (barriers are load-bearing)

```
write all three tables → dsb ishst
tlbi vmalle1 → dsb ish → isb
msr MAIR_EL1, TCR_EL1, TTBR0_EL1 → isb
mrs SCTLR_EL1 → set M|C|I → msr SCTLR_EL1 → isb   (translation live)
```

Implemented in Rust via `core::arch::asm!` for `msr`/`mrs`; descriptors written through
plain pointers (frames are flat/identity-accessible before the MMU is on). `boot.s`
unchanged.

## Components / files

| File | Responsibility |
|---|---|
| `src/mmu.rs` (new) | Descriptor/attribute constants, `init_mmu()`: allocate 3 frames, build the identity map, program registers, enable translation. |
| `src/main.rs` (modified) | `mod mmu;`, call `mmu::init_mmu()` after the frame allocator, run an MMU self-check. |
| `test.sh` (modified) | Grep for the new markers. |

## Verification

After `init_mmu()`:
- `uart::write_str` still works → Device mapping is correct (print `mmu enabled`).
- `Box::new(..)` round-trip (heap, Normal RAM) → RAM mapping is correct.
- write/read-back of a freshly allocated frame via its identity-mapped address.
- print `mmu self-check passed`; `test.sh` greps it.

A wrong mapping faults with no vector table installed, hanging silently; debug with
`qemu-system-aarch64 ... -d mmu,int`.

## Out of scope

Higher-half kernel, 4 KiB-page fine-grained permissions, heap rebasing onto frames,
userspace, exception vectors.
