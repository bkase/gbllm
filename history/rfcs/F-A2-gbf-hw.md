# RFC F-A2: `gbf-hw` — verified memory map, MBC5, LCD/timing, calibration schema

| Field          | Value                                                              |
|----------------|--------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A2)                                   |
| Status         | Draft (post-F-A1 update; single-PR closure of T-A2.0–T-A2.7)       |
| Feature bead   | `bd-3sk`                                                           |
| Open tasks     | T-A2.0 (cartridge header), T-A2.1 (target), T-A2.2 (memory), T-A2.3 (mbc5), T-A2.4 (lcd/timing), T-A2.5 (interrupts), T-A2.6 (joypad), T-A2.7 (calibration schema) |
| Closed tasks   | none — every module is still `//! Module stub.`                    |
| Plan reference | `history/planv0.md` line 115 (DMG/MBC5 facts), line 117 (VBlank timing), line 121 (ISR-residency hard rule), line 140 (workspace `gbf-hw` slot), line 200 (`gbf-hw::{target, memory, timing, calibration, mbc5, lcd, interrupts, joypad}`), line 214 (single source of truth), line 225 (calibration schema and bundle ownership), line 2091 (MBC5 RAM-enable canonical `$0A`), line 2901 (M0 scope), line 2931 (`no_std + alloc` mandate) |
| Glossary       | `history/glossary.md` (uses existing terms; introduces no new RFC vocabulary) |
| Constitution   | `CONSTITUTION.md` §I (correctness by construction), §III (shifting left), §IV (reproducibility), §VI (single source of truth) |

## Project orientation: where this feature sits

### 0.0.1 The big picture

`gbllm2` is a hardware-aware compiler plus cooperative runtime that targets a real DMG Game Boy with an MBC5 cartridge. The end goal is to run a quantized transformer (or recurrent equivalent) on a Game Boy Color-class device with a reproducible, agent-debuggable build. The architecture is decomposed into five products and three shared contracts: `gbf-hw` is one of the three shared contracts and owns the **target contract** — the verified facts about what hardware the compiler is targeting and what timing/calibration envelope the runtime must honor. (`planv0.md` lines 17–22.)

The project is delivered in milestones M0 → M6:

