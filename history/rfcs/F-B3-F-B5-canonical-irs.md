# RFC F-B3 + F-B5: Canonical IRs — `QuantGraph` (Stage 1) and `GbInferIR` (Stage 3)

## -1. Authority and amendment policy

This RFC is the source of truth for F-B3 and F-B5 implementation. `history/planv0.md`
remains the architectural context document, but this RFC is allowed to refine,
narrow, or supersede `planv0.md` wherever this RFC makes a more precise
implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence must
be recorded in an `Amends planv0` note close to the relevant decision. This is
not a request to edit `planv0.md` immediately; it is a local source-of-truth
ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B3/F-B5 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a shared
  surface (canonical JSON rule, self-hash convention, diagnostic envelope,
  StageCache key construction), the F-B2/F-B4 RFC wins. F-B3/F-B5 inherit those
  surfaces unchanged unless this RFC explicitly amends them.
* If a later RFC changes any public type, report shape, cache key, diagnostic
  code, or canonicalization rule introduced here, that later RFC must
  explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not prose
  folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft (rev 1, post-Gemini + post-Codex review amendments applied) |
| Feature beads   | bd-1b4 **F-B3 QuantGraph (Stage 1)**; bd-7m2 **F-B5 GbInferIR (Stage 3)** |
| Open tasks      | To be minted: T-B3.1..T-B3.N (canonical-tensor binding, NormPlan binding, RoutingTable assembly, ExpertSection assembly, DecodeSpec binding, SequenceSemanticsSpec binding, provenance map, schema/round-trip tests, `quant_graph.json` emitter, StageCache wiring, F-B4 placeholder retirement); T-B5.1..T-B5.M (value/effect typing, op semantics, effect linearization, semantic equivalence with QuantGraph, `infer_ir.json` emitter, schema/round-trip tests, StageCache wiring, F-C2 oracle handshake) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` §"The compiler pipeline" stages 1 and 3; §"Reports and artifacts" `quant_graph.json` and `infer_ir.json` (newly defined here); §"`ArtifactCore` is target-independent"; §"Three oracles"; §"Norm and sequence-state semantics" |
| Glossary        | `history/glossary.md` (artifact stratum, denotational stratum, value/effect IR, sequence semantics, expert section, routing table, decode spec, normalization plan, provenance) |
| Constitution    | §I correctness by construction; §II three-stratum oracle correspondence; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ValidatedInputs`, `ResolvedCompilePolicy`, and the shared `ReportEnvelope` rule); F-C2 ArtifactOracle (op-for-op correspondence target for both IRs); F-B6 ObservationPlan (consumes `GbInferIR`); F-B7 RangePlan (consumes `GbInferIR`); F-B8 StoragePlan (consumes `GbInferIR`); F12 / bd-144 SequenceSemanticsSpec (provides `SequenceSemanticsSpec` shape consumed by F-B3) |
| Sister deps     | bd-c4wg (F-C2) — strictly downstream; bd-144 (F12) — sibling, schema-only consumption |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B3 + F-B5 sits inside the
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
  F-B3  Stage 1        QuantGraph                                        ← THIS RFC
  F-B4  Stage 2        StaticBudgetReport

Transformative stages (wrapped by FeasibilityRefinementLoop):
  F-B5  Stage 3        GbInferIR (value/effect IR)                       ← THIS RFC
  F-B6  Stage 4        ObservationPlan
  F-B7  Stage 5        RangePlan
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
        (uniformization pass; F-B3/F-B5 wire K1/K3 directly here)
```

Sequencing of ~weekly chunks (bkase 2026-05-07 conversation):

```text
Chunk 1 (in flight):  F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2 (THIS RFC):   F-B3 + F-B5         Stages 1, 3
Chunk 3 (next up):    F-B6 + F-B7         Stages 4, 5
Chunk 4:              F-B8                Stage 6
Chunk 5:              F-B9 + F-B10        Stages 7, 8
Chunk 6:              F-B11 + F-B12       Stages 8.5, 9
Chunk 7:              F-B13               Stages 10, 10.5
Chunk 8:              F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop
```

### 0.3 Where F-B3 and F-B5 sit in the pipeline

F-B3 and F-B5 are the **two canonical IRs** that bracket Stage 2:

* **F-B3 (Stage 1) `QuantGraph`** is the canonical artifact graph. It is
  the first IR built from a frozen, validated `ArtifactCore`, and is the
  surface `ArtifactOracle` (F-C2) compares against. It is target-independent
  and storage-free; only quantization formats, normalization plans, routing
  topology, expert sections, decode spec, sequence semantics, and provenance
  appear here.

* **F-B5 (Stage 3) `GbInferIR`** is the hardware-aware value/effect IR. It
  is the first IR with explicit effect edges (sequence-state mutation, RNG
  progression). It is still storage-free — no tile sizes, no buffers, no
  accumulator widths — but it is the IR every transformative stage from
  F-B6 onward consumes. F-C2 also compares against F-B5 op-for-op once F-B5
  is available.

Stage 2 (`StaticBudgetReport`, F-B4) sits between them in pipeline order
but is owned by the prior chunk's RFC. F-B4 currently consumes a placeholder
`QuantGraphBudgetSource`; this chunk retires that placeholder by
implementing the trait on the real `QuantGraph` (§13.1).

### 0.4 Cross-epic interactions

F-B3 + F-B5 sit at the intersection of four epics:

```text
Epic A → Epic B
  - gbf-foundation (BlobRef, BlobCodec, Hash256 wrappers)         consumed
  - gbf-store (StageCache) for K1 / K3 cache wiring                consumed
  - gbf-artifact types (ArtifactCore, CanonicalTensor, NormPlan,
    SequenceSemanticsSpec, DecodeSpec, ExportTensorId)             consumed
  - gbf-policy (ResolvedCompilePolicy, DeterminismClass)           consumed

Epic B (internal):
  - F-B2 / F-B4 (Stage 0, 0.5, 2) products + ReportEnvelope rule   consumed
  - F-B6 / F-B7 / F-B8 / F-B13 (Stages 4–10)                       feeds
  - F-B17 StageCache cross-cut                                     compatible

Epic C → Epic B (oracle correspondence):
  - F-C2 ArtifactOracle compares vs QuantGraph and GbInferIR        provided
    (this RFC pins canonical reference semantics F-C2 will use)
  - F-C2 dependency edge to F-B5 is minted in this chunk

Epic G → Epic B:
  - LexicalSpec, DecodeCapabilitySet, charset normalization        consumed
    (already pinned in ArtifactCore by training-side export)

Epic H → Epic B (deferred):
  - F-H1 KernelSpec + signatures                                   consumed
    later by F-B13/F-B15 — not by F-B3/F-B5 directly. This chunk
    deliberately stays kernel-agnostic; kernel selection happens at
    F-B13 (GbSchedIR).
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk straddles M1 and the front of M3:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup: runtime/banking/harness/emulator
              proven for sustained integer compute. Merged: c2edbaa.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; first conformance.json; first CompileRequest
              wiring.
              ↳ F-B2/F-B4 (Chunk 1)   delivers the CompileRequest wiring.
              ↳ F-B3 (this chunk)      delivers ArtifactOracle's input
                                       surface (the canonical artifact
                                       graph).

M2            One shared micro-kernel resolved by RomWindowPlan; one
              expert payload bank; emulator diffing against
              ScheduleOracle; first ReachabilityValidation pass.
              ↳ F-B5 (this chunk) provides the IR that ScheduleOracle
                eventually consumes; ScheduleOracle correspondence is
                still M2 work owned by F-C3.

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ F-B5 unblocks the M3 commitment by providing the IR
                surface F-B6 / F-B7 / F-B8 consume; the M3 commitment
                itself requires those downstream stages, which land in
                later chunks.

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ Out of scope for this chunk. SequenceSemanticsSpec is
                consumed (schema-only) but the conformance bring-up
                happens later.
```

The two IRs in this chunk are therefore the **bridge from M1 to M3**: F-B3
finishes the M1 artifact-stratum scaffold and F-B5 starts the M3 IR
surface. Without them, the rest of Epic B is contract-frozen at Stage 2 and
Epic C cannot start beyond `DenotationalOracle` (F-C1).

### 0.6 What the project as a whole gains when this chunk lands

```text
1. ArtifactOracle (F-C2) becomes implementable.
   Without QuantGraph there is no canonical artifact graph for F-C2 to
   evaluate. With QuantGraph, F-C2 can begin; with GbInferIR, F-C2 can
   complete its op-for-op correspondence.

2. F-B4's placeholder is retired.
   Static budget projection now runs against the real artifact shape,
   not a synthetic shape that could drift.

3. F-B6 / F-B7 / F-B8 become unblocked.
   Every transformative stage from Stage 4 onward consumes GbInferIR.
   Without F-B5 they cannot start.

4. The shape of the M1 conformance.json is settled.
   ArtifactOracle's report shape (F-C2) depends on QuantGraph's reportable
   identity. This chunk pins that identity.

5. Determinism class is end-to-end.
   ArtifactCore declares DeterminismClass; QuantGraph carries it; GbInferIR
   inherits it; ArtifactOracle uses it. Equality tightens from a vague
   "should match" to a typed "BitExact / NumericallyStable / SeedStable /
   DistributionStable" gate.

6. Provenance is end-to-end.
   ExportTensorId → TensorId (QG) → ValueId (IIR) lets every later report,
   diagnostic, and certificate trace back to a training-time export id.
   Debugging a layout bug can chase the same id through every stage.

7. The "canonical IR" discipline is reusable.
   The schema/canonicalization/self-hash/StageCache pattern from F-B2/F-B4
   now extends across two more reports (quant_graph.v1, infer_ir.v1).
   F-B6 / F-B7 / F-B8's report shapes inherit the same discipline.
```

### 0.7 Reading order for reviewers

A reviewer who has just read F-B2/F-B4 and is approaching this RFC for the
first time should read:

```text
§0  (this section) — placement and dependencies
§1  Project context — milestone-specific framing
§2  Load-bearing decisions — the engineering choices that bracket the rest
§5  Authority rules — what this RFC owns vs inherits
§6  Pipeline state machine — how Stage 1 and Stage 3 plug into Stages 0/0.5/2
§8  Stage 1 contract: QuantGraph
§9  Stage 3 contract: GbInferIR
§10 Report schemas (quant_graph.v1, infer_ir.v1)
§14 Task DAG
§17 End-to-end theorem
§19 Ambiguity ledger
```

Skim §3, §4, §7, §11, §12, §13, §15, §16, §18, and the spec pack for
specifics.

## 0a. TL;DR

This chunk lands the **two canonical IRs** that bracket Stage 2's static
budget filter and feed every transformative compiler stage from Stage 4
onward. It owns two numbered stages:

* **Stage 1 — `QuantGraph`.** The canonical artifact graph: frozen canonical
  tensors, explicit quant formats, explicit `NormPlan`s, optional
  `RoutingTable`, explicit `ExpertSection`s, explicit `DecodeSpec`, explicit
  `SequenceSemanticsSpec`, and complete provenance back to exported tensor
  ids. No physical packings, no reorders, no bank-chunked lowerings, no
  scheduling, no observation/probe plumbing. Comparable op-for-op against
  `ArtifactOracle` (F-C2) because neither side has committed to storage.

* **Stage 3 — `GbInferIR`.** The hardware-aware **value/effect IR**.
  Storage-free, but not effect-free: sequence-state mutation and RNG
  progression are explicit `EffectId` edges, not hidden in buffer aliasing.
  Typed by **value kind**, **quant format**, and **effect class**. Concrete
  address space, concrete buffers, tiling, accumulator scratch, semantic
  checkpoints, operational probes, and reduction structure are owned by
  later stages (F-B6, F-B7, F-B8, F-B9, …). Comparable op-for-op against
  `ArtifactOracle`.

These two features are paired in one RFC because they share the
**canonical-typed-IR** shape: each is a typed transform from a pinned input
into a content-addressed IR, each emits a canonical JSON report, each is an
oracle-correspondence point, each is consumed by the next stage by hash, and
each shares the diagnostic envelope, JSON canonicalization rule, self-hash
convention, and `StageCache` key construction inherited from F-B2/F-B4.
Stage 2 (`StaticBudgetReport`, F-B4) sits between them in pipeline order; it
is owned by the F-B2/F-B4 RFC and consumes the schema this RFC defines.

The chunk closes only when:

1. `QuantGraph` construction is a deterministic pure function of
   `ValidatedInputs` and `ResolvedCompilePolicy` and is byte-identical across
   two consecutive regenerations on a clean checkout.
2. `GbInferIR` construction is a deterministic pure function of `QuantGraph`
   and `ResolvedCompilePolicy` and is byte-identical across two consecutive
   regenerations on a clean checkout.
3. `quant_graph.json` and `infer_ir.json` round-trip through their semantic
   validators and self-hashes.
4. F-B4's placeholder `QuantGraphBudgetSource` is retired in favor of the
   real `QuantGraph` view; the existing F-B4 fixtures and decision tables
   continue to pass.
5. `StageCache` keys for Stage 1 and Stage 3 are pinned and tested.
6. The fixture build emits enough canonical reference data for a later
   `ArtifactOracle` (F-C2) implementation to evaluate `QuantGraph` and
   `GbInferIR` op-for-op. This chunk does **not** require F-C2 to exist,
   and it does **not** require `SemanticCheckpointId`-aligned equality —
   checkpoint attachment is owned by F-B6. The closure gate for this
   chunk is `NodeAnchor`-aligned fixture equivalence under the internal
   reference evaluator (`gbf-codegen::canonical::reference`).

The chunk does **not** include:

* Observation/probe plumbing — owned by F-B6 (Stage 4).
* Reduction-plan selection — owned by F-B7 (Stage 5).
* Storage class / lifetime / materialization decisions — owned by F-B8
  (Stage 6).
* Tiling, buffer assignment, accumulator scratch — owned by F-B13
  (`GbSchedIR`).
* Backend lowering, reachability, placement, encoding — owned by F-B15.
* `SemanticCheckpointSchema` definition — already in `gbf-artifact`, consumed
  but not authored here.
* `DenotationalOracle` / `ScheduleOracle` — Epic C, F-C1 / F-C3.
* Refinement-loop repairs — F-B16.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What F-B2/F-B4 leaves on the table

Per the F-B2/F-B4 RFC, by the time this chunk begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`,
  `TargetDataLoweringArtifact`, calibration, hint bundle, and
  `CompileRequest` are all admissible, hash-bound, and traceable through
  `artifact_validation.json`.
* `ResolvedCompilePolicy` is the single answer to "what policy governed this
  build," with provenance for every load-bearing scalar.
* `RuntimeChromeBudget` is honored at the static byte-math level. F-B4 has
  already emitted `static_budget.json` against a `QuantGraphBudgetView`
  obtained from a placeholder `QuantGraphBudgetSource`.

This chunk is responsible for replacing the placeholder with a real
`QuantGraph` and for landing the second canonical IR (`GbInferIR`) that
every transformative stage from F-B6 onward consumes.

### 1.2 What M1/M2 commits to and how this chunk delivers it

Per `planv0.md` §"Milestones":

> **M1**: `DenotationalOracle` + `ArtifactOracle` plus a single quantized
> dense kernel; conformance checking between reference observations and the
> frozen artifact (first `conformance.json`); first `CompileRequest` wiring.
> **M2**: one shared micro-kernel resolved by `RomWindowPlan`, plus one
> expert payload bank, with exact emulator diffing against `ScheduleOracle`
> and checkpoint alignment against `ArtifactOracle` at `SemanticCheckpointId`
> boundaries; first `ReachabilityValidation` pass integrated into the
> backend.
> **M3**: top-1 router, expert dispatch table, value/effect `GbInferIR` +
> `ObservationPlan` + `RangePlan` + `StoragePlan` wired end-to-end for a
> routed FFN under the cooperative scheduler.

Mapping:

* M1 commitment "the artifact stratum (`ArtifactCore`, `ArtifactManifest`,
  `ArtifactSemanticPayload`, `ArtifactOracle`)" requires F-B3. Without
  `QuantGraph`, `ArtifactOracle` has no canonical graph to evaluate against.
* M3 commitment "value/effect `GbInferIR` … wired end-to-end" requires F-B5.

Because M1 lands before M3, F-B3 is the M1-shaped half of this chunk and
F-B5 is the M2/M3-shaped half. Sequencing inside the chunk (§12) reflects
that.

### 1.3 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later transformative stage receives a typed, validated `QuantGraph`
  (Stage 1 product) or a typed, validated `GbInferIR` (Stage 3 product). They
  never re-derive shape, quant format, normalization plan, routing topology,
  or sequence semantics.
* F-B4 (`StaticBudgetReport`, Stage 2) consumes `QuantGraphBudgetView` from
  the real `QuantGraph` instead of a placeholder. The placeholder trait
  remains for fixtures only.
* F-C2 (`ArtifactOracle`) becomes implementable: both IRs expose op-for-op
  evaluation under the canonical reference semantics defined here.
* F-B6 (`ObservationPlan`) consumes `GbInferIR` to attach
  `SemanticCheckpointId` and `TraceProbeId` references; it never invents
  observation points.
* F-B7 (`RangePlan`) consumes `GbInferIR` reduction sites; it never derives
  reductions from `QuantGraph` directly.
* F-B8 (`StoragePlan`) consumes `GbInferIR` value ids and effect ids; it
  never invents `ValueId`/`EffectId`s from `QuantGraph`.

This chunk's job is to retire the **canonical IR** preconditions of the rest
of the pipeline. It is the third and fourth shift-left filters in the
system, after `gbf-train preflight`, F-B2 (Stage 0/0.5), and F-B4 (Stage 2).

### 1.4 Why this is two paired Features, not one feature or three

The natural unit is "the two canonical IRs that bracket Stage 2 and feed
every transformative stage."

* If we made it one feature, the bead would carry both an artifact-graph
  binder and a value/effect IR builder. The implementation surface is large
  enough that PR review fragments. It would also force F-C2 (`ArtifactOracle`)
  to wait on the entire chunk before any oracle work can start.
* If we made it three features (e.g. F-B3, F-B5, F-B5x for op semantics), we
  would split on op classes. That split is artificial: the value/effect IR
  is one cohesive surface, and op-class splits would re-converge during
  oracle correspondence testing.
* Two features matches the natural seam: F-B3 owns "the canonical artifact
  graph," F-B5 owns "the value/effect IR." They are paired in this RFC
  because they share an oracle-correspondence target, a content-addressing
  rule, and a JSON-report convention but ship as separate beads to keep PR
  scope tight and to allow F-B3 to land independently for F-C2.

### 1.5 What this chunk is NOT

The chunk is medium in *scope* but very large in *contract surface*. To
prevent scope creep, here is what this chunk explicitly is not:

* It is **not** a transform stage in the operational sense. F-B3 binds the
  canonical artifact graph from `ArtifactCore`'s typed contents; it does not
  invent semantics. F-B5 derives a value/effect graph from `QuantGraph`; it
  does not add semantics that are not already implicit in `QuantGraph`.
* It is **not** the producer of `ArtifactCore`. That is owned by training
  export (`gbf-train` / Epic A's training side).
* It is **not** the consumer of `TargetDataLoweringArtifact` for code
  generation. F-B3 records the `lowering_manifest_hash` in
  `QuantGraph.identity` and verifies it equals the hash recorded in Stage 0.
  It does not unpack any lowering shard.
* It is **not** the producer of `SemanticCheckpointSchema`. The schema is
  exported as part of `ArtifactAux` (per `planv0.md` line 442). F-B5
  references checkpoint ids at the IR level only; F-B6 selects which
  checkpoints are mandatory for a given build.
* It is **not** a kernel or quant runtime. F-B3 records `QuantSpec`,
  `WeightEncoding`, `ScaleGranularity`, `ScaleFormat`, and `ThresholdPlan`
  by hash and shape; it does not unpack ternary bitplanes or evaluate
  weight quantization.
* It is **not** an autoregressive driver. F-B5 represents the compute for
  **one token**; multi-token autoregressive iteration is at runtime.
* It is **not** a buffer/storage planner. F-B5 has no tile sizes, no
  buffer addresses, no concrete accumulator widths.
* It is **not** an observation/probe planner. F-B5 emits `EffectId`s for
  `SequenceState` and `Rng` only. `SemanticCheckpointId` and `TraceProbeId`
  attachment is owned by F-B6.
* It is **not** a refinement loop. The IRs are immutable products of their
  stage; no Stage 1 or Stage 3 pass calls earlier passes recursively.
* It does **not** depend on F-C2 (`ArtifactOracle`). F-C2 depends on F-B3
  (and consumes F-B5); within this chunk we land the IRs and a synthetic
  dense fixture against which a future F-C2 PR can verify oracle
  correspondence.

### 1.6 Relationship to F-C2 (`ArtifactOracle`)

`ArtifactOracle` (bd-c4wg) compares the artifact's canonical evaluation
op-for-op against `GbInferIR`. The bead's listed dependencies include F-B3
(blocking) but not F-B5; the prose description, however, says
`ArtifactOracle` "compares op-for-op against `GbInferIR`."

This RFC normalizes that ambiguity (see Ambiguity Ledger A1) by pinning the
following:

* F-C2 may begin against `QuantGraph` alone (no `GbInferIR`), evaluating
  the artifact under canonical reference semantics defined in §13.
* F-C2's *full* op-for-op correspondence requires `GbInferIR`. The blocker
  edge from F-C2 to F-B5 is therefore implicit in the RFC; we will mint a
  dependency edge `bd-c4wg -> bd-7m2` as part of this chunk.

## 2. Load-bearing decisions

### 2.1 Pure-function shape (core / driver split)

Both stages have **two layers**: a pure core constructor and a thin
driver that performs IO. The core is a pure function from typed pinned
inputs to typed content-addressed products. The driver wraps the core
with JSON emission and StageCache writes.

```text
build_quant_graph_core(QuantGraphInputs)
  -> Result<(QuantGraph, ReportEnvelope<QuantGraphReportBody>), PassDiagnostics>

run_stage1(QuantGraphInputs, env)
  = build_quant_graph_core(...) then
    (on success or failure):
      emit quant_graph.json
      may write StageCache success entry
      may write StageCache failure memo

build_infer_ir_core(GbInferIRInputs)
  -> Result<(GbInferIR, ReportEnvelope<InferIrReportBody>), PassDiagnostics>

run_stage3(GbInferIRInputs, env)
  = build_infer_ir_core(...) then
    (on success or failure):
      emit infer_ir.json
      may write StageCache success entry
      may write StageCache failure memo
```

Cores never mutate `ArtifactCore`, `ResolvedCompilePolicy`, or
`QuantGraph`. Drivers are the only IO surface. Determinism is required,
not aspirational.

The chunk-level pass shape is:

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed shape derivations)
       (typed semantic checks)
       (typed provenance binding)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed IR product, ReportEnvelope<ReportV1> }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON
       writes StageCache success / failure memo
```

Every report includes `outcome: ReportOutcome` per F-B2/F-B4 §2.1.

### 2.2 Inheritance from F-B2/F-B4

This RFC inherits, **unchanged**, the following from
`F-B2-F-B4-pipeline-entry-validation.md`. Each item names the precise
F-B2/F-B4 section so a future amendment to F-B2/F-B4 cannot silently
weaken what this RFC depends on:

* `ReportEnvelope<R>` shape and public JSON conventions — F-B2/F-B4 §4.
* `Hash256`, `DomainHash(...)`, `SelfHash(report)`, `ZERO_HASH` — F-B2/F-B4
  §1.
* `CanonicalJson(x)` rule (UTF-8, lex object keys, integers only, no NaN/Inf,
  no unknown fields, explicit enum tags, deterministic array ordering where
  order is not semantically meaningful) — F-B2/F-B4 §1.
* `null` policy (only for explicit semantic absence; never for unknown,
  unmeasured, or omitted) — F-B2/F-B4 §1.
* `R-Hash`, `R-Outcome-Pass`, `R-Outcome-Fail`, `R-FlatEnvelope`,
  `R-UnknownReject` envelope laws — F-B2/F-B4 §4.
* `ValidationDiagnostic` shape (`severity`, `origin`, `code`, `detail`,
  `provenance`) — F-B2/F-B4 §5. New origins and codes are introduced in
  §12 of this RFC; they extend the closed enum without modifying existing
  variants.
* `R-HardOnly-ThisChunk`: F-B3/F-B5 reports reject `Soft` diagnostics —
  F-B2/F-B4 §4.
* `D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`, `D-Provenance`
  diagnostic laws — F-B2/F-B4 §5.
* StageCache key construction rule
  `DomainHash(crate, "StageCacheKey", schema_id, schema_version, canonical_json_bytes)`
  — F-B2/F-B4 §11.

If a later amendment to F-B2/F-B4 changes any of the above, that amendment
must explicitly amend this RFC by name (see Authority rules, §5).

This RFC adds the following to that surface:

* Two new `ValidationOrigin` variants: `QuantGraphConstruction` and
  `InferIrConstruction`.
* Two new `ReportSchemaId` variants: `quant_graph.v1` and `infer_ir.v1`.
* Two new IR product types: `QuantGraph` and `GbInferIR`.
* Two new public report bodies: `QuantGraphReportBody` (§10.1) and
  `InferIrReportBody` (§10.2).
* Two new `StageCacheKey` schemas (§11): `K1 := QuantGraphCacheKey`,
  `K3 := InferIrCacheKey`.

### 2.3 Storage-freeness, restated formally

Per `planv0.md` line 1592, `GbInferIR` is "storage-free, but not
effect-free." This RFC pins that semantics:

```text
Forbidden in QuantGraph and GbInferIR:
  TileSize
  BufferAddress
  AccumulatorWidth
  StorageClass
  LifetimeClass
  Materialization
  AliasClassId
  PageId / CommitGroupId
  RamRegion / RomRegion / SramRegion
  ConcreteByteOffset
  ConcreteRomBank
  KernelResidency
  SchedSlice / ResourceVector / FrameBudget

Permitted in QuantGraph:
  CanonicalTensorLayout (logical shape + element kind)
  QuantSpec (logical quant scheme)
  NormPlan
  RoutingTable
  ExpertSection
  DecodeSpec
  SequenceSemanticsSpec
  TensorId / ExportTensorId / LayerId / ExpertId / NormPlanId / DecodePlanId

Permitted in GbInferIR:
  ValueId / EffectId / NodeId
  ValueKind
  QuantFormat (drawn from QuantSpec)
  EffectClass
  InferOp (closed enum, §9.2)
  TokenInput, TokenInputId, TokenIngressMode, StateSlotId, RngSlot
  Provenance back to QuantGraph entities
```

### 2.4 Effect-awareness, restated formally

`GbInferIR` carries explicit effects. The RFC restricts the effect classes
to a closed set:

```text
EffectClass :=
  SequenceState(StateSlotId)
  | Rng(RngSlot)
  | FaultBoundary
```

Each effect class is **linearly ordered** in the IR: every two operations
that touch the same effect class are totally ordered along that effect
chain. Operations that touch different effect classes are partially ordered
only by value dependencies. (See §9.5.)

`SemanticCheckpoint` is **not** an effect class in this chunk. F-B6 owns
checkpoint emission. The IR does, however, include positional anchors
(NodeId-aligned) so that F-B6 can attach checkpoints without changing IR
shape.

### 2.5 Single-token convention

`GbInferIR` represents the compute for **one token**. The runtime drives
auto-regressive iteration by re-entering the IR with updated
`SequenceSemantics` state across token boundaries.

```text
GbInferIR is one IR-pass per token.
Auto-regressive multi-token decoding is at runtime.
Multi-token batched IR is forbidden in this chunk.
```

Amends planv0: planv0.md does not pin token cardinality at the IR level; it
implies one-token semantics through `DecodeToken` ops. This RFC makes that
pinning explicit so downstream stages (`StoragePlan`, `GbSchedIR`) do not
need to assume.

### 2.5a Sequence block lowering is reserved in v1

This chunk consumes and validates `SequenceSemanticsSpec` at the artifact
schema level, but it does **not** lower nontrivial sequence-state compute.
`SequenceRead`, `SequenceStep`, and `SequenceWrite` remain in the v1
`InferOp` enum as **reserved shape** for the later sequence-state
amendment, but the Stage 3 builder emits them only when the artifact
declares an identity sequence block for every layer.

```text
F-B5-SequenceV1:
  In infer_ir.v1, non-identity sequence compute is rejected.

  Allowed:
    SequenceSemanticsSpec declares no runtime state slots for any layer
    AND the artifact marks the sequence block as Identity.

  Rejected:
    Any layer ℓ with state_slots_for_layer(ℓ) non-empty;
    BoundedKv, LinearState, recurrent state, attention/KV slabs;
    sequence tensor roles requiring compute or mutation;
    any non-identity sequence_block(a, s, q, ℓ) under canonical reference
    semantics.

  Diagnostic:
    InferIrSequenceSemanticsUnsupportedV1
```

Consequences for the IR shape (v1):

* `EffectAllocation` (§9.3) emits no `SequenceState(_)` chains; only
  `Rng(Decode)` when `requires_rng = true`.
* `SequenceRead` / `SequenceStep` / `SequenceWrite` exist in `InferOp` and
  in §9.7a's signature predicate as **reserved**, but never appear in a
  v1 `g.nodes`.
* The reference-semantics evaluator (§8.7, §9.8) treats `sequence_block`
  as the identity for v1 artifacts: `b = a`, `s' = s`.
* Sequence-state slots, KV slabs, and per-layer state are declared at the
  schema level (`SequenceSemanticsSpec.state_slots`) but must be empty in
  v1; declaring slots is a Stage 1 hard reject.

