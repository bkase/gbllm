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

`gbf-asm` is the only legal authoring layer for executable Game Boy code in the entire system. T-A1.1 through T-A1.4 are already in tree: typed `Instr`, `SectionRole`, `MachineEffect`, `PrivilegeClass`, `Builder`, `SymbolName`, `InstrProvenance`. What is missing is everything from a populated `Section` to a `.gb` file that boots in SameBoy.

Shared terminology for this RFC lives in `history/glossary.md`. The F-A1
terminology decisions are resolved there: `LoweredSection` is post-`PreLayoutOp`
lowering but not necessarily byte-ready; `LegalizedSection` is encoder-ready;
the structured-op split uses phase-named `PreLayoutOp` and `LegalizationOp`;
banking requests use a narrow `BankLeaseRequest` privilege; MBC5
`$6000..=$7FFF` is a reserved register window rather than `ModeSelect`; and
symbolic branches are durable section items, not symbolic operands inside
`Instr`.

This RFC proposes a five-piece pipeline that fills the remaining stubs (`cycle_model.rs`, `layout.rs`, `relax.rs`, `encoder.rs`, `listing.rs`) plus a new `rom.rs`. The pipeline runs in a strictly ordered, deterministic sequence:

```
Vec<Section>
   │
   ▼ PreLayoutOp lowering          (early, placement-independent)
   ▼ layout::layout_into_banks     (assigns sections to banks + start addresses)
   ▼ relax::relax_branches         (iterative-monotone JR→JP / thunk decisions)
   ▼ LegalizationOp lowering       (late, placement-dependent)
   ▼ encoder::encode_section       (the only Instr→bytes path)
   ▼ rom::assemble_rom             (cartridge header + bank packing → .gb)
   ▼ listing::emit_listing         (per-section human-readable .lst)
   ▼ symbols::write_sym            (BGB-compatible .sym)
```

The five new modules add roughly 1.6 KLOC of pure-function code with extensive tests. The ROM builder produces a 32 KiB MBC5 cartridge that boots in SameBoy and BGB.

The four most load-bearing decisions in this RFC are:

1. **Structured op lowering is split by phase.** `PreLayoutOp` covers placement-independent intents that lower before layout; `LegalizationOp` covers placement-dependent intents that lower during legalization after bank/address facts are known. The encoder sees neither.
2. **Layout uses optimistic-then-grow with a strictly monotone fixed point.** Each iteration can only widen instructions or insert thunks. Termination is mechanical.
3. **Far-call thunks live in Bank 0** as per-callee-bank trampolines (`runtime.banking.thunk.<bank_n>`); cross-bank `CALL <symbol@bank_n>` is rewritten to `CALL <thunk>` plus a runtime ABI handshake that F-A4 provides. The asm layer only knows about the symbolic thunk.
4. **The encoder is the *only* function that converts `Instr` to bytes.** Every other module (layout, relax, listing, ROM builder) reads bytes through the encoder. Bit-stability is asserted by a property test that encodes the same `Section` twice.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A bit-stable, deterministic encoder for every `Instr` variant defined by T-A1.1.
- A per-instruction M-cycle cost function with branch-taken/not-taken disambiguation, validated against Pan Docs for ≥30 instructions.
- A layout pass that assigns sections to banks under `PlacementProfile::{StrictOnePerBank, Budgeted, PackedExperts}` and emits a `LayoutPlan` with concrete per-section bank + start address.
- An iterative branch-relaxation pass that converges to a fixed point in finitely many steps, producing in-range `JR` / in-bank `CALL` everywhere; out-of-range `JR` becomes `JP`; cross-bank `CALL` becomes a far-call thunk.
- Structured op lowering with clear `PreLayoutOp` and `LegalizationOp` seams, plus stub implementations so gbf-asm has a working in-tree end-to-end demo.
- A ROM builder that produces a valid MBC5 `.gb` file with correct cartridge header and checksums.
- A `.lst` listing emitter and `.sym` symbol-table emitter, both byte-stable across runs.
- An `examples/tiny_rom.rs` that builds a 32 KiB ROM that boots in SameBoy and BGB and prints "hello" to the screen via a Bank-0-resident routine. (Stretch: see §10 risks.)

### 1.2 Non-goals (deferred)

- **Whole-program `ReachabilityValidation`** — Epic B Stage 12 sub-pass. F-A1 lays the typing groundwork (`MachineEffect`, `PrivilegeClass` already shipped); the analysis itself ships in `gbf-codegen`.
- **The real `BankLease`/`BankGuard` runtime ABI** — F-A4 (`bd-1sv`). gbf-asm exposes the seam but does not implement the production version.
- **Bank-switch coalescing, hotness-driven placement, residency optimization** — Epic B Stage 12 (`PlacedRom`).
- **Stage cache integration** — F-B15 (`bd-1g7k`). gbf-asm functions are pure and content-addressable, so this is purely additive.
- **Branch hot-path versus cold-path policy.** We provide `Branch { taken, not_taken }` as a structure; consumers choose which one to use.
- **Cycle calibration drift reports** — F-E5 (`bd-3beu`). The cycle model here is the *static* prediction; bench compares it against measurement.
- **CGB / GBC features.** DMG/MBC5 only. The cartridge header CGB flag is hardcoded to 0.

## 2. Background and existing state

### 2.1 What is already in tree (T-A1.1 — T-A1.4)

The following types are already implemented in `gbf-asm/src/`:

