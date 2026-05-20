# RFC F-B14 + F-B17: Objective-Facing Cost (`ScheduleCostAnalysis`, Stage 11) and StageCache Integration Sweep Across All Stages

## -1. Authority and amendment policy

This RFC is the source of truth for F-B14 and F-B17 implementation.
`history/planv0.md` remains the architectural context document, but this RFC
is allowed to refine, narrow, or supersede `planv0.md` wherever this RFC makes
a more precise implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence
must be recorded in an `Amends planv0` note close to the relevant decision.
This is not a request to edit `planv0.md` immediately; it is a local
source-of-truth ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B14/F-B17 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a
  shared surface (canonical JSON rule, self-hash convention, diagnostic
  envelope, StageCache key construction, `ReportEnvelope` shape), the
  F-B2/F-B4 RFC wins. F-B14/F-B17 inherit those surfaces unchanged unless
  this RFC explicitly amends them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on `QuantGraph`,
  `GbInferIR`, or canonical-product handling, the F-B3/F-B5 RFC wins.
* If this RFC and `F-A6-gbf-store-migrate.md` disagree on the
  `BlobStore` / `StageCache` storage contract, the F-A6 RFC wins.
  F-B17's job is to *consume* that contract uniformly across stages, not
  to redefine it.
* F-B6 (`ObservationPlan`), F-B7 (`RangePlan`), F-B8 (`StoragePlan`),
  F-B9 (`SramPagePlan`), F-B10 (`RomWindowPlan`), F-B11 (`OverlayPlan`),
  F-B12 (`ArenaPlan`), and F-B13 (`GbSchedIR` + `ResourceStateValidation`)
  RFCs partly land before this chunk, partly are forthcoming. This RFC
  consumes their public types and reportable identities by hash; if a
  forthcoming RFC changes those public types, that RFC must explicitly
  amend the corresponding StageCache key entry in §9.2 of this RFC.
* If a later RFC introduces a new transformative or validation stage that
  participates in the StageCache, that RFC must explicitly amend §9.2 of
  this RFC and add the new stage's typed input bundle, key body, and
  cached product.
* If a later RFC changes any public type, report shape, cache key,
  diagnostic code, or canonicalization rule introduced here, that later
  RFC must explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not
  prose folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft |
| Feature beads   | bd-prw **F-B14 ScheduleCostAnalysis (Stage 11)**; bd-1g7k **F-B17 StageCache integration across all stages** |
| Open tasks      | To be minted: T-B14.1..T-B14.N (`CompileObjective` resolution, evidence-class taxonomy, calibration-bundle binding, per-mode rollup, `EstimatedCostDelta` semantic validator, `schedule_cost.json` emitter, fallback-reason emission, schema/round-trip tests, K14 StageCache wiring); T-B17.1..T-B17.M (one wiring task per F-B*x* stage, plus the cross-stage `cache_status.json` emitter, plus the per-stage typed-input-bundle conformance test) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` lines 1894–1985 (Stage 11 `ScheduleCostAnalysis`); 1770–1900 (Stage 10 `GbSchedIR` + `SchedulePack`); 1985–2080 (`BuildReports` + `schedule_cost.json` field set in `budget.json`); 770–920 (workloads, `DeployabilityEnvelope`, `RuntimeChromeBudget`, calibration); 1065–1095 (sizing realism, dense baseline, multi-timescale state); 2466–2640 (Assembly eDSL, profiles and objectives, `CompileObjective`); 2640–2870 (test classes, reports/artifacts, `schedule_cost.json` + `budget.json` + StageCache verification rules); engineering rule 20 (always-on content-addressed `StageCache`, two-component canonical key) |
| Glossary        | `history/glossary.md` (objective, evidence class, uncertainty envelope, calibration bundle, cycle model, schedule cost, StageCache, canonical input, content-addressed product, fallback reason); §3 of this RFC adds `EvidenceClass`, `UncertaintyEnvelope`, `CalibrationBundleRef`, `CycleModelRef`, `FallbackReason`, `EstimatedCostDelta`, `ScheduleCostReport`, `TypedInputBundle`, `StageCacheKeyBody`, `cache_status.json` |
| Constitution    | §I correctness by construction; §II three-stratum oracle correspondence (cost is *operational stratum* prediction, not denotational truth); §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ReportEnvelope`, `ValidationDiagnostic`, canonical JSON / self-hash, StageCache key construction §11 — the canonical-input convention F-B17 honors); F-B3/F-B5 Canonical IRs (provide `QuantGraph` / `GbInferIR` and the K1/K3 keys); F-B6 / F-B7 / F-B8 / F-B9 / F-B10 / F-B11 / F-B12 / F-B13 (provide K4..K10/K10.5 stage keys F-B17 inventories); F-A6 gbf-store (provides `BlobStore`, `StageCache`, `compose_key`, the cache primitives F-B17 uniformizes); F-B11/F-B12 (provide K11/K12, the canonical exemplar of "StageCache algebra" for a stage RFC); F-B13 (provides `SchedulePack`, `RuntimeMode` keying, and `ResourceStateValidation` — the inputs F-B14 consumes); F-B16 FeasibilityRefinementLoop (BLOCKED on oracle question — consumes per-mode `EstimatedCostDelta` to evaluate `RepairProposal`s); F-B15 Backend (Stage 12, consumes per-mode envelopes for `map.json` / `budget.json` / `compiler_feedback.json` reports); Epic E `gbf-bench` (produces calibration bundles F-B14 dereferences); Epic F `gbf-report` (canonicalizes `schedule_cost.json` and `cache_status.json` schemas) |
| Sister deps     | bd-3ix (F-B16) — strictly downstream consumer of `ScheduleCostReport`; bd-9ae (F-B13) — strictly upstream producer of `SchedulePack`; bd-3ll (F-A6) — closed; provides the `StageCache` primitive F-B17 calls into |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B14 + F-B17 sits inside the
compiler-pipeline epic, where that epic sits inside the full project, and
which adjacent chunks' contracts this RFC inherits or honors.

### 0.1 Project at a glance — the eight epics

The gbllm project compiles a tiny language model into an LR35902 ROM that
runs on real Game Boy hardware. The work is split across eight epics
(`planv0.md` §"Workspace skeleton"; bead-side mirror in `Epic *: …`
issues):

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
          Defines the three-stratum correspondence relation.
          F-B14's cost envelope is OPERATIONAL-STRATUM PREDICTION;
          it is not in the conformance equality relation.

