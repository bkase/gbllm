# RFC F-A1: `gbf-asm` — completing the typed LR35902 eDSL

| Field          | Value                                                              |
|----------------|--------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A1)                                   |
| Status         | Draft                                                              |
| Feature bead   | `bd-ssm`                                                           |
| Open tasks     | T-A1.5 (cycle), T-A1.6 (layout/relax), T-A1.7 (encoder), T-A1.8 (listing), T-A1.9 (ROM builder) |
| Closed tasks   | T-A1.1 (ISA), T-A1.2 (sections/symbols/provenance), T-A1.3 (builder), T-A1.4 (effect/privilege) |
| Plan reference | `history/planv0.md` §"Assembly eDSL" (line 2408), §"12. Backend" (line 1858), §"Engineering rules" (line 2893) |
| Glossary       | `history/glossary.md` (initial F-A1 terms, resolved terminology, and open decisions) |
| Constitution   | `CONSTITUTION.md` §I (correctness by construction), §III (shifting left), §IV (reproducibility) |

## 0. TL;DR

`gbf-asm` is the only legal authoring layer for executable Game Boy code in the entire system. T-A1.1 through T-A1.4 are already in tree: typed `Instr`, `SectionRole`, `MachineEffect`, `PrivilegeClass`, `Builder`, `SymbolName`, `InstrProvenance`. What is missing is everything from a populated `Section` to a deterministic `.gb` file plus its sibling `.lst` and `.sym` artifacts. Live emulator-driven boot validation is owned by the follow-up `gbf-emu`/`gbf-debug` feature (also targeting M0); F-A1 does not depend on it — see §1.2.

Shared terminology for this RFC lives in `history/glossary.md`. The F-A1
terminology decisions are resolved there: `Section`, `LoweredSection`, and
`LegalizedSection` use a Struct-of-Arrays (SoA) layout. Each kind of item
(instructions, labels, data blocks, alignments, structured ops, branches)
lives in its own typed `Vec<OrderedItem<T>>`. Stage transitions *physically
drop* the arrays they have lowered away: `LoweredSection` has no
`pre_layout_ops` field; `LegalizedSection` has neither `legalization_ops`
nor `branches` either. The encoder consumes a `LegalizedSection` and its
match over the remaining arrays is exhaustive at compile time — there is no
`EncodeError::OpNotLegalized`, because the un-legalized variants are not in
the type. The structured-op split uses phase-named `PreLayoutOp` and
`LegalizationOp`; structured-op privilege class is `Normal` (the privileged
work lives inside the runtime helper, identified by `SystemCallKind`, not by
a new privilege class); MBC5 `$6000..=$7FFF` is renamed to
`MbcRegisterClass::Reserved` and forbidden in every section, including
`Privileged`; and symbolic branches are durable items in the `branches`
array, not symbolic operands inside `Instr`.

This RFC proposes a five-piece pipeline that fills the remaining stubs (`cycle_model.rs`, `layout.rs`, `relax.rs`, `encoder.rs`, `listing.rs`) plus a new `rom.rs`. The pipeline runs in a strictly ordered, deterministic sequence:

```
Vec<Section>
   │
   ▼ PreLayoutOp lowering          (early, placement-independent)
   ▼ layout::layout_into_banks     (assigns sections to banks + start addresses)
   ▼ relax::relax_and_legalize     (iterative-monotone JR→JP / thunk decisions
   │                                plus placement-dependent `LegalizationOp`
   │                                lowering inside the same fixed-point loop)
   ▼ encoder::encode_section       (the only Instr→bytes path)
   ▼ rom::assemble_rom             (cartridge header + bank packing → .gb)
   ▼ listing::emit_listing         (per-section human-readable .lst)
   ▼ symbols::write_sym            (RGBDS-compatible .sym)
```

The new modules add roughly 900–1,000 LOC of production code plus about 2 KLOC of table-driven tests, fixtures, examples, and golden-output checks. The ROM builder produces a 32 KiB MBC5 cartridge with valid Pan-Docs-conformant headers and byte-stable output; live boot validation (loading the ROM in gameroy and asserting PC reaches `$0150`) is owned by the follow-up `gbf-emu`/`gbf-debug` feature. The F-A1 example demonstrates layout, relax, encode, ROM header construction, listing, and `.sym` generation without depending on the F-A5 text renderer.

The five most load-bearing decisions in this RFC are:

1. **Section IR is Struct-of-Arrays.** Each item kind (instructions, labels, data blocks, alignments, pre-layout ops, legalization ops, branches) lives in its own typed `Vec<OrderedItem<T>>` with a globally-monotone `seq_index` for ordering. Stage-transition types physically drop arrays: `LoweredSection` has no `pre_layout_ops`; `LegalizedSection` has neither `legalization_ops` nor `branches`. The encoder pattern-match over a `LegalizedSection` is exhaustive *at compile time*. "Encountered un-legalized op" is no longer a possible runtime error — it is structurally absent from the input type.
2. **Structured op lowering is split by phase.** `PreLayoutOp` covers placement-independent intents that lower before layout; `LegalizationOp` covers placement-dependent intents that lower during legalization after bank/address facts are known. The encoder sees neither (per decision 1, neither array exists in `LegalizedSection`).
3. **Layout uses optimistic-then-grow with a strictly monotone fixed point.** Each iteration can only widen instructions or insert thunks. Termination is mechanical.
4. **Far-call thunks live in Bank 0** as per-target-symbol trampolines (`runtime.banking.thunk.<target_symbol>`); cross-bank `CALL <symbol@bank_n>` is rewritten to `CALL <thunk-for-symbol>`, and the *thunk itself* bakes in the callee's bank number and 16-bit destination address. A 3-byte `CALL` carries only one 16-bit operand, so we cannot encode "switch bank N then jump to addr A" inside the call site — the thunk must own both. F-A4 provides the runtime ABI handshake; the asm layer only knows about the per-symbol thunk.
5. **The encoder is the *only* function that converts `Instr` to bytes.** Every other module (layout, relax, listing, ROM builder) reads bytes through the encoder. Bit-stability is asserted by a property test that encodes the same `Section` twice.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A bit-stable, deterministic encoder for every `Instr` variant defined by T-A1.1.
- A per-instruction M-cycle cost function with branch-taken/not-taken disambiguation, validated against Pan Docs for ≥30 instructions.
- A layout pass that assigns sections to banks under `PlacementProfile::{StrictOneExpertPerBank, Budgeted, PackedExperts}` and emits a `LayoutPlan` with concrete per-section bank + start address.
- An iterative branch-relaxation pass that converges to a fixed point in finitely many steps, producing in-range `JR` / in-bank `CALL` everywhere; out-of-range `JR` becomes `JP`; cross-bank `CALL` becomes a far-call thunk.
- Structured op lowering with clear `PreLayoutOp` and `LegalizationOp` seams, plus stub implementations so gbf-asm has a working in-tree end-to-end demo.
- A ROM builder that produces a valid MBC5 `.gb` file with correct cartridge header and checksums.
- A `.lst` listing emitter and `.sym` symbol-table emitter, both byte-stable across runs.
- An `examples/tiny_rom.rs` that builds a 32 KiB ROM with valid Pan-Docs header/checksums and a minimal Bank-0 routine. For F-A1, the routine performs a small VRAM-visible toggle rather than a full text renderer. F-A1's deliverable is the deterministic byte output (`tiny_rom.gb`, `tiny_rom.lst`, `tiny_rom.sym`); live boot validation under gameroy ships with the follow-up `gbf-emu`/`gbf-debug` feature. A `"hello"` renderer is deferred to F-A5.

### 1.2 Non-goals (deferred)

- **Whole-program `ReachabilityValidation`** — Epic B Stage 12 sub-pass. F-A1 lays the typing groundwork (`MachineEffect`, `PrivilegeClass` already shipped); the analysis itself ships in `gbf-codegen`.
- **The real `BankLease`/`BankGuard` runtime ABI** — F-A4 (`bd-1sv`). gbf-asm exposes the seam but does not implement the production version.
- **Bank-switch coalescing, hotness-driven placement, residency optimization** — Epic B Stage 12 (`PlacedRom`).
- **Stage cache integration** — F-B15 (`bd-1g7k`). gbf-asm functions are pure and content-addressable, so this is purely additive.
- **Branch hot-path versus cold-path policy.** We provide `Branch { taken, not_taken }` as a structure; consumers choose which one to use.
- **Cycle calibration drift reports** — F-E5 (`bd-3beu`). The cycle model here is the *static* prediction; bench compares it against measurement.
- **CGB / GBC features.** DMG/MBC5 only. The cartridge header CGB flag is hardcoded to 0.
- **Emulator-driven validation of `tiny_rom.gb`.** Booting the ROM in an emulator, asserting PC reaches `$0150`, and any other live-execution gate require the follow-up `gbf-emu`/`gbf-debug` feature to land. That feature owns the gameroy adapter (deterministic execution policy, breakpoint/watchpoint primitives) and the `gbf-debug` agent CLI (rquickjs-scripted, stateless session files) and is described in `history/planv0.md` (the `gbf-emu`/`gbf-debug` paragraphs and the "Emulation and the agent debugger" subsection). F-A1 produces the `.gb`/`.lst`/`.sym` artifacts and proves their structural invariants — header checksums, bank packing, byte-stable encoding, sorted/round-trippable `.sym` output. The live-boot tests move to the follow-up feature. Both features target M0, so the wait is bounded.

## 2. Background and existing state

### 2.1 What is already in tree (T-A1.1 — T-A1.4)

The following types are already implemented in `gbf-asm/src/`:

- **`isa.rs`** (~700 lines, bd-11e closed): `Instr`, `Reg8`, `Reg16`, `Reg16Pair`, `Reg16Data`, `Reg16Stack`, `Reg16Addr`, `AluSrc8`, `IncDec8Target`, `CbTarget`, `Operand8`, `Operand16`, `BitIndex`, `RstVector`, `DirectAddr`, `HighDirectOffset`, `Cond`. Every `Instr` variant has a `byte_len(self) -> u8` (constant for fixed-width variants, ALU-source-dependent for the eight-bit ALU family). 56 variants total.
- **`section.rs`** (~700 lines, bd-1e2 closed): `SectionRole`, `Section`, typed SoA item arrays (`labels`, `instrs`, `data_blocks`, `alignments`, `pre_layout_ops`, `legalization_ops`, `branches`), the borrowed `SectionItemView<'_>` traversal enum, `SectionPrivilege`, `PreLayoutOp` (`BankLease`, `BankRelease`, `Yield`, `TraceProbe`, `AssertBank`), `LegalizationOp` (`FarCall`), `BankLeaseSpec`, `LeaseId`, `MbcBankClass`, `YieldKind`, `TraceProbeId`, `ProbeLevel`. `Section::fixed_item_bytes()` returns `None` if any unknown-width array is non-empty. **There is no owned `SectionItem` storage enum and no `Raw` variant**: every byte that ends up in the ROM is materialized through `Instr`, `DataBlock::Bytes`, or `DataBlock::Words`. The cartridge header (Nintendo logo, title, MBC byte, header checksum) is emitted by the ROM builder as typed items, not as opaque bytes.
- **`provenance.rs`** (bd-1e2 closed): `InstrProvenance { stage, source_node, source_op, note }`, `PlanningStage` enum.
- **`symbols.rs`** (bd-1e2 closed): `SymbolName`, `SymbolSegment`, `SymbolId`, validated dot-separated canonical names. Constructors: `kernel`, `expert`, `runtime`, `section`.
- **`builder.rs`** (~750 lines, bd-3p1 closed): `Builder`, typed methods for every emit path, structured-op tracking, lease lifecycle, privilege validation at emit time.
- **`effect.rs`** (~1000 lines, bd-1bw closed): `MachineEffect` (33 variants), `MachineEffectKind` (parameter-free version for allowlists), `PrivilegeClass`, `classify_effect`, `classify_pre_layout_op`, `classify_legalization_op`, `privilege_of`. Static-address operations classified to concrete memory regions; dynamic-address ops keep an explicit `LoadFromDynamic { via }` / `StoreToDynamic { via }` so reachability later discharges the obligation.

### 2.2 What is stubbed

Five files exist as `//! Module stub.` placeholders and are referenced from `lib.rs`:

- `cycle_model.rs`
- `layout.rs`
- `relax.rs`
- `encoder.rs`
- `listing.rs`

A sixth file does not yet exist:

- `rom.rs` (T-A1.9, ROM builder)

### 2.3 Downstream pressure on this design

The completed five modules are consumed by:

- **F-A4** (`bd-1sv`, BankLease ABI) — needs `Builder`, `PreLayoutOpLowering` / `LegalizationOpLowering` seams, encoder for runtime banking primitives.
- **F-A5** (`bd-2r1`, Bank0 runtime) — authors the runtime nucleus through `Builder`. Needs everything in this RFC.
- **F-B13** (`bd-18d`, compiler backend) — produces `Vec<Section>` and consumes layout/relax/encoder. Needs the cycle model for `ScheduleCostAnalysis` (F-B12).
- **F-E5** (`bd-3beu`, cycle drift reports) — compares predicted cycle costs from `cycle_model` against measured runs.
- **F-H1/H2** (`bd-2f32`, `bd-3se9`, kernels) — written via `Builder`, lowered to bytes through this pipeline.

### 2.4 Engineering-rule grounding (planv0.md §"Engineering rules")

This RFC threads several rules tightly:

- **Rule 1**: All generated executable code originates from `AsmIR` / `Instr` / audited runtime builders, never from ad hoc byte pushes. → forces every consumer (F-A5 runtime, F-B13 backend, F-H2 kernels) to flow through `Builder`.
- **Rule 2**: Only the encoder translates legal instructions to bytes. → the encoder must be the unique sink. Layout, relax, listing, and ROM builder all read bytes by *calling* the encoder.
- **Rule 3**: Every instruction and data directive carries provenance. → already enforced in T-A1.2; this RFC carries provenance through to the listing.
- **Rule 4**: Every hard fit is proven in analysis/layout passes, not guessed in lowering code. → `LayoutError` is enumerated and any infeasibility is a typed error.
- **Rule 5**: ROM builds are deterministic and hashed. → bit-stability test is a primary gate.
- **Rule 7**: The harness uses symbols and `SemanticCheckpointId`s, not magic addresses. → `.sym` file is a first-class output.
- **Rule 10 (overridden, stronger)**: `planv0.md` Rule 10 read "`Raw(Vec<u8>)` remains an escape hatch, never the default." This RFC tightens that to **no escape hatch at all** — `SectionItem::Raw` and `MachineEffect::OpaqueBytes` are removed from `gbf-asm`. Every byte is authored through `Instr`, `Db`, or `Dw`. The cartridge header, the Nintendo logo, frozen micro-blobs, and test fixtures all use `Db`/`Dw`, which carry provenance and are visible to the effect classifier. The "audited escape hatch" lineage in `planv0.md` is closed: there is now nothing left to audit.
- **Rule 12**: `unsafe` is forbidden by default. → none of the new modules require unsafe.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction)** — `LayoutError`, `EncodeError`, `RelaxError`, `RomAssemblyError` are exhaustive enums; failures are typed, not strings.
- **§III (shift left)** — every invariant pushes to the cheapest layer. Cycle costs are per-variant constants asserted at compile time; address overflow is rejected at layout; encoding correctness is a property test.
- **§IV.3 (reproducible builds)** — bit-stability is an asserted invariant of the encoder, ROM builder, listing, and symbol writer.
- **§V.3 (silence on success, loud on failure)** — every error type carries enough state for a host-side debugger to reproduce.

### 2.6 T-A1.1 deep dive — ISA types (`isa.rs`, ~700 LOC, `bd-11e` closed)

#### 2.6.1 What shipped

`gbf-asm/src/isa.rs` defines the canonical `Instr` enum (56 variants) plus a tower of operand newtypes designed to make invalid operand combinations *unrepresentable per instruction family*.

```rust
pub enum Reg8 { A, B, C, D, E, H, L }                // F is intentionally absent
pub enum Reg16 { BC, DE, HL, SP, AF }                // general-purpose pairs + special

// Per-family register subsets. Each is a TryFrom<Reg16>.
pub enum Reg16Pair  { BC, DE, HL, AF }               // legal in PUSH/POP/etc but not 16-bit data ops on AF
pub enum Reg16Data  { BC, DE, HL, SP }               // r16 in `LD rr,imm16`/`INC rr`/`ADD HL,rr`
pub enum Reg16Stack { BC, DE, HL, AF }               // PUSH rr / POP rr
pub enum Reg16Addr  { BC, DE, Hli, Hld }             // (BC), (DE), (HL+), (HL-) for A transfers

pub enum Cond { NZ, Z, NC, C }
pub enum RstVector { V00, V08, V10, V18, V20, V28, V30, V38 }
```

#### 2.6.2 Constructor-validated newtypes

- **`BitIndex(u8)`** — three-bit value used by `BIT`/`RES`/`SET`. Constructor `new(u8) -> Option<Self>` rejects values ≥ 8. Constants `B0..=B7`. Serde via `#[serde(try_from = "u8")]`. Negative test: `serde_json::from_str::<BitIndex>("8")` → `Err`.
- **`DirectAddr(u16)`** — 16-bit absolute address for `LD A, (nn)` / `LD (nn), A`. Constructor rejects `addr ≥ 0xFF00`, forcing the LDH form for `$FF00..=$FFFF`. The encoder *cannot* emit the longer absolute form to a high-memory address through this type, because the type itself prohibits it. Serde via `try_from = "u16"`.
- **`HighDirectOffset(u8)`** — `$FF00 + n` operand. Constructor cannot fail.

#### 2.6.3 Per-family operand enums

```rust
pub enum AluSrc8 {                    // ADD A / ADC A / SUB A / SBC A / AND / OR / XOR / CP
    Reg(Reg8), HlIndirect, Imm(u8),
}
pub enum IncDec8Target { Reg(Reg8), HlIndirect }       // INC/DEC 8-bit targets
pub enum CbTarget      { Reg(Reg8), HlIndirect }       // CB-prefixed RMW/read targets
```

Each carries a `byte_len()` where applicable (e.g., `AluSrc8::Imm(_)` is 2 bytes, others 1).

#### 2.6.4 The `Instr` enum

56 variants, named by family + addressing form. Notable splits:

- `Ld8Reg`, `Ld8RegFromImm`, `Ld8RegFromHl`, `Ld8HlFromReg`, `Ld8HlFromImm` — five variants instead of one polymorphic `Ld8`. Each takes a narrower operand type.
- `LdAFromReg16Addr { src: Reg16Addr }` and `LdReg16AddrFromA { dst: Reg16Addr }` — A-register-only transfers through register-indirect.
- `LdAFromDirect { addr: DirectAddr }` and `LdDirectFromA { addr: DirectAddr }` — type-rejected from high memory.
- `LdAFromHighDirect`, `LdHighDirectFromA`, `LdAFromHighC`, `LdHighCFromA` — LDH forms.
- `Ld16Imm`, `LdSpFromHl`, `LdDirectFromSp { addr: u16 }` (raw `u16`, NOT `DirectAddr`, because `LD (nn), SP` writes 2 bytes that may cross a region boundary — see §2.9 `StoreToMixedStatic`), `LdHlFromSpPlus`.
- `Inc8/Dec8` take `IncDec8Target`; `Inc16/Dec16` take `Reg16Data`.
- All CB-prefixed RMW ops take `CbTarget`.

#### 2.6.5 `Instr::byte_len(self) -> u8`

Every variant has an exact, constant byte length. The method is `const`, exhaustive, single match. ALU ops dispatch through `AluSrc8::byte_len()`. The encoder MUST produce exactly `byte_len()` bytes per instruction — primary regression test for future variants (§8.5).

#### 2.6.6 Why these choices matter for T-A1.5–T-A1.9

| Decision                                       | Consequence for the open tasks                                                                                            |
|------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------|
| Per-family operand types                       | Encoder match arms can't accept invalid combinations (`PUSH SP` is non-representable); cycle model dispatches by family. |
| `DirectAddr` rejects `≥ 0xFF00`                | Encoder never has to choose between `LD A, (nn)` and `LDH A, (n)`; type already forced the choice at authoring.           |
| `byte_len()` is exact and constant per variant | Layout's pre-relax size estimate is exact for `Instr` items; only `Align`, `PreLayoutOp`, `LegalizationOp` are unknown-width. |
| Constructor-validated newtypes with serde gate | Round-trip tests are valid; deserializing untrusted JSON cannot smuggle illegal values.                                   |
| `RstVector` enum                               | Encoder maps directly to opcode bits; reachability later sees the vector parametrically.                                  |
| Stable serialization                           | Stage cache (F-B15) can persist sections.                                                                                  |

#### 2.6.7 Tests in tree

- `operand_classification` — every `Operand8`/`Operand16` mode value enumerable; `Operand8::ALL_MODES` matches variant tags.
- `instr_size_in_bytes` — table-driven, ~50 entries.
- `serde_rejects_invalid_validated_operands` — negative tests for `BitIndex`, `DirectAddr`, embedded operands in `Instr`.

#### 2.6.8 Open follow-ups (not blocking F-A1)

- **CB-prefix dispatch table** lives in `encoder.rs` (§8.7), not `isa.rs`.
- **`Display` for `Instr`** lives in `listing.rs` (§10.2), not `isa.rs`.
- **`Instr::Halt` remains canonical.** `Instr::Halt` represents exactly the LR35902 `HALT` instruction: one opcode byte (`0x76`) and one static M-cycle for the opcode fetch. HALT-bug neutralization is an authoring policy, not an ISA-table rewrite. `Builder::safe_halt()` emits two ordinary instructions, `HALT; NOP`, each with its own provenance entry unless the caller asks for shared provenance. This preserves the invariant that every `Instr` value denotes exactly one hardware instruction.

### 2.7 T-A1.2 deep dive — sections, symbols, provenance (`section.rs`/`provenance.rs`/`symbols.rs`, ~1500 LOC, `bd-1e2` closed)

#### 2.7.1 `SectionRole` (13 variants)

The shipped `SectionRole` taxonomy supersedes the older sketch in `planv0.md` (which had `RuntimeBank0`, `IsrReachable`, `CommonKernel`, etc.). The shipped version splits roles by *residency class*:

```rust
pub enum SectionRole {
    Bank0Nucleus,        // boot, ISRs, scheduler, UI, panic
    Bank0Data,           // non-executable ROM0 constants/tables
    CommonBank,          // co-resident shared kernels + tables
    CommonData,          // non-executable shared ROMX constants/tables
    ExpertBank,          // expert-local code/data
    ExpertData,          // non-executable expert-local ROMX constants/tables
    WramHotArena,        // ArenaPlan-managed WRAM
    WramOverlay,         // OverlayPlan-managed WRAM
    HramFastFlags,       // HRAM
    SramPersistent,      // SRAM (persistence)
    VramOwnedByUi,       // VRAM, owned by runtime UI
    OamOwnedByUi,        // OAM, owned by runtime UI
    HeaderCartridge,     // internal-only $0100..=$014F
}
```

`SectionRole::ALL` is a const array; `canonical_name()` returns `snake_case`; serde uses `rename_all = "snake_case"`. **Deviation from original task**: roles separate residency from ISR-reachability. The "ISR-reachable" property is derived later by `ReachabilityValidation`, not declared here. Intentional (planv0.md line 1877: "computes, rather than trusts").

`SectionRole::permits_inline_data() -> bool` partitions the ten roles into *executable* (no inline data) and *data-only* (inline data accepted):

| Executable (rejects `db`/`dw`) | Data-only (accepts `db`/`dw`)                                                        |
|--------------------------------|--------------------------------------------------------------------------------------|
| `Bank0Nucleus`                 | `HeaderCartridge`                                                                    |
| `CommonBank`                   | `Bank0Data`, `CommonData`, `ExpertData`, `WramHotArena`, `WramOverlay`, `HramFastFlags`, `SramPersistent` |
| `ExpertBank`                   | `VramOwnedByUi`, `OamOwnedByUi`                                                       |

**Why this gate exists**: an author in an executable section could otherwise hand-encode a privileged instruction byte sequence (e.g. `LD ($2000), A` is `0xEA 0x00 0x20`) as `db [0xEA, 0x00, 0x20]` and slip past the effect classifier. That is the same opaque-bytes escape hatch we removed when `SectionItem::Raw` and `MachineEffect::OpaqueBytes` were deleted; allowing arbitrary `db`/`dw` in executable sections would re-open it. The closure-skill rule reads: "Raw bytes are opaque privileged effects unless a bead explicitly narrows the claim to data-only sections." This RFC narrows it by `SectionRole::permits_inline_data()`. Inline data tables that need to live in ROM (jump tables, font glyphs, lookup tables) are emitted into their own non-executable sections — `HeaderCartridge` for the cartridge header itself, or a future data-only ROM role if compile-time tables grow common (out of scope for F-A1).

#### 2.7.2 Item types and the SoA layout

There is no monolithic owned `SectionItem` enum in the IR. Items live in typed arrays on `Section`, each a `Vec<OrderedItem<T>>` for a different `T`:

```rust
pub struct OrderedItem<T> {
    pub data: T,
    pub order: ItemOrder,         // global order within the Section
    pub provenance: InstrProvenance,
}

/// Stable lexicographic item order.
///
/// Existing author-emitted items use `sub_index = 0`. If lowering replaces
/// one author item with multiple concrete items, those concrete items reuse
/// the original `seq_index` and receive increasing `sub_index` values. This
/// prevents lowered fragments from being appended at the end of the section.
pub struct ItemOrder {
    pub seq_index: u32,
    pub sub_index: u16,
}

pub struct Label      { pub id: SymbolId, pub name: SymbolName }
pub enum   DataBlock  { Bytes(Vec<u8>), Words(Vec<u16>) }
pub struct Align(pub NonZeroU16);

pub enum SymbolicBranch {
    Jr {
        cond: Option<Cond>,
        target: SymbolName,
    },
    Jp {
        cond: Option<Cond>,
        target: SymbolName,
    },
    Call {
        cond: Option<Cond>,
        target: SymbolName,
        reachability: CallReachability,
    },
}

pub enum CallReachability {
    /// Ordinary symbolic call. Legal only when the final target is reachable
    /// without changing the selected switchable ROM bank.
    NearOnly,
    /// Explicit author request for the runtime far-call ABI. The relax pass
    /// may direct-call when the target is already reachable, or rewrite to a
    /// per-target thunk when it is not.
    AutoFar { lease_chain: Vec<LeaseId> },
}
// PreLayoutOp, LegalizationOp, Instr exist already.
```

Every emitted item carries `InstrProvenance` (engineering rule 3) via its `OrderedItem` wrapper. There is no `Raw` escape-hatch path: bytes flow through `DataBlock::Bytes` / `DataBlock::Words` only, both visible to the effect classifier. The cartridge header (logo, title, MBC byte, header checksum) is emitted as `DataBlock` items by the ROM builder. `MachineEffect::OpaqueBytes` is removed from `effect.rs` for the same reason.

A borrowed view enum exists for code that genuinely needs ordered traversal (listing, debug dumps, tests). This enum is not stored in the IR:

```rust
pub enum SectionItemView<'a> {
    Label(&'a OrderedItem<Label>),
    Instr(&'a OrderedItem<Instr>),
    DataBlock(&'a OrderedItem<DataBlock>),
    Align(&'a OrderedItem<Align>),
    PreLayoutOp(&'a OrderedItem<PreLayoutOp>),
    LegalizationOp(&'a OrderedItem<LegalizationOp>),
    Branch(&'a OrderedItem<SymbolicBranch>),
}

impl Section {
    pub fn iter_items(&self) -> Vec<SectionItemView<'_>>; // sorted by seq_index
}
```

Per-array fixed byte length:

| Array                | Fixed byte length per item              |
|----------------------|-----------------------------------------|
| `labels`             | `0`                                     |
| `instrs`             | `instr.byte_len() as u32`               |
| `data_blocks`        | `Bytes: b.len()`, `Words: w.len() * 2`  |
| `alignments`         | unknown until layout                    |
| `pre_layout_ops`     | unknown until lowering                  |
| `legalization_ops`   | unknown until legalization              |
| `branches`           | unknown until relaxation                |

`Section::fixed_item_bytes() -> Option<u32>` returns `Some(N)` only when the four "unknown" arrays are all empty. The check is cheap: four `is_empty()` comparisons, then sum two arrays. No iteration over a wrapper enum.

Effect-classified arrays (those that contribute to privilege validation): `instrs`, `pre_layout_ops`, `legalization_ops`, `branches`. The other three are pure data — they never produce a `MachineEffect`.

#### 2.7.3 `Section`, `LoweredSection`, and `LegalizedSection`

```rust
pub struct Section {
    id: SectionId,
    role: SectionRole,
    name: SymbolName,
    privilege: SectionPrivilege,
    align: NonZeroU16,
    size_hint_bytes: Option<u32>,
    next_seq_index: u32,
    labels:           Vec<OrderedItem<Label>>,
    instrs:           Vec<OrderedItem<Instr>>,
    data_blocks:      Vec<OrderedItem<DataBlock>>,
    alignments:       Vec<OrderedItem<Align>>,
    pre_layout_ops:   Vec<OrderedItem<PreLayoutOp>>,
    legalization_ops: Vec<OrderedItem<LegalizationOp>>,
    branches:         Vec<OrderedItem<SymbolicBranch>>,
}

pub struct LoweredSection {
    // ... metadata identical to Section ...
    pub privilege: SectionPrivilege,
    pub labels:           Vec<OrderedItem<Label>>,
    pub instrs:           Vec<OrderedItem<Instr>>,
    pub data_blocks:      Vec<OrderedItem<DataBlock>>,
    pub alignments:       Vec<OrderedItem<Align>>,
    // pre_layout_ops is PHYSICALLY ABSENT — every PreLayoutOp has been
    // lowered into Instrs / DataBlocks / LegalizationOps.
    pub legalization_ops: Vec<OrderedItem<LegalizationOp>>,
    pub branches:         Vec<OrderedItem<SymbolicBranch>>,
}

pub struct LegalizedSection {
    // ... metadata identical to Section ...
    pub privilege: SectionPrivilege,
    pub labels:      Vec<OrderedItem<Label>>,
    pub instrs:      Vec<OrderedItem<Instr>>,
    pub data_blocks: Vec<OrderedItem<DataBlock>>,
    pub alignments:  Vec<OrderedItem<Align>>,
    // pre_layout_ops, legalization_ops, branches: PHYSICALLY ABSENT.
    // The encoder takes &LegalizedSection and pattern-matches over only
    // the remaining four arrays — exhaustive at compile time.
}
```

`Section::fixed_item_bytes()` returns `Some(N)` only when `alignments`, `pre_layout_ops`, `legalization_ops`, and `branches` are all empty; otherwise `None`. The check is four `is_empty()` calls — no iteration over a wrapper enum. **Privilege changes revalidate all effect-producing arrays** (`instrs`, `pre_layout_ops`, `legalization_ops`, `branches`) via `validate_section_for_privilege` — a downgrade that would reject an already-emitted item returns `SectionPrivilegeError { seq_index, effect, violation }`. The `seq_index` is the global authoring index of the earliest offending item; with multiple violations, the lowest `seq_index` wins.

This is the structural payoff of SoA: deciding "is this section encoder-ready?" is a type-system question (does the value have type `LegalizedSection`?), not a runtime question (does the encoder need to handle a stray `LegalizationOp`?). The encoder cannot accidentally accept un-legalized input because it physically cannot receive one.

#### 2.7.4 `PreLayoutOp`, `LegalizationOp`, and auxiliary types

```rust
pub enum PreLayoutOp {
    BankLease(BankLeaseSpec),
    BankRelease { lease_id: LeaseId },
    Yield { kind: YieldKind },
    TraceProbe { id: TraceProbeId, level: ProbeLevel },
    AssertBank { expected: MbcBankClass, expected_n: u8 },
}

pub enum LegalizationOp {
    FarCall { target: SymbolName, lease_chain: Vec<LeaseId> },
}

// LegalizationOp::FarCall is a builder-level structured request. During
// relax/legalization it is converted into a `SymbolicBranch::Call` with
// `CallReachability::AutoFar`; ordinary `Builder::call` emits `NearOnly`.
```

Split by *placement dependence* (see §4): pre-layout ops lower before bank assignment; legalization ops lower during relaxation when target banks are known.

Auxiliary newtypes:

- `BankLeaseSpec { lease_id, class, bank }` with `MAX_ROM_BANK = 0x01FF` (MBC5 9-bit ROM bank), `MAX_SRAM_BANK = 0x000F` (4-bit SRAM bank).
- `MbcBankClass { Rom, Sram }`, `LeaseId(u32)`, `YieldKind { PollInterrupts, FrameBoundary, Cooperative }`, `TraceProbeId(u32)`, `ProbeLevel { Trace, Debug, Info }`.

The encoder rejects all structured ops (§8.3); they must be replaced by `Instr` sequences via the lowering traits before encoding.

#### 2.7.5 `SymbolName` and `SymbolSegment`

`SymbolSegment(Cow<'static, str>)` holds *one* segment with no dots; non-empty; `[a-z0-9_]` only.

`SymbolName(Cow<'static, str>)` holds a dot-separated chain. Constructors:

- `kernel(family, id)` → `kernel.<family>.<id>`
- `expert(layer, id)` → `expert.<layer>.<id>`
- `runtime(module, sym)` → `runtime.<module>.<sym>`
- `section(role, id)` → `section.<role_canonical>.<id>`
- `prefixed(prefix, tail)` → validates each prefix segment and appends the already-validated segments of `tail`.
- `runtime_thunk_for(target)` → `runtime.banking.thunk.<target segments...>`

Each constructor validates its `&str` arguments as segments — `runtime("banking.lease", "enter")` returns an error. Constructors that need to prefix an existing symbol operate on validated `SymbolName` segments rather than string-concatenating dotted names.

#### 2.7.6 `SymbolTable` and `SymbolAddress`

```rust
pub struct SymbolAddress { pub section: SectionId, pub offset: u32 }
pub struct SymbolTable { by_name: BTreeMap<SymbolName, SymbolAddress> }
```

Names are unique. **Addresses may have multiple names** — aliasing is essential for listing cross-references (e.g., `kernel.matvec.tile_8x8` and `kernel.matvec.tile_8x8.entry` at the same offset). `names_for(addr)` returns all names; `resolve(name)` returns the address. Sorted iteration → deterministic `.sym` output.

#### 2.7.7 `InstrProvenance` and `PlanningStage`

```rust
#[repr(u8)]
pub enum PlanningStage {
    QuantGraph    = 0,
    StoragePlan   = 1,
    RomWindowPlan = 2,
    OverlayPlan   = 3,
    ArenaPlan     = 4,
    GbSchedIr     = 5,
    Backend       = 6,
}

pub struct InstrProvenance {
    pub stage: PlanningStage,
    pub source_node: Option<AsmSourceValueId>,
    pub source_op: Option<Cow<'static, str>>,
    pub note: Option<Cow<'static, str>>,
}
```

Discriminants pinned at `0..=6`. **Shared with Epic B's `RepairProposal::source`** — Epic B re-uses; F-A1 owns. `AsmSourceValueId(u32)` is an adapter handle so `gbf-asm` doesn't depend on `gbf-ir`.

#### 2.7.8 Why these choices matter for T-A1.5–T-A1.9

| Decision                                                | Consequence                                                                                                                              |
|---------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------|
| `pre_layout_ops` array with unknown per-item byte length | Layout cannot estimate size until pre-layout lowering runs. `LoweredSection` has no `pre_layout_ops` field.                              |
| `legalization_ops` array survives layout                 | Layout reserves an upper bound; relax lowers once placement is known. `LegalizedSection` has no `legalization_ops` field.                |
| `Align` returns `None` for `fixed_byte_len`             | Layout assigns concrete padding once positions are known; relax may re-pad after JR→JP widening shifts.                                  |
| `SectionPrivilege` validates on emit and on change      | Layout/relax/encoder may *trust* that no `Privileged` effect appears in a `Normal` section.                                              |
| `SymbolName` validation                                 | `.sym` writer and listing pass names through unchanged (modulo the optional dot-safe escape used when consumers cannot tolerate dotted names).                                            |
| `SymbolTable` aliasing                                  | Listing shows all aliases at a label position. ROM builder doesn't need a separate cross-reference structure.                            |
| `PlanningStage` shared enum                             | Listing groups lines by stage; Epic B re-uses for `RepairProposal::source`.                                                              |
| `InstrProvenance::source_op` is `Cow<'static, str>`     | Static literals zero-allocate; runtime-formatted strings still work. Listing emits `op=<str>` faithfully.                                 |
| No `SectionItem::Raw` and no `MachineEffect::OpaqueBytes` | There is no opaque-byte path. Compiler-generated code, runtime code, and the ROM builder all emit through `Instr`/`Db`/`Dw`. The encoder never sees a "trust me" buffer; the effect classifier never sees a `?` it has to wave through. |

#### 2.7.9 Tests in tree