Amends planv0: planv0.md describes sequence-state semantics as a later
(M4) track. This RFC preserves the schema reference and the reserved op
shape but does not lower sequence compute in v1.

### 2.6 No scheduling fusion at the IR level

Both IRs are at the **canonical-semantic-site** level. Scheduling /
performance fusion (e.g. `Norm + MatMul`, `Embedding + Norm`,
cross-layer fusion, or combining FFN activation with the down projection)
happens at scheduling time (F-B13) under autotune constraints (F-B14,
F-E6). This chunk forbids implicit cross-site fusion in either IR.

Bias addition inside an affine projection, affine parameters inside a
`NormPlan`, score normalization / tie-breaking inside `RouteTop1`, and
named numeric clamp boundaries (residual combine, classify logit, FFN
activation) are part of those canonical op semantics and are **not**
scheduling fusions.

```text
F-NoScheduleFuse:
  ∀ node n ∈ GbInferIR.
    n.op is exactly one InferOp variant.
    No InferOp variant represents a fusion of two distinct canonical
    semantic sites.
```

### 2.7 Op-for-op correspondence with `ArtifactOracle`

`ArtifactOracle` (F-C2) compares the artifact's canonical evaluation
op-for-op against `GbInferIR`. To make this comparison definable, this RFC
pins **canonical reference semantics** for every `InferOp` variant (§9.2)
and for every `QuantGraph` entity that contributes to evaluation (§8.7).

Both IRs share the same canonical reference semantics. F-C2 evaluates the
artifact through `QuantGraph`'s reference semantics; the resulting per-op
and per-checkpoint values must equal `GbInferIR`'s reference semantics on
the same input.

```text
Definition: UniversalSemanticEquivalence
  GbInferIR g and QuantGraph q are UniversallySemanticallyEquivalent iff:
    ∀ token-input t, sequence-state s, RNG state gen.
      eval_canonical_qg(q, t, s, gen) = eval_canonical_ir(g, t, s, gen)
    where = is bit-exact under the artifact's
    ReferenceNumericProfile.determinism setting.

Definition: FixtureSemanticEquivalence
  GbInferIR g and QuantGraph q are FixtureSemanticallyEquivalent iff
  the internal reference evaluator (gbf-codegen::canonical::reference)
  agrees bit-for-bit on the RFC fixture input set.
  This is the closure gate for this chunk.
```

Determinism class is read from `ArtifactCore.numeric_profile`
(`DeterminismClass::BitExact | NumericallyStable | SeedStable | DistributionStable`).

This RFC defines the **universal** relation only for `BitExact`; proving
it for arbitrary workloads is deferred to F-C2 / F-C4. The chunk-closure
gate is `FixtureSemanticEquivalence`, not `UniversalSemanticEquivalence`.

### 2.8 Provenance chain end-to-end

Every artifact-derived entity in this chunk has a typed provenance edge,
and every IR entity has a typed producer/provenance record on the IR
product (`g.provenance`):

```text
ExportTensorId  --(F-B3 binds)--> TensorId (in QuantGraph)
                                  Aux ExportTensorIds bind on each
                                  QuantAuxBlobRef.export_tensor_id.

TensorId / NormPlanId / ExpertSection / RoutingTable / DecodePlanId
                --(F-B5 binds)--> NodeId via g.provenance.nodes

NodeOutput      --(F-B5 binds)--> ValueId via g.provenance.values
                                  (with ValueProducerRef::Node | External)

EffectClass instance
                --(F-B5 binds)--> EffectId via g.provenance.effects
                                  (with EffectProvenance::ExternalRoot | NodeOutput)
```

Every IR product carries the inverse maps for review and for
`ArtifactOracle` correspondence reporting. Provenance is **not** stored
inline on `GbNode`, `ValueDecl`, or `EffectDecl`; the maps on
`GbInferIR.provenance` are the single source of truth.

### 2.9 Schema versioning

Both IR schemas are versioned independently of the existing F-B2/F-B4
report schemas:

```text
quant_graph.v1
infer_ir.v1
```

Schema bumps follow F-B2/F-B4 §10's compatibility rules (any later RFC that
changes shape, canonicalization, or self-hash must amend this RFC).
Cross-major artifact schema migration is still owned by `gbf-migrate`
(deferred per F-A6b).

### 2.10 Determinism mode binding

`ArtifactCore.numeric_profile.determinism` selects the equality used by
`ArtifactOracle`. This chunk emits the determinism class verbatim into both
IR reports for downstream gates:

```text
quant_graph.report.identity.determinism = artifact_core.numeric_profile.determinism
infer_ir.report.identity.determinism    = artifact_core.numeric_profile.determinism
```

If the artifact requires `BitExact` and any reduction's canonical order is
not pinned (`reduction_order_policy != Enforced`), F-B3 emits a Hard
diagnostic `QuantGraphDeterminismRequiresEnforcedReductionOrder`. F-B5
inherits the determinism class without re-checking.

`BitExact` is meaningful only when the reduction tree is pinned **and**
mid-reduction saturation is forbidden. F-B3 rejects any `NormPlan` or
`WeightEncoding` that requires mid-reduction clipping under `BitExact`
with `QuantGraphBitExactMidReductionSaturationForbidden`. Saturation /
clamp is permitted only at **named numeric boundaries** declared by the
artifact schema:

* residual combine (`q.residual_plan.combine_policy`)
* classify-logit boundary (`q.classify_head.logit_format`)
* FFN activation output (`q.ffn_plans[ℓ].intermediate_format`)
* final activation clamp, when the artifact declares one

Future support for explicit `ReductionOrder + SaturationPolicy` requires
an explicit RFC amendment.

### 2.11 Routing presence is artifact-driven

Whether `RoutingTable` is present is determined by `ArtifactCore.model`
(`ModelSpec` indicates whether the model uses routed FFN or dense FFN per
layer). F-B3 enforces the consistency rule:

```text
∀ layer ℓ.
  ModelSpec.layer(ℓ).ffn_kind = Routed
    ⇔ exactly one routing_table_entry r ∈ QuantGraph.routing_table.layers
        with r.layer = ℓ
    ∧ ∀ expert_id e where 0 ≤ e < n_experts(ℓ).
        exactly one ExpertSection exists with layer = ℓ and expert = e

  ModelSpec.layer(ℓ).ffn_kind = Dense
    ⇔ no routing_table_entry exists with r.layer = ℓ
    ∧ exactly one ExpertSection exists with layer = ℓ and expert = 0
       (the dense-FFN section is encoded as a single "expert 0" payload)

  Under canonical reference semantics, dense FFN is mathematically
  equivalent to a routed FFN with hard router probability 1.0 on
  expert 0. This is an equivalence statement only. It does **not**
  require dense layers to contain `RouterMatVec`, `RouteTop1`, or
  `SelectExpertTop1` nodes in `GbInferIR`. Dense layers lower through
  the direct expert-0 path; routed layers carry the full routing
  pipeline.
```

Amends planv0: planv0.md leaves the dense-vs-routed distinction informal at
the `ExpertSection` boundary. This RFC pins that dense FFN is encoded as
exactly one `ExpertSection` per layer with `expert == 0` (the
"single-expert" convention from F13's dense baseline track).

### 2.12 No semantic checkpoints in this chunk

`SemanticCheckpointId` is referenced but not emitted in this chunk. F-B5
exposes a stable, hash-derived `SemanticAnchor` for every IR node so F-B6
can attach checkpoints later without altering IR shape:

```text
GbInferIR.anchors: NodeAnchorMap
NodeAnchorMap : BTreeMap<NodeId, SemanticAnchor>

SemanticAnchor :=
  DomainHash(
    "gbf-codegen", "SemanticAnchor", "v1",
    CanonicalJson({
      quant_graph_self_hash: Hash256,
      node_id: NodeId,
      op_tag: InferOpTag,
      canonical_provenance_tuple:
        (op_tag, layer?, expert?, slot?, norm_site?, state_slot?, occurrence_index)
    })
  )
```

`SemanticAnchor` is exported as opaque data inside the `GbInferIR`
product (and thus `infer_ir.json` `result.product.anchors`) so a
StageCache hit replaying the IR produces the same anchors. The anchor
contents do **not** include any `SemanticCheckpointId` or `TraceProbeId`
— those remain F-B6's authority. F-B5 enforces:

```text
F-B5-NoCheckpoint:
  GbInferIR contains no SemanticCheckpointId, TraceProbeId,
  SemanticCheckpoint effect class, or checkpoint/probe attachment field.
  (SemanticCheckpoint is not in EffectClass at all in v1; F-B6 attaches
  checkpoints to NodeAnchors without altering IR shape.)
```

### 2.13 Token ingress is a runtime decision, not an IR-shape decision

`InferOp::Embedding` consumes a single external `TokenInput` value, not a
choice between two compile-time variants. Prompt-vs-autoregressive
selection happens at runtime ingress; the IR shape is identical for both
modes. Per the single-token convention (§2.5), the IR holds exactly one
`Embedding` node per pass.

```rust
pub enum TokenIngressMode {
    PromptInput,           // first-token / prefill input
    AutoregressiveOutput,  // previous-token output of DecodeToken
}

pub struct TokenInput {
    pub token_input_id: TokenInputId,
    /// External ValueId consumed by the unique Embedding node. The
    /// corresponding ValueDecl has kind = InputToken and format =
    /// TokenIdDomain { vocab_size = ModelSpecSummary.vocab_size }.
    pub value_id: ValueId,
    /// Set of ingress modes the runtime may bind to this Embedding. The
    /// runtime selects exactly one mode at IR-pass entry. The IR shape
    /// is invariant under that choice.
    pub allowed_ingress_modes: NonEmptySet<TokenIngressMode>,
}

pub struct TokenInputId(u32);
```

The `Embedding` op carries a `TokenInputId` referencing a `TokenInput`
declared at the IR head. F-B5 emits exactly one `TokenInput` per IR pass;
its `value_id` is consumed by the unique `Embedding` node.

```rust
InferOp::Embedding { token_input: TokenInputId }
```

Amends planv0: planv0.md presents `Embedding { token_src: TokenSrc }` as
two compile-time variants. This RFC replaces that with a single
`TokenInput` value whose ingress mode is bound at runtime, eliminating the
contradiction between "IR shape is the same for both modes" and "the IR
holds one of two variants."

### 2.14 RNG is an effect, seed is runtime

```text
∀ node n ∈ GbInferIR.
  n.op = DecodeToken { plan } ∧ plan.requires_rng ⇒ Rng(rng_slot) ∈ n.effects_in
                                                   ∧ Rng(rng_slot) ∈ n.effects_out

The RNG seed is bound at runtime via InferenceState; it is not part of
QuantGraph or GbInferIR.
```

`RngSlot` is a static enumeration of distinct RNG streams the IR consumes
(in v1: `RngSlot::Decode` only). New RNG slots require RFC amendment.

### 2.15 No partial layers, no conditional compute at IR level

F-B5 does not encode "skip this layer" or "skip this expert" at the IR
level. `RouteTop1` produces both the selected expert id (`RouterDecision`)
and the gating weight; `ExpertMatVec` is unconditional. Scheduling decides
whether to short-circuit evaluation at runtime based on the routing
decision.

```text
F-AllExpertSlotsRealized:
  Every declared expert weight slot in QuantGraph (i.e. every TensorId
  with role ExpertWeight { layer, expert, slot }) appears in exactly one
  InferOp::ExpertMatVec node per IR pass.

F-RoutedSelectionTotal:
  For every layer with FfnKind = Routed, all expert candidate values
  (one per expert) feed exactly one InferOp::SelectExpertTop1 node.
  Only the selected candidate contributes to the residual under canonical
  semantics; non-selected candidates are computed but discarded.
```

This guarantees F-C2's per-op correspondence is total: there is no IR
variant that "doesn't compute on this token" the oracle has to reconcile.
Runtime short-circuiting is a scheduling decision (F-B13) that does not
change IR shape.

### 2.16 Rejection of training-graph residue

`QuantGraph` is not a training graph. F-B3 enforces:

```text
F-B3-NoTraining:
  No optimizer state, no gradient buffer, no training-time mask, no
  training-time KL/CE loss target, no router auxiliary-loss tensor, no
  EMA-shadow tensor, no straight-through-estimator placeholder appears in
  QuantGraph.tensors.
```

Any tensor whose `ExportTensorId` is marked
`ArtifactCore.export_role = TrainingOnly` triggers `QuantGraphTrainingResidue`.

### 2.17 No "quick fix" upgrade in this chunk

If `QuantGraph` construction would succeed only by silently filling in
defaults (e.g. default `NormPlan::None`, default `DecodeSpec::greedy`), F-B3
fails. Every `QuantGraph` field is derived from `ArtifactCore` or fails
loudly. This is the same shift-left discipline that F-B2/F-B4 enforce.

## 3. Glossary additions

This chunk introduces or pins the following terms beyond the F-B2/F-B4
glossary inheritance.

| Term                       | Definition                                                                                  |
|----------------------------|---------------------------------------------------------------------------------------------|
| Canonical artifact graph   | The frozen, target-independent model graph (`QuantGraph`).                                  |
| Value/effect IR            | The hardware-aware IR with explicit effect edges (`GbInferIR`).                             |
| Effect class               | One of `SequenceState`, `Rng`, `FaultBoundary`. Closed in v1.                               |
| Effect chain               | The total order over operations touching one effect class.                                  |
| TensorId                   | QuantGraph-internal identifier for a `QuantTensorRef`.                                      |
| ExportTensorId             | Artifact-internal identifier exported by `gbf-train`. Maps to `TensorId` via provenance.    |
| ValueId                    | GbInferIR-internal identifier for a value edge.                                             |
| EffectId                   | GbInferIR-internal identifier for an effect edge.                                           |
| NodeId                     | GbInferIR-internal identifier for a `GbNode`.                                               |
| StateSlotId                | Identifier for a sequence-state slot (per-layer, per-stream within `SequenceSemanticsSpec`).|
| NormPlanId                 | QuantGraph-internal identifier for a unique `NormPlan` instance.                            |
| DecodePlanId               | QuantGraph-internal identifier for the (single) `DecodeSpec` instance.                      |
| ExpertSection              | `(LayerId, ExpertId, tensor_refs)` for one routed-FFN expert; `tensor_refs` carries weights and biases for that expert. Aux artifacts (scales, thresholds, sparse meta) live on each `QuantTensorRef.aux_blob_refs`. |
| RoutingTable               | Per-layer router weight tensors plus routing semantics; absent for fully-dense models.      |
| Single-token convention    | Each IR pass represents exactly one token's compute.                                        |

## 4. Core notation

This RFC inherits §1 of F-B2/F-B4 (Hash256, Outcome, Severity, Stage,
ReportSchema, Result, Option, NonEmptyList, SortedBy, DomainHash, SelfHash,
CanonicalJson, ZERO_HASH, null policy). Additions:

```text
Stage :=
  Stage0 | Stage0_5 | Stage1 | Stage2 | Stage3   -- Stage1 and Stage3 added

ReportSchema :=
  artifact_validation.v1
  | policy_resolution.v1
  | static_budget.v1
  | quant_graph.v1            -- new
  | infer_ir.v1               -- new

ValidationOrigin (extension) :=
  ...existing F-B2/F-B4 origins...
  | QuantGraphConstruction
  | InferIrConstruction
```

Abbreviations used throughout:

```text
QG  := QuantGraph
IIR := GbInferIR (also GbInferIR or just "InferIR")
```

## 5. Authority rules

```text
Scope(F-B3/F-B5) =
  {
    Stage1,
    Stage3,
    QuantGraph,
    GbInferIR,
    quant_graph.v1,
    infer_ir.v1,
    StageCache keys for Stage1 and Stage3,
    canonical reference semantics for every QG entity and every InferOp,
    the closed EffectClass set,
    the closed TokenIngressMode set,
    the closed RngSlot set,
    F-B4 placeholder retirement (the migration of QuantGraphBudgetSource
      from a placeholder to QuantGraph)
  }

Rule Authority:
  ∀ behavior b.
    b ∈ Scope(F-B3/F-B5) ∧ RFC specifies b
    ⇒ SourceOfTruth(b) = RFC

Rule PlanContext:
  ∀ behavior b.
    b ∈ Scope(F-B3/F-B5) ∧ RFC silent on b
    ⇒ planv0 may inform implementation but is not an acceptance gate

Rule Inheritance:
  ∀ behavior b.
    b ∈ Scope(F-B2/F-B4) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B2/F-B4 RFC

Rule Amendment:
  LaterRFC changes any of:
    public QG type
    public IIR type
    report shape (quant_graph.v1, infer_ir.v1)
    cache key (K1, K3)
    diagnostic code introduced here
    canonical reference semantics
  ⇒ LaterRFC must explicitly amend this RFC

Rule DivergenceLedger:
  RFC intentionally diverges from planv0
  ⇒ nearest relevant section must contain `Amends planv0`
```

## 6. Pipeline state machine

Extending the F-B2/F-B4 state machine:

```text
State :=
  Imported(inputs)
  | Validated(validation_product)
  | PolicyResolved(policy_product)
  | QuantGraphReady(policy_product, quant_graph_product)
  | BudgetPassed(quant_graph_product, static_budget_product)
  | InferIrReady(budget_product, infer_ir_product)
  | Halted(stage, report, diagnostics)
```

Transitions (extending F-B2/F-B4):

```text
T1 build_quant_graph:
  PolicyResolved(p)
    -- build_quant_graph(p) = Ok(q) -->
  QuantGraphReady(p, q)

  PolicyResolved(p)
    -- build_quant_graph(p) = Err(e) -->
  Halted(Stage1, e.report, e.diagnostics)

T2 budget (existing F-B4):
  QuantGraphReady(p, q)
    -- static_budget(p, q.budget_view, runtime_budget) = Ok(b) -->
  BudgetPassed(q, b)

  QuantGraphReady(p, q)
    -- static_budget(...) = Err(e) -->
  Halted(Stage2, e.report, e.diagnostics)

T3 build_infer_ir:
  BudgetPassed(q, b)
    -- build_infer_ir(q, p) = Ok(g) -->
  InferIrReady(b, g)

  BudgetPassed(q, b)
    -- build_infer_ir(q, p) = Err(e) -->
  Halted(Stage3, e.report, e.diagnostics)
```

Pipeline invariants (additions to F-B2/F-B4 §3):

```text
I-Pipeline-9:
  Stage1 may run only after Stage0_5 Passed.

I-Pipeline-10:
  Stage3 may run only after Stage2 Passed.

I-Pipeline-11:
  If Stage1 fails, Stage2 and Stage3 do not run.

I-Pipeline-12:
  If Stage2 fails, Stage3 does not run.

I-Pipeline-13:
  Stage1 and Stage3 are passive in the IR-product sense:
    They produce their own product but never mutate
    ArtifactCore, ResolvedCompilePolicy, QuantGraph, or
    RuntimeChromeBudget.

I-Pipeline-14:
  quant_graph.report_self_hash is immutable after Stage1 emits it.
  infer_ir.report_self_hash    is immutable after Stage3 emits it.

I-Pipeline-15:
  Every emitted report must satisfy SelfHash(report) = report.report_self_hash.

I-Pipeline-16:
  Stage3's IR product does not change shape between two consecutive
  regenerations on the same QuantGraph and ResolvedCompilePolicy hashes.
```

## 7. Report envelope (inherited)

Both `quant_graph.json` and `infer_ir.json` use the
`ReportEnvelope<R>` shape from F-B2/F-B4 §4 unchanged:

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
`R-Outcome-Fail`, `R-FlatEnvelope`, `R-UnknownReject`, `R-HardOnly-ThisChunk`)
are inherited unchanged. Specifically: F-B3/F-B5 reports reject `Soft`
diagnostics in this chunk.

`R-NoPartialProduct` is restated for the IR products:

```text
R-NoPartialIR-QG:
  Failed quant_graph report
  ⇒ body.result = None

R-NoPartialIR-IIR:
  Failed infer_ir report
  ⇒ body.result = None
```

## 8. Stage 1 contract: `QuantGraph`

### 8.1 Type-level contract

```text
QuantGraphInputs :=
  {
    validated: ValidatedInputs,                  -- from F-B2 Stage 0
    policy: ResolvedCompilePolicy,               -- from F-B2 Stage 0.5
    artifact_core: ArtifactCoreView,             -- typed view, hash-bound
    sequence_semantics: SequenceSemanticsSpec,   -- read from artifact_core
    blob_index: ResolvedBlobIndex,               -- pure, hash-bound metadata view
  }

ResolvedBlobIndex :=
  {
    entries: BTreeMap<BlobRef, BlobMetadata>,
    self_hash: Hash256,                          -- DomainHash over canonical entries
  }

BlobMetadata :=
  {
    content_hash: Hash256,
    encoded_size_bytes: u64,
    decoded_size_bytes: u64,
    codec: BlobCodec,
  }

-- The Stage 1 driver (run_stage1) builds ResolvedBlobIndex using IO. The
-- pure build_quant_graph_core function receives only this immutable,
-- hash-bound metadata view: it never opens a file or follows a BlobRef.

QuantGraphProduct :=
  {
    quant_graph: QuantGraph,
    report: Report[QuantGraphReportBody],
    quant_graph_self_hash: Hash256,
    quant_graph_canonical_bytes_hash: Hash256
  }

QuantGraphStageFailure :=
  {
    report: Report[QuantGraphReportBody],
    diagnostics: NonEmptyList[ValidationDiagnostic]
  }
```

The public `QuantGraph` type:

```rust
pub struct QuantGraph {
    pub identity: QuantGraphIdentity,
    pub tensors: Vec<QuantTensorRef>,
    pub norm_plans: Vec<NormPlanRecord>,
    pub layer_norms: BTreeMap<LayerId, LayerNorms>,
    pub routing_table: Option<RoutingTable>,
    pub expert_sections: Vec<ExpertSection>,
    pub ffn_plans: BTreeMap<LayerId, FfnPlan>,
    pub decode_spec: DecodeSpecRecord,
    pub sequence_semantics: SequenceSemanticsSpec,
    pub provenance: TensorProvenanceMap,
    pub classify_head: ClassifyHead,
    pub residual_plan: ResidualPlan,
}

pub struct FfnPlan {
    pub layer: LayerId,
    pub activation_kind: FfnActivationKind,
    pub intermediate_format: QuantFormat,
        // Named numeric boundary applied at the FfnActivation output.
}

pub enum FfnActivationKind {
    Relu,
    Gelu,
    SiLU,
    SwiGLU,
}

pub struct ResidualPlan {
    pub activation_format: QuantFormat,
    pub combine_policy: ResidualCombinePolicy,
}

pub enum ResidualCombinePolicy {
    /// Compute x ⊕ δ in ExactAccumulator, then clamp/round to
    /// activation_format at the named residual boundary. This is the
    /// only v1 policy.
    AddThenClampNamedBoundary,
}

pub struct QuantGraphIdentity {
    pub artifact_core_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub semantic_core_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub determinism: DeterminismClass,
    pub model_spec_summary: ModelSpecSummary,
        // n_layers, n_experts(layer): u16, d_model, d_ff,
        // vocab_size: u32, per-layer ffn_kind.
        // The full per-layer FFN activation/intermediate format lives in
        // QuantGraph.ffn_plans, not here.
}

pub struct QuantTensorRef {
    pub tensor_id: TensorId,
    pub layout: CanonicalTensorLayout,
    pub quant_format: QuantFormat,
    pub role: QuantTensorRole,             // §8.6
    pub blob: ResolvedBlobRef,
    pub aux_blob_refs: Vec<QuantAuxBlobRef>,
}

pub struct ResolvedBlobRef {
    pub blob_ref: BlobRef,
    pub content_hash: Hash256,
    pub encoded_size_bytes: u64,
    pub decoded_size_bytes: u64,
    pub codec: BlobCodec,
}

pub struct QuantAuxBlobRef {
    pub kind: QuantAuxKind,                // Scale | Threshold | SparseMeta
    pub layout: CanonicalTensorLayout,
    pub format: AuxFormat,                 // Q8_8 | Q4_4 | Pow2 | I8 | I16 | etc.
    pub blob: ResolvedBlobRef,
    pub export_tensor_id: ExportTensorId,
}

pub enum QuantAuxKind {
    Scale,
    Threshold,
    SparseMeta,
}

pub enum QuantTensorRole {
    EmbeddingTable,
    NormScale { norm_plan: NormPlanId },
    NormBias  { norm_plan: NormPlanId },
    RouterWeight  { layer: LayerId },
    RouterBias    { layer: LayerId },
    ExpertWeight  { layer: LayerId, expert: ExpertId, slot: ExpertWeightSlot },
    ExpertBias    { layer: LayerId, expert: ExpertId, slot: ExpertWeightSlot },
    ClassifyWeight,
    ClassifyBias,
}
// Note: SharedDenseWeight / SharedDenseBias are intentionally absent in
// v1. A shared-dense FFN branch on routed layers is out of scope and
// requires explicit RFC amendment (see Mixed-topology note in §9.1).

pub enum ExpertWeightSlot {
    FfnGate,    // required iff ffn_activation_kind = SwiGLU; forbidden otherwise
    FfnUp,
    FfnDown,
}

pub struct LayerNorms {
    /// Norm applied before the sequence block (post-residual at layer entry).
    pub pre_sequence: NormPlanId,
    /// Norm applied before the FFN (post-sequence-residual).
    pub pre_ffn: NormPlanId,
}

pub struct NormPlanRecord {
    pub norm_plan_id: NormPlanId,
    pub site: NormSite,
    pub plan: NormPlan,                    // see planv0.md line 623
    pub input_format: QuantFormat,
    pub output_format: QuantFormat,
}

pub enum NormSite {
    LayerSequence { layer: LayerId },
    LayerFfn      { layer: LayerId },
    Final,                                 // pre-classify norm
}

pub struct RoutingTable {
    pub layers: Vec<RouterLayer>,
}

pub struct RouterLayer {
    pub layer: LayerId,
    pub n_experts: u16,
    pub router_weight: TensorId,
    pub router_bias: Option<TensorId>,
    pub semantics: RouterSemantics,
}

pub enum RouterSemantics {
    Top1Hard {
        gate_weight: RouterGateWeightSemantics,
        tie_break:   RouterTieBreak,
    },                                     // v1
}

pub enum RouterGateWeightSemantics {
    /// Selected expert contributes with multiplicative weight 1.0.
    One,
    /// Selected expert contributes with the selected router score after
    /// the artifact-defined score normalization (e.g. softmax).
    SelectedScore,
}

pub enum RouterTieBreak {
    /// argmax ties broken by the lowest ExpertId. v1 only.
    LowestExpertId,
}

pub struct ExpertSection {
    pub layer: LayerId,
    pub expert: ExpertId,
    pub tensor_refs: Vec<TensorId>,        // weights and biases for this expert
}

pub struct DecodeSpecRecord {
    pub decode_plan_id: DecodePlanId,
    pub spec: DecodeSpec,                  // selects from DecodeCapabilitySet
    pub requires_rng: bool,
}

pub struct ClassifyHead {
    pub kind: ClassifyHeadKind,
    pub weight: TensorId,
    pub bias: Option<TensorId>,
    pub logit_format: QuantFormat,
        // Named numeric boundary applied at the classify-logit clamp/round.
}

pub enum ClassifyHeadKind {
    Tied,    // shares weights with EmbeddingTable
    Untied,
}

pub type TensorProvenanceMap = BTreeMap<TensorId, ExportTensorId>;
```

Notes:

* `aux_blob_refs` replaces `scale_blob_ref` / `threshold_blob_ref`. The
  vector form gives every aux artifact (scales, thresholds, sparse
  metadata) its own typed `kind`, `layout`, `format`, hash, and export
  provenance.
* `layer_norms` is a per-layer `LayerNorms` record naming the
  `pre_sequence` and `pre_ffn` `NormPlanId`s explicitly. This eliminates
  the positional-array indexing that `planv0.md` line 1576 hints at.
* `NormSite` makes every `NormPlanRecord` self-locating; the
  evaluator does not need positional indices.
* `ExpertSection.tensor_refs` is renamed from `weight_refs` because it
  also carries biases. `residency_hint` is removed — residency is
  storage/scheduling territory and conflicts with the storage-free
  boundary.
* `ModelSpecSummary` carries `vocab_size` and per-layer
  `ffn_activation_kind` so §8.5 SC-15..17 can be enforced without
  reaching back into `ArtifactCore`.
* `lowering_manifest_hash` is bound from Stage 0's
  `validated.input_hashes.lowering_manifest_hash` and re-recorded here
  for cache-key invalidation on lowering drift.

### 8.2 Operation contract

```text
operation build_quant_graph_core (pure)
  input:
    i: QuantGraphInputs

  modifies: nothing

  does_not_modify:
    artifact on disk
    ArtifactCore in memory
    CompileRequest
    ResolvedCompilePolicy
    RuntimeChromeBudget
    artifact_validation.json
    policy_resolution.json

  returns:
    Result[QuantGraphProduct, QuantGraphStageFailure]

operation run_stage1 (driver)
  input:
    i: QuantGraphInputs
    env: PassEnvironment (writeable JSON sink, writeable StageCache)

  effects:
    emits quant_graph.json
    may write StageCache success entry
    may write StageCache failure memo

  returns:
    Result[QuantGraphProduct, QuantGraphStageFailure]
    (the same Result returned by build_quant_graph_core)
```

Preconditions:

```text
QG-Pre-1:
  i.validated, i.policy must be products of F-B2 Stage 0 / Stage 0.5
  with both reports.outcome = Passed.

QG-Pre-2:
  i.artifact_core hash must match i.validated.input_hashes.artifact_effective_core_hash.

QG-Pre-2a:
  i.blob_index must contain a BlobMetadata entry for every BlobRef
  referenced by i.artifact_core.tensors and their aux artifacts. The
  driver (run_stage1) is responsible for building the index; the pure
  core fails fast on missing entries.

QG-Pre-2b:
  i.blob_index.self_hash must equal the blob-index hash committed by the
  Stage 0 validation product. If F-B2 does not yet expose that hash, this
  RFC amends the Stage 1 input contract by adding it.

QG-Pre-3:
  i.sequence_semantics must equal i.artifact_core.sequence (as referenced
  by content hash).

QG-Pre-4:
  i.policy.requested_runtime_modes must include at least one mode for which
  i.artifact_core.decode_caps.supported is non-empty.
```

Pass postconditions:

```text
QG-Ok-1:
  result = Ok(q) ⇒ q.report.outcome = Passed

QG-Ok-2:
  result = Ok(q) ⇒ q.report.body.result = Some(...)

QG-Ok-3:
  result = Ok(q) ⇒ q.report.diagnostics = []

QG-Ok-4:
  result = Ok(q) ⇒ q.quant_graph.identity.semantic_core_hash =
                   i.validated.input_hashes.artifact_effective_core_hash

QG-Ok-5:
  result = Ok(q) ⇒ ∀ tensor ref t ∈ q.quant_graph.tensors.
                     t.tensor_id ∈ q.quant_graph.provenance.keys

QG-Ok-6:
  result = Ok(q) ⇒ ∀ tensor ref t ∈ q.quant_graph.tensors.
                     q.quant_graph.provenance[t.tensor_id] is the
                     ExportTensorId originally exported for that tensor.

QG-Ok-7:
  result = Ok(q) ⇒ for every layer ℓ in i.artifact_core.model:
                     ModelSpec.layer(ℓ).ffn_kind = Routed
                       ⇒ ∃ entry in q.quant_graph.routing_table.layers
                         ∧ ∀ e ∈ {0..n_experts(ℓ)}.
                           ∃ section ∈ q.quant_graph.expert_sections
                             with section.layer = ℓ ∧ section.expert = e
                     ModelSpec.layer(ℓ).ffn_kind = Dense
                       ⇒ no routing_table entry for ℓ
                         ∧ exactly one expert_section with
                           section.layer = ℓ ∧ section.expert = 0

QG-Ok-8:
  result = Ok(q) ⇒ q.quant_graph.classify_head is well-formed:
                     kind = Tied  ⇒ classify_head.weight = embedding_weight tensor_id
                     kind = Untied ⇒ classify_head.weight ≠ embedding_weight tensor_id

QG-Ok-9:
  result = Ok(q) ⇒ all referenced BlobRefs resolve through i.blob_index.

QG-Ok-10:
  result = Ok(q) ⇒ if q.quant_graph.identity.determinism = BitExact
                   then i.policy.numeric_profile.reduction_order_policy = Enforced
                   ∧ no NormPlan or WeightEncoding requires mid-reduction
                     saturation/clipping.
```

Failure postconditions:

```text
QG-Err-1:
  result = Err(e) ⇒ e.report.outcome = Failed

QG-Err-2:
  result = Err(e) ⇒ e.report.body.result = None

QG-Err-3:
  result = Err(e) ⇒ e.diagnostics is non-empty
                  ∧ ∀ d ∈ e.diagnostics. d.severity = Hard

QG-Err-4:
  result = Err(e) ⇒ Stage2 and Stage3 do not run.

QG-Err-5:
  result = Err(e) ⇒ no QuantGraph product is exposed.
```

### 8.3 Construction order

QuantGraph construction is staged into named binding classes. Each class
runs in numeric order; within a class, all admissible diagnostics are
collected.

```text
QGClass :=
  1 IdentityBinding
       binds artifact_core_hash, semantic_core_hash, lowering_manifest_hash,
       determinism, model_spec_summary
  2 SequenceSemanticsBinding
       binds SequenceSemanticsSpec from i.artifact_core.sequence
       and validates that state_slots declarations are well-formed
  3 NormPlanIdPreBinding
       allocates stable NormPlanId values from i.artifact_core's norm-site
       declarations, sorted by NormSite (LayerSequence{0..n-1}, LayerFfn{0..n-1},
       Final). This creates only the ids, not the full NormPlanRecord
       bodies — but it lets TensorBinding (next) reference NormPlanId in
       NormScale / NormBias roles without forward-declaration.
  4 TensorBinding
       binds tensor_id, layout, quant_format, role, blob refs, aux_blob_refs
       for every canonical tensor in i.artifact_core.tensors
       (consumes SequenceSemanticsSpec to validate sequence-state-related
        tensor shapes; consumes NormPlanIdPreBinding so NormScale/NormBias
        roles can reference stable NormPlanIds)
  5 NormPlanBinding
       binds NormPlanRecord instances with NormSite, input/output formats;
       cross-references NormScale / NormBias tensors
  6 LayerNormsBinding
       binds the BTreeMap<LayerId, LayerNorms> from per-layer
       NormPlanRecord sites (LayerSequence / LayerFfn);
       Final remains a standalone NormPlanRecord site (not in layer_norms);
       asserts every layer has both pre_sequence and pre_ffn populated
  7 RoutingBinding
       binds RoutingTable iff at least one layer has FfnKind = Routed;
       cross-references RouterWeight / RouterBias tensors
  8 ExpertBinding
       binds ExpertSection per (layer, expert); cross-references
       ExpertWeight / ExpertBias tensors with ExpertWeightSlot;
       enforces FfnGate presence ⇔ ffn_activation_kind = SwiGLU
  9 ResidualPlanBinding
       binds ResidualPlan from ArtifactCore (activation_format and
       combine_policy); v1 enforces combine_policy =
       AddThenClampNamedBoundary
  10 DecodeBinding
       binds DecodeSpecRecord from an explicitly resolved policy choice
       OR from artifact_core.decode_caps.default if and only if that
       default is explicitly present and hash-bound in ArtifactCore;
       fails QuantGraphDecodeSpecNotInCapabilitySet otherwise
  11 ClassifyHeadBinding
       binds ClassifyHead; if kind = Tied, asserts weight equality with
       EmbeddingTable and asserts output_format = embedding_table.quant_format
  12 ProvenanceBinding
       builds TensorProvenanceMap; verifies every TensorId has an
       ExportTensorId; verifies QuantAuxBlobRef.export_tensor_id values
       are also injective
  13 CanonicalSort
       canonicalizes BTreeMap / BTreeSet ordering for
       layer_norms, expert_sections (by (LayerId, ExpertId)), tensors
       (by TensorId), routing_table.layers (by LayerId), provenance
       (by TensorId), so that downstream hashing is deterministic
  14 SelfConsistency
       cross-class checks (§8.5)
```

Ordering laws:

```text
QG-Order:
  Classes run in numeric order.

QG-Accumulate:
  Within a class, collect all diagnostics that can be safely produced.

QG-ShortCircuit:
  A later class is skipped iff its inputs were invalidated by a failed
  earlier class.

QG-NoSilentDefaults:
  Every QG field must be derived from i.artifact_core or fail loudly.
  Default values that mask missing input are forbidden.
```

### 8.4 Quant format set

```text
QuantFormat :=
  Ternary2 { scale_granularity, scale_format, threshold_granularity }
  | Binary1  { scale_granularity, scale_format }
  | SparseTernaryBitplanes { scale_granularity, scale_format, sparse_meta_kind }
  | Q8_8
  | Q4_4
  | I8
  | I16
```

`QuantFormat` payloads describe interpretation and granularity only. Large
per-channel/per-block scale, threshold, and sparse metadata payloads live
in `QuantAuxBlobRef`; they are **not** inline in `QuantFormat`.

Activation formats are limited to `{ I8, I16, Q8_8, Q4_4 }`. Weight formats
are limited to `{ Ternary2, Binary1, SparseTernaryBitplanes, Q8_8, I8 }`.
Cross-product validity is encoded in `QuantTensorRole::role-format-allowed`
predicate (§8.6).

### 8.5 Self-consistency rules

```text
QG-SC-1:
  Every TensorId is unique within q.quant_graph.tensors.

QG-SC-2:
  Every ExportTensorId is unique across TensorProvenanceMap and all
  QuantAuxBlobRef.export_tensor_id values.

QG-SC-3:
  Every NormPlanId referenced by a tensor's role appears in q.norm_plans.
  Every NormPlanId referenced in q.layer_norms (pre_sequence, pre_ffn)
  also appears in q.norm_plans.

QG-SC-4:
  If at least one layer is routed, q.routing_table = Some(_); otherwise
  q.routing_table = None.
  For every RouterLayer r:
    r.layer exists in model_spec_summary
    ∧ r.n_experts = model_spec_summary.n_experts(r.layer)
    ∧ for every expert e where 0 ≤ e < r.n_experts:
        exactly one ExpertSection exists with
        section.layer = r.layer ∧ section.expert = e.

QG-SC-5:
  Every expert_section.tensor_refs entry references a TensorId that
  exists in q.tensors and has role ExpertWeight or ExpertBias for the
  same (layer, expert).

QG-SC-6:
  expert_section.tensor_refs ordering matches the ExpertWeightSlot
  declaration order (FfnGate? then FfnUp then FfnDown) when present.
  Bias TensorIds, if present, immediately follow their corresponding
  weight in this order.

QG-SC-7:
  classify_head.weight references a TensorId with role ClassifyWeight
  (when Untied) or EmbeddingTable (when Tied).

QG-SC-8:
  decode_spec.spec ∈ artifact_core.decode_caps.supported.

QG-SC-9:
  Every CanonicalTensorLayout is consistent with model_spec_summary
  (e.g. embedding has shape [vocab, d_model]; expert_up has shape
  [d_ff, d_model]; expert_down has shape [d_model, d_ff]; classify
  has shape [vocab, d_model]; norm scale has shape [d_model]).

QG-SC-10:
  No tensor has role TrainingOnly. (See §2.16.)

QG-SC-11:
  Sequence-state shape declarations live in
  SequenceSemanticsSpec.state_slots, not in QuantTensorRole. KV slabs and
  recurrent state vectors are runtime state templates and have no BlobRef
  in the artifact.
    When sequence_semantics = LinearState(_):
      no per-token K/V cache tensors appear in q.tensors.
    When sequence_semantics = BoundedKv(_):
      SequenceSemanticsSpec.state_slots must declare the KV slab shapes
      required by the BoundedKv contract; q.tensors carries no entries
      for those slabs.

QG-SC-12:
  i.policy.objective.requires_features ⊆ artifact features supported by q.

QG-SC-13:
  All QuantTensorRef.blob entries are copied from i.blob_index and their
  content_hash / codec / encoded_size_bytes / decoded_size_bytes match
  the resolved metadata.

  For each tensor:
    decoded_size_bytes =
      expected_decoded_tensor_payload_size(layout, quant_format, role)

  For each aux blob:
    decoded_size_bytes =
      expected_decoded_aux_payload_size(layout, format, kind)

  CanonicalTensorLayout.size_bytes, when present, denotes logical dense
  element storage and is **not** used to validate packed
  ternary/binary/sparse blob payload sizes.

QG-SC-14:
  Exactly one tensor in q.tensors has role EmbeddingTable.

QG-SC-15:
  classify_head.kind = Tied
    ⇒ classify_head.weight = embedding_table.tensor_id.

QG-SC-15a:
  classify_head.logit_format ∈ { I8, I16, Q8_8, Q4_4 }.

QG-SC-16:
  ModelSpecSummary contains:
    vocab_size: u32
    per-layer ffn_kind ∈ { Routed, Dense }

QG-SC-16a:
  q.ffn_plans.keys() = { 0..n_layers - 1 } exactly.
  For every layer ℓ:
    q.ffn_plans[ℓ].activation_kind ∈ { Relu, Gelu, SiLU, SwiGLU }
    ∧ q.ffn_plans[ℓ].intermediate_format ∈ { I8, I16, Q8_8, Q4_4 }.

QG-SC-17:
  For every layer ℓ and every ExpertSection in that layer:
    q.ffn_plans[ℓ].activation_kind = SwiGLU
      ⇔ section has exactly one FfnGate weight (and optional
         matching FfnGate bias).
    q.ffn_plans[ℓ].activation_kind ≠ SwiGLU
      ⇔ section has no FfnGate weight.

QG-SC-18:
  q.layer_norms.keys() = { 0..n_layers - 1 } exactly. No layer is missing
  pre_sequence or pre_ffn norms.

QG-SC-19:
  No QuantTensorRef has role TrainingOnly residue (see §2.16) AND no
  QuantTensorRef carries any field whose semantics imply storage,
  lifetime, alias, page, accumulator width, or tile size (see §2.3).

QG-SC-20:
  NormPlanRecord.site is unique across q.norm_plans.
  Exactly one NormPlanRecord has site = Final.
  For every layer ℓ:
    q.layer_norms[ℓ].pre_sequence resolves to the unique
      NormPlanRecord with site = LayerSequence { layer: ℓ }
    ∧ q.layer_norms[ℓ].pre_ffn resolves to the unique
      NormPlanRecord with site = LayerFfn { layer: ℓ }.

QG-SC-21:
  For every QuantTensorRef t, t.aux_blob_refs is consistent with
  t.quant_format:
    Ternary2 requires exactly one Scale aux ref and exactly one
      Threshold aux ref.
    Binary1 requires exactly one Scale aux ref and no Threshold aux ref.
    SparseTernaryBitplanes requires exactly one Scale aux ref and
      exactly one SparseMeta aux ref.
    Dense integer formats (Q8_8 / Q4_4 / I8 / I16) require no Threshold
      or SparseMeta aux refs unless explicitly required by their
      QuantFormat payload (none in v1).
  No QuantAuxKind is duplicated for the same tensor unless the
  QuantFormat payload explicitly permits multiple aux refs of that kind.

QG-SC-22:
  decode_spec.requires_rng = decode_spec.spec.requires_rng().

QG-SC-23:
  q.residual_plan.combine_policy = AddThenClampNamedBoundary
  ∧ q.residual_plan.activation_format ∈ { I8, I16, Q8_8, Q4_4 }.
```

### 8.6 QuantTensorRole role-format predicate

```text
allowed(role, format) :=
  match role:
    EmbeddingTable      => format ∈ { I8, Q8_8 }
    NormScale           => format ∈ { Q8_8, Q4_4, I16 }
    NormBias            => format ∈ { Q8_8, Q4_4, I16 }
    RouterWeight        => format ∈ { Q8_8, I8 }
    RouterBias          => format ∈ { Q8_8, I8, I16 }
    ExpertWeight {slot} => format ∈ { Ternary2, Binary1, SparseTernaryBitplanes }
    ExpertBias          => format ∈ { Q8_8, I8, I16 }
    ClassifyWeight      => format ∈ { I8, Q8_8 }
    ClassifyBias        => format ∈ { Q8_8, I8, I16 }
```

A tensor with role/format outside `allowed` triggers
`QuantGraphRoleFormatMismatch`.

### 8.7 Canonical reference semantics for QuantGraph

For F-C2 (`ArtifactOracle`) op-for-op correspondence, this RFC pins the
canonical reference semantics for evaluating a `QuantGraph`. Let
`t : TokenInput`, `s : SequenceState`, `g : RngState`. Norm site
resolution uses the named lookup `q.norm_plan(site)` which returns the
`NormPlanRecord` whose `site` field equals the requested `NormSite`.
Then:

```text
eval_canonical_qg(QuantGraph q, t, s, g) :=
  e_0 = embed(t, q.embedding_table)                   -- one Embedding op
  for layer ℓ in 0..n_layers:
    a = norm(e_ℓ, q.norm_plan(LayerSequence{ℓ}))
    b = sequence_block(a, s, q, ℓ)                    -- defined by SequenceSemanticsSpec
    e_ℓ' = combine_residual(e_ℓ, b, q.residual_plan)  -- post-sequence residual
                                                      -- with named clamp boundary
    f = norm(e_ℓ', q.norm_plan(LayerFfn{ℓ}))          -- pre-FFN norm
    if ffn_kind(ℓ) = Routed:
      scores = router_matvec(f, q.routing_table.layers[ℓ])
      normalized_scores = router_score_normalize(scores, q.routing_table.layers[ℓ])
      (top1, weight) = route_top1(normalized_scores, q.routing_table.layers[ℓ])
        -- top1 = argmax(normalized_scores) with ties broken by
        --        RouterTieBreak (v1 requires LowestExpertId).
        -- weight is determined by RouterGateWeightSemantics:
        --   One           ⇒ weight = 1.0
        --   SelectedScore ⇒ weight = normalized_scores[top1]
      candidates = [ expert(f, q.expert_sections[ℓ, e]) for e in 0..n_experts(ℓ) ]
      selected_candidate = candidates[top1]
      m = weight * selected_candidate                 -- gate-weight applied
    else:
      // Dense FFN: mathematically equivalent to routed-with-prob-1.0,
      // but the IR shape is direct (no router or selection nodes).
      m = expert(f, q.expert_sections[ℓ, 0])
    e_{ℓ+1} = combine_residual(e_ℓ', m, q.residual_plan)
                                                      -- post-FFN residual
                                                      -- with named clamp boundary
  z   = norm(e_n, q.norm_plan(Final))
  log = classify(z, q.classify_head)
        -- exact projection plus optional bias, then clamp/round at the
        -- named ClassifyLogitBoundary to q.classify_head.logit_format
  tok = decode(log, q.decode_spec, g)
  return (tok, s', g')
```

`combine_residual(x, δ, residual_plan)` adds in `ExactAccumulator` and
then applies `residual_plan.combine_policy` (clamp/round to
`activation_format`) at a **named numeric boundary**. This is the only
place `BitExact` permits saturation; mid-reduction clipping remains
forbidden (§2.10).

Each operator (`embed`, `norm`, `sequence_block`, `router_decision`,
`router_weight`, `expert`, `classify`, `decode`) has exact canonical
semantics defined per `NormPlan` variant, per `WeightEncoding`, per
`RouterSemantics`, per `DecodeSpec`, respectively. The semantics are
inherited from the definitions in `gbf-artifact` types and are not
re-stated here; this RFC pins the *order*, *typing*, *site naming*, and
*gate-weight consumption*, not the per-op math.

`expert(f, section)` is the composition of the FFN weight slots:

```text
expert(f, section) :=
  match ffn_activation_kind(section.layer):
    SwiGLU:
      gate    = matvec(f, section.weight[FfnGate])
      up      = matvec(f, section.weight[FfnUp])
      h       = swiglu_activation(gate, up)
      candidate = matvec(h, section.weight[FfnDown])
    Relu | Gelu | SiLU:
      up      = matvec(f, section.weight[FfnUp])
      h       = activation(up)
      candidate = matvec(h, section.weight[FfnDown])
  return candidate
```

Equality is defined only for `BitExact`:

```text
If q.identity.determinism = BitExact, eval_canonical_qg is required to
be reproducible bit-for-bit when:
  - the resolved reduction-order policy is Enforced, and
  - intermediate saturation is forbidden except at named boundaries
    (residual combine; final activation clamp).

For weaker DeterminismClass values, this RFC records the class but does
not define equality. F-C2 (ArtifactOracle) and F-C4 (ConformanceEnvelope)
own class-relative conformance.

The chunk closure gate verifies FixtureSemanticEquivalence on a
synthetic fixture input set; UniversalSemanticEquivalence is deferred
to F-C2 / F-C4.
```

### 8.8 F-B4 placeholder retirement (binding to Stage 2)

This chunk includes the migration of F-B4's placeholder
`QuantGraphBudgetSource` to the real `QuantGraph` view:

```text
QuantGraphProduct implements QuantGraphBudgetSource:
  fn quant_graph_hash() -> Hash256
    = self.quant_graph_self_hash
    -- where quant_graph_self_hash =
    --   DomainHash("gbf-codegen", "QuantGraph", "quant_graph.v1",
    --              CanonicalJson(self.quant_graph))
    --
    -- QuantGraph (the IR struct) does not contain quant_graph_self_hash;
    -- therefore no self-hash zeroing is needed for the product hash.
    -- The hash lives on QuantGraphProduct and in the product-bearing
    -- report's result.quant_graph_self_hash field.
  fn semantic_core_hash() -> Hash256
    = self.quant_graph.identity.semantic_core_hash
  fn to_budget_view() -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError>
    = derived from self.quant_graph.tensors,
      self.quant_graph.routing_table,
      self.quant_graph.expert_sections,
      self.quant_graph.classify_head,
      self.quant_graph.norm_plans,
      self.quant_graph.ffn_plans,
      and self.quant_graph.residual_plan
```

The existing F-B4 fixtures (T-B4.* fixtures under `fixtures/static_budget/`)
must continue to pass after the placeholder is retired. The placeholder
trait stub is preserved only for unit tests of F-B4 internals.

## 9. Stage 3 contract: `GbInferIR`

### 9.1 Type-level contract

```text
GbInferIRInputs :=
  {
    quant_graph: QuantGraph,                     -- from F-B3
    quant_graph_self_hash: Hash256,
    policy_projection: InferIrPolicyProjection,  -- minimal IR-shape-bearing projection
    audit_parents: InferIrAuditParents,          -- report/audit only; not IR-shape-bearing
    static_budget: StaticBudgetProduct,          -- from F-B4 (typed product, not hash alone)
    static_budget_self_hash: Hash256,            -- equal to static_budget.self_hash
  }

InferIrPolicyProjection :=
  {
    requested_runtime_modes: SortedSet<RuntimeMode>,
  }

InferIrAuditParents :=
  {
    policy_resolution_self_hash: Hash256,
    compile_request_hash: Hash256,
  }

infer_ir_policy_projection_hash :=
  DomainHash("gbf-codegen", "InferIrPolicyProjection", "infer_ir.v1",
    CanonicalJson(InferIrPolicyProjection))

-- The projection is the *only* policy surface that load-bears the IR
-- shape and the Stage 3 cache key. Audit parents are recorded in the
-- report identity for traceability; they never invalidate K3.

GbInferIRProduct :=
  {
    infer_ir: GbInferIR,
    report: Report[InferIrReportBody],
    infer_ir_self_hash: Hash256,
    infer_ir_canonical_bytes_hash: Hash256
  }

infer_ir_self_hash :=
  DomainHash("gbf-codegen", "GbInferIR", "infer_ir.v1",
    CanonicalJson(infer_ir))

-- The raw GbInferIR struct does not contain infer_ir_self_hash; therefore
-- no self-hash zeroing is needed for the product hash.

GbInferIRStageFailure :=
  {
    report: Report[InferIrReportBody],
    diagnostics: NonEmptyList[ValidationDiagnostic]
  }
```

The public `GbInferIR` type:

```rust
pub struct GbInferIR {
    pub identity: InferIrIdentity,
    pub token_inputs: Vec<TokenInput>,            // exactly one in v1
    pub nodes: Vec<GbNode>,
    pub values: Vec<ValueDecl>,
    pub effects: Vec<EffectDecl>,
    pub provenance: InferIrProvenance,
    pub anchors: NodeAnchorMap,                   // for F-B6 (serialized)
}

pub struct InferIrIdentity {
    pub quant_graph_self_hash: Hash256,
    pub infer_ir_policy_projection_hash: Hash256,
    pub static_budget_self_hash: Hash256,
    pub requested_runtime_modes_hash: Hash256,
    pub determinism: DeterminismClass,
    pub topological_order_hash: Hash256,
}
// Note: policy_resolution_self_hash and compile_request_hash live in
// InferIrReportBody.input_identity (audit parents), not in
// InferIrIdentity. The IR product is invariant under audit-parent drift.

pub struct GbNode {
    pub node_id: NodeId,
    pub op: InferOp,
    pub inputs: Vec<ValueId>,
    pub effects_in: Vec<EffectId>,
    pub outputs: Vec<ValueId>,
    pub effects_out: Vec<EffectId>,
    /// Set when the op corresponds to a Stage 2 reduction site
    /// (ExpertMatVec, Norm with TileRms*, Classify). F-B7 (RangePlan)
    /// uses this to correlate with StaticBudgetReport's
    /// ReductionSiteProjection. Other ops have None.
    pub reduction_site: Option<ReductionSiteId>,
}

pub struct ValueDecl {
    pub value_id: ValueId,
    pub kind: ValueKind,
    pub format: ValueFormat,
    pub layout: ValueLayout,                      // logical shape only
}
// Provenance lives on g.provenance.values (InferIrProvenance), not on
// ValueDecl. ValueProvenance was removed in this revision (see A74).

/// IR-level value formats. Distinct from QuantFormat: artifact tensors
/// have QuantFormat; intermediate IR values may also be exact accumulators
/// or values inhabiting semantic domains whose physical width is selected
/// later. F-B5 carries domains, not widths; widths are storage decisions
/// owned by F-B7+.
pub enum ValueFormat {
    Quant(QuantFormat),
    /// Logical accumulator value with no implementation width chosen yet.
    /// F-B7 (RangePlan) selects an admissible implementation
    /// (SingleI16 / ChunkedI16 / RenormLoop) preserving canonical numeric
    /// semantics.
    ExactAccumulator,
    TokenIdDomain  { vocab_size: u32 },
    ExpertIdDomain { n_experts: u16 },
}

pub enum ValueKind {
    InputToken,                                   // external token id (one per IR pass)
    Activation,                                   // hidden state, residual stream
    NormalizedActivation,                         // norm output
    EmbeddingOutput,                              // embedding lookup result
    SequenceStateRead,                            // value bound from a SequenceRead
    SequenceStateNext,                            // next-state value (pre-SequenceWrite)
    SequenceBlockOutput,                          // block output from SequenceStep
    RouterScore,                                  // pre-decision router scores
    RouterDecision,                               // selected expert id (top-1)
    GateWeight,                                   // scalar gate from RouteTop1
    ExpertIntermediate,                           // FfnGate / FfnUp / activation result
    ExpertCandidate,                              // post-FfnDown expert candidate
    ExpertOutput,                                 // selected and gate-weighted candidate
    LogitVector,                                  // pre-decode logits
    DecodedToken,                                 // single token id
}

pub struct EffectDecl {
    pub effect_id: EffectId,
    pub class: EffectClass,
}
// Provenance lives on g.provenance.effects (InferIrProvenance), not on
// EffectDecl. The EffectProvenance enum below is the value type stored
// in that provenance map.

/// EffectProvenance distinguishes external-root edge tokens (allocated at
/// IR-pass entry as initial state for each chain) from node-produced edge
/// tokens.
pub enum EffectProvenance {
    ExternalRoot { class: EffectClass },
    NodeOutput   { node: NodeId, class: EffectClass },
}

/// Effect classes are closed in v1. EffectId values are edge tokens
/// (per-edge, not per-class-instance): each effectful node consumes one
/// token of class c and produces a fresh token of the same class.
pub enum EffectClass {
    SequenceState(StateSlotId),
    Rng(RngSlot),
    FaultBoundary,                                // reserved; not emitted in v1
}

pub enum InferOp {
    Embedding        { token_input: TokenInputId },
    SequenceRead     { slot: StateSlotId },
    SequenceStep     { layer: LayerId },
    SequenceWrite    { slot: StateSlotId },
    RouterMatVec     { layer: LayerId },
    RouteTop1        { layer: LayerId },
    SelectExpertTop1 { layer: LayerId },
    ExpertMatVec     { layer: LayerId, expert: ExpertId, slot: ExpertWeightSlot },
    FfnActivation    { layer: LayerId, expert: ExpertId },
    CombineResidual  { layer: Option<LayerId>, site: ResidualSite },
    Norm             { plan: NormPlanId },
    Classify,
    DecodeToken      { plan: DecodePlanId },
}

pub enum ResidualSite {
    PostSequence,
    PostFfn,
}

/// Provenance lives on the IR product, not on each node (Codex: GbNode
/// must not carry an inline provenance field that postconditions
/// reference).
pub struct InferIrProvenance {
    pub nodes:   BTreeMap<NodeId, QuantGraphEntityRef>,
    pub values:  BTreeMap<ValueId, ValueProducerRef>,
    pub effects: BTreeMap<EffectId, EffectProvenance>,
}

pub enum QuantGraphEntityRef {
    Embedding,
    NormPlan(NormPlanId),
    NormSite(NormSite),
    RouterLayer(LayerId),
    RouterTensor    { layer: LayerId, tensor: TensorId },
    RouterSelection { layer: LayerId },
    ExpertSection   { layer: LayerId, expert: ExpertId },
    ExpertTensor    { layer: LayerId, expert: ExpertId,
                      slot: ExpertWeightSlot, tensor: TensorId },
    FfnActivationSite { layer: LayerId, expert: ExpertId },
    ResidualSiteRef   { layer: Option<LayerId>, site: ResidualSite },
    DecodePlan(DecodePlanId),
    ClassifyHead,
    SequenceSlot(StateSlotId),
    SequenceStep { layer: LayerId },
    TokenInput(TokenInputId),
}

pub enum ValueProducerRef {
    Node(NodeId),
    External(TokenInputId),
}
```

Differences from `planv0.md` line 1603:

* `ExpertMatVec` adds `slot: ExpertWeightSlot`. Each per-expert weight
  matrix produces its own `ExpertMatVec` node so the IR is op-level (no
  fusion of Up + Down).
* `Norm` carries `NormPlanId` rather than the full `NormPlan` to keep
  nodes small; the plan is resolved through `QuantGraph.norm_plans`.
