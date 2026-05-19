# RFC F-B8: `StoragePlan` (Stage 6) — the bridge

## -1. Authority and amendment policy

This RFC is the source of truth for F-B8 implementation. `history/planv0.md`
remains the architectural context document, but this RFC is allowed to refine,
narrow, or supersede `planv0.md` wherever this RFC makes a more precise
implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence must
be recorded in an `Amends planv0` note close to the relevant decision. This is
not a request to edit `planv0.md` immediately; it is a local source-of-truth
ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B8 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a shared
  surface (canonical JSON rule, self-hash convention, diagnostic envelope,
  StageCache key construction, `ReportEnvelope` shape), the F-B2/F-B4 RFC wins.
  F-B8 inherits those surfaces unchanged unless this RFC explicitly amends
  them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on the shape of
  `QuantGraph` or `GbInferIR` — including the closed `EffectClass` set, the
  closed `InferOp` enum, the `ValueId`/`EffectId`/`NodeId` identity discipline,
  the `SemanticAnchor` definition, the canonical reference semantics of any
  op, or the `quant_graph.v1` / `infer_ir.v1` schemas — the F-B3/F-B5 RFC
  wins. F-B8 consumes those products by hash.
* If this RFC and the forthcoming F-B6/F-B7 RFC disagree on the shape of
  `ObservationPlan` or `RangePlan`, F-B6/F-B7 wins. F-B8 consumes those
  products by hash and never re-derives observation contracts or reduction
  plans.
* If a later RFC changes any public type, report shape, cache key, diagnostic
  code, or canonicalization rule introduced here, that later RFC must
  explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not prose
  folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft (rev 0; pending Gemini + Codex review) |
| Feature beads   | bd-2k0 **F-B8 StoragePlan (Stage 6) — the bridge** |
| Open tasks      | To be minted: T-B8.1..T-B8.N (StoragePlan core types, alias-class equivalence engine, materialization decision rules, persist-binding wiring, storage_plan.json emitter, schema/round-trip tests, StageCache K6 wiring, F-B16 RepairProposal/CompileKnobs handshake, F-B9/F-B10/F-B11/F-B12/F-B13/F-B17 consumption tests) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` §"The compiler pipeline" stage 6; §"Reports and artifacts" `storage_plan.json` (newly defined here); §"Persistent record protocol"; §"Memory plan"; §"Types, passes, and tests: where each invariant lives"; §"Tests" — storage-class / materialization / alias-class tests; ROM window / kernel residency tests insofar as they consume `StoragePlan`'s materialization decisions |
| Glossary        | `history/glossary.md` (artifact stratum, denotational stratum, operational stratum, value/effect IR, sequence semantics, persistence, alias class, storage class, lifetime class, materialization, recompute-vs-spill, persist page, commit group) |
| Constitution    | §I correctness by construction; §II three-stratum oracle correspondence; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ValidatedInputs`, `ResolvedCompilePolicy`, the shared `ReportEnvelope` rule, the canonical JSON rule, the self-hash convention, the StageCache key construction, the `ValidationDiagnostic` envelope); F-B3/F-B5 Canonical IRs (provides `QuantGraph`, `GbInferIR`, `ValueId`, `EffectId`, `NodeId`, `SemanticAnchor`, the closed `EffectClass` set, the closed `InferOp` enum, op-for-op canonical reference semantics); F-B6/F-B7 ObservationPlan + RangePlan (provides `ObservationPlan` and `RangePlan` products consumed here); F-B9 SramPagePlan (consumes `Materialize { class: SramPaged, .. }` and `Persist { page, commit_group }` bindings); F-B10 RomWindowPlan (consumes `Materialize { class: RomConst, .. }` bindings); F-B11 OverlayPlan (consumes overlayable bindings exposed via the shared alias-class lens); F-B12 ArenaPlan (consumes `Materialize { class, lifetime }` and assigns concrete byte ranges); F-B13 GbSchedIR + ResourceStateValidation (consumes `AliasClassId` directly to prove resource-state safety); F-B16 FeasibilityRefinementLoop (consumes `RecomputePromotion` proposals emitted by F-B8); F-B17 StageCache integration sweep (cross-cuts the K6 cache key) |
| Sister deps     | bd-3ns (F-B9) — strictly downstream; bd-15n (F-B10) — strictly downstream; bd-3bw (F-B12) — strictly downstream; bd-3ix (F-B16) — strictly downstream, currently BLOCKED on oracle question; bd-32w5 (T-B16.6 FeasibilityRefinementLoop driver) — consumes `RecomputePromotion` proposals from this stage |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B8 sits inside the
compiler-pipeline epic, where that epic sits inside the full project, and
which adjacent chunks' contracts this RFC inherits or honors.

### 0.1 Project at a glance — the eight epics

The gbllm project compiles a tiny language model into an LR35902 ROM that
runs on real Game Boy hardware. The work is split across eight epics
(`planv0.md` §"Workspace skeleton"; bead-side mirror in `Epic *: …` issues):

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
          Defines the three-stratum correspondence relation that proves
          the deployed ROM behaves like the trained model.

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

The training side is a separate epic-level bundle (`bd-1rb` Training-Contract
Revision Pass) that produces the `ArtifactCore` Epic B consumes.

### 0.2 Epic B's anatomy — the 14-stage pipeline plus loop

Epic B (`bd-2bw`) is the compiler. Per `planv0.md` §"The compiler pipeline,"
it has 14 numbered stages bracketed by a **policy/feasibility envelope**, a
**transform pipeline**, and a **reporting envelope**, plus a bounded
`FeasibilityRefinementLoop` that wraps stages 5–11.

```text
Policy / feasibility envelope:
  F-B2  Stages 0, 0.5  ArtifactValidationAndUpgrade + ResolvedCompilePolicy
  F-B3  Stage 1        QuantGraph
  F-B4  Stage 2        StaticBudgetReport

Transformative stages (wrapped by FeasibilityRefinementLoop):
  F-B5  Stage 3        GbInferIR (value/effect IR)
  F-B6  Stage 4        ObservationPlan
  F-B7  Stage 5        RangePlan
  F-B8  Stage 6        StoragePlan ("the bridge")                    ← THIS RFC
  F-B9  Stage 7        SramPagePlan
  F-B10 Stage 8        RomWindowPlan
  F-B11 Stage 8.5      OverlayPlan
  F-B12 Stage 9        ArenaPlan
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation
  F-B14 Stage 11       ScheduleCostAnalysis
  F-B15 Stage 12       Backend (AsmIR + ReachabilityValidation +
                                PlacedRom + EncodedRom)

Cross-cutting:
  F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs
        (BLOCKED on oracle question; consumes RecomputePromotion
         proposals emitted by F-B8)
  F-B17 StageCache integration sweep across all stages
        (uniformization pass; F-B8 wires K6 directly here)
```

Sequencing of ~weekly chunks (bkase 2026-05-07 conversation):

```text
Chunk 1 (LANDED):     F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2 (LANDED):     F-B3 + F-B5         Stages 1, 3
Chunk 3 (in flight):  F-B6 + F-B7         Stages 4, 5
Chunk 4 (THIS RFC):   F-B8                Stage 6
Chunk 5:              F-B9 + F-B10        Stages 7, 8
Chunk 6:              F-B11 + F-B12       Stages 8.5, 9
Chunk 7:              F-B13               Stages 10, 10.5
Chunk 8:              F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop
```

### 0.3 Where F-B8 sits in the pipeline

F-B8 is **the bridge**. It is the first stage in the pipeline that converts
value/effect semantics into materialization, lifetime, and aliasing decisions
without committing to byte offsets, bank residency, overlay install plans,
or slice scheduling. Every later spatial stage (F-B9, F-B10, F-B11, F-B12,
F-B13) consumes its output.

```text
input strata:
  ┌─────────────────────────────────────────────────────────────────────┐
  │  ArtifactCore (frozen, immutable, target-independent)                │
  └─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │  F-B2 Stage 0:    ArtifactValidationAndUpgrade                       │
  │  F-B2 Stage 0.5:  ResolvedCompilePolicy                              │
  │  F-B3 Stage 1:    QuantGraph                                         │
  │  F-B4 Stage 2:    StaticBudgetReport                                 │
  └─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │  F-B5 Stage 3:    GbInferIR    (typed value/effect IR)               │
  │  F-B6 Stage 4:    ObservationPlan                                    │
  │  F-B7 Stage 5:    RangePlan    (ReductionPlan per reduction site)    │
  └─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
                 ╔════════════════════════════════════╗
                 ║   F-B8 Stage 6:    StoragePlan      ║   ← THE BRIDGE
                 ║   ────────────────────────────────  ║
                 ║   for every ValueId in GbInferIR:   ║
                 ║     pick Materialization            ║
                 ║       ∈ { Recompute,                ║
                 ║           Materialize { class,      ║
                 ║                          lifetime}, ║
                 ║           Persist { page,           ║
                 ║                     commit_group }} ║
                 ║     pick AliasClassId               ║
                 ║                                     ║
                 ║   honors:                           ║
                 ║     QuantGraph identity             ║
                 ║     GbInferIR shape                 ║
                 ║     ObservationPlan contracts       ║
                 ║     RangePlan reduction structure   ║
                 ║     ResolvedCompilePolicy           ║
                 ║       (StorageKnobs, lifetime caps) ║
                 ║                                     ║
                 ║   produces:                         ║
                 ║     StoragePlan (typed product)     ║
                 ║     storage_plan.json (canonical    ║
                 ║       JSON, self-hashed,            ║
                 ║       ReportEnvelope)               ║
                 ║     STORE-* diagnostics             ║
                 ║                                     ║
                 ║   does NOT produce:                 ║
                 ║     concrete byte offsets           ║
                 ║     bank residency                  ║
                 ║     overlay install plans           ║
                 ║     slice schedules                 ║
                 ║     SRAM page assignments           ║
                 ║     ROM window selections           ║
                 ╚════════════════════════════════════╝
                                  │
                ┌─────────────────┼─────────────────┐
                ▼                 ▼                 ▼
  ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
  │ F-B9  Stage 7    │ │ F-B10 Stage 8    │ │ F-B12 Stage 9    │
  │ SramPagePlan     │ │ RomWindowPlan    │ │ ArenaPlan        │
  │ (consumes        │ │ (consumes        │ │ (consumes        │
  │  SramPaged +     │ │  RomConst        │ │  Materialize     │
  │  Persist)        │ │  bindings)       │ │  bindings; only  │
  │                  │ │                  │ │  here are bytes  │
  │                  │ │                  │ │  assigned)       │
  └──────────────────┘ └──────────────────┘ └──────────────────┘
                                  │
                                  ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │  F-B11 Stage 8.5: OverlayPlan                                        │
  │  F-B13 Stage 10:  GbSchedIR (consumes AliasClassId for resource-     │
  │                              state validation; F-B13 Stage 10.5)     │
  │  F-B14 Stage 11:  ScheduleCostAnalysis                               │
  │  F-B15 Stage 12:  Backend (AsmIR + ReachabilityValidation +          │
  │                            PlacedRom + EncodedRom)                   │
  └─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
                          (CompiledBuild + reports)
```

The bridge metaphor is literal: on the input side, the pipeline carries pure
value/effect semantics with no spatial commitment; on the output side, every
later stage either consumes a `StoragePlan` binding directly or threads it
through to a binding consumer. There is no other place in the pipeline where
the question "is this value materialized, recomputed, or persisted, and if
materialized, what storage class and lifetime?" gets answered.

### 0.4 Cross-epic interactions

F-B8 sits at the intersection of four epics:

```text
Epic A → Epic B
  - gbf-foundation (BlobRef, BlobCodec, Hash256 wrappers)         consumed
  - gbf-store (StageCache) for K6 cache wiring                    consumed
  - gbf-policy (ResolvedCompilePolicy, StorageKnobs,
                RecomputePromotionLevel, CompileKnobBounds,
                CompileKnobOverrides::forced_recompute)           consumed
  - gbf-abi (PersistPageId, CommitGroupId, PersistKind,
             PersistGroupCommit, PageState as schema-only refs)   consumed

Epic B (internal):
  - F-B2 / F-B4 (Stage 0, 0.5, 2) products + ReportEnvelope rule  consumed
  - F-B3 / F-B5 (Stage 1, 3) QuantGraph + GbInferIR products      consumed
  - F-B6 / F-B7 (Stage 4, 5) ObservationPlan + RangePlan products consumed
  - F-B9 / F-B10 / F-B11 / F-B12 / F-B13 (Stages 7..10)           feeds
  - F-B16 RepairProposal::PromoteRecomputeLevel                   feeds
  - F-B17 StageCache cross-cut (K6 key)                           compatible

Epic C → Epic B (oracle correspondence):
  - F-C2 ArtifactOracle is unaffected: StoragePlan is below the
    canonical-IR stratum and is not part of artifact-stratum
    correspondence.
  - F-C3 ScheduleOracle is unaffected directly. F-C3 evaluates GbSchedIR,
    which carries forward StoragePlan's AliasClassId and Materialization
    via op-level resource-state evidence. F-B8's contribution to F-C3 is
    that two GbSchedIR programs that differ only in StoragePlan's
    refinement choices may still be ScheduleOracle-equivalent at
    SemanticCheckpointId boundaries; the equality witness is owned by
    F-C3 and out of scope here.

Epic D → Epic B:
  - gbf-runtime::persistence (page rotation, header layout, CRC
    verification) is the runtime consumer of Persist { page,
    commit_group } bindings. F-B8 emits PersistPageIds and
    CommitGroupIds plug-compatible with that runtime; the runtime
    contract (PersistHeader, PersistGroupCommit, PageState) is
    consumed by hash from gbf-abi.

Epic F → Epic B:
  - gbf-report consumes storage_plan.json as one of the build report
    products listed in BuildReports (planv0.md line 1985, line 2792).
    BuildReports is owned by F-B14/F-B17, not by this RFC.
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," F-B8 is the load-bearing piece of M3:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup. Merged: c2edbaa.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; first conformance.json; first CompileRequest
              wiring.
              ↳ F-B8 is NOT required for M1. M1's quantized dense kernel
                lives below the routed-FFN cutover; it can be compiled
                with a degenerate StoragePlan that materializes every
                non-recomputable value to WramHot or RomConst with
                LifetimeClass::Slice. The degenerate plan is a valid
                v1 StoragePlan, so the typed surface lands in M1; the
                rich decision rules (§9) come into their own at M3.

M2            One shared micro-kernel resolved by RomWindowPlan; one
              expert payload bank; emulator diffing against
              ScheduleOracle; first ReachabilityValidation pass.
              ↳ F-B8 is required for M2: RomWindowPlan (F-B10) cannot
                run without StoragePlan's RomConst bindings, and the
                shared micro-kernel commitment relies on StoragePlan
                pinning a single RomConst materialization for the
                kernel's weights and LUTs. RecomputePromotion is not
                yet exercised at M2 (RecomputePromotionLevel::None
                is the default), but the typed surface must be present.

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ F-B8 is M3's load-bearing commitment. The "wired
                end-to-end" phrase is satisfied by:
                  - one StoragePlan per build,
                  - per-ValueId Materialization decisions backed by
                    typed predicates,
                  - alias-class equivalence proven non-overlapping
                    in lifetime,
                  - per-layer Persist bindings for the routed-FFN
                    sequence-state slot family,
                  - LifetimeClass::ResumeWindow values exposed for
                    F-B13 cooperative scheduling.

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ F-B8 already carries the typed Persist binding shape
                so M4's persistence work is binding consumption, not
                schema invention. The dispatch from BoundedKv to
                CommitGroupId families is owned by F-B9 / F-D-prefix
                runtime work, not by this RFC.
```

This chunk is therefore the **bridge into M3**: until F-B8 lands, the
spatial stages (F-B9..F-B13) can be type-checked against a placeholder
shape but cannot run on real artifacts. Once F-B8 lands, the spatial
stages become a sequence of byte-budget refinements over a stable
typed-binding skeleton.

### 0.6 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later spatial stage receives a typed, validated `StoragePlan`. They
  never re-derive `Materialization`, `AliasClassId`, `LifetimeClass`,
  `StorageClass`, `PersistPageId`, or `CommitGroupId`.
* F-B9 (`SramPagePlan`) consumes `Materialize { class: SramPaged, .. }` and
  `Persist { page, commit_group }` bindings; it never re-decides that a
  value is paged. It only decides which page family hosts it, what the
  active working set is, and when commits happen.
* F-B10 (`RomWindowPlan`) consumes `Materialize { class: RomConst, .. }`
  bindings; it never re-decides that a kernel or expert weight payload is
  ROM-resident. It only decides Bank 0 vs switchable vs WRAM overlay
  residency.
* F-B11 (`OverlayPlan`) consumes overlayable bindings via the alias-class
  lens and decides regions, install schedules, and shared bank state.
* F-B12 (`ArenaPlan`) consumes `Materialize { class, lifetime }` and is
  the first stage that assigns concrete byte ranges. It never allocates
  byte ranges to pure expression nodes that F-B8 marked
  `Materialization::Recompute`.
* F-B13 (`GbSchedIR` + `ResourceStateValidation`) consumes `AliasClassId`
  directly to prove resource-state safety. The alias-class equivalence
  relation pinned by F-B8 is the resource-aliasing surface F-B13 reasons
  against.
* F-B16 (`FeasibilityRefinementLoop`) consumes `RecomputePromotion`
  repair proposals emitted by F-B8 and applies them under
  `CompileKnobs::storage::recompute_promotion`.
* F-B17 (`StageCache` integration sweep) wires the K6 cache key pinned
  in §13 below.

This chunk's job is to retire the **materialization, lifetime, persistence,
and aliasing** preconditions of the rest of the spatial pipeline. It is the
first stage to commit to spatial structure without committing to byte
offsets — a semantic position that is distinct from any other stage.

### 0.7 What the project as a whole gains when this chunk lands

```text
1. Recompute-vs-spill becomes an explicit decision.
   In every prior compiler design we have seen for a memory-constrained
   target, the recompute-vs-spill choice is a side effect of buffer
   lowering. F-B8 makes it a typed first-class choice with positive
   evidence. RecomputePromotionLevel feeds F-B16's bounded refinement
   loop, so the decision is auditable and revisable.

2. Schedule equivalence becomes definable.
   Two GbSchedIRs that differ only in StoragePlan's recompute-vs-spill
   choices on pure values are ScheduleOracle-equivalent at
   SemanticCheckpointId boundaries by construction. This is the
   foundation for F-C3's correspondence proof.

3. Alias classes become a first-class typed surface.
   Resource-state safety in F-B13 (ResourceStateValidation) is a
   property over alias classes, not over byte ranges. Pinning the
   alias-class equivalence relation here means F-B13's invariant proofs
   are local to the schedule, not global to the layout.

4. Persistent record protocol becomes plug-compatible with the runtime.
   Persist { page, commit_group } binds compiler-side persistent state
   to runtime-side PersistPageId / CommitGroupId without committing to
   byte ranges. The runtime persistence module (gbf-runtime::persistence)
   can rotate, commit, and validate pages without consulting the
   compiler. This is the cleanest possible compiler/runtime contract for
   battery-backed SRAM.

5. The byte-allocation cliff is delayed to ArenaPlan.
   Before F-B8, "memory" appears in the pipeline as ArtifactCore byte
   sizes (in StaticBudgetReport) and as runtime chrome budget envelopes.
   After F-B8, the abstract memory geometry — how many distinct values
   are simultaneously live, what their lifetime classes are, which
   storage class they require — is fully pinned. ArenaPlan's job is then
   purely about packing within that geometry. This separation is what
   makes the late stages testable in isolation.

6. The "canonical IR" discipline now extends across the bridge.
   The schema/canonicalization/self-hash/StageCache pattern from
   F-B2/F-B4 and F-B3/F-B5 now extends to storage_plan.v1. F-B9 / F-B10
   / F-B11 / F-B12 inherit the same discipline.

7. The first refinement-loop hook is wired.
   StoragePlan is the first stage in the pipeline whose decisions are
   monotonically refinable by F-B16 (RecomputePromotionLevel only
   advances). The typed RepairProposal handshake is exercised here for
   the first time, even though F-B16 itself remains BLOCKED on the
   oracle question.
```

### 0.8 Reading order for reviewers

A reviewer who has just read F-B3/F-B5 and is approaching this RFC for the
first time should read:

```text
§-1 Authority and amendment policy
§0  (this section) — placement and dependencies
§0a TL;DR
§1  Project context — what's left after F-B7 and why F-B8 is "the bridge"
§2  Load-bearing decisions — the engineering choices that bracket the rest
§5  Authority rules — what this RFC owns vs inherits
§6  Pipeline state machine — how Stage 6 plugs into Stages 5 and 7
§8  Stage 6 contract: StoragePlan
§9  Decision rules / heuristics layer (typed)
§10 Persistence binding
§11 Aliasing model
§12 Report schema (storage_plan.v1)
§14 Diagnostic algebra (STORE-*)
§15 Cross-stage interactions
§16 Task DAG, compressed
§19 End-to-end theorem
§20 Final concise contract
```

Skim §3, §4, §7, §13, §17, §18 for specifics.

## 0a. TL;DR

This chunk lands the **bridge** between value/effect semantics and spatial
scheduling. It owns one numbered stage:

* **Stage 6 — `StoragePlan`.** The bridge. For every `ValueId` declared in
  `GbInferIR`, choose exactly one `Materialization`:

  ```text
  Materialization :=
      Recompute
    | Materialize { class: StorageClass, lifetime: LifetimeClass }
    | Persist     { page: PersistPageId, commit_group: CommitGroupId }
  ```

  Pin a `StorageClass ∈ { WramHot, HramHot, SramPaged, RomConst }` and a
  `LifetimeClass ∈ { Slice, ResumeWindow, Token, Session, Persistent }`
  for materialized values. Bind a `PersistPageId` and `CommitGroupId` for
  persistent values without committing to byte ranges. Pin an
  `AliasClassId` per binding so F-B13's `ResourceStateValidation` has a
  typed equivalence relation to reason against.

  No tile sizes, no buffer addresses, no concrete byte offsets, no bank
  residency, no overlay install schedule, no slice scheduling. Comparable
  alias-class lens against `GbSchedIR` because both share the same
  `AliasClassId` pinning.

This feature ships as one bead (`bd-2k0`) because the natural unit is "the
bridge from value/effect semantics to spatial scheduling." Splitting it into
multiple features would either fragment the alias-class relation or make
recompute-vs-spill a side effect of a different stage, and both fragmentations
re-create exactly the failure mode F-B8 exists to prevent. The rich knob
surface (storage knobs, recompute promotion, materialization overrides) is
already pre-pinned by F-B2/F-B4 inside `CompileKnobs`, so F-B8 only has to
*honor* those knobs, not invent them.

The chunk shares the diagnostic envelope, JSON canonicalization rule,
self-hash convention, and `StageCache` key construction inherited from
F-B2/F-B4. The product (`StoragePlan`), the report (`storage_plan.json`),
and the `StageCache` key (K6) all follow F-B3/F-B5's pattern: typed product
+ canonical-JSON envelope + content-addressed cache key.

The chunk closes only when:

1. `StoragePlan` construction is a deterministic pure function of
   `(QuantGraph, GbInferIR, ObservationPlan, RangePlan,
   ResolvedCompilePolicy)` and is byte-identical across two consecutive
   regenerations on a clean checkout.
2. `storage_plan.json` round-trips through its semantic validator and
   self-hash.
3. The alias-class equivalence relation is provably reflexive, symmetric,
   and transitive on the StoragePlan product, with no two values in the
   same alias class having overlapping lifetimes that admit conflicting
   writes (§11).
4. Every `Materialization::Persist` binding's `(page, commit_group)` is
   plug-compatible with the SRAM persistence protocol: every
   `PersistPageId` resolves to one of the runtime-defined `PersistKind`s,
   every `CommitGroupId` references at least one binding, and the
   commit-group well-formedness invariants of §10 hold.
5. Every `RangePlan::RenormLoop { tile_len }` reduction site has a
   `StoragePlan`-bound scratch materialization with `LifetimeClass::Slice`
   or stricter (§9).
6. F-B16's `RecomputePromotion` repair proposal handshake is wired
   (proposal-only, no acceptance until F-B16 lands).
7. `StageCache` key K6 is pinned and tested.
8. The fixture build emits a `StoragePlan` for the routed-FFN fixture
   that satisfies every typed predicate in §9 and every persistence
   invariant in §10 without manual override, and a hand-crafted minimal
   degenerate fixture that exercises only `Materialize`-only decisions.

The chunk does **not** include:

* **Concrete byte offsets** — owned by F-B12 (`ArenaPlan`, Stage 9).
* **Bank residency** (Bank 0 fixed vs switchable vs WRAM overlay) —
  owned by F-B10 (`RomWindowPlan`, Stage 8).
* **Overlay install plans** (regions, share classes, install schedules)
  — owned by F-B11 (`OverlayPlan`, Stage 8.5).
* **SRAM page assignments** (which page family hosts a binding, which
  pages share a working set, when commits happen) — owned by F-B9
  (`SramPagePlan`, Stage 7).
* **Slice scheduling** (yield boundaries, resource leases, slice cycles)
  — owned by F-B13 (`GbSchedIR`, Stage 10).
* **`SemanticCheckpointId` attachment** — owned by F-B6 (`ObservationPlan`,
  Stage 4); F-B8 consumes the attached anchors but never adds new ones.
* **`ReductionPlan` selection** — owned by F-B7 (`RangePlan`, Stage 5);
  F-B8 honors `RangePlan` but never alters reduction structure.
* **`RepairPolicy` / `CompileKnobs` mutation** — F-B8 emits
  `RepairProposal`s; only F-B16's loop driver mutates `CompileKnobs`.
* **Bank-aware kernel duplication** — owned by F-B10 / F-B11.
* **Persist-page byte layout, header CRC, page rotation** — owned by
  `gbf-runtime::persistence` (Epic D); F-B8 binds `PersistPageId` and
  `CommitGroupId` schema-only.

