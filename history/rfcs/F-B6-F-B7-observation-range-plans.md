# RFC F-B6 + F-B7: Observation & Range — `ObservationPlan` (Stage 4) and `RangePlan` (Stage 5)

## -1. Authority and amendment policy

This RFC is the source of truth for F-B6 and F-B7 implementation.
`history/planv0.md` remains the architectural context document, but this RFC
is allowed to refine, narrow, or supersede `planv0.md` wherever this RFC makes
a more precise implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence must
be recorded in an `Amends planv0` note close to the relevant decision. This is
not a request to edit `planv0.md` immediately; it is a local source-of-truth
ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B6/F-B7 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a
  shared surface (canonical JSON rule, self-hash convention, diagnostic
  envelope, StageCache key construction, `ReportEnvelope<R>` shape,
  domain-separated hashing), the F-B2/F-B4 RFC wins. F-B6/F-B7 inherit
  those surfaces unchanged unless this RFC explicitly amends them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on a shared surface
  (`QuantGraph` shape, `GbInferIR` shape, `ValueId`/`EffectId`/`NodeId`
  identity, `NodeAnchorMap`, `ReductionSiteId`, `op_signature` predicate,
  `ValueFormat`, `EffectClass`, single-token convention), the F-B3/F-B5 RFC
  wins. F-B6/F-B7 inherit those surfaces unchanged unless this RFC
  explicitly amends them.
* If a later RFC changes any public type, report shape, cache key, diagnostic
  code, certificate shape, or canonicalization rule introduced here, that
  later RFC must explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not
  prose folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft (rev 0; pre-review) |
| Feature beads   | bd-1y0 **F-B6 ObservationPlan (Stage 4)**; bd-2x0 **F-B7 RangePlan (Stage 5)** |
| Open tasks      | To be minted: T-B6.1..T-B6.N (semantic-checkpoint subset selection from `SemanticCheckpointSchema`, probe-id selection by `ProbeBudgetClass`, metric-id selection, NodeAnchor binding, schema/round-trip tests for `semantic_checkpoint_schema.json` re-emit and `operational_probe_schema.json`, StageCache K4 wiring); T-B7.1..T-B7.M (per-reduction `ReductionPlan` selection, accumulator-bound proof obligation, `range_plan.json` schema, `certs/range.cert.json` schema, StageCache K5 wiring, F-B8 / F-B13 handshake) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` §"The compiler pipeline" stages 4 and 5 (lines 1618–1664); §"Reports and artifacts" `semantic_checkpoint_schema.json`, `operational_probe_schema.json`, `certs/range.cert.json` (lines 1987 and 2825); §"Three oracles" / `SemanticCheckpointSchema` (line 448); `ProbeBudgetClass` / `TraceProbeId` / `MetricId` (line 1217 et seq.); `ReductionPlanCeiling` (line 1228); `ReductionSiteId` (line 1383); `SemanticCheckpointId` (line 2280); §"Workloads" / `ObservationPolicy` (lines 770–920) |
| Glossary        | `history/glossary.md` (artifact stratum, denotational stratum, value/effect IR, observation contract, semantic checkpoint, operational probe, metric probe, probe budget class, reduction plan, accumulator certificate, named numeric boundary, stage cache, evidence ref) |
| Constitution    | §I correctness by construction; §II three-stratum oracle correspondence; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ValidatedInputs`, `ResolvedCompilePolicy`, `CompileKnobs`, `ReportEnvelope`, `ValidationDiagnostic`, `StageCache` key construction, canonical-JSON rule, self-hash convention); F-B3/F-B5 Canonical IRs (provides `QuantGraph`, `GbInferIR`, `ValueId`/`EffectId`/`NodeId`, `NodeAnchorMap`, `ReductionSiteId`, `op_signature`, `ValueFormat::ExactAccumulator`, single-token convention); F-B8 StoragePlan (Stage 6 — consumes `RangePlan` for scratch sizing, consumes `ObservationPlan` only for hash binding); F-B13 GbSchedIR (Stage 10 — consumes `RangePlan` for tile shapes); F-B16 FeasibilityRefinementLoop (named-only here; reads `CompileKnobs::observation` + `CompileKnobs::range`); F-C2 ArtifactOracle (consumes the re-emitted `semantic_checkpoint_schema.json` for checkpoint-aligned diffing); F-A8 gbf-debug (consumes `operational_probe_schema.json` to render trace events) |
| Sister deps     | bd-2k0 (F-B8) — direct consumer of `RangePlan`; bd-32w5 (T-B16.6) — refinement-loop driver, reads `RangePlan`'s certificate via `RepairProposal`; bd-3ix (F-B16, BLOCKED on oracle); bd-txth (F-F2 Certificates) — owns the certs sub-namespace |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B6 + F-B7 sits inside the
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

Transformative stages (wrapped by FeasibilityRefinementLoop on 5–11):
  F-B5  Stage 3        GbInferIR (value/effect IR)
  F-B6  Stage 4        ObservationPlan                                  ← THIS RFC
  F-B7  Stage 5        RangePlan                                        ← THIS RFC
  F-B8  Stage 6        StoragePlan ("the bridge")
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
        (BLOCKED on oracle question)
  F-B17 StageCache integration sweep across all stages
        (uniformization pass; F-B6 / F-B7 wire K4 / K5 directly here)
```

Sequencing of ~weekly chunks (bkase 2026-05-07 conversation):

```text
Chunk 1 (closed/in flight): F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2 (merged):           F-B3 + F-B5         Stages 1, 3
Chunk 3 (THIS RFC):         F-B6 + F-B7         Stages 4, 5
Chunk 4 (next up):          F-B8                Stage 6
Chunk 5:                    F-B9 + F-B10        Stages 7, 8
Chunk 6:                    F-B11 + F-B12       Stages 8.5, 9
Chunk 7:                    F-B13               Stages 10, 10.5
Chunk 8:                    F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:                    F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):          F-B16               Refinement loop
```

### 0.3 Where F-B6 and F-B7 sit in the pipeline

F-B6 and F-B7 are the **two passive planning stages** that bracket
`StoragePlan` (F-B8). Both run after `GbInferIR` is fully constructed and
before any storage, residency, scheduling, or layout decision is made.

```text
Stage 0 → Stage 0.5 → Stage 1 → Stage 2 → Stage 3 → Stage 4 → Stage 5 → Stage 6 → ...
F-B2     F-B2          F-B3      F-B4      F-B5      F-B6      F-B7      F-B8

                              [ inputs already decided ]
                               ^                       ^
                               |                       |
                  ArtifactValidation             GbInferIR (value/effect IR)
                  ResolvedCompilePolicy          ResolvedCompilePolicy
                  StaticBudgetReport             SemanticCheckpointSchema (re-emit input)
                                                 ReductionSiteProjection (from F-B4)

F-B6 ObservationPlan     F-B7 RangePlan
─────────────────────    ───────────────────
SELECT semantic          SELECT a ReductionPlan
checkpoints from         per hot reduction:
artifact's exported        SingleI16
SemanticCheckpoint-      | ChunkedI16 { chunk_len }
Schema (mandatory        | RenormLoop  { tile_len }
+ build-active subset).
                         PROVE accumulator
SELECT operational       safety as a typed
probes from the          certificate. NO
sealed probe registry    heuristic guess.
by ProbeBudgetClass      NO silent integer-
governance.              width expansion.
SELECT metrics by
profile. EMITS:
   semantic_checkpoint   EMITS:
     _schema.json           range_plan.json
     (re-emit, build       certs/range.cert
      active subset)         .json

NO IR mutation.          NO IR mutation.
NO checkpoint            NO storage decisions.
creation. Active         NO scheduling fusion.
probes alter trace       Feeds StoragePlan
shape but NOT            (chunk_len influences
semantic comparison      scratch) and GbSchedIR
contracts.               (tile shapes).
```

* **F-B6 (Stage 4) `ObservationPlan`** consumes `SemanticCheckpointSchema`
  (a sidecar of `ArtifactAux`, exported by `gbf-train`) and `GbInferIR`. It
  produces an `ObservationPlan { semantic, probes, metrics }` and re-emits
  `semantic_checkpoint_schema.json` (the build-active subset; possibly
  equal to the artifact's full schema) plus
  `operational_probe_schema.json` (the build's selected operational probes).
  It selects from already-existing checkpoint, probe, and metric ids — it
  never invents new ones — and it never mutates `GbInferIR`. Probes attach
  by `ValueId`/`EffectId`/`NodeId` reference through the `NodeAnchorMap`
  surface F-B5 emits.

* **F-B7 (Stage 5) `RangePlan`** consumes `GbInferIR` (specifically the set
  of reduction-site-bearing nodes whose `reduction_site: Option<ReductionSiteId>`
  is `Some(_)`) plus `StaticBudgetReport.reduction_sites`, whose
  `ReductionSiteProjection` entries carry the accumulator facts already
  derived from `QuantGraph` by Stage 2. F-B7 records `quant_graph_self_hash`
  for audit and cache identity, but does not read `QuantGraph.tensors`
  directly. It selects a `ReductionPlan` per reduction-site-bearing node:

  ```rust
  enum ReductionPlan {
      SingleI16,
      ChunkedI16 { chunk_len: u16 },
      RenormLoop { tile_len: u16 },
  }
  ```

  Each selection is paired with a typed `AccumulatorCertificate` proving
  that the chosen plan's intermediate accumulator domain is provably
  representable in the implementation width the plan declares (i.e. that
  no intermediate sum overflows the `i16` envelope under `SingleI16`, that
  the per-chunk partial sum is bounded under `ChunkedI16 { chunk_len }`,
  or that the per-tile renormalization preserves canonical numeric
  semantics under `RenormLoop { tile_len }`). The certificate is a
  closure-gate proof, not a heuristic guess.

Stage 6 (`StoragePlan`, F-B8) sits immediately after this chunk in
pipeline order but is owned by a later chunk's RFC. F-B8 consumes
`RangePlan` to size scratch buffers (because `RenormLoop` and
`ChunkedI16` need different scratch shapes than `SingleI16`) and
consumes `ObservationPlan` only by hash for cache-key purposes (F-B8
does not read individual checkpoint or probe ids). Stage 10 (`GbSchedIR`,
F-B13) consumes `RangePlan` again because tile shapes change with
`chunk_len`/`tile_len`.

### 0.4 Cross-epic interactions

F-B6 + F-B7 sit at the intersection of three epics, with one explicit
hand-off to a fourth:

```text
Epic A → Epic B
  - gbf-foundation (BlobRef, Hash256 wrappers, EvidenceRef)        consumed
  - gbf-store (StageCache) for K4 / K5 cache wiring                consumed
  - gbf-artifact types (SemanticCheckpointSchema,
    SemanticCheckpointId, TraceProbeId, MetricId,
    ProbeBudgetClass — types live across gbf-artifact and
    gbf-policy; F-B6 only references them, never authors them)    consumed
  - gbf-policy (ResolvedCompilePolicy, CompileKnobs::observation,
    CompileKnobs::range, CompileProfileSpec, ObservabilityMode,
    TraceBudget)                                                   consumed

Epic B (internal):
  - F-B2 / F-B4 (Stage 0, 0.5, 2) products + ReportEnvelope rule   consumed
  - F-B3 / F-B5 (Stage 1, 3) products                              consumed
  - F-B8 (Stage 6 StoragePlan)                                     feeds
  - F-B13 (Stage 10 GbSchedIR)                                     feeds
  - F-B16 (FeasibilityRefinementLoop, BLOCKED)                     compatible
  - F-B17 StageCache cross-cut                                     compatible

Epic C → Epic B (oracle correspondence):
  - F-C2 ArtifactOracle reads the re-emitted
    semantic_checkpoint_schema.json (build-active subset) so its
    checkpoint-aligned diffing matches THIS build's contract.       provided
  - F-C2 dependency on the full set of IR/QG hashes established
    in Chunk 2 (F-B5); this chunk does not change that handshake.

Epic A → Epic D (runtime trace):
  - gbf-debug consumes operational_probe_schema.json to render
    trace events with the right event-shape, level, and budget
    class.                                                          provided

Epic F → Epic B (certificates and reports):
  - gbf-report owns the schemas for semantic_checkpoint_schema.json
    (re-emit), operational_probe_schema.json, range_plan.json, and
    certs/range.cert.json.                                          consumed
  - gbf-verify (F-F1) eventually consumes certs/range.cert.json as
    an independent reference check; this chunk pins the cert
    shape so F-F1 can implement against a stable contract.          provided
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk sits squarely on the M3 critical
path:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup: runtime/banking/harness/emulator
              proven for sustained integer compute. Merged: c2edbaa.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; first conformance.json; first CompileRequest
              wiring.
              ↳ F-B2/F-B4 (Chunk 1)   delivers the CompileRequest wiring.
              ↳ F-B3 (Chunk 2)        delivers ArtifactOracle's input
                                       surface (the canonical artifact
                                       graph).
              ↳ THIS chunk's F-B6 hands ArtifactOracle the build-active
                semantic_checkpoint_schema.json so checkpoint-aligned
                diffing reflects THIS build's actual checkpoint set.
                ArtifactOracle equality at SemanticCheckpointId
                boundaries (an M2 commitment) requires the
                build-active subset to exist; this chunk lands it.

M2            One shared micro-kernel resolved by RomWindowPlan; one
              expert payload bank; emulator diffing against
              ScheduleOracle; first ReachabilityValidation pass.
              ↳ THIS chunk's F-B7 produces certs/range.cert.json — the
                first machine-checkable certificate (per planv0.md
                line 2825). gbf-verify can now verify range proofs
                independently of the compiler.
              ↳ M2's "checkpoint alignment against ArtifactOracle at
                SemanticCheckpointId boundaries" requires F-B6 to have
                already chosen the build-active checkpoint subset.

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ THIS chunk delivers the ObservationPlan + RangePlan
                halves of M3's "wired end-to-end" commitment. The
                StoragePlan half lands in Chunk 4 (F-B8). Together
                they retire M3's planning surface.

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ Out of scope for this chunk. Sequence-state probes are
                reserved-not-emitted in v1 (they require F-B5's
                sequence-state amendment first; see §1.5).
```

### 0.6 What the project as a whole gains when this chunk lands

```text
1. ArtifactOracle (F-C2) becomes checkpoint-aware.
   Without ObservationPlan, the artifact-side oracle only knows the
   exported SemanticCheckpointSchema (the artifact's full contract).
   With ObservationPlan, the oracle knows THIS build's active subset
   and the agreed encoding/source per checkpoint, so checkpoint-aligned
   diffing has a stable, build-specific contract.

2. The first machine-checkable certificate ships.
   certs/range.cert.json is the first member of the certs/ namespace
   listed in planv0.md line 2825. The shape this RFC pins is reusable
   for arena, window, sram, resource_state, and reachability
   certificates in later chunks.

3. F-B8 (StoragePlan) becomes implementable.
   StoragePlan needs to know whether each reduction site uses
   SingleI16, ChunkedI16, or RenormLoop because the scratch shape
   differs. Without F-B7, StoragePlan is blocked at the scratch-sizing
   step.

4. F-B13 (GbSchedIR) tile shapes become predictable.
   ChunkedI16's chunk_len and RenormLoop's tile_len directly inform
   GbSchedIR's tile selection. Without F-B7, GbSchedIR has to either
   guess or derive an internal duplicate range analysis.

5. The shape of the M2 conformance.json gate is unblocked.
   F-C2's per-checkpoint envelope (per planv0.md line 828) requires
   semantic_checkpoint_schema.json to be the build-active subset, not
   the full schema. This chunk lands that re-emit.

6. The "passive-pass" discipline extends.
   The schema/canonicalization/self-hash/StageCache pattern from
   F-B2/F-B4 and F-B3/F-B5 now extends across two more reports
   (observation_plan.v1, range_plan.v1) plus one re-emit
   (semantic_checkpoint_schema.v1) plus one new operational schema
   (operational_probe_schema.v1) plus one certificate schema
   (range.cert.v1). F-B8 / F-B13's report shapes will inherit the same
   discipline.

7. Profile-tunable debug instrumentation is contractualized.
   ProbeBudgetClass governance means a Bringup build's optional probes
   are NOT promoted/demoted by accident. CompileKnobs::observation
   carries the lock surface; this RFC pins how F-B6 reads it.
```

### 0.7 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later transformative stage receives a typed, validated
  `ObservationPlanProduct` and `RangePlanProduct`. They never re-derive
  checkpoint selection, probe selection, or reduction-plan choice.
* F-B8 (`StoragePlan`) knows scratch shape per reduction site. It does
  not run a hidden range analysis.
* F-B13 (`GbSchedIR`) knows tile shape constraints. It does not pick
  tile size against an unmodelled accumulator domain.
* F-B16 (`FeasibilityRefinementLoop`) has a place to plug
  `KnobDelta::DisableOptionalProbes` (against `CompileKnobs::observation`)
  and `KnobDelta::RaiseReductionCeiling` (against `CompileKnobs::range`).
  Both knob-ids already exist in F-B2's `CompileKnobs`; this chunk
  honors them on read but does not consume `RepairProposal(_)`
  provenance during the chunk itself (F-B16 is BLOCKED).
* F-C2 (`ArtifactOracle`) reads the build-active
  `semantic_checkpoint_schema.json` directly — the artifact's full
  schema is no longer the diffing contract.
* `gbf-debug` reads `operational_probe_schema.json` to render trace
  events.

This chunk's job is to retire the **observation contract** and the
**accumulator-safety proof** preconditions of the rest of the pipeline.
F-B6 retires the observation half; F-B7 retires the range half. They are
the fifth and sixth shift-left filters in the system, after
`gbf-train preflight`, F-B2 (Stage 0/0.5), F-B4 (Stage 2), F-B3 (Stage 1),
and F-B5 (Stage 3).

### 0.8 Reading order for reviewers

A reviewer who has just read F-B2/F-B4 and F-B3/F-B5 and is approaching
this RFC for the first time should read:

```text
§0  (this section) — placement and dependencies
§1  Project context — milestone-specific framing and what's NOT in scope
§2  Load-bearing decisions — the engineering choices that bracket the rest
§5  Authority rules — what this RFC owns vs inherits
§6  Pipeline state machine — how Stage 4 and Stage 5 plug into Stage 3 / Stage 6
§8  Stage 4 contract: ObservationPlan
§9  Stage 5 contract: RangePlan
§10 Report schemas (semantic_checkpoint_schema.v1 re-emit,
                    operational_probe_schema.v1, range_plan.v1,
                    certs/range.cert.v1)
§13 Cross-stage interactions
§14 Task DAG
§17 End-to-end theorem
```

Skim §3, §4, §7, §11, §12, §15, §16, §18 for specifics.

## 0a. TL;DR

This chunk lands the **two passive planning stages** that bracket Stage 6
(`StoragePlan`) and feed every transformative stage from Stage 6 onward. It
owns two numbered stages:

* **Stage 4 — `ObservationPlan`.** The build-active observation contract.
  Consumes the artifact's exported `SemanticCheckpointSchema` (a sidecar
  of `ArtifactAux`) and `GbInferIR`. SELECTS — never invents — the
  semantic checkpoints, operational probes, and metric probes that THIS
  build emits. Re-emits `semantic_checkpoint_schema.json` as the build-
  active subset (a subset of the artifact's full schema, possibly equal
  to it) and emits
  `operational_probe_schema.json` (the build's selected operational
  probes). Probes attach by `ValueId`/`EffectId`/`NodeId` reference via
  the `NodeAnchorMap` F-B5 emits; they never mutate IR. Active probes
  alter trace shape but NOT semantic comparison contracts (per
  `ObservabilityMode::Invariant`).

* **Stage 5 — `RangePlan`.** The accumulator-safety proof. Consumes
  `GbInferIR`'s reduction-site-bearing nodes (those with
  `reduction_site: Option<ReductionSiteId> = Some(_)`) and the
  accumulator facts already projected into
  `StaticBudgetReport.reduction_sites` by Stage 2. It records
  `quant_graph_self_hash` for audit/cache identity but does not read
  `QuantGraph.tensors` directly. Selects a
  `ReductionPlan` per reduction-site-bearing node:

  ```rust
  enum ReductionPlan {
      SingleI16,
      ChunkedI16 { chunk_len: u16 },
      RenormLoop { tile_len: u16, renorm: RenormSpec },
  }
  ```

  Each selection is paired with a typed `AccumulatorCertificate` proving
  the chosen plan's intermediate accumulator domain stays within the
  declared implementation width. The certificate is a closure-gate
  proof, not a heuristic guess. Emits `range_plan.json` plus
  `certs/range.cert.json`.

These two features are paired in one RFC because they share the
**passive-pass** shape inherited from F-B2/F-B4 and the
**canonical-typed-IR** input shape inherited from F-B3/F-B5: each is a
deterministic pure function of `GbInferIR` + a small projection of
`ResolvedCompilePolicy` (plus, for F-B6, the artifact's full
`SemanticCheckpointSchema`); each emits a canonical JSON report with
report-side self-hash; each is consumed by the next stage by hash; each
shares the diagnostic envelope, JSON canonicalization rule, self-hash
convention, and `StageCache` key construction inherited from F-B2/F-B4
and F-B3/F-B5. They run in numbered pipeline order (Stage 4 then Stage 5)
but they are independent: F-B7 does not read `ObservationPlan` and F-B6
does not read `RangePlan`.

The chunk closes only when:

1. `ObservationPlan` construction is a deterministic pure function of
   `GbInferIRProduct` + `SemanticCheckpointSchema` +
   hash-bound probe / metric / trace-event-layout registry snapshots +
   `ObservationPolicyProjection`, and is
   byte-identical across two consecutive regenerations on a clean
   checkout.
2. `RangePlan` construction is a deterministic pure function of
   `GbInferIRProduct` + `StaticBudgetReport` +
   `RangePolicyProjection`, with `QuantGraph` read only through
   `quant_graph_self_hash` and the accumulator facts already projected
   into `StaticBudgetReport.reduction_sites`. It is byte-identical
   across two consecutive regenerations on a clean checkout.
3. `semantic_checkpoint_schema.json` (re-emit), `operational_probe_schema.json`,
   `range_plan.json`, and `certs/range.cert.json` round-trip through their
   semantic validators and self-hashes.
4. Every reduction-site-bearing node in `GbInferIR` has exactly one
   matching `ReductionPlan` entry whose `AccumulatorCertificate`
   verifies under canonical reference semantics. No site is missing,
   duplicated, or under-proven.
5. Every checkpoint/probe/metric id selected by F-B6 exists in either
   the artifact's `SemanticCheckpointSchema` (for semantic checkpoints)
   or a typed registry (for operational probes and metrics — these
   registries live in `gbf-policy` and `gbf-artifact`; this chunk
   consumes them, never authors new ids).
6. `StageCache` keys K4 (`ObservationPlan`) and K5 (`RangePlan`) are
   pinned and tested.
7. `CompileKnobs::observation` and `CompileKnobs::range` are honored on
   read: a locked knob is not silently overridden, and an out-of-bounds
   selection is a hard diagnostic.
8. The fixture build (`fixtures/observation_plan/` and
   `fixtures/range_plan/`) emits enough data for a later
   `ArtifactOracle` (F-C2) and `gbf-verify` implementation to consume
   both reports and the certificate by hash.

The chunk does **not** include:

* Storage class / lifetime / materialization decisions — owned by F-B8
  (Stage 6).
* Tile sizing for non-reduction ops, scratch byte ranges, or scheduling
  fusion — owned by F-B13 (`GbSchedIR`, Stage 10).
* SemanticCheckpointSchema **creation**: the schema is exported by
  `gbf-train` and resides in `ArtifactAux`. F-B6 only re-emits the
  build-active subset.
* `TraceProbeId`/`MetricId` **creation**: the registries live in
  `gbf-policy` (`trace::PROBE_REGISTRY` and
  `metrics::METRIC_REGISTRY`).
  F-B6 only selects from existing ids.
* Probe-budget invention: `ProbeBudgetClass` (`Required` / `Important`
  / `Diagnostic` / `BestEffort`) is an existing closed enum in
  `gbf-policy`. F-B6 only respects the `optional_probe_floor` knob and
  the per-class budget caps on the active build profile.
* Refinement-loop repairs — F-B16 owns `RepairProposal`,
  `ConstraintDelta`, `KnobDelta::DisableOptionalProbes`,
  `KnobDelta::RaiseReductionCeiling`, the loop driver, and
  `repair_report.json`. This chunk emits the schemas F-B16 will consume
  but never accepts a `RepairProposal(_)` provenance.
* Sequence-state probe attachment: `SequenceState(StateSlotId)` effect
  chains are reserved-not-emitted in v1 of `GbInferIR` (per F-B3/F-B5
  §2.5a). This chunk follows that decision: probes attached to
  sequence-state effect ids are rejected with
  `OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED`.
* `MetricProbe` aggregation across runs — that is `gbf-bench`'s job.
  F-B6 only declares which metrics are sampled at which sites, not how
  they are statistically aggregated.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What F-B2/F-B4 and F-B3/F-B5 leave on the table

Per the F-B2/F-B4 RFC and the F-B3/F-B5 RFC, by the time this chunk
begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`,
  `TargetDataLoweringArtifact`, calibration, hint bundle,
  `CompileRequest`, and `RuntimeChromeBudget` are all admissible,
  hash-bound, and traceable through `artifact_validation.json`.
* `ResolvedCompilePolicy` is the single answer to "what policy
  governed this build," with provenance for every load-bearing scalar.
  In particular, `CompileKnobs::observation` (carrying
  `TraceDemotionLevel` and `optional_probe_floor: ProbeBudgetClass`)
  and `CompileKnobs::range` (carrying
  `reduction_ceiling: ReductionPlanCeiling` plus a possibly-empty
  `reduction_ceiling_overrides: BTreeMap<ReductionSelector, ReductionPlanCeiling>`)
  are populated and provenance-stamped.
* `RuntimeChromeBudget` is honored at the static byte-math level via
  `static_budget.json`. F-B7 reads `static_budget.json`'s
  `ReductionSiteProjection` entries to anchor its per-site analysis;
  it does not re-derive shape, term count, or input/weight max-abs.
* `QuantGraph` (F-B3) is the canonical artifact graph. F-B7 does not
  read `QuantGraph.tensors` directly. F-B4 has already projected the
  load-bearing accumulator facts into `StaticBudgetReport`.
  F-B7 records `quant_graph_self_hash` for audit and cache identity,
  and consumes accumulator facts through
  `StaticBudgetReport.reduction_sites`.
  F-B6 reads only `QuantGraph.identity` (for hash binding) and
  `QuantGraph.classify_head`, `QuantGraph.norm_plans`, and
  `QuantGraph.expert_sections` only via their hashes embedded in
  `GbInferIR.identity.quant_graph_self_hash`.
* `GbInferIR` (F-B5) is the value/effect IR. F-B6 reads the full
  `GbInferIR` for `NodeAnchorMap`, `ValueDecl` set, `EffectDecl` set,
  and `op_signature` consistency. F-B7 reads only the subset of
  `GbNode` whose `reduction_site` is `Some(_)`.
* The `SemanticAnchor` ids in `NodeAnchorMap` are stable, hash-derived,
  and exported in `infer_ir.json`'s `result.product.anchors`. F-B6
  attaches `SemanticCheckpointId` and `TraceProbeId` references to
  those anchors without changing IR shape.
* `ReductionSiteId` values are minted by F-B4 in `static_budget.json`'s
  `ReductionSiteProjection` entries. F-B5 carries the same id forward
  on `GbNode.reduction_site`. F-B7 looks up the reduction by id; it
  never mints new ids.
* The single-token convention holds: exactly one `Embedding` node,
  one `Classify` node, one `DecodeToken` node per IR pass. F-B6
  attaches `SemanticCheckpointId::PostEmbedding`,
  `SemanticCheckpointId::PostLogits`, `SemanticCheckpointId::PostDecode`
  (and per-layer variants) to the unique node corresponding to each
  checkpoint id; the attachment is unambiguous.

This chunk is responsible for selecting the build-active observation
contract from these inputs, and proving accumulator safety per hot
reduction with a typed certificate.

### 1.2 What M2/M3 commits to and how this chunk delivers it

Per `planv0.md` §"Milestones":

> **M2**: one shared micro-kernel resolved by `RomWindowPlan`, plus
> one expert payload bank, with exact emulator diffing against
> `ScheduleOracle` and **checkpoint alignment against `ArtifactOracle`
> at `SemanticCheckpointId` boundaries**; first
> `ReachabilityValidation` pass integrated into the backend.
> **M3**: top-1 router, expert dispatch table, value/effect
> `GbInferIR` + **`ObservationPlan` + `RangePlan` + `StoragePlan`**
> wired end-to-end for a routed FFN under the cooperative scheduler.

Mapping:

* M2's "checkpoint alignment against `ArtifactOracle` at
  `SemanticCheckpointId` boundaries" requires F-B6 to emit a
  build-active `semantic_checkpoint_schema.json` so that the oracle
  knows which checkpoints THIS build collects. Without that subset,
  the oracle either compares against the artifact's full schema
  (over-strict; many checkpoints will be unobservable in this build)
  or has to guess (wrong by construction). F-B6 lands the subset.

* M2's "first machine-checkable certificate" target requires F-B7 to
  ship `certs/range.cert.json` with a typed proof obligation per
  reduction site. Per `planv0.md` line 2825 the certs/ namespace also
  holds arena, window, sram, resource_state, and reachability — those
  ship in later chunks; F-B7 lands the first one and pins the cert
  shape pattern.

* M3's "value/effect `GbInferIR` + `ObservationPlan` + `RangePlan` +
  `StoragePlan` wired end-to-end" requires F-B6 + F-B7 to land before
  F-B8 can be implemented. This chunk delivers the F-B6 + F-B7 halves;
  F-B8 lands in Chunk 4.

Because M2 lands before M3, the F-B6 half of this chunk is the M2-
shaped half (checkpoint alignment unblocks `ArtifactOracle` envelope
gates) and the F-B7 half is the M2/M3-shaped half (range certs unblock
`gbf-verify` independent verification, plus tile shapes feed
`StoragePlan` and `GbSchedIR`). Sequencing inside the chunk (§14)
reflects that F-B6 may merge first (it has fewer downstream dependents)
or F-B7 may merge first (it is a smaller surface). The two are
independent and may merge in either order so long as both close before
Chunk 4 begins.

### 1.3 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* F-B8 (`StoragePlan`) consumes `RangePlan` directly: each reduction
  site's plan declares the scratch shape `StoragePlan` must size.
  F-B8 does not re-derive any range fact.
* F-B8 also consumes `ObservationPlan` only by its self-hash. F-B8
  treats the observation plan as opaque content; only F-B13
  (`GbSchedIR`) reads the per-probe attachment ids to plan trace
  emission.
* F-B13 (`GbSchedIR`) consumes `RangePlan` for tile shape constraints
  (per `planv0.md` line 1663 — `tile shapes may change`). It consumes
  `ObservationPlan` to plan trace emission slots.
* F-B14 (`ScheduleCostAnalysis`) consumes `ObservationPlan`'s
  `TraceBudget` projection to model `trace_bytes_per_frame` cost
  envelopes (per `planv0.md` line 1320).
* F-B16 (`FeasibilityRefinementLoop`) consumes `RangePlan`'s
  certificate to know whether a reduction site is currently at
  `SingleI16Only`, `AllowChunkedI16`, or `AllowRenormLoop` so the
  loop can issue `KnobDelta::RaiseReductionCeiling`. It consumes
  `ObservationPlan`'s probe budget so it can issue
  `KnobDelta::DisableOptionalProbes` against
  `disabled_optional_probes`.
* F-C2 (`ArtifactOracle`) consumes the build-active
  `semantic_checkpoint_schema.json` to align its checkpoint-by-
  checkpoint envelope checks (per `planv0.md` line 828 —
  `per_checkpoint: BTreeMap<SemanticCheckpointId, EnvelopeGate>`).
* `gbf-debug` consumes `operational_probe_schema.json` to render
  trace-event names, levels, and budget classes in the agent CLI.
* `gbf-verify` (F-F1) consumes `certs/range.cert.json` as an
  independent, slow reference check against the compiler's range
  proofs.

This chunk's job is to retire the **observation contract** and the
**range proof** preconditions of the rest of the pipeline. It is
the **fifth and sixth shift-left filters** in the system, after
`gbf-train preflight` (deployability), F-B2 (Stages 0/0.5), F-B4
(Stage 2), F-B3 (Stage 1), and F-B5 (Stage 3).

### 1.4 Why this is two paired Features, not one feature or three

The natural unit is "the two passive planning stages that bracket
Stage 6 and feed every transformative stage from Stage 6 onward."

* If we made it one feature, the bead would carry a checkpoint/probe
  selector and a reduction-plan certifier in the same PR. The
  implementation surfaces are independent (no shared types beyond
  inherited surfaces) and would only fragment review.
* If we made it three features (e.g. F-B6, F-B7, F-B7c for the
  certificate), we would split on artifact emission. That split is
  artificial: the certificate is the proof of correctness of the
  selection, not a separate selection.
* Two features matches the natural seam: F-B6 owns "the build-active
  observation contract," F-B7 owns "the accumulator-safety proof."
  They are paired in this RFC because they share an inheritance
  pattern (passive pass + canonical IR input + content-addressed
  output + StageCache wiring) but ship as separate beads to keep PR
  scope tight, and to allow either to land first.

### 1.5 What this chunk is NOT

The chunk is **medium in scope** but **very large in contract surface**.
To prevent scope creep, here is what this chunk explicitly is not:

* It is **not** a transform stage in the operational sense. F-B6 is a
  selector over an existing schema; F-B7 is a certifier of an existing
  plan. Neither rewrites `GbInferIR`, `QuantGraph`, or
  `ResolvedCompilePolicy`.
* It is **not** the producer of `SemanticCheckpointSchema`. The schema
  is exported by `gbf-train` as part of `ArtifactAux` (per `planv0.md`
  line 442). F-B6 reads it by hash, selects a build-active subset, and
  re-emits the subset. The full schema is the contract; the subset is
  the build-specific honored contract.
* It is **not** the producer of new `SemanticCheckpointId` variants.
  The closed enum lives in `gbf-abi` (per `planv0.md` line 275, 2280).
  Adding a checkpoint id is a `gbf-abi` schema change, not an F-B6
  change.
* It is **not** the producer of new `TraceProbeId` variants. The probe
  registry lives in `gbf-policy` (the canonical home for probe ids
  consumed by `CompileKnobs::observation`) and is referenced through
  `OperationalProbe.probe_id`. Adding a probe id is a `gbf-policy`
  schema change, not an F-B6 change.
* It is **not** the producer of new `MetricId` variants. The metric
  registry lives alongside the probe registry. Same rule applies.
* It is **not** a `ProbeBudgetClass` inventor. The closed enum
  `{Required, Important, Diagnostic, BestEffort}` lives in `gbf-policy`
  (per `planv0.md` line 1217). F-B6 reads
  `CompileKnobs::observation::optional_probe_floor` — every probe
  whose `budget_class` is below the floor is dropped (added to
  `disabled_optional_probes` semantically; this chunk does not write
  to that map — F-B16 does).
* It is **not** an autoregressive driver. F-B6 attaches probes to one-
  pass IR nodes; multi-token probe aggregation is at runtime
  (`gbf-debug` / `gbf-emu`) and benchmark time (`gbf-bench`).
* It is **not** a buffer/storage planner. Probes attach by reference,
  not by buffer or arena. Range plans declare implementation widths,
  not buffer addresses.
* It is **not** an op-signature checker. F-B5 owns `op_signature` and
  every `GbNode` is already validated. F-B6 reads anchors derived from
  validated nodes; F-B7 reads `reduction_site` from validated nodes.
* It is **not** a refinement loop. Both stages are immutable products
  of their inputs; no Stage 4 or Stage 5 pass calls earlier passes
  recursively. F-B16 calls Stage 5 (and Stage 4) again with revised
  knobs, but that is loop-driver behavior, not internal recursion.
* It does **not** depend on F-C2 (`ArtifactOracle`). F-C2 depends on
  the schemas this chunk emits; within this chunk we land the schemas
  and synthetic dense + routed fixtures against which a future F-C2
  PR can verify checkpoint-aligned diffing.
* It does **not** ship `KnobDelta::DisableOptionalProbes` or
  `KnobDelta::RaiseReductionCeiling` execution. Those are F-B16
  surfaces. This chunk pins the schemas they will mutate; it never
  applies a delta.
* It does **not** consume sequence-state probe attachment. Per F-B5
  §2.5a, sequence-state effect chains are reserved-not-emitted in v1.
  Probes that target a `SequenceState { .. }` effect class are rejected
  here with `OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED`.
* It does **not** consume `FaultBoundary` effect probes. Same rule as
  sequence-state: the effect class is reserved-not-emitted in v1
  (per F-B5 §9.3 / §9.5). Diagnostic
  `OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED`.
* It does **not** assume any concrete kernel choice. `RangePlan`
  declares numeric structure (single i16 vs chunked vs renorm), not
  kernel selection. Kernel selection happens in F-B13.

### 1.6 Relationship to F-B16 (`FeasibilityRefinementLoop`)

F-B16 is BLOCKED on an oracle question and is named-only here. The
relevant F-B16 surfaces F-B6/F-B7 must respect on **read**:

* `CompileKnobs::observation::trace_demotion: TraceDemotionLevel`
  values:
  * `None` — no probes are dropped by demotion. F-B6 reads it as a
    monotone floor: every selected probe stays in.
  * `DropBestEffort` — F-B6 drops every probe with
    `budget_class = BestEffort`.
  * `DropDiagnosticAndBestEffort` — F-B6 drops both `Diagnostic` and
    `BestEffort` probes.
  * `RequiredOnly` — F-B6 keeps only `Required` probes. (Optional:
    `Important` are also dropped here.)
* `CompileKnobs::observation::optional_probe_floor: ProbeBudgetClass`
  acts as a monotone bound: a probe whose
  `budget_class < optional_probe_floor` (in
  `Required > Important > Diagnostic > BestEffort` order) is dropped
  silently. The class order pins the meaning of "below."
* `CompileKnobs::range::reduction_ceiling: ReductionPlanCeiling` is
  the per-site ceiling, with the values:
  * `SingleI16Only` — F-B7 must pick `ReductionPlan::SingleI16` for
    every reduction site, or fail with
    `RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY`.
  * `AllowChunkedI16` — F-B7 may pick `SingleI16` or
    `ChunkedI16 { chunk_len }`. `RenormLoop` is forbidden under this
    ceiling.
  * `AllowRenormLoop` — F-B7 may pick any of the three.
* `CompileKnobs::range::reduction_ceiling_overrides:
  BTreeMap<ReductionSelector, ReductionPlanCeiling>` provides per-site
  or per-layer overrides. F-B7 honors a more specific override
  (`Site` beats `Layer` beats global ceiling).
* `KnobLockSet::locked: BTreeSet<CompileKnobId>` — Stage 0.5 owns
  locked-knob enforcement. F-B6 and F-B7 assume the resolved knobs
  are already valid, record lock bits for audit, and do not emit
  locked-knob drift diagnostics in this chunk.

F-B6/F-B7 never apply a `RepairProposal(_)` provenance. The set of
allowed `PolicySource` values stays
`{TargetDefault, ProfileDefault, CompileRequestOverride, HintBundle, Calibration}`
(inherited from F-B2/F-B4 §2.7). When F-B16 unblocks, it adds
`RepairProposal(_)` as a sixth legal source by amending F-B2/F-B4 and,
transitively, this RFC.

## 2. Load-bearing decisions

### 2.1 Passive-pass shape

Both stages are **passive**: each consumes pinned inputs, runs typed
checks plus typed selection or certification, and emits a canonical
JSON report (or in F-B7's case, a report plus a certificate). Neither
stage transforms `GbInferIR`, `QuantGraph`, or
`ResolvedCompilePolicy`.

The chunk-level pass shape, lifted from F-B2/F-B4 §2.1 and extended
with the F-B3/F-B5 pure-core / driver split (§2.1):

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed selection — F-B6)
       (typed plan selection + certification — F-B7)
       (typed semantic checks)
       (typed provenance binding)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed product, ReportEnvelope<ReportV1>,
                        ReportEnvelope<CertEnvelopeV1>?  -- F-B7 only }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON (one report file; F-B7 also emits cert)
       writes StageCache success / failure memo
