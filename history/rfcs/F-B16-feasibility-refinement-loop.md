# RFC F-B16: FeasibilityRefinementLoop + RepairPolicy + CompileKnobs

## -1. Authority and amendment policy

This RFC is the source of truth for F-B16 implementation. `history/planv0.md`
remains the architectural context document, but this RFC is allowed to refine,
narrow, or supersede `planv0.md` wherever the RFC makes a more precise
implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence is
recorded in an `Amends planv0` note close to the relevant decision. This is
not a request to edit `planv0.md` immediately; it is a local source-of-truth
ledger for reviewers and implementers.

This RFC also **explicitly amends F-B2/F-B4** (`history/rfcs/F-B2-F-B4-pipeline-entry-validation.md`)
in §12. The amendment surface is narrow and load-bearing:

* F-B2/F-B4 §-1 declares: *"If a later RFC changes any public type, report
  shape, cache key, or diagnostic code introduced here, that later RFC must
  explicitly amend this RFC. Source-of-truth changes must be expressed as
  typed schema changes, not prose folklore."*
* F-B2/F-B4 §2.7 forbids `PolicySource::RepairProposal(_)` and
  `ConstraintOperation::AuthorizedRelaxation` during chunk 1.
* F-B2/F-B4 §10 enumerates `ConstraintOperation` and §3204–3211 explicitly
  flags two future amendments: *"When F-B16 unblocks, it adds
  `RepairProposal(RepairProposalId)` as a sixth `PolicySource` variant ..."*
  and *"`ConstraintOperation::AuthorizedRelaxation`"*.

This RFC is the chunk-10 RFC that lands those two amendments. §12 specifies
the exact public-type changes; §13 specifies the exact report-shape changes.
No other public surface in F-B2/F-B4 is changed.

Rules:

* If this RFC and `planv0.md` disagree on F-B16 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and F-B2/F-B4 disagree on `PolicyProvenance`, `ConstraintOperation`,
  `policy_resolution.json` shape, or `StageCache` key shape, this RFC wins —
  but only in the load-bearing zones identified in §12, §13, §14. Everything
  else in F-B2/F-B4 remains authoritative.
* If a later RFC changes any public type, report shape, cache key, or
  diagnostic code introduced here, that later RFC must explicitly amend this
  RFC.
* Source-of-truth changes must be expressed as typed schema changes, not prose
  folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by chunk-10 design pass |
| Status          | **Draft (BLOCKED on oracle question — candidate definitions only)** |
| Feature beads   | bd-3ix **F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs** |
| Open tasks      | bd-3aqf **T-B16.1** core CompileKnobs types (currently recast — see §17 and bd-3aqf comment 2026-05-07: schema-only delivery moved to T-B2.0/bd-558z); bd-22h4 **T-B16.2** CompileKnobOverrides + typed selectors; bd-py29 **T-B16.3** ConstraintDelta + KnobDelta + ResourcePressureUpdate + admissibility primitives; bd-13tf **T-B16.4** per-profile defaults; bd-1r6b **T-B16.5** rename `allow_profile_fallback` → `allow_placement_profile_fallback`; bd-32w5 **T-B16.6** loop driver; bd-2swd **T-B16.7** reports |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` lines 1063–1095 (CompileKnobs named-only); 1096–1560 (compiler-pipeline preamble + refinement-loop semantics); 1665–1900 (Stages 6, 7, 8, 8.5, 9, 10, 10.5 — the loop body); 1894–1985 (Stage 11 ScheduleCostAnalysis — the loop's objective oracle); 1985–2080 (BuildReports); 2792–2870 (Reports and artifacts — `repair_report.json` content). |
| Glossary        | `history/glossary.md` (CompileKnobs, RepairPolicy, RepairProposal, ConstraintDelta, KnobDelta, MonotoneDelta, AuthorizedRelaxation, LockSet, RepairReason, PlanningStage, ObservabilityMode invariant, ResourcePressureThresholds — all added by §3 of this RFC where missing). |
| Constitution    | §I correctness by construction; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth. |
| Companion RFCs  | F-B2/F-B4 (entry/validation; §10 PolicyProvenance, §7.5 policy_resolution.json schema — amended by §12 and §13 of this RFC); F-B3/F-B5 (canonical IRs); F-B6/F-B7 (RangePlan = first stage in the loop body); F-B8 (StoragePlan); F-B9/F-B10 (SramPagePlan + RomWindowPlan); F-B11/F-B12 (OverlayPlan + ArenaPlan); F-B13 (GbSchedIR + ResourceStateValidation); F-B14 (ScheduleCostAnalysis — the **single objective oracle** the loop calls); F-B15 (Backend; placement-profile fallback may force re-layout governed by `allow_placement_profile_fallback`); F-B17 (StageCache integration sweep — consumes the invalidation rules in §14). |

**Status banner.** The bead `bd-3ix` was **BLOCKED on an oracle question**:
"What is `CompileKnobs`?" The bead's 2026-04-26 comment says the oracle has
since returned a comprehensive answer (see the bead transcript), but the
oracle answer was recorded as a comment, not as an RFC. This RFC is the RFC
form of that answer. Until the oracle answer is canonicalized **as an RFC**
the chunk remains formally `Draft (BLOCKED on oracle question — candidate
definitions only)`. Every speculative decision in §8/§9/§10/§11 is marked
with an inline `Oracle question:` annotation. §21 consolidates every such
annotation so the eventual oracle pass can answer them precisely.

The oracle answer recorded on bd-3ix (2026-04-26 11:54 UTC) is treated as a
**candidate** in this RFC. Where the oracle answer disagrees with this RFC's
text, the difference is recorded in a `Diverges from oracle:` annotation; in
every such case the chunk-10 follow-up oracle pass is the tiebreaker.

---

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B16 sits inside the compiler-pipeline
epic, where that epic sits inside the full project, and which adjacent chunks'
contracts this RFC inherits, honors, or amends.

### 0.1 Project at a glance — the eight epics

The gbllm project compiles a tiny language model into an LR35902 ROM that
runs on real Game Boy hardware. The work is split across eight epics
(`planv0.md` §"Workspace skeleton"):

```text
Epic A — M0 Foundation Stack
          gbf-asm, gbf-hw, gbf-abi, BankLease/BankGuard, Bank0 runtime,
          gbf-emu, gbf-debug, gbf-store. Provides the target/abi/asm
          contracts every other epic builds on.

Epic B — Compiler Pipeline (14 stages + refinement loop)        ← THIS EPIC
          The transform pipeline from frozen ArtifactCore +
          CompileRequest to a CompiledBuild (ROM + reports + certificates).

Epic C — Oracle Stack
          DenotationalOracle (F-C1), ArtifactOracle (F-C2),
          ScheduleOracle (F-C3), ConformanceEnvelope (F-C4).

Epic D — Runtime Beyond M0
          Persistence, harness, trace, drift, fault, SchedulePack.

Epic E — Calibration & Bench
          gbf-bench: cycle calibration, kernel timing, autotune.

Epic F — Reports & Verify
          gbf-report (build reports, certificates) + gbf-verify
          (independent slow reference implementations).

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
  F-B7  Stage 5        RangePlan                                  ← loop body start
  F-B8  Stage 6        StoragePlan ("the bridge")
  F-B9  Stage 7        SramPagePlan
  F-B10 Stage 8        RomWindowPlan
  F-B11 Stage 8.5      OverlayPlan
  F-B12 Stage 9        ArenaPlan
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation
  F-B14 Stage 11       ScheduleCostAnalysis                       ← loop body end / objective oracle
  F-B15 Stage 12       Backend (AsmIR + ReachabilityValidation +
                                PlacedRom + EncodedRom)

Cross-cutting:
  F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs   ← THIS RFC
        (BLOCKED on oracle question)
  F-B17 StageCache integration sweep across all stages
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
Chunk 8:              F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop          ← THIS RFC
```

### 0.3 Where F-B16 sits in the pipeline

F-B16 is the **only chunk in Epic B that is not a stage**. It does not
consume an IR and produce another IR. It consumes the *failure modes* of
stages 5–11 and produces *modified policy* that earlier stages then re-run
against. Its physical artifact is the **loop driver** (a function in
`gbf-codegen::refinement_loop`), the **knob lattice** (types in
`gbf-policy::knobs`), the **proposal type** (in `gbf-policy::repair`), and
two **reports** (`policy_resolution.json` extension + new
`repair_report.json`).

Diagrammatically (mirroring F-B11/F-B12 §0.3 and F-B3/F-B5 §0):

```text
                       Stages 0, 0.5, 1, 2 (validation envelope)
                                   |
                                   v
          ResolvedCompilePolicy (with CompileKnobs + RepairPolicy)
                                   |
        ┌──────────────────────────┴──────────────────────────┐
        |                  loop body, wrapped                   |
        |                                                       |
        |   Stage 3   GbInferIR       (F-B5)   not in loop      |
        |   Stage 4   ObservationPlan (F-B6)   not in loop      |
        |   Stage 5   RangePlan       (F-B7)  ◄─┐               |
        |   Stage 6   StoragePlan     (F-B8)    │               |
        |   Stage 7   SramPagePlan    (F-B9)    │ may emit      |
        |   Stage 8   RomWindowPlan   (F-B10)   │ RepairProposal|
        |   Stage 8.5 OverlayPlan     (F-B11)   │               |
        |   Stage 9   ArenaPlan       (F-B12)   │               |
        |   Stage 10  GbSchedIR       (F-B13)   │               |
        |   Stage 10.5 ResourceStateValidation ─┤               |
        |   Stage 11  ScheduleCostAnalysis  ◄───┘ (F-B14)       |
        |                       │                               |
        |        objective oracle: did the latest               |
        |        applied delta improve fit + cost?              |
        |                       │                               |
        |                       v                               |
        |         FeasibilityRefinementLoop driver  ── apply ──┐ |
        |         (F-B16, this RFC)                             | |
        |             admissibility check                       | |
        |             monotone-delta enforcement                | |
        |             iteration ceiling                         | |
        |             StageCache invalidation                   | |
        |                                                       | |
        |         CompileKnobs lattice (also F-B16)             | |
        |             values / bounds / locks / overrides       | |
        |             provenance per knob                       | |
        |                                                       | |
        └────────┬──────────────────────────────────────────────┘ |
                 |                                                 |
                 v                                                 |
       converged CompileKnobs + repair history    ─────────────────┘
                 |
                 v
       Stage 12 Backend (F-B15)
                 |
                 v
       BuildReports (F-F1 envelope)
         policy_resolution.json (now carries compile_knobs section + per-knob provenance)
         repair_report.json     (new — owned by F-B16)
```

The dotted re-entrant edge from the driver back into RangePlan / StoragePlan /
... is the *only* re-entrant edge in Epic B. Outside the loop, every Epic-B
edge is a forward edge in a DAG. Within the loop, that re-entrancy is bounded
(`max_refinement_iters`), monotone (every accepted delta tightens the
lattice — see §2.2), and observable (every accepted/rejected proposal lands
in `repair_report.json` — see §13).

### 0.4 What this chunk retires for the rest of Epic B

By the time F-B16 closes:

* Every wrapped stage (F-B7..F-B14) consumes a typed `CompileKnobs` view as
  part of its inputs and may emit `RepairProposal`s instead of bare errors.
* Every wrapped stage's `StageCache` key (F-B17) records the active
  `CompileKnobs::values` hash so a re-iteration with different knobs is a
  distinct cache entry, not a stale hit.
* `policy_resolution.json` carries a complete `compile_knobs` section with
  per-knob provenance (chains include `RepairProposal(RepairProposalId)` and
  `AuthorizedRelaxation(reason)` only after this RFC lands).
* `repair_report.json` is emitted on every build, including converged
  zero-proposal builds, and including builds that hit the iteration ceiling.
* The objective oracle for "did this proposal improve things?" is unique:
  `ScheduleCostAnalysis` (F-B14). No other pass synthesizes a
  cost-improvement signal.
* The escape hatch (`AuthorizedRelaxation`) is the *only* sanctioned way for
  a delta to relax (loosen) the lattice. Every other mutation must shrink
  the allowable space.
* Per-profile `RepairPolicy` defaults are explicit: BringUp / Default /
  Trace / Recovery each resolve to a known starting `RepairPolicy` plus a
  known initial `KnobLockSet`.

### 0.5 Cross-epic interactions

F-B16 sits at the intersection of three epics:

```text
Epic A → Epic B
  - gbf-foundation (Hash256, BlobRef, sized-byte-budget wrappers)  consumed
  - gbf-hw (TargetProfile)                                          consumed
  - gbf-abi (no direct consumption — F-B16 does not handle ABI)     n/a
  - gbf-store (StageCache invalidation rules from §14)              produces requirements

Epic B (internal):
  - F-B2 / F-B4 ResolvedCompilePolicy + CompileKnobs schema (named) consumed + AMENDED
                policy_resolution.json shape                         AMENDED
                PolicyProvenance enum                                AMENDED (new variants)
  - F-B5 / F-B6 IR products (the loop never re-enters these)         not in loop
  - F-B7..F-B14 wrapped stages                                       call into
  - F-B15 Backend                                                    consumes converged
                                                                     CompileKnobs +
                                                                     placement-profile
                                                                     fallback may force
                                                                     re-layout (gated
                                                                     by allow_placement_
                                                                     profile_fallback)
  - F-B17 StageCache integration sweep                               consumes §14
                                                                     invalidation rules

Epic F → Epic B:
  - gbf-report ReportEnvelope                                        consumed
  - F-F1 Build report emit hooks                                     consumed
  - F-F2 / future report registry                                    n/a-direct
```

### 0.6 Dependency direction inside the chunk

F-B16 has six task beads (T-B16.1..T-B16.7, with T-B16.5 a small rename) and
they form a small DAG:

```text
   T-B16.5 (rename allow_profile_fallback)
       |
       +--> T-B16.4 (per-profile defaults; depends on T-B16.5 + T-B16.1)
       |          |
       |          v
   T-B16.1 (core CompileKnobs types — recast per 2026-05-07; schema delivered by T-B2.0/bd-558z)
       |
       +--> T-B16.2 (CompileKnobOverrides + typed selectors)
       |          |
       |          v
       +--> T-B16.3 (ConstraintDelta + KnobDelta + admissibility primitives)
                  |
                  v
              T-B16.6 (FeasibilityRefinementLoop driver)
                  |
                  v
              T-B16.7 (reports — extend policy_resolution.json + emit repair_report.json)