The bridge metaphor is the load-bearing image: F-B8 stands between the
storage-free, place-less typed IRs (`QuantGraph`, `GbInferIR`,
`ObservationPlan`, `RangePlan`) and the spatial stages that commit to
banks, windows, overlays, byte ranges, and schedules. Crossing the bridge
without F-B8 either over-commits (by inlining storage choices into
`GbSchedIR`) or under-commits (by leaving alias-class safety to byte-range
inference). Both failure modes are exactly what F-B8 forbids by being a
named typed stage.

## 1. Project context — what's left after F-B7 and why F-B8 is "the bridge"

### 1.1 What F-B2/F-B4 leaves on the table

Per the F-B2/F-B4 RFC, by the time this chunk begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`,
  `TargetDataLoweringArtifact`, calibration, hint bundle, and
  `CompileRequest` are all admissible, hash-bound, and traceable through
  `artifact_validation.json`.
* `ResolvedCompilePolicy` is the single answer to "what policy governed this
  build," with provenance for every load-bearing scalar. `CompileKnobs`
  is wired but never repaired in this chunk's path; F-B16 owns repair
  acceptance.
* `RuntimeChromeBudget` is honored at the static byte-math level. F-B4 has
  already emitted `static_budget.json` and proven that the artifact's
  per-bank, per-expert byte math fits the resolved chrome budget under the
  selected `PlacementProfile`.

F-B8 consumes the following already-resolved policy fields. Any field
not listed here is not available to Stage 6 in v1:

```text
CompileKnobs.global.storage.recompute_promotion
CompileKnobs.global.storage.recompute_cycle_ceiling
CompileKnobs.bounds.max_recompute_promotion
CompileKnobOverrides.forced_recompute
RuntimeChromeBudget.wram_hot
RuntimeChromeBudget.hram_hot
StoragePressureBudget.wram_hot.soft_bytes
StoragePressureBudget.hram_hot.soft_bytes
TraceCapturePolicy.enabled_probes
TranscriptCapturePolicy.enabled
BuildProfile.kind                      -- only if DR-5 harness path remains
```

The following surfaces are intentionally not available to F-B8 v1 and
must not be referenced by decision rules until a later RFC amends this
one:

```text
storage_class_override
overlay_excluded_set
OverlayRegionSizeCeiling
kernel.staged_lut_fragments
trace_demotion.level
```

`RecomputePromotionLevel` ranges over `{ None, PureSliceValues }` in
storage_plan.v1. Wider recompute levels are reserved for a later RFC
that can make schedule-boundary evidence available before Stage 6.
`CompileKnobOverrides.forced_recompute` is a `BTreeSet<ValueSelector>`
where `ValueSelector := Value(ValueId) | AliasClass(AliasClassFingerprint)`
(see §11.7 for the fingerprint definition).

F-B8 does not invent a knob; it consumes these typed knobs.
`forced_recompute` keys against `ValueId` and `AliasClassFingerprint` — both are
F-B5/F-B8 types — so the override surface is a pin against shapes this
chunk owns.

### 1.2 What F-B3/F-B5 leaves on the table

Per the F-B3/F-B5 RFC, by the time this chunk begins, the following hold:

* `QuantGraph` is a typed canonical artifact graph: frozen canonical
  tensors, explicit quant formats, explicit `NormPlan`s, optional
  `RoutingTable`, explicit `ExpertSection`s, explicit `DecodeSpec`,
  explicit `SequenceSemanticsSpec`, and complete provenance back to
  exported tensor ids. F-B8 reads it for tensor identity, expert-section
  topology, sequence-state slot declarations, and routing presence.
* `GbInferIR` is a typed value/effect IR with:
  - `ValueId`s — every value edge in the IR has a unique identifier;
  - `EffectId`s — every effect chain has a unique identifier; `EffectClass`
    is closed at `{ SequenceState(StateSlotId), Rng(RngSlot),
    FaultBoundary }` in v1;
  - `NodeId`s — every IR node has a unique identifier;
  - `SemanticAnchor`s — every node has a hash-derived anchor that F-B6
    attaches checkpoints to;
  - `ValueProducerRef::Node | External` — every value has a typed
    producer reference;
  - `ValueDecl.kind`, `ValueDecl.format` — every value declaration carries
    a typed `ValueKind` and `QuantFormat`.

F-B8 binds against `ValueId` (one binding per `ValueId`, no coverage gaps,
no double-bind). It reads `ValueDecl.kind` and `ValueDecl.format` to make
storage-class decisions. It consumes `EffectId`s for `SequenceState` to
identify persistent values. It consumes `NodeId`s only as evidence in
diagnostics; it does not bind nodes.

### 1.3 What F-B6/F-B7 leaves on the table

Per the (in-flight) F-B6/F-B7 RFC, by the time this chunk begins, the
following hold (forward-referenced; the dependency on F-B7 is hard, not
soft, and is recorded as `bd-2k0 -> bd-2x0`):

* `ObservationPlan` declares which `SemanticCheckpointId`s are mandatory
  for the active build, which optional `TraceProbeId`s are armed, and
  which `MetricProbe`s are sampled. The plan is attached to
  `GbInferIR.anchors` via `NodeAnchorMap`.
* `RangePlan` declares one `ReductionPlan` per hot reduction site:

  ```text
  ReductionPlan :=
      SingleI16
    | ChunkedI16   { chunk_len: u16 }
    | RenormLoop   { tile_len:  u16 }
  ```

  per `planv0.md` lines 1655–1660. The plan is attached to `GbInferIR`
  by `(NodeId, ReductionSiteId)`.

F-B8 honors both plans. From `ObservationPlan`, it learns which values
back observed checkpoints (those values' lifetime classes are bounded
below by the checkpoint's required-stable-window). From `RangePlan`, it
learns which scratch tensors a `RenormLoop` reduction needs (those values
require materialization with `LifetimeClass::Slice`-or-stricter scratch).

If F-B7 ships before this chunk closes, the consumption is direct. If
F-B7's exact shape is still in flight, F-B8 consumes a `RangePlanView`
trait that exposes:

```rust
fn reduction_plan_for(site: ReductionSiteRef) -> Option<ReductionPlan>;
fn scratch_value_ids_for(site: ReductionSiteRef) -> &[ValueId];

pub struct ReductionSiteRef {
    pub node: NodeId,
    pub site: ReductionSiteId,
}
```

The placeholder
strategy mirrors F-B4's handling of `QuantGraphBudgetSource` from before
F-B3 landed.

### 1.4 What M2/M3 commits to and how this chunk delivers it

Per `planv0.md` §"Milestones":

> **M2**: one shared micro-kernel resolved by `RomWindowPlan`, plus one
> expert payload bank, with exact emulator diffing against `ScheduleOracle`
> and checkpoint alignment against `ArtifactOracle` at `SemanticCheckpointId`
> boundaries; first `ReachabilityValidation` pass integrated into the
> backend.
> **M3**: top-1 router, expert dispatch table, value/effect `GbInferIR` +
> `ObservationPlan` + `RangePlan` + `StoragePlan` wired end-to-end for a
> routed FFN under the cooperative scheduler.

Mapping:

* M2 commitment "one shared micro-kernel resolved by `RomWindowPlan`"
  requires F-B8. Without `StoragePlan::Materialize { class: RomConst, .. }`
  bindings on the kernel's weights and LUTs, `RomWindowPlan` has no
  RomConst objects to resolve.
* M2 commitment "one expert payload bank" requires F-B8. The expert's
  weight tensors materialize as `RomConst`; the expert's per-token
  activations materialize (potentially) as `WramHot`; the boundary is
  drawn here.
* M3 commitment "value/effect `GbInferIR` + `ObservationPlan` + `RangePlan`
  + `StoragePlan` wired end-to-end" names this RFC explicitly. M3
  is what this RFC delivers.

Because M2 lands before M3, the typed surface of F-B8 must be available
at M2 even if the rich decision rules (recompute promotion, persist
binding, alias-class refinement) are exercised only at M3.

### 1.5 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later spatial stage receives a typed, validated `StoragePlan`
  rather than re-deriving materialization from `GbInferIR`.
* F-B9 (`SramPagePlan`) consumes `Materialize { class: SramPaged, .. }`
  bindings as the universe of paged values to plan over. It never
  re-decides paged-vs-not.
* F-B10 (`RomWindowPlan`) consumes `Materialize { class: RomConst, .. }`
  bindings as the universe of ROM-resident objects to assign to banks.
  It never re-decides RomConst-vs-not.
* F-B11 (`OverlayPlan`) consumes overlayable bindings (those with
  `LifetimeClass` and `StorageClass` compatible with WRAM overlay
  installation; defined in §11.5 of this RFC).
* F-B12 (`ArenaPlan`) consumes `Materialize { class, lifetime }` bindings
  as the universe of byte-range allocands. It never assigns byte ranges
  to `Recompute` values.
* F-B13 (`GbSchedIR` + `ResourceStateValidation`) consumes `AliasClassId`
  as the equivalence-class lens for resource-state safety proofs.
* F-B16 (`FeasibilityRefinementLoop`) consumes `RecomputePromotion`
  proposals from this stage.

This chunk's job is to retire the **materialization, lifetime, persistence,
and aliasing** preconditions of the rest of the spatial pipeline. It is
the fourth shift-left filter in the system, after `gbf-train preflight`,
F-B2 (Stage 0/0.5), and F-B4 (Stage 2). The third shift-left filter is
F-B3/F-B5's canonical IR construction; F-B6/F-B7 add observation and
range constraints; F-B8 adds the spatial-bridge constraints.

### 1.6 Why this is exactly one Feature, not two or none

The natural unit is "the bridge from value/effect semantics to spatial
scheduling."

* If we made it zero features (i.e. inline storage decisions into
  `GbSchedIR`), the failure mode is exactly what `planv0.md` calls out
  on line 1700: "recomputation-vs-spill becomes a side effect of buffer
  lowering." The recompute decision becomes implicit, schedule equivalence
  becomes undefinable, and the alias-class equivalence relation gets
  computed from byte-range inference rather than from typed values.
* If we made it two features (e.g. F-B8a "materialization" and F-B8b
  "alias classes"), we would split on two surfaces that share the same
  set of binding decisions. Materialization decides what storage class
  and lifetime a value gets; alias-class equivalence decides which
  bindings share a resource. Both questions are answered by walking the
  same `GbInferIR` value list once. Splitting them forces two passes
  over the same IR with two halves of the same decision table — a
  duplication F-B8 exists to prevent.
* One feature matches the natural seam: F-B8 owns "the bridge." It is
  paired with no other feature in this chunk because the bridge has one
  responsibility — pin the materialization, lifetime, persistence, and
  alias-class facts that every later stage consumes.

### 1.7 What this chunk is NOT

The chunk is medium in *scope* but very large in *contract surface*. To
prevent scope creep, here is what this chunk explicitly is not:

* It is **not** a byte allocator. F-B12 (`ArenaPlan`) is the byte
  allocator; F-B8 emits abstract bindings only. No byte offset, no
  alignment pin, no concrete WRAM region appears in `StoragePlan`.
* It is **not** a bank assigner. F-B10 (`RomWindowPlan`) assigns ROM
  banks; F-B8 only pins `StorageClass::RomConst` for ROM-resident
  bindings. Bank 0 fixed vs switchable vs WRAM overlay residency is
  F-B10's authority.
* It is **not** an overlay installer. F-B11 (`OverlayPlan`) decides
  overlay regions, install schedules, and shared bank state; F-B8
  exposes overlayable bindings via the alias-class lens but never
  pins an overlay region.
* It is **not** an SRAM page assigner. F-B9 (`SramPagePlan`) decides
  page family membership, working sets, and commit boundaries; F-B8
  only pins `StorageClass::SramPaged` for paged bindings and
  `Persist { page, commit_group }` for persistent bindings.
* It is **not** a slice scheduler. F-B13 (`GbSchedIR`) decides slice
  boundaries, yield kinds, resource leases, and interrupt policies;
  F-B8 only emits `LifetimeClass`, which constrains slice scheduling
  but does not perform it.
* It is **not** a kernel selector. F-H1 / F-B13 select kernels; F-B8
  binds the values consumed and produced by kernels but does not name
  the kernel implementation.
* It is **not** a `SemanticCheckpointId` selector. F-B6 selects
  checkpoints; F-B8 consumes the selection via `ObservationPlan` and
  uses it as a constraint on lifetime classes.
* It is **not** a refinement loop. F-B16 owns the loop driver; F-B8
  emits `RepairProposal`s with `RepairReason::PromoteRecompute` and
  `KnobDelta::PromoteRecomputeLevel` (per `planv0.md` line 1449).
* It is **not** a runtime persistence module. `gbf-runtime::persistence`
  owns header layout, CRC verification, page rotation, and the
  durability-class state machine (`Writing → Committed → Retired`).
  F-B8 binds compiler-side `PersistPageId` and `CommitGroupId` such
  that the runtime can host them; the runtime contract is consumed by
  hash from `gbf-abi`.
* It is **not** a target-data-lowering producer. F-B8 records the
  lowering manifest hash as part of its identity for cache-key
  construction; it does not unpack any lowering shard or produce any
  per-target byte layout.
* It is **not** a generic value/effect IR rewriter. F-B8 produces
  `StoragePlan` as a side product over `GbInferIR`; the IR itself is
  not rewritten. `GbInferIR.report_self_hash` is a precondition to
  `StoragePlan`'s identity, never an output.

### 1.8 Relationship to F-B16 (`FeasibilityRefinementLoop`)

F-B16 (bd-3ix) is the bounded refinement loop that wraps stages 5..11.
The bead is currently OPEN but BLOCKED on an oracle question about how
to make repair admissibility decisions when calibration evidence is
heuristic. F-B8 must be aware of F-B16 in two ways:

* **Forward** (this RFC → F-B16): F-B8 emits `RepairProposal`s of class
  `RepairReason::PromoteRecompute` with `KnobDelta::PromoteRecomputeLevel`
  and `KnobDelta::ForceRecompute`. The proposals are emitted as part of
  the failure path (when a `LifetimeClass::Slice` value's materialization
  cannot fit a soft-pressure threshold) but do **not** mutate
  `CompileKnobs`. Every proposal carries a typed `RepairReason`, a
  typed `ConstraintDelta`, and a typed `EstimatedCostDelta`
  (`planv0.md` lines 1170–1176).
* **Backward** (F-B16 → this RFC): F-B8 must accept refined
  `ResolvedCompilePolicy` inputs. When F-B16 advances
  `RecomputePromotionLevel` from `None` to `PureSliceValues`, F-B8 is
  re-run with the advanced policy. The re-run is bounded by
  `StageIterationCeilings::storage` (per `planv0.md` line 1334).

Pre-F-B16, this RFC is consistent: the proposal-emit handshake is
proposal-only, and no acceptance is exercised. The handshake is wired
in this chunk so that F-B16, when it lands, only has to add an acceptance
path.

### 1.9 Relationship to F-B17 (`StageCache` integration sweep)

F-B17 is the cross-cutting StageCache integration sweep. F-B8 wires its
own `StageCache` key (K6) under the F-B2/F-B4 / F-B3/F-B5 discipline
(domain-tagged content hash; details in §13). F-B17 will, when it lands,
verify uniformity across stages; F-B8 ensures K6 is uniform with
K0/K0.5/K1/K2/K3/K4/K5 by construction.

## 2. Load-bearing decisions

### 2.1 Pure-function shape (core / driver split)

`StoragePlan` follows the F-B3/F-B5 discipline of a pure core constructor
plus a thin IO driver:

```text
build_storage_plan_core(StoragePlanInputs)
  -> Result<(StoragePlan, ReportEnvelope<StoragePlanReportBody>),
            PassDiagnostics>

run_stage6(StoragePlanInputs, env)
  = build_storage_plan_core(...) then
    (on success or failure):
      emit storage_plan.json
      may write StageCache success entry
      may write StageCache failure memo
```

The core never mutates `QuantGraph`, `GbInferIR`, `ObservationPlan`,
`RangePlan`, or `ResolvedCompilePolicy`. The driver is the only IO surface.
Determinism is required, not aspirational.

The chunk-level pass shape is:

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed shape derivations)
       (typed binding rules)
       (typed alias-class equivalence engine)
       (typed self-consistency checks)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed StoragePlan product,
                        ReportEnvelope<StoragePlanReportBody> }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON
       writes StageCache success / failure memo
```

Every report includes `outcome: ReportOutcome` per F-B2/F-B4 §2.1.

`StoragePlan` is **passive** in the IR-product sense: it produces its own
typed product but never mutates upstream products.

### 2.2 Inheritance from F-B2/F-B4 and F-B3/F-B5

This RFC inherits, **unchanged**, the following surfaces. Each item names
the source RFC so a future amendment cannot silently weaken what this RFC
depends on:

* `ReportEnvelope<R>` shape and public JSON conventions — F-B2/F-B4 §4.
* `Hash256`, `DomainHash(...)`, `SelfHash(report)`, `ZERO_HASH` —
  F-B2/F-B4 §1.
* `CanonicalJson(x)` rule (UTF-8, lex object keys, integers only, no
  NaN/Inf, no unknown fields, explicit enum tags, deterministic array
  ordering where order is not semantically meaningful) — F-B2/F-B4 §1.
* `null` policy (only for explicit semantic absence; never for unknown,
  unmeasured, or omitted) — F-B2/F-B4 §1.
* `R-Hash`, `R-Outcome-Pass`, `R-Outcome-Fail`, `R-FlatEnvelope`,
  `R-UnknownReject` envelope laws — F-B2/F-B4 §4.
* `ValidationDiagnostic` shape (`severity`, `origin`, `code`, `detail`,
  `provenance`) — F-B2/F-B4 §5. New origins and codes are introduced in
  §14 of this RFC; they extend the closed enum without modifying existing
  variants.
* `R-HardOnly-ThisChunk`: F-B8 reports reject `Soft` diagnostics —
  F-B2/F-B4 §4.
* `D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`, `D-Provenance`
  diagnostic laws — F-B2/F-B4 §5.
* `StageCache` key construction rule
  `DomainHash(crate, "StageCacheKey", schema_id, schema_version,
  canonical_json_bytes)` — F-B2/F-B4 §11.
* `QuantGraph` typed surface and `quant_graph.v1` schema — F-B3/F-B5 §8.
* `GbInferIR` typed surface, `infer_ir.v1` schema, the closed
  `EffectClass` set, the closed `InferOp` enum, the `ValueId` /
  `EffectId` / `NodeId` identity discipline, the `SemanticAnchor`
  definition, and the canonical reference semantics for every op —
  F-B3/F-B5 §9.
* The single-token convention: `GbInferIR` represents one token's
  compute — F-B3/F-B5 §2.5.
* The provenance discipline: `ExportTensorId → TensorId → NodeId` and
  `NodeOutput → ValueId` — F-B3/F-B5 §2.8. F-B8 extends this with
  `ValueId → AliasClassId` and `ValueId → Materialization`.

If a later amendment to F-B2/F-B4 or F-B3/F-B5 changes any of the above,
that amendment must explicitly amend this RFC by name.

This RFC adds the following to that surface:

* One new `ValidationOrigin` variant: `StoragePlanConstruction`.
* One new `ReportSchemaId` variant: `storage_plan.v1`.
* One new typed product: `StoragePlan`.
* One new public report body: `StoragePlanReportBody` (§12).
* One new `StageCacheKey` schema (§13): `K6 := StoragePlanCacheKey`.
* New typed enums: `StorageClass`, `LifetimeClass`, `Materialization`,
  `AliasClassId`, `PersistPageId`, `CommitGroupId` (§3, §8).
* New typed predicates over `GbInferIR` and `RangePlan`: `OpOutputRole`,
  `OpOutputFormat`, `IsPureValue`, `IsHotScalar`, `IsLargeActivation`,
  `IsExpertWeight`, `IsSequenceStateSlot`, `IsRenormLoopScratch` (§9).
* The alias-class equivalence relation as an explicit typed object (§11).

### 2.3 The bridge metaphor, restated formally

`StoragePlan` is the unique pipeline stage with the following contract:

```text
Input invariants (carried by upstream products):
  ∀ ValueId v in GbInferIR:
    v has no StorageClass.
    v has no LifetimeClass.
    v has no Materialization.
    v has no AliasClassId.
    v has no concrete byte offset.
    v has no concrete bank.
    v has no concrete SRAM page.
    v has no concrete WRAM region.
    v has no slice membership.

Output invariants (carried by StoragePlan):
  ∀ ValueId v in GbInferIR:
    v has exactly one StorageBinding b in StoragePlan.bindings.
    b.materialization is one of:
      Recompute
      Materialize { class: StorageClass, lifetime: LifetimeClass }
      Persist { page: PersistPageId, commit_group: CommitGroupId }
    b.alias_class is one of StoragePlan.alias_classes.
    None of {byte offset, concrete bank, concrete SRAM page, concrete
            WRAM region, slice membership} appear on b or anywhere in
            StoragePlan.

Cross-stage invariants (carried by downstream products):
  F-B9   may read b.materialization where class = SramPaged.
  F-B10  may read b.materialization where class = RomConst.
  F-B11  may read b.materialization where class allows overlay
         installation (typed predicate in §11.5).
  F-B12  reads every Materialize { class, lifetime } and assigns byte
         ranges; never reads Recompute.
  F-B13  reads b.alias_class as the equivalence-class lens for resource-
         state safety; F-B13 is the first stage that may add additional
         non-spatial bindings (e.g. resource leases, interrupt policy)
         on top of StoragePlan.
```

The bridge is asymmetric: the input side has "no spatial commitment," the
output side has "abstract spatial commitment without bytes." The output
is exactly the level of commitment that allows F-B9 / F-B10 / F-B11 / F-B12
/ F-B13 to make their decisions independently. This asymmetry is the core
contract of F-B8.

### 2.4 Recompute is first-class

Recompute is a positive typed decision, not a side effect of buffer
lowering. `planv0.md` line 1700 makes this explicit. F-B8 honors that
commitment by exposing `Materialization::Recompute` as a top-level
variant alongside `Materialize` and `Persist`, never as an absence of
`Materialize`.

```text
F-Recompute-FirstClass:
  ∀ ValueId v with binding b.
    b.materialization = Recompute
    ⇒ ¬∃ b' ∈ StoragePlan.bindings.
        b'.value = v
        ∧ b'.materialization ∈ {Materialize{..}, Persist{..}}

  ⇒ no later stage may treat Recompute as a memory-budget reservation.
  ⇒ F-B12 ArenaPlan does not allocate byte ranges for Recompute values.
  ⇒ F-B13 GbSchedIR is responsible for emitting recomputation ops
     (the ops that re-derive the value when consumed). The schedule for
     a Recompute value is derivable from GbInferIR's def-use graph; F-B8
     does not select the recomputation strategy beyond the typed
     promotion level.
```

A `Recompute` decision carries explicit cost evidence: the typed
`RecomputeJustification` in §9 names the predicate that admitted the
decision (in v1, `RecomputePromotion::PureSliceValues`; wider levels
are reserved for a later RFC), the `IsPureValue`
proof obligation that justified it, and the `EstimatedCostDelta` in
cycles that F-B16 (when it lands) can use as evidence of admissibility.

A `Recompute` decision is forbidden when:

* the value is the source of a `Persist { page, commit_group }` binding's
  data flow (you cannot recompute a persistent value at the persist
  boundary);
* the value is the materialization site for an `ObservationPlan`
  semantic checkpoint (you cannot recompute an observed value across a
  checkpoint boundary, because the checkpoint requires the value to be
  inspectable at a specific point);
* the value is a routing-table entry consumed by `RouteTop1` (routing
  decisions must be stable across the routed-FFN dispatch);
* the value's def-use graph crosses a boundary wider than one slice
  under F-B8's conservative lifetime estimate. Slices are F-B13's
  authority, so F-B8 only admits `Recompute` for values whose effective
  lifetime estimate is `Slice` and whose recomputation is otherwise
  legal under §9.3.

### 2.5 Persistent identity without byte ranges

Persistent values carry a `PersistPageId` and a `CommitGroupId`. Neither
is a byte range; both are runtime-protocol identifiers consumed by
`gbf-runtime::persistence` (per `planv0.md` lines 2138–2208).

```text
F-Persist-NoBytes:
  ∀ ValueId v with binding b.
    b.materialization = Persist { page, commit_group }
    ⇒ b carries no byte offset, no concrete page address, no concrete
       SRAM bank.
    ⇒ page : PersistPageId is a runtime-protocol identifier; the byte
       layout is owned by gbf-runtime::persistence and is not visible
       in StoragePlan.
    ⇒ commit_group : CommitGroupId groups bindings that must commit
       atomically (planv0.md line 2197). Same-group bindings either all
       commit or all roll back.
```

`PersistPageId` is a typed value:

```rust
#[repr(transparent)]
pub struct PersistPageId(u32);
```

with the following semantics:

* Two distinct `PersistPageId`s correspond to two distinct logical
  page families. They may share a byte range at runtime (double-
  buffered or ring-buffered), but `StoragePlan` treats them as
  independent identifiers.
* A `PersistPageId` carries an associated `PersistKind` (declared in
  `gbf-abi` per `planv0.md` line 2165:
  `SequenceState | Continuation | Transcript | Harness | Trace`).
  The kind is reachable from `PersistPageId` via a typed lookup
  function defined in §10.

`CommitGroupId` is a typed value:

```rust
#[repr(transparent)]
pub struct CommitGroupId(u32);
```

with the following semantics:

* A commit group is a non-empty set of `PersistPageId`s that must
  commit atomically. `planv0.md` line 2197: "pages that must remain
  mutually consistent (for example sequence state + transcript delta
  + token output) are assigned the same `CommitGroupId`, and the small
  `PersistGroupCommit` manifest is written last."
* A binding's `commit_group` references one of `StoragePlan`'s declared
  commit groups. A `CommitGroupId` referenced by no binding is a hard
  error.
* Two bindings with different `PersistKind`s may share a `CommitGroupId`
  only if the runtime persistence module accepts the cross-kind grouping.
  In v1 this is permitted to emit only for `(SequenceState, Transcript)`
  groups under `CommitGroupReason::SequenceStateWithTranscript`.
  Other cross-kind shapes may appear as reserved schema only where
  explicitly marked below; emitting a reserved shape is a hard error.

