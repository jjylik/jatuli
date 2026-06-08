# jatuli — Phase 20: Per-Process Address Spaces + Per-Process Rings (Design)

**Date:** 2026-06-08
**Status:** Implemented
**Goal:** Give each user program its own address space (its own page tables,
`TTBR0`-switched on context switch) and its own jring, replacing today's
singletons (`LOADED`, `USER_FRAMES`, the single ring page + waiter). Bring up
**two** processes from the same embedded ELF to prove isolation and exercise the
switch: identical user VAs (`USER_BASE…`) backed by different physical frames and
different `TTBR0`.

This folds together three TODO items — per-process address spaces, the process
table + per-process state, and the per-process ring half of "decide I/O
multiplexing" — because they are one abstraction: a `Process` that owns page
tables is the same struct that owns the per-process state and ring.

## Builds on Phase 19

Phase 19 moved every EL0-accessible VA into L0 slot 1, leaving the kernel alone in
L0 slot 0. That split is what makes this phase tractable:

- Every process's `TTBR0` shares the **same L0[0] kernel subtree**, so kernel
  threads (idle, sqpoll) and EL1 trap handlers stay correctly mapped under *any*
  process's `TTBR0`. Switching `TTBR0` between processes never unmaps the kernel.
- The ring frame lives in RAM, and **all RAM is identity-mapped in L0[0]**, so the
  kernel can reach any process's ring at its physical address regardless of which
  `TTBR0` is live. This dissolves the hazard documented on `ring::page()` in
  Phase 19.

## The `Process` abstraction

`Process` is a distinct object; the scheduler's `Task` references one (or `None`
for kernel-only tasks). Single thread per process for now.

```rust
struct Process {
    ttbr0: u64,                       // L0 table PA = the address-space root
    loaded: elf::Loaded,              // segment ranges (was the LOADED singleton)
    frames: Vec<(usize, Frame)>,      // (va, frame) owned: segments + stack (was USER_FRAMES)
    table_frames: Vec<Frame>,         // L0[1] page-table frames, for teardown
    ring_pa: usize,                   // ring frame PA; kernel access via identity map
    pending: [Option<Pending>; 8],    // parked READs (was RingState.pending)
    ring_waiter: Option<usize>,       // task blocked in enter() (was RingState.waiter)
    foreground: bool,                 // owns the keyboard
}

static PROCESSES: Locked<Vec<Process>> = …;   // populated with 2 at boot
```

`Task` (in `sched.rs`) gains `process: Option<usize>` — an index into `PROCESSES`.
Kernel tasks carry `None`.

What stays global in `RingState`: only the SQPOLL `poller` task index. The
`pending` table and `waiter` move onto `Process`, and per-process ring setup is
indicated by `ring_pa != 0` (replacing the old global `mapped` bool).

## MMU additions (`mmu.rs`)

The four user-mapping call sites (`elf.rs:153` segments, `ring.rs:106` ring,
`user.rs:108` stack, `user.rs:140` teardown unmap) must target a *specific*
process's L0. The kernel heap (`allocator.rs:35`) and both identity blocks live in
L0[0] and keep using the live-`TTBR0` path.

- **`map_page_in(l0, va, pa, flags)` / `unmap_page_in(l0, va)`** — the existing
  walk, parameterized by an explicit L0 root. `map_page` / `unmap_page` become thin
  wrappers that read the live `TTBR0_EL1` and call the `_in` form (kernel heap path,
  unchanged behavior).
- **`new_address_space() -> u64`** — allocate and zero an L0 frame; set
  `l0[0] = KERNEL_L1_PA | DESC_TABLE` to alias the shared kernel subtree; leave
  L0[1] empty (filled lazily by `map_page_in`). Returns the L0 PA = the process's
  `ttbr0`.
- **`KERNEL_L1_PA`** — captured at `init_mmu` (the table L0[0] points at), stored in
  a static for `new_address_space` to alias. `init_mmu` is otherwise unchanged: it
  still builds only the kernel subtree and installs the boot L0 in `TTBR0`.
- **`activate(ttbr0)`** — `msr ttbr0_el1, {ttbr0}` + `tlbi vmalle1` + `dsb ish` +
  `isb`. Full TLB flush on every address-space switch; ASIDs (to avoid the flush)
  remain the deferred polish item.
- **`free_address_space(l0)`** — walk the L0[1] subtree, free every table frame and
  the L0 frame; never follow L0[0] (shared kernel). With `table_frames` tracked on
  `Process`, teardown can alternatively free those directly — implementation picks
  whichever is simpler; the spec requires only that L0[1] tables and the L0 itself
  are reclaimed and L0[0] is left untouched.