```

Re-read after T-B16.5 lands: the rename is a workspace-wide refactor and is
sequenced first because every later task references the new name.

### 0.7 Cross-stage dependencies that this chunk pins

Every wrapped stage F-B7..F-B14 must:

1. Accept a `CompileKnobs` view in its inputs (§8).
2. On internal infeasibility, return a `RepairProposal` instead of a fatal
   error, **except** when the infeasibility is itself outside the repair
   surface (e.g. a numerical-determinism violation, which is a hard fail not
   a repair).
3. Tag every read of a knob value with the corresponding `CompileKnobId` so
   `StageCache` invalidation (§14) knows which stages are affected by which
   delta.

Every wrapped stage's RFC may, in its own §"Cross-stage interactions"
section, name the repair levers that stage uses. This RFC does not specify
**which** lever each stage emits in **which** failure case — that lives in
the stage's own RFC. This RFC specifies only the *shape* of `RepairProposal`,
the admissibility predicate, and the loop driver.

### 0.8 Milestone framing

Per `planv0.md`, F-B16 is **M3-tagged**. M1 ships F-B2 + F-B4 + F-B3 + F-B14
core + F-B7..F-B13 minimal (per the chunk schedule), all of which leave the
`compile_knobs` section in `policy_resolution.json` populated by
`{TargetDefault, ProfileDefault, CompileRequestOverride, HintBundle, Calibration}`
provenance only. The `RepairProposal(_)` and `AuthorizedRelaxation(_)`
provenance variants are minted in M3 by this RFC.

The M1 build path therefore behaves as a `RepairPolicy {
max_refinement_iters: 0, ... }` build: the policy structure exists, the
loop driver is a no-op stub, every stage that would emit a proposal instead
fails fast, and the `repair_report.json` envelope is either absent or
contains only the converged-with-no-proposals shape (an empty proposal list
plus `TerminalState::Converged`).

`Amends planv0`: the plan describes the loop as if it always runs; this RFC
splits the loop into "structural surface (M1)" and "loop driver behavior
(M3)" so that M1 can ship without F-B16 — the loop driver may be a stub
that always returns `Converged | StagedFailureUnrepairable` immediately.

---

## 0a. TL;DR

F-B16 is the **bounded monotone repair loop** that wraps Stages 5–11 of the
gbllm compiler pipeline. The loop allows downstream stages (RangePlan
through ScheduleCostAnalysis) to recover from local infeasibility by
proposing typed mutations (`RepairProposal`s) against a single
**named-policy surface** (`CompileKnobs`). The driver is the only
component that may apply such mutations; passes propose only.

This RFC's primary deliverables:

1. A **candidate definition** of `CompileKnobs` (§8) — the typed name-space
   of repair-mutable knobs (eight sub-knobs: placement, observation, range,
   storage, sram, rom_window, overlay, schedule), each with declared
   monotone order, bounds, lock semantics, and provenance.
2. A **candidate definition** of `RepairPolicy` (§9) — the per-build
   policy that toggles whether each repair lever is enabled, plus
   per-profile defaults (BringUp / Default / Trace / Recovery).
3. A **candidate definition** of `RepairProposal`, `ConstraintDelta`,
   `KnobDelta`, `ResourcePressureUpdate`, and the **admissibility
   predicate** (§10).
4. The **loop driver algorithm** (§11) — collect proposals, validate
   admissibility, apply, invalidate `StageCache` from the earliest
   affected stage, re-run, terminate by convergence or iteration ceiling.
5. The **PolicyProvenance amendment** (§12) — adding two variants
   (`RepairProposal(RepairProposalId)` and `AuthorizedRelaxation(RepairReason)`)
   to F-B2/F-B4's `PolicySource` enum.
6. The **report extensions** (§13) — `policy_resolution.json`'s
   `compile_knobs.provenance` chains may now include `RepairProposal(_)`
   and `AuthorizedRelaxation(_)` variants; `repair_report.json` is a new
   report owned by F-B16.
7. The **StageCache invalidation algebra** (§14) — which stages' cache
   keys become stale when which knobs change.

The chunk closes only when:

1. `CompileKnobs` is defined (every sub-knob has a concrete enum or struct
   with a declared monotone rank).
2. `RepairPolicy` is defined (per-profile defaults exist for BringUp /
   Default / Trace / Recovery).
3. The admissibility predicate is mechanically checkable (Rust function
   plus serde tests).
4. Every accepted `KnobDelta` is a monotone shrink in the relevant
   sub-knob's declared order, **or** is an explicitly-authorized
   `AuthorizedRelaxation` with PolicyProvenance recording the reason.
5. The loop driver terminates within `max_refinement_iters` on every
   fixture in §10.4.
6. `ScheduleCostAnalysis` (F-B14) is the *only* objective-improvement
   signal used by the loop. The driver does not call into other stages
   for "did this help?" answers.
7. `policy_resolution.json` carries a complete `compile_knobs` section
   with per-knob provenance chains, and `repair_report.json` records
   every accepted and rejected proposal with its rejection reason.
8. The two new `PolicyProvenance` variants
   (`RepairProposal(RepairProposalId)` + `AuthorizedRelaxation(RepairReason)`)
   round-trip through the `PolicyResolutionReport::validate_semantics`
   semantic validator after the F-B2/F-B4 amendment in §12 lands.
9. F-B17 (`StageCache` integration) honors the §14 invalidation rules so
   a re-iteration with a new `CompileKnobs::values` hash is a distinct
   cache entry, never a stale hit.
10. Every speculative claim in §8/§9/§10/§11 indexed in §21 has been
    answered by the chunk-10 oracle pass before the bead can close.

The chunk does **not** include:

* Any new transform stage (every transform stage is owned by another chunk).
* The cycle-cost producer itself (F-B14 owns `schedule_cost.json` and the
  `EstimatedCostDelta` synthesizer).
* Calibration generation (calibration is an *input* to `RepairPolicy`
  resolution; calibration generation lives in Epic E).
* Backend re-layout under placement-profile fallback (F-B15 owns the
  re-layout; F-B16 only flips the `CompileKnobs::placement.profile` knob
  and re-enters the loop).
* A runtime drift monitor (drift is a runtime concern, owned by
  `RuntimeDriftMonitor` per `planv0.md` line 1855).
* A fault-policy recovery exerciser (fault is a runtime concern).
* A safe-mode trigger evaluator (also runtime).

### 0a.1 Explicit oracle questions list (consolidated in §21)

This RFC marks every speculative decision with `Oracle question:` inline.
Below is the index; each item is fully expanded in the section it
appears. §21 collects them all.

* **OQ-K1** — Are the eight `CompileKnobs` sub-knobs (placement,
  observation, range, storage, sram, rom_window, overlay, schedule) the
  *complete* set, or are there knobs the oracle wants added/removed? (§8.1)
* **OQ-K2** — Is the declared monotone order for each sub-knob correct?
  Specifically: should `PlacementProfile` advance `StrictOnePerBank →
  Budgeted → PackedExperts` (the oracle answer recorded in bd-3ix), or is
  there an alternative ordering for the M3 milestone? (§8.2)
* **OQ-K3** — Are the bound types (`max_*` for ordered enums,
  `allowed_tile_classes: BTreeSet<TileCandidateClass>` for unordered
  enumerated sets, `RefinementIterBudget(u8)` for counts) the right
  type-level shape? (§8.2)
* **OQ-K4** — Is `CompileKnobId` the right granularity for `KnobLockSet`?
  Specifically: should every override (e.g. `RomKernelResidencyOverrides`)
  have its own `CompileKnobId` for lock purposes, or is the global knob
  (`RomKernelResidencyBias`) sufficient? (§8.3)
* **OQ-K5** — Is `ConstraintProvenance::PolicySource` the only place new
  provenance variants are minted, or should `ConstraintOperation` also
  gain new variants (e.g. `ConstraintOperation::AppliedRepairProposal`,
  `ConstraintOperation::AuthorizedRelaxation`)? (§8.4, §12)
* **OQ-R1** — Are the per-profile `RepairPolicy` defaults
  (BringUp/Default/Trace/Recovery) tabled in §9.2 correct? Specifically:
  should BringUp have `max_refinement_iters: 1` (oracle answer) or
  `max_refinement_iters: 0` (problem prompt suggestion)? (§9.2)
* **OQ-R2** — Should `RepairPolicy` itself be **lockable**? That is, may
  the `CompileRequest` lock the `RepairPolicy::allow_*` toggles, or are
  they always profile-resolved and immutable for the duration of the
  build? (§9.3)
* **OQ-R3** — Should `RepairPolicy::max_refinement_iters` be
  per-stage (F-B16's per-stage `RefinementIterBudget`) or only global,
  or both? (§9.1, §11.2)
* **OQ-D1** — Is the `KnobDelta` enum closed at 14 variants (oracle
  answer), or are there missing variants (e.g.
  `RaiseSlackReservation { arena, bytes }`,
  `CoalesceBankSwitchAggression { to: BankSwitchAggression }`)?
  (§10.1, §8.1)
* **OQ-D2** — Is `AuthorizedRelaxation` a property of the *delta* (a
  delta variant), the *proposal* (a proposal field), or the *operation*
  (a `ConstraintOperation` variant)? §10.2 picks "operation"; oracle
  to confirm. (§10.2, §12)
* **OQ-D3** — What is the canonical termination proof for the bounded
  loop? §10.3 sketches "well-founded ordering on the knob lattice," but
  the proof rests on `AuthorizedRelaxation` being separately bounded
  (it can fire at most once per build). Is that bound correct? (§10.3)
* **OQ-D4** — What is the right `RepairReason` taxonomy? §10.4 proposes
  ~12 named reasons (BankNotFitting, ArenaOverflow, ...) — oracle to
  confirm completeness. (§10.4)
* **OQ-L1** — Should the loop call `ScheduleCostAnalysis` (F-B14) on
  *every* iteration, or only when convergence is in question
  (i.e. `EstimatedCostDelta` synthesis is expensive)? §11.3 proposes
  "every iteration that produced an accepted delta." (§11.3)
* **OQ-L2** — How should the loop interpret `EstimatedCostDelta` under
  uncertainty envelopes? Specifically: when a proposal's projected cost
  *worsens* but the proposal is otherwise admissible, does the loop
  accept (because the alternative is to fail the build) or reject (and
  let the build fail loudly)? §11.3 proposes "accept and record the
  worsening in `repair_report.json`." (§11.3)
* **OQ-L3** — When the loop hits the iteration ceiling, what is the
  build's terminal status? §11.4 proposes
  `TerminalState::GlobalBudgetExhausted` produces a hard build failure;
  oracle to confirm. (§11.4)
* **OQ-P1** — Does the F-B2/F-B4 amendment in §12 require a major
  schema bump (`policy_resolution.v1` → `policy_resolution.v2`) or only
  an additive semver minor bump (variant added to enum)? §13.1 proposes
  "additive minor bump." (§13.1)
* **OQ-P2** — Does `repair_report.json` exist on **every** build (even
  zero-proposal builds), or only on builds where at least one proposal
  was emitted? §13.2 proposes "every build." (§13.2)
* **OQ-S1** — When a knob delta affects multiple stages, which stage's
  cache key invalidates first? §14 proposes "invalidate the earliest
  stage in pipeline order whose canonical-input bundle contains that
  knob." (§14)
* **OQ-S2** — Should `CompileKnobs::values` be hashed *as a whole* or
  *per sub-knob* for cache-key construction? §14 proposes
  "per sub-knob hash, then concatenate" so a delta to `range_knobs`
  doesn't invalidate stages that depend only on `placement_knobs`. (§14)

Numbering convention: `OQ-K*` for knob shape, `OQ-R*` for RepairPolicy
shape, `OQ-D*` for delta/proposal shape, `OQ-L*` for loop driver, `OQ-P*`
for PolicyProvenance/report shape, `OQ-S*` for StageCache integration.

### 0a.2 What this chunk is, by analogy

* **Like F-B2/F-B4**, F-B16 *amends a public schema* (`policy_resolution.json`,
  `PolicyProvenance`). Unlike F-B2/F-B4, the schema change is *additive
  enum variants*, not a new top-level section.
* **Like F-B14**, F-B16 *consumes a single objective oracle*
  (`ScheduleCostAnalysis`'s `EstimatedCostDelta`) but does not produce one.
* **Unlike every other Epic-B chunk**, F-B16 *re-enters earlier stages*.
  No other chunk has a re-entrant edge.
* **Unlike F-B17**, F-B16 *defines the rules* for which knob touches which
  stage's cache; F-B17 *implements* those rules across the workspace.
* **Like F-A6 (`gbf-store-migrate`)**, F-B16 has a *deferred design intent*
  shape: the `RepairProposal(_)` and `AuthorizedRelaxation(_)` provenance
  variants exist in F-B2/F-B4's enum **as forbidden today** and *become
  available* only when this RFC lands. The plan-vs-implementation gap is
  intentional.

---

## 1. Project context — where this chunk sits in the milestone sequence

### 1.1 What F-B2..F-B15 leave behind for F-B16

By the time the chunk-9 RFC (F-B15 Backend) lands, the rest of Epic B has
shipped enough infrastructure that F-B16 has nothing structural to build
on top of beyond its own loop driver and knob lattice:

* **F-B2** ships `ResolvedCompilePolicy` carrying a `CompileKnobs` field
  (named-only at the `gbf-policy` schema level via T-B2.0/bd-558z) and a
  `RepairPolicy` field (also schema-only). It also ships
  `policy_resolution.json` with a complete `compile_knobs.{global, bounds,
  locks, overrides, provenance}` section, populated by the *five* M1
  `PolicySource` variants (`TargetDefault | ProfileDefault |
  CompileRequestOverride | HintBundle | Calibration`) only.
* **F-B4** ships `static_budget.json` whose static-fit verdict is consumed
  by the loop driver (F-B16 reads `decision.fits` as one of several
  pre-loop signals — see §11.1).
* **F-B3 / F-B5** ship `QuantGraph` and `GbInferIR`. Neither is in the
  loop body; both are consumed by stages 5+ that *are* in the loop body.
* **F-B6** ships `ObservationPlan`. Also not in the loop body, but its
  output (the `TraceProbeId` set) is referenced by `KnobDelta::DisableOptionalProbes`
  and `CompileKnobOverrides::disabled_optional_probes`.
* **F-B7** through **F-B14** ship the loop-body stages. Each must accept a
  `CompileKnobs` view in its inputs and must return a typed
  `RepairProposal`-or-`Output` enum. None of them call into earlier
  stages — only the loop driver may do so (§2.1).
* **F-B14** ships `ScheduleCostAnalysis`, the **single objective oracle**.
  It produces `ScheduleCostReport { objective, per_mode:
  EstimatedCostDelta, refs }` which the loop reads to decide whether an
  applied delta improved the build.
* **F-B15** ships the Backend; relevant for F-B16 only insofar as a
  placement-profile fallback (`StrictOnePerBank → Budgeted → PackedExperts`)
  may force a re-layout. The fallback is gated by
  `RepairPolicy::allow_placement_profile_fallback`.

The pieces F-B16 alone owns:

* The **loop driver** (`gbf-codegen::refinement_loop::run_refinement_loop`).
* The **admissibility predicate** (pure function in `gbf-policy::knobs`,
  callable by the driver and by tests).
* **Per-profile defaults** (`gbf-policy::knobs::profile_defaults` —
  initial `CompileKnobs` plus initial `RepairPolicy` per
  `CompileProfile`).
* **Two new `PolicySource` variants** + amended F-B2/F-B4 schema.
* **Two reports**: `policy_resolution.json` extension (sixth + seventh
  variant) + `repair_report.json` (new).

### 1.2 What M1 commits to and what M3 commits to

`planv0.md` describes the loop as if it always runs, but the chunk
schedule splits the work across milestones:

* **M1 (head-of-line):** the structural surface — `CompileKnobs`,
  `RepairPolicy`, `KnobLockSet`, `CompileKnobBounds`, the
  `policy_resolution.json` `compile_knobs` section, the five
  pre-amendment `PolicySource` variants. F-B2/F-B4 ship this.
* **M3 (this chunk):** the *behavior* — the loop driver, the
  `RepairProposal(_)` and `AuthorizedRelaxation(_)` provenance variants,
  the `repair_report.json` schema, per-profile `RepairPolicy` defaults
  with `max_refinement_iters > 0`.

`Amends planv0:` planv0 implies the loop is always live. This RFC narrows
that claim: in M1, every wrapped stage that would emit a
`RepairProposal` instead emits a hard typed error. Only in M3 does the
loop driver materialize and start applying proposals.

### 1.3 What this chunk is NOT

The chunk is small in *scope* but large in *contract surface*. To prevent
scope creep, here is what this chunk explicitly is **not**:

* It is **not** a new transform stage. F-B16 owns no IR. Every IR
  produced or consumed by the loop body is owned by another chunk.
* It is **not** a runtime drift monitor. `RuntimeDriftMonitor` (planv0
  line 1855) lives in Epic D and is checked at runtime, not at compile
  time. F-B16's only "did this help?" signal is
  `ScheduleCostAnalysis`'s static-and-calibrated `EstimatedCostDelta`,
  not runtime measurement.
* It is **not** a fault-recovery exerciser. The runtime's fault policy
  (panic screen, hard reset, safe-mode entry) is unrelated to compile-time
  repair. The two share only the word "recovery."
* It is **not** a re-layout engine. When `allow_placement_profile_fallback`
  is `true` and a fallback is taken, F-B15 (Backend) is the engine that
  re-runs `PlacedRom` against the new profile. F-B16 only flips the knob
  and invalidates the appropriate `StageCache` entries.
* It is **not** a calibration generator. `CalibrationBundle` is an
  *input* to `RepairPolicy` resolution (specifically: calibration
  measurements may tighten `ResourcePressureThresholds`). The
  generation pipeline lives in Epic E (`gbf-bench`).
* It is **not** a backend cycle-cost producer. `ScheduleCostAnalysis`
  (F-B14) owns `schedule_cost.json` and is consumed by F-B16 — but
  F-B16 does not synthesize cycle estimates.
* It is **not** a per-stage cache implementation. F-B17 owns the actual
  `StageCache` integration sweep; F-B16 only pins the *invalidation
  rules* (§14). F-B17 implements them.
* It is **not** a profile-time relaxation surface. `Bringup` is a
  profile, not a relaxation; `AuthorizedRelaxation` is a typed,
  bounded, observable escape, not a soft-mode toggle (§2.4).
* It is **not** a hint-bundle consumer that *adds* facts. Hints are
  consumed at policy resolution (Stage 0.5, F-B2). F-B16 reads only the
  resolved knob *bounds* and *locks*; it does not re-consult
  `HintBundle`.
* It is **not** the producer of `EstimatedCostDelta`. That synthesis
  lives in F-B14 + F-B16 jointly: F-B14 produces per-mode envelopes,
  F-B16 produces a *delta* between two envelopes (before-and-after the
  applied delta). The delta synthesizer's home is `gbf-codegen::refinement_loop::cost`.

### 1.4 Why this is one Feature, not two or three

The natural unit is "the bounded monotone repair loop and its mutable
policy surface." Splitting it would either:

* **Two features (knobs + driver):** would force the loop driver to ship
  with knobs whose defaults are not yet defined; or knobs without a
  driver to apply proposals. Either way the chunk closes only when both
  land.
* **Three features (knobs + driver + reports):** the report schemas are
  literally serializations of the same types; splitting them creates
  artificial cross-bead synchronization.
* **One feature with three task DAGs:** matches reality. The DAG is
  T-B16.5 (rename) → T-B16.1 / T-B16.2 / T-B16.3 (types) → T-B16.4
  (per-profile defaults) → T-B16.6 (driver) → T-B16.7 (reports). See §17.

### 1.5 Relationship to F-B17 (`StageCache` integration sweep)

F-B17 is the workspace-wide sweep that wires every Epic-B stage's cache
key against the canonical-input convention. F-B16 supplies the **rules**
for which `CompileKnobId` invalidates which stage's cache key (§14).
F-B17 supplies the **implementation** (the actual cache-key constructors
in each stage's `gbf-codegen::stages::*` module).

The boundary:

* F-B16 names the rule: "`PlacementProfile` invalidates Stage 6
  (StoragePlan), Stage 7 (SramPagePlan), Stage 8 (RomWindowPlan), Stage
  9 (ArenaPlan)."
* F-B17 wires the rule: each stage's cache-key constructor reads
  `compile_knobs.values.placement.profile` and includes its hash in the
  cache key.

A model that passes F-B16's loop driver may still hit a stale cache
unless F-B17 has implemented the corresponding key. The chunk closes
without F-B17, but the loop is then only end-to-end testable on
fixtures whose cache state can be cleared between iterations.

### 1.6 Relationship to F-B14 (`ScheduleCostAnalysis`)

F-B14 is the **single objective oracle** for the loop. The relationship
is asymmetric:

* F-B14 produces `ScheduleCostReport { objective, per_mode:
  EstimatedCostDelta, refs }`. It is a *measurement-or-estimation*
  pass, not a decision pass.
* F-B16 consumes the report to ask: "did the latest applied
  `ConstraintDelta` improve fit + cost relative to the previous
  iteration's report?"

The asymmetry is deliberate. F-B14 must not know about repair: it is a
pure read of the current pipeline state. F-B16 must call F-B14 only
when it has applied a delta and needs to decide whether to continue or
stop. (See §11.3 for the algorithm details and **OQ-L1** for the open
question on call frequency.)

### 1.7 Relationship to F-B15 (Backend) and `allow_placement_profile_fallback`

F-B15 owns `PlacedRom`'s placement profiles
(`StrictOnePerBank | Budgeted | PackedExperts` per planv0 lines 1948–1953).
The loop may advance the profile knob through that ladder *only if*
`RepairPolicy::allow_placement_profile_fallback` is true. When it does,
the `StageCache` invalidation rules of §14 force F-B12 (ArenaPlan) and
F-B15 (Backend) to re-run.

F-B16 is *not* the re-layout engine. F-B16 changes the knob; F-B12 and
F-B15 do the actual placement work. F-B15's RFC, when it arrives, must
acknowledge this re-entrance hook.

`Diverges from oracle:` the bd-3ix oracle answer (2026-04-26) names the
field `allow_placement_profile_fallback`, distinct from
`RiskPolicy::fallback_profile`. That distinction is preserved here. The
T-B16.5 rename task (bd-1r6b) lands the workspace-wide refactor.

---

## 2. Load-bearing decisions

### 2.1 Bounded monotone repair — passes never call earlier passes recursively

The single most load-bearing decision in F-B16 is that **passes propose
mutations; only the loop driver applies them**. A pass cannot call into
an earlier pass, mutate the `CompileKnobs` lattice itself, or escalate a
local infeasibility into a re-run of an earlier stage. The pass emits a
typed `RepairProposal` and yields control to the loop driver. The driver
decides admissibility (§10.2), records the outcome (§13.2), applies the
delta if admissible, invalidates the appropriate `StageCache` entries
(§14), and re-runs the affected stages.

This decision has three consequences:

1. **Termination is mechanically checkable.** Every accepted delta
   monotonically shrinks the lattice (or fires the bounded
   `AuthorizedRelaxation` once-per-build). The `max_refinement_iters`
   ceiling provides the second termination bound. See §10.3 for the
   proof structure.
2. **Stage-level reasoning stays local.** A pass author writes "if this
   condition holds, emit `RepairProposal { reason: ..., tighten: ... }`."
   The author does not write "if this condition holds, re-run the
   prior stage with these inputs."
3. **`StageCache` invalidation has a single, type-driven rule set.**
   When a delta is applied, §14 says exactly which stages must
   re-execute. There is no ad-hoc "stage X re-runs stage Y" code.

`Amends planv0`: planv0 line 1130 says "Passes do not call earlier
passes recursively; instead they emit `RepairProposal`s against an
explicit `CompileKnobs` surface." This RFC keeps that wording but adds
the explicit invariant: *the only re-entrant edge in Epic B is from the
loop driver back to the earliest stage whose canonical-input bundle
contained an invalidated knob.* No other re-entrant edge exists.

### 2.2 Monotonicity is well-founded; it is the termination proof

The lattice on `CompileKnobs::values` is the cartesian product of
each sub-knob's lattice. Each sub-knob lattice is one of:

* **Ordered enum** (e.g. `PlacementProfile`, `TraceDemotionLevel`,
  `ReductionPlanCeiling`). The order is declared via `MonotoneKnob::rank`.
  A delta is monotone iff `to.rank() >= current.rank()`.
* **Subset-removal set** (e.g. `allowed_tile_classes:
  BTreeSet<TileCandidateClass>`). A delta is monotone iff the new set
  is a *subset* of the current set.
* **Superset-addition set** (e.g. `disabled_optional_probes:
  BTreeSet<TraceProbeId>`). A delta is monotone iff the new set is a
  *superset* of the current set.
* **Insert-or-advance map** (e.g.
  `forced_kernel_residency: BTreeMap<KernelSelector, KernelResidency>`).
  A delta is monotone iff every existing key's value is replaced with a
  later-rank value (or the key is unchanged), or a new key is inserted.
  Keys may not be removed and existing values may not be replaced with
  earlier-rank values.

The product of these lattices is bounded by the `CompileKnobBounds`
field for each sub-knob (the `max_*` for ordered enums, the
`allowed_tile_classes` set for unordered, etc.). Bounds are themselves
locked at policy resolution and not modified by the loop driver, so
the lattice has finite height.

The termination argument:

* Each accepted delta strictly advances at least one sub-knob in its
  declared lattice (else it would be a no-op, which is rejected as
  `NotMonotone`).
* The lattice has finite height (bounded by `CompileKnobBounds`).
* Therefore the number of consecutive accepted deltas without
  termination is finite.
* Combined with `max_refinement_iters` (a separate counter), the loop
  terminates either by convergence (no proposals) or by ceiling.

The `AuthorizedRelaxation` operation is the *only* exception, and it is
bounded separately: at most one relaxation per build, gated by
`RepairPolicy::allow_placement_profile_fallback`. See §10.2 and §10.3.

`Oracle question (OQ-D3)`: §10.3 elaborates the proof. The proof rests
on `AuthorizedRelaxation` firing at most once. Oracle to confirm the
once-per-build bound is the right contract, vs e.g. once-per-fallback-class.

### 2.3 `ScheduleCostAnalysis` is the single objective oracle

The loop must decide, after each accepted delta, whether to continue
(another stage may still propose) or terminate (everything fits). The
"did this help?" question is answered by exactly one source:
`ScheduleCostAnalysis`'s `EstimatedCostDelta`.

This decision is load-bearing because:

* It centralizes the "is this build acceptable?" definition. Every
  other stage's pass plan can ignore objective math.
* It pins the `EvidenceClass` hierarchy (Measured / Transferred /
  Heuristic) at one production site. The loop reads
  `EstimatedCostDelta.evidence` to know how confident the comparison is.
* It makes the "improvement" predicate a function of one report's
  output, not a join over many reports.

The loop *only* asks F-B14: every other "did the proposal succeed?"
answer is local to the proposing stage (e.g. "did StoragePlan now
materialize successfully?" is answered by StoragePlan's own
`Result<Output, RepairProposal>` return type).

`Oracle question (OQ-L1)`: should F-B14 be called on every iteration
that produced an accepted delta, or only when convergence is
contested? See §11.3.

`Oracle question (OQ-L2)`: how is `EstimatedCostDelta` interpreted under
uncertainty envelopes? When projected cost worsens but the alternative
is build failure, does the loop accept or reject? See §11.3.

### 2.4 `AuthorizedRelaxation` is the only sanctioned escape

Every accepted delta must shrink the lattice (be monotone in the
declared order). The single exception is `AuthorizedRelaxation`, which
is used exclusively when the placement-profile ladder reaches an
infeasible bottom and `allow_placement_profile_fallback == true`. The
intended use case is:

```text
1. Loop starts at PackedExperts (the most aggressive profile).
2. A stage emits a proposal that would advance the profile further —
   except there is no further. The proposal cannot be admissible under
   pure monotone shrink rules.
3. allow_placement_profile_fallback is true. The driver emits a typed
   AuthorizedRelaxation operation, recorded with PolicyProvenance::
   AuthorizedRelaxation(reason). The relaxation steps PlacementProfile
   *backward* to Budgeted (or StrictOnePerBank, depending on what fits).
4. F-B12 and F-B15 re-run against the new profile.
5. If the build now fits, terminate; if not, the relaxation has been
   used and cannot fire again — fall through to GlobalBudgetExhausted.
```

`AuthorizedRelaxation` is **not** a soft-mode toggle, **not** a
profile-time relaxation surface, and **not** a generic escape hatch.
Specifically:

* It is **not** silent. Every relaxation lands in
  `repair_report.json` with reason and provenance.
* It is **not** unbounded. At most one relaxation per build.
* It is **not** profile-conditional. Any profile may have
  `allow_placement_profile_fallback == true`; the relaxation gate is
  the toggle, not the profile.
* It is **not** generic. Only `PlacementProfile` is currently relaxable;
  no other knob has a corresponding relaxation operation.

`Oracle question (OQ-D2)`: is `AuthorizedRelaxation` properly
expressed as a `ConstraintOperation` variant (§12), as a `KnobDelta`
variant (§10.1), or as a `RepairProposal` field? §12 picks
`ConstraintOperation`. Oracle to confirm.

### 2.5 Per-profile RepairPolicy defaults are explicit, not implicit

Every `CompileProfile` (BringUp / Default / Trace / Recovery) resolves
to a known starting `RepairPolicy` plus a known initial `KnobLockSet`.
The defaults are not derived implicitly from "what the profile feels
like"; they are tabled in §9.2 and tested.

| Profile  | iters | placement_fallback | trace_demotion | overlay_promotion | recompute_promotion |
|----------|------:|--------------------|----------------|-------------------|---------------------|
| BringUp  | 1     | false              | false          | false             | false               |
| Default  | 4     | true               | true           | true              | true                |
| Trace    | 2     | false              | false          | false             | false               |
| Recovery | 6     | true               | true           | true              | true                |

`Oracle question (OQ-R1)`: the prompt suggests `BringUp:
max_refinement_iters = 0`. The bd-3ix oracle answer says
`max_refinement_iters = 1` (one iteration absorbs deterministic local
tile/slice fixes while still failing loudly on placement / overlay /
recompute / trace policy). This RFC tracks both as candidates and asks
the chunk-10 oracle pass to pick.

`Oracle question (OQ-R2)`: should `RepairPolicy` itself be
**lockable**? §9.3 elaborates.

### 2.6 StageCache invalidation is type-driven, not stage-driven

The decision of which stages re-run after a delta is encoded as a map
from `CompileKnobId` to "set of stages whose cache keys depend on this
knob." When a delta is applied, the driver consults the map, computes
the union of affected stages, and invalidates them in pipeline order
starting from the earliest.

This decision is load-bearing because the alternative ("each stage
declares its dependencies in its own RFC") creates a worse coupling:
every stage's RFC would have to enumerate every knob it cares about,
and a knob added by F-B16 would force every stage RFC to be amended.
With type-driven invalidation, F-B16 owns the map and F-B17 reads it.

`Oracle question (OQ-S1)`: when a delta affects multiple stages, is the
"earliest stage" the right invalidation start point? §14 says yes.

`Oracle question (OQ-S2)`: should `CompileKnobs::values` be hashed as a
whole or per sub-knob for cache-key construction? §14 picks per
sub-knob.

### 2.7 PolicyProvenance extension is observable

Every accepted delta leaves a trail in two places: the
`policy_resolution.json` `compile_knobs.provenance` chain (which now
includes a `RepairProposal(RepairProposalId)` entry), and the
`repair_report.json` file (a per-build proposal log). Together they
make the loop's behavior fully auditable.

The decision to extend `PolicySource` *and* introduce `repair_report.json`
(rather than only one or the other) is load-bearing because:

* `policy_resolution.json` is the single answer to "what policy
  governed this build." It must include the *final* knob values plus
  their provenance chain.
* `repair_report.json` is the single answer to "what did the loop do."
  It includes every proposal, accepted or rejected, with reasoning.
* Without both, "what policy governed this build?" cannot answer
  "what proposals were rejected and why?", which is often the fastest
  explanation of a build failure.

`Oracle question (OQ-P1)`: does the F-B2/F-B4 amendment require a
schema-version bump? §13.1 picks "additive minor bump."

`Oracle question (OQ-P2)`: is `repair_report.json` emitted on every
build (even zero-proposal builds)? §13.2 picks yes.

### 2.8 No string-typed errors anywhere

Every diagnostic, every rejection reason, every termination reason is a
typed enum. The taxonomy is fixed at:

* `RepairReason` — the proposing stage's reason for emitting the
  proposal. ~12 named reasons (§10.4).
* `DeltaRejection` — the driver's reason for rejecting a proposal.
  Six named reasons (§10.2).
* `TerminalState` — the loop's terminal classification. Four named
  states (§11.1).

There is no `String` field in any of these. Free-form prose is allowed
only inside `last_error: String` of `TerminalState::StagedFailureUnrepairable`,
which carries the raw error from the failing stage.

`Oracle question (OQ-D4)`: is the `RepairReason` taxonomy complete? §10.4
proposes the candidate set.

### 2.9 `Bringup` is a profile, not a relaxation surface (mirrors F-B2/F-B4 §2.13)

This RFC inherits the F-B2/F-B4 §2.13 invariant: there is no
profile-conditional softness. Every Bringup behavior is expressed as
typed knob bounds + locks + `RepairPolicy::max_refinement_iters: 1`,
not as soft diagnostics or hidden relaxations.

### 2.10 F-B16 does not modify `CompileKnobs::bounds` or `CompileKnobs::locks`

Bounds and locks are resolved once at policy resolution (Stage 0.5,
F-B2) and are immutable for the duration of the build. The loop driver
modifies only `CompileKnobs::values` (the current point in the lattice)
and `CompileKnobs::overrides` (the targeted-override maps). It does not
modify bounds, locks, or provenance entries for any other reason than
to record the new applied delta.

This decision is load-bearing because it makes the lattice height a
build-invariant property: the maximum number of accepted deltas is
known before the loop begins, which is what enables the termination
proof in §10.3.

### 2.11 Where the code lives

| Concern                                              | Crate                                  |
| ---------------------------------------------------- | -------------------------------------- |
| `CompileKnobs`, `CompileKnobValues`, `CompileKnobBounds`, `KnobLockSet`, `CompileKnobOverrides`, `CompileKnobId`, `MonotoneKnob` | `gbf-policy::knobs` (schema by T-B2.0/bd-558z; refinement-only extensions if needed by T-B16.1) |
| `RepairPolicy`, `RepairProposal`, `RepairProposalId`, `RepairReason` | `gbf-policy::repair`                   |
| `ConstraintDelta`, `KnobDelta`, `ResourcePressureUpdate`, `DeltaRejection`, `check_delta_admissible` | `gbf-policy::knobs::delta` (T-B16.3)   |
| `PolicySource` (six variants after this RFC), `ConstraintProvenance`, `ConstraintOperation` (with `AuthorizedRelaxation` after this RFC) | `gbf-policy::compile` (existing — amended by §12) |
| Per-profile defaults                                 | `gbf-policy::knobs::profile_defaults`  |
| `FeasibilityRefinementLoop` driver                   | `gbf-codegen::refinement_loop`         |
| `PolicyResolutionReportBody` extension (carrying new provenance variants) | `gbf-report` (existing — amended by §13.1) |
| `RepairReport`, `RepairProposalRecord`, `RepairReportBody`, `repair_report.v1` schema | `gbf-report::schemas::repair_report` (new — owned by T-B16.7) |
| StageCache invalidation rules (the map)              | `gbf-codegen::stage_cache::invalidation` |
| StageCache integration (per-stage cache-key wiring)  | F-B17 (consumes §14)                   |

No new crate is created by this chunk.

### 2.12 What stays out of `CompileKnobs` (mirrors bd-3ix oracle)

The oracle answer (recorded on bd-3ix 2026-04-26) is explicit:
`CompileKnobs` does **not** contain calibration constants, target
constants, objective weights, pass-private heuristics, or opaque
numeric tuning fields. This RFC honors that boundary.

* **Calibration constants** live in `PlatformCalibrationBundle` /
  `KernelCalibrationBundle` / `RuntimeCalibrationBundle` (Epic E).
  They are *inputs* to `RepairPolicy` resolution; they are never
  modified by the loop.
* **Target constants** live in `TargetProfile` (Epic A's `gbf-hw`).
  They are *inputs* to bounds resolution; they are never modified by
  the loop.
* **Objective weights** live in `CompileObjective` (`gbf-policy::objective`).
  The loop *reads* the objective via `ScheduleCostAnalysis`'s
  `ScheduleCostReport.objective`, but does not mutate weights.
* **Pass-private heuristics** stay inside their pass. The loop has no
  hook to a heuristic; if a heuristic is load-bearing, it must be
  promoted to a `CompileKnobs` value.
* **Opaque numeric tuning fields** are forbidden. Every
  `CompileKnobs` field is either a typed enum, a typed struct, or a
  bounded integer wrapper (`PressureLimit<T>`,
  `RefinementIterBudget(u8)`).

### 2.13 No retroactive amendment to closed RFCs

If `CompileKnobs` later needs an additional sub-knob (e.g. a
hypothetical `bank_switch_coalescing` knob), the proper procedure is:

1. A future RFC `F-B?` declares the new knob.
2. The future RFC explicitly amends *this* RFC by adding a row to §8.1.
3. The future RFC explicitly amends F-B2/F-B4 §10 if the new knob's
   `CompileKnobId` is added to `CompileKnobId` enum.
4. The amendment ships as a typed schema change (a new field on
   `CompileKnobValues`, a new variant on `CompileKnobId`).

`Amends planv0`: planv0 implies `CompileKnobs` is open-ended. This RFC
freezes the eight-sub-knob shape as the M3 contract; further sub-knobs
require an explicit amending RFC.

### 2.14 What the chunk explicitly does NOT promise

* The chunk does **not** promise that every conceivable
  build-infeasibility class is repairable. Some classes (numerical
  determinism violations, ABI-break violations, hard runtime
  identity mismatches) are *never* repairable and must fail loudly
  via `TerminalState::StagedFailureUnrepairable`.
* The chunk does **not** promise loop convergence on adversarial
  inputs. A pathological fixture can still hit
  `TerminalState::GlobalBudgetExhausted`. The promise is termination,
  not success.
* The chunk does **not** promise objective improvement on every
  iteration. `EstimatedCostDelta` may worsen between iterations under
  uncertainty envelopes; §11.3 records the worsening but does not
  reject the delta.
* The chunk does **not** promise that `repair_report.json` is small.
  Builds that hit the iteration ceiling produce reports proportional
  to `max_refinement_iters` × number-of-stages.
* The chunk does **not** promise that `StageCache` invalidation is
  optimal. F-B16 owns the rules; F-B17 owns the implementation; if
  the rules invalidate too aggressively, the cost is wall-clock time,
  not correctness.

---

## 3. Glossary additions

These terms are added or refined by this RFC. Where a term is already
defined in `history/glossary.md`, this RFC's usage is consistent unless
explicitly noted; new terms below are candidates for the glossary
amendment that should accompany F-B16's landing.

### 3.1 RepairProposal

A typed message emitted by a wrapped stage (F-B7..F-B14) when its local
infeasibility has a candidate repair. Carries:

* `source: PlanningStage` — which stage emitted the proposal.
* `reason: RepairReason` — why the stage failed.
* `tighten: ConstraintDelta` — the proposed mutation.
* `knob_delta: KnobDelta` — the canonical single-knob form when the
  delta touches one knob (when the delta touches multiple knobs, this
  field is `None` and the full `tighten` is consulted).
* `resource_pressure: Option<ResourcePressureUpdate>` — when the
  proposal affects a `ResourcePressureThresholds` field.

A proposal is *not* a side effect. The stage emits the proposal,
returns control to the loop driver, and the driver decides
admissibility. See §10.

### 3.2 ConstraintDelta

A typed bag of `KnobDelta` mutations. Specifically:

```rust
pub struct ConstraintDelta {
    pub changes: Vec<KnobDelta>,
}
```

Most proposals carry a single `KnobDelta`. Multi-knob proposals are
allowed but rare; they are useful when two knobs must move together
(e.g. advancing `PlacementProfile` and `KernelDuplicationBias` in a
single proposal so the cost report sees both changes at once). See §10.

### 3.3 KnobDelta

A typed enum with one variant per repair lever. The bd-3ix oracle
answer enumerates 14 variants (see §10.1 for the full list).

### 3.4 MonotoneDelta

A `KnobDelta` is *monotone* if applying it to the current
`CompileKnobs::values` produces a new value that is *higher* in the
relevant sub-knob's declared lattice (or, for set-typed knobs, a strict
subset/superset depending on the set's polarity). See §2.2.

### 3.5 AuthorizedRelaxation

The single sanctioned escape from monotonicity. A
`ConstraintOperation::AuthorizedRelaxation(reason)` records that the
loop driver applied a *relaxing* delta — one that moves backward in
the lattice — under the authority of
`RepairPolicy::allow_placement_profile_fallback`. Currently used only
for `PlacementProfile` ladder fallback (PackedExperts → Budgeted →
StrictOnePerBank if no further advance is feasible). See §2.4 and §12.

### 3.6 LockSet (`KnobLockSet`)

A `BTreeSet<CompileKnobId>` recording which knobs the loop driver may
not touch. Locks are resolved at policy resolution (Stage 0.5, F-B2)
and immutable for the build's duration. A delta against a locked knob
is rejected as `DeltaRejection::KnobLocked`. See §8.3.

### 3.7 RepairPolicy

Per-build typed struct toggling whether each repair lever class is
enabled, plus the iteration ceiling. See §9.1.

### 3.8 RepairReason

The proposing stage's typed reason for emitting the proposal. Not
free-form prose; one of ~12 named variants. See §10.4.

### 3.9 PlanningStage

Typed enum naming the wrapped stages of the loop. Used by
`RepairProposal::source` and by `repair_report.json`. The variants:

```rust
pub enum PlanningStage {
    RangePlan,
    StoragePlan,
    SramPagePlan,
    RomWindowPlan,
    OverlayPlan,
    ArenaPlan,
    GbSchedIR,
    ResourceStateValidation,
    ScheduleCostAnalysis,
}
```

`Oracle question (OQ-D4 part b)`: should `ObservationPlan` be in the
loop body (and therefore in `PlanningStage`)? planv0 line 1109 says no
(ObservationPlan is Stage 4, outside the loop wrapper). This RFC
follows planv0.

### 3.10 ObservabilityMode (referenced — owned by F-B2)

Already defined in F-B2 (`Invariant | Flexible`). The loop driver reads
this field; under `Invariant`, deltas affecting `ObservationKnobs` are
rejected as `DeltaRejection::InvariantObservabilityViolation`. See
§10.2.

### 3.11 ResourcePressureThresholds

A typed struct of `PressureLimit<T> { soft, hard }` per resource
(WRAM/HRAM/SRAM/ROM/cycle/trace/persist/overlay/bank-switch). Resolved
once at policy resolution; usually locked. See §8.1.

### 3.12 RefinementIterBudget

A `#[repr(transparent)] struct RefinementIterBudget(pub u8)` recording
the per-stage iteration ceiling. Effective per-iteration bound for
stage S is `min(global_iters_remaining, stage_iters_remaining.S)`. See
§9.1.