Epic D — Runtime Beyond M0
          Persistence, harness, trace, drift, fault, SchedulePack,
          RuntimeDriftMonitor. Consumes per-mode envelopes from F-B14
          to define drift triggers ("observed cycles_per_token p95
          exceeded predicted upper bound by N for K consecutive
          windows ⇒ ShrinkSlices").

Epic E — Calibration & Bench
          gbf-bench production: cycle calibration, kernel timing, autotune.
          PRODUCES the calibration bundles F-B14 CONSUMES by hash.
          Confidence taxonomy in calibration bundles is what
          EvidenceClass projects from.

Epic F — Reports & Verify
          gbf-report (schedule_cost.json, cache_status.json schemas) +
          gbf-verify (independent slow reference implementations of cost
          rollup, used as cross-check in nightly trust tests).

Epic G — Data, Lexical, Decode Pipeline
          gbf-data (corpus, charset, normalization, decode policy).
          Not directly consumed by this chunk.

Epic H — Kernel
          gbf-kernel (KernelSpec + matvec/residual/norm/route/decode kernel
          implementations). KernelSpec identity is part of CycleModelRef
          resolution (F-B14 dereferences a calibrated KernelSpec id by hash).
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
  F-B9  Stage 7        SramPagePlan
  F-B10 Stage 8        RomWindowPlan
  F-B11 Stage 8.5      OverlayPlan
  F-B12 Stage 9        ArenaPlan
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation
  F-B14 Stage 11       ScheduleCostAnalysis                      ← THIS RFC
  F-B15 Stage 12       Backend (AsmIR + ReachabilityValidation +
                                PlacedRom + EncodedRom)

Cross-cutting:
  F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs
        (BLOCKED on oracle question)
  F-B17 StageCache integration sweep across all stages           ← THIS RFC
```

Sequencing of weekly chunks:

```text
Chunk 1 (in flight):  F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2 (drafted):    F-B3 + F-B5         Stages 1, 3
Chunk 3 (next up):    F-B6 + F-B7         Stages 4, 5
Chunk 4:              F-B8                Stage 6
Chunk 5:              F-B9 + F-B10        Stages 7, 8
Chunk 6:              F-B11 + F-B12       Stages 8.5, 9
Chunk 7:              F-B13               Stages 10, 10.5
Chunk 8 (THIS RFC):   F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop
```

### 0.3 Where F-B14 and F-B17 sit in the pipeline

F-B14 and F-B17 are the **two closing acts** of the transform pipeline,
both running after every spatial decision is final but before the
backend lowers to bytes:

* **F-B14 (Stage 11) `ScheduleCostAnalysis`** is the **single
  load-bearing producer of objective-facing cost envelopes**. It runs
  over `GbSchedIR` / `SchedulePack` (F-B13) and the resolved
  `CompileObjective` (carried forward from F-B2/F-B4's
  `ResolvedCompilePolicy`), dereferences calibration bundles by hash, and
  produces a per-`RuntimeMode` `EstimatedCostDelta` map. Each estimate
  carries an `EvidenceClass`, an `UncertaintyEnvelope`, and an
  `EvidenceRef` chain back to the calibrated source, transferred source,
  or heuristic fallback that produced it. The output is the contract
  surface F-B16's refinement loop reads to decide whether a
  `RepairProposal` is worth applying, and F-B15's report emitter reads
  to populate `budget.json`'s "estimated cycles per token / observed vs
  predicted / fallback reason" rows.

* **F-B17 (cross-stage) `StageCache integration sweep`** is the
  cross-cutting wiring pass that retroactively threads
  `gbf-store::StageCache` (F-A6) into every transformative or validation
  stage's call site. Each stage was specified with a typed input bundle,
  a typed output product, and a canonical report by its owning RFC; F-B17
  is the pass that *proves* every stage adheres to the F-B2/F-B4 §11
  canonical-input convention, lands the cross-stage `cache_status.json`
  report, and pins the typed-input-bundle conformance test that fails if
  any stage's key derivation drifts from "total function of typed
  inputs."

```text
   ResolvedCompilePolicy (F-B2 §0.5)
         |
         |   (carries CompileObjective + RuntimeModeSet)
         |
         v
   QuantGraph (F-B3) → StaticBudgetReport (F-B4)
         |
         v
   GbInferIR (F-B5) → ObservationPlan (F-B6) → RangePlan (F-B7)
         |
         v
   StoragePlan (F-B8) → SramPagePlan (F-B9) → RomWindowPlan (F-B10)
         |
         v
   OverlayPlan (F-B11) → ArenaPlan (F-B12)
         |
         v
   GbSchedIR + ResourceStateValidation (F-B13)
         |
         |   per-mode SchedulePack { mode -> GbSchedIR }
         |   reservation accounting honored
         |   resource leases balanced
         |
         v
   +----------------------------------------------------------+
   | Stage 11   ScheduleCostAnalysis                          |  F-B14
   |                                                          |
   |   inputs (pinned, hash-bound):                           |
   |     SchedulePackProduct                  (F-B13)         |
   |     ResolvedCompilePolicy                (F-B2)          |
   |       └─ CompileObjective                                |
   |       └─ RuntimeModeSet                                  |
   |       └─ RiskPolicy                                      |
   |     CalibrationBundleSet                 (Epic E)        |
   |     RuntimeChromeBudget                  (gbf-policy)    |
   |     TargetProfile                        (gbf-hw)        |
   |     KernelSpecRegistry                   (Epic H)        |
   |                                                          |
   |   per-mode rollup:                                       |
   |     for each RuntimeMode m in SchedulePack.modes:        |
   |       per-slice cost rollup                              |
   |         → per-token rollup                               |
   |         → per-objective satisfaction                     |
   |         → fallback marking                               |
   |       EstimatedCostDelta {                               |
   |         cycles_per_token,                                |
   |         bank_switches_per_token,                         |
   |         sram_page_switches_per_token,                    |
   |         yields_per_token,                                |
   |         scheduler_headroom_utilization,                  |
   |         video_commit_cost_margin,                        |
   |         max_no_progress_estimate,                        |
   |         + EvidenceClass + UncertaintyEnvelope + refs     |
   |       }                                                  |
   |                                                          |
   |   emits:                                                 |
   |     schedule_cost.json (with self_hash)                  |
   |   key:                                                   |
   |     K14 = DomainHash(...)  (see §11)                     |
   +-------------------------+--------------------------------+
                             |
                             v
                  (consumed by F-B16 refinement loop;
                   embedded by F-B15 in budget.json)

   cross-stage:
   +----------------------------------------------------------+
   | F-B17  StageCache Integration Sweep                      |
   |                                                          |
   |   for every stage S in {                                 |
   |     0, 0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5, 9, 10, 10.5,    |
   |     11, 12                                               |
   |   }:                                                     |
   |     - typed input bundle TIB(S) exists                   |
   |     - StageCacheKey K(S) = DomainHash(TIB(S))            |
   |     - typed product P(S) and canonical report R(S)       |
   |       are content-addressed                              |
   |     - hit/miss/stale handling is mechanical              |
   |     - regenerate-on-stale is total                       |
   |     - per-stage test asserts key derivation              |
   |       is a TOTAL FUNCTION of typed inputs                |
   |                                                          |
   |   emits cross-stage:                                     |
   |     cache_status.json (per-stage hit/miss/stale tally)   |
   |                                                          |
   |   F-B17 itself is NOT a stage. It owns no key. It is     |
   |   a cross-cutting validator that fails build closure if  |
   |   any stage's key construction drifts from F-B2/F-B4 §11.|
   +----------------------------------------------------------+
```

These two features are paired in one RFC because they share the
**post-spatial closing-act** shape: both are the last things the
transform pipeline does after spatial decisions are final, neither
mutates IR, and both establish contract surfaces every consumer outside
Epic B reads — F-B14 establishes the *cost envelope* contract for F-B16
and F-B15; F-B17 establishes the *content-addressed product* contract
for the entire pipeline.

`Amends planv0`: `planv0.md` lists `ScheduleCostAnalysis` as Stage 11
and engineering rule 20 names "always-on content-addressed `StageCache`."
Neither is wrong, but `planv0.md` does not explicitly state that the
cache wiring across stages is itself a closure-gating cross-cutting
sweep with its own report (`cache_status.json`). This RFC narrows that
discipline by promoting the sweep to a Feature with its own bead
(bd-1g7k) and making `cache_status.json` first-class.

### 0.4 Cross-epic interactions

F-B14 + F-B17 sit at the intersection of five epics:

```text
Epic A → Epic B
  - gbf-foundation (Hash256, BlobRef, BlobCodec wrappers)         consumed
  - gbf-store::StageCache + BlobStore                              consumed
                  (F-B17 uniformizes per-stage call sites)
  - gbf-policy (CompileObjective, RuntimeMode, RiskPolicy,
    CalibrationSetRef)                                             consumed
  - gbf-hw (TargetProfile)                                         consumed

Epic B (internal):
  - F-B2 / F-B4 (StageCache key construction §11)                  consumed
  - F-B3 / F-B5 (K1, K3 + canonical-input convention)              consumed
  - F-B6 .. F-B13 (per-stage products + their K-keys)              consumed
                  (F-B17 enumerates K4..K10.5 + uniformizes)
  - F-B16 (RepairPolicy / RepairProposal cost evaluation)          feeds
  - F-B15 (Backend report emitter)                                 feeds

Epic C → Epic B (oracle correspondence):
  - F-C3 ScheduleOracle consumes SchedulePack alongside F-B14's
    cost envelope; F-C3 measures actual cycles, F-B14 predicts.
    Drift between predicted and observed is the F-C3 / F-B14 seam.

Epic D → Epic B (runtime drift):
  - RuntimeDriftMonitor consumes the per-mode upper bounds from
    F-B14 to define DriftTrigger thresholds. ShrinkSlices /
    DropTrace / DemoteMode actions trigger when observed
    metrics exceed F-B14's predicted upper bound by configured
    margins for K consecutive windows.

Epic E → Epic B (calibration ingest):
  - PlatformCalibrationBundle / KernelCalibrationBundle /
    RuntimeCalibrationBundle: F-B14 dereferences these by hash
    through CalibrationBundleRef. Their declared
    CalibrationConfidenceClass is the source EvidenceClass
    projects from. gbf-bench is the producer; F-B14 is a consumer.

Epic F → Epic B:
  - gbf-report owns schedule_cost.json + cache_status.json
    canonical schemas, semantic validators, and self-hash helpers.
  - gbf-verify owns an independent slow reference implementation
    of the cost rollup that nightly trust tests cross-check
    against the production rollup.

Epic H → Epic B:
  - KernelSpec / KernelSpecId — F-B14 dereferences a calibrated
    KernelSpec id by hash through CycleModelRef. KernelSpec
    identity is established by Epic H; F-B14 is a consumer.
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk straddles M3 and M4 and is the
last large pipeline correctness gate before the refinement loop
becomes meaningful:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; conformance.json; first CompileRequest wiring.
              ↳ F-B2/F-B4 (Chunk 1)   delivers the CompileRequest wiring.
              ↳ F-B3 (Chunk 2)        delivers ArtifactOracle's input.

M2            One shared micro-kernel resolved by RomWindowPlan;
              one expert payload bank; emulator diffing against
              ScheduleOracle; first ReachabilityValidation pass.
              ↳ F-B14 cannot start until SchedulePack exists (F-B13);
                F-B14's ScheduleOracle handshake is M3 work.

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ F-B14 (this chunk) closes by predicting per-mode
                cycles/bank-switches/SRAM-page-switches/yields per
                token for the M3 routed-FFN profile, with calibrated
                evidence where bench bundles exist and explicit
                heuristic fallbacks elsewhere.
              ↳ F-B17 (this chunk) closes by uniformizing the cache
                wiring for every stage that exists by M3 close
                (Stages 0..11 + 12 in skeleton form).

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ F-B14's per-mode envelope IS what enables
                SchedulePack mode switching to be cost-honest. The
                runtime can only switch from `Default` to `Trace` if
                F-B14 recorded that `Trace`'s predicted cycles_per_token
                still satisfy the active CompileObjective.
              ↳ RuntimeDriftMonitor consumes F-B14's UncertaintyEnvelope
                bounds as drift thresholds.
```

The two features in this chunk are therefore the **bridge between
"the schedule exists" and "the runtime can trust it"**: F-B14 finishes
the M3 transform pipeline by producing the per-mode cost envelope every
runtime decision downstream of it (drift, refinement, mode switch)
reads; F-B17 finishes the M0–M3 iteration story by ensuring every
stage's product is content-addressed by its typed inputs alone.

### 0.6 What the project as a whole gains when this chunk lands

```text
1. F-B16 (refinement loop) becomes implementable.
   Without F-B14, RepairProposal::estimated_cost has no oracle. With
   F-B14, the loop controller can compare a candidate
   ConstraintDelta's predicted EstimatedCostDelta against the current
   plan's EstimatedCostDelta and accept / reject by typed criteria.

2. budget.json is honest about evidence and uncertainty.
   Per planv0 line 2840, budget.json must carry "evidence class and
   uncertainty envelope for every load-bearing estimate" and "fallback
   reason when calibration confidence is insufficient." F-B14 is the
   producer of those fields.

3. cycle-model drift becomes mechanically detectable.
   Once F-B14 records predicted_cycles_per_token with an
   UncertaintyEnvelope at p50 / p95, a downstream
   RuntimeDriftMonitor or nightly trust test can compare the
   observed distribution against that envelope and flag drift.

4. SchedulePack mode switching is cost-honest.
   Per planv0 line 1881, a CompiledBuild may carry a SchedulePack
   keyed by RuntimeMode rather than a single GbSchedIR. F-B14's
   per-mode envelopes are what justify offering more than one
   mode: each mode's predicted cost has its own evidence class
   and uncertainty, so the runtime knows which mode satisfies
   the CompileObjective at which quantile.

5. iteration speed becomes mechanical.
   F-B17 wires StageCache into every stage. A typical training run
   shadow-compiles dozens of checkpoints; without F-B17, every
   shadow-compile pays the full pipeline cost. With F-B17, only
   the parts that actually changed re-run.

6. cache_status.json makes "why did this rebuild?" diagnosable.
   When a build hits cache misses unexpectedly, cache_status.json
   tells you which stage missed and which input identity drifted.
   This is the same kind of "shift left on the iteration loop"
   that ReachabilityValidation does for layout and ScheduleCostAnalysis
   does for cycles.

7. canonical-input convention is mechanically enforced.
   F-B17 lands a per-stage typed-input-bundle conformance test that
   fails if any stage's key derivation depends on something that is
   not in the typed input bundle. This catches an entire class of
   cache-correctness bugs before they ship.
```

### 0.7 What this chunk retires for the rest of Epic B

By the time the rest of Epic B's later chunks begin (F-B15 backend,
F-B16 refinement loop):

* The single load-bearing oracle for "what does this schedule cost?"
  exists. F-B16 reads `EstimatedCostDelta` to evaluate `RepairProposal`s;
  F-B15 reads it to populate `budget.json` and `compiler_feedback.json`.
* Every transformative or validation stage has a content-addressed
  product whose identity is a total function of typed inputs. Cache
  hits never hide unstable inputs; cache misses are mechanically
  diagnosable.
* The cross-stage `cache_status.json` report is first-class and
  participates in the build-output package. Iteration loops on
  shadow-compile and autotune become cheap.
* The canonical-input convention from F-B2/F-B4 §11 is no longer
  prose; it is a mechanically checked invariant, with a per-stage
  conformance test that fails if a stage's key construction drifts
  from "total function of typed inputs."
* The evidence-class taxonomy is established. Every cost figure in
  every report from M3 forward carries a typed `EvidenceClass`,
  `UncertaintyEnvelope`, and `EvidenceRef` chain. Reviewers can ask
  "why is this estimate trusted?" and get a typed answer instead of
  prose.

### 0.8 Reading order for reviewers

A reviewer who has just read F-B11/F-B12 and F-B13 and is approaching
this RFC for the first time should read:

```text
§0   (this section) — placement and dependencies
§0a  TL;DR + closure conditions
§1   Project context — milestone-specific framing
§2   Load-bearing decisions — the engineering choices that bracket the rest
§5   Authority rules — what this RFC owns vs inherits
§6   Pipeline state machine — how Stage 11 plugs into Stages 0..10.5
§8   Stage 11 contract: ScheduleCostAnalysis (especially §8.1 evidence-class
     taxonomy — the load-bearing claim for F-B16)
§9   F-B17 contract: StageCache integration sweep (especially §9.2 per-stage
     key index — the canonical-input enumeration the rest of Epic B inherits)
§11  StageCache algebra (Stage 11 K14 + F-B17's status as cross-stage
     validator with no key of its own)
§14  Task DAG
§17  End-to-end theorem
```

Skim §3, §4, §7, §10, §12, §13, §15, §16, §18 for specifics.

## 0a. TL;DR

This chunk lands the **objective-facing cost envelope** that brackets the
end of the transform pipeline, and the **cross-stage StageCache wiring
sweep** that retroactively proves every stage's product is content-addressed
by its typed inputs. It owns one numbered stage and one cross-cutting
feature:

* **Stage 11 — `ScheduleCostAnalysis` (F-B14).** Single load-bearing
  producer of objective-facing cost envelopes. Runs over `SchedulePack`
  (F-B13), uses calibration bundles by hash, produces per-`RuntimeMode`
  `EstimatedCostDelta` map. Each estimate carries a typed
  `EvidenceClass` (`Calibrated` / `Transferred` / `Heuristic` /
  `Fallback`), an `UncertaintyEnvelope` (p50 / p95 lower / upper), and
  an `EvidenceRef` chain back to the calibration source, cycle model,
  or heuristic fallback. The output is the contract surface F-B16's
  refinement loop reads to evaluate `RepairProposal`s and F-B15's
  report emitter reads to populate `budget.json`.

* **Cross-cutting — StageCache integration sweep (F-B17).** Wires
  `gbf-store::StageCache` (F-A6) into every transformative or
  validation stage's call site. Validates that every stage has a typed
  input bundle, a typed output product, a canonical report, a
  StageCache key derived deterministically from typed inputs alone,
  and per-stage hit/miss/stale handling that is mechanical rather than
  advisory. Lands the cross-stage `cache_status.json` report (per-stage
  hit/miss/stale tally) and the per-stage typed-input-bundle
  conformance test that fails if any stage's key derivation drifts
  from "total function of typed inputs."

These two features are paired in one RFC because they share the
**post-spatial closing-act** shape: each is the last thing the
transform pipeline does (F-B14 closes the transform stages; F-B17
closes the cache-correctness story for those stages), each is content-
addressed end-to-end, each is deterministic across two consecutive
regenerations, and each shares the diagnostic envelope, JSON
canonicalization rule, self-hash convention, and `StageCache` key
construction inherited from F-B2/F-B4. F-B14 is itself a stage with
its own key (K14, §11); F-B17 is cross-cutting and has **no key of its
own** — it is a validator over every other stage's keys.

The chunk closes only when:

1. `ScheduleCostAnalysis` construction is a deterministic pure function
   of (`SchedulePackProduct`, `ResolvedCompilePolicy`,
   `CalibrationBundleSet`, `TargetProfile`, `RuntimeChromeBudget`,
   `KernelSpecRegistry`) and is byte-identical across two consecutive
   regenerations on a clean checkout.
2. `schedule_cost.json` round-trips through its semantic validator
   and self-hash. Every `EstimatedCostDelta` field carries an
   `EvidenceClass`, an `UncertaintyEnvelope`, and at least one
   `EvidenceRef`.
3. Every per-mode envelope is **total** for its `RuntimeMode`: the
   keyset of `ScheduleCostReport.per_mode` equals
   `ResolvedCompilePolicy.requested_runtime_modes`.
4. Every estimate that falls back to a `Heuristic` or `Fallback`
   evidence class carries a typed `FallbackReason` and is recorded
   in `budget.json`'s "fallback reason when calibration confidence
   is insufficient" row (per planv0 line 2848).
5. F-B14 NEVER fabricates a calibrated estimate. A heuristic estimate
   is mechanically distinguishable from a calibrated estimate by the
   typed `EvidenceClass` field.
6. K14 (Stage 11 key) is pinned and tested.
7. **F-B17 closure**: every stage S in the pipeline (0, 0.5, 1, 2, 3,
   4, 5, 6, 7, 8, 8.5, 9, 10, 10.5, 11, 12) has:
   * a typed input bundle `TypedInputBundle(S)` whose canonical hash
     equals the cache key K(S);
   * a typed output product `Product(S)` and a canonical report
     `Report(S)` (with `report_self_hash`);
   * StageCache hit/miss/stale handling tested by a property test
     showing K(S) changes if and only if a load-bearing input changes;
   * regenerate-on-stale semantics tested by a fixture that flips one
     load-bearing input and asserts cache miss;
   * presence in `cache_status.json` with hit/miss/stale tally.
8. `cache_status.json` round-trips through its semantic validator and
   self-hash, and pins all 16 stage entries deterministically (the 14
   numbered + 0.5 + 8.5 + 10.5; F-B17 is itself NOT a row in
   `cache_status.json` — it is the report's *producer*).
9. The per-stage typed-input-bundle conformance test fails if any
   stage's key derivation depends on something not in the typed
   input bundle (e.g., a global, an environment variable, a wall-clock
   reading, or a side channel through `ResolvedCompilePolicy` that is
   not projected through the stage's `*PolicyProjection`).
10. The cross-stage `K(S1) ≠ K(S1')` test passes for every stage S
    after a deliberate one-bit flip of a load-bearing input; the
    corresponding `K(S1) = K(S1')` test passes for a deliberate flip
    of a non-load-bearing input (e.g., adding a comment to a
    fixture's filename).

The chunk does **not** include:

* The runtime drift monitor itself. F-B14 records predicted upper
  bounds; `RuntimeDriftMonitor` (Epic D) consumes them. The two are
  paired by hash through `schedule_cost.json` but the monitor is
  not in this chunk.
* The safe-mode trigger evaluator. `SafeModeTrigger`s are part of
  `ModeSwitchPolicy` (Epic D); F-B14 does not produce or evaluate them.
* The actual calibration bundle production. `gbf-bench` (Epic E)
  produces `PlatformCalibrationBundle` / `KernelCalibrationBundle`;
  F-B14 dereferences them by hash.
* The `gbf-store` implementation. F-A6 owns `BlobStore`, `StageCache`,
  `compose_key`, the cache primitives. F-B17's job is to *consume* the
  primitive uniformly, not to redefine it.
* The `--resume-from <stage>` CLI control. That is layered on top of
  F-B17's per-stage cache wiring by `gbf-cli` and is out of scope here.
* The `FeasibilityRefinementLoop` driver. F-B16 owns it. F-B14 supplies
  the cost surface F-B16 reads; the loop logic is F-B16's job.
* `RepairProposal::estimated_cost` evaluation. F-B16 owns it.
* `ConstraintDelta` application. F-B16 owns it.
* Backend codegen. F-B15 owns it.
* `gbf-verify`'s independent slow reference rollup. The reference
  implementation lives in `gbf-verify` (Epic F); this chunk lands
  the production rollup, with a parity test against the reference
  marked as a nightly trust test rather than a closure gate.
* Per-mode actual-cycle measurement. `ScheduleOracle` (F-C3) measures;
  F-B14 predicts. The two diff in nightly trust tests, not in this
  chunk's closure.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What F-B2..F-B13 leave on the table

Per the F-B2/F-B4, F-B3/F-B5, F-B6/F-B7, F-B8, F-B9/F-B10, F-B11/F-B12, and
F-B13 RFCs, by the time this chunk begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, calibration, hint bundle, and
  `CompileRequest` are all admissible, hash-bound, and traceable through
  `artifact_validation.json`.
* `ResolvedCompilePolicy` is the single answer to "what policy governed
  this build," with provenance for every load-bearing scalar. It carries
  the `CompileObjective` and the `requested_runtime_modes` set this
  chunk consumes.
* `RuntimeChromeBudget` is honored at the static byte-math level. Each
  expert fits its slot under the resolved `PlacementProfile`.
* `QuantGraph` is the canonical artifact graph. `GbInferIR` is the
  hardware-aware value/effect IR. Both are content-addressed.
* `ObservationPlan` (F-B6) pinned semantic checkpoints and operational
  probes; `RangePlan` (F-B7) pinned reduction-plan choices;
  `StoragePlan` (F-B8) decided what is materialized vs persisted vs
  recomputed.
* `SramPagePlan` (F-B9), `RomWindowPlan` (F-B10), `OverlayPlan` (F-B11),
  and `ArenaPlan` (F-B12) decided every spatial fact: kernel residency,
  SRAM page geometry, WRAM overlay regions, and concrete byte ranges.
* `GbSchedIR` (F-B13) realized the IR as slices, lease-balanced and
  resource-validated. `SchedulePack` (F-B13) keyed slice schedules by
  `RuntimeMode`, sharing the same artifact semantics, checkpoint schema,
  and continuation ABI across modes.
* Every prior stage emitted a canonical JSON report with a self-hash;
  every report has a content-addressed product backing it; every product
  is referenced by hash in the next stage's input bundle.

What is **NOT** present at the start of this chunk:

* No oracle for "what will this schedule cost?" Stage 4's
  `ObservationPlan` and Stage 7's `RomWindowPlan` produce *static*
  byte-math fits, not predicted runtime cycles. Stage 2's
  `StaticBudgetReport` is a static integer projection. None of these
  predict cycles per token, bank-switches per token, or yields per
  token under a real workload.
* No mechanism for `RepairProposal` cost comparison. F-B16's
  refinement loop has no oracle to evaluate "is this delta worth
  applying?" without F-B14.
* No uniform `StageCache` wiring. Each prior RFC defined its
  per-stage K-key (K0, K0.5, K2, K1, K3, K4, K5, K6, K7, K8, K8.5, K9,
  K10, K10.5) and the canonical-input convention they all honor, but
  no cross-cutting validator has confirmed the convention is mechanical
  rather than aspirational.

### 1.2 What M3 / M4 commits to and how this chunk delivers it

Per `planv0.md` §"Milestones":

> **M3**: Top-1 router, expert dispatch table, value/effect `GbInferIR` +
> `ObservationPlan` + `RangePlan` + `StoragePlan` wired end-to-end for a
> routed FFN under the cooperative scheduler.
>
> **M4+**: Sequence-state block (BoundedKv first, then LinearState),
> SchedulePack mode switching, persistence, drift, fault recovery.

Two distinct architectural commitments:

1. The transform pipeline closes by predicting the cost envelope of the
   M3 schedule. This is exactly what F-B14 delivers.
2. SchedulePack mode switching is cost-honest. Each `RuntimeMode` in
   `SchedulePack.modes` has its own predicted cost envelope so the
   runtime can decide which mode satisfies the active `CompileObjective`
   at which quantile. This is what F-B14's per-mode rollup delivers.

The M3 / M4 iteration story also assumes that shadow-compile is cheap.
Per `planv0.md` §"Engineering rules" rule 20 ("always-on content-addressed
StageCache"), the iteration loop relies on the cache being wired through
every stage. This is what F-B17 delivers.

Without F-B14, M3 closes the IR pipeline but the cost surface for
F-B16 is missing. Without F-B17, the M3 shadow-compile loop pays the
full pipeline cost on every checkpoint.

### 1.3 What this chunk retires for the rest of Epic B

By the time Epic B's later chunks begin (F-B15 backend, F-B16 refinement
loop):

* Every later stage (Stage 12 backend) and every cross-stage consumer
  (F-B16 refinement loop) receives a typed, validated, evidence-classed
  `EstimatedCostDelta` map keyed by `RuntimeMode`. They never
  re-derive cost; they consume `schedule_cost.json` by hash.
* Every stage's product is content-addressed by typed inputs alone.
  F-B17 closes the canonical-input convention; later stages (F-B15)
  inherit a uniformly cached pipeline with no exceptions.
* The cross-stage `cache_status.json` is first-class. Every build's
  output package contains it; tests compare across builds; iteration
  loops use it to diagnose unexpected cache misses.

This chunk's job is to retire the **cost-envelope and cache-correctness**
preconditions of the rest of the pipeline. It is the third and fourth
shift-left filter: the first is `gbf-train preflight` (Epic E); the
second is F-B2's `ArtifactValidationAndUpgrade`; the third is F-B14's
evidence-classed cost prediction (catches "this schedule won't satisfy
the objective" before backend codegen pays the cost); the fourth is
F-B17's cache-correctness sweep (catches "this stage's cache key is
not a total function of typed inputs" before a stale cache hit
silently corrupts a downstream stage).

### 1.4 Why this is one stage Feature plus one cross-cutting Feature

The natural unit is "the closing acts of the transform pipeline."

* If we made it one feature, the bead would carry both an
  evidence-classed cost rollup *and* a cross-stage cache-wiring sweep.
  The implementation surfaces are large enough that PR review fragments,
  and the cache-wiring touches every other Epic B feature's call site
  (which would force F-B14 to wait for every other stage's wiring to
  land before its own bead could close — exactly the wrong dependency
  shape).
* If we made it three features (F-B14 + per-stage F-B17.x + cross-stage
  cache_status.json), the per-stage tasks under F-B17 would multiply
  bead inventory without changing the substance of the work; the
  per-stage wiring is mechanical and fits one task each under a single
  feature bead.
* One stage Feature plus one cross-cutting Feature matches the natural
  seam: F-B14 owns the cost-envelope contract (Stage 11); F-B17 owns
  the cross-stage canonical-input enforcement and `cache_status.json`.
  They are paired in this RFC because they share post-spatial timing
  (both run after every spatial pass), report-shape discipline (both
  emit canonical JSON with self-hashes), and the F-B2/F-B4 §11
  inheritance.

### 1.5 What this chunk is NOT

The chunk is small in number of stages but big in contract surface. To
prevent scope creep, here is what this chunk explicitly is not:

* It is **not** a transform stage that produces a cost-driven IR. F-B14
  is *passive*: read `SchedulePack`, look up calibration, project cost,
  emit report. It does not rewrite `GbSchedIR` or propose
  `RepairProposal`s.
* It is **not** the runtime drift monitor. `RuntimeDriftMonitor` lives
  in Epic D and consumes F-B14's predicted upper bounds; F-B14 does not
  observe runtime metrics.
* It is **not** the safe-mode trigger evaluator. `SafeModeTrigger`s and
  `DriftTrigger`s belong to `ModeSwitchPolicy` (Epic D, planv0 line 1851
  / 1874). F-B14 records the *predictions* those triggers will be
  configured against; the trigger evaluator itself is downstream.
* It is **not** the `gbf-store` implementation. F-A6 owns `BlobStore`,
  `StageCache`, `compose_key`. F-B17 *consumes* those primitives
  uniformly across stages; it does not re-implement them.
* It is **not** the per-stage RFC. Each prior RFC owns its stage's
  typed input bundle, output product, and canonical report. F-B17's
  job is to *enumerate* those across stages, prove the canonical-input
  convention holds mechanically, and uniformize the call-site wrapper.
  F-B17 does not redefine any per-stage K-key; it consumes the K-keys
  defined by each stage's owning RFC.
* It is **not** an artifact-migration tool. `gbf-migrate` is deferred
  to F-A6b. F-B17 does not introduce migration; it inherits F-A6's
  current "schema mismatch ⇒ rebuild from sources" policy.
* It is **not** an autotune driver. `gbf-bench` (Epic E) drives
  autotune; F-B14's per-mode envelope is one of the inputs autotune
  reads, not the autotune logic itself.
* It is **not** the `--resume-from <stage>` CLI control. That is
  layered by `gbf-cli` on top of F-B17's per-stage cache primitives;
  out of scope here.
* It is **not** the producer of calibration bundles. `gbf-bench`
  (Epic E) produces them. F-B14 dereferences them by hash.
* It is **not** F-C3 `ScheduleOracle`. F-C3 measures actual cycles;
  F-B14 predicts. The two diff in nightly trust tests; the diff is
  not part of this chunk's closure.
* It is **not** an oracle for *quality*. F-B14 predicts operational
  cost, not denotational quality. Quality / conformance lives in
  `conformance.json` (Epic C / Epic F).

### 1.6 Relationship to F-B16 (`FeasibilityRefinementLoop`)

F-B16 is the bounded monotone repair loop (planv0 line 1128). It is
currently BLOCKED on an oracle question (bd-3ix). F-B14 is the
prerequisite cost-surface that F-B16 needs once it unblocks.

The boundary:

* F-B14 emits `schedule_cost.json` per-build, capturing the *current*
  schedule's predicted per-mode cost envelope. It does not run inside
  the loop; it runs once after `GbSchedIR` settles.
* When F-B16 lands and the refinement loop runs, the loop driver may
  invalidate the F-B14 product if a `ConstraintDelta` is applied that
  changes any input to `ScheduleCostAnalysis`. The recomputed
  `schedule_cost.json` is what the loop reads to decide "did this
  delta actually improve the objective satisfaction?"
* F-B14's `EvidenceClass` and `UncertaintyEnvelope` are what F-B16
  reads to decide whether a small predicted improvement justifies
  applying a delta with `Heuristic` evidence (probably no, under
  strict profiles) or `Calibrated` evidence (probably yes, under
  most profiles).
* F-B16 is the only consumer that may apply a `ConstraintDelta`.
  F-B14 is read-only; it never proposes repairs.

`Amends planv0`: planv0 line 1131 says "ScheduleCostAnalysis is the
only producer of objective-facing cost envelopes used by the refinement
loop." This RFC narrows that to "F-B14 is the *single* producer; any
later RFC that wants to add a competing cost surface must explicitly
amend this RFC."

### 1.7 Relationship to F-A6 (`gbf-store`)

F-A6 (CLOSED) shipped `BlobStore`, `StageCache`, `compose_key`,
`StageKey`, `ComponentDigestSet`, `Pinset`, `run_gc`, and the archive
format. F-B17 is the cross-stage wiring sweep that *uses* those
primitives uniformly across every Epic B stage's call site.

The boundary:

* F-A6 owns the `StageCache` primitive. `gbf-store::stage_cache::compose_key`
  is the one place a `StageKey` becomes a `StageCacheKey`; F-B17 calls
  this primitive but does not re-implement it.
* F-B17 owns the per-stage *call-site wrapper* discipline:
  - construct `TypedInputBundle(S)` for stage S;
  - canonicalize `TypedInputBundle(S)` and compute K(S) via F-A6's
    `compose_key`;
  - call `StageCache::get(K(S))`; on hit, replay the cached product
    and report; on miss or stale, run the stage; on success, store
    the new product+report keyed by K(S).
* F-A6 deliberately does not enumerate Epic B stages. `StageId` is an
  opaque newtype; concrete variants live in `gbf-codegen`. F-B17
  enumerates the Epic B `StageId` variants and binds each to its
  typed input bundle.
* F-A6's `compose_key` rules (BTreeMap for ordered fields, sorted
  feature flags, fixed-encoding for `Hash256` and `SemVer`) are
  the canonical determinism rules F-B17 inherits. F-B17 does not
  add new determinism rules.
* F-A6 is independent of Epic B; F-B17 depends on F-A6 plus every
  individual Epic B stage RFC. F-B17 cannot land any single stage's
  wiring before that stage's RFC closes.

`Amends planv0`: planv0 engineering rule 20 says "the compiler supports
an always-on content-addressed StageCache, with `--resume-from <stage>`
as the user-facing debugging control layered on top." This RFC narrows
that into a closure-gating cross-cutting sweep with its own report
(`cache_status.json`) and per-stage typed-input-bundle conformance test.

## 2. Load-bearing decisions

### 2.1 Cost is evidence-classed estimate, never folklore

Every cost figure F-B14 emits carries a typed `EvidenceClass`:

```rust
pub enum EvidenceClass {
    /// The estimate is backed by a calibration bundle whose
    /// declared CalibrationConfidenceClass is Measured for the
    /// active TargetProfile and KernelSpec set.
    Calibrated,
    /// The estimate is backed by a calibration bundle measured on
    /// a *related* target / kernel and applied through a typed
    /// transfer function. CalibrationConfidenceClass is Transferred.
    Transferred,
    /// The estimate is computed by a closed-form heuristic over the
    /// schedule structure (slice count, op count, residency choice).
    /// No calibration bundle was consulted, or the consulted bundle
    /// was below the active RiskPolicy's confidence requirement.
    Heuristic,
    /// A heuristic estimate that was further degraded because the
    /// inputs to the heuristic itself were missing or below confidence.
    /// Carries a typed FallbackReason.
    Fallback,
}
```

`EvidenceClass` ordering is `Calibrated > Transferred > Heuristic >
Fallback` for purposes of "tightest evidence wins" comparisons (e.g.,
F-B16 may prefer a `Calibrated` baseline over a `Heuristic` proposed
delta), but the ordering is *informational*, not arithmetic — the cost
values themselves are not adjusted by the evidence class. The
`UncertaintyEnvelope` is what carries the numeric reflection of
confidence.

This is the load-bearing claim for F-B16: *every cost figure in
`schedule_cost.json` is mechanically distinguishable from every other
cost figure by the evidence chain that produced it*. F-B16 may reject
a `RepairProposal` whose `estimated_cost` is `Heuristic` if the active
`RiskPolicy` requires `Calibrated` for the affected metric.

`Amends planv0`: planv0 line 2840 says budget.json must carry
"evidence class and uncertainty envelope for every load-bearing
estimate." This RFC pins the four-variant taxonomy
(`Calibrated`/`Transferred`/`Heuristic`/`Fallback`) and forbids any
other evidence class without an explicit later RFC amendment.

### 2.2 Per-mode envelopes are the contract surface for F-B16

`ScheduleCostReport.per_mode` is keyed by `RuntimeMode`. The keyset
**must** equal `ResolvedCompilePolicy.requested_runtime_modes`. There
is no "default mode" implicit fallback; if a mode is requested, an
envelope must be produced.

```rust
pub struct ScheduleCostReport {
    pub objective: CompileObjective,
    pub per_mode: BTreeMap<RuntimeMode, EstimatedCostDelta>,
    pub refs: Vec<EvidenceRef>,
}
```

Per-mode totality is enforced by a typed semantic validator:

```text
SC-PerModeTotal:
  ScheduleCostReport.per_mode.keys() ==
    ResolvedCompilePolicy.requested_runtime_modes  (set equality)
```

Empty-mode-set is illegal: a `CompileRequest` whose
`requested_runtime_modes` is empty is rejected by F-B2 (Stage 0.5)
before this stage can run. F-B14 does not need to handle that case.

A `RuntimeMode` whose `SchedulePack.modes` does not contain it (i.e.,
F-B13 did not produce a schedule for that mode) is a structural error
caught by F-B13, not by F-B14. F-B14's input invariant is
`ResolvedCompilePolicy.requested_runtime_modes ⊆ SchedulePack.modes.keys()`.

### 2.3 `CompileObjective` is loaded from `ResolvedCompilePolicy`

`CompileObjective` is the typed "what to optimize" contract (planv0
line 2638). It travels through `CompileRequest` and `ResolvedCompilePolicy`;
F-B14 does not load it from disk.

```rust
pub struct CompileObjective {
    pub primary: ObjectiveAxis,
    pub additional: Vec<ObjectiveAxis>,
    pub quantile_targets: Vec<QuantileTarget>,
    pub trace_budget: Option<TraceBudgetSatisfaction>,
    pub frame_jitter: Option<FrameJitterTarget>,
}

pub enum ObjectiveAxis {
    /// Time to first token at p95 ≤ X cycles.
    TimeToFirstToken,
    /// Sustained throughput ≥ Y tokens/sec.
    SustainedThroughput,
    /// Frame jitter p99 ≤ Z cycles.
    LowFrameJitter,
    /// Trace budget satisfied at all configured probe sites.
    TraceBudgetCompliance,
    /// Bank-switch count per token ≤ N.
    BankSwitchCeiling,
    /// Liveness margin ≥ M frames.
    LivenessMargin,
}

pub struct QuantileTarget {
    pub axis: ObjectiveAxis,
    pub quantile: Quantile,           // p50, p95, p99
    pub target_value: i64,
    pub uncertainty_tolerance_q16_16: i64,
}
```

`CompileObjective` is named-only in this chunk: F-B14 reads it,
projects per-mode satisfaction, and records the result. The full
objective taxonomy (axes, quantiles, trace-budget gates) is owned
by F-B2 / F-B4 and is consumed here without re-validation.

`Amends planv0`: planv0 line 2638 says "Each profile is also paired with
a `CompileObjective` declared in `configs/compile/*.toml`." This RFC
narrows that to: `CompileObjective` is loaded by F-B2 (Stage 0.5) into
`ResolvedCompilePolicy.objective`; F-B14 reads it from there. There is
no F-B14-side TOML loader.

### 2.4 Calibration is dereferenced by hash, never inlined

`CalibrationBundleRef` is a content-addressed handle to a calibration
bundle:

```rust
pub struct CalibrationBundleRef {
    pub bundle_kind: CalibrationBundleKind,
    pub bundle_hash: Hash256,
    pub schema_id: SchemaId,
    pub schema_version: SemVer,
    pub declared_confidence: CalibrationConfidenceClass,
}

pub enum CalibrationBundleKind {
    Platform,        // PlatformCalibrationBundle
    Kernel,          // KernelCalibrationBundle
    Runtime,         // RuntimeCalibrationBundle
}
```

F-B14 dereferences `CalibrationBundleRef` through `gbf-store::BlobStore`.
The bundle bytes are not embedded in `schedule_cost.json`; only the
content-addressed hash and a typed projection of the relevant fields
(e.g., `cycles_per_op` for the matching `KernelSpec`) appear in the
report.

This is required for two reasons:

1. **Reproducibility.** Two builds with identical `(SchedulePack,
   ResolvedCompilePolicy, CalibrationBundleRef set)` produce
   byte-identical `schedule_cost.json`.
2. **Iteration speed.** Bundle bodies are large and shared across
   builds. Storing only the hash keeps `schedule_cost.json` small;
   storing the bytes in `gbf-store::BlobStore` keeps deduplication
   automatic.

`Amends planv0`: this RFC narrows planv0's general "calibration is
content-addressed" rule to a specific shape: `CalibrationBundleRef` is
the only legal F-B14-visible handle to a calibration bundle, and
projection (e.g., picking `cycles_per_op` for a specific
`KernelSpecId`) happens inside F-B14 against the bytes resolved through
`BlobStore::get_ref`.

### 2.5 Heuristic fallbacks are typed, never silent

When calibration evidence is unavailable or below confidence, F-B14
falls back to a closed-form heuristic. The heuristic estimate is
*typed* as `EvidenceClass::Heuristic` and carries a typed
`FallbackReason`:

```rust
pub enum FallbackReason {
    /// No calibration bundle was supplied for the active target.
    NoBundleForTarget { target: TargetProfileId },
    /// The supplied bundle's declared confidence was below the
    /// active RiskPolicy::calibration_confidence_requirement.
    ConfidenceBelowRequirement {
        declared: CalibrationConfidenceClass,
        required: CalibrationConfidenceClass,
    },
    /// The KernelSpec invoked at this site is not present in the
    /// calibration bundle's KernelCalibrationBundle.
    KernelSpecNotCalibrated { kernel_spec: KernelSpecId },
    /// The calibration bundle was present but the matching record
    /// was rejected as stale by the freshness gate (target / kernel
    /// / packer / schema mismatch).
    BundleStale { stale_field: StaleField },
    /// The KernelSpec exists but no inputs match the requested
    /// per-mode, per-residency, per-tile-class measurement record.
    MeasurementShapeMismatch { detail: ShapeMismatchDetail },
    /// A composition fallback: an upstream component-level estimate
    /// fell back, so the composed estimate also falls back.
    UpstreamFallback { upstream: Box<FallbackReason> },
}
```

A heuristic estimate **must not** be reported as `Calibrated` or
`Transferred`. The semantic validator for `schedule_cost.json` enforces
this:

```text
SC-EvidenceClassConsistent:
  ∀ estimate e ∈ EstimatedCostDelta {
    if e.evidence_class ∈ {Calibrated, Transferred} ⇒
       e.refs contains at least one EvidenceRef whose source is
       a CalibrationBundleRef whose declared_confidence ≥
       active RiskPolicy::calibration_confidence_requirement.
    if e.evidence_class ∈ {Heuristic, Fallback} ⇒
       e.fallback_reason is Some(_).
  }
```

The semantic validator is mechanical; the test suite enumerates every
combination of (evidence class, fallback reason presence) and asserts
consistency.

This is the load-bearing **anti-folklore** rule: F-B14 NEVER
fabricates a calibrated estimate. If the validator fires, the build
fails.

### 2.6 Uncertainty envelopes are non-negative and contain the point estimate

```rust
pub struct UncertaintyEnvelope {
    /// p50 (median) point estimate. Always present.
    pub p50_q16_16: i64,
    /// p95 lower bound. Must be ≤ p50.
    pub p95_lower_q16_16: i64,
    /// p95 upper bound. Must be ≥ p50.
    pub p95_upper_q16_16: i64,
    /// p99 upper bound. Must be ≥ p95_upper.
    pub p99_upper_q16_16: Option<i64>,
}
```

Constraints:

```text
UE-Ordering:
  p95_lower_q16_16 ≤ p50_q16_16 ≤ p95_upper_q16_16
  p99_upper_q16_16.is_some() ⇒ p99_upper_q16_16 ≥ p95_upper_q16_16

UE-NonNegative:
  p50_q16_16 ≥ 0
  p95_lower_q16_16 ≥ 0
```

A `Calibrated` estimate's envelope is derived from the bundle's
measured distribution. A `Heuristic` estimate's envelope is computed
by a typed `HeuristicEnvelopePolicy` (e.g., "expand p50 by ±50% for
the static heuristic class") that is *itself* recorded in the
report's `refs` so reviewers can audit the policy.

Fixed-point Q16.16 representation is required because F-B2/F-B4 §2.5
forbids floating-point fields in v1 reports (reproducibility leak).

### 2.7 F-B17 cache keys are total functions of typed inputs

This is the load-bearing claim of F-B17:

```text
For every stage S in the pipeline and every two builds B, B':
  K(S, B) = K(S, B') ⟺ TypedInputBundle(S, B) = TypedInputBundle(S, B')
```

Forward direction: equal typed inputs ⇒ equal key. Trivial by
construction (`compose_key` is deterministic).

Backward direction: equal key ⇒ equal typed inputs. The contrapositive
is "different typed inputs ⇒ different key." Required for
cache-correctness: *if two distinct inputs hash to the same key, that's
a bug; if one input has multiple legal cache hits, that's a bug.*

The forward direction is enforced by F-A6's `compose_key`. The backward
direction is enforced by per-stage tests in F-B17:

* For every named field f in `TypedInputBundle(S)`:
  - construct two `TypedInputBundle(S)` instances differing only in f;
  - assert `compose_key(...) ≠ compose_key(...)`.

The test count is `Σ_S |fields(TypedInputBundle(S))|`, which for the
17 stages and ~5–8 fields each gives ~100–140 assertions. This is
mechanical and runs in the workspace's pre-commit hook.

### 2.8 Cache hits never hide unstable inputs

A cache hit replays the cached product and report bytes. The replayed
report's `report_self_hash` must equal the F-A6 `R-Hash` of the cached
body; if not, the entry is poisoned and the stage runs from scratch.

```text
F-Cache-Read-Validate:
  On cache hit, the cached report's report_self_hash must equal
  R-Hash of the cached body. If not, poison the entry and recompute.
```

This is inherited from F-A6 and the per-stage RFCs (e.g., F-B11/F-B12
§13.3). F-B17 does not redefine it; F-B17 *validates* that every stage
honors it, by a per-stage test that:

1. Inserts a corrupt entry into the cache (cached body that
   does not hash to the cached self_hash).
2. Calls the stage.
3. Asserts the stage detects the corruption and re-runs.

### 2.9 Stale-detection is mechanical, not advisory

A cached entry is **stale** when its key is still computable but the
key's input identity has drifted. By the canonical-input convention,
this happens only when one or more `TypedInputBundle(S)` fields
change. Detection is mechanical: recompute K(S) from the current
inputs and compare to the cached entry's key.

```text
StaleDetection:
  cached_key = entry.key
  current_key = compose_key(TypedInputBundle(S, current_inputs))
  if cached_key == current_key:
    HIT
  else:
    MISS (the cached entry is "stale" w.r.t. current inputs;
          regenerate from scratch and store the new product
          under current_key)
```

Stale-detection has no "advisory" mode in F-B17. There is no field
F-B17 considers but does not include in K(S); there is no "soft"
input that misses the key construction. *Every* load-bearing input is
in `TypedInputBundle(S)` and *every* `TypedInputBundle(S)` field
contributes to K(S).

This is the load-bearing **mechanical rather than advisory** rule of
F-B17.

### 2.10 Regenerate-on-stale is total, with a Trace-mode override

When a stage is stale, F-B17 regenerates the product from typed inputs.
The regenerate path is the same code as the cache-miss path; there is
no separate "stale-recovery" code path.

Trace-mode override (per planv0 line 1987 and F-B11/F-B12 §0.7):
under `Trace` builds or any time the StageCache is cold, F-B17 emits
the `stages/` directory containing serializable snapshots for every
transformative pass. This is on top of cache-miss writes, not a
substitute for them.

Trace-mode override does NOT bypass cache-key computation. The cache
key is always computed; the cache write is always attempted. The
trace-mode override is purely additive: it emits an extra `stages/<S>.json`
sidecar alongside the canonical report.

### 2.11 No fabrication, no extrapolation, no silent inference

F-B14's heuristic fallbacks are typed and recorded. There is no
"silent extrapolation" path:

* F-B14 does NOT extrapolate a `Calibrated` estimate from a target T1
  to a related target T2 unless an explicit `Transferred` evidence
  class is recorded with a typed transfer policy.
* F-B14 does NOT replace a missing `KernelSpecId` with a "similar
  enough" `KernelSpecId`. The fallback is `KernelSpecNotCalibrated`,
  recorded as a typed `FallbackReason`.
* F-B14 does NOT fill in a missing per-mode envelope from a different
  mode. If `Trace` mode has no calibration evidence, the `Trace`
  envelope is `Heuristic` with a typed `FallbackReason`, not a copy
  of the `Default` mode's envelope.

This is the same anti-folklore discipline that F-B2 enforces for
calibration freshness and `RuntimeChromeBudget` slack. Every reject
case is a typed input failure, not a silent acceptance.

### 2.12 F-B17 is a sweep, not a stage

F-B17 has **no key of its own**. It is not a stage in the pipeline
sense. It does not have a `cache_status.json` of its own that is
keyed by some K(F-B17). It is the *producer* of `cache_status.json`
in the build's report package, and `cache_status.json` is a
cross-stage tally rather than the output of a typed transform.

```text
cache_status.json schema (per §10.2):
  - per_stage: BTreeMap<StageId, StageCacheStatusEntry>
    where StageCacheStatusEntry = {
      stage_id: StageId,
      k_key: StageCacheKey,
      status: CacheStatus,    // Hit | Miss | Stale | NotApplicable
      input_identity_hash: Hash256,
      product_self_hash: Option<Hash256>,
      report_self_hash: Option<Hash256>,
    }
  - report_self_hash: Hash256
  - schema_id: SchemaId
  - schema_version: SemVer
```

F-B17 itself does not appear in `per_stage`. It is the *producer*; the
report it emits is build-output, not a stage product.

### 2.13 Where the code lives

| Concern                                                   | Crate                                              |
| --------------------------------------------------------- | -------------------------------------------------- |
| `CompileObjective`, `ObjectiveAxis`, `QuantileTarget`, `RiskPolicy` | `gbf-policy::objective` (already absorbed by F-B2 §2.12) |
| `CalibrationBundleRef`, `CalibrationConfidenceClass`      | `gbf-policy::calibration` (re-exports `gbf-hw::calibration::CalibrationConfidenceClass`) |
| `EvidenceClass`, `UncertaintyEnvelope`, `EvidenceRef`, `FallbackReason`, `EstimatedCostDelta`, `ScheduleCostReport`, `CycleModelRef` | `gbf-policy::cost` (NEW, introduced here)         |
| Stage 11 implementation (`ScheduleCostAnalysis`)          | `gbf-codegen::stages::schedule_cost`               |
| `schedule_cost.json` schema                               | `gbf-report::schedule_cost`                        |
| `cache_status.json` schema                                | `gbf-report::cache_status`                         |
| StageCache wiring per stage (F-B17)                       | `gbf-codegen::stage_cache` (existing module; F-B17 expands) |
| Per-stage `TypedInputBundle` definitions                  | `gbf-codegen::stages::*::input_bundle` (each stage's module) |
| Cross-stage `cache_status.json` emitter                   | `gbf-codegen::stage_cache::status`                 |
| Per-stage typed-input-bundle conformance test             | `gbf-codegen::stage_cache::tests::canonical_input_total` |

No new crate is created. `gbf-policy::cost` is a new module within
the existing `gbf-policy` crate.

### 2.14 No backward-compat for calibration-bundle absence

A build whose `RiskPolicy::calibration_confidence_requirement` is set
but whose `CalibrationBundleSet` does not contain a matching bundle
fails F-B14 with `CostCalibrationMissingForRequirement`. There is no
"silent heuristic fallback under strict profiles" code path.

The matrix:

| Profile     | RiskPolicy::calibration_confidence_requirement | F-B14 behavior on missing bundle |
| ----------- | ---------------------------------------------- | -------------------------------- |
| `Bringup`   | `NoMinimumConfidence`                          | `Heuristic` evidence class allowed |
| `Default`   | `Measured`                                     | hard fail `CostCalibrationMissingForRequirement` |
| `Trace`     | `Measured` (Invariant) / `Transferred` (Flexible) | hard fail / allow Transferred only |
| `Recovery`  | `Measured`                                     | hard fail |

This is consistent with F-B2's calibration freshness gate: bringup
profiles accept `BootstrapCalibrationBundle` declared
`CalibrationConfidenceClass::None`; production profiles do not. F-B14
inherits this discipline by always projecting the active
`RiskPolicy::calibration_confidence_requirement` against each
estimate's evidence class.

### 2.15 Schema versioning

`schedule_cost.json` uses `schema_id = "schedule_cost.v1"` and
`schema_version = "1.0.0"`. `cache_status.json` uses
`schema_id = "cache_status.v1"` and `schema_version = "1.0.0"`.
Neither is rev'd in this chunk; future amendments rev the schema_version
explicitly.

The `pass_version` for Stage 11 is `"stage11/v1"`. The cache-key body
includes `pass_version` so a stage logic change misses any older cache
entry.

`crate_feature_set_hash` is the canonical hash of compile-time feature
flags that affect type layout, serde shape, or pass behavior. It is
shared across stages by F-A6's `StageKey::feature_flags` set.

### 2.16 `RepairPolicy` / `CompileKnobs` are named-only

F-B16 (`RepairPolicy`, `RepairProposal`, `KnobDelta`) is BLOCKED on
oracle. This RFC keeps cost envelopes pluggable as inputs to F-B16
without committing to F-B16's exact shape:

* `EstimatedCostDelta` is the contract surface F-B16 will consume.
* F-B16 may compose two `EstimatedCostDelta` values to evaluate "is
  the proposed delta worth applying?" but F-B14 does not depend on
  the composition logic.
* `CompileKnobs::locks` is a downstream concern; F-B14 does not
  consume it.
* `RepairProposal::estimated_cost: EstimatedCostDelta` (per planv0
  line 1131) is the named contract; F-B14 produces the values, F-B16
  consumes them.

This RFC does NOT commit to:

* the exact shape of `RepairProposal`;
* the comparison rule F-B16 uses to accept / reject deltas;
* the loop bound or termination criterion;
* whether `KnobDelta` is monotone in some specific order.

Those decisions belong to F-B16's RFC and may explicitly amend the
shape of `EstimatedCostDelta` if needed.

## 3. Glossary additions

This RFC adds the following terms. They live alongside the F-B2/F-B4
and F-B3/F-B5 additions in `history/glossary.md`.

### 3.1 EvidenceClass

Status: RFC term, owned by F-B14.

The typed taxonomy of "how confident is this estimate." Four variants:
`Calibrated` (backed by a calibration bundle measured for the active
target/kernel), `Transferred` (backed by a related calibration through
a typed transfer policy), `Heuristic` (closed-form heuristic, no
calibration consulted or below confidence), `Fallback` (heuristic
further degraded because heuristic inputs were missing). Ordering is
informational, not arithmetic.

### 3.2 UncertaintyEnvelope

Status: RFC term, owned by F-B14.

The numeric reflection of an estimate's confidence. Carries
`p50_q16_16`, `p95_lower_q16_16`, `p95_upper_q16_16`, optional
`p99_upper_q16_16`. All values are non-negative Q16.16 fixed-point.
Constraints: `p95_lower ≤ p50 ≤ p95_upper` and (when present)
`p99_upper ≥ p95_upper`.

### 3.3 EvidenceRef

Status: RFC term, owned by F-B14.

A typed reference back to the calibration bundle, cycle model, or
heuristic policy that produced an estimate. Resolvable through
`gbf-store::BlobStore` to the originating bytes.

### 3.4 CalibrationBundleRef

Status: RFC term, owned by F-B14 (re-exports
`CalibrationConfidenceClass` from `gbf-hw::calibration`).

A content-addressed handle to a calibration bundle. Carries kind
(`Platform` / `Kernel` / `Runtime`), `bundle_hash`, `schema_id`,
`schema_version`, and `declared_confidence`.

### 3.5 CycleModelRef

Status: RFC term, owned by F-B14.

A content-addressed handle to a cycle model: a typed projection of a
`KernelCalibrationBundle` for a specific `KernelSpecId`. F-B14 resolves
`CycleModelRef` per kernel invocation site.

### 3.6 FallbackReason

Status: RFC term, owned by F-B14.

The typed reason a `Heuristic` or `Fallback` estimate did not get a
`Calibrated` or `Transferred` evidence class. Closed enum with
six variants (see §2.5).

### 3.7 EstimatedCostDelta

Status: RFC term, owned by F-B14 (named in planv0 §"What I would build
first" / Bottom line list).

Per-mode predicted cost over a `RuntimeMode`'s schedule. Fields:
predicted cycles per token, predicted bank-switches per token,
predicted SRAM page-switches per token, predicted yields per token,
scheduler headroom utilization, video-commit cost margin,
max-no-progress estimate, and the
`(EvidenceClass, UncertaintyEnvelope, EvidenceRef[])` chain for each.

### 3.8 ScheduleCostReport

Status: RFC term, owned by F-B14.

The Stage 11 product, embedded in `schedule_cost.json`. Per-mode
`EstimatedCostDelta` map keyed by `RuntimeMode`, plus the active
`CompileObjective` and the union of `EvidenceRef`s consulted.

### 3.9 TypedInputBundle

Status: RFC term, owned by F-B17.

The typed structure that names every load-bearing input to a stage.
Each field is hash-bound (a `Hash256`, a `BlobRef`, or a content
projection) so the bundle's canonical hash is stable. F-B17 enforces
that every stage's `StageCacheKey` is derived from
`TypedInputBundle` alone — no globals, no environment, no wall-clock,
no `ResolvedCompilePolicy` access outside the projected fields.

### 3.10 StageCacheKeyBody

Status: RFC term, owned by F-B17 (inherits from F-B2/F-B4 §11).

The canonical-JSON body of a `StageCacheKey`. Includes every field of
`TypedInputBundle(S)` plus `pass_version_S`, `crate_feature_set_hash`,
and `stage_S_schema_hash`.

### 3.11 cache_status.json

Status: RFC term, owned by F-B17.

The cross-stage hit/miss/stale tally report emitted by F-B17. Lists
every stage in the pipeline (Stages 0, 0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5,
9, 10, 10.5, 11, 12) with its key, status, and product/report self-hashes
when applicable. F-B17 itself is the producer; it does not appear as a
row.

### 3.12 CacheStatus

Status: RFC term, owned by F-B17.

The closed enum classifying a stage's cache outcome on a build:
`Hit` (cached entry replayed), `Miss` (no cached entry, stage ran),
`Stale` (cached entry's key no longer matches current input identity;
F-B17 regenerated), `NotApplicable` (the stage was not eligible for
caching this build, e.g., a fresh input where the stage is the first
to run).

### 3.13 CanonicalInputConvention

Status: RFC term, owned by F-B17 (inherits the convention shape from
F-B2/F-B4 §11).

The mechanical rule that a stage's `StageCacheKey` is a total
deterministic function of its `TypedInputBundle` plus
`pass_version_S`, `crate_feature_set_hash`, and `stage_S_schema_hash`.
No other inputs may affect the key. F-B17 lands the conformance test
that fails if any stage drifts from this convention.

### 3.14 PostSpatialClosingAct

Status: Informal description.

The pairing of F-B14 (cost envelope) and F-B17 (cache wiring) as the
"post-spatial closing acts" of the transform pipeline. Both run after
every spatial pass (F-B8..F-B12) is settled but before the backend
(F-B15) lowers to bytes.

### 3.15 ObjectiveAxis

Status: RFC term, owned by F-B14 (named-only consumed from
`gbf-policy::objective`).

The typed axis of a `CompileObjective`: time-to-first-token, sustained
throughput, low frame jitter, trace-budget compliance, bank-switch
ceiling, liveness margin. Each axis has its own quantile and uncertainty
tolerance.

### 3.16 ObjectiveSatisfaction

Status: RFC term, owned by F-B14.

The typed verdict for a single `(RuntimeMode, ObjectiveAxis,
QuantileTarget)` triple: `Satisfied` (the predicted value is within
target plus uncertainty tolerance), `Borderline` (within tolerance
but only because of uncertainty), `Violated` (predicted value exceeds
target even at the most favorable end of the uncertainty envelope).

### 3.17 ScheduleCostObjectiveSatisfactionMatrix

Status: RFC term, owned by F-B14.

The matrix of `ObjectiveSatisfaction` keyed by
`(RuntimeMode, ObjectiveAxis, Quantile)` recorded inside
`ScheduleCostReport`. Allows F-B16 / F-B15 / dashboards to query
"does mode m satisfy axis a at quantile q?" without re-running cost
analysis.

## 4. Core notation

This RFC uses the F-B3/F-B5 §4 notation conventions plus a few
additions specific to cost analysis and stage-cache enumeration.

### 4.1 Hash notation

```text
H(x)               := canonical hash of x via DomainHash convention
                      (F-B2/F-B4 §11 inherited)
DomainHash(crate, type, schema_id, schema_version, body) := the
                      domain-separated SHA-256 over body bytes
                      with the schema-bound prefix
H₂₅₆(b)             := SHA-256 over byte string b (the underlying primitive)
CanonicalJson(x)   := the byte string produced by canonical-JSON
                      serialization of x per F-B2/F-B4 §2.5
R-Hash(report)     := report.report_self_hash; computed by canonical-JSON
                      with the field temporarily set to sha256:0…0
                      (F-B2/F-B4 §2.4)
```

### 4.2 Stage notation

```text
S                  ∈ {0, 0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5, 9, 10,
                       10.5, 11, 12}
TypedInputBundle(S) := the typed input record of stage S
StageCacheKeyBody(S):= the canonical-JSON body of K(S)
K(S)               := compose_key(TypedInputBundle(S),
                      pass_version_S, crate_feature_set_hash,
                      stage_S_schema_hash)
Product(S)         := the typed output product of stage S
Report(S)          := the canonical-JSON report of stage S, with
                      report_self_hash
```

### 4.3 Cost notation

```text
m                  ∈ RuntimeMode
ECD(m)             := EstimatedCostDelta for mode m
ECD(m).cycles_per_token   :: CostEstimate
ECD(m).bank_switches_per_token :: CostEstimate
... (one per axis)

CostEstimate       := { evidence_class: EvidenceClass,
                        envelope: UncertaintyEnvelope,
                        refs: Vec<EvidenceRef>,
                        fallback_reason: Option<FallbackReason> }
```

### 4.4 Calibration notation

```text
CB                 := CalibrationBundleSet
CB.platform        := PlatformCalibrationBundle (optional)
CB.kernel          := KernelCalibrationBundle (optional)
CB.runtime         := RuntimeCalibrationBundle (optional)
CB.h(kind)         := H(BlobStore::get_ref(CB.<kind>.bundle_ref))

CycleModel(kspec, m) := projection of CB.kernel for KernelSpecId kspec
                        in RuntimeMode m. Yields cycles_per_op +
                        bank_switches_per_invocation +
                        residency_class_modifiers.
```

### 4.5 Cache-status notation

```text
status(S, B)       ∈ {Hit, Miss, Stale, NotApplicable}
CacheStatus(B)     := BTreeMap<StageId, status(S, B)>
                      keyed deterministically by StageId order
```

`StageId` ordering for `cache_status.json` is the ascending pipeline
order (Stage 0, 0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5, 9, 10, 10.5, 11, 12).
The ordering is part of the canonical-JSON contract.

### 4.6 Inheritance notation

```text
Inherited from F-B2/F-B4:
  ReportEnvelope, CanonicalJson, R-Hash, ValidationDiagnostic,
  DiagnosticSeverity, ValidationOrigin

Inherited from F-B3/F-B5:
  DomainHash, StageCacheKeyHash, K1, K3, audit_parents pattern

Inherited from F-A6:
  BlobStore, StageCache, StageKey, ComponentDigestSet, compose_key,
  StageCacheKey newtype

Inherited from F-B11/F-B12:
  K11, K12 + audit_parents discipline + cache laws
  (F-Cache-K, F-Cache-Failure, F-Cache-Drift, F-Cache-Read-Validate)
```

## 5. Authority rules

This section lists what this RFC *owns* and what it *inherits without
modification*.

### 5.1 What F-B14 owns

| Type / contract                                      | Owner    |
|------------------------------------------------------|----------|
| `EvidenceClass` taxonomy                             | F-B14    |
| `UncertaintyEnvelope` type and constraints           | F-B14    |
| `EvidenceRef` shape                                  | F-B14    |
| `CalibrationBundleRef` shape                         | F-B14    |
| `CycleModelRef` shape                                | F-B14    |
| `FallbackReason` taxonomy                            | F-B14    |
| `CostEstimate` shape                                 | F-B14    |
| `EstimatedCostDelta` shape                           | F-B14    |
| `ScheduleCostReport` shape                           | F-B14    |
| `ObjectiveSatisfaction` taxonomy                     | F-B14    |
| `ScheduleCostObjectiveSatisfactionMatrix` shape      | F-B14    |
| `schedule_cost.json` schema                          | F-B14    |
| Stage 11 implementation `gbf-codegen::stages::schedule_cost` | F-B14 |
| K14 cache-key body                                   | F-B14    |
| The "no fabrication" anti-folklore rule              | F-B14    |
| The "per-mode totality" rule                         | F-B14    |

### 5.2 What F-B17 owns

| Type / contract                                      | Owner    |
|------------------------------------------------------|----------|
| `TypedInputBundle(S)` discipline (per stage S)       | F-B17 +  |
|                                                      | each stage's owning RFC (per-stage shape) |
| The canonical-input convention (mechanical rule)     | F-B17    |
| Per-stage `StageCacheKey` enumeration                | F-B17    |
| `cache_status.json` schema                           | F-B17    |
| `CacheStatus` taxonomy                               | F-B17    |
| Per-stage typed-input-bundle conformance test        | F-B17    |
| Cross-stage `cache_status.json` emitter              | F-B17    |
| Per-stage call-site wrapper convention               | F-B17    |

### 5.3 What is inherited unchanged

| Type / contract                                      | Source  |
|------------------------------------------------------|---------|
| `ReportEnvelope<R>`                                  | F-B2/F-B4 |
| `report_self_hash` convention                        | F-B2/F-B4 §2.4 |
| Canonical-JSON rule                                  | F-B2/F-B4 §2.5 |
| `ValidationDiagnostic` taxonomy                      | F-B2/F-B4 §7.1 |
| `DomainHash` convention                              | F-B2/F-B4 §2.4 |
| `BlobStore` API                                      | F-A6     |
| `StageCache` API                                     | F-A6     |
| `StageKey`, `compose_key`                            | F-A6     |
| `Hash256`, `BlobRef`, `BlobCodec`                    | F-A6 / `gbf-foundation` |
| `QuantGraph`, `GbInferIR`                            | F-B3/F-B5 |
| K1, K3 cache-key bodies                              | F-B3/F-B5 §11 |
| K11, K12 cache-key bodies + cache laws               | F-B11/F-B12 §13 |
| `CompileObjective`, `RiskPolicy`, `RuntimeMode`      | F-B2/F-B4 (named) |
| `CalibrationConfidenceClass`                         | `gbf-hw::calibration` |
| `KernelSpec`, `KernelSpecId`                         | Epic H (named) |
| `SchedulePack`, `RuntimeMode`, `GbSchedIR`           | F-B13 (named) |
| Cache laws (F-Cache-Success, F-Cache-Failure, F-Cache-Drift, F-Cache-Read-Validate) | F-A6 / F-B2/F-B4 / F-B11/F-B12 |

### 5.4 Authority precedence

If this RFC is silent on a shared surface, the F-B2/F-B4 rule applies.
If F-B2/F-B4 is silent and F-A6 covers the matter (e.g., `compose_key`
determinism rules), the F-A6 rule applies. If both are silent, the
matter is ambiguous and must be resolved by an explicit amendment to
this RFC before implementation may proceed.

If this RFC and a per-stage RFC disagree on the shape of a
`TypedInputBundle(S)`, the per-stage RFC wins. F-B17's job is to
*enumerate* the per-stage bundles, not to re-define them. If the
enumeration discovers a per-stage RFC has not pinned its
`TypedInputBundle(S)` precisely, the per-stage RFC must amend itself
before F-B17 may proceed.

## 6. Pipeline state machine

This section describes the per-build state machine for F-B14 (Stage 11)
and the cross-stage state machine for F-B17.

### 6.1 Stage 11 state machine

```text
state ::= Pending(SchedulePack, ResolvedCompilePolicy,
                   CalibrationBundleSet, RuntimeChromeBudget,
                   TargetProfile, KernelSpecRegistry)
        | KeyComputed(StageCacheKey)
        | CacheHit(ScheduleCostReport, ReportEnvelope)
        | CacheMiss
        | CacheStale(StageCacheKey, StageCacheKey)   -- (cached, current)
        | InProgress(IntermediateRollup)
        | Completed(ScheduleCostReport, ReportEnvelope)
        | Failed(PassDiagnostics)

transitions:
  Pending(...)       --[compute K14]-->     KeyComputed(K14)
  KeyComputed(K14)   --[StageCache lookup]-->
                          (Hit  -> CacheHit; Miss -> CacheMiss;
                           Stale -> CacheStale)
  CacheHit(R, E)     --[validate self_hash]-->
                          (ok      -> Completed(R, E);
                           poisoned -> CacheMiss)
  CacheMiss          --[run rollup]-->      InProgress(...)
  CacheStale(_, K14) --[run rollup]-->      InProgress(...)
  InProgress(rollup) --[finalize]-->
                          (success  -> Completed(R, E);
                           failure -> Failed(diags))
  Completed(R, E)    --[StageCache::put(K14, R, E)]-->  Done
  Failed(diags)      --[StageCache::memo_failure(K14, diags)]-->  Halt
```

### 6.2 Cross-stage state machine (F-B17)

For each stage S in {0, 0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5, 9, 10, 10.5,
11, 12}:

```text
stage_state(S) ::=
    Skipped              -- prior stage failed; this stage didn't run
  | NotApplicable        -- e.g., Stage 0 on an empty build
  | KeyComputed(K(S))
  | CacheHit
  | CacheMiss
  | CacheStale
  | Completed(Product(S), Report(S))
  | Failed(PassDiagnostics)

per-build invariant:
  If S2 follows S1 in the pipeline, stage_state(S2) is reachable only
  if stage_state(S1) ∈ {CacheHit, Completed}.
  Skipped ⇒ all later stages are also Skipped.
  Failed ⇒ all later stages are Skipped.
  CacheMiss + Failed ⇒ failure memo stored under K(S).
```

`cache_status.json` is emitted *after* every stage has reached a
terminal state (Completed, CacheHit, Skipped, NotApplicable, or
Failed). It records the final state of each stage.

```text
For build B, cache_status.json.per_stage[S] is:
  status:    Hit / Miss / Stale / NotApplicable
  k_key:     K(S)
  input_identity_hash: H(TypedInputBundle(S))
  product_self_hash:   Some(H(Product(S))) when state ∈ {CacheHit, Completed}
                       None otherwise
  report_self_hash:    Some(R-Hash(Report(S))) when state ∈ {CacheHit, Completed}
                       None otherwise
```

Note that `Failed` stages do not appear with `Failed` status in
`cache_status.json` — they appear with `Miss` (the cache miss that led
to the run that failed) and `product_self_hash = None`. This keeps the
cache-status report focused on cache outcomes; build failure is
recorded by the stage's own report (which has `outcome = Failed`).

### 6.3 Build-level state machine

```text
build_state ::= Initiated
              | StagesRunning(set_of_completed_stages)
              | AllStagesTerminal
              | Closure(build_outputs)

transitions:
  Initiated -- run pipeline -->
              StagesRunning({S0, S0.5, ...} as they complete)
  StagesRunning(set) --
    if every S has terminal state -->
              AllStagesTerminal
  AllStagesTerminal --
    emit cache_status.json with per_stage tally -->
              Closure(build_outputs)
```

`cache_status.json` is part of the build output package (per planv0
line 2820 — `stages/` already exists as the per-stage snapshot
directory; F-B17 adds `cache_status.json` as a single file alongside
it).

## 7. Report envelope (inherited)

F-B14 and F-B17 inherit the report envelope from F-B2/F-B4 §7.2 and
F-B3/F-B5 §7. This section restates the inherited shape for
reviewers and pins a few F-B14/F-B17-specific details.

### 7.1 ReportEnvelope shape

```rust
pub struct ReportEnvelope<R> {
    pub schema_id: SchemaId,
    pub schema_version: SemVer,
    pub body: R,
    pub report_self_hash: Hash256,
    pub diagnostics: Vec<ValidationDiagnostic>,
}
```

The envelope is parameterized by report body type R. F-B14's body type
is `ScheduleCostReportBody`; F-B17's body type (for `cache_status.json`)
is `CacheStatusReportBody`.

`schema_id` is `"schedule_cost.v1"` for F-B14 and `"cache_status.v1"`
for F-B17. `schema_version` is `"1.0.0"` for both in this chunk.

### 7.2 Self-hash convention

Both reports follow the F-B2/F-B4 §2.4 convention:
* Compute `report_self_hash` over canonical JSON with the field
  temporarily set to `sha256:0…0` (64 zero hex digits).
* The hash is recorded as a lowercase `sha256:<hex>` string in JSON.
* The raw 32-byte digest is what is fed into `compose_key` when this
  report's hash is consumed by a downstream stage's
  `TypedInputBundle`.

### 7.3 Canonical-JSON rule

Inherited from F-B2/F-B4 §2.5 unchanged:

* UTF-8 JSON object keys in lexicographic order at every object level.
* No insignificant whitespace.
* Integer fields are base-10 JSON numbers.
* Floating-point fields are forbidden in v1 reports. Quantities use
  `_q8_8` or `_q16_16` fixed-point integer fields.
* Arrays whose order is semantically meaningful are explicitly
  specified per-schema.

F-B14 specifically forbids floating-point fields anywhere in
`schedule_cost.json`. Cycles, bytes, and ratios are all integer or
Q16.16 fixed-point. (Q16.16 covers the "scheduler headroom utilization
ratio" and "video-commit cost margin" fields, which would otherwise
be naturally floating-point.)

`cache_status.json` is integer-only: hashes are `sha256:<hex>` strings,
status is an enum string, key bodies are not embedded (only the
final K-key hash is recorded).

### 7.4 Diagnostic envelope

Inherited from F-B2/F-B4 §7.1:

```rust
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}
```

F-B14 extends `ValidationOrigin` with one new variant
`ScheduleCostAnalysis`. F-B17 extends with one new variant
`StageCacheValidation`.

```rust
pub enum ValidationOrigin {
    // ... existing F-B2/F-B4/F-B3/F-B5/F-B6/F-B7/F-B8/F-B9/F-B10/
    //     F-B11/F-B12/F-B13 variants ...
    ScheduleCostAnalysis,            // NEW (F-B14)
    StageCacheValidation,            // NEW (F-B17)
}
```

Both new origins extend the closed enum without modifying existing
variants.

### 7.5 Severity discipline

Inherited from F-B2/F-B4 §7.1 unchanged:

* `Hard`: build cannot proceed.
* `Soft`: recorded in report; build proceeds.

F-B14 and F-B17 emit only `Hard` diagnostics. There are no `Soft`
diagnostics in this chunk.

```text
∀ d ∈ Stage11.diagnostics. d.severity = Hard
∀ d ∈ F-B17.diagnostics.   d.severity = Hard
```

### 7.6 Pass output envelope (Stage 11)

```rust
pub struct ScheduleCostPassOutputs {
    pub product: ScheduleCostReport,
    pub report: ReportEnvelope<ScheduleCostReportBody>,
}

pub enum ScheduleCostPassResult {
    Passed(ScheduleCostPassOutputs),
    Failed(ReportEnvelope<ScheduleCostReportBody>, Vec<ValidationDiagnostic>),
}
```

`ScheduleCostReport` is the typed product; `ReportEnvelope` carries the
canonical-JSON `schedule_cost.json` body plus self-hash. On failure,
the report is still emitted (with `outcome = Failed`); the product is
not. This matches the F-B2/F-B4 §2.1 pass shape.

### 7.7 Cross-stage F-B17 envelope

F-B17 does not have a "pass" envelope of its own. It is a cross-stage
validator and producer of `cache_status.json`. The build driver
collects per-stage `CacheStatus` values as stages complete, then
emits `cache_status.json` after the last stage terminates.

```rust
pub struct CacheStatusReportBody {
    pub per_stage: BTreeMap<StageId, StageCacheStatusEntry>,
    pub build_summary: CacheStatusBuildSummary,
}

pub struct StageCacheStatusEntry {
    pub stage_id: StageId,
    pub k_key: StageCacheKey,
    pub status: CacheStatus,
    pub input_identity_hash: Hash256,
    pub product_self_hash: Option<Hash256>,
    pub report_self_hash: Option<Hash256>,
}

pub struct CacheStatusBuildSummary {
    pub total_stages: u16,
    pub hit_count: u16,
    pub miss_count: u16,
    pub stale_count: u16,
    pub not_applicable_count: u16,
}
```

`build_summary` totals must be consistent with `per_stage`: the four
counts sum to `total_stages`, which equals `per_stage.len()`.

The semantic validator for `cache_status.json` enforces:

```text
CS-PerStageOrderingTotal:
  per_stage.keys() == {0, 0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5, 9, 10,
                       10.5, 11, 12}  (all 16 stages)

CS-StatusConsistency:
  hit + miss + stale + not_applicable == total_stages

CS-ProductHashPresence:
  product_self_hash.is_some() iff status ∈ {Hit, Miss, Stale}
                                  AND the stage produced a product
  (i.e., on Hit / Miss / Stale where the stage actually ran or
   replayed a successful product).
  status == NotApplicable ⇒ product_self_hash.is_none()
                           AND report_self_hash.is_none()
```

`NotApplicable` is the status used when a stage was structurally
skipped: e.g., F-B14 when no `RuntimeMode` was requested (which is
illegal but recorded if it ever happened); or a hypothetical Stage 8.5
when `RomWindowPlan` produced no `WramOverlay` kernels (an empty but
still-applicable case is `Hit`/`Miss`/`Stale`, not `NotApplicable`).
In practice, M3 builds will not produce any `NotApplicable` entries;
the variant exists for future stage variants gated behind
`CompileKnobs`.

## 8. Stage 11 contract: `ScheduleCostAnalysis`

### 8.1 Type-level contract

This subsection pins every type F-B14 owns. Each type is `Serialize +
Deserialize` with `deny_unknown_fields`, `Eq`, `Hash`, and (where
applicable) `Ord` so canonical-JSON serialization is total.

#### 8.1.1 `EvidenceClass` — the load-bearing claim for F-B16

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    /// Backed by a calibration bundle whose declared
    /// CalibrationConfidenceClass is Measured for the active
    /// (TargetProfile, KernelSpec set, RuntimeMode) tuple.
    /// The bundle is hash-bound through CalibrationBundleRef.
    Calibrated,
    /// Backed by a calibration bundle measured on a *related*
    /// target / kernel and applied through a typed transfer
    /// function. CalibrationConfidenceClass is Transferred.
    /// The transfer function itself has its own EvidenceRef.
    Transferred,
    /// Closed-form heuristic over schedule structure (slice count,
    /// op count, residency choice). No calibration bundle was
    /// consulted, or the consulted bundle was below the active
    /// RiskPolicy::calibration_confidence_requirement. Carries a
    /// typed FallbackReason.
    Heuristic,
    /// A heuristic estimate further degraded because the inputs
    /// to the heuristic itself were missing or below confidence.
    /// Carries a typed FallbackReason::UpstreamFallback chain.
    Fallback,
}
```

`Ord` implementation: the variants are *ordered* `Calibrated <
Transferred < Heuristic < Fallback` for purposes of "tightest evidence"
comparisons (lower variant = tighter evidence). Note this ordering is
*reversed* from the everyday "stronger > weaker" convention; the choice
is to match Rust's default `Ord` for enums with declared order.

The taxonomy is closed. A future RFC that wants to add (e.g.)
`MeasuredOnDifferentMode` must explicitly amend §8.1.1.

#### 8.1.2 `UncertaintyEnvelope`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct UncertaintyEnvelope {
    /// p50 (median) point estimate. Always present.
    pub p50_q16_16: i64,
    /// p95 lower bound. Must be ≤ p50.
    pub p95_lower_q16_16: i64,
    /// p95 upper bound. Must be ≥ p50.
    pub p95_upper_q16_16: i64,
    /// p99 upper bound. Must be ≥ p95_upper.
    /// Optional because some metrics (e.g., bank-switch counts under
    /// Top1 routing) have a closed-form upper bound that is also the
    /// p99 bound; recording it twice is redundant.
    pub p99_upper_q16_16: Option<i64>,
}
```

Constraints (semantic-validator level):

```text
UE-NonNegative:
  p50_q16_16 ≥ 0
  p95_lower_q16_16 ≥ 0
  p95_upper_q16_16 ≥ 0
  (p99_upper_q16_16 ≥ 0 when present)

UE-Ordering:
  p95_lower_q16_16 ≤ p50_q16_16 ≤ p95_upper_q16_16
  p99_upper_q16_16.is_some() ⇒ p99_upper_q16_16 ≥ p95_upper_q16_16

UE-FixedPointRange:
  every q16_16 field ≤ I64::MAX
  (i.e., the integer part fits in 47 bits; the fractional part
   takes the remaining 16 bits.)
```

Q16.16 is the chosen fixed-point format because it covers the dynamic
range needed: cycles per token at p99 may reach ~10^9 (for long
prompts), and Q16.16 in i64 covers ±2^47 ≈ 1.4 × 10^14 in the
integer part, with 16 bits of fractional precision — enough to record
"bank-switch count = 1.5 (Q16.16 = 0x18000)" or "headroom utilization
= 0.875 (Q16.16 = 0xE000)" without loss.

#### 8.1.3 `EvidenceRef`

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceRef {
    /// A calibration bundle resolved through gbf-store::BlobStore.
    CalibrationBundle {
        bundle_kind: CalibrationBundleKind,
        bundle_hash: Hash256,
        record_path: CalibrationRecordPath,
        confidence: CalibrationConfidenceClass,
    },
    /// A typed cycle model derived from a kernel calibration record.
    CycleModel {
        kernel_spec: KernelSpecId,
        cycle_model_hash: Hash256,
    },
    /// A heuristic policy: closed-form rule documented in §8.5.
    HeuristicPolicy {
        policy_id: HeuristicPolicyId,
        policy_version: SemVer,
    },
    /// A typed transfer function from one calibration target to
    /// another. The transfer function itself is hash-bound.
    TransferPolicy {
        transfer_policy_id: TransferPolicyId,
        from_target: TargetProfileId,
        to_target: TargetProfileId,
        policy_hash: Hash256,
    },
    /// A composition step: the parent estimate's evidence chain
    /// is the union of the components' evidence chains.
    Composition {
        component_refs: Vec<EvidenceRef>,
    },
}
```

`CalibrationRecordPath` is the typed path within a calibration bundle
to a specific record (e.g., `kernel.matvec.dense_i8.cycles_per_op`).
It is a `gbf-policy::calibration` type (named-only here; the full
shape lands with the F-E2/F-E3 calibration-bundle features).

#### 8.1.4 `FallbackReason`

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FallbackReason {
    /// No calibration bundle was supplied for the active target.
    NoBundleForTarget {
        target: TargetProfileId,
    },
    /// The supplied bundle's declared confidence was below the
    /// active RiskPolicy::calibration_confidence_requirement.
    ConfidenceBelowRequirement {
        declared: CalibrationConfidenceClass,
        required: CalibrationConfidenceClass,
    },
    /// The KernelSpec invoked at this site is not present in the
    /// calibration bundle's KernelCalibrationBundle.
    KernelSpecNotCalibrated {
        kernel_spec: KernelSpecId,
    },
    /// The calibration bundle was present but the matching record
    /// was rejected as stale by the freshness gate.
    BundleStale {
        stale_field: StaleField,
        declared: Hash256,
        observed: Hash256,
    },
    /// The KernelSpec exists but no inputs match the requested
    /// per-mode, per-residency, per-tile-class measurement record.
    MeasurementShapeMismatch {
        detail: ShapeMismatchDetail,
    },
    /// A composition fallback: an upstream component-level estimate
    /// fell back, so the composed estimate also falls back.
    UpstreamFallback {
        upstream: Box<FallbackReason>,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleField {
    TargetProfileHash,
    KernelSetHash,
    PackerVersion,
    CalibrationSchemaHash,
    ValidityEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ShapeMismatchDetail {
    pub kernel_spec: KernelSpecId,
    pub requested_mode: RuntimeMode,
    pub requested_residency: KernelResidency,
    pub requested_tile_class: TileClassId,
    pub available_modes: BTreeSet<RuntimeMode>,
    pub available_residencies: BTreeSet<KernelResidency>,
    pub available_tile_classes: BTreeSet<TileClassId>,
}
```

`UpstreamFallback` allows the FallbackReason to chain: e.g., a per-mode
envelope's cycles_per_token estimate may fall back because the
slice-level cycle estimate fell back, which fell back because the
kernel-level cycle estimate fell back. The chain is recorded so
reviewers can read "this estimate is `Fallback` because [chain]."

#### 8.1.5 `CostEstimate` — the per-axis estimate

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CostEstimate {
    pub evidence_class: EvidenceClass,
    pub envelope: UncertaintyEnvelope,
    pub refs: Vec<EvidenceRef>,
    pub fallback_reason: Option<FallbackReason>,
}
```

Constraints:

```text
CE-EvidenceClassConsistent:
  evidence_class ∈ {Calibrated, Transferred} ⇒
    refs is non-empty AND fallback_reason.is_none()
  evidence_class ∈ {Heuristic, Fallback} ⇒
    fallback_reason.is_some()

CE-RefsForCalibrated:
  evidence_class == Calibrated ⇒
    refs contains at least one EvidenceRef::CalibrationBundle whose
    confidence ∈ {Measured} (per CalibrationConfidenceClass ordering)

CE-RefsForTransferred:
  evidence_class == Transferred ⇒
    refs contains at least one EvidenceRef::TransferPolicy AND
    at least one EvidenceRef::CalibrationBundle (the source bundle).

CE-RefsForHeuristic:
  evidence_class == Heuristic ⇒
    refs contains at least one EvidenceRef::HeuristicPolicy.

CE-RefsForFallback:
  evidence_class == Fallback ⇒
    fallback_reason is FallbackReason::UpstreamFallback OR
    refs contains at least one EvidenceRef::HeuristicPolicy.
```

These constraints are mechanically checked by the semantic validator
for `schedule_cost.json`.

#### 8.1.6 `EstimatedCostDelta` — the per-mode envelope

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EstimatedCostDelta {
    /// Predicted cycles per token. Required.
    pub cycles_per_token: CostEstimate,
    /// Predicted bank-switches per token (count). Required.
    pub bank_switches_per_token: CostEstimate,
    /// Predicted SRAM page-switches per token (count). Required
    /// when the build has any persistent sequence-state pages
    /// (i.e., F-B9 SramPagePlan produced ≥ 1 page); otherwise None.
    pub sram_page_switches_per_token: Option<CostEstimate>,
    /// Predicted yields per token (count). Required.
    pub yields_per_token: CostEstimate,
    /// Scheduler headroom utilization ratio (0..1, Q16.16). Required.
    pub scheduler_headroom_utilization: CostEstimate,
    /// Video-commit cost margin (cycles, signed: positive means under
    /// budget, negative means over budget). Required for builds that
    /// have a UI runtime nucleus; None for headless builds.
    pub video_commit_cost_margin: Option<CostEstimate>,
    /// Maximum no-progress estimate (frames). Required.
    pub max_no_progress_estimate: CostEstimate,
    /// Time-to-first-token distribution (cycles). Required.
    pub time_to_first_token: CostEstimate,
    /// Sustained throughput (tokens / 1e6 cycles, Q16.16). Required.
    pub sustained_throughput_tokens_per_megacycle: CostEstimate,
    /// Frame-jitter distribution (cycles). Required for builds whose
    /// CompileObjective includes LowFrameJitter; None otherwise.
    pub frame_jitter: Option<CostEstimate>,
}
```

Each field carries its own `CostEstimate` with its own evidence class.
Different fields may have different evidence classes within the same
`EstimatedCostDelta`: e.g., `cycles_per_token` may be `Calibrated`
while `frame_jitter` is `Heuristic` because the workload mix needed
to calibrate frame jitter has not been collected yet.

The `Option`-typed fields exist because the relevant axis may be
structurally absent: `sram_page_switches_per_token` is only meaningful
when persistent pages exist; `video_commit_cost_margin` only when the
build has a UI; `frame_jitter` only when the objective requires it.
The semantic validator pins which `Option` fields are required for
which build shapes (§8.3).

#### 8.1.7 `ScheduleCostReport` — top-level

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduleCostReport {
    pub objective: CompileObjective,
    pub per_mode: BTreeMap<RuntimeMode, EstimatedCostDelta>,
    pub satisfaction: ScheduleCostObjectiveSatisfactionMatrix,
    pub refs: Vec<EvidenceRef>,
    pub identity: ScheduleCostIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduleCostIdentity {
    pub schedule_pack_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub calibration_bundle_set_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub kernel_spec_registry_hash: Hash256,
    pub pass_version: SemVer,
    pub crate_feature_set_hash: Hash256,
    pub schedule_cost_schema_hash: Hash256,
}
```

`identity` records every load-bearing input by hash so reviewers can
diff two reports byte-against-byte and immediately identify which
input drifted.

`refs` is the *union* of every `EvidenceRef` consulted by every
`CostEstimate` in the report. The full union is recorded once at the
top level; per-`CostEstimate` `refs` lists are local subsets so the
report is browsable at multiple granularities.

#### 8.1.8 `ScheduleCostObjectiveSatisfactionMatrix`

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScheduleCostObjectiveSatisfactionMatrix {
    pub entries: BTreeMap<SatisfactionKey, ObjectiveSatisfaction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SatisfactionKey {
    pub mode: RuntimeMode,
    pub axis: ObjectiveAxis,
    pub quantile: Quantile,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveSatisfaction {
    /// Predicted value is within target plus uncertainty tolerance,
    /// even at the most pessimistic end of the uncertainty envelope.
    Satisfied,
    /// Predicted value is within target plus uncertainty tolerance
    /// only because of uncertainty headroom; pessimistic end exceeds
    /// the target.
    Borderline,
    /// Predicted value exceeds target even at the most favorable
    /// (optimistic) end of the uncertainty envelope.
    Violated,
}
```

`SatisfactionKey` is `Ord` so canonical-JSON serialization of the
entries map is total and deterministic.

The satisfaction matrix is computed deterministically from
`per_mode` and `objective`; it is *redundant* with that data but is
materialized in the report for two reasons:

1. `cache_status.json` consumers (dashboards, F-B16, F-B15 report
   emitters) want the matrix without re-running cost rollup.
2. The matrix is a compact summary readable by humans without parsing
   `EstimatedCostDelta` envelopes.

The semantic validator enforces consistency: every entry in
`satisfaction.entries` must agree with what would be computed by
re-projecting `objective` against `per_mode`.

#### 8.1.9 Quantile

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantile {
    P50,
    P95,
    P99,
}
```

Closed enum; future quantiles (e.g., `P999`) require an explicit
amendment.

#### 8.1.10 Public type table

| Type                                        | Crate / module                |
|---------------------------------------------|-------------------------------|
| `EvidenceClass`                             | `gbf-policy::cost`            |
| `UncertaintyEnvelope`                       | `gbf-policy::cost`            |
| `EvidenceRef`                               | `gbf-policy::cost`            |
| `CalibrationBundleRef`                      | `gbf-policy::cost`            |
| `CycleModelRef` (struct)                    | `gbf-policy::cost`            |
| `FallbackReason`                            | `gbf-policy::cost`            |
| `StaleField`                                | `gbf-policy::cost`            |
| `ShapeMismatchDetail`                       | `gbf-policy::cost`            |
| `CostEstimate`                              | `gbf-policy::cost`            |
| `EstimatedCostDelta`                        | `gbf-policy::cost`            |
| `ScheduleCostReport`                        | `gbf-policy::cost`            |
| `ScheduleCostIdentity`                      | `gbf-policy::cost`            |
| `ScheduleCostObjectiveSatisfactionMatrix`   | `gbf-policy::cost`            |
| `SatisfactionKey`                           | `gbf-policy::cost`            |
| `ObjectiveSatisfaction`                     | `gbf-policy::cost`            |
| `Quantile`                                  | `gbf-policy::cost`            |
| `HeuristicPolicyId`, `TransferPolicyId`     | `gbf-policy::cost`            |
| `CompileObjective`, `ObjectiveAxis`         | `gbf-policy::objective` (named-only here, owned by F-B2/F-B4) |
| Stage 11 implementation                     | `gbf-codegen::stages::schedule_cost` |
| `schedule_cost.json` schema                 | `gbf-report::schedule_cost`   |

### 8.2 Construction order

The pure-core `build_schedule_cost_core` is a deterministic function
of typed inputs. The construction order is:

```text
1. Compute per-slice cost projections.

   For each RuntimeMode m in resolved_policy.requested_runtime_modes:
     For each SchedSlice s in schedule_pack.modes[m].slices:
       For each SchedOp op in s.ops:
         resolve KernelSpecId for op (if op invokes a kernel)
         resolve CycleModelRef from CalibrationBundleSet
         compute per-op cycle estimate with EvidenceClass
         compute per-op bank-switch contribution
         compute per-op SRAM-page-switch contribution
       sum per-slice cycles, bank-switches, SRAM-page-switches
       record per-slice CostEstimate with composed EvidenceClass

2. Per-token rollup.

   For each RuntimeMode m:
     compute tokens-per-frame from SchedulePack (the per-mode
     execution budget; see §8.5 for the mapping)
     compute cycles_per_token = sum(slice cycles in token frame)
     compute bank_switches_per_token = sum(slice bank_switches)
     compute sram_page_switches_per_token = ...
     compute yields_per_token = count(YieldKind::Token)
     compute scheduler_headroom_utilization = predicted / FrameBudget
     compute video_commit_cost_margin (if UI runtime nucleus present)
     compute max_no_progress_estimate
     compute time_to_first_token (from prompt-warmup phase of m)
     compute sustained_throughput from steady-state phase of m
     compute frame_jitter (if objective requires)
     each estimate composes the EvidenceClass via the rules in §8.4
     each estimate's envelope composes per the rules in §8.5

3. Per-objective satisfaction.

   For each RuntimeMode m:
     For each (axis, quantile) in objective.quantile_targets:
       project predicted value against target
       record ObjectiveSatisfaction:
         Satisfied  if pessimistic_end ≤ target
         Borderline if predicted ≤ target but pessimistic_end > target
         Violated   if optimistic_end > target

4. Fallback marking.

   For each CostEstimate e in per_mode:
     if e.evidence_class ∈ {Heuristic, Fallback}:
       record e.fallback_reason
       if e is composed of upstream estimates that themselves
         fell back, set e.fallback_reason = UpstreamFallback(...)

5. Build top-level ScheduleCostReport.

   per_mode := the per-mode rollup map
   objective := resolved_policy.objective (read-only)
   satisfaction := the matrix from step 3
   refs := union of every CostEstimate.refs
   identity := input hashes per §8.1.7

6. Build ReportEnvelope and compute report_self_hash.

   schema_id := "schedule_cost.v1"
   schema_version := SemVer::new(1, 0, 0)
   body := ScheduleCostReportBody { report: ScheduleCostReport, ... }
   diagnostics := []  (success path; failure path emits Hard diagnostics)
   report_self_hash := R-Hash(body)

7. Return ScheduleCostPassOutputs.
```

The pure core is total: every input combination produces either
`Passed(outputs)` or `Failed(envelope, diagnostics)`. There is no
panic path.

### 8.3 Self-consistency rules

The semantic validator for `schedule_cost.json` enforces the following
rules at parse time. Failure of any rule emits a Hard diagnostic with
origin `ScheduleCostAnalysis`.

```text
SC-PerModeTotal:
  per_mode.keys() == resolved_policy.requested_runtime_modes
                     (set equality)

SC-PerModeNonEmpty:
  per_mode.keys() is non-empty.

SC-EveryEstimateClassed:
  ∀ estimate e in per_mode (recursively):
    e.evidence_class ∈ {Calibrated, Transferred, Heuristic, Fallback}.

SC-EvidenceClassRefsConsistent:
  per CE-EvidenceClassConsistent rules in §8.1.5.

SC-EvidenceRefResolves:
  ∀ EvidenceRef r appearing in any CostEstimate.refs (recursively):
    r resolves through gbf-store::BlobStore to a present blob.
    For r ∈ {CalibrationBundle, CycleModel}, the resolved bytes
    canonical-JSON-roundtrip matches the recorded bundle_hash /
    cycle_model_hash.

SC-EnvelopeOrdering:
  per UE-Ordering and UE-NonNegative in §8.1.2.

SC-FallbackReasonPresent:
  ∀ CostEstimate e: e.evidence_class ∈ {Heuristic, Fallback} ⇒
    e.fallback_reason.is_some()

SC-FallbackReasonAbsent:
  ∀ CostEstimate e: e.evidence_class ∈ {Calibrated, Transferred} ⇒
    e.fallback_reason.is_none()

SC-OptionFieldsRequired:
  - sram_page_switches_per_token.is_some() iff the build's
    SramPagePlan product has ≥ 1 page.
  - video_commit_cost_margin.is_some() iff the build's
    target profile declares a UI runtime nucleus.
  - frame_jitter.is_some() iff objective.additional contains
    LowFrameJitter or quantile_targets has an axis = LowFrameJitter.

SC-IdentityComplete:
  identity.* is non-zero except where explicitly nullable
  (no fields are nullable in v1).

SC-SatisfactionMatrixConsistent:
  ∀ key (mode, axis, quantile) in objective.quantile_targets:
    satisfaction.entries[key] equals the value computed by
    projecting per_mode[mode] against target_value at quantile.

SC-SatisfactionMatrixTotal:
  satisfaction.entries.keys() ==
    {(m, axis, q) | m ∈ requested_runtime_modes,
                    (axis, q, _) ∈ objective.quantile_targets}

SC-NoFloatingPoint:
  no field in ScheduleCostReport (recursively) is f32 or f64.
  Q16.16 fields are i64 with the documented scaling.

SC-RefsUnion:
  refs at top level == ⋃ over all CostEstimate.refs in per_mode
                       (no duplicates by structural equality)
```

The validator runs at every report deserialization (e.g., when F-B15
re-reads `schedule_cost.json` to populate `budget.json`), so a
malformed report is caught on read, not just on emit.

### 8.4 Canonical reference semantics — small-step semantics for cost rollup

This subsection pins the semantics of "compose an EvidenceClass and
UncertaintyEnvelope across components" so the rollup is deterministic
and reviewable.

#### 8.4.1 Per-op composition

For an op invoking kernel `kspec` with residency `r` and tile class
`tc` in mode `m`:

```text
lookup_record(CB, m, kspec, r, tc) :=
  if CB.kernel.records contains an exact match:
    Calibrated, with the matched record's distribution
  elif CB.kernel.records contains a transfer-eligible match:
    Transferred, with the transferred distribution
  else:
    Heuristic, with the heuristic policy producing a distribution
```

#### 8.4.2 Per-slice composition

Per-slice cycles are a *sum* of per-op cycles. Composition rules:

```text
EvidenceClass composition (per-slice):
  Calibrated  if every component is Calibrated
  Transferred if every component is ∈ {Calibrated, Transferred}
              and at least one is Transferred
  Heuristic   if every component is ∈ {Calibrated, Transferred,
                Heuristic} and at least one is Heuristic
  Fallback    if any component is Fallback

UncertaintyEnvelope composition (per-slice sum):
  p50_q16_16 := Σ_components p50_q16_16
  p95_lower_q16_16 := Σ_components p95_lower_q16_16  (best case)
  p95_upper_q16_16 := Σ_components p95_upper_q16_16  (worst case)
  p99_upper_q16_16 := if all components have p99: Σ p99_upper, else None
```

This is a **conservative** composition: it does not assume independence
across components. A more sophisticated composition (e.g., assuming
independence and adding variances in quadrature) would produce tighter
envelopes but require an explicit independence claim. The conservative
sum is the v1 default; a future amendment may introduce a typed
`IndependenceClaim` for components that may safely use quadrature.

#### 8.4.3 Per-token composition

Per-token cycles are a sum over slices in the per-token frame:

```text
cycles_per_token(m) :=
  Σ over slices s in token_frame(m):
    s.cycles_estimate
```

The composition rules from §8.4.2 apply.

`token_frame(m)` is the SchedulePack's notion of "one token of work in
mode m." For the M3 routed-FFN profile, this is the schedule from the
token-input slice through the post-classify slice. Trace mode may
include extra probe slices, which is why per-mode envelopes can differ.

#### 8.4.4 Per-objective satisfaction

For an `(axis, quantile, target_value, uncertainty_tolerance)` target:

```text
predicted := pick predicted CostEstimate for (mode, axis)
upper(quantile) :=
  if quantile == P50: predicted.envelope.p50_q16_16
  if quantile == P95: predicted.envelope.p95_upper_q16_16
  if quantile == P99: predicted.envelope.p99_upper_q16_16 or
                      predicted.envelope.p95_upper_q16_16

lower(quantile) :=
  similarly, using p95_lower / p50

satisfaction :=
  if upper(quantile) ≤ target_value + uncertainty_tolerance:
    Satisfied
  elif lower(quantile) ≤ target_value + uncertainty_tolerance:
    Borderline
  else:
    Violated
```

This is the small-step satisfaction rule. It is total: every
`(mode, axis, quantile)` triple in the objective produces exactly
one `ObjectiveSatisfaction`.

### 8.5 Calibration handshake — `gbf-bench` / calibration_bundle binding

This subsection pins how F-B14 binds to calibration bundles and how
cycle models are resolved at compile time.

#### 8.5.1 Calibration bundle resolution

`CalibrationBundleSet` is the resolved input to F-B14 (carried through
`ResolvedCompilePolicy.calibration` into the F-B14 input bundle). It
contains zero or one of each kind:

```rust
pub struct CalibrationBundleSet {
    pub platform: Option<CalibrationBundleRef>,
    pub kernel: Option<CalibrationBundleRef>,
    pub runtime: Option<CalibrationBundleRef>,
    pub schema_version: SemVer,
}
```

F-B14 dereferences each `CalibrationBundleRef` through
`gbf-store::BlobStore::get_ref`. The resolved bytes are
canonical-JSON-deserialized into the typed bundle struct (e.g.,
`PlatformCalibrationBundle`). The hash of the resolved bytes must
equal the recorded `bundle_hash`; if not, F-B14 emits
`CostCalibrationBundleHashMismatch` (Hard).

#### 8.5.2 KernelSpec resolution

For each `SchedOp` that invokes a kernel, F-B14 needs to look up the
calibrated cycle model. The lookup key is:

```text
KernelLookupKey :=
  (kernel_spec_id: KernelSpecId,
   runtime_mode:    RuntimeMode,
   residency:       KernelResidency,
   tile_class:      TileClassId)
```

The `KernelCalibrationBundle` is keyed by this tuple. F-B14 walks the
bundle's records:

```text
resolve_cycle_model(kspec, m, r, tc):
  if kernel_bundle.contains_exact(kspec, m, r, tc):
    return Calibrated(kernel_bundle.get(kspec, m, r, tc))
  elif kernel_bundle.contains_transfer_source(kspec, m, r, tc):
    apply transfer_policy
    return Transferred(transferred_distribution)
  else:
    return Heuristic(default_heuristic(kspec, m, r, tc))
```

`default_heuristic` is a closed-form policy documented under
`HeuristicPolicyId::CyclesPerOpDefault` (§8.5.4).

#### 8.5.3 Residency-class modifiers

A kernel's predicted cycles_per_op depends on residency:
* `Bank0Fixed`: cycles_per_op_baseline (no overhead).
* `WramOverlay`: cycles_per_op_baseline + overlay_install_amortized
  (one-time install cost amortized over invocations within the
  install lifetime).
* `BankSwitchable`: cycles_per_op_baseline + bank_switch_amortized.

The amortization factor comes from the `KernelCalibrationBundle`'s
`residency_class_modifiers` field. F-B14 records the amortization
choice in the `EvidenceRef` chain so reviewers can audit it.

#### 8.5.4 Heuristic policies

When falling back to `Heuristic`, F-B14 uses one of a closed set of
named policies:

| `HeuristicPolicyId`           | Applies when                                    | Definition |
|-------------------------------|-------------------------------------------------|------------|
| `CyclesPerOpDefault`          | No calibrated record for kernel                 | cycles_per_op = ROW × COL × 4 (matvec); ROW × 8 (residual); ROW × 16 (norm); fixed per-op for routing/decode |
| `BankSwitchesUpperBound`      | No calibrated bank-switch record                | conservative upper = #(distinct_banks_per_token) under StrictOnePerBank |
| `SramPageSwitchesUpperBound`  | No calibrated SRAM-page record                  | conservative upper = #(distinct_pages_per_token) |
| `YieldsPerTokenStatic`        | Always (yields are a structural property)       | yields_per_token = count(slices with yield_kind == TokenReady) |
| `HeadroomUtilizationDefault`  | No calibrated headroom record                   | predicted_headroom = predicted_cycles / FrameBudget.scheduler_target_cycles |
| `VideoCommitMarginDefault`    | No calibrated video-commit record               | budget.video_commit_target - predicted_video_commit_cycles |
| `MaxNoProgressEstimateDefault`| No calibrated no-progress record                | conservative upper = max_slice_hard_cycles / target_frame_cycles |
| `TimeToFirstTokenDefault`     | No calibrated TTFT record                       | sum(cycles for prompt-warmup slices) |
| `SustainedThroughputDefault`  | No calibrated sustained-throughput record       | reciprocal of cycles_per_token |
| `FrameJitterDefault`          | No calibrated jitter record                     | (max_slice_cycles - min_slice_cycles) over yield-bounded slices |

Each `HeuristicPolicyId` has a `policy_version: SemVer`. A change to
the heuristic formula bumps `policy_version`, which propagates into
every `EvidenceRef::HeuristicPolicy` and ultimately into the K14 cache
key (because it affects the report identity through `pass_version`).

#### 8.5.5 Transfer policies

When a calibrated record exists for a *related* target/kernel but not
the active one, F-B14 may apply a typed transfer:

```rust
pub struct TransferPolicy {
    pub policy_id: TransferPolicyId,
    pub policy_version: SemVer,
    pub from_target: TargetProfileId,
    pub to_target: TargetProfileId,
    pub policy_hash: Hash256,
}
```

Transfer policies are out-of-tree: they are part of the calibration
bundle ecosystem and are consulted by F-B14 by hash. F-B14 itself
does not implement transfer logic; it resolves a `TransferPolicyId`
through `gbf-store::BlobStore` to a typed policy struct, applies it,
and records the hash in the `EvidenceRef` chain.

In the v1 chunk, the only registered transfer policy is the identity
policy `TransferPolicyId::Identity`, which applies when source and
target profiles agree on every load-bearing dimension and only differ
in cosmetic identity (e.g., a profile family rename). This is enough
to validate the transfer machinery without committing to non-trivial
transfer math.

#### 8.5.6 Bundle freshness gate

F-B14 enforces the same calibration freshness gate as F-B2 Stage 0:

```text
freshness_check(CB.<kind>):
  bundle.target_profile_hash == active.target_profile_hash AND
  bundle.kernel_set_hash == active.kernel_set_hash AND
  bundle.packer_version == active.packer_version AND
  bundle.calibration_schema_hash == active.calibration_schema_hash AND
  bundle.validity_envelope contains active build's session_profile

  if any fails: emit FallbackReason::BundleStale { stale_field, ... }
```

A stale bundle does NOT cause F-B14 to fail; it causes the affected
estimate to fall back to `Heuristic` with a typed `BundleStale`
reason. The build proceeds with reduced confidence. Whether F-B14
emits a top-level `Failed` envelope depends on the active
`RiskPolicy::calibration_confidence_requirement` (§2.14).

#### 8.5.7 Calibration bundle hash binding

The K14 cache key includes the `CalibrationBundleSet` identity:

```text
calibration_bundle_set_hash :=
  DomainHash("gbf-policy", "CalibrationBundleSet", "v1",
    CanonicalJson({
      platform_hash: CB.platform.map(|r| r.bundle_hash),
      kernel_hash: CB.kernel.map(|r| r.bundle_hash),
      runtime_hash: CB.runtime.map(|r| r.bundle_hash),
    }))
```

A change to any bundle's content changes its hash, changes
`calibration_bundle_set_hash`, changes K14, and produces a cache miss.
A bundle whose hash is unchanged but whose `validity_envelope`
implicitly excludes the current build is detected by the freshness
gate and produces `Fallback` evidence in the report (without changing
K14, because the validity envelope is already part of the bundle's
content).

## 9. F-B17 contract: StageCache integration sweep

### 9.1 Canonical-input convention (formal)

For each stage S in the pipeline, the **canonical-input convention**
is:

> The `StageCacheKey` K(S) is a total deterministic function of a
> typed `TypedInputBundle(S)` plus three meta-fields (`pass_version_S`,
> `crate_feature_set_hash`, `stage_S_schema_hash`). No other input may
> affect K(S). `TypedInputBundle(S)` consists exclusively of
> hash-bound fields: `Hash256`s, `BlobRef`s, content projections, and
> typed scalars. There are no `Option<T>` fields whose `None` value
> hides a missing input; absent inputs are normalized to a canonical
> empty value with a stable hash.

Formally:

```text
For stage S, build B:
  TIB(S, B) :: TypedInputBundle(S)
  K(S, B)  :=  compose_key(
                 stage_id    = StageId(S),
                 shard_local = canonical_digest_set(TIB(S, B)),
                 global      = global_identity_hash(TIB(S, B)),
                 feature_flags = crate_feature_set,
                 pass_version = pass_version_S
               )

Convention enforced by F-B17:
  ∀ S, ∀ B, B':
    K(S, B) = K(S, B')  ⟺  TIB(S, B) = TIB(S, B')

Equivalent: K(S, ·) is a bijection between TypedInputBundle values
            and StageCacheKey values, for fixed pass_version_S /
            crate_feature_set / stage_S_schema_hash.
```

Two-direction property:

```text
F-Cache-Forward:
  TIB(S, B) = TIB(S, B') ⇒ K(S, B) = K(S, B')
  (Trivial: compose_key is deterministic.)

F-Cache-Backward:
  TIB(S, B) ≠ TIB(S, B') ⇒ K(S, B) ≠ K(S, B')
  (Required for cache correctness.)
```

The forward direction is enforced by F-A6's `compose_key`. The
backward direction is enforced by per-stage tests in F-B17 (§9.6).

### 9.2 Per-stage StageCache key index

This subsection enumerates *every* stage in the pipeline, its
`TypedInputBundle`, its key body, its product, and its report. A new
stage introduced by a later RFC must amend this section.

Convention: each stage is named by its index in the pipeline (Stage 0,
0.5, 1, 2, 3, 4, 5, 6, 7, 8, 8.5, 9, 10, 10.5, 11, 12). Owning RFCs are
cited; the canonical key body is restated here for the cross-stage
inventory.

#### 9.2.1 Stage 0: `ArtifactValidationAndUpgrade`

Owning RFC: F-B2/F-B4.
TypedInputBundle:
```text
TypedInputBundle(0) := {
  artifact_source_hash:                 Hash256,
  artifact_effective_core_hash:         Option<Hash256>,
  artifact_manifest_hash:               Option<Hash256>,
  artifact_aux_hash:                    Option<Hash256>,
  lowering_manifest_hash:               Option<Hash256>,
  hint_bundle_hash:                     Hash256,
  compile_request_hash:                 Hash256,
  target_profile_hash:                  Hash256,
  compile_profile_hash:                 Hash256,
  calibration_hash:                     Option<Hash256>,
  compatibility_adapter_registry_hash:  Hash256,
  pass_version:                         "stage0/v1",
  crate_feature_set_hash:               Hash256,
  artifact_validation_schema_hash:      Hash256,
}
```
Note: `Option<Hash256>` fields here are *normalized* to a
canonical-empty representation (e.g., `None` ↦ `Hash256::ZERO`) when
hashed. This is per F-B2/F-B4 §11 and the normalization prelude
discipline.

Cached output: `ValidationProduct` (success) or
`artifact_validation.json` + `Vec<ValidationDiagnostic>` (failure memo).

#### 9.2.2 Stage 0.5: `ResolvedCompilePolicy`

Owning RFC: F-B2/F-B4.
TypedInputBundle:
```text
TypedInputBundle(0.5) := {
  artifact_validation_self_hash:        Hash256,
  validated_input_hashes:               ValidatedInputHashes,
  target_defaults_hash:                 Hash256,
  compile_profile_hash:                 Hash256,
  profile_defaults_hash:                Hash256,
  compile_objective_hash:               Hash256,
  pass_version:                         "stage0_5/v1",
  crate_feature_set_hash:               Hash256,
  policy_resolution_schema_hash:        Hash256,
}
```

Cached output: `ResolvedPolicyProduct` + `policy_resolution.json`.

#### 9.2.3 Stage 1: `QuantGraph`

Owning RFC: F-B3/F-B5 §11.
TypedInputBundle:
```text
TypedInputBundle(1) := {
  artifact_validation_self_hash:    Hash256,
  policy_resolution_self_hash:      Hash256,
  artifact_effective_core_hash:     Hash256,
  lowering_manifest_hash:           Hash256,
  resolved_blob_index_hash:         Hash256,
  pass_version:                     "stage1/v1",
  crate_feature_set_hash:           Hash256,
  quant_graph_schema_hash:          Hash256,
}
```

Cached output: `QuantGraphProduct` + `quant_graph.json`.

K1 (per F-B3/F-B5):
```text
K1 := DomainHash("gbf-codegen", "StageCacheKey", "quant_graph.v1",
        schema_version, CanonicalJson(TypedInputBundle(1)))
```

#### 9.2.4 Stage 2: `StaticBudgetReport`

Owning RFC: F-B2/F-B4.
TypedInputBundle:
```text
TypedInputBundle(2) := {
  policy_resolution_self_hash:      Hash256,
  quant_graph_self_hash:            Hash256,
  runtime_chrome_budget_hash:       Hash256,
  target_profile_hash:              Hash256,
  pass_version:                     "stage2/v1",
  crate_feature_set_hash:           Hash256,
  static_budget_schema_hash:        Hash256,
}
```

Cached output: `StaticBudgetReport` + `static_budget.json`.

#### 9.2.5 Stage 3: `GbInferIR`

Owning RFC: F-B3/F-B5 §11.
TypedInputBundle:
```text
TypedInputBundle(3) := {
  quant_graph_self_hash:            Hash256,
  infer_ir_policy_projection_hash:  Hash256,
  static_budget_self_hash:          Hash256,
  pass_version:                     "stage3/v1",
  crate_feature_set_hash:           Hash256,
  infer_ir_schema_hash:             Hash256,
}
```

K3 (per F-B3/F-B5).

`infer_ir_policy_projection_hash` is the typed projection of
`ResolvedCompilePolicy` consisting only of fields F-B5 reads
(`requested_runtime_modes`, observability mode, decode policy). Other
`ResolvedCompilePolicy` fields are NOT in K3, so a change to (e.g.)
`max_refinement_iters` does not invalidate K3.

#### 9.2.6 Stage 4: `ObservationPlan`

Owning RFC: F-B6 (forthcoming).
TypedInputBundle (provisional, to be pinned by F-B6):
```text
TypedInputBundle(4) := {
  infer_ir_self_hash:               Hash256,
  observation_policy_projection_hash: Hash256,
  pass_version:                     "stage4/v1",
  crate_feature_set_hash:           Hash256,
  observation_plan_schema_hash:     Hash256,
}
```

Cached output: `ObservationPlanProduct` + `observation_plan.json` +
optional `observation.cert.json`.

If F-B6's eventual `TypedInputBundle(4)` differs from the provisional
shape above, that RFC must explicitly amend §9.2.6 here.

#### 9.2.7 Stage 5: `RangePlan`

Owning RFC: F-B7 (forthcoming).
TypedInputBundle (provisional):
```text
TypedInputBundle(5) := {
  infer_ir_self_hash:               Hash256,
  observation_plan_self_hash:       Hash256,
  static_budget_self_hash:          Hash256,
  range_plan_policy_projection_hash: Hash256,
  pass_version:                     "stage5/v1",
  crate_feature_set_hash:           Hash256,
  range_plan_schema_hash:           Hash256,
}
```

Cached output: `RangePlanProduct` + `range_plan.json` +
`range.cert.json`.

#### 9.2.8 Stage 6: `StoragePlan`

Owning RFC: F-B8 (forthcoming).
TypedInputBundle (provisional):
```text
TypedInputBundle(6) := {
  infer_ir_self_hash:               Hash256,
  range_plan_self_hash:             Hash256,
  observation_plan_self_hash:       Hash256,
  storage_plan_policy_projection_hash: Hash256,
  pass_version:                     "stage6/v1",
  crate_feature_set_hash:           Hash256,
  storage_plan_schema_hash:         Hash256,
}
```

Cached output: `StoragePlanProduct` + `storage_plan.json`.

#### 9.2.9 Stage 7: `SramPagePlan`

Owning RFC: F-B9 (forthcoming).
TypedInputBundle (provisional):
```text
TypedInputBundle(7) := {
  storage_plan_self_hash:           Hash256,
  runtime_chrome_budget_hash:       Hash256,
  target_profile_hash:              Hash256,
  sram_page_plan_policy_projection_hash: Hash256,
  pass_version:                     "stage7/v1",
  crate_feature_set_hash:           Hash256,
  sram_page_plan_schema_hash:       Hash256,
}
```

Cached output: `SramPagePlanProduct` + `sram_page_plan.json` +
`sram.cert.json`.

#### 9.2.10 Stage 8: `RomWindowPlan`

Owning RFC: F-B10 (forthcoming).
TypedInputBundle (provisional):
```text
TypedInputBundle(8) := {
  storage_plan_self_hash:           Hash256,
  range_plan_self_hash:             Hash256,
  runtime_chrome_budget_hash:       Hash256,
  target_profile_hash:              Hash256,
  rom_window_policy_projection_hash: Hash256,
  pass_version:                     "stage8/v1",
  crate_feature_set_hash:           Hash256,
  rom_window_plan_schema_hash:      Hash256,
}
```

Cached output: `RomWindowPlanProduct` + `rom_window_plan.json` +
`window.cert.json`.

#### 9.2.11 Stage 8.5: `OverlayPlan`

Owning RFC: F-B11/F-B12 §13.1.
TypedInputBundle:
```text
TypedInputBundle(8.5) := {
  storage_plan_self_hash:           Hash256,
  sram_page_plan_self_hash:         Hash256,
  rom_window_plan_self_hash:        Hash256,
  runtime_chrome_budget_hash:       Hash256,
  target_profile_hash:              Hash256,
  overlay_plan_policy_projection_hash: Hash256,
  pass_version:                     "stage8_5/v1",
  crate_feature_set_hash:           Hash256,
  overlay_plan_schema_hash:         Hash256,
}
```

K11 (per F-B11/F-B12).

Cached output: `OverlayPlanProduct` + `overlay_plan.json` + optional
`overlay.cert.json`.

#### 9.2.12 Stage 9: `ArenaPlan`

Owning RFC: F-B11/F-B12 §13.2.
TypedInputBundle:
```text
TypedInputBundle(9) := {
  storage_plan_self_hash:           Hash256,
  sram_page_plan_self_hash:         Hash256,
  rom_window_plan_self_hash:        Hash256,
  overlay_plan_self_hash:           Hash256,
  runtime_chrome_budget_hash:       Hash256,
  target_profile_hash:              Hash256,
  arena_plan_policy_projection_hash: Hash256,
  pass_version:                     "stage9/v1",
  crate_feature_set_hash:           Hash256,
  arena_plan_schema_hash:           Hash256,
}
```

K12 (per F-B11/F-B12).

Cached output: `ArenaPlanProduct` + `arena_plan.json` +
`arena.cert.json`.

#### 9.2.13 Stage 10: `GbSchedIR`

Owning RFC: F-B13 (forthcoming).
TypedInputBundle (provisional):
```text
TypedInputBundle(10) := {
  arena_plan_self_hash:             Hash256,
  overlay_plan_self_hash:           Hash256,
  rom_window_plan_self_hash:        Hash256,
  sram_page_plan_self_hash:         Hash256,
  storage_plan_self_hash:           Hash256,
  range_plan_self_hash:             Hash256,
  observation_plan_self_hash:       Hash256,
  infer_ir_self_hash:               Hash256,
  sched_ir_policy_projection_hash:  Hash256,
  pass_version:                     "stage10/v1",
  crate_feature_set_hash:           Hash256,
  sched_ir_schema_hash:             Hash256,
}
```

Cached output: `GbSchedIRProduct` (per-mode `SchedulePack`) +
`sched_ir.json`.

#### 9.2.14 Stage 10.5: `ResourceStateValidation`

Owning RFC: F-B13 (forthcoming).
TypedInputBundle (provisional):
```text
TypedInputBundle(10.5) := {
  sched_ir_self_hash:               Hash256,
  pass_version:                     "stage10_5/v1",
  crate_feature_set_hash:           Hash256,
  resource_state_schema_hash:       Hash256,
}
```

Cached output: `ResourceStateValidationProduct` +
`resource_state.cert.json`.

Note: This stage's input bundle is small because resource-state
validation is a structural property of `GbSchedIR` alone — leases,
yields, ISR-reachability are computed from the schedule. The
validation product is content-addressed but rarely useful as a cache
hit because Stage 10.5's compute is small relative to other stages.
F-B17 still wires it for consistency.

#### 9.2.15 Stage 11: `ScheduleCostAnalysis` (F-B14, this RFC)

TypedInputBundle:
```text
TypedInputBundle(11) := {
  schedule_pack_self_hash:          Hash256,
  policy_resolution_self_hash:      Hash256,
  calibration_bundle_set_hash:      Hash256,
  runtime_chrome_budget_hash:       Hash256,
  target_profile_hash:              Hash256,
  kernel_spec_registry_hash:        Hash256,
  schedule_cost_policy_projection_hash: Hash256,
  pass_version:                     "stage11/v1",
  crate_feature_set_hash:           Hash256,
  schedule_cost_schema_hash:        Hash256,
}
```

K14 (per §11 of this RFC).

`schedule_cost_policy_projection_hash` is the typed projection of
`ResolvedCompilePolicy` consisting only of fields F-B14 reads:
`compile_objective`, `risk_policy.calibration_confidence_requirement`,
`requested_runtime_modes`. Other `ResolvedCompilePolicy` fields are NOT
in K14.

Cached output: `ScheduleCostReportProduct` + `schedule_cost.json`.

#### 9.2.16 Stage 12: Backend (`AsmIR -> ReachabilityValidation -> PlacedRom -> EncodedRom`)

Owning RFC: F-B15 (forthcoming).
TypedInputBundle (provisional, to be pinned by F-B15):
```text
TypedInputBundle(12) := {
  sched_ir_self_hash:                  Hash256,
  resource_state_self_hash:            Hash256,
  schedule_cost_self_hash:             Hash256,
  arena_plan_self_hash:                Hash256,
  overlay_plan_self_hash:              Hash256,
  rom_window_plan_self_hash:           Hash256,
  policy_resolution_self_hash:         Hash256,
  target_profile_hash:                 Hash256,
  backend_policy_projection_hash:      Hash256,
  runtime_nucleus_hash:                Hash256,
  pass_version:                        "stage12/v1",
  crate_feature_set_hash:              Hash256,
  backend_schema_hash:                 Hash256,
}
```

Cached output: `BackendProduct` (encoded ROM bytes + `.sym` + `.lst`)
+ `backend.json` + `reachability_report.json` +
`reachability.cert.json` + `map.json` + `provenance.json`.

Stage 12 is the largest stage by output volume; cache hits here are
the iteration-loop speedup that F-B17 enables.

#### 9.2.17 Pipeline-order summary

```text
| Stage | Owner   | Key  | Product                       | Cert      |
| ----- | ------- | ---- | ----------------------------- | --------- |
| 0     | F-B2    | K0   | ValidationProduct             | -         |
| 0.5   | F-B2    | K0.5 | ResolvedPolicyProduct         | -         |
| 1     | F-B3    | K1   | QuantGraphProduct             | -         |
| 2     | F-B4    | K2   | StaticBudgetReport            | -         |
| 3     | F-B5    | K3   | GbInferIRProduct              | -         |
| 4     | F-B6    | K4   | ObservationPlanProduct        | optional  |
| 5     | F-B7    | K5   | RangePlanProduct              | range     |
| 6     | F-B8    | K6   | StoragePlanProduct            | -         |
| 7     | F-B9    | K7   | SramPagePlanProduct           | sram      |
| 8     | F-B10   | K8   | RomWindowPlanProduct          | window    |
| 8.5   | F-B11   | K11  | OverlayPlanProduct            | optional  |
| 9     | F-B12   | K12  | ArenaPlanProduct              | arena     |
| 10    | F-B13   | K10  | GbSchedIRProduct (SchedulePack) | -      |
| 10.5  | F-B13   | K10.5| ResourceStateValidationProduct| resource_state |
| 11    | F-B14   | K14  | ScheduleCostReportProduct     | -         |
| 12    | F-B15   | K12B | BackendProduct (ROM)          | reachability |
```

`K12B` is used to disambiguate Stage 12's key from K12 (Stage 9). The
in-tree implementation uses `StageId::Backend` as the opaque newtype,
so disambiguation by name rather than by number is mechanical.

### 9.3 Hit / miss / stale semantics

For each stage S, F-B17's call-site wrapper performs the following on
each invocation:

```text
fn run_stage_with_cache<S>(inputs: TypedInputBundle<S>, ctx: &BuildContext)
  -> Result<PassOutputs<S>, PassDiagnostics<S>>
{
  let k = compose_key(stage_id::<S>(), &inputs, ctx.pass_version, ctx.feature_flags);

  match ctx.stage_cache.get(&k)? {
    Some(entry) if entry.is_success() => {
      let report = canonical_json::deserialize(&entry.report_bytes)?;
      let body_hash = canonical_json::self_hash_check(&report)?;
      if body_hash != entry.report_self_hash {
        ctx.stage_cache.poison(&k)?;
        ctx.cache_status.record(stage_id::<S>(), CacheStatus::Stale);
        return run_stage_uncached_and_store(inputs, ctx, k);
      }
      let product = canonical_json::deserialize(&entry.product_bytes)?;
      ctx.cache_status.record(stage_id::<S>(), CacheStatus::Hit);
      Ok(PassOutputs { product, report })
    }
    Some(entry) if entry.is_failure_memo() => {
      let report = canonical_json::deserialize(&entry.report_bytes)?;
      let diags = canonical_json::deserialize(&entry.diagnostics_bytes)?;
      ctx.cache_status.record(stage_id::<S>(), CacheStatus::Hit);  // failure-memo hit
      Err(PassDiagnostics { report, diagnostics: diags })
    }
    None => {
      ctx.cache_status.record(stage_id::<S>(), CacheStatus::Miss);
      run_stage_uncached_and_store(inputs, ctx, k)
    }
  }
}
```

`Stale` status is reached only via the body-hash check failure path
above. In practice, *Stale* occurs only when a cached entry is
present but its body has been corrupted in the BlobStore (e.g., a
manual edit). The normal "input drifted" case is *Miss*, not Stale,
because input drift produces a different K(S) which is a cache miss,
not a stale hit.

`NotApplicable` status is reached when the build driver determines
the stage is structurally absent (e.g., a future Stage 8.5 invocation
on a build with no overlayable kernels) and skips it without
cache lookup.

### 9.4 Regenerate-on-stale + Trace-mode override

#### 9.4.1 Regenerate-on-stale

`run_stage_uncached_and_store` is the regenerate path:

```text
fn run_stage_uncached_and_store<S>(inputs, ctx, k)
  -> Result<PassOutputs<S>, PassDiagnostics<S>>
{
  match run_stage_pure_core::<S>(inputs)? {
    Passed(outputs) => {
      ctx.stage_cache.put_success(
        &k,
        canonical_json::serialize(&outputs.product),
        canonical_json::serialize(&outputs.report),
        outputs.report.report_self_hash,
      )?;
      Ok(outputs)
    }
    Failed(envelope, diags) => {
      ctx.stage_cache.put_failure_memo(
        &k,
        canonical_json::serialize(&envelope),
        canonical_json::serialize(&diags),
        envelope.report_self_hash,
      )?;
      Err(PassDiagnostics { report: envelope, diagnostics: diags })
    }
  }
}
```

The same code path serves `Miss` and `Stale`. There is no separate
"stale-recovery" code.

#### 9.4.2 Trace-mode override

When `ResolvedCompilePolicy.observability_mode == Invariant` *and* the
build is a Trace profile, the build driver also writes per-stage
snapshots to `stages/<stage_id>.json`:

```text
ctx.stages_dir.write_snapshot(
  stage_id::<S>(),
  canonical_json::serialize(&outputs.product),
  canonical_json::serialize(&outputs.report),
);
```

This is on top of cache writes, not a substitute for them. The
snapshot is identical to the cache entry's body bytes; reviewers may
diff `stages/<S>.json` between two builds without consulting the
cache.

`stages/` snapshot writes do NOT participate in cache-key
construction. They are a sidecar output.

#### 9.4.3 `--resume-from <stage>` (out of scope here)

`--resume-from <stage>` is a CLI-level control layered on top of the
F-B17 cache primitives by `gbf-cli`. It works by:
1. Computing K(S) for every stage S < target_stage.
2. Reading the cached product + report from the cache.
3. Skipping the actual `run_stage_pure_core` call for those stages.

This is mechanical and does not require new F-B17 surface. The
control belongs to `gbf-cli` and is tracked separately.

### 9.5 cache_status.json schema

```rust
pub struct CacheStatusReportBody {
    pub per_stage: BTreeMap<StageId, StageCacheStatusEntry>,
    pub build_summary: CacheStatusBuildSummary,
}

pub struct StageCacheStatusEntry {
    pub stage_id: StageId,
    pub k_key: StageCacheKey,
    pub status: CacheStatus,
    pub input_identity_hash: Hash256,
    pub product_self_hash: Option<Hash256>,
    pub report_self_hash: Option<Hash256>,
}

pub enum CacheStatus {
    Hit,
    Miss,
    Stale,
    NotApplicable,
}

pub struct CacheStatusBuildSummary {
    pub total_stages: u16,
    pub hit_count: u16,
    pub miss_count: u16,
    pub stale_count: u16,
    pub not_applicable_count: u16,
}
```

Canonical-JSON serialization rules (inherited from F-B2/F-B4 §2.5):
* `per_stage` is a JSON object whose keys are sorted lexicographically
  by `StageId` string form.
* `status` is the snake_case enum string ("hit", "miss", "stale",
  "not_applicable").
* `k_key` is the `sha256:<hex>` form of the StageCacheKey hash.
* `input_identity_hash` is the `sha256:<hex>` form of
  `H(TypedInputBundle(S))`.
* `product_self_hash` and `report_self_hash` are `Option<String>` —
  serialized as `null` when `None` (this is the only legal `null` use
  in `cache_status.json` v1).

#### 9.5.1 `cache_status.json` placement

`cache_status.json` lives at the root of the build output package
alongside `map.json`, `provenance.json`, `budget.json`, etc.

Per planv0 line 2820, the `stages/` directory contains per-stage
snapshots under Trace builds or any time the StageCache is cold.
`cache_status.json` is a *single file* alongside `stages/`, not inside it.

#### 9.5.2 `cache_status.json` self-hash

Like every other v1 report, `cache_status.json` carries a
`report_self_hash`. The self-hash is computed by the F-B2/F-B4 §2.4
convention.

### 9.6 Per-stage test obligations

For each stage S, F-B17 lands the following tests:

#### 9.6.1 Forward: equal typed inputs ⇒ equal key

```text
test forward_equal_inputs_equal_key:
  let inputs1 = TypedInputBundle::<S>::synthesize(seed=1)
  let inputs2 = inputs1.clone()
  assert_eq!(compose_key(&inputs1, ...), compose_key(&inputs2, ...))
```

This is mechanical and proves the cache-correctness property for
identical input replays.

#### 9.6.2 Backward: per-field load-bearing flip changes the key

For every named field f in `TypedInputBundle(S)`:

```text
test backward_field_<f>_change_changes_key:
  let inputs1 = TypedInputBundle::<S>::synthesize(seed=1)
  let mut inputs2 = inputs1.clone()
  inputs2.<f> = TypedInputBundle::<S>::flip_field_<f>(inputs1.<f>)
  assert_ne!(compose_key(&inputs1, ...), compose_key(&inputs2, ...))
```

The test count is `Σ_S |fields(TypedInputBundle(S))|` ≈ 100–140
assertions. Each is mechanical.

#### 9.6.3 Negative: non-load-bearing change does NOT change the key

For changes outside the typed input bundle (e.g., environment
variables, wall-clock readings, fixture filenames):

```text
test no_change_for_environment_variable:
  let inputs1 = TypedInputBundle::<S>::synthesize(seed=1)
  set_env("EXTRA_VAR", "anything")
  let inputs2 = TypedInputBundle::<S>::synthesize(seed=1)  // re-synth
  assert_eq!(compose_key(&inputs1, ...), compose_key(&inputs2, ...))
```

This proves the canonical-input convention: only typed-input fields
affect the key.

#### 9.6.4 Cache-poisoning recovery

```text
test cache_poison_recovers:
  let k = compose_key(...)
  ctx.stage_cache.put_success(&k, b"corrupted_bytes",
                              b"corrupted_report", Hash256::ZERO)
  let outputs = run_stage_with_cache::<S>(inputs, ctx)?
  // outputs should be a fresh run, not the corrupted bytes
  assert!(outputs.report.report_self_hash != Hash256::ZERO)
  assert_eq!(ctx.cache_status.get(stage_id::<S>()), CacheStatus::Stale)
```

This proves `F-Cache-Read-Validate` from F-B11/F-B12 §13.3 is honored
at every stage.

#### 9.6.5 Failure memo replay

```text
test failure_memo_replays:
  let k = compose_key(&bad_inputs, ...)
  let _ = run_stage_with_cache::<S>(bad_inputs.clone(), ctx)?  // first run, fails
  let result = run_stage_with_cache::<S>(bad_inputs, ctx)
  // second run should be a failure-memo hit
  assert!(result.is_err())
  assert_eq!(ctx.cache_status.get(stage_id::<S>()), CacheStatus::Hit)
```

Per F-A6 / F-B2/F-B4 §2.6, failure memos may be replayed on
byte-identical inputs.

#### 9.6.6 Cross-stage: K(S2) miss after K(S1) input flip

```text
test cross_stage_dependency:
  // build a fixture, run pipeline through Stage S1 + S2
  let outputs_S1_run1 = run_stage::<S1>(inputs1, ctx)
  let outputs_S2_run1 = run_stage::<S2>(
    inputs2_from(outputs_S1_run1), ctx)

  // flip a field in S1's typed input
  let mut inputs1_flipped = inputs1.clone()
  inputs1_flipped.<f> = ...
  let outputs_S1_run2 = run_stage::<S1>(inputs1_flipped, ctx)
  let outputs_S2_run2 = run_stage::<S2>(
    inputs2_from(outputs_S1_run2), ctx)

  // both K(S1) and K(S2) must miss
  assert_eq!(cache_status[S1, run2], Miss)
  assert_eq!(cache_status[S2, run2], Miss)
```

This proves cache invalidation cascades through the pipeline. F-B17
runs this test for every pair (S1, S2) where S2 directly consumes
S1's product.

#### 9.6.7 cache_status.json schema round-trip

```text
test cache_status_json_round_trip:
  let report = build_synthetic_cache_status_report()
  let bytes = canonical_json::serialize(&report)
  let parsed: CacheStatusReportBody = canonical_json::deserialize(&bytes)
  assert_eq!(parsed, report)
  assert_eq!(canonical_json::self_hash(&parsed),
             report.report_self_hash)
```

#### 9.6.8 cache_status.json semantic validation

```text
test cache_status_validates:
  let mut report = build_synthetic_cache_status_report()
  // perturb: hit + miss + stale + not_applicable does not equal total
  report.build_summary.hit_count += 1
  let result = semantic_validate(&report)
  assert!(matches!(result, Err(_)))
```

This catches schema drift.

### 9.7 Integration discipline

F-B17's per-stage call-site wrapper has a uniform shape:

```rust
// in gbf-codegen::stages::<stage_module>:

pub fn run_stage_S(
    inputs: StageSInputs<'_>,
    ctx: &mut BuildContext,
) -> Result<StageSOutputs, StageSDiagnostics> {
    let typed_inputs = TypedInputBundle::<S>::from(inputs);
    crate::stage_cache::run_stage_with_cache::<S>(typed_inputs, ctx)
}
```

The stage-specific work (running the pure core, building the report)
lives inside `run_stage_with_cache`'s `run_stage_pure_core::<S>` call
through a typed `StageRunner<S>` trait:

```rust
pub trait StageRunner<S>: Sized {
    fn run_pure_core(inputs: TypedInputBundle<S>)
        -> Result<PassOutputs<S>, (ReportEnvelope<S>, Vec<ValidationDiagnostic>)>;
}
```

Each stage module implements `StageRunner<S>` for its stage type.
F-B17 lands the trait definition, the wrapper, and the per-stage
trait impls for every existing stage (F-B2 Stage 0 + 0.5, F-B4 Stage
2, F-B11/F-B12 Stage 8.5/9; the F-B3/F-B5 Stage 1/3 wiring is already
landed by those RFCs and is *re-uniformized* by F-B17 to use the
`StageRunner` trait).

For stages whose RFCs have not yet landed (F-B6 Stage 4, F-B7 Stage 5,
F-B8 Stage 6, F-B9 Stage 7, F-B10 Stage 8, F-B13 Stages 10/10.5,
F-B15 Stage 12), F-B17 introduces a *placeholder* `StageRunner<S>`
impl that returns `Err(StagePlaceholder)` and is replaced by the
per-stage RFC's real impl when that stage lands. The placeholder
is enough to land the cache-status emitter and the cross-stage tests;
the real wiring incrementally replaces placeholders as stages
implement.

### 9.8 Incremental landing strategy

F-B17 explicitly supports incremental landing:

```text
F-B17 = bd-1g7k = N tasks T-B17.0 .. T-B17.M
  T-B17.0  = StageRunner trait + placeholder impls + cache_status
              schema + per-stage typed-input-bundle test scaffold
              + uniform wrapper.
  T-B17.<S> = wire stage S into the StageRunner trait
              (one task per stage S that exists today; placeholders
               for stages that don't exist yet).
  T-B17.M  = per-stage typed-input-bundle conformance test +
              cross-stage dependency test + cache_status.json
              emitter + closure.
```

Each per-stage wiring task can land independently after the
corresponding stage RFC closes. The closure of F-B17 itself happens
when:

1. T-B17.0 is closed (uniform wrapper + scaffold).
2. Every stage that exists at closure time has a real
   `StageRunner<S>` impl wired (placeholders are explicitly OK for
   stages whose RFCs are forthcoming).
3. The per-stage typed-input-bundle conformance test passes for every
   stage.
4. The cross-stage dependency test passes for every pair (S1, S2)
   where both S1 and S2 are wired.
5. `cache_status.json` round-trips and semantically validates.

Stages whose RFCs land *after* F-B17 closes must amend §9.2 here and
land their `StageRunner<S>` impl in their owning RFC's bead. F-B17's
closure does not block their later integration.

### 9.9 F-B17 NEVER computes its own key

F-B17 has no key of its own. Its outputs are:

* per-stage `StageRunner<S>` impls (already keyed by their stage's K).
* the cross-stage `cache_status.json` report (whose body is
  `BTreeMap<StageId, StageCacheStatusEntry>`, a deterministic function
  of every other stage's K and status).

The `cache_status.json` report is not itself cached. It is a
build-time aggregation, written once per build, and replayed not from
cache but from the per-stage cache-status table the build driver
maintains. Two byte-identical builds produce byte-identical
`cache_status.json` files but the file is not read back through F-A6's
`StageCache` — it is read back from disk if and only if a consumer
(dashboard, CI tool) wants it.

This is the load-bearing **F-B17 is a sweep, not a stage** rule.

## 10. Report schemas

### 10.1 `schedule_cost.json`

#### 10.1.1 Top-level shape

```text
{
  "schema_id": "schedule_cost.v1",
  "schema_version": "1.0.0",
  "body": {
    "outcome": "passed" | "failed",
    "result": null | <ScheduleCostReport>,
    "input_identity": <ScheduleCostIdentity>,
    "diagnostics": [<ValidationDiagnostic>, ...]
  },
  "report_self_hash": "sha256:<hex>"
}
```

The `body` is the `ReportEnvelope<ScheduleCostReportBody>`.

The `result` field is `Option<ScheduleCostReport>`:
* `outcome = passed ⇒ result.is_some()`.
* `outcome = failed ⇒ result.is_none()` and `diagnostics` contains
  at least one Hard diagnostic.

This matches the F-B2/F-B4 §2.1 pass shape.

Allowed `null` fields in `schedule_cost.v1`:

```text
body.result                    -- on outcome = failed only
```

No other fields are nullable in v1.

#### 10.1.2 `ScheduleCostReport` shape (per §8.1.7)

```text
{
  "objective": <CompileObjective>,
  "per_mode": {
    "<RuntimeMode>": <EstimatedCostDelta>,
    ...
  },
  "satisfaction": <ScheduleCostObjectiveSatisfactionMatrix>,
  "refs": [<EvidenceRef>, ...],
  "identity": <ScheduleCostIdentity>
}
```

Keys in `per_mode` are sorted lexicographically by `RuntimeMode`
serialized form (the snake_case name).

Keys in `satisfaction.entries` are sorted by `(mode, axis, quantile)`
tuple in lexicographic order.

#### 10.1.3 `EstimatedCostDelta` shape (per §8.1.6)

```text
{
  "cycles_per_token": <CostEstimate>,
  "bank_switches_per_token": <CostEstimate>,
  "sram_page_switches_per_token": null | <CostEstimate>,
  "yields_per_token": <CostEstimate>,
  "scheduler_headroom_utilization": <CostEstimate>,
  "video_commit_cost_margin": null | <CostEstimate>,
  "max_no_progress_estimate": <CostEstimate>,
  "time_to_first_token": <CostEstimate>,
  "sustained_throughput_tokens_per_megacycle": <CostEstimate>,
  "frame_jitter": null | <CostEstimate>
}
```

Allowed `null` fields in `EstimatedCostDelta`:
```text
sram_page_switches_per_token
video_commit_cost_margin
frame_jitter
```

These are nullable per the SC-OptionFieldsRequired rule in §8.3.
Any other null is a schema violation.

#### 10.1.4 `CostEstimate` shape (per §8.1.5)

```text
{
  "evidence_class": "calibrated" | "transferred" | "heuristic" | "fallback",
  "envelope": {
    "p50_q16_16": <i64>,
    "p95_lower_q16_16": <i64>,
    "p95_upper_q16_16": <i64>,
    "p99_upper_q16_16": null | <i64>
  },
  "refs": [<EvidenceRef>, ...],
  "fallback_reason": null | <FallbackReason>
}
```

#### 10.1.5 `EvidenceRef` shape (per §8.1.3)

Tagged enum:

```text
{ "kind": "calibration_bundle", "bundle_kind": "<kind>", "bundle_hash": "<sha256>", "record_path": "<path>", "confidence": "<class>" }
{ "kind": "cycle_model", "kernel_spec": "<id>", "cycle_model_hash": "<sha256>" }
{ "kind": "heuristic_policy", "policy_id": "<id>", "policy_version": "<semver>" }
{ "kind": "transfer_policy", "transfer_policy_id": "<id>", "from_target": "<id>", "to_target": "<id>", "policy_hash": "<sha256>" }
{ "kind": "composition", "component_refs": [<EvidenceRef>, ...] }
```

#### 10.1.6 `FallbackReason` shape (per §8.1.4)

Tagged enum:

```text
{ "kind": "no_bundle_for_target", "target": "<id>" }
{ "kind": "confidence_below_requirement", "declared": "<class>", "required": "<class>" }
{ "kind": "kernel_spec_not_calibrated", "kernel_spec": "<id>" }
{ "kind": "bundle_stale", "stale_field": "<field>", "declared": "<sha256>", "observed": "<sha256>" }
{ "kind": "measurement_shape_mismatch", "detail": <ShapeMismatchDetail> }
{ "kind": "upstream_fallback", "upstream": <FallbackReason> }
```

#### 10.1.7 `ScheduleCostIdentity` shape (per §8.1.7)

```text
{
  "schedule_pack_self_hash": "<sha256>",
  "policy_resolution_self_hash": "<sha256>",
  "calibration_bundle_set_hash": "<sha256>",
  "runtime_chrome_budget_hash": "<sha256>",
  "target_profile_hash": "<sha256>",
  "kernel_spec_registry_hash": "<sha256>",
  "pass_version": "stage11/v1",
  "crate_feature_set_hash": "<sha256>",
  "schedule_cost_schema_hash": "<sha256>"
}
```

Every field is a `sha256:<hex>` string except `pass_version` (a SemVer
prefixed string).

#### 10.1.8 Semantic validator

The semantic validator runs at parse time on every
`schedule_cost.json` deserialization. Failure emits a Hard
`ValidationDiagnostic` with origin `ScheduleCostAnalysis` and code
from §12.

The validator enforces every rule in §8.3 (SC-* rules). It rejects:

* Unknown fields (`deny_unknown_fields` on every struct).
* Unknown enum variants.
* Numeric values outside Q16.16 range.
* Inconsistent evidence-class / fallback-reason combinations.
* Missing per-mode entries.
* Missing required option fields per build shape.

#### 10.1.9 Worked example (passing build)

For a Bringup-profile build with one `RuntimeMode = Default`, one
expert, dense-only:

```text
{
  "schema_id": "schedule_cost.v1",
  "schema_version": "1.0.0",
  "body": {
    "outcome": "passed",
    "result": {
      "objective": {
        "primary": "time_to_first_token",
        "additional": ["sustained_throughput"],
        "quantile_targets": [
          { "axis": "time_to_first_token",
            "quantile": "p95",
            "target_value": 200000,
            "uncertainty_tolerance_q16_16": 6553600 }
        ],
        "trace_budget": null,
        "frame_jitter": null
      },
      "per_mode": {
        "default": {
          "cycles_per_token": {
            "evidence_class": "heuristic",
            "envelope": {
              "p50_q16_16": 9437184,
              "p95_lower_q16_16": 7864320,
              "p95_upper_q16_16": 12582912,
              "p99_upper_q16_16": null
            },
            "refs": [
              { "kind": "heuristic_policy",
                "policy_id": "cycles_per_op_default",
                "policy_version": "1.0.0" }
            ],
            "fallback_reason": {
              "kind": "no_bundle_for_target",
              "target": "dmg_mbc5_default"
            }
          },
          "bank_switches_per_token": {
            "evidence_class": "heuristic",
            "envelope": {
              "p50_q16_16": 65536,
              "p95_lower_q16_16": 65536,
              "p95_upper_q16_16": 65536,
              "p99_upper_q16_16": null
            },
            "refs": [
              { "kind": "heuristic_policy",
                "policy_id": "bank_switches_upper_bound",
                "policy_version": "1.0.0" }
            ],
            "fallback_reason": {
              "kind": "no_bundle_for_target",
              "target": "dmg_mbc5_default"
            }
          },
          "sram_page_switches_per_token": null,
          "yields_per_token": { ... },
          "scheduler_headroom_utilization": { ... },
          "video_commit_cost_margin": null,
          "max_no_progress_estimate": { ... },
          "time_to_first_token": { ... },
          "sustained_throughput_tokens_per_megacycle": { ... },
          "frame_jitter": null
        }
      },
      "satisfaction": {
        "entries": {
          "default,time_to_first_token,p95": "satisfied"
        }
      },
      "refs": [...],
      "identity": { ... }
    },
    "input_identity": { ... },
    "diagnostics": []
  },
  "report_self_hash": "sha256:..."
}
```

The values shown are illustrative; the byte-exact form is
deterministic for fixed inputs.

#### 10.1.10 Worked example (failing build — calibration missing under Default profile)

For a Default-profile build whose `RiskPolicy::calibration_confidence_requirement`
is `Measured` and no calibration bundle is supplied:

```text
{
  "schema_id": "schedule_cost.v1",
  "schema_version": "1.0.0",
  "body": {
    "outcome": "failed",
    "result": null,
    "input_identity": { ... },
    "diagnostics": [
      {
        "severity": "hard",
        "origin": "schedule_cost_analysis",
        "code": "cost_calibration_missing_for_requirement",
        "detail": {
          "required": "measured",
          "declared": null,
          "affected_modes": ["default"]
        },
        "provenance": [
          {
            "kind": "calibration_bundle",
            "bundle_kind": "kernel",
            "bundle_hash": "sha256:0000...",
            "record_path": "missing",
            "confidence": "none"
          }
        ]
      }
    ]
  },
  "report_self_hash": "sha256:..."
}
```

The `result` is `null` (per §10.1.1) and exactly one Hard diagnostic
is recorded.

### 10.2 `cache_status.json`

#### 10.2.1 Top-level shape

```text
{
  "schema_id": "cache_status.v1",
  "schema_version": "1.0.0",
  "body": {
    "per_stage": {
      "<StageId>": <StageCacheStatusEntry>,
      ...
    },
    "build_summary": <CacheStatusBuildSummary>
  },
  "report_self_hash": "sha256:<hex>"
}
```

`per_stage` keys are the canonical `StageId` strings:
`"stage_0"`, `"stage_0_5"`, `"stage_1"`, `"stage_2"`, `"stage_3"`,
`"stage_4"`, `"stage_5"`, `"stage_6"`, `"stage_7"`, `"stage_8"`,
`"stage_8_5"`, `"stage_9"`, `"stage_10"`, `"stage_10_5"`,
`"stage_11"`, `"stage_12"`. Underscore-decimal because dots are
problematic in JSON path notation; the convention is shared with
`gbf-store::stage_cache::StageId` newtype string form.

#### 10.2.2 `StageCacheStatusEntry` shape

```text
{
  "stage_id": "stage_<n>",
  "k_key": "sha256:<hex>",
  "status": "hit" | "miss" | "stale" | "not_applicable",
  "input_identity_hash": "sha256:<hex>",
  "product_self_hash": null | "sha256:<hex>",
  "report_self_hash": null | "sha256:<hex>"
}
```

Allowed `null` fields in `cache_status.v1`:

```text
per_stage.*.product_self_hash       -- when status == not_applicable
                                       OR when stage failed (no product)
per_stage.*.report_self_hash        -- when status == not_applicable
```

`product_self_hash` may be present-but-`None` when the stage failed
(failure-memo cache hits replay the failure report but the product
is None). `report_self_hash` is always present except on
`not_applicable` (failure-memo replays still produce the failure
report).

#### 10.2.3 `CacheStatusBuildSummary` shape

```text
{
  "total_stages": <u16>,
  "hit_count": <u16>,
  "miss_count": <u16>,
  "stale_count": <u16>,
  "not_applicable_count": <u16>
}
```

Constraint:

```text
hit_count + miss_count + stale_count + not_applicable_count
  == total_stages
total_stages == per_stage.len()
total_stages == 16   (in v1; pinned in §9.5.1)
```

#### 10.2.4 Semantic validator

The semantic validator runs at parse time. Failure emits a Hard
`ValidationDiagnostic` with origin `StageCacheValidation`.

Rules:

```text
CS-AllStagesPresent:
  per_stage.keys() == {stage_0, stage_0_5, stage_1, stage_2, stage_3,
                       stage_4, stage_5, stage_6, stage_7, stage_8,
                       stage_8_5, stage_9, stage_10, stage_10_5,
                       stage_11, stage_12}

CS-StatusEnumClosed:
  ∀ entry e in per_stage.values():
    e.status ∈ {hit, miss, stale, not_applicable}

CS-CountConsistency:
  hit_count == count(e | e.status == hit)
  miss_count == count(e | e.status == miss)
  stale_count == count(e | e.status == stale)
  not_applicable_count == count(e | e.status == not_applicable)

CS-NotApplicableProduct:
  ∀ e: e.status == not_applicable ⇒
    e.product_self_hash.is_none() AND e.report_self_hash.is_none()

CS-KKeyConsistency:
  ∀ e: e.k_key is a well-formed sha256:<hex> string

CS-IdentityHashConsistency:
  ∀ e: e.input_identity_hash is a well-formed sha256:<hex> string
```

#### 10.2.5 Worked example

For a fully-cached re-build (every stage hit cache):

```text
{
  "schema_id": "cache_status.v1",
  "schema_version": "1.0.0",
  "body": {
    "per_stage": {
      "stage_0":     { "stage_id": "stage_0",     "k_key": "sha256:...", "status": "hit", "input_identity_hash": "sha256:...", "product_self_hash": "sha256:...", "report_self_hash": "sha256:..." },
      "stage_0_5":   { ... "status": "hit" ... },
      "stage_1":     { ... "status": "hit" ... },
      "stage_2":     { ... "status": "hit" ... },
      "stage_3":     { ... "status": "hit" ... },
      "stage_4":     { ... "status": "hit" ... },
      "stage_5":     { ... "status": "hit" ... },
      "stage_6":     { ... "status": "hit" ... },
      "stage_7":     { ... "status": "hit" ... },
      "stage_8":     { ... "status": "hit" ... },
      "stage_8_5":   { ... "status": "hit" ... },
      "stage_9":     { ... "status": "hit" ... },
      "stage_10":    { ... "status": "hit" ... },
      "stage_10_5":  { ... "status": "hit" ... },
      "stage_11":    { ... "status": "hit" ... },
      "stage_12":    { ... "status": "hit" ... }
    },
    "build_summary": {
      "total_stages": 16,
      "hit_count": 16,
      "miss_count": 0,
      "stale_count": 0,
      "not_applicable_count": 0
    }
  },
  "report_self_hash": "sha256:..."
}
```

For a build that flipped one input at Stage 1, causing a cascade:

```text
"per_stage": {
  "stage_0":     { ..., "status": "hit" },
  "stage_0_5":   { ..., "status": "hit" },
  "stage_1":     { ..., "status": "miss" },
  "stage_2":     { ..., "status": "miss" },     // depends on Stage 1
  "stage_3":     { ..., "status": "miss" },     // depends on Stage 1
  "stage_4":     { ..., "status": "miss" },
  ...
  "stage_12":    { ..., "status": "miss" }
}
"build_summary": {
  "total_stages": 16,
  "hit_count": 2,
  "miss_count": 14,
  "stale_count": 0,
  "not_applicable_count": 0
}
```

The cascade is mechanical and predicted by the per-stage K-key
inputs: changing K1 changes Stage 2's `quant_graph_self_hash` input,
changing K3's `quant_graph_self_hash` input, etc. Reviewers can use
`cache_status.json` to identify the *first* stage that missed and
diagnose what input changed.

## 11. StageCache algebra

### 11.1 Stage 11 key (K14)

K14 follows the F-B2/F-B4 §11 / F-B3/F-B5 §11
`DomainHash(crate, "StageCacheKey", schema_id, schema_version,
canonical_json_bytes)` rule.

```text
StageCacheKeyHash(schema_id, schema_version, body) :=
  DomainHash("gbf-codegen", "StageCacheKey", schema_id, schema_version,
    CanonicalJson(body))

K14 :=
  StageCacheKeyHash("schedule_cost.v1", schema_version,
    ScheduleCostCacheKeyBody)

ScheduleCostCacheKeyBody := {
  schedule_pack_self_hash:               Hash256,
  policy_resolution_self_hash:           Hash256,
  calibration_bundle_set_hash:           Hash256,
  runtime_chrome_budget_hash:            Hash256,
  target_profile_hash:                   Hash256,
  kernel_spec_registry_hash:             Hash256,
  schedule_cost_policy_projection_hash:  Hash256,
  pass_version:                          "stage11/v1",
  crate_feature_set_hash:                Hash256,
  schedule_cost_schema_hash:             Hash256,
}
```

`schedule_cost_policy_projection_hash` is the typed projection of
`ResolvedCompilePolicy` into the F-B14-relevant subset:

```text
schedule_cost_policy_projection_hash :=
  DomainHash("gbf-codegen", "ScheduleCostPolicyProjection",
    "schedule_cost.v1", CanonicalJson(ScheduleCostPolicyProjection))

ScheduleCostPolicyProjection := {
  compile_objective:    CompileObjective,
  risk_policy_calibration_confidence_requirement: CalibrationConfidenceClass,
  requested_runtime_modes: BTreeSet<RuntimeMode>,
}
```

Cache miss occurs when any field of `ScheduleCostPolicyProjection`
changes. Other fields of the larger `ResolvedCompilePolicy` (e.g.,
`max_refinement_iters`, `observability_mode`, decode policy) that are
not in the projection do **not** invalidate K14.

`policy_resolution_self_hash` is recorded in
`ScheduleCostReportBody.identity` (audit-parent only); it is not part
of K14 because it would over-invalidate (a knob change unrelated to
F-B14 should not blow away cached cost analysis).

`calibration_bundle_set_hash` is the canonical hash of the
`CalibrationBundleSet`:

```text
calibration_bundle_set_hash :=
  DomainHash("gbf-policy", "CalibrationBundleSet", "v1",
    CanonicalJson({
      platform_hash: CB.platform.map(|r| r.bundle_hash).unwrap_or(ZERO),
      kernel_hash:   CB.kernel.map(|r| r.bundle_hash).unwrap_or(ZERO),
      runtime_hash:  CB.runtime.map(|r| r.bundle_hash).unwrap_or(ZERO),
    }))
```

`Hash256::ZERO` normalizes "no bundle of this kind" to a stable hash;
this is the same convention used by F-B2 Stage 0 for optional
identity hashes.

`kernel_spec_registry_hash` is the canonical hash of the active
KernelSpec registry — the typed catalogue of kernels available for
the build's target/profile. This pins F-B14 to a consistent registry;
a registry update changes K14.

### 11.2 K14 cache laws

Inheriting from F-A6, F-B2/F-B4, F-B3/F-B5, and F-B11/F-B12:

```text
K14-Success:
  Stage 11 result Passed ⇒ StageCache may store ScheduleCostReport
  product + schedule_cost.json report.

K14-NoFalseSuccess:
  Stage 11 result Failed ⇒ StageCache must not store success product.

K14-FailureMemo:
  Stage 11 result Failed ⇒ StageCache may memoize the canonical
  failure report (schedule_cost.json with outcome = Failed) and the
  diagnostics list under exact input-hash match.

K14-PassVersion:
  pass_version_stage11 changes ⇒ K14 changes ⇒ cache miss.

K14-SchemaVersion:
  schedule_cost.v1 schema changes ⇒ K14 changes ⇒ cache miss.

K14-FeatureSet:
  crate feature set affecting layout/serde/behavior changes ⇒
  K14 changes ⇒ cache miss.

K14-ReadValidate:
  On cache hit, the cached report's report_self_hash must equal
  R-Hash of the cached body. If not, the entry is poisoned and
  recomputed.

K14-PolicyProjection:
  schedule_cost_policy_projection_hash changes ⇒ K14 changes.
  ResolvedCompilePolicy fields outside the projection do NOT
  affect K14.

K14-CalibrationDrift:
  calibration_bundle_set_hash changes ⇒ K14 changes.
  This includes a bundle replacement, removal, or content edit.
```

### 11.3 K14 ↔ K10 cross-validation

```text
F-K14-K10-Pinning:
  K14.schedule_pack_self_hash must equal the SchedulePack product
  hash produced under K10 with the same upstream inputs.

F-K14-K10-NoStaleness:
  If K10 misses (any upstream drift), K14 must also miss because
  schedule_pack_self_hash will differ.
```

This is the cascade pattern from F-B11/F-B12 §13.4 applied to the
Stage 10 → Stage 11 dependency.

### 11.4 F-B17 has no key

F-B17 is a cross-stage validator that *enumerates* and *uniformizes*
every other stage's K-key. F-B17 has no K of its own.

`cache_status.json` is the cross-stage tally; it is emitted once per
build and is not cached. Two byte-identical builds produce
byte-identical `cache_status.json`, but the file is read from disk by
consumers (dashboards, CI), not through F-A6's `StageCache`.

Rationale: `cache_status.json` would be circular if cached. Its body
is the union of per-stage cache status; if it were itself cached,
the cache would have a "is the cache hit?" bit in its key, leading
to a fixed-point that does not improve iteration speed and obscures
the meaning of the report.

This is the load-bearing **F-B17 is not a stage** rule pinned formally.

### 11.5 K-key index (cross-stage summary)

For convenience, this subsection restates the K-key for every stage
in the pipeline. Each entry cites the owning RFC for the canonical
definition.

| Stage | K     | Owning RFC      | Key body schema_id           |
| ----- | ----- | --------------- | ---------------------------- |
| 0     | K0    | F-B2/F-B4 §7.8  | `artifact_validation.v1`     |
| 0.5   | K0.5  | F-B2/F-B4 §7.8  | `policy_resolution.v1`       |
| 1     | K1    | F-B3/F-B5 §11   | `quant_graph.v1`             |
| 2     | K2    | F-B2/F-B4 §7.8  | `static_budget.v1`           |
| 3     | K3    | F-B3/F-B5 §11   | `infer_ir.v1`                |
| 4     | K4    | F-B6 (forthcoming) | `observation_plan.v1`     |
| 5     | K5    | F-B7 (forthcoming) | `range_plan.v1`           |
| 6     | K6    | F-B8 (forthcoming) | `storage_plan.v1`         |
| 7     | K7    | F-B9 (forthcoming) | `sram_page_plan.v1`       |
| 8     | K8    | F-B10 (forthcoming) | `rom_window_plan.v1`     |
| 8.5   | K11   | F-B11/F-B12 §13.1 | `overlay_plan.v1`         |
| 9     | K12   | F-B11/F-B12 §13.2 | `arena_plan.v1`           |
| 10    | K10   | F-B13 (forthcoming) | `sched_ir.v1`            |
| 10.5  | K10.5 | F-B13 (forthcoming) | `resource_state.v1`      |
| 11    | K14   | This RFC §11.1   | `schedule_cost.v1`          |
| 12    | K12B  | F-B15 (forthcoming) | `backend.v1`             |

Note the key naming: K11 (Stage 8.5) and K12 (Stage 9) are inherited
from F-B11/F-B12; K14 (Stage 11) is introduced here; K12B disambiguates
Stage 12 from K12.

Future stages or stage variants must extend this table by amending
§11.5.

## 12. Diagnostic algebra

Inherits the F-B2/F-B4 §7.1 closed-enum surface with these additions.

### 12.1 New `ValidationOrigin` variants

```text
ValidationOrigin (extension):
  ScheduleCostAnalysis              -- F-B14
  StageCacheValidation              -- F-B17
```

Both extend the closed enum without modifying existing variants.

### 12.2 F-B14 diagnostic codes (origin = ScheduleCostAnalysis)

```text
COST-* prefix:

CostScheduleCostInputHashMismatch
  -- the schedule_pack_self_hash declared in inputs does not match
     the product's recorded self_hash.

CostCalibrationBundleHashMismatch
  -- a CalibrationBundleRef's recorded bundle_hash does not match
     the resolved blob's actual hash.

CostCalibrationBundleStale
  -- a calibration bundle's freshness gate failed (target_profile_hash
     / kernel_set_hash / packer_version / calibration_schema_hash
     mismatch). The build's per-affected-mode estimate falls back to
     Heuristic; this diagnostic records the staleness as an
     informational warning. (Severity = Hard if RiskPolicy strict.)

CostCalibrationMissingForRequirement
  -- the active RiskPolicy::calibration_confidence_requirement is
     {Measured, Transferred} but no bundle of sufficient confidence
     resolved.

CostKernelSpecNotInRegistry
  -- a SchedOp invokes a KernelSpecId not present in the resolved
     KernelSpecRegistry. (This is a structural error in F-B13's
     output, but F-B14 catches it as a diagnostic.)

CostPerModeMissing
  -- ResolvedCompilePolicy.requested_runtime_modes contains a mode
     not present in SchedulePack.modes.

CostPerModeUnexpected
  -- SchedulePack.modes contains a mode not in
     ResolvedCompilePolicy.requested_runtime_modes.

CostEvidenceClassRefsInconsistent
  -- a CostEstimate's evidence_class is Calibrated/Transferred but
     refs is empty or contains no CalibrationBundle ref.

CostFallbackReasonMissing
  -- a CostEstimate's evidence_class is Heuristic/Fallback but
     fallback_reason is None.

CostFallbackReasonPresentForCalibrated
  -- a CostEstimate's evidence_class is Calibrated/Transferred but
     fallback_reason is Some(_).

CostUncertaintyEnvelopeMalformed
  -- p95_lower > p50, p50 > p95_upper, or p99_upper < p95_upper.

CostUncertaintyEnvelopeNegative
  -- a p50/p95/p99 field is negative.

CostObjectiveSatisfactionMatrixIncomplete
  -- the satisfaction matrix does not cover every (mode, axis,
     quantile) requested by the objective.

CostObjectiveSatisfactionMatrixInconsistent
  -- a matrix entry's value does not match what would be computed
     by re-projecting the objective against per_mode.

CostHeuristicPolicyUnknown
  -- an EvidenceRef::HeuristicPolicy references a policy_id not in
     the registered HeuristicPolicyId enum.

CostTransferPolicyUnknown
  -- an EvidenceRef::TransferPolicy references a transfer_policy_id
     not resolvable through gbf-store.

CostFloatingPointFieldDetected
  -- a floating-point JSON value was found in a v1 report. This
     should never happen on emit (we use only Q16.16); it is a
     defensive check on parse.

CostScheduleCostSchemaUnknown
  -- the report declares a schema_id or schema_version this
     compiler does not understand.

CostOptionFieldMissing
  -- an Option-typed cost field is None when the build's structural
     shape requires it (e.g., sram_page_switches_per_token is None
     but the SramPagePlan has ≥ 1 page).

CostOptionFieldPresentUnexpectedly
  -- an Option-typed cost field is Some when the build's structural
     shape requires None (e.g., frame_jitter is Some on a
     non-LowFrameJitter objective).

CostRefsUnionInconsistent
  -- the top-level refs field does not equal the union of every
     CostEstimate.refs in per_mode (deduplicated by structural
     equality).

CostScheduleCostReportRoundTripFailed
  -- canonical_json::serialize then canonical_json::deserialize
     does not produce a structurally-equal value.

CostFinalNonNegativityViolation
  -- a final cost field has a derived value that is negative
     (e.g., scheduler_headroom_utilization < 0).
```

Severity:

```text
∀ d ∈ Stage11.diagnostics. d.severity = Hard
```

### 12.3 F-B17 diagnostic codes (origin = StageCacheValidation)

```text
CACHE-* prefix:

CacheKeyDeserializeFailed
  -- a stored StageCacheKey could not be deserialized.

CacheReadValidateFailed
  -- a cached entry's report_self_hash does not match R-Hash of
     the cached body. The entry is poisoned and recomputed; this
     diagnostic records the poisoning event.

CacheTypedInputBundleSchemaUnknown
  -- a stage's TypedInputBundle references a schema_id this
     compiler does not recognize.

CacheStageRunnerMissing
  -- a stage was invoked but no StageRunner<S> impl is registered
     for stage S. (Caught at compile time normally; this diagnostic
     is for dynamic test scaffolds.)

CacheStatusReportRoundTripFailed
  -- cache_status.json fails round-trip through serialize / parse.

CacheStatusReportTotalsInconsistent
  -- build_summary counts do not equal per_stage counts.

CacheStatusReportStageSetIncomplete
  -- per_stage.keys() does not contain all 16 stages.

CacheStatusReportStageSetUnexpected
  -- per_stage.keys() contains a stage not in the canonical 16-stage
     set.

CacheStatusReportInputIdentityHashMismatch
  -- a cached stage's recorded input_identity_hash does not match
     H(TypedInputBundle(S)) for the current build's inputs. (This
     is typically caught by cache miss before the report is emitted;
     the diagnostic exists for the corner case where the cached
     entry is replayed but its identity drifts.)

CacheStatusReportProductHashAbsentForCachedStage
  -- per_stage[S].product_self_hash is None when status is Hit/Miss/Stale
     and the stage produced a successful product.

CacheStatusReportProductHashPresentForNotApplicable
  -- per_stage[S].product_self_hash is Some when status is
     NotApplicable.

CacheKeyDerivationNonTotal
  -- the per-stage typed-input-bundle conformance test detected a
     stage whose K-key derivation depends on something not in the
     typed input bundle (e.g., the stage reads an environment
     variable, a wall-clock value, or a side channel). Caught by
     property test in the workspace pre-commit hook.

CacheKeyDerivationNonInjective
  -- the per-stage typed-input-bundle conformance test detected
     two distinct typed input bundles producing the same K-key.
     This is a bug in compose_key or in the per-stage canonical
     serialization.

CacheStageBundleFieldNotInBody
  -- a TypedInputBundle field is not represented in the StageCacheKeyBody.
     (Caught at compile time by the typed mapping; this diagnostic is
     for test fixtures.)

CacheCrossStageDependencyDriftFailed
  -- the cross-stage dependency test (changing input to S1 ⇒ K(S2)
     also changes) detected a stage S2 that did not invalidate when
     its upstream changed.

CacheCrossStageDependencyOverInvalidated
  -- the negative cross-stage test (changing a non-load-bearing
     input to S1 ⇒ K(S2) does NOT change) detected a stage S2 that
     invalidated unnecessarily. This points to S2's TypedInputBundle
     having a phantom dependency.
```

Severity:

```text
∀ d ∈ F-B17.diagnostics. d.severity = Hard
```

### 12.4 Diagnostic laws (inherited)

Inherited from F-B2/F-B4 §7.1:

```text
D-CodeClosed:
  Every diagnostic carries a typed enum code; no free-form ad-hoc codes.

D-NoStringOnly:
  Diagnostic detail is a typed record. String fields are allowed only
  for human-readable explanations whose machine-readable companion
  fields exist alongside.

D-Renderable:
  Every diagnostic detail must serialize to canonical JSON and be
  renderable in CLI output.

D-Provenance:
  Every diagnostic carries provenance back to a hash-bound input.
```

Additions:

```text
D-CostScheduleCostOriginExclusive:
  COST-* diagnostics use origin = ScheduleCostAnalysis; never
  StageCacheValidation or any other origin.

D-CacheStageCacheOriginExclusive:
  CACHE-* diagnostics use origin = StageCacheValidation; never
  ScheduleCostAnalysis or any other origin.

D-NoSoftDiagnostics:
  Every F-B14 / F-B17 diagnostic is severity = Hard. There is no
  Soft severity in this chunk.
```

## 13. Cross-stage interactions

### 13.1 F-B13 input (SchedulePack)

F-B14 consumes `SchedulePack` from F-B13 by hash:

```rust
impl ScheduleCostInputs {
    pub schedule_pack_self_hash: Hash256,
    pub schedule_pack_ref: BlobRef,         // resolves to SchedulePack
    ...
}
```

F-B14 dereferences `schedule_pack_ref` through `gbf-store::BlobStore`,
deserializes into `SchedulePack`, and verifies that the deserialized
struct's recomputed self-hash equals the recorded
`schedule_pack_self_hash`. A mismatch is `CostScheduleCostInputHashMismatch`
(Hard).

The `SchedulePack` is the authoritative source for:
* `modes`: per-mode `GbSchedIR` slice schedules
* `epochs`: per-mode `ResidencyEpoch` lists
* `checkpoint_schema_hash`
* `switch_policy`: legal switch points and triggers (consumed
  read-only; F-B14 does not evaluate triggers)

F-B14 does NOT cache the `SchedulePack` body inside `schedule_cost.json`;
only its hash. The deserialized struct is held in memory only for
the duration of the F-B14 run.

### 13.2 F-B16 downstream (RepairProposal cost-comparison)

F-B16's `RepairProposal::estimated_cost` is typed as
`EstimatedCostDelta` (planv0 line 1131). F-B16 does not run F-B14
directly; instead, F-B16's loop driver:

1. Applies a candidate `ConstraintDelta` to the typed input space.
2. Re-runs the affected upstream stages (F-B7..F-B13) to produce a
   candidate `SchedulePack`.
3. Re-runs F-B14 on the candidate `SchedulePack`.
4. Compares `candidate_schedule_cost.per_mode[m]` against
   `current_schedule_cost.per_mode[m]` for every requested mode.
5. Decides accept / reject based on the comparison and the active
   `RepairPolicy`.

The comparison is typed (no string parsing of cost reports). Each
`EstimatedCostDelta` carries `EvidenceClass` per axis, so F-B16 can
apply policy like "reject deltas whose primary objective improvement
is `Heuristic` and current is `Calibrated`."

F-B16's policy logic itself is out of scope for this RFC. F-B14's
contract surface is `EstimatedCostDelta`; F-B16 owns the comparison.

### 13.3 F-B15 downstream (budget.json, map.json, compiler_feedback.json)

F-B15 (Backend, Stage 12) emits the build's primary report package.
Several reports incorporate per-mode envelope data from F-B14:

* `budget.json` (per planv0 lines 2832–2865) includes
  "estimated cycles per slice", "estimated cycles per token",
  "evidence class and uncertainty envelope for every load-bearing
  estimate", "fallback reason when calibration confidence is
  insufficient", "predicted bank switches per token", "predicted SRAM
  page switches per token", "predicted yields per token", "scheduler
  headroom utilization", "video-commit cost distribution vs
  video_commit_margin", and "compile-objective satisfaction at the
  requested quantiles". All of these come from `schedule_cost.json`.
* `map.json` includes per-bank occupancy and cost-derived expert
  hotness summaries; F-B15 reads these from F-B14's per-mode envelopes
  cross-referenced with `arena_plan.json`.
* `compiler_feedback.json` includes "reasons a decision was accepted
  under transferred or heuristic evidence" — directly from F-B14's
  `EvidenceClass` and `FallbackReason` fields.

F-B15 reads `schedule_cost.json` by hash. No re-derivation of cost
in F-B15.

### 13.4 ResolvedCompilePolicy → CompileObjective

`CompileObjective` lives in `ResolvedCompilePolicy` (per F-B2/F-B4
§7.4). F-B14 reads it through the typed projection
`ScheduleCostPolicyProjection` (§11.1) which is also recorded in
`identity` for cache-key purposes.

The projection is closed: any `ResolvedCompilePolicy` field not in
the projection cannot affect F-B14's outputs (otherwise the cache
key would over-/under-invalidate). If F-B14's logic ever needs a new
policy field, the projection schema must amend §11.1 explicitly.

### 13.5 Calibration bundles → CycleModelRef

`CalibrationBundleSet` is owned by Epic E (`gbf-bench`). F-B14
consumes:

* `PlatformCalibrationBundle`: per-target cycle counts for primitive
  ops (e.g., LR35902 instruction-level cycle costs, bank-switch
  amortization).
* `KernelCalibrationBundle`: per-`KernelSpecId` measured records.
  F-B14's `CycleModelRef` projection picks the matching record.
* `RuntimeCalibrationBundle`: scheduler/UI/persistence cycle costs
  (e.g., yield overhead, video-commit cost). Consumed for fields
  like `scheduler_headroom_utilization`.

F-B14 does NOT consume these bundle types directly; it consumes the
typed projections recorded in `EvidenceRef::CalibrationBundle` /
`EvidenceRef::CycleModel`. The bundle structs are decoded once,
projections are taken, and projection hashes are recorded.

### 13.6 gbf-store → cache primitives

F-B17's per-stage call-site wrapper calls into `gbf-store`:

```text
gbf-store::stage_cache::StageCache::get(StageCacheKey) -> Option<Entry>
gbf-store::stage_cache::StageCache::put_success(...)
gbf-store::stage_cache::StageCache::put_failure_memo(...)
gbf-store::stage_cache::StageCache::poison(StageCacheKey) -> ()
gbf-store::blob::BlobStore::put(&[u8]) -> Hash256
gbf-store::blob::BlobStore::get(Hash256) -> Vec<u8>
gbf-store::stage_cache::compose_key(StageKey) -> StageCacheKey
```

F-B17 does NOT introduce new `gbf-store` API; it consumes the
existing F-A6 surface. If a new primitive is needed (e.g., bulk
status query), it lands in F-A6 via amendment, not in F-B17.

### 13.7 F-C3 ScheduleOracle (not a closure dependency)

`ScheduleOracle` (F-C3, downstream) measures actual cycles on a
schedule. F-B14 *predicts* cycles. The two diff in nightly trust
tests:

```text
Nightly trust:
  for each RuntimeMode m in CompiledBuild.SchedulePack:
    measured := ScheduleOracle.run(m, workload)
    predicted := schedule_cost.json.per_mode[m]
    drift := measured - predicted.envelope.p50
    assert drift ≤ predicted.envelope.p95_upper - p50
       OR  drift ≥ predicted.envelope.p95_lower - p50
    on violation: emit DriftReport
```

The F-C3 / F-B14 seam is a *measurement-vs-prediction* boundary, not
a *correctness* boundary. F-B14's output is fully consumable by F-B16
and F-B15 without any F-C3 measurement. Closure of this chunk does
NOT depend on F-C3.

### 13.8 RuntimeDriftMonitor (Epic D, not a closure dependency)

`RuntimeDriftMonitor` (planv0 lines 1855–1879) consumes F-B14's
`UncertaintyEnvelope.p95_upper_q16_16` as the drift threshold:

```text
DriftTrigger {
  metric: SliceCyclesP95,
  threshold: schedule_cost.json.per_mode[m].cycles_per_token.envelope.p95_upper_q16_16,
  action: ShrinkSlices,
}
```

The runtime configures these triggers from `schedule_cost.json`
fields; F-B14 does not configure them. F-B14's contract is to record
the predicted envelope; the runtime's contract is to observe and
trigger.

This is a clean seam: F-B14 hands off a typed envelope; the monitor
consumes it. Both sides are independent and testable.

### 13.9 Out-of-tree consumers

* **Dashboards** (Epic F or external): consume `schedule_cost.json`
  for visualizations. F-B14 emits canonical-JSON; no special encoding
  for dashboards.
* **Autotune** (`gbf-bench`, Epic E): consumes per-mode envelopes
  to choose tile/residency/slice knobs. F-B14 produces; autotune
  reads.
* **Failure capsules** (`FailureCapsule`, planv0 line 859): include
  `schedule_cost.json` as a sidecar when the capsule's
  `failing_stratum` is `Operational`. F-B14 does not produce capsules;
  capsule production is upstream.

## 14. Task DAG, compressed

```text
Wave 0 (schema prelude + cost types):
  T-B14.0   gbf-policy::cost module: EvidenceClass, UncertaintyEnvelope,
            EvidenceRef, CalibrationBundleRef, CycleModelRef, FallbackReason,
            StaleField, ShapeMismatchDetail, CostEstimate, EstimatedCostDelta,
            ScheduleCostReport, ScheduleCostIdentity,
            ScheduleCostObjectiveSatisfactionMatrix, SatisfactionKey,
            ObjectiveSatisfaction, Quantile, HeuristicPolicyId,
            TransferPolicyId. All with serde + deny_unknown_fields +
            round-trip tests.

  T-B17.0   gbf-codegen::stage_cache::status module: CacheStatus,
            StageCacheStatusEntry, CacheStatusReportBody,
            CacheStatusBuildSummary. StageRunner<S> trait.
            Per-stage TypedInputBundle<S> definitions for stages
            already wired (Stage 0, 0.5, 1, 2, 3, 8.5, 9). Placeholder
            impls for stages not yet wired (Stage 4, 5, 6, 7, 8, 10,
            10.5, 11, 12). cache_status.v1 schema in gbf-report.

Wave 1 (Stage 11 implementation):
  T-B14.1   gbf-codegen::stages::schedule_cost: build_schedule_cost_core
            pure-core function. Construction order per §8.2.
  T-B14.2   Per-slice cost rollup with EvidenceClass composition (§8.4.2).
  T-B14.3   Per-token rollup, including computing tokens-per-frame from
            SchedulePack (§8.5).
  T-B14.4   ObjectiveSatisfaction matrix derivation (§8.4.4).
  T-B14.5   FallbackReason chain construction.
  T-B14.6   schedule_cost.v1 schema in gbf-report::schedule_cost +
            semantic validator (§8.3).
  T-B14.7   Calibration bundle dereferencing (§8.5.1) +
            KernelSpec resolution (§8.5.2) + freshness gate (§8.5.6).
  T-B14.8   Heuristic policy registry (§8.5.4): one HeuristicPolicyId
            per metric, each with policy_version: SemVer.
  T-B14.9   Transfer policy resolution (§8.5.5): identity transfer
            policy registered.
  T-B14.10  K14 cache-key wiring (§11.1) + K14 cache laws (§11.2).
            Use the StageRunner<Stage11> impl from T-B17.0.
  T-B14.11  Synthetic fixtures: passing build, calibration-missing-failing
            build, partial-bundle-degraded build, multi-mode build.
  T-B14.12  Worked-example golden artifacts under
            docs/review/f-b14-f-b17/ (regenerator + checked-in JSON).

Wave 2 (F-B17 per-stage wiring):
  T-B17.1   Wire Stage 0 (StageRunner<Stage0> impl in F-B2's modules).
  T-B17.2   Wire Stage 0.5.
  T-B17.3   Wire Stage 1 (re-uniformize F-B3's existing wiring).
  T-B17.4   Wire Stage 2.
  T-B17.5   Wire Stage 3 (re-uniformize F-B5's existing wiring).
  T-B17.6   Wire Stage 4.    (placeholder until F-B6 lands; real impl
                              ships in F-B6's bead).
  T-B17.7   Wire Stage 5.    (placeholder until F-B7 lands).
  T-B17.8   Wire Stage 6.    (placeholder until F-B8 lands).
  T-B17.9   Wire Stage 7.    (placeholder until F-B9 lands).
  T-B17.10  Wire Stage 8.    (placeholder until F-B10 lands).
  T-B17.11  Wire Stage 8.5 (re-uniformize F-B11's existing wiring).
  T-B17.12  Wire Stage 9 (re-uniformize F-B12's existing wiring).
  T-B17.13  Wire Stage 10.   (placeholder until F-B13 lands).
  T-B17.14  Wire Stage 10.5. (placeholder until F-B13 lands).
  T-B17.15  Wire Stage 11 (calls into F-B14, atomic with T-B14.10).
  T-B17.16  Wire Stage 12.   (placeholder until F-B15 lands).

Wave 3 (cross-stage tests + reports):
  T-B17.17  Per-stage typed-input-bundle conformance test (§9.6.2):
            forward + backward + negative tests for every stage.
  T-B17.18  Cross-stage dependency drift test (§9.6.6) for every pair
            (S1, S2) where S2 directly consumes S1's product.
  T-B17.19  cache_status.json emitter in gbf-codegen::stage_cache::status.
            Round-trip test + semantic validator test.
  T-B17.20  Cache-poisoning recovery test (§9.6.4) at every stage.
  T-B17.21  Failure-memo replay test (§9.6.5) at every stage.

Wave 4 (closure):
  T-B14.13  schedule_cost.json emitter integration test:
            two-build determinism (byte-identical output across
            two consecutive regenerations).
  T-B14.14  schedule_cost.json semantic-validator regression test.
  T-B17.22  cache_status.json emitter integration test:
            two-build determinism.
  T-B17.23  Reviewer review packet under
            docs/review/f-b14-f-b17/ with the four worked examples
            (passing build, calibration-missing failure, partial
            bundle, multi-mode).

Bead inventory:
  F-B14 = bd-prw, parent feature.
    Children: T-B14.0 .. T-B14.14 (14 tasks).
  F-B17 = bd-1g7k, parent feature.
    Children: T-B17.0 .. T-B17.23 (23 tasks).

  Total: 38 tasks under the two parent features.
```

The two features can land in parallel up to T-B17.15 (Stage 11
wiring), which is atomic with T-B14.10 (K14 cache wiring). T-B17.x for
stages whose owning RFCs have not landed remain placeholders;
their real impls ship in those owning RFCs' beads.

## 15. Rejection classes (closure gate)

This section enumerates every closure-gating reject class. The chunk
closes only when every class has at least one passing test.

### 15.1 F-B14 reject classes

| Class | Code | Test fixture |
| ----- | ---- | ------------ |
| RC-COST-1 | `CostScheduleCostInputHashMismatch` | flip schedule_pack_self_hash; assert hard fail |
| RC-COST-2 | `CostCalibrationBundleHashMismatch` | corrupt calibration blob; assert hard fail on resolution |
| RC-COST-3 | `CostCalibrationBundleStale` | stale target_profile_hash on bundle; assert affected estimate falls back to Heuristic with BundleStale reason |
| RC-COST-4 | `CostCalibrationMissingForRequirement` | Default profile + no bundle; assert hard fail |
| RC-COST-5 | `CostKernelSpecNotInRegistry` | SchedOp invokes unregistered KernelSpec; assert hard fail |
| RC-COST-6 | `CostPerModeMissing` | requested_runtime_modes contains a mode SchedulePack does not; assert hard fail |
| RC-COST-7 | `CostPerModeUnexpected` | SchedulePack contains a mode requested_runtime_modes does not; assert hard fail |
| RC-COST-8 | `CostEvidenceClassRefsInconsistent` | manually-construct CostEstimate with evidence_class=Calibrated, refs=[]; assert validator fails |
| RC-COST-9 | `CostFallbackReasonMissing` | manually-construct CostEstimate with evidence_class=Heuristic, fallback_reason=None; assert validator fails |
| RC-COST-10 | `CostFallbackReasonPresentForCalibrated` | inverse of RC-COST-9 |
| RC-COST-11 | `CostUncertaintyEnvelopeMalformed` | p95_lower > p50; assert validator fails |
| RC-COST-12 | `CostUncertaintyEnvelopeNegative` | p50 < 0; assert validator fails |
| RC-COST-13 | `CostObjectiveSatisfactionMatrixIncomplete` | omit a (mode, axis, quantile); assert validator fails |
| RC-COST-14 | `CostObjectiveSatisfactionMatrixInconsistent` | hand-edit matrix entry to disagree with derived value; assert validator fails |
| RC-COST-15 | `CostHeuristicPolicyUnknown` | reference unregistered HeuristicPolicyId; assert validator fails |
| RC-COST-16 | `CostTransferPolicyUnknown` | reference unresolvable TransferPolicyId; assert validator fails |
| RC-COST-17 | `CostFloatingPointFieldDetected` | hand-edit JSON to insert a float; assert parse fails |
| RC-COST-18 | `CostScheduleCostSchemaUnknown` | unknown schema_id; assert parse fails |
| RC-COST-19 | `CostOptionFieldMissing` | Option-typed field None when build requires it; assert validator fails |
| RC-COST-20 | `CostOptionFieldPresentUnexpectedly` | Option-typed field Some when build forbids it; assert validator fails |
| RC-COST-21 | `CostRefsUnionInconsistent` | top-level refs missing a per-estimate ref; assert validator fails |
| RC-COST-22 | `CostScheduleCostReportRoundTripFailed` | inject malformed JSON; assert round-trip fails |
| RC-COST-23 | `CostFinalNonNegativityViolation` | derive a negative scheduler_headroom; assert hard fail |
| RC-COST-24 | (positive) Determinism | same inputs ⇒ byte-identical schedule_cost.json |
| RC-COST-25 | (positive) Calibrated path | full calibration bundle; assert evidence_class=Calibrated, no fallback_reason |
| RC-COST-26 | (positive) Heuristic path | no bundle; Bringup profile; assert evidence_class=Heuristic, fallback_reason=NoBundleForTarget |
| RC-COST-27 | (positive) Multi-mode | requested_runtime_modes={Default, Trace}; assert per_mode has both keys |

### 15.2 F-B17 reject classes

| Class | Code | Test fixture |
| ----- | ---- | ------------ |
| RC-CACHE-1 | `CacheKeyDeserializeFailed` | corrupt stored StageCacheKey blob; assert get returns deserialize error |
| RC-CACHE-2 | `CacheReadValidateFailed` | inject corrupt body bytes for valid key; assert poison + recompute |
| RC-CACHE-3 | `CacheTypedInputBundleSchemaUnknown` | unknown TypedInputBundle schema; assert hard fail |
| RC-CACHE-4 | `CacheStageRunnerMissing` | invoke run_stage_with_cache for unregistered stage; assert hard fail |
| RC-CACHE-5 | `CacheStatusReportRoundTripFailed` | corrupt JSON; assert round-trip fails |
| RC-CACHE-6 | `CacheStatusReportTotalsInconsistent` | hand-edit build_summary counts; assert validator fails |
| RC-CACHE-7 | `CacheStatusReportStageSetIncomplete` | omit a stage; assert validator fails |
| RC-CACHE-8 | `CacheStatusReportStageSetUnexpected` | add a synthetic stage; assert validator fails |
| RC-CACHE-9 | `CacheStatusReportInputIdentityHashMismatch` | swap identity_hash on a Hit entry; assert validator fails |
| RC-CACHE-10 | `CacheStatusReportProductHashAbsentForCachedStage` | Hit with product_self_hash=None; assert validator fails |
| RC-CACHE-11 | `CacheStatusReportProductHashPresentForNotApplicable` | NotApplicable with product_self_hash=Some; assert validator fails |
| RC-CACHE-12 | `CacheKeyDerivationNonTotal` | introduce a global-reading code path in a stage; assert per-stage conformance test fails |
| RC-CACHE-13 | `CacheKeyDerivationNonInjective` | (defensive) inject a TypedInputBundle equality flaw; assert per-stage conformance test fails |
| RC-CACHE-14 | `CacheStageBundleFieldNotInBody` | introduce a field in TypedInputBundle not represented in StageCacheKeyBody; assert build fails |
| RC-CACHE-15 | `CacheCrossStageDependencyDriftFailed` | flip Stage 1 input; assert Stage 3 misses cache (cascade) |
| RC-CACHE-16 | `CacheCrossStageDependencyOverInvalidated` | flip a non-load-bearing aspect of Stage 1's environment (e.g., env var); assert Stage 3 still hits cache |
| RC-CACHE-17 | (positive) Cold-cache build | every stage reports Miss in cache_status.json |
| RC-CACHE-18 | (positive) Warm-cache rebuild | every stage reports Hit |
| RC-CACHE-19 | (positive) Mid-pipeline change | flip Stage 5 input; Stage 0..4 hit, Stage 5..12 miss |
| RC-CACHE-20 | (positive) Failure-memo replay | flip an input that produces a Hard failure; second run replays the memo |
| RC-CACHE-21 | (positive) Cache poison recovery | corrupt a cache entry; on access, stage detects + recomputes |
| RC-CACHE-22 | (positive) Determinism | two builds with identical inputs ⇒ byte-identical cache_status.json |

The closure gate: every reject class has at least one fixture that
passes; every positive class has at least one fixture that passes.

## 16. Proof obligations

This section pins the formal proof obligations for closure.

### 16.1 F-B14 proof obligations

**Theorem F-B14-Determinism.** For all inputs `I = (SchedulePack,
ResolvedCompilePolicy, CalibrationBundleSet, RuntimeChromeBudget,
TargetProfile, KernelSpecRegistry)`, two consecutive invocations of
`build_schedule_cost_core(I)` produce structurally-equal
`ScheduleCostReport` and byte-identical `schedule_cost.json`.

*Sketch.* The pure core has no external dependencies (no env vars, no
wall-clock, no RNG). All map iteration is over `BTreeMap` (sorted).
All hash computations are deterministic. ∎

**Theorem F-B14-EvidenceClassConsistency.** For all `CostEstimate e`
in any `ScheduleCostReport`:
  (i) `e.evidence_class ∈ {Calibrated, Transferred}` ⇒
      `e.fallback_reason.is_none()` and `e.refs` contains a
      `CalibrationBundle` ref.
  (ii) `e.evidence_class ∈ {Heuristic, Fallback}` ⇒
      `e.fallback_reason.is_some()`.

*Sketch.* Enforced by the semantic validator (rule
SC-EvidenceClassRefsConsistent in §8.3). The validator runs at every
deserialization. ∎

**Theorem F-B14-PerModeTotality.** For every `ScheduleCostReport r`:
`r.per_mode.keys() == r.identity-resolved
ResolvedCompilePolicy.requested_runtime_modes`.

*Sketch.* Enforced by SC-PerModeTotal. The validator rejects reports
violating this property. ∎

**Theorem F-B14-NoFabrication.** F-B14's pure core never produces a
`CostEstimate` with `evidence_class = Calibrated` unless a
`CalibrationBundleRef` was successfully dereferenced and a matching
record was found whose `CalibrationConfidenceClass` is
`Measured`.

*Sketch.* Inspect `lookup_record` in §8.4.1: the only return path
that produces `Calibrated` is the `if CB.kernel.records contains an
exact match` branch, which requires a real CalibrationBundleRef. No
synthetic-evidence path exists. The pure core panics on attempts to
construct `Calibrated` without a `CalibrationBundleRef`-typed
`EvidenceRef`. ∎

**Theorem F-B14-K14-Soundness.** For two invocations of
`build_schedule_cost_core(I)` and `build_schedule_cost_core(I')`:
K14(I) = K14(I') ⟺ canonical-equal(I, I')` over the projected
fields in `TypedInputBundle(11)`.

*Sketch.* Inherited from F-A6 `compose_key` determinism. ∎

### 16.2 F-B17 proof obligations

**Theorem F-B17-CanonicalInputForward.** For every stage S and every
two `TypedInputBundle(S)` values t1, t2:
  t1 = t2 ⇒ K(S, t1) = K(S, t2).

*Sketch.* Trivial by F-A6 `compose_key` determinism. ∎

**Theorem F-B17-CanonicalInputBackward.** For every stage S and every
two `TypedInputBundle(S)` values t1, t2:
  t1 ≠ t2 ⇒ K(S, t1) ≠ K(S, t2).

*Sketch.* Per-field test (§9.6.2) enumerates every field f of
`TypedInputBundle(S)` and asserts that flipping f changes K(S). Since
`compose_key` is structural over the canonical-JSON encoding of the
bundle, a flip in any field produces different bytes, which produces
a different DomainHash. ∎

**Theorem F-B17-CrossStageMonotone.** For any two stages S1, S2 where
S2's TypedInputBundle has a field whose value depends on S1's product
hash:
  K(S1, B) ≠ K(S1, B') ⇒ K(S2, B) ≠ K(S2, B') (when B and B' agree
  on every other input to S2).

*Sketch.* By construction: `S1` produces `Product(S1)` whose
`product_self_hash` is recorded. `TypedInputBundle(S2)` includes
`<S1>_self_hash`. Different K(S1) ⇒ different `Product(S1)` ⇒ different
`<S1>_self_hash` ⇒ different K(S2). ∎

**Theorem F-B17-CacheReadValidate.** For every stage S, on cache hit
the wrapper validates `cached.report_self_hash ==
R-Hash(cached.body)`. If the validation fails, the entry is poisoned
and the stage runs from scratch.

*Sketch.* Inspect `run_stage_with_cache` in §9.3: the body-hash check
is unconditional; failure routes to `run_stage_uncached_and_store`. ∎

**Theorem F-B17-CacheStatusTotality.** For every build B that reaches
closure, `cache_status.json.per_stage.keys() == {0, 0.5, 1, 2, 3, 4,
5, 6, 7, 8, 8.5, 9, 10, 10.5, 11, 12}`.

*Sketch.* Enforced by CS-AllStagesPresent in §10.2.4. The validator
runs at emit; failure to populate every stage produces a malformed
report rejected by the validator. ∎

**Theorem F-B17-NoFB17Key.** F-B17's `cache_status.json` is not
itself stored in `gbf-store::StageCache`. There is no K(F-B17).

*Sketch.* `cache_status.json` is a build-level aggregation written
directly to the build output directory; the wrapper that emits it
does not call `StageCache::put`. Inspection of the emitter code path
confirms this. ∎

### 16.3 Joint obligations (F-B14 + F-B17)

**Theorem ChunkClosure-Determinism.** Two builds with identical
inputs produce byte-identical `schedule_cost.json` and byte-identical
`cache_status.json`.

*Sketch.* `schedule_cost.json` determinism follows from
F-B14-Determinism. `cache_status.json` determinism follows from the
fact that every per-stage K-key is deterministic and every per-stage
status is a deterministic function of the cache state plus inputs.
Since the cache state is content-addressed by inputs, the status is
determined by inputs alone. ∎

**Theorem ChunkClosure-NoSilentInputs.** No closure-relevant input to
any stage reaches K(S) outside of `TypedInputBundle(S)`. Equivalently:
flipping a non-`TypedInputBundle` aspect of the build environment
does NOT change K(S) for any stage S.

*Sketch.* Per-stage conformance test (§9.6.3) enumerates non-typed
inputs (env vars, fixture filenames, wall-clock) and asserts K(S) is
unchanged. ∎

**Theorem ChunkClosure-EvidenceClassPreservation.** Throughout the
build pipeline (after F-B14 emits), every `EstimatedCostDelta` value
referenced by a downstream consumer carries the same
`(EvidenceClass, UncertaintyEnvelope, EvidenceRef[],
fallback_reason)` tuple as F-B14 emitted.

*Sketch.* Downstream consumers (F-B16, F-B15, dashboards) read
`schedule_cost.json` by hash. Since F-B14's identity is recorded in
the report's self_hash, any consumer that resolves through the same
hash sees the same bytes. There is no in-process mutation path. ∎

## 17. End-to-end theorem

**Theorem ChunkEnd-to-End (informal).** When this chunk passes:

1. **Every stage's outputs are content-addressed by their typed
   inputs.** Pipeline state machine §6.2: every stage S has a
   `TypedInputBundle(S)`, a `StageCacheKey K(S)` derived from it, and
   typed `Product(S)` + `Report(S)` outputs whose self-hashes are
   recorded.

2. **Every objective has an evidence-classed cost envelope feeding
   F-B16.** F-B14 emits a `ScheduleCostReport` whose `per_mode` map
   covers every requested `RuntimeMode`; whose `satisfaction` matrix
   covers every requested `(mode, axis, quantile)` triple; whose
   estimates are typed by `EvidenceClass` with non-fabricated
   evidence chains.

3. **Iteration speed is mechanical.** Re-running the pipeline on
   identical inputs produces every-stage cache hits. Re-running with
   one input flipped produces a cache cascade exactly aligned with
   the dependency graph.

4. **Cache correctness is mechanical, not advisory.** Two
   conformance tests (forward + backward) run for every stage; one
   cross-stage drift test runs for every dependency edge. A stage
   that drifts from the canonical-input convention fails in the
   pre-commit hook.

5. **Cost estimates are honest.** Every cost figure is mechanically
   distinguishable from every other by its evidence chain. Heuristic
   fallbacks are typed; calibrated estimates require a real
   calibration bundle.

6. **The build output package is complete.** `schedule_cost.json`,
   `cache_status.json`, and the existing report files all
   round-trip and semantically validate. F-B16 / F-B15 / dashboards
   can read them as a contract surface.

This is the **post-spatial closing-act** invariant: when the chunk
closes, the transform pipeline is content-addressed end-to-end and
every objective has a typed cost prediction. The next chunks (F-B15
backend, F-B16 refinement loop) inherit a pipeline that is iterable,
diffable, and cost-honest.

## 18. Final concise contract

This RFC commits to:

* **F-B14** delivers `gbf-codegen::stages::schedule_cost`,
  `gbf-policy::cost`, and `gbf-report::schedule_cost`. Stage 11 reads
  `SchedulePack` (F-B13), `ResolvedCompilePolicy` (F-B2),
  `CalibrationBundleSet` (Epic E), `RuntimeChromeBudget`,
  `TargetProfile`, and `KernelSpecRegistry`; emits
  `schedule_cost.json` whose `per_mode: BTreeMap<RuntimeMode,
  EstimatedCostDelta>` carries typed `EvidenceClass` and
  `UncertaintyEnvelope` for every cost figure. K14 is pinned in §11.

* **F-B17** delivers `gbf-codegen::stage_cache::status` plus
  per-stage `StageRunner<S>` impls. Every stage in {0, 0.5, 1, 2, 3,
  4, 5, 6, 7, 8, 8.5, 9, 10, 10.5, 11, 12} has a typed input bundle;
  K(S) is a total function of typed inputs alone; per-stage and
  cross-stage tests validate the canonical-input convention; the
  cross-stage `cache_status.json` report is emitted per build.

* **No fabrication.** F-B14 NEVER fabricates a calibrated estimate.
  Heuristic fallbacks are typed.

* **No advisory inputs.** F-B17 cache keys are TOTAL functions of
  typed inputs. No global, no env var, no wall-clock affects K(S).

* **F-B16 unblocked (after oracle).** When F-B16's oracle question
  resolves, F-B14's per-mode envelope is the cost surface F-B16
  consumes for `RepairProposal::estimated_cost`.

* **Iteration loops cheap.** A typical training shadow-compile of
  unchanged inputs hits every stage's cache; only changed paths
  re-run. `cache_status.json` makes "why did this rebuild?"
  diagnosable.

* **Inheritance preserved.** F-B2/F-B4 ReportEnvelope, canonical
  JSON, self-hash, ValidationDiagnostic, StageCache key construction
  rules are inherited unchanged. F-A6 `BlobStore` /
  `StageCache` / `compose_key` primitives are consumed unchanged.
  F-B11/F-B12's exemplary StageCache algebra section is the pattern
  this RFC's §11 follows.

* **Closure is mechanical.** §15 enumerates every reject class;
  §16 pins formal proof obligations; §17 states the end-to-end
  theorem. The chunk closes when every reject class has a passing
  test and every proof obligation has a confirming property test.

The chunk is post-spatial closing-act and end-of-pipeline simultaneously:
F-B14 closes the cost story; F-B17 closes the cache-correctness story.
After this chunk, Epic B's transform pipeline is content-addressed,
evidence-classed, cost-honest, and iteration-cheap.

The next chunk (Chunk 9, F-B15 backend) inherits a pipeline whose
outputs are deterministic, whose intermediate stages are cached, and
whose cost predictions are typed with explicit evidence. The chunk
after that (Chunk 10, F-B16 refinement loop, blocked on oracle)
inherits the same plus a typed cost surface to drive its repair logic.

## 19. References

* `history/rfcs/F-B2-F-B4-pipeline-entry-validation.md` — TEMPLATE;
  §11 StageCache algebra and the canonical-input convention this RFC
  inherits and proves mechanical.
* `history/rfcs/F-B3-F-B5-canonical-irs.md` — second template; §11
  StageCache algebra (K1, K3) and §13.5 F-B17 integration handshake.
* `history/rfcs/F-A6-gbf-store-migrate.md` — `BlobStore`,
  `StageCache`, `compose_key` primitives F-B17 calls.
* `history/rfcs/F-B11-F-B12-overlay-arena-plans.md` — exemplary
  StageCache algebra section (§13) this RFC follows.
* `history/planv0.md` line 1894–1985 — Stage 11
  `ScheduleCostAnalysis` body.
* `history/planv0.md` line 1770–1900 — Stage 10 `GbSchedIR` +
  `SchedulePack`.
* `history/planv0.md` line 1985–2080 — `BuildReports` +
  `schedule_cost.json` + `budget.json` field set.
* `history/planv0.md` line 770–920 — workloads, calibration,
  deployability envelope.
* `history/planv0.md` line 1065–1095 — sizing realism, dense
  baseline, multi-timescale state.
* `history/planv0.md` line 2466–2640 — Assembly eDSL, profiles,
  CompileObjective shape.
* `history/planv0.md` line 2640–2870 — test classes, reports/artifacts,
  `schedule_cost.json` + `budget.json` + StageCache verification rules.
* `history/planv0.md` engineering rule 20 — always-on content-addressed
  StageCache, two-component canonical key.
* `history/glossary.md` — shared vocabulary.
* `bd-prw` — F-B14 bead.
* `bd-1g7k` — F-B17 bead.
* `bd-9ae` — F-B13 bead (input dependency).
* `bd-3ix` — F-B16 bead (downstream consumer, BLOCKED).
* `bd-3ll` — F-A6 bead (CLOSED, primitive provider).
