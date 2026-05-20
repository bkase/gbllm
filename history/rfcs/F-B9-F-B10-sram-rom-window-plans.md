# RFC F-B9 + F-B10: Single-Window Residency Plans — `SramPagePlan` (Stage 7) and `RomWindowPlan` (Stage 8)

## -1. Authority and amendment policy

This RFC is the source of truth for F-B9 and F-B10 implementation.
`history/planv0.md` remains the architectural context document, but this RFC
is allowed to refine, narrow, or supersede `planv0.md` wherever this RFC makes
a more precise implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence
must be recorded in an `Amends planv0` note close to the relevant decision.
This is not a request to edit `planv0.md` immediately; it is a local
source-of-truth ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B9/F-B10 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a
  shared surface (canonical JSON rule, self-hash convention, diagnostic
  envelope, StageCache key construction, `ReportEnvelope` shape), the
  F-B2/F-B4 RFC wins. F-B9/F-B10 inherit those surfaces unchanged unless
  this RFC explicitly amends them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on `QuantGraph` or
  `GbInferIR` shape or canonical-product handling, the F-B3/F-B5 RFC wins.
* F-B6 (`ObservationPlan`), F-B7 (`RangePlan`), and F-B8 (`StoragePlan`)
  RFCs are consumed only through their pinned public types and reportable
  identities. If a forthcoming F-B6/F-B7/F-B8 RFC changes those public
  types, that RFC must explicitly amend this RFC.
* F-B11 (`OverlayPlan`), F-B12 (`ArenaPlan`), F-B13 (`GbSchedIR`), and
  F-B15 (`Backend`) consume the products defined here. If a later RFC
  changes `KernelResidency`, `LutResidency`, `RomWindowBinding`,
  `SramPageBinding`, `CommitBoundary`, `SpillPolicy`, or `ResidencyEpoch`
  in a way that affects this chunk, that RFC must explicitly amend this
  RFC.
* If a later RFC changes any public type, report shape, cache key,
  diagnostic code, or canonicalization rule introduced here, that later
  RFC must explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not
  prose folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft |
| Feature beads   | bd-3ns **F-B9 SramPagePlan (Stage 7)**; bd-15n **F-B10 RomWindowPlan (Stage 8)** |
| Open tasks      | To be minted: T-B9.1..T-B9.N (`SramWorkingSet` derivation, `SramPageBinding` assembly, `CommitBoundary` linearization, `SpillPolicy` resolution, page-switch projection, role/format predicates, `sram_plan.json` emitter, `certs/sram.cert.json` emitter, schema/round-trip tests, StageCache wiring); T-B10.1..T-B10.M (`SimultaneousVisibilitySet` analysis, `KernelResidency` resolution, `LutResidency` resolution, Bank 0 admissibility check, ISR-reachability gate, residency-epoch construction, bank-switch-per-token projection, `rom_window_plan.json` emitter, `certs/window.cert.json` emitter, schema/round-trip tests, StageCache wiring) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` lines 113–212 (target, region map, banks, regions sizes); 1665–1712 (Stage 6 `StoragePlan` upstream); 1712–1755 (Stage 7 `SramPagePlan` + Stage 8 `RomWindowPlan` + Stage 8.5 `OverlayPlan` preamble); 1755–1900 (Stages 9, 10 `GbSchedIR` downstream); 1989–2210 (runtime architecture, banking, persistence, persistent record protocol); 2061–2138 (memory plan); 2640–2870 (tests, reports, certificates) |
| Glossary        | `history/glossary.md` (Bank 0, common bank, expert bank, residency, kernel residency, WRAM overlay, ISR-reachable, BankLease/BankGuard, page state, commit group, persistent record) |
| Constitution    | §I correctness by construction; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ReportEnvelope`, `ValidationDiagnostic`, canonical JSON / self-hash, StageCache key construction); F-B3/F-B5 Canonical IRs (provides `QuantGraph`, `GbInferIR` consumed through Stage 6 `StoragePlan`); F-B8 StoragePlan (provides `StorageBinding`, `Materialization`, `LifetimeClass`, `AliasClassId`); F-B11 OverlayPlan (consumes `KernelResidency::WramOverlay` decisions); F-B12 ArenaPlan (consumes residency reservations and persistent-page geometry); F-B13 GbSchedIR + ResourceStateValidation (consumes `ResourceLeaseKind::SramPage` and `ResourceLeaseKind::RomWindow`); F-B14 ScheduleCostAnalysis (consumes projected switch counts as cost inputs); F-B15 Backend (consumes residency epochs for `PlacedRom` and reachability roots); F-B16 FeasibilityRefinementLoop (residency / switch-count failures may feed repair proposals); F-B17 StageCache integration sweep; F-A2 gbf-hw (memory-map predicates, MBC5 register set, target profile constants); F-A4 BankLease/BankGuard ABI (every residency epoch resolves through BankLease leases); F-A5 Bank0 runtime (Bank0 budget for runtime nucleus + far-call trampolines + Bank0Fixed kernels) |
| Sister deps     | F-B16 FeasibilityRefinementLoop (BLOCKED on oracle question) — both products feed it; F-C3 ScheduleOracle (consumes downstream `GbSchedIR`); F-F2 Certificates (consumes `certs/window.cert.json` and `certs/sram.cert.json`) |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B9 + F-B10 sits inside the
compiler-pipeline epic, where that epic sits inside the full project, and
which adjacent chunks' contracts this RFC inherits or honors.

### 0.1 Project at a glance — the eight epics

The gbllm project compiles a tiny language model into an LR35902 ROM that
runs on real Game Boy hardware. The work is split across eight epics
(`planv0.md` §"Workspace skeleton"):

```text
Epic A — M0 Foundation Stack
          gbf-asm, gbf-hw, gbf-abi, BankLease/BankGuard, Bank0 runtime,
          gbf-emu, gbf-debug, gbf-store. Provides the target/abi/asm
          contracts every other epic builds on. Status: substantially
          merged on main; F-A6 (gbf-store + StageCache) CLOSED.

Epic B — Compiler Pipeline (14 stages + refinement loop)        ← THIS EPIC
          The transform pipeline from frozen ArtifactCore +
          CompileRequest to a CompiledBuild (ROM + reports + certificates).
          Where most of M1–M3 lives.

Epic C — Oracle Stack
          DenotationalOracle (F-C1), ArtifactOracle (F-C2),
          ScheduleOracle (F-C3), ConformanceEnvelope (F-C4).

Epic D — Runtime Beyond M0
          Persistence, harness, trace, drift, fault, SchedulePack.

Epic E — Calibration & Bench
          gbf-bench production: cycle calibration, kernel timing, autotune.

Epic F — Reports & Verify
          gbf-report (build reports, certificates) + gbf-verify (independent
          slow reference implementations).

Epic G — Data, Lexical, Decode Pipeline
          gbf-data (corpus, charset, normalization, decode policy).

Epic H — Kernel
          gbf-kernel (KernelSpec + matvec/residual/norm/route/decode kernel
          implementations).
```

### 0.2 Epic B's anatomy — the 14-stage pipeline plus loop

Per `planv0.md` §"The compiler pipeline," Epic B has 14 numbered stages
bracketed by a **policy/feasibility envelope**, a **transform pipeline**,
and a **reporting envelope**, plus a bounded `FeasibilityRefinementLoop`
that wraps stages 5–11.

```text
Policy / feasibility envelope:
  F-B2  Stages 0, 0.5  ArtifactValidationAndUpgrade + ResolvedCompilePolicy
  F-B3  Stage 1        QuantGraph
  F-B4  Stage 2        StaticBudgetReport

Transformative stages (wrapped by FeasibilityRefinementLoop):
  F-B5  Stage 3        GbInferIR (value/effect IR)
  F-B6  Stage 4        ObservationPlan
  F-B7  Stage 5        RangePlan
  F-B8  Stage 6        StoragePlan ("the bridge")
  F-B9  Stage 7        SramPagePlan                                   ← THIS RFC
  F-B10 Stage 8        RomWindowPlan                                  ← THIS RFC
  F-B11 Stage 8.5      OverlayPlan
  F-B12 Stage 9        ArenaPlan
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation
  F-B14 Stage 11       ScheduleCostAnalysis
  F-B15 Stage 12       Backend (AsmIR + ReachabilityValidation +
                                PlacedRom + EncodedRom)

Cross-cutting:
  F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs
        (BLOCKED on oracle question)
  F-B17 StageCache integration sweep across all stages
        (uniformization pass; F-B9/F-B10 wire K7/K8 directly here)
```

Sequencing of ~weekly chunks:

```text
Chunk 1 (DONE):       F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2 (DONE):       F-B3 + F-B5         Stages 1, 3
Chunk 3 (DONE):       F-B6 + F-B7         Stages 4, 5
Chunk 4 (DONE):       F-B8                Stage 6
Chunk 5 (THIS RFC):   F-B9 + F-B10        Stages 7, 8
Chunk 6:              F-B11 + F-B12       Stages 8.5, 9
Chunk 7:              F-B13               Stages 10, 10.5
Chunk 8:              F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop
```

### 0.3 Where F-B9 and F-B10 sit in the pipeline

F-B9 and F-B10 are the **two single-window residency planners** that bracket
Stage 8.5's overlay-install schedule (F-B11) and pin every hardware-specific
visibility decision the compiler can make before byte ranges exist:

* **F-B9 (Stage 7) `SramPagePlan`** is the **persistent-state** residency
  planner. It owns the single 8 KiB switchable SRAM window at `$A000–$BFFF`
  and decides which `Materialization::Persist { page, commit_group }` and
  `Materialization::Materialize { class: SramPaged, .. }` bindings emitted
  by `StoragePlan` share an SRAM page, when page rotations may occur, what
  the cold-spill residency policy is, and how commit groups are linearized
  into atomic boundaries against the persistent record protocol.

* **F-B10 (Stage 8) `RomWindowPlan`** is the **code-and-LUT** residency
  planner. It owns the single 16 KiB switchable ROM window at `$4000–$7FFF`
  (Bank 0 is fixed at `$0000–$3FFF`) and decides for every hot operation
  which ROM objects must be simultaneously visible, which kernels lift to
  `KernelResidency::Bank0Fixed | WramOverlay | CoResidentSwitchable`, which
  LUTs lift to `LutResidency::Bank0Inline | WramStaged | RomCoResident`,
  and where the ISR-reachable subset of code/data must reside before
  `ReachabilityValidation` (F-B15) turns those declarations into a proof.

These two stages own the **single-window invariant**. Every subsequent
spatial / scheduling / reachability stage assumes:

* exactly one switchable ROM bank is visible at `$4000–$7FFF` at any time;
* exactly one SRAM page is visible at `$A000–$BFFF` at any time;
* the visible-bank set during any *hot operation* (a closed phrase pinned in
  §2.3) is a typed `RomVisibility` value resolved by F-B10 — not invented by
  later stages;
* the visible-page set during any commit group is a typed `SramVisibility`
  value resolved by F-B9 — not invented by later stages.

Stage 8.5 (`OverlayPlan`, F-B11) lifts F-B10's `KernelResidency::WramOverlay`
and `LutResidency::WramStaged` decisions into an explicit install/region
schedule. Stage 9 (`ArenaPlan`, F-B12) lifts F-B11's WRAM reservations and
F-B9's persistent-page geometry into concrete byte ranges.

### 0.4 Cross-epic interactions

F-B9 + F-B10 sit at the intersection of five epics:

```text
Epic A → Epic B
  - gbf-foundation (BlobRef, Hash256, RomBank, SramBank, AddrSpace)  consumed
  - gbf-store (StageCache) for K7 / K8 cache wiring                  consumed
  - gbf-hw (TargetProfile, MemoryMap, BankCount, MBC5RegisterSet)    consumed
  - gbf-abi (PersistKind, CommitGroupId, PageState, persistent ABI)  consumed
  - gbf-asm (SectionRole, KernelInstanceId tagging)                  consumed

Epic B (internal):
  - F-B2/F-B4 (ReportEnvelope, ValidationDiagnostic, StageCache)     consumed
  - F-B3/F-B5 (QuantGraph + GbInferIR identities)                    consumed
                                                                 (transitively)
  - F-B8 (StoragePlan products: StorageBinding, Materialization)     consumed
  - F-B11 (OverlayPlan)                                              feeds
  - F-B12 (ArenaPlan)                                                feeds
  - F-B13 (GbSchedIR + ResourceLeaseKind::{RomWindow,SramPage})      feeds
  - F-B14 (ScheduleCostAnalysis: bank/page-switch projections)       feeds
  - F-B15 (Backend: residency epochs, ISR-residency proof inputs)    feeds
  - F-B17 StageCache cross-cut                                       compatible

Epic C → Epic B (downstream):
  - F-C3 ScheduleOracle (consumes GbSchedIR transitively)            provided

Epic F → Epic B (consumer):
  - F-F2 Certificates (sram.cert.json + window.cert.json schemas)    provided

Epic G → Epic B (consumer of LUTs only via F-B5/F-B8):
  - DecodeSpec LUT residency hints flow through HintBundle           consumed
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk straddles M2 and M3:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup: runtime/banking/harness/emulator.
M1    (DONE)  F-B2/F-B4 (Stage 0/0.5/2) + F-B3 (Stage 1).
M1.5  (DONE)  F-B5 (Stage 3) + F-B6 (Stage 4) + F-B7 (Stage 5) + F-B8 (Stage 6).

M2    (in progress, this chunk delivers the spatial filter)
              One shared micro-kernel resolved by RomWindowPlan; one expert
              payload bank; emulator diffing against ScheduleOracle; first
              ReachabilityValidation pass.
              ↳ F-B10 (this chunk) delivers the spatial residency decision
                that "RomWindowPlan resolves" refers to: kernel residency,
                LUT residency, and the per-phase visibility set under the
                single-switchable-window rule.

M3            Top-1 router, expert dispatch table, value/effect GbInferIR +
              ObservationPlan + RangePlan + StoragePlan wired end-to-end for
              a routed FFN under the cooperative scheduler.
              ↳ F-B9 (this chunk) delivers the persistent-state spatial
                filter routed FFN inference relies on: SRAM page bindings,
                commit-group linearization, page-switch projections, and
                cold-spill residency. Without F-B9, runtime SRAM-page
                pressure is invisible to the budget refinement loop.

M4+           BoundedKv first, then LinearState, SchedulePack mode
              switching, persistence, drift, fault recovery.
              ↳ Out of scope for this chunk. F-B9's `SramWorkingSet` schema
                supports per-mode SRAM page bindings, but the M4 sequence
                state work proper lands later.
```

The two stages in this chunk are therefore the **spatial filter from
M1.5/M2 to M3**: F-B10 finishes the M2 commitment that RomWindowPlan must
exist, and F-B9 finishes the M3 commitment that per-token SRAM-page
switching is planned, projected, and bounded.

### 0.6 What the project as a whole gains when this chunk lands

```text
1. The single-window invariant is enforced.
   Before this chunk, the constraint "only one switchable ROM bank visible
   at a time, only one SRAM page visible at a time" lives as prose. After
   this chunk, every legal compiler state carries a typed visibility
   tuple; impossible co-residency is a Hard diagnostic.

2. RomWindowPlan resolves the architectural contradiction.
   You cannot have "shared micro-kernels in common banks" AND "expert-local
   data in expert banks" be the hot-path default simultaneously, because
   the hardware has only one switchable ROM window. F-B10's KernelResidency
   decision picks between Bank0Fixed, WramOverlay, and CoResidentSwitchable
   per kernel instance, per phase. The compiler refuses to leave this
   contradiction unresolved.

3. SRAM page pressure becomes first-class.
   Before F-B9, SRAM-page pressure was scattered across StoragePlan,
   persistence rules, and scheduling. After F-B9, page-switch counts per
   commit boundary are typed, projected, and reportable. The
   FeasibilityRefinementLoop can target SRAM-page pressure as a repair
   axis.

4. ISR-residency becomes a typed precondition.
   Bank 0 / HRAM / fixed-WRAM residency for ISR-reachable code/data is a
   hard rule (planv0.md line 121). F-B10 emits Bank0Fixed for every
   kernel/LUT whose reachability class is ISR-reachable, fault-path
   reachable, or yield-resume reachable. F-B15's ReachabilityValidation
   later proves the Bank0 residency claim; F-B10's job is to make sure
   every legal trace of the compiler has Bank0 residency declared, so the
   proof is non-vacuous.

5. ResourceLease keys become resolvable.
   F-B13's ResourceLeaseKind::RomWindow(RomWindowBinding) and
   ResourceLeaseKind::SramPage(SramPageBinding) cannot be defined without
   the binding types this chunk owns. After this chunk, every slice's
   required leases are typed and verifiable.

6. Bank-switch and page-switch budgets are projectable.
   The chunk emits projected_bank_switches_per_token and
   projected_sram_page_switches_per_token deterministic upper bounds
   under each ResolvedCompilePolicy. F-B14 (ScheduleCostAnalysis) and
   F-B16 (FeasibilityRefinementLoop) read those projections by hash.

7. The commit-group ↔ SRAM-page mapping is canonical.
   Persistent record protocol (planv0.md line 2138) defines
   CommitGroupId at the artifact level. F-B9 lowers it to a totally
   ordered sequence of CommitBoundary values consistent with single-page
   visibility.
```

### 0.7 Reading order for reviewers

A reviewer who has just read F-B8 and is approaching this RFC for the
first time should read:

```text
§0  (this section) — placement and dependencies
§0a TL;DR — the one-paragraph summary
§1  Project context — what the upstream chunks leave on the table
§2  Load-bearing decisions — single-window invariant, ISR rules, etc.
§5  Authority rules — what this RFC owns vs inherits
§6  Pipeline state machine — how Stage 7 and Stage 8 plug into Stage 6
§8  Stage 7 contract: SramPagePlan
§9  Stage 8 contract: RomWindowPlan
§10 Single-window invariant — formal proof obligations
§11 Report schemas (sram_plan.v1, rom_window_plan.v1) + certificates
§14 Cross-stage interactions
§15 Task DAG
§17 Proof obligations
§18 End-to-end theorem
§19 Final concise contract
```

Skim §3, §4, §7, §12, §13, §16 for specifics.

## 0a. TL;DR

This chunk lands the **two single-window residency planners** that bracket
Stage 8.5's overlay-install schedule and own every load-bearing
hardware-visibility decision the compiler can make before byte ranges
exist.

* **Stage 7 — `SramPagePlan`.** The single 8 KiB switchable SRAM window at
  `$A000–$BFFF` is a typed planning surface. F-B9 derives an `SramWorkingSet`
  per residency epoch, an `SramPageBinding` per `Materialization::Persist`
  / `Materialization::Materialize { class: SramPaged }` selected by
  `StoragePlan`, a totally-ordered `CommitBoundary` sequence consistent
  with single-page visibility, and a `SpillPolicy` that pins cold-spill
  residency. Outputs: `sram_plan.json` and `certs/sram.cert.json`.

* **Stage 8 — `RomWindowPlan`.** The single 16 KiB switchable ROM window at
  `$4000–$7FFF` (with Bank 0 fixed at `$0000–$3FFF`) is a typed planning
  surface. F-B10 computes the simultaneously visible ROM set per hot
  operation, picks `KernelResidency::{Bank0Fixed | WramOverlay |
  CoResidentSwitchable}` per kernel instance, picks `LutResidency::{
  Bank0Inline | WramStaged | RomCoResident}` per LUT instance, rejects
  impossible code/data placements before layout, and assigns each kernel
  instance to exactly one `ResidencyEpoch`. ISR-reachable code/data is
  forced to `Bank0Fixed` (the proof is owed by F-B15
  `ReachabilityValidation`; F-B10 makes the declaration non-vacuous).
  Outputs: `rom_window_plan.json` and `certs/window.cert.json`.

These two features are paired in one RFC because they share the
**single-window-residency** shape: each is a pre-arena residency planner
whose product is a typed binding map plus a per-epoch visibility tuple,
each runs against a single switchable hardware window, each emits a
canonical JSON report and a machine-checkable certificate, each is
consumed by hash by F-B11/F-B12/F-B13, and each shares the diagnostic
envelope, JSON canonicalization rule, self-hash convention, and
StageCache key construction inherited from F-B2/F-B4. Stage 8.5
(`OverlayPlan`, F-B11) sits between them in the *spatial* pipeline but
is owned by the next chunk's RFC and consumes residency products this
RFC defines.

The chunk closes only when:

1. `SramPagePlan` construction is a deterministic pure function of the
   F-B8 `StoragePlan` product, the F-B6 `ObservationPlan` product, the
   `ResolvedCompilePolicy`, and the `RuntimeChromeBudget`, and is
   byte-identical across two consecutive regenerations on a clean
   checkout.
2. `RomWindowPlan` construction is a deterministic pure function of the
   F-B9 `SramPagePlan` product, the F-B8 `StoragePlan` product, the F-B6
   `ObservationPlan` product, the `ResolvedCompilePolicy`,
   `TargetProfile`, and `RuntimeChromeBudget`, and is byte-identical
   across two consecutive regenerations on a clean checkout.
3. `sram_plan.json`, `certs/sram.cert.json`, `rom_window_plan.json`, and
   `certs/window.cert.json` round-trip through their semantic validators
   and self-hashes.
4. The single-window invariants (`I-RomSingleWindow`, `I-SramSinglePage`)
   are enforced as typed laws over every legal trace of the compiler;
   the proof obligations in §17 are discharged.
5. ISR-reachability gating produces a non-vacuous Bank 0 declaration:
   every kernel/LUT whose F-B6 reachability class is ISR-reachable,
   yield-resume-reachable, or fault-path-reachable resolves to
   `KernelResidency::Bank0Fixed` (or its LUT analogue).
6. `StageCache` keys for Stage 7 (K7) and Stage 8 (K8) are pinned and
   tested; cache-miss occurs on `pass_version`, schema, or feature-set
   drift; cache-hit replays byte-identical canonical product.
7. Synthetic fixtures cover every reject class in §16 and every
   `KernelResidency` × `LutResidency` × `SpillPolicy` decision-table
   row.

The chunk does **not** include:

* **Byte ranges.** F-B12 (`ArenaPlan`) owns concrete WRAM/SRAM/HRAM byte
  ranges; F-B15 owns ROM placement. F-B9 emits `PersistPageId` ↔
  page-binding maps and reservation totals; it does not say *where* in
  SRAM a page lives.
* **Overlay install schedules.** F-B11 (`OverlayPlan`) owns explicit
  install timing, region selection, share classes, and WRAM reservation
  geometry. F-B10 emits `KernelResidency::WramOverlay` only as a
  residency *decision*, plus a typed `WramOverlayDemand` summary the
  next chunk consumes.
* **Slice scheduling.** F-B13 (`GbSchedIR`) owns slices, lease
  acquisition, and resumable control flow. F-B9/F-B10 emit
  `ResidencyEpoch` boundaries the scheduler later turns into slice
  windows; epochs are *coarser* than slices.
* **Final placement.** F-B15 (`PlacedRom`) owns the bank assignment,
  symbol resolution, far-call thunk insertion, and reachability
  *proof*. F-B10 emits the *required* placement constraints; the
  proof is downstream.
* **Cycle costs.** F-B14 (`ScheduleCostAnalysis`) owns cycle envelopes.
  F-B9/F-B10 emit *integer* projections of bank-switch and page-switch
  counts per token under the resolved policy; cycles cost lookups are
  not done here.
* **Refinement loop application.** F-B16 (`FeasibilityRefinementLoop`)
  owns repair-proposal application. F-B9/F-B10 may emit Hard
  diagnostics whose repair path will be served by F-B16; this chunk
  does not call the loop.