```

Drivers are the only IO surface. Determinism is required, not
aspirational.

Every report includes `outcome: ReportOutcome` per F-B2/F-B4 §2.1.
F-B7 also produces a `certificate.outcome: CertOutcome` whose value
is `Verified` on success and `Failed` when the emitted certificate
report contains either at least one
`AccumulatorCertificate::Failed { proof_state, witness }` or at
least one hard certificate diagnostic. If Stage 5 fails before any
certificate attempt can be constructed, `cert_report` may be absent.

### 2.2 Pure-function shape

Both stages have **two layers**: a pure core constructor and a thin
driver that performs IO. The core is a pure function from typed
pinned inputs to typed content-addressed products. The driver wraps
the core with JSON emission and StageCache writes.

```text
build_observation_plan_core(ObservationPlanInputs)
  -> Result<ObservationPlanCoreSuccess, ObservationPlanCoreFailure>

ObservationPlanCoreSuccess :=
  {
    product: ObservationPlanCoreProduct,
    observation_plan_body: ObservationPlanReportBody,
    sc_re_emit_body: SemanticCheckpointSchemaReEmitBody,
    operational_probe_body: OperationalProbeSchemaBody,
  }

ObservationPlanCoreFailure :=
  {
    observation_plan_body: ObservationPlanReportBody,
    sc_re_emit_body: Option[SemanticCheckpointSchemaReEmitBody],
    operational_probe_body: Option[OperationalProbeSchemaBody],
    diagnostics: NonEmptyList[ValidationDiagnostic],
  }

run_stage4(ObservationPlanInputs, env)
  = build_observation_plan_core(...) then
    (on success or failure):
      wrap report bodies in ReportEnvelope<R>
      emit observation_plan.json
      emit semantic_checkpoint_schema.json (build-active subset)
      emit operational_probe_schema.json
      may write StageCache success entry
      may write StageCache failure memo

build_range_plan_core(RangePlanInputs)
  -> Result<RangePlanCoreSuccess, RangePlanCoreFailure>

RangePlanCoreSuccess :=
  {
    product: RangePlanCoreProduct,
    range_plan_body: RangePlanReportBody,
    range_cert_body: RangeCertBody,
  }

RangePlanCoreFailure :=
  {
    range_plan_body: RangePlanReportBody,
    range_cert_body: Option[RangeCertBody],
    diagnostics: NonEmptyList[ValidationDiagnostic],
  }

run_stage5(RangePlanInputs, env)
  = build_range_plan_core(...) then
    (on success or failure):
      wrap report bodies in ReportEnvelope<R>
      emit range_plan.json
      emit certs/range.cert.json
      may write StageCache success entry
      may write StageCache failure memo
```

Cores never mutate `GbInferIR`, `QuantGraph`,
`ResolvedCompilePolicy`, `SemanticCheckpointSchema`, or any earlier
report. Drivers are the only IO surface.

### 2.3 Storage-freeness

F-B6 and F-B7 inherit F-B5's storage-freeness rule (F-B3/F-B5 §2.3):
neither stage emits a `TileSize`, `BufferAddress`, `AccumulatorWidth`,
`StorageClass`, `LifetimeClass`, `Materialization`, `AliasClassId`,
`PageId`, `CommitGroupId`, `RamRegion`, `RomRegion`, `SramRegion`,
`ConcreteByteOffset`, `ConcreteRomBank`, `KernelResidency`,
`SchedSlice`, `ResourceVector`, or `FrameBudget`.

`ReductionPlan::ChunkedI16 { chunk_len: u16 }` and
`ReductionPlan::RenormLoop { tile_len: u16 }` are NOT storage
decisions: `chunk_len` is the **logical reduction tile** (number of
multiply-accumulate terms before a partial sum is renormalized to
i16) and `tile_len` is the **logical renormalization stride** (number
of partial-sum-i16 terms before a renorm step). Neither commits a
buffer address, byte offset, or accumulator scratch arena; F-B8
(`StoragePlan`) reads `chunk_len` and `tile_len` and decides scratch
materialization separately. F-B13 (`GbSchedIR`) reads them and
chooses physical tile shape.

```text
F-B7-StorageFree:
  No RangePlan field declares a buffer address, page id, arena id,
  storage class, lifetime class, accumulator scratch byte range, or
  ROM bank. chunk_len and tile_len are LOGICAL reduction structure,
  not storage.
```

```text
F-B6-StorageFree:
  No ObservationPlan field declares a buffer address, trace ring
  byte range, or storage class for probes/metrics. Probe attachment
  is by typed reference (NodeId / ValueId / EffectId / SemanticAnchor)
  only.
```

### 2.4 Effect-awareness

Both stages inherit F-B5's effect-class set (F-B3/F-B5 §2.4):
`SequenceState(StateSlotId) | Rng(RngSlot) | FaultBoundary`. The
v1 emit set is `Rng { slot: RngSlot::Decode }` only; `SequenceState { .. }` and
`FaultBoundary` are reserved-not-emitted in v1.

F-B6 may attach an `OperationalProbe` to an `EffectId` only when
that effect class is **emitted in v1**. Attaching a probe to a
reserved-not-emitted class is rejected:

```text
F-B6-EffectV1:
  An OperationalProbe whose source references EffectClass C is
  legal iff:
    C ∈ { Rng { slot: RngSlot::Decode } }
  Otherwise:
    diagnostic is selected by the following precedence:
      C = SequenceState { .. }  => OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED
      C = FaultBoundary     => OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED
      otherwise             => OBSERVATION-RESERVED-EFFECT-PROBE.

F-B6-SequenceStateProbeReserved:
  An OperationalProbe targeting any SequenceState { .. } EffectId
  is rejected with
  OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED.

F-B6-FaultBoundaryProbeReserved:
  An OperationalProbe targeting a FaultBoundary EffectId is
  rejected with OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED.
```

A probe may also attach to a `ValueId` or a `NodeId` (preferred —
attachment by anchor), and this is independent of effect class. The
effect-class restriction applies only when the probe explicitly
targets an `EffectId`.

### 2.5 Single-token convention

Both stages inherit F-B5's single-token convention. F-B6 attaches
checkpoints, probes, and metrics for one IR pass. Multi-token
aggregation is at runtime / benchmark time. F-B7 certifies a
reduction over one IR pass.

`SemanticCheckpointId::PostEmbedding { layer }`,
`PostRouter { layer }`,
`PostExpertDowncast { layer, expert }`, `PostLogits`, and
`PostDecode` are the v1 ids
(per `planv0.md` line 2280). F-B6 attaches each id to the unique
`GbNode` whose canonical anchor matches the id's canonical model
path — the attachment is unambiguous per single-token convention.

```text
F-B6-CheckpointAttachmentTotal:
  ∀ semantic checkpoint id ∈ ObservationPlan.semantic.
    exists exactly one (NodeId, SemanticAnchor) pair to which the
    checkpoint id attaches.
  ∀ NodeId attached, the Node exists in g.nodes and its canonical
  provenance tuple matches the checkpoint id's canonical model
  path.
```

### 2.6 No checkpoint creation, no probe creation, no metric creation

F-B6 NEVER mints new `SemanticCheckpointId`, `TraceProbeId`, or
`MetricId` values. The following invariants are absolute:

```text
F-B6-NoSemanticCreation:
  ObservationPlan.semantic[*].checkpoint ∈
    SemanticCheckpointSchema.checkpoints[*].id

F-B6-NoProbeCreation:
  ObservationPlan.probes[*].probe_id ∈
    gbf-policy::trace::PROBE_REGISTRY

F-B6-NoMetricCreation:
  ObservationPlan.metrics[*].metric ∈
    gbf-policy::metrics::METRIC_REGISTRY
```

A selected id outside the registry is `OBSERVATION-PROBE-ID-UNKNOWN`,
`OBSERVATION-METRIC-ID-UNKNOWN`, or
`OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA` (Hard).

Amends planv0: `planv0.md` line 1620 says F-B6 "consumes the exported
SemanticCheckpointSchema" and "derives optional operational/debug
probes (TraceProbeId)s from the active build profile." This RFC
narrows "derives" to "selects from a pre-existing registry": F-B6
NEVER creates a new probe id. This narrowing matches the existing
F-B2/F-B4 invariant that input registries are sealed at validation
time.

### 2.7 ProbeBudgetClass governance

F-B6 honors `CompileKnobs::observation::optional_probe_floor` as a
monotone class floor. The class order is:

```text
Required > Important > Diagnostic > BestEffort
```

(`Required` is the highest; `BestEffort` is the lowest.)

A probe whose declared `budget_class < optional_probe_floor` is
dropped silently — it never appears in `ObservationPlan.probes`.

Additionally, F-B6 honors `CompileKnobs::observation::trace_demotion`
as a monotone drop policy applied **after** the floor:

```text
trace_demotion = None
  → no further drops (every probe at or above the floor stays in).

trace_demotion = DropBestEffort
  → in addition to the floor, drop every probe whose
    budget_class = BestEffort.

trace_demotion = DropDiagnosticAndBestEffort
  → drop every probe whose
    budget_class ∈ {Diagnostic, BestEffort}.

trace_demotion = RequiredOnly
  → drop every probe whose budget_class != Required.
```

The combined effect is: a probe survives iff it is at or above the
floor AND not in the trace-demotion drop set.

`ObservationPolicy.checkpoints` (the workload-side selection) and
`CheckpointSelection` (per `planv0.md` line 798) act as a *required*
intersection: F-B6 emits the union of (artifact-mandatory checkpoints)
and (workload-required checkpoints) intersected with (build-feasible
checkpoints) — see §8.4.

```text
F-B6-ProbeBudgetGovernance:
  ∀ probe_id ∈ CompileKnobs.observation.disabled_optional_probes.
    PROBE_REGISTRY[probe_id].budget_class != Required.
  If a Required probe id appears in disabled_optional_probes,
  Stage 4 fails with OBSERVATION-REQUIRED-PROBE-DISABLED.

  ∀ probe p ∈ ObservationPlan.probes.
    p.budget_class >= CompileKnobs.observation.optional_probe_floor
    ∧ p.budget_class ∉ trace_demotion_drop_set(
        CompileKnobs.observation.trace_demotion)
    ∧ p.probe_id ∉ CompileKnobs.observation.disabled_optional_probes.

F-B6-ObservationBudgetCapTotal:
  ∀ ProbeBudgetClass c ∈ {Required, Important, Diagnostic, BestEffort}.
    if profile_budget_cap(c) = Some(cap):
      sum_over_probes(p.weight | p.budget_class == c)
      + sum_over_metrics(metric_weight(m) | m.budget_class == c)
      ≤ cap
    else:
      no cap is enforced for c
  where profile_budget_cap is fixed in the profile spec (each
  CompileProfileSpec ships a per-class cap).

metric_weight(m) :=
  METRIC_REGISTRY[m.metric].weight
```

`profile_budget_cap(Required) = unlimited` in v1. Required probes
are never capped in v1; `required_max: None` records that explicit
choice. Future profiles may set `required_max: Some(n)` only by
amending this RFC.

### 2.8 Active probes alter trace shape but NOT semantic comparison contracts

This is a load-bearing observability rule lifted from `planv0.md`
line 2618: `ObservabilityMode::Invariant` means probes must
preserve schedule/layout decisions within declared tolerances; the
compiler must prove that claim with a paired-build comparison. In
this chunk, we **inherit** that rule but do NOT prove it: the
paired-build comparison is owned by F-B14
(`ScheduleCostAnalysis`'s `PerturbationSummary`) and finalized by
the `ObservabilityCertificate` (per `planv0.md` line 2629).

What this chunk does enforce:

```text
F-B6-NoSemanticDriftFromProbes:
  Active OperationalProbes do NOT add value-DAG edges to GbInferIR.
  They reference (NodeId, ValueId, EffectId, SemanticAnchor)
  IMMUTABLY. The semantic comparison contract — what
  ArtifactOracle compares at SemanticCheckpointId boundaries — is
  determined by ObservationPlan.semantic only, NOT by
  ObservationPlan.probes.

F-B6-SemanticCheckpointSetStability:
  The selected semantic checkpoint attachment set
    { (checkpoint, anchor, source) | checkpoint ∈ ObservationPlan.semantic }
  is a function of
    (SemanticCheckpointSchema, WorkloadObservationProjection,
     GbInferIR anchors/topology)
  and does NOT depend on optional_probe_floor, trace_demotion,
  disabled_optional_probes, or active operational probes.

  ObservationEncoding may depend on compare_domain and determinism_class
  per §8.6; therefore the full `ObservationPlan.semantic` record is not
  determinism-independent in v1.
```

In other words: Bringup vs. Default vs. Trace builds may differ in
**which operational probes are emitted**, but they MUST agree on
**which semantic checkpoints are honored**, provided they share the
same artifact, workload observation policy, and `GbInferIR` topology.
F-B6 enforces this by construction: `semantic` is built from
`(SemanticCheckpointSchema, WorkloadObservationProjection,
GbInferIR anchors/topology)` only; `probes` and `metrics` are built
from `(probe_registry, metric_registry, CompileKnobs::observation,
CompileProfileSpec)` only.

### 2.9 Reduction-plan choice as proof, not heuristic

F-B7 chooses a `ReductionPlan` per reduction site. The choice is
governed by:

* the per-site or layer-or-global `reduction_ceiling`;
* the per-site projected accumulator domain (from F-B4's
  `AccumulatorBound` and F-B5's typed `ValueFormat::ExactAccumulator`
  inputs);
* the determinism class (from `QuantGraph.identity.determinism`).

The choice is **always** the smallest (cheapest) plan whose
`AccumulatorCertificate` verifies:

```text
choose_plan(site, ceiling, facts, caps, determinism) :=
  let families = filter_by_ceiling(
    [SingleI16Family, ChunkedI16Family, RenormLoopFamily], ceiling
  ) in
  let candidates =
    families.filter_map(|family|
      canonical_candidate_for_family(family, facts, caps, determinism)
    )
  let proven = filter_by_certificate_verifies(candidates, facts)
  proven.first  -- in canonical family order:
                -- SingleI16 < ChunkedI16 < RenormLoop
```

`canonical_candidate_for_family` materializes a fully-parameterized
candidate (i.e. selects `chunk_len` / `tile_len`) for each plan
family before certificate construction. For `ChunkedI16Family`:

```text
canonical_candidate_for_family(ChunkedI16Family, facts, caps, determinism):
  let per_term = per_term_abs_max(facts)
  let raw_max_safe = floor(i16::MAX / per_term)
  let max_safe = min(raw_max_safe, profile_chunk_max)
  let admissible_lengths =
    { 2^k | 1 ≤ 2^k ≤ max_safe
          ∧ (determinism != BitExact ∨ facts.term_count % 2^k == 0)
          ∧ verifies_for(ChunkedI16 { chunk_len: 2^k }, facts) }
  if admissible_lengths empty:
    return Err(...)
  return ChunkedI16 { chunk_len: canonical_chunk_len(admissible_lengths,
                                                     facts,
                                                     determinism) }

canonical_chunk_len(lengths, facts, determinism):
  -- Prefer the largest safe power-of-two length. Cross-chunk i32
  -- boundedness is computed over the actual term_count, not padded
  -- chunk capacity, so padding does not affect the proof.
  return max(lengths)
```

If `proven` is empty (no plan within the ceiling has a verified
certificate), F-B7 emits `RANGE-NO-PROVEN-PLAN-WITHIN-CEILING`
(Hard) for that site. F-B16 may later raise the ceiling via
`KnobDelta::RaiseReductionCeiling`; this chunk does not.

```text
F-B7-PlanChoiceIsProof:
  ∀ reduction site s ∈ RangePlan.entries.
    s.plan ∈ admissible_under_ceiling(s.ceiling)
    ∧ verifies(s.certificate, s.plan, s.site_facts)
    ∧ ∀ plan p < s.plan in canonical order:
        ¬verifies_for(p, s.site_facts) ∨ p ∉ admissible_under_ceiling(s.ceiling)
```

This is the **proof, not heuristic** rule: the chosen plan is the
unique smallest admissible plan whose certificate verifies.

Amends planv0: `planv0.md` lines 1656–1660 give the `ReductionPlan`
enum but do not pin the choice rule. This RFC pins it as
"smallest-admissible-with-verified-certificate" so F-B16's repair
loop has a stable monotone direction (raise ceiling → add candidates
→ never silently downgrade).

### 2.10 Range proofs are typed certificates, not informal claims

F-B7 emits `certs/range.cert.json` containing one
`AccumulatorCertificate` per reduction site. The certificate is a
typed proof obligation:

```rust
pub enum AccumulatorCertificate {
    /// SingleI16: prove sum_bound ≤ i16::MAX without intermediate
    /// renormalization.
    SingleI16Proof {
        site: ReductionSiteId,
        term_count: u32,
        per_term_abs_max: u64,
        sum_bound: u64,                  // term_count * per_term_abs_max
        bias_abs_max: u64,
        total_abs_max: u64,              // sum_bound + bias_abs_max
        i16_envelope: u64,               // = i16::MAX (32_767)
        slack: u64,                      // i16_envelope - total_abs_max
    },

    /// ChunkedI16: prove every full chunk partial sum ≤ i16::MAX,
    /// and the exact cross-chunk accumulation over the actual term
    /// count fits i32.
    ChunkedI16Proof {
        site: ReductionSiteId,
        chunk_len: u16,
        chunk_count: u32,                // ceil(term_count / chunk_len)
        per_term_abs_max: u64,
        per_chunk_sum_bound: u64,        // chunk_len * per_term_abs_max
        per_chunk_i16_slack: u64,        // i16::MAX - per_chunk_sum_bound
        cross_chunk_sum_bound: u64,      // term_count * per_term_abs_max
        bias_abs_max: u64,
        total_abs_max: u64,              // cross_chunk_sum_bound + bias_abs_max
        i32_envelope: u64,               // = i32::MAX
        slack: u64,                      // i32_envelope - total_abs_max
    },

    /// RenormLoop: prove per-tile renormalization preserves canonical
    /// numeric semantics.
    RenormLoopProof {
        site: ReductionSiteId,
        tile_len: u16,
        tile_count: u32,
        per_term_abs_max: u64,
        per_tile_sum_bound: u64,         // tile_len * per_term_abs_max
        per_tile_i16_slack: u64,         // i16::MAX - per_tile_sum_bound
        renorm: RenormSpec,              // see §9.6
        bias_abs_max: u64,
        total_abs_max: u64,              // post-renorm bound
        slack: u64,
    },

    /// A failed certificate carries the witness ID of the failure
    /// for diagnostic correlation.
    Failed {
        site: ReductionSiteId,
        attempted_plan: ReductionPlan,
        proof_state: AccumulatorProofState,
        witness: AccumulatorFailureWitness,
    },
}

pub enum AccumulatorProofState {
    /// term_count * per_term_abs_max overflowed i16 envelope.
    SumExceedsI16Envelope { sum_bound: u64, envelope: u32 },
    /// chunk_len * per_term_abs_max overflowed i16 envelope.
    PerChunkExceedsI16Envelope { per_chunk_sum_bound: u64, envelope: u32 },
    /// cross-chunk sum overflowed i32 envelope.
    CrossChunkExceedsI32Envelope { cross_chunk_sum_bound: u64, envelope: u64 },
    /// tile_len * per_term_abs_max overflowed i16 envelope.
    PerTileExceedsI16Envelope { per_tile_sum_bound: u64, envelope: u32 },
    /// chunk_len = 0 or tile_len = 0 (degenerate).
    LengthZero { length_field: LengthField },
    /// chosen chunk_len exceeds RangeCapsSpec.profile_chunk_max.
    ChunkLenExceedsProfileMax { chunk_len: u16, profile_chunk_max: u16 },
    /// chosen tile_len is below RangeCapsSpec.profile_tile_min.
    TileLenBelowProfileMin { tile_len: u16, profile_tile_min: u16 },
    /// chosen tile_len exceeds RangeCapsSpec.profile_tile_max.
    TileLenExceedsProfileMax { tile_len: u16, profile_tile_max: u16 },
    /// term_count not divisible by chunk_len and rebalancing is forbidden
    /// (BitExact determinism — see §2.11).
    BitExactRequiresChunkDivides { term_count: u32, chunk_len: u16 },
    /// renorm strategy invalid for declared determinism class.
    DeterminismRequiresEnforcedRenorm,
}

pub enum AccumulatorFailureWitness {
    /// Site-local reason; the diagnostic carries the matching
    /// numeric breakdown.
    BoundCalculation { input_max_abs_q: u32, weight_max_abs_q: u32, term_count: u32, bias: u32 },
    /// Determinism-class clash: the certificate would require
    /// mid-reduction saturation but determinism is BitExact.
    BitExactSaturationForbidden,
}
```

`verifies(cert, plan, site_facts)` is a closed predicate over the
certificate variant (see §9.7). The certificate is independently
re-checkable by `gbf-verify` (F-F1) — the cert JSON contains every
load-bearing scalar, so an independent reference reads the JSON,
plays back the formula, and verifies the inequality.

### 2.11 No silent integer-width expansion under BitExact

Per F-B5 §2.10, `BitExact` requires
`policy.numeric_profile.reduction_order_policy == Enforced` and
forbids mid-reduction saturation except at named numeric boundaries.
F-B7 inherits this rule:

```text
F-B7-BitExactReductionRules:
  q.identity.determinism = BitExact
  ⇒ ∀ reduction site s ∈ RangePlan.entries.
    s.plan ∈ {SingleI16, ChunkedI16 { chunk_len }}
    ∧ if s.plan = ChunkedI16 { chunk_len }:
        term_count(s) % chunk_len == 0
          (no mid-reduction rebalancing — every chunk has exactly
           chunk_len terms; trailing partial chunk is forbidden under
           BitExact because its renormalization step would not
           commute with the reduction-order policy).
    ∧ RenormLoop is not generated under BitExact in v1.
      A future BoundaryWidened or ExactBoundaryRenorm plan may amend
      this RFC if it carries a proof distinct from SingleI16.