- **M0** (this RFC's milestone): bring up the foundation stack — `gbf-asm` (typed eDSL), `gbf-hw` (this), `gbf-abi` (live execution contract skeleton), `BankLease`/`BankGuard` ABI, Bank0 cooperative runtime skeleton, `gbf-emu` deterministic emulator adapter, `gbf-debug` agent CLI. The deliverable is a ROM that boots, draws text, accepts keyboard input, and is debuggable from a script.
- **M1**: oracle stack + first quantized dense kernel + first `CompileRequest`.
- **M2** through **M6**: shared micro-kernels, expert dispatch, sequence state, full interactive text generation, and finally calibration/autotune (where `gbf-bench` produces the concrete calibration bundles whose **schema** F-A2 ships).

`gbf-hw` is a *leaf* in the dependency graph (depends only on `gbf-foundation`). Every other workspace crate that touches Game Boy reality — `gbf-asm`, `gbf-runtime`, `gbf-codegen`, `gbf-bench`, `gbf-oracle`, `gbf-emu`, `gbf-policy`, `gbf-report` — depends on `gbf-hw` and never the other way around.

### 0.0.2 What this feature is for

F-A2 fills the `gbf-hw` crate with verified, Pan-Docs-anchored constants and exhaustive, constructor-validated types for everything compiler and runtime need to know about LR35902 + MBC5 + DMG hardware. It is the workspace's only legitimate home for: the cartridge header literals (Nintendo logo, cartridge type, ROM/RAM size codes), the 16-bit memory map, MBC5 banking-register semantics, PPU mode/accessibility tables, frame/VBlank timing, interrupt vectors, joypad layout, and the *layered calibration schema* (`PlatformCalibrationBundle` / `KernelCalibrationBundle` / `RuntimeCalibrationBundle`).

Concretely, F-A2 unblocks five downstream features and **closes one already-shipped TODO**:

- **F-A1 (`gbf-asm`) — already shipped**: F-A1 has merged with explicit `// TODO(F-A2): move these cartridge constants to gbf-hw once the MBC5 module is populated.` markers in `gbf-asm/src/rom.rs`. F-A2 closure includes the rewire: `gbf-asm::rom` re-exports its cartridge-header types from `gbf-hw::cartridge_header`. See §0.0.5 below.
- **F-A4 (`BankLease`/`BankGuard`)**: leasing protocol is built around the typed MBC5 register addresses + canonical `$0A` RAM-enable value.
- **F-A5 (Bank0 runtime)**: boot wires `INT_VECTOR_*`, the cooperative scheduler arms TIMA via `TAC_REGISTER`, `video_commit` guards on `PpuMode::vram_accessible_in`, the joypad reader inverts the active-low `JOYP` register.
- **Epic B `ReachabilityValidation`**: split predicates `is_isr_resident_legal_dmg` / `is_isr_resident_legal_cgb` / `is_isr_io_register_allowed` are the substrate for whole-program ISR-safety analysis.
- **Epic E (`gbf-bench`)**: produces concrete calibration bundles against the schema this RFC commits to.

### 0.0.3 Why the single-source-of-truth posture matters

The dominant failure mode this crate exists to prevent is *constants drifting across crates*. A 16-bit memory map looks small — twelve named regions, ~ten named registers, five interrupt vectors — but each constant has at least three downstream consumers. Without `gbf-hw` as a hard dependency, every consumer redeclares its own copy of `0xC000`, every consumer eventually disagrees about whether `$D000..=$DFFF` is "WRAM" or "switchable WRAM bank", and the bug surface multiplies by the consumer count. F-A2's mechanism for keeping that property is not policy: it is (a) `pub const` items doc-anchored to Pan Docs, (b) exhaustive enums whose `match` totality is compiler-checked, (c) a workspace-wide grep test that flags address-literal redeclaration in any other crate.

### 0.0.4 What this feature deliberately does *not* do

F-A2 is pure data and pure predicates. No IO. No state. No allocation outside the single `Vec<MeasuredKernelProfile>` field. It does not produce calibration bundles (Epic E owns that), it does not run the ISR residency analysis (Epic B Stage 12), it does not implement the bank-switching protocol (F-A4), and it does not drive the joypad or the LCD (F-A5). Every one of those features depends on F-A2; none of them belong inside it. The boundary is enforced by the dependency graph, not by code review.

### 0.0.5 What changed since this RFC was first drafted: F-A1 has shipped

F-A1 (`gbf-asm`) has now merged in three PRs (`ec10b45`, `53d1d82`, `7a5c687`) plus a review-packet harness (`0b9c786`). This is a non-trivial change in the implementation context for F-A2, because **F-A1 had to ship cartridge-header constants before F-A2's `cartridge_header` module existed**. F-A1's RFC §11.1 explicitly anticipated this: "Define hardware constants locally in `gbf-asm/src/rom.rs` for now (with a `// TODO(F-A2): move to gbf-hw::mbc5` annotation). When F-A2 (`bd-3sk`) populates `gbf-hw::mbc5`, `gbf-hw::memory`, and `gbf-hw::interrupts`, replace the local constants with re-exports."

The concrete consequences for F-A2 closure:

1. **F-A1 has chosen specific names for the shared cartridge-header types**, and F-A2 must adopt those names rather than introduce a parallel naming scheme. Moving the same constants under a different name would force a rename pass through `gbf-asm/src/rom.rs` and its 100+ tests for no semantic gain. The names that have already shipped (and that F-A2 inherits) are:
   - `MbcType` (not `CartridgeType`), with three variants: `Mbc5`, `Mbc5Ram`, `Mbc5RamBattery`.
   - `RomSize` with variants `Kib32`, `Kib64`, `Kib128`, `Kib256`, `Kib512`, `Mib1`, `Mib2`, `Mib4`, `Mib8`.
   - `RamSize` with variants `None`, `Kib8`, `Kib32`, `Kib128`, `Kib64` (note the order — Pan Docs lists `$04 = 128 KiB` before `$05 = 64 KiB`).
   - `DestinationCode` (`Japan` / `Overseas`) — F-A1 added this; the original F-A2 RFC did not model it.
   - `header_byte()` accessor (not `header_code()`).
   - `NINTENDO_LOGO: [u8; 48]` constant.
   - `RomSize::bank_count()` and `RomSize::bytes()` accessors. F-A2 keeps these and additionally adds `kib()` (a small additive helper) so consumers can ask "how many KiB?" without dividing by 1024.

2. **The composite `CartridgeHeader` struct stays in `gbf-asm::rom`.** F-A1 already owns `CartridgeHeader { title, mbc_type, rom_size, ram_size, destination_code, new_licensee_code, mask_rom_version }` because that struct is a *build configuration* (with user-controllable fields like `title` and `mask_rom_version`), not a hardware fact. F-A2 owns only the typed enums and the immutable logo bytes; F-A1 keeps the composite builder and the checksum algorithms. This mirrors the spirit of F-A1 §9.2 (which already lists the field-by-field cartridge layout) but with the typed enums sourced from `gbf-hw`.

3. **F-A1 has its own memory-map-ish constants in `gbf-asm/src/layout.rs`** (`ROM_BANK_SIZE`, `ROM0_START`, `ROM0_END_EXCLUSIVE`, `ROMX_START`, `ROMX_END_EXCLUSIVE`, `ROM0_THUNK_POOL_START`). These use *exclusive* end semantics (e.g., `ROM0_END_EXCLUSIVE = 0x4000`) because layout math is half-open-range-natural. F-A2's `gbf-hw::memory` provides the *inclusive* form (`ROM_BANK0_END = 0x3FFF`) because predicate math is closed-range-natural. The two views are equivalent (`*_END_EXCLUSIVE = *_END + 1`), and we keep both: layout consumers in `gbf-asm` keep their exclusive constants because the layout algorithms are written against half-open ranges; predicate and memory-map consumers in `gbf-runtime`/`gbf-codegen` use the inclusive form. The deliberate non-goal is "force `gbf-asm::layout` to redeclare from `gbf-hw::memory`"; the canonical-form rule applies to *new* consumers, not to F-A1's already-shipped layout pass. The grep test (§3.4) allowlists `gbf-asm/src/layout.rs`'s exclusive-form constants explicitly.

4. **F-A1 owns `BankIndex` and `AddressSpace` enums (in `gbf-asm/src/layout.rs`).** These describe layout-time placement targets (e.g., `BankIndex::Rom(7)`, `AddressSpace::RomX`) and are conceptually distinct from F-A2's `MemoryRegion` (which classifies a `u16` CPU address into a hardware-defined region). F-A2 does *not* introduce a competing `BankIndex` or `AddressSpace`; consumers that need both are expected to compose `gbf-hw::memory::classify(addr)` with `gbf-asm::layout::AddressSpace` as appropriate. The two enums are intentionally not isomorphic (e.g., `BankIndex` distinguishes `Rom(0)` from `Rom(1)..`, `MemoryRegion` distinguishes `RomBank0` from `RomSwitchable`).

5. **F-A1 already depends on `gbf-hw`** (per `gbf-asm/Cargo.toml`). The dependency edge exists; the migration is purely additive on the `gbf-hw` side and a `pub use` rewrite on the `gbf-asm` side. There are no circular dependencies to untangle.

The migration plan is detailed in §2.X "Closing F-A1's `// TODO(F-A2)` markers." Section §3A (cartridge header) is rewritten below to use F-A1's actual naming. The §16 claim-to-gate matrix gains an explicit row for the F-A1 import-rewire.

## 0. TL;DR

### 0.1 RFC self-check before implementation

This RFC is ready to implement only if the following are true:

- `gbf-hw::cartridge_header` exposes the cartridge-header types F-A1 already declared locally with `// TODO(F-A2)` markers, **using F-A1's existing names**: `NINTENDO_LOGO: [u8; 48]`, `MbcType` (variants `Mbc5` / `Mbc5Ram` / `Mbc5RamBattery`), `RomSize` (`Kib32..=Mib8`), `RamSize` (`None / Kib8 / Kib32 / Kib128 / Kib64` in this Pan-Docs order), `DestinationCode` (`Japan` / `Overseas`), and a `header_byte()` accessor on each enum.
- F-A1's `gbf-asm/src/rom.rs` is updated by this PR to `pub use` the canonical types from `gbf-hw::cartridge_header`; the `// TODO(F-A2)` comment is removed; F-A1's existing `gbf-asm` tests still pass without modification (they reference `RomSize::Kib32` etc. by their re-exported path).
- The composite `CartridgeHeader` builder struct stays in `gbf-asm::rom` (it carries user fields: `title`, `mask_rom_version`, etc.); F-A2 owns only the typed enums plus the logo bytes.
- ROM-size code conversion is `32 KiB * (1 << code)` for codes `$00..=$08`, not `(2 << code) * 32`.
- F-A1's exclusive-form layout constants (`ROM0_END_EXCLUSIVE`, `ROMX_END_EXCLUSIVE`, `ROM_BANK_SIZE` in `gbf-asm/src/layout.rs`) are *not* renamed or moved; `gbf-hw::memory` ships the inclusive-form predicate-natural version and the grep test allowlists F-A1's layout file.
- F-A1's `BankIndex` and `AddressSpace` enums in `gbf-asm/src/layout.rs` are unchanged; F-A2's `MemoryRegion` is a parallel-but-distinct concept (CPU-address classification vs. layout target).
- MBC5 write-control bands are described as cartridge ROM address-space bands, not as "Bank-0 ROM".
- The MBC5 bank helper is named `rom_bank_number`, not `rom_bank_address`.
- WRAM is split into fixed `$C000..=$CFFF` and `$D000..=$DFFF` (latter switchable on CGB).
- ISR residency and ISR I/O permission are separate predicates.
- `gbf-hw` uses `#![forbid(unsafe_code)]`.
- `serde_json` is dev-only.
- `MeasurementContext` does not require `chrono` in the core no_std crate.
- Calibration schema constructors reject invalid distributions and confidence states.
- `cargo deny` is not claimed to lint source literals; a custom script handles that.
- Pan Docs fragment verification checks HTML IDs, not only HTTP 200.

### 0.2 Summary

`gbf-hw` is the single source of truth for hardware constants, target/cartridge profiles, MBC5 register semantics, LCD/PPU timing, interrupt vectors, joypad layout, and the *layered* calibration schema. Every downstream crate — `gbf-asm` (already shipped; the F-A2 closure rewires its inline cartridge-header literals into `gbf-hw::cartridge_header` re-exports), `gbf-runtime` (boot, scheduler, video commit, banking, joypad reader), `gbf-codegen` (`RomWindowPlan`, `SramPagePlan`, `ReachabilityValidation`), `gbf-bench` (calibration bundle production), and the oracle stack — depends on it. Today the workspace pins `gbf-hw` as a crate (`gbf-hw/Cargo.toml`, `gbf-hw/src/lib.rs`) with eight `//! Module stub.` files and one dependency on `gbf-foundation`. F-A1 has shipped its three PRs and carries explicit `// TODO(F-A2): move these cartridge constants to gbf-hw once the MBC5 module is populated.` markers in `gbf-asm/src/rom.rs`; this RFC closes those markers as part of the F-A2 single PR. F-A4's `BankLease`/`BankGuard` ABI cannot land until MBC5 registers are typed; F-A5.6's `video_commit` cannot enforce VRAM/OAM accessibility without `PpuMode`; F-A5.2's cooperative scheduler needs `DIV`/`TIMA`/`TMA`/`TAC` register addresses and the timer interrupt vector; Epic B's `ReachabilityValidation` is gated on the split `is_isr_resident_legal_*` / `is_isr_io_register_allowed` predicates. F-A2 unblocks all of them.

The RFC proposes a nine-module crate (eight original modules plus `cartridge_header`) that fills the existing scaffold with verified constants, exhaustive enums, and constructor-validated newtypes, plus a small migration patch in `gbf-asm/src/rom.rs` to consume the new types. The pipeline is *not* a runtime pipeline — `gbf-hw` is pure data and pure predicates, no IO, no state, `#![forbid(unsafe_code)]` at crate root, no allocation outside the single `Vec<MeasuredKernelProfile>` field. It is `no_std + alloc` capable per `planv0.md` line 2931 (subject to `gbf-foundation` itself going `no_std`-friendly; today `gbf-foundation` uses `std` and the F-A2 implementation matches that posture, deferring the `#![no_std]` switch to a follow-up that lands when `gbf-foundation` is `no_std` too — see §11.1). The dependency direction is one-way: `gbf-hw` depends only on `gbf-foundation` (for `SemVer`, `Hash256`, `TargetProfileId`, and the calibration ID newtypes) and on `serde` with `default-features = false` and `["derive", "alloc"]`; every other workspace crate depends on `gbf-hw` and never the other way around. `serde_json` is dev-only; `chrono` is *not* a dependency.

```
gbf-hw/src/
  lib.rs                ──┐
  cartridge_header.rs     │  T-A2.0: NINTENDO_LOGO, MbcType, RomSize, RamSize, DestinationCode (names match F-A1)
  target.rs               │  T-A2.1: TargetProfile, ConsoleModel, CartridgeProfile, CapabilitySet
  memory.rs               │  T-A2.2: Memory map constants + MemoryRegion + classify/is_* predicates
  mbc5.rs                 │  T-A2.3: MBC5 write-band addresses + RAM-enable + 9-bit ROM bank number
  lcd.rs                  │  T-A2.4 (part 1): PpuMode + LCD register addresses + accessibility table
  timing.rs               │  T-A2.4 (part 2): Dot clock, frame/VBlank dots and M-cycles, TimingProfile
  interrupts.rs           │  T-A2.5: InterruptSource + vectors + IE/IF + DIV/TIMA/TMA/TAC + priority order
  joypad.rs               │  T-A2.6: JOYP layout + Button enum + ButtonState (post-decode active-high view)
  calibration.rs        ──┘  T-A2.7: PlatformCalibrationBundle + KernelCalibrationBundle + RuntimeCalibrationBundle + CalibrationSetRef + ValidityEnvelope (schema only)

gbf-asm/src/
  rom.rs                     T-A2.0 follow-on: replaces local `MbcType`, `RomSize`, `RamSize`, `DestinationCode`, `NINTENDO_LOGO`
                              with `pub use gbf_hw::cartridge_header::*;` (F-A1's `// TODO(F-A2)` is removed).
                              The composite `CartridgeHeader` builder + checksums stay here.
```

The new modules add roughly 600 LOC of production code plus about 1.4 KLOC of table-driven tests, Pan-Docs-derived fixtures, and serde round-trip checks. There is no example binary — `gbf-hw` is a library crate consumed by every other crate, and its acceptance is gated on a `cargo test -p gbf-hw` matrix plus a workspace-wide grep that proves no other crate locally redeclares the constants this crate exports.

The five most load-bearing decisions in this RFC are:

1. **Predicates, not magic numbers.** `gbf-hw::memory` exposes `classify(addr) -> MemoryRegion` and predicate helpers (`is_wram`, `is_fixed_wram_dmg`, `is_fixed_wram_cgb`, `is_sram_window`, `is_rom_bank0`, `is_rom_switchable`, `is_isr_resident_legal_dmg`, `is_isr_resident_legal_cgb`, `is_isr_io_register_allowed`) so consumers never compare addresses against numeric literals. The only place the literal `0xC000` appears in the workspace is in this crate. A workspace-wide grep test (run from `gbf-test`) backs this up after F-A2 closes.
2. **Memory map and MBC5 register tables are exhaustive enums.** `MemoryRegion` covers every byte of the 16-bit address space exactly once, including the unmapped `$FEA0..=$FEFF` window and the singular `IE` byte at `$FFFF`. `MbcRegisterClass` covers MBC5's four address bands; the `$6000..=$7FFF` window — which is a Reserved no-write band on MBC5 — is *named explicitly* as `MbcRegisterClass::Reserved` so it cannot drift into "unmapped" classification. This terminology aligns with the F-A1 decision that renamed it from `MbcRegisterClass::Unused`.
3. **`PpuMode` is the only authority on VRAM/OAM accessibility.** `vram_accessible_in(mode)` and `oam_accessible_in(mode)` are tabulated against Pan Docs §"PPU Modes". `video_commit` (F-A5.6) reads these predicates; no other crate is allowed to write its own table.
4. **Calibration schema is layered, with explicit `ValidityEnvelope`.** `PlatformCalibrationBundle`, `KernelCalibrationBundle`, and `RuntimeCalibrationBundle` are *separate* types so that platform-level facts (bank-switch cost) survive compiler-internal refactors, kernel-level facts survive runtime-nucleus refactors, and runtime-nucleus facts survive new kernel implementations. Each bundle carries a `ValidityEnvelope { valid_until_compiler_version, valid_until_runtime_nucleus_hash }` so a stale bundle is rejected at policy resolution time, not silently consumed.
5. **`gbf-hw` does not produce calibration bundles.** Production lives in `gbf-bench` (Epic E). F-A2 ships only the schema and the `CalibrationSetRef` shape that `CompileRequest` references. A compile-time grep test in `gbf-test` confirms `gbf-hw` has no `[dev-dependencies]` on `gbf-bench` and no `cfg(test)` measurement code.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A `cartridge_header` module that exports `NINTENDO_LOGO` (the 48-byte logo dump), `MbcType`, `RomSize`, `RamSize`, and `DestinationCode`, with names matching what F-A1 already shipped in `gbf-asm/src/rom.rs`. M0 bring-up uses `MbcType::Mbc5RamBattery` (`$1B`), `RomSize::Mib8` (`$08`), `RamSize::Kib128` (`$04`). F-A1's `gbf-asm/src/rom.rs` is rewritten by this PR to `pub use` these from `gbf-hw::cartridge_header`; the `// TODO(F-A2)` comment is removed; the composite `CartridgeHeader` builder + checksum algorithms stay in `gbf-asm::rom`.
- A `TargetProfile` that bundles `ConsoleModel`, `CartridgeProfile`, `TimingProfile`, and `CapabilitySet`, with a known-good `dmg_mbc5_8mib_128kib()` bring-up constructor.
- A complete DMG/MBC5/CGB-compatible memory map: `ROM_BANK0_BASE..HRAM_END` constants, `IE_REG`, region size constants, the WRAM0/WRAMX split (`$C000..=$CFFF` / `$D000..=$DFFF`), `MemoryRegion` exhaustive enum (with `Wram0` and `WramX` variants), `classify(addr)` total function, and the predicate suite (`is_wram`, `is_fixed_wram_dmg`, `is_fixed_wram_cgb`, `is_sram_window`, `is_rom_bank0`, `is_rom_switchable`, `is_isr_resident_legal_dmg`, `is_isr_resident_legal_cgb`, `is_isr_io_register_allowed`).
- MBC5 write-band addresses (`RAMG`, `BANK1`, `BANK2`, `RAMB`), the canonical RAM-enable value `$0A`, the disable value `$00`, an `MbcRegisterClass` enum (with `Reserved` for `$6000..=$7FFF`), `classify_mbc_write_address`, and `rom_bank_number(bank1, bank2) -> u16` that assembles a 9-bit ROM bank *number* (not address) from the BANK1/BANK2 split.
- `PpuMode` (HBlank, VBlank, OAMSearch, Drawing) with `vram_accessible_in` / `oam_accessible_in` predicates, all LCD-control register addresses (`LCDC`, `STAT`, `LY`, `LYC`, `SCY`, `SCX`, `WY`, `WX`, `BGP`, `OBP0`, `OBP1`, `DMA`), and `VBLANK_LY_THRESHOLD = 144`.
- `DOT_CLOCK_HZ`, `NORMAL_M_CYCLES_PER_SECOND`, `DOUBLE_SPEED_M_CYCLES_PER_SECOND`, `FRAME_DOTS`, `FRAME_M_CYCLES` (= 17556), `VBLANK_DOTS`, `VBLANK_M_CYCLES` (= 1140), `FRAMES_PER_SECOND` ≈ 59.7, plus a `TimingProfile` value object (`dot_clock_hz` + `dots_per_m_cycle` + `frame_dots` + `vblank_dots`, with `frame_m_cycles()` / `vblank_m_cycles()` derived) and a `dmg_timing()` const constructor.
- `InterruptSource` (VBlank, LcdStat, Timer, Serial, Joypad), the five vector addresses `$0040..=$0060`, `IE_REGISTER` and `IF_REGISTER` addresses, `TIMA_REGISTER`/`TMA_REGISTER`/`TAC_REGISTER` addresses, `vector_for(source)`, `ie_bit(source)`, `if_bit(source)`, and a Pan-Docs-aligned priority table.
- `JOYP_REGISTER`, the two select bits, the four direction/button bit constants, a `Button` enum (8 variants), and a `ButtonState { bits: u8 }` value object with `is_pressed` and a static `just_pressed(prev, cur, b)` edge-detection helper. The post-decode view is active-high; the active-low decode lives in F-A5.3's joypad reader.
- The calibration *schema*: `CycleDistribution` (constructor-validated: finite/non-negative mean, monotone percentiles), `CalibrationConfidence` (constructor-validated: non-zero sample count, finite/non-negative stddev, derived class), `CalibrationConfidenceClass` (Strong/Reasonable/Weak with public `STRONG_CONFIDENCE_MIN_SAMPLES = 1000` / `REASONABLE_CONFIDENCE_MIN_SAMPLES = 100` thresholds), `ValidityEnvelope`, `MeasurementContext` (carries a `MeasurementTarget` enum so emulator XOR hardware is enforced by the type, not by a runtime check), `UnixTimestampMillis` (in-crate timestamp newtype, no `chrono` dependency), `PlatformCalibrationBundle` (carries both `target_profile` and `target_family`), `KernelCalibrationBundle` (family-level; rejects empty profile sets), `RuntimeCalibrationBundle` (family-level), `CalibrationSetRef` (all three layers `Option`al; policy resolution decides which layers a given compile mode requires), plus the `MeasuredKernelProfile` value type.
- A `cargo test -p gbf-hw` matrix that, by itself, proves every constant matches Pan Docs (or cites Pan Docs as the source) and that every predicate is total over its domain.
- A workspace-wide grep test in `gbf-test` (added once F-A2 closes; the grep itself can land here behind a `#[ignore]` smoke test until `gbf-test` is set up) that confirms no other crate redeclares the address constants this crate exports.

### 1.2 Non-goals (deferred)

- **Concrete calibration bundles.** F-A2 ships the *schema* only. `gbf-bench` (Epic E, F-E2/F-E3) measures actual bank-switch cost, SRAM page cost, timer ISR cost, kernel cycle profiles, scheduler overhead, overlay install cost, and trace event cost on real emulator/hardware. F-A2 closure is *not* gated on any concrete bundle existing.
- **`ReachabilityValidation`.** The whole-program ISR-residency analysis is owned by Epic B Stage 12 (`bd-18d`). F-A2 supplies the `is_isr_resident_legal_*` and `is_isr_io_register_allowed` predicates that the analysis consumes; F-A2 closure does not require the analysis itself.
- **`BankLease`/`BankGuard` runtime ABI.** F-A4 (`bd-1sv`, T-A4.1 — `bd-371`, T-A4.2 — `bd-2sv`) authors the runtime primitives that *use* MBC5 register addresses to switch banks safely. F-A2 typesthe registers; F-A4 wraps them in a leasing protocol.
- **HRAM shadow registers.** The shadow state for MBC5 BANK1/BANK2/RAMB and RAM-enable lives in `gbf-runtime::banking` (T-A4.2). F-A2 declares the register addresses and the canonical RAM-enable value; the shadow protocol is F-A4's deliverable.
- **Cooperative scheduler implementation.** F-A5.2 (`bd-1cv`) authors the `yield_requested` HRAM flag and the TIMA-armed yield; F-A2 ships only the timer register addresses and the timer interrupt vector.
- **Joypad reader.** F-A5.3 (`bd-fcm`) drives the JOYP register through select-line cycling and inverts the active-low encoding. F-A2 ships the `Button` enum, `ButtonState` view, and the bit constants; F-A2 does not perform any IO.
- **Text renderer / font.** F-A5.4 (`bd-3ys`). F-A2 has no awareness of tile data, palettes, or glyphs.
- **CGB / GBC features.** `ConsoleModel::Cgb` is enumerated for future use, but F-A2 does not ship CGB-specific timing trims, double-speed mode behavior, VRAM DMA semantics, or the CGB palette format. The `dmg_mbc5_8mib_128kib()` bring-up profile sets `console = Dmg`.
- **MBC1/MBC2/MBC3/MBC6/MBC7 register semantics, and rumble variants.** F-A1 has already shipped `MbcType` with three variants (`Mbc5`, `Mbc5Ram`, `Mbc5RamBattery`); F-A2 keeps that exact shape rather than expanding to seven variants. The original F-A2 RFC proposed enumerating MBC1/2/3/6/7 and rumble flavors as reserved slots; closure walks that back. New variants (e.g., `Mbc1`, `Mbc5Rumble`) are *additive* in a follow-up bead and do not require renaming existing variants. Until then, `gbf-hw::mbc5` ships only the MBC5 register module; `gbf-hw::cartridge_header::MbcType` ships only the three Mbc5 variants F-A1 uses.
- **Layout-pass concepts.** `BankIndex` and `AddressSpace` (placement-target enums in `gbf-asm/src/layout.rs`) are layout-pass abstractions, not hardware facts. F-A2's `MemoryRegion` describes the CPU-address partition; `BankIndex`/`AddressSpace` describe where the layout pass *places* a section. The two are intentionally distinct and remain in their respective crates. F-A1's `gbf-asm::layout` exclusive-form constants (`ROM0_END_EXCLUSIVE = 0x4000`, `ROMX_END_EXCLUSIVE = 0x8000`, `ROM_BANK_SIZE = 16 * 1024`, `ROM0_THUNK_POOL_START`) are *not* moved or renamed; F-A2's `gbf-hw::memory` ships the inclusive-form (`ROM_BANK0_END = 0x3FFF`, etc.) and the grep test allowlists F-A1's exclusive-form constants in `gbf-asm/src/layout.rs` (the only legitimate reason to redeclare an address-space boundary outside `gbf-hw`).
- **Calibration *production* against gameroy.** Owned by `gbf-bench` (`gbf-emu` provides the deterministic execution policy, `gbf-bench` measures and emits bundles). F-A2 has no `dev-dependencies` on `gbf-bench` or `gbf-emu`.
- **Workspace-wide grep enforcement.** The grep test (proving no other crate redeclares `0xC000`/`0xFF80`/etc.) lives in `gbf-test` once that crate exists. F-A2 closure ships the test as a `#[ignore]`d smoke test colocated with `gbf-hw`'s `tests/` directory; promoting it to a full workspace gate is a follow-up bead.

## 2. Background and existing state

### 2.1 What is already in tree

`gbf-hw` itself is still mostly stubbed; F-A1 (`gbf-asm`) is now fully shipped on `main`.

**`gbf-hw` (this crate, mostly stubbed):**

- `gbf-hw/Cargo.toml` — pinned, `publish = false`, depends on `gbf-foundation` and `serde`/`serde_json` (today both are normal deps; F-A2 closure moves `serde_json` to `[dev-dependencies]` and pins `serde` with `default-features = false` and `features = ["derive", "alloc"]`).
- `gbf-hw/src/lib.rs` — declares the eight scaffolded modules (`calibration`, `interrupts`, `joypad`, `lcd`, `mbc5`, `memory`, `target`, `timing`) and contains a single doc-comment. F-A2 closure adds a ninth module (`cartridge_header`) and adds `#![forbid(unsafe_code)]` at the crate root.
- `gbf-hw/src/{target, memory, mbc5, lcd, timing, interrupts, joypad, calibration}.rs` — every file is exactly `//! Module stub.`.

**`gbf-foundation` (mostly stable):**

- `gbf-foundation/src/ids.rs` exposes the string-ID and numeric-ID newtype macros plus existing IDs (`TargetProfileId`, `CompileProfileId`, `TargetFamilyId`, `CheckpointId`, `WorkloadId`, `CalibrationSetRef` — currently a string-id placeholder, will be removed by F-A2 closure in favour of the struct in `gbf-hw::calibration`; `LayerId`, `ExpertId`, `BudgetSlotId`).
- F-A2 closure adds the calibration ID family (`PlatformCalibrationId`, `KernelCalibrationId`, `RuntimeCalibrationId`, `CalibrationCohortId`, `KernelImplId`, `RuntimeNucleusId`, `KernelSpecId`) as `string_id!` newtypes alongside the existing ones — purely additive, mechanical.
- `Hash256` and `SemVer` already exist (`gbf-foundation/src/hash.rs`, `gbf-foundation/src/semver.rs`); `gbf-foundation` is currently `std`-using (the modules import from `std::fmt`/`std::str::FromStr`).

**`gbf-asm` (F-A1 — fully shipped):** PRs `ec10b45`, `53d1d82`, `7a5c687`, plus review-packet harness `0b9c786`. F-A1 modules now in tree: `builder`, `cycle_model`, `effect`, `encoder`, `isa`, `layout`, `lib`, `listing`, `lowering`, `provenance`, `relax`, `rom`, `section`, `symbols`, `test_support`, plus a `tests/fixtures` directory. Three pieces of F-A1 are directly relevant to F-A2:

1. `gbf-asm/src/rom.rs` declares `NINTENDO_LOGO`, `MbcType` (variants `Mbc5` / `Mbc5Ram` / `Mbc5RamBattery`), `RomSize` (variants `Kib32` … `Mib8`), `RamSize` (variants `None`, `Kib8`, `Kib32`, `Kib128`, `Kib64`), `DestinationCode` (`Japan` / `Overseas`), and the composite `CartridgeHeader { title, mbc_type, rom_size, ram_size, destination_code, new_licensee_code, mask_rom_version }` builder, with header-checksum and global-checksum algorithms. The line `// TODO(F-A2): move these cartridge constants to gbf-hw once the MBC5 module is populated.` (currently at `rom.rs:28`) tags the migration point.
2. `gbf-asm/src/layout.rs` declares its own layout-pass constants and enums: `ROM_BANK_SIZE = 16 * 1024`, `ROM0_START = 0x0000`, `ROM0_END_EXCLUSIVE = 0x4000`, `ROM0_THUNK_POOL_START = 0x3F00`, `ROMX_START = 0x4000`, `ROMX_END_EXCLUSIVE = 0x8000`; plus the placement-target enums `BankIndex { Rom(u16), Sram(u8), Wram, Hram, Vram, Oam }` and `AddressSpace { Rom0, RomX, Wram, Hram, Sram, Vram, Oam }`. These are layout-pass abstractions and do *not* migrate into `gbf-hw`; the constants use exclusive-end semantics because the layout algorithms are written against half-open ranges.
3. `gbf-asm/Cargo.toml` already lists `gbf-hw = { path = "../gbf-hw" }` as a dependency. The dependency edge exists; the F-A2 closure only adds `pub use gbf_hw::cartridge_header::{NINTENDO_LOGO, MbcType, RomSize, RamSize, DestinationCode};` to `gbf-asm/src/rom.rs` and deletes the local declarations. F-A1's tests and ROM-builder code reference these symbols by their existing names; no test edits are required.

The `gbf-foundation` dependency is the home for shared ID newtypes. `gbf-foundation` is assumed to expose the calibration IDs by the time F-A2 lands; if not, F-A2 closure adds the minimal missing IDs as a precondition (these are pure newtype wrappers; the addition is mechanical).

### 2.2 What is stubbed

All eight (soon nine) `gbf-hw` modules. There is no test file under `gbf-hw/tests/`, no `examples/`, and no integration coverage. F-A2 fills all stubs, adds a `gbf-hw/tests/` directory with one integration test per module plus a cross-module conformance test, and updates `Cargo.toml` to move `serde_json` to `[dev-dependencies]` and (optionally, in a follow-up) add `proptest` for the small randomized portion of the test matrix.

### 2.2.1 Migration plan: closing F-A1's `// TODO(F-A2)` markers

The F-A2 single-PR closure includes a small surgical change in `gbf-asm/src/rom.rs`:

```diff
- // TODO(F-A2): move these cartridge constants to gbf-hw once the MBC5 module is
- // populated.
- pub const NINTENDO_LOGO: [u8; 48] = [
-     0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
-     0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
-     0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
- ];
- // ... three local enum declarations (MbcType, RomSize, RamSize, DestinationCode) ...
+ pub use gbf_hw::cartridge_header::{
+     DestinationCode, MbcType, NINTENDO_LOGO, RamSize, RomSize,
+ };
```

This is the *only* `gbf-asm` edit in the F-A2 PR. The composite `CartridgeHeader` builder, the `header_checksum` / `global_checksum` algorithms, and the `assemble_rom` function all stay put because they are build-configuration concerns, not hardware constants. F-A1's existing tests in `gbf-asm/src/rom.rs` (`header_checksum_known_vector`, `nintendo_logo_present`, `power_of_two_size`, `ram_size_header_bytes`, `public_enums_reject_unknown_serde_values`, etc.) keep passing because the symbols resolve identically through the re-export.

The reverse migration — anyone in the workspace using `gbf_asm::rom::{NINTENDO_LOGO, MbcType, …}` directly — is unaffected because the `pub use` keeps those names addressable at the original `gbf-asm::rom` paths. New consumers should prefer the `gbf_hw::cartridge_header::*` path; both work.

### 2.3 Downstream pressure on this design

```
gbf-hw  ──▶ gbf-asm           (rom.rs uses NINTENDO_LOGO, MBC5 cartridge type, ROM/RAM size enums; layout.rs uses ROM_SWITCHABLE_BASE/END; interrupt vector reservations come from interrupts.rs)
        ──▶ gbf-runtime       (boot wires INT_VECTOR_*; scheduler reads TAC/TIMA; video_commit guards on PpuMode; banking writes MBC5 registers; joypad reader inverts JOYP)
        ──▶ gbf-codegen       (RomWindowPlan uses ROM_SWITCHABLE_BASE/END + bank size; SramPagePlan uses SRAM_BASE/END + bank size; ReachabilityValidation uses is_isr_resident_legal_* + is_isr_io_register_allowed)
        ──▶ gbf-bench         (PlatformCalibrationBundle / KernelCalibrationBundle / RuntimeCalibrationBundle production)
        ──▶ gbf-oracle        (ScheduleOracle uses TimingProfile; conformance compares against cycle distributions)
        ──▶ gbf-emu           (DeterminismPolicy uses TimingProfile; harness mode honors the same frame budget)
        ──▶ gbf-policy        (CompileRequest carries CalibrationSetRef; ResolvedCompilePolicy validates against PlatformCalibrationBundle.confidence)
        ──▶ gbf-report        (RunManifest cites BuildIdentityBlock + CalibrationSetRef; reports embed RamSize/RomSize names)
```

Every consumer assumes:

- The constants are stable across the M0 window.
- The predicates are *total* (every `u16` classifies into exactly one `MemoryRegion`; every PPU mode answers VRAM/OAM accessibility).
- The MBC5 write-address classification handles both "the byte was written to a real MBC5 register" and "the byte was written to the reserved `$6000..=$7FFF` band" without conflating them with unmapped memory.
- The calibration bundle types are the *only* shape `CompileRequest` and `ResolvedCompilePolicy` see.
- Adding a new MBC variant or LCD register does not require touching consumers — they read predicates, not raw integers.

### 2.4 Engineering-rule grounding (`planv0.md` §"Engineering rules")

This RFC threads the rules tightly:

- **Rule 1** ("All generated executable code originates from `AsmIR` / `Instr` / audited runtime builders, never from ad hoc byte pushes"). `gbf-hw` is *not* an authoring crate; it produces no bytes. But F-A1's ROM builder consumes the `NINTENDO_LOGO` constant *as a `DataBlock::Bytes`* through the typed `Builder` API. This RFC names the constant; F-A1 wraps it. Both rules hold.
- **Rule 5** (deterministic, hashed ROM builds). The cartridge header content (logo, MBC5 cartridge-type byte, ROM size code, RAM size code) flows from `gbf-hw` into F-A1's ROM builder. If `gbf-hw`'s constant changes, the ROM hash changes — and that is the desired property. F-A2 closes the freeze: after this RFC lands, the bytes in the cartridge header are reproducibly anchored.
- **Rule 6** (`no_std + alloc` where practical). `gbf-hw` is *capable* of `no_std + alloc` (no inherent `std`-only dependency); however, the practical declaration is deferred behind `gbf-foundation`'s `no_std` conversion. F-A2 keeps `std::fmt` / `std::error::Error` for symmetry with `gbf-foundation` and `gbf-asm`, and structures the source so the switch is mechanical (`std` → `core` find-and-replace) once the foundation crate flips. See §11.1 for the rationale.
- **Rule 11** (single source of truth). The "do not redeclare these elsewhere" property is the entire point of this crate. The RFC adds a workspace-wide grep test as evidence.
- **Rule 12** (`unsafe` is forbidden by default). None of the modules require `unsafe`.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction).** `BitIndex`-style constructor-validated newtypes (here: `BitIndex` from `gbf-asm` already; in this RFC, `CalibrationConfidence`'s constructor rejects `sample_count == 0` for `Strong`; `RomSize` and `RamSize` are enums whose discriminants are the on-disk header bytes; `MbcType` is enumerated, not a free `u8`).
- **§III (shift left).** Memory region classification is a closed-form `match` on `addr` (compile-time exhaustive over `MemoryRegion`); MBC5 register classification is the same; PPU-mode accessibility is a 4×2 table baked into a `const fn`. No runtime IO, no allocation, no string parsing inside the predicates.
- **§IV.3 (reproducible builds).** Every constant is a `pub const` with a Pan-Docs citation in its doc comment. The doc-comment citations are smoke-tested by `cargo doc`'s deny-warnings mode (already wired in workspace lints) — broken intra-doc links fail the build.
- **§V.3 (silence on success, loud on failure).** `gbf-hw` validates schema-local invariants (sample count, percentile monotonicity, measurement-context disjointness, non-empty kernel profile sets) at construction and during `serde` deserialization, surfacing typed `CalibrationError` variants. Staleness against the current build identity is *not* checked inside `gbf-hw` — `gbf-hw` does not know the current compiler version or runtime nucleus hash, so that check belongs in `gbf-policy::ResolvedCompilePolicy`.
- **§VI.1 (single source of truth).** The crate exists for this rule.

### 2.6 Pan Docs as the primary specification

Every constant in this RFC cites Pan Docs (https://gbdev.io/pandocs/) by section name in its doc comment. Pan Docs is the de facto authoritative reference for LR35902, the MBC family, the PPU, the timer, the joypad, and the cartridge header. Where Pan Docs and the gekkio CPU manual disagree (rare; primarily on edge-case timing of `HALT` and the I/O register reset behavior), Pan Docs wins for `gbf-hw` because Pan Docs is the document the rest of the workspace already cites (F-A1's `gbf-asm` cycle model, F-A5's runtime documentation, `gbf-bench`'s calibration cohort definitions).

The two exceptions where this RFC deliberately deviates from Pan Docs:

1. **MBC5 RAM-enable value.** Pan Docs notes that "any value with `$A` in the low nibble" enables RAM, but warns that "relying on values other than `$0A` is not recommended for compatibility." `gbf-hw` exposes only `MBC5_RAM_ENABLE_VALUE = 0x0A` and `MBC5_RAM_DISABLE_VALUE = 0x00`. There is no `is_ram_enable_value(b: u8) -> bool` helper that accepts the loose interpretation; that would re-create the very ambiguity Pan Docs warns against.
2. **`Echo RAM` writes.** Pan Docs explicitly says "use of this area is prohibited" for Echo RAM (`$E000..=$FDFF`). `MemoryRegion::EchoRam` exists as a classification variant so consumers can detect attempted writes, but `is_isr_resident_legal_dmg($E000..=$FDFF)` returns `false`, treating Echo RAM as a forbidden region for ISR-reachable code/data. The runtime layer is permitted to read Echo RAM during forensics; the compiler never emits stores into it.

### 2.7 Relationship to other M0 features

```
                        ┌─────────────────────────┐
                        │  F-A2: gbf-hw           │   ← this RFC
                        │  (constants + types)    │
                        └────────────┬────────────┘
                                     │
        ┌────────────────────────────┼────────────────────────────┐
        ▼                            ▼                            ▼
┌─────────────────┐         ┌─────────────────┐         ┌─────────────────┐
│  F-A1: gbf-asm  │         │  F-A3: gbf-abi  │         │  F-A4: BankLease│
│  (rom builder   │         │  (live ABI)     │         │  (banking ABI)  │
│   uses logo,    │         │  (uses          │         │  (uses MBC5     │
│   MBC5 type,    │         │   InterruptSrc, │         │   register      │
│   ROM/RAM size) │         │   PpuMode names)│         │   addrs + RAM   │
└─────────────────┘         └─────────────────┘         │   enable value) │
                                                        └────────┬────────┘
                                                                 │
                                                                 ▼
                                                        ┌─────────────────┐
                                                        │  F-A5: Bank0    │
                                                        │  runtime (boot, │
                                                        │  scheduler,     │
                                                        │  video_commit,  │
                                                        │  joypad reader) │
                                                        └─────────────────┘
```

F-A2 is a *leaf* dependency. It blocks T-A2.1 dependents (T-A2.2 through T-A2.7), which in turn block the F-A4 and F-A5 task graphs. Closing F-A2 unblocks all of M0's downstream foundation work.

### 2.8 Beads under this feature

The seven child tasks under `bd-3sk` are:

| Bead     | Task     | Module(s)                | Priority |
|----------|----------|--------------------------|----------|
| (new)    | T-A2.0   | `cartridge_header.rs`    | P0       |
| `bd-17x` | T-A2.1   | `target.rs`              | P0       |
| `bd-1yu` | T-A2.2   | `memory.rs`              | P0       |
| `bd-121` | T-A2.3   | `mbc5.rs`                | P0       |
| `bd-304` | T-A2.4   | `lcd.rs`, `timing.rs`    | P1       |
| `bd-e33` | T-A2.5   | `interrupts.rs`          | P1       |
| `bd-21r` | T-A2.6   | `joypad.rs`              | P2       |
| `bd-xkp` | T-A2.7   | `calibration.rs`         | P2       |

T-A2.0 has no existing bead; the F-A2 single-PR closure creates one and closes it together with T-A2.1..T-A2.7.

T-A2.0 (cartridge header) is independent of all other F-A2 tasks. T-A2.1 blocks T-A2.2..T-A2.7 because every other module either uses `TargetProfile` or carries a `TargetProfileId`. T-A2.2 blocks T-A2.3 (MBC5 write bands live in the cartridge ROM address space) and T-A2.5 (IE/IF live in the IO and IE-byte regions). T-A2.4 has no inbound F-A2 dependencies. The order matters within the PR's commit history (the modules build cleanly when added in dependency order); it does not change the closure shape, which is one PR closing every task.

T-A2.0 is also the closure point for F-A1's already-shipped `// TODO(F-A2)` marker in `gbf-asm/src/rom.rs:28`; the same PR that fills `gbf-hw::cartridge_header` rewrites the local declarations in `gbf-asm/src/rom.rs` to a `pub use gbf_hw::cartridge_header::*;` re-export. See §0.0.5 and §2.2.1 for the migration plan.

## 3. Architecture

### 3.1 Crate-level shape

`gbf-hw` is a *pure data* crate. The entire public surface decomposes into:

1. **Constants** — `pub const` items.
2. **Enums** — exhaustive over the modeled domain.
3. **Value objects** — `Copy + Clone + Debug + Eq + PartialEq + Hash` plain structs, plus `Serialize`/`Deserialize` where the type appears in `CompileRequest`, `ResolvedCompilePolicy`, or any report.
4. **Const constructors and pure predicates** — `const fn` where possible; `fn` otherwise. No closures, no allocation, no `String`.
5. **Schema types for calibration** — `Serialize`/`Deserialize`-friendly bundles, no production logic.

There is *no* runtime state. There are *no* mutable globals. There are *no* IO entry points. The crate root uses `#![forbid(unsafe_code)]` as the primary mechanism for keeping `unsafe` out — this is enforced by the compiler at every build, not by a fragile grep. The source is `no_std + alloc`-ready in shape (no `std::collections::HashMap`, no `std::sync::*`, the only `Vec` use is in `KernelCalibrationBundle::kernel_profiles`); declaring `#![no_std]` is deferred until `gbf-foundation` is itself `no_std` (see §11.1).

### 3.2 Module responsibility table

| Module                | Owns                                                                      | Public surface |
|-----------------------|---------------------------------------------------------------------------|---------------|
| `cartridge_header.rs` | `NINTENDO_LOGO`, `CartridgeType`, `RomSize`, `RamSize`, `header_code()`/`kib()`/`bank_count()` helpers | ~20 items |
| `target.rs`           | `TargetProfile`, `ConsoleModel`, `CartridgeProfile`, `MbcType`, `CapabilitySet`, `dmg_mbc5_8mib_128kib()` | ~12 items |
| `memory.rs`           | Memory map constants (incl. WRAM0/WRAMX split), `MemoryRegion`, `classify`, `is_*` predicates, region size constants | ~32 items |
| `mbc5.rs`             | MBC5 write-band addresses, `MBC5_RAM_ENABLE_VALUE`, `MbcRegisterClass`, `classify_mbc_write_address`, `rom_bank_number` | ~14 items |
| `lcd.rs`              | `PpuMode`, LCD register addresses, `vram_accessible_in`, `oam_accessible_in`, LCD-disabled-aware variants, `VBLANK_LY_THRESHOLD`, `from_stat_bits` | ~18 items |
| `timing.rs`           | Dot clock, normal/double-speed M-cycle rates, frame/VBlank dots, `TimingProfile`, `dmg_timing()` | ~12 items |
| `interrupts.rs`       | `InterruptSource`, vector addresses, `IE_REGISTER` (re-export), `IF_REGISTER`, DIV/TIMA family, `vector_for`, `ie_bit`, `if_bit`, priority order | ~16 items |
| `joypad.rs`           | `JOYP_REGISTER`, select bits, button/direction bits, `Button`, `ButtonState` with method-only access, `is_pressed`, `just_pressed`, `just_released` | ~16 items |
| `calibration.rs`      | Calibration confidence, distribution, validity envelope, three bundle types, `CalibrationSetRef`, `MeasurementContext` + `MeasurementTarget`, `MeasuredKernelProfile`, `UnixTimestampMillis` | ~22 items |

Total public surface: ~160 items, every one anchored to a Pan Docs citation or a `planv0.md` reference.

### 3.3 Type-state of `TargetProfile`

`TargetProfile` is the *only* type in `gbf-hw` that consumers carry around as a value. Every other module's outputs are constants or pure functions. The `TargetProfile` shape is:

```rust
pub struct TargetProfile {
    pub id: TargetProfileId,                // gbf_foundation::TargetProfileId
    pub family: TargetFamilyId,             // gbf_foundation::TargetFamilyId
    pub console: ConsoleModel,
    pub cartridge: CartridgeProfile,
    pub timing: TimingProfile,              // dmg_timing() for DMG, future trims for MGB/CGB
    pub capabilities: CapabilitySet,
}
```

A `TargetProfile` has no mutating methods. To make profile invariants enforceable, its fields are private and exposed through `id()` / `family()` / `console()` / `cartridge()` / `timing()` / `capabilities()` accessors. The known-good bring-up constructor `dmg_mbc5_8mib_128kib()` is `const` and total (it cannot fail). Constructors that accept user input (typically from `gbf-policy`) are non-const and return `Result<TargetProfile, TargetProfileError>`. F-A2 does not introduce a const-panic dependency; compile-time rejection of invalid bring-up profiles is achieved via `const fn` returning `Result` only where the type system can already prove validity (which the bring-up constructor's hard-coded inputs do).

### 3.4 The "single source of truth" enforcement

The architectural commitment that `gbf-hw` is *the* place for these constants is enforced by three mechanisms:

1. **`grep_no_redundant_constants` integration test** (in `gbf-hw/tests/single_source_smoke.rs`, `#[ignore]`d until `gbf-test` lands). Walks the workspace `Cargo.toml`, finds every other crate, and greps each for `0xC000`, `0xFF80`, `0xA000`, `0xFFFF`, `0x4000` (in the address-band sense), `0x0A` (MBC5 RAM-enable), `0x0040` (VBlank vector), `0x0148`/`0x0149` (cartridge header offsets), `17556`, `1140`, and the Nintendo logo first byte `0xCE`. False positives are filtered out via an allowlist colocated with the test. Known legitimate redeclarations (allowlisted explicitly):
   - `gbf-asm/src/encoder.rs` — opcode tables contain `0x0A` as the LR35902 opcode byte for `LD A, (BC)`, not as the MBC5 RAM-enable value.
   - `gbf-asm/src/layout.rs` — the layout pass uses exclusive-end forms (`ROM0_END_EXCLUSIVE = 0x4000`, `ROMX_END_EXCLUSIVE = 0x8000`, `ROM_BANK_SIZE = 16 * 1024`); these are layout-pass abstractions written against half-open ranges, distinct from `gbf-hw::memory`'s inclusive-end forms (`ROM_BANK0_END = 0x3FFF`).
   - `gbf-asm/tests/fixtures/gbdev-opcodes.json` — encoder fixtures.
   The allowlist is a YAML file shipped under `gbf-hw/tests/single_source_smoke.allowlist.yaml`; adding to it requires a comment documenting why the redeclaration is legitimate.
2. **Doc-link denial.** Every module in the workspace that *should* depend on `gbf-hw` carries `//! Authoritative source: [`gbf_hw::{module}`].` doc comments; rustdoc's `--deny broken-intra-doc-links` flag (already enabled on workspace lints) ensures the link resolves.
3. **Source-literal audit.** A custom script `scripts/lints/no-hw-literal-redeclarations.py` scans Rust source files outside `gbf-hw` for hardware-address/header literals and applies an explicit allowlist (e.g., `0x0A` is a legitimate LR35902 opcode byte for `LD A, (BC)` inside `gbf-asm::encoder`'s opcode table). Unapproved redeclarations fail the script with a non-zero exit code. `cargo deny` is *not* used for source-literal scanning — it inspects the dependency tree, license, and advisory database, not source code — and the script ships colocated with `gbf-hw`'s `tests/single_source_smoke.rs` so the workspace-wide gate can be promoted without rewriting infrastructure.

### 3.5 Why the calibration schema lives here, not in `gbf-bench`

The calibration schema *types* (`PlatformCalibrationBundle`, etc.) are consumed by:

- `gbf-policy::CompileRequest` (carries `CalibrationSetRef`),
- `gbf-policy::ResolvedCompilePolicy` (consumes `CalibrationConfidence`, gates on `confidence_class`),
- `gbf-codegen` Stage 11 `ScheduleCostAnalysis` (reads `KernelCalibrationBundle.kernel_profiles`),
- `gbf-codegen` Stage 13 `Backend` (reads `RuntimeCalibrationBundle` for cycle inflation factors),
- `gbf-oracle::ScheduleOracle` (runs against the same cycle distributions),
- `gbf-report::RunManifest` (cites `CalibrationSetRef`),
- `gbf-bench` (produces concrete bundles).

If the schema lived in `gbf-bench`, every consumer would have to depend on `gbf-bench`, which transitively pulls in benchmarking machinery, emulator dependencies, and the `gbf-emu` adapter. That would make `gbf-policy` (a pure data crate) depend on the emulator, which is wrong both architecturally and practically. By keeping the schema in `gbf-hw`, every consumer pays only the `gbf-foundation` + `serde` cost; `gbf-bench` produces values of these types but does not own their shape.

### 3.6 What `gbf-hw` deliberately does not own

- **Cycle costs per `Instr`.** That is `gbf-asm::cycle_model` (F-A1, shipped). `gbf-hw` ships master clock and frame-budget constants; per-instruction costs are computed in `gbf-asm` because they are tied to LR35902 instruction encoding, not to a hardware constant.
- **`AsmIR` / `Instr`.** `gbf-asm::isa` (F-A1, shipped).
- **`MachineEffect` / `PrivilegeClass`.** `gbf-asm::effect` (F-A1, shipped).
- **`SymbolName` / `SymbolTable`.** `gbf-asm::symbols` (F-A1, shipped).
- **Layout-pass abstractions: `BankIndex`, `AddressSpace`, `PlacementProfile`, `LayoutPlan`, `PlacedSection`, `ReservedRange`, layout exclusive-end constants.** `gbf-asm::layout` (F-A1, shipped). These describe *where the layout pass places sections*; they are not address-space classifications. F-A2's `MemoryRegion` is a different question (CPU-address partition); the two coexist.
- **Composite `CartridgeHeader { title, mbc_type, rom_size, ram_size, destination_code, new_licensee_code, mask_rom_version }` builder, `header_checksum`, `global_checksum`, `assemble_rom`, `RomAssemblyError`.** `gbf-asm::rom` (F-A1, shipped). These are build-configuration concerns with user-controllable fields and ROM-emit logic; F-A2 owns only the typed enums and the immutable logo bytes that the builder consumes.
- **Runtime-side bank shadow state.** `gbf-runtime::banking` (F-A4).
- **Continuation/checkpoint/fault types.** `gbf-abi` (F-A3).
- **Workload manifests, prompts, interaction bundles.** `gbf-workload`, `gbf-data`.

The boundary between `gbf-hw` and these crates is enforced by the dependency graph: `gbf-hw` depends on `gbf-foundation` only; `gbf-asm`, `gbf-abi`, `gbf-runtime`, etc. depend on `gbf-hw`. There is no path back the other way.


## 3A. Cartridge header constants (T-A2.0, `cartridge_header.rs`)

**Reference**: Pan Docs §"The Cartridge Header" (`$0100..=$014F`); F-A1 RFC §9.2 (cartridge header layout); current F-A1 implementation in `gbf-asm/src/rom.rs`.

### 3A.1 Why this module exists

`gbf-hw::cartridge_header` owns the ROM-header constants consumed by `gbf-asm::rom`. These are hardware/header facts (Nintendo's fixed logo dump, Pan-Docs-tabulated cartridge type bytes, ROM/RAM size header codes), not target-profile facts. Putting them in `target.rs` would conflate "what is this build" with "what bytes does the cartridge header literally contain"; consumers of `RomSize` are mostly `gbf-asm::rom` (which stamps the header byte) and `CartridgeProfile` (which carries the typed enum), and they want different things.

F-A1 has shipped local copies of these constants in `gbf-asm/src/rom.rs` with `// TODO(F-A2): move these cartridge constants to gbf-hw once the MBC5 module is populated.` The F-A2 single-PR closure migrates the local declarations into `gbf-hw::cartridge_header`, leaves the composite `CartridgeHeader` builder + checksum algorithms in `gbf-asm::rom`, and rewrites the local declarations as `pub use gbf_hw::cartridge_header::{NINTENDO_LOGO, MbcType, RomSize, RamSize, DestinationCode};`. **F-A2 inherits F-A1's existing names** so the migration does not require any test edits.

### 3A.2 The Nintendo logo

```rust
/// Nintendo logo bytes. Stored at cartridge header range `$0104..=$0133`.
///
/// Pan Docs: the boot ROM compares this region against an internal copy and
/// halts on mismatch. The logo's first byte is `$CE`.
pub const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B,
    0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E,
    0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC,
    0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];
```

This matches the bytes already shipped in `gbf-asm/src/rom.rs:30..=34`.

### 3A.3 `MbcType`

The exact shape F-A1 ships, lifted verbatim:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MbcType {
    Mbc5,
    Mbc5Ram,
    Mbc5RamBattery,
}

impl MbcType {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Mbc5           => 0x19,
            Self::Mbc5Ram        => 0x1A,
            Self::Mbc5RamBattery => 0x1B,
        }
    }
}
```

Notes:

- **Three variants only.** F-A1 deliberately ships only the three MBC5 variants the M0 ROM builder actually emits. Other Pan-Docs-tabulated cartridge bytes (`$00 = ROM-only`, `$1C..=$1E = Mbc5 rumble flavors`, `$01..=$13 = MBC1/2/3 family, $20 = MBC6, $22 = MBC7`) are reserved for *additive* variants in a follow-up bead. Adding `Mbc5Rumble` later does not require renaming or re-discriminating.
- **`#[serde(rename_all = "snake_case")]`** keeps JSON/TOML stable for downstream report and policy crates: `"mbc5_ram_battery"`. F-A1's negative-deserialization test (`public_enums_reject_unknown_serde_values`) already pins this against future drift.
- The original F-A2 RFC proposed naming this `CartridgeType` with a `RomOnly` variant and seven MBC5 sub-variants. F-A1's decision to ship `MbcType` with three variants pre-empted that; F-A2 closure adopts F-A1's name and shape.

### 3A.4 `RomSize`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RomSize {
    Kib32,
    Kib64,
    Kib128,
    Kib256,
    Kib512,
    Mib1,
    Mib2,
    Mib4,
    Mib8,
}

impl RomSize {
    /// On-disk header byte at offset `$0148`.
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Kib32  => 0x00,
            Self::Kib64  => 0x01,
            Self::Kib128 => 0x02,
            Self::Kib256 => 0x03,
            Self::Kib512 => 0x04,
            Self::Mib1   => 0x05,
            Self::Mib2   => 0x06,
            Self::Mib4   => 0x07,
            Self::Mib8   => 0x08,
        }
    }

    /// Number of 16 KiB ROM banks (including the fixed bank 0).
    /// Pan Docs: codes $00..=$08 yield 2..=512 banks (`2 * (1 << code)`).
    #[must_use]
    pub const fn bank_count(self) -> u16 {
        match self {
            Self::Kib32  => 2,
            Self::Kib64  => 4,
            Self::Kib128 => 8,
            Self::Kib256 => 16,
            Self::Kib512 => 32,
            Self::Mib1   => 64,
            Self::Mib2   => 128,
            Self::Mib4   => 256,
            Self::Mib8   => 512,
        }
    }

    /// Total ROM size in bytes.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bank_count() as usize * 16 * 1024
    }

    /// Total ROM size in KiB. Additive helper introduced by F-A2; F-A1 uses
    /// `bytes()`. The two are consistent: `kib() * 1024 == bytes()`.
    #[must_use]
    pub const fn kib(self) -> u32 {
        32u32 << (self.header_byte() as u8)
    }
}
```

The `header_byte()`, `bank_count()`, and `bytes()` methods are taken from F-A1 verbatim. The new `kib()` accessor is additive: it gives consumers a Pan-Docs-natural "32 KiB × (1 << code)" reading without requiring division by 1024. The F-A2 RFC's earlier note that the formula is `32 * (1 << code)` (Pan Docs) and *not* the incorrect `(2 << code) * 32` form remains load-bearing.

For code `$00` this yields 32 KiB (2 banks); for code `$08` it yields 8192 KiB = 8 MiB (512 banks), which is exactly MBC5's maximum addressable bank count via the 9-bit BANK1+BANK2 split.

### 3A.5 `RamSize`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RamSize {
    None,
    Kib8,
    Kib32,
    Kib64,
    Kib128,
}

impl RamSize {
    /// On-disk header byte at offset `$0149`. Pan Docs lists $04 = 128 KiB
    /// before $05 = 64 KiB; preserving that order is part of the contract.
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::None   => 0x00,
            Self::Kib8   => 0x02,
            Self::Kib32  => 0x03,
            Self::Kib128 => 0x04,
            Self::Kib64  => 0x05,
        }
    }

    /// SRAM size in KiB.
    #[must_use]
    pub const fn kib(self) -> u32 {
        match self {
            Self::None   => 0,
            Self::Kib8   => 8,
            Self::Kib32  => 32,
            Self::Kib128 => 128,
            Self::Kib64  => 64,
        }
    }

    /// Number of 8 KiB SRAM banks (0 for `None`).
    #[must_use]
    pub const fn bank_count(self) -> u16 {
        (self.kib() / 8) as u16
    }
}
```