* **`F-B16.RepairPolicy`/`CompileKnobs` extension.** Knob shape is
  named-only here; any new knob (e.g. `max_bank_switches_per_token_cap`)
  is defined in the F-B16 RFC and consumed here by typed reference.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What F-B2/F-B4/F-B3/F-B5/F-B6/F-B7/F-B8 leave on the table

By the time this chunk begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`,
  `TargetDataLoweringArtifact`, calibration, hint bundle, and
  `CompileRequest` are all admissible, hash-bound, and traceable through
  `artifact_validation.json` (F-B2).
* `ResolvedCompilePolicy` is the single answer to "what policy governed
  this build," with provenance for every load-bearing scalar (F-B2).
* `RuntimeChromeBudget` is honored at the static byte-math level; F-B4 has
  emitted `static_budget.json` against the real `QuantGraph`.
* `QuantGraph` (Stage 1, F-B3) is the canonical artifact graph: frozen
  canonical tensors, explicit quant formats, explicit `NormPlan`s,
  optional `RoutingTable`, explicit `ExpertSection`s, explicit
  `DecodeSpec`, explicit `SequenceSemanticsSpec`, complete provenance.
* `GbInferIR` (Stage 3, F-B5) is the value/effect IR with explicit
  effect edges (`SequenceState`, `Rng`, `FaultBoundary` as a reserved
  channel), storage-free, single-token convention, NodeAnchors stable
  across cache replays.
* `ObservationPlan` (Stage 4, F-B6) attaches `SemanticCheckpointId` and
  `TraceProbeId` references to GbInferIR `NodeAnchor`s and emits
  reachability classifications (`ReachabilityClass::IsrReachable`,
  `YieldResumeReachable`, `FaultPathReachable`, `HarnessEntryReachable`,
  `BankLeaseProtected`, `NormalOnly`).
* `RangePlan` (Stage 5, F-B7) chooses logical reduction structure;
  reduction sites carry typed `ReductionSiteId`s, and accumulator
  maxima are pinned as static integers.
* `StoragePlan` (Stage 6, F-B8) decides which values are recomputed,
  materialized (with `StorageClass` ∈ {`WramHot`, `HramHot`,
  `SramPaged`, `RomConst`} and `LifetimeClass` ∈ {`Slice`, `ResumeWindow`,
  `Token`, `Session`, `Persistent`}), and which are persisted (with
  `PersistPageId` and `CommitGroupId`). `AliasClassId` partitions
  potentially aliasing values.

This chunk is responsible for taking those `Materialization` decisions
and resolving them against the **physical single-window memory regions**:

* every `Materialization::Persist { page, commit_group }` resolves to
  exactly one `SramPageBinding` (F-B9), and every
  `Materialization::Materialize { class: SramPaged, .. }` resolves to
  exactly one `SramPageBinding` for some lifetime epoch (F-B9);
* every `Materialization::Materialize { class: RomConst, .. }` resolves
  to a `RomVisibility` slot under some `KernelResidency` /
  `LutResidency` decision (F-B10);
* every kernel instance produced by F-B6 / F-B7 / F-B8 (transitively
  from `GbInferIR` op classes) resolves to exactly one `KernelResidency`
  decision (F-B10);
* every `RomVisibility` set per phase satisfies `I-RomSingleWindow`
  (§10);
* every `SramVisibility` set per commit boundary satisfies
  `I-SramSinglePage` (§10);
* every ISR-reachable / yield-resume-reachable / fault-path-reachable
  kernel or LUT lifts to Bank 0 / HRAM / fixed-WRAM residency.

### 1.2 What M2/M3 commits to and how this chunk delivers it

Per `planv0.md` §"Milestones":

> **M2**: one shared micro-kernel resolved by `RomWindowPlan`, plus one
> expert payload bank, with exact emulator diffing against
> `ScheduleOracle` and checkpoint alignment against `ArtifactOracle` at
> `SemanticCheckpointId` boundaries; first `ReachabilityValidation` pass
> integrated into the backend.
> **M3**: top-1 router, expert dispatch table, value/effect `GbInferIR` +
> `ObservationPlan` + `RangePlan` + `StoragePlan` wired end-to-end for
> a routed FFN under the cooperative scheduler.

Mapping:

* M2 commitment "shared micro-kernel resolved by `RomWindowPlan`"
  requires F-B10. Without `RomWindowPlan`, the kernel residency is
  invented at scheduling time; the `KernelResidency` enum is
  declarative without a planner that resolves it.
* M2 commitment "first `ReachabilityValidation` pass integrated into
  the backend" requires F-B10's ISR-reachability gate to make
  `ReachabilityValidation` non-vacuous. Without the gate, every kernel
  could declare itself Bank-0-fixed and the proof is trivially true; the
  bug surfaces at runtime as bank-shadow drift across an interrupt
  boundary.
* M3 commitment "wired end-to-end for a routed FFN under the
  cooperative scheduler" requires F-B9. Without `SramPagePlan`, SRAM
  page switches per token are unbounded; the cooperative scheduler
  cannot honor `RuntimeChromeBudget.persist_bytes_per_token` or
  `max_sram_page_switches_per_token`.

Because M2 lands before M3, F-B10 is the M2-shaped half of this chunk
and F-B9 is the M3-shaped half. Sequencing inside the chunk (§15)
reflects that, but the *dependency edge* runs in the opposite direction:
F-B9 is a blocker for F-B10 because F-B10 must reason about persistent
data residency in SRAM banks (`SramPaged` storage class) before it can
finalize the ROM-side visibility set per phase. (Without knowing where
the persistent state pages live, F-B10 cannot prove that no ROM phase
demands a switchable bank that conflicts with an in-flight commit
group.)

### 1.3 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later spatial / scheduling / reachability stage receives a typed,
  validated `RomWindowPlan` (Stage 8 product) and `SramPagePlan` (Stage 7
  product). They never re-derive bank assignments, page assignments,
  kernel residency classes, LUT residency classes, ISR-residency
  declarations, or per-phase visibility sets.
* F-B11 (`OverlayPlan`) consumes `KernelResidency::WramOverlay` and
  `LutResidency::WramStaged` decisions and a `WramOverlayDemand`
  summary; it never invents WRAM residency.
* F-B12 (`ArenaPlan`) consumes (i) WRAM reservations from F-B11 and
  (ii) persistent-page geometry hints from F-B9; it never invents
  page geometry.
* F-B13 (`GbSchedIR`) consumes `ResourceLeaseKind::RomWindow(b)` and
  `ResourceLeaseKind::SramPage(b)` directly; the binding values are
  defined here.
* F-B14 (`ScheduleCostAnalysis`) consumes
  `projected_bank_switches_per_token` and
  `projected_sram_page_switches_per_token` as cost inputs.
* F-B15 (`Backend`) consumes `ResidencyEpoch`s as the source of
  required bank-assignment constraints; `ReachabilityValidation`
  consumes the ISR-residency declarations as a starting set whose
  proof obligation it then discharges.
* F-B16 (`FeasibilityRefinementLoop`) may consume Hard diagnostics
  produced by this chunk as repair targets; the named knobs (e.g.
  `max_bank_switches_per_token_cap`, `max_sram_page_switches_per_token_cap`,
  `kernel_overlay_admissible`) are referenced here and defined in F-B16.

This chunk's job is to retire the **single-window residency** preconditions
of the rest of the pipeline. Without it, every later stage either
re-derives them inconsistently or punts on them and the bug surfaces in
runtime as bank thrash, page thrash, or ISR-reachable-bank divergence.

### 1.4 Why this is two paired Features, not one feature or three

The natural unit is "the two single-window residency planners that
bracket Stage 8.5's overlay-install schedule."

* If we made it one feature, the bead would carry both the SRAM-page
  planner and the ROM-window planner. The implementation surface is
  large enough that PR review fragments. It would also force F-B12
  (`ArenaPlan`) to wait on the entire chunk before any spatial-byte
  work can start (F-B12 needs only F-B9's persistent-page geometry to
  begin reservation accounting, not F-B10's kernel-residency closure).
* If we made it three features (e.g. F-B9 SramPagePlan, F-B10
  RomWindowPlan, F-B10x ResidencyEpoch construction), we would split
  on epoch-vs-binding ownership. That split is artificial: the
  residency epoch is defined by the joint ROM/SRAM visibility tuple
  per phase; pulling it out of either planner would create a third
  consumer of `RomVisibility` / `SramVisibility` and re-converge during
  scheduling.
* Two features matches the natural seam: F-B9 owns persistent-state
  spatial residency (SRAM), F-B10 owns code/LUT spatial residency (ROM).
  They are paired in this RFC because they share an invariant
  (single-window-visibility), a report-shape rule, a certificate
  schema family, and a StageCache discipline, but ship as separate beads
  to keep PR scope tight and to let F-B9 land in M3 while F-B10 lands
  in M2.

### 1.5 What this chunk is NOT

The chunk is medium in *scope* but very large in *contract surface*. To
prevent scope creep, here is what this chunk explicitly is not:

* It is **not** a transform stage in the operational sense. F-B9 binds
  SRAM page identities to existing `Materialization::Persist` /
  `Materialization::Materialize { class: SramPaged }` bindings emitted
  by F-B8; it does not invent persistence. F-B10 binds kernels and LUTs
  to residency classes derived from `StoragePlan` and reachability
  classes from `ObservationPlan`; it does not invent kernels, LUTs, or
  reachability classes.
* It is **not** the producer of `StorageBinding` or `Materialization`.
  Those are F-B8 (StoragePlan) products, consumed here by hash.
* It is **not** the producer of `ReachabilityClass`. Those are F-B6
  (ObservationPlan) products, consumed here by hash. F-B10 reads the
  reachability class of every kernel and LUT and uses it to gate the
  Bank 0 declaration.
* It is **not** the proof of ISR-reachable-bank residency. F-B15
  `ReachabilityValidation` discharges the proof. F-B10 makes the
  proof's preconditions hold (Bank 0 declared for every reachable
  kernel/LUT) by construction.
* It is **not** an arena / byte-range allocator. F-B12 (`ArenaPlan`)
  owns concrete byte ranges. F-B9's product carries `PersistPageId`
  values (the artifact-stratum page identity from `Materialization::
  Persist { page, commit_group }`) and `SramPageBinding` values (a
  compiler-stratum identity for "this commit group rotates through
  these typed pages") but no byte addresses.
* It is **not** an overlay install schedule. F-B11 (`OverlayPlan`)
  owns the explicit install/region plan. F-B10's product carries
  `WramOverlayDemand` summaries (count of overlayable kernels, total
  WRAM byte demand, share-class hints) the next chunk consumes.
* It is **not** a slice-level scheduler. F-B13 (`GbSchedIR`) owns
  slices, lease acquisition, and resumable control flow. F-B9/F-B10
  emit `ResidencyEpoch` boundaries; epochs are coarser than slices.
* It is **not** a cycle-cost producer. F-B14
  (`ScheduleCostAnalysis`) owns cycles. F-B9/F-B10 emit *integer*
  bank-switch and page-switch counts per token; cycles arise later by
  multiplying with calibrated per-switch costs.
* It is **not** an epic-A bringup. The runtime nucleus, BankLease,
  Bank0Fixed kernel ABI, and persistent record protocol are all owned
  by Epic A and consumed by hash here.
* It is **not** a refinement loop. F-B16 owns repair-proposal
  application. F-B9/F-B10 may emit Hard diagnostics whose repair the
  loop may serve, but never call the loop and never propose repairs.

### 1.6 Relationship to F-B11 (`OverlayPlan`) and F-B12 (`ArenaPlan`)

`OverlayPlan` (F-B11) and `ArenaPlan` (F-B12) are both downstream
spatial planners that consume this chunk's products. The boundary
discipline:

```text
F-B10 RomWindowPlan
  emits:
    KernelResidency per kernel instance
    LutResidency per LUT instance
    ResidencyEpoch[ ] (coarse phase boundaries)
    WramOverlayDemand summary
    bank-switch counts per token (projected)

  does NOT emit:
    overlay regions
    overlay installs
    overlay share classes
    WRAM byte ranges