* `DecodeToken` carries `DecodePlanId` rather than the full `DecodeSpec`.
* `RouterMatVec` is added so router scoring is its own reduction-bearing
  op. Without it, `RouterScore` would be orphaned (no consumer), the
  router projection would be invisible to F-B7 (`RangePlan`), and
  `RouteTop1` would conflate matvec with selection.
* `RouteTop1` consumes `RouterScore` and produces `RouterDecision` and
  `GateWeight`. Selection is now its own op, not a fused matvec+select.
* `SelectExpertTop1` consumes the `RouterDecision`, `GateWeight`, and all
  `ExpertCandidate` values for a routed layer; without it, the gate
  weight is dropped and the MoE math is wrong.
* `FfnActivation` is added so the nonlinear FFN activation
  (SwiGLU/Gelu/Relu/SiLU) is an explicit op between `FfnUp` (and optional
  `FfnGate`) and `FfnDown`. Without it, `eval_canonical_ir` cannot match
  the FFN math in §8.7.
* `SequenceStep { layer }` (no `slot` parameter) is the value-level
  operation corresponding to `sequence_block(a, s, q, ℓ)` in §8.7. It
  consumes the activation plus all `SequenceStateRead` values for the
  layer's slots, and produces one `SequenceBlockOutput` plus one
  `SequenceStateNext` per slot. `SequenceRead` and `SequenceWrite`
  carry only the per-slot effect chain.
* `CombineResidual { layer, site: ResidualSite }` carries the residual
  site (`PostSequence` / `PostFfn`) and the owning layer (None for
  non-layer combines, which v1 does not emit). The site is necessary
  for op-signature validation and provenance correlation.
* `Embedding` consumes a `TokenInputId` referencing a `TokenInput` whose
  `value_id` is the unique external `InputToken` value (see §2.13).
* `GbNode.reduction_site: Option<ReductionSiteId>` is set on
  reduction-bearing ops so F-B7 (RangePlan) can correlate IR nodes with
  Stage 2's reduction sites.
* **Mixed topology** in v1 means some layers are dense and some are
  routed. It does **not** mean a routed layer has an additional shared
  dense branch. Shared dense branches are out of scope for v1 and would
  require an explicit RFC amendment to add a `SharedDenseMatVec` op and
  matching `QuantTensorRole` variants.

### 9.2 Operation contract

```text
operation build_infer_ir_core (pure)
  input:
    i: GbInferIRInputs

  modifies: nothing

  does_not_modify:
    QuantGraph
    ResolvedCompilePolicy
    StaticBudgetReport
    artifact_validation.json
    policy_resolution.json
    quant_graph.json

  returns:
    Result[GbInferIRProduct, GbInferIRStageFailure]

operation run_stage3 (driver)
  input:
    i: GbInferIRInputs
    env: PassEnvironment

  effects:
    emits infer_ir.json
    may write StageCache success entry
    may write StageCache failure memo

  returns:
    Result[GbInferIRProduct, GbInferIRStageFailure]
    (the same Result returned by build_infer_ir_core)
```

Preconditions:

```text
IIR-Pre-1:
  i.quant_graph_self_hash must match i.quant_graph's computed self-hash.

IIR-Pre-2:
  i.audit_parents.policy_resolution_self_hash must reference a Passed
  Stage 0.5 policy-resolution report.

IIR-Pre-3:
  i.static_budget_self_hash must equal i.static_budget.self_hash, and
  i.static_budget.decision.fits must be true.

IIR-Pre-4:
  i.policy_projection.requested_runtime_modes must be non-empty.
```

Pass postconditions:

```text
IIR-Ok-1:
  result = Ok(g) ⇒ g.report.outcome = Passed

IIR-Ok-2:
  result = Ok(g) ⇒ g.report.body.result = Some(...)

IIR-Ok-3:
  result = Ok(g) ⇒ g.report.diagnostics = []

IIR-Ok-4:
  result = Ok(g) ⇒ g.infer_ir.identity.quant_graph_self_hash =
                   i.quant_graph_self_hash

IIR-Ok-5:
  result = Ok(g) ⇒ g.infer_ir.nodes form a finite DAG over (values, effects).

IIR-Ok-6:
  result = Ok(g) ⇒ Single-token convention holds:
                     exactly one InferOp::Embedding node per pass
                     exactly one InferOp::DecodeToken node per pass
                     exactly one InferOp::Classify node per pass

IIR-Ok-7:
  result = Ok(g) ⇒ Effect-class linearity: §9.5.

IIR-Ok-8:
  result = Ok(g) ⇒ Provenance totality (provenance lives on g, not GbNode):
                     ∀ node n. g.provenance.nodes[n.node_id]   exists
                     ∀ value v. g.provenance.values[v.value_id] exists
                     ∀ effect e. g.provenance.effects[e.effect_id] exists

IIR-Ok-9:
  result = Ok(g) ⇒ For every TensorId t referenced by some ExpertSection
                     with role ExpertWeight { layer, expert, slot }:
                     exactly one InferOp::ExpertMatVec
                       { layer, expert, slot }
                     node exists in g.nodes.

IIR-Ok-10:
  result = Ok(g) ⇒ For every layer with FfnKind = Routed:
                     exactly one InferOp::RouteTop1 node exists for that layer
                     ∧ exactly one InferOp::SelectExpertTop1 node exists
                       that consumes the RouterDecision, GateWeight, and
                       all ExpertCandidate values for that layer.

IIR-Ok-11:
  result = Ok(g) ⇒
    if g.report.body.result.fixture_equivalence = VerifiedFixtureBitExact:
      FixtureSemanticEquivalence(g, q) holds under bit-exact canonical
      reference semantics on the fixture input set.
      Implies q.identity.determinism = BitExact.

    if g.report.body.result.fixture_equivalence = Skipped { reason }:
      reason ∈ { NonFixtureBuild, FeatureFlagDisabled,
                 NonBitExactDeterminism }
      and no numeric equivalence claim is made by this chunk.

  The chunk-closure fixture build must produce VerifiedFixtureBitExact for
  the BitExact dense and routed fixtures.

IIR-Ok-12:
  result = Ok(g) ⇒ Topological order is canonical: §9.4.

IIR-Ok-13:
  result = Ok(g) ⇒ No SemanticCheckpointId, TraceProbeId, or
                     SemanticCheckpoint effect class appears anywhere in g.

IIR-Ok-14:
  result = Ok(g) ⇒ Every value with kind GateWeight is consumed by exactly
                     one InferOp::SelectExpertTop1 node.

IIR-Ok-15:
  result = Ok(g) ⇒ Every value with kind RouterDecision is consumed by
                     exactly one InferOp::SelectExpertTop1 node.

IIR-Ok-16:
  result = Ok(g) ⇒ Every value v with kind ∈ { ExpertOutput,
                     EmbeddingOutput, NormalizedActivation, ... non-terminal }
                     is consumed by at least one later node.
                     The only terminal value kind is DecodedToken.

IIR-Ok-17:
  result = Ok(g) ⇒ For every routed-FFN layer:
                     exactly one FfnActivation { layer, expert } node
                     exists per expert. Its inputs are the FfnUp
                     (and FfnGate when SwiGLU) outputs of that expert.

IIR-Ok-18:
  result = Ok(g) ⇒ Every node n where ReductionSiteBearing(n.op, q) holds
                     has Some(reduction_site: ReductionSiteId) that
                     matches a site evaluated in StaticBudgetReport.
                     Every other node has reduction_site = None.
```

Failure postconditions:

```text
IIR-Err-1:
  result = Err(e) ⇒ e.report.outcome = Failed
                  ∧ e.report.body.result = None
                  ∧ ∀ d ∈ e.diagnostics. d.severity = Hard
                  ∧ no GbInferIR product is exposed.
```

### 9.3 Construction order

```text
IIRClass :=
  1 IdentityBinding
       binds quant_graph_self_hash, policy_resolution_self_hash,
       static_budget_self_hash, requested_runtime_modes_hash, determinism
  2 TokenInputBinding
       declares the single TokenInput value head and its
       allowed_ingress_modes set
  3 ValueAllocation
       reserves ValueIds for embedding output, residual stream, norm outputs,
       sequence-block outputs, router scores, router decisions, gate weights,
       expert intermediates, expert candidates, expert outputs, logits,
       decoded token
  4 EffectAllocation
       allocates effect-edge tokens (NOT one id per class).
       For each effect class c, the chain is:
         root(c) -> e1(c) -> e2(c) -> ... -> final(c)
       where each effectful node consumes exactly one EffectId of class c
       and produces a fresh EffectId of the same class. Roots are external
       inputs at IR-pass entry.
       v1 emits no SequenceState chains (per F-B5-SequenceV1, §2.5a);
       non-empty sequence-state slots are rejected by Stage 3.
       v1 emits Rng(Decode) iff DecodeSpecRecord.requires_rng = true.
       FaultBoundary is reserved but never emitted in v1.
  5 NodeBuilding
       constructs GbNodes in canonical topological order (§9.4).
       v1 sequence block is identity (§2.5a, F-B5-SequenceV1), so no
       SequenceRead / SequenceStep / SequenceWrite nodes are emitted;
       the post-sequence residual collapses to identity:
       Embedding -> per-layer (NormPreSequence ->
                                CombineResidual{site=PostSequence} with
                                  identity sequence delta ->
                                NormPreFFN ->
                                if Routed:
                                  RouterMatVec -> RouteTop1 ->
                                  Experts(FfnGate?, FfnUp,
                                          FfnActivation, FfnDown)
                                  -> SelectExpertTop1
                                if Dense:
                                  Experts(FfnGate?, FfnUp,
                                          FfnActivation, FfnDown)
                                       (expert == 0; no router/select nodes)
                                -> CombineResidual{site=PostFfn})
                 -> NormFinal -> Classify -> DecodeToken
  6 ReductionSiteBinding
       sets GbNode.reduction_site = Some(...) for every node satisfying
       ReductionSiteBearing(op, q) (i.e. RouterMatVec / ExpertMatVec /
       Norm{plan ∈ TileRms*} / Classify); other nodes carry None. The
       Some(ReductionSiteId) values match Stage 2's ReductionSiteProjection.
  7 ProvenanceBinding
       fills g.provenance.{nodes, values, effects} maps
  8 AnchorBinding
       constructs NodeAnchorMap with serialized SemanticAnchor ids
       (see §2.12); each anchor =
         DomainHash("gbf-codegen", "SemanticAnchor", "v1",
           CanonicalJson({ quant_graph_self_hash, node_id, op_tag,
                           canonical_provenance_tuple }))
  9 CanonicalSort
       canonicalizes BTreeMap / BTreeSet / Vec ordering
       (provenance.nodes by NodeId, provenance.values by ValueId,
        anchors by NodeId, etc.) before hashing
  10 SelfConsistency
       cross-class checks (§9.6)
  11 SemanticEquivalenceCheck
       optional in v1: assert g semantically equivalent to q on the
       synthetic reference token under canonical reference semantics.
       Required for chunk closure (the fixture build); feature-gated
       (cfg(feature = "semantic_equivalence_check")) for non-fixture
       builds because the reference evaluator is expensive.
       Skipped automatically when q.identity.determinism != BitExact;
       result is Skipped { reason: NonBitExactDeterminism }.
```

`SemanticEquivalenceCheck` is required for the synthetic dense fixture in
`fixtures/quant_graph/`; it is feature-gated for non-fixture builds because
the reference evaluator is expensive.

### 9.4 Canonical topological order

Per `IIR-Ok-12`, `g.nodes` is sorted by canonical topological order
**before** `NodeId` is assigned (assignment order would otherwise be
circular):

```text
canonical_order(node) :=
  primary key    =
                   Embedding    = -1
                   Layer ℓ      = ℓ where 0 ≤ ℓ < n_layers
                   FinalNorm    = n_layers
                   Classify     = n_layers + 1
                   Decode       = n_layers + 2
  secondary key  = sub-stage within layer:
                     0:  NormPreSequence
                     1:  SequenceRead               (per state slot; reserved v1)
                     2:  SequenceStep               (one per layer; reserved v1)
                     3:  SequenceWrite              (per state slot; reserved v1)
                     4:  CombineResidual{PostSequence}
                     5:  NormPreFFN
                     6:  RouterMatVec               (routed only)
                     7:  RouteTop1                  (routed only)
                     8:  ExpertMatVec(FfnGate)      (per expert)
                     9:  ExpertMatVec(FfnUp)        (per expert)
                     10: FfnActivation              (per expert)
                     11: ExpertMatVec(FfnDown)      (per expert)
                     12: SelectExpertTop1           (routed only)
                     13: CombineResidual{PostFfn}
                   For FinalNorm / Classify / Decode, secondary key = 0.
  tertiary key   = expert_id (only when sub-stage involves an expert);
                   state_slot_id (only for SequenceRead / SequenceWrite,
                                  reserved v1)
  quaternary key = canonical provenance tuple:
                     (op_tag, layer?, expert?, slot?, norm_site?,
                      state_slot?, residual_site?, occurrence_index)

NodeId is assigned to the canonically-sorted sequence after this order
is established; node_id is therefore stable across regenerations.
```

`topological_order_hash` records:

```text
topological_order_hash =
  DomainHash("gbf-codegen", "InferIrTopologicalOrder", "infer_ir.v1",
    CanonicalJson(
      [ (node_id, op_tag, canonical_provenance_tuple)
        for each node in canonical order ]))
```

Any reordering changes the hash. Using the domain hash convention keeps
this consistent with the F-B2/F-B4 inheritance (§2.2).

### 9.5 Effect linearization

EffectIds are **edge tokens**, not class instances. A single
`SequenceState(slot=A)` chain consists of a sequence of EffectIds
`[e₀, e₁, e₂, ...]` where each effectful node consumes one and produces
the next.

```text
For each effect-class instance c (e.g. SequenceState(slot=A), Rng(Decode)):
  Let E_c = list of nodes whose effects_in or effects_out reference some
            EffectId of class c, ordered by canonical topological order.
  E_c is a linear chain:
    For adjacent nodes i, j in E_c (j is the immediate successor of i):
      effects_out(i) contains exactly the EffectId consumed by effects_in(j).
    No EffectId of class c is produced by more than one node.
    No EffectId of class c is consumed by more than one node.

For two distinct effect-class instances c1 ≠ c2:
  Effect chains for c1 and c2 are independent and may interleave with
  the value-DAG ordering arbitrarily.

Specifically (v1):
  SequenceState(slot=A) and SequenceState(slot=B) for A ≠ B are
    independent chains (one per slot).
  Rng(Decode) is a single chain that is non-empty iff
    DecodeSpecRecord.requires_rng = true. When present, the unique
    DecodeToken node consumes one Rng(Decode) effect token (a root edge
    token) and produces the next one. Classify is pure with respect to
    RNG and does NOT touch the Rng(Decode) chain.
  FaultBoundary is reserved but not emitted in v1. Emitting FaultBoundary
    requires an explicit RFC amendment because it would expand the Stage 3
    policy projection.
```

Validation:

```text
F-B5-EffectChain:
  ∀ effect-class instance c.
    The set of nodes referencing class-c EffectIds forms a linear chain
    in canonical order.

F-B5-EffectIdEdgeTokenUnique:
  ∀ EffectId e.
    e is produced by at most one node and consumed by at most one node.
    If e has provenance ExternalRoot, it is produced by no node (its
    role is to seed the chain at IR-pass entry).
    Final effect tokens may be produced by one node and consumed by no
    node.

F-B5-EffectClassInstanceUniqueChain:
  ∀ effect-class instance c.
    There is at most one chain for c in g (no duplicate chains).
```

### 9.6 Self-consistency rules

```text
IIR-SC-1:
  ValueId, EffectId, NodeId are unique within g.

IIR-SC-2:
  Every input ValueId of every node is the output of some earlier node,
  except the unique InputToken ValueId, which is declared by the unique
  TokenInput and consumed by the unique Embedding node.

IIR-SC-3:
  Every non-root effects_in EffectId of every node is the effects_out
  EffectId of exactly one earlier node. Roots are external inputs at IR
  entry (one per declared SequenceState slot, plus Rng(Decode) when
  required).

IIR-SC-4:
  No cycles. (The IR is a DAG over values; effect chains are linear
  inside the DAG.)

IIR-SC-5:
  ValueDecl.format is consistent with the producing op's output value
  format predicate (§9.7).

IIR-SC-6:
  For every InferOp::Norm{plan}:
    QG.norm_plans contains exactly one NormPlanRecord with
    norm_plan_id = plan; that record's input_format equals the input
    ValueDecl.format-as-Quant; its output_format equals the output
    ValueDecl.format-as-Quant.

IIR-SC-7:
  For every InferOp::ExpertMatVec{layer, expert, slot}:
    QG.expert_sections has exactly one entry for (layer, expert)
    and that entry has a tensor_ref of role
    ExpertWeight {layer, expert, slot}.

IIR-SC-8:
  For every InferOp::RouteTop1{layer}:
    QG.routing_table.layers contains exactly one entry for layer
    and that entry's semantics is RouterSemantics::Top1Hard.

IIR-SC-9:
  For every layer ℓ where QG.model_spec.layer(ℓ).ffn_kind = Dense:
    no InferOp::RouterMatVec{layer = ℓ} exists.
    no InferOp::RouteTop1{layer = ℓ} exists.
    no InferOp::SelectExpertTop1{layer = ℓ} exists.
    exactly one InferOp::ExpertMatVec{layer = ℓ, expert = 0, slot = FfnUp} exists.
    exactly one InferOp::FfnActivation{layer = ℓ, expert = 0} exists.
    exactly one InferOp::ExpertMatVec{layer = ℓ, expert = 0, slot = FfnDown} exists.
    InferOp::ExpertMatVec{layer = ℓ, expert = 0, slot = FfnGate} exists iff
      ffn_activation_kind(ℓ) = SwiGLU.
  For every routed layer ℓ:
    exactly one InferOp::RouterMatVec{layer = ℓ} exists.
    exactly one InferOp::RouteTop1{layer = ℓ} exists.

IIR-SC-10:
  For every InferOp::DecodeToken{plan}:
    QG.decode_spec.decode_plan_id = plan.
    if QG.decode_spec.requires_rng then the Rng(Decode) chain is non-empty
    and exactly one DecodeToken node consumes one Rng(Decode) edge token
    and produces the next. Classify is pure with respect to Rng and
    does NOT touch the Rng(Decode) chain.

IIR-SC-10a:
  For every layer ℓ, SequenceRead and SequenceWrite nodes are emitted for
  exactly the StateSlotIds returned by
  SequenceSemanticsSpec.state_slots_for_layer(ℓ). Inputs to and outputs
  from SequenceStep{layer = ℓ} are sorted by StateSlotId; SequenceStep
  consumes one SequenceStateRead per slot and produces one
  SequenceStateNext per slot, plus exactly one SequenceBlockOutput.

IIR-SC-11:
  Every EffectDecl.class is one of the v1 EffectClass variants:
  SequenceState(_), Rng(_), or FaultBoundary. No SemanticCheckpointId or
  TraceProbeId appears anywhere in GbInferIR. (FaultBoundary is reserved
  but never emitted in v1.)

IIR-SC-12:
  Topological order matches §9.4 canonical order exactly.

IIR-SC-13 (reachability / non-orphan):
  Every non-terminal ValueId is consumed by at least one later node.
  Terminal values are exactly DecodedToken in v1; any other terminal
  requires a later RFC amendment.

IIR-SC-14:
  GbInferIR.token_inputs.len() = 1; the embedding op's token_input id
  equals the unique TokenInputId.

IIR-SC-14a:
  The unique Embedding node's sole input ValueId equals
  GbInferIR.token_inputs[0].value_id.

IIR-SC-15:
  ∀ value v with kind GateWeight:
    v is consumed by exactly one InferOp::SelectExpertTop1 node.

IIR-SC-16:
  ∀ value v with kind RouterDecision:
    v is consumed by exactly one InferOp::SelectExpertTop1 node.

IIR-SC-17 (op histogram):
  result.op_histogram[InferOpTag::X] equals the count of nodes whose
  op tag is X. The total over all tags equals result.node_count.
  Every InferOpTag appears exactly once as a key in the histogram,
  including tags with count 0 (e.g. SequenceRead / SequenceStep /
  SequenceWrite are present with count 0 in v1).

IIR-SC-18 (op signature):
  Every node n satisfies op_signature(n.op, q) — see §9.7a.
  This includes input arity, input ValueKind/format constraints,
  output arity, output ValueKind/format constraints, effect_in /
  effect_out classes, and reduction_site obligation.
```

### 9.7 Op output value format predicate

```text
output_format(op) :=
  match op:
    Embedding             => EmbeddingOutput in Quant(EmbeddingTable.format)
    SequenceRead          => SequenceStateRead in Quant(by SequenceSemanticsSpec)
    SequenceStep          => (SequenceBlockOutput in Quant(by SequenceSemanticsSpec),
                              [SequenceStateNext in Quant(by SequenceSemanticsSpec)
                               for each slot in state_slots_for_layer(layer)])
    SequenceWrite         => no value outputs (effect-only output)
    RouterMatVec          => RouterScore in ExactAccumulator
    RouteTop1             => (RouterDecision in ExpertIdDomain { n_experts },
                              GateWeight in Quant(Q8_8))
    SelectExpertTop1      => ExpertOutput in ExactAccumulator
    ExpertMatVec(slot ∈ {FfnGate, FfnUp})
                          => ExpertIntermediate in ExactAccumulator
    ExpertMatVec(slot = FfnDown)
                          => ExpertCandidate in ExactAccumulator
    FfnActivation         => ExpertIntermediate in
                             Quant(q.ffn_plans[layer].intermediate_format)
    CombineResidual{site} => Activation in Quant(q.residual_plan.activation_format)
    Norm                  => NormalizedActivation in Quant(q.norm_plans[plan].output_format)
    Classify              => LogitVector in Quant(q.classify_head.logit_format)
    DecodeToken           => DecodedToken in TokenIdDomain { vocab_size }
```

`ExpertCandidate`, `ExpertIntermediate`, `ExpertOutput`, `RouterScore`,
and `LogitVector` are typed as `ValueFormat::ExactAccumulator` at IR
level. F-B7 (`RangePlan`) selects an admissible implementation
(`SingleI16` / `ChunkedI16` / `RenormLoop`) preserving canonical numeric
semantics. `ExactAccumulator` is **not** a member of `QuantFormat` —
quantization is an artifact-side concept, accumulators are an IR-side
concept (see Ambiguity A29 update).

`TokenIdDomain` and `ExpertIdDomain` carry semantic domain (vocab size /
expert count) instead of physical width. Width selection is a storage
decision owned by F-B7+.

### 9.7a Op signature predicate

Every `GbNode` must satisfy `op_signature(node.op, q)`. The predicate
fixes input arity, input value kinds, output arity, output value kinds,
effect classes, and the `reduction_site` obligation for every op.
Validators rely on this predicate; it is the closed contract IIR-SC-18
references.

```text
op_signature(op, q):

  Embedding{token_input}:
    inputs       = [TokenInput[token_input].value_id, whose ValueDecl.kind
                    = InputToken and format = TokenIdDomain { vocab_size }]
    outputs      = [EmbeddingOutput]
    effects_in   = []
    effects_out  = []
    reduction_site = None

  Norm{plan}:
    inputs       = [Activation | EmbeddingOutput]
    outputs      = [NormalizedActivation]
    effects_in   = []
    effects_out  = []
    reduction_site = Some(_) iff q.norm_plans[plan].plan ∈ {TileRmsThenAffineClip}
                     None otherwise

  SequenceRead{slot}:                  -- reserved, not emitted in v1
    inputs       = []
    outputs      = [SequenceStateRead]
    effects_in   = [SequenceState(slot)]
    effects_out  = [SequenceState(slot)]
    reduction_site = None

  SequenceStep{layer}:                 -- reserved, not emitted in v1
    inputs       = [NormalizedActivation]
                   ++ SequenceStateRead values for slots in
                      state_slots_for_layer(layer), sorted by StateSlotId
    outputs      = [SequenceBlockOutput]
                   ++ SequenceStateNext values for slots in
                      state_slots_for_layer(layer), sorted by StateSlotId
    effects_in   = []
    effects_out  = []
    reduction_site = None

  SequenceWrite{slot}:                 -- reserved, not emitted in v1
    inputs       = [SequenceStateNext for that slot]
    outputs      = []
    effects_in   = [SequenceState(slot)]
    effects_out  = [SequenceState(slot)]
    reduction_site = None

  RouterMatVec{layer}:
    inputs       = [NormalizedActivation]
    outputs      = [RouterScore]
    effects_in   = []
    effects_out  = []
    reduction_site = Some(_)

  RouteTop1{layer}:
    inputs       = [RouterScore]
    outputs      = [RouterDecision, GateWeight]
    effects_in   = []
    effects_out  = []
    reduction_site = None

  ExpertMatVec{layer, expert, slot ∈ {FfnGate, FfnUp}}:
    inputs       = [NormalizedActivation]
    outputs      = [ExpertIntermediate]
    effects_in   = []
    effects_out  = []
    reduction_site = Some(_)

  FfnActivation{layer, expert}:
    inputs       = [ExpertIntermediate produced by ExpertMatVec(FfnUp)]
                   ++ [ExpertIntermediate produced by ExpertMatVec(FfnGate)]
                      iff ffn_activation_kind(layer) = SwiGLU
    outputs      = [ExpertIntermediate]
    effects_in   = []
    effects_out  = []
    reduction_site = None

  ExpertMatVec{layer, expert, slot = FfnDown}:
    inputs       = [ExpertIntermediate produced by FfnActivation{layer, expert}]
    outputs      = [ExpertCandidate]
    effects_in   = []
    effects_out  = []
    reduction_site = Some(_)

  SelectExpertTop1{layer}:
    inputs       = [RouterDecision{layer}, GateWeight{layer}]
                   ++ [ExpertCandidate for (layer, e), sorted by ExpertId
                       for e in 0..n_experts(layer)]
    outputs      = [ExpertOutput]
    effects_in   = []
    effects_out  = []
    reduction_site = None

  CombineResidual{layer = ℓ, site = PostSequence}:
    inputs       = [Activation, SequenceBlockOutput from layer ℓ]
    outputs      = [Activation]
    effects_in   = []
    effects_out  = []
    reduction_site = None

  CombineResidual{layer = ℓ, site = PostFfn}:
    Routed: inputs = [Activation, ExpertOutput from SelectExpertTop1{ℓ}]
    Dense:  inputs = [Activation, ExpertCandidate from
                      ExpertMatVec{ℓ, expert=0, slot=FfnDown}]
    outputs      = [Activation]
    effects_in   = []
    effects_out  = []
    reduction_site = None

  Classify:
    inputs       = [NormalizedActivation]
    outputs      = [LogitVector]
    effects_in   = []
    effects_out  = []
    reduction_site = Some(_)

  DecodeToken{plan}:
    inputs       = [LogitVector]
    outputs      = [DecodedToken]
    effects_in   = [Rng(Decode)] iff q.decode_spec.requires_rng
    effects_out  = [Rng(Decode)] iff q.decode_spec.requires_rng
    reduction_site = None
```

Validators implement `op_signature` as a closed match over
`InferOpTag`. Any node violating its op signature fails with
`InferIrOpSignatureMismatch`.

`ReductionSiteBearing(op, q)` is true exactly when `op_signature(op, q)`
sets `reduction_site = Some(_)`. This is the predicate referenced by
IIR-Ok-18.

### 9.8 Canonical reference semantics for `GbInferIR`

`eval_canonical_ir(g, t, s, gen)` is defined by walking `g.nodes` in
canonical order and applying each `InferOp`'s reference semantics:

```text
Embedding{token_input}    : t                       -> e
                                                      via QG.embedding_table
SequenceRead{slot}        : s_slot                  -> state
                                                      via SequenceSemanticsSpec
SequenceStep{ℓ}           : (a, state_values...)    -> (block_out, state_next_values...)
                                                      via SequenceSemanticsSpec;
                                                      state_values are sorted by
                                                      StateSlotId for slots in
                                                      state_slots_for_layer(ℓ)
SequenceWrite{slot}       : state_next              -> s_slot'
                                                      via SequenceSemanticsSpec
RouterMatVec{ℓ}           : a                       -> scores
                                                      via QG.router_weight plus
                                                      optional QG.router_bias
RouteTop1{ℓ}              : scores                  -> (top1, weight)
                                                      where normalized_scores =
                                                        router_score_normalize(scores)
                                                      and top1 = argmax(normalized_scores)
                                                      with v1 tie-break LowestExpertId,
                                                      weight is determined by
                                                      RouterGateWeightSemantics:
                                                        One           ⇒ weight = 1.0
                                                        SelectedScore ⇒ weight =
                                                          normalized_scores[top1]
ExpertMatVec{ℓ,e,FfnGate} : a                       -> gate
                                                      via QG expert gate weight plus
                                                      optional matching expert bias
ExpertMatVec{ℓ,e,FfnUp}   : a                       -> up
                                                      via QG expert up weight plus
                                                      optional matching expert bias
FfnActivation{ℓ,e}        : (gate?, up)             -> h
                                                      via q.ffn_plans[ℓ].activation_kind,
                                                      then clamp/round to
                                                      q.ffn_plans[ℓ].intermediate_format
                                                      at the named FfnActivationBoundary
ExpertMatVec{ℓ,e,FfnDown} : h                       -> candidate
                                                      via QG expert down weight plus
                                                      optional matching expert bias
SelectExpertTop1{ℓ}       : (top1, weight, candidates...) -> y
                                                      -- y = weight * candidates[top1]
CombineResidual{ℓ, site}  : (x, δ)                  -> combine_residual(x, δ, q.residual_plan)
                                                      -- named numeric boundary: clamp/round
                                                      -- to q.residual_plan.activation_format
Norm{plan}                : x                       -> norm(x)      via QG.norm_plans[plan]
Classify                  : x                       -> logits
                                                      via QG.classify_head weight
                                                      plus optional bias, then
                                                      clamp/round at the named
                                                      ClassifyLogitBoundary to
                                                      q.classify_head.logit_format
DecodeToken{plan}         : (logits, gen)           -> token        via QG.decode_spec[plan]
                                                      -- consumes Rng(Decode) iff
                                                      --   q.decode_spec.requires_rng
```