- **`isa.rs`** (~700 lines, bd-11e closed): `Instr`, `Reg8`, `Reg16`, `Reg16Pair`, `Reg16Data`, `Reg16Stack`, `Reg16Addr`, `AluSrc8`, `IncDec8Target`, `CbTarget`, `Operand8`, `Operand16`, `BitIndex`, `RstVector`, `DirectAddr`, `HighDirectOffset`, `Cond`. Every `Instr` variant has a `byte_len(self) -> u8` (constant for fixed-width variants, ALU-source-dependent for the eight-bit ALU family). 56 variants total.
- **`section.rs`** (~700 lines, bd-1e2 closed): `SectionRole`, `Section`, `SectionItem` (enum: `Label`, `Instr`, `Db`, `Dw`, `Align`, `PreLayoutOp`, `LegalizationOp`, `Raw`), `SectionPrivilege`, `PreLayoutOp` (`BankLease`, `BankRelease`, `Yield`, `TraceProbe`, `AssertBank`), `LegalizationOp` (`FarCall`), `BankLeaseSpec`, `LeaseId`, `MbcBankClass`, `YieldKind`, `TraceProbeId`, `ProbeLevel`. `Section::fixed_item_bytes()` returns `None` if any item has unknown width pre-relax.
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
- **Rule 10**: `Raw(Vec<u8>)` remains an escape hatch, never the default. → `Section::Raw` is `pub(crate)` already; the encoder treats `Raw` as opaque privileged effect (already encoded in T-A1.4).
- **Rule 12**: `unsafe` is forbidden by default. → none of the new modules require unsafe.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction)** — `LayoutError`, `EncodeError`, `RelaxError`, `RomAssemblyError` are exhaustive enums; failures are typed, not strings.
- **§III (shift left)** — every invariant pushes to the cheapest layer. Cycle costs are per-variant constants asserted at compile time; address overflow is rejected at layout; encoding correctness is a property test.
- **§IV.3 (reproducible builds)** — bit-stability is an asserted invariant of the encoder, ROM builder, listing, and symbol writer.
- **§V.3 (silence on success, loud on failure)** — every error type carries enough state for a host-side debugger to reproduce.

## 3. Architecture

### 3.1 Data flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                        Authoring (existing)                              │
│  Builder → Section { items: Vec<SectionItem>, role, priv, ... }          │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<Section>
┌──────────────────────────────────────────────────────────────────────────┐
│  PreLayoutOp lowering (new — §4)                                         │
│  Section { PreLayoutOp(Yield/Trace/etc.) } → LoweredSection              │
│  - Driven by `dyn PreLayoutOpLowering`                                   │
│  - LegalizationOp values remain in the section as late obligations       │
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
│   5. Loop until fixed point or `MAX_RELAX_ITERS`.                        │
│  - Errors: NoFixedPoint, ThunkOverflow                                   │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<LegalizedSection>, LayoutPlan (final), SymbolTable
┌──────────────────────────────────────────────────────────────────────────┐
│  encoder::encode_section    (T-A1.7, §8) — the only Instr→bytes path     │
│  (LegalizedSection, PlacedSection, &SymbolTable) → EncodedSection        │
│  - One match arm per `Instr` variant                                     │
│  - `Raw`, `Db`, `Dw`, and finalized padding bytes straight through        │
│  - `Label` consumes 0 bytes if retained for provenance/listing            │
│  - PreLayoutOp/LegalizationOp reaching this point is an internal error   │
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
| `relax`          | `RelaxAction`, `RelaxError`, fixed-point driver, `LegalizationOpLowering`, `LegalizedSection` | `layout`, `encoder` (for size queries), `isa` |
| `encoder`        | `EncodedSection`, `EncodeError`, `encode_instr`, `encode_section`   | `isa`, legalized output, `symbols`, `layout`      |
| `listing`        | `ListingOptions`, `emit_listing`, mnemonic formatter                | `encoder`, `cycle_model`, `provenance`            |
| `rom` (new)      | `CartridgeHeader`, `MbcType`, `RomAssemblyError`, `assemble_rom`    | `encoder`, `layout`, `gbf-hw::mbc5` (when present)|
| `symbols::sym`   | `.sym` writer (BGB format)                                          | `layout`, `symbols`                               |

The `lowering` module is new — see §4. It is the smallest possible expansion to the module set: `PreLayoutOp` / `LegalizationOp` lowering is conceptually distinct from both layout and encoding, and giving it its own home avoids two anti-patterns (encoder calling out to runtime lowering, layout knowing runtime ABI details).

### 3.3 Determinism contract

Every public function in `cycle_model`, `layout`, `relax`, `encoder`, `listing`, `rom`, and `symbols::write_sym` MUST be deterministic. Concretely:

- **No `HashMap` iteration**. Use `BTreeMap` and `BTreeSet` everywhere ordering is observable.
- **No `SystemTime`, no `rand`, no `std::env`, no thread-local state**.
- **No iterator non-determinism** (e.g., `par_iter` without a sort).
- **Stable section ordering**: layout receives `Vec<Section>` and processes in input order, modulo placement-profile-driven moves; placement-profile moves are themselves keyed by stable predicates (size, role, hash of name as tie-breaker).
- **Stable thunk numbering**: thunk symbols are `runtime.banking.thunk.<callee_bank_n>.<caller_bank_n>` — fully derivable from caller/callee identity, no global counter.

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

The current code now exposes this split directly as `SectionItem::PreLayoutOp`
and `SectionItem::LegalizationOp`. New design text should avoid using
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
    ) -> Result<LoweredFragment, LoweringError>;
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
    pub items: Vec<SectionItem>,                // Instr / Db / Dw / Label only
    pub thunk_requests: Vec<ThunkRequest>,
}