```

A chunked plan that would silently truncate, saturate, or reorder under
`BitExact` is rejected with
`RANGE-BITEXACT-MID-REDUCTION-SATURATION-FORBIDDEN` or
`RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES`. A RenormLoop candidate under
`BitExact` is rejected with
`RANGE-BITEXACT-RENORM-LOOP-RESERVED-V1`.

For weaker `DeterminismClass` values (`NumericallyStable`,
`SeedStable`, `DistributionStable`), F-B7 records the class but does
not assert numeric equality. A trailing partial chunk and a renorm
strategy other than `ExactPostBoundary` are admissible under those
classes; the certificate field shape stays the same, but the proof
content is weaker. F-C2 / F-C4 own the relative-class envelope.

### 2.12 No scheduling fusion, no storage decisions, no overlay choices

Both stages inherit F-B5 §2.6's no-scheduling-fusion rule: neither
stage emits a fused op or a fused observation surface. F-B6 emits
one observation entry per checkpoint/probe/metric; F-B7 emits one
plan per reduction site.

Both stages also inherit F-B5 §2.3's storage-freeness; neither stage
emits a storage class, residency choice, overlay decision, or
arena.

```text
F-B6-NoFusion:
  ObservationPlan.semantic[i].checkpoint != ObservationPlan.semantic[j].checkpoint
    for i != j (no duplicates).
  ObservationPlan.probes[i].instance_id != ObservationPlan.probes[j].instance_id
    for i != j (no duplicate probe instances).
  Multiple probe instances MAY share the same TraceProbeId iff they
  attach to different sources and have distinct ProbeInstanceId values.
  ObservationPlan.metrics[i].metric    != ObservationPlan.metrics[j].metric
    for i != j (no duplicates).

F-B7-NoFusion:
  ∀ reduction sites s1, s2 ∈ RangePlan.entries.
    s1.site != s2.site (no duplicates).
  RangePlan.entries.len() == g.reduction_site_count
    (no missing sites; no extra sites).
```

### 2.13 F-B16 RepairPolicy / CompileKnobs is named-only

`RepairPolicy`, `RepairProposal`, `KnobDelta`, and the loop driver
are F-B16's territory. This chunk **reads** the relevant knob
surfaces (`CompileKnobs::observation`, `CompileKnobs::range`,
`KnobLockSet`) but never mutates them and never accepts a
`RepairProposal(_)` provenance value (per F-B2/F-B4 §2.7).

F-B16's specific knob deltas this chunk's schemas leave room for:

* `KnobDelta::DisableOptionalProbes { probes: BTreeSet<TraceProbeId> }`
  populates `CompileKnobOverrides::disabled_optional_probes`. F-B6
  reads that set on each refinement-loop iteration; this chunk
  reads it as an empty set in the non-loop case but pins the schema
  so F-B16 can plug in.
* `KnobDelta::RaiseReductionCeiling { selector: Option<ReductionSelector>, to: ReductionPlanCeiling }`
  populates `CompileKnobOverrides::reduction_ceiling_overrides`.
  F-B7 reads that map on each iteration; this chunk reads it as an
  empty map in the non-loop case but pins the schema so F-B16 can
  plug in.

Until F-B16 lands, the maps are empty and the ceiling values come
from `CompileKnobValues::observation` / `::range` only. The reports
this chunk emits MUST distinguish between "empty by absence" and
"empty by repair" so F-B16 can later re-emit faithfully. We do this
by recording `disabled_optional_probes` and
`reduction_ceiling_overrides` as `BTreeSet`/`BTreeMap` fields with
explicit empty-set semantics — never `null`.

### 2.14 Inheritance from F-B2/F-B4 and F-B3/F-B5

This RFC inherits, **unchanged**, the following from
`F-B2-F-B4-pipeline-entry-validation.md` and
`F-B3-F-B5-canonical-irs.md`. Each item names the precise prior-RFC
section so a future amendment cannot silently weaken what this RFC
depends on:

From F-B2/F-B4:

* `ReportEnvelope<R>` shape and public JSON conventions — F-B2/F-B4 §7.2.
* `Hash256`, `DomainHash(...)`, `SelfHash(report)`, `ZERO_HASH`,
  domain-separated object hash form — F-B2/F-B4 §2.4.
* `CanonicalJson(x)` rule (UTF-8, lex object keys, integers only,
  no NaN/Inf, no unknown fields, explicit enum tags, deterministic
  array ordering where order is not semantically meaningful) —
  F-B2/F-B4 §2.5.
* `null` policy (only for explicit semantic absence; never for
  unknown, unmeasured, or omitted) — F-B2/F-B4 §2.5.
* Envelope laws (`R-Hash`, `R-Outcome-Pass`, `R-Outcome-Fail`,
  `R-FlatEnvelope`, `R-UnknownReject`) — F-B2/F-B4 §7.2.
* `ValidationDiagnostic` shape (`severity`, `origin`, `code`,
  `detail`, `provenance`) — F-B2/F-B4 §7.1.
  `ValidationDiagnosticRecord` is the canonical JSON representation
  of `ValidationDiagnostic` inherited from F-B2/F-B4 §7.1; the Rust
  struct and its JSON-side record share content and differ only in
  presentation.
* Hard-only-this-chunk rule: F-B6/F-B7 reports reject `Soft`
  diagnostics — F-B2/F-B4 §2.2.
* `D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`, `D-Provenance`
  diagnostic laws — F-B2/F-B4 §7.1.
* StageCache key construction rule
  `DomainHash(crate, "StageCacheKey", schema_id, schema_version, canonical_json_bytes)`
  — F-B2/F-B4 §7.8.
* Failure memo rule: failure memos are stored only under exact key
  match — F-B2/F-B4 §2.6.

From F-B3/F-B5:

* Pure-core / driver split — F-B3/F-B5 §2.1.
* `QuantGraph`, `GbInferIR`, `ValueDecl`, `EffectDecl`, `GbNode` —
  F-B3/F-B5 §8.1, §9.1.
* `NodeAnchorMap`, `SemanticAnchor` — F-B3/F-B5 §2.12.
* `ReductionSiteId` — `gbf-policy::diagnostics::ReductionSiteId
  (transparent newtype around String)`; defined alongside the F-B3/F-B5
  surfaces. No separate `ReductionSiteKey` type is landed; cross-stage
  joins use `ReductionSiteId` directly.
* `op_signature` predicate, `ReductionSiteBearing(op, q)` — F-B3/F-B5
  §9.7a.
* `ValueFormat::ExactAccumulator`, `ValueFormat::Quant(QuantFormat)` —
  F-B3/F-B5 §9.7.
* Single-token convention — F-B3/F-B5 §2.5.
* Effect-class set (closed in v1) — F-B3/F-B5 §2.4.
* Determinism class binding — F-B3/F-B5 §2.10, §8.1.
* `quant_graph_self_hash`, `infer_ir_self_hash` (DomainHash-based) —
  F-B3/F-B5 §8.8, §9.1.

This RFC adds the following to that surface:

* Two new `ValidationOrigin` variants:
  `ObservationPlanConstruction` and `RangePlanConstruction`.
* Five new `ReportSchemaId` variants:
  `build_active_semantic_checkpoint_schema.v1` (re-emit),
  `operational_probe_schema.v1`, `observation_plan.v1`,
  `range_plan.v1`, and `range.cert.v1`.
* Two new product types: `ObservationPlan` and `RangePlan`.
* New public enums/types:
  `ObservationSource`, `ObservationEncoding`
  (refining `planv0.md` line 1631), `ProbeSource`, `ProbeLevel`
  (refining `planv0.md` line 1638), `MetricSource`,
  `MetricAggregation` (refining `planv0.md` line 1645),
  `RenormStrategy`, `AccumulatorCertificate`,
  `AccumulatorProofState`, and `AccumulatorFailureWitness`.
* Two new `StageCacheKey` schemas (§11):
  `K4 := ObservationPlanCacheKey`, `K5 := RangePlanCacheKey`.

If a later amendment to F-B2/F-B4 or F-B3/F-B5 changes any of the
inherited surfaces, that amendment must explicitly amend this RFC by
name (see Authority rules, §5).

### 2.15 Schema versioning

Each new schema is versioned independently:

```text
build_active_semantic_checkpoint_schema.v1
operational_probe_schema.v1
observation_plan.v1
range_plan.v1
range.cert.v1
```

Schema bumps follow F-B2/F-B4 §16.2's compatibility rules (any later
RFC that changes shape, canonicalization, or self-hash must amend
this RFC). The artifact-side `SemanticCheckpointSchema` itself is
already versioned by the artifact schema epoch (F-B2 owns that
gate).

### 2.16 Determinism mode binding

`ArtifactCore.numeric_profile.determinism` selects the equality used by
F-B7's certificate proof. F-B6 reports the determinism class verbatim
in `observation_plan.json` for downstream gates but does not branch on
it: the build-active checkpoint set is `DeterminismClass`-independent
(see §2.8 — semantic comparison contracts must not change with
observability mode and likewise must not change with determinism class
for fixed artifact + workload).

F-B7 reads `g.identity.determinism` directly (F-B5 already pinned that
the IR carries the class) and applies the §2.11 rule. The certificate
records the class; future schema bumps may add per-class verification
nuance, but v1 keeps the proof obligation strictly bit-exact under
`BitExact`.

### 2.17 Joins are by typed key, not by string

F-B7's join key from `GbNode.reduction_site` to `RangePlan.entries` is
the `ReductionSiteId` minted by F-B4. The join is a typed lookup, not
a name match:

```text
F-B7-Join:
  ∀ node n ∈ g.nodes where n.reduction_site = Some(rsid).
    exactly one entry e ∈ RangePlan.entries.
      e.site = rsid

  ∀ entry e ∈ RangePlan.entries.
    exactly one node n ∈ g.nodes.
      n.reduction_site = Some(e.site)
```

Likewise F-B6's join from `SemanticAnchor` to a checkpoint id is a
typed lookup against the canonical-anchor structure declared by
F-B5. `SemanticAnchor` itself is `{ anchor_id: Hash256 }`; the
`CanonicalProvenanceTuple { op_tag, layer, expert,
expert_weight_slot, norm_site, state_slot, residual_site,
occurrence_index }` that F-B5 hashes into the anchor is carried
alongside the IR via the same provenance surface (it is not a
field on `SemanticAnchor`). F-B6 matches against the checkpoint
id's canonical model path through a closed `tuple → CheckpointId`
function (§8.5 declares the function).

### 2.18 No "quick fix" upgrade in this chunk

If `ObservationPlan` construction would succeed only by silently
filling in defaults (e.g. defaulting an `ObservationEncoding` from
absent metadata), F-B6 fails. Every selected checkpoint, probe, and
metric must come from a hash-bound input or fail loudly. Same shift-
left discipline as F-B2/F-B4 §2.7.

If `RangePlan` construction would succeed only by silently truncating
the per-term abs max (e.g. by unsigned wrap), F-B7 fails with
`RANGE-INTEGER-OVERFLOW-DURING-PROOF`. Every certificate computation
uses checked `u128` internally; the certificate JSON rejects values
that cannot be represented in their declared width.

## 3. Glossary additions

This chunk introduces or pins the following terms beyond the
F-B2/F-B4 and F-B3/F-B5 glossary inheritance.

| Term                             | Definition                                                                                       |
|----------------------------------|--------------------------------------------------------------------------------------------------|
| Build-active checkpoint set      | The subset of `SemanticCheckpointSchema.checkpoints` honored by THIS build; it may be equal to the artifact's full checkpoint set. |
| Build-active probe set           | The set of `OperationalProbe` entries selected for THIS build.                                    |
| Build-active metric set          | The set of `MetricProbe` entries selected for THIS build.                                         |
| Observation contract             | The (semantic, probes, metrics) tuple emitted by `ObservationPlan`. Stable across schedule changes for a fixed artifact + workload + profile. |
| Probe registry                   | The closed registry of `TraceProbeId` values in `gbf-policy::trace::PROBE_REGISTRY`. Sealed at validation time. |
| Metric registry                  | The closed registry of `MetricId` values in `gbf-policy::metrics::METRIC_REGISTRY`. Sealed at validation time. |
| ProbeBudgetClass floor           | `CompileKnobs::observation::optional_probe_floor`. Probes below the floor are dropped silently. |
| TraceDemotionLevel drop set      | The set of `ProbeBudgetClass` values dropped by the active `TraceDemotionLevel`.                 |
| Reduction site                   | A `ReductionSiteId` (transparent `String` newtype in `gbf-policy::diagnostics`); the canonical correlation between Stage 2 budget and Stage 3 IR. |
| Reduction plan                   | One of `SingleI16 | ChunkedI16 { chunk_len } | RenormLoop { tile_len }`. Logical reduction structure, not storage. |
| ReductionPlanCeiling             | `CompileKnobs::range::reduction_ceiling`. Closed enum: `SingleI16Only | AllowChunkedI16 | AllowRenormLoop`. |
| Accumulator certificate          | Typed proof that a chosen `ReductionPlan`'s intermediate accumulator domain stays within its declared implementation envelope. |
| Accumulator proof state          | Closed enum recording the proof's resolved state (verified, sum exceeds envelope, length zero, etc.). |
| Accumulator failure witness      | Closed enum recording the load-bearing scalars that justify a failed certificate. |
| Renormalization strategy         | Closed enum naming when and how `RenormLoop` partial sums are renormalized to i16. |
| Per-term abs max                 | The largest abs(input × weight) for one term of a reduction. Derived from `ReductionSiteProjection`. |
| Sum bound                        | `term_count * per_term_abs_max`. Upper bound on un-biased reduction sum. |
| Total abs max                    | `sum_bound + bias_abs_max`. Upper bound on biased reduction. |
| i16 envelope                     | `i16::MAX = 32_767`. The bound `SingleI16` and per-chunk / per-tile partial sums must respect. |
| i32 envelope                     | `i32::MAX = 2_147_483_647`. The bound cross-chunk renormalized sums must respect. |
| Slack                            | `envelope - bound`. Non-negative for a verified certificate. |
| Named numeric boundary           | The set `{ residual combine, classify logit, FFN activation output, final clamp }` per F-B3/F-B5 §2.10. Only places where saturation is allowed under `BitExact`. |
| Trace event shape                | The wire shape of an emitted trace event: `(probe_id, level, payload_layout)`. F-B6 declares it; runtime emits it. |

## 4. Core notation

This RFC inherits §1 of F-B2/F-B4 (Hash256, Outcome, Severity, Stage,
ReportSchema, Result, Option, NonEmptyList, SortedBy, DomainHash,
SelfHash, CanonicalJson, ZERO_HASH, null policy) and §4 of F-B3/F-B5
(`QG`, `IIR`, `ValueId`, `EffectId`, `NodeId`, `SemanticAnchor`,
`ReductionSiteId`, op-signature predicate). Additions:

```text
Stage :=
  Stage0 | Stage0_5 | Stage1 | Stage2 | Stage3 | Stage4 | Stage5  -- Stage4, Stage5 added

ReportSchemaId :=
  artifact_validation.v1
  | policy_resolution.v1
  | static_budget.v1
  | quant_graph.v1
  | infer_ir.v1
  | build_active_semantic_checkpoint_schema.v1   -- new (build-active re-emit)
  | operational_probe_schema.v1     -- new
  | observation_plan.v1             -- new
  | range_plan.v1                   -- new
  | range.cert.v1                   -- new

ValidationOrigin (extension) :=
  ...existing F-B2/F-B4 + F-B3/F-B5 origins...
  | ObservationPlanConstruction
  | RangePlanConstruction
```

Abbreviations:

```text
OP  := ObservationPlan
RP  := RangePlan
SCS := SemanticCheckpointSchema
OPS := OperationalProbeSchema
```

CanonicalMap<K, V> JSON encoding:

```text
Any map whose key type is not a primitive canonical string key MUST
serialize as a sorted list of entries:

  [{ "key": K, "value": V }, ...]

Sorting is by CanonicalJson(key), bytewise lexicographic order.
Deserialization rejects duplicate keys after canonicalization.

This rule applies to every public `BTreeMap` in this RFC, including:
  - BTreeMap<SemanticCheckpointId, ...>
  - BTreeMap<ProbeInstanceId, ...>
  - BTreeMap<MetricId, ...>
  - BTreeMap<ReductionSelector, ...>
  - BTreeMap<ReductionSiteId, ...>
  - BTreeMap<ReductionPlanCeiling, ...>
  - BTreeMap<ReductionCeilingProvenanceTag, ...>
```

## 5. Authority rules

```text
Scope(F-B6/F-B7) =
  {
    Stage4,
    Stage5,
    ObservationPlan,
    RangePlan,
    build_active_semantic_checkpoint_schema.v1   (re-emit),
    operational_probe_schema.v1,
    observation_plan.v1,
    range_plan.v1,
    range.cert.v1,
    StageCache keys K4 and K5,
    AccumulatorCertificate,
    AccumulatorProofState,
    AccumulatorFailureWitness,
    RenormStrategy,
    the closed selection rule for ObservationPlan
      (semantic =
        (Mandatory(scs) ∩ build_feasible(g))
        ∪ (WorkloadRequired ∩ SchemaIds(scs) ∩ build_feasible(g))
        ∪ (WorkloadOptional ∩ Optional(scs) ∩ build_feasible(g))),
    the closed plan-choice rule
      (smallest-admissible-with-verified-certificate),
    the closed effect-probe-allowed set (Rng { slot: RngSlot::Decode } only in v1)
  }

Rule Authority:
  ∀ behavior b.
    b ∈ Scope(F-B6/F-B7) ∧ RFC specifies b
    ⇒ SourceOfTruth(b) = RFC

Rule PlanContext:
  ∀ behavior b.
    b ∈ Scope(F-B6/F-B7) ∧ RFC silent on b
    ⇒ planv0 may inform implementation but is not an acceptance gate

Rule Inheritance(F-B2/F-B4):
  ∀ behavior b.
    b ∈ Scope(F-B2/F-B4) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B2/F-B4 RFC

Rule Inheritance(F-B3/F-B5):
  ∀ behavior b.
    b ∈ Scope(F-B3/F-B5) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B3/F-B5 RFC

Rule Amendment:
  LaterRFC changes any of:
    public ObservationPlan type
    public RangePlan type
    public AccumulatorCertificate type or any of its proof-state /
      witness sub-enums
    report shape (semantic_checkpoint_schema.v1 re-emit,
                  operational_probe_schema.v1, observation_plan.v1,
                  range_plan.v1, range.cert.v1)
    cache key (K4, K5)
    diagnostic code introduced here (OBSERVATION-* or RANGE-*)
    selection rule for ObservationPlan
    plan-choice rule for RangePlan
    effect-probe-allowed set
  ⇒ LaterRFC must explicitly amend this RFC

Rule DivergenceLedger:
  RFC intentionally diverges from planv0
  ⇒ nearest relevant section must contain `Amends planv0`
```

## 6. Pipeline state machine

Extending the F-B2/F-B4 + F-B3/F-B5 state machine:

```text
BuildProductState :=
  Imported(inputs)
  | Validated(validation_product)
  | PolicyResolved(policy_product)
  | QuantGraphReady(policy_product, quant_graph_product)
  | BudgetPassed(quant_graph_product, static_budget_report)
  | InferIrReady(budget_product, infer_ir_product)
  | ObservationPlanReady(infer_ir_product, observation_plan_product)    -- new
  | RangePlanReady(infer_ir_product, range_plan_product)                -- new
  | PlanningReady(infer_ir_product,
                  observation_plan_product,
                  range_plan_product)                                  -- new
  | Halted(stage, reports, diagnostics)

Stage 4 and Stage 5 readiness states are not exclusive linear states;
they are product nodes in the build DAG. A build may contain
`RangePlanReady` even when Stage 4 failed, but it may not advance to
Stage 6 unless `PlanningReady` exists.
```

Transitions (extending F-B2/F-B4 + F-B3/F-B5):

```text
T4 build_observation_plan:
  InferIrReady(b, g)
    -- build_observation_plan(g, p, scs, op_policy) = Ok(o) -->
  ObservationPlanReady(g, o)

  InferIrReady(b, g)
    -- build_observation_plan(...) = Err(e) -->
  Halted(Stage4, e.reports, e.diagnostics)

T5 build_range_plan:
  InferIrReady(b, g)
    -- build_range_plan(g, q, range_policy_proj) = Ok(r) -->
  RangePlanReady(g, r)

  InferIrReady(b, g)
    -- build_range_plan(...) = Err(e) -->
  Halted(Stage5, e.reports, e.diagnostics)

T4+T5 join:
  ObservationPlanReady(g, o) ∧ RangePlanReady(g, r)
    -->
  PlanningReady(g, o, r)
```

Pipeline invariants (additions to F-B3/F-B5 §6):

```text
I-Pipeline-17:
  Stage4 may run only after Stage3 Passed.

I-Pipeline-18:
  Stage5 may run only after Stage3 Passed (NOT after Stage4).
  -- F-B7 does not consume ObservationPlan; it consumes only
  -- GbInferIR + RangePolicyProjection. F-B7's pipeline-order
  -- placement (Stage 5, after Stage 4) is for cache-key locality
  -- and report-emission ordering, NOT a data dependency.
  -- This RFC therefore allows Stage 4 and Stage 5 to run in
  -- parallel in the implementation, with the canonical pipeline
  -- order (4 then 5) preserved for reporting.

I-Pipeline-19:
  If Stage4 fails, Stage5 may still run (it does not depend on
  Stage4). However, the build cannot close with a failed Stage4.

I-Pipeline-19a:
  The build may proceed to Stage6 only from PlanningReady(g, o, r),
  never from ObservationPlanReady alone and never from RangePlanReady
  alone.

I-Pipeline-20:
  If Stage5 fails, the build cannot close.

I-Pipeline-21:
  Stage4 and Stage5 are passive in the IR-product sense:
    They produce their own products but never mutate
    GbInferIR, QuantGraph, ResolvedCompilePolicy,
    RuntimeChromeBudget, or any earlier stage's report.

I-Pipeline-22:
  observation_plan.report_self_hash is immutable after Stage4
  emits it.
  semantic_checkpoint_schema.report_self_hash (re-emit) is
  immutable after Stage4 emits it.
  operational_probe_schema.report_self_hash is immutable after
  Stage4 emits it.
  range_plan.report_self_hash is immutable after Stage5 emits it.
  range.cert.report_self_hash is immutable after Stage5 emits it.

I-Pipeline-23:
  Every emitted report must satisfy
  SelfHash(report) = report.report_self_hash.

I-Pipeline-24:
  Stage4's products do not change between two consecutive
  regenerations on the same (g, scs, op_policy_projection,
  observation_knobs) hashes.
  Stage5's products do not change between two consecutive
  regenerations on the same (g, range_policy_projection,
  range_knobs) hashes.

I-Pipeline-25:
  ObservationPlan's semantic checkpoint attachment set is independent of
  ObservabilityMode (Invariant vs Flexible), optional_probe_floor,
  trace_demotion, and disabled_optional_probes.

I-Pipeline-26:
  RangePlan's chosen plan per site is independent of any field
  outside RangePolicyProjection. In particular, it does not
  read ObservationPlan and is not affected by probe selection.

I-Pipeline-27:
  `range_plan.json` and `certs/range.cert.json` do not contain
  `observation_plan_self_hash`. Cross-product linkage is recorded
  only by the later build-level manifest after `PlanningReady`.
```

## 7. Report envelope (inherited)

All five new reports — `semantic_checkpoint_schema.json` (re-emit),
`operational_probe_schema.json`, `observation_plan.json`,
`range_plan.json`, and `certs/range.cert.json` — use the
`ReportEnvelope<R>` shape from F-B2/F-B4 §7.2 unchanged.
In particular, the envelope-level `outcome` is always
`ReportOutcome::{Passed, Failed}`. Certificate-specific verification
status lives inside the certificate body, not in the inherited
envelope field:

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
`R-HardOnly-ThisChunk`) are inherited unchanged. Specifically:
F-B6/F-B7 reports reject `Soft` diagnostics in this chunk.

`R-NoPartialProduct` is restated for the new products:

```text
R-NoPartialIR-OP:
  Failed observation_plan report
  ⇒ body.result = None

R-NoPartialIR-RP:
  Failed range_plan report
  ⇒ body.result = None

R-NoPartialProduct-Cert:
  Failed range.cert report
  ⇒ no RangePlanProduct is exposed as a successful Stage 5 product.
  The certificate body MAY still contain per-site
  AccumulatorCertificate::Failed { ... } entries. Those failed
  certificates are diagnostic evidence, not a partial successful
  RangePlan product.
```

Cross-report invariants:

```text
R-OPSchemaConsistency:
  observation_plan.json, semantic_checkpoint_schema.json (re-emit),
  and operational_probe_schema.json all share the same
  observation_plan_self_hash recorded in their identity sections.
  A driver emits all three together or none.

R-RPCertConsistency:
  range_plan.json and range.cert.json share the same
  range_plan_self_hash. The cert's per-site entries reference
  ReductionSiteIds that all appear in range_plan.json.entries[*].site.

R-NoReportHashCycles:
  Cross-report references may point to product self-hashes and to
  already-emitted sibling report_self_hash values, but no report_self_hash
  computation may depend on its own report_self_hash, directly or
  indirectly. In particular:
    - semantic_checkpoint_schema.json and operational_probe_schema.json
      may record observation_plan_self_hash, which is the product hash,
      not observation_plan.json.report_self_hash.
    - range.cert.json may record range_plan_self_hash, which is the
      product hash, not range_plan.json.report_self_hash.
    - observation_plan.json may record sibling report_self_hash values
      only after those sibling envelopes have been constructed.
    - range_plan.json may record range_cert_report_self_hash only after
      the cert envelope has been constructed.
```

## 8. Stage 4 contract: `ObservationPlan`

### 8.1 Type-level contract

```text
ObservationPlanInputs :=
  {
    infer_ir_product: GbInferIRProduct,
    infer_ir_self_hash: Hash256,
    quant_graph_self_hash: Hash256,            -- transitive
    semantic_checkpoint_schema: SemanticCheckpointSchema,
    semantic_checkpoint_schema_hash: Hash256,  -- computed over supplied schema
    artifact_declared_semantic_checkpoint_schema_hash: Hash256,
    probe_registry: ProbeRegistrySnapshot,
    probe_registry_hash: Hash256,
    metric_registry: MetricRegistrySnapshot,
    metric_registry_hash: Hash256,
    trace_event_layout_registry: TraceEventLayoutRegistrySnapshot,
    trace_event_layout_registry_hash: Hash256,
    op_policy_projection: ObservationPolicyProjection,
    audit_parents: ObservationPlanAuditParents,
  }

ObservationPolicyProjection :=
  {
    profile_id: CompileProfileId,
    profile_observation_caps: ObservationProfileCaps,
    determinism_class: DeterminismClass,
    observability_mode: ObservabilityMode,
    trace_budget: TraceBudget,
    trace_demotion: TraceDemotionLevel,
    optional_probe_floor: ProbeBudgetClass,
    workload_observation: WorkloadObservationProjection,
    disabled_optional_probes: BTreeSet<TraceProbeId>,   -- empty pre-F-B16
  }

WorkloadObservationProjection :=
  {
    workload_id: WorkloadId,
    checkpoints: CheckpointSelection,                   -- per planv0 line 798
    trace_level: TraceLevel,
    compare_domain: CompareDomain,
    determinism_requirement: DeterminismClass,
  }

CompareDomain is inherited from `ObservationPolicy` if already
defined there. If not, this RFC adds the closed v1 enum:

pub enum CompareDomain {
    CanonicalValue,
    TokenIdOnly,
    ExpertIdOnly,
    EnvelopeQ8_8,
    EnvelopeQ16_16,
}

ObservationProfileCaps :=
  {
    /// Per-class budget caps. Required is unlimited (None);
    /// other classes carry an explicit u16 weight cap.
    required_max:   Option<u16>,                         -- always None in v1
    important_max:  u16,
    diagnostic_max: u16,
    best_effort_max: u16,
  }

LockedObservationKnobs :=
  {
    trace_demotion_locked: bool,
    optional_probe_floor_locked: bool,
    probe_selection_locked: bool,
  }

ObservationPlanAuditParents :=
  {
    policy_resolution_self_hash: Hash256,
    compile_request_hash: Hash256,
    static_budget_self_hash: Hash256,
    artifact_aux_hash: Hash256,
    locked_observation_knobs: LockedObservationKnobs,
  }

observation_policy_projection_hash :=
  DomainHash("gbf-codegen", "ObservationPolicyProjection",
    "observation_plan.v1", CanonicalJson(ObservationPolicyProjection))

-- The projection is the load-bearing slice of ResolvedCompilePolicy
-- for Stage 4. Audit parents are recorded for traceability; they
-- do not invalidate K4.
--
-- `artifact_aux_hash` is audit-only. The sidecar content that affects
-- Stage 4 is bound by `semantic_checkpoint_schema_hash`.

ObservationPlanCoreProduct :=
  {
    observation_plan: ObservationPlan,
    observation_plan_self_hash: Hash256,
    build_active_checkpoint_schema: BuildActiveCheckpointSchema,
    build_active_checkpoint_schema_hash: Hash256,
    operational_probe_schema: OperationalProbeSchema,
    operational_probe_schema_hash: Hash256,
  }

ObservationPlanStageOutput :=
  {
    product: ObservationPlanCoreProduct,
    report: ReportEnvelope<ObservationPlanReportBody>,
    sc_re_emit_report: ReportEnvelope<SemanticCheckpointSchemaReEmitBody>,
    operational_probe_report: ReportEnvelope<OperationalProbeSchemaBody>,
  }

ObservationPlanProduct := ObservationPlanCoreProduct
  -- Legacy alias used by §13.x stage handshakes. The canonical
  -- StageCache value is `ObservationPlanCoreProduct`; the report
  -- envelopes live in `ObservationPlanStageOutput` so a cache hit
  -- can replay the byte-identical core product while the driver
  -- wraps fresh report envelopes for the current build's audit
  -- parents.