Note: dense layers do not emit `RouterMatVec`, `RouteTop1`, or
`SelectExpertTop1` nodes. The post-FFN `CombineResidual{site = PostFfn}`
takes the `ExpertCandidate` from the dense expert directly, without
gate-weight scaling. Mathematically this matches the routed-with-prob-1.0
equivalence; structurally, the IR is simpler.

Equality with `eval_canonical_qg`:

```text
FixtureSemanticEquivalence(g, q):
  if q.identity.determinism = BitExact:
    For every token-input, sequence-state, RNG state in the RFC fixture
    input set:
      eval_canonical_ir(g, t, s, gen) = eval_canonical_qg(q, t, s, gen)
    bit-for-bit, under enforced reduction order and no mid-reduction
    saturation.
  if q.identity.determinism != BitExact:
    Stage 3 records the determinism class. Fixture equivalence is not
    numerically asserted; F-C2 / F-C4 own class-relative conformance.

UniversalSemanticEquivalence(g, q) (over all inputs) is deferred to
F-C2 / F-C4.
```

This is the contract `ArtifactOracle` uses for `BitExact` artifacts at
chunk closure on the synthetic dense fixture.

### 9.9 No tile sizes, no buffers, no accumulator widths

Restated from §2.3 in IR terms:

```text
F-B5-StorageFree:
  No GbNode field declares a tile size.
  No ValueDecl declares a buffer address, page id, or arena id.
  No EffectDecl declares a storage class or lifetime class.
  No InferOp variant declares accumulator width or chunk length.
```

### 9.10 ResolvedCompilePolicy use

F-B5's pure core (`build_infer_ir_core`) reads only the
`InferIrPolicyProjection`, which carries exactly:

* `requested_runtime_modes` (so the IR identity carries that fact for
  `StageCache` keying).

`policy_resolution_self_hash` and `compile_request_hash` live in
`InferIrAuditParents` and in `InferIrReportBody.input_identity`. They
are **not** load-bearing for the embedded `GbInferIR` product and do
**not** invalidate `K3`.

F-B5 reads `DeterminismClass` from `QuantGraph.identity.determinism`,
**not** from `ResolvedCompilePolicy` or the projection. This keeps the
data flow strictly linear: F-B3 binds determinism from
`ArtifactCore.numeric_profile`, F-B5 inherits from F-B3.

The projection isolates Stage 3's cache key (`K3`) from arbitrary
unrelated `ResolvedCompilePolicy` drift: a change to a policy field that
is not in the projection does **not** invalidate Stage 3's cache.

F-B5 does **not** read:

* observation/probe selection (F-B6);
* reduction-plan ceiling (F-B7);
* recompute-promotion (F-B8);
* placement profile (irrelevant pre-storage);
* schedule knobs (F-B14);
* repair / refinement policy (F-B16).

If F-B5 ever must read those, this RFC must be amended.

## 10. Report schemas, normalized

### 10.1 `quant_graph.json`

`quant_graph.json` is a **canonical product-bearing report**. The full
`QuantGraph` product is included under `body.result.product` so later
stages may consume the IR by hash from the emitted artifact. Summary
fields (`tensor_count`, `op histograms`, etc.) are redundant review aids
and must be derivable from `product`.

```text
Report[QuantGraphReportBody]

QuantGraphReportBody :=
  {
    input_identity: {
      artifact_core_hash: Hash256,
      artifact_validation_self_hash: Hash256,
      policy_resolution_self_hash: Hash256,
      semantic_core_hash: Hash256,
      lowering_manifest_hash: Hash256,
      resolved_blob_index_hash: Hash256,
      determinism: DeterminismClass,
      model_spec_summary: ModelSpecSummary,
      sequence_semantics_kind: SequenceSemanticsKindTag,    // LinearState | BoundedKv
      ffn_topology_kind: FfnTopologyKindTag                 // Dense | Routed | Mixed
    },

    result: Option[{
      product: QuantGraph,                                  -- the full IR product

      tensor_count: u32,                                    -- review aid; derivable
      norm_plan_count: u16,                                 -- review aid; derivable
      layer_norm_count: u16,                                -- review aid; derivable
      routing_layers_count: u16,                            -- review aid; derivable
      expert_section_count: u32,                            -- review aid; derivable
      classify_head_kind: ClassifyHeadKind,                 -- review aid; derivable

      tensor_summary: List[TensorSummaryEntry],
      provenance_summary: List[ProvenanceSummaryEntry],
      decode_spec_summary: DecodeSpecSummary,
      sequence_semantics_summary: SequenceSemanticsSummary,
      classify_head_summary: ClassifyHeadSummary,

      quant_graph_self_hash: Hash256,
      quant_graph_canonical_bytes_hash: Hash256
    }],

    diagnostics: List[ValidationDiagnosticRecord]
  }
```

Semantic invariants:

```text
QG-1:
  schema = "quant_graph.v1"

QG-2:
  outcome = Passed ⇔ result = Some(_) ∧ no Hard diagnostics

QG-3:
  outcome = Failed ⇔ result = None ∧ at least one Hard diagnostic

QG-4:
  input_identity.semantic_core_hash =
    artifact_validation report's input_hashes.artifact_effective_core_hash

QG-5:
  result.tensor_count = len(QG.tensors)

QG-6:
  result.expert_section_count = len(QG.expert_sections)

QG-7:
  result.routing_layers_count = len(QG.routing_table.layers) when present, 0 otherwise

QG-8:
  result.tensor_summary sorted by TensorId

QG-9:
  result.provenance_summary sorted by TensorId
       ∧ each entry's TensorId appears in tensor_summary
       ∧ each entry's ExportTensorId is unique

QG-10:
  identity.ffn_topology_kind = Dense  ⇒ routing_layers_count = 0
  identity.ffn_topology_kind = Routed ⇒ routing_layers_count > 0
  identity.ffn_topology_kind = Mixed  ⇒ routing_layers_count > 0
       ∧ at least one layer in model_spec_summary has ffn_kind = Dense
       ∧ at least one layer has ffn_kind = Routed

QG-11:
  result.quant_graph_self_hash round-trips (parse → canonicalize → hash).
```

### 10.2 `infer_ir.json`

`infer_ir.json` is a **canonical product-bearing report**. The full
`GbInferIR` product is included under `body.result.product`. F-B6 (and any
StageCache hit replaying the IR) consumes the product directly from this
report; summary fields are redundant review aids derivable from `product`.

```text
Report[InferIrReportBody]

InferIrReportBody :=
  {
    input_identity: {
      quant_graph_self_hash: Hash256,
      policy_resolution_self_hash: Hash256,        -- audit parent
      compile_request_hash: Hash256,               -- audit parent
      static_budget_self_hash: Hash256,
      requested_runtime_modes_hash: Hash256,
      determinism: DeterminismClass,
      requested_runtime_modes: SortedSet[RuntimeMode]   -- redundant with hash; review aid
    },

    result: Option[{
      product: GbInferIR,                         -- the full IR product

      node_count: u32,                            -- review aid; derivable
      value_count: u32,                           -- review aid; derivable
      effect_count: u16,                          -- review aid; derivable
      token_input_count: u8,                      -- review aid; always 1 in v1
      topological_order_hash: Hash256,

      op_histogram: Map[InferOpTag, u32],         -- closed enum keys
      effect_class_histogram: Map[EffectClassTag, u16],
      value_kind_histogram: Map[ValueKindTag, u32],
      anchor_count: u32,

      fixture_equivalence: FixtureEquivalenceTag,
        -- VerifiedFixtureBitExact | Skipped { reason }
        -- VerifiedFixtureBitExact is required only on fixture builds (§9.3)

      infer_ir_self_hash: Hash256,
      infer_ir_canonical_bytes_hash: Hash256
    }],

    diagnostics: List[ValidationDiagnosticRecord]
  }
```

Semantic invariants:

```text
IIR-1:
  schema = "infer_ir.v1"

IIR-2:
  outcome = Passed ⇔ result = Some(_) ∧ no Hard diagnostics

IIR-3:
  outcome = Failed ⇔ result = None ∧ at least one Hard diagnostic

IIR-4:
  identity.quant_graph_self_hash matches the QG product self-hash
       referenced by Stage 2's static_budget_self_hash → policy → QG chain.

IIR-5:
  result.node_count = len(IIR.nodes)

IIR-6:
  result.op_histogram is sorted by InferOpTag canonical order (lexical).

IIR-7:
  result.value_kind_histogram and effect_class_histogram are sorted
       by their tag enum's canonical order.

IIR-8:
  result.topological_order_hash matches the in-memory
       hash produced by canonical ordering (§9.4).

IIR-9:
  result.fixture_equivalence = VerifiedFixtureBitExact
       ⇒ canonical reference semantics agreement was checked
         on the fixture input set and was bit-exact
       ∧ q.identity.determinism = BitExact.

IIR-10:
  result.fixture_equivalence = Skipped { reason }
       ⇒ reason ∈ { NonFixtureBuild, FeatureFlagDisabled,
                    NonBitExactDeterminism }
       ∧ if reason ∈ { NonFixtureBuild, FeatureFlagDisabled },
         the build is not the chunk-closure fixture build;
         if reason = NonBitExactDeterminism,
         q.identity.determinism != BitExact.

IIR-11:
  result.infer_ir_self_hash round-trips.
```

## 11. StageCache algebra

Stage 1 and Stage 3 keys, following the F-B2/F-B4 §11
`DomainHash(crate, "StageCacheKey", schema_id, schema_version, canonical_json_bytes)`
rule.

Stage 1 key:

```text
StageCacheKeyHash(schema_id, schema_version, body) :=
  DomainHash("gbf-codegen", "StageCacheKey", schema_id, schema_version,
    CanonicalJson(body))

K1 :=
  StageCacheKeyHash("quant_graph.v1", schema_version, {
    artifact_validation_self_hash,
    policy_resolution_self_hash,
    artifact_effective_core_hash,
    lowering_manifest_hash,
    resolved_blob_index_hash,
    pass_version_quant_graph,
    crate_feature_set_hash,
    quant_graph_schema_hash
  })
```

`sequence_semantics_hash` is **not** part of K1: `SequenceSemanticsSpec`
is a sub-component of `ArtifactCore`, so its identity is already captured
by `artifact_effective_core_hash`. (Sidecar rebinding of
`SequenceSemanticsSpec` is forbidden in v1; see Ambiguity A45.)

Stage 3 key:

```text
K3 :=
  StageCacheKeyHash("infer_ir.v1", schema_version, {
    quant_graph_self_hash,
    infer_ir_policy_projection_hash,
    static_budget_self_hash,
    pass_version_infer_ir,
    crate_feature_set_hash,
    infer_ir_schema_hash
  })

-- where
infer_ir_policy_projection_hash :=
  DomainHash("gbf-codegen", "InferIrPolicyProjection", "infer_ir.v1",
    CanonicalJson(InferIrPolicyProjection))

requested_runtime_modes_hash :=
  DomainHash("gbf-policy", "RuntimeModeSet", "v1",
    CanonicalJson(sorted_set(projection.requested_runtime_modes)))
-- recorded in InferIrIdentity and infer_ir.v1.input_identity; subsumed
-- by infer_ir_policy_projection_hash for K3 purposes (avoid double hashing).
```

Cache miss occurs when any field of `InferIrPolicyProjection` changes.
Arbitrary fields of the larger `ResolvedCompilePolicy` that are not in
the projection do **not** invalidate `K3`. `policy_resolution_self_hash`
and `compile_request_hash` are recorded in
`InferIrReportBody.input_identity` (audit parents only) and are not part
of the embedded `GbInferIR` product hash.

Cache laws (inherit from F-B2/F-B4 §11):

```text
C-Success-Stage1:
  Stage1 result Passed ⇒ StageCache may store QG product

C-NoFalseSuccess-Stage1:
  Stage1 result Failed ⇒ StageCache must not store success product

C-FailureMemo-Stage1:
  Stage1 result Failed ⇒ StageCache may memoize canonical failure report

C-Success-Stage3, C-NoFalseSuccess-Stage3, C-FailureMemo-Stage3:
  same shape, with K3 substituted

C-PassVersion:
  pass_version_quant_graph or pass_version_infer_ir change ⇒ cache miss

C-SchemaVersion:
  quant_graph.v1 / infer_ir.v1 schema change ⇒ cache miss

C-FeatureSet:
  crate feature set affecting layout/serde/behavior ⇒ cache miss

C-ReportRewrap-Stage3:
  A Stage 3 cache hit may replay the byte-identical GbInferIR product,
  but the driver must re-wrap it in a fresh infer_ir.json report whose
  audit-parent fields (policy_resolution_self_hash, compile_request_hash)
  match the current Stage 0 / Stage 0.5 / Stage 2 inputs. The replayed
  GbInferIR product itself is byte-identical; only the surrounding
  report identity is refreshed.
```

## 12. Diagnostic algebra

Inherits the §5 closed-enum surface from F-B2/F-B4 with these additions:

```text
ValidationOrigin (extension):
  | QuantGraphConstruction
  | InferIrConstruction

Stage1 owns codes with origin QuantGraphConstruction:
  QuantGraphTrainingResidue                       -- §2.16
  QuantGraphRoleFormatMismatch                    -- §8.6
  QuantGraphRoutingMissingForRoutedLayer          -- §2.11
  QuantGraphRoutingPresentForDenseLayer           -- §2.11
  QuantGraphRoutingExpertCoverageMismatch         -- §2.11 (n_experts vs sections)
  QuantGraphTensorIdNotUnique
  QuantGraphIdentityHashMismatch                  -- QG-Ok-4 / Stage 0 binding drift
  QuantGraphExportProvenanceMissing
  QuantGraphProvenanceImageNotInjective
  QuantGraphNormPlanReferenceUnresolved
  QuantGraphMissingLayerNorms                     -- no LayerNorms record for a layer
  QuantGraphExpertSectionWeightMissing
  QuantGraphClassifyHeadTiedMismatch
  QuantGraphClassifyHeadFormatMismatch            -- §8.5 SC-15
  QuantGraphDecodeSpecNotInCapabilitySet
  QuantGraphSequenceSemanticsTensorMismatch       -- §8.5 SC-11
  QuantGraphLayoutInconsistentWithModelSpec
  QuantGraphBlobRefUnresolvable
  QuantGraphBlobRefSizeMismatch
  QuantGraphAuxBlobRefSizeMismatch                -- §8.5 SC-13 (aux refs)
  QuantGraphDeterminismRequiresEnforcedReductionOrder
  QuantGraphRequiredFeatureUnsupported
  QuantGraphForbiddenStorageMetadata              -- §2.3 storage-freeness violation
  QuantGraphEmbeddingMissing                      -- §8.5 SC-14
  QuantGraphEmbeddingNotUnique                    -- §8.5 SC-14
  QuantGraphFfnGatePresenceMismatch               -- §8.5 SC-17
  QuantGraphLayerNormsIncomplete                  -- LayerNorms exists but
                                                  --   pre_sequence or pre_ffn missing
  QuantGraphFinalNormMissing                      -- §8.5 SC-20
  QuantGraphNormSiteDuplicate                     -- §8.5 SC-20
  QuantGraphAuxBlobKindMismatch                   -- §8.5 SC-21
  QuantGraphDecodeRequiresRngMismatch             -- §8.5 SC-22
  QuantGraphRouterGateWeightSemanticsUnsupported  -- §8.1 / RouterSemantics
  QuantGraphRouterTieBreakUnsupported             -- §8.1 / RouterSemantics
  QuantGraphBitExactMidReductionSaturationForbidden -- §2.10
  QuantGraphResidualPlanInvalid                   -- §8.5 SC-23
  QuantGraphRoutingExpertCoverageGap              -- §8.5 SC-4 (n_experts coverage)
  QuantGraphRoutingExpertCoverageExtra            -- §15.1 (extra/out-of-range section)

Stage3 owns codes with origin InferIrConstruction:
  InferIrEmbeddingNotUnique
  InferIrDecodeNotUnique
  InferIrClassifyNotUnique
  InferIrExpertCoverageMismatch                   -- §2.15 / IIR-Ok-9
  InferIrRouteCoverageMismatch                    -- IIR-Ok-10
  InferIrSemanticCheckpointEmittedHere            -- §2.12
  InferIrEffectChainNotLinear                     -- §9.5
  InferIrEffectIdEdgeTokenViolation               -- F-B5-EffectIdEdgeTokenUnique
  InferIrTopologicalOrderMismatch                 -- §9.4
  InferIrValueFormatMismatch                      -- §9.7
  InferIrNormFormatMismatch                       -- IIR-SC-6
  InferIrDecodeRngBindingMismatch                 -- IIR-SC-10
  InferIrSemanticEquivalenceFailed                -- §9.8 (fixture-only)
  InferIrCycleDetected
  InferIrUnreachableNode                          -- IIR-SC-13
  InferIrDisconnectedComponent                    -- orphaned DAG fragments
  InferIrForbiddenStorageMetadata                 -- §2.3 / §9.9
  InferIrNonV1RouterSemantics
  InferIrSemanticAnchorMissing                    -- §2.12
  InferIrFfnActivationMissing                     -- IIR-Ok-17
  InferIrExpertSelectionMissing                   -- IIR-Ok-10 / IIR-Ok-15
  InferIrGateWeightNotConsumed                    -- IIR-Ok-14 / IIR-SC-15
  InferIrTokenIngressAmbiguous                    -- IIR-SC-14
  InferIrReductionSiteMissing                     -- IIR-Ok-18
  InferIrOpHistogramTotalMismatch                 -- IIR-SC-17
  InferIrFaultBoundaryEmittedV1Forbidden          -- §9.3 / §9.5
  InferIrOpSignatureMismatch                      -- §9.7a / IIR-SC-18
  InferIrRouterScoreOrphaned                      -- IIR-SC-13 / RouterScore reachability
  InferIrSequenceSlotCoverageMismatch             -- IIR-SC-10a
  InferIrUnexpectedRngEffectOnPureOp              -- IIR-SC-10 / §9.5 (Classify et al.)
  InferIrResidualBoundaryMismatch                 -- CombineResidual semantics
  InferIrRouterMatVecMissingForRoutedLayer        -- IIR-SC-9 / IIR-Ok-10
  InferIrRouterPresentForDenseLayer               -- IIR-SC-9 (dense layer with router)
  InferIrInputTokenValueIdMismatch                -- IIR-SC-2 / IIR-SC-14
  InferIrSequenceStateNextOrphaned                -- IIR-SC-13
  InferIrSequenceSemanticsUnsupportedV1           -- §2.5a / F-B5-SequenceV1
```

Severity:

```text
∀ d ∈ Stage1.diagnostics. d.severity = Hard
∀ d ∈ Stage3.diagnostics. d.severity = Hard
```

## 13. Cross-stage interactions

### 13.1 F-B4 placeholder retirement

`QuantGraph` implements the trait `QuantGraphBudgetSource` introduced by
F-B4 (`T-B4.1`). The trait surface is unchanged; F-B3's task list includes a
sub-task `T-B3.13: retire placeholder in F-B4 fixtures`.

```text
impl QuantGraphBudgetSource for QuantGraphProduct {
  fn quant_graph_hash(&self)   -> Hash256 = self.quant_graph_self_hash;
  fn semantic_core_hash(&self) -> Hash256 =
    self.quant_graph.identity.semantic_core_hash;
  fn to_budget_view(&self)
       -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError>
       = derived from self.quant_graph.tensors,
         self.quant_graph.routing_table,
         self.quant_graph.expert_sections,
         self.quant_graph.classify_head,
         self.quant_graph.norm_plans,
         self.quant_graph.ffn_plans,
         self.quant_graph.residual_plan;
}
```

`quant_graph_self_hash` is computed via the F-B2/F-B4 `DomainHash`
convention (see §8.8). The trait targets `QuantGraphProduct`, not the
raw `QuantGraph` IR (which would otherwise have to recursively contain
its own hash). Bitwise mixing of sub-hashes is forbidden.

The placeholder trait remains for F-B4 unit tests only; new tests must use
real `QuantGraph` values.

### 13.2 F-C2 (`ArtifactOracle`) handshake

`ArtifactOracle` will consume `QuantGraph` for canonical artifact
evaluation (§8.7) and `GbInferIR` for op-for-op correspondence (§9.8).
This chunk does **not** require F-C2 to exist, and it does not require
`SemanticCheckpointId`-aligned equality (checkpoint attachment is owned
by F-B6).

The chunk closes when:

* The synthetic dense fixture under `fixtures/quant_graph/` has
  `infer_ir.report.body.result.fixture_equivalence =
   VerifiedFixtureBitExact`
  on a `BitExact` artifact, **and**
* Internal reference equivalence is verified at `NodeAnchor`-aligned
  points using `gbf-codegen::canonical::reference`.

When F-C2 lands, it may re-use the `NodeAnchor`s F-B5 emits as the
correlation points for `SemanticCheckpointId` alignment (F-B6 owns
checkpoint attachment to anchors).

### 13.3 F-B6 (`ObservationPlan`) handshake

F-B5 emits `NodeAnchorMap` so F-B6 can attach `SemanticCheckpointId` and
`TraceProbeId` references to canonical IR nodes without altering IR shape.
F-B6 binds checkpoints by `NodeId` and is responsible for emitting
`semantic_checkpoint_schema.json` and `operational_probe_schema.json`. F-B5
emits no checkpoint schema in this chunk.

### 13.4 F-B7 / F-B8 / F-B13 handshake

F-B7 (`RangePlan`) consumes `GbInferIR` reduction sites. The set of
reduction-site-bearing op variants is closed:

```text
ReductionSiteBearing(InferOp, q) :=
    RouterMatVec      -- always a reduction site (router score projection)
  | ExpertMatVec      -- always a reduction site (matvec over weight rows)
  | Norm{plan}        -- a reduction site iff
                      --   q.norm_plans[plan].plan ∈ {TileRmsThenAffineClip}
  | Classify          -- always a reduction site (final logits projection)
```

This is exactly the set of ops whose `op_signature` (§9.7a) declares
`reduction_site = Some(_)`.

For each such op, `GbNode.reduction_site` is `Some(ReductionSiteId)`
linking to the `ReductionSiteProjection` that Stage 2's
`StaticBudgetReport` already produced. F-B7 picks
`SingleI16` / `ChunkedI16` / `RenormLoop` per site by reading these ids
back through the budget product.

The Stage 2 ↔ Stage 3 join key is canonical and typed:

```text
ReductionSiteKey :=
  RouterMatVec {
    layer: LayerId,
    router_weight: TensorId,
  }
  | ExpertMatVec {
    layer: LayerId,
    expert: ExpertId,
    slot: ExpertWeightSlot,
    expert_weight: TensorId,
  }
  | Norm {
    norm_plan: NormPlanId,
    norm_site: NormSite,
  }
  | Classify {
    classify_weight: TensorId,
  }

ReductionSiteId =
  StaticBudgetReport.reduction_sites[ReductionSiteKey].reduction_site_id
```

F-B5 does **not** mint new `ReductionSiteId`s. It computes the
`ReductionSiteKey` for every reduction-bearing node and looks it up in
the passed Stage 2 product. Missing or duplicate keys fail with
`InferIrReductionSiteMissing`.

F-B8 (`StoragePlan`) consumes `ValueDecl`s and `EffectDecl`s. F-B13
(`GbSchedIR`) consumes the IR plus storage and range products to produce
slices. None of these stages may re-derive QuantGraph or GbInferIR shape.

### 13.5 F-B17 (`StageCache`) integration

`F-A6.2` (StageCache infrastructure) is closed (`bd-3ll`). F-B3 and F-B5
wire into `StageCache` directly via `K1`/`K3`. The cross-cutting `F-B17`
chunk later may add a uniform sweep, but no per-stage wiring is missing
here.

## 14. Task DAG, compressed

```text
Wave0 SchemaPrelude:
  T-B3.0 quant_graph.v1 ReportEnvelope binding
  T-B5.0 infer_ir.v1   ReportEnvelope binding
  Both depend on F-B2's ReportEnvelope/canonical-JSON/self-hash machinery.

Wave1 QGTypes:
  T-B3.1  QuantGraph type + QuantGraphIdentity
            (with lowering_manifest_hash, ModelSpecSummary fields)
  T-B3.2  QuantTensorRef + QuantTensorRole + QuantAuxBlobRef + QuantAuxKind
            + AuxFormat (replaces scale_blob_ref/threshold_blob_ref);
            blob fields use ResolvedBlobRef (content_hash, encoded/decoded
            sizes, codec)
  T-B3.2a ResolvedBlobIndex { entries, self_hash } binding;
            self_hash bound from Stage 0 product
  T-B3.3  NormPlanRecord + NormSite enum (LayerSequence/LayerFfn/Final)
  T-B3.4  LayerNorms { pre_sequence, pre_ffn } + per-layer binding
  T-B3.5  RoutingTable + RouterSemantics::Top1Hard
  T-B3.6  ExpertSection (tensor_refs, no residency_hint) + dense
            ExpertSection encoding (expert == 0)
  T-B3.7  Mixed topology summary only:
            some layers Dense, some layers Routed; no shared dense branch
            in v1 (SharedDenseMatVec / SharedDenseWeight / SharedDenseBias
            are out of scope and require explicit RFC amendment)
  T-B3.8  DecodeSpecRecord
  T-B3.9  ClassifyHead + Tied/Untied + format/bias rule
  T-B3.10 ExpertWeightSlot {FfnGate, FfnUp, FfnDown}
            + FfnPlan { activation_kind, intermediate_format }
            (per-layer in QuantGraph.ffn_plans, not ModelSpecSummary)
  T-B3.11 TensorProvenanceMap

Wave2 QGConstruction:
  T-B3.12 IdentityBinding (incl. lowering_manifest_hash)
  T-B3.13 SequenceSemanticsBinding (BEFORE TensorBinding) + state_slots
  T-B3.13a NormPlanIdPreBinding (allocate NormPlanIds from norm-site
            declarations, sorted by NormSite, before TensorBinding)
  T-B3.14 TensorBinding + role-format predicate + aux_blob_refs validation
  T-B3.15 NormPlanBinding (with NormSite)
  T-B3.16 LayerNormsBinding (per-layer pre_sequence + pre_ffn)
  T-B3.17 RoutingBinding (n_experts coverage) + ExpertBinding
            (FfnGate ⇔ SwiGLU)
  T-B3.17a ResidualPlanBinding (activation_format + combine_policy)
  T-B3.18 DecodeBinding (no silent default; explicit hash-bound or fail)
  T-B3.19 ClassifyHeadBinding (Tied output_format = embedding_format)
  T-B3.20 ProvenanceBinding (TensorProvenanceMap + aux ExportTensorIds)
  T-B3.21 CanonicalSort class (BTreeMap/BTreeSet ordering pre-hash)
  T-B3.22 SelfConsistency cross-class checks (SC-1..SC-23)
  T-B3.22a RouterSemantics + RouterGateWeightSemantics binding
  T-B3.23 retire placeholder QuantGraphBudgetSource in F-B4 fixtures
            (atomic with T-B3.24); trait now targets QuantGraphProduct,
            not QuantGraph
  T-B3.24 quant_graph.v1 schema + product-bearing report
            (body.result.product: QuantGraph) + semantic validator + tests
  T-B3.25 StageCache key K1 (DomainHash form, no sequence_semantics_hash)
            + success + failure-memo
  T-B3.26 fixture: synthetic dense Toy0/Toy1 QG + synthetic routed QG in
            fixtures/quant_graph/ (with all reject classes covered)
  T-B3.27 build_quant_graph_core (pure) / run_stage1 (driver) split;
            ResolvedBlobIndex is a pure input

Wave3 IIRTypes:
  T-B5.1  GbInferIR type + InferIrIdentity (with static_budget_self_hash,
            requested_runtime_modes_hash) + topological_order_hash
  T-B5.2  GbNode + InferOp closed enum (with ExpertMatVec slot,
            RouterMatVec, FfnActivation, SelectExpertTop1,
            SequenceStep{layer}, CombineResidual{layer, site} with
            ResidualSite) + GbNode.reduction_site
  T-B5.3  ValueDecl + ValueKind expanded (InputToken, RouterDecision,
            ExpertIntermediate, ExpertCandidate, SequenceBlockOutput,
            SequenceStateNext)
            + ValueFormat (with ExactAccumulator, TokenIdDomain,
            ExpertIdDomain; no Unit)
  T-B5.4  EffectDecl + EffectClass closed (FaultBoundary reserved-not-emitted)
            + edge-token EffectId allocation
  T-B5.5  TokenInput + TokenInputId + allowed_ingress_modes set
  T-B5.6  RngSlot closed { Decode }
  T-B5.7  InferIrProvenance map (NodeId/ValueId/EffectId -> typed refs)
  T-B5.8  NodeAnchorMap with serialized SemanticAnchor ids (DomainHash)

Wave4 IIRConstruction:
  T-B5.9  IdentityBinding + TokenInputBinding
  T-B5.10 ValueAllocation
  T-B5.11 EffectAllocation (edge-token chains per slot/RngSlot;
            FaultBoundary not emitted)
  T-B5.12 NodeBuilding + canonical topological order (NodeId after sort);
            routed layers emit RouterMatVec -> RouteTop1 -> Experts ->
            SelectExpertTop1; dense layers emit Experts directly (no
            router/select nodes)
  T-B5.12a OpSignaturePredicate implementation (§9.7a) — closed match
            over InferOpTag, used by IIR-SC-18
  T-B5.13 ReductionSiteBinding (GbNode.reduction_site for
            RouterMatVec/ExpertMatVec/Norm{TileRms*}/Classify per
            ReductionSiteBearing predicate)
  T-B5.14 ProvenanceBinding + AnchorBinding (DomainHash-derived)
  T-B5.15 CanonicalSort + topological_order_hash via DomainHash
  T-B5.16 SelfConsistency (SC-1..SC-18, including reachability,
            gate-weight consumption, op-signature predicate, and
            sequence-slot coverage)
  T-B5.17 SemanticEquivalenceCheck (BitExact only; feature-gated;
            required for fixture; Skipped{NonBitExactDeterminism} otherwise)
  T-B5.18 infer_ir.v1 schema + product-bearing report
            (body.result.product: GbInferIR) + semantic validator + tests
  T-B5.19 StageCache key K3 (DomainHash form,
            requested_runtime_modes_hash) + success + failure-memo
  T-B5.20 fixture: synthetic dense + synthetic routed IIR fixtures
            (covering all reject classes)
  T-B5.21 build_infer_ir_core (pure) / run_stage3 (driver) split

Wave5 ReviewPacket:
  T-B3.28 F-B3 review-packet sub-bundle
  T-B5.22 F-B5 review-packet sub-bundle
```

