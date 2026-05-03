# RFC F-A5: Bank0 Cooperative Runtime Skeleton — boot, ISRs, scheduler, UI

| Field          | Value                                                              |
|----------------|--------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A5)                                   |
| Status         | Draft; **single PR closes T-A5.1 through T-A5.7 plus the small upstream additions in §1.1.x; review packet ships in the same PR per §11** |
| Feature bead   | `bd-2r1`                                                           |
| Open tasks     | T-A5.1 (`bd-17b`, boot + IRQ vectors), T-A5.2 (`bd-1cv`, scheduler + yield), T-A5.3 (`bd-fcm`, joypad), T-A5.4 (`bd-3ys`, text + font), T-A5.5 (`bd-t0y`, keyboard), T-A5.6 (`bd-1d2`, video_commit + UiCommitPlan), T-A5.7 (`bd-15y`, panic) |
| Closed tasks   | none under `bd-2r1` — every F-A5-owned runtime module is still `//! Module stub.`. (Sibling F-A4 has shipped: `bd-1sv`, `bd-371`, `bd-2sv`, `bd-19j`, `bd-f5y` are closed and `gbf-runtime/src/banking.rs` is real.) F-A5 is **the** PR that fills the remaining eight reference-shell modules. |
| Plan reference | `history/planv0.md` line 117 (PPU mode/VRAM accessibility), line 121 (ISR-residency hard rule), line 127 (yielding as compiler feature + liveness contract), line 155 (workspace `gbf-runtime` slot), line 205 (`gbf-runtime::{boot, interrupts, scheduler, joypad, text, keyboard, video_commit, banking, panic, trace, harness, persistence}`), line 309 (gbf-runtime authoring posture: Rust builders over `AsmIR`), line 1626 (Bank0 nucleus content), line 1958 (ISR-residency rule restated), line 1962 (Bank0 / RuntimeNucleus content list), line 2079 (UI-owned VRAM/OAM contract), line 2167 (auto-yielding ABI), line 2343 (scheduler shape), line 2382 (`UiCommitPlan`), line 2901 (M0 scope) |
| Glossary       | `history/glossary.md` (uses existing terms; introduces no new RFC vocabulary) |
| Constitution   | `CONSTITUTION.md` §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §V.3 (silence on success, loud on failure), §VI.1 (single source of truth) |
| Sibling RFCs   | `history/rfcs/F-A1-gbf-asm.md` (shipped), `F-A2-gbf-hw.md` (shipped), `F-A3-gbf-abi.md` (shipped), `F-A4-banklease-banking.md` (shipped, commit `6feae98`), `F-A6-gbf-store-migrate.md` (Draft), `F-A7-gbf-emu.md` (Draft, gated on a gameroy adapter spike) |

## Pre-implementation blockers — resolved in this draft

The following items were raised against the previous F-A5 sketch; each is now resolved by an inline section below. They are kept as a checklist because they cause real-hardware or CI failures if reintroduced.

- **Resolved (§3A.3)**: Boot layout does not place runtime code/data inside the cartridge header `$0100..=$014F`. The cartridge entry stub is exactly 4 bytes at `$0100..=$0103`; the Nintendo logo and remaining header fields fill `$0104..=$014F`; `runtime_boot_entry` lives at `$0150` onward.
- **Resolved (§3G.3)**: Panic does not disable LCD outside VBlank on real DMG hardware. The panic path spins on `LY` until VBlank before clearing `LCDC.7`.
- **Resolved (§3F.3)**: HBlank commit budget is bounded after ISR entry overhead; OAM-in-HBlank is removed for M0 (`UiCommitPlan::max_ops_per_hblank = 1`, no OAM writes from HBlank).
- **Resolved (§3F.2)**: Queue ABI uses an explicit 8-byte `UiCommitWireOp` wire encoding rather than Rust enum layout.
- **Resolved (§3F.4)**: Queue producer/ISR-consumer publication ordering is specified — payload first, tail published last; consumer reads tail before payload and only consumes when the slot is fully published.
- **Resolved (§1.1.x and §3.3)**: `RuntimeShellModule` lives in `gbf-abi` (added by this RFC, not `gbf-policy`). `PrivilegeClass` names match the F-A1 enum exactly (`Normal`, `Privileged`, `InterruptHandler`); ISR/panic context is carried by section-level `ExecutionContext` / `InterruptDiscipline` annotations introduced by this RFC in `gbf-asm::section` (see §1.1.x).
- **Resolved (§3H.3)**: `runtime_nucleus_hash` hashes a normalized Bank 0 image with `BuildIdentityBlock` hash fields zeroed.
- **Resolved (§3B.3)**: TIMA is a deadline signal, not preemption; the contract requires a compiler-side `max_safe_point_gap_m_cycles` proof on every emitted loop and micro-kernel. The proof itself is owned by Epic B's lowering work; F-A5 declares the contract.

## Project orientation: where this feature sits

### 0.0.1 The big picture

`gbllm3` is a hardware-aware compiler plus cooperative runtime that targets a real DMG Game Boy with an MBC5 cartridge. The end goal is to run a quantized transformer (or recurrent equivalent) on a Game Boy Color-class device with a reproducible, agent-debuggable build. The architecture is decomposed into five products plus three shared contracts (`planv0.md` lines 17–22). `gbf-runtime` is the **fifth product**: the cooperative kernel that owns boot, interrupts, scheduling, UI, banking, panic, and the deferred persistence/trace/harness layers. The model's compiled inference program runs *cooperatively alongside* this nucleus — it does not own the CPU.

The project is delivered in milestones M0 → M6:

- **M0** (this RFC's milestone): bring up the foundation stack — `gbf-asm` (typed eDSL), `gbf-hw` (verified memory map + calibration schema), `gbf-abi` (live execution contract skeleton), `BankLease`/`BankGuard` ABI, **Bank0 cooperative runtime skeleton (this RFC)**, `gbf-emu` deterministic emulator adapter, `gbf-debug` agent CLI. The deliverable is a ROM that boots, draws text, accepts keyboard input, and is debuggable from a script.
- **M1**: oracle stack + first quantized dense kernel + first `CompileRequest`.
- **M2** through **M6**: shared micro-kernels, expert dispatch, sequence state, full interactive text generation, and finally calibration/autotune.

`gbf-runtime` sits between the typed authoring layer (`gbf-asm`, `gbf-hw`, `gbf-abi`) and the compiler/emulator/agent stack. It depends on `gbf-asm`, `gbf-hw`, `gbf-abi`, `gbf-foundation`. It is depended on by `gbf-codegen` (which composes its sections), `gbf-bench` (which calibrates against its nucleus), and the emulator/debug stack (which interprets its boot sequence and harness blocks). It must not know about training frameworks at all (`planv0.md` line 189).

### 0.0.2 What this feature is for

F-A5 fills the `gbf-runtime` crate with the F-A5-owned parts of the **pinned reference shell** per T2.4 (`bd-37r`):
`{Boot, Interrupts, Scheduler, Joypad, Text, Keyboard, VideoCommit}`.

`Banking` is part of the pinned reference shell, but is owned by F-A4 and is not implemented by F-A5. F-A5 may consume the F-A4 banking API at call sites once it exists.

F-A5 also ships the M0-minimal `Panic` section as an audited bring-up exception, even though full panic/fault persistence remains reserved for Epic D.

Each module is authored as Rust builders over `AsmIR` (per `planv0.md` line 309 / line 2001) and emits `Section`s that the backend places into Bank 0 of the cartridge ROM. The deliverable is a runtime nucleus that:

1. **Boots** — wires the cartridge header, the exact Nintendo logo bytes required by the boot ROM, the IRQ dispatch table at `$0040..=$0060`, and ISR entry stubs (annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler`; see §3.3).
2. **Schedules** cooperatively — arms TIMA as a deadline signal before dispatching an inference slice; the timer ISR sets `HRAM[HRAM_ADDR_YIELD_REQUESTED] = 1`; compiler-emitted code polls at safe points (`Yield` pseudo-op), with lowering proving a `max_safe_point_gap_m_cycles` bound; on observed yield, control returns to the scheduler. Liveness counters (`progress_epoch`, `last_checkpoint`, `no_progress_frames`) are bumped per the ABI in `gbf-abi`.
3. **Renders text** — an 8×8 mono font + DMG-sized 20×18 layout. Text writes never touch VRAM directly; they stage `UiCommitOp::PutGlyphCell` through `video_commit`. (Tile bitmaps are installed into VRAM once at boot; see §3D.1.)
4. **Accepts keyboard input** — an on-screen keyboard with cursor + chord input, driven by a cached joypad `ButtonState`, with the layout data-driven from `LexicalSpec.charset` (Epic G).
5. **Owns video commit** — `video_commit` is the *sole* VRAM/OAM writer. All other modules (`text`, `keyboard`, `panic`) stage `UiCommitOp`s into a WRAM ring queue; the commit drain runs from the LCD STAT and VBlank ISRs and gates each op on the current `PpuMode`. Illegal-mode-write attempts raise `FaultCode::UiCommitOutsideLegalMode`.
6. **Reads joypad** — once-per-frame JOYP read with active-low decode, caching `ButtonState` in WRAM for the keyboard layer to consume; Joypad ISR (`$0060`) is wired but is a no-op in M0. STOP-mode wake is deferred to a future power-management feature.
7. **Panics safely** — disable interrupts, dump `FaultCode` to a known location, render a minimal halted screen via direct VRAM write (audited bypass — `video_commit` may itself be broken). Panic is the one section whose `PrivilegeClass` exempts it from the no-direct-VRAM rule, recognized by `ReachabilityValidation`.

Concretely, F-A5 unblocks two things:

- **The compiler has a nucleus to schedule against.** Without F-A5, the compiler's inference slices have nowhere to yield to, no checkpoint id consumer, no UI to share frame budget with, no panic to raise on a fault.
- **`runtime_nucleus_hash` becomes computable.** The hash is computed over a *normalized* Bank 0 nucleus image (with `BuildIdentityBlock` linker-filled hash fields zeroed) plus the runtime ABI version. Compile-profile-specific identity is *not* part of `runtime_nucleus_hash`; that belongs in a separate build identity hash, because the same runtime nucleus can be shared across multiple compile profiles. The `runtime_nucleus_hash` is the load-bearing input to the T2.5 (`bd-177`) CI drift gate, the `KernelCalibrationBundle.runtime_nucleus_hash`, the `RuntimeCalibrationBundle.runtime_nucleus_hash`, the `BuildIdentityBlock.runtime_nucleus_hash`, and the `RuntimeChromeBudget.runtime_nucleus_hash`. See §3H.3 for the normalization rules.

### 0.0.3 Why "single source of truth" applies even to a runtime crate

`gbf-hw` exists because constants drift across crates. `gbf-runtime` exists because *invariants* drift across modules. The dominant failure mode this crate exists to prevent is "two modules each writing to VRAM independently, each one technically correct in isolation, together producing tearing." F-A5's mechanism for keeping that property is not policy: it is (a) `video_commit` is the only module that calls into `gbf-asm` builders that emit `MachineEffect::StoreToVram` or `MachineEffect::StoreToOam`, (b) `panic` is annotated `ExecutionContext::PanicOnly` + `PanicBypass` (see §3.3) and that combination is the *only* recognized exemption to the no-direct-VRAM rule, (c) the `text`/`keyboard` builders only emit `UiCommitOp` enqueues, never raw VRAM stores. The invariant survives because each section carries the new typed `ExecutionContext` annotation introduced in §1.1.x; the audit-walk test reads the annotation, and `ReachabilityValidation` proves the cross-section property.

### 0.0.4 What this feature deliberately does *not* do

F-A5 ships *only* the reference-shell modules. The other three `gbf-runtime` modules listed in `planv0.md` line 205 — `persistence`, `trace`, `harness` — are explicitly deferred to Epic D:

- `persistence` → F-D1 (`bd-2cna`). The versioned SRAM record protocol with `PersistHeader`, `PersistKind`, `DurabilityClass`, `PageState`, atomic `CommitGroupId`-based commit, and SRAM-authoritative recovery.
- `trace` → F-D3 (`bd-1zxn`). The SRAM ring buffer for `TraceEvent`, framing, drop policy, and harness-side reader.
- `harness` → F-D2 (`bd-29wu`). The host-side control plane that polls `HarnessCommandBlock` and emits `HarnessResultBlock`.

The reference shell pins exactly the eight minimal+UI modules. The other three are reserved as `FutureReservation` entries in `RuntimeChromeBudget` so production-shell expansion does not silently invalidate trained models (T2.4, `bd-37r`). F-A5 emits a `RuntimeShellModule::*` annotation per module so the policy layer can compute the correct subtraction; it does not emit the `RuntimeChromeBudget` itself (that is T2.2 / `bd-1g9`).

F-A5 also does not:

- Implement `BankLease`/`BankGuard`. That was F-A4 (`bd-1sv`, **closed** in commit `6feae98`). F-A5 *consumes* a small read-only surface from `gbf_runtime::banking` (the `lower_banking_shadow_zero_init` helper, the HRAM banking-shadow constants, and the `InterruptSafetyTable` declaration helpers; see §0.0.6) but performs no cross-bank far-calls itself — every IRQ stub, scheduler entry, joypad reader, text helper, keyboard step, video_commit drain, and panic body is Bank0-resident by construction.
- Implement `ReachabilityValidation`. That is Epic B Stage 12 (`bd-18d`). F-A5 *declares* `SectionRole::Bank0Nucleus`, `PrivilegeClass::*`, and the new `ExecutionContext` / `InterruptDiscipline` / `PanicBypass` annotations on its sections; the whole-program validation runs in the backend.
- Implement `RuntimeDriftMonitor` or `SafeMode` demotion. That is F-D4 (`bd-191e`). F-A5 ships only the typed `LivenessCounters` updates so the drift monitor has data to read.
- Wire `gbf-cli demo-bank0-rom` into the workspace. That is a thin downstream consumer; F-A5 ships an `examples/demo_bank0_rom.rs` (or equivalent) that the CLI can invoke once `gbf-cli` exists.
- Implement the `gbf-emu` adapter. That is F-A7 (`bd-3mxe`). F-A5's runtime is *exercised by* the emu adapter once F-A7 lands; F-A5 ships an integration test against a tiny in-tree gameroy invocation if the dependency is workable, otherwise the cross-emu integration is a follow-up bead.

### 0.0.5 What's already in tree

`gbf-runtime/src/` is mostly module-stub-only — every F-A5-owned file is the single line `//! Module stub.`. Only `banking.rs` is real (shipped by F-A4 in commit `6feae98`):

```
gbf-runtime/src/
  lib.rs              pub mod declarations only (banking, boot, harness, interrupts,
                       joypad, keyboard, panic, persistence, scheduler, text, trace,
                       video_commit) — no items, no use statements
  boot.rs              "//! Module stub."   ← F-A5 fills
  interrupts.rs        "//! Module stub."   ← F-A5 fills
  scheduler.rs         "//! Module stub."   ← F-A5 fills
  joypad.rs            "//! Module stub."   ← F-A5 fills
  text.rs              "//! Module stub."   ← F-A5 fills
  keyboard.rs          "//! Module stub."   ← F-A5 fills
  video_commit.rs      "//! Module stub."   ← F-A5 fills
  panic.rs             "//! Module stub."   ← F-A5 fills (M0 audited bypass)
  banking.rs           shipped (F-A4, 6feae98)  ← F-A4 owns; not modified by F-A5
  harness.rs           "//! Module stub."   ← F-D2 owns; not modified by F-A5
  trace.rs             "//! Module stub."   ← F-D3 owns; not modified by F-A5
  persistence.rs       "//! Module stub."   ← F-D1 owns; not modified by F-A5
```

`gbf-runtime/Cargo.toml` declares `[dependencies]` on `gbf-abi`, `gbf-asm`, `gbf-foundation`, `gbf-hw`, plus `serde`/`serde_json` from the workspace. The dependency edge to all four sibling Epic A crates is already present; F-A5 adds no new dependency edges.

Sibling Epic A status as of this RFC's drafting (verified against `git log` 2026-05-03):

- **F-A1 (`gbf-asm`)** — **shipped**. Three PRs: cycle model + encoder (`ec10b45`), layout relaxation + lowering (`53d1d82`), listing ROM packet (`7a5c687`). The `Builder` eDSL, `Section`, `SectionRole` (13 variants — `Bank0Nucleus`, `Bank0Data`, `CommonBank`, `WramHotArena`, `HramFastFlags`, …), `MachineEffect` (full algebra including `StoreToVram`, `StoreToOam`, `StoreToMbcRegister`, `InterruptControl`), `PrivilegeClass::{Normal, Privileged, InterruptHandler}`, `SectionPrivilege`, `PreLayoutOp::{BankLease, BankRelease, AssertBank, Yield, TraceProbe}`, `LegalizationOp::FarCall`, cycle model, layout helpers, encoder, ROM assembler are all in `main`. F-A5 emits `Section`s through this `Builder`.
- **F-A2 (`gbf-hw`)** — **shipped** (`a69c2e2 Implement F-A2 gbf-hw hardware contract`). `gbf-hw::interrupts::{INT_VECTOR_VBLANK, INT_VECTOR_LCD_STAT, INT_VECTOR_TIMER, INT_VECTOR_SERIAL, INT_VECTOR_JOYPAD}` and `IE_REGISTER`/`IF_REGISTER`, `gbf-hw::lcd::{PpuMode, LCDC_REGISTER, STAT_REGISTER, LY_REGISTER, vram_accessible_in, oam_accessible_in}`, `gbf-hw::joypad::{JOYP_REGISTER, JOYP_SELECT_BUTTONS, JOYP_SELECT_DIRECTIONS, JOYP_INPUT_MASK, Button, JoypadColumn}`, `gbf-hw::timing::{FRAME_M_CYCLES (= 17_556), VBLANK_M_CYCLES, NORMAL_M_CYCLES_PER_SECOND, FRAMES_PER_SECOND}`, `gbf-hw::mbc5::{RAMG, BANK1, BANK2, RAMB}`, `gbf-hw::cartridge_header::*`, `gbf-hw::memory::*` are all in tree. F-A5 must not re-declare any of those — every constant has a single source of truth in `gbf-hw`.
- **F-A3 (`gbf-abi`)** — **shipped** (`6ad156c Implement F-A3 gbf-abi ABI contract`). `gbf-abi::continuation::InferenceStateHeader`, `gbf-abi::liveness::LivenessCounters` (12-byte `#[repr(C)]` with fields `progress_epoch: u32`, `last_checkpoint: CompactCheckpointId`, `no_progress_frames: u16`, `livelock_threshold_frames: u16`, `_reserved: [u8;2]`; constructor `LivenessCounters::new(threshold)` plus `record_progress` / `note_idle_frame` / `is_livelocked` / `to_bytes` / `from_bytes`), `gbf-abi::fault::{FaultCode, FaultDomain, FaultSnapshot, BootValidationPlan, RecoveryAction}`, `gbf-abi::interrupt::{InterruptPolicy, ResourceLeaseKind, ResourceLease}`, `gbf-abi::checkpoint::{SemanticCheckpointId, CompactCheckpointId, CheckpointResolver}`, `gbf-abi::version::{AbiVersion, BuildIdentityBlock, BuildIdentityArgs}`, `gbf-abi::harness::{HarnessCommandBlock, HarnessResultBlock}`, `gbf-abi::trace::TraceEvent` are all in tree. F-A5 consumes these; it does not re-declare. Note: the **shipped `FaultCode` set does not include `UiCommitQueueFull`**; F-A5 introduces that variant as one of the bundled upstream additions described in §1.1.x.
- **F-A4 (`history/rfcs/F-A4-banklease-banking.md`)** — **shipped** (`6feae98 Implement F-A4 BankLease banking ABI`). `gbf-runtime/src/banking.rs` is now real; the parent feature bead `bd-1sv` is closed (along with T-A4.1 `bd-371` and T-A4.2 `bd-2sv`). The shipped surface F-A5 actually consumes (verified against `gbf-runtime/src/banking.rs` and `docs/review/f-a4/api-change-guide.md`):
    - `pub use gbf_asm::section::{BankLeaseSpec, LeaseGeneration, LeaseId, LeaseLifetime}` re-exports — durable wire shape lives in `gbf-asm`.
    - `ValidatedBankLeaseSpec`, `BankGuard`, `BankLease`, `ReturnState`, `ReturnRomBank`, `ReturnSramState`, `KeepCurrentProof`, `SectionResidency`, `BankAbiViolation`, `BankingEmitError`.
    - Concrete lease helpers: `lease_rom_switchable(b, spec) -> BankGuard`, `lease_sram(b, spec) -> BankGuard`, `release_bank(b, guard, return_state)`. (Note: F-A4 does **not** ship a `BankLease::acquire(...).far_call(...)` method-chain API — earlier F-A5 drafts assumed that shape; the actual API is the three top-level helpers.)
    - `lower_banking_shadow_zero_init(b: &mut Builder)` — the zero-init helper that touches exactly `$FF80..=$FF83` (the four banking shadow bytes). F-A5's `boot.rs` is the call site for this helper (per `docs/review/f-a4/known-debt.md`: "F-A5 boot zero-init call").
    - `InterruptSafety`, `InterruptSafetyKind { InterruptDisabled, InterruptEnabledBank0Only, InterruptHandler }`, `InterruptSafetyTable`, `InterruptSafetyError`, `mark_isr_unreachable`, `mark_isr_reachable`, `mark_isr`, `check_lease_emission_legal`. F-A5 is the first consumer of `InterruptSafetyTable` for its ISR/scheduler/text/keyboard/panic sections (see §3.3 update).
    - `BankingPreLayoutLowering`, `BankingAssertBankPolicy`, `mbc_write_provenance_audit`. F-A5 does not call these directly; they are placement-pipeline surfaces consumed by Epic B / `gbf-codegen`.
    - HRAM banking-shadow constants: `HRAM_SHADOW_BASE = $FF80`, `HRAM_ADDR_CURRENT_ROM_BANK_LO/HI/CURRENT_SRAM_BANK/SRAM_ENABLED` (absolute addrs), `HRAM_LDH_CURRENT_ROM_BANK_LO/HI/CURRENT_SRAM_BANK/SRAM_ENABLED` (LDH offsets `0x80..0x83`), `HRAM_BANKING_SHADOW_END_EXCLUSIVE = $FF84`. **The yield-requested HRAM byte is not in F-A4** — F-A4's HRAM ownership is exactly `$FF80..=$FF83`; per T-A4.2's bead description it explicitly "leaves scheduler/fault/fast flags to F-A5". F-A5 therefore declares its own `HRAM_ADDR_YIELD_REQUESTED = $FF84` / `HRAM_LDH_YIELD_REQUESTED = 0x84` immediately after the banking-shadow region (see §0.0.6).
    - Open follow-ups F-A4 explicitly hands to F-A5 (per `docs/review/f-a4/known-debt.md`): the boot zero-init call site (this RFC); ResumeWindow/Token lifetime restoration (deferred — see §1.2); the keep-current proof producer for `ReturnState::KeepCurrent` (deferred — see §1.2); normal-payload far-call helper materialization (F-A5/F-B13 — F-A5 ships only Bank0-resident sections, so far-call materialization stays an Epic B concern).
    - One follow-up bead opened by F-A4's commit: `bd-2j4m` (T-A8.8 — scripted runtime-ASM conformance smoke suite for emitted F-A* ROMs). It depends on F-A1, F-A4, and F-A5 emitted ROMs but is owned downstream and not on F-A5's critical path.
- **F-A6 (`history/rfcs/F-A6-gbf-store-migrate.md`)** — **RFC drafted** (`2985793 Add F-A6 RFC and defer gbf-migrate scaffolding`). Not on F-A5's critical path.
- **F-A7 (`history/rfcs/F-A7-gbf-emu.md`)** — **RFC drafted** (`338d8d3 Add F-A7 RFC and align gbf-emu beads with API-surface findings`); gbf-emu still stubbed. F-A5's `scheduler::yield_round_trip` test runs in two modes: full round-trip via gameroy if F-A7 has landed by F-A5 closure, otherwise byte-shape assertion only.

The `RuntimeShellModule` typed enum (T2.4 / `bd-37r`) is **not yet in tree** in any crate. F-A5 introduces it as a small bundled addition; see §1.1.x.

### 0.0.6 Coupling to F-A4 (banking)

F-A4 has landed (`6feae98 Implement F-A4 BankLease banking ABI`); `gbf-runtime/src/banking.rs` is real and the parent feature bead `bd-1sv` is closed. The interaction surface F-A5 actually needs from F-A4 is small and is now concrete (no more both-orderings dance):

1. **`lower_banking_shadow_zero_init(b: &mut Builder) -> Result<(), BankingEmitError>`** — the helper that zero-initializes the banking shadow at `$FF80..=$FF83`. F-A5's `boot.rs` calls this from `runtime_boot_entry` before any lease is acquired. F-A4's known-debt explicitly tags this call site as F-A5's responsibility (per `docs/review/f-a4/known-debt.md` row "F-A5 boot zero-init call"). The helper validates internally and returns `BankingEmitError`; F-A5's boot-section builder bubbles the error.

2. **The HRAM banking-shadow constants** (`HRAM_SHADOW_BASE`, `HRAM_ADDR_CURRENT_ROM_BANK_LO/HI`, `HRAM_ADDR_CURRENT_SRAM_BANK`, `HRAM_ADDR_SRAM_ENABLED`, `HRAM_BANKING_SHADOW_END_EXCLUSIVE`, plus the matching `HRAM_LDH_*` LDH offsets). F-A5 imports these read-only — only F-A4 may *write* the banking shadow bytes. F-A5's relevance is just the address discipline: the next free HRAM byte is `HRAM_BANKING_SHADOW_END_EXCLUSIVE = $FF84`, which is where F-A5 places its own `YIELD_REQUESTED` flag (item 3).

3. **F-A5's `YIELD_REQUESTED` HRAM byte (F-A5-owned, not in F-A4)**. F-A4's bead description for T-A4.2 ("F-A4 zero-init touches only those four bytes and leaves scheduler/fault/fast flags to F-A5") is explicit that the banking module owns *only* `$FF80..=$FF83` and the cooperative-yield flag is F-A5's responsibility. F-A5 therefore declares, in `gbf-runtime::scheduler` (or `gbf-runtime::interrupts`, whichever module is the natural source of truth — pinned at implementation time and recorded in the review packet):

   ```rust
   /// Absolute HRAM address of the cooperative-yield flag set by the timer ISR
   /// and polled by compiler-emitted safe-point checks. Sits immediately after
   /// the F-A4 banking shadow region, which ends at HRAM_BANKING_SHADOW_END_EXCLUSIVE = $FF84.
   pub const HRAM_ADDR_YIELD_REQUESTED: u16 = 0xFF84;

   /// Matching LDH offset for `LDH (n), A` instruction encoding.
   pub const HRAM_LDH_YIELD_REQUESTED: u8 = 0x84;
   ```

   These constants are anchored to F-A4's `HRAM_BANKING_SHADOW_END_EXCLUSIVE` via a `static_assert!(HRAM_ADDR_YIELD_REQUESTED == gbf_runtime::banking::HRAM_BANKING_SHADOW_END_EXCLUSIVE);` so the F-A4 banking shadow and the F-A5 yield-requested byte cannot ever overlap (or develop a gap) without a typed compile error. The `frame_count`, `WRAM_LAST_FAULT_ADDR`, scheduler-private "prior frame's last_checkpoint" byte, and other F-A5-owned HRAM bytes are placed sequentially after `$FF84`; the exact map is pinned in the §3.3 / §3A.2 surface.

4. **The `lease_rom_switchable` / `lease_sram` / `release_bank` lease helpers** for any cross-bank far-call. None of F-A5's reference-shell sections currently far-call: every IRQ stub, scheduler entry, joypad reader, text helper, keyboard step, video_commit drain, and panic body is Bank0-resident by construction. The shipped lease API therefore matters at compile sites in `gbf-codegen` and Epic B's lowering (where the normal-payload far-call helper materialization is owned per `known-debt.md` row 3), not in F-A5. F-A5 takes no `ValidatedBankLeaseSpec` / `BankGuard` parameter and acquires no lease.

5. **The `InterruptSafetyTable` declaration substrate.** F-A4 ships `InterruptSafetyKind { InterruptDisabled, InterruptEnabledBank0Only, InterruptHandler }` and the helpers `mark_isr`, `mark_isr_reachable`, `mark_isr_unreachable`. These are the *typed* declaration surface for ISR-residency that Epic B's `ReachabilityValidation` consumes. F-A5 is the first consumer: as part of `build_bank0_nucleus_sections`, it produces (alongside the `Vec<Section>`) an `InterruptSafetyTable` where each section is declared via the right helper:

   - ISR stubs and handler bodies (`isr_stubs`, `interrupts`) → `mark_isr` (`InterruptSafetyKind::InterruptHandler`).
   - `boot`, `scheduler`, `joypad`, `text`, `keyboard`, `video_commit` → `mark_isr_reachable` (`InterruptSafetyKind::InterruptEnabledBank0Only`).
   - `panic` → `mark_isr_unreachable` (`InterruptSafetyKind::InterruptDisabled`, since panic explicitly does `DI` and never `EI`/`RETI`).

   This is additive to (not a replacement for) the F-A5 `ExecutionContext` audit annotation introduced in §1.1.x: `InterruptSafetyKind` says "is this section reachable from an ISR / does it run as an ISR / does it run with IME=0", while `ExecutionContext` says "is this section the only legal source of `MachineEffect::StoreToVram` (`VideoCommitOnly`) or the audit-exempt panic path (`PanicOnly`)". The two annotations answer disjoint audit questions; F-A5 emits both.

6. **Out-of-scope per F-A4's known-debt ledger.** F-A4 explicitly defers the following to F-A5 *scheduler* — but only as eventual scheduler-restoration plumbing, **not** in M0 closure scope:
   - **`ReturnState::KeepCurrent` proof producer.** The F-A4 production lowerer rejects `KeepCurrent` until "a scheduler-owned proof producer lands". F-A5's M0 scheduler does not perform cross-bank slice resumption (its main_loop is Bank0-resident), so it does not need to mint `KeepCurrentProof` values. M0 leaves this as `KeepCurrentProof` having no public constructor — the post-M0 scheduler that resumes inference slices owning a borrowed bank is the right place to add it.
   - **`LeaseLifetime::ResumeWindow` / `LeaseLifetime::Token` lowering.** F-A4's production lowering rejects both lifetimes pending the scheduler-restoration owner. M0 closure does not need either lifetime; F-A5's scheduler emits no leases.

   These are flagged here so the reviewer can confirm F-A5 closure does not silently shoulder them. They are tracked under follow-up beads to be filed when the post-M0 scheduler-restoration story begins; they are out of scope for `bd-2r1` closure.

7. **MBC-write provenance.** F-A5 emits zero `MachineEffect::StoreToMbcRegister` instructions — F-A4's `mbc_write_provenance_audit` therefore has no provenance check to run against F-A5's emitted bytes. The §4.6 single-writer smoke test cross-checks this by walking the F-A5 emitted sections and asserting `StoreToMbcRegister`'s reachable set is empty.

The `gbf-runtime/Cargo.toml` `[dependencies]` edge to itself (`banking` is a sibling module in the same crate, not a separate crate) means F-A5's other modules import from `crate::banking::*` directly. The dependency graph remains: `gbf-runtime → {gbf-abi, gbf-asm, gbf-foundation, gbf-hw}`; no new external edges.

## 0. TL;DR

### 0.1 RFC self-check before implementation

This RFC is ready to implement only if the following are true. Each item is a falsifiable gate; if any is false, the PR description must say so and the matching downstream impact (typically a missed test gate or a placeholder import) must be documented.

**Upstream dependencies (consume, do not redeclare):**

- F-A1 has shipped the `Builder` eDSL, `Section`/`SectionRole`/`MachineEffect`/`PrivilegeClass`/`SectionPrivilege` typing, and the encoder. Verified: PRs `ec10b45` / `53d1d82` / `7a5c687` are on `main`.
- F-A2 has shipped `gbf-hw::interrupts::{INT_VECTOR_*, IE_REGISTER, IF_REGISTER}`, `gbf-hw::lcd::{PpuMode, LCDC_REGISTER, STAT_REGISTER, LY_REGISTER}`, `gbf-hw::joypad::{JOYP_REGISTER, JOYP_SELECT_BUTTONS, JOYP_SELECT_DIRECTIONS, JOYP_INPUT_MASK, Button}`, `gbf-hw::timing::{FRAME_M_CYCLES = 17_556, VBLANK_M_CYCLES, NORMAL_M_CYCLES_PER_SECOND}`, `gbf-hw::mbc5::*`, `gbf-hw::cartridge_header::*`, `gbf-hw::memory::*`. Verified: commit `a69c2e2`. F-A5 must not re-declare any of those constants.
- F-A3 has shipped `gbf-abi::liveness::LivenessCounters` (12-byte `#[repr(C)]` with the *exact* field set listed in §0.0.5), `gbf-abi::fault::{FaultCode, FaultDomain, FaultSnapshot, BootValidationPlan, RecoveryAction}`, `gbf-abi::interrupt::InterruptPolicy`, `gbf-abi::checkpoint::{SemanticCheckpointId, CompactCheckpointId}`, `gbf-abi::version::{AbiVersion, BuildIdentityBlock}`. Verified: commit `6ad156c`. F-A5 consumes these; it does not re-declare.
- F-A4 has shipped (commit `6feae98`); `gbf-runtime/src/banking.rs` is real. F-A5 makes no edits there. Coupling to F-A4 is described in §0.0.6 — F-A5 calls `lower_banking_shadow_zero_init` from boot, declares `HRAM_ADDR_YIELD_REQUESTED = $FF84` (F-A5-owned, immediately after F-A4's `HRAM_BANKING_SHADOW_END_EXCLUSIVE`), and emits an `InterruptSafetyTable` using F-A4's `mark_isr*` helpers. F-A5 holds no `BankGuard`, mints no `KeepCurrentProof`, and emits no MBC register writes.

**Small upstream additions bundled into this PR (§1.1.x lists each with its file):**

- `gbf-abi::fault::FaultCode::UiCommitQueueFull` (new variant in the existing `0x004x` UI domain). The shipped enum has only `UiCommitOutsideLegalMode = 0x0040`; F-A5 adds `UiCommitQueueFull = 0x0041`, plus the matching `FaultDomain::Ui` mapping update, the `FaultCode::ALL` array entry, and the `from_u16` round-trip.
- `gbf-abi::shell::RuntimeShellModule` (new module + new typed enum). Variants name the eight reference-shell modules plus the three `FutureReservation`-only modules so `RuntimeChromeBudget` (T2.2) can match against a closed set. The enum lives in `gbf-abi`, not `gbf-policy`, because `gbf-runtime` must not depend on `gbf-policy` (§3.2). F-A5's `build_*_section()` functions return `(Section, RuntimeShellModule)` pairs (or expose a sibling `runtime_shell_module() -> RuntimeShellModule` const fn) so emission is structurally typed.
- `gbf-asm::section::ExecutionContext` (new typed enum) and `gbf-asm::section::InterruptDiscipline` (new typed enum), carried as additional fields on `SectionPrivilege` (or a sibling `SectionAnnotations` struct). `PrivilegeClass` already has `InterruptHandler` for ISR sections; `ExecutionContext` adds `Normal | InterruptHandler | PanicOnly | VideoCommitOnly` (the audit-relevant context), and `InterruptDiscipline` adds `Default | ImeDisabled`. The `PanicBypass` audit tag is one bit on the same struct, recognized by `ReachabilityValidation` as exempting `panic` from the no-direct-VRAM rule. **Alternative path:** if the F-A5 reviewer prefers to keep `gbf-asm` minimal, the same information is carried in a `gbf-runtime::audit::SectionAudit` map keyed by `SectionId` and consulted by the audit-walk test instead of the asm crate. Either path is in scope; the choice is recorded in the PR description (§6 question 24).

**Behavioral invariants (must remain true for F-A5 closure):**

- The Bank 0 nucleus, when assembled, fits inside 16 KiB minus the `FutureReservation`-table ROM bytes per T2.4. Bring-up estimate (§3H.1) is ~4.7 KiB for the F-A5 modules plus 1.4 KiB future reservations, leaving ~10.2 KiB for compiled `Bank0Free` use; the assertion is enforced by `cargo test -p gbf-runtime -- nucleus_fits_bank0_budget`.
- The `video_commit` module is the sole writer to VRAM/OAM. All other modules emit `UiCommitOp` enqueues only. The `panic` module is the audited exception — section carries `ExecutionContext::PanicOnly`, `InterruptDiscipline::ImeDisabled`, and the `PanicBypass` audit tag (or the `gbf-runtime::audit::SectionAudit` equivalent per the §1.1.x alternative path); `ReachabilityValidation` (Epic B) recognizes the section as exempt.
- The cooperative-yield mechanism uses TIMA as a *deadline signal*, not preemption, and the polled flag lives in HRAM at the F-A5-owned `HRAM_ADDR_YIELD_REQUESTED = $FF84` byte (the byte immediately after F-A4's banking shadow region; F-A4 explicitly leaves "scheduler/fault/fast flags" to F-A5 per T-A4.2's bead). The compiler-emitted yield check is `emit_yield_check<B: Builder>(b: &mut B, kind: YieldKind)`; it polls the HRAM byte and falls through to a save-and-yield sequence on observed flag. The contract requires the compiler/lowering layer (Epic B) to prove a `max_safe_point_gap_m_cycles` bound for every emitted loop and micro-kernel; F-A5 declares the bound, lowering enforces it.
- Every emitted IRQ stub is annotated `PrivilegeClass::InterruptHandler` plus `ExecutionContext::InterruptHandler` (or the `gbf-runtime::audit::SectionAudit` equivalent). Reachability is `Bank0`/`HRAM`/fixed-`WRAM` only. The whole-program proof is Epic B's `ReachabilityValidation`; F-A5 ships the per-section *declarations* the validator consumes plus a local pre-validator smoke test (`isr_residency_pure`).
- `RuntimeShellModule::*` annotations are emitted from each `build_*_section()` function so `RuntimeChromeBudget` (T2.2 / `bd-1g9`) can compute slot capacity per shipped module without parsing any human-language string.
- `runtime_nucleus_hash` is computable from the encoded Bank 0 sections in a deterministic order. The order, the section boundaries, and the hash domain separator are pinned in `gbf-runtime/src/lib.rs` so the T2.5 drift gate can reproduce the hash on any clean checkout. The hash is computed over a *normalized* image with `BuildIdentityBlock` lineage hash fields zeroed (§3H.3).
- Panic is included in the M0 ship even though T2.4's pinned shell lists it as a `FutureReservation`. The compromise (per the bead's T-A5.7 description): ship a minimal panic that disables IME, waits for VBlank before clearing `LCDC.7` (so DMG hardware is not damaged per Pan Docs), and dumps `FaultCode` to a known WRAM byte + on-screen glyphs. Full `FaultSnapshot` + SRAM persistence is F-D1's job.

**PR shape:**

- F-A5 ships in a **single PR** that lands all eight modules together plus the small upstream additions enumerated above. The earlier draft considered a per-module split for review tractability; in practice the modules are too tightly cross-linked (every module references the IRQ vectors; `text` / `keyboard` / `panic` all reference `video_commit::UiCommitOp`; `scheduler` references all of them) for the split to reduce review effort. A single PR closes T-A5.1 through T-A5.7 and the parent feature bead `bd-2r1` in one step. See §5 for the within-PR implementation order.
- The review packet (§11) ships in the same PR. There is no separate "F-A5 review packet PR".

### 0.2 Summary

`gbf-runtime` is the cooperative kernel that owns Bank 0 of the cartridge. F-A5 fills seven F-A5-owned reference-shell modules — `boot`, `interrupts`, `scheduler`, `joypad`, `text`, `keyboard`, `video_commit` — plus the M0-minimal `panic` section, on top of the now-shipped `banking` module (F-A4, `6feae98`); `persistence`/`trace`/`harness` remain deferred to Epic D. Today the workspace pins `gbf-runtime` as a crate with one shipped module (`banking`), eight F-A5-owned stubs, three Epic D stubs, and dependency edges to `gbf-abi`, `gbf-asm`, `gbf-foundation`, `gbf-hw`. F-A5 closure ships:

1. A boot section that emits the cartridge header bytes (via F-A1's `assemble_rom`), places only a 4-byte entry stub at `$0100..=$0103` (`NOP; JP runtime_boot_entry`), keeps `$0104..=$014F` for the Nintendo logo + remaining header fields, and places the real `runtime_boot_entry` (init sequence + bootstrap VRAM install + jump to `scheduler::main_loop`) after the header.
2. An IRQ dispatch table whose vectors point at ISR entry stubs in Bank 0, each stub annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler`. The five DMG IRQ sources are wired: VBlank (`$0040`), LCD STAT (`$0048`), Timer (`$0050`), Serial (`$0058`), Joypad (`$0060`).
3. A cooperative scheduler that arms TIMA as a deadline signal before dispatching an inference slice, runs the slice until either the `yield_requested` flag fires (set by the timer ISR) or the slice completes, returns to the scheduler frame loop, runs UI work (joypad read → keyboard step → text re-render → video_commit drain) within a configurable `hard_ui_reserve`, and either resumes the inference slice or sleeps with `HALT` until the next interrupt. The contract requires the compiler/lowering layer to prove a maximum safe-point gap inside every emitted loop and micro-kernel.
4. A `video_commit` module that is the sole VRAM/OAM writer. The commit queue is a WRAM ring of 32 fixed-size 8-byte wire ops (independent of Rust enum layout). `UiCommitOp` covers bounded micro-ops only: `PutGlyphCell`, `FillGlyphRun`, `SetDmgPalette`, `PutOamSprite`. The drain has two entry points: `emit_commit_drain_hblank` (single-cell BG-map writes only; never OAM) and `emit_commit_drain_vblank` (VRAM + OAM, up to the per-frame cap, prioritized via `UiCommitPlan`). Producer/consumer publication ordering is specified (payload first, tail published last). Queue capacity exhaustion raises `FaultCode::UiCommitQueueFull`; an attempt to execute a VRAM/OAM write in an illegal PPU mode raises `FaultCode::UiCommitOutsideLegalMode`.
5. A text renderer with an 8×8 mono font (M0 default 128 glyphs × 16 bytes = 2048 bytes; full 256-glyph fonts are a CommonBank follow-up) and a DMG-sized 20×18 layout backed by a 32-entry-stride BG map, plus an `emit_text_print_glyph` builder helper that stages `UiCommitOp::PutGlyphCell` through `video_commit`. Tile bitmaps are installed into VRAM tile data once at boot via `video_commit::emit_bootstrap_vram_init` while LCD is off; per-frame text printing is BG-map writes only.
6. A joypad reader doing once-per-frame `JOYP` read with the standard active-low decode, caching `ButtonState` in WRAM. The Joypad ISR (`$0060`) is wired but is a no-op in M0; STOP-mode wake is deferred.
7. An on-screen keyboard with the M0 4×10 ASCII layout (lowercase a–z, digits 0–9, period, space, backspace, submit). `SpecialKey` enumerates `Backspace`, `Submit`, `Shift`, `Cancel` (the latter two are reserved variants not in the M0 default layout). Keyboard state lives in WRAM; each step reads the cached joypad state, advances the cursor, and emits the chosen character into the F-A5-owned M0 prompt buffer (`PROMPT_BUFFER_BASE_ADDR`/`PROMPT_CURSOR_ADDR`/`PROMPT_SUBMITTED_FLAG_ADDR`). When F-D2 lands, the producer/consumer protocol may be replaced but must preserve or migrate these symbols.
8. A minimal panic that disables IME, waits for VBlank before clearing `LCDC.7` (so DMG hardware is not damaged), then writes the fault code as glyphs directly to VRAM and HALTs. The fault byte is also written to `WRAM_LAST_FAULT_ADDR` so an attached emulator/debugger can read it before reset. The section carries `ExecutionContext::PanicOnly` + `InterruptDiscipline::ImeDisabled` plus the audit-exempt VRAM annotation; it is the only authorized bypass of the "no direct VRAM write" rule.

The pipeline is *not* a runtime pipeline in the sense of `gbf-codegen` — `gbf-runtime` is a builder library. Each `build_*_section()` returns an F-A1 `Section` that the backend later places. The crate is `no_std + alloc` capable in shape (subject to `gbf-foundation`'s `no_std` switch); F-A5 keeps `std::fmt`/`std::error::Error` to mirror the rest of the workspace and structures the source so the eventual switch is mechanical.

```
gbf-runtime/src/
  lib.rs                ──┐
  boot.rs                 │  T-A5.1 (part 1): build_boot_section, build_irq_vectors_section, build_isr_stubs_section
  interrupts.rs           │  T-A5.1 (part 2): IRQ dispatch table, ISR entry stubs, IF acknowledgement
  scheduler.rs            │  T-A5.2: build_scheduler_section, emit_yield_check, SchedulerPolicy, liveness wiring
  joypad.rs               │  T-A5.3: build_joypad_section, emit_joypad_read, joypad ISR
  text.rs              ──┐│  T-A5.4: build_text_section, font_bytes, TextLayout, emit_text_print_glyph
  keyboard.rs            ││  T-A5.5: build_keyboard_section, KeyboardLayoutSpec, KeyboardState, emit_keyboard_step
  video_commit.rs        ││  T-A5.6: build_video_commit_section, UiCommitPlan, UiCommitOp, emit_queue_op, emit_commit_drain
  panic.rs              ─┘│  T-A5.7: build_panic_section, emit_panic, emit_panic_screen_render
  banking.rs            ──┘  Owned by F-A4; not modified by F-A5
  harness.rs                Owned by F-D2; not modified by F-A5
  persistence.rs            Owned by F-D1; not modified by F-A5
  trace.rs                  Owned by F-D3; not modified by F-A5
```

The new modules add roughly 1.4 KLOC of production code plus about 1.2 KLOC of section-shape tests, ISR-wiring tests, video-commit mode-gating tests, yield-round-trip tests, font-shape tests, keyboard layout tests, and Bank 0 budget assertions. The font itself is a 4 KiB asset (`assets/font_8x8.bin`); the rest of the LOC is `AsmIR`-emitting Rust.

The five most load-bearing decisions in this RFC are:

1. **`video_commit` is the sole VRAM/OAM writer.** All other modules emit `UiCommitOp` enqueues only. `panic` is the one audited exception, recognized by `ReachabilityValidation`. This invariant survives because the `text`/`keyboard` modules are authored against a builder context that does not have access to direct VRAM stores.
2. **Cooperative yield uses TIMA, not VBlank-only.** VBlank alone is ~1140 M-cycles per 17556-cycle frame. Letting inference slices run only inside VBlank wastes ~94% of the available compute. F-A5 follows `planv0.md` line 2167–2170 and uses TIMA-armed deadlines: the scheduler arms TIMA before dispatching a slice, the timer ISR sets a HRAM flag, and the compiler-emitted code polls the flag at safe points. Inference therefore runs across the entire frame, never touching VRAM/OAM directly; the UI commit queue handles the legal-mode gating.
3. **Liveness counters are enforced by the scheduler, not by hope.** The scheduler bumps `LivenessCounters::no_progress_frames` if no `progress_epoch` advance occurs in a frame. Past `SchedulerPolicy::max_no_progress_frames`, it raises `FaultCode::LivenessTimeout`. Repeated revisits of the same `last_checkpoint` raise `FaultCode::RepeatedCheckpointNoProgress`. These are typed enum variants from `gbf-abi`, not ad hoc panics.
4. **The runtime nucleus emits a deterministic section order.** `runtime_nucleus_hash` is a load-bearing input for the T2.5 drift gate, the calibration bundles, the `BuildIdentityBlock`, and the `RuntimeChromeBudget`. F-A5's `lib.rs` exports a `pub fn build_bank0_nucleus_sections() -> Vec<Section>` whose order is pinned. Any reorder is a deliberate hash bump, recorded in `artifacts/calibration/PINNED_HASH_HISTORY.md` per T2.5.
5. **Panic is included in M0 even though it is in T2.4's `FutureReservation` table.** Without panic, faults silently freeze the system. The compromise: ship a minimal panic in M0 (disable IME + halt + on-screen `FaultCode` glyphs + WRAM last-fault byte). Full `FaultSnapshot` + SRAM persistence is deferred to F-D1; the SRAM record protocol does not exist yet.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A `boot` module that exports `build_boot_section() -> Section` (cartridge entry stub at `$0100..=$0103` + the post-header `runtime_boot_entry` init sequence; the cartridge header byte fields and the exact Nintendo logo bytes at `$0104..=$0133` are stamped by F-A1's `assemble_rom`), `build_irq_vectors_section() -> Section` (IRQ dispatch table at `$0040..=$0060`), and `build_isr_stubs_section() -> Section` (five ISR entry stubs annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler`).
- An `interrupts` module that exports the ISR handlers (one per DMG IRQ source) and a narrow `IF` pending-bit helper used only when software intentionally discards a pending interrupt request. Handlers are split: the *stubs* (boot's deliverable) save regs and call into Bank0-resident handler functions; the *handlers* (this module's deliverable) do the actual work — VBlank handler bumps a frame flag and invokes the VBlank portion of `video_commit::emit_commit_drain_vblank`; LCD STAT handler invokes `video_commit::emit_commit_drain_hblank` against the current `STAT_REGISTER` mode; Timer handler sets `HRAM[HRAM_ADDR_YIELD_REQUESTED] = 1`; Serial handler is a no-op; Joypad handler is a no-op in M0. The CPU acknowledges the serviced interrupt by clearing the corresponding `IF` bit before entering the handler; F-A5 handlers do not write `IF` as part of the normal ISR path.
- A `scheduler` module that exports `build_scheduler_section() -> Section`, `emit_yield_check<B: Builder>(b: &mut B, kind: YieldKind)`, `emit_arm_tima<B: Builder>(b: &mut B, deadline: TimerDeadline)`, and a `SchedulerPolicy` value object. The scheduler arms TIMA as a deadline signal, dispatches inference slices, runs UI work, polls liveness, and either resumes inference or sleeps with `HALT`. It maintains `InferenceState` per F-A3's prefix.
- A `joypad` module that exports `build_joypad_section() -> Section` and `emit_joypad_read<B: Builder>(b: &mut B)`. The joypad ISR is wired into the IRQ vector at `$0060` but is a no-op in M0. The cached `ButtonState` lives in WRAM at `JOYPAD_CACHED_STATE_ADDR`.
- A `text` module that exports `build_text_section() -> Section`, `font_bytes() -> &'static [u8]` (the M0 2 KiB 8×8 mono font; `FONT_TILE_COUNT = 128`), `TextLayout` (with `bg_map_origin`, `visible_columns: 20`, `visible_rows: 18`, `bg_map_stride: 32` for DMG), and `emit_text_print_glyph<B: Builder>(b: &mut B, x: u8, y: u8, glyph: u8)`. The print helper *stages* a `UiCommitOp::PutGlyphCell` — it does not write VRAM. Tile bitmaps are installed via `video_commit::emit_bootstrap_vram_init` from boot before LCD is enabled.
- A `keyboard` module that exports `build_keyboard_section() -> Section`, `KeyboardLayoutSpec<'a>` (lifetime-borrowed for ROM-resident layouts), `KeyboardLayoutManifest` (host-serializable counterpart), `KeyboardCell`, `KeyboardState`, `SpecialKey`, and `emit_keyboard_step<B: Builder>(b: &mut B)`. The default layout is the M0 4×10 ASCII grid (see §3E.4); consumers may override via `LexicalSpec.charset` (Epic G).
- A `video_commit` module that exports `build_video_commit_section() -> Section`, `UiCommitPlan`, `UiCommitOp` (bounded micro-ops only: `PutGlyphCell`, `FillGlyphRun`, `SetDmgPalette`, `PutOamSprite`), `UiCommitWireOp` (the stable 8-byte WRAM wire encoding), `emit_queue_op<B: Builder>(b: &mut B, op: UiCommitOp)`, `emit_commit_drain_hblank<B: Builder>(b: &mut B)`, and `emit_commit_drain_vblank<B: Builder>(b: &mut B)`. The commit queue is a 32-entry WRAM ring with explicit publication-order semantics (§3F.4). Queue capacity exhaustion raises `FaultCode::UiCommitQueueFull`; an attempt to execute an op in an illegal PPU mode raises `FaultCode::UiCommitOutsideLegalMode`.
- A `panic` module that exports `build_panic_section() -> Section`, `emit_panic<B: Builder>(b: &mut B, code: FaultCode)`, and `emit_panic_screen_render<B: Builder>(b: &mut B)`. The section carries `ExecutionContext::PanicOnly` + `InterruptDiscipline::ImeDisabled` plus the audit-exempt VRAM annotation; the annotation is recognized by `ReachabilityValidation`. The panic path waits for VBlank before clearing `LCDC.7` so DMG hardware is not damaged.
- A pinned section assembly order: `pub fn build_bank0_nucleus_sections() -> Vec<Section>` returns sections in the deterministic order `[boot, irq_vectors, isr_stubs, interrupts_handlers, scheduler, joypad, text, keyboard, video_commit, panic]`. The order is part of the runtime ABI; reordering bumps `runtime_nucleus_hash`.
- Per-module `RuntimeShellModule` annotations using a typed enum defined in an upstream shared crate (`gbf-abi` or `gbf-foundation`) so `RuntimeChromeBudget` emission (T2.2 / `bd-1g9`) can compute slot capacity per shipped module. The annotations are typed; no string parsing is allowed. `gbf-runtime` must not depend on `gbf-policy`.
- A `cargo test -p gbf-runtime` matrix that, by itself, proves: (a) the encoded Bank 0 nucleus fits under 16 KiB minus the future reservation; (b) every ISR stub carries `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler` with `Bank0/HRAM/fixed-WRAM` reachability; (c) the scheduler emits a yield-round-trip that an in-tree gameroy invocation can step through (or, lacking that dependency, the encoded byte shape matches a snapshot); (d) the video_commit drain refuses to write VRAM in PPU mode 3; (e) the text print helper stages a `UiCommitOp::PutGlyphCell` and never emits a `MachineEffect::StoreToVram` directly; (f) the panic section is the *only* runtime section whose `MachineEffect::StoreToVram` is direct, and its body waits for VBlank before clearing `LCDC.7`.
- A workspace-wide invariant test (in `gbf-test` once that crate exists; landed as a `#[ignore]` smoke test colocated with `gbf-runtime`'s `tests/` until then) that confirms no other crate emits `MachineEffect::StoreToVram` or `MachineEffect::StoreToOam` from outside `video_commit` or `panic`.

### 1.1.x Bundled upstream additions (in this PR)

F-A5 is one PR that fills `gbf-runtime` plus the following small upstream additions. Each addition is mechanical, scoped, and lives in the same PR diff so the workspace stays consistent at every commit boundary.

| File | Addition | Reason |
|------|----------|--------|
| `gbf-abi/src/fault.rs` | New variant `FaultCode::UiCommitQueueFull = 0x0041`; corresponding entries in `FaultCode::ALL`, `FaultCode::from_u16`, and `classify_fault` (mapping to `FaultDomain::Ui`); update the layout/round-trip tests to cover it. | The shipped enum has only `UiCommitOutsideLegalMode`. F-A5 distinguishes capacity exhaustion (queue full) from illegal-mode-write attempts (separate fault); see §3F.3 and §6.23. |
| `gbf-abi/src/shell.rs` (new module) | New typed enum `RuntimeShellModule` with variants `Boot`, `Interrupts`, `Scheduler`, `Banking`, `Joypad`, `Text`, `Keyboard`, `VideoCommit`, `Panic`, `FuturePersistence`, `FutureTrace`, `FutureHarness`, plus a `pub const ALL: &'static [RuntimeShellModule]` array, a `runtime_shell_module()` helper trait or sibling const, and `Serialize`/`Deserialize` derive. Re-export through `gbf-abi/src/lib.rs`. | T2.4 (`bd-37r`) commits to a typed enum the runtime emits per shipped module so `RuntimeChromeBudget` (T2.2) can match against a closed set. Lives in `gbf-abi`, not `gbf-policy`, because `gbf-runtime` must not depend on `gbf-policy`. |
| `gbf-asm/src/section.rs` | Add `ExecutionContext` enum (`Normal | InterruptHandler | PanicOnly | VideoCommitOnly`) and `InterruptDiscipline` enum (`Default | ImeDisabled`) plus a `PanicBypass(bool)` flag, all carried as additional fields on `SectionPrivilege` (or a sibling `SectionAnnotations` struct attached to `Section`). Wire the new fields through `Section::new`, `Section::with_privilege`, the SoA arrays, and the existing privilege-check tests. | F-A5's audit walk (the test that proves only `video_commit` and `panic` emit `StoreToVram`/`StoreToOam`) needs typed annotations on each section so the walk does not depend on string identity of section names. |
| `gbf-asm/src/section.rs` (alternative path, decided by reviewer) | If the F-A5 reviewer prefers to keep `gbf-asm` minimal, drop the previous row and instead add `gbf-runtime/src/audit.rs` with `pub struct SectionAudit { pub execution_context: ExecutionContext, pub interrupt_discipline: InterruptDiscipline, pub panic_bypass: bool }` keyed by `SectionId` in a per-crate map; the audit-walk test reads the runtime-side map. | Keeps `gbf-asm` smaller. The choice is pinned in the F-A5 PR description; both paths land the same test surface. |
| `gbf-runtime/Cargo.toml` | Add `static_assertions` (compile-only) and (optionally) `gameroy` to `[dev-dependencies]` if F-A7 has shipped before F-A5 closure. Otherwise omit `gameroy` and run the yield round-trip test in byte-shape mode. | Compile-time size assertions for `UiCommitWireOp` (8 bytes) and the optional cross-emu integration test. |
| `gbf-runtime/Cargo.toml` | Add `memoffset` if needed (only if F-A5 introduces a new `#[repr(C)]` POD that needs offset-pinning). | Most layouts F-A5 needs are already pinned in `gbf-abi`; this row is conditional. |

The PR description must state which alternative was taken for the `ExecutionContext`/`InterruptDiscipline` placement (`gbf-asm` vs `gbf-runtime::audit`) and why. Either choice is acceptable; the closure tests are agnostic to the choice.

### 1.2 Non-goals (deferred)

- **`persistence`, `trace`, `harness` modules.** F-D1, F-D3, F-D2. F-A5 does not modify those module stubs.
- **`BankLease`/`BankGuard` runtime ABI implementation.** F-A4 (`bd-1sv`, **closed**). F-A5 does not author `gbf-runtime/src/banking.rs`; F-A4 already shipped it (`6feae98`). F-A5 only consumes a small surface from `banking` (the `lower_banking_shadow_zero_init` helper, the `HRAM_*_*` shadow-region constants, and the `InterruptSafetyTable` declaration helpers — see §0.0.6); it acquires no leases and emits no MBC writes. If a later iteration adds far-calls (e.g., the keyboard layout in a CommonBank), that is a follow-up bead inside `gbf-codegen` / Epic B, not F-A5.
- **`ReturnState::KeepCurrent` proof producer and `LeaseLifetime::ResumeWindow`/`Token` lowering.** Both are flagged in F-A4's `docs/review/f-a4/known-debt.md` as "F-A5 scheduler". F-A5's M0 scheduler is Bank0-resident and does not resume inference slices that own a borrowed bank, so it does not need to mint `KeepCurrentProof` and uses neither `ResumeWindow` nor `Token` lifetimes. The post-M0 scheduler-restoration story owns these — a follow-up bead, not `bd-2r1`.
- **Normal-payload far-call helper materialization.** F-A5/F-B13 per F-A4's known-debt. F-A5 ships only Bank0-resident sections; the far-call materialization helper that future normal-payload code needs is owned in Epic B's lowering pipeline.
- **`RuntimeChromeBudget` emission.** T2.2 (`bd-1g9`). F-A5 ships the per-module `RuntimeShellModule` annotations and the deterministic section order; the emitter that produces the structured `RuntimeChromeBudget` artifact lives in T2.2.
- **`runtime_nucleus_hash` CI drift gate.** T2.5 (`bd-177`). F-A5 ships the deterministic section order so the hash *is* computable; the CI workflow that compares the hash across PRs is T2.5.
- **`ReachabilityValidation` whole-program pass.** Epic B Stage 12 (`bd-18d`). F-A5 ships the `SectionRole` and `PrivilegeClass` annotations the validation consumes; the validation itself runs in the backend.
- **`RuntimeDriftMonitor` + automatic mode demotion.** F-D4 (`bd-191e`). F-A5 ships the `LivenessCounters` updates and the `FaultCode::LivenessTimeout` raise; the drift monitor reads them.
- **`SafeMode` runtime variant.** Part of the `SchedulePack` story owned by Epic B. F-A5 ships only the `Default` runtime mode.
- **CGB / GBC features.** DMG-only. The interrupt mask vocabulary (VBlank, LCD STAT, Timer, Serial, Joypad) is the DMG five.
- **Full `FaultSnapshot` + SRAM persistence at panic time.** F-D1. F-A5's panic dumps `FaultCode` to a WRAM byte and renders glyph-encoded fault code on screen; `FaultSnapshot { fault, domain, recommended_action, slice, rom_bank, checkpoint, progress_epoch, flags }` lives in `gbf-abi` and is constructed by F-D5's `FaultPolicy` + `RecoveryAction` machinery.
- **`OverlayPlan` / WRAM overlay install.** Epic B Stage 8.5. F-A5's runtime does not install or invoke WRAM overlays; that belongs to the compiler's expert-kernel pathway.
- **Symbol tables for the runtime nucleus.** F-A1's `gbf-asm::symbols` owns symbol generation; F-A5 emits sections through the F-A1 builder, which produces the symbol table as a side effect. F-A5 does not author its own `.sym` file.
- **Calibrating the runtime nucleus against gameroy.** Owned by `gbf-bench` (F-E2 / `bd-2ww0` for `PlatformCalibrationBundle`, F-E4 / `bd-34pr` for `RuntimeCalibrationBundle`). F-A5 closure does not include calibration; the `runtime_nucleus_hash` is the input to those bundles.
- **`gbf-cli demo-bank0-rom` subcommand.** Lives in `gbf-cli`. F-A5 ships an `examples/demo_bank0_rom.rs` (or equivalent) that invokes `build_bank0_nucleus_sections()` and pipes through the F-A1 ROM assembler; the CLI subcommand is wired by a follow-up bead in the `gbf-cli` crate.
- **`gbf-emu` adapter integration.** F-A7 (`bd-3mxe`). F-A5 ships an in-tree gameroy invocation for the yield-round-trip test only if the dependency is workable from `gbf-runtime`'s `dev-dependencies`; otherwise the cross-emu integration is a follow-up bead and the yield-round-trip test asserts only on the *encoded section bytes* (still a meaningful test: it pins the AsmIR shape).

## 2. Background and existing state

### 2.1 What is already in tree

`gbf-runtime` has its `banking` module shipped (F-A4, commit `6feae98`); the eight F-A5-owned reference-shell modules are still stubs at the moment F-A5 begins. The four sibling Epic A crates have varying readiness states.

**`gbf-runtime` (this crate, banking shipped, eight reference-shell modules still stubbed):**

- `gbf-runtime/Cargo.toml` — pinned, `publish = false`, depends on `gbf-abi`, `gbf-asm`, `gbf-foundation`, `gbf-hw`, `serde`, `serde_json`. F-A5 closure adds `static_assertions` (compile-only), and (optionally) `gameroy` as a `[dev-dependencies]` entry for the yield-round-trip integration test.
- `gbf-runtime/src/lib.rs` — declares the thirteen scaffolded modules (`banking`, `boot`, `harness`, `interrupts`, `joypad`, `keyboard`, `panic`, `persistence`, `scheduler`, `text`, `trace`, `video_commit`) and contains a single doc-comment.
- `gbf-runtime/src/banking.rs` — **shipped** by F-A4 (`6feae98`); ~2.6 KLOC of production code plus tests; exports `ValidatedBankLeaseSpec`, `BankGuard`, `BankLease`, `ReturnState{Rom,Sram,KeepCurrent}`, `KeepCurrentProof`, `SectionResidency`, `BankAbiViolation`, `BankingEmitError`, `lease_rom_switchable`, `lease_sram`, `release_bank`, `lower_banking_shadow_zero_init`, `InterruptSafetyKind/Table`, `mark_isr*`, `check_lease_emission_legal`, `BankingPreLayoutLowering`, `BankingAssertBankPolicy`, `mbc_write_provenance_audit`, plus the `HRAM_SHADOW_BASE..HRAM_BANKING_SHADOW_END_EXCLUSIVE` constants. F-A5 does not modify it.
- `gbf-runtime/examples/banking_demo.rs` — also F-A4; consumed by the F-A4 review packet (`docs/review/f-a4/`). F-A5 may add `examples/demo_bank0_rom.rs` alongside it.
- `gbf-runtime/src/{boot, interrupts, scheduler, joypad, text, keyboard, video_commit, panic}.rs` — every file is exactly `//! Module stub.`. F-A5 fills these.
- `gbf-runtime/src/{harness, persistence, trace}.rs` — also stubs, but owned by other features. F-A5 makes no edits.

**`gbf-asm` (F-A1 — fully shipped):** PRs `ec10b45`, `53d1d82`, `7a5c687`, plus review-packet harness `0b9c786`. The relevant F-A1 facts for F-A5 authoring (verbatim against the in-tree types — no shorthand):

1. The `Builder` eDSL exposes `Section` construction with typed instructions, pseudo-ops, and data directives. F-A5 modules emit sections through this API. The current F-A1 `Builder` exposes `bank_lease(BankLeaseSpec)` / `try_bank_lease` / `bank_release(LeaseId)` / `try_bank_release_to(LeaseId, BankReleaseDisposition)` / `assert_bank(MbcBankClass, u16)` / `yield_op(YieldKind)` / `trace_probe(TraceProbeId)` / `far_call(...)` and the typed `emit(Instr)` / `try_emit(Instr)` / `db` / `dw` / `label` / `align` emitters. F-A5 calls into the typed-instruction and pseudo-op surface directly; banking-shaped calls (`bank_lease` / `try_bank_lease` / `try_bank_release_to`) flow through F-A4's `lease_rom_switchable` / `lease_sram` / `release_bank` wrappers, but F-A5's reference-shell sections do not invoke any of those because they are Bank0-resident by construction (see §0.0.6).
2. `SectionRole` is a 13-variant enum (in `gbf-asm/src/section.rs`): `Bank0Nucleus`, `Bank0Data`, `CommonBank`, `CommonData`, `ExpertBank`, `ExpertData`, `WramHotArena`, `WramOverlay`, `HramFastFlags`, `SramPersistent`, `VramOwnedByUi`, `OamOwnedByUi`, `HeaderCartridge`. F-A5's modules use `Bank0Nucleus` for executable sections (boot, IRQ vectors, ISR stubs, interrupts handlers, scheduler, joypad, text, keyboard, video_commit, panic), `Bank0Data` for the font asset and the M0 keyboard layout table, `HramFastFlags` only for the per-frame HRAM scratch (e.g., `frame_count` byte), and `HeaderCartridge` for the cartridge header sub-section consumed by F-A1's `assemble_rom`. ISR-reachability is *proved* by Epic B's `ReachabilityValidation`; F-A5 declares it via the new `ExecutionContext::InterruptHandler` annotation (see §1.1.x), not via a `SectionRole` variant.
3. `MachineEffect` is the full algebra in `gbf-asm/src/effect.rs`. The variants relevant to F-A5 are: `StoreToMbcRegister { reg: MbcRegisterClass }` (forbidden in every F-A5 section — banking is the shipped F-A4 module's exclusive domain, gated by `mbc_write_provenance_audit`), `StoreToVram` / `StoreToOam` (emitted only by `video_commit`, plus `panic` under the audit-exempt annotation), `LoadFromBank0` / `LoadFromHram` / `LoadFromWram` / `StoreToWram` / `StoreToHram` (used freely by the runtime modules), `LoadFromIo { register }` / `StoreToIo { register }` (used by joypad, video_commit drain mode read, scheduler TIMA arming, and panic LCDC writes), `InterruptControl(InterruptControlOp)` (used by ISR `RETI`, scheduler `EI`/`DI`/`HALT`, and panic `DI`), `Call` / `Return` / `Reti` / `UnconditionalBranch` / `ConditionalBranch` (control flow), `SystemCall(SystemCallKind)` (the lowering seam consumed by `Yield` / `BankLease` / `TraceProbe` / `AssertBank` / `BankRelease` / `FarCall`).
4. `PrivilegeClass` is exactly three variants: `Normal`, `Privileged`, `InterruptHandler`. `SectionPrivilege` carries a `default_privilege` plus an `allow_overrides` set; constructors are `SectionPrivilege::normal()`, `SectionPrivilege::privileged()`, `SectionPrivilege::interrupt_handler()`. F-A5's ISR stubs and handlers use `interrupt_handler()`. The video_commit drain emits `StoreToVram`/`StoreToOam` (Normal-class effects on hardware) and so its section is `normal()`; the *audit* that "only video_commit and panic emit those effects" is enforced by the ExecutionContext annotation introduced in §1.1.x and the closure-time effect/provenance walk in §4.6, not by `PrivilegeClass`. Panic uses `privileged()` (it issues `DI`) plus `ExecutionContext::PanicOnly` plus the `PanicBypass` audit tag.
5. `gbf-asm::layout` and `gbf-asm::rom::assemble_rom` own bank placement and ROM byte assembly. F-A5 emits sections; the layout pass places them. F-A5 does not call into the layout pass directly except in tests that need the encoded byte shape.
6. F-A1's cycle model exposes per-instruction M-cycle costs that F-A5 reads to assert ISR latency budgets at test time.
7. `PreLayoutOp` covers `BankLease(BankLeaseSpec)`, `BankRelease { lease_id }`, `Yield { kind: YieldKind }`, `TraceProbe { id: TraceProbeId }`, `AssertBank { expected: MbcBankClass, expected_n: u16 }`. F-A5 emits `Yield` from `emit_yield_check` and `AssertBank` at section boundaries that need a runtime bank invariant; `BankLease`/`BankRelease` flow through `gbf-runtime::banking` (F-A4) and never appear in F-A5 modules directly.

**`gbf-hw` (F-A2 — RFC drafted, open):** F-A5 depends on F-A2's surfaces:

- `gbf-hw::interrupts::{INT_VECTOR_VBLANK = 0x0040, INT_VECTOR_LCD_STAT = 0x0048, INT_VECTOR_TIMER = 0x0050, INT_VECTOR_SERIAL = 0x0058, INT_VECTOR_JOYPAD = 0x0060, IE_REGISTER = 0xFFFF, IF_REGISTER = 0xFF0F, DIV_REGISTER = 0xFF04, TIMA_REGISTER = 0xFF05, TMA_REGISTER = 0xFF06, TAC_REGISTER = 0xFF07}` and `InterruptSource::{VBlank, LcdStat, Timer, Serial, Joypad}` plus `vector_for(source)`, `ie_bit(source)`, `if_bit(source)`.
- `gbf-hw::lcd::{PpuMode::{HBlank, VBlank, OAMSearch, Drawing}, vram_accessible_in(mode), oam_accessible_in(mode), LCDC_REGISTER = 0xFF40, STAT_REGISTER = 0xFF41, LY_REGISTER = 0xFF44, VBLANK_LY_THRESHOLD = 144}` and the BG/window/OAM register addresses.
- `gbf-hw::joypad::{JOYP_REGISTER = 0xFF00, Button, ButtonState, JOYP_BIT_SELECT_BUTTONS, JOYP_BIT_SELECT_DIRECTIONS, just_pressed, just_released}`.
- `gbf-hw::timing::{FRAME_M_CYCLES = 17556, VBLANK_M_CYCLES = 1140, FRAMES_PER_SECOND ≈ 59.7, DOT_CLOCK_HZ}`.
- `gbf-hw::memory::{ROM_BANK0_BASE..=ROM_BANK0_END, WRAM_BASE..=WRAM_END, HRAM_BASE..=HRAM_END, VRAM_BASE..=VRAM_END, OAM_BASE..=OAM_END}` plus the predicates (`is_rom_bank0`, `is_wram`, `is_hram`, `is_isr_resident_legal_dmg`).

F-A2 is a hard dependency for F-A5 implementation. F-A5 must not declare local hardware constants. If F-A2 is not ready, F-A5 remains in Draft or lands only non-executable skeletons that do not encode hardware addresses.

**`gbf-abi` (F-A3 — RFC drafted, open):** F-A5 depends on F-A3's surfaces:

- `gbf-abi::version::{AbiVersion, BuildIdentityBlock}`. F-A5's boot section includes a `BuildIdentityBlock` placeholder at a known ROM offset; the four lineage hashes are filled by the backend at link time.
- `gbf-abi::continuation::{InferenceState, LivenessCounters}`. F-A5's scheduler allocates the prefix in WRAM and updates `LivenessCounters::progress_epoch`/`last_checkpoint`/`no_progress_frames` per slice.
- `gbf-abi::fault::{FaultCode, FaultDomain, FaultSnapshot}`. F-A5's panic raises `FaultCode::*`; F-A5's scheduler raises `FaultCode::LivenessTimeout` / `FaultCode::RepeatedCheckpointNoProgress` / `FaultCode::UiCommitOutsideLegalMode`.
- `gbf-abi::interrupt::{InterruptPolicy, ResourceLease}`. F-A5's scheduler uses `InterruptPolicy::Enabled` by default; `ShortCriticalSection` is reserved for narrow critical regions.
- `gbf-abi::checkpoint::{SemanticCheckpointId, CompactCheckpointId}`. F-A5's scheduler reads `LivenessCounters::last_checkpoint` (a `CompactCheckpointId`) and uses the schema (consumed via F-A3's adapter trait) to map back to durable ids when a fault is raised.

F-A3 is a hard dependency for F-A5 implementation. F-A5 must not declare local placeholder ABI types for `InferenceState`, `FaultCode`, `BuildIdentityBlock`, or checkpoint IDs. The Epic A ordering is F-A1 → F-A2 → F-A3 → F-A4 → F-A5; F-A1, F-A2, F-A3, and F-A4 have all shipped (commits `7a5c687`, `a69c2e2`, `6ad156c`, `6feae98`).

**`gbf-runtime/src/banking.rs` (F-A4 — shipped, bead `bd-1sv` closed):** F-A5 makes no edits. F-A5 imports a small read-only surface from `crate::banking` per §0.0.6: the `lower_banking_shadow_zero_init(b)` call from `boot::runtime_boot_entry`; the `HRAM_SHADOW_BASE` / `HRAM_BANKING_SHADOW_END_EXCLUSIVE` constants for `static_assert!` adjacency to F-A5's `HRAM_ADDR_YIELD_REQUESTED = $FF84`; the `InterruptSafetyKind` / `InterruptSafetyTable` / `mark_isr*` declaration helpers consumed by `build_bank0_nucleus_sections`. F-A5's runtime modules emit zero `MachineEffect::StoreToMbcRegister` instructions (verified by §4.6), so no lease helper is invoked. Per F-A4's known-debt, F-A5 also does **not** ship the `KeepCurrentProof` producer or the `LeaseLifetime::ResumeWindow`/`Token` lowering (see §1.2).

### 2.2 What is stubbed

All eight reference-shell modules in `gbf-runtime/src/`. There is no test file under `gbf-runtime/tests/`, no `examples/`, and no integration coverage. F-A5 fills the eight stubs, adds a `gbf-runtime/tests/` directory with one integration test per module plus a Bank-0-budget assertion test plus a yield-round-trip test, and updates `Cargo.toml` to add `static_assertions` and (optionally) `gameroy` as a `[dev-dependencies]` entry.

### 2.3 Downstream pressure on this design

```
gbf-runtime  ──▶ gbf-codegen     (composes Bank 0 sections; emits BuildIdentityBlock; allocates InferenceState in WRAM; emits compiler-side yield checks via emit_yield_check)
              ──▶ gbf-bench       (calibrates runtime_nucleus_hash; produces RuntimeCalibrationBundle.scheduler_overheads, .overlay_install_cost, .trace_overheads)
              ──▶ gbf-emu         (parses the cartridge boot sequence, IRQ vectors, BuildIdentityBlock; runs the scheduler in deterministic mode)
              ──▶ gbf-debug       (steps through the boot sequence, pokes joypad state, snapshots after IRQs, reads HRAM yield_requested, parses fault codes from the panic screen)
              ──▶ gbf-policy      (consumes RuntimeShellModule annotations to compute RuntimeChromeBudget slot capacity)
              ──▶ gbf-train       (preflights against runtime_nucleus_hash; halts on drift via T2.5)
              ──▶ gbf-report      (RunManifest cites runtime_nucleus_hash; FailureCapsule includes fault_code, last_checkpoint)
```

Every consumer assumes:

- The Bank 0 nucleus encodes deterministically. Two clean checkouts of the same source produce byte-identical Bank 0 ROM. `runtime_nucleus_hash` is therefore a content-hash, not a sample.
- ISR latency is bounded. The `SchedulerPolicy::max_interrupt_latency` is honored across all ISR stubs and handlers; deeper critical sections require `InterruptPolicy::ShortCriticalSection` annotations that the policy layer cross-checks.
- VRAM/OAM commits never happen during `PpuMode::Drawing`. The `video_commit` drain checks the mode at every dequeue.
- Liveness accounting is non-Option. Every slice bumps `progress_epoch` *or* contributes to `no_progress_frames`. There is no "untracked" slice variant.
- The reference shell module set is exactly `{Boot, Interrupts, Scheduler, Banking, Joypad, Text, Keyboard, VideoCommit}`. Adding a module after the M0 closure bumps `runtime_nucleus_hash` deliberately and goes through the T2.5 promotion path.

### 2.4 Engineering-rule grounding (`planv0.md` §"Engineering rules")

This RFC threads the rules tightly:

- **Rule 1** ("All generated executable code originates from `AsmIR` / `Instr` / audited runtime builders, never from ad hoc byte pushes"). Every byte F-A5 emits comes through F-A1's `Builder` eDSL. The font asset is the one explicit `Db` data block; even there, the loading is a `Section::data` invocation, not a byte push.
- **Rule 5** (deterministic, hashed ROM builds). F-A5 ships a pinned `build_bank0_nucleus_sections() -> Vec<Section>` whose order, pseudo-op choices, and font asset are all stable across runs. `runtime_nucleus_hash` is computable from the encoded result.
- **Rule 6** (`no_std + alloc` where practical). F-A5 is *capable* of `no_std + alloc` (the `Vec<UiCommitOp>` queue is `alloc`-shaped; nothing else needs `std::collections`); the practical declaration is deferred behind `gbf-foundation`'s and `gbf-asm`'s `no_std` conversions. The source uses no `std::collections::HashMap`, no `std::sync::*`.
- **Rule 7** (Bank 0 budget). F-A5's Bank 0 nucleus, encoded, must fit under 16 KiB minus the `FutureReservation`-table ROM bytes. The test `nucleus_fits_bank0_budget` enforces this; bring-up estimate is ~4.7 KiB used by F-A5 modules, plus ~1.4 KiB future reservations, leaving ~10.2 KiB free for the `Bank0Free` slot (see §3H).
- **Rule 11** (single source of truth). `video_commit` is the sole VRAM/OAM writer. The invariant is enforced by the `gbf-asm` `MachineEffect` typing plus the `ExecutionContext` annotation introduced in §1.1.x; only `video_commit` and (audited) `panic` emit `MachineEffect::StoreToVram` / `MachineEffect::StoreToOam`.
- **Rule 12** (`unsafe` is forbidden by default). F-A5 modules use `#![forbid(unsafe_code)]` at the crate root; no module body needs `unsafe`.
- **Rule 14** (cooperative yielding is a compiler feature, not a hope). F-A5's scheduler uses TIMA-armed soft deadlines + HRAM-flag polling at compiler-emitted safe points. The yield mechanism is typed (`YieldKind` consumed from `gbf-asm::section::YieldKind`); raw `JR` to `scheduler_resume` is not a legal yield. The `max_safe_point_gap_m_cycles` proof is owned by Epic B's lowering work.
- **Rule 15** (ISR-residency rule). All ISR stubs and handlers F-A5 emits are `Bank0/HRAM/fixed-WRAM` only. The annotation is `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler` per §3.3 (the latter is introduced by §1.1.x); the whole-program validation is `ReachabilityValidation` per Epic B.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction).** `UiCommitOp` is an exhaustive enum (`PutGlyphCell`, `FillGlyphRun`, `SetDmgPalette`, `PutOamSprite`); `UiCommitOpKind` is the variant-only sibling used by `UiCommitPlan::vblank_priority_ops`. `SpecialKey` is exhaustive (`Backspace`, `Submit`, `Shift`, `Cancel`; the latter two are reserved variants not in the M0 default layout). `YieldKind` is consumed from `gbf-asm::section`. The illegal-mode-write case becomes a typed `FaultCode::UiCommitOutsideLegalMode`; queue capacity exhaustion becomes the new `FaultCode::UiCommitQueueFull` variant introduced by §1.1.x — not silent corruption, and not the same fault as the illegal-mode write.
- **§III (shift left).** Every assertion that can run at compile time runs at compile time: section role typing is checked by F-A1's builder; ISR-residency annotation is a `static_assert` in tests; Bank 0 budget assertion is a test that fails the build, not a runtime check.
- **§IV.3 (reproducible builds).** The deterministic section order in `build_bank0_nucleus_sections()` makes `runtime_nucleus_hash` reproducible. Two clean checkouts produce identical hashes; the T2.5 CI gate enforces this.
- **§V.3 (silence on success, loud on failure).** Liveness violations raise typed `FaultCode::LivenessTimeout` / `FaultCode::RepeatedCheckpointNoProgress`. UI commit overflow with no legal mode raises `FaultCode::UiCommitOutsideLegalMode`. The panic screen renders the fault code as on-screen glyphs; not silent.
- **§VI.1 (single source of truth).** `video_commit` is the only VRAM/OAM writer. The `text`/`keyboard` modules cannot bypass it because the `Builder` context they receive does not have access to `MachineEffect::StoreToVram`-emitting helpers.

### 2.6 Pan Docs as the primary specification

Every register read and IRQ vector address F-A5 references comes from F-A2 (which in turn cites Pan Docs). F-A5 itself references Pan Docs only for the *behavioral* contracts: PPU mode/accessibility (`planv0.md` line 117 echoes Pan Docs §"PPU Modes"), the timer/TIMA wrap behavior, the LCD enable caveats (do not disable outside VBlank), the joypad active-low decode, and the `HALT` semantics (`planv0.md` line 2348). Where Pan Docs and the gekkio CPU manual disagree (rare; primarily on `HALT` edge cases), Pan Docs wins because that is the document the rest of the workspace cites.

The two exceptions where this RFC deliberately deviates from Pan Docs:

1. **`HALT` in the scheduler, not in inference slices.** Pan Docs notes that `HALT` wakes when an interrupt is pending, with caveats when `IME=0`. F-A5 puts `HALT` exclusively in `scheduler.rs::idle_until_next_frame` — the scheduler is the only context where `HALT` has unambiguous semantics (interrupts enabled, predictable wake source). Inference slices never `HALT`; they yield through the `yield_requested` flag.
2. **`STOP` is unused.** Pan Docs documents `STOP` for low-power mode + speed switching. F-A5 does not use `STOP`; the `Joypad` ISR exists only to satisfy the IRQ vector layout requirement, not to implement STOP-mode wake. The `Joypad` ISR handler is a no-op (it just IF-acknowledges and returns). A future power-management feature can fill it in.

### 2.7 Relationship to other M0 features

```
                        ┌─────────────────────────┐
                        │  F-A5: Bank0 Runtime    │   ← this RFC
                        │  (boot, ISRs, scheduler,│
                        │   UI, video_commit,     │
                        │   panic)                │
                        └────────────┬────────────┘
                                     │
        ┌────────────────────────────┼────────────────────────────┐
        ▼                            ▼                            ▼
┌─────────────────┐         ┌─────────────────┐         ┌─────────────────┐
│  F-A1: gbf-asm  │         │  F-A2: gbf-hw   │         │  F-A3: gbf-abi  │
│  (Builder eDSL, │         │  (IRQ vectors,  │         │  (InferenceState│
│   Section,      │         │   PpuMode, JOYP,│         │   LivenessCntrs,│
│   Privilege,    │         │   timing,       │         │   FaultCode,    │
│   MachineEffect)│         │   memory map)   │         │   InterruptPol.)│
└─────────────────┘         └─────────────────┘         └─────────────────┘

                        ┌─────────────────────────┐
                        │  F-A4: BankLease ABI    │   (shipped commit 6feae98;
                        │  (gbf-runtime::banking) │    F-A5 consumes a small
                        │                         │    read-only surface — see §0.0.6)
                        └─────────────────────────┘
```

F-A5 is a *consumer* of F-A1, F-A2, F-A3, and F-A4. It is the *producer* for `gbf-codegen` (which composes its sections), `gbf-bench` (which calibrates against its hash), `gbf-emu` (which runs its boot sequence), `gbf-debug` (which scripts its execution), `gbf-policy` (which consumes its `RuntimeShellModule` annotations), and `gbf-train` (which pins its `runtime_nucleus_hash`).

Inside Epic A, F-A5 closes after F-A1, F-A2, F-A3, and F-A4 — all four upstream Epic A features have shipped (`7a5c687`, `a69c2e2`, `6ad156c`, `6feae98`). F-A5's interaction with `crate::banking` is read-only and small (see §0.0.6); no follow-up "lease-promotion" PR is queued because F-A5's reference-shell sections are Bank0-resident by construction and emit zero `MachineEffect::StoreToMbcRegister` instructions.

### 2.8 Beads under this feature

The seven child tasks under `bd-2r1` are:

| Bead     | Task     | Module(s)                        | Priority |
|----------|----------|----------------------------------|----------|
| `bd-17b` | T-A5.1   | `boot.rs`, `interrupts.rs`       | P0       |
| `bd-1cv` | T-A5.2   | `scheduler.rs`                   | P0       |
| `bd-fcm` | T-A5.3   | `joypad.rs`                      | P2       |
| `bd-3ys` | T-A5.4   | `text.rs` + `assets/font_8x8.bin`| P1       |
| `bd-t0y` | T-A5.5   | `keyboard.rs`                    | P2       |
| `bd-1d2` | T-A5.6   | `video_commit.rs`                | P0       |
| `bd-15y` | T-A5.7   | `panic.rs`                       | P2       |

T-A5.1 (boot + IRQ dispatch) blocks every other task because every other module references the IRQ vectors or the HRAM shadow region zeroed at boot. T-A5.6 (`video_commit`) blocks T-A5.4 (text) and T-A5.5 (keyboard) and is referenced by T-A5.7 (panic) for the audited bypass annotation. T-A5.2 (scheduler) depends on T-A5.1 and on T-A2.4's frame timing constants. T-A5.3 (joypad) depends on T-A5.1 and T-A2.6. T-A5.5 (keyboard) depends on T-A5.3 and T-A5.4. T-A5.7 (panic) depends on T-A5.1, T-A5.4 (so the panic screen can render glyphs), and T-A3.4 (FaultCode).

The order matters within the PR's commit history (the modules build cleanly when added in dependency order); it does not change the closure shape, which is one PR closing every task.

## 3. Architecture

### 3.1 Crate-level shape

`gbf-runtime` is a *builder* crate. Each module exports `pub fn build_*_section() -> Section` plus a small set of helper builders (`emit_*<B: Builder>(b: &mut B, ...)`). The entire public surface decomposes into:

1. **Section builders** — `pub fn build_*_section() -> Section`. Pure functions; no IO.
2. **AsmIR-emitting helpers** — `pub fn emit_*<B: Builder>(b: &mut B, ...)`. Generic over the F-A1 `Builder` trait.
3. **Value objects** — `Copy + Clone + Debug + Eq + PartialEq + Hash` plain structs, plus `Serialize`/`Deserialize` where the type is read by host tooling (`SchedulerPolicy`, `UiCommitPlan`, `KeyboardLayoutSpec`, `TextLayout`).
4. **Const constants** — section sizes, font asset bytes, default layout dimensions.
5. **Module-level annotations** — each `build_*_section()` carries a `RuntimeShellModule::*` tag the policy layer reads.

There is *no* runtime state. There are *no* mutable globals. There are *no* IO entry points. The crate root uses `#![forbid(unsafe_code)]`. The source is `no_std + alloc`-ready in shape (the `Vec<UiCommitOp>` is the one `alloc`-typed field; everything else is `Copy`); declaring `#![no_std]` is deferred until `gbf-foundation` and `gbf-asm` are themselves `no_std`.

### 3.2 Module responsibility table

| Module           | Owns                                                                                                       | Public surface |
|------------------|------------------------------------------------------------------------------------------------------------|----------------|
| `boot.rs`        | `build_boot_section`, `build_irq_vectors_section`, `build_isr_stubs_section`, `BootInitPolicy`             | ~12 items      |
| `interrupts.rs`  | ISR handler builders (5 of them), `emit_if_acknowledge`, `emit_save_regs`/`emit_restore_regs`              | ~12 items      |
| `scheduler.rs`   | `build_scheduler_section`, `emit_yield_check`, `SchedulerPolicy`, `emit_arm_tima`, `emit_idle_until_frame` | ~14 items      |
| `joypad.rs`      | `build_joypad_section`, `emit_joypad_read`, `JOYPAD_CACHED_STATE_ADDR`, `JOYPAD_PREV_STATE_ADDR`           | ~6 items       |
| `text.rs`        | `build_text_section`, `font_bytes`, `FONT_TILE_COUNT`, `TextLayout`, `emit_text_print_glyph`, `emit_text_clear_row` | ~10 items |
| `keyboard.rs`    | `build_keyboard_section`, `KeyboardLayoutSpec<'a>`, `KeyboardLayoutManifest`, `KeyboardCell`, `KeyboardState`, `SpecialKey`, `emit_keyboard_step`, M0 prompt buffer constants | ~16 items |
| `video_commit.rs`| `build_video_commit_section`, `UiCommitPlan`, `UiCommitOp`, `UiCommitOpKind`, `UiCommitWireOp`, `emit_queue_op`, `emit_commit_drain_hblank`, `emit_commit_drain_vblank`, `emit_bootstrap_vram_init`, `COMMIT_QUEUE_BASE_ADDR`, `COMMIT_QUEUE_LEN`, `UI_COMMIT_WIRE_OP_BYTES` | ~18 items |
| `panic.rs`       | `build_panic_section`, `emit_panic`, `emit_panic_screen_render`, `WRAM_LAST_FAULT_ADDR`                    | ~6 items       |
| `lib.rs` (additions) | `RuntimeShellModule` annotation emission using the shared enum from `gbf-abi` or `gbf-foundation`, `build_bank0_nucleus_sections` (returning `(Vec<Section>, InterruptSafetyTable)` so each section is declared via F-A4's `mark_isr*` helpers at construction time), `runtime_nucleus_section_order`, `compute_runtime_nucleus_hash`, F-A5-owned HRAM constants (`HRAM_ADDR_YIELD_REQUESTED`, `HRAM_LDH_YIELD_REQUESTED`, plus `static_assert!` adjacency to `gbf_runtime::banking::HRAM_BANKING_SHADOW_END_EXCLUSIVE`) | ~10 items |

**Address naming rule:**

- `*_ADDR` means an absolute CPU address, e.g. `$C100`.
- `*_OFFSET` means an offset inside an address space, e.g. `$05` for `LDH ($05), A`.
- HRAM byte locations are represented as raw `u16` absolute addresses (`HRAM_ADDR_*`) and matching `u8` LDH offsets (`HRAM_LDH_*`), mirroring the convention F-A4 already uses for the banking shadow (`HRAM_ADDR_CURRENT_ROM_BANK_LO`, `HRAM_LDH_CURRENT_ROM_BANK_LO`, etc.). No `HramOffset` newtype is introduced; the F-A4-shipped flat-constant style is the workspace convention.
- WRAM byte locations are represented as `WramAddr` (an absolute address; `LDH` is illegal).

Total public surface: ~96 items, each one anchored to a `planv0.md` line or a Pan-Docs-derived hardware fact.

### 3.3 Section roles, privilege classes, and the new audit annotations

The shipped F-A1 `PrivilegeClass` enum is exactly three variants — `Normal`, `Privileged`, `InterruptHandler`. ISR-vs-panic context and IME state are **not** `PrivilegeClass` variants; F-A5 introduces them as new typed annotations on `Section` (or, on the alternative path, in `gbf-runtime::audit::SectionAudit`; see §1.1.x). Every emitted section therefore carries:

1. `SectionRole` — placement / reachability class. The shipped F-A1 enum is the 13-variant set listed in §2.1; F-A5 uses `Bank0Nucleus` for executable sections, `Bank0Data` for the font / keyboard layout, `HramFastFlags` for `frame_count`-style HRAM scratch, and `HeaderCartridge` for the cartridge header sub-section.
2. `PrivilegeClass` — `Normal | Privileged | InterruptHandler` (from F-A1, unchanged). Decides which `MachineEffect` variants the section's builder may emit.
3. `ExecutionContext` — `Normal | InterruptHandler | PanicOnly | VideoCommitOnly` (introduced by F-A5 in §1.1.x). Carries the VRAM-audit context the privilege class alone does not capture.
4. `InterruptDiscipline` — `Default | ImeDisabled` (introduced by F-A5 in §1.1.x). `Default` means "this section runs with whatever IME state the caller had"; `ImeDisabled` is a structural promise that `DI` has been issued and `EI`/`RETI` will be the only re-enable.
5. `PanicBypass` — a one-bit audit tag on the same struct, set only on the `panic` section. `ReachabilityValidation` (Epic B Stage 12) recognizes it as the sole exemption from the no-direct-VRAM rule.
6. `InterruptSafetyKind` (from F-A4: `InterruptDisabled | InterruptEnabledBank0Only | InterruptHandler`). F-A5 declares each section into the F-A4-shipped `InterruptSafetyTable` via `mark_isr` / `mark_isr_reachable` / `mark_isr_unreachable`. This is orthogonal to `ExecutionContext`: it answers "is this section reachable while interrupts are enabled, and does it run as an ISR" rather than "is this section the audited VRAM writer". Both annotations live on every section; Epic B's `ReachabilityValidation` reads both.

| Module          | `SectionRole`     | `PrivilegeClass`        | `ExecutionContext`  | `InterruptDiscipline` | `InterruptSafetyKind` (F-A4) | `PanicBypass` | Notable `MachineEffect` set |
|-----------------|-------------------|-------------------------|---------------------|-----------------------|------------------------------|---------------|-----------------------------|
| `boot`          | `Bank0Nucleus`    | `Privileged`            | `Normal`            | `Default`             | `InterruptEnabledBank0Only`  | no            | `StoreToIo` (LCDC/IE/IF init), `InterruptControl` |
| `irq_vectors`   | `Bank0Nucleus`    | `Normal`                | `Normal`            | `Default`             | `InterruptEnabledBank0Only`  | no            | `UnconditionalBranch` only (jump table) |
| `isr_stubs`     | `Bank0Nucleus`    | `InterruptHandler`      | `InterruptHandler`  | `ImeDisabled`         | `InterruptHandler`           | no            | `StoreToStack`, `LoadFromStack`, `Call`, `Reti` |
| `interrupts` (handlers) | `Bank0Nucleus` | `InterruptHandler` | `InterruptHandler` | `ImeDisabled`         | `InterruptHandler`           | no            | per-handler (e.g., `StoreToHram` for yield_requested) |
| `scheduler`     | `Bank0Nucleus`    | `Privileged`            | `Normal`            | `Default`             | `InterruptEnabledBank0Only`  | no            | `StoreToIo` (TIMA arming), `InterruptControl` (`HALT`/`EI`/`DI`) |
| `joypad`        | `Bank0Nucleus`    | `Normal`                | `Normal`            | `Default`             | `InterruptEnabledBank0Only`  | no            | `LoadFromIo`/`StoreToIo` (JOYP), `StoreToWram` (cache) |
| `text`          | `Bank0Nucleus`    | `Normal`                | `Normal`            | `Default`             | `InterruptEnabledBank0Only`  | no            | `StoreToWram` (queue enqueue) only — never `StoreToVram` |
| `keyboard`      | `Bank0Nucleus`    | `Normal`                | `Normal`            | `Default`             | `InterruptEnabledBank0Only`  | no            | `StoreToWram` (queue enqueue + prompt buffer) only |
| `video_commit`  | `Bank0Nucleus`    | `Normal`                | `VideoCommitOnly`   | `Default` for enqueue / `ImeDisabled` for drain (called from ISRs) | `InterruptEnabledBank0Only` (drain is invoked from ISRs but classified by its own residency) | no | **`StoreToVram`, `StoreToOam`** (sole writer) |
| `panic`         | `Bank0Nucleus`    | `Privileged`            | `PanicOnly`         | `ImeDisabled`         | `InterruptDisabled`          | **yes**       | **`StoreToVram`** (audited bypass), `StoreToIo` (LCDC), `InterruptControl` (`DI`/`HALT`) |

`ReachabilityValidation` (Epic B Stage 12) consumes these annotations and proves: (a) every ISR stub and handler is reachable only from Bank 0 / HRAM / fixed-WRAM; (b) `MachineEffect::StoreToVram` and `MachineEffect::StoreToOam` only appear in code reachable from `video_commit`'s drain entries or from a section carrying `PanicBypass` (i.e., `panic`); (c) no module emits `MachineEffect::StoreToMbcRegister` outside the shipped F-A4 banking module — a property already enforced locally by F-A4's `mbc_write_provenance_audit` (which rejects MBC writes lacking `BankingPreLayoutLowering` provenance).

### 3.4 The "video_commit is the sole VRAM writer" enforcement

The architectural commitment that `video_commit` is *the* place for VRAM/OAM writes is enforced by three mechanisms:

1. **`ExecutionContext::VideoCommitOnly` + `MachineEffect::StoreToVram` typing.** The `video_commit::emit_commit_drain_*` helpers are the only sites that actually call `Builder::emit(Instr::Ld...)` against a VRAM/OAM address; the `text` and `keyboard` builders only invoke `video_commit::emit_queue_op(b, UiCommitOp::PutGlyphCell { ... })`. The audit-walk test (§4.6) reads the `ExecutionContext` annotation off each emitted section and asserts that any section observed emitting `StoreToVram` / `StoreToOam` is annotated `VideoCommitOnly` (or carries `PanicBypass`).
2. **`ReachabilityValidation` cross-section check.** Epic B Stage 12 walks the call graph and confirms: `MachineEffect::StoreToVram` originates only from sections annotated `ExecutionContext::VideoCommitOnly` (the `video_commit` drain) or sections additionally carrying the `PanicBypass` audit annotation (the `panic` exception).
3. **`grep_no_direct_vram_writes` integration test** (in `gbf-runtime/tests/single_writer_smoke.rs`, `#[ignore]`d until `gbf-test` lands). Walks each of the eight reference-shell module source files and greps for raw `Ld` instructions with VRAM addresses (`0x8000..=0x9FFF`) or OAM addresses (`0xFE00..=0xFE9F`). Allowlist: `video_commit.rs`, `panic.rs`. Any other source file matching is a test failure.

### 3.5 Why the reference shell is exactly this set

The pinned reference shell per T2.4 (`bd-37r`) is `{Boot, Interrupts, Scheduler, Banking, Joypad, Text, Keyboard, VideoCommit}`. F-A5 ships the seven F-A5-owned modules (Banking is F-A4) plus a minimal `Panic`. Why exactly these:

- **Boot, Interrupts, Scheduler.** Without these, nothing runs. Boot wires the cartridge header and IRQ vectors; interrupts are required for cooperative scheduling; the scheduler is the entry point for inference dispatch.
- **VideoCommit.** Without this, *every* module fights for VRAM access. The single-writer rule exists from day one; the cost of retrofitting it later (refactoring text and keyboard to stage commits instead of writing directly) is the entire reason it is in the reference shell.
- **Joypad, Text, Keyboard.** The deliverable for M0 is "a ROM that boots, draws text, accepts keyboard input." Removing any of these three means M0 cannot demonstrate the runtime-nucleus loop.
- **Banking.** F-A4 (shipped). Required because every cross-bank read (e.g., reading a glyph from a CommonBank if text is moved out of Bank 0) goes through `BankLease`. F-A5 does not implement banking but coexists with it: F-A5's boot calls `lower_banking_shadow_zero_init`, F-A5's `HRAM_ADDR_YIELD_REQUESTED = $FF84` is anchored to `HRAM_BANKING_SHADOW_END_EXCLUSIVE`, and F-A5's per-section `InterruptSafetyKind` declarations populate the `InterruptSafetyTable` F-A4 ships.
- **Panic** (M0 compromise, not in T2.4's reference shell). T2.4 lists Panic as a `FutureReservation`. F-A5 ships a minimal panic for bring-up because faults must be visible from frame 1; the alternative (silent freeze) is not tractable to debug in M0.

The three modules deferred to Epic D (`Persistence`, `Trace`, `Harness`) are explicitly in T2.4's `FutureReservation` table:

| Module       | `rom_bytes_per_bank0` | `wram_bytes` | `sram_bytes` | Owner    |
|--------------|----------------------:|-------------:|-------------:|----------|
| `Persistence`|                   768 |           64 |          512 | F-D1     |
| `Trace`      |                   256 |            0 |         4096 | F-D3     |
| `Harness`    |                   384 |            0 |          256 | F-D2     |
| `Panic`      |                   256 |            8 |            0 | F-A5 M0 + F-D5 follow-up (full `FaultSnapshot`/SRAM persist) |

The sum of `rom_bytes_per_bank0` (1664) is subtracted from Bank 0's 16 KiB capacity by the `RuntimeChromeBudget` emitter to compute `Bank0Free`. The bring-up estimate of ~8 KiB used + ~8 KiB free for `Bank0Free` is consistent with this subtraction.

### 3.6 What `gbf-runtime` deliberately does not own

- **Cycle costs per `Instr`.** That is `gbf-asm::cycle_model` (F-A1, shipped). F-A5 ships the M-cycle costs of *its own helper sequences* (e.g., the ISR save-regs prologue) by composing F-A1's cycle model.
- **`AsmIR` / `Instr`.** `gbf-asm::isa` (F-A1, shipped).
- **Hardware constants** (memory map, IRQ vectors, LCD modes, JOYP layout). `gbf-hw` (F-A2). F-A5 imports; it never re-declares.
- **`InferenceState` / `LivenessCounters` / `FaultCode` / `BuildIdentityBlock`.** `gbf-abi` (F-A3). F-A5 reads/writes; it never re-declares.
- **`BankLease`/`BankGuard` ABI.** `gbf-runtime::banking` (F-A4, shipped). F-A5 consumes only `lower_banking_shadow_zero_init`, the read-only HRAM banking-shadow constants, and the `InterruptSafetyTable` declaration helpers from this module; it does not author it and does not acquire leases.
- **`SchedulePack` and runtime mode switching.** Owned by Epic B. F-A5 ships only the `Default` runtime mode; mode-switch points are pinned `SemanticCheckpointId`s consumed via F-A3.
- **The compile-time slice contract.** `GbSchedIR`'s `SchedSlice`, `ResourceLease`, `ResourceVector` (Epic B Stage 10). F-A5's scheduler reads the `InferenceState` continuation; the slice IR is the compiler's contract.
- **Persistent SRAM record protocol, trace ring buffer, harness control plane.** F-D1, F-D3, F-D2.
- **`gbf-emu` adapter, `gbf-debug` scripted CLI.** F-A7, F-A8.
- **Concrete calibration bundles.** F-E2, F-E4. F-A5 ships only the `runtime_nucleus_hash` they index by.

The boundary between F-A5 and these crates is enforced by the dependency graph: `gbf-runtime` depends on `gbf-abi`, `gbf-asm`, `gbf-foundation`, `gbf-hw` only; everything else depends on `gbf-runtime` (or on its sibling Epic A crates). There is no path back the other way.

## 3A. Boot + IRQ dispatch (T-A5.1, `boot.rs` + `interrupts.rs`)

**Reference**: `planv0.md` line 1626 (Bank 0 nucleus content list), line 1646 (boot/header/vectors as part of nucleus); F-A2 T-A2.5 (interrupt vector layout).

### 3A.1 Why this module exists

Boot is the first executable code on the cartridge. Without it, the CPU hits the cartridge entry stub at `$0100` and falls through into the header bytes. Boot wires:

- The cartridge entry stub at `$0100..=$0103` (`NOP; JP runtime_boot_entry`), where `runtime_boot_entry` is placed *after* the header bytes (`$0104..=$014F`).
- The exact Nintendo logo bytes at `$0104..=$0133` (stamped by F-A1's `assemble_rom`; the boot ROM compares this region against an internal copy and halts on mismatch).
- The remaining cartridge header byte fields at `$0134..=$014F` (also stamped by F-A1's `assemble_rom`).
- The IRQ dispatch table at `$0040..=$0060` (five 8-byte slots; each contains a `JP <handler>` to the corresponding ISR stub in Bank 0).
- The `runtime_boot_entry` init sequence (placed after the header): zero HRAM shadow region, install bootstrap VRAM tile data via `video_commit::emit_bootstrap_vram_init` while LCD is off, configure `LCDC`/`STAT`/`IE`/`IF`, jump to `scheduler::main_loop`.
- ISR entry stubs in Bank 0 (one per IRQ source). Each stub is annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler`; it saves regs, calls the matching handler in `interrupts.rs`, restores regs, RETI.

### 3A.2 Public surface

```rust
/// Build the Bank 0 boot section: cartridge entry stub at $0100..=$0103 (NOP; JP runtime_boot_entry),
/// then the runtime_boot_entry init sequence placed after the header.
/// Cartridge header byte fields and the Nintendo logo are stamped by F-A1's assemble_rom.
pub fn build_boot_section() -> Section;

/// Build the IRQ dispatch section: five 8-byte slots at $0040..=$0060, each containing
/// a `JP <handler>` to the corresponding ISR stub in Bank 0.
pub fn build_irq_vectors_section() -> Section;

/// Build the ISR entry stubs section. Each stub:
///   1. Save regs (PUSH AF, BC, DE, HL).
///   2. Call into the corresponding handler in `interrupts.rs`.
///   3. Restore regs (POP HL, DE, BC, AF).
///   4. RETI.
/// Section is annotated PrivilegeClass::InterruptHandler + ExecutionContext::InterruptHandler.
pub fn build_isr_stubs_section() -> Section;

/// Init policy controls what the boot init sequence does. For M0 bring-up,
/// `BootInitPolicy::default()` invokes `gbf_runtime::banking::lower_banking_shadow_zero_init`
/// (F-A4) plus F-A5's own HRAM fast-flag zero-init (`HRAM_ADDR_YIELD_REQUESTED` and
/// adjacent scheduler/fault bytes), powers up the LCD with `LCDC_DEFAULT`, clears IF,
/// sets IE to allow VBlank/STAT/Timer/Joypad.
pub struct BootInitPolicy {
    /// When true, boot calls `banking::lower_banking_shadow_zero_init` (F-A4 helper that
    /// zeros the four banking shadow bytes at $FF80..=$FF83) and then the F-A5-owned
    /// HRAM fast-flag zero-init (which covers $FF84 onward, starting with HRAM_ADDR_YIELD_REQUESTED).
    pub zero_hram_shadow: bool,
    pub power_up_lcd: bool,
    pub default_ie_mask: u8,
    pub default_lcdc: u8,
}

impl BootInitPolicy {
    pub const fn default() -> Self { /* ... */ }
}
```

### 3A.3 Cartridge header + Nintendo logo + `BuildIdentityBlock`

The cartridge header bytes (`$0100..=$014F`) are emitted via F-A1's `assemble_rom`. F-A5's boot section does **not** place the full init sequence at `$0100`. The executable layout is:

```text
$0100..=$0103   cartridge entry stub: NOP; JP runtime_boot_entry
$0104..=$0133   exact Nintendo logo bytes stamped by the ROM assembler
$0134..=$014F   remaining cartridge header byte fields stamped by the ROM assembler
$0150..         runtime_boot_entry:
                  zero HRAM shadow
                  install bootstrap VRAM tile data while LCD is off
                  configure LCDC/STAT/IE/IF
                  JP scheduler::main_loop
```

The `NINTENDO_LOGO` constant and cartridge header byte fields come from `gbf-hw::cartridge_header` (per F-A2). The "logo" bytes must be the *exact* bytes the boot ROM compares against; this RFC does not use the word "placeholder" anywhere for those bytes.

The `BuildIdentityBlock` (per F-A3) is reserved at a known *linker symbol*, not at a hard-coded address in this RFC. The final offset is pinned by F-A3/linker metadata and is included in the runtime symbol map. If the block is placed near the boot entry, it must be behind an explicit jump or live after the boot code; it must never occupy the first instruction stream at `runtime_boot_entry` unless that entry is itself a jump over the data block. The four lineage hashes (`artifact_core_hash`, `lowering_hash`, `compile_request_hash`, `runtime_nucleus_hash`) are zeroed by F-A5 and are filled by the backend at link time.

### 3A.4 IRQ vector layout

Per F-A2 T-A2.5 / Pan Docs:

| Vector | Address | Source     | Handler module |
|--------|---------|------------|----------------|
| 0      | `$0040` | VBlank     | `interrupts::vblank_handler`     |
| 1      | `$0048` | LCD STAT   | `interrupts::lcd_stat_handler`   |
| 2      | `$0050` | Timer      | `interrupts::timer_handler`      |
| 3      | `$0058` | Serial     | `interrupts::serial_handler`     |
| 4      | `$0060` | Joypad     | `interrupts::joypad_handler`     |

Each vector slot is 8 bytes; the slot contents are a single `JP nn` instruction (3 bytes) plus `NOP` padding to the next slot boundary. The padding is intentional — the alternative (compressing the vectors into 3 bytes each and packing extra ISR code into the slack) is documented as a future optimization (`OPT-A5.1`) and is not pursued in M0.

### 3A.5 ISR stub layout

Each ISR stub in Bank 0 is the canonical save-regs / call-handler / restore-regs / RETI sequence:

```text
isr_<source>_stub:
    PUSH AF
    PUSH BC
    PUSH DE
    PUSH HL
    CALL <source>_handler
    POP HL
    POP DE
    POP BC
    POP AF
    RETI
```

The latency budget includes:

1. CPU interrupt dispatch to the vector: 5 M-cycles (Pan Docs §"Interrupts").
2. Vector `JP nn`: 4 M-cycles.
3. Stub save-regs prologue: `PUSH AF, BC, DE, HL` = 16 M-cycles.
4. Stub `CALL <source>_handler`: 6 M-cycles.

So the minimum latency from interrupt service to the first instruction of `<source>_handler` is about 31 M-cycles before the handler body begins. The matching teardown (`POP HL, DE, BC, AF` = 12 M-cycles plus `RETI` = 4 M-cycles) is accounted separately in the total ISR occupancy budget, *not* in the entry-latency budget. `SchedulerPolicy::max_interrupt_entry_latency_m_cycles` and `SchedulerPolicy::max_interrupt_total_occupancy_m_cycles` are separate scheduler policy fields; the deepest critical section in any handler must respect both.

### 3A.6 `interrupts.rs` handler bodies

The five handlers do the minimal viable work per IRQ source:

- **VBlank** — bump a `frame_count` byte in HRAM (drives the scheduler frame loop), then invoke the VBlank portion of `video_commit::emit_commit_drain_vblank`.
- **LCD STAT** — invoke `video_commit::emit_commit_drain_hblank` against the current `STAT_REGISTER` mode. The drain checks the mode bits and dequeues an op if legal; if no op is available or the mode is illegal, it returns immediately.
- **Timer** — set `HRAM[HRAM_ADDR_YIELD_REQUESTED] = 1`. This is the cooperative-yield trigger.
- **Serial** — no body. Reserved.
- **Joypad** — no body. Reserved (STOP-mode wake is deferred).

The CPU acknowledges the serviced interrupt by clearing the corresponding `IF` bit before entering the handler (Pan Docs §"Interrupts"). F-A5 handlers do **not** write `IF` as part of the normal ISR path. A narrow `emit_clear_pending_if_bit` helper exists in `interrupts.rs` solely for the rare case where software intentionally discards a pending interrupt request; that helper preserves unrelated IF bits via `AND`/`OR` masking, never via raw store.

All handlers run with IME disabled (the CPU disables IME on IRQ entry; RETI re-enables). Each handler is small enough that `max_interrupt_total_occupancy_m_cycles` is comfortably honored.

### 3A.7 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- boot::cartridge_header_layout         # Boot section produces correct bytes at $0100-$014F
cargo test -p gbf-runtime -- boot::irq_vector_jumps                # Vectors at $0040-$0060 jump to the right stubs
cargo test -p gbf-runtime -- boot::shadow_registers_zeroed_at_init # Init sequence zeros HRAM_SHADOW_BASE..HRAM_FAST_FLAGS_END
cargo test -p gbf-runtime -- interrupts::isr_stubs_are_isr_marked  # PrivilegeClass::InterruptHandler + ExecutionContext::InterruptHandler annotation
cargo test -p gbf-runtime -- interrupts::isr_entry_latency_under_policy_bound
cargo test -p gbf-runtime -- interrupts::isr_total_occupancy_under_policy_bound
cargo test -p gbf-runtime -- interrupts::handlers_do_not_clobber_unrelated_if_bits
cargo test -p gbf-runtime -- interrupts::vblank_handler_bumps_frame_count
cargo test -p gbf-runtime -- interrupts::timer_handler_sets_yield_requested
```

### 3A.8 Constitutional checkpoints

- §I.1: Boot is structured `AsmIR` sections with typed pseudo-ops; cartridge header bytes flow through F-A1's `assemble_rom`, never via raw byte pushes.
- §VI.1: Single `boot` module; single `interrupts` module per IRQ source.
- §V.3: Init sequence sets a known LCDC value and zeros HRAM; failure to do so is observable via the post-init memory snapshot test.

## 3B. Cooperative scheduler (T-A5.2, `scheduler.rs`)

**Reference**: `planv0.md` line 2167–2170 (auto-yielding ABI), line 2343–2348 (scheduler shape), line 2360–2375 (`SchedulerPolicy` shape), line 2382–2389 (`UiCommitPlan` shape), line 2398–2399 (liveness fault rules).

### 3B.1 Why this module exists

The scheduler is the Bank 0 entry point that drives every frame. After boot finishes, control reaches `scheduler::main_loop`. The main loop:

1. **Polls frame/input flags** — read `HRAM[frame_count]`, advance UI animation state (cursor blink, scrollback).
2. **Reads joypad** — invokes `joypad::emit_joypad_read` once per frame; the cached `ButtonState` is consumed by keyboard.
3. **Runs UI work** — invokes `keyboard::emit_keyboard_step`, then any text re-render staged by the keyboard step. The scheduler reserves `SchedulerPolicy::hard_ui_reserve` M-cycles before any inference dispatch.
4. **Computes a `UiCommitPlan`** — counts dirty tiles + OAM, estimates cycles (`estimated_cycles = dirty_tiles * 8 + (dirty_oam ? 64 : 0)`), records `latest_safe_mode`. The plan is consumed by `video_commit` to gate its drain priority.
5. **Dispatches inference slices** — as long as `(frame_budget - hard_ui_reserve - UiCommitPlan::estimated_cycles)` remains positive, the scheduler arms TIMA and resumes the inference continuation. Inference yields when `yield_requested` is set; the scheduler regains control, updates `LivenessCounters`, and either dispatches another slice (if budget remains) or proceeds.
6. **Sleeps with `HALT`** — when no UI work and no inference budget remains, the scheduler `HALT`s. The CPU wakes on the next interrupt (typically VBlank or Timer); execution resumes at the next iteration of the main loop.

### 3B.2 Public surface

```rust
/// Build the scheduler section: main_loop, dispatch_inference_slice, idle_until_frame.
pub fn build_scheduler_section() -> Section;

/// Compiler helper. Emits AsmIR that polls HRAM[HRAM_ADDR_YIELD_REQUESTED] and falls through
/// to a save-and-yield sequence if set. Used by gbf-codegen at every safe point.
pub fn emit_yield_check<B: Builder>(b: &mut B, kind: YieldKind);

/// Compiler helper. Emits AsmIR that programs TIMA/TMA/TAC for the requested deadline.
/// Hardware periods are quantized by TAC and TIMA preload; the resolver picks the closest
/// representable deadline and records jitter.
pub fn emit_arm_tima<B: Builder>(b: &mut B, deadline: TimerDeadline);

/// Compiler helper. Emits AsmIR for `HALT` with the appropriate IME caveat handling.
pub fn emit_idle_until_frame<B: Builder>(b: &mut B);

pub struct TimerDeadline {
    pub tac_clock_select: TacClockSelect,
    pub tma: u8,
    pub tima_preload: u8,
    pub requested_m_cycles: u16,
    pub actual_m_cycles: u16,
    pub max_jitter_m_cycles: u8,
}

pub struct SchedulerPolicy {
    /// Total M-cycles per frame. From gbf-hw::timing::FRAME_M_CYCLES (17556).
    pub frame_budget_m_cycles: u32,
    /// Reserved for UI work before any inference dispatch.
    pub hard_ui_reserve: u32,
    /// Soft reserve for adaptive backoff under UI pressure.
    pub soft_ui_reserve: u32,
    /// Margin reserved for the UiCommitPlan's estimated cost.
    pub video_commit_margin: u32,
    /// Worst-case M-cycles a slice may run before yielding.
    pub max_slice_m_cycles: u32,
    /// Adaptive headroom; raised when UI is light, shrunk under pressure.
    pub adaptive_headroom: u16,
    /// TIMA deadline configuration. Hardware periods are quantized by TAC and TIMA preload.
    pub timer_deadline: TimerDeadline,
    /// Worst-case CPU dispatch + vector + stub latency to first handler instruction (M-cycles).
    pub max_interrupt_entry_latency_m_cycles: u16,
    /// Worst-case total ISR occupancy including teardown (M-cycles).
    pub max_interrupt_total_occupancy_m_cycles: u16,
    /// Soft deadline margin; the compiler emits early-exit paths past this.
    pub soft_deadline_margin: u32,
    /// Compiler-side maximum-safe-point-gap proof bound. Lowering must prove every loop
    /// and micro-kernel inserts a yield check at most this many M-cycles apart.
    pub max_safe_point_gap_m_cycles: u16,
    /// Liveness threshold; raise FaultCode::LivenessTimeout past this.
    pub max_no_progress_frames: u16,
    /// Default yield class for compiler-emitted yield checks.
    pub default_yield_kind: YieldKind,
}

impl SchedulerPolicy {
    /// Conservative bring-up policy: 14000 M-cycles for inference per frame, 3000 M-cycles
    /// for UI, TIMA period 500 M-cycles.
    pub const fn bring_up() -> Self { /* ... */ }
}
```

### 3B.3 Yield mechanism — TIMA is a deadline signal, not preemption

The cooperative yield uses TIMA as a *deadline signal*, not as preemption. TIMA can only set a flag; it cannot itself stop a long inner loop. Observation requires a compiler-emitted safe-point poll.

The contract therefore has two parts:

- **Runtime side (this RFC):** the scheduler arms TIMA via `emit_arm_tima(deadline)` before dispatching a slice; the timer ISR sets `HRAM[HRAM_ADDR_YIELD_REQUESTED] = 1`. TIMA periods are quantized by TAC clock select + TIMA preload + TMA reload, with edge/overflow quirks per Pan Docs §"Timer and Divider Registers"; `TimerDeadline` records both `requested_m_cycles` and `actual_m_cycles` plus `max_jitter_m_cycles`.
- **Compiler side (F-A1 lowering / Epic B):** the compiler/lowering layer must prove a maximum safe-point gap for every emitted loop and micro-kernel. The bound is `SchedulerPolicy::max_safe_point_gap_m_cycles`. Without that proof, a long inner loop never observes the flag, and TIMA is useless. F-A5 declares the contract; lowering enforces it.

End-to-end:

1. Before dispatch, the scheduler invokes `emit_arm_tima(deadline)`. This programs TAC/TMA/TIMA for the closest representable deadline and enables the timer interrupt.
2. The scheduler `JP`s into the inference continuation pointed at by `InferenceState::cont_addr`.
3. The inference slice runs. At every safe point (every `Yield` pseudo-op the compiler emitted, separated by at most `max_safe_point_gap_m_cycles` of straight-line cost), the slice polls `HRAM[HRAM_ADDR_YIELD_REQUESTED]`. If 0, fall through and continue.
4. When TIMA wraps, the timer ISR fires and sets `HRAM[HRAM_ADDR_YIELD_REQUESTED] = 1`. (The CPU clears the corresponding `IF` bit on dispatch; the handler does not write `IF`.)
5. On the next safe-point poll, the inference slice observes the flag set, saves continuation (`InferenceState::cont_addr` ← current PC + bank, `cont_slice` ← current slice id, `arena_cursor` ← current arena state), clears the flag, and `RET`s into the scheduler.
6. The scheduler re-checks: did the slice advance `LivenessCounters::progress_epoch`? If so, the slice is making progress. If not, `no_progress_frames += 1`. If `no_progress_frames > max_no_progress_frames`, raise `FaultCode::LivenessTimeout` via `panic::emit_panic`. If `last_checkpoint` did not advance since the prior frame (and `progress_epoch` is unchanged), raise `FaultCode::RepeatedCheckpointNoProgress`.

The `Yield { kind }` pseudo-op, when lowered by F-A1's lowering pass, expands to `emit_yield_check(b, kind)` from this module. So the compiler emits `Yield::Micro` / `Yield::Frame` / `Yield::NeedInput` / `Yield::TokenReady` / `Yield::Finished` / `Yield::Fault`; the lowering hooks into F-A5's helper. Lowering must additionally prove the safe-point-gap bound.

### 3B.4 Liveness wiring

`InferenceStateHeader::liveness` (per F-A3, shipped) is a `LivenessCounters` value with the exact `#[repr(C)]` shape below. F-A5 must match the field set exactly — this is not a redeclaration, just a reminder of what F-A3 ships:

```rust
// gbf-abi::liveness — already shipped in commit 6ad156c.
#[repr(C)]
pub struct LivenessCounters {
    pub progress_epoch: u32,
    pub last_checkpoint: CompactCheckpointId,   // u16 newtype
    pub no_progress_frames: u16,
    pub livelock_threshold_frames: u16,
    pub _reserved: [u8; 2],
}
// SIZE = 12, align = 4. Constructor: LivenessCounters::new(threshold).
// Helpers: record_progress(cp), note_idle_frame(), is_livelocked(), to_bytes(), from_bytes(...).
```

Note the threshold lives *inside* the counters block (`livelock_threshold_frames`), not as a separate `SchedulerPolicy::max_no_progress_frames` field. F-A5 mirrors F-A3's authoritative shape: the scheduler reads `liveness.livelock_threshold_frames` to decide when to raise `FaultCode::LivenessTimeout`. `SchedulerPolicy` carries the *initial* value (`SchedulerPolicy::initial_livelock_threshold_frames`) used at boot to call `LivenessCounters::new`; runtime adaptation is out of scope for M0.

The scheduler updates these per frame:

- After every `JP`-back from inference, compare current `progress_epoch` to the value at the start of the dispatch. If higher, the inference slice already called `record_progress`, which zeroed `no_progress_frames` (no further work needed).
- If `progress_epoch` did not change but `last_checkpoint` did, this is also progress (a checkpoint advance). The scheduler treats this case as `FaultCode::RepeatedCheckpointNoProgress` only when `last_checkpoint` repeats *without* a `progress_epoch` advance — see below.
- If `progress_epoch` did not change and the slice produced no checkpoint advance, the scheduler calls `liveness.note_idle_frame()` (saturating to `u16::MAX`).
- `liveness.is_livelocked()` (defined as `livelock_threshold_frames != 0 && no_progress_frames >= livelock_threshold_frames`) is the condition for raising `FaultCode::LivenessTimeout` and jumping to `panic::entry`.
- If `last_checkpoint` is the same as the prior frame's `last_checkpoint` AND `progress_epoch` did not change, the scheduler raises `FaultCode::RepeatedCheckpointNoProgress` (same panic path). The "prior frame's last_checkpoint" is held in a small scheduler-private HRAM byte, not in `LivenessCounters` itself.

### 3B.5 `HALT` discipline

`HALT` lives only in `scheduler::idle_until_frame`. The instruction is preceded by `EI` (ensure interrupts are enabled) and a memory barrier comment in the AsmIR provenance (so a future CGB double-speed adapter does not move it). Pan Docs warns about `HALT` with `IME=0` causing the "halt bug" where the next instruction is duplicated; F-A5's scheduler is in `IME=1` whenever it `HALT`s, so the bug is not reachable. The acceptance test `scheduler::halt_invariant` confirms this by walking every emit site and asserting `EI` precedes `HALT`.

### 3B.6 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- scheduler::yield_round_trip            # Build a tiny slice that yields, run on emu (or assert encoded bytes), assert resume
cargo test -p gbf-runtime -- scheduler::tima_deadline_is_representable # The chosen TimerDeadline is realizable on TAC/TMA/TIMA hardware
cargo test -p gbf-runtime -- scheduler::max_safe_point_gap_within_deadline # max_safe_point_gap_m_cycles <= deadline.actual_m_cycles - max_jitter
cargo test -p gbf-runtime -- scheduler::halt_invariant               # Every HALT site is preceded by EI
cargo test -p gbf-runtime -- scheduler::livelock_detection           # Frame counter bumps; threshold fires
cargo test -p gbf-runtime -- scheduler::repeated_checkpoint_no_progress
cargo test -p gbf-runtime -- scheduler::interrupt_entry_latency      # All ISR paths respect max_interrupt_entry_latency_m_cycles
cargo test -p gbf-runtime -- scheduler::default_policy_fits_bring_up # bring_up() values are within budget
```

### 3B.7 Constitutional checkpoints

- §I.1: `SchedulerPolicy` is a typed value object with constructor-validated bounds.
- §V.3: Liveness faults use typed `FaultCode` variants and dump structured logs (the `last_checkpoint`, `progress_epoch`, `no_progress_frames` fields).
- §III: Liveness rules are enforced by the scheduler at frame boundaries, not at runtime IO.

## 3C. Joypad reader (T-A5.3, `joypad.rs`)

**Reference**: `planv0.md` line 1631 (joypad in the runtime nucleus); F-A2 T-A2.6 (joypad register layout).

### 3C.1 Why this module exists

The joypad register `JOYP` ($FF00) is *active-low* — bit-clear means "pressed". The select bits (4 and 5) choose whether the lower 4 bits report the four directions or the four buttons (A/B/Start/Select). Reading both halves and OR'ing the inverted result yields a per-button view.

### 3C.2 Public surface

```rust
/// Build the joypad section: emit_joypad_read entry, cached state at JOYPAD_CACHED_STATE_ADDR.
pub fn build_joypad_section() -> Section;

/// Compiler helper. Emit AsmIR for: select directions, read JOYP, select buttons, read JOYP,
/// OR + invert + cache to WRAM at JOYPAD_CACHED_STATE_ADDR. Always deselects both halves at the end.
pub fn emit_joypad_read<B: Builder>(b: &mut B);

/// Absolute WRAM address of the cached ButtonState. Authoritative; no other module writes here.
pub const JOYPAD_CACHED_STATE_ADDR: WramAddr = WramAddr::new(0xC100);
pub const JOYPAD_PREV_STATE_ADDR: WramAddr = WramAddr::new(0xC101);
```

### 3C.3 The active-low decode dance

JOYP layout (per Pan Docs §"Joypad Input"):

- bit 5 = 0: select buttons (A/B/Select/Start)
- bit 4 = 0: select directions (Right/Left/Up/Down)

The standard sequence (note: `LDH (n), A` only addresses `$FF00 + n`; the WRAM cache write must use absolute `LD (nn), A`):

```text
LD   A, $20            ; select d-pad: bit 4 = 0, bit 5 = 1
LDH  ($00), A
LDH  A, ($00)          ; small delay
LDH  A, ($00)
AND  $0F               ; lower 4 bits = directions, active-low
SWAP A                 ; move directions into high nibble
LD   B, A
LD   A, $10            ; select buttons: bit 5 = 0, bit 4 = 1
LDH  ($00), A
LDH  A, ($00)
LDH  A, ($00)
AND  $0F               ; lower 4 bits = buttons, active-low
OR   B                 ; combine: high nibble = directions, low nibble = buttons
CPL                    ; invert: now active-high
LD   (JOYPAD_CACHED_STATE_ADDR), A
LD   A, $30            ; deselect both halves after read
LDH  ($00), A
```

The post-decode view is active-high, so a `1` bit means "pressed". This matches F-A2's `ButtonState::is_pressed` semantics.

### 3C.4 Once-per-frame, not interrupt-driven

The polling read happens once per frame, called from the scheduler's UI work. The Joypad ISR (`$0060`) is wired so the vector table is complete but is a no-op in M0; STOP-mode wake is deferred. The choice is deliberate: a polling read is simpler, has predictable latency, and integrates cleanly with the keyboard's `just_pressed` / `just_released` edge-detection.

### 3C.5 `just_pressed` / `just_released` storage

The previous-frame `ButtonState` is cached at `JOYPAD_PREV_STATE_ADDR`. After each read, the scheduler updates `prev = cached` and `cached = current`. Edge detection (which the keyboard uses) is `(cached & ~prev)` for `just_pressed` and `(~cached & prev)` for `just_released`, computed by `gbf-hw::joypad::just_pressed` / `just_released` (or by inline bit ops in `keyboard.rs`).

### 3C.6 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- joypad::read_emits_expected_sequence    # AsmIR shape matches the standard active-low dance, ends with $30 deselect
cargo test -p gbf-runtime -- joypad::cached_state_addr_in_wram       # Address is within WRAM ($C000..=$DFFF)
cargo test -p gbf-runtime -- joypad::cache_write_uses_absolute_load  # The cache write is `LD (nn), A`, not `LDH`
cargo test -p gbf-runtime -- joypad::isr_is_no_op                     # Joypad ISR body is empty
```

### 3C.7 Constitutional checkpoints

- §VI.1: Single joypad reader; UI/keyboard layers consume the cached state.

## 3D. Text renderer + font (T-A5.4, `text.rs` + `assets/font_8x8.bin`)

**Reference**: `planv0.md` line 1626 (font/assets in Bank 0 nucleus), line 1651.

### 3D.1 Why this module exists

The text renderer is what gets the model's tokens onto the screen. The font asset lives in ROM, but glyph rendering requires a one-time copy of the selected font tiles into VRAM tile data (Pan Docs §"Tile Data": tiles live at `$8000..=$97FF`).

Boot calls `video_commit::emit_bootstrap_vram_init` while LCD is off, before enabling LCDC. That bootstrap copies the M0 font tiles into the chosen VRAM tile block and initializes the BG map.

After bootstrap, text printing is cheap: every glyph print stages a BG-map cell update, `UiCommitOp::PutGlyphCell { x, y, glyph }`, into the queue. `PutGlyphCell` writes the tile ID into the BG map; it does not copy the glyph bitmap each time.

### 3D.2 Bank 0 budget concern

DMG VRAM tile data has 384 tile slots total, shared between BG and OBJ. A full 256-glyph font would consume both Bank 0 ROM (~4 KiB) and a large fraction of VRAM tile capacity. M0 ships a smaller bootstrap font (`FONT_TILE_COUNT = 128` glyphs × 16 bytes = 2048 bytes) that covers ASCII printable + a few control glyphs (cursor, backspace, submit, ellipsis) needed by the M0 keyboard. A full 256-glyph or `LexicalSpec.charset`-sized font is reserved for a CommonBank-backed font table; F-A4 banking has shipped, so the lease helpers exist — the missing piece is the normal-payload far-call materializer in `gbf-codegen` (per F-A4's known-debt). The font relocation is therefore a follow-up bead in `gbf-codegen`/Epic B, not in F-A5.

For M0, the font stays in Bank 0 because (a) every text print would otherwise pay a bank-switch tax, and (b) keeping the reference shell self-contained (no far-calls outside Bank 0) makes the boot-and-render path debuggable from the first ROM that boots.

### 3D.3 Public surface

```rust
/// Build the text section: emit_text_print_glyph, font asset embed.
pub fn build_text_section() -> Section;

/// FONT_TILE_COUNT * 16 bytes. M0 ships 128 glyphs × 16 bytes = 2048 bytes.
pub fn font_bytes() -> &'static [u8];

/// Number of glyph tiles installed into VRAM by bootstrap.
pub const FONT_TILE_COUNT: u16 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextLayout {
    pub bg_map_origin: u16,                    // VRAM address (typically 0x9800)
    pub visible_columns: u8,                   // 20 for DMG (160 / 8)
    pub visible_rows: u8,                      // 18 for DMG (144 / 8)
    pub bg_map_stride: u8,                     // 32 entries per BG-map row (DMG hardware)
}

impl TextLayout {
    pub const fn dmg_default() -> Self {
        Self {
            bg_map_origin: 0x9800,
            visible_columns: 20,
            visible_rows: 18,
            bg_map_stride: 32,
        }
    }
}

/// Compiler helper. Emit AsmIR: stage UiCommitOp::PutGlyphCell { x, y, glyph } via video_commit::emit_queue_op.
pub fn emit_text_print_glyph<B: Builder>(b: &mut B, x: u8, y: u8, glyph: u8);

/// Compiler helper. Emit AsmIR: stage a row of UiCommitOp::PutGlyphCell spaces (clearing).
/// Expands into one or more bounded FillGlyphRun queue entries.
pub fn emit_text_clear_row<B: Builder>(b: &mut B, y: u8);
```

The BG-map address for a cell is `bg_map_origin + y * bg_map_stride + x` (stride 32, not `visible_columns`).

### 3D.4 The asset path

`gbf-runtime/assets/font_8x8.bin` is a fixed blob, included via `include_bytes!` in `text.rs`. M0 defaults to 128 glyphs × 16 bytes = 2048 bytes. The font shape is documented (each glyph is 16 bytes in the standard Game Boy 2bpp tile format, but the font is mono so each pair of bytes is `(row_bits, row_bits)` — both planes are the same so the glyph reads as monochrome). The font is reproducibly generated from a public domain source and the generation script is checked into `tools/font/build_font.py`; the script produces byte-identical output across runs.

### 3D.5 Why not direct VRAM writes?

The `text` module produces no `MachineEffect::StoreToVram`. Every glyph print stages a `UiCommitOp::PutGlyphCell` into the queue. The reasons:

1. **PPU mode gating.** `text::emit_text_print_glyph` is called from anywhere in the runtime — keyboard step, scheduler frame loop, panic. The current PPU mode is unknown at the call site. Direct VRAM writes during mode 3 corrupt the display; `video_commit` handles the gating.
2. **Atomic frame consistency.** Multiple text writes within a frame are batched; the commit drain delivers them in priority order during the next legal mode window.
3. **Single-writer enforceability.** The invariant that `video_commit` is the only writer is what makes the runtime debuggable. Letting `text` write VRAM directly forecloses that property.

### 3D.6 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- text::font_size                          # FONT_TILE_COUNT * 16 bytes
cargo test -p gbf-runtime -- text::font_installed_before_lcdc_enable   # Boot installs tiles via video_commit::emit_bootstrap_vram_init while LCD is off
cargo test -p gbf-runtime -- text::layout_dmg                          # 20x18 visible, 32 BG-map stride
cargo test -p gbf-runtime -- text::print_glyph_stages                  # Generates a UiCommitOp::PutGlyphCell, never a direct VRAM write
cargo test -p gbf-runtime -- text::no_vram_access_machine_effect       # MachineEffect::StoreToVram never appears in the section
```

### 3D.7 Constitutional checkpoints

- §I.1: Text never writes VRAM directly — the invariant is enforced at the API boundary by the builder context.
- §VI.1: Single text renderer.

## 3E. Keyboard input + on-screen layout (T-A5.5, `keyboard.rs`)

**Reference**: `planv0.md` line 1626; `KeyboardLayoutSpec` (planv0 line 703 area).

### 3E.1 Why this module exists

The on-screen keyboard is how the user inputs prompts. It is a grid of glyphs with a cursor; D-pad moves the cursor, A presses, B backspaces, Start submits, Select toggles charset slice. The layout is data-driven from `LexicalSpec.charset` (Epic G) — F-A5 ships only the default ASCII layout for bring-up. When Epic G lands, the keyboard reads the real `LexicalSpec`.

### 3E.2 Public surface

```rust
/// Build the keyboard section: KeyboardState in WRAM, emit_keyboard_step entry.
pub fn build_keyboard_section() -> Section;

/// Borrowed runtime layout (ROM-resident grid; lifetime-bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyboardLayoutSpec<'a> {
    pub rows: u8,
    pub columns: u8,
    pub cells: &'a [KeyboardCell],
}

/// Host-serializable counterpart used by reports, tooling, and Epic G's LexicalSpec ingestion.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyboardLayoutManifest {
    pub rows: u8,
    pub columns: u8,
    pub cells: Vec<KeyboardCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyboardCell {
    Char(u8),
    Special(SpecialKey),
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecialKey {
    Backspace,
    Submit,
    Shift,                                    // reserved variant; not in the M0 default layout
    Cancel,                                   // reserved variant; not in the M0 default layout
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyboardState {
    pub cursor_row: u8,
    pub cursor_col: u8,
    pub charset_slice: u8,
}

/// Compiler helper. Emit AsmIR for: read joypad cached state, advance cursor / commit character.
pub fn emit_keyboard_step<B: Builder>(b: &mut B);

/// The M0 default layout. Epic G replaces with a LexicalSpec.charset-driven constructor.
pub fn default_layout() -> KeyboardLayoutSpec<'static>;

// M0 prompt buffer (F-A5-owned). When F-D2 lands, the producer/consumer protocol may be
// replaced but must preserve or migrate these symbols.
pub const PROMPT_BUFFER_BASE_ADDR: WramAddr = WramAddr::new(0xC380);
pub const PROMPT_BUFFER_LEN: u8 = 96;
pub const PROMPT_CURSOR_ADDR: WramAddr = WramAddr::new(0xC3E0);
pub const PROMPT_SUBMITTED_FLAG_ADDR: WramAddr = WramAddr::new(0xC3E1);
```

### 3E.3 Step semantics

Each keyboard step (called once per frame from the scheduler):

1. Read `WRAM[JOYPAD_CACHED_STATE_ADDR]` and `WRAM[JOYPAD_PREV_STATE_ADDR]`.
2. Compute `just_pressed = cached & ~prev`.
3. If `just_pressed` includes Up/Down/Left/Right: advance cursor (clamping to `[0, columns)` × `[0, rows)`); stage a `UiCommitOp::PutGlyphCell` for the previous cursor cell (re-render with normal palette) and the new cursor cell (re-render with cursor highlight palette).
4. If `just_pressed` includes A: write the current cell's character into `PROMPT_BUFFER_BASE_ADDR + WRAM[PROMPT_CURSOR_ADDR]`, then increment the cursor (saturating at `PROMPT_BUFFER_LEN - 1`).
5. If `just_pressed` includes B: backspace (decrement `PROMPT_CURSOR_ADDR`, saturating at 0; stage a `UiCommitOp::PutGlyphCell` to clear the last printed glyph).
6. If `just_pressed` includes Start: set `PROMPT_SUBMITTED_FLAG_ADDR = 1`. F-A5 owns the M0 prompt buffer layout (above); F-D2 may later replace the producer/consumer protocol but must preserve or migrate these symbols.
7. If `just_pressed` includes Select: cycle `charset_slice`; re-render the visible row.

### 3E.4 Default layout

The M0 default layout is exactly 4 rows × 10 columns:

```text
a b c d e f g h i j
k l m n o p q r s t
u v w x y z 0 1 2 3
4 5 6 7 8 9 . _ ⌫ ↵
```

`_` emits space, `⌫` maps to `SpecialKey::Backspace`, and `↵` maps to `SpecialKey::Submit`. `Shift` and `Cancel` are reserved variants of the `SpecialKey` enum but are *not* present in the M0 default layout cells; they may appear in a layout manifest produced by Epic G.

The 4×10 grid fits within the 20×18 DMG screen with room for prompt-area + status text above (8 rows × 20 columns = 160 cells of working area).

### 3E.5 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- keyboard::default_layout                  # Sane DMG-sized layout exists
cargo test -p gbf-runtime -- keyboard::cursor_movement                  # D-pad moves cursor; bounds clamp
cargo test -p gbf-runtime -- keyboard::special_keys                     # Backspace/Submit work
cargo test -p gbf-runtime -- keyboard::step_emits_only_queue_ops        # No direct VRAM access
```

### 3E.6 Constitutional checkpoints

- §I.1: `SpecialKey` is enumerated; `KeyboardLayoutSpec` is a typed value object.
- §VI.1: Keyboard reads cached joypad state from `joypad`; never reads JOYP directly.

## 3F. `video_commit` + `UiCommitPlan` (T-A5.6, `video_commit.rs`)

**Reference**: `planv0.md` line 117 ("VRAM accessible during modes 0, 1, 2; OAM during modes 0 and 1"), line 1635 (video_commit module owns UiCommitPlan and the commit queue), line 2010, line 2079, line 2382–2389 (`UiCommitPlan` shape).

### 3F.1 Why this module exists

VRAM and OAM are accessible only during specific PPU modes (Pan Docs §"Rendering"):

- **Mode 0** (HBlank): VRAM writable, OAM writable. Mode-0 length varies by scanline.
- **Mode 1** (VBlank): VRAM writable, OAM writable. The most permissive window (~1140 M-cycles).
- **Mode 2** (OAM Search): VRAM writable, OAM *not* writable.
- **Mode 3** (Drawing): neither writable. Writing here corrupts the display.

The `video_commit` module is the *single* writer. All other modules (`text`, `keyboard`, `panic`) stage `UiCommitOp`s into the queue, and the commit module drains the queue at LCD STAT and VBlank ISR boundaries. Each drain entry gates each op on the current PPU mode read from `STAT_REGISTER`.

### 3F.2 Public surface

```rust
/// Build the video_commit section: queue storage, emit_queue_op, emit_commit_drain_*, emit_bootstrap_vram_init.
pub fn build_video_commit_section() -> Section;

/// Bounded wire-level commit operations. Higher-level helpers (text::emit_text_clear_row,
/// keyboard cell highlight, panic glyph render) expand into one or more bounded queue entries
/// before enqueueing. Unbounded operations such as multi-row clears are not legal queue items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UiCommitOp {
    PutGlyphCell { x: u8, y: u8, glyph: u8 },
    FillGlyphRun { x: u8, y: u8, len: u8, glyph: u8 },
    SetDmgPalette { target: DmgPaletteRegister, value: u8 },
    PutOamSprite { sprite_index: u8, y: u8, x: u8, tile: u8, attrs: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UiCommitOpKind {
    PutGlyphCell,
    FillGlyphRun,
    SetDmgPalette,
    PutOamSprite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DmgPaletteRegister { Bgp, Obp0, Obp1 }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UiCommitPlan {
    pub max_ops_per_frame: u16,
    pub max_ops_per_hblank: u8,
    pub vblank_priority_ops: Vec<UiCommitOpKind>,
}

impl UiCommitPlan {
    pub fn default_v1() -> Self {
        Self {
            max_ops_per_frame: 32,
            max_ops_per_hblank: 1,                  // Bounded after ISR entry overhead — see §3F.3.
            vblank_priority_ops: vec![UiCommitOpKind::PutOamSprite, UiCommitOpKind::SetDmgPalette],
        }
    }
}

/// Compiler helper. Stage a UiCommitOp into the WRAM ring queue using the publication-ordered
/// protocol described in §3F.4.
pub fn emit_queue_op<B: Builder>(b: &mut B, op: UiCommitOp);

/// HBlank-time drain entry, called from the LCD STAT mode-0 ISR. May execute only bounded
/// single-cell BG-map writes whose worst-case instruction sequence fits after ISR entry
/// overhead and remains valid if execution spills into mode 2. Never writes OAM.
pub fn emit_commit_drain_hblank<B: Builder>(b: &mut B);

/// VBlank drain entry, called from the VBlank ISR. May execute VRAM and OAM micro-ops up to
/// UiCommitPlan::max_ops_per_frame, prioritizing UiCommitPlan::vblank_priority_ops first.
pub fn emit_commit_drain_vblank<B: Builder>(b: &mut B);

/// Boot-time helper. Copy FONT_TILE_COUNT glyph tiles into VRAM tile data and initialize
/// the BG map. Caller (boot init) must hold LCD off (LCDC bit 7 = 0). Privileged builder context.
pub fn emit_bootstrap_vram_init<B: Builder>(b: &mut B);

pub const COMMIT_QUEUE_BASE_ADDR: WramAddr = WramAddr::new(0xC200);
pub const COMMIT_QUEUE_LEN: u8 = 32;
pub const UI_COMMIT_WIRE_OP_BYTES: u8 = 8;
pub const COMMIT_QUEUE_HEAD_ADDR: WramAddr =
    COMMIT_QUEUE_BASE_ADDR.add(COMMIT_QUEUE_LEN as u16 * UI_COMMIT_WIRE_OP_BYTES as u16);
pub const COMMIT_QUEUE_TAIL_ADDR: WramAddr = COMMIT_QUEUE_HEAD_ADDR.add(1);

/// Stable WRAM queue encoding. Rust enum layout is never used as the ABI.
///
/// byte 0: opcode (matches UiCommitOpKind discriminant)
/// byte 1: flags / reserved (always 0 in M0)
/// byte 2..7: opcode payload, zero-filled when unused
///
/// Producers always write all 8 bytes before publishing the tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiCommitWireOp([u8; UI_COMMIT_WIRE_OP_BYTES as usize]);
```

### 3F.3 Drain semantics

The drain has two entry points:

- `emit_commit_drain_hblank` — called from the LCD STAT mode-0 ISR. After ISR entry overhead (~31 M-cycles per §3A.5) and the per-op decode, the remaining mode-0 + mode-2 window is narrow and varies by scanline. M0 therefore caps the HBlank drain to one bounded BG-map micro-op per ISR (`UiCommitPlan::max_ops_per_hblank = 1`) and never executes OAM writes from HBlank. `FillGlyphRun` is allowed only when its `len` × per-cell cost fits the proven worst-case window.
- `emit_commit_drain_vblank` — called from the VBlank ISR. Mode 1 (VBlank) is the most permissive window (~1140 M-cycles). The drain dequeues up to `max_ops_per_frame` ops, prioritizing the `UiCommitPlan::vblank_priority_ops` kinds first.

For each op:

1. Read current `STAT_REGISTER` mode bits.
2. Check accessibility:
   - `PutGlyphCell`, `FillGlyphRun` (BG-map / VRAM writes) — legal in modes 0, 1, 2.
   - `PutOamSprite` (OAM write) — legal in modes 0, 1 only, AND only from `emit_commit_drain_vblank` in M0 (HBlank OAM writes are deferred to avoid consuming the narrow mode-0 window after ISR entry overhead).
   - `SetDmgPalette` — legal anytime (it writes a DMG palette register, not VRAM/OAM).
3. If illegal: leave the op in place at the queue head and return. **M0 does not reorder the queue inside an ISR.** Reordering may be added later only with an explicit starvation proof.
4. If a write actually attempts to land in an illegal mode (defensive check inside the drain): raise `FaultCode::UiCommitOutsideLegalMode`.
5. Queue capacity exhaustion (producer cannot enqueue because `head == next_tail`) is a separate condition and raises `FaultCode::UiCommitQueueFull` — not the same fault.

### 3F.4 Queue ABI: publication ordering

The queue lives in WRAM (HRAM is too small: 127 bytes, while the queue alone wants 32 ops × 8 bytes = 256 bytes). The ring is single-producer (any module's `emit_queue_op`), single-consumer (the drain inside an ISR). The producer runs with IME enabled; the consumer can interleave at any HBlank or VBlank.

The protocol is **publication-ordered**:

1. Producer reads `tail`, computes `next_tail`, and checks fullness (`next_tail == head` ⇒ raise `FaultCode::UiCommitQueueFull`).
2. Producer writes the full `UiCommitWireOp` payload (all 8 bytes) into the current tail slot.
3. Producer publishes `tail = next_tail` as the **final byte write**.
4. Consumer reads `head` and `tail`. If equal, the queue is empty.
5. Consumer reads the full payload from `head`, executes or defers it, and only then publishes `head = next_head`.

The ISR consumer can observe either the old tail (queue appears empty) or the new tail (slot payload is fully written); it must never observe a new tail before the slot payload is complete. Single-byte tail/head publication on LR35902 is naturally atomic against an interrupt boundary.

### 3F.5 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- video_commit::queue_full_raises_typed_fault       # FaultCode::UiCommitQueueFull, not UiCommitOutsideLegalMode
cargo test -p gbf-runtime -- video_commit::no_writes_in_mode_3                  # Drain refuses VRAM writes in Drawing mode
cargo test -p gbf-runtime -- video_commit::no_oam_writes_in_hblank              # M0 drain_hblank never writes OAM
cargo test -p gbf-runtime -- video_commit::hblank_drain_max_one_op              # max_ops_per_hblank = 1
cargo test -p gbf-runtime -- video_commit::vblank_priority                       # vblank_priority_ops fire first
cargo test -p gbf-runtime -- video_commit::oam_only_in_modes_0_1
cargo test -p gbf-runtime -- video_commit::wire_op_size_is_8_bytes               # UI_COMMIT_WIRE_OP_BYTES, not size_of::<UiCommitOp>()
cargo test -p gbf-runtime -- video_commit::enqueue_publishes_tail_last
cargo test -p gbf-runtime -- video_commit::drain_publishes_head_after_payload
cargo test -p gbf-runtime -- video_commit::lcd_mode_gating                       # End-to-end mode-gating test against an emulator step
cargo test -p gbf-runtime -- video_commit::sole_vram_writer                       # Section is the only one with MachineEffect::StoreToVram outside panic
cargo test -p gbf-runtime -- video_commit::bootstrap_runs_with_lcd_off
```

### 3F.6 Constitutional checkpoints

- §I.1: `UiCommitOp` is bounded and enumerated; queue-full and illegal-mode are *separate* typed faults.
- §VI.1: Sole VRAM/OAM writer.
- §V.3: Illegal-mode-write and queue-full are typed faults, not silent corruption.

## 3G. Panic screen (T-A5.7, `panic.rs`)

**Reference**: `planv0.md` line 1646 (panic/debug screen in Bank 0 nucleus).

### 3G.1 Why this module exists

When the runtime takes an unrecoverable fault, it must:

1. Disable interrupts (no further state damage).
2. Dump the `FaultCode` to a known WRAM location so an attached emulator/debugger can read it before reset. This is **not** durable across power loss; SRAM persistence is F-D1.
3. Render a "Game Boy halted: <fault code>" screen via *direct* VRAM write (because `video_commit` may itself be broken — the queue may be corrupt, the LCD STAT ISR may be the source of the fault). Pan Docs §"LCD Control" warns that disabling LCDC.7 outside VBlank may damage real DMG hardware; the panic path therefore waits for VBlank before clearing LCDC.7.
4. Halt.

Without panic, faults silently freeze the system; debugging is impossible. The minimal viable panic is small (~256 bytes; matches T2.4's `FutureReservation::Panic.rom_bytes_per_bank0`).

### 3G.2 Public surface

```rust
/// Build the panic section. Annotated ExecutionContext::PanicOnly + InterruptDiscipline::ImeDisabled
/// plus the audit-exempt PanicBypass annotation that ReachabilityValidation recognizes.
pub fn build_panic_section() -> Section;

/// Compiler helper. Emit AsmIR: disable interrupts, dump FaultCode to WRAM, jump to panic_entry.
pub fn emit_panic<B: Builder>(b: &mut B, code: FaultCode);

/// Bypass video_commit (since it may be broken); write directly to VRAM.
/// Caller must already be inside the panic section's PanicBypass builder context.
pub fn emit_panic_screen_render<B: Builder>(b: &mut B);

pub const WRAM_LAST_FAULT_ADDR: WramAddr = WramAddr::new(0xC300);
```

### 3G.3 Audited bypass

The panic section is the one runtime section that emits `MachineEffect::StoreToVram` outside `video_commit`. The annotation is `ExecutionContext::PanicOnly` + `InterruptDiscipline::ImeDisabled` plus an explicit `PanicBypass` audit annotation, recognized by `ReachabilityValidation` (Epic B) as exempt from the no-direct-VRAM rule. The audit reasoning:

- The fault may have originated in `video_commit` itself (queue corruption, LCD STAT handler bug). Routing the panic screen through the queue would reproduce the bug in the panic path.
- The fault may have crashed the scheduler. Routing the panic through normal frame timing would deadlock (the scheduler is not running).
- The fault may have left LCD in mode 3. Pan Docs §"LCD Control" warns against disabling LCDC.7 outside VBlank because real DMG hardware can be damaged. The panic path therefore *waits* for VBlank before clearing LCDC.7 — at most one frame of spin (~16.7 ms) — and only then writes directly to VRAM.

The panic screen render sequence:

```text
panic_entry:
    DI
    LD   HL, WRAM_LAST_FAULT_ADDR
    LD   (HL), <fault_code_byte>
    ; If LCD is enabled, wait until VBlank before clearing LCDC.7.
    ; Panic may spin for at most one frame; it must not risk DMG LCD damage.
    CALL panic_wait_for_vblank_if_lcd_enabled
    XOR  A
    LD   ($FF40), A     ; LCDC = 0, now safe
    ; Render fault code as glyphs at known VRAM tile addresses.
    CALL panic_screen_render
    ; Re-enable LCD (BG only).
    LD   A, $80         ; LCDC bit 7 = enable
    LD   ($FF40), A
panic_halt:
    HALT
    JR   panic_halt

panic_wait_for_vblank_if_lcd_enabled:
    ; Read LCDC; if bit 7 = 0, LCD already off — return immediately.
    LDH  A, ($40)
    BIT  7, A
    RET  Z
    ; Spin until LY >= 144 (VBLANK_LY_THRESHOLD).
.wait:
    LDH  A, ($44)        ; LY
    CP   144
    JR   C, .wait
    RET
```

### 3G.4 What panic does *not* do (M0 deferral)

The full `FaultSnapshot` (PC, bank, checkpoint, regs, liveness) is not dumped to SRAM. That requires F-D1's persistence protocol. M0 panic dumps only:

- `FaultCode` byte at `WRAM_LAST_FAULT_ADDR`. This is **not** durable across power loss; it exists so an attached emulator/debugger can read it before reset.
- On-screen glyph rendering of the fault code.
- HALT.

When F-D1 lands, F-A5's panic is extended to also write a `PersistKind::Continuation` record with `FaultSnapshot` to SRAM via the F-D1 protocol. That is a follow-up bead.

### 3G.5 Acceptance gates

```bash
cargo check -p gbf-runtime
cargo test -p gbf-runtime -- panic::waits_for_vblank_before_lcdc_disable
cargo test -p gbf-runtime -- panic::emits_di_then_halt
cargo test -p gbf-runtime -- panic::renders_fault_code_glyphs
cargo test -p gbf-runtime -- panic::section_marked_exempt              # ExecutionContext + PanicBypass annotations are correct
cargo test -p gbf-runtime -- panic::wram_last_fault_byte_set
cargo test -p gbf-runtime -- panic::is_only_other_vram_writer          # Cross-section: panic is the only non-video_commit source of VramAccess
```

### 3G.6 Constitutional checkpoints

- §V.3: Panic dumps fault code to a visible WRAM location and renders on-screen; not silent.
- §I.1: `FaultCode` is structured (consumed from `gbf-abi`).
- §VI.1: Panic is the *only* audited bypass; the exemption is annotated, not implicit.

## 3H. Bank 0 budget analysis

### 3H.1 The 16 KiB constraint

Bank 0 has 16384 bytes total. The reference shell modules must fit within that budget minus the slack reserved for future modules per T2.4's `FutureReservation` table. The bring-up estimate:

| Module       | Estimated bytes | Notes |
|--------------|-----------------|-------|
| Cartridge entry stub at `$0100..=$0103` | 4 | `NOP; JP runtime_boot_entry` |
| Cartridge header byte fields incl. logo (`$0104..=$014F`) | 76 | F-A1's `assemble_rom` stamp |
| `BuildIdentityBlock` reservation (placed after header) | ~64 | F-A3 layout; hash fields zeroed by F-A5, filled by backend at link |
| `runtime_boot_entry` init sequence | ~120 | Zero HRAM, install bootstrap VRAM tiles, set LCDC/STAT/IE, jump to scheduler |
| IRQ vectors (5 × 8 bytes) | 40 | `$0040..=$0067` slack inclusive |
| ISR stubs (5 × ~14 bytes) | ~70 | Save regs, call handler, restore, RETI |
| Interrupt handlers | ~200 | VBlank, LCD STAT, Timer, Serial, Joypad bodies |
| Scheduler | ~600 | Main loop, dispatch, idle, liveness wiring |
| Joypad | ~80 | Polling read sequence |
| Text helpers (excl. font) | ~150 | print_glyph, clear_row builders |
| Font asset (M0: `FONT_TILE_COUNT = 128`) | 2048 | 128 glyphs × 16 bytes; full 256-glyph fonts deferred to a CommonBank-backed follow-up |
| Keyboard | ~300 | Step + default 4×10 layout table + M0 prompt buffer scaffolding |
| video_commit | ~700 | Queue + dual drain entries + mode gating + bootstrap_vram_init |
| Panic | ~256 | Match T2.4's reservation; includes panic_wait_for_vblank_if_lcd_enabled |
| **Subtotal: F-A5 modules** | **~4708** | |
| `FutureReservation::Persistence` | 768 | F-D1 reserved |
| `FutureReservation::Trace` | 256 | F-D3 reserved |
| `FutureReservation::Harness` | 384 | F-D2 reserved |
| **Subtotal: future reservations** | **1408** | |
| **Total Bank 0 used + reserved** | **~6116** | |
| **Bank 0 free (`Bank0Free` slot)** | **~10268** | Available for compiled inference code |

The estimate leaves >8 KiB free for `Bank0Free` use (expert hot kernels, etc.). Shrinking the M0 font from 4 KiB to 2 KiB recovers about 2 KiB of `Bank0Free` over the original draft.

### 3H.2 Test gate: `nucleus_fits_bank0_budget`

```rust
#[test]
fn nucleus_fits_bank0_budget() {
    let sections = build_bank0_nucleus_sections();
    let total: usize = sections.iter().map(|s| s.encoded_size()).sum();
    let budget = 16384
        - FutureReservation::PERSISTENCE.rom_bytes_per_bank0 as usize
        - FutureReservation::TRACE.rom_bytes_per_bank0 as usize
        - FutureReservation::HARNESS.rom_bytes_per_bank0 as usize;
    assert!(total <= budget, "Bank 0 nucleus is {} bytes, budget is {}", total, budget);
}
```

If F-A5 lands above this budget:

1. **First**, audit text/font for size. The font is the largest single contributor (~50% of used budget). A 6×8 mono font (~3072 bytes) saves ~25% with marginal readability cost.
2. **Second**, consider moving the font to a CommonBank. The keyboard layer would then far-call into the CommonBank-resident glyph table via the now-shipped `lease_rom_switchable` / `release_bank` helpers (F-A4); the missing piece is the normal-payload far-call materializer in `gbf-codegen` (F-A4 known-debt row 3). This relocates ~4 KiB out of Bank 0 at the cost of one bank-switch per glyph print (typically ~5 M-cycles).
3. **Third**, only then revisit the future reservations. Shrinking `Persistence` / `Trace` / `Harness` reservations is reviewable through the T2.5 release-note process.

### 3H.3 The runtime_nucleus_hash story

The deterministic section order in `build_bank0_nucleus_sections()` is:

1. `boot::build_boot_section()` (entry stub + post-header `runtime_boot_entry` init sequence; cartridge header byte fields stamped by `assemble_rom`)
2. `boot::build_irq_vectors_section()`
3. `boot::build_isr_stubs_section()`
4. `interrupts::build_handlers_section()`
5. `scheduler::build_scheduler_section()`
6. `joypad::build_joypad_section()`
7. `text::build_text_section()` (includes font asset)
8. `keyboard::build_keyboard_section()` (includes default layout)
9. `video_commit::build_video_commit_section()`
10. `panic::build_panic_section()`

The order is part of the runtime ABI. Reordering is a deliberate `runtime_nucleus_hash` bump.

The hash is computed over a *normalized* Bank 0 nucleus image, not directly over `Section` payloads:

```text
sha256(
  b"gbf-runtime/v1/bank0-nucleus" ||
  normalized_final_bank0_bytes
)
```

Normalization rules:

1. All linker-filled `BuildIdentityBlock` hash fields are zeroed (`artifact_core_hash`, `lowering_hash`, `compile_request_hash`, `runtime_nucleus_hash`).
2. The `runtime_nucleus_hash` field itself is zeroed (it cannot reference itself).
3. Any checksum bytes stamped by the cartridge-header assembler (e.g., header checksum at `$014D`, global checksum at `$014E..=$014F`) are either included after final stamping or explicitly zeroed; the choice is pinned here and tested. M0 pins: header checksum included, global checksum zeroed (because the global checksum depends on the entire ROM, not just Bank 0).
4. Section order and final placement (post-relocation/layout) are both included. Hashing unplaced `Section` payloads is not sufficient because relocation/layout changes are runtime-nucleus changes.
5. `runtime_nucleus_hash` covers the runtime nucleus + the runtime ABI version (`gbf-runtime/v1/...`) only. Compile-profile-specific identity is **not** part of `runtime_nucleus_hash`; that belongs to a separate build identity hash, because the same runtime nucleus can be shared across multiple compile profiles.

T2.5 (`bd-177`) consumes the deterministic hash to enforce drift detection. F-A5 ships:

- The pinned section order.
- The hash domain separator constant.
- A `pub fn compute_runtime_nucleus_hash(normalized_bank0: &[u8; 16 * 1024]) -> Hash256` helper that takes the post-link normalized image; the T2.5 CI gate provides this image after running the assembler.
- A builder-side convenience function `compute_runtime_nucleus_hash_for_test()` that assembles and normalizes the F-A5-only bring-up image so the helper is testable in `cargo test -p gbf-runtime` without a full backend run.

F-A5 does *not* ship the CI workflow itself; that is T2.5's deliverable. F-A5's `compute_runtime_nucleus_hash` returns a stable value across clean checkouts, which is the load-bearing input.

## 4. Testing strategy

### 4.1 Type-level (compile-time)

- `UiCommitOp`, `UiCommitOpKind`, `SpecialKey`, `KeyboardCell`, `DmgPaletteRegister` are exhaustive enums; `#[non_exhaustive]` is *not* used (consumers should match every variant).
- `SchedulerPolicy::bring_up` is `const`; the compiler verifies the integer literals fit.
- ISR section roles, privilege classes, execution contexts, and interrupt disciplines are checked at section construction by F-A1's `Builder`. A misannotated section fails `cargo check`.

### 4.2 Unit / property

Per-module tests against `planv0.md` and Pan Docs:

- `boot`: cartridge entry stub at `$0100..=$0103`; Nintendo logo at `$0104..=$0133`; IRQ vectors are 8-byte slots; init sequence zeros the right HRAM range; bootstrap VRAM init runs while LCD is off.
- `interrupts`: each handler's M-cycle cost is under `max_interrupt_total_occupancy_m_cycles`; handlers do not write `IF` on the normal path; the narrow `emit_clear_pending_if_bit` helper preserves unrelated IF bits; VBlank handler bumps `frame_count`; Timer handler sets `yield_requested`.
- `scheduler`: `bring_up()` policy values are within `FRAME_M_CYCLES`; `emit_yield_check` produces the expected sequence; `emit_arm_tima` programs TAC/TMA/TIMA to a representable deadline; HALT is preceded by EI; the safe-point-gap bound holds for every test slice.
- `joypad`: the active-low decode sequence matches the standard pattern; the cache write uses absolute `LD (nn), A`; the read deselects both halves at the end; Joypad ISR is a no-op.
- `text`: font is exactly `FONT_TILE_COUNT * 16` bytes; bootstrap installs tiles before LCDC.7 enable; layout reports 20×18 visible / 32 BG-map stride; print_glyph stages a queue op only.
- `keyboard`: M0 4×10 layout exists; cursor clamps within bounds; SpecialKey variants used in M0 (Backspace, Submit) have step paths; step emits no `MachineEffect::StoreToVram`; M0 prompt buffer addresses are within WRAM and non-overlapping.
- `video_commit`: HBlank drain caps at one op per ISR; HBlank drain never writes OAM; drain refuses VRAM writes in mode 3; queue full raises `FaultCode::UiCommitQueueFull`; an attempted illegal-mode write raises `FaultCode::UiCommitOutsideLegalMode`; wire op size is exactly 8 bytes (independent of Rust enum layout); enqueue publishes tail last; drain publishes head only after consuming the payload; `vblank_priority_ops` fire first.
- `panic`: panic waits for VBlank before clearing LCDC.7 when LCD is enabled; emits DI + dumps fault code byte + writes VRAM directly + HALTs; section is the only audited VRAM source outside `video_commit`.

### 4.3 Integration

- `nucleus_fits_bank0_budget`: encoded total fits under 16 KiB minus future reservations.
- `nucleus_section_order_pinned`: `build_bank0_nucleus_sections` returns sections in the expected order.
- `runtime_nucleus_hash_deterministic`: two invocations of `compute_runtime_nucleus_hash_for_test()` return the same value, and the value is independent of `BuildIdentityBlock` hash field contents (zeroed during normalization).
- `runtime_nucleus_hash_excludes_compile_profile`: changing a `CompileProfile` selector does not change the hash.
- `isr_residency_pure`: no ISR stub or handler reads or writes outside Bank0/HRAM/fixed-WRAM. (Pre-`ReachabilityValidation` smoke test; the full validation is Epic B.)
- `single_vram_writer`: walking every section's `MachineEffect` set, only `video_commit` and `panic` (with the `PanicBypass` annotation) emit `VramAccess`; only `video_commit` emits `OamAccess`.

### 4.4 Yield round-trip

`scheduler::yield_round_trip` is the most load-bearing integration test. It builds a tiny dummy "inference slice" (a single `JR` that touches a memory cell, then `Yield::Frame`), runs it via an in-tree gameroy invocation if available, and asserts:

1. The scheduler dispatches the slice (PC enters the slice address).
2. TIMA fires after the configured period; the timer ISR sets `yield_requested`.
3. The slice's yield check observes the flag and saves continuation.
4. The scheduler regains control; `LivenessCounters::progress_epoch` was advanced by the slice's manual call to a `record_progress` helper.
5. The scheduler re-dispatches; the slice resumes from the saved continuation.

If the gameroy adapter is not yet available as a `dev-dependencies` (F-A7 may not have landed), the test asserts only on the *encoded section bytes* — the AsmIR shape. Still meaningful: a regression in `emit_yield_check` or `emit_arm_tima` will fail the byte-shape assertion.

### 4.5 Snapshot

- `runtime_nucleus_hash_snapshot.txt`: pinned hash for the bring-up policy. Drift fails the test; updating it goes through the T2.5 release-note process.
- `bank0_section_sizes.json`: per-section encoded size. Updates are noted in the PR description.

### 4.6 Negative

- `panic::emits_di_then_halt`: confirms the section starts with DI and ends with a HALT/JR loop.
- `panic::waits_for_vblank_before_lcdc_disable`: confirms the LCD-enabled path spins until `LY >= 144`.
- `video_commit::queue_full_raises_typed_fault`: queue at capacity raises `FaultCode::UiCommitQueueFull` (not `UiCommitOutsideLegalMode`).
- `video_commit::illegal_mode_write_raises_typed_fault`: an attempted write in mode 3 raises `FaultCode::UiCommitOutsideLegalMode`.
- `scheduler::livelock_detection`: a slice that never advances `progress_epoch` raises `FaultCode::LivenessTimeout` past the threshold.
- `text::no_vram_access_machine_effect`: walking the encoded text section, `MachineEffect::StoreToVram` does not appear.
- `keyboard::step_emits_only_queue_ops`: same property for keyboard.

### 4.7 Workspace-wide invariant (deferred)

`grep_no_direct_vram_writes` and `grep_no_direct_mbc_writes` integration tests live in `gbf-runtime/tests/single_writer_smoke.rs` (`#[ignore]`d until `gbf-test` lands). The first confirms `video_commit` and `panic` are the only two sources of VRAM/OAM writes in the workspace; the second confirms `gbf-runtime::banking` is the only source of MBC register writes (this is F-A4's invariant; F-A5's smoke test is a courtesy cross-check).

## 5. Implementation order

### 5.1 Single-PR shape (load-bearing)

**F-A5 ships as one PR.** The PR closes seven open tasks (`bd-17b` T-A5.1, `bd-1cv` T-A5.2, `bd-fcm` T-A5.3, `bd-3ys` T-A5.4, `bd-t0y` T-A5.5, `bd-1d2` T-A5.6, `bd-15y` T-A5.7) and the parent feature bead (`bd-2r1`) in one step. The same PR lands the small upstream additions enumerated in §1.1.x (the `gbf-abi::fault::FaultCode::UiCommitQueueFull` variant, the `gbf-abi::shell::RuntimeShellModule` enum, and the `ExecutionContext` / `InterruptDiscipline` annotations) so the workspace stays consistent at every commit boundary. The review packet (§11) ships in the same PR.

The earlier draft considered a per-module split for review tractability; the practical objection is the modules are too tightly cross-linked for the split to actually reduce review effort:

- Every module references the IRQ vectors (T-A5.1).
- `text`, `keyboard`, and `panic` all reference `video_commit::UiCommitOp`.
- `scheduler` references all of them.
- `keyboard` reads the joypad cached state.
- `panic` references the font for glyph render.

A two-PR split would land half the modules in a state where their tests cannot pass (because their `match` arms reference variants the second PR adds). A one-PR landing keeps every commit green.

The PR is large by raw line count (≈3000 LOC including tests) but is lock-step with the eight reference-shell modules T2.4 already pinned, so the review surface is well-bounded: every file in the diff maps to exactly one of the eight modules or to one of the small upstream additions.

### 5.2 Within-PR DAG and recommended order

Within the single PR, the modules have a real DAG that drives the within-PR sequencing:

```
T-A5.6 video_commit             ─── depends on F-A2 (PpuMode + STAT_REGISTER), F-A3 (FaultCode + new UiCommitQueueFull)
T-A5.1 boot + interrupts        ─── depends on T-A5.6 (drain entry-point symbols), F-A2 (IRQ vectors), F-A3 (BuildIdentityBlock),
                                       F-A4 (shipped — `lower_banking_shadow_zero_init`, the HRAM banking-shadow constants for adjacency, and `mark_isr*`)
T-A5.2 scheduler                ─── depends on T-A5.1 (HRAM zeroed; Timer ISR wired), F-A2 (frame timing), F-A3 (LivenessCounters)
T-A5.3 joypad                   ─── depends on T-A5.1 (IRQ vectors wired), F-A2 (joypad register)
T-A5.4 text + font              ─── depends on T-A5.1 (boot init), T-A5.6 (UiCommitOp enum)
T-A5.5 keyboard                 ─── depends on T-A5.3 (joypad cached state), T-A5.4 (text print)
T-A5.7 panic                    ─── depends on T-A5.1 (boot vectors so the panic_entry is reachable), T-A5.4 (font for glyph render),
                                       F-A3 (FaultCode)
```

**Recommended within-PR sequencing (each step is its own commit on the PR branch; the PR squashes or preserves at the author's discretion):**

1. **Upstream additions first.** Land the `gbf-abi::fault::FaultCode::UiCommitQueueFull` variant, the `gbf-abi::shell::RuntimeShellModule` enum, and the `ExecutionContext` / `InterruptDiscipline` annotations. Every dependent step references these. (~half a day; mechanical.)
2. **T-A5.6 video_commit.** Foundational: T-A5.4 and T-A5.5 stage queue ops into it; T-A5.1 wires the drain entry points into the LCD STAT and VBlank ISRs. Implements the WRAM ring + drain + mode gating. (One day.)
3. **T-A5.1 boot + interrupts.** Wires the cartridge entry stub at `$0100..=$0103`, the IRQ dispatch table at `$0040..=$0060`, the five ISR stubs (canonical save-regs / call-handler / restore-regs / RETI prologue), and the five handler bodies. The Timer handler sets `yield_requested`; the LCD STAT handler invokes `video_commit::emit_commit_drain_hblank`; the VBlank handler invokes `video_commit::emit_commit_drain_vblank`. Calls the F-A4 `lower_banking_shadow_zero_init` helper from `runtime_boot_entry` (F-A4 has shipped per §0.0.6, so no placeholder fallback applies), then zero-initializes the F-A5-owned HRAM bytes (`HRAM_ADDR_YIELD_REQUESTED` and the rest of the F-A5 fast-flag region). (One day.)
4. **T-A5.2 scheduler.** The main loop, `dispatch_inference_slice`, `idle_until_frame`, liveness updates, `emit_yield_check`, `emit_arm_tima`. Builds against the now-shipped `gbf-abi::liveness::LivenessCounters`. (One to two days.)
5. **T-A5.3 joypad.** Polling read sequence + WRAM cache + Joypad ISR no-op stub. (Half a day; parallel with step 6.)
6. **T-A5.4 text + font.** The font asset (M0 default 128 glyphs × 16 bytes = 2 KiB), `TextLayout::dmg_default`, `emit_text_print_glyph` staging via `video_commit::emit_queue_op`. (One day; parallel with step 5.)
7. **T-A5.5 keyboard.** Default ASCII 4×10 layout + step builder + cursor handling. Depends on T-A5.3 + T-A5.4. (One day.)
8. **T-A5.7 panic.** DI + WRAM fault byte + wait-for-VBlank-then-disable-LCD + render + HALT. Depends on T-A5.1 + T-A5.4 + F-A3. (Half a day.)
9. **Pinned section order + `runtime_nucleus_hash` + budget test.** Ship `build_bank0_nucleus_sections`, `compute_runtime_nucleus_hash`, `nucleus_fits_bank0_budget`, the section-order pin, and the snapshot fixture. (Half a day.)
10. **Review packet.** Ship the review packet artifacts per §11. (Half a day; parallel with final acceptance gates.)

**Total: ~7 days of focused work.** The critical path is `upstream additions → video_commit → boot/interrupts → scheduler`, then `joypad`/`text` parallel, then `keyboard`, then `panic`, then the cross-module pins. About half the work parallelizes after the first three days.

## 6. Resolved questions

1. **Cooperative yield uses TIMA as a deadline signal, not preemption.** TIMA cannot itself stop a long inner loop; it can only set the HRAM flag. Observation requires a compiler-emitted safe-point poll. F-A5 declares the contract; lowering enforces a `max_safe_point_gap_m_cycles` proof on every emitted loop and micro-kernel. A previous draft framed TIMA as "preemption" and walked it back.
2. **The `yield_requested` flag lives in HRAM, not WRAM.** HRAM is the only RAM accessible during ISRs without bank-switching concerns. The flag must be set by the timer ISR with no danger of bank divergence; HRAM is the only safe place. F-A4 owns only the four-byte banking shadow at `$FF80..=$FF83` and T-A4.2 explicitly leaves "scheduler/fault/fast flags to F-A5"; F-A5 therefore claims the next byte (`HRAM_ADDR_YIELD_REQUESTED = $FF84`, equal to F-A4's `HRAM_BANKING_SHADOW_END_EXCLUSIVE`) and asserts the adjacency at compile time.
3. **`video_commit` is the sole VRAM writer; `panic` is the audited exception via `PanicBypass`.** The alternative (every module writes VRAM with discipline) does not survive complexity growth. The F-A1 `MachineEffect`/`PrivilegeClass` typing plus the `ExecutionContext`/`InterruptDiscipline` annotations introduced by F-A5 make the invariant compiler-enforceable.
4. **`HALT` lives only in the scheduler.** Inference slices never `HALT`; they yield through the polled flag. The reasoning: `HALT` semantics depend on IME state (Pan Docs warns about the halt bug), and the scheduler is the only context with unambiguous IME state.
5. **The M0 font stays in Bank 0 but with reduced glyph count.** The font is now `FONT_TILE_COUNT = 128` glyphs × 16 bytes = 2 KiB, not the original draft's 4 KiB. A 128-glyph bootstrap font keeps the first ROM self-contained without forcing the now-shipped F-A4 banking into F-A5's critical path. A 256-glyph or `LexicalSpec.charset`-sized font moves to a CommonBank in a follow-up bead inside `gbf-codegen` (which owns the cross-bank far-call materialization per F-A4's known-debt row 3).
6. **Panic ships in M0, but it is DMG-safe.** Without panic, faults silently freeze. The compromise is shipped: DI + WRAM fault byte + wait-for-VBlank-then-disable-LCD + on-screen glyphs + HALT. The wait-for-VBlank step exists because Pan Docs warns that disabling LCDC.7 outside VBlank may damage real DMG hardware. Full `FaultSnapshot` + SRAM persistence is F-D1.
7. **The Joypad ISR is a no-op.** F-A5 does not use STOP-mode wake. The IRQ vector exists only to satisfy the dispatch table layout. A future power-management feature can fill it in.
8. **The keyboard's default layout is the M0 4×10 ASCII grid.** Epic G (`gbf-data`) owns `LexicalSpec.charset`; F-A5 ships only the default for bring-up. When Epic G lands, `keyboard::default_layout` is replaced with a `LexicalSpec`-driven constructor.
9. **The `UiCommitPlan` is a build-time policy, not runtime data.** `vblank_priority_ops` is a `Vec<UiCommitOpKind>` that the build pins; runtime adaptation (e.g., dynamic priority based on dirty-region pressure) is a follow-up.
10. **`runtime_nucleus_hash` includes the font asset bytes but excludes `BuildIdentityBlock` hash fields.** Changing the font bumps the hash — that is correct: a font change *is* a nucleus change for the purposes of the T2.5 drift gate. The four `BuildIdentityBlock` lineage hashes are zeroed during normalization to break self-reference (`runtime_nucleus_hash` cannot include itself). `runtime_nucleus_hash` also excludes `CompileProfile` identity, since the same nucleus is shared across multiple compile profiles.
11. **No CGB support.** DMG only. The interrupt mask vocabulary is the DMG five. CGB-specific timing trims, double-speed mode, VRAM DMA semantics, and the CGB palette format are explicit non-goals. `UiCommitOp::SetDmgPalette` (not `SetTilePalette`) reflects this.
12. **The yield-round-trip test asserts on encoded bytes if gameroy is unavailable.** Cross-emu integration is gated on F-A7 landing. F-A5's test ships in two modes: (a) full round-trip via gameroy if `dev-dependencies` is workable; (b) byte-shape assertion otherwise. Both are meaningful.
13. **`SchedulerPolicy::bring_up` is the only ship-default.** A previous draft proposed shipping `Bringup` / `Default` / `Trace` / `Recovery` policies; walked back because the M0 deliverable is a single bring-up ROM. Other profiles are introduced in M1+ when there is a `CompileProfile` to switch on.
14. **Emit `RuntimeShellModule` annotations as a typed enum from an upstream shared crate.** The enum lives in `gbf-abi` or `gbf-foundation`, not `gbf-policy`, because `gbf-policy` depends on `gbf-runtime` (or sits at a peer level that consumes runtime-emitted artifacts). `gbf-runtime` must not depend on `gbf-policy`. A previous draft re-exported from `gbf-policy` and walked it back.
15. **The IRQ dispatch table uses 8-byte slots, not 3-byte packed.** A future optimization (`OPT-A5.1`) could pack the vectors into 3 bytes each and use the slack for tiny ISR code; M0 does not pursue it.
16. **Each ISR stub uses the canonical PUSH-AF/BC/DE/HL prologue.** A future optimization could use SP-relative scratch; M0 keeps the canonical prologue for simplicity and so the M-cycle accounting is uniform.
17. **The interrupt latency budget has two fields, not one.** `max_interrupt_entry_latency_m_cycles` (CPU dispatch + vector + stub save = ~31 M-cycles) and `max_interrupt_total_occupancy_m_cycles` (entry + handler body + teardown). Pan Docs gives the CPU dispatch as 5 M-cycles before PC reaches the vector; F-A5's entry-latency math reflects that.
18. **ISR handlers do not write `IF` on the normal path.** Pan Docs §"Interrupts" specifies the CPU clears the serviced IF bit on dispatch. F-A5 supplies a narrow `emit_clear_pending_if_bit` helper only for the rare case where software intentionally discards a pending interrupt request, and the helper preserves unrelated IF bits via mask arithmetic.
19. **`UiCommitOp` variants are bounded micro-ops only.** The original draft included `ClearRegion { x, y, w, h }` (unbounded work disguised as a queue item). M0 replaces it with `FillGlyphRun { x, y, len, glyph }` whose worst-case cost is bounded by `len`. `SetTilePalette` is renamed to `SetDmgPalette` to avoid a CGB-sounding name in a DMG-only RFC.
20. **The commit queue uses an explicit 8-byte wire encoding (`UiCommitWireOp`), not Rust enum layout.** `size_of::<UiCommitOp>()` is not a stable hardware ABI. `UI_COMMIT_WIRE_OP_BYTES = 8` is the wire format; the producer/consumer write/read 8 bytes per slot regardless of which `UiCommitOp` variant is encoded.
21. **The commit queue protocol is publication-ordered.** Producer writes the full payload, then publishes the tail as the final byte. Consumer reads tail before payload and only consumes when the slot is fully published. Single-byte tail/head publication on LR35902 is naturally atomic against an interrupt boundary.
22. **HBlank drain caps at one op per ISR; OAM writes are VBlank-only in M0.** Mode-0 length varies per scanline and ISR entry overhead consumes most of it. M0 does not reorder the queue inside an ISR; reordering may be added later only with an explicit starvation proof.
23. **`FaultCode::UiCommitQueueFull` and `FaultCode::UiCommitOutsideLegalMode` are *separate* faults.** Queue capacity exhaustion and an attempted illegal-mode write are different bugs; conflating them was a previous-draft mistake. The shipped `FaultCode` enum has only `UiCommitOutsideLegalMode = 0x0040`; F-A5 introduces `UiCommitQueueFull = 0x0041` as a small bundled addition (§1.1.x), keeping both faults inside `FaultDomain::Ui`.
24. **`ExecutionContext` / `InterruptDiscipline` annotations live in `gbf-asm` by default.** F-A5's audit walk needs typed annotations on each section so the walk does not depend on string identity of section names. The default placement is `gbf-asm::section` with the new fields wired through `SectionPrivilege` (or a sibling `SectionAnnotations` struct attached to `Section`). The alternative — keeping `gbf-asm` minimal and carrying the same information in a `gbf-runtime::audit::SectionAudit` map — is acceptable; the F-A5 PR description records which path was taken. The audit-walk test is identical against either path.

## 7. Risks

| Risk                                                              | Likelihood | Mitigation                                                                                                                           |
|-------------------------------------------------------------------|------------|--------------------------------------------------------------------------------------------------------------------------------------|
| Bank 0 budget overshoots                                          | Low        | The M0 font is now 2 KiB (down from 4 KiB) and `Bank0Free` headroom is >8 KiB. If overshoot still occurs, first audit text/font; second move font to a CommonBank (F-A4 has shipped, so the lease helpers exist; the missing piece is the normal-payload far-call materializer, which is owned by `gbf-codegen` per F-A4's known-debt row 3); third revisit reserved slack. |
| Interrupt entry latency exceeds policy                            | Low        | The ISR stubs are short by construction. Entry latency is computed (CPU 5 + vector 4 + PUSH×4 16 + CALL 6 = 31 M-cycles minimum); handlers do minimal work. The acceptance tests `interrupts::isr_entry_latency_under_policy_bound` and `interrupts::isr_total_occupancy_under_policy_bound` enforce both halves of the budget. |
| Compiler does not prove safe-point gap, slice never observes yield flag | Medium | F-A5 declares the `max_safe_point_gap_m_cycles` contract; lowering must enforce it. If F-A5 lands before the lowering proof, the yield mechanism is best-effort. The risk is owned by the lowering bead and the F-A5 reviewer asks call it out. |
| Yield round-trip fails on real hardware (works on emu)            | Medium     | M0 closure does not include real-hardware testing (that is F-A7's job). The yield round-trip test asserts on encoded bytes if gameroy is unavailable; real-hardware shake-out is a follow-up bead. |
| `runtime_nucleus_hash` is unstable across clean checkouts          | Low        | The deterministic section order + the hash domain separator + the explicit `BuildIdentityBlock`-zeroing normalization make the hash content-derived. The acceptance test `runtime_nucleus_hash_deterministic` confirms. |
| `video_commit` queue races between producer and ISR consumer       | Low        | Single-producer / single-consumer ring with publication-ordered enqueue/drain; head/tail invariants documented and tested. The acceptance tests `video_commit::enqueue_publishes_tail_last` and `video_commit::drain_publishes_head_after_payload` walk the corner cases. |
| Panic screen renders during PPU mode 3 or risks LCD damage        | Low        | Panic explicitly waits for VBlank before clearing LCDC.7. The acceptance test `panic::waits_for_vblank_before_lcdc_disable` enforces this. |
| Liveness threshold is too tight, false-positive `LivenessTimeout` | Medium     | `bring_up()` defaults `max_no_progress_frames` to a conservative value (60 frames ≈ 1 second). The threshold is tunable in `SchedulerPolicy`; `Default` profile may relax it. |
| F-A2 / F-A3 do not land in time                                    | Medium     | F-A2 and F-A3 are *hard* dependencies (per §1.1 / §2.1). F-A5 must not declare local hardware constants or local placeholder ABI types. If F-A2/F-A3 are not ready, F-A5 remains in Draft or lands only non-executable skeletons. |
| Font asset reproducibility: regenerating produces different bytes  | Low        | The font generation script is deterministic and checked in (`tools/font/build_font.py`). The acceptance test `text::font_size` confirms the byte count; a snapshot test confirms the bytes themselves. |
| Cross-bank far-call accidentally introduced without `BankLease`    | Low        | F-A4's `mbc_write_provenance_audit` already gates every `MachineEffect::StoreToMbcRegister` to provenance from a `BankingPreLayoutLowering` instance — any F-A5 module emitting a raw MBC write fails the audit. F-A5 emits zero MBC writes; the §4.6 single-writer smoke test cross-checks. |

## 8. Claim-to-gate matrix (closure-style)

The closure skills (`.agents/skills/asm-bead-closure/SKILL.md`, the patterns generalize) require this for non-trivial beads. Pre-emptive matrix for F-A5 closure:

| Claim                                                                                                | Gating test / artifact                                                       |
|------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------|
| Cartridge header layout is correct                                                                  | `boot::cartridge_header_layout`                                              |
| IRQ vectors at `$0040..=$0060` jump to the ISR stubs                                                | `boot::irq_vector_jumps`                                                     |
| Init sequence zeroes HRAM shadow region                                                              | `boot::shadow_registers_zeroed_at_init`                                      |
| ISR stubs are annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler`                   | `interrupts::isr_stubs_are_isr_marked`                                       |
| Handlers do not write `IF` on the normal path                                                        | `interrupts::handlers_do_not_clobber_unrelated_if_bits`                      |
| VBlank handler bumps `frame_count`                                                                   | `interrupts::vblank_handler_bumps_frame_count`                               |
| Timer handler sets `yield_requested`                                                                 | `interrupts::timer_handler_sets_yield_requested`                             |
| Each handler entry latency is under policy bound                                                     | `interrupts::isr_entry_latency_under_policy_bound`                           |
| Each handler total occupancy is under policy bound                                                   | `interrupts::isr_total_occupancy_under_policy_bound`                         |
| `emit_yield_check` polls the right HRAM byte and falls through correctly                             | `scheduler::yield_check_emits_expected_sequence`                             |
| `emit_arm_tima` programs TAC/TMA/TIMA to a representable deadline                                    | `scheduler::tima_deadline_is_representable`                                  |
| `max_safe_point_gap_m_cycles` is within the deadline minus jitter                                    | `scheduler::max_safe_point_gap_within_deadline`                              |
| Every `HALT` site is preceded by `EI`                                                                | `scheduler::halt_invariant`                                                  |
| `LivenessCounters::no_progress_frames` increments correctly                                          | `scheduler::livelock_detection`                                              |
| `FaultCode::RepeatedCheckpointNoProgress` raises on repeated last_checkpoint                         | `scheduler::repeated_checkpoint_no_progress`                                 |
| `bring_up()` policy values fit within `FRAME_M_CYCLES`                                               | `scheduler::default_policy_fits_bring_up`                                    |
| Yield round-trip works (encoded byte shape and/or gameroy)                                           | `scheduler::yield_round_trip`                                                |
| Joypad polling sequence matches the active-low pattern, ends with `$30` deselect                     | `joypad::read_emits_expected_sequence`                                       |
| Joypad cache write uses absolute `LD (nn), A`, not `LDH`                                             | `joypad::cache_write_uses_absolute_load`                                     |
| Joypad ISR is a no-op                                                                                | `joypad::isr_is_no_op`                                                       |
| Cached `ButtonState` lives in WRAM                                                                   | `joypad::cached_state_addr_in_wram`                                          |
| Font is exactly `FONT_TILE_COUNT * 16` bytes                                                         | `text::font_size`                                                            |
| Bootstrap installs font tiles into VRAM before LCDC.7 enable                                          | `text::font_installed_before_lcdc_enable`                                    |
| Default text layout is 20×18 visible / 32 BG-map stride                                              | `text::layout_dmg`                                                           |
| `emit_text_print_glyph` stages a `UiCommitOp::PutGlyphCell` (no direct VRAM)                         | `text::print_glyph_stages`                                                   |
| `text` section emits no `MachineEffect::StoreToVram`                                                  | `text::no_vram_access_machine_effect`                                        |
| M0 4×10 keyboard layout exists                                                                       | `keyboard::default_layout`                                                   |
| Cursor movement clamps to bounds                                                                     | `keyboard::cursor_movement`                                                  |
| M0 SpecialKey paths (Backspace, Submit) work                                                         | `keyboard::special_keys`                                                    |
| `keyboard` section emits no `MachineEffect::StoreToVram`                                              | `keyboard::step_emits_only_queue_ops`                                        |
| M0 prompt buffer addresses are within WRAM and non-overlapping                                       | `keyboard::prompt_buffer_addresses_valid`                                    |
| `UiCommitOp` enum is exhaustive over bounded micro-ops                                               | `video_commit::ui_commit_op_exhaustive`                                      |
| Wire-op size is exactly 8 bytes (independent of Rust enum layout)                                    | `video_commit::wire_op_size_is_8_bytes`                                      |
| HBlank drain caps at one op per ISR                                                                  | `video_commit::hblank_drain_max_one_op`                                      |
| HBlank drain never writes OAM                                                                        | `video_commit::no_oam_writes_in_hblank`                                      |
| Drain refuses VRAM writes in PPU mode 3                                                              | `video_commit::no_writes_in_mode_3`                                          |
| Drain refuses OAM writes in modes 2/3                                                                | `video_commit::oam_only_in_modes_0_1`                                        |
| `vblank_priority_ops` fire first                                                                     | `video_commit::vblank_priority`                                              |
| Queue capacity exhaustion raises `FaultCode::UiCommitQueueFull`                                      | `video_commit::queue_full_raises_typed_fault`                                |
| Illegal-mode-write attempt raises `FaultCode::UiCommitOutsideLegalMode`                              | `video_commit::illegal_mode_write_raises_typed_fault`                        |
| Enqueue publishes tail last                                                                          | `video_commit::enqueue_publishes_tail_last`                                  |
| Drain publishes head only after consuming payload                                                    | `video_commit::drain_publishes_head_after_payload`                           |
| Bootstrap VRAM init runs while LCD is off                                                            | `video_commit::bootstrap_runs_with_lcd_off`                                  |
| `video_commit` is the sole non-panic VRAM writer                                                     | `video_commit::sole_vram_writer`                                             |
| `video_commit` is the sole OAM writer                                                                | `video_commit::sole_oam_writer`                                              |
| Panic waits for VBlank before clearing LCDC.7                                                        | `panic::waits_for_vblank_before_lcdc_disable`                                |
| Panic emits DI + dumps fault byte + halts                                                            | `panic::emits_di_then_halt`                                                  |
| Panic renders fault code as glyphs                                                                   | `panic::renders_fault_code_glyphs`                                           |
| Panic section is annotated `ExecutionContext::PanicOnly` + `PanicBypass`                              | `panic::section_marked_exempt`                                               |
| Panic is the only other source of `MachineEffect::StoreToVram`                                        | `panic::is_only_other_vram_writer`                                           |
| Panic dumps fault to `WRAM_LAST_FAULT_ADDR`                                                          | `panic::wram_last_fault_byte_set`                                            |
| `build_bank0_nucleus_sections` returns sections in the pinned order                                  | `nucleus_section_order_pinned`                                               |
| Encoded Bank 0 nucleus fits under 16 KiB minus future reservations                                   | `nucleus_fits_bank0_budget`                                                  |
| `compute_runtime_nucleus_hash_for_test()` is deterministic                                           | `runtime_nucleus_hash_deterministic`                                         |
| `runtime_nucleus_hash` excludes `BuildIdentityBlock` lineage hash bytes (zeroed in normalization)    | `runtime_nucleus_hash_normalization_zeroes_lineage_fields`                   |
| `runtime_nucleus_hash` excludes `CompileProfile` identity                                            | `runtime_nucleus_hash_excludes_compile_profile`                              |
| ISR stubs and handlers are reachable only from Bank0/HRAM/fixed-WRAM (pre-`ReachabilityValidation` smoke) | `isr_residency_pure`                                                    |
| `RuntimeShellModule::*` annotations come from an upstream shared crate, not `gbf-policy`             | `runtime_shell_module_annotations`                                           |
| F-A5 introduces no `unsafe`                                                                          | `#![forbid(unsafe_code)]` at crate root (compiler-enforced)                  |
| No other workspace crate emits `MachineEffect::StoreToVram` outside `video_commit`/`panic`            | `single_writer_smoke::grep_no_direct_vram_writes` (`#[ignore]` until `gbf-test`) |

## 9. References

### Internal

- `history/planv0.md` — line 117 (PPU mode/VRAM accessibility), line 121 (ISR-residency hard rule), line 127 (yielding as compiler feature + liveness contract), line 155 (workspace `gbf-runtime` slot), line 205 (`gbf-runtime` module list), line 309 (`gbf-runtime` authoring posture), line 1626 (Bank 0 nucleus content), line 1646 (boot/header/vectors), line 1651 (font/assets), line 1958 (ISR-residency rule restated), line 1962 (Bank 0 / RuntimeNucleus content list), line 2079 (UI-owned VRAM/OAM contract), line 2167 (auto-yielding ABI), line 2343 (scheduler shape), line 2382 (`UiCommitPlan` shape), line 2398 (liveness fault rules), line 2901 (M0 scope).
- `history/glossary.md` — uses existing terms (`Bead`, `Feature`, `Task`, `Contract`, `Owner`, `RuntimeShellModule`); introduces no new RFC vocabulary.
- `CONSTITUTION.md` — §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §V.3 (silence on success, loud on failure), §VI.1 (single source of truth).
- `.agents/skills/asm-bead-closure/SKILL.md` — closure-skill checklist; the type-boundary, single-writer, and section-role rules apply transitively to F-A5.
- `bd-2r1` (F-A5 feature bead) and child tasks `bd-17b` (T-A5.1), `bd-1cv` (T-A5.2), `bd-fcm` (T-A5.3), `bd-3ys` (T-A5.4), `bd-t0y` (T-A5.5), `bd-1d2` (T-A5.6), `bd-15y` (T-A5.7).
- F-A1 RFC `history/rfcs/F-A1-gbf-asm.md` — provides the `Builder` eDSL, `Section`, `SectionRole`, `MachineEffect`, `PrivilegeClass`, cycle model, and ROM assembler that F-A5 emits sections through.
- F-A2 RFC `history/rfcs/F-A2-gbf-hw.md` — provides the IRQ vectors, `PpuMode`, LCD register addresses, JOYP layout, timer registers, frame timing, and memory-region predicates.
- F-A3 RFC `history/rfcs/F-A3-gbf-abi.md` — provides `InferenceState`/`LivenessCounters`, `FaultCode`/`FaultDomain`, `InterruptPolicy`, `SemanticCheckpointId`/`CompactCheckpointId`, `BuildIdentityBlock`.
- F-A4 feature bead `bd-1sv` (closed) and child tasks `bd-371` (T-A4.1, BankLease/BankGuard types — closed) and `bd-2sv` (T-A4.2, HRAM banking-shadow registers — closed) — F-A4 owns only the four banking-shadow bytes at `$FF80..=$FF83` and the `lower_banking_shadow_zero_init` helper F-A5's boot calls. The `YIELD_REQUESTED` HRAM byte is **F-A5-owned** and lives at `$FF84` per §0.0.6.
- F-A4 follow-up bead `bd-2j4m` (T-A8.8, scripted runtime-ASM conformance smoke suite) — opened in the F-A4 closure commit; depends on F-A1, F-A4, and F-A5 emitted ROMs but is owned downstream and not on F-A5's critical path.
- T2.4 task `bd-37r` (ReferenceShellSpec + future reservations) — defines the pinned reference shell module set and the future-reservation table; F-A5 emits the typed `RuntimeShellModule::*` annotations.
- T2.2 task `bd-1g9` (RuntimeChromeBudget emitter) — consumes F-A5's `RuntimeShellModule` annotations and the deterministic section order to compute slot capacities.
- T2.5 task `bd-177` (runtime_nucleus_hash CI drift gate) — consumes `compute_runtime_nucleus_hash()` and enforces drift detection across PRs.
- F-D1 (`bd-2cna`, persistence), F-D2 (`bd-29wu`, harness), F-D3 (`bd-1zxn`, trace), F-D4 (`bd-191e`, drift monitor), F-D5 (`bd-3ot1`, fault policy + boot validation) — own the Epic D modules deferred from F-A5.
- F-A7 task (`bd-3mxe`, gbf-emu adapter) — consumes F-A5's runtime ROM for deterministic execution; the yield-round-trip test integrates here.
- F-A8 task (`bd-1o08`, gbf-debug agent CLI) — scripts the F-A5 runtime via the F-A7 adapter.

### External

- Pan Docs root: <https://gbdev.io/pandocs/>
- Pan Docs §"Memory Map": <https://gbdev.io/pandocs/Memory_Map.html>
- Pan Docs §"Interrupts": <https://gbdev.io/pandocs/Interrupts.html>
- Pan Docs §"PPU Modes": <https://gbdev.io/pandocs/Rendering.html#ppu-modes>
- Pan Docs §"VBlank and HBlank Interrupts": <https://gbdev.io/pandocs/Rendering.html#vblank-and-hblank-interrupts>
- Pan Docs §"Timer and Divider Registers": <https://gbdev.io/pandocs/Timer_and_Divider_Registers.html>
- Pan Docs §"Joypad Input": <https://gbdev.io/pandocs/Joypad_Input.html>
- Pan Docs §"HALT": <https://gbdev.io/pandocs/Reducing_Power_Consumption.html#using-the-halt-instruction>
- Pan Docs §"Tile Data": <https://gbdev.io/pandocs/Tile_Data.html>
- gekkio CPU manual: <https://gekkio.fi/files/gb-docs/gbctr.pdf>
- gameroy emulator: <https://github.com/RodrigoDornelles/gameroy>

## 10. Appendix: file-by-file change set

| File                                          | Change             | Lines (est.) |
|-----------------------------------------------|--------------------|--------------|
| `gbf-runtime/src/lib.rs`                      | Add `#![forbid(unsafe_code)]`, `extern crate alloc;` (gated), `build_bank0_nucleus_sections`, `compute_runtime_nucleus_hash`, `runtime_nucleus_section_order`, `RUNTIME_NUCLEUS_HASH_DOMAIN` constant, `RuntimeShellModule` re-export. | +60 |
| `gbf-runtime/src/boot.rs`                     | New (replace stub) | ~180         |
| `gbf-runtime/src/interrupts.rs`               | New (replace stub) | ~220         |
| `gbf-runtime/src/scheduler.rs`                | New (replace stub) | ~300         |
| `gbf-runtime/src/joypad.rs`                   | New (replace stub) | ~120         |
| `gbf-runtime/src/text.rs`                     | New (replace stub) | ~140         |
| `gbf-runtime/src/keyboard.rs`                 | New (replace stub) | ~220         |
| `gbf-runtime/src/video_commit.rs`             | New (replace stub) | ~280         |
| `gbf-runtime/src/panic.rs`                    | New (replace stub) | ~120         |
| `gbf-runtime/assets/font_8x8.bin`             | New asset          | 4096 bytes   |
| `gbf-runtime/tests/boot.rs`                   | New                | ~120         |
| `gbf-runtime/tests/interrupts.rs`             | New                | ~150         |
| `gbf-runtime/tests/scheduler.rs`              | New                | ~200         |
| `gbf-runtime/tests/joypad.rs`                 | New                | ~80          |
| `gbf-runtime/tests/text.rs`                   | New                | ~80          |
| `gbf-runtime/tests/keyboard.rs`               | New                | ~100         |
| `gbf-runtime/tests/video_commit.rs`           | New                | ~180         |
| `gbf-runtime/tests/panic.rs`                  | New                | ~80          |
| `gbf-runtime/tests/nucleus.rs`                | New                | ~120         |
| `gbf-runtime/tests/yield_round_trip.rs`       | New                | ~150         |
| `gbf-runtime/tests/single_writer_smoke.rs`    | New (`#[ignore]`)  | ~100         |
| `gbf-runtime/Cargo.toml`                      | Add `static_assertions` (compile-only); add `gameroy` to `[dev-dependencies]` if workable | +4 |
| `gbf-runtime/examples/demo_bank0_rom.rs`      | New                | ~80          |
| `tools/font/build_font.py`                    | New (font generator) | ~150       |

**Total: ~3000 LOC, ~40% of which is tests, plus the 4 KiB font asset.** Comparable in scope to F-A2 closure.

## 11. Review packet requirements

The F-A5 PR ships with a **review packet** as a first-class artifact in the repository, alongside the implementation. The packet is authored *after* implementation (so it can describe real decisions, real surprises, real measured costs, and real Bank-0-budget numbers rather than the RFC's predictions). This RFC therefore specifies only what the packet must *cover*, not what its directory layout, file names, prose, or diagrams should look like in detail.

### 11.1 What the packet must let the reviewer do

A reviewer who is otherwise unfamiliar with F-A5 should be able to answer four questions in one sitting:

1. **Is the implementation correct?** — `planv0.md`-anchored, exhaustive over the modeled domain, total over its inputs, ISR-residency-clean, Bank-0-budget-fitting.
2. **Is it clear and maintainable?** — Single source of truth (`video_commit` is the sole VRAM writer, `panic` is the audited exception); no magic numbers leaked; module boundaries match the design surface this RFC commits to.
3. **Are the riskiest invariants actually proved?** — By tests, by type structure, by section role / privilege class annotations, or by a combination — not by prose alone.
4. **Can I reproduce every claimed output locally?** — Tests, generated artifacts, the font asset reproducibility, the deterministic `runtime_nucleus_hash`.

### 11.2 Required topics

The packet, however structured, must cover at least:

- **Scope statement** — what is in scope for this PR (the eight reference-shell modules), what is intentionally deferred (`persistence`, `trace`, `harness` modules; full `FaultSnapshot`; SRAM persist; `gbf-emu` adapter integration), and which downstream feature/bead owns each deferred path.
- **Reading order** — a recommended sequence: which file or topic to read first (probably `video_commit` then `boot`+`interrupts` then `scheduler`), which to read deeply, which to skim.
- **Diff disposition** — for every file in the PR diff, a one-line classification (deep review / boundary review / skim / mechanical / generated / fixture / config / asset). The list must be exhaustive over `gh pr diff --name-only`.
- **Architecture brief** — how `gbf-runtime` decomposes (the eight reference-shell modules + the three deferred), why each module is where it is, and what the dependency direction is. Reuses material from §3 of this RFC.
- **Bank 0 budget audit** — the actual encoded byte counts per section (vs the §3H estimate), the `nucleus_fits_bank0_budget` test result, and a bytes-per-module bar chart. If any module overshot its estimate, an explanation.
- **`runtime_nucleus_hash` reproducibility evidence** — two clean checkouts produce the same hash. The hash is recorded in the packet as a snapshot value.
- **Correctness dossier** — for each of the highest-risk surfaces (the cooperative yield mechanism; ISR latency; `video_commit` mode gating; liveness fault paths; the panic audit-exempt classification; the font asset reproducibility), the packet records the invariant, the test or type or annotation that proves it, and the failure mode if it ever drifts.
- **Cross-module invariant table** — the section-role/privilege-class/MachineEffect table (the §3.3 table), instantiated against the actual encoded sections.
- **Yield round-trip evidence** — the encoded section bytes for the yield-check helper, and (if gameroy is available as `dev-dependencies`) the live emulator step trace.
- **Pan Docs / `planv0.md` citation table** — every behavioral contract in `gbf-runtime` mapped to its Pan Docs section or `planv0.md` line.
- **Claim-to-gate matrix** — every load-bearing claim from §8 (and any new claims that surfaced during implementation) mapped to its gating test or artifact.
- **Test coverage report** — what `cargo test -p gbf-runtime` runs, how it groups, what it asserts, and any portions deliberately not covered.
- **Reproducibility report** — the exact command set a reviewer runs to regenerate every checked-in artifact (test output, encoded section bytes, font asset bytes, hash snapshot). One top-level script invocation should reproduce all of it.
- **Generated artifacts manifest** — what artifacts (test logs, encoded section snapshots, font bytes hash, runtime_nucleus_hash snapshot, diagrams) ship with the packet, and a reproducibility-fingerprint per artifact.
- **Dependency report** — `cargo tree -p gbf-runtime` plus an explicit confirmation that no `[dev-dependencies]` on training-side crates leak in.
- **Known-debt ledger** — every TODO, FIXME, deferred decision, or known-imperfect aspect introduced or carried by F-A5, with the bead/feature that owns the resolution. Includes the deferred persistence/trace/harness modules, the deferred full `FaultSnapshot`, the deferred CGB support, and the `OPT-A5.1` (packed IRQ vectors) optimization.
- **Out-of-scope ledger** — items that look like F-A5's job but explicitly are not (banking implementation, RuntimeChromeBudget emission, runtime_nucleus_hash CI gate, ReachabilityValidation, RuntimeDriftMonitor, gbf-emu adapter), each with the owning feature.
- **API guide** — the public surface of each module, the value objects, the const constants, the `RuntimeShellModule` annotation map, and the breaking-vs-additive policy.
- **Reviewer checklist** — the binary questions the reviewer should be able to mark off (e.g., "every ISR stub is annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler`", "`video_commit` is the only non-panic source of VRAM writes", "the font is exactly `FONT_TILE_COUNT * 16` bytes", "`build_bank0_nucleus_sections` returns sections in the pinned order", "panic waits for VBlank before clearing LCDC.7", etc.).
- **Cleanliness audit** — confirmation that `#![forbid(unsafe_code)]` is at the crate root, that no `std::collections::HashMap` / `std::sync::*` / `chrono` / `rand` imports were introduced, and that no module redeclares a `gbf-hw` constant.
- **Source-to-artifact traceability** — for at least one representative behavior (the cooperative yield path), a worked example showing the AsmIR shape of `emit_yield_check`, the timer ISR shape, the scheduler resume path, and the encoded byte sequence.
- **Diagrams** — at least the cooperative-yield state machine, the `video_commit` queue lifecycle (producer → ring → drain → VRAM/OAM), the IRQ dispatch flow, the keyboard layout grid, the Bank 0 budget breakdown bar chart. Mermaid sources plus rendered SVGs.
- **`runtime_nucleus_hash` history pin** — the M0 hash value, recorded in `artifacts/calibration/PINNED_HASH_HISTORY.md` (or the path T2.5 chooses) once F-A5 closes.

The packet may add other sections that turn out to be useful at implementation time, but it must not omit the topics above.

### 11.3 Reproducibility property

The packet contains a single top-level `verify-packet` script (or equivalent). Running it in a fresh checkout regenerates every artifact-the-packet-references and fails loudly if any checked-in artifact is stale relative to the current source.

### 11.4 Acceptance bar

The packet is complete only when:

- a fresh-checkout reviewer can run the verify script, all tests pass, all reproducible artifacts match;
- every claim in §8 (claim-to-gate matrix) maps to a concrete gate (test, type, annotation, citation, or generated artifact);
- every file in the PR diff appears exactly once in the diff disposition table;
- the Bank 0 budget audit shows the encoded total fits under the budget;
- `runtime_nucleus_hash` is reproducible across two clean checkouts;
- known-debt and out-of-scope ledgers are present and entries point at owning beads or RFCs;
- the cleanliness audit shows zero introduced uses of disallowed APIs.

## 12. End

This RFC stays inside the F-A5 boundary. Anything beyond the small surface F-A5 consumes from F-A4 (the `lower_banking_shadow_zero_init` boot call, the read-only HRAM banking-shadow constants, and the `InterruptSafetyTable` declaration substrate per §0.0.6) — including F-D1's persistence protocol, F-D2's harness control plane, F-D3's trace pipeline, F-D4's drift monitor, F-D5's full fault policy, Epic B's `ReachabilityValidation`, F-A4's deferred `ResumeWindow`/`Token` and `KeepCurrentProof` plumbing, or Epic E's calibration bundle production — is explicitly deferred. The proposal lets F-A5 close on top of the now-shipped F-A4 ABI without those further features existing, while leaving every seam (ISR stubs annotated `PrivilegeClass::InterruptHandler` + `ExecutionContext::InterruptHandler` + `InterruptSafetyKind::InterruptHandler`; `video_commit` annotated `ExecutionContext::VideoCommitOnly`; `panic` annotated `ExecutionContext::PanicOnly` + `PanicBypass` + `InterruptSafetyKind::InterruptDisabled`; `LivenessCounters` updates; `RuntimeShellModule` annotations sourced from `gbf-abi` per §1.1.x; `compute_runtime_nucleus_hash` deterministic over a normalized image) shaped for them to plug in cleanly.

### 12.1 Decisions (formerly reviewer asks)

The earlier draft posed five reviewer asks. Each is now resolved as a decision; cross-checks at PR review are still welcome, but the implementation should proceed against these decisions:

1. **Decision: keep the `SchedulerPolicy::bring_up()` reserve split.** `hard_ui_reserve`, `video_commit_margin`, and `adaptive_headroom` should remain separate fields because they are controlled by different evidence:
   - `hard_ui_reserve`: fixed M0 UI work before inference dispatch.
   - `video_commit_margin`: derived from the current `UiCommitPlan`.
   - `adaptive_headroom`: scheduler policy knob for later profiles.
   A fused M0 constant would be simpler but would hide the cause of budget regressions.
2. **Decision: keep the M0 font in Bank 0, but reduce the default glyph count.** A 128-glyph bootstrap font (`FONT_TILE_COUNT = 128`) keeps the first ROM self-contained without forcing the now-shipped F-A4 banking into F-A5's critical path. A 256-glyph or `LexicalSpec.charset`-sized font moves to a CommonBank in a follow-up bead inside `gbf-codegen` (the lease helpers exist; the normal-payload far-call materializer is the still-missing piece per F-A4's known-debt row 3).
3. **Decision: keep the panic bypass, but make it DMG-safe.** Panic may bypass `video_commit`, but it must wait for VBlank before disabling LCD and writing VRAM directly. The `panic::waits_for_vblank_before_lcdc_disable` test enforces this.
4. **Decision: change the M0 `UiCommitOp` set to bounded wire ops.** Use `PutGlyphCell`, `FillGlyphRun`, `SetDmgPalette`, `PutOamSprite`. Avoid unbounded `ClearRegion`; avoid CGB-sounding `SetTilePalette` in a DMG-only RFC. Higher-level helpers (clear-row, panic glyph render) expand into one or more bounded queue entries before enqueueing.
5. **Decision: TIMA is the right yield mechanism, but only as a *deadline signal*.** The contract requires a stronger compiler-side proof: lowering must guarantee a maximum safe-point gap (`SchedulerPolicy::max_safe_point_gap_m_cycles`) for every emitted loop and micro-kernel. Without that proof, TIMA can set the flag and a slice can still never observe it.

F-A1, F-A2, F-A3, and F-A4 have all shipped (`7a5c687`, `a69c2e2`, `6ad156c`, `6feae98`). F-A5 lands on top of the F-A4 banking ABI as described in §0.0.6 — calling `lower_banking_shadow_zero_init` from boot, declaring `HRAM_ADDR_YIELD_REQUESTED = $FF84` immediately after F-A4's banking-shadow region, populating `InterruptSafetyTable` via F-A4's `mark_isr*` helpers, and emitting zero MBC register writes. The lowering `max_safe_point_gap_m_cycles` proof is owned by Epic B; F-A5 declares the contract, lowering enforces it. F-A5 closure does not assume the deferred F-A4 known-debt items (`KeepCurrentProof` producer, `ResumeWindow`/`Token` lifetimes, normal-payload far-call materializer) — those remain follow-ups. With that, F-A5 closes and unblocks F-D1, F-D2, F-D3, F-D4, F-D5, F-A7's adapter integration, F-A8's debug CLI, and the M0 demo ROM that boots, draws text, and accepts keyboard input.