- `role_exhaustive` — every `SectionRole` has a unique canonical name; serde round-trips.
- `privilege_inheritance` — Normal rejects `StoreToMbcRegister`; Privileged accepts; allowlist restriction works; `interrupt_handler` accepts `Reti` and rejects `DI`.
- `section_items_carry_provenance_and_size` — `fixed_item_bytes()` returns `Some(7)` for instr+db+dw, `None` after a structured op is added.
- `canonical_naming` — each constructor produces expected dotted path; dot-injection rejected.
- `runtime_thunk_name_uses_validated_target_segments` — `runtime_thunk_for(kernel.matvec.tile_8x8)` yields `runtime.banking.thunk.kernel.matvec.tile_8x8` without accepting raw dotted input segments.
- `stage_enum_stable` — discriminants pinned at `0..=6`.

#### 2.7.10 F-A1 symbol-table addition

F-A1 adds external symbol support because the stub lowerers emit calls to runtime ABI entry points before F-A4 provides their production definitions.

```rust
pub enum SymbolKind {
    Defined(SymbolAddress),
    ExternalRuntime,
}

pub struct SymbolTable {
    by_name: BTreeMap<SymbolName, SymbolKind>,
}
```

`relax` may resolve only `Defined` symbols. `ExternalRuntime` is a forward declaration, not an address. The active stub runtime or production runtime materializer must replace every required external with a concrete `Defined` symbol before final relaxation.

```rust
pub fn resolve_runtime_externals(
    sections: Vec<LoweredSection>,
    symbols: SymbolTable,
    runtime: &dyn RuntimeMaterializer,
) -> Result<(Vec<LoweredSection>, SymbolTable), LinkError>;
```

`encode_section` never resolves externals. A remaining external after
`resolve_runtime_externals` is a `LinkError::UnresolvedExternalRuntime`, not a
relax or encode error.

### 2.8 T-A1.3 deep dive — Builder eDSL (`builder.rs`, ~750 LOC, `bd-3p1` closed)

#### 2.8.1 What shipped

```rust
pub struct Builder {
    section: Section,
    cur_provenance: InstrProvenance,
    next_label_id: u32,
    labels: BTreeSet<SymbolName>,
    active_leases: BTreeSet<LeaseId>,
}
```

#### 2.8.2 Emit methods

Each method has an *infallible* form (panics on violation) and a `try_*` form (returns `Result<_, BuilderError>`).

| Method                                                    | Notes                                                                                  |
|-----------------------------------------------------------|----------------------------------------------------------------------------------------|
| `emit(Instr)` / `try_emit`                                | Validates `classify_effect(&instr)` against section privilege.                          |
| `db(byte)` / `try_db` / `db_bytes` / `try_db_bytes`       | Rejects with `BuilderError::InlineDataInExecutableSection { role }` if `!role.permits_inline_data()`. Privilege is *not* sufficient — even a Privileged executable section rejects inline bytes. |
| `dw(word)` / `try_dw` / `dw_words` / `try_dw_words`       | Same role gate as `db`; two bytes per word, little-endian.                              |
| `label(name) -> SymbolId` / `try_label`                   | Records a label marker; rejects duplicate names.                                       |
| `align(NonZeroU16)` / `try_align(u16)`                    | Section-local alignment; `try_align` rejects zero.                                     |
| `with_provenance(p, f)`                                   | Scope guard; restores previous provenance on normal return AND caught panic.            |

(There is no `raw` / `try_raw` method. The previous "audited escape hatch" is removed; the cartridge header and any other byte-blob inputs are emitted via `db_bytes(...)` into a data-only-role section — `HeaderCartridge` for the header itself.)

#### 2.8.3 Structured-op methods

| Method                                                              | Lifecycle invariant                                                                                                                                  |
|---------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------|
| `bank_lease(spec)` / `try_bank_lease(spec)`                         | Inserts `lease_id` into `active_leases`. Returns `BuilderError::DuplicateLease` if already active. Emits `PreLayoutOp::BankLease`.                  |
| `bank_release(lease_id)` / `try_bank_release(lease_id)`             | Removes `lease_id` from `active_leases`. Returns `BuilderError::UnknownLease` if not active. Emits `PreLayoutOp::BankRelease`.                       |
| `far_call(target, lease_chain)` / `try_far_call(target, lease_chain)` | Validates every id in `lease_chain` is currently active. Emits `LegalizationOp::FarCall`.                                                            |
| `yield_op(kind)` / `try_yield_op(kind)`                             | Emits `PreLayoutOp::Yield`.                                                                                                                          |
| `trace_probe(id, level)` / `try_trace_probe(id, level)`             | Emits `PreLayoutOp::TraceProbe`.                                                                                                                     |
| `assert_bank(class, n)` / `try_assert_bank(class, n)`               | Emits `PreLayoutOp::AssertBank`. Validates SRAM bank ≤ 15.                                                                                           |

The lease lifecycle is enforced *at builder time*. By `finish()`, every emitted `BankLease` either has a matching `BankRelease` or remains in `active_leases` (sections may legitimately end with an active lease pending tail-call). The current builder does not enforce closing leases at `finish()`; F-A4 may add a stricter `lifecycle_closed_at_finish` mode.

#### 2.8.4 `with_provenance` panic safety

```rust
pub fn with_provenance<R>(&mut self, p: InstrProvenance, f: impl FnOnce(&mut Self) -> R) -> R {
    let previous = std::mem::replace(&mut self.cur_provenance, p);
    let result = catch_unwind(AssertUnwindSafe(|| f(self)));
    self.cur_provenance = previous;       // restore before re-raising
    match result { Ok(r) => r, Err(payload) => resume_unwind(payload) }
}
```

If the closure panics, `cur_provenance` is restored before the panic is re-raised. Builder remains in a valid state if the panic is caught upstream. (Closure-skill rule.)

#### 2.8.5 `BuilderError`

```rust
pub enum BuilderError {
    ZeroAlignment,
    DuplicateLabel { name: SymbolName },
    TooManyLabels,
    DuplicateLease { lease_id: LeaseId },
    UnknownLease { lease_id: LeaseId },
    SramBankOutOfRange { bank: u8 },
    PrivilegeViolation { effect: MachineEffect, violation: PrivilegeViolation },
    SectionPrivilegeViolation(SectionPrivilegeError),
}
```

#### 2.8.6 Why these choices matter for T-A1.5–T-A1.9

| Decision                              | Consequence                                                                                                                       |
|---------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------|
| Privilege check at emit time          | Layout/relax/encoder do not re-check.                                                                                              |
| Lease lifecycle in builder            | Lowering can rely on `lease_chain` arguments referring to existing leases.                                                         |
| `try_*` infallible variants           | F-A5 uses panicking forms in static contexts; F-B13 uses `try_*` in fallible contexts.                                             |
| `with_provenance` panic safety        | Compiler-generated code with scoped provenance can recover from internal panics without leaked state.                              |
| `BuilderError` enumerated             | FailureCapsule (Epic F) can match on specific failure modes.                                                                       |

#### 2.8.7 Tests in tree

- `roundtrip` — small section, 5 expected `SectionItem`s in order.
- `provenance_recorded` — `with_provenance` correctly attaches custom provenance and restores after.
- `pseudo_ops_dont_panic` — every structured-op variant produces the expected `SectionItem`.
- `builder_rejects_invalid_alignment_and_duplicate_labels` — `try_align(0)` fails; double label fails.
- `builder_validates_lease_lifecycle_and_bank_ranges` — out-of-range bank, double-lease, release-without-lease, far-call referencing unknown lease all fail.
- `builder_rejects_privileged_effects_in_normal_sections` — `LD ($2000), A` rejected from CommonBank without privileged.
- `builder_revalidates_existing_items_when_privilege_changes` — downgrade after MBC write returns `SectionPrivilegeViolation`.
- `db_dw_rejected_in_executable_sections` — `try_db`/`try_db_bytes`/`try_dw`/`try_dw_words` return `InlineDataInExecutableSection { role }` for every executable role (`Bank0Nucleus`, `CommonBank`, `ExpertBank`), including a `Privileged` `CommonBank` (privilege does not relax the gate). Each data-only role accepts the same calls.
- `provenance_scope_restores_after_caught_panic` — panic inside `with_provenance` closure → previous provenance restored.

#### 2.8.8 Open follow-ups (not blocking F-A1)

- **`Builder::push_section_marker`** — possible, not strictly needed.
- **Lease lifecycle policy**: optional strict mode at `finish()`. F-A4 decision.

### 2.9 T-A1.4 deep dive — `MachineEffect` + `PrivilegeClass` (`effect.rs`, ~1000 LOC, `bd-1bw` closed)

#### 2.9.1 The classification taxonomy

`MachineEffect` is a closed enum — significantly more detailed than the original task proposal. The expansion separates several axes:

**Static memory regions**:

```rust
LoadFromBank0, LoadFromSwitchableRom, LoadFromWram, LoadFromHram,
LoadFromSwitchableSram, LoadFromVram, LoadFromOam,
LoadFromIo { reg: IoRegister },
LoadFromUnusable,                               // $FEA0..=$FEFF dead zone
StoreToWram, StoreToHram, StoreToSwitchableSram,
StoreToVram, StoreToOam,
StoreToIo { reg: IoRegister },
StoreToUnusable,
StoreToMbcRegister { reg: MbcRegisterClass },   // $0000..=$7FFF writes
```

**Dynamic-address effects** (surfaced as a *reachability obligation*):

```rust
LoadFromDynamic       { via: DynamicAddress },
StoreToDynamic        { via: DynamicAddress },
ReadModifyWriteDynamic{ via: DynamicAddress },
```

`DynamicAddress` is `Bc | De | Hl | HlIncrement | HlDecrement`. `gbf-asm` *does not* claim that `LD A, (HL)` resolves to WRAM; it surfaces a `LoadFromDynamic { via: Hl }` and lets `ReachabilityValidation` (Epic B) discharge it.

**Stack operations** (explicitly modeled, not collapsed into PureCompute):

```rust
LoadFromStack,   // POP rr
StoreToStack,    // PUSH rr
```

Closure-skill rule: "Effect classifiers must not collapse stack-touching instructions into pure compute."

**Mixed-region two-byte stores**:

```rust
StoreToMixedStatic { first: StaticMemoryRegion, second: StaticMemoryRegion },
```

For `LD (nn), SP` where `nn` straddles a region boundary (e.g., `$9FFF` writes byte 1 to VRAM and byte 2 to SRAM).

**Privileged effects**:

```rust
StoreToMbcRegister { reg: MbcRegisterClass },
```

(Note: `OpaqueBytes` was removed alongside the `Raw` escape hatch — there is no opaque effect in the taxonomy. The exact variant count is derived from `MachineEffectKind::ALL`, not from prose.)

**Control flow**:

```rust
InterruptControl(InterruptControlOp),           // EI/DI/HALT/STOP
UnconditionalBranch, ConditionalBranch, Call, Return, Reti,
Rst { vector: RstVector },
```

`Reti` is its own variant so privilege can require `InterruptHandler` for it.

**Structured-op effects**:

```rust
SystemCall(SystemCallKind),                     // BankLease/BankRelease/FarCall/Yield/TraceProbe/AssertBank
```

Both `PreLayoutOp` and `LegalizationOp` map to `SystemCall(<kind>)`.

#### 2.9.2 `MachineEffectKind`

A parameter-free version (33 variants) for `BTreeSet<MachineEffectKind>` in `SectionPrivilege::allowed_effects`. `MachineEffect::kind()` projects.

#### 2.9.3 `PrivilegeClass`

```rust
pub enum PrivilegeClass { Normal, Privileged, InterruptHandler }
```

Three orthogonal classes. `Privileged` accepts `Normal+Privileged`; `InterruptHandler` accepts `Normal+InterruptHandler`. `Privileged` does **not** accept `InterruptHandler` effects (and vice versa) — banking ABI cannot be used inside an ISR; `RETI` cannot appear in a banking helper.

`SectionPrivilege` adds `allows_interrupt_disabled` (whether `DI` is acceptable) and `allowed_effects: Option<BTreeSet<MachineEffectKind>>` allowlist.

#### 2.9.4 Classifier functions (all `const fn`)

```rust
pub const fn classify_effect(instr: &Instr) -> MachineEffect;
pub fn       classify_pre_layout_op(op: &PreLayoutOp) -> MachineEffect;
pub fn       classify_legalization_op(op: &LegalizationOp) -> MachineEffect;
pub const fn privilege_of(effect: &MachineEffect) -> PrivilegeClass;
```

`classify_effect` is exhaustive over all 56 `Instr` variants. Adding a new variant is a compile error here. Static-address `LD A, (nn)` dispatches through `static_region(addr.get())`.

`privilege_of` notable mappings:

- `StoreToMbcRegister { _ }` → `Privileged`
- `InterruptControl(_)` → `Privileged`
- `SystemCall(_)` → `Normal`
  Structured calls are safe authoring-level requests. The privileged work happens only inside the runtime helper sections that implement those requests; those helper sections are classified and validated through their concrete `Instr` bodies.
- `Reti` → `InterruptHandler`
- `StoreToMixedStatic` → `Privileged` if either region is Bank0/SwitchableRom (= MBC write)
- everything else → `Normal`

Validation has one additional hard rejection independent of privilege class:

```rust
if matches!(
    effect,
    MachineEffect::StoreToMbcRegister {
        reg: MbcRegisterClass::Reserved
    }
) {
    return Err(PrivilegeViolation::ForbiddenMbcRegister {
        reg: MbcRegisterClass::Reserved,
    });
}
```

MBC5 `$6000..=$7FFF` is not a privileged register class; it is forbidden for every section, including runtime helper sections.

#### 2.9.5 The reachability obligation API

```rust
impl MachineEffect {
    pub const fn requires_dynamic_address_proof(self) -> bool { ... }
    pub const fn disables_interrupts(self) -> bool { ... }
}
```

`requires_dynamic_address_proof` returns `true` for `LoadFromDynamic`, `StoreToDynamic`, `ReadModifyWriteDynamic`. Epic B's `ReachabilityValidation` queries this and discharges the obligation using provenance + section context. The `via` field tells reachability which register to track.

#### 2.9.6 Why these choices matter for T-A1.5–T-A1.9

| Decision                                                  | Consequence                                                                                                                              |
|-----------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------|
| Closed taxonomy with dynamic-address obligations          | Layout/relax/encoder treat effects as classified; they trust emit-time validation. `OpaqueBytes` is absent.                              |
| No opaque-byte effect                                     | Every byte the encoder emits has a typed source (`Instr`, `Db`, `Dw`); there is no "trust me" path past the classifier.                   |
| `StoreToMbcRegister` → `Privileged`, except `Reserved` → hard error | Encoder doesn't special-case MBC addresses; the builder/effect validator rejects forbidden MBC5 ranges before lowering. |
| `SystemCall(_)` → `Normal`                                | Normal code can request the safe banking/yield/assert ABI without being allowed to emit privileged hardware writes directly. |
| `Reti` → `InterruptHandler`                                | An ISR section accepts `RETI`; a regular function section using `Normal` rejects it.                                                      |
| `classify_effect` is `const fn` and exhaustive             | Cycle model dispatches without runtime cost; future variants force compile-time updates.                                                  |
| Structured-op effects modeled as `SystemCall`              | Layout sees `PreLayoutOp` as opaque sized markers; lowering replaces them; encoder rejects them.                                          |
| `is_hram_addr` correctly handles IO/HRAM split             | `$FF80..=$FFFE` = HRAM; `$FF00..=$FF7F + $FFFF` = IO.                                                                                     |

#### 2.9.7 Tests in tree

- `classify_exhaustive` — table of ~70 `(Instr, MachineEffectKind)` pairs.
- `mbc_writes_are_privileged` — every `LdDirectFromA { addr }` to an MBC register classifies as `StoreToMbcRegister` and `privilege_of` → `Privileged`.
- `mbc_reserved_range_is_forbidden` — every write to `$6000..=$7FFF` classifies as `StoreToMbcRegister { Reserved }` and validation rejects it even for `SectionPrivilege::privileged()`.
- `structured_ops_are_normal_authoring_effects` — `BankLease`, `BankRelease`, `FarCall`, `Yield`, `TraceProbe`, and `AssertBank` can be emitted from a normal section; their runtime helper implementations remain separately privileged.
- Mixed-region store coverage — `LdDirectFromSp { addr: 0x9FFF }` → `StoreToMixedStatic { first: Vram, second: SwitchableSram }`.
- HRAM/IO boundary — `LdHighDirectFromA { offset: 0x80 }` (= `$FF80`) is `StoreToHram`; `offset: 0x40` (= `$FF40`, LCDC) is `StoreToIo`.

#### 2.9.8 Open follow-ups (not blocking F-A1)

- **Address-static reachability discharge in `gbf-asm`?** Decision: leave to Epic B. `gbf-asm` should not grow control-flow analysis. Closure skill: "Dynamic-address load/store effects must be named as reachability obligations; do not silently call them fixed-region effects without a proof from a later pass."
- **`MachineEffect::Reti` privilege**: `InterruptHandler`-only is correct; F-A5 ISR vector code emits `RETI` from `InterruptHandler` sections.

## 3. Architecture