The `header_byte()` table is taken from F-A1 verbatim. The new `kib()` and `bank_count()` accessors are additive: F-A1 only exposes the header byte, but `gbf-runtime::persistence` (F-A4) needs a programmatic way to ask "how many SRAM banks must I steward?", and `gbf-codegen::SramPagePlan` (Epic B) needs the byte count.

Pan Docs lists code `$01` as historically reserved (some early documentation used it for 2 KiB SRAM, but the canonical table treats it as unused). F-A2 omits that code from the enum. Code `$05` is 64 KiB, which is out of order relative to `$04`'s 128 KiB; this matches Pan Docs and is preserved verbatim. F-A1 already has a regression test (`ram_size_header_bytes`) that pins `RamSize::Kib64.header_byte() == 0x05` and `RamSize::Kib128.header_byte() == 0x04`; that test continues to pass once the type is re-exported from `gbf-hw`.

### 3A.5.1 `DestinationCode`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCode {
    Japan,
    Overseas,
}

impl DestinationCode {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Japan    => 0x00,
            Self::Overseas => 0x01,
        }
    }
}
```

The original F-A2 RFC did not model `DestinationCode`; F-A1 added it. F-A2 closure absorbs it because it is a hardware-defined header byte at offset `$014A` with exactly two valid values. The composite `CartridgeHeader::default().destination_code = DestinationCode::Overseas` lives in `gbf-asm::rom`.

### 3A.6 Tests

```bash
cargo test -p gbf-hw -- cartridge_header::nintendo_logo_first_byte    # NINTENDO_LOGO[0] == 0xCE
cargo test -p gbf-hw -- cartridge_header::nintendo_logo_length        # NINTENDO_LOGO.len() == 48
cargo test -p gbf-hw -- cartridge_header::mbc_type_header_bytes        # Mbc5 family bytes match Pan Docs ($19/$1A/$1B)
cargo test -p gbf-hw -- cartridge_header::rom_size_kib_formula         # 32 << code for $00..=$08
cargo test -p gbf-hw -- cartridge_header::rom_size_bank_count          # 8 MiB -> 512 banks
cargo test -p gbf-hw -- cartridge_header::rom_size_bytes_consistent    # kib() * 1024 == bytes()
cargo test -p gbf-hw -- cartridge_header::ram_size_kib_table           # round-trip kib() matches Pan Docs
cargo test -p gbf-hw -- cartridge_header::ram_size_64kib_after_128kib  # encoding order matches Pan Docs
cargo test -p gbf-hw -- cartridge_header::destination_code_header_bytes
cargo test -p gbf-hw -- cartridge_header::serde_snake_case_round_trip  # rename_all = snake_case sticks across all four enums
cargo test -p gbf-hw -- cartridge_header::serde_unknown_variants_rejected
```

The serde tests pin `rename_all = "snake_case"` against future drift: a JSON document `{"mbc_type": "mbc5_ram_battery"}` deserializes; `{"mbc_type": "mbc5RamBattery"}` does not. The `serde_unknown_variants_rejected` test mirrors F-A1's existing `public_enums_reject_unknown_serde_values` pattern.

The F-A1-side migration is verified by the F-A1 closure tests: `nintendo_logo_present`, `header_checksum_known_vector`, `power_of_two_size`, `ram_size_header_bytes`, `bank_n_at_correct_offset`, `unused_regions_are_ff`, `deterministic`, `invalid_title_rejected`, `user_header_range_rejected`, `overlapping_sections_are_rejected`, `section_size_mismatch_is_rejected`, `entry_point_is_required`, `invalid_rom_size_for_layout`, `public_enums_reject_unknown_serde_values`. All of these resolve `MbcType`, `RomSize`, `RamSize`, `DestinationCode`, `NINTENDO_LOGO` through the re-export and pass without modification.

### 3A.7 Constitution checkpoints

- §I.1: `MbcType`, `RomSize`, `RamSize`, and `DestinationCode` are exhaustive over the variants F-A2 commits to (which match F-A1's already-shipped variants).
- §VI.1: Cartridge header constants live only in `gbf-hw::cartridge_header`. `gbf-asm::rom` re-exports them via `pub use`.

## 4. Target profile (T-A2.1, `target.rs`)

**Bead**: `bd-17x` (P0). **Reference**: `planv0.md` line 213.

### 4.1 Why `TargetProfile` is the keystone type

`TargetProfile` is the only type in the workspace that bundles "which Game Boy + which cartridge + which capabilities" for a build. `CompileRequest` carries a `TargetProfileId`; the policy resolver looks it up against a registry; every downstream pass operates against the resolved `TargetProfile`. If `TargetProfile` were the wrong shape — too narrow, too loose, or split across multiple types — every pass would either re-derive its own model or carry a bag of ad-hoc fields. Both outcomes destroy the single-source-of-truth property.

The shape this RFC commits to:

```rust
pub struct TargetProfile {
    pub id: TargetProfileId,
    pub family: TargetFamilyId,
    pub console: ConsoleModel,
    pub cartridge: CartridgeProfile,
    pub timing: TimingProfile,                  // populated by T-A2.4
    pub capabilities: CapabilitySet,
}
```

Field-by-field justification:

- **`id: TargetProfileId`** — content-addressed identifier. The same physical Game Boy + cartridge gets the same id whether the profile is constructed in-memory or loaded from disk. The id is computed (in `gbf-foundation`) from a stable serialization of the other fields, so two profiles with identical `console`/`cartridge`/`timing`/`capabilities` are guaranteed to have the same id. This is the property `CompileRequest`'s cache lookup relies on.
- **`family: TargetFamilyId`** — coarser than `id`. All DMG profiles share a `TargetFamilyId`. Calibration bundles carry both the profile and family level when useful: platform measurements are profile-sensitive because cartridge/MBC facts matter (so `PlatformCalibrationBundle` carries both `target_profile: TargetProfileId` and `target_family: TargetFamilyId`), while kernel/runtime measurements are usually family-sensitive (so `KernelCalibrationBundle` and `RuntimeCalibrationBundle` carry only `target_family`).
- **`console: ConsoleModel`** — the major architectural variant. DMG and MGB share an architecture but differ in audio trim and clock-stability characteristics; SGB is DMG-mode under a Super Game Boy adapter; CGB has double-speed mode, VRAM DMA, palettes, and a distinct boot ROM. M0 ships only `Dmg` as a *consumed* variant, but `ConsoleModel` enumerates the others to keep `match` exhaustive across the workspace.
- **`cartridge: CartridgeProfile`** — the MBC + ROM/SRAM size + battery + RTC. `MbcType::Mbc5` is the only fully typed variant in F-A2.
- **`timing: TimingProfile`** — populated from T-A2.4. Owning timing here, rather than in a separate type, lets `TargetProfile` answer "what's the frame budget?" without an extra lookup.
- **`capabilities: CapabilitySet`** — feature flags describing optional hardware (CGB double-speed, VRAM DMA, RTC). M0's bring-up profile sets all three to `false`.

### 4.2 `ConsoleModel`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ConsoleModel {
    Dmg,    // original Game Boy
    Mgb,    // Game Boy Pocket — same architecture, slightly different timing trim
    Sgb,    // Super Game Boy — DMG-mode-only under SGB adapter
    Cgb,    // Game Boy Color — distinct architecture, M0 does not target it
}
```