Amends planv0: `planv0.md` describes `PersistGroupCommit` and
`CommitGroupId` shape (line 2157) but does not pin the binding-to-group
membership rules. This RFC pins them: every binding's `commit_group`
references a non-empty group; every group references at least one
binding; cross-kind groupings are allowed only for the
`(SequenceState, Transcript)` family in v1.

### 2.6 Aliasing as a typed equivalence relation

Aliasing in F-B8 is a typed equivalence relation over bindings, not a
property of byte ranges. The relation is exposed as `AliasClassId` per
binding and as a typed `AliasClass` declaration per equivalence class.

```text
F-Alias-Typed:
  ∀ ValueId v_a, v_b with bindings b_a, b_b.
    b_a.alias_class = b_b.alias_class
    ⇔ (v_a, v_b) ∈ AliasEquivalence(StoragePlan)

  AliasEquivalence is reflexive, symmetric, transitive.

F-Alias-NoConflict:
  ∀ alias class A.
    ∀ bindings b_x, b_y ∈ A with b_x.value ≠ b_y.value.
      LiveRange(b_x) ∩ LiveRange(b_y) = ∅
      OR
      A.intent ∈ {PingPong, ResumeOverlap, PersistRotation}
      and permits b_x and b_y to overlap with no conflicting writes.

F-Alias-DeclaredFirst:
  ∀ AliasClass A. A.intent ∈
    {NoAlias, ScratchReuse, PingPong, ResumeOverlap, PersistRotation}.
    Any other intent requires explicit RFC amendment.
```

The alias-class layer exists so that F-B13 (`ResourceStateValidation`)
can reason about resource conflicts using a typed equivalence relation
rather than byte-range overlap. Two bindings in the same alias class
are declarations that "these two values are allowed to share a
resource"; the resource may be a WRAM region, an SRAM page, a register
file, or a shared scratch buffer, but the precise resource is F-B12's
or F-B13's authority.

The alias-class relation is *not* a byte-range relation. Two bindings
with the same `AliasClassId` may end up at distinct byte ranges in
`ArenaPlan` (e.g. ping-pong activations); two bindings with different
`AliasClassId`s may end up at the same byte range over disjoint
lifetimes (when `ArenaPlan` decides to coalesce them after F-B8 has
proven their lifetimes disjoint).

### 2.7 Lifetime taxonomy is closed and ordered

```rust
pub enum LifetimeClass {
    Slice,         // shortest: lives only within one schedule slice
    ResumeWindow,  // lives across a yield boundary inside the same token
    Token,         // lives across the whole one-token IR pass
    Session,       // lives across multiple tokens in one session
    Persistent,    // lives across power cuts; backed by SRAM
}
```

The order is `Slice < ResumeWindow < Token < Session < Persistent`. F-B8
treats lifetime as monotonically extensible: a value declared `Slice`
may be promoted to `ResumeWindow` by a refinement-loop repair; a value
declared `Persistent` may not be demoted.

```text
F-Lifetime-Monotone:
  ∀ refinement step s.
    Lifetime_after(s) ≥ Lifetime_before(s)
    (under the Slice < ResumeWindow < Token < Session < Persistent order)

  ⇒ lifetime promotion is admissible to F-B16; lifetime demotion is
    forbidden in v1 and requires explicit RFC amendment.
```

`LifetimeClass::Persistent` and `Materialization::Persist { .. }` are
related but distinct: every `Persist` binding has lifetime `Persistent`
in the operational sense, but `LifetimeClass::Persistent` may also be
attached to a `Materialize { class: SramPaged, lifetime: Persistent, .. }`
binding for non-Persist persistent objects (large transcript pages that
do not need atomic-commit semantics, for example). The two are kept
distinct so that F-B9 can plan paged persistent objects (which need
working-set rotation) separately from Persist objects (which need
commit-group rotation).

Amends planv0: `planv0.md` does not separate `Persist`-with-commit-group
from `LifetimeClass::Persistent` materialized pages. This RFC makes the
distinction explicit so F-B9's commit-group plan does not collapse onto
F-B9's working-set plan.

### 2.8 Storage taxonomy is closed and ordered by hotness

```rust
pub enum StorageClass {
    WramHot,    // 8 KiB internal WRAM; fastest random access
    HramHot,    // 127 bytes high RAM; ISR-safe, fastest scratch
    SramPaged,  // 8 KiB switchable external SRAM window; paged
    RomConst,   // ROM-resident constants; immutable, banked
}
```

The hotness order is informal but consistent:
`HramHot ≻ WramHot ≻ RomConst ≻ SramPaged` for read latency, and
`HramHot ≻ WramHot ≻ SramPaged` for write latency (RomConst is not
writable). F-B8 uses this order as a tie-breaker in the decision rules
of §9 but never as a hard constraint.

The four classes correspond to `planv0.md`'s memory plan (lines
2061–2120): WRAM hot arena, HRAM fast flags, SRAM persistent arena, and
ROM banks. The mapping is:

```text
StorageClass::WramHot   ↔  WRAM hot arena (planv0.md lines 2090–2098)
StorageClass::HramHot   ↔  HRAM fast flags (planv0.md lines 2106–2112)
StorageClass::SramPaged ↔  SRAM persistent arena (planv0.md lines
                            2114–2120) — non-Persist paged objects
StorageClass::RomConst  ↔  ROM banks 00..N (planv0.md lines 2065–2088)
```

`StorageClass::RomConst` does not distinguish Bank 0 fixed from
switchable. That distinction is F-B10's authority (`RomWindowPlan`
selects `KernelResidency::Bank0Fixed | WramOverlay |
CoResidentSwitchable` per `planv0.md` line 1731). F-B8 only pins that
the binding is ROM-resident.

`StorageClass::SramPaged` covers both:

* paged persistent objects with `LifetimeClass::Persistent` that do not
  need atomic-commit semantics (large transcript scrollback);
* paged transient objects with `LifetimeClass::Session` or
  `LifetimeClass::Token` that are too large to fit in WRAM.

Persistent objects with atomic-commit semantics use
`Materialization::Persist { .. }`, not
`Materialize { class: SramPaged, lifetime: Persistent }`.

### 2.9 RangePlan-aware materialization

`RangePlan` (F-B7) selects a `ReductionPlan` per reduction site:

```text
ReductionPlan :=
    SingleI16
  | ChunkedI16   { chunk_len: u16 }
  | RenormLoop   { tile_len:  u16 }
```

per `planv0.md` lines 1655–1660. `planv0.md` line 1663 makes the
F-B8-aware claim explicit: "The outputs of `RangePlan` feed `StoragePlan`
(because a `RenormLoop` reduction may imply different scratch
materialization than a `SingleI16` reduction)."

F-B8 honors `RangePlan` via two typed predicates over the value
declarations of `GbInferIR`:

```text
IsRenormLoopScratch(v: ValueId) :=
  ∃ ReductionSiteRef site. v ∈ ValueDecls(site.node)
            ∧ ReductionPlanFor(site) = RenormLoop { tile_len }
            ∧ ValueRole(site.node, v) = Scratch

IsSingleI16Accum(v: ValueId) :=
  ∃ ReductionSiteRef site. v ∈ ValueDecls(site.node)
            ∧ ReductionPlanFor(site) = SingleI16
            ∧ ValueRole(site.node, v) = Accumulator
```

The decision rule (§9) maps:

* `IsRenormLoopScratch(v) ⇒ Materialize { class ∈ { WramHot, HramHot },
  lifetime: Slice }`. The scratch must be hot and short-lived; renorm
  loops re-touch the scratch on every tile.
* `IsSingleI16Accum(v) ⇒ Materialize { class: WramHot, lifetime: Slice }`
  is the default. HramHot is permitted only through the deterministic
  HRAM admission rule for hot scalars, not through recompute promotion.
* `IsChunkedI16Accum(v) ⇒ Materialize { class: WramHot, lifetime: Slice }`
  with no exception.

Amends planv0: `planv0.md` line 1663 hints at the dependency without
pinning the predicate. This RFC pins `IsRenormLoopScratch` as a typed
predicate over the joint `(GbInferIR, RangePlan)` product so the
materialization rule is mechanically checkable.

### 2.10 ResolvedCompilePolicy honoring

F-B8 honors `ResolvedCompilePolicy` via `CompileKnobs::storage` and
`CompileKnobs::overrides`:

```text
StorageKnobs.recompute_promotion: RecomputePromotionLevel
  ∈ { None, PureSliceValues }   -- v1: wider levels reserved

CompileKnobOverrides.forced_recompute: BTreeSet<ValueSelector>
  where ValueSelector := Value(ValueId)
                       | AliasClass(AliasClassFingerprint)
```

The honoring rule:

```text
F-Honor-RecomputePromotion:
  ∀ ValueId v with binding b.
    b.materialization = Recompute
    ⇒ AdmittingPredicate(v) ≤ recompute_promotion
       (under the v1 order None < PureSliceValues; wider levels are
        reserved for a later RFC)
       OR Value(v) ∈ forced_recompute
       OR ∃ A. b.alias_class = A ∧ AliasClass(A.fingerprint)
                                   ∈ forced_recompute

F-Honor-Bounds:
  ∀ refinement step s.
    recompute_promotion(s) ≤ CompileKnobBounds.max_recompute_promotion

F-Honor-Locks:
  CompileKnobId::StorageRecomputePromotion ∈ KnobLockSet.locked
  ⇒ recompute_promotion is invariant under any RepairProposal
    F-B8 emits.
```

The honoring is local: F-B8 reads the knob values once at stage entry
and uses them for the whole pass. F-B16 (when it lands) advances
`recompute_promotion` between passes; F-B8 itself never advances a knob.

### 2.11 No semantic checkpoints in this chunk

`SemanticCheckpointId` is referenced through `ObservationPlan` but never
emitted in this chunk. F-B6 owns checkpoint emission. F-B8 reads the
attached anchors via `ObservationPlan.semantic` and uses them as a
constraint on lifetime classes (a value backing an observed checkpoint
must have lifetime ≥ `LifetimeClass::Slice` and must be observable at
the checkpoint, which the binding's `Materialize { .. }` form ensures).

```text
F-B8-NoCheckpointEmission:
  StoragePlan contains no SemanticCheckpointId, TraceProbeId, or
  observation/probe attachment field. F-B8 reads ObservationPlan but
  never extends it.
```

### 2.12 Schema versioning

`StoragePlan` schema is versioned independently of the existing
F-B2/F-B4 / F-B3/F-B5 report schemas:

```text
storage_plan.v1
```

Schema bumps follow F-B2/F-B4 §10's compatibility rules (any later RFC
that changes shape, canonicalization, or self-hash must amend this RFC).
Cross-major artifact schema migration is still owned by `gbf-migrate`
(deferred per F-A6b).

### 2.13 Determinism

`StoragePlan` construction is a deterministic pure function of its
inputs:

```text
F-StoragePlan-Determinism:
  build_storage_plan_core(inputs) = build_storage_plan_core(inputs')
    whenever inputs.canonical_hash = inputs'.canonical_hash
```

The canonical input hash is the K6 cache key (§13). Determinism is
required for cache hits to be safe and for `repair_report.json` (F-B16)
to be replayable.

### 2.14 No partial product

A failed `storage_plan.json` carries no product:

```text
R-NoPartialIR-StoragePlan:
  Failed storage_plan report
  ⇒ body.result = None
```

This mirrors `R-NoPartialIR-QG` and `R-NoPartialIR-IIR` from F-B3/F-B5
§7.

## 3. Glossary additions

This chunk introduces or pins the following terms beyond the F-B2/F-B4
and F-B3/F-B5 glossary inheritance.

| Term                       | Definition                                                                                                |
|----------------------------|-----------------------------------------------------------------------------------------------------------|
| StoragePlan                | The typed product of Stage 6: a complete map from `ValueId` to `StorageBinding` plus alias classes plus declared persist pages and commit groups. |
| StorageBinding             | A typed record containing `(ValueId, Materialization, AliasClassId, AbstractLiveRange, BindingJustification)`. One binding per `ValueId`. |
| Materialization            | A typed enum `Recompute | Materialize { class, lifetime } | Persist { page, commit_group }`.              |
| StorageClass               | A typed enum `WramHot | HramHot | SramPaged | RomConst`. Closed in v1.                                    |
| LifetimeClass              | A typed enum `Slice | ResumeWindow | Token | Session | Persistent`. Closed in v1; ordered by lifetime length. |
| AliasClassId               | A typed identifier for one alias-equivalence class. Two bindings with the same `AliasClassId` may share a resource. |
| AliasClass                 | The typed object backing one `AliasClassId`: a non-empty set of `ValueId`s plus a typed `AliasIntent`.    |
| AliasIntent                | A typed enum `NoAlias \| ScratchReuse \| PingPong \| ResumeOverlap \| PersistRotation`. Declares the legitimate reason an alias class exists; `NoAlias` is required for singleton classes. Closed in v1. |
| PersistPageId              | A typed identifier for a logical persistent-page family. Resolves to a runtime-protocol byte layout under `gbf-runtime::persistence`. |
| CommitGroupId              | A typed identifier for an atomic-commit group. References one or more `PersistPageId`s that commit together. |
| PersistKind                | A typed enum `SequenceState | Continuation | Transcript | Harness | Trace`. Inherited from `gbf-abi`.     |
| RecomputePromotionLevel    | A typed enum `None \| PureSliceValues` in v1. Wider levels (`PureResumeWindowValues`, `PureTokenValues`) are reserved for a later RFC that makes schedule-boundary evidence available before Stage 6. The knob F-B8 honors for recompute admissibility. Inherited from `CompileKnobs`. |
| ValueRole                  | A typed predicate over `(NodeId, ValueId)` indicating the value's role in the op (`InputToken`, `OutputToken`, `Scratch`, `Accumulator`, `RouterDecision`, `RouterScore`, `EmbeddingTable`, `LogitProj`, ...). |
| Reduction site             | A `NodeId` whose op participates in a reduction; bound to a `ReductionPlan` by `RangePlan`.               |
| Hot scalar                 | A small value with `ValueRole ∈ {Accumulator, RouterScore, RouterDecision}` and `LifetimeClass = Slice`, eligible for `HramHot` only if it fits within `AllocatableHramBudget(policy)` after runtime reservations. |
| Expert weight              | A `RomConst` value bound by provenance to an `ExpertSection.tensor_refs` entry. Always `Materialize { class: RomConst, lifetime: Persistent }`. |
| Sequence-state slot        | A `StateSlotId`-bound value carrying inter-token state; always `Persist { page, commit_group }` when present. v1 forbids non-identity sequence blocks (per F-B3/F-B5 §2.5a), so this term is reserved shape in v1. |

## 4. Core notation

This RFC inherits §1 of F-B2/F-B4 and §4 of F-B3/F-B5 (Hash256, Outcome,
Severity, Stage, ReportSchema, Result, Option, NonEmptyList, SortedBy,
DomainHash, SelfHash, CanonicalJson, ZERO_HASH, null policy). Additions:

```text
Stage :=
  Stage0 | Stage0_5 | Stage1 | Stage2 | Stage3
  | Stage4 | Stage5
  | Stage6                                       -- new

ReportSchema :=
  artifact_validation.v1
  | policy_resolution.v1
  | static_budget.v1
  | quant_graph.v1
  | infer_ir.v1
  | observation_plan.v1                          -- expected from F-B6
  | range_plan.v1                                -- expected from F-B7
  | storage_plan.v1                              -- new

ValidationOrigin (extension) :=
  ...existing F-B2/F-B4 / F-B3/F-B5 / F-B6/F-B7 origins...
  | StoragePlanConstruction
```

Abbreviations used throughout:

```text
QG    := QuantGraph
IIR   := GbInferIR (also "InferIR")
OP    := ObservationPlan
RP    := RangePlan
SP    := StoragePlan                             -- new in this RFC
v     := ValueId
A     := AliasClassId
M     := Materialization
LC    := LifetimeClass
SC    := StorageClass
PG    := PersistPageId
CG    := CommitGroupId
```

## 5. Authority rules

```text
Scope(F-B8) =
  {
    Stage6,
    StoragePlan,
    storage_plan.v1,
    StageCache key K6,
    canonical reference for Materialization, LifetimeClass, StorageClass,
      AliasClassId, PersistPageId, CommitGroupId, AliasClass, AliasIntent,
    closed enum constants for StorageClass, LifetimeClass, AliasIntent,
    decision rules / typed predicates over (QuantGraph, GbInferIR,
      ObservationPlan, RangePlan, ResolvedCompilePolicy) → bindings,
    persist-binding well-formedness invariants,
    alias-class equivalence relation invariants,
    RepairProposal class for RecomputePromotion (proposal-only handshake;
      acceptance is F-B16's authority)
  }

Rule Authority:
  ∀ behavior b.
    b ∈ Scope(F-B8) ∧ RFC specifies b
    ⇒ SourceOfTruth(b) = RFC

Rule PlanContext:
  ∀ behavior b.
    b ∈ Scope(F-B8) ∧ RFC silent on b
    ⇒ planv0 may inform implementation but is not an acceptance gate

Rule Inheritance:
  ∀ behavior b.
    b ∈ Scope(F-B2/F-B4) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B2/F-B4 RFC
  ∀ behavior b.
    b ∈ Scope(F-B3/F-B5) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B3/F-B5 RFC
  ∀ behavior b.
    b ∈ Scope(F-B6/F-B7) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B6/F-B7 RFC

Rule Amendment:
  LaterRFC changes any of:
    public StoragePlan type
    storage_plan.v1 schema
    cache key K6
    diagnostic code introduced here (STORE-* family)
    decision rules / typed predicates introduced here
    closed enum constants introduced here
  ⇒ LaterRFC must explicitly amend this RFC

Rule DivergenceLedger:
  RFC intentionally diverges from planv0
  ⇒ nearest relevant section must contain `Amends planv0`
```

## 6. Pipeline state machine

Extending the F-B6/F-B7 (forthcoming) state machine:

```text
State :=
  Imported(inputs)
  | Validated(validation_product)
  | PolicyResolved(policy_product)
  | QuantGraphReady(policy_product, quant_graph_product)
  | BudgetPassed(quant_graph_product, static_budget_product)
  | InferIrReady(budget_product, infer_ir_product)
  | ObservationPlanReady(infer_ir_product, observation_plan_product)
  | RangePlanReady(observation_plan_product, range_plan_product)
  | StoragePlanReady(range_plan_product, storage_plan_product)   -- new
  | Halted(stage, report, diagnostics)
```