F-B11 OverlayPlan (next chunk)
  consumes:
    F-B10.WramOverlayDemand
    F-B6 ReachabilityClass (for install legality)
    F-B8 StorageBinding (for overlayable values' lifetime)

F-B9 SramPagePlan
  emits:
    SramWorkingSet[ ] (per epoch)
    SramPageBinding per persistent record / SramPaged binding
    CommitBoundary (totally ordered)
    SpillPolicy (one)
    page-switch counts per token (projected)

  does NOT emit:
    SRAM byte ranges
    persistent record header byte layout
    page rotation schedule by slice (F-B13)

F-B12 ArenaPlan (next chunk)
  consumes:
    F-B11 overlay regions and installs (WRAM)
    F-B9 SramPageBinding[] + persistent-page demand
    F-B11/F-B9 reservation totals
```

The clean boundary: **F-B9/F-B10 own *which* page or window is visible;
F-B11 owns *when* WRAM overlays are installed; F-B12 owns *where in
arenas* bytes live**.

## 2. Load-bearing decisions

### 2.1 Pure-function shape (core / driver split)

Both stages have **two layers**: a pure core constructor and a thin
driver that performs IO. The core is a pure function from typed pinned
inputs to typed content-addressed products. The driver wraps the core
with JSON emission, certificate emission, and StageCache writes.

```text
build_sram_page_plan_core(SramPagePlanInputs)
  -> Result<(SramPagePlan, ReportEnvelope<SramPagePlanReportBody>,
            CertEnvelope<SramCertBody>),
            PassDiagnostics>

run_stage7(SramPagePlanInputs, env)
  = build_sram_page_plan_core(...) then
    (on success or failure):
      emit sram_plan.json
      emit certs/sram.cert.json (success only)
      may write StageCache success entry
      may write StageCache failure memo

build_rom_window_plan_core(RomWindowPlanInputs)
  -> Result<(RomWindowPlan, ReportEnvelope<RomWindowPlanReportBody>,
            CertEnvelope<WindowCertBody>),
            PassDiagnostics>

run_stage8(RomWindowPlanInputs, env)
  = build_rom_window_plan_core(...) then
    (on success or failure):
      emit rom_window_plan.json
      emit certs/window.cert.json (success only)
      may write StageCache success entry
      may write StageCache failure memo
```

Cores never mutate `StoragePlan`, `ObservationPlan`, `RangePlan`,
`GbInferIR`, `QuantGraph`, `ResolvedCompilePolicy`, or
`RuntimeChromeBudget`. Drivers are the only IO surface. Determinism is
required, not aspirational.

The chunk-level pass shape is:

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed visibility derivations)
       (typed residency assignment)
       (typed switch-count projections)
       (typed proof-obligation discharge for §10/§17)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed plan product,
                        ReportEnvelope<ReportV1>,
                        CertEnvelope<CertV1> (success only) }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON for plan + cert
       writes StageCache success / failure memo
```

Every report includes `outcome: ReportOutcome` per F-B2/F-B4 §2.1.

### 2.2 Inheritance from F-B2/F-B4 and F-B3/F-B5

This RFC inherits, **unchanged**, the following from
`F-B2-F-B4-pipeline-entry-validation.md` and
`F-B3-F-B5-canonical-irs.md`:

* `ReportEnvelope<R>` shape and public JSON conventions — F-B2/F-B4 §4.
* `Hash256`, `DomainHash(...)`, `SelfHash(report)`, `ZERO_HASH` — F-B2/F-B4
  §1.
* `CanonicalJson(x)` rule (UTF-8, lex object keys, integers only, no
  NaN/Inf, no unknown fields, explicit enum tags, deterministic array
  ordering where order is not semantically meaningful) — F-B2/F-B4 §1.
* `null` policy (only for explicit semantic absence; never for unknown,
  unmeasured, or omitted) — F-B2/F-B4 §1.
* `R-Hash`, `R-Outcome-Pass`, `R-Outcome-Fail`, `R-FlatEnvelope`,
  `R-UnknownReject` envelope laws — F-B2/F-B4 §4.
* `ValidationDiagnostic` shape (`severity`, `origin`, `code`, `detail`,
  `provenance`) — F-B2/F-B4 §5. New origins and codes are introduced in
  §13 of this RFC; they extend the closed enum without modifying
  existing variants.
* `R-HardOnly-ThisChunk`: F-B9/F-B10 reports reject `Soft` diagnostics —
  F-B2/F-B4 §4.
* `D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`, `D-Provenance`
  diagnostic laws — F-B2/F-B4 §5.
* StageCache key construction rule
  `DomainHash(crate, "StageCacheKey", schema_id, schema_version,
  canonical_json_bytes)` — F-B2/F-B4 §11.
* `QuantGraph`, `GbInferIR` reportable identities (`quant_graph_self_hash`,
  `infer_ir_self_hash`) — F-B3/F-B5 §10. Consumed transitively as audit
  parents via F-B6/F-B7/F-B8 product identities.
* `NodeAnchorMap` discipline: every IR-anchored decision is keyed by
  `SemanticAnchor` (DomainHash-derived) so cache replays produce
  byte-identical bindings — F-B3/F-B5 §2.12.

If a later amendment to F-B2/F-B4 or F-B3/F-B5 changes any of the above,
that amendment must explicitly amend this RFC by name (see Authority
rules, §5).

This RFC adds the following to that surface:

* Two new `ValidationOrigin` variants: `SramPagePlanConstruction` and
  `RomWindowPlanConstruction`.
* Four new `ReportSchemaId` variants: `sram_plan.v1`, `sram_cert.v1`,
  `rom_window_plan.v1`, `window_cert.v1`.
* Two new product types: `SramPagePlan` and `RomWindowPlan`.
* Four new public report bodies: `SramPagePlanReportBody`,
  `SramCertBody`, `RomWindowPlanReportBody`, `WindowCertBody`.
* Two new `StageCacheKey` schemas: `K7 := SramPagePlanCacheKey`,
  `K8 := RomWindowPlanCacheKey`.

### 2.3 The single-window invariant — formal preliminaries

The two laws this chunk owns are:

```text
I-RomSingleWindow:
  ∀ phase φ ∈ ResidencyEpochs.
    |RomVisibility(φ) ∩ SwitchableBanks| ≤ 1

I-SramSinglePage:
  ∀ commit boundary β ∈ CommitBoundaries.
    |SramVisibility(β) ∩ SramPages| ≤ 1
```

Where:

* `SwitchableBanks ⊆ RomBank` is the set of banks that map into the
  switchable window `$4000–$7FFF`. Bank 0 is fixed and not in
  `SwitchableBanks`.
* `SramPages ⊆ SramBank` is the set of SRAM pages addressable through
  the switchable RAM window `$A000–$BFFF`. (MBC5 supports up to 16
  pages of 8 KiB each per `gbf-hw`.)
* "Phase" (`Phase`) is the closed concept defined in §2.4: a
  `ResidencyEpoch` plus a position-relative window of `GbInferIR` ops
  whose visibility set is constant.
* "Commit boundary" (`CommitBoundary`) is the closed concept defined
  in §2.6: a totally-ordered position in the schedule between
  consecutive `Materialization::Persist` write epochs at which a
  `CommitGroupId` becomes durable.
* `RomVisibility(φ)` and `SramVisibility(β)` are typed sets defined in
  §2.5 and §2.6.

The proofs that `I-RomSingleWindow` and `I-SramSinglePage` hold across
every legal trace of the compiler are §17.PO-W1 and §17.PO-S1.

A "hot operation" (an informal phrase common in plan prose) is pinned
here as: **any `GbInferIR` node whose `StorageBinding` for any input or
output is `Materialization::Materialize { class: RomConst, .. }`,
`Materialization::Materialize { class: SramPaged, .. }`, or
`Materialization::Persist { .. }`, and whose enclosing kernel instance
is in the canonical operational set (i.e. not pure recompute and not
purely WRAM-resident).** Hot operations dominate bank-switch and
page-switch frequency; they are precisely the operations whose
visibility set drives `I-RomSingleWindow` and `I-SramSinglePage`.

Amends planv0: planv0.md uses "hot operation" informally throughout.
This RFC pins the term to exactly the operations that participate in
the single-window invariants and emits a typed predicate
`is_hot_operation(node, storage_plan) -> bool` in
`gbf-codegen::stages::window`.

### 2.4 Phase, ResidencyEpoch, and the visibility tuple

A `ResidencyEpoch` (planv0.md line 1832) is a contiguous range of
operations in `GbInferIR`'s canonical order during which the visibility
tuple `(rom_window, sram_page, overlay)` is constant. F-B9/F-B10 emit
`ResidencyEpoch` boundaries (epoch ids and ordered op-anchor ranges)
but **not** the slice-level decomposition inside an epoch (that is
F-B13).

```rust
pub struct EpochId(u32);

pub struct ResidencyEpochSummary {
    pub id: EpochId,
    pub op_range: NodeAnchorRange,      // [first, last_inclusive]
    pub rom_window: RomWindowBinding,   // §9.3
    pub sram_page: SramPageBinding,     // §8.3
    pub overlay_demand: WramOverlayDemand,
}
```

`NodeAnchorRange` is a typed pair of `SemanticAnchor` values defined in
F-B3/F-B5 §2.12, ordered by canonical NodeId order. The op range
identifies which `GbInferIR` operations belong to the epoch.

Within an epoch, all operations see exactly one `RomWindowBinding`
(possibly Bank 0 only) and exactly one `SramPageBinding` (possibly
"no SRAM page bound this epoch"). Across epoch boundaries,
`RomWindowBinding` may change (a bank-switch occurs) and
`SramPageBinding` may change (a page-switch occurs). The number of
epoch-boundary bank switches and page switches is the source of the
`projected_bank_switches_per_token` and
`projected_sram_page_switches_per_token` projections in the report.

`RuntimeMode` is an existing concept (planv0.md line 1840) but is
**out of scope** for this chunk: a single `SchedulePack` may carry
multiple `RuntimeMode`-keyed `GbSchedIR`s, but the F-B9/F-B10 product
is *one* per `(RuntimeMode, BuildIdentity)`. F-B13 wires the
mode-keyed `SchedulePack` over the per-mode F-B9/F-B10 products.

### 2.5 RomVisibility and the Bank 0 / switchable distinction

```rust
pub enum RomBankClass {
    Bank0Fixed,                         // bank 0 at $0000–$3FFF
    Switchable { bank: RomBank },       // some bank K at $4000–$7FFF
}

pub struct RomVisibility {
    pub bank0: Bank0VisibilityFlag,     // always Visible
    pub switchable: Option<RomBank>,    // None means no switchable bank
                                        // mapped this phase
}

pub enum Bank0VisibilityFlag {
    Visible,
}
```

Because bank 0 is fixed, every phase has bank 0 visible. The
*switchable* slot is `None` only when no hot operation in the phase
needs a switchable-bank object (rare but legal when a phase runs
purely from Bank 0 / WRAM / HRAM / shared LUTs that fit in Bank 0
and the kernel is `Bank0Fixed`).

The single-window invariant `I-RomSingleWindow` reduces to:

```text
∀ phase φ.
  (RomVisibility(φ).switchable = Some(b₁)
    ∧ RomVisibility(φ).switchable = Some(b₂))
  ⇒ b₁ = b₂
```

i.e. at most one switchable bank per phase. The constructive form: the
type itself (`Option<RomBank>`) makes more than one switchable bank
ill-typed. F-B10's job is to enumerate the simultaneously-required
switchable-bank set per phase and reject if cardinality exceeds 1.

```rust
pub struct PhaseSwitchableDemand {
    pub phase: EpochId,
    pub demanded_banks: NonEmptySet<RomBank>,
}
```

If `PhaseSwitchableDemand.demanded_banks.len() > 1` for any phase,
`RomMultipleSwitchableBanksDemandedInPhase` is emitted (Hard) and the
plan fails. The repair surface (F-B16) may suggest cloning a kernel
between Bank 0 and an overlay, splitting the phase, duplicating tiny
LUTs across banks, or relaxing co-residency hints.

### 2.6 SramVisibility, CommitBoundary, and the persistent record protocol

```rust
pub struct SramVisibility {
    pub page: Option<SramPage>,         // None means no SRAM mapped
}

pub struct SramPage {
    pub bank: SramBank,                 // 0..=15 on MBC5
}

pub struct CommitBoundary {
    pub id: CommitBoundaryId,
    pub before_epoch: EpochId,
    pub after_epoch: EpochId,
    pub commit_group: CommitGroupId,    // from StoragePlan
    pub generation_delta: u32,          // monotone within commit_group
    pub durability_class: DurabilityClass, // from gbf-abi
}

pub struct CommitBoundaryId(u32);
```

A `CommitBoundary` is the canonical location at which a
`CommitGroupId`'s pages are written, finalized, and become recoverable
under the persistent record protocol (planv0.md line 2138). Boundaries
are totally ordered by canonical IR position (F-B5's NodeAnchor order
applied to the IR-level event that triggers the commit), and that
total order is reflected in the `commit_boundaries` list.

The single-page invariant `I-SramSinglePage` ranges over commit
boundaries as well as over individual epochs:

```text
∀ commit boundary β.
  SramVisibility(β).page is well-defined (exactly one SramPage value or None)
  ∧ for every (epoch e, commit boundary β) where e is the epoch
    immediately preceding β:
       SramVisibility(β).page = SramVisibility(e).page when defined,
       OR β includes a typed PageRotation event recorded in
       page_rotations[].
```

Page rotations are *visible* events: F-B9 emits an explicit
`PageRotation` record per epoch boundary at which the visible SRAM
page changes. The cumulative count of `PageRotation` events per
canonical token boundary is the source of
`projected_sram_page_switches_per_token`.

Atomicity: a `CommitGroupId`'s pages are *all* visible during the
boundary's commit window or *none* are. Because only one SRAM page is
visible at a time, a commit group whose member pages span more than
one SRAM bank must serialize the writes:

```text
∀ commit_group c.
  pages(c) = {p₁, ..., pₖ}
  bank(p) ∈ SramBank for each p ∈ pages(c)
  if k > 1 ∨ |{bank(p) : p ∈ pages(c)}| > 1:
    member writes are serialized across page rotations within the
    commit boundary;
    the PersistGroupCommit manifest (gbf-abi) is written last, on
    the page selected by spill_policy.persist_manifest_residency.
```

The serialization order is canonical (lexicographic by
`PersistPageId`), so two builds with the same inputs produce the same
ordering.

### 2.7 Kernel residency, LUT residency, and the contradiction resolution

The contradiction (planv0.md line 1741): **you cannot have "shared
micro-kernels in common banks" AND "expert-local data in expert banks"
be the hot-path default simultaneously, because there is only one
switchable ROM window at a time.**

F-B10's resolution is to give every kernel instance and every LUT
instance a typed residency choice:

```rust
pub enum KernelResidency {
    Bank0Fixed,                         // executes from bank 0
    WramOverlay,                        // copied into WRAM at install time
    CoResidentSwitchable,               // executes from the same switchable
                                        // bank as its data
}

pub enum LutResidency {
    Bank0Inline,                        // inlined in bank 0
    WramStaged,                         // staged into WRAM at install time
    RomCoResident,                      // co-located with the consuming kernel
                                        // in the switchable bank
}
```

Per phase, the joint constraint is:

```text
∀ kernel k, ∀ LUT ℓ used by k, ∀ tensor t streamed by k.
  if KernelResidency(k) = CoResidentSwitchable
    ∧ Materialization(t).class = RomConst
    ∧ Materialization(t).rom_bank = b
    ⇒ RomVisibility(phase(k)).switchable = Some(b)
       ∧ LutResidency(ℓ) ∈ { Bank0Inline, WramStaged, RomCoResident
                              with bank = b }

  if KernelResidency(k) = Bank0Fixed
    ⇒ k's code resides in bank 0
       ∧ k must not depend on the switchable window for control flow
         (data-streaming through the switchable window is allowed
          provided the data tensor's RomConst bank matches RomVisibility)

  if KernelResidency(k) = WramOverlay
    ⇒ k's code is a copy in WRAM
       ∧ k must not depend on the switchable window for control flow
       ∧ install timing is owned by F-B11 OverlayPlan
```

F-B10's job is to pick a residency assignment that satisfies the joint
constraint for every phase, prefers `Bank0Fixed` when ISR-reachable,
prefers `CoResidentSwitchable` when `RuntimeChromeBudget` permits,
and falls back to `WramOverlay` otherwise — subject to the
`KernelOverlayAdmissible` knob (named-only here, defined in F-B16).

The default selection rule is:

```text
SelectKernelResidency(k):
  if reachability(k) ∈ { IsrReachable, YieldResumeReachable,
                          FaultPathReachable }:
    => Bank0Fixed
  else if k's code fits in Bank 0 reserved slack
        ∧ Bank0Fixed_admissible(k):
    => Bank0Fixed
  else if k's code fits in WRAM reserved slack for overlays
        ∧ KernelOverlayAdmissible(k):
    => WramOverlay
  else if exists b ∈ SwitchableBanks. CoResidentSwitchable_legal(k, b):
    => CoResidentSwitchable
  else:
    => emit RomNoLegalKernelResidency(k); Hard
```

Tie-breaking (when two of the above succeed) follows a
`KernelResidencyPreferenceOrder` in `ResolvedCompilePolicy`
(`compile_knobs`); the default order is `Bank0Fixed >
CoResidentSwitchable > WramOverlay` for non-overlay-friendly kernels
and `WramOverlay > CoResidentSwitchable` otherwise.

Amends planv0: planv0.md says `RomWindowPlan` "computes which ROM
objects must be simultaneously visible." This RFC pins the semantic
boundary: F-B10 owns the *visibility set*, but the *placement* of an
overlay kernel in WRAM is owned by F-B11. F-B10 emits `WramOverlay`
as a residency *decision*; F-B11 turns the decision into an install
schedule.

### 2.8 ISR-reachability is a hard precondition for Bank 0 residency

Per planv0.md line 121: **all ISR code and ISR-reachable data live in
Bank 0, HRAM, or fixed WRAM only; no interrupt handler may depend on
the currently selected switchable ROM or SRAM bank.** This is computed
by `ReachabilityValidation` (F-B15) — *not* declared and hoped for.

F-B10's job is to make the proof's preconditions hold by construction.
F-B6 (`ObservationPlan`) classifies every kernel and LUT into
`ReachabilityClass` ∈ { `IsrReachable`, `YieldResumeReachable`,
`FaultPathReachable`, `HarnessEntryReachable`, `BankLeaseProtected`,
`NormalOnly` }.

The rule:

```text
F-IsrBank0:
  ∀ kernel k.
    reachability(k) ∈ { IsrReachable, YieldResumeReachable,
                          FaultPathReachable }
    ⇒ KernelResidency(k) = Bank0Fixed

F-LutIsrBank0:
  ∀ LUT ℓ.
    reachability(ℓ) ∈ { IsrReachable, YieldResumeReachable,
                          FaultPathReachable }
    ⇒ LutResidency(ℓ) ∈ { Bank0Inline,
                           WramStaged with install_class = AlwaysResident }
```

`HarnessEntryReachable` and `BankLeaseProtected` are weaker classes:
the harness ABI is reachable from the harness command block in SRAM
under specific bank lease protocols, so harness-entry kernels may
reside in switchable banks provided they are protected by a
`BankLease`. Bank-lease-protected kernels may reside in any class
provided the lease ABI is honored at scheduling time.

When `F-IsrBank0` fails (e.g. a kernel is ISR-reachable but does not
fit in Bank 0 reserved slack), F-B10 emits
`RomIsrReachableKernelExceedsBank0Slack` with the kernel's byte size
and the available Bank 0 slack. The repair path may suggest:

* shrinking the kernel via `KernelInlineThreshold` knob;
* hoisting a sub-kernel to Bank 0 fixed and leaving the rest as an
  overlay (only if the sub-kernel's reachability strictly excludes
  the ISR path);
* moving harness entry kernels (if the diagnostic stems from a
  borderline class) to `BankLeaseProtected` via runtime policy.

Amends planv0: planv0.md states the ISR rule as a hard runtime
invariant. This RFC turns it into a *static precondition* for the
F-B15 proof: if F-B10 emits a non-Bank0 residency for an ISR-reachable
kernel, F-B15's reachability proof fails by construction. F-B10's
gate is therefore the shift-left filter for an entire class of
runtime hangs.

### 2.9 Co-residency closures for switchable banks

When `KernelResidency(k) = CoResidentSwitchable`, the kernel's code,
the data tensors it streams, and any RomCoResident LUTs it consumes
must all live in the **same** switchable bank (or live in Bank 0 /
HRAM / fixed-WRAM but those don't count against the switchable
window). Formally:

```rust
pub struct CoResidentClosure {
    pub bank: RomBank,
    pub kernels: NonEmptyVec<KernelInstanceId>,
    pub luts: Vec<LutInstanceId>,
    pub tensors: NonEmptyVec<TensorMaterializationRef>,
}

pub struct TensorMaterializationRef {
    pub tensor_id: TensorId,
    pub binding: StorageBindingHandle,
}
```

A co-resident closure is well-formed iff:

```text
∀ k ∈ closure.kernels.
  KernelResidency(k) = CoResidentSwitchable

∀ ℓ ∈ closure.luts.
  LutResidency(ℓ) = RomCoResident with bank = closure.bank

∀ t ∈ closure.tensors.
  Materialization(t).class = RomConst
  ∧ rom_bank(Materialization(t)) = closure.bank

∀ phase φ where any member of the closure executes.
  RomVisibility(φ).switchable = Some(closure.bank)
```

Two distinct `CoResidentClosure` values may not share a phase unless
they share `bank`. Closures are computed by union-find over the
co-residency demand graph (kernels share an edge with the tensors
they stream and the LUTs they consume).

If a phase contains kernels from two closures with different banks,
`RomCoResidencyClosureBankConflict` is emitted (Hard).

### 2.10 Bank switches are an integer projection, not a cycle estimate

F-B10 emits `projected_bank_switches_per_token`: a static integer count
of bank-switch events per token under the resolved policy. This is
computed by counting epoch-boundary changes in `RomVisibility` along
the canonical IR traversal of one token's compute.

```text
ProjectedBankSwitchesPerToken =
  | { β ∈ epoch_boundaries(one_token)
        : RomVisibility(epoch_before(β)).switchable
          ≠ RomVisibility(epoch_after(β)).switchable
      } |
```

Likewise, F-B9 emits `projected_sram_page_switches_per_token` as the
count of `PageRotation` events per token.

These counts are *static integer projections* under the resolved
policy. They feed `static_budget.json` (the F-B4 report) at re-run
time when budgets need to be re-checked, and they feed
`schedule_cost.json` (F-B14) as cost inputs. They are *not* cycle
estimates: cycles arise by multiplying with calibrated per-switch cost
constants (per `gbf-bench`), which F-B14 owns.

F-B10 also records `projected_bank_switches_per_phase[]` for finer
granularity. The same applies to F-B9's per-phase page switches.

Caps:

```text
projected_bank_switches_per_token ≤ resolved_policy.max_bank_switches_per_token
projected_sram_page_switches_per_token ≤ resolved_policy.max_sram_page_switches_per_token
```

Cap violations are Hard diagnostics
(`RomBankSwitchesPerTokenExceedsCap`,
`SramPageSwitchesPerTokenExceedsCap`). The cap values themselves come
from `RuntimeChromeBudget` and `compile_knobs`; the names are
F-B16-defined.

### 2.11 Determinism and reproducibility

Both stages are deterministic functions of pinned inputs. The
construction algorithms use canonical orderings:

* Kernels are visited in lexicographic order by `(layer_id,
  expert_id?, kernel_kind, occurrence_index)`.
* LUTs are visited in lexicographic order by `(decode_plan_id?,
  norm_plan_id?, lut_kind, occurrence_index)`.
* `PersistPageId` values are visited in lexicographic order.
* `CommitGroupId` values are visited in lexicographic order.
* Tie-breaking in residency selection follows
  `KernelResidencyPreferenceOrder` (a closed enum permutation in
  `ResolvedCompilePolicy.compile_knobs`).
* Tie-breaking in spill policy follows `SpillPreferenceOrder` (likewise
  in `compile_knobs`).

Two builds with identical pinned inputs (identical `StoragePlan`,
`ObservationPlan`, `RangePlan`, `ResolvedCompilePolicy`,
`TargetProfile`, `RuntimeChromeBudget`, `QuantGraph`, `GbInferIR`)
must produce byte-identical `sram_plan.json`, `rom_window_plan.json`,
and the corresponding certificates. This is the chunk-closure
reproducibility gate.

### 2.12 Where the code lives

| Concern | Crate / module |
|---|---|
| `SramPagePlan`, `SramWorkingSet`, `SramPageBinding`, `CommitBoundary`, `SpillPolicy` types | `gbf-codegen::stages::sram_page` |
| `RomWindowPlan`, `KernelResidency`, `LutResidency`, `RomWindowBinding`, `ResidencyEpoch`, `WramOverlayDemand` types | `gbf-codegen::stages::window` |
| Stage 7 implementation (`build_sram_page_plan_core` + driver) | `gbf-codegen::stages::sram_page::run` |
| Stage 8 implementation (`build_rom_window_plan_core` + driver) | `gbf-codegen::stages::window::run` |
| `sram_plan.v1`, `sram_cert.v1`, `rom_window_plan.v1`, `window_cert.v1` schemas | `gbf-report::schemas::{sram_plan, sram_cert, rom_window_plan, window_cert}` |
| Shared `ValidationDiagnostic` taxonomy extensions | `gbf-policy::diagnostics` (`SRAM-*`, `ROM-*` codes) |
| StageCache integration | `gbf-store` consumed by `gbf-codegen::stage_cache` |
| Memory map predicates | `gbf-hw::memory` (consumed unchanged) |
| MBC5 register set | `gbf-hw::mbc5` (consumed unchanged) |
| BankLease / BankGuard ABI | `gbf-runtime::banking` (consumed by F-B13/F-B15; not by F-B9/F-B10) |

No new crate is created by this chunk.

### 2.13 No profile-time relaxation

Every Stage 7 / Stage 8 gate is a hard typed input. There is no
profile-conditional softness, no in-flight `RuntimeChromeBudget`
mutation, and no soft diagnostic in this chunk. Reduced reserved-slack
for Bringup is an explicit input (`bringup-*.chrome_budget.json`); F-B9
/ F-B10 never mutate the source budget.

`DiagnosticSeverity::Soft` remains in the taxonomy for downstream
stages, but F-B9/F-B10 report semantic validators reject any `Soft`
diagnostic.

### 2.14 No `RepairProposal` provenance

`compile_knobs` provenance values consumed here are restricted to
`TargetDefault | ProfileDefault | CompileRequestOverride | HintBundle |
Calibration`. `PolicySource::RepairProposal(_)` is *forbidden* in
F-B9/F-B10 v1, exactly as in F-B2/F-B4. F-B16 introduces it later by
explicit amendment.

### 2.15 Reserved overlay-install schedule

F-B10 emits `KernelResidency::WramOverlay` and `LutResidency::WramStaged`
as residency *decisions*, plus a `WramOverlayDemand` summary the next
chunk consumes:

```rust
pub struct WramOverlayDemand {
    pub kernels: Vec<OverlayKernelDemand>,
    pub luts: Vec<OverlayLutDemand>,
    pub total_overlay_bytes: u32,
    pub total_install_count_per_token_upper_bound: u16,
    pub share_class_hints: Vec<OverlayShareClassHint>,
}

pub struct OverlayKernelDemand {
    pub kernel: KernelInstanceId,
    pub install_class: OverlayInstallClass,
    pub byte_size: u32,
    pub reachability: ReachabilityClass,
}

pub enum OverlayInstallClass {
    AlwaysResident,                     // installed once at boot
    PerToken,                           // re-installed each token
    PerEpoch,                           // re-installed at epoch boundary
}
```

F-B11 (`OverlayPlan`) consumes `WramOverlayDemand` and emits
`OverlayRegion`, `OverlayInstall`, `OverlayShareClass`. F-B10 does
**not** emit those types.

### 2.16 Spill policy is total

F-B9 emits exactly one `SpillPolicy` value per build:

```rust
pub struct SpillPolicy {
    pub default_residency: SpillResidency,
    pub persist_manifest_residency: PersistManifestResidency,
    pub cold_spill_residency: ColdSpillResidency,
    pub preference_order: SpillPreferenceOrder,
}

pub enum SpillResidency {
    NeverSpill,                         // spills are forbidden; runtime
                                        // OOM if working set exceeds bound
    SpillToSram { class: SramSpillClass },
}

pub enum SramSpillClass {
    DedicatedSpillPage,                 // a page reserved for spills
    SharedColdPage,                     // shares with low-priority cold data
    OverflowGroup { group: SpillGroupId }, // overflow into a numbered group
}

pub enum PersistManifestResidency {
    SamePageAsLastMember,
    DedicatedManifestPage,
}

pub enum ColdSpillResidency {
    NoColdSpill,
    BoundedColdSpill { max_pages: u8 },
}
```

The total / per-build constraint means the policy is set once for the
entire build (per `RuntimeMode`), not chosen per binding. The choices
are determined by `compile_knobs.spill_*` values and may not be
overridden by individual bindings.

Amends planv0: planv0.md sketches `SpillPolicy` (line 1722) without
pinning shape. This RFC pins the closed enum shape and the totality
property.

### 2.17 No model topology is required to test the pass

Both passes are deterministic functions of typed inputs. Their tests use
synthetic `StoragePlan`, `ObservationPlan`, `RangePlan`,
`ResolvedCompilePolicy`, `TargetProfile`, `RuntimeChromeBudget`,
`QuantGraph`, `GbInferIR` fixtures. The chunk does not require a real
M2/M3 model, a real emulator run, or any neural-network semantics.
This is the chunk's main schedule advantage over F-B13/F-B15: every
test is in-process unit-testable.

### 2.18 F-B16 RepairPolicy / CompileKnobs is named-only

The following knobs are *referenced* by name from F-B9/F-B10
construction but *defined* by F-B16:

* `KernelResidencyPreferenceOrder`
* `LutResidencyPreferenceOrder`
* `KernelOverlayAdmissible`
* `LutOverlayAdmissible`
* `MaxBankSwitchesPerTokenCap`
* `MaxSramPageSwitchesPerTokenCap`
* `SpillPreferenceOrder`
* `Bank0FixedAdmissible`
* `CoResidentSwitchableAdmissible`
* `OverlayShareClassPolicy`

The provenance of every consumed knob value is recorded in
`policy_resolution.json`. The provenance is *not* recorded again in
F-B9/F-B10's reports (those reports cite
`policy_resolution_self_hash` as audit parent).

If F-B16 introduces a new knob that F-B9/F-B10 must consume, F-B16 must
amend this RFC by name.

### 2.19 Reservation accounting is a downstream concern

F-B9 emits **demand totals**: total bytes per persistent page kind,
total commit-group manifest bytes, total cold-spill page count under
the policy. F-B10 emits **demand totals**: total bank-resident kernel
bytes per bank, total Bank 0 fixed kernel bytes,
`WramOverlayDemand.total_overlay_bytes`, `LutResidency` byte totals.

F-B12 (`ArenaPlan`) does the reservation *accounting*: comparing
`RuntimeChromeBudget.wram_overlay_bytes` against
`F-B11.OverlayPlan.regions.byte_total` and rejecting if the overlay
demand exceeds the reservation. F-B9/F-B10 do not perform the
comparison; they emit the demand and let F-B12 reject.

**Exception** (named in §2.10): bank-switch and page-switch caps
*are* checked here, because they bound *behavior* not bytes, and
their cap source (`RuntimeChromeBudget.max_bank_switches_per_token`)
is consumed alongside other behavior-bounding inputs. Byte caps remain
F-B12's concern.

### 2.20 Single-window invariant proofs are constructive

Rather than "we test that the invariant holds on fixtures," this RFC
demands a constructive proof: the type system makes the impossible
unrepresentable wherever feasible.

```text
Constructive: RomVisibility.switchable: Option<RomBank>
              -- at most one switchable bank per phase, by typing.

Constructive: SramVisibility.page: Option<SramPage>
              -- at most one SRAM page per visibility window, by typing.

Constructive: ResidencyEpoch holds exactly one RomWindowBinding and
              exactly one SramPageBinding -- changes are epoch
              boundaries, not intra-epoch state.

Imposed: I-RomSingleWindow holds for every legal trace.
         (Proof: every assignment goes through SelectKernelResidency
          and a typed conflict-detection pass that emits Hard
          diagnostics on multi-bank phases. Production attempt to
          assign two banks to one phase results in
          RomMultipleSwitchableBanksDemandedInPhase before the plan
          is constructed.)

Imposed: I-SramSinglePage holds for every legal trace.
         (Proof: pages are bound by canonical lexicographic order;
          two bindings to the same epoch and different pages emit
          SramMultiplePagesDemandedInEpoch.)
```

The proof obligations §17.PO-W1, §17.PO-S1, §17.PO-W2, §17.PO-S2 walk
through the formal arguments.

## 3. Glossary additions

This chunk introduces or pins the following terms beyond the F-B2/F-B4
and F-B3/F-B5 glossary inheritance.