The enum is exhaustive over the consoles the workspace targets. `Cgb-in-DMG-mode` is *not* a separate variant; that is a `CompileProfile` choice (whether to set the CGB compat byte at offset `$0143` to `0x00` or `0x80`), not a `ConsoleModel` distinction. The header byte computation is owned by F-A1's ROM builder; F-A2 only enumerates the console family.

Pan Docs §"Console Models" anchors the four variants.

### 4.3 `CartridgeProfile` and `MbcType`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CartridgeProfile {
    mbc: MbcType,
    rom_size: RomSize,
    ram_size: RamSize,
    cartridge_type: CartridgeType,
    has_battery: bool,
    has_rtc: bool,
}

impl CartridgeProfile {
    /// Bring-up profile: MBC5 + 8 MiB ROM + 128 KiB battery-backed SRAM.
    pub const fn dmg_mbc5_8mib_128kib_battery() -> Self {
        Self {
            mbc: MbcType::Mbc5,
            rom_size: RomSize::Rom8MiB,
            ram_size: RamSize::Ram128KiB,
            cartridge_type: CartridgeType::Mbc5RamBattery,
            has_battery: true,
            has_rtc: false,
        }
    }

    /// Validates field consistency before returning a profile.
    /// In particular: RTC requires MBC3; MBC5 max ROM is 8 MiB; MBC5 RAM
    /// sizes are 8/32/128 KiB (or None for Mbc5/Mbc5Rumble); the
    /// cartridge_type byte must agree with mbc, has_battery, and (when
    /// applicable) ram_size != None.
    pub fn try_new(
        mbc: MbcType,
        rom_size: RomSize,
        ram_size: RamSize,
        cartridge_type: CartridgeType,
        has_battery: bool,
        has_rtc: bool,
    ) -> Result<Self, TargetProfileError> {
        // checks elided here; see §4.6 for the error variants.
        // Implementation rejects:
        //   - has_rtc && mbc != MbcType::Mbc3
        //   - mbc == Mbc5 && rom_size > Mib8       (impossible today: RomSize maxes at Mib8;
        //                                           kept as an explicit branch reserved for a
        //                                           future enum extension. Not testable until then.)
        //   - mbc == Mbc2 && ram_size != RamSize::None
        //   - cartridge_type implies battery but has_battery == false
        //   - cartridge_type implies RAM but ram_size == None
        //   - cartridge_type implies a non-Mbc5 MBC family but mbc == Mbc5
        Ok(Self { mbc, rom_size, ram_size, cartridge_type, has_battery, has_rtc })
    }

    pub const fn mbc(&self) -> MbcType { self.mbc }
    pub const fn rom_size(&self) -> RomSize { self.rom_size }
    pub const fn ram_size(&self) -> RamSize { self.ram_size }
    pub const fn cartridge_type(&self) -> CartridgeType { self.cartridge_type }
    pub const fn has_battery(&self) -> bool { self.has_battery }
    pub const fn has_rtc(&self) -> bool { self.has_rtc }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MbcType {
    None,    // no MBC — bare 32 KiB ROM, no banking, no SRAM
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
    Mbc6,
    Mbc7,
}
```

Constraints enforced inside `try_new`:

- `rom_size` and `ram_size` are typed; KiB and bank counts come from `RomSize::kib()` / `RomSize::bank_count()` / `RamSize::kib()` / `RamSize::bank_count()` (see §3A). The conversion is `32 * (1 << code)` for ROM codes `$00..=$08`, matching Pan Docs.
- `has_rtc` is only legal when `mbc = Mbc3`. F-A2's `dmg_mbc5_8mib_128kib()` constructor sets `has_rtc = false`; a future MBC3 constructor gates `has_rtc = true` on `mbc = Mbc3`.
- `has_battery` must agree with `cartridge_type` (for example, `Mbc5RamBattery` implies `has_battery = true`).
- All fields are private and accessed through `mbc()` / `rom_size()` / etc. methods, so external code cannot construct a `CartridgeProfile` whose `cartridge_type` disagrees with its other fields.
- `gbf-runtime::persistence` reads `has_battery()` to decide whether SRAM write-back is durable.

### 4.4 `CapabilitySet`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Default)]
pub struct CapabilitySet {
    pub double_speed_mode: bool,    // CGB only — system clock × 2 via KEY1
    pub vram_dma: bool,             // CGB — HDMA / GDMA via $FF55
    pub rtc_present: bool,          // MBC3 only
}
```

`CapabilitySet::default()` returns all-false; the bring-up profile uses `Default::default()`.

### 4.5 Bring-up constructor

```rust
/// DMG with MBC5, 8 MiB ROM, 128 KiB battery-backed SRAM, no RTC.
/// This is the M0 bring-up target. `const`, infallible.
pub const fn dmg_mbc5_8mib_128kib() -> TargetProfile {
    TargetProfile::from_validated_parts(
        TargetProfileId::DMG_MBC5_8MIB_128KIB,    // const id from gbf_foundation
        TargetFamilyId::DMG,                       // const family id
        ConsoleModel::Dmg,
        CartridgeProfile::dmg_mbc5_8mib_128kib_battery(),
        dmg_timing(),
        CapabilitySet { double_speed_mode: false, vram_dma: false, rtc_present: false },
    )
}
```

This constructor is `const`. It cannot fail. Its outputs are the same on every call. Tests verify:

- The id matches the precomputed `TargetProfileId::DMG_MBC5_8MIB_128KIB`.
- A roundtrip through `serde_json` produces an identical value.
- The ROM size is reachable in `MbcType::Mbc5`'s addressable range (8 MiB = 512 banks of 16 KiB; MBC5's 9-bit BANK1+BANK2 covers exactly 512 banks).

### 4.6 Errors

`TargetProfile` constructors that take user input (from `gbf-policy`) return `Result<TargetProfile, TargetProfileError>`:

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TargetProfileError {
    /// Reserved for a future `RomSize` extension. With today's enum
    /// (`Kib32..=Mib8`), this variant cannot fire — the enum saturates at
    /// `Mib8`, which is exactly MBC5's 8 MiB capacity ceiling. Kept so an
    /// extended `RomSize::Mib16` (etc.) can plug in without an error-shape
    /// change.
    RomSizeExceedsMbcCapacity { mbc: MbcType, max_rom_kib: u32, requested_kib: u32 },
    SramSizeExceedsMbcCapacity { mbc: MbcType, max_sram_kib: u32, requested_kib: u32 },
    RtcWithoutMbc3 { mbc: MbcType },
    SramWithMbc2 { requested_kib: u32 },               // MBC2 has only 512 nibbles of internal RAM
    UnsupportedMbcInM0 { mbc: MbcType },               // any non-Mbc5 build path during M0
}

impl core::fmt::Display for TargetProfileError { /* explicit, no thiserror */ }
impl core::error::Error for TargetProfileError {}
```

Per `CONSTITUTION.md` §I, every variant carries enough state to reproduce the error.

### 4.7 Tests (T-A2.1 acceptance)

```bash
cargo test -p gbf-hw -- target::dmg_mbc5_constructor              # bring-up profile is well-formed and round-trips
cargo test -p gbf-hw -- target::serde_round_trip                  # arbitrary profile through JSON
cargo test -p gbf-hw -- target::capability_set_exhaustive         # each cap has a documented semantics
cargo test -p gbf-hw -- target::mbc_capacity_validation           # rejects MBC2 with SRAM, MBC1 with RTC. (The `RomSize > Mib8` branch is reserved for a future enum extension and has no test today.)
cargo test -p gbf-hw -- target::id_is_content_addressed           # two distinct profiles get distinct ids
cargo test -p gbf-hw -- target::family_id_groups_by_console_arch  # all DMG profiles share TargetFamilyId::DMG
```

### 4.8 Constitution checkpoints

- §I.1: `MbcType` and `ConsoleModel` are enumerated; constructing a `match` over them is exhaustive.
- §VI.1: `TargetProfile` is the single workspace point that bundles target identity.

## 5. Memory map (T-A2.2, `memory.rs`)

**Bead**: `bd-1yu` (P0). **Reference**: `planv0.md` line 115; Pan Docs §"Memory Map".

### 5.1 The address-space contract

Every consumer of `gbf-hw::memory` agrees on five rules:

1. The 16-bit address space `$0000..=$FFFF` is partitioned into a finite set of named regions plus the singleton `IE` byte at `$FFFF`.
2. `MemoryRegion` is exhaustive: every `u16` belongs to exactly one variant.
3. Region constants are paired (`*_BASE`, `*_END`), with `*_END` *inclusive*. Half-open `*_BASE..(*_END + 1)` is the standard ranged-pattern idiom; `*_BASE..=*_END` is the standard inclusive form.
4. Region size constants are computed from the base/end pair and asserted in tests.
5. Predicates (`is_wram`, `is_fixed_wram_dmg`, `is_fixed_wram_cgb`, `is_sram_window`, `is_rom_bank0`, `is_rom_switchable`, `is_isr_resident_legal_dmg`, `is_isr_resident_legal_cgb`, `is_isr_io_register_allowed`) are total functions of `u16`. They never return `Option<bool>`.

### 5.2 Constants

```rust
// ROM
pub const ROM_BANK0_BASE:           u16 = 0x0000;
pub const ROM_BANK0_END:            u16 = 0x3FFF;     // inclusive
pub const ROM_SWITCHABLE_BASE:      u16 = 0x4000;
pub const ROM_SWITCHABLE_END:       u16 = 0x7FFF;     // inclusive

// VRAM
pub const VRAM_BASE:                u16 = 0x8000;
pub const VRAM_END:                 u16 = 0x9FFF;

// SRAM (cartridge external RAM window)
pub const SRAM_BASE:                u16 = 0xA000;
pub const SRAM_END:                 u16 = 0xBFFF;

// WRAM (work RAM). Split for CGB compatibility:
//   $C000..=$CFFF is fixed bank 0 on every console.
//   $D000..=$DFFF is fixed on DMG/MGB; switchable (banks 1..=7) on CGB.
pub const WRAM0_BASE:               u16 = 0xC000;
pub const WRAM0_END:                u16 = 0xCFFF;
pub const WRAMX_BASE:               u16 = 0xD000;
pub const WRAMX_END:                u16 = 0xDFFF;

// Echo RAM (mirror of $C000..$DDFF; Pan Docs: prohibited)
pub const ECHO_RAM_BASE:            u16 = 0xE000;
pub const ECHO_RAM_END:             u16 = 0xFDFF;

// OAM (sprite attribute table)
pub const OAM_BASE:                 u16 = 0xFE00;
pub const OAM_END:                  u16 = 0xFE9F;

// Unmapped (Pan Docs: "Not Usable")
pub const UNMAPPED_BASE:            u16 = 0xFEA0;
pub const UNMAPPED_END:             u16 = 0xFEFF;

// I/O registers
pub const IO_BASE:                  u16 = 0xFF00;
pub const IO_END:                   u16 = 0xFF7F;

// HRAM (high RAM, 127 bytes)
pub const HRAM_BASE:                u16 = 0xFF80;
pub const HRAM_END:                 u16 = 0xFFFE;

// Interrupt enable register (singleton)
pub const IE_REG:                   u16 = 0xFFFF;

// Region size constants
pub const BANK0_SIZE_BYTES:           u32 = 16 * 1024;          // 16 KiB
pub const SWITCHABLE_BANK_SIZE_BYTES: u32 = 16 * 1024;          // 16 KiB
pub const SRAM_BANK_SIZE_BYTES:       u32 = 8 * 1024;           // 8 KiB
pub const WRAM0_SIZE_BYTES:           u32 = 4 * 1024;           // 4 KiB ($C000..=$CFFF)
pub const WRAMX_SIZE_BYTES:           u32 = 4 * 1024;           // 4 KiB ($D000..=$DFFF, switchable on CGB)
pub const WRAM_SIZE_BYTES:            u32 = WRAM0_SIZE_BYTES + WRAMX_SIZE_BYTES;
pub const VRAM_SIZE_BYTES:            u32 = 8 * 1024;           // 8 KiB
pub const HRAM_SIZE_BYTES:            u32 = 127;                // $FF80..=$FFFE
pub const OAM_SIZE_BYTES:             u32 = 160;                // 40 sprites × 4 bytes
pub const IO_SIZE_BYTES:              u32 = 128;                // $FF00..=$FF7F
pub const ECHO_RAM_SIZE_BYTES:        u32 = 0xFE00 - 0xE000;    // 7680 bytes
pub const UNMAPPED_SIZE_BYTES:        u32 = 0xFF00 - 0xFEA0;    // 96 bytes
```

Every constant carries a Pan Docs section reference in its doc comment.

### 5.3 `MemoryRegion`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MemoryRegion {
    RomBank0,             // $0000..=$3FFF
    RomSwitchable,        // $4000..=$7FFF
    Vram,                 // $8000..=$9FFF
    Sram,                 // $A000..=$BFFF
    Wram0,                // $C000..=$CFFF — fixed WRAM bank, every console
    WramX,                // $D000..=$DFFF — fixed on DMG/MGB; switchable on CGB
    EchoRam,              // $E000..=$FDFF — Pan Docs: prohibited
    Oam,                  // $FE00..=$FE9F
    Unmapped,             // $FEA0..=$FEFF — Pan Docs: not usable
    Io,                   // $FF00..=$FF7F
    Hram,                 // $FF80..=$FFFE
    InterruptEnable,      // $FFFF (singleton)
}
```

The enum is exhaustive over the address space. Adding a new variant is a breaking change to consumers; F-A2 does not anticipate any.

### 5.4 `classify`

```rust
pub const fn classify(addr: u16) -> MemoryRegion {
    match addr {
        0x0000..=0x3FFF => MemoryRegion::RomBank0,
        0x4000..=0x7FFF => MemoryRegion::RomSwitchable,
        0x8000..=0x9FFF => MemoryRegion::Vram,
        0xA000..=0xBFFF => MemoryRegion::Sram,
        0xC000..=0xCFFF => MemoryRegion::Wram0,
        0xD000..=0xDFFF => MemoryRegion::WramX,
        0xE000..=0xFDFF => MemoryRegion::EchoRam,
        0xFE00..=0xFE9F => MemoryRegion::Oam,
        0xFEA0..=0xFEFF => MemoryRegion::Unmapped,
        0xFF00..=0xFF7F => MemoryRegion::Io,
        0xFF80..=0xFFFE => MemoryRegion::Hram,
        0xFFFF          => MemoryRegion::InterruptEnable,
    }
}
```

The `match` is exhaustive over `u16`. The compiler proves totality at compile time. There is no `_ => unreachable!()` arm.

### 5.5 Predicates

```rust
pub const fn is_rom_bank0(addr: u16) -> bool       { matches!(classify(addr), MemoryRegion::RomBank0) }
pub const fn is_rom_switchable(addr: u16) -> bool  { matches!(classify(addr), MemoryRegion::RomSwitchable) }
pub const fn is_vram(addr: u16) -> bool            { matches!(classify(addr), MemoryRegion::Vram) }
pub const fn is_sram_window(addr: u16) -> bool     { matches!(classify(addr), MemoryRegion::Sram) }
pub const fn is_wram(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Wram0 | MemoryRegion::WramX)
}

/// On DMG/MGB, both $C000..=$CFFF and $D000..=$DFFF are fixed WRAM.
pub const fn is_fixed_wram_dmg(addr: u16) -> bool {
    is_wram(addr)
}

/// On CGB, only $C000..=$CFFF is fixed WRAM; $D000..=$DFFF is switchable.
pub const fn is_fixed_wram_cgb(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Wram0)
}
pub const fn is_oam(addr: u16) -> bool             { matches!(classify(addr), MemoryRegion::Oam) }
pub const fn is_io(addr: u16) -> bool              { matches!(classify(addr), MemoryRegion::Io) }
pub const fn is_hram(addr: u16) -> bool            { matches!(classify(addr), MemoryRegion::Hram) }

/// True iff ISR code or ISR-resident static data may be *placed* at `addr`
/// on DMG (and MGB). This is a placement/residency predicate, not a
/// permission predicate for every load/store an ISR may perform.
///
/// Consumed by Epic B's `ReachabilityValidation` for placement decisions.
/// The DMG variant treats both `$C000..=$CFFF` and `$D000..=$DFFF` as fixed
/// WRAM; on CGB, the residency rule is tighter (use `is_isr_resident_legal_cgb`).
pub const fn is_isr_resident_legal_dmg(addr: u16) -> bool {
    matches!(
        classify(addr),
        MemoryRegion::RomBank0
            | MemoryRegion::Wram0
            | MemoryRegion::WramX
            | MemoryRegion::Hram
            | MemoryRegion::InterruptEnable
    )
}

/// CGB-aware variant. `$D000..=$DFFF` is switchable on CGB and therefore
/// cannot host ISR-resident code/data unless the runtime explicitly pins it
/// to bank 1 with a documented exception.
pub const fn is_isr_resident_legal_cgb(addr: u16) -> bool {
    matches!(
        classify(addr),
        MemoryRegion::RomBank0
            | MemoryRegion::Wram0
            | MemoryRegion::Hram
            | MemoryRegion::InterruptEnable
    )
}

/// True iff a vetted ISR path may directly access this I/O register.
///
/// F-A2 keeps this list intentionally tiny. Runtime-owned ISR scaffolding
/// (F-A4 / F-A5) may widen it via explicitly-tested exceptions; this default
/// admits only the IF register (write-1-to-clear during ISR-internal logic)
/// and the IE register (mask manipulation around critical sections).
///
/// Note that the canonical ISR *exit* mechanism is `RETI`/`EI` + `RET`, not a
/// write to IF; the CPU clears the relevant IF bit and IME automatically when
/// servicing an interrupt. IF is included here for cases where an ISR
/// re-enables a peer source mid-handler.
pub const fn is_isr_io_register_allowed(addr: u16) -> bool {
    matches!(
        addr,
        crate::interrupts::IF_REGISTER | crate::interrupts::IE_REGISTER
    )
}
```

Note that `is_isr_resident_legal_dmg` deliberately does *not* include `MemoryRegion::Io`, because I/O access is not a residency property. Some I/O registers are safe inside a vetted ISR path, but those exceptions belong in `is_isr_io_register_allowed` or in runtime-owned ISR scaffolding, not in the placement predicate. `ReachabilityValidation` uses `is_isr_resident_legal_dmg` for placement and `is_isr_io_register_allowed` for explicitly-vetted I/O accesses; widening either set is a follow-up bead, not a default behavior.

### 5.6 Tests

```bash
cargo test -p gbf-hw -- memory::region_classification              # deterministic full sweep: every u16 maps to a containing region
cargo test -p gbf-hw -- memory::isr_resident_legal_dmg             # exact match: { Bank0, Wram0, WramX, Hram, InterruptEnable }
cargo test -p gbf-hw -- memory::isr_resident_legal_cgb             # exact match: { Bank0, Wram0, Hram, InterruptEnable } — WramX excluded
cargo test -p gbf-hw -- memory::isr_io_register_allowed            # exact match: { IF_REGISTER, IE_REGISTER }
cargo test -p gbf-hw -- memory::region_sizes                       # constants match Pan Docs; computed sizes match base/end pairs
cargo test -p gbf-hw -- memory::predicate_totality                 # full-sweep: is_* predicates are total over u16
cargo test -p gbf-hw -- memory::no_predicate_overlap               # for every addr, exactly one of is_* returns true (excluding is_isr_resident_legal_* which are unions)
cargo test -p gbf-hw -- memory::echo_ram_is_prohibited             # is_isr_resident_legal_dmg($E000..=$FDFF) returns false
cargo test -p gbf-hw -- memory::unmapped_is_prohibited             # is_isr_resident_legal_dmg($FEA0..=$FEFF) returns false
cargo test -p gbf-hw -- memory::ie_byte_is_singleton               # classify($FFFF) == InterruptEnable; is_isr_resident_legal_dmg($FFFF) == true
cargo test -p gbf-hw -- memory::wram_split_dmg_vs_cgb              # is_fixed_wram_dmg($D500) == true; is_fixed_wram_cgb($D500) == false
```