### 3.1 Data flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                        Authoring (existing)                              │
│  Builder → Section { typed SoA arrays, role, priv, ... }                 │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<Section>
┌──────────────────────────────────────────────────────────────────────────┐
│  PreLayoutOp lowering (new — §4)                                         │
│  Section → LoweredSection                                                │
│  - Driven by `dyn PreLayoutOpLowering`                                   │
│  - Drains the entire `pre_layout_ops` array; the resulting type has      │
│    no such field. LegalizationOps remain as late obligations.            │
│  - Emits a `LoweringReport` mapping lowered sites → resulting items      │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<LoweredSection>
┌──────────────────────────────────────────────────────────────────────────┐
│  layout::layout_into_banks  (T-A1.6, §6)                                 │
│  Vec<LoweredSection> + PlacementProfile → LayoutPlan                     │
│  - Bank assignment under profile semantics                               │
│  - Per-section start address                                             │
│  - Header section pinned to bank 0 / $0100                               │
│  - Thunk pool sections reserved at end-of-Bank0                          │
│  - Errors: SectionTooBig, NoBankFits, ProfileViolation                   │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  LayoutPlan (initial)
┌──────────────────────────────────────────────────────────────────────────┐
│  relax::relax_branches      (T-A1.6, §7)                                 │
│  iterative-monotone:                                                     │
│   1. Resolve symbols against current LayoutPlan                          │
│   2. For each branch/call, check legality:                               │
│      - JR target within ±127? else upgrade to JP                         │
│      - CALL target same bank? else replace with `CALL <thunk>` and       │
│        add the thunk to bank-0 thunk pool                                │
│   3. Lower LegalizationOp values now that placement is known.             │
│   4. If any item grew or any thunk was added, re-layout addresses.       │
│   5. Loop until fixed point or data-dependent iteration cap.             │
│  - Errors: NoFixedPoint, ThunkOverflow                                   │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<LegalizedSection>, LayoutPlan (final), SymbolTable
┌──────────────────────────────────────────────────────────────────────────┐
│  encoder::encode_section    (T-A1.7, §8) — the only Instr→bytes path     │
│  (LegalizedSection, PlacedSection) → EncodedSection                      │
│  - One match arm per `Instr` variant                                     │
│  - DataBlock and finalized padding bytes straight through                │
│  - `Label` consumes 0 bytes if retained for provenance/listing           │
│  - LegalizedSection has no pre_layout_ops/legalization_ops/branches      │
│    arrays at all — un-legalized input is a compile error, not a runtime │
│    error. (Match is exhaustive over the four remaining arrays.)          │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<EncodedSection>
┌──────────────────────────────────────────────────────────────────────────┐
│  rom::assemble_rom          (T-A1.9, §9)                                 │
│  Vec<EncodedSection> + CartridgeHeader → Vec<u8>                         │
│  - Place each EncodedSection at bank * 0x4000 + start                    │
│  - Fill gaps with 0xFF                                                   │
│  - Inject Nintendo logo at $0104..=$0133                                 │
│  - Compute and inject header + global checksums                          │
│  - Pad to power-of-two ROM size                                          │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  .gb bytes
┌──────────────────────────────────────────────────────────────────────────┐
│  listing::emit_listing       (T-A1.8, §10)                               │
│  symbols::write_sym          (post-T-A1.2, §10)                          │
│  Both byte-stable across runs.                                           │
└──────────────────────────────────────────────────────────────────────────┘
```

### 3.2 Module ownership

| Module           | Owns                                                                | Reads                                             |
|------------------|---------------------------------------------------------------------|---------------------------------------------------|
| `cycle_model`    | `CycleCost`, `cycle_cost(&Instr)`                                   | `isa::Instr` only                                 |
| `lowering`       | `PreLayoutOpLowering`, `StubPreLayoutOpLowering`, `LoweredSection`  | `section`, `effect`, `symbols`                    |
| `layout`         | `LayoutPlan`, `PlacedSection`, `PlacementProfile`, `LayoutError`    | `section`, `cycle_model`, lowered output          |
| `relax`          | `RelaxAction`, `RelaxError`, fixed-point driver, `LegalizationOpLowering`, `LegalizedSection` | `layout`, `isa`, `lowering` |
| `encoder`        | `EncodedSection`, `EncodeError`, `encode_instr`, `encode_section`   | `isa`, legalized output, `symbols`, `layout`      |
| `listing`        | `ListingOptions`, `emit_listing`, mnemonic formatter                | `encoder`, `cycle_model`, `provenance`            |
| `rom` (new)      | `CartridgeHeader`, `MbcType`, `RomAssemblyError`, `assemble_rom`    | `encoder`, `layout`, `gbf-hw::mbc5` (when present)|
| `symbols::sym`   | `.sym` writer + `FromStr` parser (RGBDS-compatible Game Boy format) | `layout`, `symbols`                               |

The `lowering` module is new — see §4. It is the smallest possible expansion to the module set: `PreLayoutOp` / `LegalizationOp` lowering is conceptually distinct from both layout and encoding, and giving it its own home avoids two anti-patterns (encoder calling out to runtime lowering, layout knowing runtime ABI details).

### 3.3 Determinism contract

Every public function in `cycle_model`, `layout`, `relax`, `encoder`, `listing`, `rom`, and `symbols::write_sym` MUST be deterministic. Concretely:

- **No `HashMap` iteration**. Use `BTreeMap` and `BTreeSet` everywhere ordering is observable.
- **No `SystemTime`, no `rand`, no `std::env`, no thread-local state**.
- **No iterator non-determinism** (e.g., `par_iter` without a sort).
- **Stable section ordering**: layout receives `Vec<Section>` and processes in input order, modulo placement-profile-driven moves; placement-profile moves are themselves keyed by stable predicates (size, role, hash of name as tie-breaker).
- **Stable thunk naming**: thunk symbols are `runtime.banking.thunk.<target_symbol>` (one thunk per cross-bank CALL target) — fully derivable from the callee's `SymbolName`, no global counter, and trivially deduplicated across multiple call sites that share a target.

A property test in `gbf-asm::tests::determinism` will run the full pipeline on a curated section set twice and assert byte-equal `.gb`, `.lst`, and `.sym` output.

## 4. Structured op lowering (the seams)

### 4.1 Why structured ops cannot be lowered inside the encoder

The encoder's job is "translate one `Instr` to its canonical byte sequence." A structured op such as `BankLease { lease_id: 7, class: Rom, bank: 42 }` is *not* an `Instr` — it represents an entire calling convention into runtime code that:

1. Saves caller state if needed.
2. Writes MBC5 ROMB / RAMB registers.
3. Updates HRAM bank shadow registers (so ISRs see consistent state).
4. Returns control with the new bank visible.

Each of those steps is multiple `Instr`s. Their exact form depends on the F-A4 ABI choice (which calling convention, which HRAM offsets, which ISR-disable strategy). Putting that decision tree inside the encoder would bind the encoder to F-A4 and violate the engineering rule "only the encoder translates legal instructions to bytes" — because then the encoder also translates abstract structured ops, which is a different category of work.

### 4.2 Two phase-named op classes

The RFC uses phase names so the lowering order is visible from the term:

- `PreLayoutOp`: placement-independent intent lowered before layout.
- `LegalizationOp`: placement-dependent intent lowered during legalization,
  after final placement facts are available.

The current code now exposes this split directly as the typed `pre_layout_ops`
and `legalization_ops` arrays on `Section`. New design text should avoid using
"pseudo-op" when it means only one side of this split.

### 4.3 The early seam: `PreLayoutOpLowering`

`gbf-asm/src/section.rs` defines the op enums; `gbf-asm/src/lowering.rs` (new)
defines the lowering traits:

```rust
/// Placement-independent structured op lowered before layout.
pub enum PreLayoutOp {
    BankLease(BankLeaseSpec),
    BankRelease { lease_id: LeaseId },
    Yield { kind: YieldKind },
    TraceProbe { id: TraceProbeId, level: ProbeLevel },
    AssertBank { expected: MbcBankClass, expected_n: u8 },
}

/// Placement-dependent structured op lowered during legalization.
pub enum LegalizationOp {
    FarCall { target: SymbolName, lease_chain: Vec<LeaseId> },
}

/// Resolves a pre-layout op into concrete section items before layout.
///
/// Implementations live downstream: `gbf-runtime::banking` for the production
/// `BankLease`/`BankGuard` ABI; `gbf-runtime::scheduler` for `Yield`; etc.
/// `gbf-asm` itself ships only `StubPreLayoutOpLowering` for in-tree examples and
/// tests.
pub trait PreLayoutOpLowering {
    fn lower(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
    ) -> Result<LoweredFragment, LoweringError>;
}

/// Resolves a placement-dependent op during legalization.
pub trait LegalizationOpLowering {
    fn lower(
        &self,
        op: &LegalizationOp,
        ctx: &LegalizationContext<'_>,
    ) -> Result<LegalizedFragment, LoweringError>;
}

pub struct LegalizedFragment {
    pub labels:         Vec<FragmentItem<Label>>,
    pub instrs:         Vec<FragmentItem<Instr>>,
    pub data_blocks:    Vec<FragmentItem<DataBlock>>,
    pub alignments:     Vec<FragmentItem<Align>>,
    pub branches:       Vec<FragmentItem<SymbolicBranch>>,
    pub thunk_requests: Vec<ResolvedThunkRequest>,
}

/// Per-call-site context for lowering decisions.
pub struct LoweringContext<'a> {
    pub source_section_id: SectionId,
    pub source_section_role: SectionRole,
    pub provenance: &'a InstrProvenance,
    pub symbols: &'a SymbolTable,
}

/// One structured-op site replaced with concrete items and any thunk-table
/// requests this site introduced.
pub struct LoweredFragment {
    pub labels:           Vec<FragmentItem<Label>>,
    pub instrs:           Vec<FragmentItem<Instr>>,
    pub data_blocks:      Vec<FragmentItem<DataBlock>>,
    pub alignments:       Vec<FragmentItem<Align>>,
    pub legalization_ops: Vec<FragmentItem<LegalizationOp>>,
    pub branches:         Vec<FragmentItem<SymbolicBranch>>,
}

pub struct FragmentItem<T> {
    pub data: T,
    /// Position within the expansion of the structured op site.
    pub sub_index: u16,
    pub provenance: InstrProvenance,
}

/// Resolved only during relax/legalization, after the target's final bank and
/// CPU-visible address are known.
pub struct ResolvedThunkRequest {
    /// Canonical thunk name: `runtime.banking.thunk.<target_symbol>`. One
    /// thunk per cross-bank CALL target, NOT per callee bank: a 3-byte CALL
    /// has only one 16-bit operand, so the thunk body must encode both the
    /// callee bank *and* the destination address itself. Multiple call sites
    /// targeting the same symbol share a single thunk.
    pub thunk_symbol: SymbolName,
    /// The original cross-bank CALL target. The relax pass rewrites
    /// `CALL <target>` to `CALL <thunk_symbol>`; the thunk pool builder uses
    /// `(target, callee_bank, target_cpu_addr)` to materialize the thunk body.
    pub target: SymbolName,
    pub callee_bank: BankIndex,
    pub target_cpu_addr: u16,
    pub policy: ThunkPolicy,
}

pub enum LoweringError {
    UnknownTargetSymbol(SymbolName),
    UnsupportedStructuredOp(SystemCallKind),    // when a stub doesn't handle some op
    PolicyViolation(LoweringPolicyError),
}
```

The pre-layout lowering pass walks each `Section` and lowers `PreLayoutOp`
values. A `LoweredSection` carries this partially lowered output. It is not
necessarily encoder-ready: labels, alignments, symbolic branches, and
`LegalizationOp` obligations may still require layout and relaxation.
`LoweredFragment` MUST NOT contain `PreLayoutOp`; pre-layout lowering is non-recursive and linear. It may contain `LegalizationOp` and `SymbolicBranch` because those are intentionally later-phase obligations.

Every fragment inserted by a lowerer is revalidated against the source
section's `SectionPrivilege` before it is spliced into the section. A lowerer
may emit ordinary calls to privileged runtime helper symbols from a normal
section, but it may not inline privileged hardware effects into that normal
section. Runtime helper sections remain separately authored, classified, and
validated.

### 4.4 `StubPreLayoutOpLowering` and `StubLegalizationOpLowering`

For F-A1 to ship without F-A4, `gbf-asm` provides stubs configured by an explicit policy:

```rust
pub struct StubLoweringConfig {
    pub trace_policy: TraceLoweringPolicy,
    pub assert_bank_policy: AssertBankLoweringPolicy,
}

pub enum TraceLoweringPolicy {
    EmitCalls,
    Elide,
}

pub enum AssertBankLoweringPolicy {
    EmitRuntimeCheck,
    Elide,
}
```

- `BankLease(spec)` → `SymbolicBranch::Call { target: runtime.banking.acquire.<class>.<bank>, reachability: NearOnly }` (placeholder symbol, resolved by the active stub/runtime materializer in tests/examples).
- `BankRelease(lease_id)` → `SymbolicBranch::Call { target: runtime.banking.release.<lease_id>, reachability: NearOnly }` (same).
- `Yield { kind }` → `SymbolicBranch::Call { target: runtime.scheduler.yield.<kind>, reachability: NearOnly }`.
- `TraceProbe { id, level }` → controlled by `StubLoweringConfig::trace_policy`.
- `AssertBank { ... }` → controlled by `StubLoweringConfig::assert_bank_policy`.
- `FarCall { target, lease_chain }` → legalization-time request for `target`'s bank, then `CALL <thunk>`.

The stub registers each external symbol as `SymbolKind::ExternalRuntime`. The example `tiny_rom` defines those symbols with hand-rolled `Builder` sections in its own `runtime` module, gated behind `cfg(feature = "stub-runtime")` so production builds (against F-A4) do not include the stub.

Acceptance: `cargo run -p gbf-asm --example tiny_rom` invokes the stub lowerers and produces a bootable `.gb`.

### 4.5 Why this layering is honest

The constitutional rule (planv0.md §1, line 1873) says compiler-generated code may **not** emit raw MBC writes directly; it must go through the `BankLease`/`BankGuard` ABI in `gbf-runtime::banking`. The phase-named lowerer traits are exactly that interface. `gbf-asm` defines the seams; `gbf-runtime::banking` (F-A4) implements production lowering. Compiler-generated code produces a structured `BankLease` op, never `Instr::LdDirectFromA { addr: 0x2000 }`. The privilege validator in `effect.rs` already rejects `StoreToMbcRegister` from `Normal` sections, so the rule is enforced at builder time.

### 4.6 Risk: stub lowerers create a parallel ABI

The risk is that the stub's symbol naming and calling convention diverge from F-A4's, so swapping the production lowering breaks every test that depended on the stub's exact byte layout. Mitigations:

- Tests assert *behavior* (boots, prints expected text) not exact bytes for stub-lowered sections.
- The stub is `pub(crate)` outside the `examples/` directory: production code cannot accidentally use it.
- F-A4's closure must include a "stub→prod migration" test that runs the same `tiny_rom` source against both lowerings and verifies they produce semantically equivalent output (same trace events on a fixture prompt).

## 5. Cycle model (T-A1.5, `cycle_model.rs`)

### 5.1 Design

```rust
/// Static M-cycle cost for one canonical instruction shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CycleCost {
    /// Fixed-cost instructions (most loads, ALU, NOP, RST, etc.).
    Fixed(NonZeroU8),
    /// Conditional branches, conditional calls, conditional returns.
    Branch { taken: NonZeroU8, not_taken: NonZeroU8 },
}

impl CycleCost {
    /// Worst-case M-cycles assumed when the consumer cannot predict the path.
    pub const fn worst_case(self) -> u8 { ... }
    /// Best-case M-cycles for slack analysis.
    pub const fn best_case(self) -> u8 { ... }
    /// T-state conversion (1 M-cycle = 4 T-states on LR35902). Always
    /// lossless — every documented LR35902 timing is divisible by 4 — so
    /// `gbf-bench` and absolute-time consumers can recover T-state precision
    /// from the M-cycle representation without a separate type.
    pub const fn t_states(self) -> TStateCost { ... }
}

/// T-state view of a `CycleCost`. Always equals `CycleCost * 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TStateCost {
    Fixed(NonZeroU16),
    Branch { taken: NonZeroU16, not_taken: NonZeroU16 },
}