| Term                       | Definition                                                                                  |
|----------------------------|---------------------------------------------------------------------------------------------|
| Bank 0 (fixed)             | The MBC5 bank fixed at `$0000–$3FFF`. Always visible. Owns the runtime nucleus.             |
| Switchable ROM window      | The 16 KiB window at `$4000–$7FFF` mapping one switchable ROM bank at a time.               |
| Switchable SRAM window     | The 8 KiB window at `$A000–$BFFF` mapping one SRAM page at a time. Up to 16 pages on MBC5.  |
| Single-window invariant    | The pair of laws `I-RomSingleWindow` and `I-SramSinglePage`: at most one switchable bank in the ROM window per phase, at most one SRAM page in the SRAM window per epoch / commit boundary. |
| ResidencyEpoch             | A contiguous range of `GbInferIR` operations during which the visibility tuple is constant. F-B9/F-B10 emit; F-B13 consumes. |
| Phase                      | Synonym for `ResidencyEpoch` plus its op-anchor range. Used in invariant statements.         |
| Hot operation              | A `GbInferIR` node whose `StorageBinding` is `Materialize { class: RomConst }`, `Materialize { class: SramPaged }`, or `Persist`, and whose enclosing kernel is in the operational set. Drives the visibility laws. |
| RomWindowBinding           | A typed `(EpochId, Option<RomBank>)` plus reachability and co-residency closure id; what `ResourceLeaseKind::RomWindow` carries downstream. |
| SramPageBinding            | A typed `(CommitBoundaryRange, SramPage)` plus working-set membership and persistence kind; what `ResourceLeaseKind::SramPage` carries downstream. |
| RomVisibility              | The set `{Bank0Fixed} ∪ {switchable bank if any}` visible during a phase.                   |
| SramVisibility             | The set `{SRAM page if any}` visible during a phase or commit boundary.                     |
| KernelResidency            | One of `Bank0Fixed | WramOverlay | CoResidentSwitchable`. Closed in v1.                     |
| LutResidency               | One of `Bank0Inline | WramStaged | RomCoResident`. Closed in v1.                            |
| CoResidentClosure          | The set of kernels, LUTs, and tensors that must share a single switchable bank.             |
| WramOverlayDemand          | F-B10's typed summary of overlay residency demand, consumed by F-B11.                       |
| CommitBoundary             | A canonical totally-ordered position in the schedule at which a `CommitGroupId` becomes durable. |
| PageRotation               | A typed event recording an epoch-boundary change in `SramVisibility.page`.                  |
| SramWorkingSet             | The set of typed bindings active in one residency epoch under the SRAM-paged storage class. |
| SpillPolicy                | The total per-build policy controlling cold-spill residency, manifest residency, preference order. |
| ISR-reachable              | A reachability class assigned by F-B6 to kernels and LUTs the interrupt service routine may execute or load from. Forces Bank 0 / HRAM / fixed-WRAM residency. |
| Yield-resume reachable     | A reachability class assigned by F-B6 to code/data the cooperative yield/resume path may execute or load from. Forces Bank 0 residency. |
| Fault-path reachable       | A reachability class assigned by F-B6 to code/data the fault recovery path may execute or load from. Forces Bank 0 residency. |
| Bank0Fixed admissible      | A boolean knob (F-B16) saying whether the kernel is permitted to live in Bank 0.            |
| CoResidentSwitchable legal | A boolean predicate testing whether a kernel can co-reside with its data tensors in a single switchable bank without exceeding `RuntimeChromeBudget` or violating closure constraints. |
| KernelInstanceId           | F-B6/F-B7-internal identifier for a unique kernel instance (e.g. one expert's matvec).      |
| LutInstanceId              | F-B6/F-B7-internal identifier for a unique LUT instance (e.g. a softmax LUT used by one classifier head). |

## 4. Core notation

This RFC inherits §1 of F-B2/F-B4 (Hash256, Outcome, Severity, Stage,
ReportSchema, Result, Option, NonEmptyList, NonEmptyVec, NonEmptySet,
SortedBy, DomainHash, SelfHash, CanonicalJson, ZERO_HASH, null policy)
and §4 of F-B3/F-B5. Additions:

```text
Stage :=
  Stage0 | Stage0_5 | Stage1 | Stage2 | Stage3 | Stage4 | Stage5
  | Stage6                                  -- F-B8 added
  | Stage7                                  -- F-B9 added
  | Stage8                                  -- F-B10 added
  | Stage8_5                                -- F-B11 (next chunk)
  | Stage9                                  -- F-B12 (next chunk)

ReportSchema :=
  artifact_validation.v1
  | policy_resolution.v1
  | static_budget.v1
  | quant_graph.v1
  | infer_ir.v1
  | observation_plan.v1
  | range_plan.v1
  | storage_plan.v1
  | sram_plan.v1                            -- new
  | sram_cert.v1                            -- new
  | rom_window_plan.v1                      -- new
  | window_cert.v1                          -- new

ValidationOrigin (extension) :=
  ...existing F-B2..F-B8 origins...
  | SramPagePlanConstruction
  | RomWindowPlanConstruction

StageCacheKey :=
  K0 | K0_5 | K1 | K2 | K3 | K4 | K5 | K6
  | K7 := SramPagePlanCacheKey
  | K8 := RomWindowPlanCacheKey
```

Abbreviations used throughout:

```text
SPP := SramPagePlan
RWP := RomWindowPlan
KRes := KernelResidency
LRes := LutResidency
RV   := RomVisibility
SV   := SramVisibility
CB   := CommitBoundary
RE   := ResidencyEpoch
WOD  := WramOverlayDemand
RC   := ReachabilityClass
```

## 5. Authority rules

```text
Scope(F-B9/F-B10) =
  {
    Stage7,
    Stage8,
    SramPagePlan,
    RomWindowPlan,
    sram_plan.v1,
    sram_cert.v1,
    rom_window_plan.v1,
    window_cert.v1,
    StageCache keys for Stage7 and Stage8,
    KernelResidency closed enum,
    LutResidency closed enum,
    RomVisibility / SramVisibility shape,
    ResidencyEpoch / Phase,
    CommitBoundary / PageRotation,
    SpillPolicy closed enum,
    WramOverlayDemand shape,
    CoResidentClosure shape,
    ISR-residency precondition F-IsrBank0 / F-LutIsrBank0,
    bank-switch and page-switch projection algorithms,
    cap-exceedance diagnostics (RomBankSwitchesPerTokenExceedsCap,
                                 SramPageSwitchesPerTokenExceedsCap),
    single-window invariant proofs (PO-W1, PO-W2, PO-S1, PO-S2),
    is_hot_operation predicate
  }

Rule Authority:
  ∀ behavior b.
    b ∈ Scope(F-B9/F-B10) ∧ RFC specifies b
    ⇒ SourceOfTruth(b) = RFC

Rule PlanContext:
  ∀ behavior b.
    b ∈ Scope(F-B9/F-B10) ∧ RFC silent on b
    ⇒ planv0 may inform implementation but is not an acceptance gate

Rule InheritanceFromF-B2/F-B4:
  ∀ behavior b.
    b ∈ Scope(F-B2/F-B4) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B2/F-B4 RFC

Rule InheritanceFromF-B3/F-B5:
  ∀ behavior b.
    b ∈ Scope(F-B3/F-B5) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B3/F-B5 RFC

Rule InheritanceFromF-B6/F-B7/F-B8:
  ∀ behavior b.
    b is in F-B6/F-B7/F-B8 scope and that RFC has been finalized
    ⇒ SourceOfTruth(b) = the corresponding RFC

Rule Amendment:
  LaterRFC changes any of:
    public SPP type, public RWP type
    KernelResidency, LutResidency closed sets
    SpillPolicy closed set
    report shape (sram_plan.v1, sram_cert.v1, rom_window_plan.v1, window_cert.v1)
    cache key (K7, K8)
    diagnostic code introduced here
    single-window invariant statement
    is_hot_operation predicate
  ⇒ LaterRFC must explicitly amend this RFC

Rule DivergenceLedger:
  RFC intentionally diverges from planv0
  ⇒ nearest relevant section must contain `Amends planv0`

Rule ResourceLeaseEdge:
  F-B13 owns ResourceLeaseKind::RomWindow / SramPage and consumes
  RomWindowBinding / SramPageBinding by hash; F-B13 may not redefine
  the binding shape without amending this RFC.

Rule OverlayInstallEdge:
  F-B11 owns OverlayInstall, OverlayRegion, OverlayShareClass and
  consumes WramOverlayDemand by hash; F-B11 may not redefine
  WramOverlayDemand without amending this RFC.

Rule ArenaReservationEdge:
  F-B12 owns concrete byte ranges and reservation comparisons; F-B12
  must not redefine SramPageBinding or PersistPageId semantics.

Rule ReachabilityProofEdge:
  F-B15 owns ReachabilityValidation and discharges the proof of
  Bank 0 residency for ISR-reachable code/data; F-B10's job is to
  make the precondition non-vacuous.
```

## 6. Pipeline state machine

Extending the F-B2/F-B4 + F-B3/F-B5 + F-B6/F-B7 + F-B8 state machine:

```text
State :=
  Imported(inputs)
  | Validated(validation_product)
  | PolicyResolved(policy_product)
  | QuantGraphReady(policy_product, quant_graph_product)
  | BudgetPassed(quant_graph_product, static_budget_product)
  | InferIrReady(budget_product, infer_ir_product)
  | ObservationPlanReady(ir_product, observation_product)
  | RangePlanReady(observation_product, range_product)
  | StoragePlanReady(range_product, storage_product)
  | SramPagePlanReady(storage_product, sram_page_product)            -- new
  | RomWindowPlanReady(sram_page_product, rom_window_product)        -- new
  | Halted(stage, report, diagnostics)
```

Transitions (extending earlier state machines):

```text
T6 build_sram_page_plan:
  StoragePlanReady(s)
    -- build_sram_page_plan(...) = Ok(p) -->
  SramPagePlanReady(s, p)

  StoragePlanReady(s)
    -- build_sram_page_plan(...) = Err(e) -->
  Halted(Stage7, e.report, e.diagnostics)

T7 build_rom_window_plan:
  SramPagePlanReady(s, p)
    -- build_rom_window_plan(...) = Ok(w) -->
  RomWindowPlanReady(p, w)

  SramPagePlanReady(s, p)
    -- build_rom_window_plan(...) = Err(e) -->
  Halted(Stage8, e.report, e.diagnostics)
```

Pipeline invariants (additions to F-B2/F-B4 §3):

```text
I-Pipeline-Stage7:
  Stage7 may run only after Stage6 Passed.

I-Pipeline-Stage8:
  Stage8 may run only after Stage7 Passed.

I-Pipeline-NoSkip:
  Stage8 may not run from StoragePlanReady directly; SramPagePlanReady
  is mandatory.

I-Pipeline-Determinism:
  build_sram_page_plan_core and build_rom_window_plan_core are pure
  functions of their typed inputs. Two invocations on hash-identical
  inputs produce hash-identical outputs.

I-Pipeline-VisibilityCarried:
  Every RomWindowBinding (resp. SramPageBinding) emitted by Stage 8
  (resp. Stage 7) is referenced by exactly one ResidencyEpoch in the
  same product, and that ResidencyEpoch's op_range refers to canonical
  NodeAnchor identities present in the GbInferIR product cited in the
  audit parents.

I-Pipeline-PhaseCoverage:
  ⋃ epoch.op_range = full op range of GbInferIR (for the relevant
  RuntimeMode). Epochs partition the op range; no overlap, no gap.
```

Pipeline placement, restated:

```text
gbf-codegen::stages::storage     [F-B8]
   |
   v   StoragePlan product (StorageBinding[], AliasClass, Materialization)
+----------------------------------------+
| Stage 7   SramPagePlan                 |  F-B9
|                                        |
|   inputs:                              |
|     StoragePlan product                |
|     ObservationPlan product (reach.)   |
|     RangePlan product (reduction sites) |
|     ResolvedCompilePolicy              |
|     RuntimeChromeBudget                |
|     QuantGraph identity                |
|     GbInferIR identity                 |
|                                        |
|   computes:                            |
|     SramWorkingSet[ ] (per epoch)      |
|     SramPageBinding[ ]                 |
|     CommitBoundary[ ] (totally ordered)|
|     PageRotation[ ]                    |
|     SpillPolicy                        |
|     proj. SRAM page switches per token |
|                                        |
|   emits:                               |
|     sram_plan.json                     |
|     certs/sram.cert.json               |
+--------------------+-------------------+
                     |
                     v   SramPagePlan
+----------------------------------------+
| Stage 8   RomWindowPlan                |  F-B10
|                                        |
|   inputs:                              |
|     SramPagePlan product               |
|     StoragePlan product                |
|     ObservationPlan product (reach.)   |
|     ResolvedCompilePolicy              |
|     RuntimeChromeBudget                |
|     TargetProfile (memory map, banks)  |
|     QuantGraph identity                |
|     GbInferIR identity                 |
|                                        |
|   computes:                            |
|     KernelResidency[ ]                 |
|     LutResidency[ ]                    |
|     RomWindowBinding[ ]                |
|     CoResidentClosure[ ]               |
|     ResidencyEpoch[ ]                  |
|     WramOverlayDemand                  |
|     proj. ROM bank switches per token  |
|                                        |
|   emits:                               |
|     rom_window_plan.json               |
|     certs/window.cert.json             |
+--------------------+-------------------+
                     |
                     v   (F-B11 OverlayPlan, F-B12 ArenaPlan, etc.)
```

## 7. Report envelope (inherited)

This RFC inherits the `ReportEnvelope<R>` shape from F-B2/F-B4 §7.2
unchanged. Public JSON for both Stage 7 and Stage 8 reports is flat:

```json
{
  "schema": "...",
  "schema_version": "1.0.0",
  "outcome": "Passed" | "Failed",
  "report_self_hash": "sha256:...",
  ...body fields...
}
```

The `gbf-report` crate owns the serializer/deserializer that merges
envelope and body. `compute_self_hash` zeros `report_self_hash` to the
all-zero sentinel before hashing; `round_trip_self_hash` re-hashes
after parse and rejects if `stored != computed`. Canonical JSON rules
are inherited unchanged: UTF-8, lex object keys, integers only, no
NaN/Inf, no unknown fields, explicitly tagged enums, deterministic
array ordering.

For Stage 7 and Stage 8 reports, **floating-point fields are
forbidden**; fractional quantities (e.g. WRAM byte fractions for
overlays) use fixed-point integer fields with the scale in the field
name (e.g. `_q16_16`). All hashes serialize as `sha256:<lower-hex>`
strings. Domain-separated hashes use:

```text
gbf:<crate>:<type>:<schema-id>:<schema-version>\0<canonical-json-bytes>
```

Example domains for this chunk:

```text
gbf:gbf-report:SramPagePlanReport:sram_plan.v1:1.0.0\0...
gbf:gbf-report:SramCert:sram_cert.v1:1.0.0\0...
gbf:gbf-report:RomWindowPlanReport:rom_window_plan.v1:1.0.0\0...
gbf:gbf-report:WindowCert:window_cert.v1:1.0.0\0...
gbf:gbf-codegen:SramPagePlanCacheKey:sram_plan.v1:1.0.0\0...
gbf:gbf-codegen:RomWindowPlanCacheKey:rom_window_plan.v1:1.0.0\0...
```

Allowed nullable fields per schema are pinned in §11 (the report
schemas section). Outside those explicit allowances, `null` is illegal.

`Soft` diagnostics are rejected by `sram_plan.v1`, `sram_cert.v1`,
`rom_window_plan.v1`, and `window_cert.v1` semantic validators.

## 8. Stage 7 contract: `SramPagePlan`

### 8.1 Inputs

```rust
pub struct SramPagePlanInputs<'a> {
    pub storage_plan: &'a StoragePlan,
    pub observation_plan: &'a ObservationPlan,
    pub range_plan: &'a RangePlan,
    pub resolved_policy: &'a ResolvedCompilePolicy,
    pub runtime_chrome_budget: &'a RuntimeChromeBudget,
    pub target_profile: &'a TargetProfile,
    pub quant_graph_identity: QuantGraphIdentity,
    pub infer_ir_identity: InferIrIdentity,
    pub audit_parents: SramPagePlanAuditParents,
}

pub struct SramPagePlanAuditParents {
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
}
```

`StoragePlan` provides the `StorageBinding` list, the `AliasClassId`
partition, and the `Materialization` per binding. F-B9 reads only:

* `StorageBinding.materialization` filtering for
  `Materialize { class: SramPaged, .. }` and `Persist { .. }`;
* `StorageBinding.alias_class` for SRAM-paged groups;
* `StorageBinding.value` (a `ValueId` from `GbInferIR`) for op-range
  derivation.

`ObservationPlan` provides `ReachabilityClass` per kernel/LUT and per
storage binding (transitively via the kernel that consumes the
binding). F-B9 needs reachability only for `Persist { .. }` bindings
where the persist write is on the yield-resume path (forces
`SamePageAsLastMember` manifest residency to avoid cross-page
manifest write in a yield).

`RangePlan` provides reduction-site identifiers; F-B9 references those
only when an SRAM-paged accumulator scratch is selected (rare but
legal under heavy budget pressure; see §8.5).

`ResolvedCompilePolicy.compile_knobs` provides the named knobs from
§2.18 plus `SpillPreferenceOrder`, `MaxSramPageSwitchesPerTokenCap`,
and `PersistManifestPlacement`.

`RuntimeChromeBudget` provides `sram_total_bytes`,
`sram_reserved_slack`, `max_sram_page_switches_per_token`,
`persist_bytes_per_token`, and `cold_spill_max_pages`.

`TargetProfile` provides `MemoryMap` (so SRAM page count is bounded by
the cartridge profile's `sram_bank_count`) and the persistent record
protocol page geometry (4 KiB/8 KiB/16 KiB depending on profile).

The `QuantGraph` and `GbInferIR` identities are recorded for audit but
not consumed for construction (storage / reachability has already
distilled the relevant information).

### 8.2 Output product

```rust
pub struct SramPagePlan {
    pub identity: SramPagePlanIdentity,
    pub active_sets: Vec<SramWorkingSet>,
    pub page_bindings: Vec<SramPageBinding>,
    pub commit_boundaries: Vec<CommitBoundary>,
    pub page_rotations: Vec<PageRotation>,
    pub spill_policy: SpillPolicy,
    pub projections: SramSwitchProjections,
    pub provenance: SramPagePlanProvenance,
}

pub struct SramPagePlanIdentity {
    pub schema_version: SemVer,
    pub determinism: DeterminismClass,
    pub target_profile_id: TargetProfileId,
    pub runtime_mode: RuntimeMode,
}
```

#### `SramWorkingSet`

```rust
pub struct SramWorkingSet {
    pub epoch: EpochId,
    pub op_range: NodeAnchorRange,
    pub bindings: NonEmptyVec<SramPageBindingRef>,
    pub bytes_in_use: u32,
    pub bytes_reserved: u32,
    pub commit_boundaries_in_range: Vec<CommitBoundaryId>,
}

pub struct SramPageBindingRef {
    pub binding: SramPageBindingId,
    pub residency_role: SramResidencyRole,
}

pub enum SramResidencyRole {
    PersistentSequenceState,
    PersistentTranscript,
    PersistentHarness,
    PersistentTrace,
    SramPagedScratch,
    SramPagedSpill,
}
```

A working set is the typed set of SRAM-paged bindings active in one
epoch. The `bytes_in_use` field is the sum of the active members'
nominal byte demand; `bytes_reserved` includes alignment and
manifest-page overhead.

#### `SramPageBinding`

```rust
pub struct SramPageBinding {
    pub id: SramPageBindingId,
    pub page: SramPage,                 // exactly one
    pub kind: SramPageBindingKind,
    pub commit_groups: Vec<CommitGroupId>, // groups whose pages share this binding
    pub alias_classes: Vec<AliasClassId>,  // F-B8 alias classes contributing
    pub durability: DurabilityClass,
    pub generation_strategy: PersistGenerationStrategy,
    pub provenance: Vec<EvidenceRef>,
}

pub struct SramPageBindingId(u32);

pub enum SramPageBindingKind {
    Persistent {
        kind: PersistKind,              // SequenceState | Continuation |
                                        // Transcript | Harness | Trace
        page_state_machine: PersistPageStateMachineRef,
    },
    Paged {
        scratch_class: SramScratchClass,
    },
    Spill {
        group: SpillGroupId,
    },
    ManifestOnly {
        commit_group: CommitGroupId,
    },
}

pub enum SramScratchClass {
    AccumulatorOverflow,
    LargeStateScratch,
    DecodeBufferOverflow,
}

pub enum PersistGenerationStrategy {
    DoubleBuffered,                     // two pages rotated by generation
    SingleBufferedWithCommit,           // one page; commit word last
    RingBuffered { depth: u8 },         // fixed-depth ring
}
```

A binding identifies *which* page (or page family) hosts a particular
class of persistent or paged data. The `Persistent` variant carries a
`PersistPageStateMachineRef` referencing the artifact-stratum state
machine (`Writing -> Committed -> Retired`, planv0.md line 2188); the
binding supplies the *physical* page identity, the state machine
supplies the *logical* progression.

A `Paged` binding is `Materialize { class: SramPaged }` and is not
durable; the binding rotates pages epoch-by-epoch as working sets
change.

A `Spill` binding is the cold-spill target for the active spill group
(§2.16's `SramSpillClass::OverflowGroup`).

A `ManifestOnly` binding holds only the `PersistGroupCommit` manifest
for a commit group whose member pages live on other bindings (per the
manifest residency policy).

#### `CommitBoundary`

```rust
pub struct CommitBoundary {
    pub id: CommitBoundaryId,
    pub before_epoch: EpochId,
    pub after_epoch: EpochId,
    pub commit_group: CommitGroupId,
    pub generation_delta: u32,
    pub durability_class: DurabilityClass,
    pub member_pages: NonEmptyVec<SramPageBindingId>,
    pub manifest_binding: SramPageBindingId,
    pub serialization_order: Vec<SramPageBindingId>,
    pub yield_safe: YieldSafetyClass,
}

pub enum YieldSafetyClass {
    NoYieldDuringCommit,
    YieldOnlyAfterManifest,
    YieldAcrossPageRotations,           // requires PersistPageStateMachine
                                        // to be re-entrant with a strict
                                        // Writing -> Committed transition
                                        // bracket
}
```

`CommitBoundary.id` values are assigned in canonical order and are
totally ordered by canonical IR position. The `serialization_order`
list is canonical (lexicographic by `SramPageBindingId`).

#### `PageRotation`

```rust
pub struct PageRotation {
    pub at_epoch_boundary: (EpochId, EpochId),
    pub from: Option<SramPage>,
    pub to: Option<SramPage>,
    pub triggered_by: PageRotationTrigger,
}

pub enum PageRotationTrigger {
    EpochBoundary,
    CommitGroup { commit_boundary: CommitBoundaryId },
    PersistentRotation { binding: SramPageBindingId, generation: u32 },
    Spill { group: SpillGroupId },
}
```

Page rotations are the *visible* events that count against
`projected_sram_page_switches_per_token`. A page rotation with
`from = to` is illegal (it is not a rotation; it is a no-op) and is
elided from the list.

#### `SpillPolicy`

(See §2.16 for the closed enum.) F-B9 emits exactly one `SpillPolicy`
per build.

#### `SramSwitchProjections`

```rust
pub struct SramSwitchProjections {
    pub projected_sram_page_switches_per_token: u16,
    pub upper_bound_per_token: u16,
    pub per_phase: Vec<PerPhaseSwitchCount>,
    pub source: SwitchProjectionSource,
}

pub struct PerPhaseSwitchCount {
    pub epoch: EpochId,
    pub switches: u16,
}

pub enum SwitchProjectionSource {
    StaticEnumerationAtNodeAnchorBoundaries,
    StaticEnumerationWithAliasClassFolding,
}
```

The projection is the integer count of `PageRotation` events
encountered along the canonical IR traversal of one token's compute,
under the resolved residency. The upper-bound field accounts for
worst-case traversal through any branchy structure (currently equal
to the projection in v1; routed FFNs are handled by enumerating all
expert candidates per `F-AllExpertSlotsRealized` from F-B5 §2.15).

#### `SramPagePlanProvenance`

```rust
pub struct SramPagePlanProvenance {
    pub binding_to_storage_binding: BTreeMap<SramPageBindingId, Vec<StorageBindingHandle>>,
    pub commit_boundary_to_persist_event: BTreeMap<CommitBoundaryId, PersistEventId>,
    pub epoch_to_node_range: BTreeMap<EpochId, NodeAnchorRange>,
}
```

Every `SramPageBinding` has at least one `StorageBindingHandle` it
shadows; every `CommitBoundary` has exactly one `PersistEventId`
trigger derived from `StoragePlan.commit_groups`.

### 8.3 Construction order

```text
build_sram_page_plan_core(inputs):

  1. validate inputs:
     - storage_plan, observation_plan, range_plan match audit parents
     - storage_plan contains at least one SramPaged or Persist binding
       (else SramPagePlanEmpty -- but emit a degenerate but well-typed
        plan rather than failing hard; see §8.7)

  2. enumerate persistent commit groups:
     - for each Persist { page, commit_group } in storage_plan,
       group by commit_group
     - canonicalize commit-group order (lex by CommitGroupId)

  3. enumerate sram-paged bindings:
     - for each Materialize { class: SramPaged, .. },
       canonicalize order (lex by AliasClassId, then ValueId)

  4. resolve epoch boundaries from observation_plan +
     storage_plan + range_plan:
     - epoch boundaries fall at:
         * commit-group boundaries (CommitBoundaryId)
         * persistent-page rotation events
         * SramPaged working-set changes (alias-class transitions)
         * yield-resume entry points (from observation_plan)
     - assign EpochIds in canonical order

  5. compute SramWorkingSet per epoch:
     - canonical alias-class folding within each epoch
     - bytes_in_use = sum of nominal demand
     - bytes_reserved = bytes_in_use + alignment + manifest overhead

  6. assign SramPageBinding per binding:
     - persistent bindings: pick page based on commit_group and
       PersistGenerationStrategy
     - paged bindings: pick page based on working-set membership and
       SpillPreferenceOrder
     - spill bindings: from spill_policy.cold_spill_residency
     - manifest bindings: from spill_policy.persist_manifest_residency

  7. construct CommitBoundary per commit-group event:
     - serialization_order canonical
     - manifest_binding from spill_policy.persist_manifest_residency
     - yield_safe from observation_plan

  8. enumerate PageRotation events:
     - per epoch boundary; elide from = to

  9. project switch counts:
     - count PageRotation events along canonical token traversal

 10. enforce caps:
     - if projected > resolved_policy.max_sram_page_switches_per_token:
         emit SramPageSwitchesPerTokenExceedsCap; Hard

 11. validate single-page invariant:
     - per epoch e, |distinct pages in active_sets[e]| ≤ 1
     - per commit boundary β, |distinct pages in member_pages
       \∪ {manifest_binding.page}| handled via
       serialization_order (rotations are explicit)

 12. emit SramPagePlan + report + cert
```

### 8.4 Self-consistency invariants

These hold over every legal `SramPagePlan` value (proved by the
constructor):

```text
F-SPP-Total:
  Every Persist { page, commit_group } in storage_plan has exactly
  one SramPageBinding with kind = Persistent and one CommitBoundary
  whose member_pages includes the binding's page.

F-SPP-CommitContiguity:
  For every commit_group c, the commit_boundaries with
  commit_group = c form a contiguous subsequence (by id) in
  commit_boundaries[].

F-SPP-CommitOrdered:
  commit_boundaries[] is sorted by id; ids are assigned in canonical
  order over the IR-level persist events.

F-SPP-SerializationCanonical:
  CommitBoundary.serialization_order is the canonical
  lexicographic order over member_pages by SramPageBindingId.

F-SPP-EpochCoverage:
  ⋃ epoch.op_range = full op range of the GbInferIR product cited in
  audit parents, partitioned (no overlap, no gap).

F-SPP-PersistKindMatch:
  For every SramPageBinding with kind = Persistent { kind: k, .. },
  k matches the artifact-stratum PersistKind declared in
  storage_plan for the underlying commit_group.

F-SPP-WorkingSetByteFit:
  For every SramWorkingSet w, w.bytes_reserved ≤
  target_profile.sram_page_size_bytes.

F-SPP-SpillTotal:
  spill_policy is exactly one value, applied uniformly over the build.

F-SPP-CapsHonored:
  projections.projected_sram_page_switches_per_token ≤
    resolved_policy.max_sram_page_switches_per_token.

F-SPP-SinglePageVisibility:
  ∀ epoch e ∈ active_sets.
    |distinct pages of bindings in e| ≤ 1   -- the visibility law

F-SPP-NoColdSpillUnlessAllowed:
  spill_policy.cold_spill_residency = NoColdSpill
    ⇒ no SramPageBinding with kind = Spill { .. }
  spill_policy.cold_spill_residency = BoundedColdSpill { max_pages: n }
    ⇒ |{binding : kind = Spill}| ≤ n

F-SPP-ManifestResidency:
  ∀ commit_boundary β.
    if spill_policy.persist_manifest_residency = SamePageAsLastMember:
      β.manifest_binding ∈ β.member_pages
    if spill_policy.persist_manifest_residency = DedicatedManifestPage:
      β.manifest_binding has kind = ManifestOnly
        ∧ β.manifest_binding ∉ β.member_pages
```

### 8.5 Canonical reference semantics

A `SramPagePlan` is *canonically referenceable* if a future evaluator
(F-B11/F-B12/F-B13) can resolve every `Materialize { class: SramPaged }`
or `Persist { page, commit_group }` from `StoragePlan` to exactly one
`SramPageBinding` in the plan. The canonical resolution:

```text
ResolveBinding(storage_binding) -> SramPageBinding:
  case storage_binding.materialization of:
    Persist { page = p, commit_group = g }:
      let candidates = page_bindings filter
        kind = Persistent and g ∈ commit_groups
        and PersistPageStateMachineRef -> p
      assert |candidates| = 1
      return candidates[0]

    Materialize { class: SramPaged, lifetime = L, .. }:
      let alias = storage_binding.alias_class
      let candidates = page_bindings filter
        kind = Paged
        and alias ∈ alias_classes
      assert |candidates| ≥ 1
      // pick the candidate whose working-set membership covers the
      // binding's lifetime
      return select_by_lifetime(candidates, L)

    other: unreachable (filtered upstream)
```

The constructor must satisfy:

```text
∀ storage_binding ∈ storage_plan.bindings.
  is_sram_relevant(storage_binding)
  ⇒ ResolveBinding(storage_binding) is well-defined
```

If not, `SramPagePlanResolutionAmbiguous` (Hard) is emitted.

### 8.6 Role / format predicates

These predicates are typed and exported by `gbf-codegen::stages::sram_page`:

```rust
pub fn is_sram_relevant(b: &StorageBinding) -> bool {
    matches!(
        b.materialization,
        Materialization::Materialize { class: StorageClass::SramPaged, .. }
            | Materialization::Persist { .. }
    )
}

pub fn is_persistent_kind(k: &SramPageBindingKind) -> bool {
    matches!(k, SramPageBindingKind::Persistent { .. })
}

pub fn is_yield_safe_at(boundary: &CommitBoundary,
                       epoch: EpochId) -> bool {
    boundary.yield_safe == YieldSafetyClass::YieldAcrossPageRotations
        || (boundary.yield_safe == YieldSafetyClass::YieldOnlyAfterManifest
            && epoch == boundary.after_epoch)
}
```

These predicates are referenced by F-B11/F-B12/F-B13 and are the
single source of truth for the boolean questions they answer. Any
later RFC that needs a different predicate must amend this RFC.

### 8.7 Empty plans are well-typed

A build whose `StoragePlan` contains no SRAM-relevant bindings
(`is_sram_relevant` false for every binding) emits a well-typed but
degenerate `SramPagePlan`:

```text
SramPagePlan {
    active_sets: [],
    page_bindings: [],
    commit_boundaries: [],
    page_rotations: [],
    spill_policy: SpillPolicy::default_for(target_profile),
    projections: SramSwitchProjections {
        projected_sram_page_switches_per_token: 0,
        upper_bound_per_token: 0,
        per_phase: [],
        source: StaticEnumerationAtNodeAnchorBoundaries,
    },
    ...
}
```

Empty plans are legal because dense, prompt-only, non-streaming model
configurations may have no SRAM-paged or persistent state at all.
The plan still emits `sram_plan.json` and `certs/sram.cert.json` for
audit; the certificate records the empty-plan claim explicitly.

### 8.8 StageCache key (K7)

```rust
pub struct SramPagePlanCacheKey {
    pub schema: ReportSchemaId,         // = sram_plan.v1
    pub schema_version: SemVer,
    pub pass_version: PassVersion,
    pub feature_set: FeatureSet,
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub runtime_mode: RuntimeMode,
}
```

The key is `DomainHash("gbf-codegen", "StageCacheKey", "sram_plan.v1",
"1.0.0", canonical_json_bytes(SramPagePlanCacheKey))`. Cache miss
occurs on any drift in the listed inputs, on `pass_version` bump, or
on `feature_set` change.

A failure memo, when present, replays the original `report_self_hash`
unchanged.

## 9. Stage 8 contract: `RomWindowPlan`

### 9.1 Inputs

```rust
pub struct RomWindowPlanInputs<'a> {
    pub sram_page_plan: &'a SramPagePlan,
    pub storage_plan: &'a StoragePlan,
    pub observation_plan: &'a ObservationPlan,
    pub range_plan: &'a RangePlan,
    pub resolved_policy: &'a ResolvedCompilePolicy,
    pub runtime_chrome_budget: &'a RuntimeChromeBudget,
    pub target_profile: &'a TargetProfile,
    pub quant_graph_identity: QuantGraphIdentity,
    pub infer_ir_identity: InferIrIdentity,
    pub audit_parents: RomWindowPlanAuditParents,
}

pub struct RomWindowPlanAuditParents {
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
}
```

`SramPagePlan` is consumed because Stage 8 must reason about which
SRAM page (if any) is visible during each epoch when computing the
ROM-window visibility set: a kernel that streams an `SramPaged`
working-set tile and a `RomConst` data tensor in the same epoch
demands both visibility windows to coexist, and F-B10's epoch boundary
must reflect both transitions.

`StoragePlan` provides every `Materialize { class: RomConst }` binding
(the data tensors that live in ROM) and every kernel-bearing binding
(via `AliasClassId` aggregation).

`ObservationPlan` provides `ReachabilityClass` per kernel and per LUT.
F-B10 relies on this for the ISR-residency precondition.

`RangePlan` provides reduction-site identifiers; F-B10 uses them only
to confirm that a kernel's reduction subordinates are in the same
co-residency closure (avoiding the case where a tail reduction lives
in a different bank from its main reduction).

`ResolvedCompilePolicy.compile_knobs` provides the named knobs from
§2.18 plus `KernelInlineThreshold` and `OverlayShareClassPolicy`.

`RuntimeChromeBudget` provides Bank 0 reserved slack, WRAM reserved
slack for overlays, ROM bank count, and `max_bank_switches_per_token`.

`TargetProfile` provides `MemoryMap`, `RomBankCount`, `Mbc5RegisterSet`
references (consumed by hash; F-B10 doesn't perform the writes).

### 9.2 Output product

```rust
pub struct RomWindowPlan {
    pub identity: RomWindowPlanIdentity,
    pub kernel_residency: BTreeMap<KernelInstanceId, KernelResidency>,
    pub lut_residency: BTreeMap<LutInstanceId, LutResidency>,
    pub rom_window_bindings: Vec<RomWindowBinding>,
    pub residency_epochs: Vec<ResidencyEpoch>,
    pub co_resident_closures: Vec<CoResidentClosure>,
    pub overlay_demand: WramOverlayDemand,
    pub bank0_demand: Bank0Demand,
    pub projections: RomSwitchProjections,
    pub provenance: RomWindowPlanProvenance,
}

pub struct RomWindowPlanIdentity {
    pub schema_version: SemVer,
    pub determinism: DeterminismClass,
    pub target_profile_id: TargetProfileId,
    pub runtime_mode: RuntimeMode,
}
```

#### `RomWindowBinding`

```rust
pub struct RomWindowBinding {
    pub id: RomWindowBindingId,
    pub epoch: EpochId,
    pub visibility: RomVisibility,
    pub assigned_kernels: Vec<KernelInstanceId>,
    pub assigned_luts: Vec<LutInstanceId>,
    pub assigned_tensors: Vec<TensorMaterializationRef>,
    pub closure: Option<CoResidentClosureId>,
    pub provenance: Vec<EvidenceRef>,
}

pub struct RomWindowBindingId(u32);
```

A `RomWindowBinding` records the ROM-side visibility tuple per epoch,
plus the set of kernels/LUTs/tensors using that visibility. There is
exactly one `RomWindowBinding` per `EpochId`. The same `EpochId` is
shared between the SRAM-side `SramPageBinding` (in the corresponding
`ResidencyEpochSummary` from F-B9) and this binding.

#### `ResidencyEpoch`

```rust
pub struct ResidencyEpoch {
    pub id: EpochId,
    pub op_range: NodeAnchorRange,
    pub rom_window_binding: RomWindowBindingId,
    pub sram_page_binding: Option<SramPageBindingId>,
    pub overlay_state: OverlayState,
    pub yield_kind: YieldKindHint,
}

pub enum OverlayState {
    NoOverlayActive,
    OverlayActive { share_class: OverlayShareClassHint },
}

pub enum YieldKindHint {
    NoYieldsExpected,
    YieldsAtCommitBoundaries,
    YieldsAtTokenBoundary,
}
```

`ResidencyEpoch.id` identities are shared with F-B9: a single epoch
spans the same op range in both products.

#### `Bank0Demand`

```rust
pub struct Bank0Demand {
    pub kernels: Vec<Bank0KernelDemand>,
    pub luts: Vec<Bank0LutDemand>,
    pub total_kernel_bytes: u32,
    pub total_lut_bytes: u32,
    pub remaining_slack_bytes: i32,     // signed; negative if over budget
}

pub struct Bank0KernelDemand {
    pub kernel: KernelInstanceId,
    pub byte_size: u32,
    pub reachability: ReachabilityClass,
}

pub struct Bank0LutDemand {
    pub lut: LutInstanceId,
    pub byte_size: u32,
    pub reachability: ReachabilityClass,
}
```

`remaining_slack_bytes` may be negative; if so, the diagnostic
`RomBank0OverBudget` is emitted (Hard) before the plan is returned.
The signed integer makes "by how much over" reportable without
saturating at zero.

#### `WramOverlayDemand`

(See §2.15 for the definition.)

#### `CoResidentClosure`

(See §2.9 for the definition; `CoResidentClosureId` is `u32`.)

#### `RomSwitchProjections`

```rust
pub struct RomSwitchProjections {
    pub projected_bank_switches_per_token: u16,
    pub upper_bound_per_token: u16,
    pub per_phase: Vec<PerPhaseSwitchCount>,
    pub source: SwitchProjectionSource,
}
```

The projection is the integer count of epoch-boundary
`RomVisibility` changes along the canonical IR traversal of one
token's compute.

#### `RomWindowPlanProvenance`

```rust
pub struct RomWindowPlanProvenance {
    pub kernel_to_storage_aliases: BTreeMap<KernelInstanceId, Vec<AliasClassId>>,
    pub kernel_to_reachability: BTreeMap<KernelInstanceId, ReachabilityClass>,
    pub lut_to_reachability: BTreeMap<LutInstanceId, ReachabilityClass>,
    pub epoch_to_node_range: BTreeMap<EpochId, NodeAnchorRange>,
    pub closure_to_kernels: BTreeMap<CoResidentClosureId, Vec<KernelInstanceId>>,
}
```

### 9.3 Construction order

```text
build_rom_window_plan_core(inputs):

  1. validate inputs:
     - sram_page_plan, storage_plan, observation_plan, range_plan
       match audit parents
     - sram_page_plan.identity.runtime_mode = inputs.runtime_mode

  2. enumerate kernels and LUTs from observation_plan:
     - canonical order: lex by (layer_id, expert_id?, kernel_kind,
                                 occurrence_index)

  3. enumerate RomConst tensors from storage_plan:
     - canonical order: lex by (layer_id, role, tensor_id)

  4. resolve epoch boundaries:
     - take F-B9's epoch boundaries as the seed
     - extend with kernel/LUT residency transitions where a kernel
       moves between Bank0Fixed and overlay-installed states (rare
       but possible when overlay install timing crosses an SRAM
       commit boundary)
     - canonical EpochId order (already set by F-B9; F-B10 may
       only refine, not reassign)

  5. compute reachability classification:
     - read ReachabilityClass per kernel and per LUT from
       observation_plan
     - apply F-IsrBank0 / F-LutIsrBank0 to lock those into
       Bank0Fixed / Bank0Inline

  6. assign KernelResidency:
     - iterate kernels in canonical order
     - apply SelectKernelResidency rule from §2.7 with tie-breaking
     - record decision with provenance

  7. assign LutResidency:
     - similar to kernels
     - prefer Bank0Inline for Bank0Fixed kernels
     - prefer RomCoResident for CoResidentSwitchable kernels

  8. compute CoResidentClosures via union-find:
     - kernel-to-tensor edge iff tensor is streamed by kernel
       and tensor's class = RomConst
     - kernel-to-LUT edge iff LUT is consumed by kernel
       and LUT residency = RomCoResident
     - check well-formedness per §2.9

  9. compute PhaseSwitchableDemand per epoch:
     - for each epoch, enumerate the bank set demanded by
       CoResidentSwitchable kernels and RomConst tensors active
       in the epoch
     - if |demanded_banks| > 1, emit
       RomMultipleSwitchableBanksDemandedInPhase; Hard

 10. assign RomWindowBinding per epoch:
     - one binding per epoch
     - visibility.switchable = the unique demanded bank or None

 11. compute Bank0Demand and check slack:
     - sum byte sizes of all Bank0Fixed kernels and Bank0Inline LUTs
     - remaining_slack = bank0_reserved_slack - total
     - if remaining_slack < 0:
         emit RomBank0OverBudget; Hard

 12. compute WramOverlayDemand:
     - iterate overlayable kernels and LUTs
     - assign install_class from observation_plan + lifetime
     - record share-class hints from compile_knobs

 13. project switch counts:
     - count epoch-boundary changes in RomVisibility along canonical
       token traversal

 14. enforce caps:
     - if projected > resolved_policy.max_bank_switches_per_token:
         emit RomBankSwitchesPerTokenExceedsCap; Hard

 15. validate single-window invariant:
     - per epoch e, |demanded_banks(e)| ≤ 1
     - per closure c, all members refer to bank c.bank
     - per kernel k with Bank0Fixed residency, k's data dependencies
       are not bound to a switchable bank by a different closure

 16. emit RomWindowPlan + report + cert
```

### 9.4 Self-consistency invariants

```text
F-RWP-Total:
  Every kernel instance and LUT instance enumerated by
  observation_plan has exactly one residency assignment.

F-RWP-IsrBank0:
  ∀ k ∈ kernel_residency.keys.
    reachability(k) ∈ { IsrReachable, YieldResumeReachable,
                          FaultPathReachable }
    ⇒ kernel_residency[k] = Bank0Fixed

F-RWP-LutIsrBank0:
  ∀ ℓ ∈ lut_residency.keys.
    reachability(ℓ) ∈ { IsrReachable, YieldResumeReachable,
                          FaultPathReachable }
    ⇒ lut_residency[ℓ] = Bank0Inline
       ∨ lut_residency[ℓ] = WramStaged with install_class = AlwaysResident

F-RWP-SinglePhaseBank:
  ∀ epoch e ∈ residency_epochs.
    |demanded_banks(e)| ≤ 1

F-RWP-EpochCoverage:
  ⋃ epoch.op_range = full op range of the GbInferIR product cited in
  audit parents, partitioned (no overlap, no gap).

F-RWP-EpochAlignedWithSPP:
  ∀ epoch e ∈ residency_epochs.
    ∃ epoch e' ∈ sram_page_plan.active_sets / page_bindings such that
      e.id = e'.id ∧ e.op_range = e'.op_range.

F-RWP-ClosureBank:
  ∀ closure c ∈ co_resident_closures.
    All members of c are assigned bank = c.bank under their respective
    residency rules.

F-RWP-Bank0BudgetHonored:
  bank0_demand.remaining_slack_bytes ≥ 0.

F-RWP-OverlayBudgetHonored:
  overlay_demand.total_overlay_bytes ≤
    runtime_chrome_budget.wram_overlay_reserved_bytes.
  (Honored as a soft handshake: if exceeded, F-B11 will reject.
   This RFC emits a Hard diagnostic
   RomOverlayDemandExceedsWramReservation here so the failure is
   localized.)

F-RWP-CapsHonored:
  projections.projected_bank_switches_per_token ≤
    resolved_policy.max_bank_switches_per_token.

F-RWP-NoBank0FixedSwitchableData:
  ∀ kernel k with kernel_residency[k] = Bank0Fixed.
    ∀ tensor t streamed by k.
      Materialization(t).class = RomConst
      ⇒ Materialization(t).rom_bank ∈ SwitchableBanks
        ⇒ ∃ epoch e where k executes ∧ visibility(e).switchable = Some(rom_bank(t))
  -- Bank0Fixed kernels may stream switchable-bank data only in the
  -- epochs where the relevant bank is mapped.

F-RWP-OverlayKernelNoSwitchableControl:
  ∀ kernel k with kernel_residency[k] = WramOverlay.
    k's control-flow targets resolve in WRAM or Bank 0 only;
    (data-streaming through the switchable window is allowed under
     the same epoch-mapped rule).

F-RWP-CoResidentLegality:
  ∀ closure c.
    ∀ kernel k ∈ c.
      ∀ epoch e where k executes.
        visibility(e).switchable = Some(c.bank).
```

### 9.5 Canonical reference semantics

A `RomWindowPlan` is *canonically referenceable* if:

* every kernel instance and LUT instance that appears in
  `ObservationPlan` and is reachable from the IR (by `is_hot_operation`
  or by being a Bank 0 / overlay scaffold) has exactly one residency
  assignment;
* every `Materialize { class: RomConst }` binding in `StoragePlan`
  has an assigned `RomBank` (recorded in
  `provenance.tensor_to_bank_assignment`) consistent with the closure
  membership;
* every epoch's `(rom_window_binding, sram_page_binding)` pair is
  consistent (no kernel demands a bank or page that conflicts with
  the binding).

The resolution function for downstream consumers:

```text
ResolveKernelResidency(kernel_instance) -> KernelResidency:
  return kernel_residency[kernel_instance]

ResolveLutResidency(lut_instance) -> LutResidency:
  return lut_residency[lut_instance]

ResolveRomVisibilityAtNode(node_anchor) -> RomVisibility:
  let epoch = epoch_for_node_anchor(node_anchor)
  return rom_window_bindings[epoch].visibility
```

These resolution functions are exposed by
`gbf-codegen::stages::window` and are the single source of truth for
the questions they answer.

### 9.6 Empty / Bank-0-only plans are well-typed

A build whose every kernel and LUT resolves to `Bank0Fixed` /
`Bank0Inline` (e.g. a tiny dense model whose weights and kernels all
fit in Bank 0) emits a plan with no switchable bank in any
`RomWindowBinding.visibility.switchable` (all `None`):

```text
RomWindowPlan {
    kernel_residency: {... all Bank0Fixed ...},
    lut_residency: {... all Bank0Inline ...},
    rom_window_bindings: [
        RomWindowBinding {
            id: 0,
            epoch: 0,
            visibility: RomVisibility { bank0: Visible, switchable: None },
            ...
        }
    ],
    co_resident_closures: [],
    overlay_demand: WramOverlayDemand { ..., total_overlay_bytes: 0 },
    bank0_demand: Bank0Demand { ..., remaining_slack_bytes: > 0 },
    ...
}
```

Empty closures are legal.

### 9.7 StageCache key (K8)

```rust
pub struct RomWindowPlanCacheKey {
    pub schema: ReportSchemaId,         // = rom_window_plan.v1
    pub schema_version: SemVer,
    pub pass_version: PassVersion,
    pub feature_set: FeatureSet,
    pub artifact_validation_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub runtime_mode: RuntimeMode,
}
```

The key is `DomainHash("gbf-codegen", "StageCacheKey",
"rom_window_plan.v1", "1.0.0", canonical_json_bytes)`.

## 10. Single-window invariant — formal proof obligations

The two single-window laws are owned by this chunk. The constructors
implement them; the proofs that the constructors implement them
correctly are §17.

### 10.1 The laws

```text
I-RomSingleWindow:
  ∀ phase φ ∈ ResidencyEpochs.
    |RomVisibility(φ) ∩ SwitchableBanks| ≤ 1

  Equivalently: each phase's RomVisibility.switchable is either None
  or Some(b) for exactly one bank b.

I-SramSinglePage:
  ∀ commit boundary β ∈ CommitBoundaries.
    |SramVisibility(β) ∩ SramPages| ≤ 1

  Equivalently: each commit boundary has at most one SramPage exposed
  during its commit window. Pages other than the currently visible
  page are addressed via PageRotation events (which are explicit
  events, not concurrent visibility).
```

### 10.2 What "phase" and "commit boundary" mean formally

Both phrases were defined operationally in §2.4 / §2.6. The formal
forms are:

```text
Phase:
  Phase = ResidencyEpoch
  visibility(phase) =
    let b = phase.rom_window_binding in
    rom_window_bindings.find(id = b).visibility
    (typed pair {bank0: Visible, switchable: Option<RomBank>})

CommitBoundary:
  CommitBoundary as in §2.6
  visibility(boundary) =
    let s = boundary.serialization_order[0] in
    -- the first page exposed when the commit begins
    sram_page_bindings.find(id = s).page
    (typed Option<SramPage>)
```

The serialization order during a commit boundary may rotate through
multiple pages, but only one page is mapped at any instant — this is
recorded in `PageRotation` events and is the `I-SramSinglePage` law's
"at most one page visible" reading.

### 10.3 Proof obligation PO-W1: I-RomSingleWindow holds for every legal trace

```text
PO-W1: ∀ legal traces of build_rom_window_plan_core(inputs) = Ok(w).
        ∀ phase φ ∈ w.residency_epochs.
          |RomVisibility(φ) ∩ SwitchableBanks| ≤ 1

Proof sketch:
  1. RomVisibility.switchable has type Option<RomBank>. By the
     types alone, |RomVisibility.switchable as a set| ≤ 1.
  2. The intersection with SwitchableBanks (which excludes Bank 0)
     is therefore at most 1 element regardless of construction.
  3. The only way the law could fail is if a single epoch were
     constructed with two different RomBanks needed
     simultaneously — i.e. if PhaseSwitchableDemand for that epoch
     had |demanded_banks| > 1.
  4. Step 9 of build_rom_window_plan_core (§9.3) emits
     RomMultipleSwitchableBanksDemandedInPhase (Hard) if and only if
     |demanded_banks| > 1. A Hard diagnostic causes the constructor
     to return Err, so the only successful return paths have
     |demanded_banks| ≤ 1 per epoch.
  5. Therefore RomVisibility.switchable.unwrap() (when present) is
     the unique element of demanded_banks, and the law holds.

QED.
```

The constructive form (the `Option<RomBank>` typing) is the *primary*
mechanism; the diagnostic check is the *gate* that ensures the
constructor honors the typing.

### 10.4 Proof obligation PO-W2: visibility is consistent with closure membership

```text
PO-W2: ∀ legal traces of build_rom_window_plan_core(inputs) = Ok(w).
        ∀ epoch e ∈ w.residency_epochs.
          let b = w.rom_window_bindings[e.rom_window_binding].visibility.switchable
          ∀ kernel k assigned to e where kernel_residency[k] = CoResidentSwitchable.
            ∃ closure c such that k ∈ c.kernels ∧ c.bank = b.unwrap()
          ∀ tensor t with Materialization { class: RomConst, rom_bank: b' } streamed
                  by some kernel in e.
            b' ∈ SwitchableBanks ⇒ b = Some(b')
            b' = Bank 0 ⇒ no constraint (Bank 0 is always visible)

Proof sketch:
  1. Step 8 of construction performs union-find over the co-residency
     demand graph; closure membership reflects the actual data /
     control / LUT dependencies.
  2. Step 10 assigns visibility.switchable = the unique demanded bank
     across the closures intersecting the epoch.
  3. Step 15 verifies F-RWP-ClosureBank: every closure member's
     epoch has the closure's bank in visibility.switchable.
  4. By F-RWP-NoBank0FixedSwitchableData and the typed switchable-bank
     filter, no Bank0Fixed kernel forces a visibility change other
     than to a bank it actually depends on for data.
  5. Therefore the law holds at construction.

QED.
```

### 10.5 Proof obligation PO-S1: I-SramSinglePage holds for every legal trace

```text
PO-S1: ∀ legal traces of build_sram_page_plan_core(inputs) = Ok(p).
        ∀ commit boundary β ∈ p.commit_boundaries.
          at every instant during β's commit window, |SramVisibility| ≤ 1

Proof sketch:
  1. SramVisibility.page has type Option<SramPage>. By the types alone,
     |SramVisibility.page as a set| ≤ 1.
  2. The commit window is decomposed into a serialization order over
     member_pages; between any two consecutive members of the order,
     a PageRotation event is emitted that shifts visibility from the
     previous page to the next.
  3. At each instant in the commit window, visibility is a single
     SramPage value (or None when no page is being written, e.g.
     during a commit-word-only update on the manifest page after the
     last rotation).
  4. Therefore the law holds at every instant.

QED.
```

### 10.6 Proof obligation PO-S2: page rotations correspond to canonical events

```text
PO-S2: ∀ legal traces of build_sram_page_plan_core(inputs) = Ok(p).
        ∀ epoch boundary (e₁, e₂) where SramVisibility(e₁) ≠ SramVisibility(e₂).
          ∃ rotation r ∈ p.page_rotations such that
            r.at_epoch_boundary = (e₁, e₂)
            ∧ r.from = SramVisibility(e₁)
            ∧ r.to   = SramVisibility(e₂)

Proof sketch:
  1. PageRotation is emitted at every epoch boundary where the
     visible page changes (Step 8 of §8.3).
  2. The trigger is recorded as one of {EpochBoundary, CommitGroup,
     PersistentRotation, Spill}; at least one trigger always exists
     because rotation events are caused by typed input transitions
     (epoch boundary, commit group event, persistent rotation, or
     spill).
  3. No rotation is elided when from ≠ to (Step 8 only elides when
     from = to).
  4. Therefore the correspondence is total.

QED.
```

### 10.7 Proof obligation PO-V1: visibility tuples are consistent across F-B9 / F-B10

```text
PO-V1: ∀ legal traces where build_sram_page_plan_core(inputs) = Ok(p)
       and build_rom_window_plan_core(inputs') = Ok(w),
       where inputs'.sram_page_plan = p.
         ∀ epoch e.
           e ∈ p.active_sets / page_bindings (by id)
           ⇔ e ∈ w.residency_epochs (by id)
           ∧ epoch.op_range is byte-identical between p and w (when
             present in both)

Proof sketch:
  1. Step 4 of §9.3 takes p's epoch boundaries as the seed and may
     only refine, not reassign.
  2. Refinement means inserting a new epoch boundary, never removing
     or changing the position of an existing one.
  3. EpochId is allocated in canonical order; F-B9 allocates first,
     F-B10 may extend.
  4. F-RWP-EpochAlignedWithSPP confirms the alignment at the
     constructor-output layer; the property is checked.

QED.
```

Note that F-B10 may *split* an epoch (insert a new boundary) but may
not *merge* two F-B9 epochs into one. The split case happens when an
overlay install crosses an SRAM-page boundary inserted by F-B9; the
new boundary's `(rom, sram, overlay)` tuple is constant within each
half.

### 10.8 Proof obligation PO-I1: ISR-residency precondition is non-vacuous

```text
PO-I1: ∀ legal traces of build_rom_window_plan_core(inputs) = Ok(w).
        ∀ kernel k where reachability(k) ∈ {IsrReachable,
                                              YieldResumeReachable,
                                              FaultPathReachable}.
          w.kernel_residency[k] = Bank0Fixed
        ∀ LUT ℓ where reachability(ℓ) ∈ {IsrReachable,
                                           YieldResumeReachable,
                                           FaultPathReachable}.
          w.lut_residency[ℓ] ∈ {Bank0Inline,
                                 WramStaged with install_class = AlwaysResident}

Proof sketch:
  1. Step 5 of §9.3 reads ReachabilityClass per kernel and per LUT.
  2. Step 6 / 7 apply F-IsrBank0 / F-LutIsrBank0 *first*, before any
     other selection rule.
  3. If a Bank-0-fixed kernel exceeds Bank 0 reserved slack, Step 11
     emits RomBank0OverBudget (Hard) and the constructor fails.
  4. Therefore any successful Ok(w) has every ISR-reachable kernel
     in Bank0Fixed.

QED.
```

PO-I1 is what makes `ReachabilityValidation` (F-B15) non-vacuous:
without it, F-B15 could declare "all ISR-reachable code is in Bank 0"
trivially (because there is no ISR-reachable code), or declare it
without checking. With it, every ISR-reachable kernel and LUT is
forced into Bank 0 by a Hard diagnostic gate, and F-B15's proof has
content.

### 10.9 Combined invariant: the joint visibility tuple

Together, `I-RomSingleWindow` and `I-SramSinglePage` define the joint
single-window invariant:

```text
I-Joint:
  ∀ epoch e ∈ ResidencyEpochs.
    let r = w.rom_window_bindings[e.rom_window_binding].visibility
    let s = if e.sram_page_binding is Some(b)
              then p.page_bindings[b].page
              else None
    in:
      |r.switchable as set| ≤ 1
      ∧ |s as set| ≤ 1
      ∧ all hot operations in e.op_range may execute under (r, s)
        without triggering an additional bank or page switch
```

The third clause is the "epoch is the unit of constancy" property:
within an epoch, no hot operation requires a different visibility
tuple. Across epoch boundaries, exactly one of `r.switchable`,
`s.page`, or both may change; each change is reflected as a
bank-switch event (counted by F-B10's projection) or a page-rotation
event (counted by F-B9's projection).

### 10.10 Why constructive proofs matter here

A reviewer might ask: why bother with the typing trick and the
constructor gate when a runtime check would suffice?

The answer is that this chunk owns a *contract* for downstream
consumers: F-B12 (`ArenaPlan`), F-B13 (`GbSchedIR`), F-B15
(`Backend`). Those consumers assume the single-window invariants
*hold by typing* — they do not re-check. If F-B9 / F-B10 produce a
plan that violates the invariants, every downstream stage is
implicitly compromised.

By making the invariants constructive (typing + gate-or-fail), we
get the strongest possible guarantee at the F-B9 / F-B10 boundary:
**a typed plan cannot violate the invariants by construction, and the
constructor only succeeds when no Hard diagnostic was triggered.**
Any successful plan satisfies the invariants. This is the shift-left
discipline at its purest.

## 11. Report schemas

### 11.1 `sram_plan.v1`

Public JSON shape:

```json
{
  "schema": "sram_plan.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed" | "Failed",
  "report_self_hash": "sha256:...",
  "identity": {
    "schema_version": "1.0.0",
    "determinism": "BitExact" | "NumericallyStable" | "SeedStable" | "DistributionStable",
    "target_profile_id": "...",
    "runtime_mode": "Default" | "Trace" | "Bringup" | "Recovery",
    "audit_parents": {
      "artifact_validation_self_hash": "sha256:...",
      "policy_resolution_self_hash": "sha256:...",
      "static_budget_self_hash": "sha256:...",
      "quant_graph_self_hash": "sha256:...",
      "infer_ir_self_hash": "sha256:...",
      "observation_plan_self_hash": "sha256:...",
      "range_plan_self_hash": "sha256:...",
      "storage_plan_self_hash": "sha256:..."
    },
    "input_identity": {
      "runtime_chrome_budget_hash": "sha256:..." | null,
      "target_profile_hash": "sha256:..."
    }
  },
  "result": {
    "product": {
      "active_sets": [
        {
          "epoch": 0,
          "op_range": { "first_anchor": "...", "last_anchor": "..." },
          "bindings": [
            { "binding": 0, "residency_role": "PersistentSequenceState" }
          ],
          "bytes_in_use": 4096,
          "bytes_reserved": 4192,
          "commit_boundaries_in_range": [0]
        }
      ],
      "page_bindings": [
        {
          "id": 0,
          "page": { "bank": 1 },
          "kind": {
            "Persistent": {
              "kind": "SequenceState",
              "page_state_machine_ref": "..."
            }
          },
          "commit_groups": ["..."],
          "alias_classes": [0],
          "durability": "Critical",
          "generation_strategy": "DoubleBuffered"
        }
      ],
      "commit_boundaries": [
        {
          "id": 0,
          "before_epoch": 0,
          "after_epoch": 1,
          "commit_group": "...",
          "generation_delta": 1,
          "durability_class": "Critical",
          "member_pages": [0],
          "manifest_binding": 0,
          "serialization_order": [0],
          "yield_safe": "YieldOnlyAfterManifest"
        }
      ],
      "page_rotations": [
        {
          "at_epoch_boundary": [0, 1],
          "from": { "bank": 1 },
          "to": { "bank": 2 },
          "triggered_by": { "CommitGroup": { "commit_boundary": 0 } }
        }
      ],
      "spill_policy": {
        "default_residency": { "SpillToSram": { "class": "DedicatedSpillPage" } },
        "persist_manifest_residency": "SamePageAsLastMember",
        "cold_spill_residency": { "BoundedColdSpill": { "max_pages": 2 } },
        "preference_order": "..."
      },
      "projections": {
        "projected_sram_page_switches_per_token": 3,
        "upper_bound_per_token": 3,
        "per_phase": [
          { "epoch": 0, "switches": 0 },
          { "epoch": 1, "switches": 1 }
        ],
        "source": "StaticEnumerationAtNodeAnchorBoundaries"
      }
    },
    "diagnostics": []
  }
}
```

#### Allowed nullable fields in `sram_plan.v1`

```text
identity.input_identity.runtime_chrome_budget_hash
result   (when outcome = Failed; product/diagnostics swap places)
```

No other nullable fields. `null` is rejected by the semantic validator
elsewhere.

#### Semantic validator

```text
ValidateSramPlanReport(report):
  if report.outcome = Passed:
    assert report.result.product is present
    assert report.result.diagnostics has no Hard entries
    apply F-SPP-Total, F-SPP-CommitContiguity, F-SPP-CommitOrdered,
          F-SPP-SerializationCanonical, F-SPP-EpochCoverage,
          F-SPP-PersistKindMatch, F-SPP-WorkingSetByteFit,
          F-SPP-SpillTotal, F-SPP-CapsHonored,
          F-SPP-SinglePageVisibility, F-SPP-NoColdSpillUnlessAllowed,
          F-SPP-ManifestResidency
  if report.outcome = Failed:
    assert report.result is None or carries an error envelope
    assert report.result.diagnostics has at least one Hard entry
    assert no Soft diagnostics
  validate report_self_hash via round_trip_self_hash
  validate canonical JSON shape (lex keys, no unknown fields, no NaN)
```

### 11.2 `sram_cert.v1`

The certificate is a small, machine-checkable artifact under
`certs/sram.cert.json` that other tools (F-F2 Certificates, F-C3
ScheduleOracle harness, F-B16 refinement loop) can audit without
parsing the full plan report.

Public JSON shape:

```json
{
  "schema": "sram_cert.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed",
  "report_self_hash": "sha256:...",
  "claim": {
    "sram_plan_self_hash": "sha256:...",
    "single_page_invariant_holds": true,
    "all_persists_resolved": true,
    "all_sram_paged_resolved": true,
    "spill_policy_total": true,
    "commit_groups_contiguous": true,
    "page_switches_per_token": 3,
    "page_switches_cap": 8,
    "page_switches_per_token_within_cap": true,
    "isr_persists_yield_safe": true
  },
  "evidence": {
    "active_set_count": 5,
    "page_binding_count": 7,
    "commit_boundary_count": 3,
    "page_rotation_count": 4,
    "persistent_kind_distribution": {
      "SequenceState": 2,
      "Continuation": 1,
      "Transcript": 1,
      "Harness": 0,
      "Trace": 0
    }
  }
}
```

The certificate carries the *summary* of the plan: it does not
re-emit the full binding map, but it records the boolean claims
F-F2 / F-C3 / F-B16 audit. Self-hash applies to the certificate
itself.

#### Failed-certificate policy

`sram.cert.json` is emitted **only** on success. On failure,
`sram_plan.json` is emitted with `outcome = Failed`, carrying the
diagnostics; no certificate is emitted. This matches the
`certs/range.cert.json` and `certs/arena.cert.json` discipline from
planv0.md line 2825.

### 11.3 `rom_window_plan.v1`

Public JSON shape:

```json
{
  "schema": "rom_window_plan.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed" | "Failed",
  "report_self_hash": "sha256:...",
  "identity": {
    "schema_version": "1.0.0",
    "determinism": "...",
    "target_profile_id": "...",
    "runtime_mode": "...",
    "audit_parents": {
      "artifact_validation_self_hash": "sha256:...",
      "policy_resolution_self_hash": "sha256:...",
      "static_budget_self_hash": "sha256:...",
      "quant_graph_self_hash": "sha256:...",
      "infer_ir_self_hash": "sha256:...",
      "observation_plan_self_hash": "sha256:...",
      "range_plan_self_hash": "sha256:...",
      "storage_plan_self_hash": "sha256:...",
      "sram_page_plan_self_hash": "sha256:..."
    },
    "input_identity": {
      "runtime_chrome_budget_hash": "sha256:..." | null,
      "target_profile_hash": "sha256:..."
    }
  },
  "result": {
    "product": {
      "kernel_residency": [
        { "kernel": "...", "residency": "Bank0Fixed" }
      ],
      "lut_residency": [
        { "lut": "...", "residency": "Bank0Inline" }
      ],
      "rom_window_bindings": [
        {
          "id": 0,
          "epoch": 0,
          "visibility": {
            "bank0": "Visible",
            "switchable": null
          },
          "assigned_kernels": ["..."],
          "assigned_luts": ["..."],
          "assigned_tensors": [],
          "closure": null
        }
      ],
      "residency_epochs": [
        {
          "id": 0,
          "op_range": { "first_anchor": "...", "last_anchor": "..." },
          "rom_window_binding": 0,
          "sram_page_binding": null,
          "overlay_state": "NoOverlayActive",
          "yield_kind": "NoYieldsExpected"
        }
      ],
      "co_resident_closures": [],
      "overlay_demand": {
        "kernels": [],
        "luts": [],
        "total_overlay_bytes": 0,
        "total_install_count_per_token_upper_bound": 0,
        "share_class_hints": []
      },
      "bank0_demand": {
        "kernels": [
          { "kernel": "...", "byte_size": 1024, "reachability": "IsrReachable" }
        ],
        "luts": [],
        "total_kernel_bytes": 1024,
        "total_lut_bytes": 0,
        "remaining_slack_bytes": 14336
      },
      "projections": {
        "projected_bank_switches_per_token": 0,
        "upper_bound_per_token": 0,
        "per_phase": [],
        "source": "StaticEnumerationAtNodeAnchorBoundaries"
      }
    },
    "diagnostics": []
  }
}
```

#### Allowed nullable fields in `rom_window_plan.v1`

```text
identity.input_identity.runtime_chrome_budget_hash
result   (when outcome = Failed)
result.product.rom_window_bindings[].visibility.switchable
result.product.rom_window_bindings[].closure
result.product.residency_epochs[].sram_page_binding
```

#### Semantic validator

```text
ValidateRomWindowPlanReport(report):
  if report.outcome = Passed:
    assert report.result.product is present
    assert report.result.diagnostics has no Hard entries
    apply F-RWP-Total, F-RWP-IsrBank0, F-RWP-LutIsrBank0,
          F-RWP-SinglePhaseBank, F-RWP-EpochCoverage,
          F-RWP-EpochAlignedWithSPP, F-RWP-ClosureBank,
          F-RWP-Bank0BudgetHonored, F-RWP-OverlayBudgetHonored,
          F-RWP-CapsHonored, F-RWP-NoBank0FixedSwitchableData,
          F-RWP-OverlayKernelNoSwitchableControl,
          F-RWP-CoResidentLegality
  if report.outcome = Failed:
    assert report.result is None or carries an error envelope
    assert report.result.diagnostics has at least one Hard entry
    assert no Soft diagnostics
  validate report_self_hash via round_trip_self_hash
  validate canonical JSON shape (lex keys, no unknown fields, no NaN)
```

### 11.4 `window_cert.v1`

Public JSON shape:

```json
{
  "schema": "window_cert.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed",
  "report_self_hash": "sha256:...",
  "claim": {
    "rom_window_plan_self_hash": "sha256:...",
    "single_window_invariant_holds": true,
    "isr_kernels_in_bank0": true,
    "isr_luts_in_bank0_or_always_resident": true,
    "all_kernels_have_residency": true,
    "all_luts_have_residency": true,
    "co_residency_closures_well_formed": true,
    "bank0_demand_within_slack": true,
    "overlay_demand_within_wram_reservation": true,
    "bank_switches_per_token": 4,
    "bank_switches_cap": 8,
    "bank_switches_per_token_within_cap": true
  },
  "evidence": {
    "kernel_residency_distribution": {
      "Bank0Fixed": 12,
      "WramOverlay": 3,
      "CoResidentSwitchable": 5
    },
    "lut_residency_distribution": {
      "Bank0Inline": 4,
      "WramStaged": 1,
      "RomCoResident": 2
    },
    "co_resident_closure_count": 5,
    "residency_epoch_count": 8,
    "bank0_kernel_bytes": 2048,
    "bank0_lut_bytes": 512,
    "wram_overlay_bytes": 1024
  }
}
```

The certificate carries the boolean claims F-F2 / F-C3 / F-B16 audit
plus distribution evidence. As with `sram_cert.v1`, certificates are
emitted only on success.

### 11.5 Report-emission policy

```text
Stage 7 emits sram_plan.json:
  always (on success and failure)
  whenever audit parents are computable

Stage 7 emits certs/sram.cert.json:
  on success only

Stage 8 emits rom_window_plan.json:
  always (on success and failure)
  whenever audit parents are computable

Stage 8 emits certs/window.cert.json:
  on success only
```

Failure reports record at least one `Hard` diagnostic and have
`outcome = Failed`. Failure reports never carry a product (the
`result.product` field is absent or null per the schema's nullability).

### 11.6 Determinism class binding

Both reports inherit the `DeterminismClass` from `QuantGraph.identity`
verbatim. There is no F-B9-or-F-B10-specific determinism check; the
class is recorded for downstream gates (e.g. F-C2/F-C4 conformance
class-relative equality).

A `BitExact` requirement does not propagate special semantics into
F-B9 / F-B10 because residency is not a numeric decision; the bit-
exact / numerically-stable distinction lives at the IR semantics
layer. We record the class to preserve audit lineage.

## 12. StageCache algebra

### 12.1 Cache key construction

Both keys use the F-B2/F-B4 `DomainHash` rule:

```text
StageCacheKey(K) = DomainHash(
    "gbf-codegen", "StageCacheKey",
    K.schema, K.schema_version,
    CanonicalJson(K)
)
```

### 12.2 Cache miss conditions

For Stage 7:

```text
Cache miss for K7 occurs when any of the following changes:
  - any audit parent self-hash
  - storage_plan_self_hash (most common driver)
  - runtime_chrome_budget_hash
  - target_profile_hash
  - runtime_mode
  - pass_version
  - feature_set
  - schema_version
```

For Stage 8:

```text
Cache miss for K8 occurs when any of the above plus:
  - sram_page_plan_self_hash (i.e. K7 product changed)
```

### 12.3 Cache hit behavior

A cache hit replays the canonical product byte-for-byte. The replay
includes:

* `SramPagePlan` / `RomWindowPlan` product (typed serializable form);
* `sram_plan.json` / `rom_window_plan.json` body bytes (pre-emitted);
* `certs/sram.cert.json` / `certs/window.cert.json` body bytes (on
  success);
* `report_self_hash` (immutable across replay);
* `cert_self_hash` (immutable across replay).

### 12.4 Failure memoization

A failure memo for K7 or K8 records the diagnostics list, the
report's `report_self_hash` (computed for the failure report), and
the input key. It does **not** record a product (because none was
produced).

A failure memo replay is byte-identical to the original failure:
same diagnostics, same `report_self_hash`. CI may disable failure
memoization.

### 12.5 Cross-stage hash plumbing

The `RomWindowPlanCacheKey.sram_page_plan_self_hash` field is the
only cross-stage hash dependency in the K7/K8 algebra. It
ensures that a change in F-B9's product (even if F-B9's inputs
appear to map identically) produces a fresh K8.

A degenerate case: F-B9 produces an empty plan (no SRAM-relevant
bindings). Its `report_self_hash` is still well-defined and stable
across runs; F-B10 still depends on it. This is intentional: the
empty plan is a *commitment* to the empty residency, not an absence
of one.

## 13. Diagnostic algebra

This RFC extends the closed `ValidationCode` enum (F-B2/F-B4 §7.1)
with the codes below. All codes carry typed structured information so
they can be rendered in the standard `<crate>: <severity>: <origin>:
<code>: <detail>` form.

### 13.1 SRAM-* codes (F-B9, origin = SramPagePlanConstruction)

```rust
SramPagePlanInputAuditParentMismatch {
    parent: SramAuditParentName,
    expected: Hash256,
    observed: Hash256,
},
SramPagePlanInputRuntimeModeMismatch {
    declared: RuntimeMode,
    observed: RuntimeMode,
},
SramPagePlanResolutionAmbiguous {
    storage_binding: StorageBindingHandle,
    candidate_count: u32,
},
SramPagePlanResolutionUnresolved {
    storage_binding: StorageBindingHandle,
},
SramPersistKindMismatch {
    binding: SramPageBindingId,
    declared: PersistKind,
    expected: PersistKind,
},
SramWorkingSetExceedsPageSize {
    epoch: EpochId,
    bytes_reserved: u32,
    page_size_bytes: u32,
},
SramMultiplePagesDemandedInEpoch {
    epoch: EpochId,
    pages: NonEmptySet<SramPage>,
},
SramCommitGroupNonContiguous {
    commit_group: CommitGroupId,
    boundaries: Vec<CommitBoundaryId>,
},
SramSerializationOrderInconsistent {
    boundary: CommitBoundaryId,
    declared: Vec<SramPageBindingId>,
    expected_canonical: Vec<SramPageBindingId>,
},
SramPageRotationMissing {
    epoch_boundary: (EpochId, EpochId),
    expected_from: Option<SramPage>,
    expected_to: Option<SramPage>,
},
SramPageRotationFromEqualsTo {
    epoch_boundary: (EpochId, EpochId),
    page: SramPage,
},
SramPageSwitchesPerTokenExceedsCap {
    decision_value: u16,
    upper_bound: u16,
    cap: u16,
    source: SwitchProjectionSource,
},
SramSpillPolicyConflictsBudget {
    policy: SpillPolicySummary,
    budget_max_pages: u8,
    declared_pages: u8,
},
SramManifestResidencyConflict {
    boundary: CommitBoundaryId,
    declared: PersistManifestResidency,
    member_pages: Vec<SramPageBindingId>,
    manifest_binding: SramPageBindingId,
},
SramYieldUnsafeAcrossCommitWindow {
    boundary: CommitBoundaryId,
    declared: YieldSafetyClass,
    reachability: ReachabilityClass,
},
SramPersistentRotationStrategyInfeasible {
    binding: SramPageBindingId,
    strategy: PersistGenerationStrategy,
    page_count_available: u8,
},
SramColdSpillExceedsBudget {
    declared_pages: u8,
    budget_max_pages: u8,
},
SramAliasClassSpansEpochsWithDifferentPages {
    alias: AliasClassId,
    distinct_pages: NonEmptySet<SramPage>,
},
SramPersistKindUnsupportedInRuntimeMode {
    kind: PersistKind,
    runtime_mode: RuntimeMode,
},

pub enum SramAuditParentName {
    StoragePlan,
    ObservationPlan,
    RangePlan,
    StaticBudget,
    PolicyResolution,
    QuantGraph,
    InferIr,
    ArtifactValidation,
}
```

### 13.2 ROM-* codes (F-B10, origin = RomWindowPlanConstruction)

```rust
RomWindowPlanInputAuditParentMismatch {
    parent: RomAuditParentName,
    expected: Hash256,
    observed: Hash256,
},
RomWindowPlanInputRuntimeModeMismatch {
    declared: RuntimeMode,
    observed: RuntimeMode,
},
RomKernelResidencyUnresolved {
    kernel: KernelInstanceId,
    reachability: ReachabilityClass,
    candidate_set: NonEmptySet<KernelResidency>,
},
RomLutResidencyUnresolved {
    lut: LutInstanceId,
    reachability: ReachabilityClass,
    candidate_set: NonEmptySet<LutResidency>,
},
RomMultipleSwitchableBanksDemandedInPhase {
    epoch: EpochId,
    demanded_banks: NonEmptySet<RomBank>,
    contributing_kernels: Vec<KernelInstanceId>,
    contributing_tensors: Vec<TensorMaterializationRef>,
},
RomCoResidencyClosureBankConflict {
    epoch: EpochId,
    closures: NonEmptySet<CoResidentClosureId>,
    banks: NonEmptySet<RomBank>,
},
RomBank0OverBudget {
    total_kernel_bytes: u32,
    total_lut_bytes: u32,
    bank0_reserved_slack: u32,
    over_by_bytes: i64,
},
RomIsrReachableKernelExceedsBank0Slack {
    kernel: KernelInstanceId,
    byte_size: u32,
    bank0_remaining_slack: i64,
},
RomIsrReachableLutNotResident {
    lut: LutInstanceId,
    declared: LutResidency,
},
RomNoLegalKernelResidency {
    kernel: KernelInstanceId,
    reachability: ReachabilityClass,
    bank0_admissible: bool,
    overlay_admissible: bool,
    co_resident_legal: bool,
},
RomBankSwitchesPerTokenExceedsCap {
    decision_value: u16,
    upper_bound: u16,
    cap: u16,
    source: SwitchProjectionSource,
},
RomOverlayDemandExceedsWramReservation {
    declared_bytes: u32,
    wram_reserved_bytes: u32,
},
RomBank0FixedKernelStreamsBankNotMapped {
    kernel: KernelInstanceId,
    epoch: EpochId,
    declared_bank: RomBank,
    visibility_in_epoch: Option<RomBank>,
},
RomCoResidentSwitchableKernelClosureMismatch {
    kernel: KernelInstanceId,
    declared_bank: RomBank,
    closure_bank: RomBank,
},
RomEpochSplitDisturbsSramPagePlan {
    epoch_introduced: EpochId,
    sram_plan_self_hash: Hash256,
},
RomPhaseHasUnreachableKernel {
    epoch: EpochId,
    kernel: KernelInstanceId,
},
RomLutShareClassConflict {
    lut: LutInstanceId,
    declared: OverlayShareClassHint,
    consumer_kernels: Vec<KernelInstanceId>,
},
RomBank0FixedKernelHasSwitchableControlFlow {
    kernel: KernelInstanceId,
    target: KernelInstanceId,
    target_residency: KernelResidency,
},
RomOverlayKernelHasSwitchableControlFlow {
    kernel: KernelInstanceId,
    target: KernelInstanceId,
    target_residency: KernelResidency,
},

pub enum RomAuditParentName {
    SramPagePlan,
    StoragePlan,
    ObservationPlan,
    RangePlan,
    StaticBudget,
    PolicyResolution,
    QuantGraph,
    InferIr,
    ArtifactValidation,
}
```

### 13.3 Severity discipline

All F-B9 / F-B10 codes are emitted at `DiagnosticSeverity::Hard`. The
`Soft` severity remains in the taxonomy (F-B2/F-B4 §7.1) but is
rejected by both Stage 7 and Stage 8 report semantic validators.

### 13.4 Provenance discipline

Every diagnostic carries a non-empty `provenance: Vec<EvidenceRef>`
when an evidence path exists in the inputs (e.g. the offending
`StorageBindingHandle`, `KernelInstanceId`, `EpochId`). Diagnostics
about input validation (audit parent mismatch) carry the relevant
input identity hashes.

### 13.5 Decision-table coverage

Synthetic fixtures must cover every code in §13.1 and §13.2 plus the
following decision-table cells:

| Reachability \ Residency selection | Bank0Fixed | WramOverlay | CoResidentSwitchable |
|---|---|---|---|
| `IsrReachable` | force | reject (`RomIsrReachableLutNotResident` / `RomNoLegalKernelResidency`) | reject |
| `YieldResumeReachable` | force | reject | reject |
| `FaultPathReachable` | force | reject | reject |
| `HarnessEntryReachable` | preferred | admissible (with lease) | admissible (with lease) |
| `BankLeaseProtected` | admissible | admissible | preferred |
| `NormalOnly` | admissible | admissible | admissible |

| Bank 0 fit \ Overlay admissible \ Co-resident legal | Bank0Fixed | WramOverlay | CoResidentSwitchable |
|---|---|---|---|
| Yes / Yes / Yes (preference: Bank0 first) | selected | n/a | n/a |
| No / Yes / Yes | n/a | selected | n/a |
| No / No / Yes | n/a | n/a | selected |
| No / No / No | `RomNoLegalKernelResidency` | n/a | n/a |

| Persist kind \ Page state strategy | DoubleBuffered | SingleBufferedWithCommit | RingBuffered |
|---|---|---|---|
| `SequenceState` | preferred | admissible | admissible |
| `Continuation` | required | reject | reject |
| `Transcript` | admissible | preferred | admissible |
| `Harness` | admissible | preferred | reject |
| `Trace` | admissible | admissible | preferred |

The fixture suite must hit every "preferred" / "selected" cell at
least once and every "reject" cell at least once.

## 14. Cross-stage interactions

### 14.1 Upstream (consumed)

* **F-B2 (`ArtifactValidationAndUpgrade`).** Consumed by hash via
  `audit_parents.artifact_validation_self_hash`.
* **F-B2 (`ResolvedCompilePolicy`).** Consumed for `compile_knobs`,
  cap values, residency preference orders, and the determinism
  class. Recorded in `audit_parents.policy_resolution_self_hash`.
* **F-B4 (`StaticBudgetReport`).** Consumed for projected byte
  totals (already validated; F-B9 / F-B10 do not re-check). Recorded
  in `audit_parents.static_budget_self_hash`.
* **F-B3 (`QuantGraph`).** Consumed transitively via F-B5/F-B6/F-B7/
  F-B8. The identity is recorded for audit (`quant_graph_self_hash`).
* **F-B5 (`GbInferIR`).** Consumed transitively via F-B6/F-B7/F-B8.
  The IR's `NodeAnchor` map is the source of `op_range` references
  in `ResidencyEpoch`. Identity recorded.
* **F-B6 (`ObservationPlan`).** Consumed for `ReachabilityClass` per
  kernel and per LUT. Forces ISR-residency preconditions.
* **F-B7 (`RangePlan`).** Consumed for reduction-site identifiers
  (used only when an SRAM-paged accumulator scratch is selected).
* **F-B8 (`StoragePlan`).** Primary input. Provides every
  `Materialization` decision F-B9/F-B10 turns into a binding.
* **`TargetProfile` (gbf-hw).** Memory-map predicates, MBC5 register
  set, ROM bank count, SRAM bank count, persistent record protocol
  page geometry.
* **`RuntimeChromeBudget` (gbf-policy).** Bank 0 reserved slack,
  WRAM overlay reserved bytes, `max_bank_switches_per_token`,
  `max_sram_page_switches_per_token`, persist bytes per token,
  cold-spill max pages.

### 14.2 Downstream (produced)

* **F-B11 (`OverlayPlan`).** Consumes `WramOverlayDemand` from F-B10
  and `KernelResidency::WramOverlay` / `LutResidency::WramStaged`
  decisions. Turns the demand into an explicit install schedule.
  *Edge contract:* F-B11 must not redefine `WramOverlayDemand`
  shape without amending this RFC.
* **F-B12 (`ArenaPlan`).** Consumes (i) F-B11 overlay regions /
  installs and (ii) F-B9 persistent-page geometry hints. Reservation
  accounting happens in F-B12; F-B9 / F-B10 emit demand totals only.
  *Edge contract:* F-B12 must not redefine `SramPageBinding` or
  `PersistPageId` semantics.
* **F-B13 (`GbSchedIR` + `ResourceStateValidation`).** Consumes
  `ResourceLeaseKind::RomWindow(RomWindowBinding)` and
  `ResourceLeaseKind::SramPage(SramPageBinding)` directly. Slice
  scheduling assigns lease lifetimes against the residency epochs
  emitted here.
  *Edge contract:* F-B13 may not redefine the binding shape without
  amending this RFC.
* **F-B14 (`ScheduleCostAnalysis`).** Consumes
  `projected_bank_switches_per_token` and
  `projected_sram_page_switches_per_token` as cost inputs.
  Per-switch cycle costs come from `gbf-bench` calibration; F-B14
  multiplies. *Edge contract:* F-B14 may not redefine the projection
  source taxonomy without amending this RFC.
* **F-B15 (`Backend`).**
  - `AsmIR` consumes `KernelResidency` to choose section roles.
  - `ReachabilityValidation` consumes ISR-residency declarations as
    its starting set; the proof obligation discharges the claim.
  - `PlacedRom` consumes `RomWindowBinding` constraints to drive
    common-bank vs expert-bank partitioning.
  *Edge contract:* F-B15's `ReachabilityValidation` may only
  *narrow* the set of ISR-reachable code/data F-B6 declared (e.g.
  by proving that some declared-reachable kernel is in fact
  unreachable); it may not *widen* the set in a way that turns a
  Bank0Fixed kernel into a switchable one.

### 14.3 Sister / cross-cutting

* **F-B16 (`FeasibilityRefinementLoop`).** Consumes Hard
  diagnostics from F-B9 / F-B10 as repair targets. Repair surfaces:
  - `RomMultipleSwitchableBanksDemandedInPhase` → split phase, clone
    kernel between Bank 0 and overlay, duplicate LUT.
  - `RomBank0OverBudget` → tighten `KernelInlineThreshold`, hoist
    kernels to overlays.
  - `RomBankSwitchesPerTokenExceedsCap` → coarsen residency, prefer
    `CoResidentSwitchable` over `WramOverlay` (or vice versa).
  - `SramPageSwitchesPerTokenExceedsCap` → adjust `SpillPolicy`,
    re-bank persistent state.
  All knobs F-B9/F-B10 consume are F-B16-defined; this RFC
  references them by name.
* **F-B17 (`StageCache integration sweep`).** Compatible. The K7/K8
  keys defined here use the F-B2/F-B4 `DomainHash` rule and
  participate in the unified `gbf-store` cache.
* **F-A2 (`gbf-hw`).** Memory-map predicates and MBC5 register set
  are consumed unchanged.
* **F-A4 (`BankLease` / `BankGuard` ABI).** F-B13 wires bank-lease
  acquisitions against F-B10's `RomWindowBinding`s; F-B9 / F-B10 do
  not perform the writes. The lease ABI is the single legal path
  to MBC writes (planv0.md line 2051).
* **F-A5 (Bank 0 runtime).** F-B10's `Bank0Demand` budget is
  computed against `RuntimeChromeBudget.bank0_reserved_slack`,
  which the runtime nucleus build (F-A5) emits.
* **F-C2 (`ArtifactOracle`).** Not a consumer of this chunk. F-C2
  evaluates the artifact at the IR level; spatial residency is below
  its concerns.
* **F-C3 (`ScheduleOracle`).** Indirectly consumes F-B13's product
  (which consumes this chunk's products).
* **F-F2 (Certificates).** Consumes `certs/sram.cert.json` and
  `certs/window.cert.json`.

### 14.4 Hash plumbing summary

```text
F-B8 storage_plan_self_hash
   |
   v
F-B9 SramPagePlanInputs.audit_parents.storage_plan_self_hash
   |
   v
F-B9 sram_page_plan_self_hash
   |
   v
F-B10 RomWindowPlanInputs.audit_parents.sram_page_plan_self_hash
   |
   v
F-B10 rom_window_plan_self_hash
   |
   v
F-B11 OverlayPlanInputs (next chunk) consumes
        F-B10.WramOverlayDemand by hash
F-B12 ArenaPlanInputs (next chunk) consumes
        F-B11 reservations + F-B9 page geometry by hash
F-B13 GbSchedIRInputs consumes
        F-B10 binding map + F-B9 binding map by hash
F-B14 ScheduleCostAnalysisInputs consumes
        F-B10 / F-B9 projections by hash
F-B15 BackendInputs consumes
        F-B10 / F-B9 / F-B11 / F-B12 / F-B13 products by hash
```

## 15. Task DAG, compressed

### 15.1 F-B9 task DAG (T-B9.*)

```text
T-B9.0   Wave-0 schema absorption (no upstream stubs needed; all
         types live in gbf-codegen::stages::sram_page;
         gbf-policy::diagnostics extension; gbf-report schema bodies)
            -> T-B9.0a SramWorkingSet, SramPageBinding,
                       SramPageBindingKind, SramScratchClass,
                       PersistGenerationStrategy types
            -> T-B9.0b CommitBoundary, CommitBoundaryId,
                       YieldSafetyClass types
            -> T-B9.0c PageRotation, PageRotationTrigger types
            -> T-B9.0d SpillPolicy, SramSpillClass,
                       PersistManifestResidency, ColdSpillResidency
                       types
            -> T-B9.0e SramSwitchProjections,
                       SwitchProjectionSource (shared with F-B10)
            -> T-B9.0f SramPagePlanProvenance type +
                       NodeAnchorRange type (shared with F-B10)

T-B9.1   build_sram_page_plan_core skeleton (input audit, empty-plan
         path, identity emission)
            depends on: T-B9.0a, T-B9.0b, T-B9.0c, T-B9.0d, T-B9.0e

T-B9.2   epoch boundary derivation from StoragePlan + ObservationPlan
            depends on: T-B9.1, T-B9.0f

T-B9.3   active set / working set construction
            depends on: T-B9.2

T-B9.4   page binding assignment (Persistent kind)
            depends on: T-B9.3

T-B9.5   page binding assignment (Paged + Spill + ManifestOnly kinds)
            depends on: T-B9.3, T-B9.4

T-B9.6   commit boundary construction with serialization order
            depends on: T-B9.4, T-B9.5

T-B9.7   page rotation enumeration
            depends on: T-B9.6

T-B9.8   switch-count projection + cap enforcement
            depends on: T-B9.7

T-B9.9   single-page invariant verification (typed gate + check loop)
            depends on: T-B9.5, T-B9.6, T-B9.7

T-B9.10  sram_plan.json emitter + semantic validator
            depends on: T-B9.1..T-B9.9

T-B9.11  certs/sram.cert.json emitter + semantic validator
            depends on: T-B9.10

T-B9.12  StageCache K7 wiring
            depends on: T-B9.10

T-B9.13  driver run_stage7 (IO + cache write + report emit)
            depends on: T-B9.10, T-B9.11, T-B9.12

T-B9.14  fixture suite covering all SRAM-* diagnostic codes,
         decision-table cells, and reproducibility regenerations
            depends on: T-B9.13

T-B9.15  reviewer review packet under
         docs/review/f-b9-f-b10/sram/
            depends on: T-B9.14
```

### 15.2 F-B10 task DAG (T-B10.*)

```text
T-B10.0  Wave-0 schema absorption (gbf-codegen::stages::window;
         gbf-policy::diagnostics extension; gbf-report schema bodies)
            -> T-B10.0a KernelResidency, LutResidency closed enums
            -> T-B10.0b RomVisibility, RomBankClass,
                        Bank0VisibilityFlag types
            -> T-B10.0c RomWindowBinding, RomWindowBindingId types
            -> T-B10.0d ResidencyEpoch, OverlayState, YieldKindHint
            -> T-B10.0e CoResidentClosure, CoResidentClosureId,
                        TensorMaterializationRef
            -> T-B10.0f WramOverlayDemand, OverlayKernelDemand,
                        OverlayLutDemand, OverlayInstallClass,
                        OverlayShareClassHint
            -> T-B10.0g Bank0Demand, Bank0KernelDemand,
                        Bank0LutDemand
            -> T-B10.0h RomSwitchProjections, RomWindowPlanProvenance
            -> T-B10.0i is_hot_operation predicate

T-B10.1  build_rom_window_plan_core skeleton (input audit, empty plan
         path, identity emission)
            depends on: T-B10.0a..T-B10.0i, T-B9.13 (sram plan exists)

T-B10.2  reachability classification ingestion from ObservationPlan
            depends on: T-B10.1

T-B10.3  kernel/LUT enumeration in canonical order
            depends on: T-B10.1

T-B10.4  KernelResidency assignment with SelectKernelResidency rule
            depends on: T-B10.2, T-B10.3

T-B10.5  LutResidency assignment
            depends on: T-B10.4

T-B10.6  CoResidentClosure construction (union-find)
            depends on: T-B10.4, T-B10.5

T-B10.7  PhaseSwitchableDemand computation per epoch + single-bank
         gate
            depends on: T-B10.6

T-B10.8  RomWindowBinding assignment per epoch
            depends on: T-B10.7

T-B10.9  ResidencyEpoch construction (alignment with F-B9 epochs)
            depends on: T-B10.8

T-B10.10 Bank0Demand computation + slack check
            depends on: T-B10.4, T-B10.5

T-B10.11 WramOverlayDemand computation + share-class hint emission
            depends on: T-B10.4, T-B10.5

T-B10.12 switch-count projection + cap enforcement
            depends on: T-B10.9

T-B10.13 single-window invariant verification (typed gate + check loop)
            depends on: T-B10.7, T-B10.8

T-B10.14 ISR-residency precondition gate (F-IsrBank0, F-LutIsrBank0)
            depends on: T-B10.4, T-B10.5

T-B10.15 rom_window_plan.json emitter + semantic validator
            depends on: T-B10.1..T-B10.14

T-B10.16 certs/window.cert.json emitter + semantic validator
            depends on: T-B10.15

T-B10.17 StageCache K8 wiring
            depends on: T-B10.15

T-B10.18 driver run_stage8 (IO + cache write + report emit)
            depends on: T-B10.15, T-B10.16, T-B10.17

T-B10.19 fixture suite covering all ROM-* diagnostic codes,
         decision-table cells, residency-class × reachability
         coverage, and reproducibility regenerations
            depends on: T-B10.18

T-B10.20 reviewer review packet under
         docs/review/f-b9-f-b10/rom/
            depends on: T-B10.19
```

### 15.3 Cross-task dependencies

```text
T-B9.13   blocks   T-B10.1     (F-B10 inputs need F-B9's product)
T-B9.13   blocks   F-B11.*     (next chunk)
T-B10.18  blocks   F-B11.*
T-B10.18  blocks   F-B12.*
T-B10.18  blocks   F-B13.*
```

### 15.4 Shared infrastructure tasks

Two tasks live in both DAGs because their products are shared:

* `T-Shared.0a` `NodeAnchorRange` type definition in
  `gbf-codegen::canonical::node_anchor` (consumed by F-B6, F-B9, F-B10,
  F-B11, F-B12, F-B13 — but F-B6 owns the seed; this chunk's task is
  to extend with `NodeAnchorRange` if it does not yet exist).
* `T-Shared.0b` `SwitchProjectionSource` enum lives in
  `gbf-policy::projection` (shared between F-B4 / F-B9 / F-B10 / F-B14).

If T-Shared.0a or T-Shared.0b is already provided by an earlier chunk,
this chunk's tasks are no-ops; otherwise they live under T-B9.0e /
T-B10.0i with explicit cross-chunk citations.

## 16. Rejection classes

These are the *categories* of inputs F-B9 / F-B10 reject. Every reject
maps to one or more diagnostic codes from §13. Synthetic fixtures must
exercise each class.

### 16.1 SRAM-side rejections (F-B9)

* **R-SRAM-1: Audit parent mismatch.** Any
  `audit_parents.<X>_self_hash` does not match the corresponding
  upstream stage's report. → `SramPagePlanInputAuditParentMismatch`.
* **R-SRAM-2: Multi-page demand in epoch.** Two SRAM-paged bindings
  active in the same epoch resolve to different pages. →
  `SramMultiplePagesDemandedInEpoch`.
* **R-SRAM-3: Working set exceeds page size.** A working set's
  `bytes_reserved` exceeds the target's `sram_page_size_bytes`. →
  `SramWorkingSetExceedsPageSize`.
* **R-SRAM-4: Commit group non-contiguous.** A `CommitGroupId`'s
  boundaries are not contiguous in `commit_boundaries[]`. →
  `SramCommitGroupNonContiguous`.
* **R-SRAM-5: Switches per token over cap.** Projection exceeds the
  resolved cap. → `SramPageSwitchesPerTokenExceedsCap`.
* **R-SRAM-6: Spill policy conflicts budget.** Cold-spill page demand
  exceeds the budget allowance. → `SramColdSpillExceedsBudget`,
  `SramSpillPolicyConflictsBudget`.
* **R-SRAM-7: Manifest residency conflict.** Declared manifest
  residency does not match the actual placement. →
  `SramManifestResidencyConflict`.
* **R-SRAM-8: Yield unsafe across commit window.** A yield-resume-
  reachable persist write declares `NoYieldDuringCommit` while its
  reachability class admits yields. →
  `SramYieldUnsafeAcrossCommitWindow`.
* **R-SRAM-9: Persistent rotation strategy infeasible.** A binding
  declares `DoubleBuffered` strategy but has only one page available.
  → `SramPersistentRotationStrategyInfeasible`.
* **R-SRAM-10: Alias class spans epochs with different pages.** An
  `AliasClassId` is bound to two distinct pages across epochs. →
  `SramAliasClassSpansEpochsWithDifferentPages`.
* **R-SRAM-11: Persist kind unsupported in runtime mode.** A
  `Trace` persist binding requested in `Recovery` runtime mode. →
  `SramPersistKindUnsupportedInRuntimeMode`.
* **R-SRAM-12: Resolution ambiguous / unresolved.** A storage binding
  has zero or multiple candidate page bindings. →
  `SramPagePlanResolutionAmbiguous`,
  `SramPagePlanResolutionUnresolved`.
* **R-SRAM-13: Persist kind mismatch.** Binding's declared
  `PersistKind` does not match the artifact-stratum kind from
  `StoragePlan`. → `SramPersistKindMismatch`.
* **R-SRAM-14: Page rotation invalid.** Missing rotation, identical
  from/to, or non-canonical serialization order. →
  `SramPageRotationMissing`, `SramPageRotationFromEqualsTo`,
  `SramSerializationOrderInconsistent`.

### 16.2 ROM-side rejections (F-B10)

* **R-ROM-1: Audit parent mismatch.** →
  `RomWindowPlanInputAuditParentMismatch`.
* **R-ROM-2: Multiple switchable banks demanded in phase.** →
  `RomMultipleSwitchableBanksDemandedInPhase`.
* **R-ROM-3: Co-residency closure bank conflict.** Two closures
  with different banks share a phase. →
  `RomCoResidencyClosureBankConflict`.
* **R-ROM-4: Bank 0 over-budget.** Total Bank-0 kernel + LUT bytes
  exceed reserved slack. → `RomBank0OverBudget`,
  `RomIsrReachableKernelExceedsBank0Slack`.
* **R-ROM-5: ISR-reachable LUT not resident.** A LUT marked
  ISR-reachable was assigned a non-Bank-0 residency. →
  `RomIsrReachableLutNotResident`.
* **R-ROM-6: No legal kernel residency.** All three residency
  options were ruled out. → `RomNoLegalKernelResidency`.
* **R-ROM-7: Bank switches per token over cap.** →
  `RomBankSwitchesPerTokenExceedsCap`.
* **R-ROM-8: Overlay demand exceeds WRAM reservation.** →
  `RomOverlayDemandExceedsWramReservation`.
* **R-ROM-9: Bank0Fixed kernel streams unmapped bank.** Kernel
  resides in Bank 0 but its data tensor's bank is not mapped in the
  same epoch. → `RomBank0FixedKernelStreamsBankNotMapped`.
* **R-ROM-10: Co-resident switchable kernel closure mismatch.** A
  kernel claims `CoResidentSwitchable` with bank `b₁` but its
  closure says `b₂`. → `RomCoResidentSwitchableKernelClosureMismatch`.
* **R-ROM-11: Epoch split disturbs SRAM page plan.** F-B10's epoch
  refinement breaks F-B9's epoch alignment. →
  `RomEpochSplitDisturbsSramPagePlan`.
* **R-ROM-12: Phase has unreachable kernel.** A kernel assigned to
  an epoch is unreachable from the IR. →
  `RomPhaseHasUnreachableKernel`.
* **R-ROM-13: LUT share-class conflict.** A LUT declared shared with
  share class `c₁` is consumed by a kernel set whose share class
  policy disagrees. → `RomLutShareClassConflict`.
* **R-ROM-14: Bank0Fixed / Overlay kernel has switchable control
  flow.** Code reaches a kernel target in a switchable bank from a
  Bank-0-fixed or overlay kernel. →
  `RomBank0FixedKernelHasSwitchableControlFlow`,
  `RomOverlayKernelHasSwitchableControlFlow`.
* **R-ROM-15: Kernel/LUT residency unresolved.** A kernel or LUT has
  no candidate residency. → `RomKernelResidencyUnresolved`,
  `RomLutResidencyUnresolved`.

## 17. Proof obligations

These obligations are discharged by the constructors and by the
chunk-level fixture suite. They are the *external* contracts F-B11 /
F-B12 / F-B13 / F-B15 rely on.

### 17.1 PO-W1, PO-W2, PO-S1, PO-S2

(Statements proved in §10.3, §10.4, §10.5, §10.6 respectively.)

### 17.2 PO-V1: visibility tuples consistent across F-B9 / F-B10

(Statement proved in §10.7.)

### 17.3 PO-I1: ISR-residency precondition non-vacuous

(Statement proved in §10.8.)

### 17.4 PO-D1: Determinism under canonical orderings

```text
PO-D1: ∀ pinned inputs i.
        let r₁ = build_sram_page_plan_core(i)
        let r₂ = build_sram_page_plan_core(i)
        in r₁ = r₂  (byte-identical)
       ∀ pinned inputs i'.
        let s₁ = build_rom_window_plan_core(i')
        let s₂ = build_rom_window_plan_core(i')
        in s₁ = s₂  (byte-identical)

Proof: each step of the construction order (§8.3, §9.3) uses
canonical orderings (lex by typed ids). No floating-point math is
performed. No environment-dependent inputs are consulted (e.g. no
clock, no random source). Therefore output is a pure function of
input.
```

### 17.5 PO-C1: Cache key stability

```text
PO-C1: ∀ inputs i, i'.
        if K7(i) = K7(i') then build_sram_page_plan_core(i) =
                              build_sram_page_plan_core(i')
       similarly for K8.

Proof: K7 (resp. K8) records the hash of every input that is
consulted by the constructor. Equal hashes imply equal inputs (by
collision resistance of SHA-256). Equal inputs imply equal outputs by
PO-D1.
```

### 17.6 PO-C2: Coverage on canonical fixture set

```text
PO-C2: ∀ diagnostic code c ∈ §13.1 ∪ §13.2.
        ∃ fixture f such that running build_sram_page_plan_core(f)
          (resp. build_rom_window_plan_core(f)) produces a
          diagnostic with code c.

Proof: enumerated in the fixture suite. Coverage matrix is
machine-checked at chunk closure.
```

### 17.7 PO-RT: Round-trip self-hash

```text
PO-RT: ∀ outputs (sram_plan_report, sram_cert_report,
                  rom_window_plan_report, window_cert_report).
        round_trip_self_hash(report) = Ok(()) for each.

Proof: each report's body uses serde-roundtrippable types,
canonical JSON emission preserves byte order, and the self-hash
computation zeros the field before hashing. The round-trip helper
is shared with F-B2/F-B4 and inherited unchanged.
```

### 17.8 PO-AC: Audit chain validity

```text
PO-AC: ∀ successful build B.
        sram_plan_report.audit_parents.storage_plan_self_hash =
          storage_plan_report.report_self_hash
        rom_window_plan_report.audit_parents.sram_page_plan_self_hash =
          sram_plan_report.report_self_hash
        (similarly for all upstream parents)

Proof: drivers fail-fast on audit-parent mismatch via
SramPagePlanInputAuditParentMismatch /
RomWindowPlanInputAuditParentMismatch.
```

## 18. End-to-end theorem

```text
Theorem F-B9-F-B10-Correctness:
  Suppose:
    validate_artifact_and_request(i)            = Ok(v)              [F-B2]
    resolve_policy(v)                           = Ok(p)              [F-B2]
    build_quant_graph({v, p, ...})              = Ok(q)              [F-B3]
    static_budget({p, q, runtime_budget})       = Ok(b)              [F-B4]
    build_infer_ir({q, p, b})                   = Ok(g)              [F-B5]
    build_observation_plan({g, p})              = Ok(o)              [F-B6]
    build_range_plan({g, o, p})                 = Ok(r)               [F-B7]
    build_storage_plan({g, o, r, p, b})         = Ok(s)              [F-B8]
    build_sram_page_plan({s, o, r, p, b, t, q, g, audit_parents})
                                                = Ok(p_sram)         [F-B9]
    build_rom_window_plan({p_sram, s, o, r, p, b, t, q, g, audit_parents})
                                                = Ok(p_rom)          [F-B10]

  Then:
    1. p_sram is a valid SramPagePlan: F-SPP-Total,
       F-SPP-CommitContiguity, F-SPP-CommitOrdered,
       F-SPP-SerializationCanonical, F-SPP-EpochCoverage,
       F-SPP-PersistKindMatch, F-SPP-WorkingSetByteFit,
       F-SPP-SpillTotal, F-SPP-CapsHonored,
       F-SPP-SinglePageVisibility,
       F-SPP-NoColdSpillUnlessAllowed, F-SPP-ManifestResidency hold.

    2. p_rom is a valid RomWindowPlan: F-RWP-Total, F-RWP-IsrBank0,
       F-RWP-LutIsrBank0, F-RWP-SinglePhaseBank,
       F-RWP-EpochCoverage, F-RWP-EpochAlignedWithSPP,
       F-RWP-ClosureBank, F-RWP-Bank0BudgetHonored,
       F-RWP-OverlayBudgetHonored, F-RWP-CapsHonored,
       F-RWP-NoBank0FixedSwitchableData,
       F-RWP-OverlayKernelNoSwitchableControl,
       F-RWP-CoResidentLegality hold.

    3. The single-window invariants I-RomSingleWindow,
       I-SramSinglePage, and I-Joint hold for every legal trace.

    4. The ISR-residency precondition is non-vacuous:
       every kernel/LUT in {IsrReachable, YieldResumeReachable,
       FaultPathReachable} has Bank 0 / Bank0Inline residency.

    5. p_sram and p_rom are content-addressed and reproducible
       across two consecutive regenerations on hash-bound inputs.

    6. F-B11 (OverlayPlan) may consume p_rom.overlay_demand by hash
       without re-deriving overlay-eligible kernels/LUTs.

    7. F-B12 (ArenaPlan) may consume p_sram.page_bindings and
       p_rom.bank0_demand reservation totals by hash without
       re-deriving page geometry.

    8. F-B13 (GbSchedIR) may consume
       ResourceLeaseKind::RomWindow(b ∈ p_rom.rom_window_bindings)
       and ResourceLeaseKind::SramPage(b ∈ p_sram.page_bindings)
       without re-deriving binding shape.

    9. F-B15 (Backend) may consume ResidencyEpochs as required
       bank-assignment constraints; ReachabilityValidation
       discharges the Bank-0-residency proof against a non-vacuous
       precondition.

  Not proven:
    overlay install schedule          (F-B11)
    arena byte ranges                 (F-B12)
    slice-level scheduling            (F-B13)
    schedule cost envelopes           (F-B14)
    backend reachability proof        (F-B15)
    refinement-loop convergence       (F-B16)
    ScheduleOracle correspondence     (F-C3)
```

## 19. Final concise contract

```text
F-B9 / F-B10 is correct when:

1. SramPagePlan is constructed deterministically from
   StoragePlan + ObservationPlan + RangePlan + ResolvedCompilePolicy +
   RuntimeChromeBudget + TargetProfile + audit-parent identities,
   with every Persist { page, commit_group } and every
   Materialize { class: SramPaged } resolving to exactly one
   SramPageBinding, every CommitGroupId producing a contiguous
   subsequence in commit_boundaries[], every commit boundary's
   serialization_order canonical, and exactly one SpillPolicy.

2. RomWindowPlan is constructed deterministically from
   SramPagePlan + StoragePlan + ObservationPlan + RangePlan +
   ResolvedCompilePolicy + RuntimeChromeBudget + TargetProfile +
   audit-parent identities, with every kernel instance and LUT
   instance assigned exactly one residency class, every co-residency
   closure well-formed, every ISR-reachable kernel/LUT in
   Bank 0 / Bank0Inline residency, every phase having at most one
   switchable bank, and every Bank-0-fixed kernel's switchable-bank
   data dependencies aligned with epoch visibility.

3. The single-window invariants I-RomSingleWindow,
   I-SramSinglePage, and I-Joint hold for every legal trace,
   established by typing (Option<RomBank>, Option<SramPage>) and
   enforced by Hard diagnostic gates (RomMultipleSwitchableBanks-
   DemandedInPhase, SramMultiplePagesDemandedInEpoch).

4. ISR-residency precondition is non-vacuous: every kernel and LUT
   whose ObservationPlan reachability class is in {IsrReachable,
   YieldResumeReachable, FaultPathReachable} resolves to
   Bank0Fixed / Bank0Inline (or WramStaged AlwaysResident for
   LUTs). F-B15 ReachabilityValidation may now discharge a
   non-trivial proof.

5. Bank-switch and SRAM-page-switch projections per token are
   integer counts of epoch-boundary visibility changes; both are
   bounded by resolved policy caps; cap exceedance is a Hard
   diagnostic.

6. WramOverlayDemand summarizes overlay-eligible kernels / LUTs /
   share classes for F-B11; install timing is deferred to F-B11.
   ArenaPlan reservation accounting is deferred to F-B12.

7. CoResidentClosure construction is union-find over the data /
   control / LUT dependency graph; closure membership ⇒ same
   switchable bank; closures whose phases overlap with different
   banks emit RomCoResidencyClosureBankConflict.

8. Spill policy is total per build (one SpillPolicy value), bound
   by RuntimeChromeBudget.cold_spill_max_pages, with manifest
   residency policy applied uniformly to every commit boundary.

9. Both reports (sram_plan.json, rom_window_plan.json) are
   canonical, deterministic, and self-hash-valid; both certificates
   (certs/sram.cert.json, certs/window.cert.json) are emitted on
   success only and carry the boolean claim summary F-F2 audits.

10. StageCache keys K7 and K8 use DomainHash; cache miss occurs on
    pass_version, schema, feature-set, or any audit-parent drift;
    cache hit replays byte-identical canonical product. K8 includes
    sram_page_plan_self_hash so any drift in F-B9's product
    triggers F-B10 re-run.

11. The product reports do not contain RepairProposal provenance or
    AuthorizedRelaxation operations; pure cores
    (build_sram_page_plan_core / build_rom_window_plan_core) are
    isolated from IO drivers (run_stage7 / run_stage8). All audit
    parents propagate by hash.

12. KernelResidency, LutResidency, SpillPolicy, OverlayInstallClass,
    OverlayShareClassHint, PersistGenerationStrategy,
    PersistManifestResidency, ColdSpillResidency,
    SramPageBindingKind, SramScratchClass, SramResidencyRole,
    YieldSafetyClass, OverlayState, YieldKindHint,
    SwitchProjectionSource, RomBankClass are closed enums in v1;
    new variants require RFC amendment.

13. Floating-point JSON values are forbidden in
    sram_plan.v1, sram_cert.v1, rom_window_plan.v1, window_cert.v1
    and Soft diagnostics are rejected by all four semantic
    validators.

14. The "hot operation" predicate is_hot_operation is the single
    source of truth for "which GbInferIR nodes participate in the
    single-window invariants." Any later RFC needing a different
    predicate must amend this RFC.

15. Determinism class is read from QuantGraph.identity.determinism
    and recorded verbatim in both reports without F-B9/F-B10-
    specific class checks.

16. Empty plans (no SRAM-relevant bindings; all-Bank-0 residency)
    are well-typed; reports and certificates still emit; downstream
    consumers handle the empty case via standard Option-typed
    visibility.
```

## 20. Ambiguity ledger

|  ID | Ambiguity | Chosen path in this RFC | Clarifying question | Suggested final decision |
|---|---|---|---|---|
| A1 | planv0.md uses "hot operation" informally throughout. | This RFC pins it to GbInferIR nodes whose StorageBinding is RomConst / SramPaged / Persist and whose enclosing kernel is in the operational set. | Should the predicate be `is_hot_operation` or be split into per-class predicates? | One predicate with closed semantics. |
| A2 | F-B9/F-B10 sequencing under "F-B10 first" interpretation. | F-B9 strictly precedes F-B10; F-B10 consumes p_sram. | Could F-B10 run before F-B9 in degenerate (no-SRAM) builds? | No. F-B9 emits the empty plan in the no-SRAM case; F-B10 still cites the (empty) self-hash. |
| A3 | KernelResidency closed in v1: 3 variants. | Closed: Bank0Fixed, WramOverlay, CoResidentSwitchable. | Could a future variant add HramOverlay? | Possibly in M4+; new variant requires RFC amendment. |
| A4 | LutResidency closed in v1: 3 variants. | Closed: Bank0Inline, WramStaged, RomCoResident. | Could LUTs live in HRAM? | Likely no (HRAM is too tight); if yes, RFC amendment. |
| A5 | OverlayInstallClass: 3 variants. | Closed: AlwaysResident, PerToken, PerEpoch. | Could install be PerSlice (finer than PerEpoch)? | Maybe; F-B11 owns install timing and may extend with amendment. |
| A6 | SpillPolicy is per-build, not per-binding. | One value per build; total. | Could a future profile permit per-binding spill policy? | Not in v1; would force a richer SpillPolicyMap type. |
| A7 | Manifest residency is policy-driven, not boundary-specific. | One policy per build; same applies to every commit boundary. | Could individual commit groups override? | No in v1. |
| A8 | Bank-switch and page-switch projection are static integer counts. | Yes; cycles are F-B14. | Should the projection include a worst-case cycle estimate? | No. Stay integer-only here; F-B14 multiplies by calibration. |
| A9 | F-IsrBank0 is constructive, not just declarative. | F-B10 forces Bank0Fixed at construction; F-B15 proves. | Could F-B10 declare and let F-B15 verify-or-fail? | No. Constructive is shift-left. |
| A10 | Co-resident closure construction is union-find. | Yes. | Could it be a SAT solve instead? | No in v1; union-find is enough for the v1 model topology. |
| A11 | Empty plans are well-typed and emit certificates. | Yes; certificate records empty-plan claim. | Could empty plans skip the certificate? | No. Audit lineage needs a record. |
| A12 | Determinism class is read from QuantGraph and propagated. | Yes; not re-checked here. | Could residency add deterministic-tie-breaking constraints? | Indirectly via KernelResidencyPreferenceOrder; the order is closed. |
| A13 | F-B16 knobs are named-only. | Yes; consumed by reference. | Could F-B16 amend without amending this RFC? | No. New knob shape requires explicit RFC amendment here. |
| A14 | RuntimeMode is out of scope (one plan per (RuntimeMode, BuildIdentity)). | Yes. | Could a single plan span multiple runtime modes? | No. F-B13's SchedulePack wires per-mode plans. |
| A15 | EpochId is shared between F-B9 and F-B10 (F-B9 allocates; F-B10 may extend). | Yes. | Could F-B10 reassign EpochIds? | No. Reassignment would break audit. |
| A16 | SwitchProjectionSource is an enum, not a free string. | Yes; closed enum. | Could the source be runtime-extensible? | No. New sources require RFC amendment. |
| A17 | Bank0VisibilityFlag has only one variant. | Yes; "Visible" only. | Why have a single-variant enum? | Future-proofing if Bank 0 ever gets a partial-visibility mode (e.g. write-protected); explicit closed enum makes amendment paths typed. |
| A18 | Failed reports never carry a product. | Yes; result.product is null/absent. | Could partial products be useful for repair? | No in v1. F-B16 reads diagnostics, not partial products. |
| A19 | Bringup remains a profile, not a relaxation surface. | Yes; per F-B2/F-B4 §2.13. | Could Bringup soften the single-window invariants? | No. The invariants are hard. |
| A20 | sram_cert.v1 and window_cert.v1 are emitted only on success. | Yes. | Could a failure cert record the violations? | No; failure diagnostics live in the plan report. |
| A21 | Persistent state slot count is unconstrained. | F-B5 §2.5a forbids non-empty SequenceSemanticsSpec.state_slots in v1, so SequenceState persists are absent in v1 builds. | When sequence state lands (M4+), do we need a new persist kind? | Maybe SequenceStateV2 if the semantics change; new kind requires RFC amendment. |
| A22 | TargetProfile.sram_bank_count caps SramPage. | Yes. | Could a profile expose more banks than the cartridge supports? | No; gbf-hw enforces. |

## 21. Closing notes

This chunk is the **spatial filter** between value/effect IR
(F-B5) + StoragePlan (F-B8) and concrete byte arenas (F-B12). It is
the place the compiler stops pretending the Game Boy has more than
one switchable ROM bank or one SRAM page mapped at a time, and
starts honoring the consequences.

Two consequences in particular make this chunk non-negotiable:

1. **The contradiction in plan prose** — "shared micro-kernels in
   common banks AND expert-local data in expert banks" cannot both
   be hot defaults — *only* gets resolved here. Without F-B10, the
   contradiction lives in the runtime story as latent bank thrash.
2. **The ISR / yield-resume / fault-path residency rule** can only
   become a proof at F-B15 if F-B10 has constructively assigned
   Bank 0 to every reachable kernel and LUT. Without F-B10,
   `ReachabilityValidation` declares victory over a vacuous
   precondition.

The chunk's surface area is narrow (two stages, two reports, two
certificates, two cache keys) but its contract surface is large
(thirteen self-consistency invariants, eight proof obligations,
twenty-plus rejection classes, and the joint single-window
invariant). The discipline is the F-B2/F-B4 / F-B3/F-B5 discipline,
extended one more turn: **type-the-impossible, gate-the-construction,
prove-by-induction-on-the-trace, fail-fast-with-typed-diagnostics**.

When this RFC is implemented, every later compiler stage stops
inventing residency. Every later runtime debugging session stops
chasing "why did bank N get switched in just before the ISR fired."
Every later optimization stops second-guessing whether a kernel
should live in Bank 0 or an overlay.

That is what this chunk buys.