The `region_classification` test does a deterministic full sweep over every `u16` — classifies each address and asserts that the resulting region's `*_BASE..=*_END` actually contains the address. Coupled with exhaustiveness of the `match`, this is a closed-form proof of totality (65536 cases, no randomness, no proptest).

### 5.7 Constitution checkpoints

- §I.1: `MemoryRegion` is exhaustive; `classify` is total.
- §VI.1: Memory map constants live only here.

## 6. MBC5 register semantics (T-A2.3, `mbc5.rs`)

**Bead**: `bd-121` (P0). **Reference**: `planv0.md` line 2091 ("Pan Docs notes the canonical RAM-enable value is `$0A`"); Pan Docs §"MBC5".

### 6.1 The four MBC5 register bands

MBC5 exposes write-intercepted control bands inside the cartridge ROM address space (`$0000..=$7FFF`). This range spans both fixed ROM bank 0 (`$0000..=$3FFF`) and the switchable ROM window (`$4000..=$7FFF`); the MBC intercepts *writes* to these addresses regardless of which underlying ROM region they fall in. Writes do *not* land in ROM; they are intercepted by the cartridge MBC and update an internal banking state. Reads from these bands return the underlying ROM byte. The bands are:

| Band    | Range               | Purpose                                | Width |
|---------|---------------------|----------------------------------------|-------|
| `RAMG`  | `$0000..=$1FFF`     | RAM gate (enable/disable SRAM access)  | 8-bit |
| `BANK1` | `$2000..=$2FFF`     | ROM bank low byte                      | 8-bit |
| `BANK2` | `$3000..=$3FFF`     | ROM bank high bit                      | 1-bit |
| `RAMB`  | `$4000..=$5FFF`     | SRAM bank select (0..=15)              | 4-bit |

The remaining band `$6000..=$7FFF` is *reserved no-write* on MBC5. F-A1 renamed this from the colloquial "Unused" to `MbcRegisterClass::Reserved`, signalling that writing to it is a guarded illegality (the runtime must not emit such writes; `ReachabilityValidation` flags any code that does). Note that `$6000..=$7FFF` is not described as a "register" in Pan Docs's MBC5 register list; the classifier function name reflects this: it classifies *write addresses*, not registers per se.

### 6.2 Constants

```rust
pub const MBC5_RAMG_BASE:          u16 = 0x0000;
pub const MBC5_RAMG_END:           u16 = 0x1FFF;
pub const MBC5_BANK1_BASE:         u16 = 0x2000;
pub const MBC5_BANK1_END:          u16 = 0x2FFF;
pub const MBC5_BANK2_BASE:         u16 = 0x3000;
pub const MBC5_BANK2_END:          u16 = 0x3FFF;
pub const MBC5_RAMB_BASE:          u16 = 0x4000;
pub const MBC5_RAMB_END:           u16 = 0x5FFF;
pub const MBC5_RESERVED_BASE:      u16 = 0x6000;
pub const MBC5_RESERVED_END:       u16 = 0x7FFF;

/// Canonical RAM-enable value. Pan Docs notes that any byte with `$A` in the
/// low nibble enables the SRAM gate, but warns that "relying on values other
/// than `$0A` is not recommended for compatibility." `gbf-hw` therefore
/// exposes only the canonical value; the loose interpretation is intentionally
/// not provided.
pub const MBC5_RAM_ENABLE_VALUE:   u8 = 0x0A;

/// Canonical RAM-disable value (any non-`$0A` value disables; `$00` is the
/// canonical choice).
pub const MBC5_RAM_DISABLE_VALUE:  u8 = 0x00;
```

### 6.3 `MbcRegisterClass`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MbcRegisterClass {
    Ramg,           // RAM enable gate ($0000..=$1FFF)
    Bank1,          // ROM bank low byte ($2000..=$2FFF)
    Bank2,          // ROM bank high bit ($3000..=$3FFF)
    Ramb,           // SRAM bank select ($4000..=$5FFF)
    Reserved,       // $6000..=$7FFF — MBC5 ignores writes; emitting them is illegal
}
```

The enum is exhaustive over MBC5 write-address semantics in the cartridge ROM address space (`$0000..=$7FFF`, spanning fixed bank 0 and the switchable window). `Reserved` is named explicitly (not "Unused") so consumers cannot accidentally treat `$6000..=$7FFF` as a free write band.

### 6.4 `classify_mbc_write_address`

```rust
pub const fn classify_mbc_write_address(addr: u16) -> Option<MbcRegisterClass> {
    match addr {
        0x0000..=0x1FFF => Some(MbcRegisterClass::Ramg),
        0x2000..=0x2FFF => Some(MbcRegisterClass::Bank1),
        0x3000..=0x3FFF => Some(MbcRegisterClass::Bank2),
        0x4000..=0x5FFF => Some(MbcRegisterClass::Ramb),
        0x6000..=0x7FFF => Some(MbcRegisterClass::Reserved),
        _               => None,    // outside the cartridge ROM address space
    }
}
```

The function is named `classify_mbc_write_address` rather than `classify_mbc_register` because `$6000..=$7FFF` is not described as a "register" in Pan Docs's MBC5 register list; it is a no-write reserved band. `Option` is the right shape here because addresses outside `$0000..=$7FFF` are not MBC write addresses at all; classifying `$8000` as "no MBC band" is meaningfully different from classifying it as "Reserved". The caller (typically `gbf-runtime::banking` or `ReachabilityValidation`) decides what to do with `None`.

### 6.5 9-bit ROM bank number assembly

MBC5 supports up to 8 MiB of ROM via a 9-bit bank *number* assembled from `BANK1` (low 8 bits) and `BANK2` (low 1 bit). The function returns a bank number, not a CPU address; the corresponding CPU address is always `$4000..=$7FFF` once the bank is selected.

```rust
pub const fn rom_bank_number(bank1: u8, bank2: u8) -> u16 {
    let bank2_bit = (bank2 & 0x01) as u16;
    let bank1_byte = bank1 as u16;
    (bank2_bit << 8) | bank1_byte
}
```

The output range is `0..=511` (9 bits). Bank 0 (`bank1 = 0`, `bank2 = 0`) is a special case: writing 0 to BANK1 selects ROM bank 1 in some MBCs (notably MBC1), but on MBC5 writing 0 selects bank 0. F-A2 documents this in the doc comment but does not encode the special case in the function (because there isn't one for MBC5). Consumers that need cross-MBC behavior should compose with their MBC-specific helpers.

### 6.6 What `gbf-hw::mbc5` does *not* own

- **Bank shadow state.** The HRAM-resident shadow of the current BANK1/BANK2/RAMB values is owned by `gbf-runtime::banking` (T-A4.2, `bd-2sv`).
- **`BankLease`/`BankGuard` ABI.** Owned by F-A4 (T-A4.1, `bd-371`).
- **MBC5 register *write* code.** `gbf-runtime::banking` emits `LD (rom_addr), reg` instructions through the typed `gbf-asm::Builder`; the addresses come from `gbf-hw::mbc5`, but the emit logic is in the runtime.
- **Critical-section discipline.** Writing BANK1 then BANK2 on MBC5 must be atomic with respect to interrupts to avoid an ISR firing between the two writes and observing inconsistent bank state. The `BankLease` ABI handles this; F-A2 does not.

### 6.7 Tests

```bash
cargo test -p gbf-hw -- mbc5::write_address_classification       # every $0000..=$7FFF address classifies
cargo test -p gbf-hw -- mbc5::address_outside_rom_returns_none   # $8000..=$FFFF returns None
cargo test -p gbf-hw -- mbc5::ram_enable_value                    # MBC5_RAM_ENABLE_VALUE == 0x0A
cargo test -p gbf-hw -- mbc5::bank_number_assembly                # bank1=0, bank2=0 -> 0; bank1=0xFF, bank2=0x01 -> 0x1FF; bank1=0x42, bank2=0 -> 0x042
cargo test -p gbf-hw -- mbc5::bank_number_high_bit_only_uses_lsb  # bank2 = 0xFE has same effect as bank2 = 0
cargo test -p gbf-hw -- mbc5::reserved_band_is_named               # classify_mbc_write_address(0x7000) == Some(Reserved), not Some(Unused)
cargo test -p gbf-hw -- mbc5::loose_ram_enable_not_provided        # there is no exported is_ram_enable_value helper
```

The last test is a *grep-based* test: it checks the public surface of `gbf-hw::mbc5` for the symbol `is_ram_enable_value` and asserts it is absent. This pins the API decision against future drift.

### 6.8 Constitution checkpoints

- §I.1: `MbcRegisterClass` is exhaustive over MBC5's address-band semantics.
- §VI.1: MBC5 semantics live only here; F-A4 imports them.

## 7. LCD modes and frame timing (T-A2.4, `lcd.rs` + `timing.rs`)

**Bead**: `bd-304` (P1). **Reference**: `planv0.md` line 117; Pan Docs §"PPU Modes" + §"Timing".

### 7.1 Why LCD and timing live in two files

Pan Docs treats the PPU and the master-clock timing as two separate subsystems even though they are entangled (the PPU's mode boundaries are clock-aligned). `gbf-hw` mirrors that split: `lcd.rs` owns the *qualitative* PPU state (modes, accessibility, register addresses) and `timing.rs` owns the *quantitative* frame budget. Two consumers want different things — `video_commit` (F-A5.6) needs the mode + register addresses; `gbf-codegen` Stage 11 `ScheduleCostAnalysis` needs the frame budget — and decoupling them lets each consumer import only what it uses.

### 7.2 `PpuMode` (lcd.rs)

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum PpuMode {
    HBlank    = 0,    // mode 0 — VRAM accessible, OAM accessible
    VBlank    = 1,    // mode 1 — VRAM accessible, OAM accessible
    OamSearch = 2,    // mode 2 — VRAM accessible, OAM NOT accessible
    Drawing   = 3,    // mode 3 — VRAM NOT accessible, OAM NOT accessible
}

impl PpuMode {
    pub const ALL: [PpuMode; 4] = [
        PpuMode::HBlank,
        PpuMode::VBlank,
        PpuMode::OamSearch,
        PpuMode::Drawing,
    ];

    /// Decode the low two bits of the STAT register into a PpuMode.
    /// Safe Rust cannot `as`-cast `u8` into a `#[repr(u8)]` enum directly;
    /// this is the canonical (and only) decode path consumers should use.
    pub const fn from_stat_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0 => PpuMode::HBlank,
            1 => PpuMode::VBlank,
            2 => PpuMode::OamSearch,
            _ => PpuMode::Drawing,
        }
    }
}
```

The discriminants match the values exposed in the `STAT` register's mode bits (`STAT[1:0]`). Runtime code must use `PpuMode::from_stat_bits(stat)` rather than attempting an integer-to-enum cast — `as` only converts enum-to-integer in safe Rust. `#[repr(u8)]` keeps the enum-to-integer direction stable for serde and for any code that wants to compare `mode as u8` against a known constant.

### 7.3 Accessibility predicates

```rust
/// VRAM accessibility *while the LCD is enabled and in the given mode*.
pub const fn vram_accessible_in(mode: PpuMode) -> bool {
    matches!(mode, PpuMode::HBlank | PpuMode::VBlank | PpuMode::OamSearch)
}

/// OAM accessibility *while the LCD is enabled and in the given mode*.
pub const fn oam_accessible_in(mode: PpuMode) -> bool {
    matches!(mode, PpuMode::HBlank | PpuMode::VBlank)
}

/// VRAM accessibility, accounting for LCD-disabled state.
/// Pan Docs §"Accessing VRAM and OAM" notes that when the LCD is disabled,
/// VRAM is always accessible regardless of PPU mode.
pub const fn vram_accessible(lcd_enabled: bool, mode: PpuMode) -> bool {
    !lcd_enabled || vram_accessible_in(mode)
}

/// OAM accessibility, accounting for LCD-disabled state.
/// Pan Docs: when the LCD is disabled, OAM is also always accessible.
pub const fn oam_accessible(lcd_enabled: bool, mode: PpuMode) -> bool {
    !lcd_enabled || oam_accessible_in(mode)
}
```

These are the *only* place in the workspace that encodes the PPU-mode/accessibility table. `video_commit` imports them; the test suite compares them against a transposed table embedded in the test source for cross-checking.

The `OamSearch` row is the subtle one: VRAM is accessible during mode 2 but OAM is *not* (the PPU is reading OAM to decide which sprites to fetch in the upcoming line). Consumers that conflate the two are buggy; the predicate split prevents the bug.

The `vram_accessible` / `oam_accessible` variants take an explicit `lcd_enabled: bool` because Pan Docs §"Accessing VRAM and OAM" is explicit that an LCD-disabled Game Boy permits unrestricted VRAM and OAM access regardless of PPU mode. F-A1's `tiny_rom` and any future bring-up demos rely on this: disabling the LCD before a bulk VRAM upload is the canonical way to bypass mode-3 contention. Consumers must call the LCD-disabled-aware variants when the LCD-on/off state is uncertain; the `_in` variants are still exported for callers (such as `video_commit`'s mid-frame fast path) that have already proven the LCD is on.

### 7.4 LCD register addresses

```rust
pub const LCDC_REG: u16 = 0xFF40;    // LCD control
pub const STAT_REG: u16 = 0xFF41;    // LCD status (mode, LYC=LY flag, IRQ source enables)
pub const SCY_REG:  u16 = 0xFF42;    // Background scroll Y
pub const SCX_REG:  u16 = 0xFF43;    // Background scroll X
pub const LY_REG:   u16 = 0xFF44;    // Current scanline (read-only)
pub const LYC_REG:  u16 = 0xFF45;    // LY compare
pub const DMA_REG:  u16 = 0xFF46;    // OAM DMA source high byte
pub const BGP_REG:  u16 = 0xFF47;    // BG palette
pub const OBP0_REG: u16 = 0xFF48;    // OBJ palette 0
pub const OBP1_REG: u16 = 0xFF49;    // OBJ palette 1
pub const WY_REG:   u16 = 0xFF4A;    // Window Y
pub const WX_REG:   u16 = 0xFF4B;    // Window X (offset by 7)

/// LY value at which VBlank begins. Pan Docs: scanlines 144..=153 are VBlank.
pub const VBLANK_LY_THRESHOLD: u8 = 144;
```

`WX` is offset by 7 in hardware (a true window X position of 7 is encoded as `WX = 14`); F-A2 ships only the register address, not the offset arithmetic. The arithmetic lives in F-A5.6's `video_commit` and is local to that module's window-drawing helpers.

### 7.5 Timing constants (timing.rs)

Pan Docs uses three distinct units that are easy to conflate:

- **Dot clock**: a 2^22 Hz time unit, fixed across CGB normal-speed and double-speed modes. One dot is one PPU pixel-fetch tick.
- **Normal-speed M-cycle**: 4 dots. The CPU runs at this rate on DMG/MGB and on CGB normal speed.
- **Double-speed M-cycle**: 2 dots. The CPU runs at this rate on CGB double speed (`KEY1` register selects).

CGB double speed does *not* double the dot clock; it changes the dots-per-M-cycle ratio from 4 to 2. The constants below name each unit explicitly to keep this distinction visible.

```rust
/// Dot clock frequency, in Hz. Pan Docs: one dot is a 2^22 Hz time unit;
/// the dot clock is fixed across both CGB normal and double speed modes.
pub const DOT_CLOCK_HZ:  u32 = 4_194_304;

/// Normal-speed M-cycles per second. Normal speed has 4 dots per M-cycle,
/// so 4194304 / 4 = 1048576 M-cycles/sec.
pub const NORMAL_M_CYCLES_PER_SECOND:  u32 = DOT_CLOCK_HZ / 4;

/// CGB double-speed M-cycles per second. Double speed has 2 dots per M-cycle,
/// so 4194304 / 2 = 2097152 M-cycles/sec. Provided for forward compatibility;
/// M0 targets DMG only, where this rate is unused.
pub const DOUBLE_SPEED_M_CYCLES_PER_SECOND: u32 = DOT_CLOCK_HZ / 2;

/// Total dots per frame (Pan Docs: 70224).
pub const FRAME_DOTS:       u32 = 70_224;

/// Total normal-speed M-cycles per frame. 70224 / 4 = 17556.
pub const FRAME_M_CYCLES:   u32 = FRAME_DOTS / 4;

/// VBlank dots (Pan Docs: 10 scanlines × 456 dots = 4560).
pub const VBLANK_DOTS:      u32 = 4_560;

/// VBlank M-cycles. 4560 / 4 = 1140.
pub const VBLANK_M_CYCLES:  u32 = VBLANK_DOTS / 4;

/// Approximate frames per second. NORMAL_M_CYCLES_PER_SECOND / FRAME_M_CYCLES.
/// Pan Docs: ~59.7275 Hz on real DMG hardware.
pub const FRAMES_PER_SECOND: f32 =
    NORMAL_M_CYCLES_PER_SECOND as f32 / FRAME_M_CYCLES as f32;
```

`FRAMES_PER_SECOND` is the only `f32` constant in the crate. It is computed at compile time from integer constants; the test suite asserts it falls in `[59.6, 59.8]` (a 0.1 Hz tolerance band that catches arithmetic mistakes without false-positive on the inherent quantization).

The previous draft of this RFC named these constants `MASTER_CLOCK_HZ` and `SYSTEM_CLOCK_HZ`; both names were misleading. `MASTER_CLOCK_HZ = 4_194_304` was actually the dot clock; `SYSTEM_CLOCK_HZ = MASTER_CLOCK_HZ / 4` was actually "normal-speed M-cycles per second", which is neither the dot clock nor the LR35902 instruction-fetch rate in any straightforward sense. The renamed constants leave no ambiguity about which unit is being measured.

### 7.6 `TimingProfile`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct TimingProfile {
    pub dot_clock_hz: u32,
    pub dots_per_m_cycle: u8,    // 4 for DMG/normal-speed CGB, 2 for CGB double-speed
    pub frame_dots: u32,
    pub vblank_dots: u32,
}

impl TimingProfile {
    /// Derived: M-cycles per frame at the profile's clock-rate setting.
    pub const fn frame_m_cycles(&self) -> u32 {
        self.frame_dots / self.dots_per_m_cycle as u32
    }

    /// Derived: M-cycles per VBlank window.
    pub const fn vblank_m_cycles(&self) -> u32 {
        self.vblank_dots / self.dots_per_m_cycle as u32
    }
}

pub const fn dmg_timing() -> TimingProfile {
    TimingProfile {
        dot_clock_hz: DOT_CLOCK_HZ,
        dots_per_m_cycle: 4,
        frame_dots: FRAME_DOTS,
        vblank_dots: VBLANK_DOTS,
    }
}
```

`TimingProfile` carries dots and the dots-per-M-cycle ratio rather than baking M-cycle counts directly. Consumers that need M-cycles call `frame_m_cycles()` / `vblank_m_cycles()`. This makes a future `cgb_double_speed_timing()` constructor a one-line change (`dots_per_m_cycle: 2`) instead of a re-derivation of every M-cycle constant. `dmg_timing()` is `const`; `TargetProfile`'s bring-up constructor uses it.

Future work introduces constructors such as `cgb_normal_speed_timing()` and `cgb_double_speed_timing()`. CGB double speed does not double the dot clock; it changes the dots-per-M-cycle ratio from 4 to 2. M0 ships only `dmg_timing()`.

### 7.7 Tests

```bash
cargo test -p gbf-hw -- lcd::vram_oam_accessibility_table       # matches Pan Docs §"PPU Modes" line by line
cargo test -p gbf-hw -- lcd::lcd_disabled_unrestricted           # vram_accessible(false, _) == true; oam_accessible(false, _) == true
cargo test -p gbf-hw -- lcd::from_stat_bits_round_trip           # PpuMode::from_stat_bits(mode as u8) == mode for every mode
cargo test -p gbf-hw -- lcd::ppu_mode_discriminants             # discriminants are 0/1/2/3 in declaration order
cargo test -p gbf-hw -- lcd::vblank_ly_threshold                # VBLANK_LY_THRESHOLD == 144
cargo test -p gbf-hw -- lcd::register_addresses_in_io_region    # every LCD register address is in IO_BASE..=IO_END
cargo test -p gbf-hw -- timing::frame_cycles                    # FRAME_M_CYCLES == 17556
cargo test -p gbf-hw -- timing::vblank_cycles                   # VBLANK_M_CYCLES == 1140
cargo test -p gbf-hw -- timing::fps_close_to_597                # FRAMES_PER_SECOND in [59.6, 59.8]
cargo test -p gbf-hw -- timing::dot_clock_constant_across_speeds # DOT_CLOCK_HZ identical for normal/double speed
cargo test -p gbf-hw -- timing::dmg_timing_round_trip           # serde round-trip
cargo test -p gbf-hw -- timing::vblank_smaller_than_frame       # VBLANK_M_CYCLES < FRAME_M_CYCLES
cargo test -p gbf-hw -- timing::profile_derives_m_cycles        # TimingProfile.frame_m_cycles() / vblank_m_cycles() match constants
```

### 7.8 Constitution checkpoints

- §I.1: `PpuMode` is exhaustive.
- §VI.1: All LCD/timing constants live here.

## 8. Interrupts (T-A2.5, `interrupts.rs`)

**Bead**: `bd-e33` (P1). **Reference**: Pan Docs §"Interrupts".

### 8.1 The five interrupt sources

LR35902 has exactly five interrupt sources, with vectors at fixed Bank-0 addresses. Pan Docs §"Interrupts" lists them in priority order; F-A2 mirrors that order in the enum's discriminant assignment.

| Source     | Vector  | IE/IF bit | Priority |
|------------|---------|-----------|----------|
| VBlank     | `$0040` | 0         | highest  |
| LCD STAT   | `$0048` | 1         |          |
| Timer      | `$0050` | 2         |          |
| Serial     | `$0058` | 3         |          |
| Joypad     | `$0060` | 4         | lowest   |

When multiple interrupts are pending and IME is set, the lowest-numbered bit fires first; the priority order is fixed in hardware.

### 8.2 `InterruptSource`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InterruptSource {
    VBlank   = 0,
    LcdStat  = 1,
    Timer    = 2,
    Serial   = 3,
    Joypad   = 4,
}

impl InterruptSource {
    /// All sources in priority order (highest first).
    pub const ALL: [InterruptSource; 5] = [
        InterruptSource::VBlank,
        InterruptSource::LcdStat,
        InterruptSource::Timer,
        InterruptSource::Serial,
        InterruptSource::Joypad,
    ];
}
```