DAG law:

```text
Wave0 → Wave1 → Wave2 → Wave3 → Wave4 → Wave5
T-B3.23 (placeholder retirement) lands together with T-B3.24 (QG schema)
        at the latest, so F-B4 fixtures continue to pass after PR.
T-B5.17 is feature-gated in non-fixture builds; the fixture build flips
        the flag and gates closure on it.
```

Feature merge law:

```text
F-B3 must merge before F-B5.
F-B5 must not import a real QuantGraph from a non-merged F-B3 PR.
F-C2 (bd-c4wg) gains an explicit dependency edge to bd-7m2 once F-B5 lands.
```

## 15. Rejection classes (closure gate)

This chunk closes only when every class below is exercised by a fixture.

### 15.1 F-B3 reject classes

```text
QG-Reject-1:  QuantGraphTrainingResidue                 -- §2.16
QG-Reject-2:  QuantGraphRoleFormatMismatch              -- §8.6
QG-Reject-3:  QuantGraphRoutingMissingForRoutedLayer    -- §2.11
QG-Reject-4:  QuantGraphRoutingPresentForDenseLayer     -- §2.11
QG-Reject-5:  QuantGraphRoutingExpertCoverageMismatch   -- §2.11
QG-Reject-6:  QuantGraphTensorIdNotUnique               -- §8.5 SC-1
QG-Reject-7:  QuantGraphIdentityHashMismatch            -- QG-Ok-4
QG-Reject-8:  QuantGraphExportProvenanceMissing         -- §8.5 SC-2 / §2.8
QG-Reject-9:  QuantGraphProvenanceImageNotInjective     -- §8.5 SC-2
QG-Reject-10: QuantGraphNormPlanReferenceUnresolved     -- §8.5 SC-3
QG-Reject-11: QuantGraphExpertSectionWeightMissing      -- §8.5 SC-5
QG-Reject-12: QuantGraphClassifyHeadTiedMismatch        -- §8.5 SC-7
QG-Reject-13: QuantGraphClassifyHeadFormatMismatch      -- §8.5 SC-15
QG-Reject-14: QuantGraphDecodeSpecNotInCapabilitySet    -- §8.5 SC-8
QG-Reject-15: QuantGraphSequenceSemanticsTensorMismatch -- §8.5 SC-11
QG-Reject-16: QuantGraphLayoutInconsistentWithModelSpec -- §8.5 SC-9
QG-Reject-17: QuantGraphBlobRefUnresolvable             -- §8.5 SC-13
QG-Reject-18: QuantGraphBlobRefSizeMismatch             -- §8.5 SC-13
QG-Reject-19: QuantGraphAuxBlobRefSizeMismatch          -- §8.5 SC-13
QG-Reject-20: QuantGraphDeterminismRequiresEnforcedReductionOrder -- §2.10
QG-Reject-21: QuantGraphRequiredFeatureUnsupported      -- §8.5 SC-12
QG-Reject-22: QuantGraphForbiddenStorageMetadata        -- §2.3
QG-Reject-23: QuantGraphEmbeddingMissing                -- §8.5 SC-14
QG-Reject-24: QuantGraphEmbeddingNotUnique              -- §8.5 SC-14
QG-Reject-25: QuantGraphFfnGatePresenceMismatch         -- §8.5 SC-17
QG-Reject-26: QuantGraphLayerNormsIncomplete            -- §8.5 SC-18
QG-Reject-27: QuantGraphFinalNormMissing                -- §8.5 SC-20
QG-Reject-28: QuantGraphNormSiteDuplicate               -- §8.5 SC-20
QG-Reject-29: QuantGraphAuxBlobKindMismatch             -- §8.5 SC-21
QG-Reject-30: QuantGraphDecodeRequiresRngMismatch       -- §8.5 SC-22
QG-Reject-31: QuantGraphRouterGateWeightSemanticsUnsupported -- §8.1
QG-Reject-32: QuantGraphResidualPlanInvalid             -- §8.5 SC-23
QG-Reject-33: QuantGraphRoutingExpertCoverageGap        -- §8.5 SC-4 (missing)
QG-Reject-34: QuantGraphRoutingExpertCoverageExtra      -- §8.5 SC-4 (extra/out-of-range)
QG-Reject-35: QuantGraphBitExactMidReductionSaturationForbidden -- §2.10
QG-Reject-36: QuantGraphRouterTieBreakUnsupported       -- §8.1 / RouterSemantics
```

### 15.2 F-B5 reject classes

```text
IIR-Reject-1:  InferIrEmbeddingNotUnique                -- §2.5 / IIR-Ok-6
IIR-Reject-2:  InferIrDecodeNotUnique                   -- §2.5
IIR-Reject-3:  InferIrClassifyNotUnique                 -- §2.5
IIR-Reject-4:  InferIrExpertCoverageMismatch            -- §2.15 / IIR-Ok-9
IIR-Reject-5:  InferIrRouteCoverageMismatch             -- §9.6 SC-9 / IIR-Ok-10
IIR-Reject-6:  InferIrSemanticCheckpointEmittedHere     -- §2.12
IIR-Reject-7:  InferIrEffectChainNotLinear              -- §9.5
IIR-Reject-8:  InferIrEffectIdEdgeTokenViolation        -- F-B5-EffectIdEdgeTokenUnique
IIR-Reject-9:  InferIrTopologicalOrderMismatch          -- §9.4
IIR-Reject-10: InferIrValueFormatMismatch               -- §9.7
IIR-Reject-11: InferIrNormFormatMismatch                -- §9.6 SC-6
IIR-Reject-12: InferIrDecodeRngBindingMismatch          -- §9.6 SC-10
IIR-Reject-13: InferIrSemanticEquivalenceFailed         -- §9.8 (fixture only)
IIR-Reject-14: InferIrCycleDetected                     -- §9.6 SC-4
IIR-Reject-15: InferIrUnreachableNode                   -- §9.6 SC-13
IIR-Reject-16: InferIrDisconnectedComponent             -- orphaned DAG components
IIR-Reject-17: InferIrForbiddenStorageMetadata          -- §9.9
IIR-Reject-18: InferIrNonV1RouterSemantics              -- §8.5 / §9.6 SC-8
IIR-Reject-19: InferIrSemanticAnchorMissing             -- §2.12
IIR-Reject-20: InferIrFfnActivationMissing              -- IIR-Ok-17
IIR-Reject-21: InferIrExpertSelectionMissing            -- IIR-Ok-10
IIR-Reject-22: InferIrGateWeightNotConsumed             -- IIR-Ok-14 / IIR-SC-15
IIR-Reject-23: InferIrTokenIngressAmbiguous             -- IIR-SC-14
IIR-Reject-24: InferIrReductionSiteMissing              -- IIR-Ok-18
IIR-Reject-25: InferIrOpHistogramTotalMismatch          -- IIR-SC-17
IIR-Reject-26: InferIrFaultBoundaryEmittedV1Forbidden   -- §9.3 / §9.5
IIR-Reject-27: InferIrOpSignatureMismatch               -- §9.7a / IIR-SC-18
IIR-Reject-28: InferIrRouterScoreOrphaned               -- IIR-SC-13 / reachability
IIR-Reject-29: InferIrSequenceSlotCoverageMismatch      -- IIR-SC-10a
IIR-Reject-30: InferIrUnexpectedRngEffectOnPureOp       -- IIR-SC-10 / §9.5
IIR-Reject-31: InferIrResidualBoundaryMismatch          -- CombineResidual semantics
IIR-Reject-32: InferIrRouterMatVecMissingForRoutedLayer -- IIR-SC-9 / IIR-Ok-10
IIR-Reject-33: InferIrRouterPresentForDenseLayer        -- IIR-SC-9
IIR-Reject-34: InferIrInputTokenValueIdMismatch         -- IIR-SC-2 / IIR-SC-14
IIR-Reject-35: InferIrSequenceStateNextOrphaned         -- IIR-SC-13
IIR-Reject-36: InferIrSequenceSemanticsUnsupportedV1    -- §2.5a
```

Each reject class is gated by a typed fixture under
`fixtures/quant_graph/reject/` or `fixtures/infer_ir/reject/`.

## 16. Proof obligations

```text
O1 QG/IIR determinism:
  Same inputs generate byte-identical quant_graph.json and infer_ir.json
  across two clean regenerations.

O2 Self-hash + product round-trip:
  Both reports and their embedded products round-trip through
  parse → canonicalize → semantic validation → self-hash.

O3 QG rejection completeness:
  Every QG-Reject-* class has a fixture and typed diagnostic.

O4 IIR rejection completeness:
  Every IIR-Reject-* class has a fixture and typed diagnostic.

O5 Provenance totality:
  Every TensorId has an ExportTensorId; every QuantAuxBlobRef has an
  ExportTensorId; every NodeId/ValueId/EffectId in g.provenance maps
  to a typed entity.

O6 Storage-freeness:
  No QG/IIR field carries a storage class, lifetime class, materialization,
  alias class, byte offset, page id, commit-group id, accumulator width,
  or tile size. ValueFormat::ExactAccumulator declares "implementation
  format chosen later" without committing to a width here.

O7 Single-token convention:
  Exactly one Embedding (with TokenInputId), one Classify, and one
  DecodeToken per IIR pass.

O8 Routing topology consistency:
  Routed layers have routing_table entries, per-expert sections, exactly
  one RouteTop1 node, and exactly one SelectExpertTop1 node consuming the
  GateWeight and RouterDecision and all candidates.
  Dense layers have no routing_table entry, no RouteTop1 / SelectExpertTop1
  node, and exactly one expert_section with expert == 0.

O9 Effect linearity:
  Every effect-class instance forms a linear chain of edge-token EffectIds.
  Each EffectId is produced by at most one node and consumed by at most
  one node.

O10 Topological order stable:
  topological_order_hash is stable and deterministic across regenerations.
  NodeId is assigned after canonical sort, not before.

O11 FixtureSemanticEquivalence (BitExact only):
  q.identity.determinism = BitExact ⇒ for every (t, s, gen) in the RFC
  fixture input set, the fixture build's
    eval_canonical_ir(g, t, s, gen) = eval_canonical_qg(q, t, s, gen)
  bit-for-bit. UniversalSemanticEquivalence and weaker DeterminismClass
  conformance are owned by F-C2 / F-C4.

O12 No semantic checkpoints:
  No SemanticCheckpointId, TraceProbeId, or SemanticCheckpoint effect
  class appears anywhere in IIR.

O13 Cache soundness:
  Failure memo is never usable as success product. Cache miss occurs on
  pass_version, schema, or feature-set drift. Cache hit replays
  byte-identical canonical product (including embedded IR).

O14 F-B4 wire-up:
  F-B4 fixtures continue to pass after the placeholder is retired and
  QuantGraph implements QuantGraphBudgetSource via DomainHash-based
  quant_graph_self_hash.

O15 F-C2 readiness:
  ArtifactOracle is unblocked: the fixture build emits a canonical
  product-bearing report for QuantGraph and GbInferIR with NodeAnchors
  serialized so a later F-C2 implementation can consume them by hash.

O16 No hidden defaults:
  No QG/IIR field is silently filled by a default; every value derives
  from a hash-bound input or fails loudly. DecodeBinding may use
  artifact_core.decode_caps.default only when that default is explicitly
  present and hash-bound in ArtifactCore.

O17 No scheduling fusion:
  No InferOp variant represents a fusion of two distinct canonical
  semantic sites. Bias / affine parameters inside a canonical
  affine / norm op are allowed. ExpertMatVec slots, FfnActivation, and
  SelectExpertTop1 are separate ops.

O18 Expert/slot coverage:
  Every TensorId with role ExpertWeight {layer, expert, slot} is
  realized exactly once in IIR as InferOp::ExpertMatVec
  {layer, expert, slot}. Every routed layer's expert candidates feed
  exactly one SelectExpertTop1.

O19 Determinism class binding:
  IIR.identity.determinism = QG.identity.determinism =
  ArtifactCore.numeric_profile.determinism.

O20 Reachability:
  Every non-terminal ValueId is consumed by at least one later node;
  no orphaned subgraphs exist (IIR-SC-13). DecodedToken is the only
  v1 terminal.

O21 ResolvedBlobIndex invariant:
  All BlobRef and QuantAuxBlobRef references in QuantGraph are copied
  from the Stage 1 ResolvedBlobIndex, whose self_hash is committed by
  the Stage 0 validation product. Declared logical layouts and quant
  formats agree with the resolved decoded payload sizes.

O22 Pure-function shape:
  build_quant_graph_core and build_infer_ir_core are pure functions of
  their typed inputs. Side effects (JSON emission, StageCache writes)
  are isolated in the run_stageN drivers (§6 amendment).

O23 Stage 1/3 reports forbid repair provenance:
  No diagnostic in quant_graph.json or infer_ir.json carries a
  RepairProposal source or any AuthorizedRelaxation operation.
```

## 17. End-to-end theorem

```text
Theorem CanonicalIRPipelineSoundness:

Given:
  Imported inputs i
  validate_artifact_and_request(i) = Ok(v)
  resolve_policy(v)                = Ok(p)
  All BlobRefs and QuantAuxBlobRefs in v resolve through the Stage 1
    ResolvedBlobIndex, and that index is committed by Stage 0.  [F-B2/F-B3]
  build_quant_graph({v, p, ac, ss, resolved_blob_index}) = Ok(q)
  static_budget({p, q.budget_view, runtime_budget})   = Ok(b)   [F-B4]
  b.decision.fits = true
  build_infer_ir({q, q.self_hash, p, b, b.self_hash}) = Ok(g)

Then:
  1. q is a valid QuantGraph: total provenance, role/format consistent,
     routing topology consistent, dense-as-router-prob-1.0 coherent,
     per-layer LayerNorms populated, embedding unique, classify-head
     tied/untied consistent, decode spec ∈ capability set, sequence
     semantics consistent with model topology, no training residue, no
     storage metadata, lowering_manifest_hash bound from Stage 0.
  2. g is a valid GbInferIR: storage-free, effect-linear (edge-token
     unique), topologically canonical (NodeId assigned after canonical
     sort), op-and-effect coverage total over q's entities, no semantic
     checkpoint emission, GateWeight and RouterDecision consumed by
     SelectExpertTop1, FfnActivation present on every routed expert,
     reduction_site bound for every reduction-bearing op.
  3. If this is the chunk-closure fixture build and
     q.identity.determinism = BitExact:
       FixtureSemanticEquivalence(g, q) holds bit-for-bit on the fixture
       input set under canonical reference semantics with enforced
       reduction order and saturation only at named numeric boundaries
       (residual combine, classify logit, FFN activation, final clamp).
     Otherwise:
       Stage 3 records the determinism class but does not assert numeric
       equality; F-C2 / F-C4 own class-relative conformance.
       UniversalSemanticEquivalence is deferred to F-C2 / F-C4 in all
       cases.
  4. q and g are content-addressed and reproducible across two
     consecutive regenerations on the same hash-bound inputs.
     topological_order_hash is stable.
  5. F-B4 fixtures continue to pass with QuantGraph implementing
     QuantGraphBudgetSource via DomainHash-based quant_graph_self_hash.
  6. F-C2 (ArtifactOracle) is unblocked: the fixture build emits a
     canonical product-bearing report so a later F-C2 implementation can
     consume QuantGraph and GbInferIR by hash and align via NodeAnchors.
  7. F-B6, F-B7, F-B8, F-B13 may consume g without re-deriving QG/IIR
     shape: F-B6 reads anchors, F-B7 reads reduction_site ids, F-B8
     reads ValueDecls/EffectDecls, F-B13 reads the full IR.

Not proven:
  UniversalSemanticEquivalence(g, q) over arbitrary workloads
                                        (F-C2 / F-C4 own; this chunk
                                         only verifies fixture equivalence
                                         on the synthetic fixture input set)
  op-for-op correspondence on arbitrary workloads
                                        (F-C2 / F-C4 own)
  observation/probe selection           (F-B6)
  reduction-plan structure              (F-B7)
  storage class / lifetime / aliasing   (F-B8)
  spatial residency                     (F-B9, F-B10, F-B11)
  arena byte ranges                     (F-B12)
  scheduled slices / lease lifecycle    (F-B13)
  schedule cost envelopes               (F-B14)
  backend reachability / placement      (F-B15)
  refinement-loop convergence           (F-B16)
  conformance against ConformanceEnvelope (F-C4)
```

## 18. Final concise contract

```text
F-B3/F-B5 is correct when:

1. QuantGraph is constructed deterministically from ValidatedInputs +
   ResolvedCompilePolicy + ArtifactCore + SequenceSemanticsSpec, with
   every tensor, norm plan (with NormSite), per-layer LayerNorms
   (pre_sequence + pre_ffn), routing entry, expert section, decode spec,
   classify head, and aux blob ref bound from artifact contents and never
   from silent defaults. lowering_manifest_hash is recorded in identity.

2. QuantGraph carries total provenance back to exported tensor ids
   (including aux QuantAuxBlobRefs), with the provenance image injective
   and complete.

3. QuantGraph rejects training residue, role-format mismatches, missing
   routing, expert-coverage gaps, decode-cap mismatches, sequence-semantics
   tensor mismatches (KV slabs live in SequenceSemanticsSpec.state_slots,
   not in QuantTensorRole), blob ref drift, missing layer norms, and any
   storage metadata.

4. GbInferIR is constructed deterministically from QuantGraph + a minimal
   subset of ResolvedCompilePolicy fields + StaticBudgetProduct, with
   every node, value, and effect typed and provenance-linked back to a
   QuantGraph entity through g.provenance maps. DeterminismClass is read
   from QuantGraph.identity.determinism, not from policy.

5. GbInferIR is storage-free (no tile sizes, no buffers, no accumulator
   widths via QuantFormat, no materialization). ValueFormat::ExactAccumulator
   declares "implementation chosen later" without committing widths.
   Effect chains are edge-token linear: each EffectId is produced by at
   most one node and consumed by at most one node. NodeId is assigned
   after canonical sort. No semantic checkpoints are emitted; FaultBoundary
   is reserved but never emitted in v1.

6. Single-token convention holds: exactly one Embedding (consuming a
   TokenInputId whose allowed_ingress_modes set is bound at runtime),
   one Classify, one DecodeToken per IR pass.

7. The routed FFN math is complete: RouterMatVec produces RouterScore;
   RouteTop1 consumes RouterScore and produces RouterDecision +
   GateWeight; FfnActivation wires gate/up to a nonlinear activation;
   SelectExpertTop1 consumes RouterDecision, GateWeight, and all expert
   candidates and produces the gate-weighted selected expert output.
   Dense FFN is *mathematically* equivalent to a routed FFN with hard
   probability 1.0 on expert 0, but its IR shape is direct: dense layers
   emit no RouterMatVec, RouteTop1, or SelectExpertTop1 nodes.

8. FixtureSemanticEquivalence(GbInferIR, QuantGraph) is asserted ONLY
   when q.identity.determinism = BitExact (verified at chunk closure on
   the synthetic fixture input set). Weaker DeterminismClass values are
   recorded but not numerically asserted; F-C2 / F-C4 own
   class-relative conformance and UniversalSemanticEquivalence in all
   cases.

9. F-B4's placeholder QuantGraphBudgetSource is retired; F-B4 fixtures
   continue to pass with QuantGraph as the real source. Hashes use the
   F-B2/F-B4 DomainHash convention; bitwise mixing of sub-hashes is
   forbidden.

10. ArtifactOracle (F-C2) is unblocked: the fixture build emits a
    canonical product-bearing report (quant_graph.json and infer_ir.json
    include body.result.product) with serialized NodeAnchors derived by
    DomainHash, so a later F-C2 implementation can consume QuantGraph
    and GbInferIR by hash. SemanticCheckpointId-aligned equality is NOT
    a chunk-closure requirement (F-B6 owns checkpoint attachment).

11. Both reports are canonical, deterministic, and self-hash-valid;
    StageCache keys K1 and K3 use DomainHash; cache miss occurs on
    pass_version, schema, or feature-set drift; cache hit replays
    byte-identical canonical product.

12. Stage 1 and Stage 3 reports do not contain RepairProposal
    provenance or AuthorizedRelaxation operations; pure cores
    (build_quant_graph_core / build_infer_ir_core) are isolated from IO
    drivers (run_stage1 / run_stage3). Audit parents
    (policy_resolution_self_hash, compile_request_hash) live in
    InferIrAuditParents and InferIrReportBody.input_identity but never
    invalidate K3.

13. Stage 3 sequence-block lowering is reserved-not-emitted in v1
    (§2.5a). SequenceRead / SequenceStep / SequenceWrite remain in the
    InferOp enum as reserved shape; non-empty sequence-state slots are
    rejected with InferIrSequenceSemanticsUnsupportedV1. The reference
    evaluator treats the sequence block as identity for v1 artifacts.

14. QuantTensorRef and QuantAuxBlobRef carry ResolvedBlobRef (content
    hash, encoded/decoded sizes, codec). ResolvedBlobIndex.self_hash is
    bound into K1. QuantGraph is content-addressed end to end.

15. Per-layer FfnPlan { activation_kind, intermediate_format } and
    explicit ClassifyHead.logit_format pin the named numeric boundaries
    that BitExact relies on. RouterSemantics::Top1Hard carries
    RouterTieBreak::LowestExpertId for deterministic argmax.

16. Stage 2 ↔ Stage 3 reduction-site joins use a typed
    ReductionSiteKey; F-B5 never mints ReductionSiteIds.
```

## 19. Ambiguity ledger