observation_plan_self_hash :=
  DomainHash("gbf-codegen", "ObservationPlan", "observation_plan.v1",
    CanonicalJson(observation_plan))

build_active_checkpoint_schema_hash :=
  DomainHash("gbf-codegen", "BuildActiveCheckpointSchema",
    "build_active_semantic_checkpoint_schema.v1",
    CanonicalJson(build_active_checkpoint_schema))

operational_probe_schema_hash :=
  DomainHash("gbf-codegen", "OperationalProbeSchema",
    "operational_probe_schema.v1",
    CanonicalJson(operational_probe_schema))

ObservationPlanStageFailure :=
  {
    report: ReportEnvelope<ObservationPlanReportBody>,
    sc_re_emit_report: Option[ReportEnvelope<SemanticCheckpointSchemaReEmitBody>],
    operational_probe_report: Option[ReportEnvelope<OperationalProbeSchemaBody>],
    diagnostics: NonEmptyList[ValidationDiagnostic],
  }

-- The failure shape pairs the primary report with whichever
-- ancillary report bodies were already constructed before failure.
-- Reports that were not constructed are None.
```

The public `ObservationPlan` type:

```rust
pub struct ObservationPlan {
    pub identity: ObservationPlanIdentity,
    pub semantic: Vec<SemanticObservation>,
    pub probes: Vec<OperationalProbe>,
    pub metrics: Vec<MetricProbe>,
    pub anchor_table: AnchorAttachmentTable,
    pub provenance: ObservationProvenance,
    pub trace_budget_projection: TraceBudgetProjection,
}

pub struct TraceBudgetProjection {
    pub projected_max_events_per_slice: u32,
    pub projected_max_bytes_per_frame: u32,
    pub fits_declared_budget: bool,
}

pub struct ObservationPlanIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub semantic_checkpoint_schema_hash: Hash256,
    pub observation_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
    pub observability_mode: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub workload_id: WorkloadId,
    pub probe_registry_hash: Hash256,
    pub metric_registry_hash: Hash256,
    pub trace_event_layout_registry_hash: Hash256,
}

// probe_registry_hash :=
//   DomainHash("gbf-policy", "ProbeRegistry", "operational_probe_schema.v1",
//              CanonicalJson(i.probe_registry))
//
// metric_registry_hash :=
//   DomainHash("gbf-policy", "MetricRegistry", "operational_probe_schema.v1",
//              CanonicalJson(i.metric_registry))
//
// trace_event_layout_registry_hash :=
//   DomainHash("gbf-policy", "TraceEventLayoutRegistry",
//              "operational_probe_schema.v1",
//              CanonicalJson(i.trace_event_layout_registry))

pub struct SemanticObservation {
    pub checkpoint: SemanticCheckpointId,
    pub source: ObservationSource,
    pub encoding: ObservationEncoding,
    pub anchor: SemanticAnchor,                   // bound to a NodeId
    pub artifact_role: SemanticCheckpointRole,    // Mandatory | Optional
}

pub enum ObservationSource {
    /// The activation produced by the named node's primary value
    /// output (e.g. PostEmbedding consumes the unique Embedding
    /// node's EmbeddingOutput value).
    NodeOutput { node: NodeId, value: ValueId },
    /// The router decision and gate weight for a layer (PostRouter).
    RouterDecision { node: NodeId, decision: ValueId, weight: ValueId },
    /// The post-FfnDown candidate for a (layer, expert) (PostExpertDowncast).
    ExpertCandidate { node: NodeId, candidate: ValueId,
                      layer: LayerId, expert: ExpertId },
    /// The classify-head logits (PostLogits).
    LogitVector { node: NodeId, value: ValueId },
    /// The decoded token (PostDecode).
    DecodedToken { node: NodeId, value: ValueId },
}

pub enum ObservationEncoding {
    /// Bit-exact dump of the source value at canonical reference
    /// boundary. Used when the workload demands BitExact diffing.
    Canonical,
    /// Argmax token id only (e.g. PostDecode encoding when the
    /// workload's compare_domain is TokenIdOnly).
    TokenId,
    /// Argmax expert id (e.g. PostRouter encoding when the workload
    /// only needs the route, not the gate weight).
    ExpertId,
    /// Quantized representation suitable for envelope-class diffing
    /// (NumericallyStable / SeedStable / DistributionStable).
    QuantizedQ8_8,
    QuantizedQ16_16,
}

pub enum SemanticCheckpointRole {
    Mandatory,    // declared mandatory by SemanticCheckpointSchema
    Optional,     // declared optional but selected for THIS workload
}

pub struct OperationalProbe {
    pub instance_id: ProbeInstanceId,
    pub probe_id: TraceProbeId,
    pub source: ProbeSource,
    pub level: ProbeLevel,
    pub budget_class: ProbeBudgetClass,
    pub event_shape: TraceEventShape,
    pub frequency_bound: TraceFrequencyBound,
    pub weight: u16,
}

pub struct ProbeInstanceId {
    pub probe_id: TraceProbeId,
    pub source_fingerprint: Hash256,
}

// probe_instance_source_fingerprint(probe_id, source) :=
//   DomainHash("gbf-codegen", "ProbeInstanceSource",
//              "operational_probe_schema.v1",
//              CanonicalJson({ probe_id, source }))

pub enum TraceFrequencyBound {
    PerPass { max_events: u32 },
    PerToken { max_events_per_token: u32 },
    PerNodeExecution { max_events_per_execution: u32 },
    PerFrame { max_events_per_frame: u32 },
    FaultOnly { max_events_per_frame: u32 },
}

pub enum ProbeSource {
    /// Probe attached to a specific node's pre/post entry.
    NodePreEntry  { node: NodeId },
    NodePostEntry { node: NodeId },
    /// Probe attached to a specific value edge.
    ValueEdge     { value: ValueId },
    /// Probe attached to an effect edge token.
    EffectEdge    { effect: EffectId, class: EffectClass },
    /// Probe attached to a SemanticAnchor (preferred — independent
    /// of NodeId churn across builds).
    Anchor        { anchor: SemanticAnchor },
}

pub enum ProbeLevel {
    /// Always-on lightweight metric event.
    Metric,
    /// Event-level probe; emitted only under TraceLevel ≥ Standard.
    Event,
    /// Heavy probe; emitted only under TraceLevel ≥ Verbose.
    Verbose,
    /// Faulting probe; only fires on a fault path.
    Fault,
}

pub struct TraceEventShape {
    pub payload_layout: TraceEventPayloadLayout,
    pub max_payload_bytes: u16,
    pub stable_id: TraceEventId,                // closed enum in gbf-policy
}

pub enum TraceEventPayloadLayout {
    Empty,
    U8,
    U16,
    U32,
    Q8_8,
    Q16_16,
    TokenId,
    ExpertId,
    /// Fixed-size byte tuple naming a closed shape.
    Tuple(TraceEventTupleSpecId),
}

pub struct MetricProbe {
    pub metric: MetricId,
    pub source: MetricSource,
    pub aggregation: MetricAggregation,
    pub budget_class: ProbeBudgetClass,
    pub weight: u16,
}

pub enum MetricSource {
    /// Counter incremented on each pass.
    PerPass,
    /// Per-token sample.
    PerToken,
    /// Reserved in v1. Schedule-slice metrics are enabled by the
    /// F-B13 amendment that introduces concrete SchedSlice ids.
    PerSliceReserved,
    /// Per-bank-switch sample.
    PerBankSwitch,
    /// Per-frame sample (frame = video frame).
    PerFrame,
}

pub enum MetricAggregation {
    Sum,
    Mean,
    Max,
    Min,
    P50,
    P90,
    P99,
    Histogram { bucket_count: u8 },
}

// MetricAggregation invariant:
//   Histogram.bucket_count > 0.
//   bucket_count = 0 is rejected with
//   OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO.

pub struct AnchorAttachmentTable {
    pub semantic: BTreeMap<SemanticCheckpointId, SemanticAttachment>,
    pub probes:   BTreeMap<ProbeInstanceId, ProbeSource>,
    pub metrics:  BTreeMap<MetricId, MetricSource>,
}

pub struct SemanticAttachment {
    pub anchor: SemanticAnchor,
    pub source: ObservationSource,
}

pub struct ObservationProvenance {
    pub semantic_provenance: BTreeMap<SemanticCheckpointId, EvidenceRef>,
    pub probe_provenance:    BTreeMap<ProbeInstanceId, EvidenceRef>,
    pub metric_provenance:   BTreeMap<MetricId, EvidenceRef>,
}
```

Notes:

* `SemanticObservation.anchor` is bound to a `NodeId` via the
  `NodeAnchorMap` F-B5 emits. The bound is total — every checkpoint
  in `semantic` corresponds to exactly one anchor and one node.
* `OperationalProbe.event_shape` declares the wire shape. Runtime
  (`gbf-runtime::trace`) and consumer (`gbf-debug`) read this shape
  to render trace events. The closed enum keeps the shape stable
  across builds so `gbf-debug` does not need build-specific decoders.
* `MetricProbe.aggregation` is declarative; `gbf-bench` uses it to
  set up histograms / windows. F-B6 only emits the declaration.
* `AnchorAttachmentTable` is a denormalized view over the three
  vectors for fast lookup; it is emitted in `observation_plan.json`
  as redundant review aid (derivable from `semantic`/`probes`/`metrics`).

### 8.2 Operation contract

```text
operation build_observation_plan_core (pure)
  input:
    i: ObservationPlanInputs

  modifies: nothing

  does_not_modify:
    GbInferIR
    QuantGraph
    ResolvedCompilePolicy
    SemanticCheckpointSchema
    StaticBudgetReport
    artifact_validation.json
    policy_resolution.json
    quant_graph.json
    infer_ir.json

  returns:
    Result[ObservationPlanCoreSuccess, ObservationPlanCoreFailure]

operation run_stage4 (driver)
  input:
    i: ObservationPlanInputs
    env: PassEnvironment

  effects:
    emits observation_plan.json
    emits semantic_checkpoint_schema.json (build-active subset, re-emit)
    emits operational_probe_schema.json
    may write StageCache success entry
    may write StageCache failure memo

  returns:
    Result[ObservationPlanCoreSuccess, ObservationPlanCoreFailure]
```

Preconditions:

```text
OP-Pre-1:
  i.infer_ir_self_hash must match i.infer_ir_product's computed
  self-hash.

OP-Pre-2:
  i.semantic_checkpoint_schema_hash must equal
  i.artifact_declared_semantic_checkpoint_schema_hash. Stage 4 does
  not re-fetch the sidecar; the F-B2/F-B4 driver provides the
  hash-bound view. A mismatch is
  OBSERVATION-SC-HASH-MISMATCH.

OP-Pre-3:
  i.op_policy_projection.observability_mode must equal the
  policy.observability mode resolved by Stage 0.5.

OP-Pre-3a:
  i.op_policy_projection.determinism_class must equal
  i.infer_ir_product.ir.identity.determinism.

OP-Pre-4:
  i.audit_parents.static_budget_self_hash must reference a passing
  Stage 2 report (decision.fits = true).
```

Pass postconditions:

```text
OP-Ok-1:
  result = Ok(o) ⇒ o.report.outcome = Passed

OP-Ok-2:
  result = Ok(o) ⇒ o.report.body.result = Some(...)

OP-Ok-3:
  result = Ok(o) ⇒ o.report.body.diagnostics = []
                  ∧ o.sc_re_emit_report.body.diagnostics = []
                  ∧ o.operational_probe_report.body.diagnostics = []

OP-Ok-4:
  result = Ok(o) ⇒ ∀ entry ∈ o.observation_plan.semantic.
                     entry.checkpoint ∈
                       i.semantic_checkpoint_schema.checkpoints[*].id

OP-Ok-5:
  result = Ok(o) ⇒ ∀ entry ∈ o.observation_plan.semantic.
                     there exists exactly one NodeId n such that
                     g.anchors[n] = entry.anchor, n refers to an
                     existing g.nodes entry, and that node's canonical
                     provenance tuple matches the checkpoint id's
                     canonical model path (§8.5).

OP-Ok-6:
  result = Ok(o) ⇒ every artifact-Mandatory checkpoint in
                   i.semantic_checkpoint_schema appears in
                   o.observation_plan.semantic with role = Mandatory.
                   Equivalently, a successful Stage 4 implies every
                   artifact-Mandatory checkpoint is build-feasible.

OP-Ok-7:
  result = Ok(o) ⇒ every workload-required checkpoint in
                   i.op_policy_projection.workload_observation.checkpoints
                   that is present in i.semantic_checkpoint_schema
                   and feasible in g appears in
                   o.observation_plan.semantic.

OP-Ok-8:
  result = Ok(o) ⇒ ∀ probe p ∈ o.observation_plan.probes.
                     p.budget_class ≥ i.op_policy_projection.optional_probe_floor
                   ∧ p.budget_class ∉ trace_demotion_drop_set(
                       i.op_policy_projection.trace_demotion)
                   ∧ p.probe_id ∉
                       i.op_policy_projection.disabled_optional_probes.

OP-Ok-9:
  result = Ok(o) ⇒ for every ProbeBudgetClass c.
                     sum_over_probes(p.weight | p.budget_class == c)
                     + sum_over_metrics(m.weight | m.budget_class == c)
                       ≤ profile_observation_caps[c]
                   (Required has no cap.)

OP-Ok-10:
  result = Ok(o) ⇒ ∀ probe p ∈ o.observation_plan.probes.
                     p.source references a valid GbInferIR entity:
                     - NodePreEntry/NodePostEntry: node ∈ g.nodes
                     - ValueEdge: value ∈ g.values
                     - EffectEdge: effect ∈ g.effects
                                   ∧ class ∈ {Rng { slot: RngSlot::Decode }}    (v1)
                     - Anchor: anchor ∈ g.anchors

OP-Ok-11:
  result = Ok(o) ⇒ ∀ entry ∈ o.observation_plan.semantic.
                     entry.encoding consistent with
                     i.op_policy_projection.workload_observation.compare_domain
                     (e.g. TokenIdOnly compare_domain ⇒ all PostDecode
                      entries use ObservationEncoding::TokenId).

OP-Ok-12:
  result = Ok(o) ⇒ provenance is total:
                     ∀ checkpoint ∈ semantic. semantic_provenance[checkpoint] = Some(_)
                     ∀ probe ∈ probes. probe_provenance[probe.instance_id] = Some(_)
                     ∀ metric ∈ metrics. metric_provenance[metric.metric] = Some(_)

OP-Ok-13:
  result = Ok(o) ⇒ the re-emitted semantic_checkpoint_schema.json
                   is a subset of i.semantic_checkpoint_schema
                   and may be equal to it:
                     re_emit.checkpoints ⊆ i.semantic_checkpoint_schema.checkpoints
                   ∧ re_emit.checkpoints corresponds 1-1 with
                     o.observation_plan.semantic.

OP-Ok-14:
  result = Ok(o) ⇒ operational_probe_schema.json carries one entry
                   per probe in o.observation_plan.probes, ordered
                   by `(TraceProbeId canonical order,
                         ProbeInstanceId.source_fingerprint canonical order)`.

OP-Ok-15:
  result = Ok(o) ⇒ ObservabilityMode = Invariant
                   ⇒ the active probe set's TraceBudget projection
                     fits the declared TraceBudget (max_events_per_slice,
                     max_bytes_per_frame). A budget bust under
                     Invariant is a Hard diagnostic
                     OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED.
                   (Under Flexible, a budget bust is recorded but
                   may proceed. The bust is recorded in
                   ObservationPlan.trace_budget_projection, not as a
                   ValidationDiagnostic, because this chunk rejects
                   Soft diagnostics.)
```

Failure postconditions:

```text
OP-Err-1:
  result = Err(e) ⇒ e.report.outcome = Failed

OP-Err-2:
  result = Err(e) ⇒ e.report.body.result = None

OP-Err-3:
  result = Err(e) ⇒ e.diagnostics is non-empty
                  ∧ ∀ d ∈ e.diagnostics. d.severity = Hard

OP-Err-4:
  result = Err(e) ⇒ Stage5 may still run independently
                  (per I-Pipeline-18) but the build cannot close.

OP-Err-5:
  result = Err(e) ⇒ no ObservationPlan product is exposed.
```

### 8.3 Construction order

ObservationPlan construction is staged into named binding classes.
Each class runs in numeric order; within a class, all admissible
diagnostics are collected.

```text
OPClass :=
  1 IdentityBinding
       binds infer_ir_self_hash, quant_graph_self_hash,
       semantic_checkpoint_schema_hash,
       observation_policy_projection_hash, artifact_aux_hash,
       determinism, observability_mode, trace_budget, workload_id

  2 SchemaIngestion
       parses i.semantic_checkpoint_schema; verifies its hash
       matches i.semantic_checkpoint_schema_hash; partitions
       checkpoints into Mandatory / Optional sets

  3 BuildFeasibilityFilter
       computes build_feasible_set(g) — the set of
       SemanticCheckpointIds whose corresponding canonical model
       path produces a node in g.nodes. Per-layer/per-expert ids
       outside g (e.g. routed-only ids on a dense build) are
       filtered. Mandatory checkpoints not in build_feasible_set
       are a Hard diagnostic
       OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE.

  4 SemanticSelection
       selects o.semantic from
         (Mandatory ∩ build_feasible) ∪
         (workload_required ∩ build_feasible) ∪
         (workload_optional ∩ build_feasible ∩ Optional)
       ordered by SemanticCheckpointId canonical order.

  5 SemanticAnchorBinding
       for every selected checkpoint, looks up the unique
       NodeAnchor matching the checkpoint's canonical model path
       through the closed Anchor → CheckpointId function (§8.5).
       Missing anchor: OBSERVATION-CHECKPOINT-NOT-ATTACHABLE (Hard).
       Ambiguous anchor (more than one match): unreachable per
       single-token convention; if it ever happens,
       OBSERVATION-CHECKPOINT-AMBIGUOUS (Hard).

  6 ObservationEncodingBinding
       binds ObservationEncoding per checkpoint based on the
       workload's compare_domain. The mapping is closed and
       deterministic; see §8.6.

  7 ProbeRegistryInstantiation
       reads the global probe registry's `ProbeSourceSelector`
       templates and instantiates each selector against the current
       `GbInferIR` / `NodeAnchorMap`, producing zero or more concrete
       `OperationalProbe` instances with concrete `ProbeSource`
       values. This produces the build-feasible probe set.

       ProbeRegistryEntry :=
         {
           probe_id: TraceProbeId,
           source_selector: ProbeSourceSelector,
           level: ProbeLevel,
           budget_class: ProbeBudgetClass,
           event_shape: TraceEventShape,
           frequency_bound: TraceFrequencyBound,
           weight: u16,
           evidence: EvidenceRef
         }

       ProbeSourceSelector :=
         | ByAnchorCheckpoint { checkpoint: SemanticCheckpointId,
                                timing: ProbeTiming }
         | ByInferOpTag       { op_tag: InferOpTag,
                                timing: ProbeTiming }
         | ByEffectClass      { class: EffectClass }
         | ByValueRole        { role: ValueRole }

       ProbeTiming := PreEntry | PostEntry

  8 ProbeBudgetGovernance
       drops probes below the optional_probe_floor; drops probes
       in the trace_demotion drop set; drops probes listed in
       i.op_policy_projection.disabled_optional_probes (empty
       pre-F-B16). Computes per-class weight totals and rejects
       any class whose total exceeds its profile cap.

  9 ProbeOrdering
       sorts surviving probes by `(TraceProbeId canonical order,
       source_fingerprint canonical order)`.

  10 MetricRegistryFilter
       filters the global metric registry to metrics whose source
       is feasible in g. `PerSliceReserved` metrics are rejected in
       v1 because Stage 4 has no SchedSlice ids.

  11 MetricSelection
       selects metrics whose budget_class is at or above the
       optional_probe_floor and not in the trace_demotion drop set.
       (Metrics share the floor with probes; a Diagnostic-class
       metric is dropped under DropDiagnosticAndBestEffort exactly
       like a Diagnostic-class probe.)

  12 MetricOrdering
       sorts surviving metrics by MetricId canonical order.

  13 AnchorTableBinding
       constructs AnchorAttachmentTable from selected
       semantic/probes/metrics for fast lookup.

  14 ProvenanceBinding
       fills semantic_provenance, probe_provenance, metric_provenance
       maps. Each entry carries an EvidenceRef pointing back to:
       - SemanticCheckpointSchema sidecar for semantic entries;
       - PROBE_REGISTRY entry for probes;
       - METRIC_REGISTRY entry for metrics.

  15 SchemaReEmit
       constructs the semantic_checkpoint_schema.json (re-emit) body
       containing only the build-active checkpoints with their
       per-checkpoint metadata (mandatory/optional role, encoding,
       attachment node id) and the original schema's hash recorded
       as a parent reference.

  16 OperationalProbeSchemaEmit
       constructs operational_probe_schema.json containing one entry
       per active probe with its event shape, level, budget class,
       and attachment source. Closed enum; gbf-debug consumes it.

  17 InvariantBudgetCheck
       under ObservabilityMode = Invariant, project the active
       probe set's per-slice / per-frame trace cost against the
       declared TraceBudget. A bust is a Hard diagnostic
       OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED. Under Flexible,
       a bust is recorded in the report but does not fail.

  18 SelfConsistency
       cross-class checks (§8.7).

  19 CanonicalSort
       canonicalizes BTreeMap / Vec ordering for semantic, probes,
       metrics, anchor_table, and provenance maps before hashing.
```

Ordering laws (parallel to F-B3/F-B5 §8.3):

```text
OP-Order:
  Classes run in numeric order.

OP-Accumulate:
  Within a class, collect all diagnostics that can be safely produced.

OP-ShortCircuit:
  A later class is skipped iff its inputs were invalidated by a failed
  earlier class.

OP-NoSilentDefaults:
  Every observation field must be derived from a hash-bound input or
  fail loudly. Default values that mask missing input are forbidden.
```

### 8.4 Selection rule for `semantic` (closed)

```text
build_feasible_set(g) :=
  { id ∈ SemanticCheckpointId
  | exists exactly one node n ∈ g.nodes
      with canonical_provenance_tuple(n) matching id's canonical
      model path }

Mandatory(scs) :=
  { ckpt.id | ckpt ∈ scs.checkpoints, ckpt.role = Mandatory }

Optional(scs) :=
  { ckpt.id | ckpt ∈ scs.checkpoints, ckpt.role = Optional }

WorkloadRequired(workload_obs) :=
  { id | id ∈ workload_obs.checkpoints.required }

WorkloadOptional(workload_obs) :=
  { id | id ∈ workload_obs.checkpoints.optional }

selected_semantic :=
  ( Mandatory(scs) ∩ build_feasible_set(g) )
  ∪ ( WorkloadRequired(workload_obs)
      ∩ build_feasible_set(g)
      ∩ SchemaIds(scs) )
  ∪ ( WorkloadOptional(workload_obs)
      ∩ build_feasible_set(g)
      ∩ Optional(scs) )

SchemaIds(scs) :=
  { ckpt.id | ckpt ∈ scs.checkpoints }
```

A `WorkloadRequired` id that is not present in SchemaIds(scs) is
`OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA` (Hard).

A `WorkloadRequired` id that is present in SchemaIds(scs) but not
feasible (e.g. asks for
`PostRouter { layer: 5 }` on a dense build) is `OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE`
(Hard).

A `Mandatory` id from `scs` that is not feasible is
`OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE` (Hard) — this means
the artifact's contract requires a checkpoint that this build's IR
shape cannot produce, which is an artifact / build mismatch and
must be diagnosed early.

`OBSERVATION-OPTIONAL-CHECKPOINT-NOT-FEASIBLE` is reserved but never
emitted: an optional id that is not feasible is just dropped silently
from the union (it's optional). The reserved slot exists so future
amendments can flip the rule if a workload's optional list gains
a "fail-if-not-feasible" sub-bit.

Amends planv0: `planv0.md` line 1620 says `ObservationPlan` "consumes
the exported `SemanticCheckpointSchema`" without specifying whether
all schema entries become observation entries. This RFC narrows that:
**only the build-feasible mandatory entries are unconditionally
included; optional entries are workload-driven**. The selection rule
above is the authoritative one.

### 8.5 Anchor → CheckpointId function (closed)

`SemanticAnchor` is `{ anchor_id: Hash256 }` (single hash field) in
the landed F-B5 code. The canonical provenance information is the
`CanonicalProvenanceTuple` struct that F-B5's `compute_semantic_anchor`
consumes; F-B6 reads the same struct out of the per-node provenance
the IR carries.

```rust
pub struct CanonicalProvenanceTuple {
    pub op_tag: InferOpTag,
    pub layer: Option<LayerId>,
    pub expert: Option<ExpertId>,
    pub expert_weight_slot: Option<ExpertWeightSlot>,
    pub norm_site: Option<NormSite>,
    pub state_slot: Option<StateSlotId>,
    pub residual_site: Option<ResidualSite>,
    pub occurrence_index: u32,
}
```

The mapping is a closed match over `CanonicalProvenanceTuple`:

```text
anchor_to_checkpoint(t: CanonicalProvenanceTuple) :=
  match t:
    CanonicalProvenanceTuple {
      op_tag: Embedding, occurrence_index: 0, ..
    } => Some(SemanticCheckpointId::PostEmbedding { layer: 0 })
       -- planv0.md line 2281: PostEmbedding takes a layer; we map
       -- the unique Embedding node to layer 0 for v1's single-token
       -- convention. (Per-layer PostEmbedding is reserved for
       -- multi-pass IR amendment; v1 emits only layer-0.)

    CanonicalProvenanceTuple {
      op_tag: CombineResidual, layer: Some(ℓ),
      residual_site: Some(ResidualSite::PostSequence),
      occurrence_index: 0, ..
    } => None  -- post-sequence residual is not a v1 checkpoint id

    CanonicalProvenanceTuple {
      op_tag: RouteTop1, layer: Some(ℓ), occurrence_index: 0, ..
    } => Some(SemanticCheckpointId::PostRouter { layer: ℓ })

    CanonicalProvenanceTuple {
      op_tag: ExpertMatVec, layer: Some(ℓ), expert: Some(e),
      occurrence_index: 0, ..
    } => Some(SemanticCheckpointId::PostExpertDowncast {
                 layer: ℓ, expert: e })
       -- PostExpertDowncast { layer, expert } is a per-expert
       -- candidate checkpoint, attached to the post-FfnDown
       -- ExpertMatVec output for the concrete ExpertId. F-B6 does
       -- not reinterpret `expert` as a runtime-selected value.

    CanonicalProvenanceTuple {
      op_tag: Classify, occurrence_index: 0, ..
    } => Some(SemanticCheckpointId::PostLogits)

    CanonicalProvenanceTuple {
      op_tag: DecodeToken, occurrence_index: 0, ..
    } => Some(SemanticCheckpointId::PostDecode)

    _ => None
```

The function is closed and total over the v1 op tag set
(per `INFER_OP_TAG_CANONICAL_ORDER`). Tuples that map to `None` are
not eligible to receive a checkpoint id; the dual-direction lookup
(CheckpointId → tuple → anchor) is unambiguous under the
single-token convention (`token_inputs.len() == 1` enforced by
`GbInferIR::new`).

For `PostExpertDowncast`, F-B6 attaches the id to the per-expert
candidate node corresponding to the concrete `ExpertId` in the
checkpoint id. The runtime-selected expert id may be recorded as
part of the observation payload, but it is not substituted into a
build-time `SemanticCheckpointId`.

### 8.6 ObservationEncoding mapping (closed)

```text
encoding_for(checkpoint, compare_domain, determinism) :=
  match (checkpoint, compare_domain):
    (PostDecode,      TokenIdOnly)        => TokenId
    (PostDecode,      _)                  => TokenId
                                            -- decoded token is always
                                            -- a token id
    (PostRouter { .. }, ExpertIdOnly)     => ExpertId
    (PostRouter { .. }, _)                => Canonical
                                            -- includes router score and
                                            -- gate weight under canonical
    (PostEmbedding { .. }, _)             => Canonical
    (PostLogits,      _)                  =>
       match determinism:
         BitExact            => Canonical
         NumericallyStable   => QuantizedQ8_8
         SeedStable          => QuantizedQ8_8
         DistributionStable  => QuantizedQ16_16
    (PostExpertDowncast { .. }, _)        => Canonical
```

A workload may override the encoding via
`ObservationPolicy.compare_domain`; the override must be in the
allowed set per checkpoint id. An invalid override is
`OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT` (Hard).

### 8.7 Self-consistency rules

```text
OP-SC-1:
  No duplicate SemanticCheckpointId in semantic.

OP-SC-2:
  No duplicate ProbeInstanceId in probes.

OP-SC-2a:
  Multiple probe instances MAY share the same TraceProbeId iff their
  ProbeInstanceId values differ. Disabling a TraceProbeId via
  `disabled_optional_probes` disables all instances with that id.

OP-SC-3:
  No duplicate MetricId in metrics.

OP-SC-3a:
  v1 permits at most one source binding per MetricId. If a later
  profile needs the same MetricId sampled at multiple sources, that
  later RFC must add `MetricInstanceId` and amend this invariant.

OP-SC-4:
  ∀ entry ∈ semantic.
    entry.checkpoint ∈ scs.checkpoints[*].id
    ∧ entry.anchor ∈ g.anchors
    ∧ anchor_to_checkpoint(entry.anchor) = Some(entry.checkpoint).

OP-SC-5:
  ∀ probe ∈ probes.
    probe.budget_class ≥ optional_probe_floor
    ∧ probe.budget_class ∉ trace_demotion_drop_set(trace_demotion)
    ∧ probe.probe_id ∈ PROBE_REGISTRY
    ∧ probe.level, probe.budget_class, event_shape,
      frequency_bound, and weight are copied from the matching
      PROBE_REGISTRY entry
    ∧ probe.source references a valid g entity (per OP-Ok-10).

OP-SC-6:
  ∀ metric ∈ metrics.
    metric.budget_class ≥ optional_probe_floor
    ∧ metric.budget_class ∉ trace_demotion_drop_set(trace_demotion)
    ∧ metric.metric ∈ METRIC_REGISTRY.

OP-SC-7:
  Per-class weight totals do not exceed profile caps.
  Required has no cap; the field is None and the sum is unbounded.

OP-SC-8:
  ObservabilityMode = Invariant
  ⇒ projected trace cost computed from
     TraceEventShape.max_payload_bytes × TraceFrequencyBound
     for each active probe fits TraceBudget.

OP-SC-9:
  AnchorAttachmentTable is consistent with the three vectors:
  ∀ entry ∈ semantic.
    anchor_table.semantic[entry.checkpoint].anchor = entry.anchor
    ∧ anchor_table.semantic[entry.checkpoint].source = entry.source.
  ∀ probe ∈ probes. anchor_table.probes[probe.instance_id] = probe.source.
  ∀ metric ∈ metrics. anchor_table.metrics[metric.metric] = metric.source.