Transitions (extending F-B3/F-B5 §6 and F-B6/F-B7's expected shape):

```text
T6 build_storage_plan:
  RangePlanReady(rp)
    -- build_storage_plan(qg, iir, op, rp, policy) = Ok(sp) -->
  StoragePlanReady(rp, sp)

  RangePlanReady(rp)
    -- build_storage_plan(qg, iir, op, rp, policy) = Err(e) -->
  Halted(Stage6, e.report, e.diagnostics)
```

Pipeline invariants (additions to F-B3/F-B5 §6 and F-B6/F-B7):

```text
I-Pipeline-23:
  Stage6 may run only after Stage5 (RangePlan) Passed.

I-Pipeline-24:
  If any of {Stage1, Stage3, Stage4, Stage5} fail, Stage6 does not run.

I-Pipeline-25:
  Stage6 is passive in the IR-product sense:
    It produces StoragePlan but never mutates QuantGraph, GbInferIR,
    ObservationPlan, RangePlan, or ResolvedCompilePolicy.

I-Pipeline-26:
  storage_plan.report_self_hash is immutable after Stage6 emits it.

I-Pipeline-27:
  Every emitted report must satisfy:
    report.report_self_hash =
      SelfHash(report with report_self_hash set to ZERO_HASH).

I-Pipeline-28:
  Stage6's StoragePlan product does not change shape between two
  consecutive regenerations on the same (QuantGraph, GbInferIR,
  ObservationPlan, RangePlan, ResolvedCompilePolicy) hashes.

I-Pipeline-29 (refinement):
  When the FeasibilityRefinementLoop (F-B16) advances
  RecomputePromotionLevel, Stage6 is re-run with the advanced policy.
  Stage6 itself never advances any knob.

I-Pipeline-30 (binding coverage):
  ∀ ValueId v ∈ GbInferIR.values.
    ∃! StorageBinding b ∈ StoragePlan.bindings. b.value = v.
  (No coverage gap, no double-bind.)
```

## 7. Report envelope (inherited)

`storage_plan.json` uses the `ReportEnvelope<R>` shape from F-B2/F-B4
§4 unchanged:

```rust
pub struct ReportEnvelope<R> {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,
    pub body: R,
}
```

Public JSON shape, envelope laws (`R-Hash`, `R-Outcome-Pass`,
`R-Outcome-Fail`, `R-FlatEnvelope`, `R-UnknownReject`,
`R-HardOnly-ThisChunk`) are inherited unchanged. Specifically: F-B8
reports reject `Soft` diagnostics in this chunk.

`R-NoPartialProduct` is restated for the StoragePlan product:

```text
R-NoPartialIR-StoragePlan:
  Failed storage_plan report
  ⇒ body.result = None
```

## 8. Stage 6 contract: `StoragePlan`

### 8.1 Type-level contract

```text
StoragePlanInputs :=
  {
    policy:              ResolvedCompilePolicy,   -- from F-B2 Stage 0.5
    policy_hash:         Hash256,                 -- = policy.canonical_hash
    quant_graph:         QuantGraph,              -- from F-B3 Stage 1
    quant_graph_hash:    Hash256,                 -- = qg.report_self_hash
    infer_ir:            GbInferIR,               -- from F-B5 Stage 3
    infer_ir_hash:       Hash256,                 -- = iir.report_self_hash
    observation_plan:    ObservationPlan,         -- from F-B6 Stage 4
    observation_plan_hash: Hash256,
    range_plan:          RangePlan,               -- from F-B7 Stage 5
    range_plan_hash:     Hash256,
  }

StoragePlan :=
  {
    bindings:        BTreeMap<ValueId, StorageBinding>,
    alias_classes:   BTreeMap<AliasClassId, AliasClass>,
    persist_pages:   BTreeMap<PersistPageId, PersistPageDecl>,
    commit_groups:   BTreeMap<CommitGroupId, CommitGroupDecl>,
    repair_proposals: Vec<RepairProposal>,
    provenance:      StorageProvenance,
    input_identity:  StoragePlanInputIdentity,
  }
```

The product carries `StoragePlanInputIdentity` as the
content-addressed identity of its inputs. That identity is the input
to `StorageCacheKey` (K6) and is computed before rule evaluation; it
contains no output hash. The envelope's `report_self_hash` (§12.1) is
the only self-hash of the output and is owned by the `ReportEnvelope`,
not by any nested field of the body.

### 8.2 `StorageBinding`, `Materialization`, `StorageClass`, `LifetimeClass`

The core type surface, in Rust syntax:

```rust
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum StorageClass {
    WramHot,
    HramHot,
    SramPaged,
    RomConst,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum LifetimeClass {
    Slice,
    ResumeWindow,
    Token,
    Session,
    Persistent,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum Materialization {
    Recompute,
    Materialize {
        class: StorageClass,
        lifetime: LifetimeClass,
    },
    Persist {
        page: PersistPageId,
        commit_group: CommitGroupId,
    },
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct StorageBinding {
    pub value: ValueId,
    pub materialization: Materialization,
    pub alias_class: AliasClassId,
    pub live_range: AbstractLiveRange,
    pub justification: BindingJustification,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct AbstractLiveRange {
    /// Producer node in GbInferIR topological order.
    pub def_node: NodeId,
    /// First use node in GbInferIR topological order, if any.
    pub first_use_node: Option<NodeId>,
    /// Last use node in GbInferIR topological order, if any.
    pub last_use_node: Option<NodeId>,
    /// Conservative lifetime class derived from the def-use interval
    /// and ObservationPlan constraints.
    pub lifetime_class: LifetimeClass,
    /// True when an ObservationPlan semantic checkpoint requires the
    /// value to be inspectable at a stable point.
    pub checkpoint_stable: bool,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum BindingJustification {
    /// Default decision rule fired for this value's role/format.
    DecisionRule(DecisionRuleId),
    /// Override applied via CompileKnobOverrides.forced_recompute.
    ForcedRecompute,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct DecisionRuleId(pub u32);
```

`BindingJustification` is the typed evidence that admitted the binding.
It is mandatory: every binding carries a justification, and the
justification names the rule (typed `DecisionRuleId`) or the override
(typed reference).

The five `LifetimeClass` variants and four `StorageClass` variants are
closed in v1. Adding a new variant requires explicit RFC amendment.

### 8.3 `AliasClass`, `AliasClassId`, `AliasIntent`

```rust
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct AliasClassId(pub u32);

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct AliasClassFingerprint(pub Hash256);

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct AliasClass {
    pub id: AliasClassId,
    pub fingerprint: AliasClassFingerprint,
    pub members: NonEmptySortedSet<ValueId>,
    pub intent: AliasIntent,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum AliasIntent {
    /// Singleton class. Operationally equivalent to no aliasing.
    NoAlias,
    /// Two short-lived scratch values that share the same hot
    /// scratch buffer at distinct times.
    ScratchReuse,
    /// Two activation buffers used in alternating slices for
    /// double-buffered streaming.
    PingPong,
    /// A value that survives a yield is allowed to overlap with
    /// scratch on the resume side, provided no conflicting writes.
    ResumeOverlap,
    /// Two persistent pages in a rotation pair (e.g. double-buffered
    /// SequenceState pages where one is Writing while the other is
    /// Committed).
    PersistRotation,
}
```

The five `AliasIntent` variants are closed in v1. `AliasIntent` is the
typed reason that *justifies* two values being placed in the same alias
class. Without a declared intent (other than `NoAlias`), two values
cannot share an alias class.

```text
F-Alias-IntentDeclared:
  ∀ AliasClass A.
    |A.members| = 1 ⇒ A.intent = NoAlias
    |A.members| > 1 ⇒ A.intent ∈ {ScratchReuse, PingPong,
                                  ResumeOverlap, PersistRotation}
```

The relation between `AliasIntent` and `Materialization` is constrained:

```text
F-Alias-IntentMatchesMaterialization:
  ∀ AliasClass A.
    A.intent = NoAlias ⇒
      |A.members| = 1

    A.intent = ScratchReuse ⇒
      ∀ v ∈ A.members. SP.bindings[v].materialization =
        Materialize { class ∈ {WramHot, HramHot}, lifetime: Slice }
      AND |A.members| > 1

    A.intent = PingPong ⇒
      ∀ v ∈ A.members. SP.bindings[v].materialization =
        Materialize { class: WramHot, lifetime: Slice | ResumeWindow }

    A.intent = ResumeOverlap ⇒
      ∀ v ∈ A.members. SP.bindings[v].materialization =
        Materialize { class ∈ {WramHot, HramHot},
                      lifetime ∈ {Slice, ResumeWindow} }

    A.intent = PersistRotation ⇒
      ∀ v ∈ A.members. SP.bindings[v].materialization =
        Persist { page, commit_group }
      AND all members share the same commit_group
      AND all members' page ids differ pairwise
```

The intent rules are mechanically checkable from the binding map.

### 8.4 `PersistPageDecl`, `CommitGroupDecl`, `PersistPageId`, `CommitGroupId`

```rust
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct PersistPageId(pub u32);

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct CommitGroupId(pub u32);

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct PersistPageDecl {
    pub id: PersistPageId,
    pub kind: PersistKind,
    pub durability: DurabilityClass,
    pub schema_pin: PersistSchemaPin,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct CommitGroupDecl {
    pub id: CommitGroupId,
    pub members: NonEmptySortedSet<PersistPageId>,
    pub kind_set: SortedSet<PersistKind>,
    /// Constrains which PersistGroupCommit headers are runtime-legal.
    pub atomicity: CommitAtomicityClass,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum CommitAtomicityClass {
    /// All members must commit; failure of any member rolls all back.
    AllOrNothing,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct PersistSchemaPin {
    /// state_schema field of PersistHeader (planv0.md line 2147).
    pub state_schema: u16,
    /// Whether semantic_state_hash is required to be the artifact's
    /// canonical semantic state hash (true for SequenceState pages
    /// in v1; false for Transcript/Harness/Trace).
    pub requires_semantic_state_hash: bool,
    /// Whether resume_abi_hash is required (true for Continuation;
    /// false otherwise).
    pub requires_resume_abi_hash: bool,
    /// Whether build_identity_hash is required (true for Harness/Trace;
    /// false otherwise).
    pub requires_build_identity_hash: bool,
}
```

`PersistKind` and `DurabilityClass` are inherited from `gbf-abi`
(`planv0.md` lines 2165–2177). `PersistPageDecl.schema_pin` is the
typed evidence that the runtime persistence module's recovery rules
match the compiler's expectation.

`CommitAtomicityClass::AllOrNothing` is the only v1 variant. A
future `OrderedRecoverable` shape may be added by RFC amendment when
the runtime persistence module implements ordered-recoverable commit;
v1 does not carry the variant in the type or the schema.

Amends planv0: `planv0.md` line 2199 names per-kind compatibility rules
(SequenceState validates `semantic_state_hash`, Continuation validates
`resume_abi_hash`, harness/trace validates `build_identity_hash`) but
does not pin them as a typed schema. This RFC pins them in
`PersistSchemaPin` so F-B8's persist-page declarations are
mechanically verifiable against the runtime persistence module's
recovery contract.

### 8.5 `StoragePlanInputIdentity`

```rust
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct StoragePlanInputIdentity {
    /// Hash of the QuantGraph that fed this StoragePlan.
    pub quant_graph_hash: Hash256,
    /// Hash of the GbInferIR that fed this StoragePlan.
    pub infer_ir_hash: Hash256,
    /// Hash of the ObservationPlan consumed.
    pub observation_plan_hash: Hash256,
    /// Hash of the RangePlan consumed.
    pub range_plan_hash: Hash256,
    /// Hash of ResolvedCompilePolicy (full canonical policy hash).
    pub policy_hash: Hash256,
    /// Determinism class inherited from QuantGraph (and ultimately
    /// from ArtifactCore.numeric_profile.determinism).
    pub determinism: DeterminismClass,
    /// Schema id and version.
    pub schema: ReportSchemaId,           // == storage_plan.v1
    pub schema_version: SemVer,           // == 1.0.0
}
```

`StoragePlanInputIdentity` is the content-addressed identity of the
Stage 6 inputs. It is computed before rule evaluation and is the basis
for K6. It does not contain any output hash.

`report_self_hash` is owned only by the `ReportEnvelope`. It covers the
entire emitted `storage_plan.json` envelope under the inherited
self-hash convention. No nested field in the body duplicates
`report_self_hash`.

### 8.6 `StorageProvenance`

```rust
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct StorageProvenance {
    /// For every binding, which decision rule fired (or which
    /// override was honored).
    pub bindings: BTreeMap<ValueId, BindingProvenance>,
    /// For every alias class, the rule that admitted membership.
    pub alias_classes: BTreeMap<AliasClassId, AliasClassProvenance>,
    /// For every persist page, the QuantGraph entity that backed it.
    pub persist_pages: BTreeMap<PersistPageId, PersistPageProvenance>,
    /// For every commit group, the rule that admitted the grouping.
    pub commit_groups: BTreeMap<CommitGroupId, CommitGroupProvenance>,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct BindingProvenance {
    pub admitting_predicate: AdmittingPredicateId,
    pub decision_rule: DecisionRuleId,
    pub policy_refinement_applied: bool,
    pub evidence: SortedVec<EvidenceRef>,
    pub op_output_role: Option<ValueRole>,
    pub op_output_format: Option<ValueFormat>,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct AliasClassProvenance {
    pub admitting_intent: AliasIntent,
    pub evidence: SortedVec<EvidenceRef>,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct PersistPageProvenance {
    /// The QuantGraph entity (TensorId, StateSlotId, etc.) that
    /// backs this page.
    pub source: PersistPageSource,
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum PersistPageSource {
    /// SequenceState slot from QuantGraph.sequence_semantics.
    SequenceStateSlot { layer: LayerId, slot: StateSlotId },
    /// Continuation record (compiler-internal, runtime-bound).
    Continuation,
    /// Transcript page family.
    Transcript { family: TranscriptFamilyId },
    /// Harness page family.
    Harness { family: HarnessFamilyId },
    /// Trace page family.
    Trace { family: TraceFamilyId },
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct CommitGroupProvenance {
    pub reason: CommitGroupReason,
    pub evidence: SortedVec<EvidenceRef>,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum CommitGroupReason {
    /// One commit group per SequenceState slot (default).
    PerSequenceStateSlot,
    /// SequenceState + Transcript paired so transcript truncation
    /// matches state rollback.
    SequenceStateWithTranscript,
    /// Continuation paired with SequenceState for resume safety.
    /// Reserved in storage_plan.v1; emitting this reason is STORE-030
    /// until an RFC amendment lifts the reserved gate.
    ContinuationWithSequenceState,
    /// Harness/Trace independent groups.
    Independent,
}
```

The provenance maps are **not** stored inline on `StorageBinding`,
`AliasClass`, `PersistPageDecl`, or `CommitGroupDecl`; the maps on
`StoragePlan.provenance` are the single source of truth. This mirrors
F-B5's discipline of putting provenance on the IR product, not on
individual nodes.

### 8.7 Construction order

The pure core constructs `StoragePlan` in a specific order so that
self-consistency checks can run incrementally without backtracking:

```text
build_storage_plan_core(inputs):
  1. canonicalize inputs (verify upstream product hashes match,
     including policy.canonical_hash == inputs.policy_hash)
  2. build the typed-predicate environment over (qg, iir, op, rp, policy)
  3. for every ValueId v ∈ iir.values:
       3a. derive (op_output_role, op_output_format) from iir.value_decls
       3b. evaluate the decision rules of §9 in priority order
       3c. emit a tentative StorageBinding
  4. build persist_pages and commit_groups from tentative Persist bindings
  5. validate persist invariants that do not depend on alias classes (§10)
  6. construct initial alias-class equivalence classes:
       6a. seed each binding with a fresh singleton AliasClassId
       6b. collect candidate alias edges from §11.4, each tagged with
           exactly one AliasIntent
       6c. reject any connected component containing edges with more
           than one AliasIntent (emit STORE-031)
       6d. for ScratchReuse, require pairwise live-range disjointness
           across the entire connected component, not merely along
           each edge
       6e. for PingPong and PersistRotation, enforce the intent's
           cardinality constraints before unioning (emit STORE-032
           on violation)
       6f. canonicalize alias-class identifiers by sorting connected
           components by AliasClassFingerprint and assigning dense
           AliasClassIds in that order
  7. apply alias-class-level forced recompute selectors, if any:
       7a. match selectors against the initial alias partition
       7b. require RecomputeAllowed(v) for every selected member
       7c. convert selected members to Recompute
       7d. split selected members into singleton alias classes
       7e. recompute affected alias-class ids
  8. validate AliasIntent vs Materialization (§8.3)
  9. validate alias-dependent persist invariants (§10, §11)
  10. validate cross-stage invariants (§15)
  11. compute report self-hash
  12. emit ReportEnvelope<StoragePlanReportBody>
```

Steps 3a, 3b, and 3c are local to one `ValueId`; steps 4–7 are
whole-plan invariants. The two phases are explicit so that diagnostics
can be tagged either as "binding-local" (`ValueId` provenance) or
"plan-wide" (no `ValueId` provenance, only typed reason).

### 8.8 Self-consistency invariants

The product carries the following self-consistency invariants. Every
invariant is mechanically checkable and is part of the closure gate.

```text
SC1 BindingCoverage:
  ∀ ValueId v ∈ iir.values.
    ∃! StorageBinding b ∈ SP.bindings. b.value = v.

SC2 BindingFunctional:
  SP.bindings is a function from ValueId → StorageBinding.
  (No double-bind on any ValueId.)

SC3 AliasClassWellFormed:
  ∀ AliasClassId A ∈ SP.alias_classes.
    SP.alias_classes[A].id = A.
    SP.alias_classes[A].members is a non-empty sorted set.
    ∀ v ∈ SP.alias_classes[A].members. SP.bindings[v].alias_class = A.

SC4 AliasMembershipFunctional:
  ∀ ValueId v.
    SP.bindings[v].alias_class ∈ SP.alias_classes.

SC5 RecomputeAliasIsolation:
  ∀ ValueId v with SP.bindings[v].materialization = Recompute.
    SP.bindings[v].alias_class is a singleton in SP.alias_classes
    and that singleton has intent = NoAlias.
  (Recomputed values do not alias materialized values.)

SC6 PersistAliasRotationOnly:
  ∀ ValueId v with SP.bindings[v].materialization = Persist {..}.
    SP.bindings[v].alias_class is either a singleton OR has
    AliasIntent::PersistRotation.

SC7 PersistPageReferenced:
  ∀ PersistPageId p ∈ SP.persist_pages.
    ∃ ValueId v.
      SP.bindings[v].materialization = Persist { page: p, commit_group: _ }.

SC8 CommitGroupReferenced:
  ∀ CommitGroupId g ∈ SP.commit_groups.
    ∃ ValueId v.
      SP.bindings[v].materialization = Persist { page: _, commit_group: g }.

SC9 CommitGroupMembership:
  ∀ CommitGroupId g.
    SP.commit_groups[g].members =
      { p : ∃ v. SP.bindings[v].materialization = Persist { page: p,
                                                            commit_group: g } }
    ∧ |SP.commit_groups[g].members| ≥ 1.

SC10 LifetimeBoundsAdmissible:
  ∀ ValueId v.
    MinRequiredLifetime(v) ≤ LifetimeOf(SP.bindings[v])
    ∧ LifetimeOf(SP.bindings[v]) ≤ MaxAdmissibleLifetime(v)
    where MinRequiredLifetime(v) is derived from:
      - ObservationPlan stability requirements for observed values;
      - RangePlan scratch/reduction constraints for reduction values;
      - persistence and routing-stability requirements;
    and MaxAdmissibleLifetime(v) is derived from:
      - QuantGraph provenance when v is backed by a persistent or
        immutable graph entity;
      - GbInferIR def-use structure for intermediate values;
      - target policy constraints.

SC11 NoForbiddenStorageEnums:
  storage_plan.json contains no object key or enum tag from
  ForbiddenStage6SpatialSurface:

    {
      "byte_offset", "byte_alignment", "byte_address",
      "concrete_bank", "rom_bank", "sram_bank",
      "slice_id", "lease_id", "overlay_region", "overlay_install",
      "page_byte_address", "kernel_residency",
      "sram_page_family_id", "sram_working_set_id",
      "ResourceVector", "SchedSlice", "ResidencyEpoch",
      "OverlayId", "OverlayInstall", "KernelResidency",
      "BankClass", "RomBank", "SramBank", "Residency"
    }

  Legal Stage 6 enum tags such as `WramHot`, `SramPaged`, and
  `RomConst` are not violations.

SC12 IdentityWellFormed:
  SP.input_identity.{quant_graph_hash, infer_ir_hash,
                     observation_plan_hash, range_plan_hash,
                     policy_hash} match the input hashes.
  The envelope's report_self_hash matches SelfHash(envelope(body))
  under the inherited self-hash convention; it is owned by the
  ReportEnvelope, not by SP.input_identity.
```

`SC11` is the negative invariant that pins what F-B8 is NOT producing.
The list is exhaustive; any new spatial enum introduced by a later RFC
must be added to `SC11` if it must remain absent from F-B8's output.

### 8.9 `LifetimeOf` and op-output role/format predicates

```text
LifetimeOf : StorageBinding → LifetimeClass

LifetimeOf(b) =
  match b.materialization with
  | Recompute                           ⇒ Slice  (recomputed values
                                                  effectively live for
                                                  one slice each time
                                                  they are recomputed)
  | Materialize { lifetime, .. }        ⇒ lifetime
  | Persist { .. }                      ⇒ Persistent
```

`ValueRole` and `ValueFormat` are typed predicates over `(NodeId,
ValueId)` derived from `GbInferIR.value_decls` and the canonical
reference semantics of `InferOp` (per F-B3/F-B5 §9). The closed
enumeration in v1 is:

```rust
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum ValueRole {
    /// Value flowing through canonical model semantics; no special role.
    Activation,
    /// Accumulator for a reduction site (RangePlan-bound).
    Accumulator,
    /// Per-tile or per-chunk scratch for a reduction (RangePlan-bound).
    Scratch,
    /// Routing decision (selected expert id, gate weight) — MUST stay
    /// stable across the routed-FFN dispatch.
    RouterDecision,
    /// Routing score (raw or normalized router logits).
    RouterScore,
    /// Routing-table weight tensor read-only constant.
    RouterWeight,
    /// Embedding-table read-only constant.
    EmbeddingTable,
    /// Logit projection read-only constant.
    LogitProj,
    /// Norm scale/bias parameter read-only constant.
    NormParam,
    /// Expert weight tensor read-only constant.
    ExpertWeight,
    /// Sequence-state slot (per-layer, per-stream).
    SequenceStateSlot,
    /// Decode-spec read-only constant (e.g. tied lookup tables).
    DecodeConst,
    /// External token input (the unique TokenInput value).
    InputToken,
    /// Output token from DecodeToken.
    OutputToken,
    /// LUT or constant table fragment.
    LutFragment,
    /// FFN intermediate activation.
    FfnIntermediate,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum ValueFormat {
    /// Quantized integer value; format references QuantSpec entries.
    QuantInt    { quant_format_id: QuantFormatId },
    /// Floating-point reference value (canonical reference semantics
    /// only; no production deployment uses this format on hardware).
    FloatRef    { precision: FloatPrecision },
    /// Integer accumulator domain (i16, i32, etc.).
    IntAccum    { width_bits: u8 },
    /// Token-id domain (vocab-bounded u16/u32).
    TokenIdDomain { vocab_size: u32 },
    /// Bool flag domain.
    Flag,
    /// Opaque pointer to a const tensor (RomConst-only).
    ConstTensorRef { tensor_id: TensorId },
}
```

Both enums are closed in v1. The role enum is the **canonical role**
of the value in the IR-level semantics; the format enum is the
**element-level numeric format** of the value's data. Roles and formats
combine into typed predicates the decision rules consume:

```text
IsHotScalar(v) :=
  let b = inferred_role_format(v) in
  b.role ∈ {Accumulator, RouterScore, RouterDecision}
  ∧ b.format ∈ {IntAccum {width_bits: 8 | 16},
                Flag,
                TokenIdDomain {..}}
  ∧ logical_byte_size(v) ≤ 128

IsLargeActivation(v) :=
  let b = inferred_role_format(v) in
  b.role ∈ {Activation, FfnIntermediate}
  ∧ logical_byte_size(v) > WramHotPerValueEligibilityCeiling(policy)

IsExpertWeight(v) :=
  let b = inferred_role_format(v) in
  b.role = ExpertWeight
  OR (b.role = LutFragment ∧ ∃ ExpertSection.aux LUT linkage)

IsRouterTable(v) :=
  let b = inferred_role_format(v) in
  b.role ∈ {RouterWeight, EmbeddingTable, LogitProj}

IsSequenceStateSlot(v) :=
  let b = inferred_role_format(v) in
  b.role = SequenceStateSlot

IsRenormLoopScratch(v) :=
  ∃ ReductionSiteRef site.
            ValueDeclsOf(site.node) ∋ v
            ∧ ReductionPlanFor(site) = RenormLoop {..}
            ∧ inferred_role_format(v).role = Scratch

IsPureValue(v) :=
  no def-use path from v reaches an effect chain that mutates
  SequenceState or Rng, AND
  v is not the output of an InferOp variant whose canonical reference
  semantics depend on hidden state.

IsObservedCheckpointBackingValue(v) :=
  ObservationPlan declares a SemanticCheckpointId that requires v
  to be inspectable at a stable point.

RecomputeAllowed(v) :=
  IsPureValue(v)
  ∧ ¬IsObservedCheckpointBackingValue(v)
  ∧ ¬IsSequenceStateSlot(v)
  ∧ ValueRoleOf(v) ∉ {RouterDecision, RouterWeight, ExpertWeight,
                      EmbeddingTable, LogitProj, NormParam,
                      DecodeConst, LutFragment}
  ∧ effective_lifetime_estimate(v) = Slice

RoleKnown(v) :=
  role_of(v) successfully classified into the closed `ValueRole` set.

FormatKnown(v) :=
  format_of(v) successfully classified into the closed `ValueFormat` set.

LogicalSizeKnown(v) :=
  value_decl_of(v).shape and value_decl_of(v).format imply a finite
  logical byte size.
```

These predicates are pure functions of the inputs and are the contract
F-B8 publishes for the decision rules of §9.

### 8.10 Determinism

`StoragePlan` construction is deterministic. The construction order in
§8.7, the alias-class canonicalization (smallest ValueId-derived hash
wins), and the BTreeMap-backed binding/page/group maps together ensure
that two runs on the same inputs produce byte-identical
`storage_plan.json`. Determinism is required by `R-Hash` and by the
StageCache cache-hit safety property.

## 9. Decision rules / heuristics layer (typed)

### 9.1 The decision rules are typed predicates, not prose

Every materialization decision in F-B8 is the application of a typed
decision rule. The rules are organized as an ordered list and evaluated
in priority order. The first rule whose predicate evaluates to true on
a given `ValueId` wins. If it returns `Bind`, F-B8 emits the
corresponding tentative binding. If it returns `Reject`, F-B8 emits the
corresponding hard diagnostic and emits no partial product. If no rule
fires, F-B8 emits `StorageNoAdmittingDecisionRule`.

```rust
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct DecisionRuleId(pub u32);

pub struct DecisionRule {
    pub id: DecisionRuleId,
    pub name: &'static str,
    pub predicate: fn(&PredicateEnv, ValueId) -> bool,
    pub outcome: fn(&PredicateEnv, ValueId) -> DecisionRuleOutcome,
    pub priority: u32,
}

pub enum DecisionRuleOutcome {
    Bind(Materialization),
    Reject(DiagnosticCode),
}
```

The rule list is closed in v1 and authored entirely in this RFC. New
rules require explicit RFC amendment. The order is meaningful: rules
fire in priority order, so adding a new high-priority rule changes the
binding of every value previously bound by a lower-priority rule.

### 9.2 The closed v1 rule list

The thirteen v1 rules, in priority order, are:

```text
DR-1  ForcedRecomputeValueOverride
DR-1b IllegalForcedRecomputeValueOverride
DR-2  PersistSequenceStateSlot
DR-3a MaterializeResumeContinuation
DR-3  PersistContinuationRecord
DR-4  PersistTranscriptPage
DR-5  PersistHarnessOrTracePage
DR-6  RomConstExpertWeight
DR-7  RomConstRouterTable
DR-8  RomConstLut
DR-9  RomConstEmbeddingOrLogitProj
DR-10 RenormLoopScratch
DR-11 RecomputeForPureSliceValue
DR-12 HotScalarHram
DR-13 DefaultMaterializeWramHot
```

Each rule is fully typed. The predicate names below are defined in §8.9
and §11.

```text
DR-1 ForcedRecomputeValueOverride
  predicate: Value(v) ∈ policy.compile_knobs.overrides.forced_recompute
             ∧ RecomputeAllowed(v)
  binding:   Recompute
  rationale: A value-level override takes priority over default
             materialization, but it cannot override semantic
             non-recomputability. Alias-class-level forced-recompute
             selectors are applied later by §8.7 step 7, after the
             initial alias partition exists.

DR-1b IllegalForcedRecomputeValueOverride
  predicate: Value(v) ∈ policy.compile_knobs.overrides.forced_recompute
             ∧ ¬RecomputeAllowed(v)
  outcome:   Reject(
               if IsObservedCheckpointBackingValue(v) then STORE-006
               else STORE-033 StorageForcedRecomputeNotAllowed
             )
  rationale: Overrides are policy requests, not permission to violate
             observation, persistence, routing-stability, or constness
             invariants.

DR-2 PersistSequenceStateSlot
  predicate: IsSequenceStateSlot(v)
  binding:   Persist {
               page = persist_page_for_slot(v),
               commit_group = commit_group_for_slot(v)
             }
  rationale: Sequence-state slots are persistent state per
             planv0.md §"Persistent record protocol." v1 forbids
             non-identity sequence blocks (F-B3/F-B5 §2.5a), so this
             rule fires for zero values in v1; it is reserved shape.

DR-3a MaterializeResumeContinuation
  predicate: ValueRoleOf(v) = Activation
             ∧ v participates in resume across a yield boundary
             ∧ ¬MustSurvivePowerLoss(v)
  binding:   Materialize { class: WramHot, lifetime: ResumeWindow }
  rationale: A continuation that only crosses a cooperative yield
             inside a powered session is a hot WRAM value with a
             ResumeWindow lifetime; it is not a SRAM-persistent record.

DR-3 PersistContinuationRecord
  predicate: ValueRoleOf(v) = Activation
             ∧ ∃ slice s. v ∈ ContinuationLiveSet(s)
             ∧ v participates in resume across a yield boundary
                that must survive power loss
  binding:   Persist {
               page = continuation_page(),
               commit_group = continuation_commit_group()
             }
  rationale: The continuation record is represented as
             `PersistKind::Continuation` only when it must survive
             reset/power-loss under the SRAM persistence protocol.
             v1 evaluation: this rule never fires in v1 because
             slice boundaries are F-B13's authority and the IR-pass-
             level continuation is single-token. Reserved shape.

DR-4 PersistTranscriptPage
  predicate: ValueRoleOf(v) = OutputToken
             ∧ policy.transcript_capture.enabled = true
             ∧ logical_byte_size(v) > TranscriptInlineCeiling(policy)
  binding:   Persist {
               page = transcript_page(family),
               commit_group = transcript_commit_group(family)
             }
  rationale: Transcript pages are SRAM-persistent per planv0.md
             line 2117. v1 evaluation: the in-flight ObservationPlan
             may not yet expose transcript_capture; if absent,
             treat as Disabled and skip this rule.

DR-5 PersistHarnessOrTracePage
  predicate: (
               ValueRoleOf(v) ∈ {InputToken, OutputToken}
               ∧ v participates in a HarnessOp ingress/egress
             )
             OR (
               v is a TraceProbe-attached value
               ∧ TraceCapturePolicy admits capture for v
             )
  binding:   Persist { page, commit_group }
  rationale: Harness command/result blocks (planv0.md lines 2118–2119)
             and trace pages live in SRAM. v1 evaluation: harness
             rule fires only when the build profile is Harness or
             Bringup; trace rule fires only when policy enables
             trace probes for v's anchor.

DR-6 RomConstExpertWeight
  predicate: IsExpertWeight(v)
  binding:   Materialize { class: RomConst, lifetime: Persistent }
  rationale: Expert weights are immutable and are loaded from ROM
             (planv0.md lines 2076–2087, ExpertBanks). LifetimeClass
             is Persistent because ROM contents persist across
             power cuts; F-B10 RomWindowPlan decides bank residency.

DR-7 RomConstRouterTable
  predicate: IsRouterTable(v) ∧ ValueRoleOf(v) = RouterWeight
  binding:   Materialize { class: RomConst, lifetime: Persistent }
  rationale: Router weights live in CommonBanks (planv0.md line 2026).

DR-8 RomConstLut
  predicate: ValueRoleOf(v) = LutFragment
             -- v1 has no staged-LUT input to F-B8.
  binding:   Materialize { class: RomConst, lifetime: Persistent }
  rationale: LUT fragments live in ROM unless an overlay-promoting
             pass (F-B11 OverlayPlan) has marked them for staging,
             which v1 does not expose.

DR-9 RomConstEmbeddingOrLogitProj
  predicate: ValueRoleOf(v) ∈ {EmbeddingTable, LogitProj, NormParam,
                                DecodeConst}
  binding:   Materialize { class: RomConst, lifetime: Persistent }
  rationale: All other named const tensors live in ROM CommonBanks.

DR-10 RenormLoopScratch
  predicate: IsRenormLoopScratch(v)
  binding:   Materialize { class: WramHot, lifetime: Slice }
  rationale: RenormLoop tile scratch must be hot and short-lived.
             HramHot is permitted only when an explicit storage-class
             override surface exists; v1 emits WramHot by default
             (HramHot is 127 bytes total and is contended by ISR
             scratch).

DR-11 RecomputeForPureSliceValue
  predicate: IsPureValue(v)
             ∧ ¬IsRenormLoopScratch(v)
             ∧ effective_lifetime_estimate(v) = Slice
             ∧ policy.compile_knobs.global.storage.recompute_promotion
               ≥ PureSliceValues
             ∧ recompute_cost_estimate(v).cycles ≤
               recompute_cycle_ceiling(policy)
  binding:   Recompute
  rationale: When the policy admits PureSliceValues recompute and v's
             lifetime is bounded by one slice, recomputation is cheaper
             than spilling. This is the first refinement-loop-controlled
             rule. RenormLoop scratch is excluded so it is materialized
             hot by DR-10, never recomputed.

DR-12 HotScalarHram
  predicate: IsHotScalar(v)
             ∧ effective_lifetime_estimate(v) = Slice
             ∧ v ∈ PrecomputedHramAdmittedSet
  binding:   Materialize { class: HramHot, lifetime: Slice }
  rationale: Hot scalars (router score, accumulator) under tight HRAM
             budget land in HRAM. Admission is global and
             precomputed (see §9.3) so the cumulative budget is
             checked once, not via local first-match.

DR-13 DefaultMaterializeKnownIntermediate
  predicate: RoleKnown(v)
             ∧ FormatKnown(v)
             ∧ LogicalSizeKnown(v)
             ∧ ValueRoleOf(v) ∉ {
                 SequenceStateSlot,
                 RouterWeight,
                 ExpertWeight,
                 EmbeddingTable,
                 LogitProj,
                 NormParam,
                 DecodeConst,
                 LutFragment
               }
  binding:   Materialize {
               class: derived from logical_byte_size(v),
               lifetime: derived from value's longest live edge
             }
             where:
               class =
                 if logical_byte_size(v) ≤
                    WramHotPerValueEligibilityCeiling(policy)
                   then WramHot
                   else SramPaged
               lifetime =
                 longest_live_lifetime(v)
                   = match longest_live_window(v)
                       | within_one_slice                 ⇒ Slice
                       | crosses_one_yield_within_token   ⇒ ResumeWindow
                       | crosses_token_boundary           ⇒ Token
                       | crosses_session_boundary         ⇒ Session
                       | crosses_power_cut                ⇒ Persistent
  rationale: Any value that did not match a higher-priority rule lands
             in WRAM if it is individually eligible, or SRAM if a
             single value is too large. This is not a cumulative WRAM
             fit proof; cumulative pressure is summarized in
             StoragePlanSummary.abstract_pressure and may trigger
             repair proposals.
```

### 9.3 Predicate semantics

The predicates referenced above are precisely defined:

```text
effective_lifetime_estimate(v): LifetimeClass
  = max over all NodeIds {n_def, n_use_1, n_use_2, ...}
        of slice_class_estimate(GbInferIR, n_*)
  where slice_class_estimate is a typed function from a NodeId to a
  LifetimeClass that returns the conservative upper bound based on
  GbInferIR's def-use graph and the IR-pass single-token convention
  (F-B3/F-B5 §2.5).

  Conservative means:
    - if v's def-use chain stays within one logical reduction site
      and no checkpoint anchor crosses it, return Slice;
    - else if v's def-use chain crosses one yield boundary within
      the token (e.g. resume after wakeup), return ResumeWindow;
    - else if v's def-use chain spans the whole IR pass, return
      Token;
    - else (sequence-state, transcript) return Session/Persistent.

  v1: ResumeWindow and Session are conservatively mappable to Token
  because slice boundaries are F-B13's authority. The conservative
  fallback is documented as "v1 collapse" and is mechanically
  identifiable in the provenance.

logical_byte_size(v): u32
  = element_count(value_decl_of(v).shape)
    × bytes_per_element(value_decl_of(v).format)

WramHotPerValueEligibilityCeiling(policy): u32
  = checked_sub(
      policy.storage_pressure_budget.wram_hot.soft_bytes,
      policy.runtime_chrome_budget.wram_hot.reserved_bytes
    )
  or STORE-034 StoragePolicyBudgetUnderflow.

AllocatableHramBudget(policy): u32
  = checked_sub(
      policy.storage_pressure_budget.hram_hot.soft_bytes,
      policy.runtime_chrome_budget.hram_hot.reserved_bytes
    )
  or STORE-034 StoragePolicyBudgetUnderflow.
  (HramHot reservations include ISR scratch and active-page shadow
  registers per planv0.md lines 2106–2112.)

soft_pressure_threshold_bytes(policy): u32
  = policy.storage_pressure_budget.wram_hot.soft_bytes
    × SOFT_PRESSURE_FRACTION
  where SOFT_PRESSURE_FRACTION is a typed rational with default
  `{ numerator: 85, denominator: 100 }`,
  policy-overridable via CompileKnobs::storage in a future RFC
  amendment (v1: hardcoded constant in this RFC). Encoding as an
  explicit numerator/denominator keeps F-B8 within the inherited
  integers-only canonical JSON rule.

recompute_cycle_ceiling(policy): u32
  = policy.compile_knobs.global.storage.recompute_cycle_ceiling
    or the v1 default `RECOMPUTE_CYCLE_CEILING`.

recompute_cost_estimate(v): CostEstimate
  = derived from GbInferIR.value_decls[v].producing_node and the
  canonical-reference-semantics complexity class of that node's op.
  v1: heuristic based on op-class (matvec ⇒ expensive; norm ⇒
  cheap; embedding ⇒ table lookup; clamp ⇒ trivial).
  Carries `cycles: u32` as the dimensioned scalar compared against
  `recompute_cycle_ceiling`. F-B8 never compares cycles to bytes.

PrecomputedHramAdmittedSet:
  Before evaluating DR-12, collect all `IsHotScalar` candidates, sort
  them deterministically by:
    1. role priority: RouterDecision, RouterScore, Accumulator, Flag
    2. smaller logical_byte_size first
    3. ValueId canonical order
  Admit candidates greedily until adding the next candidate would
  exceed `AllocatableHramBudget(policy)`.

  Candidates not admitted by this set fall through to DR-13.

ContinuationLiveSet(s): SortedSet<ValueId>
  = the values that must survive yield boundary s.
  v1: empty set (slice boundaries are F-B13's authority; the
  IR-pass-level continuation is single-token, so no value in
  the v1 IR pass survives a yield boundary inside the same pass).
```

### 9.4 Rule ordering rationale

The priority order is constructed to satisfy three properties:

```text
Property P1 (Override-first):
  User overrides (DR-1) and persistence rules (DR-2..DR-5) fire before
  default placement rules (DR-6..DR-13).
  Rationale: persistence is a contract with the runtime that cannot be
  re-derived from byte sizes; overrides reflect user knowledge that
  overrules the default heuristic.

Property P2 (Const-before-volatile):
  RomConst rules (DR-6..DR-9) fire before recompute and default
  materialize rules.
  Rationale: ROM-resident constants are immutable and have the
  cheapest possible read latency; placing them in RomConst is
  almost never wrong.

Property P3 (RenormLoop-before-recompute):
  DR-10 (RenormLoopScratch) fires before DR-11 (RecomputeForPureSlice)
  so pure scratch values that must remain hot are never converted to
  Recompute. DR-11 then fires before DR-12, DR-13 because recompute is
  a positive decision for the remaining pure values.

Property P4 (Hot-before-paged):
  DR-10 (RenormLoopScratch ⇒ WramHot/Slice) and DR-12 (HotScalarHram
  ⇒ HramHot/Slice) fire before DR-13 (DefaultMaterializeWramHot)
  because hotness is a hard requirement for those values; the default
  rule is the catchall.
```

### 9.5 Recompute promotion as the refinement-loop hook

`RecomputePromotionLevel` is the only `StorageKnobs` knob in v1. It
gates DR-11. The promotion ladder, per `planv0.md` line 1238, is:

```text
RecomputePromotionLevel := None
                         | PureSliceValues
                         -- PureResumeWindowValues and PureTokenValues
                         -- are reserved for a later RFC
```

The order is monotonic. Advancing one step admits more values into the
recompute decision. F-B8 emits a `RepairProposal` when:

* DR-13 fires for a value that would have been admitted to DR-11 under
  a higher promotion level, AND
* the WRAM hot pressure (or HRAM hot, or SRAM paged) is above its soft
  threshold.

The proposal is:

```rust
RepairProposal {
    source: PlanningStage::StoragePlan,
    reason: RepairReason::PromoteRecompute,
    tighten: ConstraintDelta {
        changes: vec![
            KnobDelta::PromoteRecomputeLevel { to: <next level> },
        ],
    },
    estimated_cost: EstimatedCostDelta {
        cycles: Some(...),    // sum of recompute_cost_estimate
                              // over the values that would be promoted
        bytes: Some(...),     // sum of logical_byte_size
                              // over the values that would be promoted
        ...
    },
}
```

F-B8 emits the proposal but does not apply it. F-B16's loop driver,
when it lands, may accept or reject the proposal under
`RepairPolicy::allow_recompute_promotion`. In v1 (pre-F-B16),
proposals are emitted and recorded in the report's
`repair_proposals` field but never fed back. The wired-but-not-driven
state is intentional: it lets us verify the typed handshake before
F-B16's acceptance logic exists.

`KnobDelta::ForceRecompute { values }` (per `planv0.md` line 1450) is
emitted when DR-1 would have admitted a specific `ValueId` if the
override were present. F-B8 emits `KnobDelta::ForceRecompute` proposals
for individual values rather than a bulk `PromoteRecomputeLevel`
when the recompute decision can be localized to a small set.

### 9.6 What the rules do NOT decide

The decision rules of §9.2 do NOT decide:

* concrete byte offsets (F-B12);
* concrete WRAM regions or SRAM pages (F-B12 / F-B9);
* concrete ROM banks (F-B10);
* overlay regions or install schedules (F-B11);
* slice boundaries or yield kinds (F-B13);
* which kernel implements an op (F-H1 / F-B13);
* schedule equivalence classes (F-C3);
* observation/probe attachment (F-B6).

These are explicit non-decisions; F-B8's diagnostic algebra (§14)
rejects any binding that pretends to make one of them.

### 9.7 Examples (v1 routed-FFN dense fixture)

The following examples sketch the rule firings for the M3 routed-FFN
dense fixture. The fixture has one routed FFN layer with eight experts
and `RouteTop1`. The exact `ValueId`s are fixture-internal; the role
classification is what matters.

```text
Example A: expert weight tensor for layer 0, expert 3
  ValueRole = ExpertWeight, ValueFormat = ConstTensorRef
  Predicates fired in order:
    DR-1  no
    DR-2..DR-5  no (not a persist value)
    DR-6  IsExpertWeight(v) ⇒ YES
  Binding:
    Materialize { class: RomConst, lifetime: Persistent }
  Alias class: singleton (RomConst values do not alias across experts).

Example B: router top-1 selected expert id (small u8)
  ValueRole = RouterDecision, ValueFormat = TokenIdDomain { vocab = 8 }
  Predicates fired in order:
    DR-1  no
    DR-2..DR-9  no
    DR-10 IsRenormLoopScratch(v) ⇒ no
    DR-11 IsPureValue(v) — but RouterDecision is excluded from
          RecomputeAllowed (routing stability), so the rule does NOT
          fire even when recompute_promotion ≥ PureSliceValues
    DR-12 IsHotScalar(v) ⇒ YES (admitted by PrecomputedHramAdmittedSet)
  Binding:
    Materialize { class: HramHot, lifetime: Slice }
  Alias class: NoAlias singleton in v1. RouterDecision is HramHot and
               is excluded from PP-PingPong, which is WramHot-only and
               activation/FFN-intermediate-only.

Example C: ChunkedI16 accumulator scratch for layer 0 expert 3
  ValueRole = Accumulator, ValueFormat = IntAccum { width_bits: 16 }
  Predicates fired in order:
    DR-1  no
    DR-2..DR-9  no
    DR-10 IsRenormLoopScratch(v) ⇒ no  (ChunkedI16, not RenormLoop)
    DR-11 IsPureValue(v) ⇒ YES, but lifetime = Slice and
          recompute_promotion = None ⇒ rule does NOT fire
    DR-12 IsHotScalar(v)? logical_byte_size = N×i16, exceeds the hot
          scalar size envelope for non-tiny N ⇒ no
    DR-13 default ⇒ Materialize { class: WramHot, lifetime: Slice }
  Alias class: ScratchReuse with another accumulator/scratch value only
               if their abstract live ranges are pairwise disjoint;
               otherwise NoAlias singleton. Accumulator values are not
               PP-PingPong candidates in v1.

Example D: ResumeWindow activation that crosses one yield (post-router,
           pre-expert-dispatch)
  ValueRole = Activation, ValueFormat = QuantInt
  Predicates fired in order:
    DR-1  no
    DR-2..DR-9  no
    DR-10 ⇒ no (not RenormLoop scratch)
    DR-11 ⇒ no (lifetime is ResumeWindow, not Slice; v1 promotion
          level None)
    DR-12 ⇒ no
    DR-13 default ⇒ Materialize { class: WramHot, lifetime: ResumeWindow }
  Alias class: ResumeOverlap with no scratch overlap (NoAlias singleton
               in v1).

Example E: SequenceState slot for layer 0 (REJECTED in v1)
  ValueRole = SequenceStateSlot
  Predicates fired in order:
    DR-1  no
    DR-2 IsSequenceStateSlot(v) ⇒ YES
  Binding attempt: Persist { page, commit_group }
  Diagnostic:
    InferIrSequenceSemanticsUnsupportedV1 (already emitted by F-B5)
    StoragePersistSequenceStateUnsupportedV1 (emitted by F-B8 if F-B5
      did not catch it; defensive check)
  Result: hard reject; no binding emitted.
```

The rules are designed to handle Examples A–D without manual override
and to reject Example E with a typed diagnostic.

## 10. Persistence binding

### 10.1 The persistence contract is plug-compatible with the runtime

Per `planv0.md` lines 2138–2208, persistent state in SRAM is governed
by the persistent-record protocol:

* `PersistHeader` carries `magic`, `kind`, `page_state`, `state_schema`,
  `artifact_hash`, `semantic_state_hash`, `resume_abi_hash`,
  `build_identity_hash`, `generation`, `durability`, `checksum`.
* `PersistGroupCommit` carries `id`, `generation`, `member_mask`,
  `checksum`, `commit_word`.
* `PageState` is a typed state machine: `Writing → Committed →
  Retired`.
* Boot validates the newest committed group via per-kind compatibility
  rules (`SequenceState` validates `semantic_state_hash`,
  `Continuation` validates `resume_abi_hash`,
  harness/trace validate `build_identity_hash`).

F-B8's `Persist { page, commit_group }` binding is the compiler-side
identifier for one element of this protocol. The compiler emits typed
ids; the runtime interprets them under the protocol. F-B8 must
guarantee:

```text
F-Persist-PlugCompat:
  ∀ PersistPageId p emitted by F-B8.
    ∃ PersistKind k. SP.persist_pages[p].kind = k.
    ∃ PersistSchemaPin sp. SP.persist_pages[p].schema_pin = sp.

  ⇒ at runtime, gbf-runtime::persistence can construct a PersistHeader
    for p with kind = k and the appropriate {semantic_state_hash,
    resume_abi_hash, build_identity_hash} populated per sp.

F-Persist-CommitGroupAtomic:
  ∀ CommitGroupId g emitted by F-B8.
    SP.commit_groups[g].atomicity = AllOrNothing  (v1)
    SP.commit_groups[g].members ≠ ∅
    SP.commit_groups[g].kind_set ⊆ {SequenceState, Transcript,
                                     Continuation, Harness, Trace}

  ⇒ at runtime, gbf-runtime::persistence may emit a PersistGroupCommit
    manifest for g where every page in members has been committed.
    Boot resumes only the newest fully committed group (planv0.md
    line 2198).
```

The plug-compatibility is not a stylistic preference; it is the only
way to guarantee that compiler-emitted persist bindings can be realized
by the runtime persistence module without further negotiation.

### 10.2 Per-kind binding rules

Each `PersistKind` has typed binding rules:

```text
PersistKind::SequenceState
  Source:        SequenceStateSlot (v1: never fires; F-B5 §2.5a rejects
                 non-identity sequence blocks)
  CommitGroup:   PerSequenceStateSlot (one group per slot) by default;
                 may be merged with Transcript via
                 SequenceStateWithTranscript reason.
  SchemaPin:     requires_semantic_state_hash = true
                 requires_resume_abi_hash = false
                 requires_build_identity_hash = false
  Durability:    Critical (default) or Recoverable (configurable;
                 v1 hardcodes Critical until calibration evidence
                 supports Recoverable).

PersistKind::Continuation
  Source:        ContinuationLiveSet(slice) (v1: never fires;
                 slice boundaries are F-B13's authority)
  CommitGroup:   ContinuationWithSequenceState (one group per
                 (continuation, sequence-state) pair) by default;
                 reserved shape in v1.
  SchemaPin:     requires_semantic_state_hash = false
                 requires_resume_abi_hash = true
                 requires_build_identity_hash = false
  Durability:    Recoverable (continuation can always be cold-started).

PersistKind::Transcript
  Source:        OutputToken capture, when ObservationPlan enables it
  CommitGroup:   Independent OR SequenceStateWithTranscript
  SchemaPin:     requires_semantic_state_hash = false
                 requires_resume_abi_hash = false
                 requires_build_identity_hash = true
  Durability:    BestEffort when Independent.
                 Critical when grouped with SequenceState under
                 SequenceStateWithTranscript, because the transcript
                 delta must roll back atomically with sequence state.

PersistKind::Harness
  Source:        HarnessOp ingress/egress
  CommitGroup:   Independent (harness is per-command)
  SchemaPin:     requires_semantic_state_hash = false
                 requires_resume_abi_hash = false
                 requires_build_identity_hash = true
  Durability:    BestEffort.

PersistKind::Trace
  Source:        TraceProbe-attached values, when policy enables capture
  CommitGroup:   Independent
  SchemaPin:     requires_semantic_state_hash = false
                 requires_resume_abi_hash = false
                 requires_build_identity_hash = true
  Durability:    BestEffort.
```

The binding rules are typed and mechanically checkable. A Persist
binding whose `(page, commit_group, kind)` triple violates the
per-kind rule is a hard error (`StorePersistBindingKindMismatch`).

### 10.3 Commit-group well-formedness

A commit group is well-formed when every member is consistent with the
group's atomicity class and durability constraints.

```text
CG-Wf-1 NonEmpty:
  |SP.commit_groups[g].members| ≥ 1.

CG-Wf-2 KindCompatibility:
  ∀ p ∈ SP.commit_groups[g].members.
    SP.persist_pages[p].kind ∈ SP.commit_groups[g].kind_set.

CG-Wf-3 AllowedCrossKinds:
  SP.commit_groups[g].kind_set is one of:
    {SequenceState}                          (PerSequenceStateSlot)
    {SequenceState, Transcript}              (SequenceStateWithTranscript)
    {Continuation, SequenceState}            (ContinuationWithSequenceState;
                                              v1 reserved)
    {Continuation}                           (ContinuationOnly; v1 reserved)
    {Transcript}                             (Independent transcript)
    {Harness}                                (Independent harness)
    {Trace}                                  (Independent trace)
  Any other kind_set requires explicit RFC amendment.

CG-Wf-4 DurabilityConsistency:
  ∀ p ∈ SP.commit_groups[g].members.
    SP.persist_pages[p].durability is consistent across the group:
      a Critical page may share a group only with Critical pages;
      a Recoverable page may share a group with Critical or
        Recoverable pages;
      a BestEffort page may share a group only with BestEffort pages.
      a Transcript page grouped under SequenceStateWithTranscript is
        not BestEffort; it is promoted to Critical for that group
        (see §10.2 PersistKind::Transcript).
  (BestEffort pages must not be in the same group as state-critical
   pages; loss of a transcript page must not contaminate sequence-state
   recovery, per planv0.md line 2204.)

CG-Wf-5 GenerationsParallel:
  At runtime, all members of g share a generation counter. The
  PersistGroupCommit manifest for g pins this generation. F-B8 does
  not emit generations (those are runtime), but it emits the typed
  group such that the runtime can.

CG-Wf-6 NoOrphan:
  ∀ CommitGroupId g.
    ∃ ValueId v.
      SP.bindings[v].materialization = Persist {
        page: _, commit_group: g
      }
  (Mechanically equivalent to SC8 above; restated here for the persist
   context.)
```

`CG-Wf-3` is the closed cross-kind compatibility table for v1. Adding
a new entry is an RFC amendment.

### 10.4 What persistent binding does NOT include

`Persist { page, commit_group }` is a binding identifier. It does NOT
include:

* page byte size (runtime allocates the byte layout based on
  `state_schema` and `kind`);
* page byte offset in the active SRAM bank;
* SRAM bank id or page family selection (F-B9's authority);
* `PersistHeader.magic`, `checksum`, or `generation` (runtime owns
  these);
* the rotation strategy (double-buffered, ring-buffered) (F-B9 /
  runtime owns this);
* the read/write protocol for the page (runtime persistence module's
  authority);
* the recovery policy (boot resumes only the newest fully committed
  group; runtime owns this).

The line is sharp: F-B8 emits typed ids; F-B9 binds those ids to
page-family decisions; the runtime persistence module realizes them
as bytes, headers, and CRC-checked records.

### 10.5 Reserved persistence shape in v1

Several `PersistKind` variants are reserved shape in v1:

* `SequenceState`: F-B5 §2.5a rejects non-identity sequence blocks;
  no values with `ValueRole::SequenceStateSlot` exist in v1.
  `DR-2` is therefore unreachable in storage_plan.v1. The rule is
  present as a named reserved rule, but the v1 producer must reject
  any attempt to emit its binding with `STORE-007`. Enabling `DR-2`
  for production requires either:

  1. an explicit amendment to this RFC that lifts the reserved-v1
     gate, or
  2. a schema bump to `storage_plan.v2`.

  Keeping the rule name in v1 preserves the intended M4 shape; it
  does not make the shape legal to emit in v1.
* `Continuation`: slice boundaries are F-B13's authority. The IR-pass-
  level continuation in v1 is single-token; no value survives a yield
  inside the same IR pass. `DR-3` is therefore unreachable in v1.
* `Transcript`, `Harness`, `Trace`: these are reachable when the
  active build profile includes transcript capture, harness mode,
  or trace probes. v1 fixture builds may exercise harness mode and
  trace mode; transcript mode is reserved until the M4 transcript
  pipeline lands.

The closure gate (§17) explicitly verifies that v1 fixtures do not
exercise the reserved variants. Reserved-shape exercise is a hard
error.

## 11. Aliasing model

### 11.1 The alias-class layer is a typed equivalence relation

Aliasing in F-B8 is a typed equivalence relation over `ValueId`s. Two
values are alias-equivalent iff they share the same `AliasClassId`.
The relation is reflexive, symmetric, and transitive on the
`StoragePlan` product.

```text
F-Alias-Equivalence:
  AliasEquivalence ⊆ ValueId × ValueId
  is the smallest equivalence relation such that:
    (v_a, v_b) ∈ AliasEquivalence
      ⇔ SP.bindings[v_a].alias_class = SP.bindings[v_b].alias_class

  ⇒ AliasEquivalence is reflexive:
      ∀ v. (v, v) ∈ AliasEquivalence (every binding shares its own class)
  ⇒ AliasEquivalence is symmetric:
      (v_a, v_b) ∈ AliasEquivalence ⇒ (v_b, v_a) ∈ AliasEquivalence
  ⇒ AliasEquivalence is transitive:
      (v_a, v_b), (v_b, v_c) ∈ AliasEquivalence
        ⇒ (v_a, v_c) ∈ AliasEquivalence
```

The equivalence classes form a partition of `ValueId`. Every
`AliasClassId` corresponds to one block of the partition.

### 11.2 Why a typed equivalence relation, not a byte-range relation

Two reasons. First, byte-range overlap is `ArenaPlan`'s authority and
is not yet known at Stage 6. Second, `ResourceStateValidation`
(F-B13's stage 10.5) reasons about resource conflicts at the typed-
equivalence level, not at the byte level.

Concretely: two values that share an `AliasClassId` may end up at
distinct byte ranges in `ArenaPlan` (e.g. ping-pong activations); two
values with different `AliasClassId`s may end up at the same byte range
over disjoint lifetimes (when `ArenaPlan` decides to coalesce them
after F-B8 has proven their lifetimes disjoint). The equivalence
relation pinned by F-B8 is a guarantee about *intentional sharing*,
not about *byte-range coincidence*.

### 11.3 The `AliasIntent` declares the legitimate reason

Every alias class with more than one member carries a declared
`AliasIntent`. The intent is the typed reason that justifies sharing.
Without a declared intent, two values cannot share an alias class.

```text
AliasIntent::ScratchReuse
  Two short-lived scratch values share the same hot scratch buffer at
  distinct times. Lifetimes do not overlap.
  Allowed StorageClasses: WramHot, HramHot.
  Allowed LifetimeClasses: Slice (only).

AliasIntent::PingPong
  Two activation buffers used in alternating slices for double-buffered
  streaming. Lifetimes overlap, but writes do not conflict because
  the schedule alternates which is "active."
  Allowed StorageClasses: WramHot.
  Allowed LifetimeClasses: Slice, ResumeWindow.

AliasIntent::ResumeOverlap
  A value that survives a yield is allowed to overlap with scratch on
  the resume side, provided no conflicting writes. Used when a yield
  boundary makes scratch reusable across the boundary.
  Allowed StorageClasses: WramHot, HramHot.
  Allowed LifetimeClasses: Slice, ResumeWindow.

AliasIntent::PersistRotation
  Two persistent pages in a rotation pair (e.g. double-buffered
  SequenceState pages where one is Writing while the other is
  Committed). Lifetimes overlap by design; writes do not conflict
  because the runtime persistence protocol enforces that only one
  page is in PageState::Writing at a time.
  Allowed StorageClasses: SramPaged (via Persist binding only).
  Allowed LifetimeClasses: Persistent (always).
```

### 11.4 Construction of alias classes

The alias-class equivalence relation is constructed by union-find from
typed pair predicates:

```text
union_find:
  initial: every ValueId v starts in its own singleton class.
  for every (v_a, v_b) admitted by one of the pair predicates below:
    union(v_a, v_b) under the corresponding AliasIntent.

Pair predicates:

PP-ScratchReuse(v_a, v_b):
  SP.bindings[v_a].materialization = Materialize { class: hot, lifetime: Slice }
  SP.bindings[v_b].materialization = Materialize { class: hot, lifetime: Slice }
  ∧ ValueRoleOf(v_a) ∈ {Scratch, Accumulator}
  ∧ ValueRoleOf(v_b) ∈ {Scratch, Accumulator}
  ∧ AbstractLiveRange(v_a) and AbstractLiveRange(v_b) are disjoint
    in GbInferIR topological order.

PP-PingPong(v_a, v_b):
  SP.bindings[v_a].materialization = Materialize { class: WramHot,
                                                    lifetime: Slice | ResumeWindow }
  SP.bindings[v_b].materialization = Materialize { class: WramHot,
                                                    lifetime: Slice | ResumeWindow }
  ∧ ValueRoleOf(v_a) = ValueRoleOf(v_b) ∈ {Activation, FfnIntermediate}
  ∧ v_a and v_b are paired by GbInferIR's def-use as
    "alternating-tile" or "ping-pong activation," identified by a
    typed predicate IsPingPongPair(GbInferIR, v_a, v_b).

PP-ResumeOverlap(v_a, v_b):
  SP.bindings[v_a].materialization = Materialize { class: hot,
                                                    lifetime: ResumeWindow }
  SP.bindings[v_b].materialization = Materialize { class: hot,
                                                    lifetime: Slice }
  ∧ ResumeBoundary(GbInferIR, v_a, v_b) is a typed predicate that
    returns true when v_b's lifetime is entirely on one side of the
    yield that v_a survives.

PP-PersistRotation(v_a, v_b):
  SP.bindings[v_a].materialization = Persist { page: p_a, commit_group: g }
  SP.bindings[v_b].materialization = Persist { page: p_b, commit_group: g }
  ∧ p_a ≠ p_b
  ∧ SP.persist_pages[p_a].kind = SP.persist_pages[p_b].kind
  ∧ the runtime persistence protocol declares the (p_a, p_b) pair
    as a rotation-pair (default for PerSequenceStateSlot groups with
    double-buffered durability).
```

The four pair predicates are closed in v1. New pair predicates require
explicit RFC amendment. The closure of pair predicates is the closure
of the alias-class equivalence relation: F-B8 emits no alias edges
that are not justified by one of the four predicates.

### 11.5 Overlay-eligibility lens

F-B11 (`OverlayPlan`, Stage 8.5) consumes a derived view of
`StoragePlan` that exposes overlay-eligible bindings. The lens is:

```text
IsOverlayEligible(sp: StoragePlan, b: StorageBinding) :=
  b.materialization = Materialize { class: RomConst, .. }
  ∧ sp.provenance.bindings[b.value].op_output_role ∈
      Some({LutFragment, ExpertWeight, RouterWeight})
  -- v1 has no OverlayRegionSizeCeiling or overlay exclusion override
  -- available to F-B8; F-B11 may filter the candidate set later.
```

The lens is a typed predicate on F-B8's product, specifically on
`StoragePlan.bindings` plus `StoragePlan.provenance.bindings`.
F-B11 uses it to enumerate candidate overlay objects without
re-reading `GbInferIR`. F-B11 then decides which candidates actually
share an overlay region and what the install schedule is.

### 11.6 Resource-state-validation lens for F-B13

F-B13's `ResourceStateValidation` (Stage 10.5) reasons about
resource conflicts at the alias-class level. F-B8 exposes the
equivalence relation directly; F-B13 reads it without modification.

```text
F-Alias-NoConflictingWritesAcrossClass:
  ∀ alias class A.
    ∀ (v_a, v_b) ∈ A.members × A.members with v_a ≠ v_b.
      LiveRange(v_a) ∩ LiveRange(v_b) ≠ ∅
      ⇒ A.intent ∈ {PingPong, ResumeOverlap, PersistRotation}
        AND the schedule (F-B13) emits writes to v_a and v_b that
        are sequenced by the alias intent's coordination rule:
          PingPong:        alternating-slice writes;
          ResumeOverlap:   pre-yield writes vs post-yield writes;
          PersistRotation: writing-vs-committed page state.

F-Alias-NoConflictForScratchReuse:
  ∀ alias class A with intent = ScratchReuse.
    ∀ (v_a, v_b) ∈ A.members × A.members with v_a ≠ v_b.
      LiveRange(v_a) ∩ LiveRange(v_b) = ∅.
  (ScratchReuse never has overlapping abstract live ranges.)
```

`ResourceStateValidation` consumes these properties as preconditions
and emits a typed certificate (`certs/resource_state.cert.json`, per
`planv0.md` line 2829). F-B8's role is to make the properties
mechanically checkable on the product.

### 11.7 Alias-class identity is content-addressed

`AliasClassFingerprint` is content-addressed by the canonical sorted-set
of its member `ValueId`s plus the typed intent:

```text
AliasClassFingerprint(A) :=
  DomainHash(
    "gbf-codegen", "AliasClassId", "v1",
    CanonicalJson({
      members: SortedVec(A.members),
      intent:  A.intent
    })
  ).
```

`AliasClassId` is a dense deterministic local id assigned by sorting
classes by:

```text
(AliasClassFingerprint, CanonicalJson({ members, intent }))
```

If two distinct canonical alias-class payloads produce the same
`AliasClassFingerprint`, F-B8 emits `STORE-035
StorageAliasClassFingerprintCollision` rather than retrying with
non-canonical salt.

`AliasClassFingerprint` is the cross-build stable identity. Policy
selectors and report comparisons that need cross-run stability use the
fingerprint, not the dense id. F-B17's StageCache identity sweep
compares fingerprints to verify two cached products are truly
equivalent.

### 11.8 Singleton classes and the degenerate case

A binding whose alias class is a singleton (only one member) carries
the explicit intent `AliasIntent::NoAlias`. Singletons are emitted for:

* `Materialize { class: RomConst, .. }` bindings (RomConst values do
  not alias);
* `Recompute` bindings (per `SC5`, recomputed values do not alias
  materialized values);
* any binding whose default decision rule did not match a pair
  predicate.

The singleton-classes-only `StoragePlan` is the degenerate plan: every
value is in its own `NoAlias` singleton class. This is a valid v1
`StoragePlan` and is the closure-gate fixture for the M1-degenerate
build (per §0.5).

### 11.9 Alias classes do not commit to byte ranges

This is restated for emphasis. An `AliasClassId` is a **typed
equivalence-class identifier**, not a byte-range identifier. Two
values in the same alias class may end up at distinct byte ranges
(ping-pong is exactly this case) or at the same byte range (scratch
reuse with disjoint lifetimes), as `ArenaPlan` decides. The intent of
the alias class is the contract; the byte range is `ArenaPlan`'s
implementation.

```text
F-Alias-NoBytes:
  StoragePlan.alias_classes contains no field that mentions:
    byte offset, byte size, byte alignment, byte region, byte address,
    bank id, page id (for paged classes), WRAM region selector,
    SRAM region selector, ROM region selector.
  AliasClass.intent is the only typed reason; AliasClass.members is
  the only typed extent. No byte-range information.
```

## 12. Report schema — `storage_plan.json`

### 12.1 Top-level envelope

```json
{
  "schema": "storage_plan.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed" | "Failed",
  "report_self_hash": "<Hash256 hex>",
  "body": {
    "outcome": "Passed" | "Failed",
    "result": null | { ... StoragePlanReportResult ... },
    "diagnostics": [ ... ValidationDiagnostic[] ... ],
    "input_identity": { ... StoragePlanInputIdentity ... },
    "summary":  null | { ... StoragePlanSummary ... }
  }
}
```

The envelope law `R-FlatEnvelope` is inherited from F-B2/F-B4: `schema`,
`schema_version`, `outcome`, `report_self_hash` appear at the top level
of the JSON; `body` carries everything else.

### 12.2 `StoragePlanReportResult`

The non-null `result` carries the full `StoragePlan` typed product in
canonical JSON form:

```text
StoragePlanReportResult :=
  {
    input_identity: StoragePlanInputIdentityJson,
    bindings:       SortedVec<StorageBindingJson>,
    alias_classes:  SortedVec<AliasClassJson>,
    persist_pages:  SortedVec<PersistPageDeclJson>,
    commit_groups:  SortedVec<CommitGroupDeclJson>,
    repair_proposals: SortedVec<RepairProposalJson>,
    provenance:     StorageProvenanceJson
  }

StorageBindingJson :=
  {
    value:           ValueIdJson,
    materialization: MaterializationJson,
    alias_class:     AliasClassIdJson,
    live_range:      AbstractLiveRangeJson,
    justification:   BindingJustificationJson
  }

AbstractLiveRangeJson :=
  {
    def_node:          NodeIdJson,
    first_use_node:   null | NodeIdJson,
    last_use_node:    null | NodeIdJson,
    lifetime_class:   "Slice" | "ResumeWindow" | "Token"
                    | "Session" | "Persistent",
    checkpoint_stable: <bool>
  }

MaterializationJson :=
  | { "tag": "Recompute" }
  | { "tag": "Materialize",
      "class": "WramHot" | "HramHot" | "SramPaged" | "RomConst",
      "lifetime": "Slice" | "ResumeWindow" | "Token"
                | "Session" | "Persistent" }
  | { "tag": "Persist",
      "page": PersistPageIdJson,
      "commit_group": CommitGroupIdJson }

BindingJustificationJson :=
  | { "tag": "DecisionRule", "rule_id": <u32> }
  | { "tag": "ForcedRecompute" }

AliasClassJson :=
  {
    id:           AliasClassIdJson,
    fingerprint:  AliasClassFingerprintJson,
    members:      SortedVec<ValueIdJson>,
    intent:       "NoAlias" | "ScratchReuse" | "PingPong"
                | "ResumeOverlap" | "PersistRotation"
  }

PersistPageDeclJson :=
  {
    id:        PersistPageIdJson,
    kind:      "SequenceState" | "Continuation" | "Transcript"
             | "Harness" | "Trace",
    durability: "Critical" | "Recoverable" | "BestEffort",
    schema_pin: PersistSchemaPinJson
  }

PersistSchemaPinJson :=
  {
    state_schema:                      <u16>,
    requires_semantic_state_hash:      <bool>,
    requires_resume_abi_hash:          <bool>,
    requires_build_identity_hash:      <bool>
  }

CommitGroupDeclJson :=
  {
    id:        CommitGroupIdJson,
    members:   SortedVec<PersistPageIdJson>,
    kind_set:  SortedVec<PersistKindJson>,
    atomicity: "AllOrNothing"
  }

RepairProposalJson :=
  {
    source: "StoragePlan",
    reason: "PromoteRecompute",
    tighten: ConstraintDeltaJson,
    estimated_cost: EstimatedCostDeltaJson,
    proposal_id: <u32>
  }

StorageProvenanceJson :=
  {
    bindings:      SortedVec<{ key: ValueIdJson,
                               value: BindingProvenanceJson }>,
    alias_classes: SortedVec<{ key: AliasClassIdJson,
                               value: AliasClassProvenanceJson }>,
    persist_pages: SortedVec<{ key: PersistPageIdJson,
                               value: PersistPageProvenanceJson }>,
    commit_groups: SortedVec<{ key: CommitGroupIdJson,
                               value: CommitGroupProvenanceJson }>
  }
```

`SortedVec<T>` indicates the canonical-JSON encoding sorts the array by
`T`'s canonical-JSON byte order. Where a map shape is needed (provenance
sub-objects), F-B8 encodes it as a sorted vector of explicit
`{ key, value }` entries so the JSON layer never relies on non-string
object keys.

All id types used as JSON object keys (where any other section uses a
JSON object instead of the `SortedVec<{key,value}>` encoding) encode as
canonical lowercase string ids. Numeric object keys are forbidden.

### 12.3 `StoragePlanSummary`

The summary is a small projection of the product useful for
dashboards, reports, and downstream reasoning without fully
deserializing the bindings array.

```text
StoragePlanSummary :=
  {
    counts: {
      total_bindings:                  <u32>,
      bindings_recompute:              <u32>,
      bindings_materialize_wram_hot:   <u32>,
      bindings_materialize_hram_hot:   <u32>,
      bindings_materialize_sram_paged: <u32>,
      bindings_materialize_rom_const:  <u32>,
      bindings_persist:                <u32>,
      alias_classes:                   <u32>,
      alias_classes_singleton:         <u32>,
      alias_classes_no_alias:          <u32>,
      alias_classes_ping_pong:         <u32>,
      alias_classes_scratch_reuse:     <u32>,
      alias_classes_resume_overlap:    <u32>,
      alias_classes_persist_rotation:  <u32>,
      persist_pages:                   <u32>,
      commit_groups:                   <u32>,
      repair_proposals:                <u32>
    },
    abstract_pressure: {
      wram_hot_logical_bytes:    <u32>,
      hram_hot_logical_bytes:    <u32>,
      sram_paged_logical_bytes:  <u32>,
      rom_const_logical_bytes:   <u32>
    },
    rule_firings: BTreeMap<DecisionRuleIdJson, <u32>>,
    promotion_levels: {
      effective_recompute_promotion: "None" | "PureSliceValues"
    }
  }
```

The `abstract_pressure` is the sum of `logical_byte_size(v)` over all
bindings in each storage class. It is **not** a byte budget; it is a
sum of logical byte sizes. F-B12 (`ArenaPlan`) computes the actual
byte pressure, which may differ due to alignment, padding, and
scratch reuse.

### 12.4 Canonical JSON, self-hash, determinism

Inherited from F-B2/F-B4 §2.5:

* UTF-8, lex object keys, integers only.
* No NaN, no Inf, no `-0`.
* Arrays whose order is not semantically meaningful are sorted by
  element canonical-JSON bytes.
* `report_self_hash` is computed over the canonical-JSON bytes of the
  body with `report_self_hash` set to `ZERO_HASH`, then patched in.
* Two consecutive regenerations on a clean checkout produce
  byte-identical `storage_plan.json`.

### 12.5 Pass and fail variants

A `Passed` `storage_plan.json` carries:

* `outcome = "Passed"` at both the envelope and body level;
* `result` non-null;
* `diagnostics` empty (no Hard, no Soft, since `R-HardOnly-ThisChunk`
  applies);
* `body.input_identity` populated with the upstream and policy hashes,
  determinism class, and schema id/version;
* `body.result.input_identity = body.input_identity` (the result-level
  duplicate keeps the full typed product round-trippable; the body-level
  copy is the convenience view consumers read without parsing `result`);
* `summary` populated;
* `report_self_hash` computed at the envelope level only.

A `Failed` `storage_plan.json` carries:

* `outcome = "Failed"` at both the envelope and body level;
* `result = null` per `R-NoPartialIR-StoragePlan`;
* `diagnostics` non-empty, with at least one Hard diagnostic;
* `input_identity` populated with all upstream hashes;
  `report_self_hash` still computed at the envelope level; no nested
  field of the body carries `report_self_hash`;
* `summary = null`. Failed reports do not emit a degenerate summary.

### 12.6 Schema rejection

The schema has an explicit closed object key set. Unknown fields are a
`R-UnknownReject` violation:

```text
R-UnknownReject (storage_plan.v1):
  ∀ object o in storage_plan.json.
    ∀ field f ∈ o.
      f ∈ DeclaredKeys(o.schema) for o.schema = storage_plan.v1.

  ⇒ unknown fields in canonical JSON cause hard rejection at parse time.
```

The declared key set is fixed by §12.2 and §12.3 above. New fields
require explicit RFC amendment (or a schema bump to `storage_plan.v2`).

## 13. StageCache algebra — Stage 6

### 13.1 The K6 cache key

```text
K6 := DomainHash(
        "gbf-codegen",
        "StageCacheKey",
        "storage_plan.v1",
        "1.0.0",
        CanonicalJson(StoragePlanCacheKeyInputs)
      )

StoragePlanCacheKeyInputs :=
  {
    quant_graph_hash:       Hash256,
    infer_ir_hash:          Hash256,
    observation_plan_hash:  Hash256,
    range_plan_hash:        Hash256,
    policy_hash:            Hash256,
    determinism:            DeterminismClass,
    schema:                 ReportSchemaId,    -- == storage_plan.v1
    schema_version:         SemVer,            -- == 1.0.0
    decision_rule_set_hash: Hash256,           -- DomainHash over the
                                               -- canonical rule manifest:
                                               -- ids, names, priorities,
                                               -- semantic predicate ids,
                                               -- semantic binding ids,
                                               -- and RFC revision string
    persist_compat_hash:    Hash256,           -- DomainHash over the
                                               -- §10.2/§10.3 per-kind
                                               -- and cross-kind manifests
    alias_rule_set_hash:    Hash256            -- DomainHash over the
                                               -- §11.4 pair-predicate
                                               -- manifest and intent
                                               -- cardinality constraints
  }

DecisionRuleSetManifest :=
  SortedVec<{
    id: DecisionRuleIdJson,
    name: string,
    priority: u32,
    predicate_semantics: string,
    outcome_semantics: string,
    rfc_revision: string
  }>
```

The Stage 6 input identity consists of four upstream product hashes
(`quant_graph_hash`, `infer_ir_hash`, `observation_plan_hash`,
`range_plan_hash`) plus the resolved policy hash (`policy_hash`). The `determinism` class is
included because two builds with the same upstream products but
different determinism classes may produce different bindings (e.g.
`BitExact` may forbid certain recompute promotions that
`NumericallyStable` permits).

`decision_rule_set_hash`, `persist_compat_hash`, and
`alias_rule_set_hash` mechanically pin the semantic manifests for rule
firings, persist compatibility tables, and alias pair predicates,
respectively. They do not hash function pointers or compiler-specific
code addresses. Any amendment that changes those manifests changes the
corresponding hash, which invalidates K6 without needing a separate
string version field.

### 13.2 K6 success entries

A successful Stage 6 run writes a K6 success entry to the StageCache:

```text
StageCacheSuccessEntry<K6> :=
  {
    key:           K6,
    product:       StoragePlan,            -- typed product
    report_hash:   Hash256,                -- self-hash of envelope
    artifact_path: PathBuf                 -- where storage_plan.json
                                           -- is written
  }
```

A K6 cache hit replays the StoragePlan product without re-evaluating
the decision rules.

### 13.3 K6 failure memos

A failed Stage 6 run may write a K6 failure memo to the StageCache:

```text
StageCacheFailureMemo<K6> :=
  {
    key:         K6,
    diagnostics: SortedVec<ValidationDiagnostic>,
    report_hash: Hash256
  }
```

The memo records the typed Hard diagnostics that caused the failure.
A K6 cache hit on a failure memo replays the failure without
re-running the decision rules.

### 13.4 K6 invalidation

K6 entries are invalidated when:

* `quant_graph_hash`, `infer_ir_hash`, `observation_plan_hash`,
  `range_plan_hash`, or `policy_hash` changes;
* `determinism` class changes;
* `storage_plan.v1` schema is bumped to `storage_plan.v2`;
* `decision_rule_set_hash`, `persist_compat_hash`, or
  `alias_rule_set_hash` changes.

The invalidation is automatic via the cache key. There is no manual
invalidation surface.

### 13.5 K6 vs K0..K5 uniformity

K6 follows the same construction shape as K0 (artifact validation),
K0.5 (policy resolution), K1 (QuantGraph), K2 (StaticBudgetReport), K3
(GbInferIR), K4 (ObservationPlan, expected from F-B6), K5 (RangePlan,
expected from F-B7):

```text
K_n := DomainHash(crate, "StageCacheKey", schema_id, schema_version,
                  CanonicalJson(StageNCacheKeyInputs))
```

The uniformity is what makes F-B17's integration sweep possible: every
stage's cache key follows the same shape, so the cross-cut can verify
uniformity without per-stage bespoke logic.

## 14. Diagnostic algebra — `STORE-*` codes

### 14.1 Origin and code namespace

F-B8 introduces one new `ValidationOrigin`:

```rust
pub enum ValidationOrigin {
    // ...existing F-B2/F-B4 / F-B3/F-B5 / F-B6/F-B7 origins...
    StoragePlanConstruction,
}
```

All F-B8 diagnostics carry `origin = StoragePlanConstruction` and a
typed code from the `STORE-*` family below.

### 14.2 Closed code list

All F-B8 codes are closed in v1. Adding a new code requires explicit
RFC amendment.

```text
STORE-001  StorageNoAdmittingDecisionRule
  Severity: Hard
  Detail:   No decision rule of §9.2 fired for this ValueId.
  Fix:      Either add the value's role/format to the predicate of an
            existing rule (RFC amendment) or fix the upstream IR so
            the value's role is recognized.
  Provenance: ValueId, NodeId of producer, ValueRole, ValueFormat.

STORE-002  StorageBindingCoverageGap
  Severity: Hard
  Detail:   GbInferIR declares ValueId v but no StorageBinding
            references v.
  Fix:      Internal F-B8 bug; the construction order in §8.7 should
            visit every ValueId. If this fires, the rule list is
            incomplete or the iteration order skipped a node.
  Provenance: ValueId v, NodeId producer.

STORE-003  StorageBindingDoubleBind
  Severity: Hard
  Detail:   ValueId v has more than one StorageBinding in
            SP.bindings.
  Fix:      Internal F-B8 bug; SP.bindings is a function from
            ValueId → StorageBinding by SC2.
  Provenance: ValueId v, the two bindings.

STORE-004  StorageRomConstWriteViolation
  Severity: Hard
  Detail:   A ValueId bound Materialize { class: RomConst, .. } has a
            producer node whose op is not RomConst-eligible
            (e.g. it is the output of an effectful op or a non-
            constant tensor reference).
  Fix:      Either change the rule that selected RomConst (rule
            misfire) or fix the upstream tensor classification
            (QuantGraph entity is not a const).
  Provenance: ValueId v, producing NodeId, op tag.

STORE-005  StorageHramAdmissionInvariantViolation
  Severity: Hard
  Detail:   The precomputed HRAM admitted set exceeds
            AllocatableHramBudget(policy), which should be impossible
            under the deterministic admission algorithm of §9.3.
  Fix:      Internal F-B8 bug; fix HRAM candidate admission.
  Provenance: admitted set, cumulative pressure, budget.

STORE-006  StorageRecomputeForbiddenForObservedValue
  Severity: Hard
  Detail:   DR-11 (or DR-1) attempted to bind v as Recompute, but v is
            the materialization site for an ObservationPlan checkpoint.
  Fix:      Either remove the override (DR-1) or bound the rule with
            the ObservationPlan-checkpoint exclusion.
  Provenance: ValueId v, SemanticAnchor, SemanticCheckpointId.

STORE-007  StoragePersistSequenceStateUnsupportedV1
  Severity: Hard
  Detail:   DR-2 fired (a SequenceStateSlot ValueId was found) but
            v1 forbids non-identity sequence blocks (F-B5 §2.5a).
  Fix:      Upstream IR construction should have rejected the artifact;
            this fires defensively if F-B5 missed the check.
  Provenance: ValueId v, StateSlotId, layer.

STORE-008  StoragePersistBindingKindMismatch
  Severity: Hard
  Detail:   A Persist binding's (page, commit_group, kind) triple
            violates the per-kind binding rule of §10.2.
  Fix:      Adjust the binding to match the per-kind rule.
  Provenance: ValueId v, PersistPageId, CommitGroupId, kind, expected.

STORE-009  StoragePersistPageNotReferenced
  Severity: Hard
  Detail:   SP.persist_pages contains a PersistPageId not referenced
            by any binding.
  Fix:      Internal F-B8 bug; SC7 is violated.
  Provenance: PersistPageId.

STORE-010  StorageCommitGroupEmpty
  Severity: Hard
  Detail:   SP.commit_groups[g].members is empty.
  Fix:      Internal F-B8 bug; SC9 is violated.
  Provenance: CommitGroupId.

STORE-011  StorageCommitGroupKindMix
  Severity: Hard
  Detail:   SP.commit_groups[g].kind_set is not in the allowed
            cross-kind table of §10.3 CG-Wf-3.
  Fix:      Either split the group into per-kind groups (default) or
            amend §10.3 CG-Wf-3 to allow the new cross-kind mix
            (RFC amendment).
  Provenance: CommitGroupId, kind_set, allowed table.

STORE-012  StorageCommitGroupDurabilityMix
  Severity: Hard
  Detail:   SP.commit_groups[g] mixes durability classes in a way
            that violates §10.3 CG-Wf-4 (e.g. a Critical
            SequenceState page with a BestEffort Trace page).
  Fix:      Split the group by durability class.
  Provenance: CommitGroupId, member durabilities.

STORE-013  StorageAliasIntentMaterializationMismatch
  Severity: Hard
  Detail:   An AliasClass's intent is incompatible with the
            materialization of one or more members per §8.3.
  Fix:      Either change the intent (RFC amendment if a new intent
            is needed) or split the alias class.
  Provenance: AliasClassId, members, intent, materialization mix.

STORE-014  StorageAliasClassOverlapWithoutIntent
  Severity: Hard
  Detail:   Two members of the same alias class have overlapping
            abstract live ranges, but the class's intent does not
            permit overlap (intent = ScratchReuse but
            LiveRange(v_a) ∩ LiveRange(v_b) ≠ ∅).
  Fix:      Either prove disjointness (fix the live-range estimate) or
            promote the intent to PingPong/ResumeOverlap with explicit
            schedule coordination.
  Provenance: AliasClassId, members with overlap.

STORE-015  StorageAliasClassMembershipFunctionalViolation
  Severity: Hard
  Detail:   ValueId v has SP.bindings[v].alias_class = A but
            SP.alias_classes[A].members does not contain v.
  Fix:      Internal F-B8 bug; SC3/SC4 are violated.
  Provenance: ValueId, AliasClassId.

STORE-016  StorageRecomputeAliasNotIsolated
  Severity: Hard
  Detail:   A Recompute binding's alias class has more than one member.
  Fix:      Internal F-B8 bug; SC5 is violated.
  Provenance: ValueId, AliasClassId.

STORE-017  StorageLifetimeAdmissibilityViolation
  Severity: Hard
  Detail:   LifetimeOf(SP.bindings[v]) falls outside the interval
            [MinRequiredLifetime(v), MaxAdmissibleLifetime(v)].
  Fix:      Tighten or promote the lifetime so it satisfies both the
            observation/range/persistence lower bounds and the upstream
            admissibility upper bound.
  Provenance: ValueId, computed lifetime, allowed interval, source.

STORE-018  StorageForbiddenSpatialEnumLeak
  Severity: Hard
  Detail:   storage_plan.json contains a field that mentions a
            forbidden spatial enum per SC11.
  Fix:      Internal F-B8 bug; the schema must not include any
            byte-offset, bank, or slice id.
  Provenance: JSON path, forbidden field.

STORE-019  StorageDeterminismRequiresStableRules
  Severity: Hard
  Detail:   The artifact requires DeterminismClass::BitExact, but
            one of the decision rules fired with non-stable input
            (e.g. recompute_cost_estimate fluctuates run-to-run).
  Fix:      Pin the cost estimator's inputs (typically a side effect
            of pinning calibration evidence under EvidenceClass::
            Measured or Heuristic with declared seed).
  Provenance: rule id, instability evidence.

STORE-020  StorageRangePlanHashMismatch
  Severity: Hard
  Detail:   The RangePlan input's report_self_hash does not equal the
            range_plan_hash recorded in StoragePlanInputs. This is
            a concurrency or input-pinning bug.
  Fix:      Re-run the upstream stage; if the mismatch persists, the
            cache layer is inconsistent.
  Provenance: range_plan_hash recorded, range_plan.report_self_hash.

STORE-021  StorageInferIrHashMismatch
  Severity: Hard
  Detail:   The GbInferIR input's report_self_hash does not equal the
            infer_ir_hash recorded in StoragePlanInputs.
  Fix:      Same as STORE-020.
  Provenance: infer_ir_hash recorded, infer_ir.report_self_hash.

STORE-022  StorageObservationPlanHashMismatch
  Severity: Hard
  Detail:   The ObservationPlan input's report_self_hash does not
            equal the observation_plan_hash recorded in
            StoragePlanInputs.
  Fix:      Same as STORE-020.
  Provenance: observation_plan_hash recorded,
              observation_plan.report_self_hash.

STORE-023  StorageQuantGraphHashMismatch
  Severity: Hard
  Detail:   The QuantGraph input's report_self_hash does not equal
            the quant_graph_hash recorded in StoragePlanInputs.
  Fix:      Same as STORE-020.
  Provenance: quant_graph_hash recorded, quant_graph.report_self_hash.

STORE-024  StoragePolicyHashMismatch
  Severity: Hard
  Detail:   ResolvedCompilePolicy.canonical_hash does not equal the
            policy_hash recorded in StoragePlanInputs.
  Fix:      Same as STORE-020.
  Provenance: policy_hash recorded, policy.canonical_hash.

STORE-025  StorageIterationInputInvalid
  Severity: Hard
  Detail:   Stage 6 received an explicit storage iteration index
            greater than policy.compile_knobs.global.schedule.
            stage_iters.storage.
  Fix:      F-B16 should stop before invoking Stage 6 past the
            ceiling. The iteration ceiling is owned by F-B16's loop
            driver; F-B8 fires this diagnostic only when an invalid
            iteration index reaches it.
  Provenance: iteration count, ceiling.

STORE-026  StorageOverlayLensViolation
  Severity: Hard
  Detail:   The overlay-eligibility lens (§11.5) returned true for a
            binding whose materialization is not RomConst, or returned
            false for a binding the override forced into the lens.
  Fix:      Internal F-B8 bug; the lens predicate is local to
            §11.5.
  Provenance: ValueId, materialization, override.

STORE-027  StorageRepairProposalIllegal
  Severity: Hard
  Detail:   A RepairProposal in repair_proposals violates the
            constraint shape (e.g. ConstraintDelta references a
            CompileKnobId that is locked, or PromoteRecomputeLevel
            advances past CompileKnobBounds).
  Fix:      Internal F-B8 bug; F-B8 must check
            CompileKnobBounds.max_recompute_promotion before emitting
            a proposal.
  Provenance: proposal_id, ConstraintDelta, locks/bounds.

STORE-028  StorageInferIrEffectClassUnknown
  Severity: Hard
  Detail:   GbInferIR declares an EffectClass not in the closed v1
            set { SequenceState, Rng, FaultBoundary }. F-B8 cannot
            infer materialization for an unknown effect class.
  Fix:      F-B5 should reject this earlier; if it does not, this
            fires defensively.
  Provenance: EffectId, EffectClass tag.

STORE-029  StorageQuantGraphRoutingMismatch
  Severity: Hard
  Detail:   GbInferIR declares a routed FFN (RouteTop1 + ExpertMatVec
            chain) but QuantGraph.routing_table has no entry for the
            corresponding layer.
  Fix:      F-B3/F-B5 should reject this earlier.
  Provenance: layer id, expected routing entry.

STORE-030  StorageReservedShapeEmitted
  Severity: Hard
  Detail:   A schema-reserved shape was emitted by the v1 producer.
  Fix:      Remove the reserved variant from the emitted report, or
            bump the schema/RFC when the shape becomes legal.
  Provenance: JSON path, reserved tag.

STORE-031  StorageAliasMixedIntentComponent
  Severity: Hard
  Detail:   Alias candidate edges would merge values admitted by
            multiple AliasIntent variants into one equivalence class.
  Fix:      Split the component or choose a single explicit intent
            with a mechanically checkable coordination rule.
  Provenance: component members, candidate edges, intents.

STORE-032  StorageAliasIntentCardinalityViolation
  Severity: Hard
  Detail:   An alias class has a member count incompatible with its
            intent, such as a PingPong class with more than two active
            buffers or a PersistRotation class that is not a rotation
            pair.
  Fix:      Split the class or amend the intent definition.
  Provenance: AliasClassId, intent, members.

STORE-033  StorageForcedRecomputeNotAllowed
  Severity: Hard
  Detail:   A forced-recompute override selected a value that is not
            recomputable under `RecomputeAllowed`.
  Fix:      Remove the override or change the upstream value so it is
            pure, non-observed, non-persistent, non-routing-stability-
            critical, and within the admitted lifetime class.
  Provenance: ValueId, failed RecomputeAllowed predicates.

STORE-034  StoragePolicyBudgetUnderflow
  Severity: Hard
  Detail:   Computing a per-class budget (WramHot or HramHot) by
            subtracting the runtime-chrome reservation from the
            storage soft budget produced a negative value under
            checked subtraction.
  Fix:      Tighten the runtime-chrome reservation or raise the
            storage soft budget so the subtraction is non-negative.
  Provenance: storage class, soft_bytes, reserved_bytes.

STORE-035  StorageAliasClassFingerprintCollision
  Severity: Hard
  Detail:   Two distinct alias-class payloads produced the same
            AliasClassFingerprint.
  Fix:      Internal hash-domain failure; change the fingerprint
            domain or schema version by RFC amendment.
  Provenance: both canonical alias-class payloads.
```

### 14.3 Diagnostic shape

Inherited from F-B2/F-B4 §5:

```rust
pub struct ValidationDiagnostic {
    pub severity: Severity,             // == Hard for all F-B8 codes
    pub origin:   ValidationOrigin,     // == StoragePlanConstruction
    pub code:     DiagnosticCode,       // STORE-*
    pub detail:   String,               // human-readable rendering
    pub provenance: DiagnosticProvenance,
}
```

`DiagnosticProvenance` carries typed references (`ValueId`, `NodeId`,
`AliasClassId`, `PersistPageId`, `CommitGroupId`, `RuleId`, etc.) per
`D-Provenance`. No diagnostic is string-only (`D-NoStringOnly`).

### 14.4 Diagnostic ordering

Diagnostics are sorted by `(code, provenance.canonical_form)` for
determinism. Two consecutive failed runs on the same inputs produce
the same ordered diagnostic list.

### 14.5 Soft diagnostic prohibition

Per `R-HardOnly-ThisChunk`, F-B8 emits only Hard diagnostics. Soft
diagnostics are forbidden in this chunk. If a future RFC introduces a
Soft diagnostic for F-B8 (e.g. a budget warning), it must amend
`R-HardOnly-ThisChunk` for `storage_plan.v1`.

## 15. Cross-stage interactions

### 15.1 F-B7 (`RangePlan`) — input

F-B8 consumes `RangePlan` via:

* `range_plan.report_self_hash` for the cache key (K6) and the identity.
* `range_plan.reductions: BTreeMap<ReductionSiteRef, ReductionPlan>`
  for the reduction-plan-aware predicates (`IsRenormLoopScratch`,
  `IsSingleI16Accum`, `IsChunkedI16Accum`).
* `range_plan.scratch_value_ids: BTreeMap<ReductionSiteRef,
  SortedSet<ValueId>>` for the scratch-value lookup.

If `RangePlan`'s exact shape is not yet pinned by F-B7's RFC, F-B8
consumes a `RangePlanView` trait that exposes the two methods above.
The placeholder strategy mirrors F-B4's handling of
`QuantGraphBudgetSource` from before F-B3 landed.

```text
F-B8 → F-B7 dependency:
  Hash binding:    range_plan_hash flows into K6 and StoragePlanInputIdentity.
  Schema binding:  RangePlanView trait or range_plan.v1 schema.
  Failure mode:    STORE-020 (RangePlan hash mismatch).
```

### 15.2 F-B5 (`GbInferIR`) — input

F-B8 consumes `GbInferIR` via:

* `infer_ir.report_self_hash` for the cache key and identity.
* `infer_ir.values: BTreeMap<ValueId, ValueDecl>` for the binding
  iteration.
* `infer_ir.nodes: BTreeMap<NodeId, GbNode>` for the role/format
  inference.
* `infer_ir.effects: BTreeMap<EffectId, EffectDecl>` for the
  EffectClass classification.
* `infer_ir.anchors: NodeAnchorMap` for the anchor-aware predicates
  (e.g. `IsObserved`).
* `infer_ir.provenance: InferIrProvenance` for the
  `ExportTensorId → TensorId → NodeId → ValueId` chain that
  identifies expert weights, router weights, embedding tables, etc.

```text
F-B8 → F-B5 dependency:
  Hash binding:    infer_ir_hash flows into K6 and StoragePlanInputIdentity.
  Schema binding:  infer_ir.v1.
  Failure mode:    STORE-021, STORE-028, STORE-029.
```

### 15.3 F-B3 (`QuantGraph`) — input

F-B8 consumes `QuantGraph` via:

* `quant_graph.report_self_hash` for the cache key and identity.
* `quant_graph.expert_sections: SortedVec<ExpertSection>` for
  `IsExpertWeight` predicate evidence.
* `quant_graph.routing_table: Option<RoutingTable>` for `IsRouterTable`
  predicate evidence.
* `quant_graph.norm_plans: BTreeMap<NormPlanId, NormPlan>` for
  `NormParam` role classification.
* `quant_graph.decode_spec: DecodeSpec` for `DecodeConst` role
  classification.
* `quant_graph.sequence_semantics: SequenceSemanticsSpec` for the v1
  sequence-state-rejection check (DR-2's reserved-shape gate).

```text
F-B8 → F-B3 dependency:
  Hash binding:    quant_graph_hash flows into K6 and StoragePlanInputIdentity.
  Schema binding:  quant_graph.v1.
  Failure mode:    STORE-023.
```

### 15.4 F-B6 (`ObservationPlan`) — input

F-B8 consumes `ObservationPlan` via:

* `observation_plan.report_self_hash` for the cache key and identity.
* `observation_plan.semantic: SortedVec<SemanticObservation>` for the
  observed-value check that constrains DR-11 (recompute is forbidden
  for observed values via the RecomputeAllowed predicate).
* `observation_plan.probes: SortedVec<OperationalProbe>` for the
  trace-page-eligibility check that constrains DR-5.
* `observation_plan.metrics: SortedVec<MetricProbe>` for metric-probe-
  attached lifetime constraints.

```text
F-B8 → F-B6 dependency:
  Hash binding:    observation_plan_hash flows into K6 and identity.
  Schema binding:  observation_plan.v1.
  Failure mode:    STORE-022.
```

### 15.5 F-B9 (`SramPagePlan`) — output consumer

F-B9 consumes:

* All bindings with `materialization = Materialize { class: SramPaged,
  .. }`.
* All bindings with `materialization = Persist { page, commit_group }`.
* All `PersistPageDecl` entries with non-`Continuation` kind.
* All `CommitGroupDecl` entries (for commit-boundary planning).
* `AliasClass`es with intent `PersistRotation`.

F-B9 plans `active_sets: Vec<SramWorkingSet>`,
`page_bindings: Vec<SramPageBinding>`,
`commit_boundaries: Vec<CommitBoundary>`, and `spill_policy:
SpillPolicy` (per `planv0.md` lines 1717–1722). F-B9 never re-decides
that a binding is paged or persistent.

```text
F-B8 → F-B9 contract:
  F-B9 reads StoragePlan.bindings filtered to SramPaged + Persist.
  F-B9 reads StoragePlan.persist_pages, StoragePlan.commit_groups.
  F-B9 reads StoragePlan.alias_classes filtered to PersistRotation.
  F-B9 emits sram_page_plan.json with bindings on F-B8's typed ids.
  F-B9 may emit RepairProposal of class
       RepairReason::AdvanceSramPageAggression
       (per planv0.md line 1252) but never modifies StoragePlan.
```

### 15.6 F-B10 (`RomWindowPlan`) — output consumer

F-B10 consumes:

* All bindings with `materialization = Materialize { class: RomConst,
  .. }`.
* The lens for "which RomConst objects are kernels" vs "which are LUTs"
  vs "which are expert weights" (derivable from `ValueRole` in the
  bindings' provenance).

F-B10 decides `KernelResidency` per kernel:

```text
KernelResidency :=
  Bank0Fixed | WramOverlay | CoResidentSwitchable
```

(per `planv0.md` line 1731). F-B10 never re-decides that a kernel is
ROM-resident.

```text
F-B8 → F-B10 contract:
  F-B10 reads StoragePlan.bindings filtered to RomConst.
  F-B10 reads StorageProvenance.bindings for ValueRole evidence.
  F-B10 emits rom_window_plan.json keyed by F-B8's ValueIds.
  F-B10 may emit RepairProposal of class
        RepairReason::AdvanceKernelResidencyBias
        (per planv0.md line 1262) but never modifies StoragePlan.
```

### 15.7 F-B11 (`OverlayPlan`) — output consumer

F-B11 consumes the overlay-eligibility lens (§11.5):

* All bindings where `IsOverlayEligible(b)` returns true.
* `AliasClass`es with intent compatible with overlay sharing.

F-B11 decides `regions: Vec<OverlayRegion>`,
`installs: Vec<OverlayInstall>`, `share_classes: Vec<OverlayShareClass>`
(per `planv0.md` lines 1748–1752).

```text
F-B8 → F-B11 contract:
  F-B11 reads StoragePlan.bindings filtered to overlay-eligible.
  F-B11 reads StoragePlan.alias_classes for shared-region candidates.
  F-B11 emits overlay_plan.json with bindings on F-B8's typed ids.
  F-B11 may emit RepairProposal of class
        RepairReason::PromoteOverlay
        (per planv0.md line 1281) but never modifies StoragePlan.
```

### 15.8 F-B12 (`ArenaPlan`) — output consumer

F-B12 consumes:

* All bindings with `materialization = Materialize { class, lifetime }`.
* `AliasClass`es with intent `ScratchReuse`, `PingPong`,
  `ResumeOverlap` (for byte-range coalescing decisions).

F-B12 assigns concrete byte ranges. It never reads `Recompute`
bindings (no byte range is needed). It reads `Persist` bindings only
indirectly via F-B9 (F-B12 does not assign byte ranges to persistent
pages; F-B9 does).

```text
F-B8 → F-B12 contract:
  F-B12 reads StoragePlan.bindings filtered to Materialize.
  F-B12 reads StoragePlan.alias_classes (all intents except
        PersistRotation).
  F-B12 emits arena_plan.json with byte ranges keyed by F-B8's
        ValueIds.
  F-B12 never assigns a byte range to a Recompute binding.
  F-B12 may emit RepairProposal of class
        RepairReason::TightenArenaPressure (not in planv0; local
        to F-B12's RFC) but never modifies StoragePlan.
```

### 15.9 F-B13 (`GbSchedIR` + `ResourceStateValidation`) — output consumer

F-B13 consumes `AliasClassId` directly. The alias-class equivalence
relation is the resource-aliasing lens that
`ResourceStateValidation` reasons against.

```text
F-B8 → F-B13 contract:
  F-B13 reads StoragePlan.bindings.alias_class for every value
        consumed in a SchedSlice.
  F-B13 reads StoragePlan.alias_classes for the equivalence classes.
  F-B13 reads StoragePlan.alias_classes[A].intent for the typed reasons.
  F-B13 emits resource_state.cert.json witnessing F-Alias-NoConflict-*
        properties from §11.6.
  F-B13 may emit schedule-local bindings that reference
        StoragePlan.bindings and StoragePlan.alias_classes
        (e.g. resource leases, interrupt policies). It never mutates,
        extends, or rewrites the StoragePlan product.
```

### 15.10 F-B16 (`FeasibilityRefinementLoop`) — both directions

F-B16 is bidirectional:

* **Forward** (F-B8 → F-B16): F-B8 emits `RepairProposal`s with
  `RepairReason::PromoteRecompute` and (less commonly)
  `KnobDelta::ForceRecompute` for individual values that fall just
  outside the soft pressure threshold.
* **Backward** (F-B16 → F-B8): When F-B16 advances
  `RecomputePromotionLevel` from `None` to `PureSliceValues` (or
  further), F-B8 is re-run with the advanced policy. The re-run is
  bounded by `StageIterationCeilings::storage`.

```text
F-B8 → F-B16 contract:
  F-B8 emits proposals only; never accepts.
  F-B16 (when it lands) accepts proposals via
        RepairPolicy::allow_recompute_promotion and re-runs F-B8.
  F-B16's re-run is bounded by StageIterationCeilings::storage.
  STORE-025 fires on ceiling exceeded.
  STORE-027 fires on illegal proposal (locks/bounds violated).
```

### 15.11 F-B17 (`StageCache` integration sweep) — cross-cut

F-B17 is the cross-cutting integration sweep. F-B8 wires K6 under the
F-B2/F-B4 / F-B3/F-B5 discipline so F-B17's uniformity check passes
without per-stage bespoke logic.

```text
F-B8 → F-B17 contract:
  F-B17 verifies: K6 uses DomainHash with the same shape as K0..K5.
  F-B17 verifies: storage_plan.v1 envelope uses ReportEnvelope<R>.
  F-B17 verifies: failure memos and success entries follow the
                  StageCacheSuccessEntry / StageCacheFailureMemo
                  shapes.
```

### 15.12 F-C2 / F-C3 (`ArtifactOracle` / `ScheduleOracle`) — indirect

F-C2 (`ArtifactOracle`) is unaffected by F-B8: oracle correspondence
at the artifact-stratum is op-for-op against `QuantGraph` and
`GbInferIR` (per F-B3/F-B5), and `StoragePlan` is below that
correspondence boundary.

F-C3 (`ScheduleOracle`) consumes `GbSchedIR`, which carries forward
F-B8's `Materialization` and `AliasClassId` via op-level resource-
state evidence. F-B8 does not interact with F-C3 directly; the
schedule-oracle correspondence is owned by F-B13's `GbSchedIR` shape
and F-C3's evaluator.

```text
F-B8 → F-C2 contract:    none (ArtifactOracle is artifact-stratum).
F-B8 → F-C3 contract:    indirect via GbSchedIR (F-B13).
```

## 16. Task DAG, compressed

This section enumerates the implementation tasks that bd-2k0 fans out
to. Each task is a candidate task bead (T-B8.*) under the F-B8 feature.
The DAG is shaped so that two consecutive tasks at the same level can
run in parallel; the level dependencies are explicit.

```text
Level 0 — Type surface

  T-B8.1  StorageClass / LifetimeClass / Materialization enums;
          PersistPageId, CommitGroupId, AliasClassId newtypes;
          DecisionRuleId newtype; PersistKind, DurabilityClass
          re-exports from gbf-abi.
          Dependencies: F-B2/F-B4 envelope; gbf-abi PersistKind
                        (already in tree).
          Touches: gbf-codegen::storage_plan::types.

  T-B8.2  StorageBinding, AliasClass, AliasIntent; PersistPageDecl,
          CommitGroupDecl, PersistSchemaPin; CommitAtomicityClass.
          Dependencies: T-B8.1.

  T-B8.3  StoragePlan typed product (BTreeMap<ValueId, StorageBinding>,
          BTreeMap<AliasClassId, AliasClass>, BTreeMap<PersistPageId,
          PersistPageDecl>, BTreeMap<CommitGroupId, CommitGroupDecl>,
          repair_proposals: Vec<_>, provenance: StorageProvenance,
          input_identity: StoragePlanInputIdentity). Constructor stub.
          Dependencies: T-B8.1, T-B8.2.

Level 1 — Predicate environment

  T-B8.4  PredicateEnv: typed view over (QuantGraph, GbInferIR,
          ObservationPlan, RangePlan, ResolvedCompilePolicy) with
          methods role_of(v), format_of(v), is_pure(v), is_observed(v),
          reduction_plan_of(n), scratch_value_ids_of(n),
          longest_live_window(v), logical_byte_size(v),
          recompute_cost_estimate(v).
          Dependencies: T-B8.1; F-B3 QuantGraph; F-B5 GbInferIR;
                        F-B6 ObservationPlan (or trait shim);
                        F-B7 RangePlan (or RangePlanView shim).

  T-B8.5  ValueRole / ValueFormat enums and inference rules from
          GbInferIR.value_decls and InferOp tags.
          Dependencies: T-B8.4.

  T-B8.6  Pair predicates IsPingPongPair, ResumeBoundary,
          IsRotationPair (typed predicates over GbInferIR for
          alias-class pair-predicate seeding).
          Dependencies: T-B8.4.

Level 2 — Decision rules

  T-B8.7  Decision rule list (DR-1..DR-13) with typed predicate
          functions and binding constructors. Rule firing engine
          (priority-ordered, first-match-wins).
          Dependencies: T-B8.3, T-B8.4, T-B8.5.

  T-B8.8  Recompute promotion handshake: emit RepairProposal of class
          PromoteRecompute or ForceRecompute when soft pressure
          crosses threshold.
          Dependencies: T-B8.7; F-B2/F-B4 RepairProposal shape.

  T-B8.9  Persist binding resolver: per-PersistKind binding rules
          (§10.2), per-kind PersistSchemaPin construction.
          Dependencies: T-B8.7.

Level 3 — Whole-plan invariants

  T-B8.10 Alias-class union-find construction over pair predicates
          (§11.4); AliasClassId content-addressing with collision
          retry (§11.7).
          Dependencies: T-B8.6, T-B8.7.

  T-B8.11 Self-consistency check engine (SC1..SC12 from §8.8).
          Dependencies: T-B8.10.

  T-B8.12 Persist invariants check (CG-Wf-1..CG-Wf-6 from §10.3;
          F-Persist-PlugCompat from §10.1).
          Dependencies: T-B8.9, T-B8.10.

Level 4 — Diagnostics, report, cache

  T-B8.13 STORE-* diagnostic codes (full closed list from §14.2);
          DiagnosticProvenance attachment.
          Dependencies: T-B8.7, T-B8.11, T-B8.12.

  T-B8.14 storage_plan.json schema and emitter (envelope, body,
          summary, identity); canonical-JSON serializer.
          Dependencies: T-B8.3, T-B8.13.

  T-B8.15 Self-hash computation; report_self_hash patching.
          Dependencies: T-B8.14.

  T-B8.16 K6 cache key construction; StageCache success/failure
          memo wiring.
          Dependencies: T-B8.14, T-B8.15; F-A6 gbf-store.

Level 5 — Tests

  T-B8.17 Unit tests for decision rules (one fixture per rule,
          including DR-1..DR-13 negative-positive matrix).
          Dependencies: T-B8.7.

  T-B8.18 Unit tests for alias-class pair predicates and the
          equivalence relation invariants.
          Dependencies: T-B8.10.

  T-B8.19 Unit tests for persist binding resolver and commit-group
          well-formedness.
          Dependencies: T-B8.9, T-B8.12.

  T-B8.20 Property tests for storage-plan alias-class invariants
          (per planv0.md line 2711).
          Dependencies: T-B8.11, T-B8.18.

  T-B8.21 Round-trip tests for storage_plan.json (canonical-JSON
          determinism, self-hash).
          Dependencies: T-B8.14, T-B8.15.

  T-B8.22 Snapshot tests: GbInferIR -> StoragePlan snapshots (per
          planv0.md line 2697).
          Dependencies: T-B8.7, T-B8.14.

  T-B8.23 K6 cache hit/miss tests; failure-memo replay test.
          Dependencies: T-B8.16.

Level 6 — Integration

  T-B8.24 Routed-FFN dense fixture: end-to-end Stage 6 run on the M3
          fixture; verify every binding satisfies §9 rules without
          override.
          Dependencies: T-B8.7, T-B8.14, T-B8.21, T-B8.22.

  T-B8.25 Degenerate fixture: minimal Materialize-only StoragePlan
          (no Recompute, no Persist) for the M1 dense baseline.
          Dependencies: T-B8.24.

  T-B8.26 F-B9 / F-B10 / F-B11 / F-B12 / F-B13 input-shape conformance
          tests: verify that F-B8's product is consumable by every
          downstream stage's expected input shape (where the
          downstream stage exists in tree; otherwise verify against
          a trait shim).
          Dependencies: T-B8.24.

  T-B8.27 F-B16 RepairProposal handshake test: verify that
          RecomputePromotion proposals are emitted with correct
          ConstraintDelta and EstimatedCostDelta shapes (no
          acceptance path; F-B16 is BLOCKED).
          Dependencies: T-B8.8.

Level 7 — Closure

  T-B8.28 Closure gate: rejection-class enumeration (§17) verified by
          STORE-* unit tests; proof obligations (§18) checked.
          Dependencies: T-B8.13, T-B8.17, T-B8.18, T-B8.19, T-B8.20,
                        T-B8.21, T-B8.22, T-B8.23, T-B8.24, T-B8.25,
                        T-B8.26, T-B8.27.
```

Tasks at the same level can run in parallel. Cross-level dependencies
are honored by the bead `depends-on` graph. The total fan-out is 28
tasks; the critical path is roughly Level 0 → Level 1 → Level 2 →
Level 4 → Level 6 → Level 7 (≈ 6 sequential PRs, plus parallel test
work).

## 17. Rejection classes

The closure gate verifies that F-B8 rejects every malformed-input
class enumerated below. Each class corresponds to one or more
diagnostic codes from §14.2.

### 17.1 Upstream-product hash mismatches

```text
RC-1  QuantGraph hash mismatch
      Diagnostic: STORE-023.
      Trigger:    inputs.quant_graph.report_self_hash ≠
                  inputs.quant_graph_hash.

RC-2  GbInferIR hash mismatch
      Diagnostic: STORE-021.
      Trigger:    inputs.infer_ir.report_self_hash ≠
                  inputs.infer_ir_hash.

RC-3  ObservationPlan hash mismatch
      Diagnostic: STORE-022.
      Trigger:    inputs.observation_plan.report_self_hash ≠
                  inputs.observation_plan_hash.

RC-4  RangePlan hash mismatch
      Diagnostic: STORE-020.
      Trigger:    inputs.range_plan.report_self_hash ≠
                  inputs.range_plan_hash.

RC-5  Policy hash mismatch
      Diagnostic: STORE-024.
      Trigger:    inputs.policy.canonical_hash ≠ inputs.policy_hash.
```

### 17.2 Decision-rule failures

```text
RC-6  No admitting rule
      Diagnostic: STORE-001.
      Trigger:    a ValueId's role/format does not match any
                  predicate in DR-1..DR-13.

RC-7  Recompute on observed value
      Diagnostic: STORE-006.
      Trigger:    A forced-recompute override (DR-1) attempted to
                  bind Recompute on an observed value (¬RecomputeAllowed),
                  triggering DR-1b. STORE-006 also fires defensively
                  if DR-11 admits Recompute on a value that backs an
                  ObservationPlan checkpoint.

RC-8  RomConst on non-const value
      Diagnostic: STORE-004.
      Trigger:    DR-6/DR-7/DR-8/DR-9 admitted RomConst, but the
                  producer is not a const tensor reference.

RC-9  HRAM admission invariant violation
      Diagnostic: STORE-005.
      Trigger:    The precomputed HRAM admitted set exceeds
                  AllocatableHramBudget (internal F-B8 bug).

RC-10 Sequence-state binding in v1
      Diagnostic: STORE-007.
      Trigger:    DR-2 fired (a SequenceStateSlot value found),
                  which is reserved shape in v1.
```

### 17.3 Persist-binding violations

```text
RC-11 Persist kind mismatch
      Diagnostic: STORE-008.
      Trigger:    a Persist binding's (page, kind) violates §10.2.

RC-12 Orphan persist page
      Diagnostic: STORE-009.
      Trigger:    SP.persist_pages contains a PersistPageId not
                  referenced by any binding.

RC-13 Empty commit group
      Diagnostic: STORE-010.
      Trigger:    SP.commit_groups[g].members is empty.

RC-14 Forbidden cross-kind commit group
      Diagnostic: STORE-011.
      Trigger:    SP.commit_groups[g].kind_set ∉ §10.3 CG-Wf-3
                  allowed table.

RC-15 Durability class mix
      Diagnostic: STORE-012.
      Trigger:    Critical and BestEffort pages share a commit group.
```

### 17.4 Alias-class violations

```text
RC-16 Alias intent / materialization mismatch
      Diagnostic: STORE-013.
      Trigger:    An AliasClass's intent is incompatible with the
                  members' materializations (§8.3).

RC-17 Alias overlap without intent
      Diagnostic: STORE-014.
      Trigger:    Two members of a ScratchReuse class have
                  overlapping lifetimes.

RC-18 Alias membership functional violation
      Diagnostic: STORE-015.
      Trigger:    SP.bindings[v].alias_class = A but
                  SP.alias_classes[A].members ∌ v.

RC-19 Recompute alias not isolated
      Diagnostic: STORE-016.
      Trigger:    A Recompute binding shares an alias class with
                  another value.
```

### 17.5 Coverage violations

```text
RC-20 Binding coverage gap
      Diagnostic: STORE-002.
      Trigger:    GbInferIR declares ValueId v but SP.bindings has
                  no entry for v.

RC-21 Binding double-bind
      Diagnostic: STORE-003.
      Trigger:    SP.bindings has two distinct entries for the
                  same ValueId.
```

### 17.6 Lifetime monotonicity

```text
RC-22 Lifetime monotone violation
      Diagnostic: STORE-017.
      Trigger:    LifetimeOf(SP.bindings[v]) > LifetimeOf(QuantGraph
                  entity backing v).
```

### 17.7 Schema invariants

```text
RC-23 Forbidden spatial enum leak
      Diagnostic: STORE-018.
      Trigger:    storage_plan.json contains a forbidden spatial
                  enum per SC11.

RC-24 Determinism instability
      Diagnostic: STORE-019.
      Trigger:    DeterminismClass::BitExact required, but a rule
                  fires with non-stable inputs.
```

### 17.8 Knob bounds and refinement

```text
RC-25 Refinement ceiling exceeded
      Diagnostic: STORE-025.
      Trigger:    F-B16 has re-run Stage 6 more than
                  StageIterationCeilings::storage times without
                  converging.

RC-26 Illegal repair proposal
      Diagnostic: STORE-027.
      Trigger:    A RepairProposal violates locks/bounds.
```

### 17.9 Reserved-shape and unsupported

```text
RC-27 Unknown EffectClass
      Diagnostic: STORE-028.
      Trigger:    GbInferIR has an EffectClass not in the closed v1
                  set.

RC-28 Routing mismatch
      Diagnostic: STORE-029.
      Trigger:    GbInferIR has a routed FFN but QuantGraph has no
                  routing entry.

RC-29 Reserved shape emitted
      Diagnostic: STORE-030.
      Trigger:    A v1 producer emitted a schema-reserved variant
                  (e.g. OrderedRecoverable atomicity, HintBundle
                  justification, ContinuationWithSequenceState
                  cross-kind group).

RC-30 Overlay lens violation
      Diagnostic: STORE-026.
      Trigger:    Overlay-eligibility lens contradicts binding's
                  materialization.

RC-31 Mixed-intent alias component
      Diagnostic: STORE-031.
      Trigger:    Union-find merges candidate edges admitted by more
                  than one AliasIntent into a single equivalence class.

RC-32 Alias intent cardinality violation
      Diagnostic: STORE-032.
      Trigger:    A class member count is incompatible with its intent.
```

The rejection classes above are the complete enumeration of v1
malformed-input handling. New rejection classes require explicit RFC
amendment.

## 18. Proof obligations

This section enumerates the typed proof obligations the implementation
must discharge. Each obligation is checkable by code or by a test, not
by review-time prose.

### 18.1 Pure-function determinism

```text
PO-1  Pure-function determinism:
      ∀ inputs i.
        build_storage_plan_core(i) is byte-identical to
        build_storage_plan_core(i') whenever
        canonical_input_hash(i) = canonical_input_hash(i').

  Witnessed by: T-B8.21 round-trip tests; T-B8.23 cache replay tests.
```

### 18.2 Binding coverage and functionality

```text
PO-2  Binding coverage:
      ∀ ValueId v ∈ inputs.infer_ir.values.
        ∃! StorageBinding b ∈ output.bindings. b.value = v.

  Witnessed by: SC1, SC2; T-B8.11 unit tests; STORE-002, STORE-003
                rejection tests.
```

### 18.3 Alias-class equivalence relation

```text
PO-3  Alias equivalence:
      The relation { (v_a, v_b) :
                     output.bindings[v_a].alias_class =
                     output.bindings[v_b].alias_class }
      is reflexive, symmetric, transitive on output.bindings.

  Witnessed by: T-B8.18 unit tests; T-B8.20 property tests.

PO-4  Alias intent / materialization compatibility:
      ∀ AliasClass A.
        A.intent and the materializations of A.members are compatible
        per §8.3 F-Alias-IntentMatchesMaterialization.

  Witnessed by: T-B8.18; STORE-013 rejection test.

PO-5  Alias no-conflict:
      ∀ AliasClass A.
        ∀ (v_a, v_b) ∈ A.members × A.members with v_a ≠ v_b.
          LiveRange(v_a) ∩ LiveRange(v_b) ≠ ∅
          ⇒ A.intent ∈ {PingPong, ResumeOverlap, PersistRotation}.

  Witnessed by: T-B8.20 property tests; STORE-014 rejection test.
```

### 18.4 Recompute decisions

```text
PO-6  Recompute first-class:
      ∀ ValueId v with output.bindings[v].materialization = Recompute.
        - The binding's alias class is a singleton (PO-7) with
          intent NoAlias.
        - The decision is admitted by DR-1 (forced, RecomputeAllowed)
          or DR-11 (pure slice value under recompute_promotion).
        - The justification/provenance names DR-1 or DR-11 and records
          whether the active policy was produced by a prior refinement
          via BindingProvenance.policy_refinement_applied.

  Witnessed by: T-B8.17 unit tests; SC5; STORE-016 rejection test.

PO-7  Recompute alias isolation:
      ∀ Recompute binding b.
        |output.alias_classes[b.alias_class].members| = 1.

  Witnessed by: SC5; T-B8.11 unit tests; STORE-016.

PO-8  Recompute observation forbidden:
      ∀ Recompute binding b.
        ¬ ∃ SemanticObservation s ∈ inputs.observation_plan.semantic.
          s.anchor = anchor_of(b.value).

  Witnessed by: T-B8.17 unit tests for DR-11's exclusion; STORE-006
                rejection test.
```

### 18.5 Persistence bindings

```text
PO-9  Persist plug-compatible:
      ∀ Persist binding b.
        ∃ PersistPageDecl pd. pd.id = b.materialization.page.
        ∃ CommitGroupDecl cg. cg.id = b.materialization.commit_group.

  Witnessed by: SC7, SC8, SC9; T-B8.19 unit tests; STORE-009, STORE-010.

PO-10 Persist commit-group well-formed:
      ∀ CommitGroupDecl cg.
        |cg.members| ≥ 1.
        cg.kind_set ⊆ allowed_cross_kind_table_v1.
        cg.atomicity = AllOrNothing  (v1).
        ∀ p ∈ cg.members. SP.persist_pages[p].kind ∈ cg.kind_set.

  Witnessed by: §10.3; T-B8.19; STORE-011, STORE-012, STORE-008.

PO-11 Persist alias-rotation only:
      ∀ Persist binding b.
        SP.alias_classes[b.alias_class] is either singleton or has
        intent = PersistRotation.

  Witnessed by: SC6; T-B8.18; STORE-013.

PO-12 Persist sequence-state v1 reject:
      ∀ ValueId v.
        ValueRole(v) = SequenceStateSlot ⇒ STORE-007 fires.

  Witnessed by: T-B8.19; STORE-007.
```

### 18.6 Lifetime monotonicity

```text
PO-13 Lifetime bounds:
      ∀ ValueId v.
        MinRequiredLifetime(v) ≤ LifetimeOf(SP.bindings[v])
        ≤ MaxAdmissibleLifetime(v).

  Witnessed by: SC10; STORE-017 rejection test.

PO-14 Refinement monotone:
      ∀ refinement step that re-runs Stage 6.
        recompute_promotion_after ≥ recompute_promotion_before
        AND every binding's lifetime is non-decreasing.

  Witnessed by: T-B8.27; F-B16 acceptance (post-F-B16).
```

### 18.7 Schema invariants

```text
PO-15 No spatial enum leak:
      storage_plan.json contains no field that mentions any forbidden
      spatial enum from SC11.

  Witnessed by: SC11; T-B8.14 schema tests; STORE-018.

PO-16 Self-hash invariant:
      report.report_self_hash = SelfHash(envelope_with_zero_hash).

  Witnessed by: PO-1; T-B8.15, T-B8.21.

PO-17 Schema unknown reject:
      ∀ unknown field f in storage_plan.json.
        parse rejects f at validation time.

  Witnessed by: T-B8.14; R-UnknownReject.
```

### 18.8 RangePlan-aware decisions

```text
PO-18 RenormLoop scratch hot:
      ∀ ValueId v with IsRenormLoopScratch(v) = true.
        SP.bindings[v].materialization =
          Materialize { class ∈ {WramHot, HramHot}, lifetime: Slice }.

  Witnessed by: §9.2 DR-10; T-B8.17 unit tests.

PO-19 RangePlan hash binding:
      output.input_identity.range_plan_hash =
        inputs.range_plan.report_self_hash.

  Witnessed by: SC12; STORE-020 rejection test.
```

### 18.9 Knob honoring

```text
PO-20 Knob bounds honored:
      ∀ refinement step.
        recompute_promotion_after ≤
          policy.compile_knobs.bounds.max_recompute_promotion.

  Witnessed by: F-Honor-Bounds; T-B8.27; STORE-027.

PO-21 Knob locks honored:
      CompileKnobId::StorageRecomputePromotion ∈ KnobLockSet.locked
      ⇒ no refinement step advances recompute_promotion.

  Witnessed by: F-Honor-Locks; T-B8.27.

PO-22 Forced recompute honored:
      ∀ Value(v) ∈ policy.compile_knobs.overrides.forced_recompute
                  with RecomputeAllowed(v).
        SP.bindings[v].materialization = Recompute.
        SP.bindings[v].justification = ForcedRecompute.

      ∀ Value(v) ∈ policy.compile_knobs.overrides.forced_recompute
                  with ¬RecomputeAllowed(v).
        STORE-006 or STORE-033 fires (DR-1b); the binding is not
        forced.

  Witnessed by: §9.2 DR-1, DR-1b; T-B8.17 unit tests.
```

### 18.10 RepairProposal shape

```text
PO-23 Repair proposal admissibility:
      ∀ RepairProposal p ∈ output.repair_proposals.
        p.source = PlanningStage::StoragePlan.
        p.reason ∈ {PromoteRecompute}.
        p.tighten.changes ⊆ {PromoteRecomputeLevel, ForceRecompute}.
        p.estimated_cost.cycles is Some(_) under EvidenceClass::
          {Measured | Transferred | Heuristic}.

  Witnessed by: §9.5; T-B8.27; STORE-027.
```

The 23 proof obligations above are mechanically checkable. Each is
tied to a witness (a test or invariant). The closure gate (§19) is
the conjunction of all 23 obligations.

## 19. End-to-end theorem

### 19.1 Statement

```text
End-to-End Theorem (F-B8 closure):

  ∀ valid F-B8 inputs i = (qg, iir, op, rp, policy) where:
    qg.report_self_hash = i.quant_graph_hash,
    iir.report_self_hash = i.infer_ir_hash,
    op.report_self_hash = i.observation_plan_hash,
    rp.report_self_hash = i.range_plan_hash,
    policy.canonical_hash = i.policy_hash,
    determinism class is consistent across qg/iir/op/rp/policy.

  Then build_storage_plan_core(i) either:

    (a) returns Ok((sp, env)) where:
        - sp is a StoragePlan satisfying SC1..SC12,
        - env is a ReportEnvelope<StoragePlanReportBody> with
          outcome = Passed,
        - sp satisfies all proof obligations PO-1..PO-23,
        - sp's bindings cover every ValueId in iir,
        - sp's alias-class equivalence relation is the smallest
          equivalence relation closed under §11.4's pair predicates,
        - sp's persist_pages and commit_groups are plug-compatible
          with gbf-runtime::persistence's protocol,
        - env.report_self_hash =
            SelfHash(canonical_json(env with report_self_hash = ZERO_HASH)),
        - env is byte-identical across two consecutive runs on the
          same i;

    (b) returns Err(diagnostics) where:
        - diagnostics is a non-empty sorted list of Hard
          ValidationDiagnostics,
        - every diagnostic carries origin = StoragePlanConstruction,
        - every diagnostic carries a code in the closed STORE-* set,
        - the set of diagnostics is the canonical evidence of one or
          more rejection classes from §17,
        - the corresponding storage_plan.json carries
          outcome = Failed and result = null per
          R-NoPartialIR-StoragePlan.
```

### 19.2 Proof sketch

The theorem is a conjunction of:

* **Construction completeness** (§8.7 step 3): every `ValueId` is
  visited; every visit emits exactly one `StorageBinding` or fires a
  `STORE-001` (no admitting rule). Mechanically witnessed by
  `T-B8.7`'s rule firing engine and `T-B8.11`'s coverage check.

* **Decision-rule soundness** (§9.2): each rule's predicate and
  binding are typed. Soundness — "the binding is consistent with the
  predicate" — is local to each rule and witnessed by `T-B8.17`'s
  per-rule unit tests.

* **Alias-class equivalence relation closure** (§11.4): the
  equivalence relation is constructed by union-find over four pair
  predicates. Reflexivity, symmetry, and transitivity follow from
  union-find. Closure under the pair predicates is the construction
  invariant: every alias edge is justified by one pair predicate.
  Witnessed by `T-B8.10`, `T-B8.18`, `T-B8.20`.

* **Persist plug-compatibility** (§10.1): every `Persist` binding
  resolves to a `PersistPageDecl` and `CommitGroupDecl` whose typed
  `kind` and `schema_pin` match `gbf-abi`'s `PersistKind` table and
  `PersistGroupCommit` shape. Witnessed by `T-B8.19` and the
  cross-stage tests of `T-B8.26`.

* **Self-consistency invariants** (§8.8): SC1..SC12 are mechanically
  checkable on the product. Witnessed by `T-B8.11`'s self-consistency
  engine and `T-B8.20`'s property tests.

* **Determinism** (§2.13, §13.4): the construction order, the
  BTreeMap-backed product, and the canonical-JSON serializer
  guarantee byte-identical output on identical inputs. Witnessed by
  `T-B8.21`'s round-trip and `T-B8.23`'s cache replay tests.

* **Diagnostic algebra closure** (§14.2): the STORE-* code list is
  closed in v1; every rejection class in §17 maps to one or more
  codes. Witnessed by `T-B8.13`'s diagnostic tests and the
  rejection-class enumeration of `T-B8.28`.

The conjunction of these properties is the closure gate: F-B8 closes
when every obligation has a witness in the test suite and every
rejection class has a STORE-* code. There is no escape hatch: an
obligation without a witness or a rejection class without a code
blocks closure.

### 19.3 Theorem corollary: F-B8 unblocks F-B9..F-B13

```text
Corollary 1 (F-B9 unblocked):
  ∀ valid F-B8 output sp.
    sp.bindings filtered to {Materialize { class: SramPaged, .. },
                              Persist { .. }} is well-typed for
    F-B9's consumption surface.

Corollary 2 (F-B10 unblocked):
  ∀ valid F-B8 output sp.
    sp.bindings filtered to Materialize { class: RomConst, .. } is
    well-typed for F-B10's consumption surface.

Corollary 3 (F-B11 unblocked):
  ∀ valid F-B8 output sp.
    sp.bindings under the IsOverlayEligible lens (§11.5) is
    well-typed for F-B11's consumption surface.

Corollary 4 (F-B12 unblocked):
  ∀ valid F-B8 output sp.
    sp.bindings filtered to Materialize is well-typed for F-B12's
    byte-allocation pass; Recompute bindings are excluded from
    arena allocation by construction.

Corollary 5 (F-B13 unblocked):
  ∀ valid F-B8 output sp.
    sp.alias_classes plus sp.bindings.alias_class is the
    equivalence-class lens F-B13's ResourceStateValidation
    consumes; the relation is reflexive, symmetric, transitive, and
    closed under the four declared non-singleton sharing intents.

Corollary 6 (F-B16 wired):
  ∀ valid F-B8 output sp.
    sp.repair_proposals is a well-typed list of PromoteRecompute
    proposals admissible to F-B16's loop driver under
    RepairPolicy::allow_recompute_promotion (when F-B16 lands).
```

These six corollaries, together with the End-to-End Theorem, are the
M3 commitment: "value/effect `GbInferIR` + `ObservationPlan` +
`RangePlan` + `StoragePlan` wired end-to-end for a routed FFN under
the cooperative scheduler." The "wired end-to-end" claim is satisfied
when the theorem and all six corollaries hold under the routed-FFN
fixture.

## 20. Final concise contract

### 20.1 What F-B8 owns

```text
Stage 6: StoragePlan
Schema:  storage_plan.v1
Cache:   K6
Codes:   STORE-001..STORE-035
Rules:   DR-1, DR-1b, DR-2, DR-3a, DR-3..DR-13
Types:   StorageClass, LifetimeClass, Materialization, StorageBinding,
         AbstractLiveRange, AliasClassId, AliasClassFingerprint,
         AliasClass, AliasIntent, PersistPageId, PersistPageDecl,
         CommitGroupId, CommitGroupDecl, CommitAtomicityClass,
         PersistSchemaPin, BindingJustification, DecisionRuleId,
         ValueRole, ValueFormat, StoragePlan,
         StoragePlanInputIdentity, StorageProvenance.
Reports: storage_plan.json (always emitted on success or failure).
Repair:  RepairReason::PromoteRecompute proposals (proposal-only;
         acceptance is F-B16's authority).
```

### 20.2 What F-B8 does NOT own

```text
Byte ranges:        F-B12 ArenaPlan.
Bank residency:     F-B10 RomWindowPlan.
Overlay installs:   F-B11 OverlayPlan.
SRAM page families: F-B9  SramPagePlan.
Slice schedules:    F-B13 GbSchedIR.
Resource leases:    F-B13 GbSchedIR.
Interrupt policy:   F-B13 GbSchedIR.
Kernel selection:   F-H1 KernelSpec / F-B13.
Reduction plans:    F-B7  RangePlan.
Observation plans:  F-B6  ObservationPlan.
Quant graphs:       F-B3  QuantGraph.
Value/effect IRs:   F-B5  GbInferIR.
Refinement loop:    F-B16 FeasibilityRefinementLoop.
Persist bytes:      gbf-runtime::persistence (Epic D).
```

### 20.3 The bridge metaphor, restated

F-B8 is the unique stage where:

* **Inputs** carry no spatial commitment: every ValueId in
  `GbInferIR` has no storage class, no lifetime class, no
  materialization, no alias class, no concrete byte offset, no
  concrete bank, no concrete page, no slice membership.
* **Outputs** carry abstract spatial commitment: every ValueId has
  exactly one `StorageBinding` with a typed `Materialization`,
  `AliasClassId`, and (for materialized values) `StorageClass` and
  `LifetimeClass`; every `Persist` binding has a typed
  `PersistPageId` and `CommitGroupId` plug-compatible with the SRAM
  persistence protocol.
* **No byte-range, bank, page, slice, overlay, or kernel-residency
  decision is committed.** Those are the next stages' authority.

The bridge is asymmetric and irreplaceable: bypassing it forces
storage decisions to leak into either upstream (storage-free) IRs or
downstream (byte-addressed) stages, and both leaks recreate the
exact failure mode F-B8 exists to prevent.

### 20.4 Closure summary

F-B8 closes when, on a clean checkout:

1. The closed type surface (§8.2, §8.3, §8.4) compiles.
2. The decision-rule list (§9.2) fires correctly on the routed-FFN
   dense fixture and the M1 degenerate fixture.
3. `storage_plan.json` round-trips through canonical JSON and
   self-hash on both fixtures.
4. The alias-class equivalence relation satisfies PO-3, PO-4, PO-5
   on both fixtures.
5. Every Persist binding satisfies PO-9, PO-10, PO-11 on the
   harness/trace test fixture.
6. K6 cache hit/miss tests pass.
7. Every rejection class (§17) has a corresponding STORE-* unit test.
8. F-B16's RepairProposal handshake is wired (proposal-only; no
   acceptance).
9. All 23 proof obligations (§18) have witnesses in the test suite.
10. The fixture build emits `stages/storage_plan.json` under
    `Trace` builds or cold StageCache (per `planv0.md` line 2820).
11. K6 is computable before Stage 6 output exists and contains no
    output self-hash.
12. Every multi-member alias class has exactly one intent, satisfies
    that intent's cardinality constraints, and carries live-range
    evidence sufficient to check non-conflict without byte ranges.
13. No v1 report emits a schema-reserved variant.

Non-negotiable v1 consistency checks:

* `StoragePlanInputs` carries every hash later used by identity, K6,
  and STORE-* mismatch diagnostics.
* `storage_plan.json` round-trips the full typed `StoragePlan` product:
  no typed field may be omitted from the JSON `result` unless the RFC
  explicitly declares the report to be a projection.
* Every named promotion level admits at least one additional class of
  values, or the level is removed/reserved in v1.
* Every example must satisfy the formal pair predicates in §11.4.
* Every reserved-v1 shape is either non-emittable with a STORE-* gate
  or explicitly legal; no shape may be both reserved and closure-valid.

The bridge is then load-bearing: every later spatial stage can plug
into F-B8's typed surface without re-deriving materialization,
lifetime, persistence, or aliasing. The transformative pipeline can
proceed.