|  ID | Ambiguity                                                                                                                                  | Chosen path in this RFC                                                                                                          | Clarifying question                                                                          | Suggested final decision                                                                       |
| --: | ------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
|  A1 | F-C2 (`bd-c4wg`) lists F-B3 as a blocker but not F-B5; the description says "compares op-for-op against `GbInferIR`."                       | This RFC pins F-C2's full op-for-op correspondence to require F-B5. The chunk mints the dependency edges: `bd-c4wg depends_on bd-7m2`, and `bd-7m2 blocks bd-c4wg's full op-for-op gate`. | Should F-C2 begin with `QuantGraph` alone or wait on `GbInferIR`?                            | Begin against `QuantGraph` for canonical evaluation; require `GbInferIR` for op-for-op gate.   |
|  A2 | `planv0.md` line 1576 lists "routing tables, expert sections" without pinning dense-FFN encoding.                                          | Dense-FFN is encoded as exactly one `ExpertSection` with `expert == 0`, no routing table entry.                                  | Should dense FFN be a separate `DenseFFN` field or modeled as one expert?                    | Stay with one-expert convention for v1; revisit when F13 dense baseline lands.                  |
|  A3 | `planv0.md` line 1604 leaves `TokenSrc` open.                                                                                              | **Superseded by A53.** `TokenSrc` enum is replaced with a single `TokenInput` value carrying `allowed_ingress_modes: NonEmptySet<TokenIngressMode>`; runtime binds the mode at IR-pass entry. | Are there v1 token sources beyond prompt and autoregressive output?                          | No. New ingress modes require RFC amendment.                                                    |
|  A4 | `InferOp::ExpertMatVec` per `planv0.md` lacks a slot field; FFN is multi-matrix.                                                            | Add `slot: ExpertWeightSlot` so each matmul is its own node (no fusion).                                                         | Should `ExpertMatVec` be split per slot or fused?                                            | Split. Fusion lives in scheduling (F-B13).                                                       |
|  A5 | `planv0.md` line 1610 says `Norm { plan: NormPlan }`.                                                                                       | IR carries `NormPlanId` only; `QuantGraph.norm_plans` resolves the body.                                                          | Should the IR carry the full plan body or just an id?                                        | Just an id, to keep nodes small and IR/QG provenance clean.                                      |
|  A6 | `planv0.md` line 1612 says `DecodeToken { plan: DecodeSpec }`.                                                                              | IR carries `DecodePlanId`; `QuantGraph.decode_spec` resolves the body.                                                            | Same question as A5 for decode.                                                              | Same answer.                                                                                    |
|  A7 | `EffectClass` is implicitly named in `planv0.md` (sequence-state, RNG); the precise closed set is not pinned.                              | Closed in v1: `SequenceState(slot)`, `Rng(slot)`, `FaultBoundary`. Semantic-checkpoint emission excluded — owned by F-B6.        | Should `SemanticCheckpoint` be an effect class here?                                          | No in v1. F-B6 owns checkpoint emission. Re-open if F-B6 needs it.                              |
|  A8 | `planv0.md` does not pin one-token-per-IR-pass.                                                                                            | This RFC pins single-token convention; auto-regressive iteration at runtime.                                                    | Could a future IR variant be multi-token batched?                                            | No in v1; would require an explicit RFC amendment.                                              |
|  A9 | `RouterSemantics` is implicit; `RouteTop1` is the only variant in `planv0.md` line 1607.                                                    | `RouterSemantics::Top1Hard` is the v1 closed set; soft top-1 etc. require an amendment.                                          | Should soft top-1 routing be admissible at IR level?                                          | No. Soft routing is a training-side artifact; deployed model is hard top-1.                    |
| A10 | `RngSlot` is unspecified.                                                                                                                  | Closed in v1: `RngSlot::Decode` only.                                                                                            | Will any other slots exist in v1?                                                            | No. New RNG slots require RFC amendment.                                                        |
| A11 | `ClassifyHead.Tied` vs `Untied` is not in `planv0.md`.                                                                                      | Make it an explicit `ClassifyHeadKind` enum on `QuantGraph`.                                                                     | Should `ArtifactCore` carry the tied flag, or should it be inferred?                         | Carry it explicitly via `ArtifactCore.model.classify_head_kind`.                                |
| A12 | `QuantTensorRef.role` is implicit in `planv0.md`.                                                                                          | Add `QuantTensorRole` closed enum (§8.6).                                                                                        | Are roles fixed or extensible?                                                               | Fixed in v1; new roles require RFC amendment.                                                   |
| A13 | `QuantTensorRef` may need scale and threshold tensors (per `TernaryWeightPlan`).                                                            | **Superseded.** Use `aux_blob_refs: Vec<QuantAuxBlobRef>` with each entry carrying typed `kind` (Scale/Threshold/SparseMeta), layout, format, blob ref, and export provenance — see §8.1. | Should scales be separate tensors or inline?                                                 | Separate `QuantAuxBlobRef` with its own hash, layout, and provenance; never an `Option<BlobRef>` pair. |
| A14 | `ExpertWeightSlot` ordering (Gate? Up Down) is not in `planv0.md`.                                                                          | Canonical ordering `FfnGate` (when present) → `FfnUp` → `FfnDown`.                                                                | Could a non-SwiGLU FFN omit `FfnGate`?                                                       | Yes. `FfnGate` slot is optional; `FfnUp` and `FfnDown` are required.                            |
| A15 | `SequenceSemanticsSpec` carries `LinearState` or `BoundedKv` per artifact; QuantGraph might also need per-layer variants.                  | Treat `SequenceSemanticsSpec` as global (whole-model) for v1; per-layer variants require an amendment.                           | Will mixed-block models (LinearState in some layers, BoundedKv in others) need this?         | Defer until M4 lands.                                                                           |
| A16 | `SemanticCheckpointId` attachment point is unclear.                                                                                        | **Updated.** Emit opaque `NodeAnchorMap` inside `GbInferIR` and therefore inside `infer_ir.v1`'s product. Each anchor is a `DomainHash`-derived id (§2.12). Do **not** encode `SemanticCheckpointId` or `TraceProbeId` in this chunk; F-B6 attaches them later via the anchor. | Should the JSON include anchor metadata?                                                     | Yes — opaque anchors are serialized so a StageCache hit replaying the IR produces the same anchors. Checkpoint/probe ids are not. |
| A17 | `topological_order_hash` is new.                                                                                                            | Hash over canonical-ordered `(node_id, op_tag)` sequence (§9.4).                                                                  | Should it include value/effect ids too?                                                      | Not in v1; that would couple to allocation order.                                               |
| A18 | `TensorId` numbering is unspecified.                                                                                                        | `TensorId` is `u32`, allocated in canonical iteration order over `ArtifactCore.tensors` so two regenerations produce the same map. | Should `TensorId` survive across artifact lineages?                                          | No. `TensorId` is QG-internal. Lineage uses `ExportTensorId`.                                   |
| A19 | `eval_canonical_qg` and `eval_canonical_ir` are not provided as code in `planv0.md`.                                                        | This RFC pins op order and typing only; per-op math is inherited from `gbf-artifact` types.                                       | Should this RFC ship a reference evaluator?                                                  | Yes, in `gbf-codegen::canonical::reference` as a private internal crate.                       |
| A20 | `DeterminismClass::BitExact` requires reduction-order pinning, but `QuantGraph` does not carry reduction order.                            | Reduction order is policy-side; `policy.numeric_profile.reduction_order_policy` must equal `Enforced` when QG declares `BitExact`. | Should QG carry reduction order?                                                             | No. Policy already owns it; QG only carries `DeterminismClass` for reporting.                  |
| A21 | "No fusion at the IR level" is not in `planv0.md`.                                                                                          | Pinned in §2.6 with a typed law `F-NoFuse`.                                                                                       | Should we permit fused `Norm+MatMul` at IR level for performance?                            | No. Fusion is a scheduling decision (F-B13/F-B14).                                              |
| A22 | "No partial layers" is not in `planv0.md`.                                                                                                  | Pinned in §2.15. Every layer-expert pair is realized.                                                                              | Could ablation profiles skip a layer?                                                        | Not at IR level. Ablation is a separate compile.                                                |
| A23 | `ExpertSection.tensor_refs` (renamed from `weight_refs`) ordering rule is implicit.                                                         | Required to match canonical `ExpertWeightSlot` order (FfnGate? → FfnUp → FfnDown), with each weight's bias TensorId immediately following the weight when present (§8.5 SC-6). | Should be a runtime property or a schema property?                                            | Schema. F-B3 enforces; tests pin.                                                                |
| A24 | `ClassifyHead.kind = Tied` shares weights with the embedding table; shape constraints differ from a separate head.                          | When `Tied`, `classify_head.weight = embedding tensor_id`; QG enforces equality and shape consistency.                            | Should output bias be optional even when tied?                                               | Yes. Bias is independent of weight tying.                                                       |
| A25 | `BoundedKv` semantics need a per-token slab; how does it appear in QG?                                                                     | **Updated.** KV slab shapes live in `SequenceSemanticsSpec.state_slots`. They are not `QuantTensorRef`s and have no `BlobRef` (they are runtime state, not frozen artifact tensors). F-B5 wires `SequenceRead` / `SequenceStep{layer}` / `SequenceWrite` nodes from those slots. | Should KV slab shape live in QG or only in `SequenceSemanticsSpec`?                          | Only in `SequenceSemanticsSpec.state_slots` for v1. Adding them to `QuantTensorRole` would force fake `BlobRef`s for runtime state. |
| A26 | `LinearState` semantics imply per-layer state vector(s); `StateSlotId` per layer is not pinned.                                            | Each layer's `LinearState` declares `state_dim` and a `StateSlotId`; F-B5 emits one `SequenceRead`/`Write` pair per slot.         | Could one layer have multiple state slots (e.g. multi-timescale T12.5)?                      | Yes. T12.5 multi-timescale yields multiple slots per layer.                                     |
| A27 | `ExpertResidencyHint` on `ExpertSection`: load-bearing or not?                                                                              | **Removed from `QuantGraph` in v1.** Residency is downstream storage/scheduling information and conflicts with the storage-free boundary (§2.3). HintBundle remains the only carrier for residency hints; F-B10 reads it directly. | Should the hint live on QG or HintBundle?                                                    | HintBundle only. Do not duplicate into QG.                                                       |
| A28 | `ExpertWeightSlot::FfnGate` presence depends on FFN flavor; SwiGLU vs ReLU FFN.                                                            | If `ModelSpec.ffn.activation = SwiGLU`, `FfnGate` is required; otherwise forbidden.                                               | Should activation kind be in QG?                                                             | Yes, derived from `ModelSpec.ffn.activation`.                                                   |
| A29 | `ValueKind::ExpertOutput` accumulator format is intentionally unconstrained; how is `infer_ir.v1` JSON serialized?                          | **Do not** extend `QuantFormat`. Use `ValueFormat::ExactAccumulator` (an IR-side enum, not artifact-side) whose semantics are "F-B7 will pin a legal implementation preserving canonical numeric semantics." `QuantFormat` remains artifact quantization only. | Is sentinel format acceptable in canonical JSON?                                              | Yes — `ValueFormat::ExactAccumulator` has an explicit enum tag and round-trips. |
| A30 | `infer_ir.v1` `op_histogram` keys: `InferOpTag` enum vs op variant strings.                                                                | Use closed `InferOpTag` enum; lexical canonical order over the tag names.                                                          | Could histogram keys drift across schema bumps?                                              | No. Tag enum is closed and pinned per schema version.                                            |
| A31 | "feature-gated SemanticEquivalenceCheck" is new; what feature flag and where does it live?                                                  | `cfg(feature = "semantic_equivalence_check")` in `gbf-codegen`; the fixture build enables it.                                     | Should the flag default on or off?                                                           | Off by default; fixture/CI enables it.                                                         |
| A32 | `DecodeSpec` may not be in `DecodeCapabilitySet` (e.g. policy override).                                                                    | F-B3 rejects with `QuantGraphDecodeSpecNotInCapabilitySet` if so.                                                                  | Should policy override be allowed to widen capabilities?                                     | No. `DecodeCapabilitySet` is artifact-side; policy can narrow but never widen.                  |
| A33 | `requested_runtime_modes` affects IR identity but not IR shape in v1.                                                                       | `infer_ir.v1.identity.requested_runtime_modes` is recorded; IR shape is mode-independent for v1.                                  | Will multi-mode IR shape differ in M4 (`SchedulePack`)?                                       | Possibly; `SchedulePack` keys per `RuntimeMode` are F-B13 territory, not IR.                   |
| A34 | `static_budget_self_hash` is required in IIR identity; what if Stage 2 is bypassed in fixtures?                                             | The fixture build runs Stage 0 → 0.5 → 1 → 2 → 3; bypassing Stage 2 is forbidden in non-test builds.                              | Are there test-mode bypasses?                                                                | Tests may construct synthetic Stage 2 stub products only with explicit `cfg(test)` gating.      |
| A35 | `provenance` shape: should `QuantGraph.provenance` carry intermediate `LoweringTensorId`s for traceability through `TargetDataLoweringArtifact`? | In v1 the map is `TensorId → ExportTensorId` only. Lowering traceability is via `lowering_manifest_hash` recorded in identity.    | Should provenance be three-step (TensorId → LoweringTensorId → ExportTensorId)?              | Defer. Add the middle id only when a stage needs it explicitly.                                 |
| A36 | `EffectId` allocation: shared across SequenceState slots vs per-slot.                                                                       | **One chain per effect-class instance (per slot); EffectIds are per-edge tokens within that chain, not aliases for the class.** Each effectful node consumes one `EffectId` of the class and produces a fresh one. | Could read/write share an `EffectId` for compactness?                                        | No. Read consumes one edge token; Write produces the next edge token; same chain, distinct ids. |
| A37 | `FaultBoundary` effect: is it always present?                                                                                               | **Reserved but never emitted in v1.** The enum variant is reserved so a later repair/refinement RFC can amend this RFC without changing the effect-class namespace. Emitting `FaultBoundary` would expand Stage 3's policy projection and requires explicit amendment. | Should F-B5 emit `FaultBoundary` based on policy?                                             | No in v1. F-B5 reads no repair/refinement policy; future amendment required to enable.         |
| A38 | `op_histogram` may include `EmbeddingTag = 1`, `DecodeTokenTag = 1`, `ClassifyTag = 1` always; redundant?                                   | Keep the histogram total even when uniform — allows downstream consumers to assert without conditionals.                          | Could reports omit single-instance tags for compactness?                                     | No. Uniformity beats compactness for review tooling.                                            |
| A39 | `ModelSpecSummary` in QG identity vs full `ModelSpec` in `ArtifactCore`.                                                                   | Summary only: `n_layers`, `d_model`, `d_ff`, `n_experts(layer)`, `ffn_topology_kind`. Full spec stays in `ArtifactCore`.          | Should full `ModelSpec` round-trip into QG?                                                  | No. Hash binding is enough; redundant duplication invites drift.                                |
| A40 | `SequenceSemanticsKindTag` vs full `SequenceSemanticsSpec` in QG identity.                                                                  | Tag in identity (`LinearState | BoundedKv`); full spec in body.                                                                    | Why duplicate?                                                                               | Tag is for cheap report-level branching; body is the source of truth.                           |
| A41 | `ClassifyHead.output_format` may differ from `EmbeddingTable.format` even when tied.                                                        | When `Tied`, `output_format` must equal the embedding format; F-B3 rejects mismatch.                                              | Could output_format be widened (e.g. `I8` weight, `Q8_8` output)?                            | Output format is read at the head's quant boundary; widening is a schema bump.                  |
| A42 | `T-B3.13` (placeholder retirement) merges atomically with T-B3.14. What if test infra needs the placeholder later?                          | Placeholder trait stub is preserved for unit tests of F-B4 internals only; new tests use real QuantGraph.                          | Should the trait be public?                                                                  | Trait is `pub(crate)` after retirement; public surface is `QuantGraph` itself.                  |
| A43 | `T-B5.11 SemanticEquivalenceCheck` is feature-gated; which builds enable it?                                                                | The fixture build under `fixtures/quant_graph/` enables `semantic_equivalence_check`. CI runs that build.                          | Should production builds run the check?                                                      | No. Production builds run F-C2 instead.                                                        |
| A44 | `infer_ir.v1.identity.static_budget_self_hash` is `Option`?                                                                                 | Required in non-test builds; optional only in `cfg(test)` and gated by A34's stub-mode rule.                                       | Should this be a separate `infer_ir.v1.test.identity` schema?                                | No. Use a single schema with explicit None gating in test mode only.                            |
| A45 | `K1` stage-cache key originally included `sequence_semantics_hash` — is that redundant?                                                     | **Yes, redundant. Removed from K1.** `SequenceSemanticsSpec` is part of `ArtifactCore`, so `artifact_effective_core_hash` already captures its identity. Sidecar rebinding of `SequenceSemanticsSpec` is **forbidden in v1** — it must always equal `ArtifactCore.sequence` by content hash. If a future amendment introduces sidecar rebinding, that amendment must add a precise `DomainHash("gbf-artifact", "SequenceSemanticsSpec", "v1", CanonicalJson(input.sequence_semantics))` to K1. | Should sidecar rebinding be allowed in v1?                                                    | No. Defer to a future amendment if needed.                                                     |
| A46 | `K3` includes `requested_runtime_modes_hash`; what's the hash domain?                                                                       | `DomainHash("gbf-policy", "RuntimeModeSet", "v1", canonical_json(sorted_set))`.                                                    | Should it be a sub-hash of `policy_resolution_self_hash`?                                    | No. Independent hash so Stage 3 caching is sensitive only to mode set drift.                    |
| A47 | `op_histogram` for an empty `routing_layers_count` model still includes `RouteTop1: 0`?                                                     | Yes. Tag is present with count 0, not absent.                                                                                      | Could absent equal zero?                                                                     | No. Absent vs present must be distinct in canonical JSON for review tooling.                    |
| A48 | "No partial layers" prevents ablation experiments at IR level; how do experiments swap layers?                                              | Build a different `ArtifactCore` (with the desired layer set) and recompile. IR shape stays total.                                | Should ablation be IR-level?                                                                 | No. Ablations are artifact-level.                                                              |
| A49 | `SemanticEquivalence` uses `BitExact`; weaker classes are deferred to F-C2.                                                                 | **Updated.** This RFC asserts `FixtureSemanticEquivalence` (not universal) only when `q.identity.determinism = BitExact`. Weaker DeterminismClass values are out of scope here. `UniversalSemanticEquivalence` is deferred to F-C2 / F-C4 in all cases. | Should we still log a `Verified` token when DeterminismClass weakens?                        | No in v1. Emit `Skipped { reason: NonBitExactDeterminism }` and let F-C2 refine.               |
| A50 | `ExpertSlotAffinity` (in `CompilePreferences`, hint side) vs `ExpertResidencyHint` in QG.                                                   | `ExpertResidencyHint` is removed from QG (see A27). `ExpertSlotAffinity` in HintBundle remains the only carrier of expert-slot residency hints. | Should they be the same field?                                                               | No. Hints live in HintBundle; QG is storage-free. |
| A51 | Reports as products vs reports as summaries. Earlier draft kept summary-only `result` blocks.                                              | **Reports are canonical product-bearing.** `quant_graph.json` includes `body.result.product: QuantGraph`; `infer_ir.json` includes `body.result.product: GbInferIR`. Summary fields are redundant review aids, derivable from `product`. | Should JSON consumers use product or summary?                                                | Use `product`. Summary fields exist only for review/diff readability. |
| A52 | `quant_graph_self_hash` mixing rule. Earlier draft used `semantic_core_hash ⊕ canonical_bytes_hash`.                                       | **Forbidden.** Use `DomainHash("gbf-codegen", "QuantGraph", "quant_graph.v1", CanonicalJson(quant_graph))`. The raw `QuantGraph` does not contain its own hash, so no self-hash zeroing is needed. Bitwise mixing is collision-hostile and underspecified. The same pattern applies to `infer_ir_self_hash`. | Should sub-hashes be combined for compactness?                                                | No. Use the F-B2/F-B4 `DomainHash` convention everywhere. |
| A53 | `TokenSrc` enum vs single `TokenInput` value. Earlier draft had a closed `TokenSrc` enum with two compile-time variants while also claiming "the IR shape is the same for both modes." | **Single `TokenInput` value with `allowed_ingress_modes` set.** Prompt-vs-autoregressive selection is a runtime ingress decision, not an IR shape decision. Resolves the contradiction. | Should the IR carry the variant?                                                              | No. The runtime binds the ingress mode at IR-pass entry; the IR shape is identical across modes. |
| A54 | Routed expert selection. Earlier draft computed all expert candidates but had no op consuming `GateWeight` and `RouterDecision`.            | **Add `InferOp::SelectExpertTop1`.** Without this op, the gate weight is dropped and the routed FFN math is wrong. `SelectExpertTop1` consumes the `RouterDecision`, `GateWeight`, and all expert candidate values; under canonical reference semantics it returns `weight * candidates[top1]`. | Should this be a new op or an extension of `CombineResidual`?                                | New op. `CombineResidual` stays `(x, δ) -> x ⊕ δ` and remains valid for both routed and dense paths after selection. |
| A55 | FFN nonlinear activation. Earlier draft split `ExpertMatVec` per slot but had no op for the `(gate, up) -> activation -> down` step.        | **Add `InferOp::FfnActivation { layer, expert }`.** Without it, `eval_canonical_ir` cannot match `eval_canonical_qg`'s SwiGLU/GeLU/ReLU semantics op-for-op. | Should activation be folded into `ExpertMatVec(FfnDown)` for compactness?                    | No. Op-level discipline (§2.6) requires it as a separate node. Fusion happens in scheduling. |
| A56 | Pre-FFN LayerNorm placement. Earlier draft had only one `Norm` per layer and the second norm (pre-FFN) was missing in `eval_canonical_qg`.  | **Add `LayerNorms { pre_sequence: NormPlanId, pre_ffn: NormPlanId }` per layer.** Each layer carries both norm plan ids; `NormPlanRecord` carries `NormSite` so `q.norm_plan(LayerFfn{ℓ})` is a named lookup, not positional. | Should the second norm be optional?                                                          | No. Standard transformer requires both. Architectures that omit one require an explicit RFC amendment. |
| A57 | KV slabs in `QuantTensorRole` vs in `SequenceSemanticsSpec`. Gemini suggested adding `KvSlab` to `QuantTensorRole`; Codex disagreed.         | **KV slabs live in `SequenceSemanticsSpec.state_slots`, not in `QuantTensorRole`.** Runtime KV slabs are not frozen artifact tensors and have no `BlobRef`. Adding them to `QuantTensorRole` would force a fake `BlobRef` for runtime state. | Where do KV slab shapes belong?                                                              | In `SequenceSemanticsSpec.state_slots` only. F-B5's `SequenceRead/Step/Write` ops reference these slots. |
| A58 | Reduction sites and F-B7 handshake. F-B7 (`RangePlan`) needs to correlate IR nodes with Stage 2's `ReductionSiteProjection`s.               | **Add `pub reduction_site: Option<ReductionSiteId>` to `GbNode`.** Set on every node satisfying `ReductionSiteBearing(op, q)` (`RouterMatVec`, `ExpertMatVec`, `Norm{plan ∈ TileRms*}`, `Classify`); `None` otherwise. F-B7 reads these ids. | Should this be a separate map or a node field?                                               | Field. Keeps the IR node self-contained and avoids a parallel map. |
| A59 | Saturation under `BitExact`. The plain DeterminismClass binding does not address mid-reduction clipping.                                    | **Forbid mid-reduction saturation under `BitExact`.** Saturation is permitted only at named boundaries (e.g. final activation clamp). `QuantGraphDeterminismRequiresEnforcedReductionOrder` extends to cover this. | Could mid-reduction saturation be allowed if pinned by an explicit `ReductionOrder`?         | Not in v1. Future amendment may relax if a precise `ReductionOrder + SaturationPolicy` schema lands. |
| A60 | Spec-pack D13 introduced "reports forbid RepairProposal provenance / AuthorizedRelaxation" — but this rule is only in the spec-pack, not in the normative body. | **Lift D13 to a normative invariant in §16 (O23).** Stage 1/3 diagnostics must not contain `RepairProposal` source or any `AuthorizedRelaxation` operation. F-B16 introduces those surfaces in a later RFC by amendment. | Where should the rule live?                                                                  | Both: normative §16 and spec-pack D13.                                                          |
| A61 | Earlier draft tried to evaluate dense FFN through `SelectExpertTop1` "the same path." That contradicted §9.6 SC-9, which forbids `Route*` / `Select*` nodes for dense layers. | **Updated.** Dense layers are *mathematically* equivalent to routed-with-prob-1.0, but the IR shape is direct: no `RouterMatVec` / `RouteTop1` / `SelectExpertTop1` for dense layers. §8.7 `eval_canonical_qg` and §9.8 `eval_canonical_ir` now branch on `ffn_kind(ℓ)`. | Should dense layers ever route at the IR level?                                                | No in v1. The math equivalence is a reference-semantics fact, not an IR-shape rule.            |
| A62 | Earlier draft had `RouteTop1` produce `RouterScore`, `RouterDecision`, and `GateWeight`, but `RouterScore` had no consumer. That orphaned the value and hid the router projection from F-B7. | **Add `RouterMatVec`.** `RouterMatVec` produces `RouterScore` (an `ExactAccumulator` reduction site). `RouteTop1` consumes `RouterScore` and produces `RouterDecision` + `GateWeight`. The router projection is now visible to F-B7. | Should router scoring be a separate op or stay folded into `RouteTop1`?                       | Separate. Op-level discipline (§2.6) requires it; F-B7 needs the reduction site.               |
| A63 | Earlier draft included `SharedDenseMatVec` + `SharedDenseWeight` + `SharedDenseBias` for "Mixed" topology. But "Mixed" was used in two senses: (a) some layers dense / some routed; (b) routed layers with a shared dense branch. Only (a) was defined. | **Removed.** Sense (a) — some-dense / some-routed — is the only v1 meaning of Mixed. Sense (b) — shared dense branch on routed layers — is out of scope for v1 and requires explicit RFC amendment to add the op + roles. | Should shared-dense-branch be in v1?                                                          | No. Underspecified; defer to a future amendment that defines topology, semantics, and shape.   |
| A64 | Earlier draft modeled `Embedding`'s external token as `ValueProducerRef::External(TokenInputId)` — but no `ValueDecl` existed for the external token, so its provenance was implicit and irregular. | **`TokenInput` carries an explicit `value_id`.** A `ValueDecl` with kind `InputToken` and format `TokenIdDomain` is the unique external value, consumed like any other node input. `ValueProducerRef::External(TokenInputId)` still exists for provenance, pointing back to the `TokenInput`. | Should the embedding's input be a special-case path?                                          | No. Make it a regular `ValueId`; validators stay closed.                                       |
| A65 | Earlier draft used `TokenIdWidth` and `ExpertIdWidth` (physical widths) as `ValueFormat` variants. That is a storage decision and contradicts §2.3. | **Replaced with `TokenIdDomain { vocab_size }` and `ExpertIdDomain { n_experts }`.** Physical width is a storage decision owned by F-B7+. | Should the IR commit to widths?                                                                | No. Domains only.                                                                              |
| A66 | Earlier draft had `ValueFormat::Unit` for effect-only outputs. That conflated "no value output" with "a value of unit type." | **Removed `Unit`.** Effect-only producers (`SequenceWrite`) have empty `outputs: []` and only produce effect tokens. | Should `Unit` survive for shape uniformity?                                                    | No. Empty `outputs` is more honest.                                                            |
| A67 | Earlier draft said `SequenceStep { layer, slot }`, implying one step per slot. That conflicts with the `sequence_block` reference, which produces one block output plus all next-state values. | **`SequenceStep { layer }` (no slot).** A single `SequenceStep` per layer consumes all `SequenceStateRead` values for that layer's slots and produces one `SequenceBlockOutput` plus one `SequenceStateNext` per slot. | Should `SequenceStep` be per-slot for parallelism?                                            | No. Per-layer matches the reference semantics; per-slot would multiply nodes spuriously.       |
| A68 | Earlier draft said RNG chain spans Classify -> DecodeToken. Classify is pure with respect to RNG. | **Updated.** RNG chain consists of one root token consumed and one final token produced by the unique `DecodeToken` node when `requires_rng = true`. Classify is pure. | Could Classify ever consume RNG?                                                              | No in v1. Classify is a deterministic projection. RNG enters only at decode.                  |
| A69 | Earlier draft did not name the residual quantization boundary, so `BitExact` saturation rules were ambiguous. | **Add `ResidualPlan` to `QuantGraph`** with `activation_format` and `combine_policy`. `CombineResidual` is a named numeric boundary; `BitExact` permits saturation here and forbids it mid-reduction. | Should the IR still combine in `ExactAccumulator`?                                            | Yes. The named boundary clamps the residual back to `activation_format` after the add.         |
| A70 | Earlier draft said Stage 3 reads `ResolvedCompilePolicy` "fields"; `K3` included `policy_resolution_self_hash`, making the cache sensitive to arbitrary unrelated policy drift. | **Add `InferIrPolicyProjection`.** Stage 3 reads only the projection; `K3` keys off the projection hash; `policy_resolution_self_hash` is preserved in `InferIrIdentity` for audit only. | Should Stage 3 cache invalidate on every policy edit?                                          | No. Only on projection drift.                                                                  |
| A71 | Earlier draft had `ArtifactResolver` in the pure core's input. Filesystem-backed resolution is IO. | **Add `ResolvedBlobIndex`.** The pure core receives a hash-bound metadata view. The Stage 1 driver builds the index using IO; the core never opens a file. | Should the pure core ever do IO?                                                              | No. Cores are pure; drivers do IO.                                                             |
| A72 | Earlier draft assigned `NodeId` after canonical sort, but assigned ordering used `node_id` as a fallback tiebreaker — circular. | **`NodeId` is assigned after a fully canonical sort key** that includes provenance tuples (op_tag, layer, expert, slot, norm_site, state_slot, residual_site, occurrence_index). No `node_id` appears in the sort key. | Should occurrence_index be derived from input order?                                           | Yes — based on the order the construction class iterates inputs (deterministic).               |
| A73 | Earlier draft did not enforce op signatures uniformly; output-only predicates were not enough. | **Add §9.7a op signature predicate.** Every node must satisfy `op_signature(op, q)`. Validators implement it as a closed match. | Should signatures live in the type system?                                                   | They are pinned in the RFC. Type-level enforcement (e.g. via Rust phantom types) is an implementation choice. |
| A74 | Earlier draft had `provenance: ValueProvenance` inline on `ValueDecl` and `provenance: EffectProvenance` inline on `EffectDecl`, while §2.8 said provenance lives on the IR product. The duplication was contradictory and `ValueProvenance` was never defined. | **Removed inline provenance.** `ValueDecl` and `EffectDecl` no longer carry inline provenance; `g.provenance.values` (`ValueProducerRef`) and `g.provenance.effects` (`EffectProvenance`) are the single source of truth. | Should some provenance survive inline for convenience?                                       | No. Single-source-of-truth wins.                                                               |
| A75 | Earlier draft made `SequenceStep { layer, slot }` opaque: it could carry nontrivial sequence compute, but `QuantGraph` had no sequence weight roles, F-B7 had no sequence reduction sites, and the v1 closure gate was unverifiable. | **Reserved-not-emitted in v1 (§2.5a).** `SequenceRead` / `SequenceStep` / `SequenceWrite` remain in `InferOp` as reserved shape for the sequence-state amendment. Stage 3 emits them only when the artifact declares an identity sequence block for every layer; otherwise rejects with `InferIrSequenceSemanticsUnsupportedV1`. | Should sequence compute land in v1?                                                          | No. Sequence-state bring-up is M4 territory.                                                  |
| A76 | Earlier draft included `policy_resolution_self_hash` and `compile_request_hash` in `InferIrPolicyProjection`, but §9.10 said they are audit-only — every unrelated policy edit invalidated `K3`. | **Split into `InferIrPolicyProjection` (load-bearing: `requested_runtime_modes` only) and `InferIrAuditParents` (`policy_resolution_self_hash`, `compile_request_hash`).** Audit parents land in `InferIrReportBody.input_identity`, not in the embedded product. | Should the cache be sensitive to all policy edits?                                            | No. Only to load-bearing projection edits.                                                    |
| A77 | Earlier draft had `quant_graph_self_hash` defined as `DomainHash(... CanonicalJson(product-with-self-hash-zeroed))`, while `QuantGraphProduct` actually contains the hash. That was either circular or required an undefined zeroing convention. | **Hash the raw IR struct.** `quant_graph_self_hash = DomainHash("gbf-codegen", "QuantGraph", "quant_graph.v1", CanonicalJson(quant_graph))`. The raw `QuantGraph` does not contain its own hash, so no zeroing is needed. Same for `infer_ir_self_hash`. | Should the hash cover the report envelope?                                                   | No. The IR self-hash is over the IR struct only; envelope/report self-hashes are separate.    |
| A78 | Earlier draft validated blob existence through `i.blob_index` but `QuantTensorRef` only stored `BlobRef`; the resolved hash/size/codec metadata was not part of the product, so `QuantGraph` was not actually content-addressed. | **`QuantTensorRef.blob: ResolvedBlobRef`** (and same on aux refs) carries content_hash + encoded_size_bytes + decoded_size_bytes + codec. `ResolvedBlobIndex.self_hash` lands in `K1`. `BlobRef`-as-content-address is now an explicit invariant. | Should we instead define `BlobRef` as content-addressed?                                     | No. Carry the resolved metadata explicitly so the product is self-describing.                 |
| A79 | Earlier draft kept `ClassifyHead.output_format` but emitted `LogitVector in ExactAccumulator`, ignoring it. Worse, "tied" classify equality required output format equality with the embedding table — but the actual constraint is on the weight, not the logit format. | **Renamed `output_format` → `logit_format`** and used it in `Classify`'s output predicate as a named numeric boundary (`ClassifyLogitBoundary`). The Tied rule now constrains only the weight identity. | Should logits stay in `ExactAccumulator` and be clamped only later?                          | No. The artifact already declares a logit format; honor it.                                   |
| A80 | Earlier draft routed `FfnActivation`'s output through "by NormPlan or activation kind", which was not a precise format. F-B7 cannot reduction-plan a value whose format is not pinned. | **Add `FfnPlan { activation_kind, intermediate_format }`** per layer. `FfnActivation` outputs `Quant(q.ffn_plans[ℓ].intermediate_format)` at a named boundary. | Should activation_kind move to a per-section field?                                          | No. Per-layer is the right scope for v1.                                                      |
| A81 | Earlier draft pinned `RouterSemantics::Top1Hard` but did not pin tie-break behavior; `BitExact` argmax requires deterministic ties. | **Add `RouterTieBreak { LowestExpertId }`** to `RouterSemantics::Top1Hard`. Reference semantics resolve ties using this rule. | Should other tie-break strategies exist?                                                    | Not in v1. Future amendments may add `HighestScoreFirst` etc.                                 |
| A82 | Earlier draft said Stage 3 looks up `ReductionSiteId` "by matching" Stage 2's projections, but did not define the matching key. | **Add `ReductionSiteKey`** with typed cases per reduction-bearing op (`RouterMatVec`, `ExpertMatVec`, `Norm`, `Classify`). F-B5 computes the key from QG entities and looks up Stage 2's `reduction_site_id`. | Should F-B5 mint its own ids?                                                                | No. Stage 2 is the authoritative source.                                                      |
| A83 | Earlier draft said "no fusion" but every `Norm`, `Classify`, `RouteTop1` etc. is composite (affine, normalization, clamp). The rule was too strict to apply. | **Renamed to "no scheduling fusion"** and clarified that bias / affine / score normalization / named numeric boundaries inside canonical sites are not fusions. Cross-site fusion remains forbidden. | Should the rule cover any other in-op composites?                                            | They are listed exhaustively in §2.6.                                                          |
| A84 | Earlier draft had `BitExact` saturation language with a future-looking exception for explicit `ReductionOrder + SaturationPolicy`. A59 said "not in v1" but the §2.10 prose still hedged. | **Strict in v1.** Mid-reduction saturation is forbidden. Saturation is permitted only at named boundaries (residual, classify logit, FFN activation, final clamp). Diagnostic: `QuantGraphBitExactMidReductionSaturationForbidden`. | Should v1 carry a deferred-saturation knob?                                                  | No. Defer to a future amendment.                                                              |
| A85 | Earlier draft made `IIR-Ok-11` a universal postcondition for every `BitExact` build, but §9.3 / §10.2 allowed skipping in non-fixture builds. | **Made the postcondition report-driven.** `IIR-Ok-11` now reads off `g.report.body.result.fixture_equivalence`: `VerifiedFixtureBitExact` ⇒ equivalence holds; `Skipped { reason }` ⇒ no claim. The chunk-closure fixture build must produce `VerifiedFixtureBitExact`. | Should non-fixture BitExact builds also verify?                                              | No, by default. Fixture build is the closure gate.                                            |
| A86 | Earlier draft had stale `i.policy.report.outcome` and `i.policy.requested_runtime_modes` references in IIR preconditions, plus an `O21` proof obligation citing `i.resolver`. Both surfaces were renamed/replaced. | **Patched.** IIR preconditions now reference `i.audit_parents.policy_resolution_self_hash` and `i.policy_projection.requested_runtime_modes`; `O21` now cites `ResolvedBlobIndex`. | Should audit-parent surfaces have been kept?                                                 | They are kept, but moved to audit-only (`InferIrAuditParents` / `InferIrReportBody.input_identity`). |

---

# Spec pack: F-B3 + F-B5 (compact formalization)

```text
Spec:
  F-B3/F-B5 Canonical IRs