pub struct ThunkRequest {
    pub symbol: SymbolName,                     // canonical `runtime.banking.thunk.<bank>`
    pub callee_bank: BankIndex,
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
`LoweredFragment::items` MUST NOT contain another structured op for the site it
handled (no recursive lowering — keeps it linear).

### 4.4 `StubPreLayoutOpLowering` and `StubLegalizationOpLowering`

For F-A1 to ship without F-A4, `gbf-asm` provides stubs:

- `BankLease(spec)` → `CALL runtime.banking.acquire.<class>.<bank>` (placeholder symbol, gets resolved as a forward-declared external in tests/examples).
- `BankRelease(lease_id)` → `CALL runtime.banking.release.<lease_id>` (same).
- `Yield { kind }` → `CALL runtime.scheduler.yield.<kind>`.
- `TraceProbe { id, level }` → at `Trace` build profile: `CALL runtime.trace.emit.<id>`. Otherwise: zero items.
- `AssertBank { ... }` → debug builds only: 4 bytes of `LD A, (HRAM_BANK_SHADOW); CP n; JR NZ, panic` style. Release builds: zero items.
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
}

/// Pure function: cost is fully determined by the variant.
#[must_use]
pub const fn cycle_cost(instr: &Instr) -> CycleCost;
```

`NonZeroU8` rules out the silent zero-cost trap that the closure skill warns about. (Pan Docs has no zero-cost LR35902 instruction.)

The function is `const`. It compiles to a single jump table. It makes no allocation. It does not depend on layout, addresses, or any environment.

### 5.2 Per-variant table (M-cycles, from Pan Docs `gbdev.io/pandocs/CPU_Instruction_Set.html`)

The full table covers all 56 `Instr` variants. Highlights with their references:

| Family                        | Cost (M-cycles)         | Notes                                                  |
|-------------------------------|-------------------------|--------------------------------------------------------|
| `Nop`, `Halt`, `Stop`         | 1                       | Stop is 1 cycle but consumes 2 bytes (`10 00`).        |
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

`PreLayoutOp` and `LegalizationOp` values are lowered to `Instr` sequences before the cycle model is consulted. Therefore `cycle_cost` does not need a structured-op variant. **However**, Epic B's `ScheduleCostAnalysis` (F-B12) needs an estimate of structured-op cost *before* lowering (during `RangePlan` / `StoragePlan` reasoning, when the schedule is sketching cycles per slice). For that, we expose:

```rust
pub fn structured_op_cost_estimate(
    op: &StructuredOpRef<'_>,
    calibration: Option<&PlatformCalibrationBundle>,
) -> CycleEstimate;
```

with three confidence classes:

- `CycleEstimate::Static(N)` — the cost is the cost of the lowered sequence, deterministic.
- `CycleEstimate::CalibratedRange { low, high }` — needs a calibration bundle to be tight; falls back to a documented worst-case.
- `CycleEstimate::WorstCase(N)` — no calibration; very loose.

This is a hint, not the final cost. The final cost comes from running `cycle_cost` over the lowered fragment. The estimate API exists so Epic B can budget without fully lowering. (`CycleEstimate` itself lives in `gbf-foundation` so `gbf-codegen` can use it without a `gbf-asm` dependency for that subset of work.)

### 5.4 Tests (per T-A1.5 acceptance + skill checklist)

- `cycle_model::known_instructions` — 30 spot-check entries verified against Pan Docs by URL.
- `cycle_model::no_zero_cost` — every `Instr` returns `Fixed(_)` or `Branch { ... }` with `NonZeroU8`. (Compile-time guarantee via `NonZeroU8`, plus a runtime sanity assertion that iterates all variants.)
- `cycle_model::branch_invariant` — for every `Branch { taken, not_taken }`, `taken.get() >= not_taken.get()` and they differ by exactly 1 (the cycle to take the branch). Holds for all GB conditional ops; tested explicitly.
- `cycle_model::exhaustive_coverage` — proc-macro-free static check: a const test that constructs one example of each `Instr` variant and asserts `cycle_cost(...)` does not panic. (The function is `const`, so this is enforced at compile time — but we add the test anyway as a regression for any future variant added without updating the table.)
- `cycle_model::serde_roundtrip` — `CycleCost` serde stable.

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
    StrictOnePerBank,
    Budgeted { reserve_bytes_per_bank: u16 },
    PackedExperts,
}

pub struct PinnedPlacement {
    pub section_id: SectionId,
    pub bank: BankIndex,                        // u16; 0 for fixed
    pub start: u16,                             // address within bank
}

pub struct LayoutPlan {
    pub sections: Vec<PlacedSection>,
    pub thunk_pool: Vec<ThunkSlot>,
    pub bank_count: u16,
    pub free_bytes_per_bank: BTreeMap<BankIndex, u32>,
}

pub struct PlacedSection {
    pub id: SectionId,
    pub bank: BankIndex,
    pub start: u16,
    pub estimated_size: u16,                    // upper bound pre-relax
}

pub struct ThunkSlot {
    pub symbol: SymbolName,
    pub bank: BankIndex,                        // always 0 (Bank 0)
    pub start: u16,
    pub callee_bank: BankIndex,
}
```

### 6.2 Algorithm

1. **Categorize sections by role:**
   - `HeaderCartridge` → bank 0 at `$0000-$014F` (pinned). The Nintendo logo and header bytes go here as a `Raw`-only section assembled by the ROM builder.
   - `Bank0Nucleus` → bank 0 at `$0150` and onwards.
   - `CommonBank` → switchable banks 1..K.
   - `ExpertBank` → switchable banks K+1..N.
   - `WramHotArena`, `WramOverlay`, `HramFastFlags`, `SramPersistent`, `VramOwnedByUi`, `OamOwnedByUi` → not in ROM, but the layout pass still records their placement for the symbol table. Bank index is encoded as `BankIndex::Wram`, `BankIndex::Hram`, etc.