/// Pure function: cost is fully determined by the variant.
#[must_use]
pub const fn cycle_cost(instr: &Instr) -> CycleCost;
```

`NonZeroU8` rules out the silent zero-cost trap that the closure skill warns about. (Pan Docs has no zero-cost LR35902 instruction.) The chosen unit is **M-cycles**, not T-states: every LR35902 instruction timing is divisible by 4 T-states, so M-cycles are a lossless representation that fits in `NonZeroU8` (max 6 for unconditional `CALL`); T-state callers (`gbf-bench`, absolute-time analyses) project via `t_states()` (resolves §14 question 1).

The function is `const`. It compiles to a single jump table. It makes no allocation. It does not depend on layout, addresses, or any environment.

### 5.2 Per-variant table (M-cycles, from Pan Docs `gbdev.io/pandocs/CPU_Instruction_Set.html`)

The full table covers all 56 `Instr` variants. Highlights with their references:

| Family                        | Cost (M-cycles)         | Notes                                                  |
|-------------------------------|-------------------------|--------------------------------------------------------|
| `Nop`, `Stop`                 | 1                       | Stop is 1 cycle but consumes 2 bytes (`10 00`).        |
| `Halt`                        | 1                       | Canonical hardware instruction. HALT-bug neutralization is provided by `Builder::safe_halt()`, which emits `HALT; NOP` as two instructions. The NOP's cost is charged only if execution reaches it after wake-up. |
| `Di`, `Ei`                    | 1                       |                                                        |
| `Ccf`, `Scf`, `Cpl`, `Daa`    | 1                       |                                                        |
| `Ld8Reg { dst, src }`         | 1                       | reg-to-reg                                             |
| `Ld8RegFromImm`               | 2                       | reg ← imm8                                             |
| `Ld8RegFromHl`                | 2                       | A ← (HL)                                               |
| `Ld8HlFromReg`                | 2                       | (HL) ← reg                                             |
| `Ld8HlFromImm`                | 3                       | (HL) ← imm8                                            |
| `LdAFromReg16Addr`            | 2                       | A ← (BC/DE/HL+/HL-)                                    |
| `LdAFromDirect`               | 4                       | A ← (nn) — three-byte instruction, two memory cycles  |
| `LdDirectFromA`               | 4                       |                                                        |
| `LdAFromHighDirect`           | 3                       | A ← ($FF00+n) — `LDH A,(n)`                            |
| `LdHighDirectFromA`           | 3                       |                                                        |
| `LdAFromHighC`                | 2                       | A ← ($FF00+C)                                          |
| `Ld16Imm`                     | 3                       | rr ← imm16                                             |
| `LdSpFromHl`                  | 2                       |                                                        |
| `LdDirectFromSp`              | 5                       | (nn) ← SP                                              |
| `LdHlFromSpPlus`              | 3                       | HL ← SP + i8                                           |
| `AddA { src: Reg }`           | 1                       |                                                        |
| `AddA { src: HlIndirect }`    | 2                       |                                                        |
| `AddA { src: Imm }`           | 2                       |                                                        |
| `Inc8 { Reg }`                | 1                       |                                                        |
| `Inc8 { HlIndirect }`         | 3                       | RMW                                                    |
| `Inc16`                       | 2                       |                                                        |
| `AddHl`                       | 2                       |                                                        |
| `AddSp`                       | 4                       |                                                        |
| `Rlca`, `Rrca`, `Rla`, `Rra`  | 1                       | non-CB rotates                                         |
| `Rlc/Rl/Rrc/Rr { Reg }`       | 2                       | CB-prefixed reg                                        |
| `Rlc/Rl/Rrc/Rr { HlIndirect }`| 4                       | CB-prefixed (HL)                                       |
| `Sla/Sra/Srl/Swap { Reg }`    | 2                       |                                                        |
| `Sla/Sra/Srl/Swap { HlIndirect }` | 4                   |                                                        |
| `Bit { Reg }`                 | 2                       |                                                        |
| `Bit { HlIndirect }`          | 3                       | (read only)                                            |
| `Res/Set { Reg }`             | 2                       |                                                        |
| `Res/Set { HlIndirect }`      | 4                       | RMW                                                    |
| `JpAbs { cond: None }`        | 4                       |                                                        |
| `JpAbs { cond: Some(_) }`     | Branch { 4, 3 }         |                                                        |
| `JpHl`                        | 1                       |                                                        |
| `JrRel { cond: None }`        | 3                       |                                                        |
| `JrRel { cond: Some(_) }`     | Branch { 3, 2 }         |                                                        |
| `Call { cond: None }`         | 6                       |                                                        |
| `Call { cond: Some(_) }`      | Branch { 6, 3 }         |                                                        |
| `Ret { cond: None }`          | 4                       |                                                        |
| `Ret { cond: Some(_) }`       | Branch { 5, 2 }         | unconditional/conditional differ                       |
| `Reti`                        | 4                       |                                                        |
| `Rst`                         | 4                       |                                                        |
| `Push`                        | 4                       |                                                        |
| `Pop`                         | 3                       |                                                        |

(Rest of the table — e.g., `LdReg16AddrFromA`, `LdHighCFromA`, the eight ALU sources for each ALU op — fills in similarly. Full enumeration in the implementation.)

### 5.3 Cycle cost for structured ops?

`PreLayoutOp` and `LegalizationOp` values are lowered to `Instr` sequences before the cycle model is consulted. Therefore `cycle_cost` does not need a structured-op variant. **However**, Epic B's `ScheduleCostAnalysis` (F-B12) may need an estimate of structured-op cost *before* lowering. That estimate is not part of F-A1's static LR35902 instruction cycle model.

F-A1 records the intended shape for a later API, but does not ship it from
`cycle_model.rs`. The later API should live in the crate that owns both the
scheduler's estimate vocabulary and the runtime ABI calibration bundle, to
avoid a dependency inversion.

Candidate shape:

```rust
pub fn structured_op_cost_estimate(...) -> CycleEstimate;
```

with three confidence classes:

- `CycleEstimate::Static(N)` — the cost is the cost of the lowered sequence, deterministic.
- `CycleEstimate::CalibratedRange { low, high }` — needs a calibration bundle to be tight; falls back to a documented worst-case.
- `CycleEstimate::WorstCase(N)` — no calibration; very loose.

This is a hint, not the final cost. The final cost comes from running `cycle_cost` over the lowered fragment. The estimate API exists so Epic B can budget without fully lowering. (`CycleEstimate` itself lives in `gbf-foundation` so `gbf-codegen` can use it without a `gbf-asm` dependency for that subset of work.)

### 5.4 Tests (per T-A1.5 acceptance + skill checklist)

- `cycle_model::known_instructions` — 30 spot-check entries verified against Pan Docs by URL.
- `cycle_model::no_zero_cost` — every `Instr` returns `Fixed(_)` or `Branch { ... }` with `NonZeroU8`. (Compile-time guarantee via `NonZeroU8`, plus a runtime sanity assertion that iterates all variants.)
- `cycle_model::branch_invariant` — for every `Branch { taken, not_taken }`, `taken.get() >= not_taken.get()`. The exact delta is instruction-family-specific:
  - conditional `JR`: `3/2` M-cycles, delta `1`;
  - conditional `JP`: `4/3` M-cycles, delta `1`;
  - conditional `CALL`: `6/3` M-cycles, delta `3`;
  - conditional `RET`: `5/2` M-cycles, delta `3`.
  The test asserts the expected pair per family instead of applying one global delta rule.
- `cycle_model::exhaustive_coverage` — proc-macro-free static check: a const test that constructs one example of each `Instr` variant and asserts `cycle_cost(...)` does not panic. (The function is `const`, so this is enforced at compile time — but we add the test anyway as a regression for any future variant added without updating the table.)
- `cycle_model::serde_roundtrip` — `CycleCost` serde stable.
- `cycle_model::t_states_lossless` — for every `Instr`, `cost.t_states()` round-trips back to `cost` via integer division by 4 (asserts the lossless conversion claim, including conditional branch projection).

### 5.5 Open question: cycle costs for `Stop`

`STOP` enters very-low-power mode. Pan Docs documents 1 M-cycle for the opcode itself, but the CPU then halts indefinitely until a button interrupt. For schedule-cost purposes, treating `Stop` as 1 M-cycle is correct (the cost charged to the slice), but `ScheduleCostAnalysis` should refuse to schedule a slice that contains `Stop` outside a power-down request. We model this as: cycle cost is 1, plus an `effect::InterruptControl(Stop)` annotation that downstream passes can detect. No special handling here.

## 6. Layout (T-A1.6 part 1, `layout.rs`)

### 6.1 Inputs

```rust
pub fn layout_into_banks(
    sections: Vec<LoweredSection>,
    profile: PlacementProfile,
    target: &TargetProfile,                     // from gbf-hw, when present; M0 fallback
    pinned: &[PinnedPlacement],                 // header at $0100, vectors at $0040-$0060
) -> Result<LayoutPlan, LayoutError>;

pub enum PlacementProfile {
    StrictOneExpertPerBank,
    Budgeted { reserve_bytes_per_bank: u16 },
    PackedExperts,
}

pub enum BankIndex {
    Rom(u16),      // 0..=511 for MBC5
    Sram(u8),      // 0..=15 for MBC5
    Wram,
    Hram,
    Vram,
    Oam,
}

pub struct PinnedPlacement {
    pub section_id: SectionId,
    pub bank: BankIndex,
    pub cpu_start: u16,
}

pub struct ReservedRange {
    pub bank: BankIndex,
    pub start: u16,
    pub end_inclusive: u16,
    pub reason: ReservedRangeReason,
}

pub enum ReservedRangeReason {
    CartridgeHeader,
    ResetVector,
    InterruptVector,
    ThunkPool,
    UserPinned,
}

pub struct LayoutPlan {
    pub sections: Vec<PlacedSection>,
    pub thunk_pool: Vec<ThunkSlot>,
    pub bank_count: u16,
    /// `BankIndex` is a non-string key. Serde's default JSON object encoding
    /// would panic at runtime (serde_json refuses non-string map keys). We
    /// gate this by serializing `BankIndex` to a stable canonical *string*
    /// representation when used as a map key — see §11.4 for the full rule.
    pub free_bytes_per_bank: BTreeMap<BankIndex, u32>,
}

pub struct PlacedSection {
    pub id: SectionId,
    pub space: AddressSpace,
    pub bank: BankIndex,
    /// CPU-visible start address.
    ///
    /// ROM0 sections use `$0000..=$3FFF`; ROMX sections use
    /// `$4000..=$7FFF`. This is not a ROM file offset.
    pub cpu_start: u16,
    pub final_size: u16,
    pub estimated_size: u16,                    // upper bound pre-relax
    /// Concrete padding chosen by layout for each `Align` item in the
    /// section. The encoder is not allowed to recompute this, because a
    /// different recomputation would break layout/listing/ROM agreement.
    pub alignment_padding: BTreeMap<ItemOrder, u16>,
}

pub enum AddressSpace {
    Rom0,
    RomX,
    Wram,
    Hram,
    Sram,
    Vram,
    Oam,
}

impl PlacedSection {
    pub fn cpu_end_exclusive(&self) -> u32 {
        self.cpu_start as u32 + self.final_size as u32
    }
}

impl PlacedSection {
    pub fn rom_file_offset(&self) -> Result<Option<usize>, LayoutError> {
        match (self.space, self.bank) {
            (AddressSpace::Rom0, BankIndex::Rom(0)) => {
                if self.cpu_start > 0x3FFF {
                    return Err(LayoutError::CpuAddressOutOfRange {
                        section_id: self.id,
                        space: self.space,
                        cpu_start: self.cpu_start,
                    });
                }
                Ok(Some(self.cpu_start as usize))
            }
            (AddressSpace::RomX, BankIndex::Rom(n)) if n >= 1 => {
                if !(0x4000..=0x7FFF).contains(&self.cpu_start) {
                    return Err(LayoutError::CpuAddressOutOfRange {
                        section_id: self.id,
                        space: self.space,
                        cpu_start: self.cpu_start,
                    });
                }
                Ok(Some(n as usize * 0x4000 + (self.cpu_start as usize - 0x4000)))
            }
            (AddressSpace::Wram | AddressSpace::Hram | AddressSpace::Sram | AddressSpace::Vram | AddressSpace::Oam, _) => {
                Ok(None)
            }
            _ => Err(LayoutError::BankSpaceMismatch {
                section_id: self.id,
                space: self.space,
                bank: self.bank,
            }),
        }
    }
}

pub struct ThunkSlot {
    /// Canonical name `runtime.banking.thunk.<target_symbol>`.
    pub symbol: SymbolName,
    pub bank: BankIndex,                        // always Rom(0) (Bank 0)
    pub cpu_start: u16,
    /// The original cross-bank CALL target — baked into the thunk body so
    /// the destination address survives the rewrite of the call site. The
    /// CALL operand at the call site is the thunk's address; the thunk
    /// itself jumps to `target` after switching to `callee_bank`.
    pub target: SymbolName,
    pub callee_bank: BankIndex,
}
```

### 6.2 Algorithm

1. **Categorize sections by role:**
   - `HeaderCartridge` → internal-only role used by `rom::build_header_section`. User-provided `HeaderCartridge` sections are rejected unconditionally in F-A1. Layout reserves bank 0 `$0100..=$014F`; the ROM builder constructs a typed internal header section at `$0100` and overlays it after all ordinary ROM sections have been placed.
   - `Bank0Nucleus`, `Bank0Data` → bank 0, outside reserved vectors/header regions.
   - `CommonBank`, `CommonData` → switchable banks 1..K.
   - `ExpertBank`, `ExpertData` → switchable banks K+1..N.
   - `WramHotArena`, `WramOverlay`, `HramFastFlags`, `SramPersistent`, `VramOwnedByUi`, `OamOwnedByUi` → not in ROM, but the layout pass still records their placement for the symbol table. Bank index is encoded as `BankIndex::Wram`, `BankIndex::Hram`, etc.

2. **Reserve a thunk pool slot at the high end of bank 0.** The pool size is computed as `lowering_pass.thunk_requests.iter().map(|r| r.estimated_size()).sum::<u32>()` plus a small slack (256 bytes default). `ThunkSlot::start` is set so the pool grows downward from `$3FFF`. If thunk pool overruns the available Bank 0 space, return `LayoutError::Bank0ThunkOverflow`.

3. **Profile-driven bank packing for `ExpertBank` / `CommonBank`:**
   - `StrictOneExpertPerBank`: each `ExpertBank` section gets its own bank. `CommonBank` packs greedily (biggest first).
   - `Budgeted { reserve_bytes_per_bank }`: a section may use only `bank_size - reserve` of any bank; remainder is reserved for runtime patches.
   - `PackedExperts`: multiple ExpertBank sections may co-reside if they collectively fit. Greedy-first-fit.

4. **Within each bank, assign concrete `start` addresses with an interval allocator:**
   - Seed each bank with reserved ranges. For Bank 0, this includes the cartridge header `$0100..=$014F`, any reset/interrupt-vector ranges not explicitly occupied by a pinned section, and the downward-growing thunk pool at the high end of `$0000..=$3FFF`.
   - Insert pinned placements first; a collision with an existing reserved range is a typed error unless the pinned placement is the owner of that reservation.
   - Then place unpinned sections in deterministic order. For greedy packing, sort by `(estimated_size descending, role, name, section_id)`; for source-order-sensitive profiles, sort by `(role, name, section_id)`.
   - For each candidate start, apply the section-level alignment and each `Align` item, record concrete padding in `PlacedSection::alignment_padding`, and reject placements that cross an occupied interval or bank boundary.
   - Track free intervals per bank, not a single cursor.

5. **Validate global invariants:**
   - No ROM0 section has `cpu_start < $0000` or `cpu_end_exclusive > $4000`.
   - No ROMX section has `cpu_start < $4000` or `cpu_end_exclusive > $8000`.
   - No section crosses its address-space boundary. All arithmetic is performed in `u32` before downcasting to `u16`.
   - All pinned placements honored.
   - All sections fit within their assigned bank.
   - Bank count is power-of-two and ≥2 (smallest MBC5 ROM is 32 KiB = 2 banks).

6. **Record `free_bytes_per_bank`** so the relax pass can know how much room is available for instruction widening.

### 6.3 `LayoutError`

```rust
pub enum LayoutError {
    SectionTooBig { id: SectionId, size: u32, bank_capacity: u32 },
    NoBankFits { id: SectionId, role: SectionRole },
    Bank0ThunkOverflow { requested: u32, available: u32 },
    PinnedPlacementCollision { section_a: SectionId, section_b: SectionId },
    PinnedPlacementOutOfRange { section_id: SectionId, start: u16, bank_capacity: u16 },
    DuplicatePinned { section_id: SectionId },
    InvalidBankCount { requested: u16, max_for_mbc5: u16 },
    AlignmentOutOfRange { section_id: SectionId, align: u16 },
}
```

Every variant carries enough state to reproduce the failure without reaching for the original `Vec<Section>`.

### 6.4 Section size estimation during layout/relax

Layout has two size views:

1. `current_size(relax_state)`: the byte size under the current relaxation decisions. Initial state assumes short symbolic `JR` and direct symbolic `CALL`.
2. `max_size()`: a conservative upper bound used only for early "can this ever fit?" diagnostics.

Concrete placement uses `current_size(relax_state)`, not `max_size()`, otherwise relative-branch distances are computed against pessimistic addresses and the relaxation pass can needlessly pessimize code.

| `SectionItem`      | Size estimate                                             |
|--------------------|-----------------------------------------------------------|
| `Label`            | 0                                                         |
| `Instr(I)`         | `I.byte_len()` (already exact and constant — see §isa)    |
| `Db(bytes)`        | `bytes.len()`                                             |
| `Dw(words)`        | `words.len() * 2`                                         |
| `Align(n)`         | up to `n - 1` padding bytes (worst case)                  |
| `PreLayoutOp(_)`   | should not exist after pre-layout lowering — internal error if seen |
| `LegalizationOp(_)` | known upper bound from legalization sizing policy         |
| `Branch::Jr(_)`    | current: `2` until widened, then `3`; max: `3` |
| `Branch::Call(_)`  | current: `3`; max: `3` plus possible thunk-pool growth |

Initial layout uses optimistic current sizes. After each relaxation iteration, layout recomputes concrete padding and section sizes from the updated relax state.

### 6.5 Tests

- `layout::no_section_crosses_bank` — random inputs, assert post-layout no `start + size > BANK_SIZE` for any placed section.
- `layout::strict_one_per_bank_semantics` — under `StrictOneExpertPerBank`, distinct ExpertBank sections never share a bank.
- `layout::budgeted_reserve_respected` — `Budgeted { reserve_bytes_per_bank: 256 }` leaves at least 256 bytes free at the end of every used bank that contains an ExpertBank/CommonBank section.
- `layout::packed_experts_packs_smaller_into_one` — under `PackedExperts`, two 8 KiB experts share a 16 KiB bank.
- `layout::pinned_header_at_correct_offset` — header section pinned at `$0100`.
- `layout::bank0_allocator_respects_header_and_vectors` — unpinned Bank0 sections are never placed into `$0100..=$014F` or reserved vector ranges; pinned vector sections may occupy their exact reserved interval.
- `layout::thunk_pool_at_bank0_top` — a thunk request creates a thunk slot at the top of Bank 0.
- `layout::deterministic_ordering` — same input twice → byte-equal `LayoutPlan`.
- `layout::error_section_too_big` — a 17 KiB section fails with `SectionTooBig`, not by panicking.
- Negative tests for every `LayoutError` variant.

## 7. Branch relaxation (T-A1.6 part 2, `relax.rs`)

### 7.1 The problem

LR35902 has three branch family choices:

- `JR off: i8` — 2 bytes, ±127 from the byte after the opcode.
- `JP addr: u16` — 3 bytes, anywhere in the 16-bit address space.
- `CALL addr: u16` — 3 bytes, but jumps to the **same address space**, so a cross-bank call would land in whatever bank is currently selected — usually the wrong one.

The `Builder` lets authors emit symbolic branches (e.g., `JR target_label`) without committing to encoding choice. Layout assigns concrete addresses. **Relaxation** is the pass that picks the right encoding once addresses are known.

### 7.2 Strategy: optimistic-then-grow with monotone fixed point

Algorithm:

1. **Initial assumption (optimistic):** every `JR cond?, target` is in range; every `CALL cond?, target` is intra-bank. Layout's initial size estimates use `JR.byte_len() == 2` and `CALL.byte_len() == 3`.
2. **Resolve symbols** against current layout addresses.
3. **For each branch instruction**, check legality:
   - `JR off, target`: first require that `target` is reachable in the currently visible CPU address space. Same-bank ROMX and Bank 0 targets are reachable; a different switchable ROM bank is not. If unreachable, return `RelaxError::CrossBankBranchUnsupported`. If reachable, compute `delta = target_cpu_addr - (here_cpu_addr + 2)`. If `delta ∈ [-128, 127]`, `KeepRelative`; otherwise `UpgradeToAbsolute`.
   - `JP target`: require the same reachability check. A plain `JP` never performs bank selection.
   - `CALL target` with `CallReachability::NearOnly`: require the same reachability check as `JP`. If the target is in a different switchable ROM bank, return `RelaxError::CrossBankBranchUnsupported`; do **not** silently introduce bank switching.
   - `CALL target` with `CallReachability::AutoFar { .. }`: if the target is reachable without changing the visible bank, emit an ordinary concrete `CALL`. Otherwise, rewrite the call operand to the **per-target** thunk symbol (`runtime.banking.thunk.<target>`) in Bank 0. The original destination address is *not* discarded — it is baked into the thunk body, since the 3-byte CALL cannot carry both bank number and destination.
4. **Apply changes:**
   - For each `UpgradeToAbsolute`: replace the `JrRel` `SectionItem` with `JpAbs`, increasing section size by 1 byte at that offset.
   - For each `AutoFar` site requiring a thunk: rewrite the concrete `CALL` operand to the per-target `runtime.banking.thunk.<target_symbol>` thunk. If a thunk for that target doesn't yet exist in the pool, allocate one (recording `(target, callee_bank, target_cpu_addr)` so the thunk body can be materialized). Two call sites that share a target share a thunk.
5. **Re-run layout** if any branch width changed or thunks were added.
6. **Loop** to step 2 until no changes. The iteration cap is data-dependent:

```rust
let max_relax_iters =
    1 + relaxable_branch_count + unique_cross_bank_call_target_count;