Owns:
  Stage 1   QuantGraph
  Stage 3   GbInferIR
  Schema    quant_graph.v1
  Schema    infer_ir.v1
  Cache     K1, K3
  Migration F-B4 placeholder retirement

Does not own:
  Observation/probe selection         (F-B6)
  Reduction-plan structure            (F-B7)
  Storage class / lifetime / aliasing (F-B8)
  Spatial residency                   (F-B9, F-B10, F-B11)
  Arena byte ranges                   (F-B12)
  Schedule slices / leases            (F-B13)
  Schedule cost                       (F-B14)
  Backend / placement / encoding      (F-B15)
  Refinement loop                     (F-B16)
  Conformance envelope                (F-C4)
  ArtifactOracle implementation       (F-C2)
  Stage cache infra                   (F-A6 — already CLOSED)

Chosen normalization decisions:

D1. quant_graph.json and infer_ir.json inherit ReportEnvelope + canonical JSON
    + self-hash from F-B2/F-B4 unchanged.

D2. QuantGraph construction is a pure function of ValidatedInputs +
    ResolvedCompilePolicy + ArtifactCore + SequenceSemanticsSpec.

D3. GbInferIR construction is a pure function of QuantGraph,
    StaticBudgetProduct, and InferIrPolicyProjection. Determinism is not
    read from policy; it is inherited from QuantGraph.identity.determinism.

D4. Provenance is total and lives on the IR product:
    QuantGraph: TensorId → ExportTensorId, plus aux ExportTensorId on
                every QuantAuxBlobRef.
    GbInferIR:  g.provenance.nodes:   NodeId   → QuantGraphEntityRef
                g.provenance.values:  ValueId  → ValueProducerRef
                g.provenance.effects: EffectId → EffectProvenance
    Provenance is NOT stored inline on GbNode/ValueDecl/EffectDecl.

D5. SequenceSemanticsSpec is consumed (not constructed) by F-B3.

D6. Routing topology is artifact-driven: dense layers carry one
    ExpertSection (expert == 0); routed layers carry routing_table entries.

D7. EffectClass is closed in v1: SequenceState(slot), Rng(slot), FaultBoundary.
    SemanticCheckpoint emission is owned by F-B6.

D8. F-B5 is storage-free; no tile sizes, buffer addresses, page ids,
    accumulator widths, materialization, alias classes.

D9. FixtureSemanticEquivalence(g, q) is verified only for BitExact
    artifacts on the synthetic fixture input set.
    UniversalSemanticEquivalence and weaker determinism classes are
    owned by F-C2 / F-C4.

D10. F-B4's placeholder QuantGraphBudgetSource is retired; F-B4 fixtures
     continue to pass with QuantGraph as the real source.

D11. Single-token convention: exactly one Embedding, one Classify, one
     DecodeToken per IR pass.

D12. No scheduling fusion at the IR level; no partial layers. Bias
     inside affine ops, affine inside `NormPlan`, score normalization
     inside `RouteTop1`, and named numeric clamp boundaries (residual,
     classify logit, FFN activation) are part of canonical op semantics
     and are not scheduling fusions.

D13. Stage1 and Stage3 reports reject Soft diagnostics in this chunk.
     RepairProposal provenance and AuthorizedRelaxation are forbidden in
     Stage 1 and Stage 3 reports for this chunk (also normative as O23
     in §16).

D14. F-C2 (ArtifactOracle) gains an explicit dependency edge to F-B5
     during this chunk; F-C2 may begin against QuantGraph alone but
     full op-for-op correspondence requires GbInferIR.

D15. ExpertMatVec is split per ExpertWeightSlot (FfnGate? → FfnUp → FfnDown);
     fusion happens in scheduling.

D16. Norm and DecodeToken nodes carry Plan ids; full plan bodies live in
     QuantGraph.

D17. No semantic checkpoints in GbInferIR. F-B6 attaches them via NodeAnchorMap.

D18. RngSlot is closed in v1: { Decode } only.

D19. RouterSemantics is closed in v1: { Top1Hard } only.

D20. Token ingress is bound at runtime: `Embedding` consumes a single
     `TokenInput` value with `allowed_ingress_modes` set; the IR shape
     is invariant under prompt-vs-autoregressive selection.

D21. Reports are canonical product-bearing. `quant_graph.json` and
     `infer_ir.json` include the full IR product under
     `body.result.product`; summary fields are redundant review aids.

D22. Self-hashes use `DomainHash`. Bitwise mixing of sub-hashes
     (`⊕`-style) is forbidden. `quant_graph_self_hash`, `K1`, `K3`, and
     `topological_order_hash` all use the F-B2/F-B4 `DomainHash`
     convention.

D23. Routed FFN includes `RouterMatVec`, `RouteTop1`,
     `SelectExpertTop1`, and `FfnActivation` ops. Dense FFN is
     mathematically equivalent to routed-with-probability-1.0-on-expert-0,
     but its `GbInferIR` shape is direct: no `RouterMatVec`, `RouteTop1`,
     or `SelectExpertTop1` node is emitted for dense layers.

D24. Per-layer `LayerNorms { pre_sequence, pre_ffn }` is mandatory.
     `NormPlanRecord` carries `NormSite` so reference semantics use
     named lookup, not positional indexing.

D25. F-B5 reads `DeterminismClass` from `QuantGraph.identity.determinism`
     only. Strict linear data flow: ArtifactCore → QG → IIR.

D26. `ValueFormat::ExactAccumulator` (IR-side) is the sentinel for
     pre-RangePlan accumulator values. `QuantFormat` (artifact-side)
     never gets an `AccumulatorTBD` variant.

D27. KV slabs and per-layer recurrent state are declared in
     `SequenceSemanticsSpec.state_slots`; they are not `QuantTensorRef`s
     and have no `BlobRef`.

D28. `FaultBoundary` is reserved but never emitted in v1; future
     repair/refinement RFC amends if needed.

D29. `GbNode.reduction_site: Option<ReductionSiteId>` is set on every
     node satisfying `ReductionSiteBearing(op, q)` — i.e. `RouterMatVec`,
     `ExpertMatVec`, `Norm{plan ∈ TileRms*}`, `Classify` — so F-B7
     (RangePlan) can correlate with Stage 2's `ReductionSiteProjection`.

D30. Pure-function shape: `build_quant_graph_core` /
     `build_infer_ir_core` are pure; IO is isolated in
     `run_stage1` / `run_stage3` drivers. The pure cores receive a
     `ResolvedBlobIndex` (immutable, hash-bound) instead of an
     `ArtifactResolver`.

D31. Routing has its own matvec op. Routed layers emit
     `RouterMatVec` -> `RouteTop1` -> Experts -> `SelectExpertTop1`.
     Dense layers emit Experts directly (no router/select nodes). The
     routed-vs-dense math is equivalent under canonical reference
     semantics, but the IR shapes are different.

D32. `RouterSemantics::Top1Hard` carries explicit
     `RouterGateWeightSemantics` (`One` or `SelectedScore`) so the gate
     weight's numerical contract is unambiguous for oracle equality.

D33. `QuantGraph` carries an explicit `ResidualPlan` with
     `activation_format` and `combine_policy`. `CombineResidual` is the
     **only** named numeric saturation boundary in the IR (along with
     final activation clamps); `BitExact` permits saturation here and
     forbids it elsewhere.

D34. `InferIrPolicyProjection` is the minimal IR-shape-bearing
     projection: `{ requested_runtime_modes }`. `K3` is sensitive only
     to projection drift. `policy_resolution_self_hash` and
     `compile_request_hash` are audit parents, recorded in
     `InferIrAuditParents` and in `InferIrReportBody.input_identity`,
     and never invalidate `K3`.

D35. Op signatures are a closed predicate (§9.7a). Validators implement
     `op_signature(op, q)` as a closed match over `InferOpTag`; any
     violation is `InferIrOpSignatureMismatch`.

D36. `TokenInput` carries an explicit `value_id`; the unique `Embedding`
     node consumes that `ValueId` like any other node-input. There is
     no special-case external-value short-circuit in the validator.

D37. `ValueFormat` carries semantic domains (`TokenIdDomain`,
     `ExpertIdDomain`), not physical widths; physical widths are storage
     decisions owned by F-B7+.

D38. `QuantGraphBudgetSource` is implemented on `QuantGraphProduct`,
     not on `QuantGraph` itself (the IR struct does not contain its own
     hash). The trait reads `self.quant_graph_self_hash` and
     `self.quant_graph.identity.semantic_core_hash`.

D39. `FixtureSemanticEquivalence` (chunk closure gate) is distinct from
     `UniversalSemanticEquivalence` (deferred to F-C2 / F-C4). The
     report tag is `VerifiedFixtureBitExact | Skipped { reason }`.

D40. `QuantGraphBudgetView` (deployed in F-B4) carries
     `shared_dense_ffn: Option<SharedDenseFfnProjection>` as forward-
     compat surface. F-B3's `to_budget_view()` MUST emit
     `shared_dense_ffn = None` in v1. SharedDense* roles, ops, and
     tensor coverage remain out of scope for v1 (A63 unchanged).

D41. `ReductionSiteId` is `pub struct ReductionSiteId(pub String)` in
     `gbf-policy::diagnostics` (deployed). F-B3 mints the string
     using a canonical scheme; F-B5 reconstructs the same scheme to
     look up the matching `ReductionSiteProjection`. Stage 2 records
     and validates; it does not mint.

D42. `StaticBudgetReport` (NOT `StaticBudgetProduct`) is the deployed
     wrapper type around `ReportEnvelope<StaticBudgetReportBody>`.
     `static_budget_self_hash` is computed as the `report_self_hash`
     of that envelope (post-`with_computed_self_hash`).

D43. `gbf_report::ReportBody::validate_semantics(&self,
     outcome: ReportOutcome)` is the deployed signature (outcome is
     a parameter, not derived from `self`). The trait also carries
     `REPORT_TYPE`, `SCHEMA_ID`, and `SCHEMA_VERSION` `&'static str`
     constants. `quant_graph.v1` and `infer_ir.v1` `ReportBody` impls
     follow the same shape.

D44. `RoutingProjection.model: RoutingModelSection { kind: String }`
     is a string-typed Stage 2 summary (e.g. `"synthetic-top1"`).
     F-B3/F-B5 own the typed `RouterSemantics::Top1Hard` independently;
     the projection's `kind` is informational and never load-bears
     the IR shape.
```

## 20. Post-implementation reconciliation (F-B2/F-B4 landed 2026-05-10, PR #14)

This section was added after PR #14 (`5c29aaf`) merged the full F-B2/F-B4
implementation into `main`. The RFC's pre-implementation prose was a
contract negotiated against placeholder seams; the deployed shapes
diverged in five small but load-bearing ways. Each delta is captured
below with: *what the RFC pinned*, *what shipped*, and *which clause
governs going forward* (so future readers can resolve apparent
contradictions without re-reading the entire chunk).

This section is **normative for F-B3 and F-B5 implementation work**.
Where it conflicts with §8 / §9 / §13 prose, this section wins; the
upstream prose is preserved as the original design record.

### 20.1 `shared_dense_ffn` carrier exists but is always `None` in v1

F-B4's deployed `QuantGraphBudgetView` (`gbf-codegen::budget`):

```rust
pub struct QuantGraphBudgetView {
    pub semantic_core_hash: Hash256,
    pub quant_graph_hash: Hash256,
    pub layers: Vec<LayerId>,
    pub experts: Vec<ExpertProjection>,
    pub shared_kernels: Vec<SharedKernelProjection>,
    pub shared_luts: Vec<SharedLutProjection>,
    #[serde(default)]
    pub shared_dense_ffn: Option<SharedDenseFfnProjection>,    // <-- forward-compat
    pub reduction_sites: Vec<ReductionSiteProjection>,
    pub sequence_state: SequenceStateProjection,
    pub routing: RoutingProjection,
}
```

The `shared_dense_ffn` field is forward-compat surface for a future
shared-dense-FFN topology amendment. **In v1, F-B3's
`to_budget_view()` implementation MUST emit
`shared_dense_ffn: None`.** Ambiguity A63 is unchanged: shared-dense
branches require an explicit RFC amendment that defines:

* A `SharedDenseMatVec` op variant (and matching `op_signature`).
* `QuantTensorRole::SharedDenseWeight` / `SharedDenseBias` variants.
* The IR-shape rule for routed-with-shared-dense layers
  (currently §9.6 SC-9 forbids any router/select op on dense layers
  and forbids any expert op on routed layers' shared branch).

T-B3.23 (F-B4 placeholder retirement, `bd-1r2b`) inherits this rule:
the trait impl must always emit `None` and the v1 fixtures
(T-B3.26, `bd-2sp5`) must include a regression that fails if a
non-`None` value is ever returned.

Diagnostic: any `shared_dense_ffn != None` reaching Stage 2 is a
malformed builder; F-B4 already validates this through
`QuantGraphBudgetViewError::Malformed`. F-B3's own validator must
mint a typed reject before it propagates that far.

### 20.2 `ReductionSiteId` minting authority moves to F-B3

The RFC §13.4 prose said "F-B5 does not mint new `ReductionSiteId`s.
It computes the `ReductionSiteKey` for every reduction-bearing node
and looks it up in the passed Stage 2 product." (Ambiguity A82.)

What shipped:

```rust
// gbf-policy::diagnostics
pub struct ReductionSiteId(pub String);

// gbf-codegen::budget
pub struct ReductionSiteProjection {
    pub site: ReductionSiteId,             // <-- string-typed, minted upstream
    pub layer: Option<LayerId>,
    pub expert: Option<ExpertId>,
    pub term_count: u32,
    pub input_max_abs_q: u32,
    pub weight_max_abs_q: u32,
    pub bias_max_abs_q: Option<u32>,
    pub accumulator_domain: AccumulatorDomain,
}
```

`ReductionSiteId` is opaque-string. There is no schema field carrying
a `ReductionSiteKey` enum; the projection has only `(site, layer,
expert, ...)` plus accumulator-domain metadata. **The minting
authority therefore lives with whoever produces the
`QuantGraphBudgetView`** — i.e. the `QuantGraphBudgetSource` impl
(F-B3's `QuantGraphProduct::to_budget_view()`). F-B4 records and
validates ordering / uniqueness; F-B5 reconstructs the same canonical
scheme to perform a string lookup.

The canonical scheme that F-B3 v1 mints (and F-B5 reconstructs):

```text
RouterMatVec{layer = ℓ}                     -> "router.<ℓ>"
ExpertMatVec{layer = ℓ, expert = e, slot}   -> "expert.<ℓ>.<e>.<slot_tag>"
                                               where slot_tag ∈ {gate, up, down}
Norm{plan = NormPlanId(p)}                  -> "norm.<p>"
                                               (with NormSite already
                                               disambiguated by NormPlanId
                                               since SC-20 makes the
                                               site → NormPlanId map
                                               injective)
Classify                                    -> "classify"
```

F-B5 builds the same string for a given reduction-bearing node and
matches against `static_budget.report.body.result.budget_view
.reduction_sites[i].site` in O(n). Missing or duplicate matches fail
with `InferIrReductionSiteMissing` (T-B5.13, `bd-qwto`).

The `ReductionSiteKey` enum from §13.4 is reframed as **F-B5's
internal correlation predicate** — it lives in `gbf-codegen` only,
not in any schema, and its sole purpose is to assert
`canonical_reduction_site_id_string(key) == projection.site` during
binding. The `*_weight: TensorId` fields on `ReductionSiteKey` (which
have no counterpart in the deployed `ReductionSiteProjection`) are
diagnostic-only: F-B5 can include them in `InferIrReductionSiteMissing`
to make the failure message actionable, but they are NOT part of the
match.

Ambiguity A82's "Stage 2 is the authoritative source" guidance is
amended to: *the canonical id-minting scheme is the authoritative
source, and F-B3 implements it on the producer side*. F-B5 still
does not mint independently — it derives.

### 20.3 `StaticBudgetReport` is the deployed name; no separate `Product`

The RFC referenced `StaticBudgetProduct` as the F-B5 input and
`static_budget_self_hash` as a separate hash field. What shipped:

```rust
// gbf-codegen::budget
pub struct StaticBudgetReport {
    pub report: ReportEnvelope<StaticBudgetReportBody>,
}
```

There is no separate `StaticBudgetProduct` type. The
`StaticBudgetReport` wraps a single `ReportEnvelope`; the
`report_self_hash` field on the envelope is the
`static_budget_self_hash` referenced throughout F-B5's design.

F-B5's input contract therefore resolves as:

```rust
pub struct GbInferIRInputs<'a> {
    pub quant_graph: &'a QuantGraph,
    pub quant_graph_self_hash: Hash256,
    pub policy_projection: InferIrPolicyProjection,
    pub audit_parents: InferIrAuditParents,
    pub static_budget: &'a StaticBudgetReport,                 // <-- not Product
    pub static_budget_self_hash: Hash256,
        // == static_budget.report.report_self_hash
}
```

IIR-Pre-3 (`i.static_budget_self_hash` must equal
`i.static_budget.self_hash`) is concretely:
`i.static_budget_self_hash == i.static_budget.report.report_self_hash`.

T-B5.9 (`bd-7crx`), T-B5.13 (`bd-qwto`), T-B5.21 (`bd-37d1`) inherit
this naming; their bead bodies refer to `StaticBudgetProduct` for
historical reasons but the deployed type is `StaticBudgetReport`.

### 20.4 `ReportBody::validate_semantics` takes `outcome` as a parameter

F-B2's deployed `gbf-report::canonical_json`:

```rust
pub trait ReportBody: Sized {
    const REPORT_TYPE: &'static str;
    const SCHEMA_ID: &'static str;
    const SCHEMA_VERSION: &'static str;

    fn validate_semantics(
        &self,
        outcome: ReportOutcome,
    ) -> Result<(), Vec<ValidationDiagnostic>>;
}
```

Three implementation details that affect F-B3/F-B5 schema work
(T-B3.0 `bd-23xt`, T-B3.24 `bd-30wi`, T-B5.0 `bd-1eb0`,
T-B5.18 `bd-3sv0`):

* `validate_semantics(&self, outcome)` takes `outcome` as a parameter.
  The QG-2/IIR-2 `outcome ⇔ result.is_some()` invariant is enforced
  inside `validate_semantics`, not by a separate cross-check. A
  `Passed` envelope with `result = None` (or vice versa) emits a
  `Vec<ValidationDiagnostic>` with one Hard diagnostic.

* `REPORT_TYPE` is a third `&'static str` constant beyond `SCHEMA_ID`
  and `SCHEMA_VERSION`. For `quant_graph.v1` and `infer_ir.v1` it is
  `"quant_graph"` and `"infer_ir"` respectively (the schema id minus
  the version suffix).

* The envelope's `report_self_hash: Hash256` field is set to
  `Hash256::ZERO` during canonical-JSON serialization for hashing,
  then back to the computed hash via `with_computed_self_hash()`.
  The raw IR struct (`QuantGraph`, `GbInferIR`) still does **not**
  carry its own self-hash (Ambiguity A77 unchanged); the hash lives
  on the report envelope and on `QuantGraphProduct` /
  `GbInferIRProduct` wrappers.

`compute_self_hash<R>(env)` and `round_trip_self_hash<R>(env)` are
the deployed helpers (in `gbf-report::canonical_json`); F-B3/F-B5
should reuse them rather than reinvent.

### 20.5 `RoutingProjection.model.kind` is a Stage 2 summary string

```rust
// gbf-codegen::budget
pub struct RoutingProjection {
    pub model: RoutingModelSection,
    pub projected_bank_switches_per_token: u16,
    pub expected_bank_switches_q16_16: Option<u32>,
}

pub struct RoutingModelSection {
    pub kind: String,             // e.g. "synthetic-top1"
}
```

`RoutingModelSection.kind` is a Stage 2 summary string for
diagnostic / dashboard consumption. **It does not load-bear the IR
shape.** F-B3 binds the typed `RouterSemantics::Top1Hard { gate_weight,
tie_break }` (T-B3.5, `bd-22c5`) and F-B5 enforces v1 closure on the
typed enum (T-B3.22a, `bd-2yx8`). The projection's `kind` field is
populated for review tooling and may be `"synthetic-top1"` even when
the QG-side router semantics are `RouterSemantics::Top1Hard
{ gate_weight: SelectedScore, tie_break: LowestExpertId }`. The
string is informational, not load-bearing — F-B5 never reads it.

If a future amendment introduces additional `RouterSemantics`
variants, the `kind` string MAY widen accordingly, but the typed
enum remains the source of truth.

### 20.6 Other deployed-shape notes

These do not require a normative reframing but are worth recording so
F-B3/F-B5 implementations match the wire shape:

* **`ResolvedPolicyProduct`** (in `gbf-codegen::policy`) is the
  deployed name for the F-B2 Stage-0.5 product type; the
  `BudgetInputs<'a, Q: QuantGraphBudgetSource + ?Sized>` field
  `policy: &'a ResolvedPolicyProduct` is its consumer surface. T-B3.18
  DecodeBinding (`bd-2l1s`) and T-B5.11 EffectAllocation
  (`bd-f8hv`) read decode/RNG flags from this type.

* **`AccumulatorDomain { RawIntegerProducts, PostScaleQ8_8,
  PostScaleQ16_16 }`** is the closed enum carried by every
  `ReductionSiteProjection`. F-B7 (RangePlan, future) consumes it to
  pick `SingleI16` / `ChunkedI16` / `RenormLoop` per site; F-B5's
  `ValueFormat::ExactAccumulator` is the IR-side sentinel that
  declares "implementation chosen later", and the two are not
  in 1:1 correspondence (the projection carries domain info; the IR
  carries the deferred-choice marker).

* **`ScaleFormatByteWidths`** is the deployed type that pins
  target-profile-overridable scale-byte widths (e.g. `Pow2 = 1` byte
  by default). F-B3's tensor binding (T-B3.14, `bd-1q8k`) does not
  touch this directly — it is owned entirely by F-B4's per-expert
  byte math. F-B3 reads the artifact's declared `scale_format` and
  trusts F-B4 to apply target-profile overrides when projecting.

* **`SwitchProjectionSource`** (referenced from
  `BankSwitchesPerTokenOverCap` and
  `SramPageSwitchesPerTokenOverCap` BudgetFailure variants) is a new
  diagnostic enum from PR #14. F-B3 produces no switch projections;
  F-B5 reads no switch projections. Cross-stage handshake is unaffected.

* **`gbf-artifact::TernaryWeightPlan::compute_byte_cost`** is now
  scoped to "target-independent artifact/model diagnostics" only
  (per the F-B2/F-B4 RFC amendment in the same PR). Stage 2's
  `expert_payload_bytes` is the canonical deployed byte-cost owner.
  F-B3 does not call either: its tensor-binding size validation
  (QG-SC-13) uses `expected_decoded_tensor_payload_size(layout,
  quant_format, role)` (T-B3.14), which is an F-B3-internal helper
  with its own canonical formula. The three byte-math owners
  (artifact diagnostic, F-B4 deployed projection, F-B3 SC-13 size
  check) deliberately serve different purposes and are not unified
  in v1.

### 20.7 Bead consolidation

The chunk's task DAG was originally minted as 57 fine-grained beads
(T-B3.0..T-B3.28, T-B5.0..T-B5.22) one-per-construction-class. After
F-B2/F-B4 landed and the implementation surface stabilized, the bead
set was consolidated to **22 thematic anchors** (closed-on-merge:
35 beads, retained: 22) on the same date. The §14 task DAG below
reflects the consolidated structure; the closed beads' bodies remain
visible via `br show <id>` for the full pre-consolidation prose.

Anchor map (post-consolidation):

```text
Wave 0 — Schema preludes (2):
  T-B3.0  bd-23xt  quant_graph.v1 ReportEnvelope
  T-B5.0  bd-1eb0  infer_ir.v1 ReportEnvelope

Wave 1 — F-B3 types (4):
  T-B3.1  bd-8rot  QG core + tensors + ResolvedBlobIndex + provenance
                     (absorbs T-B3.2 / T-B3.2a / T-B3.11)
  T-B3.3  bd-n6av  NormPlanRecord + NormSite + LayerNorms
                     (absorbs T-B3.4)
  T-B3.5  bd-22c5  Routing + ExpertSection + ExpertWeightSlot
                     + FfnPlan + topology tag
                     (absorbs T-B3.6 / T-B3.7 / T-B3.10)
  T-B3.9  bd-3del  ClassifyHead + DecodeSpecRecord
                     (absorbs T-B3.8)

Wave 2 — F-B3 construction (6):
  T-B3.12 bd-o3jg  Identity + SequenceSemantics + NormPlanIdPre
                     + TensorBinding (Wave 2 classes 1–4;
                     absorbs T-B3.13 / T-B3.13a / T-B3.14)
  T-B3.15 bd-2yye  Norm / LayerNorms / Routing / Expert / Residual
                     / Decode / Classify bindings
                     (Wave 2 classes 5–11;
                     absorbs T-B3.16 / T-B3.17 / T-B3.17a
                     / T-B3.18 / T-B3.19)
  T-B3.20 bd-u8mu  Provenance + CanonicalSort + SelfConsistency
                     + RouterSemantics check
                     (Wave 2 classes 12–14;
                     absorbs T-B3.21 / T-B3.22 / T-B3.22a)
  T-B3.23 bd-1r2b  F-B4 retirement + quant_graph.v1 schema
                     + StageCache K1
                     (absorbs T-B3.24 / T-B3.25)
  T-B3.26 bd-2sp5  Synthetic dense + routed QG fixtures
                     + per-reject-class fixtures
  T-B3.27 bd-ou73  build_quant_graph_core / run_stage1 split

Wave 3 — F-B5 types (3):
  T-B5.1  bd-3m1j  GbInferIR + GbNode + InferOp + Value{Decl,Kind,Format}
                     (absorbs T-B5.2 / T-B5.3)
  T-B5.4  bd-1cin  EffectDecl + EffectClass + RngSlot + TokenInput
                     (absorbs T-B5.5 / T-B5.6)
  T-B5.7  bd-i19q  InferIrProvenance + NodeAnchorMap
                     (absorbs T-B5.8)

Wave 4 — F-B5 construction (5):
  T-B5.9  bd-7crx  Identity + TokenInput + Value/Effect alloc
                     + NodeBuilding + OpSignature
                     (Wave 4 classes 1–5;
                     absorbs T-B5.10 / T-B5.11 / T-B5.12 / T-B5.12a)
  T-B5.13 bd-qwto  ReductionSiteBinding + Provenance/Anchor
                     + CanonicalSort + SelfConsistency
                     (Wave 4 classes 6–10;
                     absorbs T-B5.14 / T-B5.15 / T-B5.16)
  T-B5.17 bd-3lmy  SemanticEquivalenceCheck + infer_ir.v1 schema
                     + StageCache K3
                     (absorbs T-B5.18 / T-B5.19)
  T-B5.20 bd-3fd2  Synthetic dense + routed IIR fixtures
                     + 36 reject counterexamples
                     + BitExact closure gate
  T-B5.21 bd-37d1  build_infer_ir_core / run_stage3 split
                     + audit-parent rewrap

Wave 5 — Review packets (2):
  T-B3.28 bd-2vx3  F-B3 review-packet sub-bundle
  T-B5.22 bd-2mne  F-B5 review-packet sub-bundle
```

Each anchor carries a "Consolidation note (2026-05-10)" comment
listing the absorbed bead IDs and titles. Each absorbed (closed)
bead carries a close reason pointing to its anchor. The §14 prose
DAG above is preserved as the original design record; the consolidated
anchor map is the work-tracking record.

Bead body amendments (post-PR-#14) live as additional comments on
each affected anchor, recording the §20.1 through §20.6 deltas that
apply to that anchor's scope.