### 3.13 RepairProposalId

A typed newtype identifying a proposal within a build. Used by
`PolicySource::RepairProposal(RepairProposalId)` to chain a knob's
final value back to the proposal that produced it. See §8.4 and §12.

### 3.14 EstimatedCostDelta (referenced — owned by F-B14)

Already defined by F-B14. F-B16 reads
`ScheduleCostReport::per_mode: BTreeMap<RuntimeMode, EstimatedCostDelta>`
to decide whether the latest applied delta improved the build. See
§11.3.

### 3.15 TerminalState

The loop's terminal classification.

```rust
pub enum TerminalState {
    Converged,
    AcceptedRefinementBudgetExhausted { stage: PlanningStage },
    GlobalBudgetExhausted,
    StageBudgetExhausted { stage: PlanningStage },
    StagedFailureUnrepairable { stage: PlanningStage, last_error: String },
}
```

See §11.1.

### 3.16 DeltaRejection

The driver's typed reason for rejecting a proposal.

```rust
pub enum DeltaRejection {
    AcceptedRefinementBudgetExhausted { max_refinement_iters: u8 },
    KnobLocked { knob: CompileKnobId },
    PolicyToggleDisabled { knob: CompileKnobId, toggle: &'static str },
    BeyondBounds { knob: CompileKnobId, attempted: String, max: String },
    NotMonotone { knob: CompileKnobId, current: String, attempted: String },
    InvariantObservabilityViolation { knob: CompileKnobId },
    EffectfulRecompute { value: ValueSelector },
}
```

See §10.2.

---

## 4. Core notation

This RFC uses the following notation throughout:

* **Lattice ranks**: `MonotoneKnob::rank() -> u8`. A knob at rank `r`
  may advance to rank `r' >= r`. Ranks are dense (no skipped values)
  and start at 0.
* **Bound predicates**: `bounds.is_within(value)` — true iff `value`'s
  rank is `<= bounds.max_rank()` (for ordered enums) or `value` is in
  `bounds.allowed` (for enumerated sets).
* **Lock predicates**: `locks.is_locked(knob_id)` — true iff
  `knob_id ∈ locks.locked`.
* **Provenance chain**: `provenance.chain` for a knob is the ordered
  sequence of `ConstraintProvenance` entries that produced its current
  value. A chain is non-empty by F-B2/F-B4 §7.5 invariants.
* **Iteration counter**: `LoopState::global_iters_remaining: u8`,
  starting at `RepairPolicy::max_refinement_iters` and decremented per
  iteration (whether or not a delta was accepted).
* **Per-stage counter**: `LoopState::stage_iters_remaining:
  StageIterationCeilings`, decremented per iteration that ran a given
  stage.
* **Snapshot identity**: `KnobsSnapshotHash` — the canonical hash of
  the current `CompileKnobs::values` for cache-key construction. See
  §14.
* **Pipeline order**: the wrapped stages run in the order
  `RangePlan → StoragePlan → SramPagePlan → RomWindowPlan → OverlayPlan
  → ArenaPlan → GbSchedIR → ResourceStateValidation →
  ScheduleCostAnalysis`. The loop driver never re-orders.
* **Re-run semantics**: when a delta is accepted, the driver
  invalidates `StageCache` entries for every stage whose canonical
  inputs include any modified knob, then re-runs from the earliest
  invalidated stage in pipeline order.

Hashes follow the F-B2/F-B4 convention (`sha256:<64 lowercase hex>`).
Domain separator (per F-B2/F-B4 §2.4):

```text
gbf:gbf-policy:CompileKnobs:compile_knobs:1.0.0\0<canonical-json-bytes>
gbf:gbf-policy:RepairPolicy:repair_policy:1.0.0\0<canonical-json-bytes>
gbf:gbf-report:RepairReport:repair_report.v1:1.0.0\0<canonical-json-bytes>
```

---

## 5. Authority rules

This RFC is authoritative for:

* **`CompileKnobs::values` candidate definition** — the eight sub-knobs
  in §8.1, their ordered enums, their declared monotone ranks. (Subject
  to oracle confirmation, OQ-K1, OQ-K2.)
* **`CompileKnobs::bounds` candidate definition** — the per-sub-knob
  bound types in §8.2. (Subject to OQ-K3.)
* **`KnobLockSet` candidate definition** — the `CompileKnobId` granularity
  in §8.3. (Subject to OQ-K4.)
* **`CompileKnobs::overrides` typed selector set** — already pinned by
  bd-22h4/T-B16.2 oracle answer. §8.6.
* **`RepairPolicy` candidate definition** — five fields (max_iters +
  four allow_* toggles) plus per-stage `RefinementIterBudget`. (Subject
  to OQ-R1, OQ-R2, OQ-R3.)
* **Per-profile RepairPolicy defaults** — BringUp / Default / Trace /
  Recovery in §9.2. (Subject to OQ-R1.)
* **`RepairProposal` typed shape** — §10.1.
* **`ConstraintDelta` and `KnobDelta` typed shape** — 14 variants in
  §10.1. (Subject to OQ-D1.)
* **`ResourcePressureUpdate` typed shape** — 12 variants in §10.1.
* **Admissibility predicate** — `check_delta_admissible` in §10.2,
  with six rejection reasons.
* **Termination proof structure** — §10.3.
* **`RepairReason` taxonomy** — ~12 named reasons in §10.4. (Subject
  to OQ-D4.)
* **Loop driver algorithm** — §11.1.
* **Iteration ceiling semantics** — §11.2.
* **`ScheduleCostAnalysis` call rules** — §11.3. (Subject to OQ-L1,
  OQ-L2.)
* **Failure modes** — §11.4. (Subject to OQ-L3.)
* **`PolicyProvenance` extension** — two new variants
  (`PolicySource::RepairProposal(RepairProposalId)` and
  `ConstraintOperation::AuthorizedRelaxation(RepairReason)`). §12.
* **`policy_resolution.json` extensions** — §13.1.
* **`repair_report.json` schema** — §13.2.
* **StageCache invalidation rules** — §14.
* **Diagnostic codes (REPAIR-*)** — §15.

This RFC is **not** authoritative for:

* `CompileKnobs` *schema-level* definition — that is owned by F-B2's
  T-B2.0 (bd-558z). This RFC describes the *behavior* of those types
  inside the loop.
* `ScheduleCostReport` shape — F-B14.
* `EstimatedCostDelta` synthesis — F-B14.
* `RuntimeChromeBudget` shape — F-B2/F-B4.
* `ObservabilityMode` shape — F-B2 (carried into resolved policy).
* `PlanningStage` enumeration of stages — owned by `gbf-codegen` per
  the chunk DAG; this RFC pins the relevant variants.
* Per-stage cache key construction — F-B17.
* Backend re-layout under `allow_placement_profile_fallback` — F-B15.

When this RFC and a non-authoritative companion RFC disagree on a
non-authoritative surface, the companion RFC wins.

---

## 6. Pipeline state machine

### 6.1 Loop state

The driver maintains a single `LoopState`:

```rust
pub struct LoopState {
    pub knobs: CompileKnobs,
    pub repair_policy: RepairPolicy,
    pub observability: ObservabilityMode,
    pub global_iters_remaining: u8,
    pub stage_iters_remaining: StageIterationCeilings,
    pub history: RepairHistory,                    // for repair_report.json
    pub last_cost_report: Option<ScheduleCostReport>,
    pub authorized_relaxation_used: bool,          // bounds the once-per-build escape
}

pub struct RepairHistory {
    pub proposals: Vec<RepairProposalRecord>,      // accepted + rejected
    pub stage_iteration_counts: BTreeMap<PlanningStage, u8>,
    pub global_iters_used: u8,
}
```

Initial state at loop entry:

* `knobs`: resolved by Stage 0.5 (F-B2). Includes the per-profile
  initial values (§9.2) plus any `CompileRequest` overrides plus any
  `HintBundle` constraints plus any calibration-derived tightening.
* `repair_policy`: resolved by Stage 0.5 from the `CompileProfile`
  defaults (§9.2).
* `observability`: resolved by Stage 0.5.
* `global_iters_remaining`: equal to `repair_policy.max_refinement_iters`.
* `stage_iters_remaining`: equal to `knobs.values.schedule.stage_iters`.
* `history`: empty.
* `last_cost_report`: `None`.
* `authorized_relaxation_used`: `false`.

### 6.2 Stage outcomes

Each wrapped stage's run function signature is:

```rust
pub trait WrappedStage {
    type Output;
    fn run(&self, inputs: &StageInputs<'_>) -> StageOutcome<Self::Output>;
}

pub enum StageOutcome<T> {
    Success(T),
    NeedsRepair(RepairProposal),
    UnrepairableFailure(StageError),
}
```

The driver consumes `StageOutcome` and dispatches:

* `Success(t)`: the stage produced its IR; continue to the next stage.
* `NeedsRepair(p)`: the stage cannot proceed; the driver tries to
  apply `p` (admissibility check).
* `UnrepairableFailure(e)`: the stage failed in a way no repair could
  fix (e.g. ABI violation, numerical-determinism break). The driver
  terminates with `TerminalState::StagedFailureUnrepairable`.

### 6.3 Iteration boundary

An *iteration* of the loop is one pass through stages 5–11 with a
fixed `LoopState::knobs`. The driver:

```text
while global_iters_remaining > 0 and not terminated:
    iteration_proposals := []
    for stage in [RangePlan, StoragePlan, ..., ScheduleCostAnalysis]:
        decrement stage_iters_remaining[stage]
        outcome := stage.run(inputs_with_current_knobs)
        case outcome:
            Success(_): continue
            NeedsRepair(p):
                iteration_proposals.append(p)
                break out of inner loop  # restart from earliest invalidated stage
            UnrepairableFailure(e):
                terminate(StagedFailureUnrepairable(stage, e))
    if iteration_proposals.is_empty():
        if last_cost_report indicates no further improvement attempted:
            terminate(Converged)
    else:
        for p in iteration_proposals:
            decision := check_delta_admissible(p, knobs, repair_policy, observability)
            history.proposals.append(record(p, decision))
            if decision is Ok:
                apply_delta(p, knobs)
                invalidate_stage_cache(p.affected_stages)
                # restart from earliest invalidated stage on next iteration
            else:
                # rejection recorded; stage may emit a different proposal next iter
                pass
    global_iters_remaining -= 1

if global_iters_remaining == 0 and not terminated:
    terminate(GlobalBudgetExhausted)
```

The exact pseudocode is refined in §11.1; the state-machine view above
is for orientation.

### 6.4 Terminal states

The loop ends in one of four states:

1. `TerminalState::Converged` — every stage succeeded with no
   proposal in the most recent iteration.
2. `TerminalState::AcceptedRefinementBudgetExhausted { stage }` — a
   stage produced an admissible repair proposal after
   `RepairPolicy::max_refinement_iters` accepted deltas had already
   been consumed. The proposal is recorded as rejected with
   `DeltaRejection::AcceptedRefinementBudgetExhausted`.
3. `TerminalState::GlobalBudgetExhausted` — `global_iters_remaining`
   reached 0 with at least one stage still needing repair.
4. `TerminalState::StageBudgetExhausted { stage }` — a per-stage
   counter reached 0 while the stage was still needing repair.
5. `TerminalState::StagedFailureUnrepairable { stage, last_error }` —
   a stage returned `UnrepairableFailure`.

Only the first is a build success. The other four are typed
build failures that emit `repair_report.json` plus
`policy_resolution.json` (with the *current* — possibly partially
mutated — knobs and provenance, recording every accepted delta up to
the failure point).

### 6.5 Determinism

The loop is deterministic given:

* a fixed initial `CompileKnobs`,
* a fixed `RepairPolicy`,
* a fixed wrapping-stage implementation set,
* a fixed `StageCache` state.

Two runs with byte-identical inputs produce byte-identical
`repair_report.json` and byte-identical converged
`policy_resolution.json`. This is asserted by tests (§19, proof
obligation 6).

---

## 7. Report envelope (inherited)

F-B16's reports use the F-B2/F-B4 `ReportEnvelope` shape. Every report
emitted by F-B16 includes:

* `schema`: a string of the form `"<report_kind>.v<major>"`.
* `report_self_hash`: per F-B2/F-B4 §2.4 (sha256 over canonical JSON
  with the field temporarily zeroed).
* `report_inputs`: an identity section with the input hashes the
  report depends on.
* `outcome`: `Passed | Failed`.
* `body`: the report-specific payload.

`repair_report.json` is owned by F-B16 (§13.2). The
`policy_resolution.json` extension reuses the existing envelope; only
its `body.compile_knobs.provenance` chain shape changes (additional
permitted variants in `PolicySource` and `ConstraintOperation`).

The canonical JSON convention, the lowercase `sha256:<hex>` hash
serialization rule, the `gbf:<crate>:<type>:<schema-id>:<schema-version>\0`
domain separator — all inherited from F-B2/F-B4 §2.4 unchanged.

---

## 8. CompileKnobs (candidate definition — major oracle question section)

This section is the load-bearing speculative core of the RFC. It is the
chunk-10 RFC's primary deliverable. Every claim in this section is a
*candidate* answer; the chunk-10 oracle pass must confirm or revise.

The bd-3ix oracle answer (recorded as a comment, 2026-04-26 11:54 UTC)
is the most concrete prior-art and is treated as the *candidate of
record* for everything in §8. Where this RFC differs from that
candidate, the difference is flagged with `Diverges from oracle:`.

### 8.1 Type-level contract

`CompileKnobs` is a single typed struct with five fields:

```rust
pub struct CompileKnobs {
    pub global: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub overrides: CompileKnobOverrides,
    pub locks: KnobLockSet,
    pub provenance: BTreeMap<CompileKnobId, ConstraintProvenance>,
}
```

`global` is the current point in the lattice (the set of resolved
knob values for this build). `bounds` is the per-knob upper bound (or
allowed-set); `bounds` is immutable after policy resolution. `overrides`
is the targeted-override map (per-selector knob values). `locks` is the
set of `CompileKnobId` values that the loop driver may not touch.
`provenance` records, per knob, the chain of operations that produced
the current value (`SeedDefault → TightenBound → ApplyOverride → ...`).

`CompileKnobValues` is itself a struct of eight sub-knobs:

```rust
pub struct CompileKnobValues {
    pub placement: PlacementKnobs,
    pub observation: ObservationKnobs,
    pub range: RangeKnobs,
    pub storage: StorageKnobs,
    pub sram: SramKnobs,
    pub rom_window: RomWindowKnobs,
    pub overlay: OverlayKnobs,
    pub schedule: ScheduleKnobs,
}
```

`Oracle question (OQ-K1)`: are these eight sub-knobs the *complete*
set? Specifically, the prompt suggests considering:

* "bank-switch coalescing aggressiveness"
* "ROM-window co-residency aggressiveness"
* "per-arena reservation slacks"
* "chunk-tile sizes (for RangePlan ChunkedI16 / RenormLoop)"

Each of these may or may not be a separate sub-knob. This RFC's
candidate position:

* **bank-switch coalescing aggressiveness**: subsumed under
  `ScheduleKnobs::tile_search` and `slice_coarsening`. Larger tiles +
  coarser slices → fewer bank switches. **Oracle question (OQ-K1.a)**:
  is a separate `BankSwitchCoalescingLevel` knob warranted, or is the
  composite knob sufficient?
* **ROM-window co-residency aggressiveness**: subsumed under
  `RomWindowKnobs::kernel_residency_bias` (specifically the
  `PreferCoResident` value advances co-residency). **Oracle question
  (OQ-K1.b)**: is `KernelResidencyBias` granular enough, or should
  there be a separate `CoResidencyAggression` knob?
* **per-arena reservation slacks**: subsumed under
  `ScheduleKnobs::pressure: ResourcePressureThresholds`, where each
  arena has a soft/hard pressure limit. **Oracle question (OQ-K1.c)**:
  should the per-arena slack have its own dedicated knob, or is the
  pressure threshold sufficient?
* **chunk-tile sizes (RangePlan ChunkedI16 / RenormLoop)**: subsumed
  under `RangeKnobs::reduction_ceiling` (which advances through
  `SingleI16Only → AllowChunkedI16 → AllowRenormLoop`). The chunk
  *size* is not directly knob-controlled; it is computed by RangePlan
  given the ceiling. **Oracle question (OQ-K1.d)**: should chunk size
  itself be a knob (e.g. `ChunkTileSize: BoundedU16`)?

This RFC's default position is "no additional sub-knobs"; the eight
listed are sufficient for the M3 contract. The chunk-10 oracle pass is
the tiebreaker.

#### 8.1.1 Sub-knob definitions

```rust
pub struct PlacementKnobs {
    pub profile: PlacementProfile,
}

// Already defined in gbf-policy by F-B15/Backend RFC.
pub enum PlacementProfile {
    StrictOnePerBank,
    Budgeted,
    PackedExperts,
}

pub struct ObservationKnobs {
    pub trace_demotion: TraceDemotionLevel,
    pub optional_probe_floor: ProbeBudgetClass,
}

pub enum TraceDemotionLevel {
    None,
    DropBestEffort,
    DropDiagnosticAndBestEffort,
    RequiredOnly,
}

pub enum ProbeBudgetClass {
    Required,
    Important,
    Diagnostic,
    BestEffort,
}

pub struct RangeKnobs {
    pub reduction_ceiling: ReductionPlanCeiling,
}

pub enum ReductionPlanCeiling {
    SingleI16Only,
    AllowChunkedI16,
    AllowRenormLoop,
}

pub struct StorageKnobs {
    pub recompute_promotion: RecomputePromotionLevel,
}

pub enum RecomputePromotionLevel {
    None,
    PureSliceValues,
    PureResumeWindowValues,
    PureTokenValues,
}

pub struct SramKnobs {
    pub page_aggression: SramPageAggression,
    pub spill_policy: SpillPolicy,
}

pub enum SramPageAggression {
    KeepHotInWram,
    BatchSramAccesses,
    AllowColdSpills,
    FitFirstPagedSpills,
}

// SpillPolicy already defined by F-B9/SramPagePlan RFC.
pub enum SpillPolicy {
    NoSpill,
    SpillOnPressure,
    SpillEager,
}

pub struct RomWindowKnobs {
    pub kernel_residency_bias: KernelResidencyBias,
    pub kernel_duplication_bias: KernelDuplicationBias,
}

pub enum KernelResidencyBias {
    ProfileDefault,
    PreferCoResident,
    PreferBank0Streaming,
    PreferWramOverlay,
    FitFirst,
}

pub enum KernelDuplicationBias {
    ShareKernels,
    DuplicateEntryStubs,
    DuplicateTinyKernels,
    DuplicateToSatisfyWindow,
}

pub struct OverlayKnobs {
    pub promotion: OverlayPromotionLevel,
}

pub enum OverlayPromotionLevel {
    None,
    KernelsOnly,
    KernelsAndLutFragments,
    AnyOverlayable,
}

pub struct ScheduleKnobs {
    pub tile_search: TileSearchKnobs,
    pub slice_coarsening: SliceCoarseningLevel,
    pub pressure: ResourcePressureThresholds,
    pub stage_iters: StageIterationCeilings,
}

pub struct TileSearchKnobs {
    pub allowed_classes: BTreeSet<TileCandidateClass>,
}

pub enum TileCandidateClass {
    SmallWorkingSet,
    Balanced,
    SwitchAmortized,
}

pub enum SliceCoarseningLevel {
    Fine,
    ProfileDefault,
    CoarseWithinLatency,
    EpochCoalesced,
}

pub struct ResourcePressureThresholds {
    pub wram_hot: PressureLimit<ByteBudget>,
    pub hram_hot: PressureLimit<ByteBudget>,
    pub bank0_rom: PressureLimit<ByteBudget>,
    pub switchable_rom_window: PressureLimit<ByteBudget>,
    pub sram_window: PressureLimit<ByteBudget>,
    pub slice_cycles: PressureLimit<CycleBudget>,
    pub interrupt_latency: PressureLimit<CycleBudget>,
    pub trace_bytes_per_frame: PressureLimit<u16>,
    pub persist_bytes_per_frame: PressureLimit<u16>,
    pub overlay_installs_per_frame: PressureLimit<u8>,
    pub bank_switches_per_token: PressureLimit<u16>,
    pub sram_page_switches_per_token: PressureLimit<u16>,
}

pub struct PressureLimit<T> {
    pub soft: T,
    pub hard: T,
}

pub struct StageIterationCeilings {
    pub range: RefinementIterBudget,
    pub storage: RefinementIterBudget,
    pub sram_page: RefinementIterBudget,
    pub rom_window: RefinementIterBudget,
    pub overlay: RefinementIterBudget,
    pub arena: RefinementIterBudget,
    pub schedule: RefinementIterBudget,
}

#[repr(transparent)]
pub struct RefinementIterBudget(pub u8);
```

#### 8.1.2 The `MonotoneKnob` trait

To make admissibility checks type-driven (not match-statement-driven),
each ordered enum implements:

```rust
pub trait MonotoneKnob: Eq {
    /// Dense rank, starting at 0; lower rank = less aggressive,
    /// higher rank = more aggressive (advancing through the lattice).
    fn rank(&self) -> u8;

    /// Default monotone test: a knob may advance to higher-or-equal rank.
    fn is_monotone_advance(from: &Self, to: &Self) -> bool {
        from.rank() <= to.rank()
    }

    /// For a strict advance (rank strictly higher), used when a
    /// no-op delta should be rejected.
    fn is_strict_advance(from: &Self, to: &Self) -> bool {
        from.rank() < to.rank()
    }
}
```

Implementations are mechanical:

```rust
impl MonotoneKnob for PlacementProfile {
    fn rank(&self) -> u8 {
        match self {
            PlacementProfile::StrictOnePerBank => 0,
            PlacementProfile::Budgeted         => 1,
            PlacementProfile::PackedExperts    => 2,
        }
    }
}

impl MonotoneKnob for TraceDemotionLevel {
    fn rank(&self) -> u8 {
        match self {
            TraceDemotionLevel::None                       => 0,
            TraceDemotionLevel::DropBestEffort             => 1,
            TraceDemotionLevel::DropDiagnosticAndBestEffort => 2,
            TraceDemotionLevel::RequiredOnly                => 3,
        }
    }
}

// ... and so on for each ordered enum.
```

`Oracle question (OQ-K2)`: are these declared monotone orders correct?
The bd-3ix oracle answer pins each ordering; the chunk-10 oracle pass
should confirm none of them have changed under the M3 surface.

### 8.2 Per-knob bounds

`CompileKnobBounds` records, per knob, the maximum value the loop
driver may advance to (or the allowed set, for unordered knobs):

```rust
pub struct CompileKnobBounds {
    pub max_placement_profile: PlacementProfile,
    pub max_trace_demotion: TraceDemotionLevel,
    pub max_reduction_ceiling: ReductionPlanCeiling,
    pub max_recompute_promotion: RecomputePromotionLevel,
    pub max_sram_page_aggression: SramPageAggression,
    pub max_kernel_residency_bias: KernelResidencyBias,
    pub max_kernel_duplication_bias: KernelDuplicationBias,
    pub max_overlay_promotion: OverlayPromotionLevel,
    pub allowed_tile_classes: BTreeSet<TileCandidateClass>,
    pub max_slice_coarsening: SliceCoarseningLevel,
    pub stage_iters: StageIterationCeilings,
}
```

Bounds resolution at policy time (Stage 0.5, F-B2):

1. Start from `TargetProfile` defaults → seed every `max_*` and the
   `allowed_tile_classes` set.
2. Apply `CompileProfile` defaults — may *tighten* (lower a `max_*`,
   shrink an `allowed_*`) but never loosen.
3. Apply `HintBundle::constraints` — may *tighten*, never loosen.
4. Apply `CompileRequest::constraint_overrides` — may *tighten*,
   never loosen. Per F-B2/F-B4 §2.7.
5. Apply `CalibrationBundle` data-driven tightening (e.g. measured
   pressure thresholds).
6. Verify monotonicity: every bound is at most as loose as the target
   default.

`Oracle question (OQ-K3)`: are the bound types correct? Specifically:

* For ordered enums, `max_<knob>: <enum>` is the maximum *rank*
  reachable. Is this clearer as `max_<knob>_rank: u8`, or is the
  enum-typed bound preferred?