## Scheduler: `TTBR0` switch (`sched.rs`)

In `schedule()`, after selecting `next`: if `next.process` is `Some(p)` and
`PROCESSES[p].ttbr0` differs from the currently installed root, call
`mmu::activate(ttbr0)` **before** `context_switch`. Kernel tasks (`process == None`)
leave `TTBR0` as-is — safe because L0[0] is shared. Track the installed root (a
static, or derive from the outgoing task's process) to avoid a redundant flush when
consecutive tasks share an address space.

This is the one genuinely risky path; the two-process bring-up below is what
exercises it.

## Ring, per process

- Each process owns its own ring frame. The kernel reaches it via the **identity
  PA** (`process.ring_pa`), never the user VA — sound under any `TTBR0`. Userspace
  still maps it at `USER_RING_VA` in its own L0[1].
- `ring::setup` maps the ring into the **calling** process's L0[1] and records
  `ring_pa` on it. `page()` becomes `page_of(process) -> &RingPage` deref'ing
  `ring_pa`. `drain`, `complete`, `process_sqe`, `poll_pending`, `enter` all take
  the target process.
- **SQPOLL round-robins** over live processes, draining each one's SQ each pass
  (single core, one poller). Per-ring `NEED_WAKEUP` is preserved: each `submit()`
  checks its own ring's flag, and `enter` revives the poller. **Fallback:** if the
  per-ring wakeup handshake proves fiddly during implementation, drop `NEED_WAKEUP`
  and keep the poller always-awake; restore the sleep optimization in a later pass.
- `is_user_range` / `is_user_range_writable` / `copy_to_user` validate against the
  **current** process's `loaded` ranges and stack, not a singleton — they look up
  the running task's process.

## Input: foreground policy

A global foreground pid. The UART RX interrupt buffers the byte (unchanged), then
`poll_pending` completes only the **foreground** process's parked reads; background
processes' reads stay parked. Process 0 (jsh) is foreground; process 1's reads park
indefinitely while its PRINTs still flow. Foreground *switching* is deferred — this
phase commits only to "one designated process owns the keyboard," enough to run two
processes without a multiplexing model we'd regret.

## Bring-up: two processes

At boot, after the self-checks:

1. Create two address spaces; `elf::load(USER_ELF, …)` the **same** embedded jsh ELF
   into each (it gains the target address-space root). Map a stack and a ring into
   each.
2. Spawn a `user_task` per process, each bound (via `Task.process`) to its `Process`;
   each ERETs to EL0 on its own kernel stack.
3. Process 0 = foreground (gets input); process 1 = background (banner prints, reads
   park).

Both use identical user VAs backed by different frames and different `TTBR0`, so
isolation holds by construction.

## Teardown

On exit or fault-kill of a process: drop its parked reads and waiter, unmap and free
its `frames`, `free_address_space(ttbr0)` to reclaim its L0[1] tables and L0, and
remove it from `PROCESSES`. The kernel subtree (L0[0]) and the *other* process are
untouched — a fault in one process must not disturb the other.

## Verification

`./test.sh` extended:

- **Both** processes' banners appear (tag PRINT output by pid so the two are
  distinguishable in the serial transcript).
- A kernel self-check confirms the two processes' `ttbr0` values differ while both
  map `USER_BASE` to **different** physical addresses (walk each L0) — direct proof
  of isolation.
- jsh (foreground) still responds to typed input (`help`, `spam`, `exit`) while
  process 1's reads stay parked.
- Run 2 (`crash`) kills only the faulting process and frees its address space; the
  other process and the kernel survive (`*** EXCEPTION` still rejected, no panic).

## Implementation staging (not one commit)

1. **MMU primitives** — `map_page_in` / `new_address_space` / `activate` /
   `free_address_space` / `KERNEL_L1_PA`. Route today's single user program through a
   `Process` whose `ttbr0` is the boot L0; verify no regression.
2. **`Process` + table + scheduler switch** — move the singletons onto `Process`, add
   `Task.process`, install `TTBR0` in `schedule()`. Still N=1; verify no regression.
3. **Per-process ring** — identity-PA access, SQPOLL round-robin, foreground input.
4. **Second process** — bring up the two-process boot and extend `test.sh`.

Each stage is independently bootable and testable; this is deliberately not a
big-bang change.

## Out of scope

`OP_SPAWN` / `OP_WAIT` (user-initiated spawn + child-exit completion), multiple
*distinct* embedded programs, foreground switching, ASIDs, and the broader I/O
multiplexing policy (whose SQ gets priority, fair keystroke routing). Those follow in
later phases.