OP-SC-10:
  Mandatory(scs) ∩ build_feasible_set(g) ⊆
    { entry.checkpoint | entry ∈ semantic, entry.artifact_role = Mandatory }
  -- every artifact-mandatory feasible checkpoint is present.

OP-SC-11:
  ∀ entry ∈ semantic where entry.artifact_role = Mandatory.
    entry.checkpoint ∈ Mandatory(scs).

OP-SC-12:
  ObservationProvenance is total:
    semantic_provenance.keys() = semantic[*].checkpoint
    probe_provenance.keys()    = probes[*].instance_id
    metric_provenance.keys()   = metrics[*].metric.

OP-SC-13:
  ∀ probe p ∈ probes.
    p.source = EffectEdge { class, .. }
    ⇒ class ∈ { Rng { slot: RngSlot::Decode } }    -- v1 emit set (§2.4)

OP-SC-14:
  ∀ probe p ∈ probes where p.source = NodePreEntry { node }
                       ∨ p.source = NodePostEntry { node }.
    g.nodes contains a node with that NodeId.

OP-SC-15:
  ∀ probe p ∈ probes where p.source = ValueEdge { value }.
    g.values contains a ValueDecl with that ValueId.

OP-SC-16:
  ∀ probe p ∈ probes where p.source = Anchor { anchor }.
    g.anchors contains that anchor (i.e. some NodeId maps to it
    in NodeAnchorMap).

OP-SC-17:
  ∀ entry ∈ semantic.
    entry.encoding ∈ encoding_allowed_for(entry.checkpoint).

OP-SC-18:
  observation_plan_self_hash =
    DomainHash("gbf-codegen", "ObservationPlan", "observation_plan.v1",
      CanonicalJson(observation_plan)).

OP-SC-19:
  ∀ probe p ∈ probes.
    let reg = PROBE_REGISTRY[p.probe_id].
    p.event_shape = reg.event_shape
    ∧ p.level = reg.level
    ∧ p.frequency_bound = reg.frequency_bound
    ∧ p.weight = reg.weight
    ∧ p.budget_class = reg.budget_class
      unless the registry entry explicitly declares a profile-variant
      override selected by `profile_id`.
```

### 8.8 Inheritance from F-B3/F-B5

F-B6 inherits the following from F-B3/F-B5 and **does not re-derive
or re-validate** them:

* `g.identity.determinism` is read; F-B6 records it but does not
  re-assert reduction-order policy.
* `g.anchors` is read; F-B6 attaches checkpoints to existing
  anchors. F-B6 does not recompute anchor hashes.
* `g.nodes`, `g.values`, `g.effects` are read for source validity
  (OP-Ok-10). F-B6 does not validate `op_signature` — that is
  F-B5's job and is already guaranteed.
* `infer_ir_self_hash` is the cache-key load-bearing identity of
  the IR; F-B6 records it.
* `quant_graph_self_hash` is recorded in identity (transitively
  available through F-B5's `infer_ir.identity.quant_graph_self_hash`)
  for downstream auditability. F-B6 does not consume `QuantGraph`
  fields directly.

### 8.9 Sequence-state probe attachment is reserved

Per §2.4, probes targeting `SequenceState { .. }` effect classes are
rejected with `OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED`. This
matches F-B5 §2.5a — sequence-state effect chains are not emitted
in v1, so even if a probe exists in the registry, it has no edge
to attach to.

When the sequence-state amendment lands, F-B5 will start emitting
`SequenceState { .. }` chains, F-B6 will be amended to enable
sequence-state probe attachment, and the reserved diagnostic will
become an active capability.

### 8.10 ObservationPlan does not feed RangePlan

Per I-Pipeline-26, F-B7 does not read `ObservationPlan`. The two
stages run in pipeline order (Stage 4 then Stage 5) for cache-key
locality and report-emission ordering, but they are independent
data-dependency-wise. An implementation may run them in parallel.

This independence is load-bearing for two reasons:

1. F-B16's refinement loop can re-run F-B7 on a new
   `RangeKnobs::reduction_ceiling` without touching F-B6 (probes
   stay stable across range-plan repairs).
2. F-B16's refinement loop can re-run F-B6 on a new
   `ObservationKnobs::optional_probe_floor` without touching F-B7
   (range certs stay stable across probe demotion).

Both loops are monotone (per F-B2/F-B4 §1) and the independence
above means they don't have to interleave.

## 9. Stage 5 contract: `RangePlan`

### 9.1 Type-level contract

```text
RangePlanInputs :=
  {
    infer_ir_product: GbInferIRProduct,
    infer_ir_self_hash: Hash256,
    quant_graph_self_hash: Hash256,
    static_budget_report: StaticBudgetReport,
    static_budget_self_hash: Hash256,
    range_policy_projection: RangePolicyProjection,
    audit_parents: RangePlanAuditParents,
  }

RangePolicyProjection :=
  {
    profile_id: CompileProfileId,
    range_caps: RangeCapsSpec,
    reduction_ceiling: ReductionPlanCeiling,
    reduction_ceiling_overrides:
      BTreeMap<ReductionSelector, ReductionPlanCeiling>,
    determinism_class: DeterminismClass,
  }

LockedRangeKnobs :=
  {
    reduction_ceiling_locked: bool,
  }

RangePlanAuditParents :=
  {
    policy_resolution_self_hash: Hash256,
    compile_request_hash: Hash256,
    artifact_aux_hash: Hash256,
    locked_range_knobs: LockedRangeKnobs,
  }

range_policy_projection_hash :=
  DomainHash("gbf-codegen", "RangePolicyProjection",
    "range_plan.v1", CanonicalJson(RangePolicyProjection))

-- The projection is the load-bearing slice of ResolvedCompilePolicy
-- for Stage 5. determinism_class is *recorded* but is read from the
-- IR (g.identity.determinism); the projection's copy is for audit.

RangePlanCoreProduct :=
  {
    range_plan: RangePlan,
    range_cert: RangeCertBody,
    range_plan_self_hash: Hash256,
    range_cert_body_hash: Hash256,
  }

RangePlanStageOutput :=
  {
    product: RangePlanCoreProduct,
    report: ReportEnvelope<RangePlanReportBody>,
    cert_report: ReportEnvelope<RangeCertBody>,
  }

RangePlanProduct := RangePlanCoreProduct
  -- Legacy alias used by §13.x stage handshakes. The canonical
  -- StageCache value is `RangePlanCoreProduct`; the report
  -- envelopes live in `RangePlanStageOutput` so a cache hit can
  -- replay the byte-identical core product (including the
  -- canonical certificate body) while the driver wraps fresh
  -- report envelopes for the current build's audit parents.

range_plan_self_hash :=
  DomainHash("gbf-codegen", "RangePlan", "range_plan.v1",
    CanonicalJson(range_plan))

range_cert_body_hash :=
  DomainHash("gbf-codegen", "RangeCertBody", "range.cert.v1",
    CanonicalJson(range_cert))

RangePlanStageFailure :=
  {
    report: ReportEnvelope<RangePlanReportBody>,
    cert_report: Option[ReportEnvelope<RangeCertBody>],
    diagnostics: NonEmptyList[ValidationDiagnostic],
  }
```

The public `RangePlan` type:

```rust
pub struct RangePlan {
    pub identity: RangePlanIdentity,
    pub entries: Vec<RangePlanEntry>,
    pub provenance: RangePlanProvenance,
}

pub struct RangePlanIdentity {
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub range_policy_projection_hash: Hash256,
    pub determinism: DeterminismClass,
}

pub struct RangePlanEntry {
    pub site: ReductionSiteId,
    pub plan: ReductionPlan,
    pub site_facts: ReductionSiteFacts,
    pub effective_ceiling: ReductionPlanCeiling,
        // ceiling AFTER overrides applied (Site beats Layer beats global)
    pub ceiling_provenance: ReductionCeilingProvenance,
}

// NOTE: earlier drafts of this RFC paired `site` with a separate
// `ReductionSiteKey`. The landed F-B3/F-B5 code (`gbf-policy::ReductionSiteId
// (String)`) makes the site id uniquely identifying on its own; no
// `ReductionSiteKey` type exists. Cross-stage joins use `ReductionSiteId`
// directly.

/// The four-tuple of facts F-B7 reads from F-B4 + F-B5 to compute
/// each certificate. Recorded in the report so the cert is
/// independently re-checkable.
pub struct ReductionSiteFacts {
    pub site:               ReductionSiteId,
    pub layer:              Option<LayerId>,
    pub expert:             Option<ExpertId>,
    pub slot:               Option<ExpertWeightSlot>,
    pub norm_site:          Option<NormSite>,
    pub term_count:         u32,
        // From F-B4's ReductionSiteProjection.term_count
    pub input_max_abs_q:    u32,
        // From F-B4's input_max_abs_q
    pub weight_max_abs_q:   u32,
        // From F-B4's weight_max_abs_q
    pub per_term_abs_max_q: u64,
        // Copied verbatim from F-B4's ReductionSiteProjection.
        // This is the authoritative per-term product bound.
    pub bias_max_abs_q:     Option<u32>, // None when no bias; verifier treats absent as 0
    pub accumulator_domain: AccumulatorDomain,
        // From F-B4: RawIntegerProducts | PostScaleQ8_8 | PostScaleQ16_16
    pub op_tag:             InferOpTag,
        // routerMatVec | ExpertMatVec | Norm | Classify
}

pub enum ReductionPlan {
    SingleI16,
    ChunkedI16 { chunk_len: u16 },
    RenormLoop { tile_len: u16, renorm: RenormSpec },
}

pub struct RenormSpec {
    pub strategy: RenormStrategy,
    pub recurrence: RenormRecurrence,
}

pub enum ReductionCeilingProvenance {
    /// Came from CompileKnobs::range::reduction_ceiling (global default).
    Global { source: PolicySource },
    /// Came from a Layer override.
    LayerOverride { layer: LayerId, source: PolicySource },
    /// Came from a Site override.
    SiteOverride { site: ReductionSiteId, source: PolicySource },
}

pub struct RangePlanProvenance {
    pub site_to_node:    BTreeMap<ReductionSiteId, NodeId>,
    pub site_to_qg:      BTreeMap<ReductionSiteId, QuantGraphEntityRef>,
}
```

The certificate report body:

```rust
pub struct RangeCertBody {
    pub identity: RangeCertIdentity,
    pub cert_outcome: CertOutcome,
    pub certificates: Vec<CertifiedReduction>,
    pub site_to_certificate_index: BTreeMap<ReductionSiteId, u32>,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct RangeCertIdentity {
    pub range_plan_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub determinism: DeterminismClass,
}

pub struct CertifiedReduction {
    pub site: ReductionSiteId,
    pub plan: ReductionPlan,
    pub facts: ReductionSiteFacts,
    pub proof: AccumulatorCertificate,
}

pub enum AccumulatorCertificate {
    SingleI16Proof {
        site: ReductionSiteId,
        term_count: u32,
        per_term_abs_max: u64,
        sum_bound: u64,                  // term_count * per_term_abs_max
        bias_abs_max: u64,
        total_abs_max: u64,              // sum_bound + bias_abs_max
        i16_envelope: u64,               // = i16::MAX (32_767)
        slack: u64,                      // i16_envelope - total_abs_max
    },
    ChunkedI16Proof {
        site: ReductionSiteId,
        chunk_len: u16,
        chunk_count: u32,                // ceil(term_count / chunk_len)
        per_term_abs_max: u64,
        per_chunk_sum_bound: u64,        // chunk_len * per_term_abs_max
        per_chunk_i16_slack: u64,        // i16::MAX - per_chunk_sum_bound
        cross_chunk_sum_bound: u64,      // term_count * per_term_abs_max
        bias_abs_max: u64,
        total_abs_max: u64,              // cross_chunk_sum_bound + bias_abs_max
        i32_envelope: u64,               // = i32::MAX
        slack: u64,                      // i32_envelope - total_abs_max
    },
    RenormLoopProof {
        site: ReductionSiteId,
        tile_len: u16,
        tile_count: u32,
        per_term_abs_max: u64,
        per_tile_sum_bound: u64,         // tile_len * per_term_abs_max
        per_tile_i16_slack: u64,         // i16::MAX - per_tile_sum_bound
        renorm: RenormSpec,
        bias_abs_max: u64,
        total_abs_max: u64,              // post-renorm bound
        slack: u64,
    },
    Failed {
        site: ReductionSiteId,
        attempted_plan: ReductionPlan,
        proof_state: AccumulatorProofState,
        witness: AccumulatorFailureWitness,
    },
}

pub enum RenormStrategy {
    /// Renormalize at the end of each tile (post-named-boundary).
    /// Required under BitExact.
    ExactPostBoundary,
    /// Renormalize when the partial sum approaches i16::MAX with
    /// declared margin. Forbidden under BitExact.
    DynamicMargin { margin_q16_16: u32 },
}

pub struct RenormRecurrence {
    pub input_scale_q16_16: u32,
    pub output_scale_q16_16: u32,
    pub rounding: RenormRounding,
    pub saturation: RenormSaturationPolicy,
    pub max_rounding_error_q16_16: u32,
}

pub enum RenormRounding {
    TowardZero,
    NearestEven,
}

pub enum RenormSaturationPolicy {
    Forbidden,
    AtNamedNumericBoundary { boundary: NamedNumericBoundary },
}

pub enum AccumulatorProofState {
    SumExceedsI16Envelope { sum_bound: u64, envelope: u32 },
    PerChunkExceedsI16Envelope { per_chunk_sum_bound: u64, envelope: u32 },
    CrossChunkExceedsI32Envelope { cross_chunk_sum_bound: u64, envelope: u64 },
    PerTileExceedsI16Envelope { per_tile_sum_bound: u64, envelope: u32 },
    LengthZero { length_field: LengthField },
    /// chosen chunk_len exceeds RangeCapsSpec.profile_chunk_max.
    ChunkLenExceedsProfileMax { chunk_len: u16, profile_chunk_max: u16 },
    /// chosen tile_len is below RangeCapsSpec.profile_tile_min.
    TileLenBelowProfileMin { tile_len: u16, profile_tile_min: u16 },
    /// chosen tile_len exceeds RangeCapsSpec.profile_tile_max.
    TileLenExceedsProfileMax { tile_len: u16, profile_tile_max: u16 },
    BitExactRequiresChunkDivides { term_count: u32, chunk_len: u16 },
    TileLenExceedsU16 { term_count: u32 },
    DeterminismRequiresEnforcedRenorm,
}

pub enum AccumulatorFailureWitness {
    BoundCalculation {
        input_max_abs_q: u32,
        weight_max_abs_q: u32,
        term_count: u32,
        bias: u32,
    },
    BitExactSaturationForbidden,
}

pub enum LengthField {
    ChunkLen,
    TileLen,
}
```

Notes:

* `ReductionSiteFacts.term_count`, `input_max_abs_q`,
  `weight_max_abs_q`, `bias_max_abs_q`, and `accumulator_domain` are
  copied from F-B4's `ReductionSiteProjection` for the matching
  `ReductionSiteId`. F-B7 never re-derives these scalars from
  `QuantGraph` — that is F-B4's job and is already validated. This
  means F-B7's certificate is independently re-checkable: anyone
  with the JSON can run the formula `term_count * per_term_abs_max
  + bias_abs_max ≤ envelope` and verify.
* `per_term_abs_max := site_facts.per_term_abs_max_q`. F-B7 copies
  this value from F-B4's `ReductionSiteProjection` and checks that
  it is internally consistent with any optional explanatory
  input/weight maxima when those are present. F-B7 does not rederive
  the authoritative per-term bound from QuantGraph.
* `chunk_len` and `tile_len` are `u16` per `planv0.md` lines
  1657–1659. A `chunk_len = 0` or `tile_len = 0` is a degenerate
  certificate; F-B7 emits `AccumulatorProofState::LengthZero`
  rather than panicking.
* `RenormStrategy::DynamicMargin { margin_q16_16 }` carries the
  margin as a fixed-point fraction of `i16::MAX`. Under
  `BitExact`, only `ExactPostBoundary` is admissible.
* The `Failed` variant carries enough information for F-B16 to
  decide what knob to advance: the `attempted_plan` and `proof_state`
  jointly tell F-B16 whether to raise the ceiling, narrow tiles, or
  give up.

### 9.2 Operation contract

```text
operation build_range_plan_core (pure)
  input:
    i: RangePlanInputs

  modifies: nothing

  does_not_modify:
    GbInferIR
    QuantGraph
    StaticBudgetReport
    ResolvedCompilePolicy
    artifact_validation.json
    policy_resolution.json
    quant_graph.json
    infer_ir.json
    static_budget.json
    observation_plan.json    (independent of Stage 4)

  returns:
    Result[RangePlanCoreSuccess, RangePlanCoreFailure]

operation run_stage5 (driver)
  input:
    i: RangePlanInputs
    env: PassEnvironment

  effects:
    emits range_plan.json
    emits certs/range.cert.json
    may write StageCache success entry
    may write StageCache failure memo

  returns:
    Result[RangePlanCoreSuccess, RangePlanCoreFailure]
```

Preconditions:

```text
RP-Pre-1:
  i.infer_ir_self_hash must match i.infer_ir_product's computed
  self-hash.

RP-Pre-2:
  i.quant_graph_self_hash must match
  i.infer_ir_product.ir.identity.quant_graph_self_hash
  (audit binding only; F-B7 does not read QuantGraph directly).

RP-Pre-3:
  i.static_budget_self_hash must equal
  i.static_budget_report.static_budget_self_hash, and
  i.static_budget_report.report.outcome must be Passed.

RP-Pre-4:
  i.range_policy_projection.determinism_class must equal
  g.identity.determinism (i.e. the policy projection's recorded
  determinism agrees with the IR-side determinism).

RP-Pre-5:
  i.range_policy_projection.reduction_ceiling must be a valid
  ReductionPlanCeiling value: SingleI16Only | AllowChunkedI16 |
  AllowRenormLoop.

RP-Pre-6:
  i.range_policy_projection.reduction_ceiling_overrides keys must
  reference selectors that resolve in g (Site → existing
  ReductionSiteId; Layer → layer in g.model_spec_summary).

RP-Pre-7:
  In v1, ReductionSiteId is node-unique in GbInferIR:
    no two GbNode entries may carry the same
    reduction_site = Some(rsid).
  A duplicate site id in g.nodes is RANGE-DUPLICATE-REDUCTION-SITE-ID.
```

Pass postconditions:

```text
RP-Ok-1:
  result = Ok(r) ⇒ r.report.outcome = Passed
                 ∧ r.cert_report.outcome = Passed
                 ∧ r.cert_report.body.cert_outcome = Verified

RP-Ok-2:
  result = Ok(r) ⇒ r.report.body.result = Some(...)
                 ∧ r.cert_report.body.certificates does not contain
                   any Failed variant.

RP-Ok-3:
  result = Ok(r) ⇒ r.report.body.diagnostics = []
                 ∧ r.cert_report.body.diagnostics = [].

RP-Ok-4:
  result = Ok(r) ⇒ r.range_plan.entries.len() =
                   |{ n ∈ g.nodes | n.reduction_site = Some(_) }|.

RP-Ok-5:
  result = Ok(r) ⇒ ∀ entry e ∈ r.range_plan.entries.
                     ∃ exactly one node n ∈ g.nodes.
                       n.reduction_site = Some(e.site).

RP-Ok-6:
  result = Ok(r) ⇒ ∀ node n ∈ g.nodes where n.reduction_site = Some(rsid).
                     ∃ exactly one entry e ∈ r.range_plan.entries.
                       e.site = rsid.

RP-Ok-7:
  result = Ok(r) ⇒ ∀ entry e ∈ r.range_plan.entries.
                     e.plan ∈ admissible_under_ceiling(e.effective_ceiling)
                     -- per §9.4 closed mapping.

RP-Ok-8:
  result = Ok(r) ⇒ ∀ entry e ∈ r.range_plan.entries.
                     verifies(cert_for_site(r.cert_report.body, e.site),
                              e.plan, e.site_facts).

RP-Ok-9:
  result = Ok(r) ⇒ ∀ entry e ∈ r.range_plan.entries.
                     ∀ plan p < e.plan in canonical order
                       (SingleI16 < ChunkedI16 < RenormLoop):
                       p ∉ admissible_under_ceiling(e.effective_ceiling)
                       ∨ ¬verifies_for(p, e.site_facts).

RP-Ok-10:
  result = Ok(r) ⇒ ∀ entry e ∈ r.range_plan.entries.
                     e.site_facts copies the matching fields from
                     the Stage 2 `ReductionSiteProjection` for
                     `e.site` verbatim (see §13.2a for the access
                     trait). F-B7 does NOT recompute term_count,
                     input_max_abs_q, weight_max_abs_q, or
                     accumulator_domain from QuantGraph.

RP-Ok-11:
  result = Ok(r) ⇒ if g.identity.determinism = BitExact:
                     ∀ entry e where e.plan = ChunkedI16 { chunk_len }:
                       e.site_facts.term_count % chunk_len = 0.
                     ∀ entry e where e.plan = RenormLoop { tile_len }:
                       e.cert.renorm_strategy = ExactPostBoundary.

RP-Ok-12:
  result = Ok(r) ⇒ provenance is total:
                     site_to_node maps every site to a NodeId in g.nodes.
                     site_to_qg maps every site to the originating
                       QuantGraphEntityRef (Tensor / NormPlan / Classify head).

RP-Ok-13:
  result = Ok(r) ⇒ ceiling_provenance for every entry records the
                   effective source: Global, Layer override, or
                   Site override (most-specific wins).

RP-Ok-14:
  result = Ok(r) ⇒ no diagnostic carries a RepairProposal source
                   (per O23 in F-B3/F-B5, lifted normatively here).
```

Failure postconditions:

```text
RP-Err-1:
  result = Err(e) ⇒ e.report.outcome = Failed
                  ∧ e.report.body.result = None

RP-Err-2:
  result = Err(e) ⇒ e.cert_report.outcome = Failed (when emitted)
                  ∧ (
                       ∃ at least one AccumulatorCertificate::Failed
                         entry naming a failing site
                       OR e.cert_report.body.diagnostics contains at
                         least one Hard diagnostic for a pre-certificate
                         failure
                     ).

RP-Err-3:
  result = Err(e) ⇒ ∀ d ∈ e.diagnostics. d.severity = Hard.

RP-Err-4:
  result = Err(e) ⇒ no RangePlan product is exposed.

RP-Err-5:
  result = Err(e) ⇒ the build cannot close.
```

### 9.3 Construction order

```text
RPClass :=
  1 IdentityBinding
       binds infer_ir_self_hash, quant_graph_self_hash,
       static_budget_self_hash, range_policy_projection_hash,
       determinism. Asserts determinism agreement.

  2 ReductionSiteEnumeration
       walks g.nodes for nodes with reduction_site = Some(rsid).
       Builds the ordered set of (NodeId, ReductionSiteId).
       Verifies set is total (every reduction-bearing op has a site).

  3 SiteFactsBinding
       for each (NodeId, ReductionSiteId), looks up the matching
       Stage 2 `ReductionSiteProjection` via the access trait
       described in §13.2a. Copies term_count, input_max_abs_q,
       weight_max_abs_q, bias_max_abs_q, and accumulator_domain into
       ReductionSiteFacts. Computes per_term_abs_max_q internally
       with `checked_u128(input_max_abs_q) *
       checked_u128(weight_max_abs_q)` and stores it on
       `ReductionSiteFacts` as a derived authoritative bound.
       If Stage 2 later publishes `per_term_abs_max_q` directly,
       F-B7 prefers Stage 2's value and falls back to the checked
       product when absent. F-B7 never reads `QuantGraph.tensors`.
       If `accumulator_domain != RawIntegerProducts` in v1, emit
       RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1 for that site.
       Rejects `term_count = 0` with RANGE-TERM-COUNT-ZERO.

  4 EffectiveCeilingBinding
       for each site, resolves the effective ceiling per the
       most-specific-wins rule:
         override_for_site > override_for_layer > global ceiling.
       Records the source via ReductionCeilingProvenance.
       Multiple overrides with the same selector are rejected during
       canonical map deserialization as duplicate keys. If two
       different selectors of the same specificity resolve to the
       same site, emit RANGE-CEILING-OVERRIDE-AMBIGUOUS.

  5 PlanCandidateGeneration
       for each site, enumerates candidate plans admissible under
       the effective ceiling:
         SingleI16Only       => [SingleI16]
         AllowChunkedI16     => [SingleI16, ChunkedI16]
         AllowRenormLoop     => [SingleI16, ChunkedI16, RenormLoop]
       Within ChunkedI16 and RenormLoop, the candidate plan does
       not yet have a chunk_len or tile_len; that is chosen in
       the next class.

  6 PlanLengthSelection
       for each ChunkedI16 / RenormLoop candidate, chooses chunk_len
       (or tile_len) per the closed length-selection rule:
         - default chunk_len: largest power-of-two ≤ chunk_max
                              such that per_chunk_sum_bound ≤ i16::MAX
                              and (under BitExact) chunk_len divides
                              term_count.
         - default tile_len: smallest tile_len ≥ tile_min such that
                              per_tile_sum_bound ≤ i16::MAX with
                              required margin under the renorm strategy.
       (chunk_max, tile_min are fixed in the profile spec; see §9.5.)

  7 CertificateConstruction
       for each (site, candidate plan), constructs the corresponding
       AccumulatorCertificate variant. If proof fails (sum exceeds
       envelope, length zero, BitExact requires divisibility, etc.),
       emits AccumulatorCertificate::Failed with the proof state
       and witness.

  8 PlanChoice
       for each site, picks the smallest candidate plan whose
       certificate is non-Failed (i.e. verifies). If no candidate
       verifies:
         - if effective_ceiling = SingleI16Only and SingleI16 fails,
           emit RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY.
         - if effective_ceiling = AllowChunkedI16, SingleI16 and
           ChunkedI16 fail, and RenormLoop would verify, emit
           RANGE-CEILING-VIOLATED-NO-RENORM-LOOP.
         - otherwise emit RANGE-NO-PROVEN-PLAN-WITHIN-CEILING.

  9 ProvenanceBinding
       fills site_to_node and site_to_qg maps.

  10 CanonicalSort
       sorts r.range_plan.entries by ReductionSiteId canonical order.
       Sorts cert_report.certificates by ReductionSiteId.
       Sorts site_to_node and site_to_qg.

  11 SelfConsistency
       cross-class checks (§9.6).
```

Ordering laws (parallel to F-B3/F-B5 §8.3):

```text
RP-Order:
  Classes run in numeric order.

RP-Accumulate:
  Within a class, collect all admissible diagnostics.

RP-ShortCircuit:
  A later class is skipped iff its inputs were invalidated.

RP-NoSilentDefaults:
  Every plan field must be derived from a hash-bound input or fail
  loudly. Default values that mask missing input are forbidden.
```

### 9.4 ReductionPlanCeiling admissibility (closed)

```text
admissible_under_ceiling(ceiling) :=
  match ceiling:
    SingleI16Only    => { SingleI16 }
    AllowChunkedI16  => { SingleI16, ChunkedI16 }
    AllowRenormLoop  => { SingleI16, ChunkedI16, RenormLoop }

canonical_order: SingleI16 < ChunkedI16 < RenormLoop
```

### 9.5 Plan-length selection rule (closed)

`chunk_len` and `tile_len` are derived deterministically from
`ReductionSiteFacts` and the active `CompileProfileSpec`'s
length-selection caps. v1 uses fixed caps:

```text
profile_chunk_max := range_policy_projection.range_caps.profile_chunk_max
profile_tile_min  := range_policy_projection.range_caps.profile_tile_min
renorm_strategy_policy :=
  range_policy_projection.range_caps.renorm_strategy

choose_chunk_len(facts, determinism):
  let per_term = per_term_abs_max(facts)
  assert facts.term_count > 0     -- enforced by SiteFactsBinding
  if per_term = 0:
    let max_len = min(profile_chunk_max, u16::MAX)
    match determinism:
      BitExact:
        let pow2_divisors = { 2^k | 1 ≤ 2^k ≤ max_len
                                   ∧ facts.term_count % 2^k = 0 }
        if pow2_divisors empty:
          return Err(BitExactRequiresChunkDivides)
        return max(pow2_divisors)
      _:
        return max_pow2_le(max_len)
  let raw_max_safe = floor(i16::MAX / per_term)
  if raw_max_safe = 0:
    return Err(PerChunkExceedsI16Envelope)
  let max_safe = min(raw_max_safe, profile_chunk_max)
  match determinism:
    BitExact:
      -- chunk_len must divide term_count; pick largest divisor
      -- of term_count that is ≤ max_safe and is a power of two
      let pow2_divisors = { 2^k | 2^k ≤ max_safe ∧ term_count % 2^k = 0 }
      if pow2_divisors empty:
        return Err(BitExactRequiresChunkDivides)
      return max(pow2_divisors)
    _:
      -- pick largest power-of-two ≤ max_safe
      return max_pow2_le(max_safe)

choose_tile_len(facts, renorm_policy, determinism):
  let per_term = per_term_abs_max(facts)
  if per_term = 0:
    return profile_tile_min
  let renorm_strategy =
    match (determinism, renorm_policy):
      (BitExact, _) => ExactPostBoundary
      (_, ExactPostBoundaryOnly) => ExactPostBoundary
      (_, DynamicMargin { margin_q16_16 }) => DynamicMargin { margin_q16_16 }