* For unordered enumerated sets, `allowed_tile_classes: BTreeSet<...>`
  is the explicit allowed set. Should it instead be a closed
  enumeration (e.g. `TileSearchPolicy::AllClasses |
  TileSearchPolicy::SmallAndBalanced | TileSearchPolicy::SmallOnly`)?
* For counts, `RefinementIterBudget(u8)` allows any 0..=255. Is a
  bounded `RefinementIterBudget(BoundedU8<0, 16>)` warranted?

This RFC's candidate position: keep the candidate types from bd-3ix
oracle answer. The chunk-10 oracle pass is the tiebreaker.

### 8.3 LockSet semantics

A `KnobLockSet` records which `CompileKnobId` values are locked for
this build:

```rust
pub struct KnobLockSet {
    pub locked: BTreeSet<CompileKnobId>,
}
```

Lock semantics:

* A knob in `locks.locked` is **frozen** at its initial value. The
  loop driver may not propose, accept, or apply any delta against it.
* A knob is locked because:
  (a) the `CompileProfile` declared it locked (e.g. BringUp locks
      almost everything),
  (b) a `CompileRequest::constraint_overrides` field set the value
      explicitly *and* declared it locked,
  (c) a `HintBundle::constraints` field set the value explicitly,
  (d) the `ObservabilityMode::Invariant` rule forces certain knobs
      locked (specifically, all of `ObservationKnobs` and any
      behavior-affecting knob that would influence trace semantics).
* Locks resolve at Stage 0.5 (policy resolution) and are immutable
  for the build duration.

The granularity decision:

`Oracle question (OQ-K4)`: is `CompileKnobId` the right granularity
for locks? Specifically, the enum currently distinguishes
`RomKernelResidencyBias` (the global knob) from
`RomKernelResidencyOverrides` (the targeted-override map). Should both
be lockable independently, or is one global lock sufficient? This RFC
keeps them separate; the chunk-10 oracle pass is the tiebreaker.

The full `CompileKnobId` enum:

```rust
pub enum CompileKnobId {
    PlacementProfile,
    ObservationTraceDemotion,
    ObservationProbeSelection,
    RangeReductionCeiling,
    StorageRecomputePromotion,
    StorageMaterializationOverrides,
    SramPageAggression,
    SramSpillPolicy,
    RomKernelResidencyBias,
    RomKernelDuplicationBias,
    RomKernelResidencyOverrides,
    OverlayPromotion,
    ScheduleTileSearch,
    ScheduleSliceCoarsening,
    ScheduleResourcePressure,
    StageIterationCeilings,
}
```

This matches planv0 line 1404–1421.

### 8.4 Provenance per knob

Each knob's current value has a provenance chain in
`CompileKnobs::provenance`:

```rust
// gbf-policy
pub struct ConstraintProvenance {
    pub source: PolicySource,
    pub operation: ConstraintOperation,
    pub evidence: Vec<EvidenceRef>,
}

pub enum PolicySource {
    TargetDefault,
    ProfileDefault,
    CompileRequestOverride,
    HintBundle,
    Calibration,
    RepairProposal(RepairProposalId),                  // NEW — added by §12
}

pub enum ConstraintOperation {
    SeedDefault,
    TightenBound,
    ApplyPreference,
    ApplyHardConstraint,
    ApplyOverride,
    ApplyCalibration,
    AppliedRepairProposal(RepairProposalId),           // NEW — added by §12
    AuthorizedRelaxation(RepairReason),                // NEW — added by §12
}

#[repr(transparent)]
pub struct RepairProposalId(pub u32);
```

`Oracle question (OQ-K5)`: should `ConstraintOperation` gain new
variants (`AppliedRepairProposal`, `AuthorizedRelaxation`), or is it
sufficient to add the new variant only on `PolicySource`? This RFC's
candidate: add to both, because:

* `PolicySource::RepairProposal` answers "*what* set this value?"
* `ConstraintOperation::AppliedRepairProposal` answers "*how* did the
  setter act on it?" (specifically: by applying a `KnobDelta`).
* `ConstraintOperation::AuthorizedRelaxation` answers "*how* and *why*
  did the setter loosen this value?"

Without the operation-side enrichment, the chain entry "`source:
RepairProposal(_)`, `operation: ApplyOverride`" conflates a normal
override with a repair. The chunk-10 oracle pass is the tiebreaker.

### 8.5 Oracle question summary for §8

For the chunk-10 oracle pass, §8 raises the following questions
indexed in §21:

| ID     | Question                                                            | This RFC's candidate                                       |
|--------|---------------------------------------------------------------------|------------------------------------------------------------|
| OQ-K1  | Are eight sub-knobs the complete set?                                | Yes (per bd-3ix).                                          |
| OQ-K1.a| Separate bank-switch coalescing knob?                                | No — subsumed under tile_search + slice_coarsening.        |
| OQ-K1.b| Separate co-residency aggression knob?                               | No — subsumed under KernelResidencyBias.                   |
| OQ-K1.c| Per-arena reservation slack as own knob?                             | No — subsumed under ResourcePressureThresholds.            |
| OQ-K1.d| Chunk tile size as own knob?                                         | No — derived from ReductionPlanCeiling.                    |
| OQ-K2  | Declared monotone orders correct?                                    | Yes (per bd-3ix).                                          |
| OQ-K3  | Bound types correct?                                                 | Yes (per bd-3ix).                                          |
| OQ-K4  | CompileKnobId granularity for locks?                                 | Yes — keep BiasOverride distinct from BiasGlobal.          |
| OQ-K5  | Add ConstraintOperation variants?                                    | Yes — `AppliedRepairProposal` + `AuthorizedRelaxation`.    |

### 8.6 CompileKnobOverrides — typed selectors (already pinned by T-B16.2)

The targeted-override surface is already pinned by bd-22h4
(T-B16.2)'s oracle answer:

```rust
pub struct CompileKnobOverrides {
    pub disabled_optional_probes: BTreeSet<TraceProbeId>,
    pub forced_kernel_residency: BTreeMap<KernelSelector, KernelResidency>,
    pub forced_recompute: BTreeSet<ValueSelector>,
    pub reduction_ceiling_overrides: BTreeMap<ReductionSelector, ReductionPlanCeiling>,
    pub tile_class_overrides: BTreeMap<TileSelector, BTreeSet<TileCandidateClass>>,
}

pub enum KernelSelector {
    KernelSpec(KernelSpecId),
    LayerExpert { layer: LayerId, expert: ExpertId },
    Section(SectionId),
}

pub enum ValueSelector {
    Value(ValueId),
    AliasClass(AliasClassId),
}

pub enum ReductionSelector {
    Site(ReductionSiteId),
    Layer(LayerId),
}

pub enum TileSelector {
    Kernel(KernelSpecId),
    Layer(LayerId),
    SliceClass(SliceClass),
}

pub enum SliceClass {
    Micro,
    Frame,
    TokenBoundary,
    TraceHeavy,
}
```

Override monotonicity (referenced by §10.2):

* `disabled_optional_probes`: superset addition only.
* `forced_kernel_residency`: insert-only or move-to-later-rank in
  `KernelResidency`'s order. Existing keys may be moved later; never
  removed.
* `forced_recompute`: superset addition only.
* `reduction_ceiling_overrides`: insert-only or move-to-later-rank in
  `ReductionPlanCeiling`'s order.
* `tile_class_overrides`: insert-only; for an existing key, the new
  set must be a *subset* of the old set (subset removal).

Semantic observations (per bd-22h4) are *never* listed in
`disabled_optional_probes`. Only `ProbeBudgetClass::{Diagnostic,
BestEffort}` probes are eligible for trace demotion; `Required` and
`Important` probes are immune.

### 8.7 Worked example: a typical CompileKnobs at policy resolution

Walking through a Default-profile build of a tiny synthetic fixture:

```text
Initial seed (TargetDefault):
  placement.profile         = Budgeted               (target default)
  observation.trace_demotion = None                   (target default)
  observation.optional_probe_floor = Diagnostic       (target default)
  range.reduction_ceiling   = AllowRenormLoop          (target default)
  storage.recompute_promotion = None                   (target default)
  sram.page_aggression       = BatchSramAccesses       (target default)
  sram.spill_policy          = SpillOnPressure         (target default)
  rom_window.kernel_residency_bias = ProfileDefault    (target default)
  rom_window.kernel_duplication_bias = ShareKernels    (target default)
  overlay.promotion          = None                    (target default)
  schedule.tile_search.allowed_classes = {Small, Balanced, SwitchAmortized}
  schedule.slice_coarsening = ProfileDefault            (target default)
  schedule.pressure          = <RuntimeChromeBudget-resolved>
  schedule.stage_iters       = <RefinementIterBudget(2) per stage>

ProfileDefault (Default profile):
  storage.recompute_promotion = PureSliceValues        (profile default)
  rom_window.kernel_residency_bias = PreferCoResident  (profile default)

CompileRequestOverride: (none in this build)

HintBundle: (none in this build)

Calibration:
  schedule.pressure.wram_hot = PressureLimit { soft: 4096, hard: 4608 }
                              <-- tightened by calibration measurement

Bounds:
  max_placement_profile         = PackedExperts        (Default cap)
  max_trace_demotion            = DropDiagnosticAndBestEffort
  max_reduction_ceiling         = AllowRenormLoop
  max_recompute_promotion       = PureResumeWindowValues
  max_sram_page_aggression      = AllowColdSpills
  max_kernel_residency_bias     = PreferWramOverlay
  max_kernel_duplication_bias   = DuplicateTinyKernels
  max_overlay_promotion         = KernelsAndLutFragments
  allowed_tile_classes          = {Small, Balanced, SwitchAmortized}
  max_slice_coarsening          = CoarseWithinLatency
  stage_iters                   = <RefinementIterBudget(2) per stage>

Locks:
  ScheduleResourcePressure      (Default profile lock)
  StageIterationCeilings        (Default profile lock)

Provenance for placement.profile:
  chain = [
    { source: TargetDefault,  operation: SeedDefault,    evidence: [target_profile_hash] },
  ]
```

After loop iteration 1, suppose StoragePlan emitted a proposal that
advanced `recompute_promotion` to `PureResumeWindowValues`. The
provenance chain for `StorageRecomputePromotion` becomes:

```text
chain = [
  { source: TargetDefault,                        operation: SeedDefault,           evidence: [target_profile_hash] },
  { source: ProfileDefault,                       operation: ApplyOverride,         evidence: [profile_hash] },
  { source: RepairProposal(RepairProposalId(7)),  operation: AppliedRepairProposal(RepairProposalId(7)), evidence: [proposal_hash, source_stage_StoragePlan_hash] },
]
```

The `policy_resolution.json` for this build now carries this enriched
chain. The `repair_report.json` records the proposal-7 acceptance.

### 8.8 What `CompileKnobs` does NOT contain

Repeated for clarity (and to make the boundary self-evident in §8):

* No calibration constants (lives in `CalibrationBundle`).
* No target constants (lives in `TargetProfile`).
* No objective weights (lives in `CompileObjective`).
* No pass-private heuristics (lives inside the pass).
* No opaque numeric tuning fields (every field is a typed enum,
  typed struct, or bounded integer wrapper).

This keeps the repair surface finite, typed, profile-bounded, and
auditable — per the bd-3ix oracle answer and engineering rule 25
(see §15.2).

---

## 9. RepairPolicy (per-profile defaults — second oracle question section)

### 9.1 Type-level

```rust
pub struct RepairPolicy {
    pub max_refinement_iters: u8,
    /// PlacementProfile fallback only:
    /// StrictOnePerBank → Budgeted → PackedExperts.
    ///
    /// Full CompileProfile fallback remains RiskPolicy::fallback_profile.
    pub allow_placement_profile_fallback: bool,
    pub allow_trace_demotion: bool,
    pub allow_overlay_promotion: bool,
    pub allow_recompute_promotion: bool,
}
```

The five fields:

* `max_refinement_iters: u8` — the global iteration ceiling. The loop
  driver decrements `LoopState::global_iters_remaining` on every
  iteration (regardless of whether a delta was accepted) and
  terminates with `TerminalState::GlobalBudgetExhausted` when the
  counter reaches 0.
* `allow_placement_profile_fallback: bool` — gates *both* the monotone
  advance through `StrictOnePerBank → Budgeted → PackedExperts` *and*
  the once-per-build `AuthorizedRelaxation` that may step backward
  through the same ladder when no further forward step exists. A
  single toggle covers both because the placement-profile ladder is
  the only knob class with a relaxation operation.
* `allow_trace_demotion: bool` — gates `KnobDelta::SetTraceDemotion`
  and `KnobDelta::DisableOptionalProbes`. Forced false under
  `ObservabilityMode::Invariant` (see §10.2 admissibility check #6).
* `allow_overlay_promotion: bool` — gates `KnobDelta::PromoteOverlay`
  and any `KnobDelta::ForceKernelResidency` whose target is
  `KernelResidency::WramOverlay`.
* `allow_recompute_promotion: bool` — gates
  `KnobDelta::PromoteRecomputeLevel` and `KnobDelta::ForceRecompute`.
  Even when allowed, `KnobDelta::ForceRecompute` is rejected as
  `DeltaRejection::EffectfulRecompute` if any selected value is
  effectful (see §10.2 admissibility check #5).

`Oracle question (OQ-R3)`: should `max_refinement_iters` be per-stage
or only global? This RFC keeps both: the global counter
(`max_refinement_iters: u8`) and the per-stage `StageIterationCeilings`
(part of `ScheduleKnobs`). The per-stage ceilings live inside
`CompileKnobs::values.schedule.stage_iters`, not inside
`RepairPolicy`. This separation means the *cap* on iterations is
profile-policy (RepairPolicy), but the *fairness* across stages is
schedule-policy (StageIterationCeilings).

`Diverges from oracle:` the bd-3ix answer puts `StageIterationCeilings`
inside `ScheduleKnobs`. This RFC preserves that. The chunk-10 oracle
should confirm it does not want `StageIterationCeilings` promoted into
`RepairPolicy`.

### 9.2 Per-profile defaults

Each `CompileProfile` resolves to a known initial `RepairPolicy` plus
a known initial `KnobLockSet`. The candidate table:

#### 9.2.1 BringUp profile

```rust
RepairPolicy {
    max_refinement_iters: 1,
    allow_placement_profile_fallback: false,
    allow_trace_demotion: false,
    allow_overlay_promotion: false,
    allow_recompute_promotion: false,
}
```

Initial `CompileKnobs::values`:

* `placement.profile = StrictOnePerBank`
* `observation.trace_demotion = None`
* `observation.optional_probe_floor = Diagnostic`
* `range.reduction_ceiling = AllowChunkedI16`
* `storage.recompute_promotion = None`
* `sram.page_aggression = KeepHotInWram`
* `rom_window.kernel_residency_bias = ProfileDefault`
* `rom_window.kernel_duplication_bias = ShareKernels`
* `overlay.promotion = None`
* `schedule.tile_search.allowed_classes = {SmallWorkingSet, Balanced}`
* `schedule.slice_coarsening = Fine`

`CompileKnobBounds`:

* `max_placement_profile = StrictOnePerBank` (cannot advance)
* `max_trace_demotion = None`
* `max_reduction_ceiling = AllowChunkedI16` (NOT `AllowRenormLoop`)
* `max_recompute_promotion = None`
* `max_sram_page_aggression = KeepHotInWram`
* `max_kernel_residency_bias = ProfileDefault`
* `max_kernel_duplication_bias = ShareKernels`
* `max_overlay_promotion = None`
* `allowed_tile_classes = {SmallWorkingSet, Balanced}`
* `max_slice_coarsening = Fine`
* `stage_iters = <RefinementIterBudget(1) for every stage>`

`KnobLockSet`: locks **everything except** `ScheduleTileSearch` and
`ScheduleSliceCoarsening`. Specifically, `locks.locked` contains:

```text
{ PlacementProfile, ObservationTraceDemotion, ObservationProbeSelection,
  RangeReductionCeiling, StorageRecomputePromotion,
  StorageMaterializationOverrides, SramPageAggression, SramSpillPolicy,
  RomKernelResidencyBias, RomKernelDuplicationBias,
  RomKernelResidencyOverrides, OverlayPromotion,
  ScheduleResourcePressure, StageIterationCeilings }
```

Rationale: BringUp is debug-clarity-first. The single iteration absorbs
deterministic local tile/slice fixes (e.g. RangePlan narrowing tile
classes after observing a slice budget overrun). All structural
decisions (placement, trace, overlay, recompute) are frozen so that
build failures are sharp signals about the inputs, not blurred by
silent autonomous policy mutation.

`Diverges from oracle:` the prompt says "BringUp profile:
max_refinement_iters=0 (refinement disabled — fail-fast for debug
clarity)." The bd-3ix oracle answer says
"`max_refinement_iters: 1`. Bringup defaults to 1, not 0. One
iteration absorbs deterministic local tile/slice fixes while still
failing loudly on placement / overlay / recompute / trace policy."

This RFC tracks both as candidates and asks **OQ-R1** (§21) to pick.
A `BringupFirstFit` / `--strict-first-fit` mode setting iters=0 is
implemented as a narrow policy artifact (`RepairPolicyProfile::
BringupFirstFit` and `RepairPolicy::bringup_strict_first_fit()`).
It is a separate setting from canonical Bringup, regardless of which
candidate wins.

#### 9.2.2 Default profile

```rust
RepairPolicy {
    max_refinement_iters: 4,
    allow_placement_profile_fallback: true,
    allow_trace_demotion: true,
    allow_overlay_promotion: true,
    allow_recompute_promotion: true,
}
```

Wait — this RFC follows the bd-3ix oracle answer, which says
`allow_placement_profile_fallback: true` for Default. The prompt
suggested "false" for Default. This RFC's candidate matches the
oracle answer (true). `Oracle question (OQ-R1.b)`: should Default
allow placement-profile fallback?

Initial `CompileKnobs::values`:

* `placement.profile = Budgeted`
* `observation.trace_demotion = None`
* `range.reduction_ceiling = AllowRenormLoop`
* `storage.recompute_promotion = PureSliceValues`
* `sram.page_aggression = BatchSramAccesses`
* `sram.spill_policy = SpillOnPressure`
* `rom_window.kernel_residency_bias = PreferCoResident`
* `rom_window.kernel_duplication_bias = ShareKernels`
* `overlay.promotion = None`
* `schedule.tile_search.allowed_classes = {SmallWorkingSet, Balanced, SwitchAmortized}`
* `schedule.slice_coarsening = ProfileDefault`

`CompileKnobBounds`:

* `max_placement_profile = PackedExperts`
* `max_trace_demotion = DropDiagnosticAndBestEffort`
* `max_reduction_ceiling = AllowRenormLoop`
* `max_recompute_promotion = PureResumeWindowValues`
* `max_sram_page_aggression = AllowColdSpills`
* `max_kernel_residency_bias = PreferWramOverlay`
* `max_kernel_duplication_bias = DuplicateTinyKernels`
* `max_overlay_promotion = KernelsAndLutFragments`
* `allowed_tile_classes = {SmallWorkingSet, Balanced, SwitchAmortized}`
* `max_slice_coarsening = CoarseWithinLatency`
* `stage_iters = <RefinementIterBudget(2) per stage>`

`KnobLockSet`: locks only `ScheduleResourcePressure` and
`StageIterationCeilings`. Everything else is repair-mutable.

#### 9.2.3 Trace profile (Invariant)

```rust
RepairPolicy {
    max_refinement_iters: 2,
    allow_placement_profile_fallback: false,
    allow_trace_demotion: false,                 // forced false under Invariant
    allow_overlay_promotion: false,
    allow_recompute_promotion: false,
}
```

Resolution rule for Trace (Invariant): resolve behavior-affecting
knobs from the *paired Default-compatible build* first, then **freeze**
them before trace instrumentation is inserted. If `TraceBudget` cannot
fit, fail with a clear diagnostic (`REPAIR-TraceBudgetExceeded`); do
**not** silently demote.

`KnobLockSet` (Invariant Trace): locks **everything**, including
behavior-affecting knobs (placement, observation, range, storage, sram,
rom_window, overlay) **and** schedule-class knobs (tile_search,
slice_coarsening, pressure, stage_iters). Effectively `RepairPolicy`
is irrelevant under Trace because nothing is repair-mutable; the only
loop iteration possible is "iteration 1: every stage runs successfully
or fails."

Trace (Flexible): falls back to Default-like behavior but emits
`trace_loss_report.json` + `observability.cert.json` to record the
demotion. **Out of scope for this RFC**; see F-B6 (`ObservationPlan`).

`Oracle question (OQ-R1.c)`: is `max_refinement_iters: 2` the right
ceiling for Trace? With everything locked, the ceiling is mostly
cosmetic (the loop will converge or fail fast). This RFC keeps 2 to
match the oracle answer.

#### 9.2.4 Recovery profile

```rust
RepairPolicy {
    max_refinement_iters: 6,
    allow_placement_profile_fallback: true,
    allow_trace_demotion: true,
    allow_overlay_promotion: true,
    allow_recompute_promotion: true,
}
```

Initial `CompileKnobs::values`:

* `placement.profile = Budgeted` (or `PackedExperts` if calibration
  indicates fit-first)
* `observation.trace_demotion = None`
* `range.reduction_ceiling = AllowRenormLoop`
* `storage.recompute_promotion = PureSliceValues`
* `sram.page_aggression = BatchSramAccesses`
* `rom_window.kernel_residency_bias = PreferCoResident`
* `rom_window.kernel_duplication_bias = ShareKernels`
* `overlay.promotion = KernelsOnly`
* `schedule.tile_search.allowed_classes = {SmallWorkingSet, Balanced, SwitchAmortized}`
* `schedule.slice_coarsening = ProfileDefault`

`CompileKnobBounds`:

* `max_placement_profile = PackedExperts`
* `max_trace_demotion = RequiredOnly` (most aggressive)
* `max_reduction_ceiling = AllowRenormLoop`
* `max_recompute_promotion = PureTokenValues` (most aggressive)
* `max_sram_page_aggression = FitFirstPagedSpills`
* `max_kernel_residency_bias = FitFirst`
* `max_kernel_duplication_bias = DuplicateToSatisfyWindow`
* `max_overlay_promotion = AnyOverlayable`
* `allowed_tile_classes = {SmallWorkingSet, Balanced, SwitchAmortized}`
* `max_slice_coarsening = CoarseWithinLatency` (rarely `EpochCoalesced`)
* `stage_iters = <RefinementIterBudget(3) per stage>`

`KnobLockSet`: locks only `ScheduleResourcePressure` and
`StageIterationCeilings` (same as Default).

Recovery does NOT bypass:

* semantic observations (always emitted),
* persistence correctness (PersistKind + commit-group invariants),
* `ResourceStateValidation` checks,
* `ReachabilityValidation` checks,
* interrupt-latency bounds,
* liveness bounds.

Recovery is fit-first within the typed envelope, not a free-for-all.

#### 9.2.5 Profile defaults summary table

| Profile  | iters | placement_fallback | trace_demotion | overlay_promotion | recompute_promotion | locked classes                                                  |
|----------|------:|--------------------|----------------|-------------------|---------------------|------------------------------------------------------------------|
| BringUp  | **1** | false              | false          | false             | false               | everything except `{TileSearch, SliceCoarsening}`                 |
| Default  | 4     | true               | true           | true              | true                | `{ResourcePressure, StageIters}`                                  |
| Trace    | 2     | false              | false          | false             | false               | everything                                                       |
| Recovery | 6     | true               | true           | true              | true                | `{ResourcePressure, StageIters}`                                  |

`Oracle question (OQ-R1)` (consolidating 1.a–1.c):
* should BringUp's `max_refinement_iters` be 0 or 1?
* should Default `allow_placement_profile_fallback` be true or false?
* should Trace's `max_refinement_iters` be 0, 1, or 2?

### 9.3 Lock semantics — which RepairPolicy fields are locked?

`Oracle question (OQ-R2)`: should `RepairPolicy` itself be locked?

This RFC's candidate position: **`RepairPolicy` is fully resolved at
policy time and immutable for the build duration**. There is no
runtime mutation of `RepairPolicy::*` by the loop driver, by the
`CompileRequest`, or by any later stage. The reasoning:

1. `RepairPolicy` toggles behavior of the loop. If they could mutate
   mid-build, the loop's termination proof would have to account for
   policy-changing-mid-flight, which is not currently in scope.
2. The lock set lives in `CompileKnobs::locks` (knob-side), not in
   `RepairPolicy` (policy-side). Locking is about which *knobs* may
   move, not about which *policies* may change.
3. Locking `RepairPolicy` would require introducing a parallel
   `RepairPolicyLockSet`, which doubles the lock surface for no
   observable benefit.

`Diverges from oracle:` the bd-3ix oracle answer is silent on this
point. This RFC's position is that `RepairPolicy` is structurally
immutable for the build duration, period. The chunk-10 oracle pass
should confirm.

The corollary: a `CompileRequest::repair_policy_overrides` field (if
introduced) tightens `RepairPolicy` at policy resolution time only.
After Stage 0.5 completes, `RepairPolicy` is frozen.

### 9.4 RepairPolicy resolution from inputs

At Stage 0.5 (F-B2), `RepairPolicy` is resolved as follows:

1. **Profile defaults**: seed from `CompileProfile`'s
   `repair_policy_default`.
2. **CompileRequest overrides**: a
   `CompileRequest::repair_policy_overrides: Option<RepairPolicyDelta>`
   field may *tighten* (set `allow_*: true → false`, lower
   `max_refinement_iters`). Loosening is rejected as
   `RepairPolicyOverrideLoosens` (Hard).
3. **HintBundle constraints**: a hint bundle may declare
   `RepairPolicyTightening` constraints. Same tighten-only rule.
4. **ObservabilityMode-driven forcing**: if
   `ObservabilityMode == Invariant`, `allow_trace_demotion` is forced
   `false` regardless of profile or override.
5. **Calibration**: no calibration-driven `RepairPolicy` mutation.
   Calibration affects `CompileKnobBounds` (specifically
   `ResourcePressureThresholds`), not `RepairPolicy`.

The resolved `RepairPolicy` is recorded in `policy_resolution.json`'s
`resolved.repair` field (already wired by F-B2/F-B4 §7.5).

Implementation note: the checked-in resolver consumes the typed profile
spec directly:

```rust
pub fn resolve_initial_knobs_from_profile_spec(
    profile: &CompileProfileSpec,
) -> Result<CompileKnobs, InitialKnobsResolveError>;

pub fn resolve_repair_policy_from_profile_spec(
    profile: &CompileProfileSpec,
) -> Result<RepairPolicy, InitialKnobsResolveError>;
```

Runtime chrome, objective, scheduler, and trace-budget inputs feed the
separate `resolve_resource_pressure_thresholds(...)` path rather than
the initial profile-spec-only knob resolver.

### 9.5 Oracle question summary for §9

| ID     | Question                                                            | This RFC's candidate                                       |
|--------|---------------------------------------------------------------------|------------------------------------------------------------|
| OQ-R1  | BringUp `max_refinement_iters`: 0 or 1?                              | 1 (per bd-3ix); a separate `BringupFirstFit` mode sets 0. |
| OQ-R1.b| Default `allow_placement_profile_fallback`: true or false?           | true (per bd-3ix).                                         |
| OQ-R1.c| Trace `max_refinement_iters` value?                                  | 2 (per bd-3ix).                                            |
| OQ-R2  | Is `RepairPolicy` itself lockable?                                   | No — fully resolved at policy time, immutable for build.  |
| OQ-R3  | Per-stage iteration budget vs global?                                | Both — global in `RepairPolicy`, per-stage in `ScheduleKnobs::stage_iters`. |

---

## 10. RepairProposal + ConstraintDelta + KnobDelta

### 10.1 Type-level

`RepairProposal` is the typed message a wrapped stage emits when its
local infeasibility has a candidate repair:

```rust
pub struct RepairProposal {
    pub source: PlanningStage,
    pub reason: RepairReason,
    pub tighten: ConstraintDelta,
    pub knob_delta: Option<KnobDelta>,         // canonical single-knob form
    pub resource_pressure: Option<ResourcePressureUpdate>,
}