`InterruptSource::ALL` is the canonical traversal. Any consumer that wants to iterate sources uses `ALL` rather than open-coding an array.

### 8.3 Vector and bit constants

```rust
pub const INT_VECTOR_VBLANK:    u16 = 0x0040;
pub const INT_VECTOR_LCD_STAT:  u16 = 0x0048;
pub const INT_VECTOR_TIMER:     u16 = 0x0050;
pub const INT_VECTOR_SERIAL:    u16 = 0x0058;
pub const INT_VECTOR_JOYPAD:    u16 = 0x0060;

/// Re-exported from `gbf-hw::memory::IE_REG` so consumers can import the
/// interrupt-enable register from either module without two declarations.
pub use crate::memory::IE_REG as IE_REGISTER;
pub const IF_REGISTER:  u16 = 0xFF0F;

pub const DIV_REGISTER:  u16 = 0xFF04;    // divider register (free-running, increments at 16384 Hz)
pub const TIMA_REGISTER: u16 = 0xFF05;    // timer counter (increments according to TAC clock-select; overflow reloads from TMA and requests Timer IRQ)
pub const TMA_REGISTER:  u16 = 0xFF06;    // timer modulo (reload value after TIMA overflow)
pub const TAC_REGISTER:  u16 = 0xFF07;    // timer control (enable + clock select)

/// TAC enable bit. When set, TIMA increments at the configured rate.
pub const TAC_ENABLE_BIT:    u8 = 0b0000_0100;

/// TAC clock-select mask (bits 0..1).
pub const TAC_CLOCK_SELECT_MASK: u8 = 0b0000_0011;
```

`DIV`/`TIMA`/`TMA`/`TAC` are colocated with the interrupt sources because the cooperative scheduler (F-A5.2) uses TAC's clock-select to arm the timer interrupt at frame-aligned intervals. `DIV` increments at a fixed 16384 Hz independent of TAC and is included for completeness (Pan Docs §"Timer and Divider Registers"). `TIMA` is *not* free-running; it increments according to TAC's clock-select bits and only when TAC's enable bit is set. On TIMA overflow, the hardware reloads TIMA from TMA and requests the Timer interrupt. Putting the timer registers in `interrupts.rs` rather than a separate `timer.rs` module is a deliberate scope choice: the timer's only role in M0 is to drive the timer interrupt for cooperative yielding. A future feature that uses `TIMA` for non-IRQ work would justify a dedicated module.

### 8.4 Helper functions

```rust
pub const fn vector_for(source: InterruptSource) -> u16 {
    match source {
        InterruptSource::VBlank  => INT_VECTOR_VBLANK,
        InterruptSource::LcdStat => INT_VECTOR_LCD_STAT,
        InterruptSource::Timer   => INT_VECTOR_TIMER,
        InterruptSource::Serial  => INT_VECTOR_SERIAL,
        InterruptSource::Joypad  => INT_VECTOR_JOYPAD,
    }
}

pub const fn ie_bit(source: InterruptSource) -> u8 {
    1u8 << (source as u8)
}

pub const fn if_bit(source: InterruptSource) -> u8 {
    1u8 << (source as u8)
}
```

`ie_bit` and `if_bit` are deliberately separate functions (despite having identical bodies) because IE and IF are distinct registers with subtly different semantics (IE is read-write enable; IF is set-by-hardware-clear-by-software). Naming them separately keeps `gbf-runtime`'s ISR scaffolding readable: `iflags |= if_bit(VBlank)` is clearer than `iflags |= bit_of(VBlank)`.

### 8.5 Tests

```bash
cargo test -p gbf-hw -- interrupts::vector_table                # vector_for matches Pan Docs values
cargo test -p gbf-hw -- interrupts::ie_if_bit_layout            # bits 0..=4 in declaration order
cargo test -p gbf-hw -- interrupts::priority_order              # ALL is in priority order; ALL[0] == VBlank
cargo test -p gbf-hw -- interrupts::timer_registers_in_io_region # DIV/TIMA/TMA/TAC are in IO_BASE..=IO_END
cargo test -p gbf-hw -- interrupts::ie_register_is_singleton    # IE_REGISTER == 0xFFFF == IE_REG (memory.rs)
cargo test -p gbf-hw -- interrupts::vectors_in_bank0             # every vector is in ROM_BANK0_BASE..=ROM_BANK0_END
cargo test -p gbf-hw -- interrupts::tac_enable_bit_is_bit_2     # TAC_ENABLE_BIT == 0b0000_0100
```

The `vectors_in_bank0` test is non-trivial: it cross-checks against `gbf-hw::memory::is_rom_bank0` to confirm the vectors really do land in the bank-0 region. This catches a class of typo bug where a vector accidentally became `0x4040` instead of `0x0040`.

### 8.6 Constitution checkpoints

- §I.1: `InterruptSource` is exhaustive over LR35902's interrupt sources.
- §VI.1: Vectors and IE/IF bits live only here.

## 9. Joypad (T-A2.6, `joypad.rs`)

**Bead**: `bd-21r` (P2). **Reference**: Pan Docs §"Joypad".

### 9.1 The JOYP register

The joypad sits at `$FF00` and is read through a select-line cycling protocol: the program writes a byte that selects either the four directions or the four buttons, then reads back the column. The hardware register is *active-low*: bit `i = 0` means "button is pressed". `gbf-hw` exposes the register address, the select bits, and the post-decode active-high view; the active-low decode lives in F-A5.3's joypad reader.

### 9.2 Constants

```rust
pub const JOYP_REGISTER: u16 = 0xFF00;

/// Write this bit pattern to JOYP to select the directions column.
/// Bit 4 = 0 (selects directions); bit 5 = 1 (deselects buttons).
pub const JOYP_SELECT_DIRECTIONS:  u8 = 0b0010_0000;

/// Write this bit pattern to JOYP to select the buttons column.
/// Bit 5 = 0 (selects buttons); bit 4 = 1 (deselects directions).
pub const JOYP_SELECT_BUTTONS:     u8 = 0b0001_0000;

// Buttons (when bit 5 is selected and reading from JOYP):
pub const JOYP_BIT_A:      u8 = 0b0000_0001;
pub const JOYP_BIT_B:      u8 = 0b0000_0010;
pub const JOYP_BIT_SELECT: u8 = 0b0000_0100;
pub const JOYP_BIT_START:  u8 = 0b0000_1000;

// Directions (when bit 4 is selected):
pub const JOYP_BIT_RIGHT: u8 = 0b0000_0001;
pub const JOYP_BIT_LEFT:  u8 = 0b0000_0010;
pub const JOYP_BIT_UP:    u8 = 0b0000_0100;
pub const JOYP_BIT_DOWN:  u8 = 0b0000_1000;
```

The button and direction bits *overlap* (A shares column 0 with Right; B shares column 1 with Left; etc.) because they are read through the same four-bit column. The select bits disambiguate which column is being read.

### 9.3 `Button` and `ButtonState`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Button {
    A      = 0,
    B      = 1,
    Select = 2,
    Start  = 3,
    Up     = 4,
    Down   = 5,
    Left   = 6,
    Right  = 7,
}

impl Button {
    pub const ALL: [Button; 8] = [
        Button::A, Button::B, Button::Select, Button::Start,
        Button::Up, Button::Down, Button::Left, Button::Right,
    ];
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
pub struct ButtonState {
    /// Bit `Button::ALL[i] as u8` is 1 iff the i-th button is pressed.
    /// This is the *post-decode active-high* view; the joypad reader inverts.
    /// Field is private; access through `bits()` and construction through
    /// `from_bits()` so the storage layout remains a single decision point.
    bits: u8,
}

impl ButtonState {
    /// Construct a ButtonState from its raw bit pattern. The joypad reader
    /// (F-A5.3) is the canonical caller; tests also use this constructor.
    pub const fn from_bits(bits: u8) -> Self {
        Self { bits }
    }

    /// Read the raw bit pattern. Consumers that need to serialize or compare
    /// state to a known mask should use this accessor; do not synthesize the
    /// layout from `JOYP_BIT_*` constants.
    pub const fn bits(self) -> u8 {
        self.bits
    }

    pub const fn is_pressed(&self, b: Button) -> bool {
        (self.bits & (1u8 << (b as u8))) != 0
    }

    /// True iff `b` is pressed in `cur` but not in `prev`.
    pub const fn just_pressed(prev: ButtonState, cur: ButtonState, b: Button) -> bool {
        cur.is_pressed(b) && !prev.is_pressed(b)
    }

    /// True iff `b` is released in `cur` but was pressed in `prev`.
    pub const fn just_released(prev: ButtonState, cur: ButtonState, b: Button) -> bool {
        prev.is_pressed(b) && !cur.is_pressed(b)
    }
}
```

`ButtonState` is a single `u8` because there are exactly eight buttons. The storage layout is stable for M0 but intentionally accessed through methods (`from_bits` / `bits` / `is_pressed`) rather than a public field. Consumers must not compare `ButtonState::bits()` directly to `JOYP_BIT_*` constants, because the JOYP bit constants are a *hardware register layout* (overlapping columns) and `ButtonState` is a *post-decode flat layout* (one bit per `Button` variant). The two are deliberately distinct; method-only access keeps that distinction enforced at the type level.

### 9.4 Tests

```bash
cargo test -p gbf-hw -- joypad::register_address              # JOYP_REGISTER == 0xFF00
cargo test -p gbf-hw -- joypad::select_bits                   # JOYP_SELECT_BUTTONS == 0x10; JOYP_SELECT_DIRECTIONS == 0x20
cargo test -p gbf-hw -- joypad::button_enum_exhaustive        # Button::ALL has 8 variants in declaration order
cargo test -p gbf-hw -- joypad::is_pressed_table_driven       # for each of the 256 ButtonState values, is_pressed returns the expected bit
cargo test -p gbf-hw -- joypad::just_pressed_edge             # (released, pressed) edge fires; (pressed, pressed) does not; (pressed, released) does not
cargo test -p gbf-hw -- joypad::just_released_edge            # mirror of just_pressed
cargo test -p gbf-hw -- joypad::default_state_is_no_buttons   # ButtonState::default().bits == 0
cargo test -p gbf-hw -- joypad::serde_round_trip              # ButtonState round-trips
```

### 9.5 Constitution checkpoints

- §I.1: `Button` is exhaustive over the eight Game Boy buttons.
- §VI.1: Joypad layout is centralized.

## 10. Calibration schema (T-A2.7, `calibration.rs`)

**Bead**: `bd-xkp` (P2). **Reference**: `planv0.md` line 221.

### 10.1 Why calibration is layered

A naïve calibration design would carry a flat `CalibrationBundle { bank_switch_cost, kernel_profiles, scheduler_overhead, ... }` and re-run all measurements whenever any input changed. That is wasteful: bank-switch cost is a property of the cartridge and the emulator; it does not change when a kernel implementation changes. Kernel cycle profiles depend on the kernel implementation and the runtime nucleus; they do not change when an unrelated platform fact is re-measured. Scheduler overhead depends on the runtime nucleus; it does not change when kernels change.

The three-layer split keeps each measurement valid as long as its inputs are unchanged:

1. **`PlatformCalibrationBundle`** — bank-switch cost, SRAM page cost, timer ISR cost. Invalidated only when the `MeasurementContext` changes (different emulator, different hardware, different gbllm version).
2. **`KernelCalibrationBundle`** — per-kernel cycle profiles. Invalidated when the kernel implementation hash or the runtime nucleus hash changes.
3. **`RuntimeCalibrationBundle`** — scheduler overhead, overlay install cost, trace event cost. Invalidated when the runtime nucleus hash changes.

`CompileRequest` carries a `CalibrationSetRef { platform: Id, kernel: Option<Id>, runtime: Option<Id> }` so a build can opt into the layers it has measurements for. A bring-up build that has no kernel measurements yet uses `kernel: None` and the compiler falls back to static cycle estimates from `gbf-asm::cycle_model`.

### 10.2 `CycleDistribution`

```rust
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CycleDistributionRepr")]
pub struct CycleDistribution {
    mean: f32,
    p50:  u32,
    p90:  u32,
    p99:  u32,
}

impl CycleDistribution {
    pub fn new(mean: f32, p50: u32, p90: u32, p99: u32) -> Result<Self, CalibrationError> {
        if !mean.is_finite() || mean < 0.0 {
            return Err(CalibrationError::InvalidCycleDistribution);
        }
        if !(p50 <= p90 && p90 <= p99) {
            return Err(CalibrationError::InvalidCycleDistribution);
        }
        Ok(Self { mean, p50, p90, p99 })
    }