  let margin_abs =
    match renorm_strategy:
      ExactPostBoundary => 0
      DynamicMargin { margin_q16_16 } =>
        floor(i16::MAX * margin_q16_16 / 2^16)
  let safe_envelope = i16::MAX - margin_abs
  let raw_max_safe_under_margin = floor(safe_envelope / per_term)
  if raw_max_safe_under_margin < profile_tile_min:
    return Err(PerTileExceedsI16Envelope)
  let max_safe_under_margin =
    min(raw_max_safe_under_margin, profile_tile_max)
  match (determinism, renorm_strategy):
    (BitExact, ExactPostBoundary):
      -- BitExact forbids mid-reduction renormalization. RenormLoop
      -- may only act at the reduction site's named numeric boundary.
      -- Therefore the only legal tile length is the full reduction.
      if facts.term_count > u16::MAX:
        return Err(TileLenExceedsU16)
      if facts.term_count < profile_tile_min:
        return Err(TileLenBelowProfileMin)
      if facts.term_count > max_safe_under_margin:
        return Err(PerTileExceedsI16Envelope)
      return facts.term_count as u16
    (BitExact, DynamicMargin { .. }):
      return Err(DeterminismRequiresEnforcedRenorm)
    _:
      return max_pow2_between(profile_tile_min, max_safe_under_margin)
        or Err(PerTileExceedsI16Envelope)

per_term_abs_max(facts) :=
  checked_u128(facts.per_term_abs_max_q)
```

Amends planv0: `planv0.md` does not pin `chunk_len`/`tile_len`
selection. This RFC pins it via the closed rule above so two
regenerations on the same inputs always pick the same length, and
so F-B16's monotone repair direction
(`KnobDelta::RaiseReductionCeiling`) has a stable downstream effect.

The rule's three knobs (`profile_chunk_max`, `profile_tile_min`,
`profile_renorm_margin_q16_16`) live in `CompileProfileSpec`'s
`range_caps` field (a new field added to F-B2/F-B4's profile spec
schema by this RFC; see §13.1). The values above are v1 defaults
shared by Bringup / Default / Trace / Recovery; future profiles
may tighten them.

### 9.6 Certificate verification predicate (closed)

```text
verifies(cert, plan, facts) :=
  facts.accumulator_domain = RawIntegerProducts
  ∧
  match cert:
    SingleI16Proof { site, term_count, per_term_abs_max, sum_bound,
                     bias_abs_max, total_abs_max, i16_envelope, slack }:
      site = facts.site
      ∧ term_count = facts.term_count
      ∧ per_term_abs_max = facts.per_term_abs_max_q
      ∧ sum_bound = term_count * per_term_abs_max                            -- checked u64
      ∧ bias_abs_max = facts.bias_max_abs_q.unwrap_or(0)
      ∧ total_abs_max = sum_bound + bias_abs_max                             -- checked u64
      ∧ i16_envelope = 32_767
      ∧ total_abs_max ≤ i16_envelope
      ∧ slack = i16_envelope - total_abs_max
      ∧ plan = SingleI16

    ChunkedI16Proof { site, chunk_len, chunk_count, per_term_abs_max,
                      per_chunk_sum_bound, per_chunk_i16_slack,
                      cross_chunk_sum_bound, bias_abs_max,
                      total_abs_max, i32_envelope, slack }:
      site = facts.site
      ∧ chunk_len > 0
      ∧ chunk_count = ceil(facts.term_count / chunk_len) (= facts.term_count / chunk_len
                                                           when divides)
      ∧ per_term_abs_max = facts.per_term_abs_max_q
      ∧ per_chunk_sum_bound = chunk_len * per_term_abs_max
      ∧ per_chunk_sum_bound ≤ 32_767                       -- per-chunk i16 fits
      ∧ per_chunk_i16_slack = 32_767 - per_chunk_sum_bound
      ∧ cross_chunk_sum_bound = facts.term_count * per_term_abs_max
      ∧ bias_abs_max = facts.bias_max_abs_q.unwrap_or(0)
      ∧ total_abs_max = cross_chunk_sum_bound + bias_abs_max
      ∧ i32_envelope = 2_147_483_647
      ∧ total_abs_max ≤ i32_envelope
      ∧ slack = i32_envelope - total_abs_max
      ∧ plan = ChunkedI16 { chunk_len = chunk_len }

    RenormLoopProof { site, tile_len, tile_count, per_term_abs_max,
                      per_tile_sum_bound, per_tile_i16_slack,
                      renorm,
                      bias_abs_max, total_abs_max, slack }:
      site = facts.site
      ∧ tile_len > 0
      ∧ tile_count = ceil(facts.term_count / tile_len)
      ∧ per_term_abs_max = facts.per_term_abs_max_q
      ∧ per_tile_sum_bound = tile_len * per_term_abs_max
      ∧ per_tile_sum_bound ≤ 32_767
      ∧ per_tile_i16_slack = 32_767 - per_tile_sum_bound
      ∧ bias_abs_max = facts.bias_max_abs_q.unwrap_or(0)
      ∧ renorm_recurrence_verifies(facts,
                                   tile_len,
                                   tile_count,
                                   renorm.strategy,
                                   renorm.recurrence,
                                   total_abs_max)
      ∧ total_abs_max ≤ 32_767                              -- post-renorm bound
      ∧ slack = 32_767 - total_abs_max
      ∧ plan = RenormLoop { tile_len = tile_len, renorm = renorm }
      ∧ (determinism = BitExact
         ⇒ renorm.strategy = ExactPostBoundary
            ∧ tile_len = facts.term_count
            ∧ tile_count = 1)

    Failed { .. }:
      false   -- a Failed certificate never verifies
```

`verifies` is implemented as a closed match. Any certificate whose
internal scalars do not satisfy the inequality is treated as
`Failed` (the constructor enforces the inequality at build time;
deserialization rejects malformed certificates with
`RANGE-CERT-MALFORMED`).

The renorm recurrence predicate (closed):

```text
renorm_recurrence_verifies(facts, tile_len, tile_count,
                           strategy, recurrence, claimed_total_abs_max) :=
  match facts.accumulator_domain:
    RawIntegerProducts =>
      -- v1 supports only recurrence metadata sufficient to prove
      -- boundedness. Equality to canonical reference semantics is
      -- asserted only under BitExact + ExactPostBoundary +
      -- saturation Forbidden/AtNamedNumericBoundary.
      recurrence.output_scale_q16_16 > 0
      ∧ recurrence.max_rounding_error_q16_16 is computed from the
        declared rounding mode
      ∧ claimed_total_abs_max is the closed-form bound produced by
        applying the recurrence for `tile_count` tiles plus bias.
    _ =>
      false
```

The verification is **independently checkable**: every load-bearing
scalar (`term_count`, `per_term_abs_max`, `sum_bound`,
`total_abs_max`, `slack`, `chunk_len`, `tile_len`) is in the JSON,
so `gbf-verify` (F-F1) can read the cert, run the formula, and
verify.

`verifies_for(plan, facts)` is the dual: given a candidate plan and
site facts, attempt to construct the corresponding certificate and
return whether it would verify. Implementation:

```text
verifies_for(plan, facts) :=
  match plan:
    SingleI16:
      total = facts.term_count * facts.per_term_abs_max_q
              + facts.bias_max_abs_q.unwrap_or(0)
      return total ≤ 32_767

    ChunkedI16 { chunk_len }:
      per_chunk = chunk_len * facts.per_term_abs_max_q
      if per_chunk > 32_767: return false
      chunks = ceil(facts.term_count / chunk_len)
      cross = facts.term_count * facts.per_term_abs_max_q
              + facts.bias_max_abs_q.unwrap_or(0)
      return cross ≤ 2_147_483_647