```

The default warning threshold remains `> 4` iterations because that is abnormal for real programs, but the hard safety cap is derived from the finite decision lattice.

This bound assumes each allocated thunk has a stable maximum size once its
target symbol is known. A `ThunkMaterializer` whose body size depends on the
thunk's own assigned address must report that dependency explicitly or be
rejected by the relax driver.

### 7.3 Why monotone

Each iteration can only make monotone decisions:

- Replace a symbolic short branch decision with a long branch decision.
- Add a previously absent per-target thunk slot.

No decision ever reverts. Alignment padding is recomputed after each layout and may shrink locally when an earlier item grows, but this does not invalidate the monotone decision lattice: each branch site widens at most once, and each cross-bank call target allocates at most one thunk. Therefore the fixed point is bounded by the number of relaxable branch sites plus the number of unique far-call targets.

The closure skill explicitly notes: "Iterative-monotone branch relaxation: out-of-range JR -> JP, cross-bank -> far-call thunk. Convergence to fixed point asserted by test." That requirement is met.

### 7.4 Far-call thunk shape

There is **one thunk per cross-bank CALL target symbol**, not one per callee bank. A 3-byte `CALL` carries a single 16-bit operand: rewriting that operand to point at a per-bank trampoline would erase the callee's destination address. The thunk therefore owns *both* the bank number and the 16-bit destination.

A thunk for target `kernel.matvec` (resident in callee bank N at `$ADDR`) is a small Bank-0 parameter stub. It does not directly write MBC registers and it does not tail-jump to the callee. Instead, it loads the callee bank/address into registers and jumps to a Bank-0 far-call helper that owns save/switch/call/restore/return semantics:

```text
runtime.banking.thunk.kernel.matvec:           ; in Bank 0
    LD   A, low8(N)                            ; callee ROM bank low bits
    LD   B, high1(N)                           ; callee ROM bank high bit, 0 or 1
    LD   HL, $ADDR                             ; callee CPU-visible address, usually $4000..$7FFF
    JP   runtime.banking.far_call_entry        ; Bank-0 helper restores caller bank before RET
```

Both `N` and `$ADDR` are emitted as concrete instruction immediates by the thunk-pool builder once layout has placed `kernel.matvec`. The CALL site only needs to reach the thunk; the thunk and helper together know where to go and how to restore the caller's selected bank.

This is illustrative; the **actual helper ABI is owned by F-A4** (interrupt-disable strategy, lease-chain handshake, register/flag preservation, return-bank restoration, error reporting). From the `gbf-asm` perspective, the thunk is a parameterized black box whose materialized instruction sequence is produced by the active `ThunkMaterializer`.

```rust
pub trait ThunkMaterializer {
    fn estimated_thunk_size(&self, request: &ResolvedThunkRequest) -> u16;

    fn materialize_thunk(
        &self,
        request: &ResolvedThunkRequest,
        ctx: &ThunkMaterializationContext<'_>,
    ) -> Result<LegalizedSection, LoweringError>;
}
```

`relax` owns the decision to allocate one thunk per target symbol. The
materializer owns the bytes and ABI shape of the thunk body. The materializer
must not decide to allocate additional unreported thunks; any such need must
be represented as another `ResolvedThunkRequest` so the fixed-point bound
remains valid.

For F-A1's stub lowering, each per-target thunk is a fixed-size sequence (default 12 bytes) of the shape above with `N` and `$ADDR` substituted from the `ResolvedThunkRequest`. Stubs deduplicate: two `CALL kernel.matvec` sites in different sections share `runtime.banking.thunk.kernel.matvec`. Examples that need real bank-switching wait for F-A4.

### 7.5 Errors

```rust
pub enum RelaxError {
    NoFixedPoint { iters: u8 },
    ThunkPoolExhausted { requested: u32, capacity: u32 },
    UnresolvedSymbol { name: SymbolName, used_in: SectionId },
    InvalidRelativeOffset { offset: i32 },
    CrossBankBranchUnsupported {
        used_in: SectionId,
        source_bank: BankIndex,
        target: SymbolName,
        target_bank: BankIndex,
    },
}
```

`NoFixedPoint` should never fire under correct inputs but exists as a safety net so a bug in the size accounting is loud rather than silent.

### 7.6 Tests

- `relax::out_of_range_jr_becomes_jp` — emit a section with a `JR` whose target is 200 bytes away; relax produces `JpAbs`.
- `relax::cross_bank_call_becomes_far_call` — caller in bank 1, callee in bank 2; relax rewrites to `CALL <thunk>` and creates a thunk in Bank 0. Asserts the rewritten CALL operand is the thunk's address and the thunk body encodes both the original target address and the callee bank. A separate stub-runtime test asserts the thunk enters `runtime.banking.far_call_entry`, not the callee directly.
- `relax::plain_cross_bank_call_is_rejected` — `CallReachability::NearOnly` from bank 1 to bank 2 returns `CrossBankBranchUnsupported`; only `AutoFar` may allocate a thunk.
- `relax::explicit_far_call_becomes_thunk_when_needed` — `Builder::far_call` / `CallReachability::AutoFar` from bank 1 to bank 2 rewrites to `CALL <thunk>` and materializes a per-target thunk.
- `relax::two_callsites_share_one_thunk` — two `CALL kernel.matvec` sites in different sections produce exactly one `runtime.banking.thunk.kernel.matvec` slot.
- `relax::distinct_targets_get_distinct_thunks` — `CALL foo` and `CALL bar` (both in callee bank 2) produce two distinct thunks, not one shared per-bank thunk.
- `relax::reaches_fixed_point` — randomized input over a generator; relax converges in ≤ 4 iterations with high probability.
- `relax::no_fixed_point_is_caught` — a synthetic adversarial input (we add via test-only API) returns `RelaxError::NoFixedPoint`, not infinite loop.
- `relax::same_bank_jr_stays_short` — within range, `JR` is preserved (no needless widening).
- `relax::cross_bank_jr_is_rejected` — a symbolic `JR` from bank 1 to bank 2 returns `CrossBankBranchUnsupported`, not `JP $4000`.
- `relax::cross_bank_jp_is_rejected` — a symbolic absolute branch from bank 1 to bank 2 returns `CrossBankBranchUnsupported` unless a future explicit `FarJump` operation is used.
- `relax::deterministic_thunk_naming` — two builds with the same inputs produce thunks with byte-equal symbol names.
- `relax::idempotent` — running relax on already-relaxed output returns the same `LayoutPlan` (no change).

## 8. Encoder (T-A1.7, `encoder.rs`)

### 8.1 Public surface

```rust
/// One section after encoding. `bytes.len() == placed.final_size`.
pub struct EncodedSection {
    pub id: SectionId,
    pub bytes: Vec<u8>,
    /// Byte spans for every ordered item that materialized bytes.
    pub item_spans: Vec<EncodedItemSpan>,
}

pub struct EncodedItemSpan {
    pub order: ItemOrder,
    pub kind: EncodedItemKind,
    pub offset: u16,
    pub len: u16,
}

pub enum EncodedItemKind {
    Instr,
    DataBlock,
    AlignmentPadding,
}

/// Encode one fully-lowered, fully-relaxed, fully-legalized section.
pub fn encode_section(
    section: &LegalizedSection,
    placed: &PlacedSection,
) -> Result<EncodedSection, EncodeError>;

/// Encode one concrete LR35902 instruction.
///
/// Symbol resolution and relative-offset range checks happen in `relax`.
/// By the time an `Instr` reaches this function, all operands are concrete.
pub fn encode_instr(
    instr: &Instr,
) -> Result<SmallVec<[u8; 4]>, EncodeError>;
```

(`SmallVec<[u8; 4]>` because no LR35902 instruction is wider than 3 bytes; 4 leaves room for one CB-prefix byte.)

### 8.2 Per-variant encoding

The encoder is one giant `match` over `Instr`. Reference: Pan Docs `gbdev.io/pandocs/CPU_Instruction_Set.html` and the GameBoy CPU manual. Highlights:

- `Nop` → `[0x00]`
- `Halt` → `[0x76]`
- `Nop` → `[0x00]`
  `Builder::safe_halt()` emits these as two adjacent instruction items when the author wants HALT-bug neutralization.
- `Stop` → `[0x10, 0x00]` (canonical two-byte form)
- `Ld8Reg { dst, src }` → `[0b01_DDD_SSS]` where `DDD/SSS` are three-bit register codes (B=000, C=001, D=010, E=011, H=100, L=101, A=111)
- `Ld8RegFromImm { dst, imm }` → `[0b00_DDD_110, imm]`
- `Ld8RegFromHl { dst }` → `[0b01_DDD_110]`
- `Ld8HlFromReg { src }` → `[0b01_110_SSS]`
- `Ld16Imm { dst: BC/DE/HL/SP, imm }` → `[0b00_PP_0001, lo, hi]` where PP = pair code
- `LdAFromDirect { addr }` → `[0xFA, lo, hi]` (note: `addr.get() < 0xFF00`, enforced by `DirectAddr` constructor)
- `LdAFromHighDirect { offset }` → `[0xF0, offset]` (LDH form)
- ... (full table in code)
- `JpAbs { cond: Some(NZ), addr }` → `[0xC2, lo, hi]`
- `JpAbs { cond: None, addr }` → `[0xC3, lo, hi]`
- `JrRel { cond: None, off }` → `[0x18, off as u8]`
- `Call { cond: None, addr }` → `[0xCD, lo, hi]`
- `Rst { vector }` → `[0xC7 | (vector_code << 3)]`
- CB-prefixed: `[0xCB, op]` where `op` encodes operation × target.

The encoder iterates the four typed arrays of `LegalizedSection` in `seq_index` order (cheap k-way merge over four streams). The match has exactly four arms — one per array — and no fall-through:

- `instrs[k]` → `encode_instr(&item.data)`. The listing records the CPU-visible address as `placed.cpu_start + offset`. It never adds `bank * BANK_SIZE`; that value is a ROM-file offset, not a CPU address.
- `data_blocks[k]` →
  - `DataBlock::Bytes(b)` → bytes pass through.
  - `DataBlock::Words(w)` → little-endian (LR35902 is LE) — `[w & 0xFF, w >> 8]` for each word.
- `alignments[k]` → padding bytes (0xFF) as decided by layout. The encoder reads the actual padding count from `placed.alignment_padding[&item.order]` and returns `EncodeError::MissingAlignmentPlan` if no entry exists.
- `labels[k]` → 0 bytes. Recorded by the listing's symbol cross-reference via the section's label array; `EncodedSection::item_spans` only carries spans for items that materialize bytes.

`pre_layout_ops`, `legalization_ops`, and `branches` **do not appear** in `LegalizedSection` — they are physically absent fields, not empty vectors. The encoder cannot reach them. There is no `EncodeError::OpNotLegalized` and no `EncodeError::UnresolvedBranch`.

### 8.3 `EncodeError`

```rust
pub enum EncodeError {
    EncodedLengthMismatch { expected: u8, actual: u8, instr: Instr },
    MissingAlignmentPlan { section_id: SectionId, order: ItemOrder },
    NonRomSectionEncoded { section_id: SectionId, space: AddressSpace },
}
```

`OpNotLegalized`, `UnresolvedSymbol`, and `RelativeOffsetOutOfRange` are gone from the encoder. They are eliminated structurally by `LegalizedSection` and operationally by `relax`.

### 8.4 Bit-stability

The encoder MUST produce identical bytes for identical inputs. Two specific risks:

- **Symbol resolution order**: `SymbolTable` uses `BTreeMap` (already in T-A1.2).
- **Iteration order across the SoA arrays**: items are merged by `seq_index` (a `u32` monotonically incremented at each push). The four arrays themselves preserve insertion order; the merge is total and deterministic.

Property test:

```rust
#[test]
fn encoder_bit_stable() {
    for _ in 0..100 {
        let s: LegalizedSection = curated_legalized_section();
        let placed = layout_one_legalized(&s);
        let a = encode_section(&s, &placed).unwrap();
        let b = encode_section(&s, &placed).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.item_spans, b.item_spans);
    }
}
```

### 8.5 Encoder ↔ `byte_len` round-trip

Every `Instr` variant declares its `byte_len()` (T-A1.1). The encoder MUST produce exactly that many bytes. Test:

```rust
#[test]
fn encoder_matches_byte_len() {
    for instr in all_canonical_instr_variants() {
        let bytes = encode_instr(&instr).unwrap();
        assert_eq!(bytes.len(), instr.byte_len() as usize, "{:?}", instr);
    }
}
```

This is the single most useful regression — any future variant that mis-declares its `byte_len()` fails this test loudly.

### 8.6 Hand-disassembly cross-check (the "Pan Docs spot check")

A targeted test asserts ~30 instructions byte-for-byte against the Pan Docs opcode table. URL pinned in the test docstring. Examples:

```rust
assert_eq!(enc(Instr::Nop), [0x00]);
assert_eq!(enc(Instr::Ld8Reg { dst: Reg8::A, src: Reg8::B }), [0x78]);
assert_eq!(enc(Instr::Ld8RegFromImm { dst: Reg8::A, imm: 0x42 }), [0x3E, 0x42]);
assert_eq!(enc(Instr::Ld16Imm { dst: Reg16Data::HL, imm: 0xCAFE }), [0x21, 0xFE, 0xCA]);
assert_eq!(enc(Instr::JpAbs { cond: None, addr: 0x0150 }), [0xC3, 0x50, 0x01]);
assert_eq!(enc(Instr::JrRel { cond: None, off: -2 }), [0x18, 0xFE]);  // tight loop
assert_eq!(enc(Instr::Call { cond: None, addr: 0x4000 }), [0xCD, 0x00, 0x40]);
assert_eq!(enc(Instr::Rst { vector: RstVector::V38 }), [0xFF]);
// CB-prefixed
assert_eq!(enc(Instr::Bit { bit: BitIndex::B7, target: CbTarget::Reg(Reg8::H) }), [0xCB, 0x7C]);
assert_eq!(enc(Instr::Swap { target: CbTarget::HlIndirect }), [0xCB, 0x36]);
// LDH forms
assert_eq!(enc(Instr::LdAFromHighDirect { offset: HighDirectOffset::new(0x44) }), [0xF0, 0x44]);
```

(30 entries minimum, distributed across families.)

## 9. ROM builder (T-A1.9, `rom.rs` — new file)

### 9.1 Public surface

```rust
pub struct CartridgeHeader {
    pub title: ArrayString<11>,                 // 11 ASCII bytes, padded with 0
    pub mbc_type: MbcType,                      // MBC5 / MBC5RAM / MBC5RAMBattery
    pub rom_size: RomSize,                      // 32K..=8M, power-of-two
    pub ram_size: RamSize,                      // None / 8K / 32K / 64K / 128K
    pub destination_code: DestinationCode,      // Japan / Overseas
    pub new_licensee_code: [u8; 2],             // 2 ASCII bytes; default "00"
    pub mask_rom_version: u8,
}

impl CartridgeHeader {
    /// Validates the DMG-only, modern-header layout used by F-A1:
    /// - title is at most 11 bytes and ASCII;
    /// - no interior NUL bytes in the provided title;
    /// - manufacturer code is fixed to `0000`;
    /// - CGB and SGB flags are fixed to `0x00`.
    pub fn validate(&self) -> Result<(), RomAssemblyError>;
}

pub enum MbcType { Mbc5, Mbc5Ram, Mbc5RamBattery }
pub enum RomSize { Kib32, Kib64, Kib128, Kib256, Kib512, Mib1, Mib2, Mib4, Mib8 }
pub enum RamSize { None, Kib8, Kib32, Kib64, Kib128 }

impl RamSize {
    pub const fn header_byte(self) -> u8 {
        match self {
            RamSize::None => 0x00,
            RamSize::Kib8 => 0x02,
            RamSize::Kib32 => 0x03,
            RamSize::Kib128 => 0x04,
            RamSize::Kib64 => 0x05,
        }
    }
}
pub enum DestinationCode { Japan, Overseas }

pub fn assemble_rom(
    encoded: &[(EncodedSection, PlacedSection)],
    layout: &LayoutPlan,
    header: &CartridgeHeader,
) -> Result<Vec<u8>, RomAssemblyError>;

fn build_header_section(header: &CartridgeHeader) -> Result<LegalizedSection, RomAssemblyError>;
```

### 9.2 The cartridge header (Pan Docs `gbdev.io/pandocs/The_Cartridge_Header.html`)

| Range          | Field                | Value source                                              |
|----------------|----------------------|-----------------------------------------------------------|
| `$0100-$0103`  | Entry point          | Internal typed header section emits `Instr::Nop; Instr::JpAbs { addr: 0x0150 }` |
| `$0104-$0133`  | Nintendo logo        | Hardcoded constant (48 bytes)                             |
| `$0134-$013E`  | Title                | `header.title`, padded with `0` to 11 bytes               |
| `$013F-$0142`  | Manufacturer code    | `0000` (we use the older 11-byte title format)            |
| `$0143`        | CGB flag             | `0x00` (DMG-only)                                         |
| `$0144-$0145`  | New licensee code    | `header.new_licensee_code`                                |
| `$0146`        | SGB flag             | `0x00`                                                    |
| `$0147`        | Cartridge type       | `0x19` (MBC5), `0x1A` (MBC5+RAM), `0x1B` (MBC5+RAM+BATT)  |
| `$0148`        | ROM size             | `0x00`..`0x08` per `RomSize`                              |
| `$0149`        | RAM size             | `0x00`..`0x05` per `RamSize`                              |
| `$014A`        | Destination          | `0x00` (JP) or `0x01` (overseas)                          |
| `$014B`        | Old licensee         | `0x33` (use new licensee)                                 |
| `$014C`        | Mask ROM version     | `header.mask_rom_version`                                 |
| `$014D`        | Header checksum      | `wrapping_sub` algorithm over `$0134..=$014C`             |
| `$014E-$014F`  | Global checksum      | Big-endian sum of all bytes except itself                 |

Header checksum algorithm (verbatim from Pan Docs):

```rust
let mut x: u8 = 0;
for byte in rom[0x0134..=0x014C].iter() {
    x = x.wrapping_sub(*byte).wrapping_sub(1);
}
rom[0x014D] = x;
```

Global checksum:

```rust
let mut s: u16 = 0;
for (i, b) in rom.iter().enumerate() {
    if i == 0x014E || i == 0x014F { continue; }
    s = s.wrapping_add(*b as u16);
}
rom[0x014E] = (s >> 8) as u8;       // big-endian
rom[0x014F] = (s & 0xFF) as u8;
```

The Nintendo logo bytes are the standard 48-byte sequence documented in Pan Docs. They are materialized as `DataBlock::Bytes` in the internal header section, not copied through a `Raw` escape hatch.

### 9.3 Bank packing

```rust
let bank_size = 16 * 1024;
let bank_count = header.rom_size.bank_count();   // 2..=512
let mut rom = vec![0xFF; bank_count as usize * bank_size];