pub struct ConstraintDelta {
    pub changes: Vec<KnobDelta>,
}
```

Most proposals carry exactly one `KnobDelta` and use the
`knob_delta: Some(...)` field. Multi-knob proposals (rare) leave
`knob_delta: None` and the driver iterates over `tighten.changes`.

The full `KnobDelta` enum (15 variants — bd-3ix plus the
SramSpillPolicy follow-up):

```rust
pub enum KnobDelta {
    AdvancePlacementProfile { to: PlacementProfile },
    SetTraceDemotion { to: TraceDemotionLevel },
    DisableOptionalProbes { probes: BTreeSet<TraceProbeId> },
    RaiseReductionCeiling {
        selector: Option<ReductionSelector>,
        to: ReductionPlanCeiling,
    },
    PromoteRecomputeLevel { to: RecomputePromotionLevel },
    ForceRecompute { values: BTreeSet<ValueSelector> },
    AdvanceSramPageAggression { to: SramPageAggression },
    AdvanceSramSpillPolicy { to: SramSpillPolicy },
    AdvanceKernelResidencyBias { to: KernelResidencyBias },
    AdvanceKernelDuplicationBias { to: KernelDuplicationBias },
    ForceKernelResidency {
        selector: KernelSelector,
        to: KernelResidency,
    },
    PromoteOverlay { to: OverlayPromotionLevel },
    NarrowTileClasses {
        selector: TileSelector,
        remaining: BTreeSet<TileCandidateClass>,
    },
    SetSliceCoarsening { to: SliceCoarseningLevel },
    UpdatePressureThreshold { update: ResourcePressureUpdate },
}

pub enum ResourcePressureUpdate {
    WramHot(PressureLimit<ByteBudget>),
    HramHot(PressureLimit<ByteBudget>),
    Bank0Rom(PressureLimit<ByteBudget>),
    SwitchableRomWindow(PressureLimit<ByteBudget>),
    SramWindow(PressureLimit<ByteBudget>),
    SliceCycles(PressureLimit<CycleBudget>),
    InterruptLatency(PressureLimit<CycleBudget>),
    TraceBytesPerFrame(PressureLimit<u16>),
    PersistBytesPerFrame(PressureLimit<u16>),
    OverlayInstallsPerFrame(PressureLimit<u8>),
    BankSwitchesPerToken(PressureLimit<u16>),
    SramPageSwitchesPerToken(PressureLimit<u16>),
}
```

`Oracle question (OQ-D1)`: is the `KnobDelta` enum closed at 15
variants? Specifically:

* Should there be a `RaiseSlackReservation { arena: ArenaId, bytes:
  ByteBudget }` variant? This RFC's candidate: no — slack is part of
  `ResourcePressureThresholds.<arena>` and is updated via
  `UpdatePressureThreshold`.
* Should there be a `CoalesceBankSwitchAggression { to:
  BankSwitchAggression }` variant? This RFC's candidate: no — bank
  switches are pressure-thresholded and coalescing is implicit in
  tile/coarsening choices.
* Should there be a `RetuneCommitGroupBoundaries { ... }` variant?
  This RFC's candidate: no — commit-group boundaries are an artifact
  of `StoragePlan`'s `Materialization::Persist`, not knob-controlled.

This RFC's default is to keep the enum closed. The chunk-10 oracle
pass is the tiebreaker.

`KnobDelta` invariant: every variant maps 1:1 to a `CompileKnobId`:

| KnobDelta variant                  | CompileKnobId                                |
|------------------------------------|----------------------------------------------|
| AdvancePlacementProfile            | PlacementProfile                              |
| SetTraceDemotion                   | ObservationTraceDemotion                      |
| DisableOptionalProbes              | ObservationProbeSelection                     |
| RaiseReductionCeiling              | RangeReductionCeiling                         |
| PromoteRecomputeLevel              | StorageRecomputePromotion                     |
| ForceRecompute                     | StorageMaterializationOverrides               |
| AdvanceSramPageAggression          | SramPageAggression                            |
| AdvanceSramSpillPolicy             | SramSpillPolicy                               |
| AdvanceKernelResidencyBias         | RomKernelResidencyBias                        |
| AdvanceKernelDuplicationBias       | RomKernelDuplicationBias                      |
| ForceKernelResidency               | RomKernelResidencyOverrides                   |
| PromoteOverlay                     | OverlayPromotion                              |
| NarrowTileClasses                  | ScheduleTileSearch                            |
| SetSliceCoarsening                 | ScheduleSliceCoarsening                       |
| UpdatePressureThreshold            | ScheduleResourcePressure                      |
| (no KnobDelta yet for StageIters)  | StageIterationCeilings                         |

`OQ-D1.b resolution`: `SramSpillPolicy` has a backing field and the
typed `AdvanceSramSpillPolicy { to: SramSpillPolicy }` delta. The only
remaining knob without a mutation variant is `StageIterationCeilings`,
which is intentionally loop-internal construction input rather than a
runtime repair lever.

`ScheduleTileSearch` intentionally remains an enum-only knob. Per-stage
or per-tile narrowing is not modeled as a second public knob; it is
carried by `KnobDelta::NarrowTileClasses { selector, remaining }`, so
the report preserves one stable `CompileKnobId::ScheduleTileSearch`
surface while still naming selector-specific narrowing evidence.

### 10.2 Admissibility predicate

A proposed `KnobDelta` is **admissible** iff all seven checks pass:

```rust
pub fn check_delta_admissible(
    delta: &KnobDelta,
    current: &CompileKnobs,
    policy: &RepairPolicy,
    observability: ObservabilityMode,
) -> Result<(), DeltaRejection>;
```

The seven checks, in order:

1. **Knob not locked.** `current.locks.is_locked(delta.knob_id())`
   must be false. Else `DeltaRejection::KnobLocked { knob }`.
2. **Policy toggle enabled.** Each `KnobDelta` variant maps to a
   `RepairPolicy.allow_*` toggle (see §10.2.2). The toggle must be
   true. Else `DeltaRejection::PolicyToggleDisabled { knob, toggle }`.
3. **Within bounds.** The new value must be at or below
   `current.bounds.<corresponding-max>` (for ordered enums) or in
   `current.bounds.<corresponding-allowed-set>` (for unordered).
   Else `DeltaRejection::BeyondBounds { knob, attempted, max }`.
4. **Supported mutable surface.** Every delta variant either mutates a
   backing `CompileKnobs` field/override or is rejected before
   application. `UpdatePressureThreshold` writes the typed
   `ResourcePressureThresholds` field in `ScheduleKnob`.
5. **Monotone.** The new value's rank must be ≥ the current value's
   rank (for ordered enums), or the new set must be a subset (for
   subset-removal sets) or superset (for superset-addition sets).
   Else `DeltaRejection::NotMonotone { knob, current, attempted }`.
6. **Pure recompute (for `KnobDelta::ForceRecompute` only).** Every
   `ValueSelector` in the delta must reference a value whose
   `effects_out: BTreeSet<EffectClass>` is empty (no `SequenceWrite`,
   `RngAdvance`, `PersistenceCommit`, or `TraceEmit`). Else
   `DeltaRejection::EffectfulRecompute { value }`.
7. **Invariant observability.** If `observability ==
   ObservabilityMode::Invariant`, the delta must not affect any of
   `ObservationTraceDemotion`, `ObservationProbeSelection`. Else
   `DeltaRejection::InvariantObservabilityViolation { knob }`.

`DeltaRejection`:

```rust
pub enum DeltaRejection {
    KnobLocked { knob: CompileKnobId },
    PolicyToggleDisabled { knob: CompileKnobId, toggle: &'static str },
    BeyondBounds { knob: CompileKnobId, attempted: String, max: String },
    NotMonotone { knob: CompileKnobId, current: String, attempted: String },
    InvariantObservabilityViolation { knob: CompileKnobId },
    EffectfulRecompute { value: ValueSelector },
}
```

#### 10.2.1 Why the check order matters

Order is fixed because each check's evidence is a strict subset of
the next:

1. Lock checks need no other inputs.
2. Policy-toggle checks need `RepairPolicy`.
3. Bounds checks need `current.bounds`.
4. Supported-mutable-surface checks need only the closed `KnobDelta`
   implementation capabilities.
5. Monotone checks need `current.values`.
6. Pure-recompute checks need `GbInferIR` effect-edge data
   (expensive — performed last among the cheap checks).
7. Invariant-observability is a derived check that may or may not
   apply; checked last so the rejection type is precise.

#### 10.2.2 KnobDelta-to-toggle mapping

| KnobDelta variant                  | RepairPolicy toggle             |
|------------------------------------|----------------------------------|
| AdvancePlacementProfile            | allow_placement_profile_fallback |
| SetTraceDemotion                   | allow_trace_demotion             |
| DisableOptionalProbes              | allow_trace_demotion             |
| RaiseReductionCeiling              | (always allowed if not locked)   |
| PromoteRecomputeLevel              | allow_recompute_promotion        |
| ForceRecompute                     | allow_recompute_promotion        |
| AdvanceSramPageAggression          | (always allowed if not locked)   |
| AdvanceSramSpillPolicy             | (always allowed if not locked)   |
| AdvanceKernelResidencyBias         | (always allowed if not locked)   |
| AdvanceKernelDuplicationBias       | (always allowed if not locked)   |
| ForceKernelResidency (target=WramOverlay) | allow_overlay_promotion   |
| ForceKernelResidency (target≠WramOverlay) | (always allowed if not locked) |
| PromoteOverlay                     | allow_overlay_promotion          |
| NarrowTileClasses                  | (always allowed if not locked)   |
| SetSliceCoarsening                 | (always allowed if not locked)   |
| UpdatePressureThreshold            | (always allowed if not locked and monotone within `max_pressure_thresholds`) |

`Oracle question (OQ-D1.c)`: are the four `allow_*` toggles
(placement_profile_fallback, trace_demotion, overlay_promotion,
recompute_promotion) the right granularity, or should there be a
fifth toggle for, say, `allow_kernel_duplication`? This RFC's
candidate: keep four. The bd-3ix oracle answer keeps four.

#### 10.2.3 The `AuthorizedRelaxation` operation

When `KnobDelta::AdvancePlacementProfile { to }` would *advance*
beyond the current bound (i.e. `to.rank() > current.bounds.max_placement_profile.rank()`),
the standard admissibility check fails with `BeyondBounds`. The
escape hatch:

* If `policy.allow_placement_profile_fallback == true`,
* and `LoopState::authorized_relaxation_used == false` (once-per-build),
* and the proposed `to` is a *legal `PlacementProfile` value* (not
  beyond `PackedExperts`),

then the driver may instead apply an **`AuthorizedRelaxation`**: it
steps `current.values.placement.profile` *backward* (to a *lower*
rank) — typically `PackedExperts → Budgeted` or `Budgeted →
StrictOnePerBank` — and records the operation as
`ConstraintOperation::AuthorizedRelaxation(reason)` in the
provenance chain.

The relaxation is applied to the *current value*, not to the bound.
The bound stays at its original `max`. This is the only operation
that makes the current value *decrease* in rank.

After applying the relaxation:

* `LoopState::authorized_relaxation_used` is set `true`.
* The relaxation is recorded in `repair_report.json` as a
  `RepairProposalRecord` whose outcome is
  `Accepted { applied_at_iter, knobs_delta: AuthorizedRelaxation { ... } }`.
* The loop continues from the earliest invalidated stage.

The bound on relaxation count: **at most one per build**. A second
relaxation request fails with
`DeltaRejection::AuthorizedRelaxationAlreadyUsed`.

`Oracle question (OQ-D2)`: is the once-per-build bound the right
contract? The alternative is once-per-knob-class (which only matters
because today only `PlacementProfile` is relaxable). This RFC's
candidate: once-per-build, since the only relaxable knob class is
`PlacementProfile`.

`Oracle question (OQ-D2.b)`: should `AuthorizedRelaxation` be
expressible as a separate `KnobDelta` variant
(`KnobDelta::RelaxPlacementProfile { to: PlacementProfile }`), or
remain only as a `ConstraintOperation` variant applied via
`AdvancePlacementProfile`? This RFC's candidate: keep it as a
`ConstraintOperation` variant. The driver detects the relax-vs-advance
case from the rank comparison, not from a separate `KnobDelta`.

### 10.3 Termination proof structure

The loop terminates. Proof:

**Lemma 1 (lattice height is finite).** `CompileKnobs::values` is a
product of finitely-many sub-knob lattices. Each sub-knob lattice is
either:

* an ordered enum with `< 256` declared ranks (in practice, ≤ 5),
* a finite set with cardinality bounded by the universe of valid
  set members (e.g. `BTreeSet<TileCandidateClass>` has 8 possible
  states from a 3-element universe), or
* a finite map with bounded key universe.

The product lattice has height bounded by the sum of sub-lattice
heights, which is finite. Call this height `H`.

**Lemma 2 (each accepted delta strictly advances).** A `KnobDelta` is
admissible only if it strictly advances at least one sub-knob's rank
(check 4 in §10.2 rejects no-op deltas as `NotMonotone`). Therefore
each accepted delta increases the cumulative "rank sum" of
`CompileKnobs::values` by ≥ 1.

**Lemma 3 (AuthorizedRelaxation is bounded).** Each build applies at
most one `AuthorizedRelaxation` operation, gated by
`LoopState::authorized_relaxation_used`. After the first relaxation,
subsequent relax requests are rejected.

**Theorem (loop termination).** For any input,
`run_refinement_loop(initial_state, ...)` returns in finitely many
iterations.

*Proof.* Let `K` be the number of accepted deltas. By Lemma 2, the
cumulative rank sum increases by ≥ 1 per accepted delta. By Lemma 1,
the rank sum has upper bound `H`. Therefore `K ≤ H`.

By Lemma 3, the relaxation is applied at most once. Each relaxation
*decreases* the rank sum by some amount `D` ≤ `H`, but is bounded to
one occurrence per build. Therefore the cumulative rank sum across
the entire loop run is bounded by `H + D ≤ 2H`.

The number of *rejected* deltas in any iteration is bounded by the
number of stages × the maximum proposals per stage per iteration
(both finite).

The number of *iterations* is bounded by `LoopState::global_iters_remaining`,
which starts at `RepairPolicy::max_refinement_iters` and is decremented
each iteration. After at most `max_refinement_iters` iterations, the
loop terminates with `TerminalState::GlobalBudgetExhausted` if not
already converged.

Therefore the loop terminates in at most `max_refinement_iters`
iterations, with at most `K + 1` accepted deltas, with at most a
finite number of rejected deltas per iteration. ∎

`Oracle question (OQ-D3)`: is the proof structure correct?
Specifically, is the `AuthorizedRelaxation` once-per-build bound
sufficient to prevent infinite oscillation between forward and
backward steps on the placement-profile ladder? This RFC's candidate:
yes, because (a) the relaxation is bounded to once, and (b) any
subsequent forward step uses a normal `AdvancePlacementProfile`, which
is bounded by `bounds.max_placement_profile`. The chunk-10 oracle pass
should confirm.

### 10.4 RepairReason taxonomy

```rust
pub enum RepairReason {
    /// RangePlan: the requested reduction structure exceeds i16
    /// accumulator headroom for some site.
    AccumulatorOverflow { site: ReductionSiteId, projected: u32, cap: u32 },

    /// StoragePlan: the materialized value set exceeds an arena's
    /// projected slack budget.
    ArenaOverflow { arena: ArenaId, projected_bytes: u32, cap_bytes: u32 },

    /// SramPagePlan: the active set cannot fit within a single
    /// SRAM page rotation.
    SramPagePressure { active_set_bytes: u32, page_bytes: u32 },

    /// RomWindowPlan: the simultaneously-visible ROM set for some
    /// hot operation exceeds the switchable window's 16 KiB budget.
    RomWindowOverflow { hot_op: HotOpId, projected_bytes: u32 },

    /// RomWindowPlan: a kernel cannot satisfy any residency choice.
    KernelResidencyImpossible { kernel: KernelSelector },

    /// OverlayPlan: the WRAM overlay region cannot host the proposed
    /// overlay set without violating the WRAM hot-arena pressure cap.
    OverlayBudgetExceeded { region: OverlayRegionId, projected_bytes: u32 },

    /// ArenaPlan: an expert payload exceeds its assigned bank's
    /// effective capacity.
    BankNotFitting { layer: LayerId, expert: ExpertId, slot: BudgetSlotId, payload_bytes: u32 },

    /// GbSchedIR: a slice's projected hard_cycles_to_safe_point
    /// exceeds the slice-cycles pressure cap.
    SliceCycleOverrun { slice: SliceId, projected: u32, cap: u32 },

    /// GbSchedIR: a slice's projected interrupt latency exceeds
    /// the interrupt-latency cap.
    InterruptLatencyExceeded { slice: SliceId, projected: u32, cap: u32 },

    /// ResourceStateValidation: a lease imbalance or illegal yield
    /// is reported.
    ResourceStateValidationFailed { detail: ResourceStateError },