    RenormLoop { tile_len }:
      per_tile = tile_len * facts.per_term_abs_max_q
      return per_tile ≤ 32_767
      -- post-renorm bound is by construction ≤ 32_767 assuming the
      -- renorm strategy is honored
```

### 9.7 Self-consistency rules

```text
RP-SC-1:
  Every ReductionSiteId in r.entries is unique.

RP-SC-2:
  Every entry's site appears in g.nodes via reduction_site.

RP-SC-3:
  Every g.node with reduction_site = Some(_) appears in r.entries.

RP-SC-4:
  For every entry e:
    e.plan ∈ admissible_under_ceiling(e.effective_ceiling).

RP-SC-4a:
  For every entry e:
    e.site_facts.accumulator_domain = RawIntegerProducts.
  Non-raw accumulator domains are rejected in v1 with
  RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1 unless this RFC is amended
  with closed formulas for those domains.

RP-SC-5:
  For every entry e:
    verifies(cert_for_site(cert_report.body, e.site),
             e.plan, e.site_facts) holds.

RP-SC-6:
  For every entry e:
    if e.plan = SingleI16:
      no smaller plan exists, so this is automatically the smallest.
    if e.plan = ChunkedI16 { chunk_len }:
      ¬verifies_for(SingleI16, e.site_facts)
        -- otherwise SingleI16 would have been chosen.
    if e.plan = RenormLoop { tile_len }:
      ¬verifies_for(SingleI16, e.site_facts)
      ∧ ¬verifies_for(ChunkedI16 { chunk_len = best_chunk(e.site_facts) },
                      e.site_facts)
        -- otherwise ChunkedI16 would have been chosen.

RP-SC-7:
  RangePlan.identity.determinism = g.identity.determinism.

RP-SC-8:
  if RangePlan.identity.determinism = BitExact:
    every ChunkedI16 entry has chunk_len dividing term_count.
    every RenormLoop entry has renorm_strategy = ExactPostBoundary.

RP-SC-9:
  Every entry's site_facts copies match
  static_budget.reduction_sites[site] verbatim.

RP-SC-10:
  ceiling_provenance for every entry is well-formed:
    Global ceiling is always available (it's the resolved knob).
    Layer override, if used, has matching layer in g.model_spec_summary.
    Site override, if used, has matching site in r.entries.

RP-SC-11:
  range_plan_self_hash =
    DomainHash("gbf-codegen", "RangePlan", "range_plan.v1",
      CanonicalJson(range_plan)).

RP-SC-12:
  No diagnostic in range_plan.json body diagnostics or
  range.cert.json body diagnostics carries a
  RepairProposal source or AuthorizedRelaxation operation.
```

### 9.8 Op output value-format predicate (RangePlan ↔ ValueFormat)

F-B7's choice of `ReductionPlan` for a site has a corresponding
**implicit binding** to the IR's `ValueFormat::ExactAccumulator`
domain at that site's output. F-B5 emits
`ValueFormat::ExactAccumulator` for `RouterScore`, `ExpertCandidate`,
`LogitVector`, etc.; F-B7's plan is the bridge from "logical exact
accumulator" to "implementation choice."

```text
plan_implies_value_format_compatibility(plan, value_format):
  value_format = ExactAccumulator
  ∧ plan ∈ {SingleI16, ChunkedI16 { _ }, RenormLoop { _ }}
  -- All three plans realize ExactAccumulator under their declared
  -- proof obligations. They differ in how the implementation reads
  -- back; F-B8 (StoragePlan) and F-B13 (GbSchedIR) handle that
  -- read-back.
```

This is **not** a value-format change. F-B5's IR keeps
`ValueFormat::ExactAccumulator` at the IR level; F-B7's plan is
metadata on the *site*, not on the *value*. F-B8 reads the plan to
size scratch; F-B13 reads the plan to choose tile shape.

```text
F-B7-NoValueFormatChange:
  No RangePlan field changes the IR's ValueFormat for any value.
  Every value declared as ExactAccumulator stays ExactAccumulator
  in the IR. The plan refines the *implementation* of the
  reduction, not the *type* of its output.
```

Amends planv0: `planv0.md` line 1656–1660 does not specify whether
`ReductionPlan` changes the IR's value format. This RFC pins that
it does not. F-B5 §A29's discussion of `ValueFormat::ExactAccumulator`
already takes this position; this RFC restates it as a normative
invariant.

### 9.9 No silent integer-width expansion

F-B7 uses `u128` internally for every scalar product, sum, and
intermediate quantity. The certificate JSON represents proof bounds
as `u64`; the implementation envelope is still the declared i16/i32
bound recorded in the proof. A value that does not fit the JSON field
width is
`RANGE-INTEGER-OVERFLOW-DURING-PROOF` (Hard).

```text
F-B7-CheckedArithmetic:
  Every multiplication and addition during certificate construction
  uses checked arithmetic on u128 internally. A field is only set to
  a u32 / u64 value after the value has been checked to fit.

F-B7-NoSilentExpansion:
  No certificate field is silently widened to a larger integer width
  to make a proof verify. If the declared envelope (i16 for SingleI16
  per-chunk/per-tile; i32 for ChunkedI16 cross-chunk) cannot hold the
  bound, the certificate is Failed; an alternate plan must be tried,
  or RANGE-NO-PROVEN-PLAN-WITHIN-CEILING is emitted.
```

### 9.10 RangePlan does not feed ObservationPlan

Symmetric to §8.10: F-B6 does not consume `RangePlan`. The two
stages are independent at the data-flow level.

### 9.11 ReductionSiteId minting belongs to F-B4

F-B7 does not mint new `ReductionSiteId` values. Per F-B3/F-B5
§13.4, every `ReductionSiteId` is already minted by F-B4 in
`static_budget.json`'s `ReductionSiteProjection` entries. F-B5
carries the id forward via `GbNode.reduction_site`. F-B7 looks up
the id and the matching projection.

A site that exists in `g.nodes` but has no matching projection in
`static_budget` is `RANGE-SITE-MISSING-FROM-STATIC-BUDGET` (Hard).
This is a F-B4 / F-B5 join bug; F-B7 surfaces it.

A site that exists in `static_budget` but has no matching node in
`g.nodes` is `RANGE-STATIC-BUDGET-SITE-ORPHANED` (Hard). Symmetric
case.

### 9.12 Pure-function shape: F-B7's pure core reads QuantGraph by hash only

F-B7's pure core reads `QuantGraph` only through
`i.quant_graph_self_hash` (for identity recording in
`RangePlanIdentity`) and through the transitive
`ReductionSiteProjection.accumulator_domain` field that F-B4 already
populated. F-B7 does **not** read `QuantGraph.tensors[*].quant_format`
directly — that read happened in F-B4 and produced the projection.

This keeps F-B7's input surface minimal (just `GbInferIR`,
`StaticBudgetReport`, and `RangePolicyProjection`) and makes the
StageCache key K5 small.

```text
F-B7-MinimalSemanticInputs:
  build_range_plan_core reads only:
    - GbInferIR (specifically: nodes with reduction_site = Some(_),
      identity.determinism, identity.quant_graph_self_hash)
    - StaticBudgetReport (specifically: reduction_sites[*])
    - RangePolicyProjection
  It does NOT read QuantGraph directly. It records
  quant_graph_self_hash for audit only.

Cache invalidators such as pass_version_range_plan,
crate_feature_set_hash, range_plan_schema_hash, and
range_cert_schema_hash are part of K5 but are not semantic inputs to
the pure plan-selection function.
```

This is also the reason F-B7 emits a smaller report than F-B6 — it
has fewer inputs to summarize.

## 10. Report schemas, normalized

### 10.1 `semantic_checkpoint_schema.json` (build-active re-emit; schema id `build_active_semantic_checkpoint_schema.v1`)

`semantic_checkpoint_schema.json` is the **build-active subset**
re-emit of the artifact's `SemanticCheckpointSchema`. It is a
canonical product-bearing report whose body lists only the
checkpoints THIS build honors, with their per-checkpoint encoding
and attachment NodeId. The schema id is
`build_active_semantic_checkpoint_schema.v1`; the file name retains
`semantic_checkpoint_schema.json` for consumer compatibility.

```text
ReportEnvelope<SemanticCheckpointSchemaReEmitBody>

SemanticCheckpointSchemaReEmitBody :=
  {
    input_identity: {
      observation_plan_self_hash: Option<Hash256>,
      original_schema_hash: Hash256,            -- the artifact-side hash
      infer_ir_self_hash: Hash256,
      quant_graph_self_hash: Hash256,
      artifact_aux_hash: Hash256,
      determinism: DeterminismClass,
      workload_id: WorkloadId
    },
    result: Option[{
      schema_hash: Hash256,                      -- new build-active hash
      checkpoints: List[ReEmittedCheckpointEntry],
      build_active_count: u16,
      mandatory_count: u16,
      optional_count: u16
    }],
    diagnostics: List[ValidationDiagnosticRecord]
  }

ReEmittedCheckpointEntry :=
  {
    id: SemanticCheckpointId,
    artifact_role: SemanticCheckpointRole,       -- Mandatory | Optional
    original_checkpoint_metadata: SemanticCheckpointMetadata,
      -- copied verbatim from the artifact-side SemanticCheckpointSchema
    encoding: ObservationEncoding,
    source: ObservationSource,
    attachment_node_id: NodeId,
    attachment_anchor: SemanticAnchor,
    canonical_provenance_tuple: CanonicalProvenanceTuple
  }
```

Semantic invariants:

```text
SCRE-1: schema = "build_active_semantic_checkpoint_schema.v1"
SCRE-1a: outcome = Passed ⇒ input_identity.observation_plan_self_hash = Some(_)
SCRE-2: outcome = Passed ⇔ result = Some(_) ∧ no Hard diagnostics
SCRE-3: outcome = Failed ⇔ result = None ∧ ≥ 1 Hard diagnostic
SCRE-4: result.checkpoints sorted by SemanticCheckpointId canonical order
SCRE-5: result.build_active_count = len(result.checkpoints)
SCRE-6: result.mandatory_count + result.optional_count = build_active_count
SCRE-7: every entry's id ∈ original schema's checkpoint id set
SCRE-7a: every entry.source references valid GbInferIR entities and
         is identical to the corresponding ObservationPlan.semantic
         source binding
SCRE-8: result.schema_hash =
          DomainHash("gbf-codegen", "BuildActiveCheckpointSchema",
                     "build_active_semantic_checkpoint_schema.v1",
                     CanonicalJson(result.checkpoints))
SCRE-9: report_self_hash round-trips
```

### 10.2 `operational_probe_schema.json`

The build-active operational probe schema. Read by `gbf-debug` to
render trace events. Read by `gbf-runtime::trace` to install probe
hooks.

```text
ReportEnvelope<OperationalProbeSchemaBody>

OperationalProbeSchemaBody :=
  {
    input_identity: {
      observation_plan_self_hash: Option<Hash256>,
      infer_ir_self_hash: Hash256,
      quant_graph_self_hash: Hash256,
      determinism: DeterminismClass,
      observability_mode: ObservabilityMode,
      trace_budget: TraceBudget,
      profile_id: CompileProfileId,
      workload_id: WorkloadId
    },
    result: Option[{
      schema_hash: Hash256,
      probes: List[ProbeSchemaEntry],
      metrics: List[MetricSchemaEntry],
      probe_count: u16,
      metric_count: u16,
      per_class_probe_weight_total: PerClassWeightTotal,
      per_class_metric_weight_total: PerClassWeightTotal,
      per_class_total_weight: PerClassWeightTotal
    }],
    diagnostics: List[ValidationDiagnosticRecord]
  }

ProbeSchemaEntry :=
  {
    instance_id: ProbeInstanceId,
    probe_id: TraceProbeId,
    level: ProbeLevel,
    budget_class: ProbeBudgetClass,
    event_shape: TraceEventShape,
    source: ProbeSource,
    weight: u16
  }

MetricSchemaEntry :=
  {
    metric: MetricId,
    aggregation: MetricAggregation,
    source: MetricSource,
    budget_class: ProbeBudgetClass,
    weight: u16
  }

PerClassWeightTotal :=
  {
    required:    u32,
    important:   u32,
    diagnostic:  u32,
    best_effort: u32
  }
```

Semantic invariants:

```text
OPS-1: schema = "operational_probe_schema.v1"
OPS-1a: outcome = Passed ⇒ input_identity.observation_plan_self_hash = Some(_)
OPS-2: outcome = Passed ⇔ result = Some(_) ∧ no Hard diagnostics
OPS-3: result.probes sorted by
       `(TraceProbeId canonical order, source_fingerprint canonical order)`
OPS-4: result.metrics sorted by MetricId canonical order
OPS-5: result.probe_count = len(result.probes)
OPS-6: result.metric_count = len(result.metrics)
OPS-7: per_class_probe_weight_total[c] =
         sum_over_probes(weight | budget_class == c)
OPS-7a: per_class_metric_weight_total[c] =
         sum_over_metrics(weight | budget_class == c)
OPS-7b: per_class_total_weight[c] =
         per_class_probe_weight_total[c] + per_class_metric_weight_total[c]
OPS-8: result.schema_hash =
         DomainHash("gbf-codegen", "OperationalProbeSchema",
                    "operational_probe_schema.v1",
                    CanonicalJson({ probes, metrics }))
OPS-9: report_self_hash round-trips
```

### 10.3 `observation_plan.json`

The product-bearing report for `ObservationPlan`.

```text
ReportEnvelope<ObservationPlanReportBody>

ObservationPlanReportBody :=
  {
    input_identity: {
      infer_ir_self_hash: Hash256,
      quant_graph_self_hash: Hash256,
      semantic_checkpoint_schema_hash: Hash256,
      observation_policy_projection_hash: Hash256,
      static_budget_self_hash: Hash256,             -- audit
      policy_resolution_self_hash: Hash256,         -- audit
      compile_request_hash: Hash256,                -- audit
      artifact_aux_hash: Hash256,
      determinism: DeterminismClass,
      observability_mode: ObservabilityMode,
      trace_budget: TraceBudget,
      profile_id: CompileProfileId,
      workload_id: WorkloadId
    },
    result: Option[{
      product: ObservationPlan,                     -- the full product

      semantic_count: u16,                          -- review aid; derivable
      probe_count: u16,
      metric_count: u16,
      mandatory_semantic_count: u16,
      optional_semantic_count: u16,
      per_class_probe_count: PerClassCount,
      per_class_metric_count: PerClassCount,

      sc_re_emit_report_self_hash: Hash256,
      operational_probe_schema_report_self_hash: Hash256,
      observation_plan_self_hash: Hash256
    }],
    diagnostics: List[ValidationDiagnosticRecord]
  }

PerClassCount :=
  {
    required: u16, important: u16, diagnostic: u16, best_effort: u16
  }
```

Semantic invariants:

```text
OP-1: schema = "observation_plan.v1"
OP-2: outcome = Passed ⇔ result = Some(_) ∧ no Hard diagnostics
OP-3: outcome = Failed ⇔ result = None ∧ ≥ 1 Hard diagnostic
OP-4: result.semantic_count = len(result.product.semantic)
OP-5: result.probe_count = len(result.product.probes)
OP-6: result.metric_count = len(result.product.metrics)
OP-7: result.product.semantic sorted by SemanticCheckpointId
OP-8: result.product.probes sorted by
      `(TraceProbeId, ProbeInstanceId.source_fingerprint)`
OP-9: result.product.metrics sorted by MetricId
OP-10: result.observation_plan_self_hash =
        DomainHash("gbf-codegen", "ObservationPlan", "observation_plan.v1",
                   CanonicalJson(result.product))
OP-11: report_self_hash round-trips
OP-12: every diagnostic d has d.severity = Hard
OP-13: no diagnostic carries a RepairProposal source
OP-14: under ObservabilityMode = Invariant, projected trace cost
       fits TraceBudget
```

### 10.4 `range_plan.json`

The product-bearing report for `RangePlan`.

```text
ReportEnvelope<RangePlanReportBody>

RangePlanReportBody :=
  {
    input_identity: {
      infer_ir_self_hash: Hash256,
      quant_graph_self_hash: Hash256,
      static_budget_self_hash: Hash256,
      range_policy_projection_hash: Hash256,
      policy_resolution_self_hash: Hash256,         -- audit
      compile_request_hash: Hash256,                -- audit
      artifact_aux_hash: Hash256,
      determinism: DeterminismClass
    },
    result: Option[{
      product: RangePlan,                           -- the full product

      entry_count: u32,                             -- review aid; derivable
      single_i16_count: u32,
      chunked_i16_count: u32,
      renorm_loop_count: u32,

      effective_ceiling_histogram: BTreeMap<ReductionPlanCeiling, u32>,
      ceiling_provenance_histogram: BTreeMap<ReductionCeilingProvenanceTag, u32>,

      range_cert_report_self_hash: Hash256,
      range_plan_self_hash: Hash256
    }],
    diagnostics: List[ValidationDiagnosticRecord]
  }
```

Semantic invariants:

```text
RP-1: schema = "range_plan.v1"
RP-2: outcome = Passed ⇔ result = Some(_) ∧ no Hard diagnostics
RP-3: outcome = Failed ⇔ result = None ∧ ≥ 1 Hard diagnostic
RP-4: result.entry_count = len(result.product.entries)
RP-5: result.single_i16_count + result.chunked_i16_count + result.renorm_loop_count = result.entry_count
RP-6: result.product.entries sorted by ReductionSiteId
RP-7: result.range_plan_self_hash =
        DomainHash("gbf-codegen", "RangePlan", "range_plan.v1",
                   CanonicalJson(result.product))
RP-8: result.range_cert_report_self_hash matches the emitted cert report's self-hash
RP-9: report_self_hash round-trips
RP-10: every diagnostic d has d.severity = Hard
RP-11: no diagnostic carries a RepairProposal source
```

### 10.5 `certs/range.cert.json`

The independent verification certificate. Lives in
`certs/range.cert.json` per `planv0.md` line 2825. Read by
`gbf-verify` for slow reference re-verification.

```text
ReportEnvelope<RangeCertBody>

RangeCertBody :=
  {
    identity: RangeCertIdentity,
    cert_outcome: CertOutcome,
    certificates: List[CertifiedReduction],
    site_to_certificate_index: BTreeMap<ReductionSiteId, u32>,
    diagnostics: List[ValidationDiagnosticRecord]
  }

RangeCertIdentity :=
  {
    range_plan_self_hash: Option<Hash256>,
    infer_ir_self_hash: Hash256,
    quant_graph_self_hash: Hash256,
    static_budget_self_hash: Hash256,
    determinism: DeterminismClass
  }

cert_for_site(body, site) :=
  body.certificates[body.site_to_certificate_index[site]]
```

The certificate body carries a dedicated verification outcome:

```rust
pub enum CertOutcome {
    /// Every certificate in the list is non-Failed and verifies.
    Verified,
    /// At least one certificate is Failed.
    Failed,
}
```

Semantic invariants:

```text
RC-1: schema = "range.cert.v1"
RC-1a: envelope.outcome = Passed ⇒ body.identity.range_plan_self_hash = Some(_)
RC-1b: envelope.outcome = Failed may use None when Stage 5 failed before a
       successful RangePlan product hash could be constructed.
RC-2: envelope.outcome = Passed ⇔
        body.cert_outcome = Verified
        ∧ no AccumulatorCertificate::Failed entries
        ∧ no Hard diagnostics

RC-3: envelope.outcome = Failed ⇔
        body.cert_outcome = Failed
        ∧ (≥ 1 AccumulatorCertificate::Failed entry
           OR ≥ 1 Hard diagnostic)
RC-4: certificates sorted by ReductionSiteId
RC-5: site_to_certificate_index is total over certificates and bijective
RC-6: ∀ certified ∈ certificates where certified.proof != Failed:
        verifies(certified.proof, certified.plan, certified.facts)
        holds (per §9.6)
RC-7: ∀ certified where certified.proof = Failed { attempted_plan, ... }:
        certified.proof.witness is consistent with certified.proof.proof_state
        ∧ certified.plan = attempted_plan
        ∧ certified.facts.site = certified.site
        ∧ no successful RangePlan entry is implied by this failed
          certificate when the envelope outcome is Failed.
RC-8: report_self_hash round-trips
RC-9: every diagnostic d has d.severity = Hard
RC-10: no diagnostic carries a RepairProposal source
```

The certificate is the load-bearing proof artifact of F-B7. The
`RangePlan` is the load-bearing planning artifact consumed by later
pipeline stages; `certs/range.cert.json` proves that the planning
artifact's per-site choices are valid. Independent verification:

```text
Independent_Verify_Range_Cert(cert.json):
  parse cert.json
  verify report_self_hash round-trips
  for each certified in certificates:
    if certified.proof is Failed:
      pass-through (failure is its own evidence)
    else:
      re-run verifies(certified.proof,
                      certified.plan,
                      certified.facts)
      and assert it holds
```

`gbf-verify` does this without any access to `gbf-codegen` internals
— the cert JSON contains every load-bearing scalar.

### 10.6 Cross-report consistency

```text
R-Stage4-Triple:
  On Stage 4 success:
  observation_plan.json, semantic_checkpoint_schema.json (re-emit),
  and operational_probe_schema.json all share:
    - infer_ir_self_hash
    - quant_graph_self_hash
    - observation_plan_self_hash (same value)
  A successful driver emits the three together or none.
  On Stage 4 failure, the driver MUST emit observation_plan.json
  with outcome Failed and MAY emit any ancillary failure reports that
  were fully constructed before the failing class.

R-Stage5-Pair:
  On Stage 5 success:
  range_plan.json and range.cert.json share:
    - infer_ir_self_hash
    - quant_graph_self_hash
    - static_budget_self_hash
    - range_plan_self_hash (same value)
  A successful driver emits the pair together or none.
  On Stage 5 failure, the driver MUST emit range_plan.json with
  outcome Failed and SHOULD emit certs/range.cert.json when at least
  one per-site certificate attempt was constructed.
```

## 11. StageCache algebra

Stage 4 and Stage 5 keys, following the F-B2/F-B4 §7.8 +
F-B3/F-B5 §11 `DomainHash(crate, "StageCacheKey", schema_id, schema_version, canonical_json_bytes)`
rule.

Stage 4 key:

```text
StageCacheKeyHash(schema_id, schema_version, body) :=
  DomainHash("gbf-codegen", "StageCacheKey", schema_id, schema_version,
    CanonicalJson(body))

K4 :=
  StageCacheKeyHash("observation_plan.v1", schema_version, {
    infer_ir_self_hash,
    quant_graph_self_hash,                      -- transitive; stable
    semantic_checkpoint_schema_hash,
    observation_policy_projection_hash,
    pass_version_observation_plan,
    crate_feature_set_hash,
    observation_plan_schema_hash,
    build_active_semantic_checkpoint_schema_schema_hash,
    operational_probe_schema_schema_hash,
    probe_registry_hash,
    metric_registry_hash,
    trace_event_layout_registry_hash
  })
```

Stage 5 key:

```text
K5 :=
  StageCacheKeyHash("range_plan.v1", schema_version, {
    infer_ir_self_hash,
    quant_graph_self_hash,                      -- transitive; stable
    static_budget_self_hash,
    range_policy_projection_hash,
    pass_version_range_plan,
    crate_feature_set_hash,
    range_plan_schema_hash,
    range_cert_schema_hash
  })

-- where
range_policy_projection_hash :=
  DomainHash("gbf-codegen", "RangePolicyProjection", "range_plan.v1",
    CanonicalJson(RangePolicyProjection))

observation_policy_projection_hash :=
  DomainHash("gbf-codegen", "ObservationPolicyProjection",
    "observation_plan.v1",
    CanonicalJson(ObservationPolicyProjection))
```

Notes:

* `observation_policy_projection_hash` carries
  `disabled_optional_probes` and `optional_probe_floor` and
  `trace_demotion`. Any change to those (e.g. via F-B16's
  `KnobDelta::DisableOptionalProbes`) bumps the projection hash
  and invalidates K4.
* `range_policy_projection_hash` carries
  `reduction_ceiling_overrides` and `reduction_ceiling`. Any
  change to those (e.g. via F-B16's
  `KnobDelta::RaiseReductionCeiling`) bumps the projection hash
  and invalidates K5.
* `policy_resolution_self_hash` and `compile_request_hash` are
  **not** in K4 or K5 — they are audit-only (per F-B3/F-B5
  §9.10, §A76). Drift in unrelated policy fields does not
  invalidate the caches.

Cache laws (inherit from F-B2/F-B4 §7.8 + F-B3/F-B5 §11):

```text
C-Success-Stage4:
  Stage4 result Passed ⇒ StageCache may store
  `ObservationPlanCoreProduct`, not the emitted `ReportEnvelope`s.

C-NoFalseSuccess-Stage4:
  Stage4 result Failed ⇒ StageCache must not store success product

C-FailureMemo-Stage4:
  Stage4 result Failed ⇒ StageCache may memoize canonical failure
                         core bodies and diagnostics, not emitted
                         ReportEnvelope values. On replay, the driver
                         re-wraps failure bodies with the current build's
                         audit parents exactly as it does for success.

C-Success-Stage5:
  Stage5 result Passed ⇒ StageCache may store `RangePlanCoreProduct`,
  including the canonical certificate body, not the emitted envelopes.

C-NoFalseSuccess-Stage5, C-FailureMemo-Stage5:
  same shape, with K5 substituted; failure memo bundles
  range_plan and cert bodies plus diagnostics, not emitted envelopes,
  when the certificate body was constructed.

C-PassVersion:
  pass_version_observation_plan or pass_version_range_plan change
  ⇒ cache miss on K4 / K5 respectively

C-SchemaVersion:
  any schema body named in K4 or K5 changes ⇒ cache miss.
  For K4 this includes observation_plan.v1,
  build_active_semantic_checkpoint_schema.v1 re-emit, and
  operational_probe_schema.v1. For K5 this includes
  range_plan.v1 and range.cert.v1.

C-FeatureSet:
  crate feature set affecting layout/serde/behavior ⇒ cache miss

C-ReportRewrap-Stage4:
  A Stage 4 cache hit replays the byte-identical
  `ObservationPlanCoreProduct`, but the driver wraps it in fresh
  observation_plan.json / semantic_checkpoint_schema.json /
  operational_probe_schema.json reports whose audit-parent fields
  (policy_resolution_self_hash, compile_request_hash,
  static_budget_self_hash) match the current build.

C-ReportRewrap-Stage5:
  A Stage 5 cache hit replays the byte-identical RangePlan product
  AND the byte-identical certificates, but the driver re-wraps
  them in fresh range_plan.json / range.cert.json reports whose
  audit-parent fields are refreshed.

C-FailureReportRewrap:
  K4/K5 exclude audit-only parents. Therefore cached failure memo replay
  MUST refresh policy_resolution_self_hash, compile_request_hash, and
  other audit-parent fields in emitted failure reports. A cached failure
  memo is never emitted byte-for-byte unless the audit parents also match.
```

## 12. Diagnostic algebra

Inherits the F-B2/F-B4 §7.1 closed-enum surface with these
additions:

```text
ValidationOrigin (extension):
  | ObservationPlanConstruction
  | RangePlanConstruction

Stage4 owns codes with origin ObservationPlanConstruction:
  OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE
  OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE
  OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA
  OBSERVATION-CHECKPOINT-NOT-ATTACHABLE
  OBSERVATION-CHECKPOINT-AMBIGUOUS
  OBSERVATION-PROBE-ID-UNKNOWN
    -- emitted only when disabled_optional_probes names a TraceProbeId
    -- absent from the hash-bound ProbeRegistrySnapshot.
  OBSERVATION-REQUIRED-PROBE-DISABLED
  -- OBSERVATION-METRIC-ID-UNKNOWN reserved; v1 selects metrics only
  -- from MetricRegistrySnapshot and has no external metric-id list.
  OBSERVATION-METRIC-SOURCE-RESERVED-V1
  OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO
  OBSERVATION-PROBE-SOURCE-INVALID            -- source ref doesn't exist in g
  OBSERVATION-RESERVED-EFFECT-PROBE
  OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED
  OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED
  OBSERVATION-PROBE-CLASS-CAP-EXCEEDED
  OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED
  OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT
  OBSERVATION-DETERMINISM-MISMATCH            -- projection vs IR
  -- OBSERVATION-LOCKED-KNOB-DRIFT reserved; locked-knob drift is
  --   enforced by Stage 0.5, not Stage 4
  OBSERVATION-SC-HASH-MISMATCH                -- supplied schema sidecar
                                              -- differs from artifact_aux's
                                              -- recorded hash

Stage5 owns codes with origin RangePlanConstruction:
  RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1
  RANGE-TERM-COUNT-ZERO
  RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY
  RANGE-CEILING-VIOLATED-NO-RENORM-LOOP
  RANGE-NO-PROVEN-PLAN-WITHIN-CEILING
  RANGE-SITE-MISSING-FROM-STATIC-BUDGET
  RANGE-STATIC-BUDGET-SITE-ORPHANED
  RANGE-DUPLICATE-REDUCTION-SITE-ID
  RANGE-INTEGER-OVERFLOW-DURING-PROOF
  RANGE-CERT-MALFORMED                        -- deserialization invariant
  RANGE-CHUNK-LEN-ZERO
  RANGE-TILE-LEN-ZERO
  RANGE-BITEXACT-MID-REDUCTION-SATURATION-FORBIDDEN
  RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES
  RANGE-BITEXACT-RENORM-LOOP-RESERVED-V1
  RANGE-DETERMINISM-MISMATCH                  -- projection vs IR
  -- RANGE-LOCKED-KNOB-DRIFT reserved; locked-knob drift is enforced
  --   by Stage 0.5, not Stage 5
  RANGE-CEILING-OVERRIDE-INVALID-SELECTOR     -- selector doesn't resolve
  RANGE-CEILING-OVERRIDE-AMBIGUOUS
  RANGE-SITE-FACTS-INCONSISTENT
  RANGE-CHUNK-LEN-EXCEEDS-PROFILE-MAX
  RANGE-TILE-LEN-BELOW-PROFILE-MIN
  RANGE-TILE-LEN-EXCEEDS-PROFILE-MAX
  RANGE-TILE-LEN-EXCEEDS-U16
  RANGE-RENORM-STRATEGY-UNSUPPORTED-V1        -- renorm strategy outside
                                              -- {ExactPostBoundary,
                                              --  DynamicMargin {..}}
  RANGE-CAPS-INVALID
```

Severity:

```text
∀ d ∈ Stage4.diagnostics. d.severity = Hard
∀ d ∈ Stage5.diagnostics. d.severity = Hard
```

## 13. Cross-stage interactions

### 13.1 F-B2/F-B4 schema additions

This RFC requires `CompileProfileSpec` to carry a new
`range_caps: RangeCapsSpec` field:

```rust
pub struct RangeCapsSpec {
    pub profile_chunk_max: u16,           // 256 in v1
    pub profile_tile_max: u16,            // 256 in v1
    pub profile_tile_min: u16,            // 16 in v1
    pub renorm_strategy: RenormStrategyPolicy,
}

pub enum RenormStrategyPolicy {
    ExactPostBoundaryOnly,
    DynamicMargin { margin_q16_16: u32 },
}

// RangeCapsSpec invariants:
//   profile_chunk_max > 0
//   profile_tile_max > 0
//   profile_tile_min > 0
//   profile_tile_min <= profile_tile_max
//   if renorm_strategy = DynamicMargin { margin_q16_16 }:
//     margin_q16_16 < 0x1_0000
//
// Violations are rejected during Stage 0.5 profile validation. Stage 5
// assumes RangeCapsSpec is already valid.
```

And a new `observation_caps: ObservationProfileCaps` field on the
profile spec. Both fields ship in the four compile_profile_spec.v2 fixtures
(Bringup, Default, Trace, Recovery) with the default values introduced
by this RFC.

This is a profile-spec schema **extension** and a breaking change to
the v1 profile fixture shape unless Stage 0.5 provides an explicit
typed upgrade rule. In this RFC, no silent defaulting is allowed:
all four canonical profile fixtures must gain `range_caps` and
`observation_caps`, the profile schema/pass version must bump, and
`policy_resolution.json` must be re-emitted.

ProfileSpec schema bump:
  compile_profile_spec.v1 -> compile_profile_spec.v2

Stage 0.5 accepts compile_profile_spec.v2 for this chunk. It does
not auto-upgrade v1 fixtures; v1 fixtures are rejected unless an
explicit typed upgrade rule is added by an amendment.

Fixtures without the new required fields fail to load because the
required fields are missing under the profile schema's required-field
validation. `deny_unknown_fields` rejects extra unknown fields; it is
not the mechanism that rejects missing fields.

Amends F-B2/F-B4 §7.4: the four v1 profile specs gain
`range_caps` and `observation_caps` fields. The CompileKnobs
schema is unchanged.

### 13.2 F-B3/F-B5 input shape

This RFC reads `GbInferIRProduct` (the full product, including the
embedded `GbInferIR` and the report's `result`) and depends on the
following invariants from F-B3/F-B5 (matching the landed
implementation in `gbf-codegen/src/s3/infer_ir.rs`):

* `GbInferIRProduct.infer_ir_self_hash` — the canonical product hash
  used as Stage 4/5 input. `GbInferIR.identity: InferIrIdentity`
  contains `quant_graph_self_hash`, `infer_ir_policy_projection_hash`,
  `static_budget_self_hash`, `requested_runtime_modes_hash`,
  `determinism: DeterminismClass`, and `topological_order_hash`.
  The product-level `infer_ir_self_hash` is computed over the
  serialized `GbInferIR` and lives on `GbInferIRProduct`, not on
  `InferIrIdentity`.
* `g.anchors: NodeAnchorMap = BTreeMap<NodeId, SemanticAnchor>` —
  every node has a serialized `SemanticAnchor { anchor_id: Hash256 }`
  per F-B3/F-B5 §2.12. The anchor id is a domain-separated hash over
  `(quant_graph_self_hash, node_id, op_tag, canonical_provenance_tuple)`;
  `SemanticAnchor` itself does not carry the tuple.
* `g.nodes[*].reduction_site: Option<ReductionSiteId>` — set
  exactly on `ReductionSiteBearing` ops per F-B3/F-B5 §9.7a.
  In v1, `ReductionSiteId` is node-unique (see RP-Pre-7).
* `g.values[*].format: ValueFormat` — `ExactAccumulator` is the
  variant set on reduction-bearing op outputs. Other variants are
  `Quant { format: QuantFormat }`, `TokenIdDomain { vocab_size }`,
  `ExpertIdDomain { n_experts }`.
* `op_signature(op, q)` — every node satisfies its closed signature,
  enforced by F-B5's `validate_node_op_signature_and_bindings`.
* `g.provenance: InferIrProvenance` maps `NodeId →
  QuantGraphEntityRef`, `ValueId → ValueProducerRef`, and
  `EffectId → EffectProvenance`. The
  `CanonicalProvenanceTuple { op_tag, layer, expert,
  expert_weight_slot, norm_site, state_slot, residual_site,
  occurrence_index }` used by `compute_semantic_anchor` is not
  stored on `SemanticAnchor`; F-B6's anchor → checkpoint mapping
  (§8.5) recomputes or re-reads it from the same canonical source
  F-B5 used.

If F-B3/F-B5 changes any of these, this RFC must be amended.

### 13.2a Static budget access trait (Stage 2 surface for Stage 5)

F-B7's `SiteFactsBinding` class needs full per-site facts
(`term_count`, `input_max_abs_q`, `weight_max_abs_q`,
`bias_max_abs_q`, `accumulator_domain`). The landed Stage 2 report
body `StaticBudgetReportBody.projections.accumulator_maxima:
Vec<AccumulatorBound>` only carries `{ site, projected_max_abs,
i16_safe, i32_safe }` — it does not surface the per-site
projection fields F-B7 must copy.

F-B7 reads facts through an extension trait, parallel to
`StaticBudgetReductionSites` already landed on `StaticBudgetReport`:

```rust
pub trait StaticBudgetReductionSiteFacts {
    fn reduction_site_projection(&self, site: &ReductionSiteId)
        -> Option<&ReductionSiteProjection>;
}
```

To satisfy this trait, Stage 2 MUST either:

1. Embed `Vec<ReductionSiteProjection>` in `StaticBudgetReportBody`
   alongside `accumulator_maxima`; or
2. Retain `QuantGraphBudgetView` (or its `reduction_sites` slice)
   on `StaticBudgetReport` as a non-report side channel exposed via
   this trait.

This is a **required Stage 2 surface amendment**, not optional.
Without it, F-B7 cannot construct certificates from typed Stage 2
inputs without crossing into Stage 1 internals. The amendment is
backward-compatible with the `static_budget.v1` report schema if
Stage 2 attaches the projection slice as a non-report field
(option 2). Option 1 requires bumping `static_budget.v1` to
`static_budget.v2`.

Until this amendment lands, F-B7 reads from
`StaticBudgetReport.report.body.projections.accumulator_maxima` for
site enumeration and reads `ReductionSiteProjection` via the trait
above; tests stub the trait against fixtures.

### 13.3 F-B8 (StoragePlan) handshake

F-B8 (Stage 6) consumes `RangePlan` directly:

```text
F-B8 reads:
  - r.entries[*].plan to size scratch buffers per reduction site:
    SingleI16                  -> no logical chunk/renorm scratch requirement
    ChunkedI16 { chunk_len }   -> logical chunk structure with `chunk_len`
    RenormLoop { tile_len, renorm } -> logical renorm structure with
                                      `tile_len` and implementation
                                      recurrence metadata
  - r.entries[*].site_facts.term_count for total-term scratch
                                           when needed.

F-B8 also receives the `ObservationPlanProduct` by the normal
`PlanningReady(g, o, r)` pipeline state. Any cross-product build
manifest may record both `observation_plan_self_hash` and
`range_plan_self_hash`, but Stage 5 does not record Stage 4's hash.
```

The F-B8 RFC will define the exact scratch shape. This RFC pins
only that the per-site storage requirement is a function of
`e.plan` and `e.site_facts.term_count`, not of hidden range analysis.

### 13.4 F-B13 (GbSchedIR) handshake

F-B13 (Stage 10) consumes `RangePlan` for tile shape constraints:

```text
F-B13 reads:
  - r.entries[*].plan to constrain tile shape:
    SingleI16             -> tile shape unconstrained (single
                              accumulator per tile);
    ChunkedI16 { chunk_len } -> tile MUST honor chunk boundaries;
                                tile_len_in_terms is a multiple
                                of chunk_len.
    RenormLoop { tile_len, renorm } -> tile MUST honor renorm boundaries;
                                       tile_len_in_terms == tile_len;
                                       schedule emits the declared
                                       renorm recurrence.
  - observation_plan.probes[*].source for probe attachment slots
    in the schedule (when to emit trace events).
```

F-B13 may further refine tile shape against `ScheduleCostAnalysis`
inputs but cannot violate the RangePlan's chunk/tile constraints.

### 13.5 F-B14 (ScheduleCostAnalysis) handshake

F-B14 reads `ObservationPlan`'s `TraceBudget` projection to model
`trace_bytes_per_frame` cost envelopes (per `planv0.md` line 1320).
F-B14 also reads `RangePlan`'s entries to model accumulator cost
per reduction site (each plan has a different cycle envelope under
the calibration set).

This chunk does not own those models; F-B14 will define them. This
chunk's job is to make both products available by hash.

### 13.6 F-B16 (FeasibilityRefinementLoop) handshake

F-B16 is BLOCKED on an oracle question. When it lands, it will
read:

* `range.cert.json`'s per-site certificates to know which sites
  failed and why (`AccumulatorCertificate::Failed` carries
  `attempted_plan` and `proof_state`).
* `observation_plan.json`'s probe set to know which probes are
  active.

F-B16 will issue `KnobDelta::RaiseReductionCeiling` against
`reduction_ceiling_overrides` and `KnobDelta::DisableOptionalProbes`
against `disabled_optional_probes`. Both maps are pinned in the
F-B2/F-B4 schema; this chunk reads them on each iteration.

This RFC pins the **schema shape** F-B16 will mutate; it does not
implement F-B16 nor accept any `RepairProposal(_)` provenance
during the chunk.

### 13.7 F-C2 (ArtifactOracle) handshake

F-C2 reads the build-active `semantic_checkpoint_schema.json`
(re-emit) directly. Specifically:

* For each `(SemanticCheckpointId, EnvelopeGate)` pair in
  `ConformanceEnvelope.per_checkpoint`, F-C2 looks up the
  attachment NodeId in the build-active schema and the matching
  `ObservationEncoding`. The encoding determines which equality
  predicate F-C2 uses (BitExact byte equality vs Q8_8 envelope
  vs token-id equality).
* For checkpoints in `ConformanceEnvelope.per_checkpoint` that are
  **not** in the build-active schema, F-C2 either skips (under
  `Workload.optional`) or fails the conformance gate (under
  `Workload.required`).

This handshake unblocks M2's "checkpoint alignment against
ArtifactOracle at SemanticCheckpointId boundaries" commitment.

### 13.8 F-A8 (gbf-debug) handshake

`gbf-debug` reads `operational_probe_schema.json` to render
trace-event names, levels, and budget classes in the agent CLI.
The `TraceEventShape` field declares wire format; `gbf-debug`
implements decoders for the closed
`TraceEventPayloadLayout` set. Adding a new payload layout
requires a `gbf-policy` schema bump and a coordinated `gbf-debug`
release.

### 13.9 F-F1 (gbf-verify) handshake

`gbf-verify` reads `certs/range.cert.json` as an independent
slow-reference check. It re-runs the `verifies` predicate (§9.6)
on each non-Failed certificate; a mismatch is a verifier-side
diagnostic. `gbf-verify` does not depend on `gbf-codegen`; the
cert JSON's load-bearing scalars are sufficient.

### 13.10 F-B17 (StageCache) integration

F-A6.2 (StageCache infrastructure) is closed (`bd-3ll`). F-B6 and
F-B7 wire into `StageCache` directly via `K4`/`K5`. The
cross-cutting `F-B17` chunk later may add a uniform sweep, but no
per-stage wiring is missing here.

## 14. Task DAG, compressed

```text
Wave0 SchemaPrelude:
  T-B6.0 observation_plan.v1 + semantic_checkpoint_schema.v1 (re-emit)
         + operational_probe_schema.v1 ReportEnvelope binding
  T-B7.0 range_plan.v1 + range.cert.v1 ReportEnvelope binding
  Both depend on F-B2/F-B4's ReportEnvelope/canonical-JSON/self-hash
  machinery and on F-B3/F-B5's DomainHash convention.

Wave1 OPTypes:
  T-B6.1  ObservationPlan type + ObservationPlanIdentity
            (with infer_ir_self_hash, quant_graph_self_hash,
             semantic_checkpoint_schema_hash, ...).
  T-B6.2  SemanticObservation + ObservationSource + ObservationEncoding
            + SemanticCheckpointRole closed enums.
  T-B6.3  OperationalProbe + ProbeSource + ProbeLevel + TraceEventShape
            + TraceEventPayloadLayout closed enums (with stable_id).
  T-B6.4  MetricProbe + MetricSource + MetricAggregation closed enums.
  T-B6.5  AnchorAttachmentTable + ObservationProvenance maps.
  T-B6.6  ObservationPolicyProjection + ObservationProfileCaps +
            LockedObservationKnobs (and their hash).
  T-B6.7  Profile-spec extension: observation_caps field on
            CompileProfileSpec for Bringup / Default / Trace / Recovery.
  T-B6.8  Probe registry surface in gbf-policy::trace::PROBE_REGISTRY
            (consumed by Stage 4; this chunk only validates against it).
  T-B6.9  Metric registry surface in gbf-policy::metrics::METRIC_REGISTRY
            (consumed by Stage 4; this chunk only validates against it).

Wave2 OPConstruction:
  T-B6.10 IdentityBinding (incl. workload_id, observability_mode,
            trace_budget).
  T-B6.11 SchemaIngestion + Mandatory/Optional partition.
  T-B6.12 BuildFeasibilityFilter (per checkpoint id ↔ canonical
            anchor existence).
  T-B6.13 SemanticSelection (selected_semantic union per §8.4).
  T-B6.14 SemanticAnchorBinding via anchor_to_checkpoint (§8.5).
  T-B6.15 ObservationEncodingBinding via encoding_for (§8.6).
  T-B6.16 ProbeRegistryFilter + ProbeBudgetGovernance
            (optional_probe_floor, trace_demotion drop set,
             disabled_optional_probes empty pre-F-B16).
  T-B6.17 ProbeOrdering + per-class weight cap check.
  T-B6.18 MetricRegistryFilter + MetricSelection + MetricOrdering.
  T-B6.19 AnchorTableBinding + ProvenanceBinding.
  T-B6.20 SchemaReEmit (build-active subset of SemanticCheckpointSchema)
            + canonical sort + self-hash.
  T-B6.21 OperationalProbeSchemaEmit + canonical sort + self-hash.
  T-B6.22 InvariantBudgetCheck (Invariant mode; budget bust diagnostic).
  T-B6.23 SelfConsistency cross-class checks (OP-SC-1..OP-SC-18).
  T-B6.24 observation_plan.v1 schema + product-bearing report
            + semantic validator + tests.
  T-B6.25 semantic_checkpoint_schema.v1 (re-emit) schema + validator + tests.
  T-B6.26 operational_probe_schema.v1 schema + validator + tests.
  T-B6.27 StageCache key K4 (DomainHash form;
            observation_policy_projection_hash) + success
            + failure-memo.
  T-B6.28 fixture: synthetic dense + synthetic routed Toy0/Toy1
            ObservationPlan fixtures (with all reject classes covered).
  T-B6.29 build_observation_plan_core (pure) / run_stage4 (driver) split.
  T-B6.30 Wave-0-style profile fixtures: observation_caps for the
            four canonical profiles (no shared math beyond probe caps).
  T-B6.31 Sequence-state probe attachment reserved-not-emitted: tests
            for OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED and
            OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED.

Wave3 RPTypes:
  T-B7.1  RangePlan type + RangePlanIdentity (with infer_ir_self_hash,
            quant_graph_self_hash, static_budget_self_hash,
            range_policy_projection_hash, determinism).
  T-B7.2  RangePlanEntry + ReductionPlan closed enum + ReductionSiteFacts.
  T-B7.3  ReductionCeilingProvenance (Global / LayerOverride / SiteOverride).
  T-B7.4  AccumulatorCertificate closed enum + AccumulatorProofState +
            AccumulatorFailureWitness + RenormStrategy.
  T-B7.5  RangePolicyProjection + LockedRangeKnobs (and their hash).
  T-B7.6  Profile-spec extension: range_caps field on CompileProfileSpec.

Wave4 RPConstruction:
  T-B7.7  IdentityBinding + determinism agreement check.
  T-B7.8  ReductionSiteEnumeration over g.nodes.
  T-B7.9  SiteFactsBinding (copy from F-B4 reduction_sites; checked u128).
  T-B7.10 EffectiveCeilingBinding (most-specific-wins).
  T-B7.11 PlanCandidateGeneration (admissible_under_ceiling).
  T-B7.12 PlanLengthSelection (closed rule §9.5).
  T-B7.13 CertificateConstruction (per-variant builders, checked u128).
  T-B7.14 PlanChoice (smallest-admissible-with-verified-cert).
  T-B7.15 ProvenanceBinding (site_to_node + site_to_qg).
  T-B7.16 CanonicalSort + SelfConsistency cross-class checks
            (RP-SC-1..RP-SC-12).
  T-B7.17 range_plan.v1 schema + product-bearing report + validator + tests.
  T-B7.18 range.cert.v1 schema + validator + tests
            (independent verifier predicate).
  T-B7.19 StageCache key K5 (DomainHash form;
            range_policy_projection_hash) + success + failure-memo.
  T-B7.20 fixture: synthetic dense + synthetic routed Toy0/Toy1
            RangePlan fixtures + degenerate cases (chunk_len = 0,
            tile_len = 0, BitExact-divisibility-fail,
            no-proven-plan-within-ceiling).
  T-B7.21 build_range_plan_core (pure) / run_stage5 (driver) split.

Wave5 ReviewPacket:
  T-B6.32 F-B6 review-packet sub-bundle (under
            docs/review/f-b6-f-b7/ — shared with F-B7).
  T-B7.22 F-B7 review-packet sub-bundle.
  T-B6.33 + T-B7.23 chunk regen / verify scripts under
            scripts/review/f-b6-f-b7/.
```

DAG law:

```text
Wave0 → { Wave1, Wave3 }
Wave1 → Wave2
Wave3 → Wave4
{ Wave2, Wave4 } → Wave5

Wave1 (OPTypes) is independent of Wave3 (RPTypes); they may be
implemented in parallel once Wave0 lands.
Wave2 (OPConstruction) depends on Wave1.
Wave4 (RPConstruction) depends on Wave3.
Wave2 and Wave4 are independent and may proceed in parallel.

T-B6.7 (profile-spec observation_caps extension) and T-B7.6
(profile-spec range_caps extension) are coordinated:
they bump the profile spec schema together to keep one re-emission
of policy_resolution.json per chunk.
```

Feature merge law:

```text
F-B6 must merge atomically with profile-spec observation_caps fixture
update.
F-B7 must merge atomically with profile-spec range_caps fixture update.
Either F-B6 or F-B7 may merge first.
F-B8 (next chunk) consumes both products by hash; both must merge
before F-B8 begins.
F-C2 (oracle) gains an explicit dependency edge to bd-1y0 (F-B6) once
F-B6 lands; the build-active checkpoint subset is required for
checkpoint-aligned diffing.
```

## 15. Rejection classes (closure gate)

This chunk closes only when every class below is exercised by a
fixture.

### 15.1 F-B6 reject classes

```text
OP-Reject-1:  OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE
                (artifact-mandatory ckpt has no canonical anchor in g)
OP-Reject-2:  OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE
                (workload-required ckpt not in build_feasible_set)
OP-Reject-3:  OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA
                (a workload-required id is not in
                 SemanticCheckpointSchema.checkpoints)
OP-Reject-4:  OBSERVATION-CHECKPOINT-NOT-ATTACHABLE
                (no anchor in NodeAnchorMap matches the checkpoint id)
OP-Reject-5:  OBSERVATION-CHECKPOINT-AMBIGUOUS
                (more than one anchor matches; unreachable per
                 single-token convention but covered by a fixture
                 in case of future amendment)
OP-Reject-6:  OBSERVATION-PROBE-ID-UNKNOWN
                (disabled_optional_probes names an id absent from
                 ProbeRegistrySnapshot)
OP-Reject-7:  reserved
                (metric ids are selected only from MetricRegistrySnapshot
                 in v1; no external metric-id list exists)
OP-Reject-8:  OBSERVATION-PROBE-SOURCE-INVALID
                (NodeId / ValueId / EffectId / Anchor not in g)
OP-Reject-9:  OBSERVATION-RESERVED-EFFECT-PROBE
                (probe targets an EffectClass outside the v1 emit set)
OP-Reject-10: OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED
                (probe targets SequenceState { .. }; reserved-not-emitted)
OP-Reject-11: OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED
                (probe targets FaultBoundary; reserved-not-emitted)
OP-Reject-12: OBSERVATION-PROBE-CLASS-CAP-EXCEEDED
                (per-class weight total > profile cap)
OP-Reject-13: OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED
                (Invariant + projected trace cost > TraceBudget)
OP-Reject-14: OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT
                (workload override produces an encoding outside the
                 allowed set for that checkpoint id)
OP-Reject-15: OBSERVATION-DETERMINISM-MISMATCH
                (op_policy_projection.determinism_class disagrees
                 with g.identity.determinism)
OP-Reject-16: reserved
                (locked-knob drift is enforced by Stage 0.5; Stage 4
                 records locked knob bits for provenance only)
OP-Reject-17: OBSERVATION-SC-HASH-MISMATCH
                (provided SemanticCheckpointSchema's hash differs
                 from artifact_aux's recorded sidecar hash)
OP-Reject-18: OBSERVATION-REQUIRED-PROBE-DISABLED
                (disabled_optional_probes contains a Required probe id)
```

### 15.2 F-B7 reject classes

```text
RP-Reject-1:  RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY
                (no SingleI16 plan verifies under SingleI16Only ceiling)
RP-Reject-2:  RANGE-CEILING-VIOLATED-NO-RENORM-LOOP
                (no SingleI16/ChunkedI16 verifies under AllowChunkedI16)
RP-Reject-3:  RANGE-NO-PROVEN-PLAN-WITHIN-CEILING
                (every admissible plan's certificate is Failed)
RP-Reject-4:  RANGE-SITE-MISSING-FROM-STATIC-BUDGET
                (g.node has reduction_site = Some(rsid) but
                 static_budget has no matching projection)
RP-Reject-5:  RANGE-STATIC-BUDGET-SITE-ORPHANED
                (static_budget has projection for rsid but no g.node
                 references it)
RP-Reject-6:  RANGE-INTEGER-OVERFLOW-DURING-PROOF
                (per_term_abs_max or sum_bound exceeds u64 internally)
RP-Reject-7:  RANGE-CERT-MALFORMED
                (a deserialized cert violates verifies invariants)
RP-Reject-8:  RANGE-CHUNK-LEN-ZERO
                (chunked plan with chunk_len = 0)
RP-Reject-9:  RANGE-TILE-LEN-ZERO
                (renorm plan with tile_len = 0)
RP-Reject-10: RANGE-BITEXACT-MID-REDUCTION-SATURATION-FORBIDDEN
                (BitExact + RenormLoop with non-ExactPostBoundary
                 strategy, OR ChunkedI16 with mid-chunk saturation)
RP-Reject-11: RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES
                (BitExact + ChunkedI16 with chunk_len not dividing
                 term_count)
RP-Reject-12: RANGE-DETERMINISM-MISMATCH
                (range_policy_projection.determinism vs
                 g.identity.determinism)
RP-Reject-13: reserved
                (locked-knob drift is enforced by Stage 0.5; Stage 5
                 records locked knob bits for provenance only)
RP-Reject-14: RANGE-CEILING-OVERRIDE-INVALID-SELECTOR
                (a Site override references an unknown ReductionSiteId,
                 or a Layer override references a layer not in g.model_spec_summary)
RP-Reject-15: RANGE-SITE-FACTS-INCONSISTENT
                (optional explanatory maxima in ReductionSiteFacts
                 contradict the authoritative per_term_abs_max_q copied
                 from Stage 2; Stage 5 never replaces the authoritative
                 bound by recomputing it)
RP-Reject-16: RANGE-CHUNK-LEN-EXCEEDS-PROFILE-MAX
                (chunk_len > profile_chunk_max from RangeCapsSpec)
RP-Reject-17: RANGE-TILE-LEN-BELOW-PROFILE-MIN
                (tile_len < profile_tile_min from RangeCapsSpec)
RP-Reject-18: RANGE-RENORM-STRATEGY-UNSUPPORTED-V1
                (a strategy outside {ExactPostBoundary, DynamicMargin})
RP-Reject-19: RANGE-TERM-COUNT-ZERO
                (static_budget projects a reduction site with
                 term_count = 0)
```

Each reject class is gated by a typed fixture under
`fixtures/observation_plan/reject/` or
`fixtures/range_plan/reject/`.

## 16. Proof obligations

```text
O1 OP/RP determinism:
  Same inputs generate byte-identical observation_plan.json,
  semantic_checkpoint_schema.json (re-emit), operational_probe_schema.json,
  range_plan.json, and certs/range.cert.json across two clean
  regenerations.

O2 Self-hash + product round-trip:
  All five reports and their embedded products round-trip through
  parse → canonicalize → semantic validation → self-hash.

O3 OP rejection completeness:
  Every OP-Reject-* class has a fixture and typed diagnostic.

O4 RP rejection completeness:
  Every RP-Reject-* class has a fixture and typed diagnostic.

O5 No checkpoint / probe / metric creation:
  ObservationPlan.semantic[*].checkpoint ⊆
    SemanticCheckpointSchema.checkpoints[*].id.
  ObservationPlan.probes[*].probe_id ⊆ PROBE_REGISTRY.
  ObservationPlan.metrics[*].metric ⊆ METRIC_REGISTRY.

O6 Probe/metric budget governance:
  ∀ probe p ∈ probes.
    p.budget_class ≥ optional_probe_floor
    ∧ p.budget_class ∉ trace_demotion_drop_set(trace_demotion).
  ∀ metric m ∈ metrics.
    m.budget_class ≥ optional_probe_floor
    ∧ m.budget_class ∉ trace_demotion_drop_set(trace_demotion).
  ∀ class c.
    sum_over_probes(p.weight | p.budget_class = c)
    + sum_over_metrics(m.weight | m.budget_class = c)
    ≤ profile_cap(c), except Required where the v1 cap is None.

O7 Mandatory checkpoint coverage:
  Mandatory(scs) ∩ build_feasible_set(g) ⊆ {entry.checkpoint | entry ∈ semantic}.

O8 Anchor attachment totality:
  ∀ entry ∈ semantic.
    entry.anchor ∈ g.anchors
    ∧ anchor_to_checkpoint(entry.anchor) = Some(entry.checkpoint).

O9 Active probes don't drift semantic contract:
  The semantic checkpoint attachment set is independent of
  optional_probe_floor, trace_demotion, disabled_optional_probes, and
  active probe selection. Two builds with different probe sets but the
  same SemanticCheckpointSchema, WorkloadObservationProjection, and
  GbInferIR topology have identical checkpoint ids, anchors, and sources.
  Encodings may still differ when compare_domain or determinism differs.

O10 Reduction-site coverage totality:
  ∀ node n ∈ g.nodes where n.reduction_site = Some(rsid):
    ∃ exactly one entry ∈ r.entries with site = rsid.
  ∀ entry ∈ r.entries:
    ∃ exactly one node n ∈ g.nodes with reduction_site = Some(entry.site).

O11 Plan choice is proof, not heuristic:
  ∀ entry ∈ r.entries.
    entry.plan ∈ admissible_under_ceiling(entry.effective_ceiling)
    ∧ verifies(cert_for_entry, entry.plan, entry.site_facts)
    ∧ ∀ plan p < entry.plan:
        p ∉ admissible_under_ceiling(entry.effective_ceiling)
        ∨ ¬verifies_for(p, entry.site_facts).

O12 Certificate independence:
  ∀ non-Failed cert ∈ certs.
    every load-bearing scalar (term_count, per_term_abs_max,
    sum_bound, total_abs_max, slack, chunk_len, tile_len) is in
    the JSON, and verifies(cert, plan, facts) re-runs purely from
    the JSON without any gbf-codegen access.

O13 BitExact reduction rules:
  q.identity.determinism = BitExact
  ⇒ ∀ entry ∈ r.entries.
      entry.plan = ChunkedI16 { chunk_len }
      ⇒ entry.site_facts.term_count % chunk_len = 0.
      entry.plan = RenormLoop { tile_len }
      ⇒ cert.renorm_strategy = ExactPostBoundary.

O14 No silent integer-width expansion:
  Every certificate field's value fits its declared width. Overflow
  during proof construction is RANGE-INTEGER-OVERFLOW-DURING-PROOF.

O15 Cache soundness:
  Failure memo is never usable as success product. Cache miss occurs
  on pass_version, schema, projection, or feature-set drift. Cache
  hit replays byte-identical canonical product.

O16 No hidden defaults:
  No OP/RP field is silently filled by a default; every value derives
  from a hash-bound input or fails loudly.

O17 No scheduling fusion:
  Each observation entry corresponds to exactly one checkpoint /
  probe / metric. Each range entry corresponds to exactly one
  reduction site.

O18 Pure-function shape:
  build_observation_plan_core and build_range_plan_core are pure
  functions of their typed inputs. Side effects are isolated in
  the run_stage4 / run_stage5 drivers.

O19 Stage 4 / 5 reports forbid repair provenance:
  No diagnostic in any of the five reports carries a
  RepairProposal source or any AuthorizedRelaxation operation.

O20 Reserved effect classes:
  No probe attaches to SequenceState { .. } or FaultBoundary in v1.
  When the sequence-state amendment lands, both classes' probe
  attachment becomes valid by amendment.

O21 Determinism class binding:
  RangePlan.identity.determinism = QuantGraph.identity.determinism =
  GbInferIR.identity.determinism =
  ArtifactCore.numeric_profile.determinism.
  ObservationPlan.identity.determinism is the same value.

O22 Stage independence:
  RangePlan does not read ObservationPlan.
  ObservationPlan does not read RangePlan.
  Stage 4 and Stage 5 may run in parallel.

O23 Profile-spec extension:
  CompileProfileSpec gains observation_caps and range_caps fields,
  populated for Bringup / Default / Trace / Recovery in v1 fixtures.
  Backward compatibility: profile fixtures without the new fields
  fail to load under required-field validation. `deny_unknown_fields`
  rejects extra fields, not missing required fields.
```

## 17. End-to-end theorem

```text
Theorem ObservationRangePipelineSoundness:

Given:
  Imported inputs i
  validate_artifact_and_request(i) = Ok(v)
  resolve_policy(v)                = Ok(p)
  build_quant_graph({v, p, ac, ss, resolved_blob_index}) = Ok(q)
  static_budget({p, q.budget_view, runtime_budget})      = Ok(b)   [fits = true]
  build_infer_ir({q, q.self_hash, p, b, b.self_hash})    = Ok(g)
  build_observation_plan({g, scs, op_policy_projection,
                          audit_parents}) = Ok(o)
  build_range_plan({g, b, range_policy_projection,
                    audit_parents}) = Ok(r)

Then:
  1. o is a valid ObservationPlan: total provenance,
     mandatory-and-feasible coverage, no orphan checkpoints / probes
     / metrics, every probe attaches to an existing g entity,
     no probe targets a reserved effect class, per-class budget
     caps respected, encoding consistent with workload compare_domain,
     Invariant trace cost fits TraceBudget, semantic field is
     independent of probe set.
  2. The re-emitted semantic_checkpoint_schema.json is a strict
     subset of the artifact's full schema, ordered canonically,
     with each entry's attachment_node_id pointing at a valid
     g.nodes entry.
  3. operational_probe_schema.json declares one entry per active
     probe with its event shape and budget class; gbf-debug can
     decode every event using the closed `TraceEventPayloadLayout`
     enum plus the hash-bound trace event layout registry for
     `Tuple(TraceEventTupleSpecId)` payloads.
  4. r is a valid RangePlan: every reduction-site-bearing op in g
     has exactly one entry, every entry's plan is the smallest
     admissible plan whose certificate verifies, every certificate
     is independently re-checkable from its JSON.
  5. certs/range.cert.json's outcome is Verified iff every
     certificate is non-Failed; under BitExact, every chunked
     plan has chunk_len dividing term_count and every renorm plan
     uses ExactPostBoundary.
  6. All five emitted reports' identity sections agree on the
     shared hashes (infer_ir_self_hash, quant_graph_self_hash,
     determinism); cross-report consistency invariants hold.
  7. K4 / K5 are stable across two regenerations on the same
     hash-bound inputs; cache hits replay byte-identical products
     and certificates.
  8. F-B8 (StoragePlan) is unblocked: it can size scratch per
     reduction site directly from r.entries[*].plan.
  9. F-B13 (GbSchedIR) is unblocked: it knows tile shape constraints
     per reduction site directly from r.entries[*].plan and per-
     probe attachment slots from o.probes[*].source.
  10. F-C2 (ArtifactOracle) is unblocked: it can run checkpoint-
      aligned diffing against the build-active subset, with the
      correct ObservationEncoding per checkpoint.
  11. gbf-verify (F-F1) can independently verify range_cert without
      any gbf-codegen access; the cert JSON is sufficient because
      every certified entry embeds its chosen ReductionPlan,
      ReductionSiteFacts, and proof scalars.
  12. F-B16 (FeasibilityRefinementLoop) has the schemas it needs to
      issue KnobDelta::DisableOptionalProbes (against
      disabled_optional_probes) and KnobDelta::RaiseReductionCeiling
      (against reduction_ceiling_overrides). Until F-B16 lands,
      both fields are present as explicit empty set/map values; failure
      memos preserve that distinction.

Not proven:
  storage class / lifetime / aliasing                    (F-B8)
  spatial residency                                      (F-B9, F-B10, F-B11)
  arena byte ranges                                      (F-B12)
  scheduled slices / lease lifecycle                     (F-B13)
  schedule cost envelopes                                (F-B14)
  backend reachability / placement                       (F-B15)
  refinement-loop convergence                            (F-B16)
  ObservabilityCertificate (paired-build comparison)     (F-B14 + F-B17)
  conformance against ConformanceEnvelope                (F-C4)
  per-token expert_id binding for PostExpertDowncast     (runtime)
  trace event aggregation across runs                    (gbf-bench)
  emulator / hardware re-verification of cert claims     (later milestones)
```

## 18. Final concise contract

```text
F-B6/F-B7 is correct when:

1. ObservationPlan is constructed deterministically from
   GbInferIRProduct + SemanticCheckpointSchema +
   ObservationPolicyProjection. Selection rule:
     (Mandatory(scs) ∩ build_feasible(g))
     ∪ (WorkloadRequired ∩ SchemaIds(scs) ∩ build_feasible(g))
     ∪ (WorkloadOptional ∩ Optional(scs) ∩ build_feasible(g)).
   No new SemanticCheckpointId / TraceProbeId / MetricId is invented.

2. Every selected SemanticCheckpointId attaches to exactly one
   NodeAnchor via the closed anchor_to_checkpoint function (§8.5).
   The single-token convention guarantees uniqueness.

3. Probes survive iff their budget_class is at or above
   optional_probe_floor AND not in the trace_demotion drop set,
   AND not listed in disabled_optional_probes (empty pre-F-B16).
   Per-class weight totals do not exceed profile caps.

4. ObservabilityMode = Invariant ⇒ projected trace cost fits
   TraceBudget; otherwise OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED.

5. semantic_checkpoint_schema.json (re-emit) is a subset
   of the artifact's full schema, possibly equal to it, ordered canonically;
   every entry includes its
   attachment_node_id, attachment_anchor, and ObservationEncoding.

6. operational_probe_schema.json carries one entry per active
   probe with closed enum event shape, level, budget class.

7. RangePlan is constructed deterministically from
   GbInferIRProduct + StaticBudgetReport +
   RangePolicyProjection. The pure core does NOT read
   QuantGraph.tensors directly; QuantGraph contributes only through
   `quant_graph_self_hash` and the Stage-2 accumulator facts copied into
   `StaticBudgetReport.reduction_sites`.

8. ∀ reduction-site-bearing node n ∈ g.nodes (n.reduction_site = Some(_)):
     ∃ exactly one entry ∈ RangePlan.entries with site = n.reduction_site
     ∧ entry.plan = smallest admissible plan whose AccumulatorCertificate
       verifies under canonical reference semantics
     ∧ entry.effective_ceiling honors most-specific-wins overrides
       (Site > Layer > global).

9. AccumulatorCertificate is a typed proof: every load-bearing
   scalar (term_count, per_term_abs_max, sum_bound, total_abs_max,
   slack, chunk_len, tile_len, chunk_count, cross_chunk_sum_bound,
   tile_count, per_chunk_i16_slack, per_tile_i16_slack,
   renorm_strategy) is in the JSON. The verifies predicate is
   closed and independently re-runnable.

10. Under BitExact, every ChunkedI16 plan has chunk_len dividing
    term_count, every RenormLoop plan uses ExactPostBoundary, no
    mid-reduction saturation. Saturation is permitted only at
    named numeric boundaries (residual combine, classify logit,
    FFN activation, final clamp) per F-B5 §2.10.

11. No silent integer-width expansion: all internal arithmetic uses
    checked u128; declared widths in the cert JSON enforce overflow
    detection.

12. No storage decisions, no overlay choices, no scheduling fusion:
    chunk_len and tile_len are LOGICAL reduction structure, not
    storage. RangePlan does not declare buffer addresses, page ids,
    or arena ids. ObservationPlan does not declare trace ring
    addresses or storage classes for probes.

13. F-B16 RepairPolicy / CompileKnobs is named-only: this chunk
    reads CompileKnobs::observation and CompileKnobs::range,
    populates disabled_optional_probes and
    reduction_ceiling_overrides as empty pre-F-B16, but never
    accepts a RepairProposal(_) provenance.

14. All five emitted reports use ReportEnvelope + canonical JSON +
    DomainHash-based self-hash inherited from F-B2/F-B4 unchanged.
    Public JSON is flat. report_self_hash round-trips. Soft
    diagnostics are rejected.

15. K4 (ObservationPlan) and K5 (RangePlan) cache keys use
    DomainHash; pass_version, schema, projection, and feature-set
    drift each invalidate the cache. policy_resolution_self_hash
    and compile_request_hash are audit-only, not in K4 or K5.

16. Stage 4 and Stage 5 are independent: F-B6 does not read
    RangePlan; F-B7 does not read ObservationPlan. They may run
    in parallel. F-B16 may re-run either alone.

17. F-B6 attaches no semantic-comparison-altering edges to g.
    The semantic checkpoint attachment set is invariant under
    ObservabilityMode and ProbeBudgetClass floor for fixed
    (artifact, workload, profile, GbInferIR topology).
    Different probe sets ⇒ same semantic contract.

18. F-B7 emits a verified certs/range.cert.json — the first
    machine-checkable certificate per planv0.md line 2825.
    gbf-verify (F-F1) can independently re-verify without
    gbf-codegen access.
```

## 18a. Pre-implementation closure checklist

Before implementation tasks are minted, reviewers must confirm:

1. The build-active checkpoint re-emit uses schema id
   `build_active_semantic_checkpoint_schema.v1` everywhere.
2. Stage 4 and Stage 5 pure cores return cacheable core products and
   report bodies, not emitted ReportEnvelope values.
3. Observation registry snapshots are explicit Stage 4 inputs and are
   represented in K4.
4. StageCache failure memos rewrap audit-parent fields and never replay
   stale emitted envelopes.
5. `ObservationPlan.semantic` stability claims refer to checkpoint
   attachments, not encodings.
6. Metric weights are present in `MetricProbe` and
   `operational_probe_schema.json`.
7. Required probes cannot be disabled through
   `disabled_optional_probes`.
8. `ChunkedI16.cross_chunk_sum_bound` is computed from actual
   `term_count`, not padded chunk capacity.
9. `RenormLoop` carries enough strategy/recurrence information in
   `RangePlan` for downstream implementation, or downstream stages are
   explicitly required to read the certificate.
10. BitExact RenormLoop behavior is either forbidden in v1 or backed by
    a proof distinct from SingleI16.
11. All proof scalar field widths match the verifier equations.
12. Every listed diagnostic is reachable by a defined input surface, or
    explicitly marked reserved and excluded from the fixture closure gate.

## 20. Landed-code reconciliation (F-B1 .. F-B5 post-merge audit, 2026-05-14)

After F-B1 (compute bringup), F-B2/F-B4 (entry validation + static
budget), and F-B3/F-B5 (canonical IRs) landed on `main`, an audit
revealed several places where this RFC's type names or shapes do not
match the landed reality. Per §-1 authority rules, F-B3/F-B5 and the
M0 ABI (gbf-abi) crates win on shared surfaces; this RFC is amended
here. Each subsection below is binding on the implementation beads.

### 20.1 SemanticCheckpointId is a dotted-string newtype

**Landed:** `gbf-abi::checkpoint::SemanticCheckpointId(Cow<'static, str>)`
with grammar validation (lowercase + digit + underscore + dot,
≤128 chars, no leading/trailing/double dots).

**RFC contradiction:** Throughout the RFC `SemanticCheckpointId` is
treated as a structured enum
(`SemanticCheckpointId::PostEmbedding { layer }`, etc.).

**Resolution:** The structured enum is renamed internally to
`SemanticCheckpointKind` (a NEW F-B6 construction-side helper enum) and
encoded to the dotted-string `SemanticCheckpointId` via a closed
function:

```rust
fn semantic_checkpoint_kind_to_id(k: SemanticCheckpointKind)
    -> SemanticCheckpointId
{
    let s = match k {
        SemanticCheckpointKind::PostEmbedding { layer }
            => format!("layer.{layer}.post_embedding"),
        SemanticCheckpointKind::PostRouter { layer }
            => format!("layer.{layer}.post_router"),
        SemanticCheckpointKind::PostExpertDowncast { layer, expert }
            => format!("layer.{layer}.expert.{expert}.post_downcast"),
        SemanticCheckpointKind::PostLogits   => "post_logits".to_owned(),
        SemanticCheckpointKind::PostDecode   => "post_decode".to_owned(),
    };
    SemanticCheckpointId::from_owned(s).expect("grammar-valid")
}
```

`anchor_to_checkpoint` in §8.5 returns `Option<SemanticCheckpointKind>`;
the driver immediately wraps via `semantic_checkpoint_kind_to_id`.

The inverse `id → kind` parser is also a closed function used by F-B6
when comparing workload-supplied id strings against the schema.

### 20.2 SemanticCheckpointSchema has no `role` field; carries `stratum`

**Landed** (`gbf-abi::checkpoint::SemanticCheckpointSchema` under
feature `host`):

```rust
pub struct SemanticCheckpointSchema {
    pub schema_version: u16,
    pub abi_version: AbiVersion,
    pub build_hash: [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub checkpoints: Vec<CheckpointEntry>,
}

pub struct CheckpointEntry {
    pub semantic: SemanticCheckpointId,
    pub compact:  CompactCheckpointId,
    pub stratum:  SemanticStratum,    // Denotation | Artifact | Operational
    pub source_op: Option<Cow<'static, str>>,
}
```

**RFC contradiction:** §8.4 and §10.1 reference
`SemanticCheckpointRole { Mandatory, Optional }` as a per-entry field
on the schema.

**Resolution:** `SemanticCheckpointRole` is a NEW F-B6-owned enum
that is **derived from `stratum`**:

```rust
fn role_from_stratum(s: SemanticStratum) -> SemanticCheckpointRole {
    match s {
        SemanticStratum::Denotation => SemanticCheckpointRole::Mandatory,
        SemanticStratum::Artifact   => SemanticCheckpointRole::Mandatory,
        SemanticStratum::Operational => SemanticCheckpointRole::Optional,
    }
}
```

Denotation + Artifact strata are semantic-comparison-essential
(mandatory); Operational stratum is observability-add-on (optional).
The F-B6 RFC's `Mandatory(scs)` and `Optional(scs)` are computed by
applying this mapping. The mapping is closed in v1 and may amend
later.

### 20.3 WorkloadManifest.ObservationPolicy is shallow in v1

**Landed** (`gbf-workload::manifest::ObservationPolicy`):

```rust
pub struct ObservationPolicy {
    pub checkpoints: CheckpointSelection,            // single-variant enum
    pub trace_level: TraceLevel,                     // Summary | Checkpoints
    pub compare_domain: CompareDomain,               // TokenLogits | GeneratedBytes
    pub determinism_requirement: DeterminismRequirement, // SeededDecode
}

pub enum CheckpointSelection { SemanticAndOperational }
pub enum TraceLevel          { Summary, Checkpoints }
pub enum CompareDomain       { TokenLogits, GeneratedBytes }
pub enum DeterminismRequirement { SeededDecode }
```

**RFC contradiction:** §8.1 `WorkloadObservationProjection` references
`checkpoints: CheckpointSelection` as `{ required: BTreeSet<id>,
optional: BTreeSet<id> }`, with `compare_domain` having five variants
and `determinism_requirement` mapping to F-B3's `DeterminismClass`.

**Resolution (v1):**
- `CheckpointSelection::SemanticAndOperational` in v1 implicitly says
  "select every Mandatory checkpoint AND every Operational-stratum
  checkpoint that is build-feasible". There is no per-checkpoint opt-in
  set; the rule becomes:
  ```text
  selected_semantic_v1 :=
    (Mandatory(scs) ∩ build_feasible_set(g))
    ∪ (Optional(scs) ∩ build_feasible_set(g))
  ```
  i.e. the RFC's three-way union collapses to a two-way union and
  `SchemaIds(scs)` filter is automatic.
- The RFC's planned `WorkloadRequired ∩ SchemaIds ∩ feasible` /
  `WorkloadOptional ∩ Optional ∩ feasible` split is **reserved for a
  future CheckpointSelection enum variant** (e.g.
  `ExplicitRequiredAndOptional { required: ..., optional: ... }`).
- `TraceLevel` v1 has only `Summary` / `Checkpoints`; the RFC's
  `Standard` / `Verbose` are aliases. Implementation MUST treat
  `Summary == Standard` and `Checkpoints == Verbose` for now, or
  introduce explicit `From` impls.
- `CompareDomain` v1 has two variants. The RFC's five-variant superset
  (`CanonicalValue`, `TokenIdOnly`, `ExpertIdOnly`, `EnvelopeQ8_8`,
  `EnvelopeQ16_16`) lives as a **policy-side** enum `policy::CompareDomain`
  introduced by F-B6 with a `From<workload::CompareDomain>` mapping:
  ```text
  TokenLogits     -> CanonicalValue
  GeneratedBytes  -> TokenIdOnly
  ```
- `DeterminismRequirement::SeededDecode` v1 maps to
  `DeterminismClass::BitExact` for v1 builds.

Beads T-B6.D, T-B6.G adopt these mappings.

### 20.4 ProbeLevel + TraceProbeId vocabulary collisions

**Landed:**
- `gbf-abi::trace::TraceProbeId(pub u16)` — runtime probe id
- `gbf-policy::diagnostics::TraceProbeId(pub u16)` — diagnostic-context id
- `gbf-abi::trace::ProbeLevel { Always, OnError, Verbose }` — runtime level
- `gbf-abi::trace::ProbeBudgetClass { PerSlice, PerFrame, PerSession }` — runtime budget window

**RFC vocabulary:**
- `ProbeLevel { Metric, Event, Verbose, Fault }` (RFC's four-variant)
- `ProbeBudgetClass { Required, Important, Diagnostic, BestEffort }` (RFC's policy importance)

**Resolution:**

| RFC name | Landed source | F-B6 policy-side name |
|----------|---------------|------------------------|
| `TraceProbeId` (build-time selection) | `gbf-policy::diagnostics::TraceProbeId` | reuse — same type, same shape (u16 newtype). The two `TraceProbeId`s coexist and are byte-identity-convertible. |
| `TraceProbeId` (runtime trace event) | `gbf-abi::trace::TraceProbeId` | reuse |
| `ProbeBudgetClass` (RFC importance) | NEW | introduce `gbf-policy::probe::ProbeImportanceClass { Required, Important, Diagnostic, BestEffort }` (per T-B6.A) |
| `ProbeBudgetClass` (M0 runtime windowing) | `gbf-abi::trace::ProbeBudgetClass` | reuse |
| `ProbeLevel` (RFC categorical) | `gbf-abi::trace::ProbeLevel` | reuse the ABI's three-variant enum; remap RFC's four variants per: `Metric → Always`, `Event → Always`, `Verbose → Verbose`, `Fault → OnError` |

Both `TraceProbeId`s carry identical wire shape (u16); a F-B6 helper
provides `From<gbf_policy::diagnostics::TraceProbeId>
  for gbf_abi::trace::TraceProbeId` and the inverse, used at the
emission boundary between Stage 4 selection and runtime trace
installation.

Each `OperationalProbe` carries BOTH:
- `level: ProbeLevel` (ABI three-variant, runtime semantic)
- `importance: ProbeImportanceClass` (policy four-variant, build-time governance)

### 20.5 MetricId is brand-new

**Landed:** `MetricId` does not exist anywhere in the repo.

**Resolution:** T-B6.B introduces `gbf-policy::metrics::MetricId(pub
String)` as a transparent newtype with grammar validation matching
`SemanticCheckpointId`'s rules (lowercase + digit + underscore + dot,
≤128 chars). Initial MetricRegistry is seeded with a small set per
T-B6.B's subtasks. `gbf-policy::metrics` is a NEW module.

### 20.6 CompileProfileSpec versioning is `1.0.0` not `v1`/`v2`

**Landed:** `gbf-policy::compile` uses semver with the domain
separator
`b"gbf:gbf-policy:CompileProfileSpec:compile_profile_spec:1.0.0\0"`.

**RFC contradiction:** §13.1 and T-B6.C bead use `v1 → v2` shorthand.

**Resolution:** T-B6.C bumps `1.0.0 → 2.0.0`. The new domain separator
byte string becomes
`b"gbf:gbf-policy:CompileProfileSpec:compile_profile_spec:2.0.0\0"`.
Stage 0.5 accepts only `2.0.0`; `1.0.0` fixtures are rejected.

`CompileProfileSpec` adds two new public fields:
- `range_caps: RangeCapsSpec`
- `observation_caps: ObservationProfileCaps`

The fixture-shipped `defaults_hash` re-computes against the new
domain separator.

### 20.7 LockedObservationKnobs / LockedRangeKnobs derive from KnobLockSet

**Landed:** `gbf-policy::compile::KnobLockSet { locked: BTreeSet<CompileKnobId> }`
and `CompileKnobId { Placement, Observation, Range, Storage, Sram,
RomWindow, Overlay, Schedule }`.

**RFC contradiction:** §8.1 / §9.1 reference
`LockedObservationKnobs { trace_demotion_locked: bool, ... }` /
`LockedRangeKnobs { reduction_ceiling_locked: bool }` as ground-truth
types.

**Resolution:** These are **NEW projected types** derived at Stage 0.5
from `KnobLockSet` + sub-knob fields. F-B6 / F-B7 read them as
already-projected; Stage 0.5 owns the projection logic. The audit
parents on Stage 4 / Stage 5 carry these projected forms.

### 20.8 DeterminismClass lives in gbf-codegen::s1::quant_graph

**Landed:** `gbf-codegen::s1::quant_graph::DeterminismClass` (F-B3
owned).

**Resolution:** All Stage 4 / Stage 5 references must import from this
path. Beads T-B6.D, T-B7.B already reference correctly.

### 20.9 ReductionSiteKey does not exist

**Landed:** Only `gbf-policy::diagnostics::ReductionSiteId(pub String)`
is landed. No `ReductionSiteKey` type.

**Resolution:** Cross-stage joins use `ReductionSiteId` directly. The
RFC text already noted this in the F-B3/F-B5 audit; bead T-B7.A
records the absence inline. No `site_key` field on `RangePlanEntry`.

### 20.10 TraceBudget is identical across gbf-policy and gbf-abi

**Landed:** Both `gbf-policy::compile::TraceBudget` and
`gbf-abi::trace::TraceBudget` carry `{ max_events_per_slice: u16,
max_bytes_per_frame: u16, drop_policy: TraceDropPolicy }`. Same shape.

**Resolution:** `ObservationPolicyProjection.trace_budget` uses the
gbf-policy variant. M0 runtime trace path uses the gbf-abi variant.
Convertible via `From` impl in either direction (identity).

### 20.11 §20 amends §-1 authority rules

This entire §20 section is normative. The construction beads under
F-B6 / F-B7 implement against §20-amended contracts. The original
§-1 authority rules stand: F-B3/F-B5 wins on shared IR surfaces;
M0 ABI wins on wire/runtime surfaces; F-B6/F-B7 owns Stage 4 / Stage 5
internals.

## 19. References

* `history/planv0.md` — §"The compiler pipeline" stages 4 and 5
  (lines 1618–1664); §"Reports and artifacts"
  `semantic_checkpoint_schema.json`, `operational_probe_schema.json`,
  `certs/range.cert.json` (lines 1987 and 2825); §"Three oracles"
  / `SemanticCheckpointSchema` (line 448);
  `ProbeBudgetClass` / `TraceProbeId` / `MetricId` (line 1217 et
  seq.); `ReductionPlanCeiling` (line 1228); `ReductionSiteId`
  (line 1383); `SemanticCheckpointId` (line 2280); §"Workloads" /
  `ObservationPolicy` (lines 770–920).
* `history/rfcs/F-B2-F-B4-pipeline-entry-validation.md` — pass-
  shape rhetoric, canonical JSON rule, self-hash convention,
  `ReportEnvelope<R>`, `ValidationDiagnostic`, `CompileKnobs`,
  StageCache key construction, profile-spec shape.
* `history/rfcs/F-B3-F-B5-canonical-irs.md` — `QuantGraph`,
  `GbInferIR`, `NodeAnchorMap`, `ReductionSiteId`, `op_signature`,
  `ValueFormat::ExactAccumulator`, single-token convention,
  effect-class set (`Rng { slot: RngSlot }`,
  `SequenceState { slot: StateSlotId }`, `FaultBoundary`),
  determinism class binding, DomainHash.
* Landed F-B3/F-B5 code in `gbf-codegen/src/s1/quant_graph.rs` and
  `gbf-codegen/src/s3/infer_ir.rs`; `ReductionSiteId` is defined in
  `gbf-policy/src/diagnostics.rs` as
  `pub struct ReductionSiteId(pub String)`.
* `history/rfcs/F-A1-gbf-asm.md` — `MachineEffect`, `PrivilegeClass`
  (referenced by downstream stages, not this chunk).
* `history/rfcs/F-A6-gbf-store-migrate.md` — `gbf-store`,
  StageCache infrastructure (closed; this chunk wires K4 / K5).
* `history/rfcs/F-A8-gbf-debug.md` — agent debugger consumption
  of `operational_probe_schema.json`.
* `history/glossary.md` — semantic checkpoint, operational probe,
  metric probe, probe budget class, reduction plan, accumulator
  certificate, named numeric boundary.
* `CONSTITUTION.md` — Doctrine of Correctness (§I), Velocity of
  Tooling (§II), Shifting Left (§III), Immutable Runtime (§IV),
  Observability (§V), Knowledge Graph (§VI).
* `CLAUDE.md` — beads workflow, pre-commit hook, session protocol,
  project skills.