for (encoded, placed) in encoded {
    let Some(abs) = placed.rom_file_offset()? else {
        return Err(RomAssemblyError::NonRomSection {
            id: placed.id,
            space: placed.space,
        });
    };
    rom[abs..abs + encoded.bytes.len()].copy_from_slice(&encoded.bytes);
}
```

Bank 0 CPU `$0000..=$3FFF` maps to ROM file offsets `0x0000..=0x3FFF`. ROMX bank `N >= 1` CPU `$4000..=$7FFF` maps to ROM file offsets `N * 0x4000 .. (N + 1) * 0x4000`, with file offset `N * 0x4000 + (cpu_addr - 0x4000)`. The `0xFF` fill is used for unused ROM regions.

### 9.4 `RomAssemblyError`

```rust
pub enum RomAssemblyError {
    UserHeaderSectionRejected { id: SectionId },
    SectionExceedsBankBoundary {
        id: SectionId,
        bank: BankIndex,
        cpu_start: u16,
        len: u32,
        end_exclusive: u32,
        bank_end_exclusive: u32,
    },
    BankIndexOutOfRange {
        id: SectionId,
        bank: BankIndex,
        max_valid_bank: u16,
    },
    InvalidTitle { reason: &'static str },
    InvalidLicenseeCode { code: [u8; 2] },
    InvalidRomSizeForLayout { requested_banks: u16, header_banks: u16 },
    NonRomSection { id: SectionId, space: AddressSpace },
}

// Implementation rule:
// `Builder::new` rejects `SectionRole::HeaderCartridge` unless called through
// an internal crate-private token owned by `rom::build_header_section`.
```

### 9.5 Why `gbf-asm` owns the ROM builder rather than `gbf-codegen`

The plan (line 1929) puts `EncodedRom` inside the Stage 12 backend. Why is the ROM builder here in `gbf-asm` instead?

- The Stage 12 backend lives in `gbf-codegen` (Epic B). `gbf-codegen` doesn't yet exist; F-A1 needs to ship a working `tiny_rom` example without it.
- The ROM builder is a thin transcription of cartridge format facts. It is pure code → bytes; there is no Stage 12 logic (placement, residency, banking decisions) inside it.
- The Stage 12 backend will *call* `rom::assemble_rom` after running its own placement passes. It does not duplicate the cartridge-header logic.

This is the same pattern as `encoder` — a deterministic byte-emitter that any compiler stage may call.

### 9.6 Tests

- `rom::header_checksum_known_vector` — encode a fixed header fixture and verify `$014D` against an independently computed expected byte using the documented `$0134..=$014C` wrapping-sub algorithm.
- `rom::global_checksum_round_trip` — sum the ROM (excluding `$014E-$014F`) and verify the stored bytes match.
- `rom::power_of_two_size` — for every `RomSize`, the output `Vec<u8>` length is a power of two ≥ 32 KiB.
- `rom::nintendo_logo_present` — bytes at `$0104-$0133` match the constant.
- `rom::ram_size_header_bytes` — asserts `Kib64 -> 0x05` and `Kib128 -> 0x04`.
- `rom::bank_n_at_correct_offset` — section in bank 3 appears at offset `3 * 0x4000`.
- `rom::unused_regions_are_ff` — every byte not covered by an `EncodedSection` (and not in the header) is `0xFF`.
- `rom::deterministic` — same inputs twice → byte-equal output.
- *Live boot validation moved.* The headless gameroy boot test (`tiny_rom_boots`, asserting PC reaches `$0150` within 100 frames with no fault) is owned by the follow-up `gbf-emu`/`gbf-debug` feature. F-A1 ships only structural and byte-stability tests for `rom`; see §1.2.

## 10. Listing and `.sym` (T-A1.8, `listing.rs` and `symbols::write_sym`)

### 10.1 `.lst` listing format

One logical record per ordered item view. Instructions fit on one physical
line. Data blocks and alignment padding may span multiple physical lines,
using deterministic continuation lines. Fixed-width columns are used for
stable diffs:

```
; section: kernel.matvec.tile_8x8 (CommonBank)
; bank=03 origin=$4000 size=0x012C
$4000  CD 34 40    ; call $4034              ; stage=Backend op=far_call_thunk
$4003  21 00 C0    ; ld   hl, $C000          ; stage=ArenaPlan op=load_input_tile
$4006  AF          ; xor  a                  ; stage=Backend
$4007  77          ; ld   (hl), a            ; stage=Backend
...
```

Instruction-line format:

```text
$<addr:04X>  <hex_bytes:9>  ; <mnemonic:24>  ; stage=<stage> op=<op?>[ note=<note>]
```

Hex bytes column is 8 chars (3 bytes max + spaces) plus 1 separator. Mnemonic column is left-aligned to 24 chars. Provenance suffix only present when `ListingOptions::show_provenance`.

Data-line format:

```text
$<addr:04X>  <hex_bytes:47>  ; db <N> bytes             ; stage=<stage> ...
```

Data and padding are chunked at 16 bytes per physical line. Continuation
lines reuse the same `ItemOrder` in the listing metadata but advance the
displayed CPU address.

### 10.2 Mnemonic formatting

A `format_instr(instr: &Instr, here: u16, symbols: &SymbolTable) -> String` function in `listing.rs` produces canonical mnemonics matching common LR35902 disassembler conventions:

```text
nop
ld   a, b
ld   a, $42
ld   hl, $C000
ld   (hl), a
ldh  a, ($44)             ; LdAFromHighDirect
jr   z, +5                ; relative
jr   nz, $4023            ; with symbol cross-reference if available
jp   $0150
call $4000
rst  $38
bit  7, h
swap (hl)
```

Lower-case mnemonics, comma-separated operands, addresses as `$XXXX`, immediates as `$XX` if hex-shaped or decimal otherwise. Symbol cross-references rendered as `; → runtime.scheduler.yield` after the mnemonic when applicable.

### 10.3 `ListingOptions`

```rust
pub struct ListingOptions {
    pub show_provenance: bool,           // default true
    pub show_cycle_costs: bool,          // adds `; cycles=N` to each line
    pub show_bytes: bool,                // default true
    pub include_section_header: bool,    // default true
    pub address_radix: AddressRadix,     // Hex (default) or Decimal
}

pub enum AddressRadix { Hex, Decimal }
```

### 10.4 `.sym` (RGBDS-compatible Game Boy format)

The standard line-oriented Game Boy `.sym` format — emitted from `gbf-asm`,
consumed by gameroy, by the in-tree `gbf-debug` agent CLI when forging a
session, and by other Game Boy emulators and debuggers — looks like:

```
00:0150 entry
00:0153 main
03:4000 kernel_matvec_tile_8x8
100:4000 expert_large_bank_entry
C000 wram_scratch
```

ROM symbols are emitted as `<bank_hex>:<addr_4hex> <symbol>` per line, with a minimum width of two hex digits but no maximum truncation (`00`, `03`, `100`, ...). Non-ROM address-space symbols are emitted in bankless form `<addr_4hex> <symbol>` unless `SymOptions::rom_only` is set. Lines are sorted by `(location_kind, bank, addr, symbol)`.

Dots are legal in the RGBDS-compatible `.sym` grammar, but names with two or more dots are implementation-defined in consumers. Therefore, when `dot_safe_separator` is enabled, the writer rewrites canonical dotted names using the injective escape below rather than relying on consumer-specific local-label behavior. **A naive `dot → underscore` substitution silently collides** — for example `foo.bar_baz` and `foo_bar.baz` both map to `foo_bar_baz` and the `.sym` file becomes ambiguous. The `.sym` writer therefore enforces collision-safety as a hard invariant:

1. **Pass 1** — apply an injective ASCII escape and prefix the result with `gbf_` so the output starts with a letter:

   ```text
   ASCII letter or digit -> unchanged
   '.'                   -> '_d'
   '_'                   -> '__'
   ```

   Examples:

   ```text
   kernel.matvec.tile_8x8 -> gbf_kernel_dmatvec_dtile__8x8
   foo.bar_baz            -> gbf_foo_dbar__baz
   foo_bar.baz            -> gbf_foo__bar_dbaz
   foo__bar               -> gbf_foo____bar
   ```

   This is injective over the current canonical alphabet because original underscores never produce a single-underscore escape introducer.
2. **Pass 2** — assert injectivity by inserting every rewritten name into a `BTreeSet<&str>` and returning `SymError::DotSafeNameCollision { rewritten, originals: [SymbolName, SymbolName] }` if any duplicate appears. `originals` carries both colliding inputs so the failure is debuggable without re-running the pipeline.

The escape function is the contract. The collision check remains as a hard regression gate even though the escape is designed to be injective.

```rust
pub fn write_sym(layout: &LayoutPlan, symbols: &SymbolTable, opts: &SymOptions)
    -> Result<String, SymError>;

pub struct SymOptions {
    pub include_externals_as_comments: bool, // unresolved externals have no address
    pub rom_only: bool,
    pub dot_safe_separator: bool,      // canonical injective escape: '.' → '_d', '_' → '__'
}

pub enum SymError {
    DotSafeNameCollision { rewritten: String, originals: [SymbolName; 2] },
    ExternalRequestedAsEntry { name: SymbolName },
}
```

### 10.5 Tests

- `listing::byte_stable` — encode + emit listing twice → byte-equal output.
- `listing::all_options_render` — toggling each option produces a strictly different output.
- `listing::provenance_visible` — `stage=Backend` appears for stage-Backend items.
- `listing::format_instr_canonical` — table-driven test of ~30 mnemonics matching expected strings.
- `listing::large_data_block_is_chunked_deterministically` — a 40-byte `DataBlock::Bytes` renders as three physical lines with stable continuation formatting.
- `listing::large_alignment_padding_is_chunked_deterministically` — large concrete padding renders without overflowing the instruction hex column.
- `symbols::write_sym_sorted` — `.sym` output is sorted by `(bank, addr)`.
- `symbols::write_sym_dot_safe` — under `dot_safe_separator`, no `.` appears in any symbol line; every emitted name begins with `gbf_`; `_` and `.` are escaped according to the injective table.
- `symbols::write_sym_dot_safe_escape_avoids_naive_collision` — passes a symbol set containing `foo.bar_baz` and `foo_bar.baz` and asserts the injective escape yields two distinct names. This test demonstrates why naive dot-to-underscore rewriting is forbidden.
- `symbols::write_sym_dot_safe_collision_detected` — uses a test-only lossy escape hook or deliberately duplicated rewritten names to assert the hard collision branch reports both originals. The production injective escape should not collide for canonical `SymbolName` values.
- `symbols::write_sym_dot_safe_collision_table_driven` — table-driven set of dotted helper-argument shapes (`a.b_c`, `a_b.c`, `x.y.z`, `x_y.z`, `x.y_z`, etc.) round-tripping through the rewrite without producing duplicate output strings.
- `listing::cycle_cost_shown` — `; cycles=4` appears for `JpAbs { cond: None }`.

## 11. Cross-cutting concerns

### 11.1 `gbf-hw` dependency status

`gbf-hw` is currently mostly stubbed (one-line `//! Module stub.` files). The ROM builder ideally pulls `MbcType` codes, `BANK_SIZE`, the Nintendo logo bytes, and timing constants from `gbf-hw`. Practical plan:

- Define hardware constants locally in `gbf-asm/src/rom.rs` for now (with a `// TODO(F-A2): move to gbf-hw::mbc5` annotation).
- When F-A2 (`bd-3sk`) populates `gbf-hw::mbc5`, `gbf-hw::memory`, and `gbf-hw::interrupts`, replace the local constants with re-exports.
- This is acceptable because: (a) `gbf-asm` already depends on `gbf-hw` (see `Cargo.toml`); (b) the constants are short enough to live in two places transiently; (c) F-A2 is in the M0 critical path and will land within the same milestone.

### 11.2 `gbf-kernel` dependency

`gbf-asm` depends on `gbf-kernel`. This is a forward-compatibility hook — `gbf-kernel` (Epic H) will define `KernelSpecId`, calling conventions, and AsmIR builders for matvec/residual/norm/etc. None of the F-A1 work consumes `gbf-kernel` types yet; the dependency is reserved.

### 11.3 `no_std + alloc` capability

Engineering rule 11: `gbf-asm` should be `no_std + alloc` capable where practical. Current state: the existing modules use `std::collections::BTreeMap`, `std::fmt`, `std::panic` (in `Builder::with_provenance`). The first two have direct `alloc::collections::BTreeMap` and `core::fmt` equivalents. `panic::catch_unwind` is `std`-only.

**Decision for F-A1**: target `std` for now. The `no_std` migration is a follow-up — beads issue to be filed after F-A1 closes. Rationale: the rule says "where practical"; right now we are not running `gbf-asm` on a constrained host, and the panic-safety scope guard in `Builder::with_provenance` is load-bearing. We can revisit when the host-side `gbf-cli` and bench code shake out their portability needs.

### 11.4 `serde` boundary

All public types derive `Serialize`/`Deserialize` (already done for shipped types). Constructor-validated newtypes use `#[serde(try_from = "...")]` so deserialization runs through the same gate (closure skill rule). New newtypes in this RFC (`PlacementProfile`, `BankIndex`, `MbcType`, `RomSize`, `RamSize`, `CycleCost`) will follow the same pattern. Negative deserialization tests are required for each.

**JSON map-key rule (asm-bead-closure skill)**: `serde_json` rejects non-string map keys at runtime, so any `BTreeMap<K, V>` in a serializable public type must use a `K` whose `Serialize` impl emits a string scalar. `BankIndex` is a structured enum. When used as a map key it is serialized via a stable canonical string — `"rom:0"`, `"rom:1"`, ..., `"sram:0"`, `"sram:1"`, ..., `"wram"`, `"hram"`, `"vram"`, `"oam"` — and parsed back through `TryFrom<&str>`. The string form is part of the artifact contract (matching `oracle-context` / report consumers); the in-memory `BankIndex` uses its native discriminator. `LayoutPlan::free_bytes_per_bank` is the first map keyed on `BankIndex`; future maps follow the same rule. A round-trip JSON test on `LayoutPlan` (see §16) is the gate — without it, a refactor that changes `BankIndex` could panic at serialization time in a downstream pipeline.

### 11.5 Error handling style

All public error types implement `Debug + Clone + PartialEq + Eq + Display + std::error::Error`. No `anyhow` / `thiserror` macros — explicit `impl Display` and `impl Error` keep error shape stable for serde and for the report layer (Epic F).

### 11.6 Performance targets

The constitution's §II.1 caps the test suite at 2 minutes. F-A1's tests must be in milliseconds:

- Cycle model: sub-microsecond per instruction.
- Encoder: O(n) over `Section.items()`; ~10 µs for a 256-item section.
- Layout: O(n log n) sort + linear assignment; ~100 µs for 100 sections.
- Relax: bounded iterations × O(n_branches); few hundred microseconds typical.
- ROM builder: O(rom_size); 32 KiB → ~10 µs.

Total `tiny_rom` end-to-end: < 5 ms. Fully within budget.

## 12. Implementation order

Within F-A1, the open tasks have a real DAG:

```
T-A1.5 cycle_model     ─── independent
T-A1.6 layout            ──┐
T-A1.6 relax              ─┼── T-A1.6 part 2 needs layout + instruction byte lengths
T-A1.7 encoder           ──┘
T-A1.8 listing             ─── needs encoder + cycle_model
T-A1.9 rom builder         ─── needs encoder + layout
```

**Recommended order:**

1. **T-A1.5 cycle_model.rs** (smallest, highest leverage). One day. Unblocks listing's cycle-cost option and Epic B's `ScheduleCostAnalysis`.
2. **T-A1.7 encoder.rs first cut** (sans structured ops; just `Instr` → bytes; takes `&LegalizedSection` so un-legalized ops are statically impossible — see §2.7.3 SoA layout). Two days. This unlocks listing, ROM assembly, and opcode regression tests. Layout/relax use `Instr::byte_len()` and sizing policies, not encoder byte emission.
3. **T-A1.6 layout.rs** (without relax). Two days.
4. **T-A1.6 relax.rs** (closes the loop — requires encoder + layout). Two days.
5. **lowering.rs** (`PreLayoutOpLowering`, `LegalizationOpLowering`, and stub lowerers). One day.
6. **T-A1.8 listing.rs**. One day.
7. **T-A1.9 rom.rs**. One day.
8. **`examples/tiny_rom.rs`** + structural and golden-byte integration tests (no live emulator boot — that ships with the follow-up `gbf-emu`/`gbf-debug` feature). One to two days.

**Total: ~10–12 days of focused work.** The critical path is encoder → layout → relax (5 days). Cycle model and listing parallelize.

PR shape: I propose three PRs to keep review tractable:

- **PR 1**: cycle_model + encoder (covers T-A1.5 and T-A1.7). The encoder PR is the largest single piece; pairing it with the small cycle_model keeps both in the same review context for the cycle-cost tests in listing.
- **PR 2**: layout + relax + lowering (covers T-A1.6 and the new lowering module).
- **PR 3**: listing + rom + tiny_rom example (covers T-A1.8, T-A1.9).

Each PR closes one or more child task beads.

## 13. Testing strategy summary

Aligned with constitution §III (shift left) and the closure skill checklists:

| Layer                              | Coverage                                                      |
|------------------------------------|---------------------------------------------------------------|
| Type-level (compile-time)          | `LayoutError`, `EncodeError`, `RelaxError`, `RomAssemblyError`, `LoweringError` are exhaustive enums. `NonZeroU8` rules out zero cycle costs. `DirectAddr` constructor rules out high-mem encoding through Direct. `PlacementProfile`, `BankIndex` newtype-bounded. |
| Unit / property                    | Per-variant encoder spot-checks (≥30). Cycle model spot-checks (≥30). Branch invariants. Round-trip `byte_len` ↔ encoder. Determinism (encode twice → byte-equal). Layout error variants individually. |
| Integration                        | `Builder` → layout → relax → encode → ROM round-trip on a curated 5-section program. Listing emits expected lines. `.sym` round-trips through `FromStr` + `Display` with a proptest invariant. |
| Snapshot                           | `tiny_rom.gb` byte-stable across runs (golden). `tiny_rom.lst` byte-stable. `tiny_rom.sym` byte-stable. |
| Emulator integration (deferred)    | Owned by the follow-up `gbf-emu`/`gbf-debug` feature: headless gameroy boot of `tiny_rom.gb`, PC-reaches-`$0150` assertion, no-fault-within-100-frames assertion. F-A1 closes without it. |
| Negative                           | Bad inputs to every public function fail with the typed error variant — no panics from public API except documented `expect()` paths in builder ergonomic methods (already shipped). |
| Skill checklist                    | Constructor-validated newtypes have negative deserialization tests. Builder mutation undone on caught panic (already shipped, regression-test added). Effect classifier doesn't collapse stack ops into pure compute (shipped). Symbol naming-collision tests (shipped). |

All tests run as part of the workspace pre-commit hook (`cargo test --workspace --all-features`). No tests gated behind environment variables; emulator-backed validation lives in the follow-up `gbf-emu`/`gbf-debug` feature and does not gate F-A1 closure.

## 14. Resolved questions

These were the questions I planned to surface in PR review. Each is now resolved; the decisions below are load-bearing for closure.

1. **`cycle_cost` returns M-cycles, not T-states.** LR35902 instruction timings are strictly divisible by 4 T-states, so M-cycles are a lossless representation that fits comfortably inside `NonZeroU8`. T-state conversion is exposed for downstream callers (e.g., `gbf-bench`) via `CycleCost::t_states(self) -> TStateCost`, defined as `4 * <m_cycles>` for `Fixed`, and `Branch { taken: 4*taken, not_taken: 4*not_taken }` for the conditional case. No precision is lost in either direction.

2. **Relax iteration cap is derived from the input.** The relax algorithm is monotone over a finite decision lattice: each relaxable branch can widen at most once, and each unique cross-bank call target can allocate at most one thunk. The hard cap is `1 + relaxable_branch_count + unique_cross_bank_call_target_count`. A `> 4` warning is still emitted as a `tracing::warn!` event because real programs should usually converge earlier.

3. **`PreLayoutOp` / `LegalizationOp` placement is correct.** The structural rule is: if the emitted byte sequence depends on the final assigned address/bank of the caller, callee, or a thunk slot, the op is a `LegalizationOp`; otherwise it is a `PreLayoutOp`. By that rule, `Yield`, `TraceProbe`, `BankLease`, `BankRelease`, and `AssertBank` are all `PreLayoutOp`s (they emit calls to fixed runtime symbols), and `FarCall` is the only `LegalizationOp` for F-A1 because it needs concrete `(caller_bank, callee_bank, target_addr)` to choose a thunk and bake the trampoline. Future placement-dependent ops F-A4 introduces follow the same rule.

4. **`BankIndex` lives in `gbf-asm` (specifically `section.rs` / `layout.rs`).** It is deeply tied to `PlacementProfile` and `LayoutPlan`, both assembly-layer concepts. Moving it into `gbf-foundation` now would widen the foundation API surface with assembly-specific layout ontology Epic B does not yet need. Epic B may promote it later when `gbf-codegen::PlacedRom` is real; until then the home stays here. Purely a refactor, no behavior change.

5. **Stub thunk naming `runtime.banking.thunk.<target_symbol>` is intentionally not part of any public ABI.** Per §7.4, F-A1 emits one thunk per cross-bank CALL target (forced by the 3-byte CALL operand limit; see Fix 1 / §0). This proves `gbf-asm` can perform the structural rewrite end-to-end. F-A4 owns the production naming, the production thunk shape, and the migration test. Stub deduplication is purely a layout-determinism property (two calls to the same target share a slot) and not an ABI contract.

6. **`.sym` ships in F-A1.** Both the `gbf-debug` agent CLI (when forging a session from a freshly built ROM) and the test/Epic-D harness read `.sym` to translate symbol names to addresses; deferring would push that integration cost into Epic D and the debugger feature needlessly. The implementation is ~50 LOC for the writer plus ~50 LOC for the matching `FromStr` parser, with a proptest enforcing `parse(write(x)) == x` over arbitrary symbol-table inputs. Collision-safety (Fix 3 in §10.4) is enforced by `SymError::DotSafeNameCollision`.

7. **CB-prefix encoding is verified exhaustively, not just spot-checked.** 256 CB-prefixed opcodes are generated by bitwise logic (`(op, bit, target) → u8`) — spot-checking ≤30 leaves 226 untested encodings where a typo could silently break an obscure bit-test instruction. F-A1 ships a programmatic test (`encoder::cb_exhaustive`, listed in §16) that generates all 256 CB encodings and asserts each matches a Pan-Docs-derived table. This is a hard blocker for closure, not a stretch goal.

8. **`tiny_rom` is option (a): empty boot + minimal observable state change.** Runs a small Bank-0 loop that reaches `$0150`, updates a WRAM/HRAM sentinel, and optionally performs a VRAM write only after either disabling LCD or waiting for VBlank. This demonstrates the full pipeline (layout, relax, encode, ROM build, header checksum, listing, `.sym`) without depending on F-A5's text renderer. Option (b) ("hello world" via tile fonts) waits for F-A5 to land a real text renderer; including a hand-rolled font now would add `Db` fixtures and a feature dependency without strengthening F-A1's closure surface.

## 15. Risks

| Risk                                                                | Likelihood | Mitigation                                                                                            |
|---------------------------------------------------------------------|------------|-------------------------------------------------------------------------------------------------------|
| Encoder bug produces wrong bytes for an obscure variant             | Medium     | Per-variant Pan Docs spot-checks + `byte_len` round-trip + CB-prefix exhaustive check.                |
| Relaxation oscillates (size grows, then a thunk insertion shrinks)  | Low        | Algorithm is provably monotone. `NoFixedPoint` safety net. Property test on randomized inputs.        |
| Stub lowering and F-A4 production lowering diverge in subtle ways   | Medium     | F-A4 closure must include cross-lowering parity test. Stub naming is internal-only.                   |
| ROM builder produces a `.gb` that fails to boot                     | Low        | Pan Docs-driven implementation + independently-verified header checksums + golden-byte snapshot of `tiny_rom.gb`. Live boot validation lands with the follow-up `gbf-emu`/`gbf-debug` feature (also targeting M0).   |
| `gbf-hw` constants (Nintendo logo, MBC5 codes) duplicated and drift | Low        | Local copies marked `// TODO(F-A2)`; F-A2 closure must replace them.                                  |
| Performance regression on larger sections                           | Low        | All passes are O(n) or O(n log n) in section count; encoder is O(n) in items. `cargo bench` if needed.|
| Determinism subtly broken by a future change                        | Medium     | Property test runs the full pipeline twice on every PR; failures will be loud.                        |

## 16. Claim-to-gate matrix (closure-style)

The closure skill (`.agents/skills/asm-bead-closure/SKILL.md`) requires this for non-trivial ASM beads. Pre-emptive matrix for F-A1 closure:

| Claim                                                                              | Gating test / artifact                                                                      |
|------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| Cycle costs match Pan Docs for the 30 spot-check instructions                       | `cycle_model::known_instructions`                                                           |
| No cycle cost is zero                                                               | `cycle_model::no_zero_cost` (compile-time `NonZeroU8` + runtime traverse)                   |
| Branch costs match their family-specific taken/not-taken timings                    | `cycle_model::conditional_branch_timings_by_family`                                          |
| `CycleCost::t_states()` is a lossless 4× projection                                 | `cycle_model::t_states_lossless`                                                             |
| Encoder produces correct bytes for every `Instr` variant                            | `encoder::known_opcodes` + `encoder::cb_exhaustive` + `encoder::byte_len_matches`            |
| Encoder is bit-stable                                                               | `encoder::bit_stable` (encodes the same `Section` 100 times, asserts byte-equal)             |
| Encoder cannot encounter un-legalized ops or unresolved branches                    | Eliminated structurally: `LegalizedSection` has no `pre_layout_ops`, `legalization_ops`, or `branches` field — `cargo check` is the gate. Pinned by `section::legalized_section_drops_unencoded_arrays_at_the_type_level` (asserts the JSON shape names none of the dropped arrays). |
| Encoder does not resolve symbols                                                    | `encoder::public_api_has_no_symbol_table_argument` plus `relax::all_symbolic_operands_are_concrete_after_legalization` |
| Runtime externals are resolved before final relax/encode                            | `link::unresolved_external_runtime_is_error_before_relax` |
| ROMX CPU addresses map to correct ROM file offsets                                  | `rom::romx_file_offset_subtracts_0x4000` — bank 3 CPU `$4000` writes at file offset `3 * 0x4000`, not `4 * 0x4000` |
| CPU-address boundary checks are not vulnerable to `u16` overflow                    | `layout::cpu_end_exclusive_uses_u32_arithmetic` |
| Cross-bank non-call branches are rejected                                            | `relax::cross_bank_jr_is_rejected` + `relax::cross_bank_jp_is_rejected` |
| Far-call thunk does not tail-jump directly to callee                                 | `relax::stub_far_call_thunk_enters_bank0_helper` |
| Structured ops are normal authoring effects                                          | `effect::structured_ops_are_normal` |
| Effect-kind count and names cannot drift silently                                    | `effect::machine_effect_kind_all_is_complete_and_unique` derives the count from `MachineEffectKind::ALL`, not from RFC prose |
| MBC5 reserved register range is forbidden even for privileged sections               | `effect::mbc5_reserved_register_range_forbidden` |
| Header ownership is unique                                                           | `rom::user_header_section_rejected` + `rom::internal_header_section_encoded_through_encoder` |
| Dot-safe symbol-name escape is injective over canonical symbols                      | `symbols::dot_safe_escape_is_injective_for_generated_cases` |
| Layout never crosses bank boundaries                                                | `layout::no_section_crosses_bank` (property test over generated inputs)                      |
| Encoder uses layout's concrete alignment decisions, not a second computation         | `encoder::alignment_padding_comes_from_layout_plan`                                           |
| Layout under `StrictOneExpertPerBank` does not pack ExpertBanks                     | `layout::strict_one_per_bank_semantics`                                                      |
| Relaxation upgrades out-of-range JR to JP                                           | `relax::out_of_range_jr_becomes_jp`                                                          |
| Relaxation rewrites cross-bank CALL via per-target thunk; original target address survives in the thunk body | `relax::cross_bank_call_becomes_far_call` (asserts CALL operand → thunk addr; thunk body encodes both `(target_addr, callee_bank)`) |
| Two call sites with the same target share one thunk                                 | `relax::two_callsites_share_one_thunk`                                                       |
| Two call sites with distinct targets in the same callee bank get distinct thunks    | `relax::distinct_targets_get_distinct_thunks`                                                |
| Relaxation reaches fixed point in finite iterations                                 | `relax::reaches_fixed_point` (randomized) + `relax::no_fixed_point_is_caught`                |
| Thunk materializer estimates are conservative and stable                            | `relax::thunk_estimate_is_at_least_materialized_size` + `relax::thunk_size_does_not_depend_on_own_address_unless_reported` |
| ROM builder produces a valid 32 KiB MBC5 cartridge                                  | `rom::header_checksum_pan_docs_example` + `rom::global_checksum_round_trip`                  |
| ROM is power-of-two                                                                 | `rom::power_of_two_size`                                                                     |
| Listing is byte-stable across runs                                                  | `listing::byte_stable`                                                                       |
| Listing reflects all `ListingOptions` flags                                         | `listing::all_options_render`                                                                |
| `.sym` is sorted and RGBDS-compatible                                                | `symbols::write_sym_sorted` + `symbols::write_sym_dot_safe`                                |
| `.sym` dot-safe rewrite is collision-free for dotted helper arguments                | `symbols::write_sym_dot_safe_collision_detected` + `symbols::write_sym_dot_safe_collision_table_driven` |
| `LayoutPlan` JSON serialization does not panic on `BankIndex` map keys              | `layout::layout_plan_json_round_trip` (encodes `LayoutPlan` with multi-bank `free_bytes_per_bank` to JSON via `serde_json::to_string`, asserts no panic, decodes back, asserts equality; covers `Rom(_)`, `Wram`, `Hram`, `Sram`, `Vram`, `Oam`) |
| `BankIndex` round-trips through its canonical string key form                       | `layout::bank_index_string_key_round_trip` — every `BankIndex` variant encodes to a stable string and parses back via `TryFrom<&str>`, with negative tests for malformed strings such as `rom:-1`, `rom:512`, bare `sram`, and `sram:16` |
| `Instr::Halt` remains the canonical one-byte instruction                            | `encoder::halt_is_one_byte` (asserts `[0x76]`) + `cycle_model::halt_one_mcycle` + `encoder::byte_len_matches` |
| HALT-bug neutralization is explicit and visible in provenance                       | `builder::safe_halt_emits_halt_then_nop` (asserts two ordered instruction items, `HALT` followed by `NOP`) |
| `db`/`dw` cannot smuggle hand-encoded privileged opcodes into executable sections   | `builder::db_dw_rejected_in_executable_sections` (every executable role rejects all four `db`/`dw` entry points, including a Privileged `CommonBank`; every data-only role accepts them) |
| `examples/tiny_rom` produces a structurally valid `.gb`                              | `cargo run -p gbf-asm --example tiny_rom` produces a 32 KiB file with valid Pan-Docs header/checksums and golden-byte snapshot equality; live boot validation under gameroy ships with the follow-up `gbf-emu`/`gbf-debug` feature |
| `tiny_rom` does not rely on illegal VRAM access timing                              | `tiny_rom::vram_write_guarded_by_lcd_off_or_vblank_wait`, or the demo uses WRAM/HRAM only    |
| Stub structured-op lowering is gated behind `cfg(feature = "stub-runtime")`         | grep test / `cargo check` without the feature succeeds and stub types are absent             |
| No new `unsafe` introduced                                                          | `grep -r "unsafe" gbf-asm/src/` has only the pre-existing zero hits                          |
| End-to-end determinism                                                              | `gbf-asm::tests::determinism::full_pipeline_byte_stable`                                     |

## 17. References

### Internal

- `history/planv0.md` — line 2408 (Assembly eDSL), line 1858 (Backend / EncodedRom), line 2893 (Engineering rules), line 1500 (AsmIR overview), line 121 (ISR residency rule), line 305 (gbf-runtime Bank0/banking authoring).
- `CONSTITUTION.md` — §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds).
- `.agents/skills/asm-bead-closure/SKILL.md` — closure-skill checklist.
- `bd-ssm` (F-A1 feature bead) and child tasks `bd-2k5`, `bd-c4s`, `bd-2o3`, `bd-pz3`, `bd-1gc`.
- Existing source: `gbf-asm/src/{isa,section,builder,effect,provenance,symbols}.rs`.