    /// ScheduleCostAnalysis: the projected cost (cycles, bank
    /// switches, etc.) misses the CompileObjective target.
    ScheduleCostMissedTarget { objective: CompileObjective, missed_field: &'static str },

    /// Generic: a stage detected fit pressure outside the named cases.
    /// Used sparingly; prefer named reasons.
    StagePressureGeneric { detail: String },
}
```

`Oracle question (OQ-D4)`: is the taxonomy complete? Notable
candidates considered and rejected:

* `TraceBudgetExceeded` — not in this list because trace budget
  failures under Invariant Trace are *unrepairable* (lock everything),
  not repairable proposals. Under Flexible Trace, a `SetTraceDemotion`
  proposal is the standard repair, with the `RepairReason` being one
  of `OverlayBudgetExceeded` / `ArenaOverflow` / etc. (the underlying
  trace-causing pressure). **Oracle question (OQ-D4.a)**: should there
  be an explicit `TraceBudgetExceeded` reason?
* `BankSwitchOvercount` — covered by
  `ScheduleCostMissedTarget { missed_field: "bank_switches_per_token" }`.
* `OverlayInstallOvercount` — covered by `OverlayBudgetExceeded` plus
  `UpdatePressureThreshold` for `overlay_installs_per_frame`.
* `PersistenceWindowOverflow` — covered by `ArenaOverflow` for the
  persistent-page arena.

This RFC's candidate: keep the 12 named variants plus
`StagePressureGeneric` as a typed escape. The chunk-10 oracle pass
should confirm.

`Oracle question (OQ-D4.b)`: is `String` acceptable in the
`StagePressureGeneric` variant? This RFC's candidate: yes, but only
as a fallback; named reasons are preferred. The string field is
included in `repair_report.json` for postmortem.

### 10.5 Worked example: a typical proposal flow

StoragePlan iteration 2 of a Default-profile build:

```text
Inputs:
  - CompileKnobs::values.storage.recompute_promotion = PureSliceValues
  - CompileKnobs::bounds.max_recompute_promotion = PureResumeWindowValues
  - RepairPolicy.allow_recompute_promotion = true
  - ObservabilityMode::Flexible (Default profile)
  - GbInferIR with effect annotations on every value
  - Materialization plan attempt: ArenaOverflow at activation arena
    (projected 5_120 bytes vs cap 4_608 bytes; overrun 512 bytes)

StoragePlan emits:
  RepairProposal {
    source: PlanningStage::StoragePlan,
    reason: ArenaOverflow {
      arena: ArenaId::ActivationPingPong,
      projected_bytes: 5120,
      cap_bytes: 4608,
    },
    tighten: ConstraintDelta { changes: [
      KnobDelta::PromoteRecomputeLevel { to: PureResumeWindowValues },
    ] },
    knob_delta: Some(KnobDelta::PromoteRecomputeLevel { to: PureResumeWindowValues }),
    resource_pressure: None,
  }

Driver receives proposal; calls check_delta_admissible:
  1. Lock check: StorageRecomputePromotion not in locks.locked   ✓
  2. Toggle check: allow_recompute_promotion == true             ✓
  3. Bounds check: PureResumeWindowValues.rank() == 2;
     max.rank() == 2; 2 ≤ 2                                       ✓
  4. Monotone check: current = PureSliceValues (rank 1);
     attempted = PureResumeWindowValues (rank 2); 2 > 1            ✓
  5. (n/a — not ForceRecompute)
  6. Invariant check: observability == Flexible                   ✓

Decision: Ok. Driver:
  - records (proposal, Accepted { applied_at_iter: 2, knobs_delta: ... })
    in history
  - applies delta to knobs.values.storage.recompute_promotion
  - appends provenance entry:
    { source: RepairProposal(RepairProposalId(7)),
      operation: AppliedRepairProposal(RepairProposalId(7)),
      evidence: [proposal_hash, source_stage_StoragePlan_hash] }
  - invalidates StageCache for stages with canonical input
    StorageRecomputePromotion: {StoragePlan, SramPagePlan, ArenaPlan,
                                GbSchedIR, ScheduleCostAnalysis}
    (per §14)
  - re-runs from StoragePlan with knobs.values.storage.recompute_promotion
    = PureResumeWindowValues
```

The next iteration (3) re-runs StoragePlan. With the higher
recompute level, a few values are now recompute-flagged, the
activation arena's projected bytes drop to 4_352 (under the 4_608
cap), and StoragePlan succeeds. SramPagePlan, ArenaPlan, GbSchedIR
all succeed. ScheduleCostAnalysis runs and reports
`EstimatedCostDelta` indicating cycles increased by ~3% (the
recompute cost) but the build now fits. The loop converges with
`TerminalState::Converged`.

### 10.6 Oracle question summary for §10

| ID     | Question                                                            | This RFC's candidate                                       |
|--------|---------------------------------------------------------------------|------------------------------------------------------------|
| OQ-D1  | Is `KnobDelta` enum closed at 14 variants?                            | Yes (per bd-3ix); add `AdvanceSpillPolicy` for completeness. |
| OQ-D1.b| Add `AdvanceSpillPolicy` variant?                                    | Yes — to make CompileKnobId-to-KnobDelta a 1:1 mapping.    |
| OQ-D1.c| Are four `allow_*` toggles the right granularity?                     | Yes (per bd-3ix).                                          |
| OQ-D2  | Is `AuthorizedRelaxation` once-per-build the right bound?             | Yes — only PlacementProfile is relaxable.                  |
| OQ-D2.b| Is `AuthorizedRelaxation` a `ConstraintOperation` or `KnobDelta` variant? | `ConstraintOperation` (per §12).                       |
| OQ-D3  | Is the termination proof complete?                                   | Yes — assumes `AuthorizedRelaxation` once-per-build bound. |
| OQ-D4  | Is `RepairReason` taxonomy complete?                                 | Yes (12 named + `StagePressureGeneric`).                  |
| OQ-D4.a| Add explicit `TraceBudgetExceeded` reason?                            | No — covered by `OverlayBudgetExceeded`/`ArenaOverflow` under Flexible Trace. |
| OQ-D4.b| Is `String` acceptable in `StagePressureGeneric`?                     | Yes — fallback only, named reasons preferred.              |

---

## 11. Loop driver

### 11.1 Algorithm

The loop driver is a single function:

```rust
pub fn run_refinement_loop(
    initial_state: LoopState,
    pipeline: &mut CompilerPipeline,
) -> Result<TerminalState, RefinementLoopError>;
```

`CompilerPipeline` exposes per-stage `run` functions matching the
`WrappedStage` trait from §6.2. The driver's algorithm:

```rust
fn run_refinement_loop(initial_state, pipeline) -> Result<TerminalState, _> {
    let mut state = initial_state;

    loop {
        if state.global_iters_remaining == 0 {
            return Ok(TerminalState::GlobalBudgetExhausted);
        }

        // 1. Run wrapped stages in order.
        let mut iteration_proposals: Vec<RepairProposal> = vec![];
        let mut earliest_failing_stage: Option<PlanningStage> = None;

        for stage in PIPELINE_ORDER {
            // Decrement per-stage iteration counter.
            state.stage_iters_remaining.decrement(stage)?;

            let outcome = pipeline.run_stage(stage, &state.knobs);

            match outcome {
                StageOutcome::Success(_) => continue,
                StageOutcome::NeedsRepair(proposal) => {
                    iteration_proposals.push(proposal);
                    earliest_failing_stage = Some(stage);
                    break;  // do not run later stages with stale knobs
                }
                StageOutcome::UnrepairableFailure(err) => {
                    return Ok(TerminalState::StagedFailureUnrepairable {
                        stage,
                        last_error: err.to_string(),
                    });
                }
            }
        }

        // 2. If no proposal, the iteration succeeded.
        if iteration_proposals.is_empty() {
            // Now call ScheduleCostAnalysis (last stage) to confirm
            // the build fits the objective. If the cost report
            // indicates objective miss, the proposal will come from
            // ScheduleCostAnalysis itself; otherwise, converged.
            return Ok(TerminalState::Converged);
        }

        // 3. Validate admissibility for each proposal.
        let mut applied_any = false;
        for proposal in iteration_proposals {
            for knob_delta in proposal.tighten.changes.iter() {
                let decision = check_delta_admissible(
                    knob_delta,
                    &state.knobs,
                    &state.repair_policy,
                    state.observability,
                );

                let outcome = match decision {
                    Ok(()) => {
                        let prop_id = state.history.next_proposal_id();
                        let summary = apply_delta(knob_delta, &mut state.knobs, prop_id);
                        invalidate_stage_cache(knob_delta.knob_id());
                        applied_any = true;
                        ProposalOutcome::Accepted {
                            applied_at_iter: state.history.global_iters_used,
                            knobs_delta: summary,
                        }
                    }
                    Err(rejection) => {
                        // Special case: BeyondBounds for AdvancePlacementProfile
                        // when allow_placement_profile_fallback == true and
                        // !state.authorized_relaxation_used
                        // → consider AuthorizedRelaxation
                        if let DeltaRejection::BeyondBounds { knob, .. } = rejection {
                            if knob == CompileKnobId::PlacementProfile
                                && state.repair_policy.allow_placement_profile_fallback
                                && !state.authorized_relaxation_used
                            {
                                let prop_id = state.history.next_proposal_id();
                                let relax_summary = apply_authorized_relaxation(
                                    knob_delta,
                                    &mut state.knobs,
                                    proposal.reason.clone(),
                                    prop_id,
                                );
                                state.authorized_relaxation_used = true;
                                invalidate_stage_cache(knob_delta.knob_id());
                                applied_any = true;
                                ProposalOutcome::Accepted {
                                    applied_at_iter: state.history.global_iters_used,
                                    knobs_delta: relax_summary,
                                }
                            } else {
                                ProposalOutcome::Rejected { reason: rejection }
                            }
                        } else {
                            ProposalOutcome::Rejected { reason: rejection }
                        }
                    }
                };

                state.history.record(proposal.clone(), outcome);
            }
        }

        // 4. Decrement global iter counter (regardless of accept/reject).
        state.global_iters_remaining = state.global_iters_remaining.saturating_sub(1);
        state.history.global_iters_used += 1;

        // 5. If nothing was applied, no progress is possible. Fail.
        if !applied_any {
            // The proposal(s) were rejected; the originating stage
            // cannot proceed. Treat as StageBudgetExhausted on the
            // first failing stage if we hit per-stage budget, else
            // GlobalBudgetExhausted on next iteration.
            if let Some(stage) = earliest_failing_stage {
                if state.stage_iters_remaining.is_exhausted(stage) {
                    return Ok(TerminalState::StageBudgetExhausted { stage });
                }
                // Otherwise, we'll loop and hit GlobalBudgetExhausted
                // at the top of the next iteration if this remains
                // the case. But if every proposal is rejected, no
                // forward progress is being made — bail out.
                return Ok(TerminalState::GlobalBudgetExhausted);
            }
        }
        // else: continue to next iteration.
    }
}
```

The pseudocode above is canonical; the Rust implementation in
`gbf-codegen::refinement_loop` matches this structure.

### 11.2 Iteration ceiling and termination criteria

Two counters bound the loop:

* **Global**: `LoopState::global_iters_remaining: u8`, initialized
  from `RepairPolicy::max_refinement_iters`. Decremented every
  iteration regardless of accept/reject.
* **Per-stage**: `LoopState::stage_iters_remaining:
  StageIterationCeilings`, with one `RefinementIterBudget(u8)` per
  stage. Decremented when a stage runs (whether successful or
  proposing).

The effective per-iteration bound for stage S is:

```text
min(global_iters_remaining, stage_iters_remaining.<S>)
```

Termination occurs at any of:

1. **Convergence** — every stage in an iteration succeeds with no
   proposals. → `TerminalState::Converged`.
2. **Global ceiling** — `global_iters_remaining` reaches 0. →
   `TerminalState::GlobalBudgetExhausted`.
3. **Per-stage ceiling** — `stage_iters_remaining.<S>` reaches 0 with
   stage S still needing repair. →
   `TerminalState::StageBudgetExhausted { stage: S }`.
4. **Unrepairable failure** — a stage returns
   `StageOutcome::UnrepairableFailure`. →
   `TerminalState::StagedFailureUnrepairable { stage, last_error }`.

Only convergence is a build success. The other three are typed build
failures.

`Oracle question (OQ-L3)`: when the loop hits `GlobalBudgetExhausted`,
should the build fail outright (Hard diagnostic, no
`policy_resolution.json` `outcome: Passed`) or succeed with a soft
warning? This RFC's candidate: **fail outright**. A build that hits
the ceiling is, by definition, one whose final state did not satisfy
all stages. Emitting a "Passed" build with a budget-exhausted note
would let a non-converged build slip into production. The chunk-10
oracle pass should confirm.

### 11.3 ScheduleCostAnalysis as the objective oracle

`ScheduleCostAnalysis` (F-B14, Stage 11) is the **final stage of the
wrapped pipeline** and the **single objective oracle** for the loop.
It produces a `ScheduleCostReport`:

```rust
pub struct ScheduleCostReport {
    pub objective: CompileObjective,
    pub per_mode: BTreeMap<RuntimeMode, EstimatedCostDelta>,
    pub refs: Vec<EvidenceRef>,
}
```

The driver consumes this report in two ways:

1. **As a stage outcome.** If `ScheduleCostAnalysis` finds that the
   projected cost misses `CompileObjective`'s target on any mode, it
   returns `StageOutcome::NeedsRepair(RepairProposal { reason:
   ScheduleCostMissedTarget { ... }, tighten: ..., ... })`. The
   driver then applies the proposal as it would for any other
   stage's proposal.
2. **As a sanity check after every accepted delta.** The driver
   records the most recent report in `LoopState::last_cost_report`
   and uses it to decide whether the loop is *making progress* on
   the objective.

The progress predicate is:

```text
for each runtime_mode:
  delta = current_report.per_mode[mode] - last_report.per_mode[mode]
  if delta.cycles > some_threshold or delta.bank_switches > 0:
    record "objective regression" in repair_report.json
```

`Oracle question (OQ-L1)`: should the driver call
`ScheduleCostAnalysis` on **every** iteration that produced an
accepted delta, or only when convergence is contested (i.e. no later
stage has emitted a proposal but the build hasn't formally
converged)? This RFC's candidate: **only as part of the wrapped
pipeline's normal stage 11 invocation**. The driver does not call
`ScheduleCostAnalysis` out of band. If the cost report from a normal
iteration indicates objective miss, the report itself emits a
proposal; otherwise, the loop converges.

`Oracle question (OQ-L2)`: when an `EstimatedCostDelta` shows projected
cost *worsens* (e.g. recompute promotion adds cycles), but the
proposal otherwise enables fit, does the loop accept the proposal?

This RFC's candidate: **yes, accept and record the worsening**. The
alternative is to fail the build, but that would reject deltas that
trade cost for fit when fit is otherwise impossible. The
`repair_report.json` records the worsening with
`EstimatedCostDelta.evidence: EvidenceClass::Heuristic` (or
`Transferred`/`Measured` as available); a downstream review can
notice the regression and tune knob bounds in a future build.

The exception: if `EstimatedCostDelta` exceeds an explicit
`CompileObjective` *hard target* (not just the *soft* target), the
loop emits a `ScheduleCostMissedTarget` proposal in the next
iteration, which may chain further repairs. If the next iteration
also misses the hard target, the loop eventually hits the iteration
ceiling and fails.

### 11.4 Failure modes

#### 11.4.1 Loop diverges (no real divergence — bounded)

Termination is *mechanically guaranteed* by §10.3. The "loop
diverges" failure mode does not exist; the loop always terminates
within `max_refinement_iters`. A perceived "divergence" — the loop
hitting the ceiling without converging — is in fact
`TerminalState::GlobalBudgetExhausted`.

#### 11.4.2 Loop hits ceiling

`TerminalState::AcceptedRefinementBudgetExhausted { stage }`,
`TerminalState::GlobalBudgetExhausted`, or
`TerminalState::StageBudgetExhausted { stage }` are the typed
ceiling-hit terminal states. All three are build failures.

`repair_report.json` records:

```rust
TerminalStateRecord::GlobalBudgetExhausted
TerminalStateRecord::AcceptedRefinementBudgetExhausted { stage }
TerminalStateRecord::StageBudgetExhausted { stage }
```

Plus the full proposal history (every accepted and rejected proposal,
with reasons).

The build's `policy_resolution.json` records the *current* knob
values (which may include partially-applied deltas) and provenance
(which records every accepted delta up to the ceiling). The
`outcome` field is `Failed`.

#### 11.4.3 Proposal rejected by admissibility

Most rejections are not loop-level failures; they are recorded in
`repair_report.json` and the originating stage may emit a different
proposal in the next iteration.

A rejection becomes a loop-level failure only when:

* The same stage repeatedly emits the same rejected proposal, and
* No alternative proposal is forthcoming.

In that case, the stage will eventually exhaust its
`stage_iters_remaining` budget (or the global budget), and the loop
terminates with `*BudgetExhausted`.

Implementation note: `gbf-codegen::refinement_loop` records the
rejection, invokes the wrapped-stage `handle_rejected_repair` callback,
and applies a returned alternative proposal in the same stage attempt.
If the stage returns no alternative, the loop records a terminal
`StagedFailureUnrepairable` with the rejection reason.

#### 11.4.4 AuthorizedRelaxation requested but not policy-allowed

When a `KnobDelta::AdvancePlacementProfile` would exceed bounds and
`allow_placement_profile_fallback == false`, the rejection is
`DeltaRejection::BeyondBounds`. The relaxation is not considered.
This is the standard rejection path; the proposal is recorded in
`repair_report.json` with `outcome: Rejected { reason: BeyondBounds }`.

When `allow_placement_profile_fallback == true` but
`authorized_relaxation_used == true`, the second relaxation request
is rejected with `DeltaRejection::AuthorizedRelaxationAlreadyUsed`
(an additional variant added to `DeltaRejection`).

`Oracle question (OQ-L4)`: should `AuthorizedRelaxationAlreadyUsed`
be a separate `DeltaRejection` variant? This RFC's candidate: yes —
distinguishing the once-used case from `BeyondBounds` makes the
rejection reason precise.

#### 11.4.5 ScheduleCostAnalysis missing or incomplete

If `ScheduleCostAnalysis` is invoked but cannot produce a report
(e.g. calibration measurements are stale), the loop treats this as
`StageOutcome::UnrepairableFailure`. The build terminates with
`TerminalState::StagedFailureUnrepairable { stage:
ScheduleCostAnalysis, last_error: ... }`.

This case must be distinguished from *no calibration*, which is
caught at Stage 0 (F-B2) and never reaches the loop.

### 11.5 Observability of the loop

Every iteration's progress is recorded:

* `repair_report.json` (§13.2) records every proposal, the outcome,
  the iteration index, and the affected stages.
* `policy_resolution.json` (§13.1) records the *final* knob values
  and provenance; the chain entries make every applied proposal
  visible.
* `schedule_cost.json` (F-B14) records the per-mode
  `EstimatedCostDelta` for the *final* iteration's cost.
* `stages/` snapshots (under Trace builds) record the IR at every
  iteration's entry to each stage. (Note: Trace under Invariant has
  the loop frozen, so only one snapshot per stage exists.)

The combination is sufficient to reconstruct, postmortem, exactly
what the loop did:

1. Read `repair_report.json`'s proposal list to see what each stage
   asked for.
2. Cross-reference accepted proposals against
   `policy_resolution.json`'s `compile_knobs.provenance` chains to
   confirm the deltas actually applied.
3. Read `schedule_cost.json` to see the final objective math.

### 11.6 Oracle question summary for §11

| ID     | Question                                                            | This RFC's candidate                                       |
|--------|---------------------------------------------------------------------|------------------------------------------------------------|
| OQ-L1  | Call ScheduleCostAnalysis every iteration?                           | Only as part of the wrapped pipeline's stage 11 invocation. |
| OQ-L2  | Accept proposals with worsening EstimatedCostDelta?                  | Yes — accept and record the worsening.                     |
| OQ-L3  | TerminalState::GlobalBudgetExhausted: fail outright?                 | Yes — hard build failure.                                  |
| OQ-L4  | Add `AuthorizedRelaxationAlreadyUsed` to `DeltaRejection`?           | Yes.                                                       |

---

## 12. PolicyProvenance amendment to F-B2/F-B4 §10

This section is the **explicit amendment to F-B2/F-B4 §10**. It
documents the two new variants this RFC mints and their semantics.

### 12.1 What is amended

F-B2/F-B4 §7.4 defines the type:

```rust
// gbf-policy
pub enum PolicySource {
    TargetDefault,
    ProfileDefault,
    CompileRequestOverride,
    HintBundle,
    Calibration,
}

pub enum ConstraintOperation {
    SeedDefault,
    TightenBound,
    ApplyPreference,
    ApplyHardConstraint,
    ApplyOverride,
    ApplyCalibration,
}
```

F-B2/F-B4 §2.7 forbids `RepairProposal(_)` and explicitly says:

> `PolicySource::RepairProposal(_)` is **forbidden** in F-B2. F-B16
> introduces it. Any code path that could populate
> `RepairProposal(_)` must be unreachable in this chunk; tests must
> assert this.
>
> `ConstraintOperation::AuthorizedRelaxation` is also forbidden in
> this chunk. There are no authorized-relaxation fields, no
> bringup-relaxation records, and no report-visible relaxation
> operations in F-B2/F-B4.

### 12.2 What this RFC adds

This RFC adds **two `PolicySource` variants** and **two
`ConstraintOperation` variants**:

```rust
// gbf-policy — amended by F-B16
pub enum PolicySource {
    TargetDefault,
    ProfileDefault,
    CompileRequestOverride,
    HintBundle,
    Calibration,
    // NEW (F-B16):
    RepairProposal(RepairProposalId),
}

pub enum ConstraintOperation {
    SeedDefault,
    TightenBound,
    ApplyPreference,
    ApplyHardConstraint,
    ApplyOverride,
    ApplyCalibration,
    // NEW (F-B16):
    AppliedRepairProposal(RepairProposalId),
    AuthorizedRelaxation(RepairReason),
}
```

The new `RepairProposalId`:

```rust
#[repr(transparent)]
pub struct RepairProposalId(pub u32);
```

`RepairProposalId` is monotonically assigned per build, starting at
1, by the loop driver.

### 12.3 Semantics of the new variants

#### 12.3.1 `PolicySource::RepairProposal(RepairProposalId)`

A `ConstraintProvenance` chain entry whose `source` is
`RepairProposal(id)` indicates that the value was *introduced* by an
applied repair proposal. The `id` field links to the proposal record
in `repair_report.json`.

Invariants:

* Every `id` in the policy resolution chain matches at least one
  `RepairProposalRecord` in the same build's `repair_report.json`.
* The `id` is unique within a build.
* The chain entry's `evidence` field carries at least one
  `EvidenceRef::RepairProposal { id, source_stage, reason }`.

#### 12.3.2 `ConstraintOperation::AppliedRepairProposal(RepairProposalId)`

The "operation" complement to `PolicySource::RepairProposal(id)`. A
chain entry with `source: RepairProposal(id), operation:
AppliedRepairProposal(id)` records "value was changed by applying
proposal `id`." The two `id`s are required to match.

#### 12.3.3 `ConstraintOperation::AuthorizedRelaxation(RepairReason)`

The "operation" used when the loop driver applies the once-per-build
escape. A chain entry with `operation: AuthorizedRelaxation(reason)`
records "value was *relaxed* (moved backward in lattice) under the
authority of `RepairPolicy::allow_placement_profile_fallback`,
because of `reason`."

The corresponding `source` is also `RepairProposal(id)` — the
proposal whose `BeyondBounds` rejection triggered the relaxation. The
combination `(source: RepairProposal(id), operation:
AuthorizedRelaxation(reason))` is the unique signal that this entry
represents a relaxation, not a normal application.

### 12.4 Backward compatibility

The amendment is **additive enum variants only**. Existing F-B2/F-B4
code paths that exhaustively match `PolicySource` or
`ConstraintOperation` must be updated to handle the new variants.

This RFC's compatibility plan:

* Bump `gbf-policy`'s pass-version constants
  (`pass_version_resolve`, `pass_version_validate`) when this RFC's
  changes land. (F-B17 may change these in its own pass.)
* Bump the `policy_resolution.v1` schema's `crate_feature_set_hash`
  (per F-B2/F-B4 §7.8) so caches built before this RFC are
  invalidated.
* Do NOT bump the schema major version (`v1 → v2`); §13.1 picks
  additive minor bump.

`Oracle question (OQ-P1)`: is additive minor bump correct, or does
the addition of new enum variants warrant major bump? This RFC's
candidate: minor bump. Reasoning: pre-amendment readers see the new
variants as unrecognized enum values and can fail-closed; they do
not silently misinterpret them. Strict-additive enum changes are
conventionally minor bumps.

### 12.5 Tests required by the amendment

The amendment must be covered by tests in both `gbf-policy` and
`gbf-report`:

```bash
# gbf-policy serde round-trip with new variants
cargo test -p gbf-policy -- compile::policy_source_serde_round_trip_repair_proposal
cargo test -p gbf-policy -- compile::constraint_operation_serde_round_trip_authorized_relaxation
cargo test -p gbf-policy -- compile::repair_proposal_id_uniqueness_within_build

# gbf-report semantic validator accepts the new variants under F-B16
cargo test -p gbf-report -- f_b16_policy_resolution_v1_accepts_repair_proposal_provenance
cargo test -p gbf-report -- f_b16_policy_resolution_v1_accepts_authorized_relaxation_operation

# gbf-policy: forbidden-in-F-B2 test still passes for build paths
# that did not run F-B16 (i.e. M1 builds with max_refinement_iters: 0)
cargo test -p gbf-codegen -- f_b2_resolve_policy_no_repair_proposal_provenance_in_chunk
```

The last test is preserved — F-B2's "no `RepairProposal(_)` in this
chunk" assertion remains valid for builds that have not run the
loop. After F-B16 lands and the loop is wired up, builds that have
run the loop may legitimately carry `RepairProposal(_)` provenance.

### 12.6 What is NOT amended

This RFC does NOT change:

* `ResolvedCompilePolicy` shape (the existing fields).
* `CompileKnobs` struct shape (the existing five fields).
* `ConstraintFrame`, `KnobLockSet`, `CompileKnobOverrides` shapes
  (existing).
* `policy_resolution.json` top-level body shape (only the *contents*
  of `compile_knobs.provenance` chains change).
* `static_budget.json` shape (F-B4-owned; not relevant).
* `artifact_validation.json` shape (F-B2-owned; not relevant).
* `StageCache` key construction (existing — F-B17 owns the per-stage
  rules; F-B16 supplies §14 invalidation rules).

---

## 13. Report schemas

### 13.1 `policy_resolution.json` extensions

The existing F-B2/F-B4 §7.5 schema is extended **additively**. Concrete
changes:

#### 13.1.1 Body shape — unchanged

```rust
pub struct PolicyResolutionReportBody {
    pub artifact_identity: ArtifactIdentitySection,
    pub compile_request: CompileRequestSection,
    pub result: Option<PolicyResolutionSuccessSection>,
    pub hint_consumption: HintConsumptionSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}
```

This struct is *unchanged* by F-B16.

#### 13.1.2 PolicyResolutionSuccessSection — unchanged shape, enriched chains

```rust
pub struct PolicyResolutionSuccessSection {
    pub resolved: ResolvedSection,
    pub compile_knobs: CompileKnobsSection,
    pub provenance: PolicyProvenanceSection,
}
```

Field shapes are unchanged. The chains *inside* `compile_knobs.provenance`
(of type `Vec<CompileKnobProvenanceEntry>`) may now contain entries
whose `source: PolicySource` is `RepairProposal(_)` and whose
`operation: ConstraintOperation` is `AppliedRepairProposal(_)` or
`AuthorizedRelaxation(_)`.

The semantic validator amendment (§12.5):

```text
* every CompileKnobValues and CompileKnobBounds subfield references
  a provenance chain whose PolicySource values are all in
  TargetDefault | ProfileDefault | CompileRequestOverride | HintBundle |
  Calibration | RepairProposal(_)        // <-- now allowed
```

And:

```text
* a provenance chain may contain ConstraintOperation::AuthorizedRelaxation(_)
                                                                       // <-- now allowed
```

The chain ordering invariant is unchanged: chain entries are ordered
by application time (earliest first). A `RepairProposal(id)` entry
appears *after* the corresponding `TargetDefault → ProfileDefault →
HintBundle → CompileRequestOverride → Calibration` entries — i.e.
the proposal-applied entry is the *last* entry in any chain that
records a repair.

#### 13.1.3 New invariant: id-consistency

For every chain entry with `source: PolicySource::RepairProposal(id)`,
there must exist a matching `RepairProposalRecord` in the same build's
`repair_report.json` with the same `id`. This is asserted by
`PolicyResolutionReport::validate_semantics_with_repair_report(..., &repair_report)`.

#### 13.1.4 Schema version

Schema version stays at `policy_resolution.v1` (additive minor bump).
The `report_self_hash` is recomputed under the new domain separator
(if the canonical JSON now includes new variant strings).

`Oracle question (OQ-P1)`: schema version policy. This RFC's candidate:
additive minor — the version string remains `v1`, but the `crate_feature_set_hash`
in the StageCache key is bumped (per F-B2/F-B4 §7.8) so old caches
are invalidated.

### 13.2 `repair_report.json` (new — owned by F-B16)

This is a **new** report owned by F-B16 (T-B16.7). Schema:

```rust
// gbf-report::schemas::repair_report
pub struct RepairReportEnvelope {
    pub schema: String,                  // "repair_report.v1"
    pub report_self_hash: Hash256,
    pub report_inputs: RepairReportInputsSection,
    pub outcome: ReportOutcome,
    pub body: RepairReportBody,
}

pub struct RepairReportInputsSection {
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub static_budget_self_hash: Option<Hash256>,
    pub schedule_cost_self_hash: Option<Hash256>,
}

pub struct RepairReportBody {
    pub initial_knobs: CompileKnobsSnapshot,
    pub final_knobs: CompileKnobsSnapshot,
    pub proposals: Vec<RepairProposalRecord>,
    pub stage_iteration_counts: Vec<StageIterationCount>,
    pub global_iters_used: u8,
    pub termination: TerminalStateRecord,
    pub authorized_relaxation_applied: bool,
}

pub struct StageIterationCount {
    pub stage: PlanningStage,
    pub iterations: u8,
}

pub struct CompileKnobsSnapshot {
    pub values: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub overrides: CompileKnobOverrides,
    pub locks: KnobLockSet,
    /// Hash over canonical JSON of the snapshot (excluding self-hash).
    pub snapshot_hash: Hash256,
}

pub struct RepairProposalRecord {
    pub id: RepairProposalId,
    pub source_stage: PlanningStage,
    pub reason: RepairReason,
    pub delta: ConstraintDelta,
    pub knob_delta: Option<KnobDelta>,
    pub resource_pressure: Option<ResourcePressureUpdate>,
    pub estimated_cost_delta: Option<EstimatedCostDelta>,
    pub iter_emitted: u8,
    pub outcome: ProposalOutcome,
}

pub enum ProposalOutcome {
    Accepted {
        applied_at_iter: u8,
        knobs_delta: KnobDeltaSummary,
    },
    Rejected {
        reason: DeltaRejection,
    },
}

pub struct KnobDeltaSummary {
    pub changed_knobs: BTreeSet<CompileKnobId>,
    pub changes: Vec<KnobDelta>,
    pub per_knob: Vec<PerKnobDeltaSummary>,
    pub before: CompileKnobs,
    pub after: CompileKnobs,
}

pub struct PerKnobDeltaSummary {
    pub knob: CompileKnobId,
    pub before: String,    // canonical-string of the prior value
    pub after: String,     // canonical-string of the new value
    pub operation: ConstraintOperation,
}

pub enum TerminalStateRecord {
    Converged,
    AcceptedRefinementBudgetExhausted { stage: PlanningStage },
    GlobalBudgetExhausted,
    StageBudgetExhausted { stage: PlanningStage },
    StagedFailureUnrepairable { stage: PlanningStage, last_error: String },
}
```

Implementation note: the public JSON uses a deterministic row list for
`stage_iteration_counts` instead of a JSON object keyed by
`PlanningStage`, preserving the order invariant while avoiding
non-string report-facing map keys. `RepairReportInputsSection` is
required to carry non-zero `policy_resolution_self_hash` and
`artifact_validation_self_hash`; optional static-budget/schedule-cost
hashes may be `null` when those upstream reports are absent, but a
present hash may not be zero.

`CompileKnobsSnapshot` excludes provenance and hashes
`{values,bounds,overrides,locks}`. The report also retains whole
before/after `CompileKnobs` in `KnobDeltaSummary` for debugging, while
the RFC per-knob before/after/operation shape is emitted as
`knobs_delta.per_knob`.

#### 13.2.1 Semantic invariants

* `schema == "repair_report.v1"`.
* `report_self_hash` round-trips per F-B2/F-B4 §2.4.
* `outcome == Passed` iff `body.termination == TerminalStateRecord::Converged`.
* `outcome == Failed` iff `body.termination` is one of
  `GlobalBudgetExhausted | StageBudgetExhausted | StagedFailureUnrepairable`.
* `body.proposals` is ordered by `(iter_emitted, id)` ascending.
* Every `RepairProposalRecord.id` is unique within the report.
* For every `Accepted` outcome, the `knobs_delta.after` must match
  the corresponding chain entry in `policy_resolution.json`'s
  `compile_knobs.provenance.<knob>` final value.
* `body.global_iters_used` counts wrapped-stage execution attempts, not
  just accepted deltas. `LoopState::from_profile(...)` initializes that
  attempt budget from `ScheduleKnobs::stage_iteration_ceilings`, while
  `RepairPolicy::max_refinement_iters` separately bounds accepted repair
  deltas.
* `body.stage_iteration_counts` is a deterministic row list ordered by
  `PlanningStage` for stages that ran at all.
* `body.initial_knobs` is the `CompileKnobs` state at loop entry
  (i.e. the result of Stage 0.5 resolution).
* `body.final_knobs` is the `CompileKnobs` state at loop exit
  (whether Converged or Failed).
* `body.authorized_relaxation_applied` is true iff at least one
  `RepairProposalRecord.outcome == Accepted` carries a
  `knobs_delta.operation == AuthorizedRelaxation(_)`.

#### 13.2.2 Emit policy

`Oracle question (OQ-P2)`: is `repair_report.json` emitted on every
build, even zero-proposal builds?

This RFC's candidate: **yes**. Even on a `TerminalStateRecord::Converged`
build with `body.proposals.is_empty() == true`, the report is emitted.
Reasoning:

1. Reports are tooling input; consumers should never have to check for
   "did the report exist" in addition to "did the report show
   success."
2. The report's `body.initial_knobs == body.final_knobs` invariant on
   converged-empty builds is itself a useful regression check.
3. The report is small (proposals empty, termination Converged, two
   `CompileKnobsSnapshot` instances), and the cost of always-emitting
   is negligible.

A future amendment may relax this to "emit only if at least one
proposal was emitted *or* the build failed," but the default this
RFC sets is always-emit.

#### 13.2.3 Worked example: a zero-proposal converged build

```json
{
  "schema": "repair_report.v1",
  "report_self_hash": "sha256:...",
  "report_inputs": {
    "policy_resolution_self_hash": "sha256:...",
    "artifact_validation_self_hash": "sha256:...",
    "static_budget_self_hash": "sha256:...",
    "schedule_cost_self_hash": "sha256:..."
  },
  "outcome": "Passed",
  "body": {
    "initial_knobs": { "values": {...}, "bounds": {...}, "overrides": {...}, "locks": [...], "snapshot_hash": "sha256:abc..." },
    "final_knobs":   { "values": {...}, "bounds": {...}, "overrides": {...}, "locks": [...], "snapshot_hash": "sha256:abc..." },
    "proposals": [],
    "stage_iteration_counts": {
      "RangePlan": 1, "StoragePlan": 1, "SramPagePlan": 1,
      "RomWindowPlan": 1, "OverlayPlan": 1, "ArenaPlan": 1,
      "GbSchedIR": 1, "ResourceStateValidation": 1, "ScheduleCostAnalysis": 1
    },
    "global_iters_used": 1,
    "termination": "Converged",
    "authorized_relaxation_used": false
  }
}
```

(`initial_knobs.snapshot_hash == final_knobs.snapshot_hash` because no
delta was applied.)

#### 13.2.4 Worked example: an iteration-ceiling failure

```json
{
  "schema": "repair_report.v1",
  "outcome": "Failed",
  "body": {
    "initial_knobs": { ..., "snapshot_hash": "sha256:abc..." },
    "final_knobs":   { ..., "snapshot_hash": "sha256:def..." },
    "proposals": [
      {
        "id": 1,
        "source_stage": "ArenaPlan",
        "reason": { "BankNotFitting": { "layer": 0, "expert": 0, "slot": "expert_slot_0", "payload_bytes": 17000 } },
        "delta": { "changes": [{ "PromoteRecomputeLevel": { "to": "PureSliceValues" } }] },
        "knob_delta": { "PromoteRecomputeLevel": { "to": "PureSliceValues" } },
        "resource_pressure": null,
        "estimated_cost_delta": null,
        "iter_emitted": 1,
        "outcome": { "Accepted": { "applied_at_iter": 1, "knobs_delta": { "knob": "StorageRecomputePromotion", "before": "None", "after": "PureSliceValues", "operation": { "AppliedRepairProposal": 1 } } } }
      },
      {
        "id": 2,
        "source_stage": "ArenaPlan",
        "reason": { "BankNotFitting": { "layer": 0, "expert": 0, "slot": "expert_slot_0", "payload_bytes": 16800 } },
        "delta": { "changes": [{ "AdvancePlacementProfile": { "to": "PackedExperts" } }] },
        "knob_delta": { "AdvancePlacementProfile": { "to": "PackedExperts" } },
        "resource_pressure": null,
        "estimated_cost_delta": null,
        "iter_emitted": 2,
        "outcome": { "Rejected": { "reason": { "PolicyToggleDisabled": { "knob": "PlacementProfile", "toggle": "allow_placement_profile_fallback" } } } }
      },
      ... more rejected proposals each iteration ...
    ],
    "stage_iteration_counts": { ..., "ArenaPlan": 4, ... },
    "global_iters_used": 4,
    "termination": "GlobalBudgetExhausted",
    "authorized_relaxation_used": false
  }
}
```

The build hit the iteration ceiling because every proposal after #1
was rejected (by policy toggle in this case), and the original
infeasibility (BankNotFitting) was not actually fixed by recompute
promotion. The build fails; the report explains why.

### 13.3 What both reports together prove

After F-B16 lands, every successful build proves:

1. **Resolved policy is fully provenanced.** Every load-bearing knob
   value has a chain that ends at one of six `PolicySource` variants;
   every chain entry's `operation` is one of eight
   `ConstraintOperation` variants.
2. **Loop terminated correctly.** `repair_report.json`'s `termination
   == Converged` means the loop reached a fixed point where every
   stage was satisfied with no proposal. The chain of accepted
   proposals demonstrates how the lattice was advanced.
3. **No silent escape.** Every relaxation is recorded with reason. A
   build with `authorized_relaxation_used == true` is observable; a
   review can ask "why did this build need to step backward?"

A failed build's reports prove:

1. **Why the loop failed.** `termination` carries the reason
   (Exhausted, Unrepairable). The proposal list shows what was
   attempted.
2. **Where the failure originated.** `StageBudgetExhausted { stage
   }` and `StagedFailureUnrepairable { stage, last_error }` name
   the originating stage.
3. **What policy state is salvageable.** `final_knobs` records the
   state at exit; `initial_knobs` records the entry state. A
   reviewer can compare to see what the loop *did* manage to
   accomplish.

### 13.4 Oracle question summary for §13

| ID     | Question                                                            | This RFC's candidate                                       |
|--------|---------------------------------------------------------------------|------------------------------------------------------------|
| OQ-P1  | Schema version: minor or major bump?                                 | Minor — additive enum variants only.                       |
| OQ-P2  | Emit `repair_report.json` on every build?                            | Yes — even zero-proposal converged builds.                 |

---

## 14. StageCache algebra — knob-to-stage invalidation rules

F-B16 does not have its own stage and does not own a `StageCache`
key. What F-B16 *does* own is the **map from `CompileKnobId` to "set
of stages whose canonical-input bundle includes that knob."** The
loop driver consults this map after applying a delta to invalidate
the appropriate stage cache entries.

F-B17 is the workspace-wide sweep that *implements* the cache key
construction in each stage's module. F-B16 supplies the rules; F-B17
wires them.

### 14.1 The invalidation map

```rust
// gbf-codegen::stage_cache::invalidation
pub fn stages_affected_by(knob: CompileKnobId) -> &'static [PlanningStage] {
    match knob {
        CompileKnobId::PlacementProfile => &[
            PlanningStage::StoragePlan,
            PlanningStage::SramPagePlan,
            PlanningStage::RomWindowPlan,
            PlanningStage::OverlayPlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::ObservationTraceDemotion => &[
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::ObservationProbeSelection => &[
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::RangeReductionCeiling => &[
            PlanningStage::RangePlan,
            PlanningStage::StoragePlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::StorageRecomputePromotion => &[
            PlanningStage::StoragePlan,
            PlanningStage::SramPagePlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::StorageMaterializationOverrides => &[
            PlanningStage::StoragePlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
        ],
        CompileKnobId::SramPageAggression => &[
            PlanningStage::SramPagePlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::SramSpillPolicy => &[
            PlanningStage::SramPagePlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::RomKernelResidencyBias => &[
            PlanningStage::RomWindowPlan,
            PlanningStage::OverlayPlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::RomKernelDuplicationBias => &[
            PlanningStage::RomWindowPlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::RomKernelResidencyOverrides => &[
            PlanningStage::RomWindowPlan,
            PlanningStage::OverlayPlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
        ],
        CompileKnobId::OverlayPromotion => &[
            PlanningStage::OverlayPlan,
            PlanningStage::ArenaPlan,
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::ScheduleTileSearch => &[
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::ScheduleSliceCoarsening => &[
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::ScheduleResourcePressure => &[
            PlanningStage::GbSchedIR,
            PlanningStage::ScheduleCostAnalysis,
        ],
        CompileKnobId::StageIterationCeilings => &[],  // doesn't affect cache keys
    }
}
```

### 14.2 Why these particular sets

The reasoning per knob:

* **`PlacementProfile`**: changes which `PlacementProfile` is fed to
  every spatial-plan stage and to the backend's `PlacedRom`. Every
  stage that consumes the placement decision is affected. Note:
  `RangePlan` is NOT affected because it is purely logical (no
  placement consumption).
* **`ObservationTraceDemotion` / `ObservationProbeSelection`**: only
  affects which probes are emitted. `GbSchedIR` and
  `ScheduleCostAnalysis` consume the demoted set; earlier spatial
  stages are unaffected.
* **`RangeReductionCeiling`**: changes RangePlan's output (chunk
  structure, accumulator widths). StoragePlan consumes the
  materialized values; ArenaPlan assigns bytes; GbSchedIR commits
  the slices.
* **`StorageRecomputePromotion`**: changes StoragePlan's
  Materialization decisions (Recompute vs Materialize). Affects
  every downstream stage that consumes the materialization plan.
* **`StorageMaterializationOverrides`**: typed targeted overrides
  inserted into StoragePlan's `forced_recompute`. Affects only the
  stages that consume the materialization plan; cost analysis is
  optional (an override might not change cycle count meaningfully).
* **`SramPageAggression`**: changes SRAM working-set policy. Affects
  SramPagePlan, downstream ArenaPlan, GbSchedIR, ScheduleCostAnalysis.
* **`SramSpillPolicy`**: changes spill policy. Affects SramPagePlan
  and downstream slice plans / costs. ArenaPlan consumes the spill
  decision indirectly via the persistent-page geometry.
* **`RomKernelResidencyBias`**: changes RomWindowPlan's residency
  selection. Affects every downstream stage that consumes the
  residency.
* **`RomKernelDuplicationBias`**: changes RomWindowPlan's
  duplication strategy. Affects ArenaPlan (bytes), GbSchedIR
  (which dispatch stub is called), and cost.
* **`RomKernelResidencyOverrides`**: targeted overrides inserted
  into RomWindowPlan. Affects RomWindowPlan and downstream spatial
  stages.
* **`OverlayPromotion`**: changes overlay set. Affects OverlayPlan,
  ArenaPlan (WRAM reservation), GbSchedIR, cost.
* **`ScheduleTileSearch` / `ScheduleSliceCoarsening`**: changes
  GbSchedIR's tile choice. Affects GbSchedIR and cost; spatial plans
  are unaffected (already committed).
* **`ScheduleResourcePressure`**: changes pressure thresholds.
  Affects GbSchedIR's pressure tests and cost projection. Spatial
  plans use the *bounds* form, which is locked.
* **`StageIterationCeilings`**: doesn't affect cache keys
  (pass-internal counter).

### 14.3 Hashing strategy

`Oracle question (OQ-S2)`: should `CompileKnobs::values` be hashed as
a whole, or per sub-knob?

This RFC's candidate: **per sub-knob, then the cache key for stage S
is a tuple of the sub-knob hashes that affect S**. Concretely, each
stage's `StageCache` key includes a substructure:

```rust
pub struct StageCacheKnobBundle {
    pub placement_profile_hash: Option<Hash256>,         // present iff stage is in PlacementProfile's set
    pub observation_trace_demotion_hash: Option<Hash256>,
    pub observation_probe_selection_hash: Option<Hash256>,
    pub range_reduction_ceiling_hash: Option<Hash256>,
    pub storage_recompute_promotion_hash: Option<Hash256>,
    // ... one per CompileKnobId ...
}
```

A delta to `placement.profile` invalidates only those stages whose
`placement_profile_hash` field is `Some` and whose value changes.
Stages whose key has `placement_profile_hash: None` are not
invalidated by a placement-profile delta.

The alternative (hashing the whole `CompileKnobs::values`) would
invalidate every stage on every delta, which is correct but
unnecessarily aggressive.

Per-sub-knob hashing requires that each sub-knob's canonical-JSON
serialization is stable across builds (no nondeterministic key
ordering, no platform-dependent floating-point representation). This
is the standard F-B2/F-B4 §2.4 canonical-JSON convention.

### 14.4 Invalidation order

When a delta is applied, the driver:

1. Computes the affected stage set: `affected = stages_affected_by(knob)`.
2. Determines the **earliest** affected stage in pipeline order:
   `earliest = affected.iter().min_by_key(|s| s.pipeline_index())`.
3. Invalidates `StageCache` entries for *every* stage in `affected`
   (not just `earliest`).
4. Re-enters the loop body from `earliest`.

`Oracle question (OQ-S1)`: is "earliest in pipeline order" the right
re-entry point? This RFC's candidate: **yes**. The alternative is
to re-enter from the failing stage (the one that emitted the proposal),
but that would skip earlier stages whose cached output is now stale.
Re-entering from the earliest affected stage is the only safe choice.

### 14.5 Multi-knob deltas

When a `ConstraintDelta` carries multiple `KnobDelta` mutations
(rare), the affected stage set is the union of per-knob affected
sets, and the re-entry point is the earliest stage in the union.

### 14.6 What §14 does NOT specify

* The actual `StageCache` key construction inside each stage's
  module — owned by F-B17.
* The `StageCache` storage backend (`gbf-store`'s on-disk layout) —
  owned by F-A7 / F-B17.
* The `crate_feature_set_hash` policy when this RFC's amendments
  land — owned by F-B17 (cache invalidation must include the
  `gbf-policy` feature-set hash).

---

## 15. Diagnostic algebra — REPAIR-* codes

F-B16 introduces a typed diagnostic code prefix: `REPAIR-*`. These
are the codes used in `repair_report.json` and in any
`policy_resolution.json` diagnostics that arise from the loop.

### 15.1 REPAIR-* codes

```rust
pub enum RepairDiagnosticCode {
    /// A proposal failed admissibility for a recoverable reason
    /// (recorded as Rejected; the loop continues).
    REPAIR_Inadmissible,

    /// The loop hit max_refinement_iters without converging.
    REPAIR_CeilingHit,

    /// A per-stage iteration budget exhausted.
    REPAIR_StageCeilingHit,

    /// A relaxation was requested but RepairPolicy disallowed it.
    REPAIR_RelaxationDenied,

    /// The objective oracle (ScheduleCostAnalysis) reports cost
    /// regression after an applied delta.
    REPAIR_ObjectiveNotImproved,

    /// A stage returned an UnrepairableFailure.
    REPAIR_Unrepairable,

    /// AuthorizedRelaxation was used (informational; not Hard).
    REPAIR_RelaxationApplied,

    /// A multi-knob proposal had at least one inadmissible delta;
    /// the entire proposal is rejected.
    REPAIR_PartialDeltaInadmissible,

    /// The same proposal was emitted in two consecutive iterations
    /// without policy mutation; the loop detects this and rejects
    /// to avoid wasted iteration.
    REPAIR_DuplicateProposal,

    /// AuthorizedRelaxationAlreadyUsed.
    REPAIR_RelaxationAlreadyUsed,
}
```

### 15.2 Engineering rule 25 (added)

`Amends planv0`: this RFC adds a new engineering rule under the
"Engineering rules" section of `planv0.md`:

> **Rule 25.** `CompileKnobs` is the only mutable repair surface for
> the bounded feasibility loop. Passes may emit `RepairProposal`s,
> but only the loop controller may apply `ConstraintDelta`s. Every
> repair mutation must be typed, monotone in a declared finite order,
> bounded by profile policy, rejected if locked, and recorded in
> `policy_resolution.json` / `repair_report.json` with provenance.

This rule is enforced by:

* The `gbf-policy` API: `CompileKnobs` exposes only `&CompileKnobs`
  to wrapped stages; mutation goes through a private
  `apply_delta(&mut self, ...)` callable only by the loop driver.
* The semantic validators: `RepairReport::validate_semantics` rejects
  any `RepairProposalRecord` whose chain references a
  `CompileKnobId` not in the canonical enum.
* Tests: `gbf-codegen::refinement_loop::tests::passes_cannot_mutate_knobs`
  asserts that a wrapped stage attempting to call the private
  `apply_delta` does not compile (compile-fail test).

### 15.3 Diagnostic-to-rejection mapping

Most `RepairDiagnosticCode` variants map 1:1 to `DeltaRejection`
variants:

| Diagnostic code              | DeltaRejection mapped from           |
|------------------------------|--------------------------------------|
| REPAIR_Inadmissible          | (parent of all KnobLocked / ToggleDisabled / BeyondBounds / NotMonotone / InvariantObservabilityViolation / EffectfulRecompute) |
| REPAIR_CeilingHit            | (loop-level; not from DeltaRejection) |
| REPAIR_StageCeilingHit       | (loop-level)                         |
| REPAIR_RelaxationDenied      | DeltaRejection::PolicyToggleDisabled (with toggle == "allow_placement_profile_fallback" and current value at bound max) |
| REPAIR_ObjectiveNotImproved  | (cost-oracle-level; informational)   |
| REPAIR_Unrepairable          | (loop-level; from StageOutcome::UnrepairableFailure) |
| REPAIR_RelaxationApplied     | (informational; ConstraintOperation::AuthorizedRelaxation) |
| REPAIR_PartialDeltaInadmissible | (any DeltaRejection on a multi-knob delta) |
| REPAIR_DuplicateProposal     | (loop-level; not from DeltaRejection) |
| REPAIR_RelaxationAlreadyUsed | DeltaRejection::AuthorizedRelaxationAlreadyUsed |

---

## 16. Cross-stage interactions

### 16.1 F-B7 (RangePlan) interaction

RangePlan emits proposals when its requested reduction structure
exceeds i16 accumulator headroom. Common levers:

* `KnobDelta::RaiseReductionCeiling { selector: Some(site_id), to: AllowChunkedI16 }`
* `KnobDelta::RaiseReductionCeiling { selector: Some(site_id), to: AllowRenormLoop }`
* `KnobDelta::RaiseReductionCeiling { selector: None, to: AllowRenormLoop }` (global)

`RepairReason::AccumulatorOverflow { site, projected, cap }` carries
the projection details.

`StageCache` invalidation on `RangeReductionCeiling`:

```text
{RangePlan, StoragePlan, ArenaPlan, GbSchedIR, ScheduleCostAnalysis}
```

### 16.2 F-B8 (StoragePlan) interaction

StoragePlan emits proposals when its materialization decisions
overflow an arena's projected slack. Common levers:

* `KnobDelta::PromoteRecomputeLevel { to: PureSliceValues }`
* `KnobDelta::PromoteRecomputeLevel { to: PureResumeWindowValues }`
* `KnobDelta::PromoteRecomputeLevel { to: PureTokenValues }` (only if Recovery)
* `KnobDelta::ForceRecompute { values: { selector(s) } }` — typed
  pure-value selectors; admissibility checks the values are
  effect-free.

`RepairReason::ArenaOverflow { arena, projected_bytes, cap_bytes }`.

### 16.3 F-B9 (SramPagePlan) interaction

SramPagePlan emits proposals when active-set + spill geometry
exceeds a single SRAM page's 8 KiB visibility window. Common levers:

* `KnobDelta::AdvanceSramPageAggression { to: BatchSramAccesses }`
* `KnobDelta::AdvanceSramPageAggression { to: AllowColdSpills }`
* `KnobDelta::AdvanceSramPageAggression { to: FitFirstPagedSpills }` (Recovery)
* `KnobDelta::AdvanceSpillPolicy { to: SpillEager }` (if §10.1 OQ-D1.b
  resolves to "yes")

`RepairReason::SramPagePressure { active_set_bytes, page_bytes }`.

### 16.4 F-B10 (RomWindowPlan) interaction

RomWindowPlan emits proposals when the simultaneously-visible ROM
set exceeds the 16 KiB switchable window. Common levers:

* `KnobDelta::AdvanceKernelResidencyBias { to: PreferCoResident }`
* `KnobDelta::AdvanceKernelResidencyBias { to: PreferBank0Streaming }`
* `KnobDelta::AdvanceKernelResidencyBias { to: PreferWramOverlay }`
* `KnobDelta::AdvanceKernelDuplicationBias { to: DuplicateEntryStubs }`
* `KnobDelta::AdvanceKernelDuplicationBias { to: DuplicateTinyKernels }`
* `KnobDelta::ForceKernelResidency { selector, to: WramOverlay }`
* `KnobDelta::PromoteOverlay { to: KernelsOnly }` — bumps OverlayKnobs

Multiple kernel residency proposals may be combined in a single
multi-knob `ConstraintDelta`.

`RepairReason::RomWindowOverflow { hot_op, projected_bytes }` or
`RepairReason::KernelResidencyImpossible { kernel }`.

### 16.5 F-B11 (OverlayPlan) interaction

OverlayPlan emits proposals when the WRAM overlay region cannot host
the proposed overlay set. Common levers:

* `KnobDelta::PromoteOverlay { to: KernelsAndLutFragments }`
* `KnobDelta::PromoteOverlay { to: AnyOverlayable }` (Recovery)
* `KnobDelta::UpdatePressureThreshold { update: WramHot(...) }` —
  rare; only if `ScheduleResourcePressure` is unlocked.

`RepairReason::OverlayBudgetExceeded { region, projected_bytes }`.

### 16.6 F-B12 (ArenaPlan) interaction

ArenaPlan emits proposals when expert payloads exceed slot
capacities. Common levers:

* `KnobDelta::AdvancePlacementProfile { to: PackedExperts }` (gated
  by `allow_placement_profile_fallback`; may trigger
  `AuthorizedRelaxation` if already at max)
* `KnobDelta::PromoteRecomputeLevel { to: ... }` — indirect, asks
  StoragePlan to recompute more values.

`RepairReason::BankNotFitting { layer, expert, slot, payload_bytes }`.

ArenaPlan is a special case for the once-per-build
`AuthorizedRelaxation`: when `placement.profile` is at
`PackedExperts` and ArenaPlan still cannot fit, the relaxation may
step backward to `Budgeted` (or `StrictOnePerBank`) and re-run
F-B12 against the new profile. The relaxation is allowed only if
`allow_placement_profile_fallback == true` and
`!authorized_relaxation_used`.

### 16.7 F-B13 (GbSchedIR + ResourceStateValidation) interaction

GbSchedIR emits proposals when slice-level pressure is exceeded.
Common levers:

* `KnobDelta::NarrowTileClasses { selector, remaining: { Small, Balanced } }`
* `KnobDelta::SetSliceCoarsening { to: CoarseWithinLatency }`
* `KnobDelta::UpdatePressureThreshold { update: SliceCycles(...) }` —
  rare; pressure is usually locked.

`RepairReason::SliceCycleOverrun { slice, projected, cap }` or
`RepairReason::InterruptLatencyExceeded { slice, projected, cap }`.

ResourceStateValidation emits `RepairReason::ResourceStateValidationFailed`
when lease balance, illegal-yield, or residency-mismatch checks
fail. These are usually *unrepairable* by knob mutation (the failure
is structural, not dimensional) and result in
`StageOutcome::UnrepairableFailure`.

### 16.8 F-B14 (ScheduleCostAnalysis) interaction

ScheduleCostAnalysis is the **objective oracle**. It emits proposals
only when the projected cost misses `CompileObjective`'s targets.
Common levers:

* `KnobDelta::SetSliceCoarsening { to: CoarseWithinLatency }` — to
  reduce per-slice overhead
* `KnobDelta::NarrowTileClasses { selector, remaining: { Balanced, SwitchAmortized } }`
* `KnobDelta::PromoteRecomputeLevel { to: ... }` — indirect, asks
  StoragePlan to reduce arena pressure (which may indirectly reduce
  bank switches by improving locality)

`RepairReason::ScheduleCostMissedTarget { objective, missed_field }`.

### 16.9 F-B15 (Backend) interaction

F-B15 is **outside the loop body**. It runs after the loop converges.
The loop's `placement.profile` decision is read by F-B15's
`PlacedRom` substage; an `AuthorizedRelaxation` flipping the profile
backward forces F-B15 to re-run.

The interaction:

* During the loop: F-B15 is not invoked.
* After convergence: F-B15 reads `compile_knobs.values.placement.profile`
  and runs `PlacedRom` against it.
* If `PlacedRom` itself fails (a Backend-level infeasibility — e.g.
  branch range, far-call thunk insertion, ISR placement failure),
  this is **not** an F-B16 concern. It is a hard build failure
  emitted by F-B15.

`Oracle question (OQ-X1)`: should F-B15 also be wrapped by the
loop? Specifically, can `PlacedRom` fit-failures emit `RepairProposal`s?

This RFC's candidate: **no**. The reasoning:

1. F-B15 is post-loop in `planv0.md` line 1122 (Stage 12 backend).
2. F-B15 failures (branch range, ISR placement) are typically
   structural, not dimensional. Knob deltas would not fix them.
3. `PlacedRom` placement-profile failures *can* trigger
   `AuthorizedRelaxation`, but that is handled by F-B15 emitting an
   error that the orchestrator (`gbf-codegen::compile`) catches and
   maps to a re-loop request *if and only if*
   `allow_placement_profile_fallback == true`.

The chunk-10 oracle pass should confirm that F-B15 stays out of the
loop body.

### 16.10 F-B17 (StageCache integration sweep) interaction

F-B17 consumes §14's invalidation rules and wires per-stage cache
keys. F-B16 supplies the rules; F-B17 implements them. Specifically,
F-B17 must:

1. Add `StageCacheKnobBundle` field to every stage's cache key
   constructor.
2. Implement `Hash256` computation for each
   `StageCacheKnobBundle.<knob>_hash` field by canonicalizing the
   sub-knob and hashing per F-B2/F-B4 §2.4.
3. Bump every stage's `pass_version_*` constant when this RFC lands.
4. Bump every stage's `crate_feature_set_hash` to invalidate caches
   built before this RFC.

If F-B17 has not yet landed when F-B16 lands, the loop's correctness
is unaffected (the loop driver still applies deltas correctly), but
the cache will not honor knob-driven invalidation; in that case,
test fixtures must clear the cache between iterations to prove
correctness.

### 16.11 Oracle question summary for §16

| ID     | Question                                                            | This RFC's candidate                                       |
|--------|---------------------------------------------------------------------|------------------------------------------------------------|
| OQ-X1  | Should F-B15 (Backend) be wrapped by the loop?                       | No — F-B15 is post-loop; only AuthorizedRelaxation re-enters. |

---

## 17. Task DAG, compressed

The seven child task beads of F-B16, with their bd-3ix-recorded
dependency edges, mapped to RFC sections:

```text
T-B16.5 (bd-1r6b)  rename allow_profile_fallback → allow_placement_profile_fallback
   |  - workspace-wide refactor
   |  - blocked by T0.1 (workspace scaffold)
   |  - blocks T-B16.4
   |  - covered in §9.1 (RepairPolicy field declaration)
   v
T-B16.1 (bd-3aqf)  core CompileKnobs types
   |  - depends on T-B2.0 (bd-558z): SCHEMA already moved to T-B2.0
   |    per the 2026-05-07 amendment in bd-3aqf comments. T-B16.1 is
   |    recast as "any refinement-only schema extensions" or no-op.
   |  - covered in §8.1 (type-level contract)
   |
T-B16.2 (bd-22h4)  CompileKnobOverrides + typed selectors
   |  - depends on T-B16.1
   |  - introduces KernelSelector, ValueSelector, ReductionSelector,
   |    TileSelector, SliceClass, ReductionSiteId, SectionId
   |  - covered in §8.6
   |
T-B16.3 (bd-py29)  ConstraintDelta + KnobDelta + ResourcePressureUpdate +
   |               admissibility primitives (check_delta_admissible)
   |  - depends on T-B16.1, T-B16.2
   |  - 14 (or 15 if AdvanceSpillPolicy added per OQ-D1.b) KnobDelta variants
   |  - 12 ResourcePressureUpdate variants
   |  - covered in §10.1, §10.2
   v
T-B16.4 (bd-13tf)  per-profile defaults
   |  - depends on T-B16.1, T-B16.5, T11.2 (bd-ymo: BringUp profile)
   |  - resolves initial CompileKnobs from each CompileProfile
   |  - covered in §9.2
   v
T-B16.6 (bd-32w5)  FeasibilityRefinementLoop driver
   |  - depends on T-B16.1..T-B16.5 + every wrapped stage feature
   |    (F-B7..F-B14 — bd-2x0, bd-2k0, bd-3ns, bd-15n, bd-140, bd-3bw,
   |     bd-9ae, bd-prw)
   |  - implements the algorithm in §11.1
   |  - implements admissibility checks (calls T-B16.3)
   |  - implements StageCache invalidation (per §14)
   v
T-B16.7 (bd-2swd)  reports — extend policy_resolution.json + emit
                   repair_report.json
       - depends on T-B16.6, F-B2 (bd-2fj), F-F1 (bd-ow2e)
       - implements §13.1 + §13.2
```

### 17.1 Sequencing notes

* T-B16.5 lands first because the workspace-wide rename is small and
  unblocks T-B16.4.
* T-B16.1 is mostly delivered by T-B2.0 (bd-558z) per the 2026-05-07
  amendment on bd-3aqf. T-B16.1's residual scope is "extensions
  discovered when the F-B16 oracle resolution returns" — possibly
  zero-LOC, in which case the bead closes as a no-op.
* T-B16.2 and T-B16.3 are independent of each other but both depend
  on T-B16.1's types.
* T-B16.4 needs T-B16.1 (types) plus T-B16.5 (rename); it does not
  need T-B16.2 or T-B16.3 (it sets values, not deltas).
* T-B16.6 is the heavyweight task: it depends on every other T-B16.*
  plus on every wrapped-stage feature (F-B7..F-B14) being landed.
  This is intentional: the driver cannot be tested without the
  wrapped stages.
* T-B16.7 is last because the report schemas reflect the actual
  driver behavior (e.g. `TerminalStateRecord` mirrors
  `TerminalState` from the driver).

### 17.2 Critical path

```text
T-B2.0 (M1) ──> T-B16.1 (recast, possibly no-op)
T0.1 ──> T-B16.5 (rename) ──> T-B16.4 (defaults)
T-B16.1 ──> T-B16.2 ──> T-B16.3 ──> T-B16.6
                                    ↑
T-B16.4 ─────────────────────────────┘
F-B7..F-B14 (every wrapped stage) ───┘

T-B16.6 ──> T-B16.7 ──> F-B16 close
F-F1 (build report envelope) ──> T-B16.7
F-B2 (policy_resolution.json) ──> T-B16.7
```

The critical path is `F-B7..F-B14 → T-B16.6 → T-B16.7 → F-B16 close`
because each wrapped stage feature is independently large; F-B16
cannot ship before all of F-B7..F-B14 have wired in their
`StageOutcome::NeedsRepair` returns.

### 17.3 Mapping to RFC sections

| Task     | Bead     | Owns sections                                   |
|----------|----------|-------------------------------------------------|
| T-B16.1  | bd-3aqf  | §8.1, §8.7, §3.1–§3.16                          |
| T-B16.2  | bd-22h4  | §8.6, §3.6                                      |
| T-B16.3  | bd-py29  | §10.1, §10.2, §3.2–§3.4, §3.16                  |
| T-B16.4  | bd-13tf  | §9.1, §9.2, §9.4                                |
| T-B16.5  | bd-1r6b  | §9.1 (field name), §1.7                         |
| T-B16.6  | bd-32w5  | §11.1, §11.2, §11.4, §6.1–§6.5                  |
| T-B16.7  | bd-2swd  | §13.1, §13.2, §13.3                             |

§14 (StageCache invalidation) is jointly owned by T-B16.6 (driver
calls invalidate) and F-B17 (workspace-wide cache key sweep).

§12 (PolicyProvenance amendment) is jointly owned by T-B16.7 (report
schema) and the F-B2/F-B4 amendment work-item that lands when this
RFC ships.

---

## 18. Rejection classes (closure gate)

The chunk closes only when each rejection class below has at least
one passing test that demonstrates correct behavior.

### 18.1 Knob-locked rejection

```bash
cargo test -p gbf-policy -- delta::admissibility_locked
# Asserts: a delta against a knob in locks.locked → KnobLocked rejection.
```

### 18.2 Policy-toggle rejection

```bash
cargo test -p gbf-policy -- delta::admissibility_policy_toggle
# Asserts: KnobDelta::PromoteOverlay with allow_overlay_promotion: false
#          → PolicyToggleDisabled rejection.
```

### 18.3 Bounds rejection

```bash
cargo test -p gbf-policy -- delta::admissibility_bounds
# Asserts: KnobDelta::AdvancePlacementProfile { to: PackedExperts }
#          when bounds.max_placement_profile == Budgeted
#          → BeyondBounds rejection.
```

### 18.4 Monotonicity rejection

```bash
cargo test -p gbf-policy -- delta::admissibility_monotone
# Asserts: KnobDelta::SetTraceDemotion { to: None }
#          when current is DropBestEffort → NotMonotone rejection.
```

### 18.5 Effectful-recompute rejection

```bash
cargo test -p gbf-codegen -- refinement_loop::effectful_recompute_rejected
# Asserts: KnobDelta::ForceRecompute targeting a value with
#          effects_out: { SequenceWrite } → EffectfulRecompute rejection.
```

### 18.6 Invariant-observability rejection

```bash
cargo test -p gbf-codegen -- refinement_loop::invariant_blocks_trace_demotion
# Asserts: KnobDelta::SetTraceDemotion under
#          ObservabilityMode::Invariant → InvariantObservabilityViolation.
```

### 18.7 AuthorizedRelaxationAlreadyUsed rejection

```bash
cargo test -p gbf-codegen -- refinement_loop::relaxation_used_twice_rejected
# Asserts: a second AuthorizedRelaxation request after the first was
#          consumed → AuthorizedRelaxationAlreadyUsed rejection.
```

### 18.8 Loop-level rejection: AcceptedRefinementBudgetExhausted

```bash
cargo test -p gbf-codegen -- refinement_loop::accepted_refinement_budget_exhausted_records_admissible_proposal
# Asserts: an admissible proposal emitted after the accepted-delta
#          budget is consumed is recorded as a rejected proposal and
#          terminates with TerminalState::AcceptedRefinementBudgetExhausted.
```

### 18.9 Loop-level rejection: GlobalBudgetExhausted

```bash
cargo test -p gbf-codegen -- refinement_loop::global_budget_exhausted
# Asserts: a fixture whose every iteration produces a rejected
#          proposal → TerminalState::GlobalBudgetExhausted within
#          max_refinement_iters iterations.
```

### 18.10 Loop-level rejection: StageBudgetExhausted

```bash
cargo test -p gbf-codegen -- refinement_loop::stage_budget_exhausted
# Asserts: a fixture whose ArenaPlan exceeds its per-stage budget →
#          TerminalState::StageBudgetExhausted { stage: ArenaPlan }.
```

### 18.11 Unrepairable failure pass-through

```bash
cargo test -p gbf-codegen -- refinement_loop::unrepairable_failure_passes_through
# Asserts: a fixture where ResourceStateValidation returns
#          UnrepairableFailure → TerminalState::StagedFailureUnrepairable.
```

### 18.11 PolicyProvenance new-variant round-trip

```bash
cargo test -p gbf-policy -- compile::policy_source_serde_round_trip_repair_proposal
cargo test -p gbf-policy -- compile::constraint_operation_serde_round_trip_authorized_relaxation
cargo test -p gbf-report -- f_b16_policy_resolution_v1_accepts_repair_proposal_provenance
cargo test -p gbf-report -- f_b16_policy_resolution_v1_accepts_authorized_relaxation_operation
```

### 18.12 repair_report.json schema and round-trip

```bash
cargo test -p gbf-codegen -- reports::repair_report_round_trip
cargo test -p gbf-codegen -- reports::repair_report_includes_rejected
cargo test -p gbf-report -- schemas::repair_report_versioned
cargo test -p gbf-test    -- e2e_repair_report
```

### 18.13 Determinism

```bash
cargo test -p gbf-codegen -- refinement_loop::deterministic_byte_identical_two_runs
# Asserts: two consecutive runs with byte-identical inputs produce
#          byte-identical repair_report.json (and policy_resolution.json).
```

### 18.14 Integration: every wrapped stage emits at least one proposal

```bash
cargo test -p gbf-codegen -- refinement_loop::range_plan_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::storage_plan_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::sram_page_plan_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::rom_window_plan_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::overlay_plan_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::arena_plan_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::sched_ir_emits_proposal
cargo test -p gbf-codegen -- refinement_loop::schedule_cost_analysis_emits_proposal
```

These tests assert the wiring: every wrapped stage's
`StageOutcome::NeedsRepair` path is exercised at least once on a
synthetic fixture.

### 18.15 Convergence on happy-path fixture

```bash
cargo test -p gbf-codegen -- refinement_loop::convergence_no_proposals
# Asserts: a happy-path fixture (Default profile, fits without repair)
#          → TerminalState::Converged in iteration 1, with empty
#          proposal list and matching initial/final knobs snapshot
#          hashes.
```

### 18.16 AuthorizedRelaxation happy path

```bash
cargo test -p gbf-codegen -- refinement_loop::authorized_relaxation_advances_then_relaxes
# Asserts: a fixture that advances PlacementProfile to PackedExperts,
#          fails ArenaPlan, then relaxes back to Budgeted under
#          allow_placement_profile_fallback: true.
```

---

## 19. Proof obligations

This section enumerates the proofs the chunk must discharge. Each is
a typed claim about the loop's behavior.

### 19.1 Termination

**Claim.** For any input, `run_refinement_loop` returns in finitely
many iterations.

**Proof structure.** §10.3 Lemmas 1–3 + Theorem.

**Mechanical check.** §18.8, §18.9 demonstrate that the iteration
ceiling fires when no progress is possible.

### 19.2 Admissibility

**Claim.** Every accepted delta passes the six admissibility checks
(§10.2) at the moment of application.

**Proof structure.** The loop driver (§11.1) calls
`check_delta_admissible` before applying every delta and rejects
non-admissible deltas without applying.

**Mechanical check.** §18.1–§18.7 demonstrate each rejection class.
Additionally:

```bash
cargo test -p gbf-codegen -- refinement_loop::accepted_deltas_pass_all_six_checks
# Asserts: in a long-running fixture with many accepted proposals,
#          every accepted KnobDelta in repair_report.json passes
#          check_delta_admissible against the knob state at its
#          applied_at_iter.
```

### 19.3 Monotonicity (no hidden relaxation)

**Claim.** Every accepted delta is monotone in the relevant
sub-knob's lattice, *unless* its `ConstraintOperation` is
`AuthorizedRelaxation(_)`.

**Proof structure.** §10.2 admissibility check #4 enforces
monotonicity for normal deltas. §10.2 §10.2.3 documents the single
exception (AuthorizedRelaxation), which carries a typed operation
variant.

**Mechanical check.**

```bash
cargo test -p gbf-policy -- monotone::every_non_relaxation_delta_strictly_advances
# Asserts: for every RepairProposalRecord in repair_report.json with
#          outcome.knobs_delta.operation != AuthorizedRelaxation(_),
#          before.rank() < after.rank() (or the corresponding
#          set-membership invariant for unordered knobs).
```

### 19.4 Provenance completeness

**Claim.** Every knob value in `policy_resolution.json`'s
`compile_knobs.global` and `compile_knobs.bounds` has a non-empty
provenance chain.

**Proof structure.** F-B2/F-B4 §7.5 already enforces this for the
five M1 `PolicySource` variants. F-B16's amendment (§12) preserves
the invariant: every applied repair proposal extends the chain.

**Mechanical check.**

```bash
cargo test -p gbf-report -- policy_resolution::every_knob_has_non_empty_chain_after_repair
# Asserts: after a converged loop run, every CompileKnobValues field
#          has chain.len() >= 1.
```

### 19.5 ID consistency between reports

**Claim.** Every `RepairProposalId` referenced in
`policy_resolution.json`'s provenance chains is also present in
`repair_report.json`'s `body.proposals` list.

**Proof structure.** §13.1.3 invariant; the validator in
`PolicyResolutionReport::validate_semantics_with_repair_report` checks
this directly.

**Mechanical check.**

```bash
cargo test -p gbf-report -- cross_report::policy_resolution_repair_report_id_consistency
# Asserts: for every proposal id in policy_resolution.json's chains,
#          the same id appears in repair_report.json with matching
#          source_stage and reason.
```

### 19.6 Determinism

**Claim.** Two runs with byte-identical inputs produce byte-identical
`repair_report.json` and byte-identical `policy_resolution.json`.

**Proof structure.** Every randomness source is named: there are
none. The loop driver iterates over stages in fixed order; proposals
within an iteration are processed in `Vec` order; canonical JSON
preserves order.

**Mechanical check.** §18.13.

### 19.7 No silent escape

**Claim.** No delta loosens the lattice without recording an
`AuthorizedRelaxation(_)` operation.

**Proof structure.** §10.2 admissibility check #4 rejects backward
deltas as `NotMonotone`. The single exception path (§10.2.3) records
`ConstraintOperation::AuthorizedRelaxation(reason)` in the chain.

**Mechanical check.**

```bash
cargo test -p gbf-policy -- monotone::no_chain_entry_loosens_without_authorized_relaxation
# Asserts: for every chain entry where rank(after) < rank(before),
#          operation == AuthorizedRelaxation(_).
```

### 19.8 ScheduleCostAnalysis is the unique objective oracle

**Claim.** No code path other than `ScheduleCostAnalysis` produces
an `EstimatedCostDelta` consumed by the loop.

**Proof structure.** Code-level: `EstimatedCostDelta` is constructed
only in `gbf-codegen::stages::schedule_cost_analysis`. The
`gbf-codegen::refinement_loop` module reads but does not construct.

**Mechanical check.**

```bash
# Compile-fail test: any other crate constructing EstimatedCostDelta
# fails the build.
cargo test -p gbf-codegen -- refinement_loop::estimated_cost_delta_only_constructed_by_schedule_cost_analysis
```

### 19.9 Loop is bounded by max_refinement_iters

**Claim.** `body.global_iters_used <=
RepairPolicy::max_refinement_iters` for every emitted
`repair_report.json`.

**Proof structure.** §11.1 algorithm step 4 decrements the global
counter every iteration; the loop exits when the counter reaches 0.

**Mechanical check.**

```bash
cargo test -p gbf-codegen -- refinement_loop::global_iters_never_exceeds_max
# Asserts: across many fixtures, body.global_iters_used <=
#          policy.max_refinement_iters.
```

### 19.10 AuthorizedRelaxation is bounded once-per-build

**Claim.** `body.authorized_relaxation_used == true` implies at most
one chain entry has `operation: AuthorizedRelaxation(_)`.

**Proof structure.** `LoopState::authorized_relaxation_used` is set
to `true` after the first relaxation; subsequent relaxation requests
are rejected as `AuthorizedRelaxationAlreadyUsed`.

**Mechanical check.**

```bash
cargo test -p gbf-codegen -- refinement_loop::authorized_relaxation_at_most_once
# Asserts: in any repair_report.json, count of accepted proposals
#          with knobs_delta.operation == AuthorizedRelaxation(_) <= 1.
```

---

## 20. End-to-end theorem

**Theorem.** When F-B16 lands, every successful build proves:

1. **Resolved policy is auditable.** Every load-bearing
   `CompileKnobs::values` field has a provenance chain whose
   entries enumerate every operation that produced its current
   value, and the chain ends at a typed `PolicySource` variant.
2. **Loop terminated correctly.** `repair_report.json` records
   `TerminalStateRecord::Converged`, indicating the loop reached a
   fixed point where every wrapped stage was satisfied with no
   further proposals.
3. **Every applied delta was admissible.** Every
   `RepairProposalRecord.outcome == Accepted` in
   `repair_report.json` corresponds to a delta that passed all six
   admissibility checks (§10.2) at the moment of application.
4. **Every applied delta was monotone (or explicitly relaxed).**
   For every applied delta, either (a) it strictly advances the
   relevant sub-knob's rank, or (b) its operation is
   `AuthorizedRelaxation(_)` and `body.authorized_relaxation_used`
   was previously `false`.
5. **The objective oracle was consulted.** The build's
   `schedule_cost.json` (F-B14) reports the final
   `EstimatedCostDelta` against the build's `CompileObjective`;
   `repair_report.json`'s final state has been re-validated against
   the cost report.
6. **Every rejected proposal is recorded.** `repair_report.json`
   includes every rejected proposal with its
   `DeltaRejection` reason, so postmortem reviews can trace why a
   build failed *and* why specific repairs were not viable.

When F-B16 lands, every failed build proves:

1. **Failure mode is typed.** `TerminalStateRecord` is one of
   `GlobalBudgetExhausted | StageBudgetExhausted { stage } |
   StagedFailureUnrepairable { stage, last_error }`. There is no
   `String`-only failure path.
2. **Failure attribution is explicit.** Stage-budget and
   unrepairable failures name the originating stage; global-budget
   failures show the per-stage iteration counts so the reviewer can
   see which stage consumed the most budget.
3. **Partial state is preserved.** `final_knobs` records the loop's
   state at exit; comparing to `initial_knobs` shows what the loop
   *did* manage to accomplish before failing.

The two reports together (`policy_resolution.json` +
`repair_report.json`) are the single source of truth for "what
policy governed this build, what proposals were emitted, and what
happened to each." No other report duplicates this information.

---

## 21. Oracle question consolidated list

This section consolidates every `Oracle question:` annotation in the
RFC. The chunk-10 oracle pass must answer each question (or
explicitly defer) before the bead can close.

### 21.1 CompileKnobs shape (OQ-K*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-K1   | §8.1    | Are eight sub-knobs the *complete* set?                                | Yes (per bd-3ix).                                          | Hard     |
| OQ-K1.a | §8.1    | Separate `BankSwitchCoalescingLevel` knob?                              | No — subsumed under tile_search + slice_coarsening.        | Soft     |
| OQ-K1.b | §8.1    | Separate `CoResidencyAggression` knob?                                  | No — subsumed under KernelResidencyBias.                   | Soft     |
| OQ-K1.c | §8.1    | Per-arena reservation slack as own knob?                                | No — subsumed under ResourcePressureThresholds.            | Soft     |
| OQ-K1.d | §8.1    | Chunk tile size as own knob?                                            | No — derived from ReductionPlanCeiling.                    | Soft     |
| OQ-K2   | §8.1.2  | Are declared monotone orders correct?                                  | Yes (per bd-3ix).                                          | Hard     |
| OQ-K3   | §8.2    | Are the bound types correct?                                           | Yes (per bd-3ix).                                          | Hard     |
| OQ-K4   | §8.3    | `CompileKnobId` granularity for locks?                                 | Yes — keep BiasOverride distinct from BiasGlobal.          | Hard     |
| OQ-K5   | §8.4    | Add `ConstraintOperation::AppliedRepairProposal`/`AuthorizedRelaxation`? | Yes — both.                                               | Hard     |

### 21.2 RepairPolicy shape and defaults (OQ-R*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-R1   | §9.2.1  | BringUp `max_refinement_iters`: 0 or 1?                                 | 1 (per bd-3ix); separate `BringupFirstFit` mode for 0.    | Hard     |
| OQ-R1.b | §9.2.2  | Default `allow_placement_profile_fallback`: true or false?              | true (per bd-3ix).                                         | Hard     |
| OQ-R1.c | §9.2.3  | Trace `max_refinement_iters` value?                                    | 2 (per bd-3ix).                                            | Soft     |
| OQ-R2   | §9.3    | Is `RepairPolicy` itself lockable?                                     | No — fully resolved at policy time, immutable for build.  | Hard     |
| OQ-R3   | §9.1    | Per-stage iteration budget vs global?                                  | Both — global in RepairPolicy, per-stage in ScheduleKnobs. | Hard     |

### 21.3 RepairProposal/ConstraintDelta/KnobDelta shape (OQ-D*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-D1   | §10.1   | Is `KnobDelta` enum closed at 14 variants?                              | Yes (per bd-3ix); add `AdvanceSpillPolicy` for completeness. | Hard   |
| OQ-D1.b | §10.1   | Add `AdvanceSpillPolicy` variant?                                      | Yes — to make CompileKnobId-to-KnobDelta a 1:1 mapping.    | Soft     |
| OQ-D1.c | §10.2.2 | Are four `allow_*` toggles the right granularity?                       | Yes (per bd-3ix).                                          | Hard     |
| OQ-D2   | §2.4, §10.2.3 | Is `AuthorizedRelaxation` once-per-build the right bound?         | Yes — only PlacementProfile is relaxable.                  | Hard     |
| OQ-D2.b | §10.2.3 | Is `AuthorizedRelaxation` a `ConstraintOperation` or `KnobDelta` variant? | `ConstraintOperation` (per §12).                       | Hard     |
| OQ-D3   | §10.3   | Is the termination proof complete?                                     | Yes — assumes once-per-build relaxation bound.             | Hard     |
| OQ-D4   | §10.4   | Is `RepairReason` taxonomy complete?                                   | Yes (12 named + `StagePressureGeneric`).                  | Soft     |
| OQ-D4.a | §10.4   | Add explicit `TraceBudgetExceeded` reason?                              | No — covered by `OverlayBudgetExceeded`/`ArenaOverflow`.   | Soft     |
| OQ-D4.b | §10.4   | Is `String` acceptable in `StagePressureGeneric`?                       | Yes — fallback only, named reasons preferred.              | Soft     |

### 21.4 Loop driver behavior (OQ-L*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-L1   | §11.3   | Call ScheduleCostAnalysis every iteration?                              | Only as part of the wrapped pipeline's stage 11 invocation. | Hard    |
| OQ-L2   | §11.3   | Accept proposals with worsening EstimatedCostDelta?                    | Yes — accept and record the worsening.                     | Hard     |
| OQ-L3   | §11.2   | TerminalState::GlobalBudgetExhausted: fail outright?                    | Yes — hard build failure.                                  | Hard     |
| OQ-L4   | §11.4.4 | Add `AuthorizedRelaxationAlreadyUsed` to `DeltaRejection`?              | Yes.                                                       | Soft     |

### 21.5 PolicyProvenance / report shape (OQ-P*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-P1   | §12.4, §13.1.4 | Schema version: minor or major bump?                            | Minor — additive enum variants only.                       | Hard     |
| OQ-P2   | §13.2.2 | Emit `repair_report.json` on every build?                              | Yes — even zero-proposal converged builds.                 | Soft     |

### 21.6 StageCache invalidation (OQ-S*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-S1   | §14.4   | "Earliest in pipeline order" the right re-entry point?                  | Yes.                                                       | Hard     |
| OQ-S2   | §14.3   | Per-sub-knob hash vs whole-CompileKnobs hash?                           | Per sub-knob.                                              | Hard     |

### 21.7 Cross-stage interaction (OQ-X*)

| ID      | Section | Question                                                              | Candidate                                                  | Severity |
|---------|---------|------------------------------------------------------------------------|------------------------------------------------------------|----------|
| OQ-X1   | §16.9   | Should F-B15 (Backend) be wrapped by the loop?                         | No — F-B15 is post-loop; only AuthorizedRelaxation re-enters. | Hard  |

### 21.8 Severity legend

* **Hard**: a divergent answer changes a public type, schema, or the
  loop's correctness. Must be answered before bead close.
* **Soft**: a divergent answer changes naming, a non-load-bearing
  default, or an internal-only choice. May be deferred to a
  follow-up RFC if the chunk-10 oracle pass declines to answer.

### 21.9 Resolution log

(empty until chunk-10 oracle pass returns)

| ID      | Resolution                                  | Date       | Resolved by  |
|---------|---------------------------------------------|------------|--------------|
| (none yet)                                                                   |

---

## 22. Final concise contract

This RFC's load-bearing claims, in seven bullets:

1. **`CompileKnobs` is the only mutable repair surface.** Eight sub-knobs
   (placement, observation, range, storage, sram, rom_window, overlay,
   schedule), each with declared monotone order, bounded, lockable,
   and provenanced.
2. **Passes propose; only the driver applies.** Wrapped stages
   (F-B7..F-B14) emit `RepairProposal`s on local infeasibility; the
   loop driver checks admissibility (six checks) and either applies
   or rejects.
3. **Termination is mechanically guaranteed.** Each accepted delta
   strictly advances the lattice (or fires the once-per-build
   `AuthorizedRelaxation`); the lattice has finite height; the
   `max_refinement_iters` ceiling is the second termination bound.
4. **`ScheduleCostAnalysis` is the single objective oracle.** The
   loop reads `EstimatedCostDelta` from F-B14's report and from
   nowhere else. F-B14 runs as the wrapped pipeline's last stage.
5. **`AuthorizedRelaxation` is the only sanctioned escape.** Once
   per build, gated by `RepairPolicy::allow_placement_profile_fallback`,
   used only for the `PlacementProfile` ladder. Recorded with reason
   in provenance and `repair_report.json`.
6. **`policy_resolution.json` extends additively.** Two new
   `PolicySource` variants (`RepairProposal(RepairProposalId)`) and
   two new `ConstraintOperation` variants
   (`AppliedRepairProposal(RepairProposalId)`,
   `AuthorizedRelaxation(RepairReason)`). Schema version stays at
   `policy_resolution.v1` (additive minor bump).
7. **`repair_report.json` is always emitted.** Records every
   proposal (accepted + rejected) with reasoning. Cross-validated
   against `policy_resolution.json` chains by id-consistency.

The chunk closes when:

* Every `Oracle question` in §21 (Hard severity) has a recorded
  resolution in §21.9.
* Every test in §18 passes.
* The PolicyProvenance amendment in §12 is reflected in F-B2/F-B4's
  semantic validators (the validators accept the new variants under
  F-B16 builds and continue to reject them under non-F-B16 builds).
* `cargo test --workspace --all-features` passes on a clean checkout.
* Two consecutive runs of any fixture produce byte-identical
  `repair_report.json` and `policy_resolution.json` (determinism).

The chunk does **not** close on:

* Soft-severity oracle questions remaining unresolved (they may be
  deferred to follow-up RFCs).
* F-B17 not yet shipping (the loop driver works without per-stage
  cache wiring; it is just not optimal).
* F-B15 Backend not yet shipping (the loop terminates before
  Backend; F-B15's interaction with `AuthorizedRelaxation` is a
  separate F-B15 concern).

---

## 23. References

* `history/planv0.md` lines 1063–1095 (CompileKnobs named-only;
  refinement-loop preamble).
* `history/planv0.md` lines 1096–1560 (compiler-pipeline preamble +
  refinement-loop semantics; `RepairPolicy`, `RepairProposal`,
  `CompileKnobs`, `ConstraintDelta`, `KnobDelta`, `ResourcePressureUpdate`,
  `CompileKnobBounds`, `CompileKnobOverrides`, `KernelSelector`,
  `ValueSelector`, `ReductionSelector`, `TileSelector`, `SliceClass`,
  `KnobLockSet`, `CompileKnobId`, `ConstraintProvenance`,
  `PolicySource`, `EstimatedCostDelta`, `EvidenceClass`).
* `history/planv0.md` lines 1665–1900 (Stages 6, 7, 8, 8.5, 9, 10,
  10.5 — the loop body; `StorageBinding`, `Materialization`,
  `LifetimeClass`, `KernelResidency`, `OverlayPlan`, `ArenaPlan`,
  `SchedSlice`, `ResourceLease`, `SchedulePack`).
* `history/planv0.md` lines 1894–1985 (Stage 11 ScheduleCostAnalysis —
  the loop's objective oracle; `ScheduleCostReport`).
* `history/planv0.md` lines 1985–2080 (BuildReports —
  `repair_report.json`, `policy_resolution.json` extension).
* `history/planv0.md` lines 2792–2870 (Reports and artifacts —
  `repair_report.json` contents).
* `history/glossary.md` (terms added by §3).
* `history/rfcs/F-B2-F-B4-pipeline-entry-validation.md` §-1, §0, §2.7,
  §7.4, §7.5, §10 (PolicyProvenance + ResolvedCompilePolicy +
  CompileKnobs schema; this RFC explicitly amends §7.4 and §7.5
  via §12).
* `history/rfcs/F-B3-F-B5-canonical-irs.md` §0 (placement diagram
  template).
* `history/rfcs/F-B11-F-B12-overlay-arena-plans.md` §0 (placement
  diagram template; OverlayPlan + ArenaPlan as typed products that
  may be invalidated by knob changes).
* `history/rfcs/F-B13-sched-ir-resource-state.md` (GbSchedIR +
  ResourceStateValidation as wrapped stages).
* `bd-3ix` — F-B16 feature bead (BLOCKED-on-oracle; oracle answer
  recorded as 2026-04-26 11:54 UTC comment).
* `bd-3aqf` — T-B16.1 (core CompileKnobs types; recast 2026-05-07).
* `bd-22h4` — T-B16.2 (CompileKnobOverrides + typed selectors).
* `bd-py29` — T-B16.3 (ConstraintDelta + KnobDelta + admissibility).
* `bd-13tf` — T-B16.4 (per-profile defaults).
* `bd-1r6b` — T-B16.5 (rename `allow_profile_fallback` →
  `allow_placement_profile_fallback`).
* `bd-32w5` — T-B16.6 (loop driver).
* `bd-2swd` — T-B16.7 (reports).
* Engineering rule 25 (added by §15.2).