    pub const fn mean(&self) -> f32 { self.mean }
    pub const fn p50(&self)  -> u32 { self.p50 }
    pub const fn p90(&self)  -> u32 { self.p90 }
    pub const fn p99(&self)  -> u32 { self.p99 }
}
```

Fields are private. `new` validates that:

- `mean` is finite (no NaN, no infinity) and non-negative.
- `p50 <= p90 <= p99` (the percentiles are monotone non-decreasing).

`mean` is `f32` because the mean of M-cycle counts is generally non-integer; `p50`/`p90`/`p99` are `u32` because percentiles of M-cycle counts are integers (the underlying measurements are integer cycle counts). The struct is `PartialEq` (not `Eq`) because `f32` is not `Eq`. Serde goes through `#[serde(try_from = "CycleDistributionRepr")]` so deserialization runs the same validation as construction.

Future work (Epic E) may extend with `stddev`, `min`, `max`, and a histogram. F-A2 ships only the four fields the M0 compiler consumes.

### 10.3 `CalibrationConfidence`

```rust
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CalibrationConfidenceClass {
    Strong,        // >= STRONG_CONFIDENCE_MIN_SAMPLES samples, bounded stddev
    Reasonable,    // >= REASONABLE_CONFIDENCE_MIN_SAMPLES samples
    Weak,          // fewer samples, or unbounded stddev
}

pub const STRONG_CONFIDENCE_MIN_SAMPLES:     u32 = 1_000;
pub const REASONABLE_CONFIDENCE_MIN_SAMPLES: u32 = 100;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CalibrationConfidenceRepr")]
pub struct CalibrationConfidence {
    class:        CalibrationConfidenceClass,
    sample_count: u32,
    stddev:       Option<f32>,
}

impl CalibrationConfidence {
    pub fn new(sample_count: u32, stddev: Option<f32>) -> Result<Self, CalibrationError> {
        if sample_count == 0 {
            return Err(CalibrationError::SampleCountIsZero);
        }
        if let Some(s) = stddev {
            if !s.is_finite() || s < 0.0 {
                return Err(CalibrationError::InvalidStddev);
            }
        }

        let class = if sample_count >= STRONG_CONFIDENCE_MIN_SAMPLES && stddev.is_some() {
            CalibrationConfidenceClass::Strong
        } else if sample_count >= REASONABLE_CONFIDENCE_MIN_SAMPLES {
            CalibrationConfidenceClass::Reasonable
        } else {
            CalibrationConfidenceClass::Weak
        };

        Ok(Self { class, sample_count, stddev })
    }

    pub const fn class(&self) -> CalibrationConfidenceClass { self.class }
    pub const fn sample_count(&self) -> u32 { self.sample_count }
    pub const fn stddev(&self) -> Option<f32> { self.stddev }
}
```

Fields are private. `new` rejects:

- `sample_count == 0` (you cannot have a confidence value over zero samples).
- `stddev = Some(NaN)` or `Some(infinity)` or `Some(negative)`.

The class is *derived* from `sample_count` and `stddev`, not chosen by the caller, so a malicious or buggy producer cannot label a one-sample measurement as `Strong`. The class boundaries (`STRONG_CONFIDENCE_MIN_SAMPLES = 1000`, `REASONABLE_CONFIDENCE_MIN_SAMPLES = 100`) are public constants; tightening them in a future RFC is a workspace-wide change, not a hidden tweak.

### 10.4 `ValidityEnvelope`

```rust
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidityEnvelope {
    pub valid_until_compiler_version:    SemVer,        // gbf_foundation::SemVer
    pub valid_until_runtime_nucleus_hash: Option<Hash256>,    // None for platform layer
}
```

The envelope says "this measurement is valid for compilers up to (and including) this version, and as long as the runtime nucleus hash is `Some(h)` (or always, if `None`)". Policy resolution rejects bundles whose envelope is older than the current build.

### 10.5 `MeasurementContext` and `UnixTimestampMillis`

```rust
/// Milliseconds since the Unix epoch, UTC.
///
/// `gbf-hw` deliberately does *not* depend on `chrono`. Formatting and parsing
/// belong in report or bench crates that are already linking date/time
/// libraries; the schema crate keeps a plain integer to stay no_std-friendly.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct UnixTimestampMillis(pub i64);

/// What the measurement was taken against. Disjoint by construction:
/// every `MeasurementContext` value either points at an emulator *or* at
/// hardware, never both, never neither. Replaces the original
/// `Option<EmulatorAdapter> + Option<HardwareIdentifier>` design, which had
/// to lean on a `try_new` runtime check + `MeasurementContextEmpty` /
/// `MeasurementContextOverdetermined` error variants. Per
/// `CONSTITUTION.md` §I.1 (correctness by construction), the type carries
/// the invariant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MeasurementTarget {
    Emulator(EmulatorAdapter),                              // gbf_foundation::EmulatorAdapter
    Hardware(HardwareIdentifier),                            // gbf_foundation::HardwareIdentifier
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasurementContext {
    pub target: MeasurementTarget,
    pub measured_at: UnixTimestampMillis,
    pub gbllm_version: SemVer,
}
```

`MeasurementContext` carries a `MeasurementTarget` enum, so the "exactly one of emulator/hardware" invariant is enforced by the type system (correctness by construction); no `try_new` and no serde `try_from` are needed for this disjointness check, and no `MeasurementContextEmpty` / `MeasurementContextOverdetermined` error variants exist.

`UnixTimestampMillis` is the only timestamp type in `gbf-hw`. The original draft used `chrono::DateTime<chrono::Utc>`, but `chrono` pulls in a non-trivial transitive surface (and historically depends on `std`'s time facilities by default), which conflicts with the `no_std + alloc` posture. The newtype is the smallest representation that survives serialization round-trips and stays interpretable by report/bench crates that have access to a real time library.

### 10.6 The three bundle types

`PlatformCalibrationBundle` carries both `target_profile` (cartridge/MBC sensitivity) and `target_family` (cohort grouping), because platform measurements vary between (for example) MBC5+8MiB+128KiB and MBC5+2MiB+32KiB cartridges in ways the `TargetFamilyId` alone does not capture. Kernel and runtime calibration are family-sensitive — they depend on the LR35902 family and the runtime nucleus, not on cartridge-specific facts — so they carry only `target_family`.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlatformCalibrationBundle {
    pub id: PlatformCalibrationId,
    pub target_profile: TargetProfileId,
    pub target_family: TargetFamilyId,
    pub measurement_context: MeasurementContext,
    pub bank_switch_cost: CycleDistribution,
    pub sram_page_cost: CycleDistribution,
    pub timer_isr_cost: CycleDistribution,
    pub confidence: CalibrationConfidence,
    pub valid_for: ValidityEnvelope,
    pub cohort: CalibrationCohortId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelCalibrationBundle {
    pub id: KernelCalibrationId,
    pub target_family: TargetFamilyId,
    pub kernel_impl_hash: Hash256,
    pub runtime_nucleus_hash: Hash256,
    pub kernel_profiles: Vec<MeasuredKernelProfile>,
    pub confidence: CalibrationConfidence,
    pub valid_for: ValidityEnvelope,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCalibrationBundle {
    pub id: RuntimeCalibrationId,
    pub target_family: TargetFamilyId,
    pub runtime_nucleus_hash: Hash256,
    pub scheduler_overhead: CycleDistribution,
    pub overlay_install_cost: CycleDistribution,
    pub trace_event_cost: CycleDistribution,
    pub confidence: CalibrationConfidence,
    pub valid_for: ValidityEnvelope,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredKernelProfile {
    pub kernel_spec_id: KernelSpecId,            // gbf_foundation::KernelSpecId
    pub tile_dims: TileDims,                     // tile shape used for the measurement
    pub cycles: CycleDistribution,
    pub bytes_in: u32,
    pub bytes_out: u32,
}
```

`Vec<MeasuredKernelProfile>` is the only `alloc`-backed field. The crate is structured so the `Vec` import comes from `alloc::vec::Vec` once `gbf-foundation` is `no_std`; today it transitively comes from `std::vec::Vec` via the foundation crate. Bundle constructors validate that `kernel_profiles` is non-empty for `KernelCalibrationBundle` and that all `CycleDistribution` fields validate (using `CycleDistribution::new`); a builder-style `try_new` returns `Result<Self, CalibrationError>`.

### 10.7 `CalibrationSetRef`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CalibrationSetRef {
    pub platform: Option<PlatformCalibrationId>,
    pub kernel:   Option<KernelCalibrationId>,
    pub runtime:  Option<RuntimeCalibrationId>,
}
```

All three layers are *optional at the schema level*. `gbf-policy` decides whether a given compile mode requires measured platform calibration or may fall back to static estimates (typically: `Bringup` permits `platform: None` and falls back to `gbf-asm::cycle_model`'s static M-cycle estimates; `Default` and stricter profiles require at least a `platform: Some`). Keeping the schema permissive avoids the bootstrapping problem the original draft created — under a mandatory `platform` field, no compile could succeed before the first `gbf-bench` run produced a `PlatformCalibrationBundle`, and `gbf-bench` itself depends on running compiled ROMs.

### 10.8 Errors

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CalibrationError {
    SampleCountIsZero,
    InvalidCycleDistribution,            // mean not finite/non-negative, or percentiles not monotone
    InvalidStddev,                        // stddev not finite/non-negative when present
    StaleEnvelope { current_version: SemVer, valid_until: SemVer },
    NucleusHashMismatch { expected: Hash256, found: Hash256 },
    EmptyKernelProfiles,                  // KernelCalibrationBundle requires at least one profile
}
```

Schema-local invariants (sample-count, distribution monotonicity, non-empty profiles) are enforced inside `gbf-hw` at construction and during `serde` deserialization. Measurement-context disjointness is enforced by the `MeasurementTarget` enum's type structure (no runtime check, no error variant). *Staleness* against the current build identity — `StaleEnvelope` and `NucleusHashMismatch` — is a policy-resolution concern owned by `gbf-policy`, because `gbf-hw` does not (and should not) know the current compiler version or the current runtime nucleus hash.

### 10.9 Tests

```bash
cargo test -p gbf-hw -- calibration::serde_round_trip                # all three bundles round-trip
cargo test -p gbf-hw -- calibration::confidence_class_ordering       # Strong > Reasonable > Weak
cargo test -p gbf-hw -- calibration::set_ref_all_layers_optional     # schema allows uncalibrated bring-up; policy may reject
cargo test -p gbf-hw -- calibration::confidence_rejects_zero_samples # SampleCountIsZero on construction
cargo test -p gbf-hw -- calibration::confidence_class_is_derived     # caller cannot label 1-sample data Strong; new() picks the class
cargo test -p gbf-hw -- calibration::cycle_distribution_rejects_nan_mean
cargo test -p gbf-hw -- calibration::cycle_distribution_rejects_negative_mean
cargo test -p gbf-hw -- calibration::cycle_distribution_rejects_non_monotone   # p50 > p90 -> InvalidCycleDistribution
cargo test -p gbf-hw -- calibration::stddev_rejects_nan_or_negative
cargo test -p gbf-hw -- calibration::kernel_bundle_rejects_empty_profiles
cargo test -p gbf-hw -- calibration::serde_runs_validation           # malicious JSON cannot bypass invariants
cargo test -p gbf-hw -- calibration::bundle_id_is_content_addressed  # equal contents -> equal id
```

### 10.10 Constitution checkpoints

- §I.1: `CalibrationConfidenceClass` is enumerated; `RiskPolicy::require_confidence_at_least` matches exhaustively.
- §VI.1: Calibration schema is single-sourced here.

## 11. Cross-cutting concerns

### 11.1 `no_std + alloc` (deferred — current state aligns with `gbf-foundation`)

The original draft of this RFC committed to declaring `#![no_std]` + `extern crate alloc;` at the crate root. After F-A1 shipped, the constraint chain became visible: `gbf-hw` depends on `gbf-foundation`, and `gbf-foundation` itself currently uses `std` (`gbf-foundation/src/hash.rs`, `semver.rs`, `ids.rs` all `use std::fmt; use std::str::FromStr;`). Declaring `#![no_std]` on `gbf-hw` while transitively depending on `std` would either fail to compile or produce a misleading "no_std" claim that does not actually hold.

**Decision for F-A2 closure**: F-A2 ships `#![forbid(unsafe_code)]` at the crate root (compiler-enforced, matches the engineering rule), uses `std::fmt` / `std::error::Error` to match the workspace style established by `gbf-foundation` and `gbf-asm`, and *does not* declare `#![no_std]`. The "no_std + alloc capable" engineering-rule property is deferred to a follow-up bead that switches `gbf-foundation` first; the F-A2 source is structured so that switch is mechanical (`std::fmt` → `core::fmt`, `std::str::FromStr` → `core::str::FromStr`, `std::error::Error` → `core::error::Error`). This is consistent with F-A1 RFC §11.3, which made the same deferral with the same reasoning. There are no `std::collections::HashMap` or `std::sync::*` imports in `gbf-hw`; the only `std`-leak is the formatting/error trait imports, which `core` will provide once the foundation crate is converted.

`cargo check -p gbf-hw` is part of the workspace's pre-commit hook (`cargo fmt --check`, `cargo clippy --workspace --all-features -- -D warnings`, `cargo test --workspace --all-features`). The `--no-default-features` smoke check is added once `gbf-foundation` is `no_std`.

### 11.2 `serde` policy

`gbf-hw` depends on `serde` via the workspace dependency (`workspace.dependencies.serde`); the workspace pin is `serde = { version = "=1.0.228", features = ["derive"] }`. That pin currently has `default-features = true`. F-A2 keeps the dependency declaration as `serde.workspace = true`. Once `gbf-foundation` is converted to `no_std` (see §11.1 deferral), the workspace serde pin is updated to `default-features = false` + `["derive", "alloc"]` in one workspace-wide commit; `gbf-hw` does not need a per-crate override. `serde_json` is *dev-only* — production binaries that link `gbf-hw` do not pull in JSON parsing. Every public type derives `Serialize + Deserialize`. Constructor-validated newtypes (`CalibrationConfidence`, `CycleDistribution`, `KernelCalibrationBundle`) use `#[serde(try_from = "...")]` so deserialization goes through the same validation path as construction; the `serde::serde_runs_validation` test produces malicious JSON and confirms the typed errors fire. `MeasurementContext` does *not* need `serde(try_from)` for the emulator/hardware disjointness invariant because that invariant is carried by the `MeasurementTarget` enum's type structure rather than by a runtime check.

The four cartridge-header enums (`MbcType`, `RomSize`, `RamSize`, `DestinationCode`) carry `#[serde(rename_all = "snake_case")]` to match F-A1's existing serialization shape; the F-A1 negative-deserialization test pattern (rejecting unknown variants like `"mbc1"` or `"kib16"`) is mirrored in F-A2's `cartridge_header::serde_unknown_variants_rejected`.

JSON is the canonical inspection format for *tests and reports*. The workspace standard is JSON for human-inspectable artifacts and `bincode` for stage-cache blobs (the latter is owned by `gbf-store`, not by `gbf-hw`). F-A2 does not adopt CBOR, MessagePack, or any other format.

### 11.3 Error style

Every error type:

- Is a `#[derive(Copy, Clone, Debug, Eq, PartialEq)]` (or `Clone, Debug, PartialEq` if it carries `f32`) enum.
- Has explicit `impl core::fmt::Display` and `impl core::error::Error` (not `thiserror`, not `anyhow`).
- Carries enough state in its variants to fully reproduce the failing input.

This matches F-A1's error style and `CONSTITUTION.md` §V.3.

### 11.4 Performance targets

`gbf-hw` is consumed at compile time and at policy-resolution time. It has no hot path. Targets:

- `classify(addr)` — single-cycle `match`; sub-nanosecond.
- `vector_for(source)` — single-cycle `match`; sub-nanosecond.
- `is_pressed(b)` — single bit-and; sub-nanosecond.
- Calibration bundle deserialization — should not appear in any inference loop or compile hot path; serde JSON is fine.

The crate itself adds ~600 lines to the workspace; total cargo build cost is negligible.

### 11.5 Versioning

`gbf-hw` is `0.1.0` for M0. The crate stabilizes at `1.0.0` only after F-A4 (BankLease) and F-A5 (Bank0 runtime) have validated the constants against real hardware via `gbf-bench`. Pre-`1.0.0`, breaking changes to constants or types are coordinated through the bead graph.

### 11.6 Documentation

Every public item carries a doc comment with at minimum:

- A short description.
- A Pan Docs section reference where applicable.
- A `# Examples` block if the item has non-trivial usage.
- A `# Provenance` block citing the `planv0.md` line that motivated the item.

Doc comments are linted via `rustdoc`'s `--deny broken-intra-doc-links` and `--deny missing-docs` (the latter is workspace-level).

## 12. Implementation order

Within F-A2, the open tasks have a real DAG:

```
T-A2.0 cartridge_header ─── independent (consumed by F-A1 closure follow-up)
T-A2.1 target          ─── needs T-A2.0 (CartridgeProfile uses CartridgeType/RomSize/RamSize)
T-A2.2 memory          ──┐ depends on T-A2.1 (uses TargetProfile shape implicitly)
T-A2.3 mbc5             ─┼── depends on T-A2.2 (MBC5 write bands live in cartridge ROM address space)
T-A2.4 lcd + timing    ──┘
T-A2.5 interrupts       ─── depends on T-A2.2 (vectors live in bank 0; IE lives in IO/IE-byte regions; re-exports memory::IE_REG)
T-A2.6 joypad           ─── depends on T-A2.2 (JOYP lives in IO region)
T-A2.7 calibration       ─── depends on T-A2.1 (bundles reference TargetProfileId / TargetFamilyId)
```

**Recommended order:**

1. **T-A2.0 cartridge_header.rs** (foundational, independent). Half a day. Unblocks F-A1 closure follow-up.
2. **T-A2.1 target.rs**. One day. Unblocks everything else.
3. **T-A2.2 memory.rs**. One day. The most constants; the core predicates `ReachabilityValidation` and `RomWindowPlan` need.
4. **T-A2.3 mbc5.rs** (parallel with T-A2.4). One day. Unblocks F-A4.
5. **T-A2.4 lcd.rs + timing.rs** (parallel with T-A2.3). One day. Unblocks F-A5.6.
6. **T-A2.5 interrupts.rs**. Half a day. Unblocks F-A5.1 and F-A5.2.
7. **T-A2.6 joypad.rs**. Half a day. Unblocks F-A5.3.
8. **T-A2.7 calibration.rs**. One day. Unblocks `gbf-policy::CompileRequest` plumbing.

**Total: ~6.5 days of focused work.** The critical path is cartridge_header → target → memory → mbc5/lcd-timing/interrupts (in parallel) → joypad/calibration. Half the work parallelizes.

**PR shape (closure):** F-A2 ships in a single PR that lands all nine modules together. The earlier draft proposed a two-PR split for review tractability; in practice the modules are too tightly cross-linked (interrupts re-exports `memory::IE_REG`; lcd+timing tests cross-check against `memory::is_io`; calibration carries `TargetProfileId`/`TargetFamilyId` from `target.rs`) for the split to reduce review effort. A single PR closes T-A2.0 through T-A2.7 and the parent feature bead `bd-3sk` in one step. **F-A1 has shipped** (PRs `ec10b45`, `53d1d82`, `7a5c687`); `gbf-asm/src/rom.rs` exists in `main` and carries the `// TODO(F-A2)` marker that this PR removes. Closing that marker (rewriting the inline cartridge-header declarations to `pub use gbf_hw::cartridge_header::*;`) is *part of* the F-A2 PR — the `gbf-asm/src/rom.rs` migration is included in this PR and is the load-bearing acceptance gate that F-A2's `cartridge_header` module is consumable. See §0.0.5 and §2.2.1.

## 13. Testing strategy summary

| Layer                              | Coverage                                                                                                    |
|------------------------------------|--------------------------------------------------------------------------------------------------------------|
| Type-level (compile-time)          | `MemoryRegion`, `MbcRegisterClass`, `PpuMode`, `InterruptSource`, `Button`, `MbcType`, `ConsoleModel`, `CalibrationConfidenceClass` are exhaustive enums. `#[repr(u8)]` discriminants are stable. `const fn` predicates compile-check totality.|
| Unit / property                    | Per-module tests against Pan Docs (≥30 spot-checks across the crate). `region_classification` deterministic full sweep over every `u16` (no proptest, no randomness). `is_pressed_table_driven` deterministic full sweep over the 256 `ButtonState` values × 8 buttons. Bank-number assembly `rom_bank_number` proptest over `(u8, u8)`. |
| Integration                        | Cross-module conformance: vectors are in bank 0; LCD registers are in IO; DIV/TIMA/TMA/TAC are in IO; JOYP is in IO; IE is at `$FFFF` (and `interrupts::IE_REGISTER` is the same constant as `memory::IE_REG`). `bring_up_profile_smoke` (constructs `dmg_mbc5_8mib_128kib()`, walks every field, runs every predicate at the field's value). |
| Snapshot                           | `target::dmg_mbc5_constructor` JSON-equality snapshot. Calibration bundle JSON-shape snapshot for downstream report consumers. |
| Negative                           | `CalibrationConfidence::new(0)` → `SampleCountIsZero`. (Measurement-target disjointness is enforced at the type level via the `MeasurementTarget` enum, not via runtime errors.) Stale envelope → `StaleEnvelope`. `TargetProfileError` constructor variants (RTC-without-MBC3, MBC2 with SRAM, etc.). |
| Skill checklist                    | Constructor-validated newtypes have negative deserialization tests. Effect-classifier-style traversals (here: `classify` for `MemoryRegion`, `classify_mbc_write_address` for MBC5) have totality/partiality tests. JSON-facing schemas have round-trip tests. |
| Workspace-wide grep (deferred)     | `grep_no_redundant_constants` ships as `#[ignore]` smoke test in F-A2; promotion to a full workspace gate lands once `gbf-test` exists. |

All tests run as part of the workspace pre-commit hook (`cargo test --workspace --all-features`).

## 14. Resolved questions

These were the questions I planned to surface in PR review. Each is now resolved; the decisions below are load-bearing for closure.

1. **Memory map predicates are `const fn`, not free functions.** This makes them usable in `const` contexts (e.g., a `const ASSERT_BANK0: () = assert!(is_rom_bank0(SOME_ADDR));` at crate root would catch a constant typo at compile time). Free `fn` would force every consumer to evaluate at runtime. The cost is a strict subset of `const fn` features in the body — but `match` over `u16`, integer literals, and `matches!` are all const-stable.
2. **`MemoryRegion::Unmapped` is named, not absent.** A previous draft considered modeling `$FEA0..=$FEFF` as "no region" via `Option<MemoryRegion>`. That makes `classify` partial, which forces every consumer to handle the `None` case. Naming the region `Unmapped` keeps `classify` total and pushes the "what to do" decision to the consumer (typically: refuse to emit code that references it; `is_isr_resident_legal_dmg` returns `false`). Same reasoning applies to `EchoRam`.
3. **`ButtonState` is method-only.** The original draft exposed `bits` as a public field, which made the storage layout an immutable public ABI. The revised version makes the field private and exposes `from_bits` / `bits` accessors. Consumers should use `is_pressed(button)` rather than masking against `JOYP_BIT_*`; the internal layout (one bit per `Button` variant in declaration order) is documented but not a public ABI.
4. **`vram_accessible_in` and `oam_accessible_in` are separate functions.** A combined `accessibility(mode) -> Accessibility` value object would be more "type-y" but adds an enum layer for no benefit; the two predicates are the actual queries `video_commit` makes.
5. **Calibration schema lives in `gbf-hw`, not `gbf-bench`.** Justified in §3.5. The alternative (schema in `gbf-bench`) would force `gbf-policy` and `gbf-codegen` to depend on the emulator; that's the wrong dependency graph.
6. **`MeasuredKernelProfile.tile_dims` is in calibration, not `gbf-kernel`.** The tile dims used for a measurement are an attribute of the measurement, not of the kernel spec. Two measurements of the same kernel with different tile dims have different `MeasuredKernelProfile` values; the kernel spec id is unchanged. Putting `tile_dims` in `gbf-kernel`'s `KernelSpec` would make the spec change every time tile sweep parameters changed; that's wrong.
7. **`TimingProfile` is owned by `TargetProfile`, not by a separate registry.** Pulling timing into a separate `TimingProfileRegistry` would mean a build needs two lookups (TargetProfileId → TargetProfile → TimingProfileId → TimingProfile). The compression isn't worth the layer; the timing struct is 12 bytes.
8. **`InterruptSource::ALL` is in priority order (highest first), not declaration order.** They happen to coincide for LR35902 (priority order *is* the discriminant order), but the constant's name explicitly states the property so future changes (CGB-specific interrupt extensions) cannot break the order silently.
9. **`MBC5_RAM_ENABLE_VALUE = 0x0A` is the only enable; no loose interpretation.** Pan Docs warns against the loose interpretation. F-A2 deliberately does not provide a helper that accepts other low-nibble-A values. Documented in §6.2 and tested by absence in §6.7.
10. **`MbcRegisterClass::Reserved`, not `Unused`.** F-A1 already adopted this naming for `gbf-asm::effect`; F-A2 mirrors it.
11. **All `CalibrationSetRef` layers are `Option`.** The original draft made `platform` mandatory, which made bring-up impossible without a pre-existing `PlatformCalibrationBundle`, which in turn required a working compiler — a circular dependency. The revised schema makes every layer optional; `gbf-policy` decides which layers a given compile profile requires. `Bringup` profiles may compile with `platform: None` and fall back to `gbf-asm::cycle_model`'s static estimates; `Default` and stricter profiles require at least `platform: Some`.
12. **Calibration bundle target fields are split between profile and family.** `PlatformCalibrationBundle` carries both `target_profile: TargetProfileId` and `target_family: TargetFamilyId` because platform measurements are cartridge-sensitive. `KernelCalibrationBundle` and `RuntimeCalibrationBundle` carry only `target_family` because kernel/runtime timings are family-sensitive but not cartridge-sensitive.
13. **`is_isr_legal` is split into residency and I/O-permission predicates.** Placement of ISR code/data is a different question from "may an ISR access this I/O register"; combining them produced an ambiguous predicate that included `IE` (correct as a residency target) but conceptually conflated it with the I/O-permission rule. The split (`is_isr_resident_legal_dmg`, `is_isr_resident_legal_cgb`, `is_isr_io_register_allowed`) makes both questions answerable independently.
14. **`UnixTimestampMillis`, not `chrono::DateTime<Utc>`.** Keeping `gbf-hw` no_std-friendly means dropping `chrono`. Reports and bench crates that need formatted timestamps already link a real time library and can convert on demand; the schema crate keeps a plain `i64` newtype.
15. **Cartridge header constants live in `cartridge_header.rs`, not `target.rs` or `rom.rs`.** F-A1's ROM builder needs `NINTENDO_LOGO`, `MbcType`, `RomSize`, `RamSize`, and `DestinationCode` as typed constants; making them part of `target.rs` would conflate "what this build is" with "what bytes the cartridge header literally contains." A dedicated module keeps the surface area focused. F-A2 adopts the names F-A1 already shipped (e.g., `MbcType` not `CartridgeType`, `Kib32`/`Mib8` not `Rom32KiB`/`Rom8MiB`); see §0.0.5.
16. **Timing constants are renamed.** `MASTER_CLOCK_HZ` was actually the dot clock, and `SYSTEM_CLOCK_HZ` was actually "normal-speed M-cycles per second" — neither name accurately described the unit. The new names (`DOT_CLOCK_HZ`, `NORMAL_M_CYCLES_PER_SECOND`, `DOUBLE_SPEED_M_CYCLES_PER_SECOND`) leave no ambiguity. CGB double speed changes the dots-per-M-cycle ratio (4 → 2), not the dot clock.
17. **`PpuMode::from_stat_bits`, not `as`-cast.** Safe Rust cannot `as`-cast `u8` into a `#[repr(u8)]` enum directly; the constructor function is the canonical decode path.
18. **`#![forbid(unsafe_code)]` is the primary unsafe gate.** Compiler-enforced, not grep-based. The grep is a redundant smoke test, not the primary gate.
19. **WRAM is split into `Wram0` and `WramX`.** `MemoryRegion::Wram` lumping `$C000..=$DFFF` was wrong on CGB, where `$D000..=$DFFF` is switchable. The split is correct on every console and lets `is_fixed_wram_cgb` differ from `is_fixed_wram_dmg` without rewriting the predicate set.
20. **`MeasurementContext` carries a `MeasurementTarget` enum, not two `Option` fields.** The original draft used `emulator: Option<EmulatorAdapter>` + `hardware: Option<HardwareIdentifier>` and enforced "exactly one is `Some`" via a `try_new` runtime check plus `MeasurementContextEmpty` and `MeasurementContextOverdetermined` error variants. Per `CONSTITUTION.md` §I.1 (correctness by construction), the disjointness is now carried by the type itself: `enum MeasurementTarget { Emulator(EmulatorAdapter), Hardware(HardwareIdentifier) }`. The two error variants are deleted; the runtime test for disjointness is deleted; the type system enforces the invariant.
21. **`TargetProfileError::RomSizeExceedsMbcCapacity` is reserved for a future `RomSize` extension and is untestable today.** Today's `RomSize` enum saturates at `Mib8`, which equals MBC5's 8 MiB capacity ceiling. The `RomSizeExceedsMbcCapacity` variant is therefore unreachable; F-A2 keeps the variant in the public error enum so a future `RomSize::Mib16` (or similar) can plug in without an error-shape change, but does not pretend to test it. The §16 claim-to-gate matrix and §4.7 test list both note this explicitly.

## 15. Risks

| Risk                                                                | Likelihood | Mitigation                                                                                                                                  |
|---------------------------------------------------------------------|------------|---------------------------------------------------------------------------------------------------------------------------------------------|
| A constant typo (off-by-one in a region boundary)                   | Low        | Per-constant tests with literal values; cross-module conformance (e.g., interrupt vectors are in bank 0). The deterministic full sweep over `classify(u16)` catches off-by-one. |
| `is_isr_resident_legal_*` / `is_isr_io_register_allowed` too tight  | Medium     | The split is *deliberately* tight to surface accidental I/O dependence and CGB residency assumptions. F-A4 (`bd-f5y`, T-A4.4) audits both predicates in real ISR code; widening is a follow-up bead. |
| Calibration schema turns out wrong shape for `gbf-bench`            | Low        | Schema is informed by `planv0.md` line 225 and the existing T-A2.7 task description. Future field additions are additive (new optional fields preserve backward compat). |
| `MbcRegisterClass::Reserved` is silently confused with `Unmapped`   | Low        | The two enums are distinct types (`MbcRegisterClass` vs `MemoryRegion`). The `Reserved` band is `$6000..=$7FFF` (in `RomBank0`); `Unmapped` is `$FEA0..=$FEFF`. Tests cross-check. |
| Workspace-wide grep test produces false positives                   | Medium     | Allowlist ships colocated with the test. False positives are reviewed at promotion time (when `gbf-test` lands), not during F-A2. |
| Pan Docs revises a value (e.g., a register address)                 | Very Low   | Pan Docs is stable for LR35902 hardware. The doc-comment citations make any drift visible.                                                  |
| `f32` in `CycleDistribution.mean` causes `Eq` headaches downstream  | Low        | Only `PartialEq` is derived; consumers compare by value-equality where it matters and structural equality elsewhere. `Hash256` is the cross-build identity. |
| Adding MBC1/MBC3 breaks F-A2 closure                                 | Very Low   | F-A2 ships only MBC5. `MbcType` enumerates other variants without typed register modules; adding a module is purely additive.                |

## 16. Claim-to-gate matrix (closure-style)

The closure skills (`.agents/skills/asm-bead-closure/SKILL.md`, but the patterns generalize) require this for non-trivial beads. Pre-emptive matrix for F-A2 closure:

| Claim                                                                       | Gating test / artifact                                                                                  |
|-----------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|
| `TargetProfile::dmg_mbc5_8mib_128kib()` is a well-formed bring-up profile   | `target::dmg_mbc5_constructor`                                                                          |
| `TargetProfile` JSON serialization round-trips                              | `target::serde_round_trip`                                                                              |
| `MbcType` and `ConsoleModel` are exhaustive                                 | `target::capability_set_exhaustive` + `target::mbc_capacity_validation` (both rely on exhaustive `match`) |
| `TargetProfileError` is enumerated                                          | `target::error_variants_are_typed`                                                                      |
| Memory regions partition the address space exactly                          | `memory::region_classification` (full sweep), `memory::predicate_totality`                              |
| `is_isr_resident_legal_dmg` matches `{Bank0, Wram0, WramX, Hram, InterruptEnable}` exactly   | `memory::isr_resident_legal_dmg`, `memory::echo_ram_is_prohibited`, `memory::unmapped_is_prohibited` |
| `is_isr_resident_legal_cgb` matches `{Bank0, Wram0, Hram, InterruptEnable}` exactly          | `memory::isr_resident_legal_cgb`, `memory::wram_split_dmg_vs_cgb`                                       |
| `is_isr_io_register_allowed` matches `{IF_REGISTER, IE_REGISTER}` exactly                    | `memory::isr_io_register_allowed`                                                                       |
| Region size constants match Pan Docs                                        | `memory::region_sizes`                                                                                  |
| Predicates do not overlap (except `is_isr_resident_legal_*` which are unions) | `memory::no_predicate_overlap`                                                                        |
| MBC5 write-band addresses classify correctly                                | `mbc5::write_address_classification`, `mbc5::address_outside_rom_returns_none`                          |
| `MBC5_RAM_ENABLE_VALUE == 0x0A` exactly                                     | `mbc5::ram_enable_value`                                                                                |
| 9-bit ROM bank-number assembly is bit-correct                               | `mbc5::bank_number_assembly`, `mbc5::bank_number_high_bit_only_uses_lsb`                                |
| `MbcRegisterClass::Reserved` is named explicitly (not `Unused`)             | `mbc5::reserved_band_is_named`                                                                          |
| No loose RAM-enable predicate is exported                                   | `mbc5::loose_ram_enable_not_provided`                                                                   |
| `PpuMode` discriminants match the STAT register's mode bits                 | `lcd::ppu_mode_discriminants`                                                                           |
| VRAM accessibility table matches Pan Docs                                   | `lcd::vram_oam_accessibility_table`                                                                     |
| LCD register addresses live in the IO region                                | `lcd::register_addresses_in_io_region`                                                                  |
| `VBLANK_LY_THRESHOLD == 144`                                                | `lcd::vblank_ly_threshold`                                                                              |
| `FRAME_M_CYCLES == 17556`                                                   | `timing::frame_cycles`                                                                                  |
| `VBLANK_M_CYCLES == 1140`                                                   | `timing::vblank_cycles`                                                                                 |
| FPS is in `[59.6, 59.8]`                                                    | `timing::fps_close_to_597`                                                                              |
| Interrupt vectors live in bank 0                                            | `interrupts::vectors_in_bank0`                                                                          |
| IE/IF bit layout matches `InterruptSource` discriminant order               | `interrupts::ie_if_bit_layout`                                                                          |
| `InterruptSource::ALL` is in priority order                                 | `interrupts::priority_order`                                                                            |
| DIV/TIMA/TMA/TAC live in the IO region                                      | `interrupts::timer_registers_in_io_region`                                                              |
| `IE_REGISTER == 0xFFFF`                                                     | `interrupts::ie_register_is_singleton`                                                                  |
| `JOYP_REGISTER == 0xFF00`                                                   | `joypad::register_address`                                                                              |
| `Button` enum has eight variants in canonical order                         | `joypad::button_enum_exhaustive`                                                                        |
| `is_pressed`/`just_pressed`/`just_released` are correct over the 256-state space | `joypad::is_pressed_table_driven`, `joypad::just_pressed_edge`, `joypad::just_released_edge`        |
| Calibration schema round-trips through JSON                                 | `calibration::serde_round_trip`                                                                         |
| `CalibrationConfidenceClass` ordering is `Strong > Reasonable > Weak`       | `calibration::confidence_class_ordering`                                                                |
| All `CalibrationSetRef` layers are optional at the schema level             | `calibration::set_ref_all_layers_optional`                                                              |
| `CalibrationConfidence::new(0)` is rejected                                 | `calibration::confidence_rejects_zero_samples`                                                          |
| Confidence class is derived from sample_count + stddev, not chosen          | `calibration::confidence_class_is_derived`                                                              |
| `CycleDistribution::new` rejects NaN/negative mean                          | `calibration::cycle_distribution_rejects_nan_mean`, `calibration::cycle_distribution_rejects_negative_mean` |
| `CycleDistribution::new` rejects non-monotone percentiles                    | `calibration::cycle_distribution_rejects_non_monotone`                                                  |
| `MeasurementContext` is disjoint by construction (emulator XOR hardware)    | The `MeasurementTarget` enum guarantees this at compile time; no runtime test is needed and no error variant exists for the empty/overdetermined case. |
| `KernelCalibrationBundle` rejects empty profile set                         | `calibration::kernel_bundle_rejects_empty_profiles`                                                     |
| Serde validation runs on deserialization                                    | `calibration::serde_runs_validation`                                                                    |
| Stale envelope checks live in `gbf-policy`, not `gbf-hw`                    | (out-of-scope guard — verified by absence of staleness checks in gbf-hw)                                |
| `gbf-hw` is `no_std + alloc`-shaped (deferred declaration)                  | The source uses no `std::collections::*` or `std::sync::*`; only `std::fmt` / `std::error::Error` (mirroring `gbf-foundation`). The `#![no_std]` declaration is a follow-up bead that lands when `gbf-foundation` switches first. (See §11.1.) |
| No other workspace crate redeclares `gbf-hw` constants                      | `single_source_smoke::grep_no_redundant_constants` (`#[ignore]` until `gbf-test` lands)                 |
| F-A1's `// TODO(F-A2)` marker in `gbf-asm/src/rom.rs:28` is removed by closure | The F-A2 PR rewrites `gbf-asm/src/rom.rs` to delete the local `MbcType`/`RomSize`/`RamSize`/`DestinationCode`/`NINTENDO_LOGO` declarations and adds `pub use gbf_hw::cartridge_header::{NINTENDO_LOGO, MbcType, RomSize, RamSize, DestinationCode};` instead. The Nintendo logo's first byte is `0xCE`, not `0x33`. F-A1's existing tests (`nintendo_logo_present`, `header_checksum_known_vector`, `ram_size_header_bytes`, `public_enums_reject_unknown_serde_values`, etc.) keep passing without modification because the symbols resolve identically through the re-export. |
| F-A1's layout-pass exclusive-end constants are not redeclared in gbf-hw     | `gbf-asm/src/layout.rs` retains `ROM0_END_EXCLUSIVE = 0x4000`, `ROMX_END_EXCLUSIVE = 0x8000`, `ROM_BANK_SIZE = 16 * 1024`. The grep test allowlist (§3.4) names `gbf-asm/src/layout.rs` explicitly. |
| F-A1's composite `CartridgeHeader` builder stays in `gbf-asm`                | `gbf-asm/src/rom.rs` retains `CartridgeHeader`, `header_checksum`, `global_checksum`, `assemble_rom`, `RomAssemblyError`. F-A2 PR diff in `gbf-asm/src/rom.rs` is limited to the local-decl deletion + `pub use` add. |
| F-A2 introduces no `unsafe`                                                 | `#![forbid(unsafe_code)]` at the crate root (compiler-enforced); the grep is a redundant smoke test     |

## 17. References

### Internal

- `history/planv0.md` — line 115 (DMG/MBC5 facts), line 117 (VBlank ~59.7 Hz, 1.1 ms), line 121 (ISR-residency rule), line 140 (workspace `gbf-hw` slot), line 200 (module list), line 214 (single source of truth claim), line 225 (calibration schema), line 2091 (MBC5 RAM-enable canonical `$0A`), line 2901 (M0 scope), line 2931 (`no_std + alloc` mandate).
- `history/glossary.md` — uses existing terms (`Bead`, `Feature`, `Task`, `Contract`, `Owner`); introduces no new RFC vocabulary.
- `CONSTITUTION.md` — §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §VI.1 (single source of truth).
- `.agents/skills/asm-bead-closure/SKILL.md` — closure-skill checklist; the type-boundary and JSON-facing schema rules apply transitively to F-A2.
- `bd-3sk` (F-A2 feature bead) and child tasks `bd-17x`, `bd-1yu`, `bd-121`, `bd-304`, `bd-e33`, `bd-21r`, `bd-xkp`.
- F-A1 RFC `history/rfcs/F-A1-gbf-asm.md` — uses `gbf-hw::memory` constants (Nintendo logo, MBC5 cartridge type) and is gated behind `// TODO(F-A2)` markers in `gbf-asm/src/rom.rs`.
- F-A4 task `bd-371` (BankLease/BankGuard) — depends on `gbf-hw::mbc5`.
- F-A5 tasks `bd-17b` (boot), `bd-1cv` (scheduler), `bd-fcm` (joypad reader), `bd-1d2` (video_commit) — depend on `gbf-hw::interrupts`, `gbf-hw::lcd`, `gbf-hw::joypad`.

### External

- Pan Docs root: <https://gbdev.io/pandocs/>
- Pan Docs §"Memory Map": <https://gbdev.io/pandocs/Memory_Map.html>
- Pan Docs §"MBC5": <https://gbdev.io/pandocs/MBC5.html>
- Pan Docs §"PPU Modes": <https://gbdev.io/pandocs/Rendering.html#ppu-modes>
- Pan Docs §"Timing": <https://gbdev.io/pandocs/Rendering.html#vblank-and-hblank-interrupts>
- Pan Docs §"Interrupts": <https://gbdev.io/pandocs/Interrupts.html>
- Pan Docs §"Joypad": <https://gbdev.io/pandocs/Joypad_Input.html>
- Pan Docs §"The Cartridge Header": <https://gbdev.io/pandocs/The_Cartridge_Header.html>
- gekkio CPU manual: <https://gekkio.fi/files/gb-docs/gbctr.pdf>

## 18. Appendix: file-by-file change set

| File                                  | Change           | Lines (est.) |
|---------------------------------------|------------------|--------------|
| `gbf-hw/src/cartridge_header.rs`      | New                | ~140         |
| `gbf-hw/src/target.rs`                | New (replace stub) | ~140         |
| `gbf-hw/src/memory.rs`                | New (replace stub) | ~150         |
| `gbf-hw/src/mbc5.rs`                  | New (replace stub) | ~80          |
| `gbf-hw/src/lcd.rs`                   | New (replace stub) | ~80          |
| `gbf-hw/src/timing.rs`                | New (replace stub) | ~50          |
| `gbf-hw/src/interrupts.rs`            | New (replace stub) | ~80          |
| `gbf-hw/src/joypad.rs`                | New (replace stub) | ~80          |
| `gbf-hw/src/calibration.rs`           | New (replace stub) | ~150         |
| `gbf-hw/src/lib.rs`                   | Add `#![no_std]`, `#![forbid(unsafe_code)]`, `extern crate alloc;`, doc comments, re-export `cartridge_header` | +25 |
| `gbf-hw/tests/cartridge_header.rs`    | New                | ~100         |
| `gbf-hw/tests/target.rs`              | New                | ~90          |
| `gbf-hw/tests/memory.rs`              | New                | ~150         |
| `gbf-hw/tests/mbc5.rs`                | New                | ~80          |
| `gbf-hw/tests/lcd.rs`                 | New                | ~60          |
| `gbf-hw/tests/timing.rs`              | New                | ~50          |
| `gbf-hw/tests/interrupts.rs`          | New                | ~80          |
| `gbf-hw/tests/joypad.rs`              | New                | ~80          |
| `gbf-hw/tests/calibration.rs`         | New                | ~150         |
| `gbf-hw/tests/cross_module_conformance.rs` | New           | ~120         |
| `gbf-hw/tests/single_source_smoke.rs` | New (#[ignore])    | ~80          |
| `gbf-hw/Cargo.toml`                   | Move `serde_json` to `[dev-dependencies]`; pin `serde` features. (`chrono` is not added; `proptest` may be added in a follow-up.) | +4 |
| `gbf-asm/src/rom.rs`                  | Delete local `MbcType` / `RomSize` / `RamSize` / `DestinationCode` / `NINTENDO_LOGO` declarations and the `// TODO(F-A2)` comment; add `pub use gbf_hw::cartridge_header::{NINTENDO_LOGO, MbcType, RomSize, RamSize, DestinationCode};`. The composite `CartridgeHeader`, checksum algorithms, and `assemble_rom` stay. | ~150 deletions, ~3 additions |
| `gbf-foundation/src/ids.rs`           | Add `string_id!(PlatformCalibrationId)`, `KernelCalibrationId`, `RuntimeCalibrationId`, `CalibrationCohortId`, `KernelImplId`, `RuntimeNucleusId`, `KernelSpecId`. (Existing `CalibrationSetRef` string-id is removed; the struct lives in `gbf-hw::calibration`.) | +12 |
| `gbf-foundation/src/lib.rs`           | Update re-exports for the new ID newtypes; drop the obsolete `CalibrationSetRef` re-export. | +5 |
| `scripts/lints/no-hw-literal-redeclarations.py` | New              | ~120         |

**Total: ~2200 LOC, ~58% of which is tests, fixtures, and table-driven Pan-Docs-derived tables.**

## 19. Review packet requirements

The F-A2 PR ships with a **review packet** as a first-class artifact in the repository, alongside the implementation. The packet is authored *after* implementation (so it can describe real decisions, real surprises, real measured costs, and real Pan-Docs cross-checks rather than the RFC's predictions). This RFC therefore specifies only what the packet must *cover*, not what its directory layout, file names, prose, or diagrams should look like in detail. Those are decided at packet-creation time.

### 19.1 What the packet must let the reviewer do

A reviewer who is otherwise unfamiliar with F-A2 should be able to answer four questions in one sitting:

1. **Is the implementation correct?** — Pan-Docs-anchored, exhaustive over the modeled domain, total over its inputs.
2. **Is it clear and maintainable?** — Single source of truth, no magic numbers leaked into other crates, names that match the design surface this RFC commits to.
3. **Are the riskiest invariants actually proved?** — By tests, by type structure, by Pan-Docs citations, or by a combination — not by prose alone.
4. **Can I reproduce every claimed output locally?** — Tests, generated artifacts, dependency reports, and any cross-crate workspace effects.

### 19.2 Required topics

The packet, however structured, must cover at least:

- **Scope statement** — what is in scope for this PR, what is intentionally deferred, and (when the deferred path has an owner) which downstream feature/bead picks it up.
- **Reading order** — a recommended sequence: which file or topic to read first, which to read deeply, which to skim, which to ignore.
- **Diff disposition** — for every file in the PR diff, a one-line classification (deep review / boundary review / skim / mechanical re-export / generated / fixture / config). The list must be exhaustive over `gh pr diff --name-only`.
- **Architecture brief** — how `gbf-hw` decomposes (the nine modules), why each module is where it is, and what the dependency direction is. Reuses material from §3 of this RFC.
- **Correctness dossier** — for each of the highest-risk surfaces (memory-map classification totality, ISR-residency split, MBC5 RAM-enable canonical value, PPU mode/accessibility table, interrupt priority order, calibration schema invariants, F-A1 cartridge-header re-exports), the packet records the invariant, the test or type or citation that proves it, and the failure mode if it ever drifts.
- **Pan Docs citation table** — every public constant in `gbf-hw` mapped to a Pan Docs section, with an HTML-fragment-ID check confirming the link still resolves. The check passes the link as a structural test, not just an HTTP 200 ping.
- **Claim-to-gate matrix** — every load-bearing claim from §16 (and any new claims that surfaced during implementation) mapped to its gating test or artifact. New rows are added if the implementation discovered new invariants.
- **Test coverage report** — what `cargo test -p gbf-hw` runs, how it groups, what it asserts, and any portions deliberately not covered (with reasoning).
- **Reproducibility report** — the exact command set a reviewer runs to regenerate every checked-in artifact (test output, dependency tree, grep audit, diagrams). One top-level script invocation should reproduce all of it.
- **Generated artifacts manifest** — what artifacts (test logs, cargo trees, grep outputs, diagrams, etc.) ship with the packet, and a reproducibility-fingerprint per artifact (so the reviewer can confirm "the file in the packet is what the script produces").
- **Dependency report** — `cargo tree -p gbf-hw` plus an explicit confirmation that no `[dev-dependencies]` on `gbf-bench`, `gbf-emu`, or training-side crates leak in.
- **Known-debt ledger** — every TODO, FIXME, deferred decision, or known-imperfect aspect introduced or carried by F-A2, with the bead/feature that owns the resolution. Includes the `no_std` switch deferral (§11.1) and the workspace-wide grep gate promotion.
- **Out-of-scope ledger** — items that look like F-A2's job but explicitly are not (calibration bundle production, ISR analysis, BankLease ABI, joypad reader, MBC1/2/3 register modules, etc.), each with the owning feature.
- **API guide** — the public surface of each module, the constructor-validated newtypes (with their invariants), the const-fn predicates, the serde shape commitments, and the breaking-vs-additive policy.
- **Reviewer checklist** — the binary questions the reviewer should be able to mark off (e.g., "every `u16` classifies into exactly one `MemoryRegion`", "MBC5 RAM-enable is exclusively `0x0A`", "F-A1's `// TODO(F-A2)` marker no longer appears in `gbf-asm/src/rom.rs`", etc.).
- **Cleanliness audit** — confirmation that `#![forbid(unsafe_code)]` is at the crate root, that no `std::collections::HashMap` / `std::sync::*` / `SystemTime` / `rand` / `chrono` imports were introduced, and that `serde_json` stays in `[dev-dependencies]`.
- **Source-to-artifact traceability** — for at least one representative constant (e.g., `ROM_SWITCHABLE_BASE`), a worked example showing the constant's Pan Docs source, its consumers in `gbf-asm`/`gbf-runtime`/`gbf-codegen`/`gbf-bench`, and the grep-audit allowlist entry (if any).
- **Diagrams** — at least the memory map, the MBC5 banding diagram, the PPU mode state machine, the interrupt priority order, and the calibration layering. Mermaid sources plus rendered SVGs; the rendering is reproducible.
- **F-A1 migration evidence** — the diff in `gbf-asm/src/rom.rs` (the local-decl deletion + `pub use` add), confirmation that F-A1's existing tests still pass, and an explicit note that the `// TODO(F-A2)` marker is gone.

The packet may add other sections that turn out to be useful at implementation time (e.g., a discussion of a cross-cutting decision that surfaced during coding), but it must not omit the topics above.

### 19.3 Reproducibility property

The packet contains a single top-level `verify-packet` script (or equivalent). Running it in a fresh checkout regenerates every artifact-the-packet-references and fails loudly if any checked-in artifact is stale relative to the current source. The exact script name, location, and output format are decided at packet-creation time; the contract is that one command suffices.

### 19.4 Acceptance bar

The packet is complete only when:

- a fresh-checkout reviewer can run the verify script, all tests pass, all reproducible artifacts match;
- every claim in §16 (claim-to-gate matrix) maps to a concrete gate (test, type, citation, or generated artifact);
- every file in the PR diff appears exactly once in the diff disposition table;
- every Pan Docs link resolves at the document-fragment level, not just HTTP 200;
- every `// TODO(F-A2)` marker that existed in `main` before this PR is either removed or re-tagged with a follow-up bead reference;
- the cleanliness audit shows zero introduced uses of disallowed APIs;
- known-debt and out-of-scope ledgers are present and entries point at owning beads or RFCs.

### 19.5 What the packet should explicitly not pre-commit to in *this* RFC

This RFC deliberately stops short of specifying packet directory structure, file names, README templates, exact diff-map columns, or the exact reviewer-question wording. The reasons:

- The packet describes the *implementation that actually shipped*. Pre-committing to a fixed structure here would force the implementation to match an a-priori shape that may not fit what was built.
- Implementation decisions (e.g., a redesign of a constructor's error variants, a reorganisation of test fixtures, an additive helper that didn't exist when this RFC was drafted) need to be reflected in the packet without first amending this RFC.
- The packet is itself a deliverable; locking its structure here makes the RFC into a packet template, which is the wrong level of abstraction.

The reviewer asks under §20 are part of *this* RFC's deliverable, not the packet's; they remain in this document.


## 20. End

This RFC stays inside the F-A2 boundary. Anything that requires F-A4's runtime ABI, F-A5's runtime nucleus, Epic B's `ReachabilityValidation`, Epic E's calibration bundle production, or `gbf-test`'s workspace-wide enforcement is explicitly deferred. The proposal lets F-A2 close without those features existing, while leaving every seam (`is_isr_resident_legal_*`, `is_isr_io_register_allowed`, `PpuMode`, MBC5 write-band addresses, `CalibrationSetRef`, `TargetProfile`, the three calibration bundle types, the cartridge-header enums) shaped for them to plug in cleanly.

Reviewer asks I would value most:

1. Is the **memory map predicate set** at the right granularity? Specifically, is the split between `is_isr_resident_legal_*` (placement) and `is_isr_io_register_allowed` (I/O permission) the right shape, and is the I/O allowlist's `{IF_REGISTER, IE_REGISTER}` set too tight?
2. Is the **`MeasurementTarget` shape** right? F-A2 commits to `enum MeasurementTarget { Emulator(EmulatorAdapter), Hardware(HardwareIdentifier) }`, which is disjoint by construction. The alternative — for "calibrating-against" provenance where an emulator is being checked against real hardware — would require a third variant (e.g., `EmulatorAgainstHardware { emulator, hardware }`) or a separate cross-link record outside `MeasurementContext`. F-A2's posture is that the cross-link belongs in a higher-level conformance report, not in the per-measurement context; cross-check welcome.
3. Is the **`PlatformCalibrationBundle.cohort`** field redundant with `target` + `MeasurementContext`? `cohort` exists to let two physically-distinct platforms (e.g., a known-good DMG batch and a known-bad DMG batch) share a `target` while remaining distinguishable; if that distinction is unneeded, `cohort` is dead weight.
4. Is the **MBC5 9-bit bank assembly direction correct** (BANK2 is the high *bit*, not the high *byte*)? Pan Docs uses both phrasings depending on the section. F-A2 commits to "high bit"; cross-check welcome.
5. Are the **TIMA/TMA/TAC registers** correctly placed in `interrupts.rs` rather than a dedicated `timer.rs`? The argument for colocation is that TIMA's only M0 role is driving the timer interrupt; the argument for separation is that TIMA is conceptually a clock peripheral, not an interrupt mechanism.

If those land cleanly, F-A2 closes and unblocks F-A4, F-A5, and the Epic B passes that depend on the memory map predicates.