### External

- Pan Docs cartridge header: <https://gbdev.io/pandocs/The_Cartridge_Header.html>
- Pan Docs CPU instruction set: <https://gbdev.io/pandocs/CPU_Instruction_Set.html>
- Pan Docs MBC5: <https://gbdev.io/pandocs/MBC5.html>
- Pan Docs interrupt sources: <https://gbdev.io/pandocs/Interrupt_Sources.html>
- Pan Docs LR35902 instruction timing tables: <https://gbdev.io/pandocs/CPU_Instruction_Set.html#instruction-set>
- gameroy emulator (Rust-native backend, owned by the follow-up `gbf-emu`/`gbf-debug` feature): <https://github.com/Rodrigodd/gameroy>
- rquickjs (scripting host for `gbf-debug`): <https://github.com/delskayn/rquickjs>
- GameBoy CPU manual (gekkio): <https://gekkio.fi/files/gb-docs/gbctr.pdf>

## 18. Appendix: file-by-file change set

| File                                  | Change           | Lines (est.) |
|---------------------------------------|------------------|--------------|
| `gbf-asm/src/cycle_model.rs`          | New (replace stub) | ~250         |
| `gbf-asm/src/layout.rs`               | New (replace stub) | ~450         |
| `gbf-asm/src/relax.rs`                | New (replace stub) | ~350         |
| `gbf-asm/src/encoder.rs`              | New (replace stub) | ~700         |
| `gbf-asm/src/listing.rs`              | New (replace stub) | ~300         |
| `gbf-asm/src/lowering.rs`             | New module         | ~200         |
| `gbf-asm/src/rom.rs`                  | New module         | ~250         |
| `gbf-asm/src/symbols.rs`              | Add `write_sym`    | +80          |
| `gbf-asm/src/lib.rs`                  | Add `mod` lines    | +2           |
| `gbf-asm/examples/tiny_rom.rs`        | New                | ~150         |
| `gbf-asm/tests/determinism.rs`        | New (integration)  | ~150         |
| `gbf-asm/tests/tiny_rom_snapshot.rs`  | New (golden)       | ~80          |
| `gbf-asm/Cargo.toml`                  | Add `smallvec`, `arrayvec`/`arraystring` | +2 lines |

**Total: ~3000 LOC, ~70% of which is tests, fixtures, and table-driven encoding tables.**

## 19. End

This RFC stays inside the F-A1 boundary. Anything that requires F-A4's runtime ABI, F-A5's runtime nucleus, or Epic B's `ReachabilityValidation` is explicitly deferred. The proposal lets F-A1 close without those features existing, while leaving every seam (`PreLayoutOpLowering`, `LegalizationOpLowering`, `MachineEffect`, `PrivilegeClass`, `PlacementProfile`, `LoweredSection`, `LegalizedSection`) shaped for them to plug in cleanly.

Reviewer asks I would value most:

1. Are the **`PreLayoutOpLowering` / `LegalizationOpLowering` seams** at the right granularity? (The placement-dependence rule in §14 question 3 is the resolution; flag any structured op whose emitted bytes depend on placement that I have placed in the wrong phase.)
2. Are the **stub structured-op lowerers** worth shipping, or do they create more risk than they remove?
3. The **far-call thunk model is per-target, not per-callee-bank.** The 3-byte LR35902 `CALL` opcode carries a single 16-bit operand; if that operand were rewritten to a per-callee-bank trampoline, the original target address would be unrecoverable. The fix is one thunk per cross-bank target symbol (`runtime.banking.thunk.<target_symbol>`) with both the callee bank *and* the destination address baked into the thunk body — see §7.2/§7.4. This retires the per-callee-bank framing entirely; any earlier wording in this RFC suggesting per-bank thunks is a stale artifact and should be flagged. Reviewer ask: confirm that the per-target shape is the right F-A1 shape given the operand constraint, or propose a different mechanism (e.g., a wider call ABI that uses RST + bank register) that you believe F-A4 will want.
4. Should the **ROM builder live in `gbf-asm`** or move to `gbf-codegen` once that crate exists? My read: it's a thin transcription that any compiler stage can call, so it belongs here.
5. Anything in the **claim-to-gate matrix (§16)** missing for closure?