2. **Reserve a thunk pool slot at the high end of bank 0.** The pool size is computed as `lowering_pass.thunk_requests.iter().map(|r| r.estimated_size()).sum::<u32>()` plus a small slack (256 bytes default). `ThunkSlot::start` is set so the pool grows downward from `$3FFF`. If thunk pool overruns the available Bank 0 space, return `LayoutError::Bank0ThunkOverflow`.

3. **Profile-driven bank packing for `ExpertBank` / `CommonBank`:**
   - `StrictOnePerBank`: each ExpertBank section gets its own bank. CommonBank packs greedily (biggest first).
   - `Budgeted { reserve_bytes_per_bank }`: a section may use only `bank_size - reserve` of any bank; remainder is reserved for runtime patches.
   - `PackedExperts`: multiple ExpertBank sections may co-reside if they collectively fit. Greedy-first-fit.

4. **Within each bank, assign concrete `start` addresses:**
   - Process pinned placements first (they are non-negotiable).
   - Then place sections in deterministic order (sort by `(role, name)` so input order doesn't leak through).
   - Apply alignment directives (`SectionItem::Align`) by inserting padding `Db(0xFF)` items.
   - Track `cursor` per bank.

5. **Validate global invariants:**
   - No section crosses a bank boundary.
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

### 6.4 Section size estimation pre-relax

Each `SectionItem` returns a *known upper-bound* size:

| `SectionItem`      | Size estimate                                             |
|--------------------|-----------------------------------------------------------|
| `Label`            | 0                                                         |
| `Instr(I)`         | `I.byte_len()` (already exact and constant — see §isa)    |
| `Db(bytes)`        | `bytes.len()`                                             |
| `Dw(words)`        | `words.len() * 2`                                         |
| `Align(n)`         | up to `n - 1` padding bytes (worst case)                  |
| `PreLayoutOp(_)`   | should not exist after pre-layout lowering — internal error if seen |
| `LegalizationOp(_)` | known upper bound from legalization sizing policy         |
| `Raw(bytes)`       | `bytes.len()`                                             |

Initial estimate uses the conservative value for `Align`; layout assigns the *actual* padding count once positions are known. Relaxation deals with branch widening (see §7).

### 6.5 Tests

- `layout::no_section_crosses_bank` — random inputs, assert post-layout no `start + size > BANK_SIZE` for any placed section.
- `layout::strict_one_per_bank_semantics` — under `StrictOnePerBank`, distinct ExpertBank sections never share a bank.
- `layout::budgeted_reserve_respected` — `Budgeted { reserve_bytes_per_bank: 256 }` leaves at least 256 bytes free at the end of every used bank that contains an ExpertBank/CommonBank section.
- `layout::packed_experts_packs_smaller_into_one` — under `PackedExperts`, two 8 KiB experts share a 16 KiB bank.
- `layout::pinned_header_at_correct_offset` — header section pinned at `$0100`.
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
   - `JR off, target`: compute `delta = target_addr - (here_addr + 2)`. If `delta ∈ [-128, 127]`, `KeepRelative`. Else `UpgradeToAbsolute`. **Note**: `JP` has the same `byte_len` semantics as a 3-byte op, so size grows by 1.
   - `CALL addr, target`: if `target_bank == here_bank` or `target_bank == 0`, `KeepInBank`. Else `InsertFarCallThunk`. The CALL instruction now points to the thunk symbol in Bank 0.
4. **Apply changes:**
   - For each `UpgradeToAbsolute`: replace the `JrRel` `SectionItem` with `JpAbs`, increasing section size by 1 byte at that offset.
   - For each `InsertFarCallThunk`: rewrite the `Call.addr` to point at the corresponding `runtime.banking.thunk.<bank>` symbol. If that thunk doesn't exist yet in the pool, allocate it.
5. **Re-run layout** if any size changed or thunks were added.
6. **Loop** to step 2 until no changes (fixed point) or `MAX_RELAX_ITERS` (default 8) is exceeded.

### 7.3 Why monotone

Each iteration can only:

- Replace `JrRel` (2 bytes) with `JpAbs` (3 bytes) — +1 byte.
- Add a thunk slot — +N bytes in Bank 0.
- Insert padding for alignment — non-negative.

No iteration ever shrinks. Therefore the fixed point is reached when no branch needs upgrading, which is bounded by the initial number of branches × 1 (each branch can upgrade at most once). The MAX_RELAX_ITERS is a safety net; under the algorithm above, 2-3 iterations are enough for typical programs.

The closure skill explicitly notes: "Iterative-monotone branch relaxation: out-of-range JR -> JP, cross-bank -> far-call thunk. Convergence to fixed point asserted by test." That requirement is met.

### 7.4 Far-call thunk shape

A thunk for callee bank N looks like (this is the **stub** thunk; F-A4 will define the production version):

```text
runtime.banking.thunk.N:
    PUSH AF                        ; preserve flags
    LD   A, N % 256
    LDH  ($FF00 + ROMB_LOW), A     ; HRAM bank shadow + mirror to MBC5
    LD   A, (HL+)                  ; pop target address into HL via 16-bit fetch
    ; ...
    POP  AF
    RET
```

This is illustrative; the **actual thunk shape is owned by F-A4**. From the `gbf-asm` perspective, the thunk is a black box: layout reserves its size (estimated by F-A4's lowering function), the encoder encodes the instructions F-A4 emits, relax sees a thunk symbol with a known address.

For F-A1's stub lowering, the thunk is a 12-byte sequence that performs no real bank switch but sets up a `CALL` to a runtime-resolved label. Examples that need real bank-switching wait for F-A4.

### 7.5 Errors

```rust
pub enum RelaxError {
    NoFixedPoint { iters: u8 },
    ThunkPoolExhausted { requested: u32, capacity: u32 },
    UnresolvedSymbol { name: SymbolName, used_in: SectionId },
    InvalidRelativeOffset { offset: i32 },
}
```

`NoFixedPoint` should never fire under correct inputs but exists as a safety net so a bug in the size accounting is loud rather than silent.

### 7.6 Tests

- `relax::out_of_range_jr_becomes_jp` — emit a section with a `JR` whose target is 200 bytes away; relax produces `JpAbs`.
- `relax::cross_bank_call_becomes_far_call` — caller in bank 1, callee in bank 2; relax rewrites to `CALL <thunk>` and creates a thunk in Bank 0.
- `relax::reaches_fixed_point` — randomized input over a generator; relax converges in ≤ 4 iterations with high probability.
- `relax::no_fixed_point_is_caught` — a synthetic adversarial input (we add via test-only API) returns `RelaxError::NoFixedPoint`, not infinite loop.
- `relax::same_bank_jr_stays_short` — within range, `JR` is preserved (no needless widening).
- `relax::deterministic_thunk_naming` — two builds with the same inputs produce thunks with byte-equal symbol names.
- `relax::idempotent` — running relax on already-relaxed output returns the same `LayoutPlan` (no change).

## 8. Encoder (T-A1.7, `encoder.rs`)

### 8.1 Public surface

```rust
/// One section after encoding. `bytes.len() == placed.size`.
pub struct EncodedSection {
    pub id: SectionId,
    pub bytes: Vec<u8>,
    /// (instruction index in the source section, byte offset within `bytes`).
    pub instr_offsets: Vec<(usize, u16)>,
}

/// Encode one fully-lowered, fully-relaxed, fully-legalized section.
pub fn encode_section(
    section: &LegalizedSection,
    placed: &PlacedSection,
    symbols: &SymbolTable,
) -> Result<EncodedSection, EncodeError>;

/// Encode one instruction at a known address. The address is needed for `JrRel`
/// (relative offset) and any future PC-relative addressing.
pub fn encode_instr(
    instr: &Instr,
    here: u16,
    symbols: &SymbolTable,
) -> Result<SmallVec<[u8; 4]>, EncodeError>;
```

(`SmallVec<[u8; 4]>` because no LR35902 instruction is wider than 3 bytes; 4 leaves room for one CB-prefix byte.)

### 8.2 Per-variant encoding

The encoder is one giant `match` over `Instr`. Reference: Pan Docs `gbdev.io/pandocs/CPU_Instruction_Set.html` and the GameBoy CPU manual. Highlights:

- `Nop` → `[0x00]`
- `Halt` → `[0x76]`
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

For each `SectionItem` variant:

- `Instr(i)` → `encode_instr(i, here, symbols)`. `here` is the byte offset within the encoded section plus the section's `placed.start` plus `placed.bank * BANK_SIZE` (for cross-bank symbol math).
- `Db(bytes)` → bytes pass through.
- `Dw(words)` → little-endian (LR35902 is LE) — `[w & 0xFF, w >> 8]` for each word.
- `Align(n)` → padding bytes (0xFF) as decided by layout. The encoder reads the actual padding count from `placed.alignments[i]`, emitted by layout.
- `Label(_)` → 0 bytes. Records `(item_index, current_offset)` in `instr_offsets` for the listing's symbol cross-reference.
- `PreLayoutOp(_)` / `LegalizationOp(_)` → **internal error**. Structured ops must be lowered before encoding. Returning `EncodeError::OpNotLegalized`.
- `Raw(bytes)` → bytes pass through unchanged.

### 8.3 `EncodeError`

```rust
pub enum EncodeError {
    OpNotLegalized { section_id: SectionId, item_index: usize, kind: SystemCallKind },
    UnresolvedSymbol { name: SymbolName, used_in: SectionId },
    RelativeOffsetOutOfRange { from: u16, to: u16, offset: i32 },
    InvalidImmediate { reason: &'static str },
}
```

`RelativeOffsetOutOfRange` should be statically impossible after relax (relax rewrites all out-of-range JRs to JPs), but the encoder still validates and fails loudly if relax was bypassed.

### 8.4 Bit-stability

The encoder MUST produce identical bytes for identical inputs. Two specific risks:

- **Symbol resolution order**: `SymbolTable` uses `BTreeMap` (already in T-A1.2).
- **Iteration over `Vec<SectionItem>`**: the section's `items` vector preserves insertion order (T-A1.2 invariant). Encoder iterates left-to-right, no parallelism.

Property test:

```rust
#[test]
fn encoder_bit_stable() {
    for _ in 0..100 {
        let s = curated_section();
        let placed = layout_one(&s);
        let symbols = build_symbols(&[(&s, &placed)]);
        let a = encode_section(&s, &placed, &symbols).unwrap();
        let b = encode_section(&s, &placed, &symbols).unwrap();
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.instr_offsets, b.instr_offsets);
    }
}
```

### 8.5 Encoder ↔ `byte_len` round-trip

Every `Instr` variant declares its `byte_len()` (T-A1.1). The encoder MUST produce exactly that many bytes. Test:

```rust
#[test]
fn encoder_matches_byte_len() {
    for instr in all_canonical_instr_variants() {
        let bytes = encode_instr(&instr, 0, &SymbolTable::default()).unwrap();
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

pub enum MbcType { Mbc5, Mbc5Ram, Mbc5RamBattery }
pub enum RomSize { Kib32, Kib64, Kib128, Kib256, Kib512, Mib1, Mib2, Mib4, Mib8 }
pub enum RamSize { None, Kib8, Kib32, Kib64, Kib128 }
pub enum DestinationCode { Japan, Overseas }

pub fn assemble_rom(
    encoded: &[(EncodedSection, PlacedSection)],
    layout: &LayoutPlan,
    header: &CartridgeHeader,
) -> Result<Vec<u8>, RomAssemblyError>;
```

### 9.2 The cartridge header (Pan Docs `gbdev.io/pandocs/The_Cartridge_Header.html`)

| Range          | Field                | Value source                                              |
|----------------|----------------------|-----------------------------------------------------------|
| `$0100-$0103`  | Entry point          | `00 C3 50 01` — `nop; jp $0150`                           |
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

The Nintendo logo bytes are the standard 48-byte sequence `CE ED 66 66 ...` documented in Pan Docs. Hardcoded as a `const NINTENDO_LOGO: [u8; 48]`.

### 9.3 Bank packing

```rust
let bank_size = 16 * 1024;
let bank_count = header.rom_size.bank_count();   // 2..=512
let mut rom = vec![0xFF; bank_count as usize * bank_size];

for (encoded, placed) in encoded {
    let bank_offset = (placed.bank.0 as usize) * bank_size;
    let abs = bank_offset + (placed.start as usize);
    rom[abs..abs + encoded.bytes.len()].copy_from_slice(&encoded.bytes);
}
```

Bank 0 sections write to `0x0000..0x4000`; bank N writes to `N * 0x4000..(N+1) * 0x4000`. The 0xFF fill is the standard for unused ROM regions and matches what flash carts present.

### 9.4 `RomAssemblyError`

```rust
pub enum RomAssemblyError {
    SectionExceedsBankBoundary { id: SectionId, bank: BankIndex, end: u32 },
    BankIndexOutOfRange { id: SectionId, bank: BankIndex, max: u16 },
    InvalidTitle { reason: &'static str },
    InvalidLicenseeCode { code: [u8; 2] },
    InvalidRomSizeForLayout { requested_banks: u16, header_banks: u16 },
}
```

### 9.5 Why `gbf-asm` owns the ROM builder rather than `gbf-codegen`

The plan (line 1929) puts `EncodedRom` inside the Stage 12 backend. Why is the ROM builder here in `gbf-asm` instead?

- The Stage 12 backend lives in `gbf-codegen` (Epic B). `gbf-codegen` doesn't yet exist; F-A1 needs to ship a working `tiny_rom` example without it.
- The ROM builder is a thin transcription of cartridge format facts. It is pure code → bytes; there is no Stage 12 logic (placement, residency, banking decisions) inside it.
- The Stage 12 backend will *call* `rom::assemble_rom` after running its own placement passes. It does not duplicate the cartridge-header logic.

This is the same pattern as `encoder` — a deterministic byte-emitter that any compiler stage may call.

### 9.6 Tests

- `rom::header_checksum_pan_docs_example` — encode a known title and verify the byte at `$014D` matches the Pan Docs example.
- `rom::global_checksum_round_trip` — sum the ROM (excluding `$014E-$014F`) and verify the stored bytes match.
- `rom::power_of_two_size` — for every `RomSize`, the output `Vec<u8>` length is a power of two ≥ 32 KiB.
- `rom::nintendo_logo_present` — bytes at `$0104-$0133` match the constant.
- `rom::bank_n_at_correct_offset` — section in bank 3 appears at offset `3 * 0x4000`.
- `rom::unused_regions_are_ff` — every byte not covered by an `EncodedSection` (and not in the header) is `0xFF`.
- `rom::deterministic` — same inputs twice → byte-equal output.
- `rom::sameboy_boots` (integration, optional) — load the ROM into SameBoy in `--no-display --boot-rom-skip` mode and assert PC reaches `$0150` within 100 frames. Gated behind a `cfg(feature = "emulator-integration")` so CI without an emulator passes.

## 10. Listing and `.sym` (T-A1.8, `listing.rs` and `symbols::write_sym`)

### 10.1 `.lst` listing format

One line per `SectionItem`. Fixed-width columns for stable diffs:

```
; section: kernel.matvec.tile_8x8 (CommonBank)
; bank=03 origin=$4000 size=0x012C
$4000  CD 34 80    ; call $8034              ; stage=Backend op=far_call_thunk
$4003  21 00 C0    ; ld   hl, $C000          ; stage=ArenaPlan op=load_input_tile
$4006  AF          ; xor  a                  ; stage=Backend
$4007  77          ; ld   (hl), a            ; stage=Backend
...
```

Format:

```text
$<addr:04X>  <hex_bytes:9>  ; <mnemonic:24>  ; stage=<stage> op=<op?>[ note=<note>]
```

Hex bytes column is 8 chars (3 bytes max + spaces) plus 1 separator. Mnemonic column is left-aligned to 24 chars. Provenance suffix only present when `ListingOptions::show_provenance`.

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

### 10.4 `.sym` (BGB-compatible)

BGB and SameBoy both consume a simple `.sym` format:

```
00:0150 entry
00:0153 main
03:4000 kernel_matvec_tile_8x8
03:4012 kernel_matvec_tile_8x8.loop
```

`<bank:02X>:<addr:04X> <symbol>` per line, sorted by `(bank, addr)`. Symbols are emitted with `_` separators rather than dots (BGB chokes on dots in some versions). Internal canonical name `kernel.matvec.tile_8x8` becomes `kernel_matvec_tile_8x8` in the `.sym` file.

```rust
pub fn write_sym(layout: &LayoutPlan, symbols: &SymbolTable, opts: &SymOptions) -> String;

pub struct SymOptions {
    pub include_externals: bool,         // include unresolved external runtime symbols
    pub bgb_compat_separator: bool,      // dot → underscore
}
```

### 10.5 Tests

- `listing::byte_stable` — encode + emit listing twice → byte-equal output.
- `listing::all_options_render` — toggling each option produces a strictly different output.
- `listing::provenance_visible` — `stage=Backend` appears for stage-Backend items.
- `listing::format_instr_canonical` — table-driven test of ~30 mnemonics matching expected strings.
- `symbols::write_sym_sorted` — `.sym` output is sorted by `(bank, addr)`.
- `symbols::write_sym_bgb_compat` — under `bgb_compat_separator`, no `.` appears in any symbol line.
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
T-A1.6 relax              ─┼── T-A1.6 part 2 needs encoder for size queries
T-A1.7 encoder           ──┘
T-A1.8 listing             ─── needs encoder + cycle_model
T-A1.9 rom builder         ─── needs encoder + layout
```

**Recommended order:**

1. **T-A1.5 cycle_model.rs** (smallest, highest leverage). One day. Unblocks listing's cycle-cost option and Epic B's `ScheduleCostAnalysis`.
2. **T-A1.7 encoder.rs first cut** (sans structured ops; just `Instr` → bytes; emits `EncodeError::OpNotLegalized` for `PreLayoutOp` / `LegalizationOp`). Two days. This unlocks layout (which needs an encoder for size queries during relax), listing, and rom.
3. **T-A1.6 layout.rs** (without relax). Two days.
4. **T-A1.6 relax.rs** (closes the loop — requires encoder + layout). Two days.
5. **lowering.rs** (`PreLayoutOpLowering`, `LegalizationOpLowering`, and stub lowerers). One day.
6. **T-A1.8 listing.rs**. One day.
7. **T-A1.9 rom.rs**. One day.
8. **`examples/tiny_rom.rs`** + integration tests. One to two days.

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
| Integration                        | `Builder` → layout → relax → encode → ROM round-trip on a curated 5-section program. Listing emits expected lines. `.sym` parses with BGB-format conventions. |
| Snapshot                           | `tiny_rom.gb` byte-stable across runs (golden). `tiny_rom.lst` byte-stable. `tiny_rom.sym` byte-stable. |
| Emulator integration (optional)    | `cfg(feature = "emulator-integration")`: load `tiny_rom.gb` in SameBoy headless, assert PC reaches `$0150`, assert no fault within 100 frames. |
| Negative                           | Bad inputs to every public function fail with the typed error variant — no panics from public API except documented `expect()` paths in builder ergonomic methods (already shipped). |
| Skill checklist                    | Constructor-validated newtypes have negative deserialization tests. Builder mutation undone on caught panic (already shipped, regression-test added). Effect classifier doesn't collapse stack ops into pure compute (shipped). Symbol naming-collision tests (shipped). |

All tests run as part of the workspace pre-commit hook (`cargo test --workspace --all-features`). No tests gated behind environment variables except the optional emulator integration.

## 14. Open questions

These are the questions I would surface in PR review or to the oracle if needed.

1. **Should `cycle_cost` return T-states (4× M-cycles) instead of M-cycles?** Pan Docs uses both; the runtime scheduler thinks in M-cycles (cooperative slices); `gbf-bench` may want T-state precision later. **Proposal**: stay in M-cycles in the type; provide `CycleCost::t_states(self) -> u32` for callers that need T-states. Reversible decision.

2. **How tight is the `MAX_RELAX_ITERS` bound?** I claimed monotone convergence in 2–3 iterations for typical programs. Adversarial inputs (every JR exactly at the boundary) could push this. **Proposal**: default 8, log a warning at >4, return `RelaxError::NoFixedPoint` past 8. The 8 is large enough for any realistic program and small enough to fail fast on a bug.

3. **Are the current `PreLayoutOp` / `LegalizationOp` placements correct for production F-A4 lowering?** **Decision**: yes for F-A1. `Yield`, placement-independent `TraceProbe`, `BankLease`, `BankRelease`, and the current `AssertBank` are `PreLayoutOp`s; `FarCall` is a `LegalizationOp` because it needs final caller/callee bank placement. If F-A4/F-A5 later introduces a banking or assertion body whose emitted shape depends on caller bank, callee bank, final address, or thunk placement, that distinct variant belongs in `LegalizationOp`.

4. **Does `BankIndex` belong in `gbf-asm` or `gbf-foundation`?** Both `layout` and the eventual `gbf-codegen::PlacedRom` will use it. **Proposal**: define in `gbf-asm/src/section.rs` for now; promote to `gbf-foundation` if Epic B wants it. Purely a refactor, no behavior change.

5. **Is the stub structured-op lowering's symbol naming forward-compatible with F-A4?** I named stub thunks `runtime.banking.thunk.<bank_n>.<caller_bank_n>`. F-A4 may want a different scheme (e.g., per-thunk-policy variants). **Proposal**: the stub's names are not part of any public ABI. F-A4 owns the production naming and the migration test. The stub's only job is "boots tiny_rom and produces deterministic output."

6. **Should we skip `.sym` for now?** It's small but not strictly needed for M0 boot. **Decision**: ship it. The harness (Epic D F-D2) reads `.sym` to translate symbol names to addresses; deferring would push that integration cost into Epic D needlessly. The implementation is ~50 LOC.

7. **Coverage on the CB-prefix encoding tables.** There are 256 CB-prefixed opcodes (8 ops × 8 targets × 4 modes for some, plus BIT/RES/SET × 8 bits × 8 targets). Spot-checking 30 doesn't cover the full surface. **Proposal**: in addition to the spot-check, generate all CB encodings programmatically and verify each matches a `(op, bit, target) → u8` table derived from Pan Docs. This is one ~100-LOC test that exhaustively checks the CB family.

8. **What does the `tiny_rom` example actually do?** Two options:
   - **(a) Empty boot**: runs a 4-instruction loop in Bank 0 that reads joypad and toggles a tile in VRAM. Boots in any emulator, demonstrates the pipeline, no kernel work.
   - **(b) Hello world**: prints "hello" via VRAM tile writes from Bank 0. Requires a tile font (~80 bytes of `Db`).
   
   **Proposal**: ship (a) for F-A1; (b) waits for F-A5 (which provides a real text renderer). (a) demonstrates everything F-A1 needs to demonstrate without dependency on F-A5.

## 15. Risks

| Risk                                                                | Likelihood | Mitigation                                                                                            |
|---------------------------------------------------------------------|------------|-------------------------------------------------------------------------------------------------------|
| Encoder bug produces wrong bytes for an obscure variant             | Medium     | Per-variant Pan Docs spot-checks + `byte_len` round-trip + CB-prefix exhaustive check.                |
| Relaxation oscillates (size grows, then a thunk insertion shrinks)  | Low        | Algorithm is provably monotone. `NoFixedPoint` safety net. Property test on randomized inputs.        |
| Stub lowering and F-A4 production lowering diverge in subtle ways   | Medium     | F-A4 closure must include cross-lowering parity test. Stub naming is internal-only.                   |
| ROM builder produces a `.gb` that fails to boot                     | Low        | Pan Docs-driven implementation + emulator integration test. Header checksums independently verified.   |
| `gbf-hw` constants (Nintendo logo, MBC5 codes) duplicated and drift | Low        | Local copies marked `// TODO(F-A2)`; F-A2 closure must replace them.                                  |
| Performance regression on larger sections                           | Low        | All passes are O(n) or O(n log n) in section count; encoder is O(n) in items. `cargo bench` if needed.|
| Determinism subtly broken by a future change                        | Medium     | Property test runs the full pipeline twice on every PR; failures will be loud.                        |

## 16. Claim-to-gate matrix (closure-style)

The closure skill (`.agents/skills/asm-bead-closure/SKILL.md`) requires this for non-trivial ASM beads. Pre-emptive matrix for F-A1 closure:

| Claim                                                                              | Gating test / artifact                                                                      |
|------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| Cycle costs match Pan Docs for the 30 spot-check instructions                       | `cycle_model::known_instructions`                                                           |
| No cycle cost is zero                                                               | `cycle_model::no_zero_cost` (compile-time `NonZeroU8` + runtime traverse)                   |
| Branch costs satisfy `taken == not_taken + 1`                                       | `cycle_model::branch_invariant`                                                             |
| Encoder produces correct bytes for every `Instr` variant                            | `encoder::known_opcodes` + `encoder::cb_exhaustive` + `encoder::byte_len_matches`            |
| Encoder is bit-stable                                                               | `encoder::bit_stable` (encodes the same `Section` 100 times, asserts byte-equal)             |
| Encoding for non-legalized structured ops fails loudly                              | `encoder::op_not_legalized_returns_typed_error`                                              |
| Layout never crosses bank boundaries                                                | `layout::no_section_crosses_bank` (property test over generated inputs)                      |
| Layout under `StrictOnePerBank` does not pack ExpertBanks                           | `layout::strict_one_per_bank_semantics`                                                      |
| Relaxation upgrades out-of-range JR to JP                                           | `relax::out_of_range_jr_becomes_jp`                                                          |
| Relaxation rewrites cross-bank CALL via thunk                                       | `relax::cross_bank_call_becomes_far_call`                                                    |
| Relaxation reaches fixed point in finite iterations                                 | `relax::reaches_fixed_point` (randomized) + `relax::no_fixed_point_is_caught`                |
| ROM builder produces a valid 32 KiB MBC5 cartridge                                  | `rom::header_checksum_pan_docs_example` + `rom::global_checksum_round_trip`                  |
| ROM is power-of-two                                                                 | `rom::power_of_two_size`                                                                     |
| Listing is byte-stable across runs                                                  | `listing::byte_stable`                                                                       |
| Listing reflects all `ListingOptions` flags                                         | `listing::all_options_render`                                                                |
| `.sym` is sorted and BGB-compatible                                                 | `symbols::write_sym_sorted` + `symbols::write_sym_bgb_compat`                                |
| `examples/tiny_rom` produces a `.gb` that boots                                     | `cargo run -p gbf-asm --example tiny_rom` produces a 32 KiB file that loads in SameBoy       |
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
- SameBoy emulator (test target): <https://github.com/LIJI32/SameBoy>
- BGB emulator + `.sym` format: <https://bgb.bircd.org/manual.html>
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

1. Are the **`PreLayoutOpLowering` / `LegalizationOpLowering` seams** at the right granularity?
2. Are the **stub structured-op lowerers** worth shipping, or do they create more risk than they remove?
3. Is the **far-call thunk model** (per-callee-bank thunks in Bank 0) reasonable, or should we wait for F-A4 to define its own thunk strategy and have F-A1 ship without far-call legalization at all?
4. Should the **ROM builder live in `gbf-asm`** or move to `gbf-codegen` once that crate exists? My read: it's a thin transcription that any compiler stage can call, so it belongs here.
5. Anything in the **claim-to-gate matrix (§16)** missing for closure?
