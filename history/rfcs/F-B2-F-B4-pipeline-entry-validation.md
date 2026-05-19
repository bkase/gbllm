# RFC F-B2 + F-B4: Pipeline Entry & Validation (Stages 0, 0.5, 2)

## -1. Authority and amendment policy

This RFC is the source of truth for F-B2 and F-B4 implementation.
`history/planv0.md` remains the architectural context document, but this RFC is
allowed to refine, narrow, or supersede `planv0.md` wherever the RFC makes a
more precise implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence must
be recorded in an `Amends planv0` note close to the relevant decision. This is
not a request to edit `planv0.md` immediately; it is a local source-of-truth
ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B2/F-B4 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If a later RFC changes any public type, report shape, cache key, or
  diagnostic code introduced here, that later RFC must explicitly amend this
  RFC.
* Source-of-truth changes must be expressed as typed schema changes, not prose
  folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft |
| Feature beads   | bd-2fj **F-B2 ArtifactValidationAndUpgrade + ResolvedCompilePolicy**; bd-2ps **F-B4 StaticBudgetReport** |
| Open tasks      | To be minted: T-B2.1..T-B2.N (validation runtime, policy resolution, calibration binding, `policy_resolution.json` emitter, schema/round-trip tests, StageCache wiring); T-B4.1..T-B4.M (budget projection from QuantGraph, RuntimeChromeBudget comparison, `static_budget.json` emitter, schema/round-trip tests) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` §"The compiler pipeline" stages 0, 0.5, 2; `history/planv0.md` §"Reports and artifacts" `policy_resolution.json`, `repair_report.json`, `budget.json` content; §"Artifact/contract evolution" |
| Glossary        | `history/glossary.md` (artifact stratum, denotational stratum, policy provenance, calibration, hint bundle, deployability envelope, runtime chrome budget, placement profile, residency, common bank) |
| Constitution    | §I correctness by construction; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B3 QuantGraph (Stage 1, separately scoped); F-B16 FeasibilityRefinementLoop (provides the repair side of `policy_resolution.json` / `repair_report.json` once oracle returns); F-A2 gbf-hw (TargetProfile); F-A3 gbf-abi (`BuildIdentityBlock`, `CompatibilityEnvelope`) |

## 0. TL;DR

Chunk 1 is the **policy/feasibility envelope** that brackets the compiler pipeline. It owns three numbered stages:

* **Stage 0 — `ArtifactValidationAndUpgrade`.** Verify the artifact's schema,
  semantic core hash, manifest invariants, required-feature set, lowering
  round-trip under the declared `packer_version`, calibration freshness, and
  `CompileRequest` admissibility against the resolved `TargetProfile`. Stage 0
  may perform **lossless in-memory compatibility upgrades** for registered
  same-major schema adapters only. Cross-major schema changes are migration,
  not compatibility, and are out of scope until `gbf-migrate`. Stage 0 must
  not perform lossy migration, target/profile/calibration/placement injection,
  or on-disk artifact rewriting. Fail fast with a clear diagnostic before any
  compiler stage touches the artifact.

  Amends planv0: `planv0.md` currently says schema mismatch should fail closed
  until `gbf-migrate` exists. This RFC narrows that rule: Stage 0 may accept an
  older schema only when a typed, registered, hash-preserving, lossless
  same-major in-memory compatibility adapter exists and is covered by
  round-trip tests. All other schema mismatches fail closed. Cross-major
  adapters, on-disk migration, lossy migration, target/profile/calibration/
  placement injection, and artifact rewriting remain out of scope.
* **Stage 0.5 — `ResolvedCompilePolicy`.** Resolve `CompileRequest` → `ResolvedCompilePolicy`, including `CompileKnobs` (global + bounds + locks + overrides) with `PolicyProvenance` for every constraint. This is the single answer to "what policy governed this build."
* **Stage 2 — `StaticBudgetReport`.** Pre-lowering sanity check that consumes `QuantGraph` (Stage 1, owned by F-B3), the resolved policy, and `RuntimeChromeBudget`, and reports per-expert payload fits, projected WRAM/SRAM/HRAM peaks, accumulator maxima, projected bank-switch count per token, and likely common-bank footprint. If a budget is busted, the build fails before lowering.

These two features are paired in one RFC because they share the **shift-left-validator** shape: each is a passive pass that consumes pinned inputs, runs typed checks, emits a canonical JSON report, and either continues or fails fast. They share a validation diagnostic taxonomy, calibration/identity hash binding, JSON canonicalization rule, self-hash convention, and `StageCache` key construction. Stage 1 (`QuantGraph`) sits between them in pipeline order but is not a passive validator; it is the canonical artifact-stratum IR and is scoped to F-B3.

The chunk closes only when:

1. The validation runtime rejects every malformed-input class enumerated in §11.
2. `ArtifactValidationAndUpgrade` emits `artifact_validation.json` on both
   success and failure whenever enough input identity is available.
3. `ResolvedCompilePolicy` emits `policy_resolution.json` that round-trips
   through its semantic validator and self-hash.
4. `StaticBudgetReport` emits `static_budget.json` whose static-fit verdict
   matches a synthetic-fixture decision table.
5. All three reports are deterministic across two consecutive regenerations on a clean checkout.
6. `StageCache` keys for Stage 0, Stage 0.5, and Stage 2 are pinned and tested.
7. F-B3 (`QuantGraph`) provides the input shape `StaticBudgetReport` consumes; the QuantGraph **schema** is consumed, but the QuantGraph **construction** is owned by F-B3.

The chunk does **not** include refinement-loop repair logic. `RepairProposal`, `ConstraintDelta`, the `FeasibilityRefinementLoop` driver, and `repair_report.json` emission are scoped to F-B16, which is currently blocked on an oracle question (see `bd-3ix`). F-B2 must, however, leave the `CompileKnobs` schema in `policy_resolution.json` already wired, with provenance values populated by `TargetDefault | ProfileDefault | CompileRequestOverride | HintBundle | Calibration` only — never `RepairProposal(_)` until F-B16 lands.
`ConstraintOperation::AuthorizedRelaxation` is likewise not part of this chunk; F-B16 may introduce it by explicitly amending this RFC.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What M0 ships and what M0.5 (F-B1) retires

Per `planv0.md` and the F-B1 RFC:

* M0 delivers the shared infrastructure: `gbf-asm`, `gbf-hw`, `gbf-abi`, `BankLease`/`BankGuard`, the Bank0 runtime nucleus, `gbf-emu`, `gbf-debug`, `gbf-store`. These are the surfaces this chunk builds on, but the chunk does not require any of them to be model-aware.
* M0.5 (F-B1, "Compute Bringup") retires the **operational** risk for M1: it proves the runtime/banking/harness/emulator stack can host sustained integer compute. F-B1 deliberately does **not** add quantization, oracles, conformance, or a real `CompileRequest`.

This chunk is the **first M1-shape pipeline work**. It is the entry point `gbf-codegen` will use for every real compile from M1 forward.

### 1.2 What M1 commits to

Per `planv0.md`:

> M1: `DenotationalOracle` + `ArtifactOracle` plus a single quantized dense kernel; conformance checking between reference observations and the frozen artifact (first `conformance.json`); first `CompileRequest` wiring.

Five distinct architectural commitments:

1. The denotational stratum (`ReferenceModelBundle`, `ReferenceProgram`, `DenotationalOracle`).
2. The artifact stratum (`ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`, `ArtifactOracle`).
3. The first quantised dense kernel (real `QuantSpec`, real `TernaryWeightPlan` or honest dense-int kernel).
4. The first `conformance.json` against `ConformanceEnvelope`.
5. The first real `CompileRequest` wiring — i.e. policy/feasibility/transform/reporting bracketing, `ResolvedCompilePolicy`, `PolicyProvenance`, calibration set refs.

Commitment (5) is what this chunk delivers head-on. Without F-B2 there is no honest entry point; without F-B4 the first model build can run all the way to `AsmIR` before discovering that an expert does not fit a bank.

### 1.3 What this chunk retires for the rest of Epic B

By the time Epic B's later chunks begin:

* Every later stage receives a typed, validated `ResolvedCompilePolicy` rather than a half-resolved `CompileRequest`.
* Every later stage can assume the `ArtifactCore` schema, semantic core hash, lowering round-trip, and calibration are already verified. They never re-validate.
* Every later stage can assume the `RuntimeChromeBudget` is honored at the static level. If a quantised dense kernel cannot statically fit a bank, the pipeline never reaches `RomWindowPlan`.
* Every later stage can assume `policy_resolution.json` and `static_budget.json` already pin every load-bearing scalar (placement profile, knob bounds, projected sizes) so its own report is local-only.
* The `StageCache` key shape for these two stages is pinned: every later stage that uses `gbf-store` plumbs the same canonical-input convention.

This chunk's job is to retire the **schema, identity, and fit** preconditions of the rest of the pipeline. It is the second shift-left filter in the system — the first is `gbf-train preflight` against `DeployabilityEnvelope` and `RuntimeChromeBudget`, which runs even before export. F-B2 is the second; F-B4 is the third (after `QuantGraph` exists but before lowering commits).

### 1.4 Why this is two paired Features, not one feature or three

The natural unit is "passive shift-left validators that bracket the transform pipeline."

* If we made it one feature, the bead would carry both a schema/identity validator and a multi-input budget projector. The implementation surface is large enough that PR review fragments. It would also force the `QuantGraph` dependency (F-B3) into F-B2's path even though Stage 0/0.5 do not touch `QuantGraph`.
* If we made it three features, we would split on stage number. Stage 0 and Stage 0.5 share so much code and report shape that a split is artificial. Stage 2 is meaningfully separate (different inputs, different report, depends on F-B3).
* Two features matches the natural seam: F-B2 owns "the validation envelope at the head" (Stages 0 + 0.5), F-B4 owns "the static budget filter that runs before lowering" (Stage 2). They are paired in this RFC because they share a chunk-level surface (diagnostics, reports, canonical JSON, self-hash, StageCache) but ship as separate beads to keep PR scope tight.

### 1.5 What this chunk is NOT

The chunk is small in *scope* but big in *contract surface*. To prevent scope
creep, here is what this chunk explicitly is not:

* It is **not** a transform stage. Stages 0/0.5/2 are passive: validate, project, report. None of them rewrites `ArtifactCore`, mutates `CompileRequest`, or constructs `QuantGraph`/`GbInferIR`/etc.
* It is **not** the producer of `RuntimeChromeBudget`. That artifact is emitted by the runtime-shell build (per `planv0.md` §"Deployability envelope"). F-B4 consumes it by hash.
* It is **not** the producer of `BootstrapCalibrationBundle`. The bundle is checked into `fixtures/calibration/` next to the target profile; it is content-addressed and version-pinned. This chunk consumes it.
* It is **not** the implementer of `QuantGraph`. F-B4 consumes a `QuantGraphBudgetSource` trait. F-B3 lands the real implementation later.
* It is **not** a refinement loop. F-B16 owns `RepairProposal`, `ConstraintDelta`, the loop driver, and `repair_report.json`. This chunk emits the `compile_knobs` schema that F-B16 will plug into, but no proposal is ever applied here.
* It is **not** the cycle-cost producer. `schedule_cost.json` and observed-vs-projected cycle fields belong to F-B14 and the runtime measurement pipeline. F-B4's report contains static integer counts only.
* It is **not** the runtime drift monitor, the fault-policy recovery exerciser, or the safe-mode trigger evaluator. Those live downstream.
* It is **not** an artifact migration tool. `gbf-migrate` is deferred to F-A6b. Stage 0 admits only registered, hash-preserving, lossless in-memory adapters; everything else fails closed.
* It is **not** a surface for adding profile-time relaxations. There is exactly zero "soft under Bringup" code path in F-B2/F-B4 (§2.13).
* It does **not** assume any concrete model topology. Tests use synthetic fixtures; the chunk closes before the M1 quantised dense kernel exists.

### 1.6 Relationship to `gbf-train preflight`

`gbf-train preflight` (per `planv0.md` line 921) is the *first* shift-left
filter, run before any export. It checks proposed model/config/checkpoint
against `DeployabilityEnvelope` and `RuntimeChromeBudget`. This chunk is the
*second and third* shift-left filters, run by `gbf-codegen` after export and
before lowering.

The boundary:

* `gbf-train preflight` operates on **proposed** model configs that have not
  yet produced an artifact. It uses estimators against `DeployabilityEnvelope`.
  It can reject "this model will never fit a target family."
* F-B2 operates on a **frozen** `ArtifactCore`. It can reject "this artifact
  is malformed, mismatched, or compiled against a stale target."
* F-B4 operates on a **resolved policy + frozen QuantGraph**. It can reject
  "this build's experts do not fit the requested chrome budget."

A model that passes `gbf-train preflight` may still fail F-B2 (e.g. calibration
drift between training and compile time) or F-B4 (e.g. expert byte math
exceeded the slot once exported). A model that passes F-B2/F-B4 has *not*
proved deployability — `RomWindowPlan` (F-B10), `ArenaPlan` (F-B12),
`ResourceStateValidation` (F-B13), and final layout remain authoritative.

This chunk is the single source of truth for "did the inputs to compilation
hold up the contract before any work began."

## 2. Load-bearing decisions

### 2.1 Passive pass shape

Stages 0, 0.5, and 2 are **passive**: they consume pinned inputs, run typed checks, and emit a canonical JSON report. They do not transform the IR. Stage 0.5 produces `ResolvedCompilePolicy`, which is a transform from `CompileRequest`; that is a resolution step, not an IR rewrite.

The chunk-level pass shape is:

```text
PassInputs (pinned, hash-bound)
  -> PassRuntime
       (typed checks)
       (typed projections)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed product, ReportEnvelope<ReportV1> }
       PassDiagnostics := list of typed ValidationDiagnostic
```

Every validator report includes an `outcome`:

```rust
pub enum ReportOutcome {
    Passed,
    Failed,
}
```

Report emission policy:

* Stage 0 emits `artifact_validation.json` on pass or fail whenever identity
  can be computed.
* Stage 0.5 emits `policy_resolution.json` on successful policy resolution and
  on policy-resolution failure whenever Stage 0 produced a validation product.
  A failed policy report has `outcome = Failed`, `result = None`, no
  `ResolvedCompilePolicy` product, and at least one `Hard` diagnostic.
* Stage 0.5 must not mutate `artifact_validation.json`; that report's
  `report_self_hash` is immutable once Stage 0 emits it.
* Stage 2 emits `static_budget.json` on pass or budget failure whenever
  `policy_resolution_self_hash` is available. If `RuntimeChromeBudget` is
  missing, Stage 2 emits the missing-budget failure report without calling
  `QuantGraphBudgetSource::to_budget_view()`. All other Stage 2 reports also
  require `quant_graph_hash`.
* successful reports have no `Hard` diagnostics;
* failed reports have at least one `Hard` diagnostic.

Every chunk member shares this shape. Every chunk member's `PassInputs` is recorded by hash inside the emitted report so two builds with the same inputs produce byte-identical reports.

### 2.2 Fail-fast taxonomy

Diagnostics are typed. There is no `String`-only error path.

```rust
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}

pub enum DiagnosticSeverity {
    Hard,    // build cannot proceed
    Soft,    // recorded in report; build proceeds
}

pub enum ValidationOrigin {
    Schema,
    SemanticCore,
    Manifest,
    Lowering,
    Calibration,
    HintBundle,
    Workload,
    GoldenVector,
    CompileRequest,
    PolicyResolution,
    Budget,
}
```

Stage 0, Stage 0.5, and Stage 2 emit only `Hard` diagnostics in this chunk —
every expected reject case is a hard stop. `DiagnosticSeverity::Soft` remains
in the taxonomy for downstream stages, but F-B2/F-B4 report semantic validators
reject any `Soft` diagnostic. See §2.13.

### 2.3 Identity is per-input, not per-build

All three reports record canonical hashes of every load-bearing input available
to that stage (`ArtifactCore`, `ArtifactManifest`, lowering manifest,
`CompileRequest`, `TargetProfile`, compile profile, calibration bundle, hint
bundle, `QuantGraph`, runtime chrome budget). The build's own identity
(`BuildIdentityBlock`) is computed downstream, not here.

`HintBundle` absence is normalized to a canonical empty hint bundle with a
stable hash. Reports never encode "no hints" as `null`.

### 2.4 Report self-hash convention

All three reports — `artifact_validation.json`, `policy_resolution.json`, and
`static_budget.json` — follow the F-B1 self-hash convention: compute
`report_self_hash` over canonical JSON with the field temporarily set to:

```text
sha256:0000000000000000000000000000000000000000000000000000000000000000
```

This rule is shared, not per-stage. `gbf-report` exposes one `compute_self_hash` helper that every chunk member uses.

All report-visible hashes serialize as lowercase strings of the form:

```text
sha256:<64 lowercase hex digits>
```

The `sha256:` prefix is part of the JSON schema. It is not part of the raw
digest bytes fed into hash computations.

Canonical object hashes use a domain separator:

```text
gbf:<crate>:<type>:<schema-id>:<schema-version>\0<canonical-json-bytes>
```

Examples:

```text
gbf:gbf-policy:CompileRequest:compile_request:1.0.0\0...
gbf:gbf-report:PolicyResolutionReport:policy_resolution.v1:1.0.0\0...
```

This prevents two different schema families with identical JSON bytes from
colliding at the application layer.

### 2.5 Canonical JSON rule

Lifted unchanged from F-B1 §10.2:

* UTF-8 JSON object keys are emitted in lexicographic order at every object level.
* No insignificant whitespace.
* Integer fields are base-10 JSON numbers.
* Floating-point fields are forbidden in F-B2/F-B4 v1 reports.
  Quantities that would normally be fractional use fixed-point integer fields
  with the scale in the field name, e.g. `_q8_8` or `_q16_16`.
* Arrays whose order is semantically meaningful are explicitly specified in the schema.
* Fields with unknown or unmeasured values are rejected for checked-in reports rather than encoded as `0`, `null`, or omitted.

The rule lives in `gbf-report::canonical_json`, not duplicated.

Nullability rule:

* `null` is legal only for explicitly optional semantic absence.
* `null` is never legal for "unknown", "unmeasured", "not yet computed", or
  "omitted by accident".
* Every nullable field in these v1 reports must be listed here or in the
  corresponding schema section.

Allowed nullable fields in `artifact_validation.v1`:

```text
identity.artifact_source_hash
identity.artifact_effective_core_hash
identity.artifact_manifest_hash
identity.semantic_core_hash
identity.artifact_aux_hash
identity.lowering_manifest_hash
identity.calibration_hash
identity.compatibility_adapter_hash
compatibility.decision
```

Allowed nullable fields in `policy_resolution.v1`:

```text
result
```

Allowed nullable fields in `static_budget.v1`:

```text
identity.runtime_chrome_budget_hash
runtime_chrome_budget
projections.common_bank_footprint.shared_dense_ffn_bytes
projections.projected_*_switches_per_token.expected_q16_16
```

### 2.6 StageCache stores success outputs and may memoize failure diagnostics

If a stage fails, the build halts and `StageCache` does not record a successful
entry for that input set. However, because all cache keys are content-addressed,
the cache may memoize failure diagnostics and failure reports for byte-identical
inputs.

Failure memoization rules:

* a failure memo is never usable as a successful stage product;
* a failure memo may only be replayed when every key input hash matches exactly;
* replayed failure reports preserve the original `report_self_hash`;
* a hand edit changes the input hash and therefore misses the failure memo;
* CI may disable failure memoization to exercise validators on every run.

This improves malformed-fixture test speed without hiding edits.

### 2.7 `CompileKnobs` schema in `policy_resolution.json` is wired but unrepaired

Per the bd-2fj amendment, `policy_resolution.json` carries `compile_knobs.{global,bounds,locks,overrides,provenance}`. F-B2 must populate this section in full.

Allowed `PolicySource` values during F-B2:

```text
TargetDefault
ProfileDefault
CompileRequestOverride
HintBundle
Calibration
```

`PolicySource::RepairProposal(_)` is **forbidden** in F-B2. F-B16 introduces it.
Any code path that could populate `RepairProposal(_)` must be unreachable in
this chunk; tests must assert this.

`ConstraintOperation::AuthorizedRelaxation` is also forbidden in this chunk.
There are no authorized-relaxation fields, no bringup-relaxation records, and
no report-visible relaxation operations in F-B2/F-B4.

### 2.8 Calibration binding is uniformly fail-closed

Every passing build references a `CalibrationBundleSet` by hash. There is no
profile-time relaxation of this rule and no passing "calibration absent" state.
If the compile request does not resolve to a calibration set, Stage 0 emits
`CalibrationMissing` and fails.

Stage 0 enforces two gates against the referenced `CalibrationBundleSet`:

1. **Resolution** — every referenced layer resolves to a present bundle.
   Unresolved or missing layers are `CalibrationMissing` (Hard).
2. **Freshness and confidence** — each bundle's `target_profile_hash`,
   `kernel_set_hash`, `packer_version`, `calibration_schema_hash`, and declared
   validity envelope match the active build inputs available to Stage 0. A
   bundle whose declared `CalibrationConfidenceClass` is below
   `RiskPolicy::calibration_confidence_requirement` is
   `CalibrationConfidenceTooLow`
   (Hard). A bundle whose hashes do not match is `CalibrationStale` (Hard).

Bringup builds before any measurements exist reference an explicit
`BootstrapCalibrationBundle` (content-addressed, declares
`CalibrationConfidenceClass::None`, no measurements). It passes the resolution
gate. Profiles whose
`RiskPolicy::calibration_confidence_requirement == NoMinimumConfidence`
(e.g. `Bringup`) accept it; `Default`, `Trace`, and `Recovery` reject it via
the existing `CalibrationConfidenceTooLow` diagnostic. This is a profile knob,
not a profile-time relaxation. See §2.13.

`CalibrationConfidenceClass::None` is the bundle's declared confidence class.
`NoMinimumConfidence` is the profile's minimum-confidence requirement. These
are intentionally distinct so "no measurements in the bundle" is not confused
with "no calibration requirement in the profile".

Runtime-nucleus mismatch is checked only by a stage that has a runtime identity
input and calibration contents. No such runtime/calibration comparison is made
in F-B2/F-B4. In this chunk, missing `RuntimeChromeBudget` remains the Stage 2
`BudgetMissingRuntimeChromeBudget` failure rather than being silently folded into
Stage 0 calibration handling.

### 2.9 `RuntimeChromeBudget` is an input, not an output

`RuntimeChromeBudget` is emitted by the current UI/runtime shell build. It is
**not** computed by F-B4 and is **not** embedded in `CompileRequest`.
F-B4 consumes it as a compile invocation input alongside the artifact,
compile request, target profile, and stage products.

```rust
pub struct CompileInvocationInputs {
    pub artifact_ref: ArtifactRef,
    pub compile_request: CompileRequest,
    pub target_profile: TargetProfile,
    pub runtime_chrome_budget: Option<RuntimeChromeBudget>,
}
```

If no `RuntimeChromeBudget` is available — e.g. M1 head-of-line where the
runtime shell hasn't been baselined yet — F-B4 must fail closed with
`BudgetMissingRuntimeChromeBudget` rather than silently substituting a dummy
budget. The missing-budget report also records
`BudgetFailure::MissingRuntimeChromeBudget` so that
`decision.fits == decision.failures.is_empty()` remains an invariant.
Closure-eligible artifacts must include a real `RuntimeChromeBudget`.

Amends planv0: this RFC keeps `CompileRequest` narrower than some plan prose
implies. `CompileRequest` says what build is requested; invocation inputs
supply the current runtime-shell budget artifact.

### 2.10 Stage 2 is per-bank, not just per-expert

The seed bead description says "does each expert fit under the requested placement profile?" but reading `planv0.md` carefully, Stage 2 must also project **common-bank footprint**, **per-bank occupancy**, and **bank-switch counts per token** because those are the M1-relevant quantities the lowering pipeline cannot recover later cheaply. Stage 2's projection covers:

* per-expert payload bytes;
* per-bank candidate occupancy under each candidate `PlacementProfile`;
* `common_bank_footprint` (aggregate cost of shared kernels, LUTs, optional shared dense FFN per `bd-33q`);
* projected accumulator maxima per reduction site;
* projected WRAM / SRAM / HRAM peaks;
* projected bank-switch count per token under the resolved routing model;
* projected SRAM-page-switch count per token (when sequence state is paged).

These projections are *static* and live below measurement-defined cycle costs. They do not require an emulator run.

### 2.11 No model topology is required to test the pass

Both passes are deterministic functions of typed inputs. Their tests use synthetic `ArtifactCore`, `ArtifactManifest`, `CompileRequest`, `TargetProfile`, and (for F-B4) synthetic `QuantGraph` fixtures. The chunk does not require a real M1 model, a real emulator run, or any neural-network semantics. This is the chunk's main schedule advantage over F-B5..F-B15: every test is in-process unit-testable.

### 2.12 Where the code lives

Per `planv0.md` §"What each crate is responsible for":

| Concern                                                   | Crate                                              |
| --------------------------------------------------------- | -------------------------------------------------- |
| `CompileRequest`, `ResolvedCompilePolicy`, `RepairPolicy`, `DeployabilityEnvelope`, `CompileKnobs` types | `gbf-policy`                                       |
| `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`, `TargetDataLoweringArtifact`, `HintBundle` types | `gbf-artifact`                                     |
| `RuntimeChromeBudget`, `RomBudgetSlot` types              | `gbf-policy` (per `planv0.md` line 142, 144)       |
| Stage 0 implementation (`ArtifactValidationAndUpgrade`)   | `gbf-codegen::stages::validate`                    |
| Stage 0.5 implementation (`resolve_policy`)               | `gbf-codegen::stages::policy`                      |
| Stage 2 implementation (`StaticBudgetReport`)             | `gbf-codegen::stages::budget`                      |
| `policy_resolution.json` and `static_budget.json` schema  | `gbf-report`                                       |
| Shared `ValidationDiagnostic` taxonomy                    | `gbf-policy::diagnostics`                          |
| Shared canonical JSON / self-hash helpers                 | `gbf-report::canonical_json`                       |
| StageCache integration                                    | `gbf-store` consumed by `gbf-codegen::stage_cache` |

No new crate is created by this chunk.

Implementation note: `gbf-foundation::Hash256` currently exists as the shared
digest wrapper. F-B2 requires its report-facing serde representation to be the
`sha256:<hex>` string above, not a JSON byte array. Raw bytes remain available
through `Hash256::as_bytes()`.

Current-code note:

* `gbf-policy/src/{budget,envelope,objective,repair,risk}.rs` are currently
  module stubs.
* `gbf-policy/src/compile.rs` contains only minimal placeholder compile-policy
  types.
* `gbf-artifact/src/{manifest,aux,lowerings}.rs` are currently module stubs.
* `gbf-artifact/src/{export_facts,preferences,weight_plan,core,...}.rs` are
  real, but `HintBundle` is not assembled and `BuildConstraints` does not exist.
* `gbf-workload/src/manifest.rs` is currently a module stub.
* `gbf-policy::calibration` does not yet exist as a module;
  `gbf-hw::calibration::CalibrationConfidenceClass` is real and re-used
  unchanged.
* `gbf-foundation` already provides typed ids, `Hash256`, `BlobRef`, `SemVer`,
  and `ByteCost`.

F-B2/F-B4 must therefore introduce, at minimum:

* `gbf-policy::diagnostics`;
* `gbf-policy::budget::{RuntimeChromeBudget, RomBudgetSlot, BudgetSlotClass}`;
* `gbf-policy::compile::{CompileRequest, ResolvedCompilePolicy,
  CompileProfileSpec, CompileKnobs, CompileKnobPath, PolicyProvenance}`;
* `gbf-policy::objective::{CompileObjective, RiskPolicy}`;
* `gbf-policy::repair::RepairPolicy`;
* `gbf-policy::calibration::{CalibrationLayer, CalibrationBundle,
  CalibrationBundleSet, CalibrationSetRef, BootstrapCalibrationBundle,
  ValidityEnvelope}` (re-using `gbf-hw::calibration::CalibrationConfidenceClass`);
* `gbf-artifact::manifest::{ArtifactManifest, ManifestComponent,
  ManifestInvariant, ArtifactSchemaVersion, ArtifactFeature, LineageId}`;
* `gbf-artifact::aux::{ArtifactAux, SidecarKind}` plus minimal placeholder
  `*Ref` structs for sidecar kinds whose full bodies live in Epic G or F-C;
* `gbf-artifact::lowerings::{TargetDataLoweringArtifact, LoweringShard,
  LoweringShardRef, LoweringManifest, PackerVersion, DataLoweringProfileId,
  LoweringShardKind}`;
* `gbf-artifact::HintBundle` (assembled from existing `export_facts` +
  `preferences`) plus the missing third leg `BuildConstraints` and
  `EvidenceScope`;
* `gbf-workload::manifest::{WorkloadManifest, WorkloadManifestRef,
  WorkloadId, WorkloadLocator}`;
* `gbf-report` report envelope, canonical JSON, and v1 report schemas;
* `gbf-codegen::stages::{validate, policy, budget}` stage implementations.

The `gbf-policy` items, the `gbf-artifact` items, the `gbf-workload` item,
and the `gbf-policy::calibration` item are absorbed by F-B2 under the
boundary defined in §2.14. Every absorbed module is **schema only** in
this chunk; validator dispatch and transform behavior remain owned by
the named stage tasks.

### 2.13 Bringup is a profile, not a relaxation surface

Every Stage 0/0.5/2 gate is a hard typed input. There is no profile-conditional
softness, no in-flight `RuntimeChromeBudget` mutation, and no soft diagnostic
in this chunk.

Bringup builds before any calibration measurements exist reference an explicit
`BootstrapCalibrationBundle` artifact: content-addressed, version-pinned,
declares `CalibrationConfidenceClass::None`, and ships next to its associated
target profile in `fixtures/calibration/`. The same `CalibrationConfidenceTooLow`
gate that protects `Default`/`Trace`/`Recovery` from accidentally accepting
weak calibration also protects Bringup from accidentally accepting *any*
calibration: the bundle is present, but its declared confidence is `None`, and
only profiles with `RiskPolicy::calibration_confidence_requirement == NoMinimumConfidence` accept it.

Reduced reserved-slack for Bringup is similarly an explicit input. Each
target profile ships a `bringup-*.chrome_budget.json` variant whose
`reserved_slack` already reflects the bringup-tolerated value. Bringup builds
pass *that* `RuntimeChromeBudget` as their invocation input. F-B4 never
mutates the source `RuntimeChromeBudget`. The effective cap is therefore:

```text
effective_cap_bytes: i64 = usable_bytes - reserved_slack
```

All operands are widened to `i64`. If `reserved_slack > usable_bytes`, the
effective cap is negative and the relevant placement/budget check fails;
implementations must not saturate to zero or wrap unsigned arithmetic.

Amends planv0: this RFC keeps `Bringup` as a profile selection (different
`ObservabilityMode`, `TraceBudget`, `RepairPolicy` defaults, and knob defaults)
without permitting profile-time relaxation of calibration freshness or
`RuntimeChromeBudget` slack. The four-canonical-profiles list remains; only
the relaxation surface is removed.

Consequences for this chunk:

* `BringupRelaxationRecord`, `CompileProfileRelaxation`,
  `ConstraintFrame.authorized_relaxations`, and `bringup_*` report fields all
  do not exist.
* Stage 0/0.5/2 emit only `Hard` diagnostics. `DiagnosticSeverity::Soft`
  remains in the taxonomy for downstream stages but is rejected by F-B2/F-B4
  report semantic validators.
* Provenance is uniform: every value comes from a hash-named input.

### 2.14 Schema absorption from upstream stubs

Stage 0's validation classes (§7.3) reference types in `gbf-artifact`,
`gbf-workload`, and `gbf-policy::calibration` whose modules are still
`//! Module stub.` on disk. No active F-A, F-G, or F-E task owns the schema
half of those types — Epic A (F-A2..F-A8) is closed and never touched
`gbf-artifact::{manifest, aux, lowerings}` or `gbf-workload::manifest`;
Epic G is downstream (M2/M3); F-E1 is a P2 *consumer* that waits for these
contracts to exist; `bd-209g` is the closest existing bead and is also a
downstream consumer, not a creator.

This RFC therefore absorbs the **schema halves** of those stubs into the
F-B2 chunk by the same logic §2.12 absorbs `gbf-policy` schema:

* the chunk is the only M1 consumer of the schema;
* the schema is small (one to a few types per module);
* logic remains owned by the consuming stage tasks (T-B2.5 owns manifest
  invariants; T-B2.6 owns aux sidecar validation; T-B2.7 owns lowering
  round-trip; T-B2.8 owns calibration freshness; T-B2.9 owns hint
  provenance and workload-ref resolution; T-B2.17 owns hint consumption);
* future epics (Epic G, F-C, F-E1) extend these schemas via amendment, not
  retroactive replacement.

The absorption boundary is **schema only**: type definitions, serde
derives, `deny_unknown_fields`, explicitly-tagged enums, round-trip tests,
and minimal builder helpers in `gbf-test`. Any validator dispatch,
canonical-hash computation, or transform behavior remains owned by the
named stage task.

The absorbed surface is partitioned across seven Wave-0 tasks (T-B2.0
through T-B2.0f, see §8.5 and §12). T-B2.0f depends on T-B2.0c (it uses
`PackerVersion`); T-B2.0d depends on T-B2.0 (it uses `CompileKnobId` and
`ConstraintValue`). The remaining five are independent and parallel-able.

| Wave-0 task | Crate / module                 | Types introduced                                                                       |
|-------------|--------------------------------|----------------------------------------------------------------------------------------|
| T-B2.0      | `gbf-policy::compile`/`objective`/`repair`/`budget` | `CompileRequest`, `ResolvedCompilePolicy`, `CompileKnobs*`, `RuntimeChromeBudget`, ... |
| T-B2.0a     | `gbf-artifact::manifest`       | `ArtifactManifest`, `ManifestInvariant`, `ArtifactSchemaVersion`, `ArtifactFeature`, `LineageId` |
| T-B2.0b     | `gbf-artifact::aux`            | `ArtifactAux`, `SidecarKind`, sidecar `*Ref` placeholders                              |
| T-B2.0c     | `gbf-artifact::lowerings`      | `TargetDataLoweringArtifact`, `LoweringShard`, `LoweringShardRef`, `PackerVersion`, ... |
| T-B2.0d     | `gbf-artifact::HintBundle`     | `HintBundle` assembly, `BuildConstraints`, `EvidenceScope`                             |
| T-B2.0e     | `gbf-workload::manifest`       | `WorkloadManifest`, `WorkloadManifestRef`, `WorkloadId`, `WorkloadLocator`             |
| T-B2.0f     | `gbf-policy::calibration`      | `CalibrationLayer`, `CalibrationBundle`, `CalibrationBundleSet`, `BootstrapCalibrationBundle`, ... |

Wave 0 is the gate that lets every later Wave's "Inputs" section
type-check. Until Wave 0 closes, no Wave-1+ task can build.

## 3. Goals

This chunk implements:

1. `ValidationDiagnostic` taxonomy in `gbf-policy::diagnostics`, including `DiagnosticSeverity`, `ValidationOrigin`, `ValidationCode`, `ValidationDetail`, and structured provenance.
2. `ReportEnvelope<R>` wrapper in `gbf-report`, including `ReportEnvelope::self_hash`, `ReportEnvelope::canonicalize`, and a self-hash convention test.
3. Canonical JSON emitter / parser in `gbf-report::canonical_json` (key sorting, integer-only base-10 for these schemas, and rejection of unknown values).
4. Stage 0 (`ArtifactValidationAndUpgrade`) implementation in `gbf-codegen::stages::validate`, covering the ten validation classes in §7.3.
5. Stage 0.5 (`resolve_policy`) implementation in `gbf-codegen::stages::policy`, including `CompileKnobs` resolution from defaults / overrides / hints / calibration with full provenance.
6. `policy_resolution.json` schema in `gbf-report`, including the `compile_knobs` section per the bd-2fj amendment, plus a semantic validator.
7. Stage 2 (`StaticBudgetReport`) implementation in `gbf-codegen::stages::budget`, including per-expert payload, per-bank occupancy, common-bank footprint, accumulator maxima, projected WRAM/SRAM/HRAM peaks, projected bank-switch and SRAM-page-switch counts per token.
8. `static_budget.json` schema in `gbf-report`, including a semantic validator and the `fits` decision table.
9. StageCache key construction for Stage 0, Stage 0.5, and Stage 2 in `gbf-codegen::stage_cache`, with regression tests for key stability.
10. Synthetic fixtures and a unit-test suite that proves every reject class in §11.
11. Checked-in golden artifacts
    (`artifact_validation.golden.json`, `policy_resolution.golden.json`,
    `static_budget.golden.json`) under `docs/review/f-b2-f-b4/artifacts/`
    with a deterministic regenerator.
12. A reviewer review packet under `docs/review/f-b2-f-b4/`.
13. Wave-0 schema absorption tasks T-B2.0..T-B2.0f introduce foundational
    type definitions in `gbf-policy`, `gbf-artifact`, and `gbf-workload`
    per §2.12 and §2.14. Schema-only deliverables; validator dispatch
    remains owned by the named stage tasks.

## 4. Non-goals

This chunk does **not** implement:

* `RepairProposal`, `ConstraintDelta`, `KnobDelta`, `RepairPolicy` accept/reject logic.
* The `FeasibilityRefinementLoop` driver itself.
* `repair_report.json` emission.
* `QuantGraph` construction (consumed only via stub trait until F-B3 lands).
* `ObservationPlan`, `RangePlan`, `StoragePlan`, `RomWindowPlan`, `OverlayPlan`, `ArenaPlan`, `GbSchedIR`, `ScheduleCostAnalysis`, backend.
* Any `cargo bench` measurement.
* Runtime drift monitor.
* `Safe` runtime mode.
* Fault-policy recovery.
* Any emulator integration (these stages do not need the emulator).
* `gbf-train preflight` (lives in `gbf-train`, not here).
* `RuntimeChromeBudget` *production* (consumed only).
* Real `ArtifactOracle` or `DenotationalOracle` consultations.
* `conformance.json`.
* Lossy or on-disk migration from older artifact schemas. `gbf-migrate` is
  deferred per F-A6 §0.0.0. A schema mismatch fails closed unless §7.0 accepts
  it through a registered lossless in-memory compatibility adapter.

## 5. Anti-goals

Reviewers must refuse these changes:

| Anti-pattern                                                                | Reason                                                                                                  | Correct answer                                                                                |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Add `RepairProposal(_)` provenance values in F-B2                           | Pulls F-B16's loop logic into a passive validator                                                       | Allowed `PolicySource` values in F-B2 are `TargetDefault`/`ProfileDefault`/`CompileRequestOverride`/`HintBundle`/`Calibration` only |
| Stub a `RuntimeChromeBudget` when none is available                         | Hides "the runtime shell hasn't been baselined" as silent success                                       | Hard reject `BudgetMissingRuntimeChromeBudget`                                                |
| Compute `BuildIdentityBlock` here                                           | Build identity belongs to the backend (Stage 12); validation must not pre-commit it                     | Record only input hashes (`compile_request_hash`, `artifact_core_hash`, etc.)                 |
| Mutate `CompileRequest` during resolution                                   | `ResolvedCompilePolicy` is the resolved object; the request is immutable input                          | Resolve into a fresh `ResolvedCompilePolicy`; record provenance of every value                |
| Run any emulator                                                            | These stages are pure functions of typed inputs                                                         | Unit-test against synthetic fixtures                                                          |
| Defer schema validation to a downstream stage                               | "Shift-left filter" is the entire point                                                                 | Stage 0 fails fast at the head of the pipeline                                                |
| Add floating-point fields to F-B2/F-B4 v1 reports                           | Reproducibility leak and contradicts the fixed-point report contract                                    | Use explicitly scaled integer fields such as `_q8_8` or `_q16_16`                             |
| Allow any profile to skip the calibration bundle                            | Defeats calibration-confidence gate; produces "soft under Bringup" folklore                             | Every build references a `CalibrationBundleSet` by hash; Bringup uses an explicit `BootstrapCalibrationBundle` with `CalibrationConfidenceClass::None` |
| Add a profile-time `bringup_relaxation` field to any pass output            | Profile-conditional softness erodes uniform gate semantics                                              | Ship a `bringup-*.chrome_budget.json` variant and pass it as the invocation input             |
| Make `policy_resolution.json` describe runtime knobs                        | Conflates static policy with runtime configuration                                                      | Runtime knobs live in `runtime_knobs` of `realism_report.v1.json` (F-B1) or in F-A3 surfaces  |
| Compute kernel cycles / bandwidth in `static_budget.json`                   | Cycle costs are measurement-defined (see F-B1 §15)                                                      | F-B4 records *projected static counts*, not measured cycles. Cycles live in `schedule_cost.json` (F-B14) |
| Hand-write the `compile_knobs` section without provenance                   | Loses auditability                                                                                      | Every `CompileKnobValues` field carries `ConstraintProvenance`                                |
| Add a new crate for validation                                              | `gbf-codegen::stages::validate` is named in `planv0.md` line 206                                        | Implement under `gbf-codegen`                                                                 |
| Use `String` errors                                                         | Type-erased diagnostics break triage                                                                    | All diagnostics typed via `ValidationDiagnostic`                                              |
| Make Stage 0 idempotent on cache hit but not on cache miss                  | Caching successful passes is fine; caching failures hides hand-edits                                    | Successful passes only enter the cache                                                        |
| Treat a schema mismatch as accepted without a registered lossless adapter   | `gbf-migrate` is deferred to F-A6b; implicit compatibility is migration by folklore                     | Stage 0 either records a tested `LosslessInMemoryUpgrade` decision or errors loudly             |
| Consume `QuantGraph` by import path before F-B3 lands                       | Cross-feature build-order breakage                                                                      | F-B4 consumes a `QuantGraphBudgetSource` trait that produces a validated `QuantGraphBudgetView`; F-B3 implements the trait later |
| Treat the `RuntimeChromeBudget` excerpt in `static_budget.json` as source of truth | Drift between budget owner and budget consumer                                                          | The report may embed a canonical review excerpt, but it must hash-match the source artifact; the source-of-truth budget lives in its own file |
| Emit `ConstraintOperation::AuthorizedRelaxation` in this chunk              | Reintroduces profile-time latitude under a different name                                                | No authorized-relaxation operation exists until a later RFC explicitly amends this RFC         |
| Let a malformed `QuantGraphBudgetView` fall through as a generic budget failure | Hides a schema/view seam error between F-B3 and F-B4                                                     | Hard reject with `BudgetQuantGraphViewMalformed`                                               |

## 6. Pipeline

The Epic B headline pipeline (per `planv0.md` line 1068):

```text
Policy / feasibility envelope:
  0.   ArtifactValidationAndUpgrade            <-- F-B2
  0.5  ResolvedCompilePolicy                   <-- F-B2

Transformative stages (wrapped by FeasibilityRefinementLoop, F-B16):
  1.   QuantGraph                              <-- F-B3 (separate chunk)
  2.   StaticBudgetReport                      <-- F-B4
  3.   GbInferIR (value/effect IR)             <-- F-B5
  4.   ObservationPlan                         <-- F-B6
  5.   RangePlan                               <-- F-B7
  6.   StoragePlan                             <-- F-B8
  7.   SramPagePlan                            <-- F-B9
  8.   RomWindowPlan                           <-- F-B10
  8.5  OverlayPlan                             <-- F-B11
  9.   ArenaPlan                               <-- F-B12
  10.  GbSchedIR                               <-- F-B13
  10.5 ResourceStateValidation                 <-- F-B13
  11.  ScheduleCostAnalysis                    <-- F-B14
  12.  Backend (AsmIR -> ... -> EncodedRom)    <-- F-B15

Reporting envelope:
  13.  BuildReports
```

This RFC's surface inside that pipeline:

```text
gbf-codegen::import
   |
   v
+--------------------------------------------------------+
| Stage 0   ArtifactValidationAndUpgrade                 |  F-B2
|                                                        |
|   inputs:                                              |
|     ArtifactCore (gbf-artifact)                        |
|     ArtifactManifest                                   |
|     ArtifactAux                                        |
|     TargetDataLoweringArtifact* (sharded ok)           |
|     HintBundle  (ExportFacts/Prefs/Constraints)        |
|     WorkloadManifest references                        |
|     GoldenVector references                            |
|     CompileRequest (gbf-policy)                        |
|     TargetProfile (gbf-hw)                             |
|     ArtifactResolver / store resolver                  |
|     CalibrationSetRef -> CalibrationBundle*            |
|                                                        |
|   checks:                                              |
|     schema epoch admissibility                         |
|     semantic core hash matches manifest                |
|     manifest invariants                                |
|     required_features ⊆ TargetProfile capabilities    |
|     lowering round-trip under packer_version           |
|     calibration freshness vs target/kernel/packer/schema |
|     hint-bundle provenance is internally consistent    |
|     workload references resolve                        |
|     golden vector references resolve                   |
|                                                        |
|   output:                                              |
|     ValidatedInputs (typed handle, hash-bound)         |
+----------------------------+---------------------------+
                             |
                             v
+--------------------------------------------------------+
| Stage 0.5 ResolvedCompilePolicy                        |  F-B2
|                                                        |
|   inputs:                                              |
|     ValidatedInputs                                    |
|     TargetProfile defaults                             |
|     CompileProfile defaults                            |
|     HintBundle preferences/constraints                 |
|     CompileRequest overrides (typed)                   |
|     CalibrationBundleSet                               |
|                                                        |
|   resolves:                                            |
|     ResolvedCompilePolicy {                            |
|       target, profile, objective,                      |
|       effective_constraints,                           |
|       observability, trace_budget,                     |
|       requested_runtime_modes,                         |
|       knobs (CompileKnobs),                            |
|       repair (RepairPolicy),                           |
|       provenance (PolicyProvenance)                    |
|     }                                                  |
|                                                        |
|   emits:                                               |
|     policy_resolution.json (with self_hash)            |
+----------------------------+---------------------------+
                             |
                             v
                       (Stage 1: F-B3 QuantGraph; not in this RFC)
                             |
                             v
+--------------------------------------------------------+
| Stage 2   StaticBudgetReport                           |  F-B4
|                                                        |
|   inputs:                                              |
|     ResolvedCompilePolicy                              |
|     QuantGraph         (F-B3 - via trait stub)         |
|     RuntimeChromeBudget (gbf-policy)                   |
|     TargetProfile                                      |
|                                                        |
|   projections:                                         |
|     per_expert_payload[ExpertId] -> ByteBudget         |
|     per_bank_occupancy[(BankRole, BankId)]             |
|     common_bank_footprint                              |
|     accumulator_maxima[ReductionSiteId]                |
|     projected_wram / projected_sram / projected_hram   |
|     projected_bank_switches_per_token                  |
|     projected_sram_page_switches_per_token             |
|                                                        |
|   decision:                                            |
|     fits: bool (under requested PlacementProfile)      |
|     failures: Vec<BudgetFailure>                       |
|                                                        |
|   emits:                                               |
|     static_budget.json (with self_hash)                |
+--------------------------------------------------------+
                             |
                             v
                  (Stage 3+: F-B5..F-B15)
```

The chunk does not own:

* the `import` stage (consumed),
* `QuantGraph` (consumed via trait stub; produced by F-B3),
* `RuntimeChromeBudget` (consumed; produced by the runtime shell build per `planv0.md` §"Deployability envelope"),
* `BuildReports` (later chunk; this chunk's reports are individual JSON sidecars),
* `repair_report.json` (F-B16).

## 7. Core types

### 7.0 Schema compatibility policy

Stage 0 owns a narrow compatibility gate. It does not own full artifact
migration.

```rust
pub enum ArtifactCompatibilityDecision {
    CurrentSchema,
    LosslessInMemoryUpgrade {
        from_schema: SemVer,
        to_schema: SemVer,
        adapter: CompatibilityAdapterId,
        adapter_hash: Hash256,
    },
}

pub enum ArtifactCompatibilityFailure {
    UnsupportedEpoch { observed: SemVer, supported: SemVer },
    AdapterMissing { observed: SemVer, target: SemVer },
    AdapterNotLossless { adapter: CompatibilityAdapterId },
    SemanticHashChanged {
        before: Hash256,
        after: Hash256,
    },
}
```

Rules:

* "Same epoch" means same SemVer major version. F-B2 does not accept
  cross-major compatibility adapters.
* The canonical artifact semantic hash must not change across a lossless
  in-memory upgrade.
* Adapters are pure functions over typed schema views.
* Adapters may not introduce target profile, compile profile, calibration, or
  placement information into artifact identity.
* Adapters may not rewrite the source artifact on disk.
* Every accepted adapter path appears in `artifact_validation.json`.
* Lossy migration remains out of scope for this chunk.
* Cross-major schema changes are rejected with `SchemaEpochUnsupported` unless
  and until `gbf-migrate` lands and a later RFC explicitly amends this
  compatibility policy.

Formal admissibility:

```text
Accept(adapter) ⇔ adapter.from.major == adapter.to.major ∧ adapter.lossless ∧ SemanticHash(before) == SemanticHash(after)
```

### 7.1 Shared diagnostic taxonomy

```rust
// gbf-policy::diagnostics

pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}

pub enum DiagnosticSeverity {
    Hard,
    Soft,
}

pub enum ValidationOrigin {
    Schema,
    SemanticCore,
    Manifest,
    Lowering,
    Calibration,
    HintBundle,
    Workload,
    GoldenVector,
    CompileRequest,
    PolicyResolution,
    Budget,
}

pub enum ValidationCode {
    SchemaEpochUnsupported,
    SchemaCompatibilityAdapterMissing { observed: SemVer, target: SemVer },
    SchemaCompatibilityAdapterNotLossless { adapter: CompatibilityAdapterId },
    SemanticCoreHashMismatch,
    ArtifactTransportManifestMismatch,
    ManifestInvariantViolated { invariant: ManifestInvariant },
    ArtifactPayloadMalformed { field: FieldPath },
    ArtifactBlobDigestMismatch { blob: BlobRef, expected: Hash256, observed: Hash256 },
    ArtifactAuxMalformed { field: FieldPath },
    ArtifactAuxSidecarMissing { kind: SidecarKind },
    ArtifactAuxSidecarDigestMismatch { kind: SidecarKind, expected: Hash256, observed: Hash256 },
    ArtifactForbiddenBuildIdentityField { field: FieldPath },
    ArtifactRequiredFeatureUnsupported { feature: ArtifactFeature },
    LoweringMissingForTarget { target: TargetProfileId, lowering_profile: DataLoweringProfileId },
    LoweringRoundTripFailed { shard: LoweringShardRef },
    LoweringPackerVersionMismatch { artifact_version: PackerVersion, runtime_version: PackerVersion },
    CalibrationMissing { class: CalibrationLayer },
    CalibrationStale { class: CalibrationLayer, declared: Hash256, observed: Hash256 },
    CalibrationConfidenceTooLow { required: CalibrationConfidenceClass, observed: CalibrationConfidenceClass },
    HintProvenanceInconsistent { fact: TraceProbeId },
    WorkloadRefUnresolved { workload: WorkloadId },
    GoldenVectorMissing { vector: GoldenVectorId },
    GoldenVectorDigestMismatch { vector: GoldenVectorId, expected: Hash256, observed: Hash256 },
    CompileRequestUnsupportedFeature { feature: CompilerFeature },
    CompileRequestProfileForbidsObjective { profile: CompileProfileId, reason: ObjectiveRejection },
    CompileRequestRuntimeModeUnsupported { mode: RuntimeMode },
    CompileRequestTargetIncompatible { target: TargetProfileId, reason: TargetIncompatibilityReason },
    PolicyKnobOutOfBounds { knob: CompileKnobId, requested: KnobValueDescriptor, bounds: CompileKnobBounds },
    PolicyConstraintUnsatisfiable { knob: CompileKnobId, left: CompileKnobBounds, right: CompileKnobBounds },
    PolicyKnobLockedAndOverridden { knob: CompileKnobId },
    BudgetMissingRuntimeChromeBudget,
    BudgetQuantGraphViewMalformed { field: FieldPath },
    BudgetExpertExceedsSlot {
        layer: LayerId,
        expert: ExpertId,
        slot: BudgetSlotId,
        payload_bytes: u32,
        cap_bytes: u32,
    },
    BudgetCommonBankExceedsCap { assigned_bytes: u32, cap_bytes: u32 },
    BudgetWramPeakExceeds { peak: u32, cap: u32 },
    BudgetSramPeakExceeds { peak: u32, cap: u32 },
    BudgetHramPeakExceeds { peak: u32, cap: u32 },
    BudgetAccumulatorOverflow { site: ReductionSiteId, projected_max_abs: u64 },
    BudgetSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    BudgetSramPageSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    BudgetPlacementProfileInfeasible {
        profile: PlacementProfile,
        reason: PlacementInfeasibilityReason,
    },
}

pub enum ValidationDetail {
    None,
    HashMismatch { expected: Hash256, observed: Hash256 },
    Bytes { observed: u32, cap: u32 },
    Range { observed_lo: i64, observed_hi: i64, cap_lo: i64, cap_hi: i64 },
    Selector(SelectorPath),
    Field(FieldPath),
}
```

Validation:

* every constructor is `pub fn` with typed inputs;
* every `code` value carries enough structured information to be rendered in `<crate>: <severity>: <origin>: <code>: <detail>` form by `gbf-cli`;
* `provenance` references are `EvidenceRef` (already in `gbf-foundation`) and resolve into the on-disk artifact bundle.

`ValidationCode` is a closed enum. Every new validation gate must add a variant.

F-B2/F-B4 report semantic validators reject `DiagnosticSeverity::Soft`. The
variant exists only so later chunks can introduce non-fatal diagnostics by
explicitly amending their own report semantics.

### 7.2 Pass output envelope

```rust
// gbf-report

/// Logical Rust wrapper for report bodies.
///
/// Public JSON is still flat:
/// `{ schema, schema_version, outcome, report_self_hash, ...body_fields }`.
/// `gbf-report` owns the custom serializer/deserializer that merges envelope
/// fields with body fields. Implementations must not rely on serde `flatten`
/// here, because `flatten` and `deny_unknown_fields` interact poorly and make
/// unknown-field rejection ambiguous.
pub struct ReportEnvelope<R> {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,
    pub body: R,
}

pub trait ReportBody: Sized {
    const SCHEMA_ID: &'static str;
    const SCHEMA_VERSION: &'static str;
    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>>;
}

pub fn canonicalize<R: ReportBody + serde::Serialize>(
    env: &ReportEnvelope<R>,
) -> Result<Vec<u8>, CanonicalJsonError>;

pub fn compute_self_hash<R: ReportBody + serde::Serialize>(
    env: &ReportEnvelope<R>,
) -> Result<Hash256, ReportSelfHashError>;

pub fn round_trip_self_hash<R: ReportBody + serde::Serialize + serde::de::DeserializeOwned>(
    env: &ReportEnvelope<R>,
) -> Result<(), ReportSelfHashError>;
```

Validation:

* `compute_self_hash` zeros `report_self_hash` to the all-zero sentinel before hashing;
* `round_trip_self_hash` re-hashes the report after parse and rejects if `stored != computed`;
* `canonicalize` produces UTF-8 bytes with sorted keys, no insignificant
  whitespace, and base-10 integers. `artifact_validation.v1`,
  `policy_resolution.v1`, and `static_budget.v1` reject floating-point JSON
  numbers entirely; fractional quantities must use explicitly scaled integer
  fields such as `_q8_8` or `_q16_16`;
* the trait object exists per-schema; downstream code does not write its own canonicalizer.
* public JSON remains flat: `schema`, `schema_version`, `outcome`, and
  `report_self_hash` are top-level fields, followed by the report body fields;
* report body structs must not duplicate `schema`, `schema_version`, `outcome`,
  or `report_self_hash`;
* all report body structs use `#[serde(deny_unknown_fields)]`;
* all report enums use an explicitly tagged representation so unknown variants
  are rejected by deserialization.
* `artifact_validation.v1`, `policy_resolution.v1`, and `static_budget.v1`
  reject any diagnostic whose severity is `Soft`.
* nullable report fields are restricted to the explicit nullability list in
  §2.5 or the corresponding schema section.

### 7.3 Stage 0 — `ArtifactValidationAndUpgrade`

```rust
// gbf-codegen::stages::validate

use std::borrow::Cow;

pub struct ValidateInputs<'a> {
    pub artifact: &'a ImportedArtifactView,
    pub lowerings: &'a [TargetDataLoweringArtifact],
    pub workloads: &'a [WorkloadManifestRef],
    pub golden_vectors: &'a [GoldenVectorRef],
    pub compile_request: &'a CompileRequest,
    pub target_profile: &'a TargetProfile,
    /// The selected profile spec, looked up by `compile_request.profile`.
    ///
    /// Stage 0 may inspect this only for profile/request admissibility and to
    /// read `RiskPolicy::require_confidence_at_least` when applying the
    /// calibration-confidence gate. Full knob/default/bound resolution remains
    /// Stage 0.5's job.
    pub compile_profile: &'a CompileProfileSpec,
    pub calibration: Option<&'a CalibrationBundleSet>,
    /// Resolver used for artifact blobs, sidecars, evidence refs, workload
    /// refs, and golden-vector refs. Stage 0 must not depend on ambient global
    /// store state.
    pub resolver: &'a dyn ArtifactResolver,
}

pub trait ArtifactResolver {
    fn resolve_blob(&self, blob: &BlobRef) -> Result<ResolvedBlob, ArtifactResolveError>;
    fn resolve_sidecar(&self, sidecar: &SidecarRef) -> Result<ResolvedSidecar, ArtifactResolveError>;
    fn resolve_evidence(&self, evidence: &EvidenceRef) -> Result<ResolvedEvidence, ArtifactResolveError>;
    fn resolve_workload(&self, workload: &WorkloadManifestRef) -> Result<ResolvedWorkload, ArtifactResolveError>;
    fn resolve_golden_vector(&self, vector: &GoldenVectorRef) -> Result<ResolvedGoldenVector, ArtifactResolveError>;
}

pub struct ImportedArtifactView {
    pub core: ArtifactCore,
    pub aux: ArtifactAux,
    /// Missing hint bundle is normalized during import to a canonical empty
    /// `HintBundle` with a stable hash. Downstream reports never encode
    /// `hint_bundle_hash = null`.
    pub hint_bundle: HintBundle,
    pub reference: Option<ReferenceLink>,
    pub transport: ArtifactTransportIdentity,
}

pub struct ArtifactTransportIdentity {
    pub source_uri: Option<String>,
    pub transport_hash: Hash256,
    pub import_tool_hash: Hash256,
}

pub struct ValidatedInputs<'a> {
    /// Borrowed for current-schema artifacts; owned when Stage 0 applied a
    /// registered lossless in-memory compatibility adapter.
    pub artifact: Cow<'a, ImportedArtifactView>,
    pub lowerings: &'a [TargetDataLoweringArtifact],
    pub workloads: &'a [WorkloadManifestRef],
    pub golden_vectors: &'a [GoldenVectorRef],
    pub compile_request: &'a CompileRequest,
    pub target_profile: &'a TargetProfile,
    /// Passing Stage 0 always has a resolved calibration set. Missing
    /// calibration is represented only as a Stage-0 failure report.
    pub calibration: &'a CalibrationBundleSet,
    pub input_hashes: ValidatedInputHashes,
    _private: PrivateValidatedInputs,
}

struct PrivateValidatedInputs(());

pub struct ValidationProduct<'a> {
    pub validated: ValidatedInputs<'a>,
    pub report: ReportEnvelope<ArtifactValidationReportBody>,
    pub artifact_validation_self_hash: Hash256,
    pub artifact_validation_canonical_bytes_hash: Hash256,
}

pub struct ValidationStageFailure {
    pub report: ReportEnvelope<ArtifactValidationReportBody>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

pub struct ValidatedInputHashes {
    /// Hash of the imported source artifact before any compatibility adapter.
    pub artifact_source_hash: Hash256,
    /// Hash of the current-schema view consumed by downstream stages.
    pub artifact_effective_core_hash: Hash256,
    pub artifact_manifest_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub compile_profile_hash: Hash256,
    pub calibration_hash: Hash256,
    pub compatibility_adapter_hash: Option<Hash256>,
}

pub fn validate_artifact_and_request<'a>(
    inputs: ValidateInputs<'a>,
) -> Result<ValidationProduct<'a>, ValidationStageFailure>;
```

The Stage 0 validation classes are:

Stage 0 treats `artifact.core.manifest` as the only manifest of record. Any
outer transport manifest is import metadata only. If transport metadata and
`artifact.core.manifest` disagree on semantic core hash, schema version,
component digest, or lineage, Stage 0 rejects the artifact with
`ArtifactTransportManifestMismatch`. The typed transport carrier is
`ArtifactTransportManifestMetadata`; when an importer does not provide it,
Stage 0 still applies the source-hash identity check but cannot claim
field-level transport/manifest agreement.

| #  | Class                        | Rejects                                                                                                                                                        | Code(s)                                                                                                                                                                                       |
| -- | ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1  | Schema epoch                 | unsupported `ArtifactSchemaVersion`                                                                                                                            | `SchemaEpochUnsupported`                                                                                                                                                                      |
| 2  | Semantic core hash           | manifest's recorded core hash does not match canonical recomputation                                                                                           | `SemanticCoreHashMismatch`                                                                                                                                                                    |
| 3  | Manifest invariants          | feature set internally inconsistent with declared epoch; missing required components; component digest mismatch; forbidden build-identity field present          | `ManifestInvariantViolated`, `ArtifactForbiddenBuildIdentityField`                                                                                                                            |
| 4  | Artifact semantic payload    | canonical tensor blob missing/mismatched; logical LUT malformed; quant spec malformed; sequence semantics missing or inconsistent; decode capabilities malformed | `ArtifactPayloadMalformed`, `ArtifactBlobDigestMismatch`                                                                                                                                      |
| 5  | Artifact aux sidecars        | golden vector sidecar missing; checkpoint schema sidecar missing when required; conformance/reference cache digest mismatch; interaction bundle malformed       | `ArtifactAuxSidecarMissing`, `ArtifactAuxSidecarDigestMismatch`, `ArtifactAuxMalformed`                                                                                                       |
| 6  | Target-data lowering         | no compatible lowering for target family/profile; per-shard packer round-trip fails; per-shard hashes inconsistent; assembled lowering manifest hash mismatch; `packer_version` not supported | `LoweringMissingForTarget`, `LoweringRoundTripFailed`, `LoweringPackerVersionMismatch`                                                                                                        |
| 7  | Calibration binding          | calibration ref unresolved; calibration target/kernel/packer/schema hash mismatch; confidence below profile requirement                                        | `CalibrationMissing`, `CalibrationStale`, `CalibrationConfidenceTooLow`                                                                                                                       |
| 8  | Hint provenance              | a fact references an `EvidenceScope` outside the active artifact/workload/target/lowering scope                                                                | `HintProvenanceInconsistent`                                                                                                                                                                  |
| 9  | Workload/golden refs         | workload references unresolved; golden vector references unresolved or digest-mismatched                                                                       | `WorkloadRefUnresolved`, `GoldenVectorMissing`, `GoldenVectorDigestMismatch`                                                                                                                  |
| 10 | CompileRequest admissibility | required artifact feature unsupported by target; required compiler feature unsupported by this compiler build; profile forbids objective; runtime mode unsupported; target profile incompatible with artifact target-family lowering | `ArtifactRequiredFeatureUnsupported`, `CompileRequestUnsupportedFeature`, `CompileRequestProfileForbidsObjective`, `CompileRequestRuntimeModeUnsupported`, `CompileRequestTargetIncompatible` |

Validation order is fixed (1 → 10). A class's failure short-circuits subsequent classes only when continuing would dereference an unverified value (e.g. a manifest with a wrong semantic core hash means we cannot trust embedded references). Within a class, all diagnostics are accumulated and returned together — Stage 0 reports as much as it can.

Forbidden build-identity fields are any artifact, manifest, aux, or lowering
field whose path is one of:

```text
/build_identity
/build_identity_block
/compatibility_envelope
/encoded_rom_hash
/backend_identity
/stage12_identity
```

Those values belong to Stage 12/backend output identity, not to frozen input
artifacts.

`ValidatedInputs` is a private-module-constructed handle. Downstream stages
cannot synthesize one because the witness field is private to
`gbf-codegen::stages::validate`. This pattern lifts F-A3's `BankLease` token
discipline into the validation layer.

**Deferred (later chunks):**

* Cross-stage validation (e.g. "this expert references a packer the schedule has not approved").
* Migration logic from older artifact schemas (`gbf-migrate` is F-A6b).
* `ArtifactOracle` consultation.

### 7.4 Stage 0.5 — `resolve_policy`

F-B2 introduces typed `CompileProfileSpec` records for the canonical first-wave
profiles: `Bringup`, `Default`, `Trace`, and `Recovery`.

```rust
pub struct CompileProfileSpec {
    pub id: CompileProfileId,
    pub defaults_hash: Hash256,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub repair_policy: RepairPolicy,
    pub risk_policy: RiskPolicy,
    pub knob_defaults: CompileKnobPartialValues,
    pub knob_bounds: CompileKnobPartialBounds,
    pub locks: KnobLockSet,
}
```

F-B2 must ship deterministic profile fixtures for all four canonical profiles.
No profile carries a relaxation surface — see §2.13. `Bringup` is distinguished
from `Default`/`Trace`/`Recovery` only by its knob defaults, observability,
trace budget, repair policy, and explicit `risk_policy`. The
`risk_policy.calibration_confidence_requirement` field determines
whether `BootstrapCalibrationBundle` is acceptable.

```rust
// gbf-codegen::stages::policy

pub fn resolve_policy<'a>(
    validation: &'a ValidationProduct<'a>,
) -> Result<ResolvedPolicyProduct, PolicyResolutionStageFailure>;

pub struct ResolvedPolicyProduct {
    pub policy: ResolvedCompilePolicy,
    pub input_hashes: ValidatedInputHashes,
    pub artifact_validation_self_hash: Hash256,
    pub report: ReportEnvelope<PolicyResolutionReportBody>,
    pub policy_resolution_self_hash: Hash256,
    pub policy_resolution_canonical_bytes_hash: Hash256,
}

pub struct PolicyResolutionStageFailure {
    pub report: ReportEnvelope<PolicyResolutionReportBody>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}
```

The returned `ResolvedCompilePolicy` (defined in `gbf-policy`, per `planv0.md` line 645):

```rust
// gbf-policy

pub struct ResolvedCompilePolicy {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub effective_constraints: EffectiveConstraints,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub knobs: CompileKnobs,
    pub repair: RepairPolicy,
    pub provenance: PolicyProvenance,
}
```

Resolution model:

Policy resolution collects typed `ConstraintFrame`s and merges them into a
single `ResolvedCompilePolicy`. The implementation may process frames in the
order below, but correctness is defined by the merge rules, not by incidental
mutation order.

```rust
pub struct ConstraintFrame {
    pub source: PolicySource,
    pub evidence: Vec<EvidenceRef>,
    pub defaults: CompileKnobPartialValues,
    pub hard_bounds: CompileKnobPartialBounds,
    pub preferences: CompileKnobPreferences,
    pub locks: KnobLockSet,
}
```

Frame sources:

1. Start from `TargetProfile` defaults → seed `CompileKnobs::global`, `bounds`, `RepairPolicy` defaults, `EffectiveConstraints::target_caps`.
2. Apply `CompileProfile` defaults → may tighten bounds, set `ObservabilityMode`, set `TraceBudget`, set `RiskPolicy::require_confidence_at_least`.
3. Apply `HintBundle::preferences` for soft nudges that fall inside `bounds`.
4. Apply `HintBundle::constraints` as additional hard constraints.
5. Apply `CompileRequest::constraint_overrides` → may further tighten, never loosen.
6. Apply `CalibrationBundle` data-driven knobs (e.g. `pressure.slice_cycles`).
7. Verify monotonicity: every `bounds` field is at least as tight after resolution as before.
8. Emit `PolicyProvenance` recording `target_defaults`, `profile_defaults`, `hint_bundle_hash`, `compile_request_hash`, `calibration_hash`, plus per-knob `ConstraintProvenance`.

Merge rules:

* defaults fill unset values only;
* target-profile fixtures must leave profile-specific knob values unset unless
  the target schema explicitly marks the field as profile-selectable;
* profile defaults may set unset values and tighten bounds, but may not
  overwrite an already set target default unless the target field is explicitly
  profile-selectable;
* hard bounds merge by meet in the knob's declared partial order;
* an empty meet is a hard `PolicyConstraintUnsatisfiable`, not a generic
  string error;
* preferences are considered only after hard bounds are known;
* preferences outside bounds are ignored and recorded;
* compile-request overrides may tighten bounds or select a value within bounds;
* compile-request overrides may not loosen a bound under any circumstance;
* calibration may set values or tighten thresholds but may not weaken hard
  compile-request constraints;
* locks apply after profile frames and before all later frames;
* any later frame — hint, compile-request, or calibration — that attempts to
  change a locked knob to a non-identical value is a hard diagnostic;
* locked override attempts are hard diagnostics.

All `KnobLockSet` entries set by the `CompileProfile` are honored. A `CompileRequest` override against a locked knob is a Hard `PolicyKnobLockedAndOverridden`.

A knob value outside its `bounds` is a Hard `PolicyKnobOutOfBounds`. There is no profile-time relaxation of a `bounds` field; see §2.13.

`ResolvedCompilePolicy::provenance` retains path-level provenance so reviewers
can answer "where did this exact scalar come from" for every load-bearing
policy value.

```rust
pub struct CompileKnobPath {
    pub knob: CompileKnobId,
    pub selector: Option<SelectorPath>,
    pub field: Option<FieldPath>,
}

pub struct ConstraintProvenance {
    pub source: PolicySource,
    pub operation: ConstraintOperation,
    pub evidence: Vec<EvidenceRef>,
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

Every field in `CompileKnobValues`, every field in `CompileKnobBounds`, every
locked knob, and every targeted override must have a corresponding
`CompileKnobPath` provenance entry.

`ConstraintOperation::AuthorizedRelaxation` does not exist in the F-B2 schema.
A later RFC may add it only by explicitly amending this RFC.

**Deferred (F-B16):**

* `RepairProposal::source = PlanningStage::*` provenance value paths.
* Loop-driven `KnobDelta` application.
* `EstimatedCostDelta` synthesis (lives in F-B14 + F-B16).
* `repair_report.json` emission.

### 7.4.5 `artifact_validation.json` schema

`artifact_validation.json` is the Stage 0 report. It is emitted before
`policy_resolution.json` exists and is the canonical diagnostic artifact for
schema, identity, lowering, hint, workload, golden-vector, compile-request, and
calibration failures.

```rust
pub struct ArtifactValidationReportBody {
    pub identity: ArtifactValidationIdentitySection,
    pub compatibility: ArtifactCompatibilitySection,
    pub checked_inputs: ArtifactValidationInputSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct ArtifactValidationIdentitySection {
    /// Hash of the imported source artifact before any compatibility adapter.
    pub artifact_source_hash: Option<Hash256>,
    /// Hash of the current-schema view consumed by downstream stages.
    pub artifact_effective_core_hash: Option<Hash256>,
    pub artifact_manifest_hash: Option<Hash256>,
    pub semantic_core_hash: Option<Hash256>,
    pub artifact_aux_hash: Option<Hash256>,
    pub lowering_manifest_hash: Option<Hash256>,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub compile_profile_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
    pub compatibility_adapter_hash: Option<Hash256>,
}

pub struct ArtifactCompatibilitySection {
    pub decision: Option<ArtifactCompatibilityDecision>,
    pub failures: Vec<ArtifactCompatibilityFailure>,
}

pub struct ArtifactValidationInputSection {
    pub workload_refs: Vec<WorkloadId>,
    pub golden_vector_refs: Vec<GoldenVectorId>,
    pub required_artifact_features: BTreeSet<ArtifactFeature>,
    pub required_compiler_features: BTreeSet<CompilerFeature>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}
```

Semantic invariants:

* `schema == "artifact_validation.v1"`;
* `outcome == Failed` iff at least one `Hard` diagnostic is present;
* `outcome == Passed` implies no `Hard` diagnostics;
* `outcome == Passed` implies all identity hash fields except
  `compatibility_adapter_hash` are `Some`/present, and
  `calibration_hash.is_some()`;
* if `compatibility.decision` is `LosslessInMemoryUpgrade`, the before/after
  semantic core hash is identical;
* `hint_bundle_hash` is always present. No-hint builds use the canonical empty
  `HintBundle` hash;
* `workload_refs` and `golden_vector_refs` are sorted;
* every diagnostic has typed provenance;
* no diagnostic has severity `Soft`;
* `report_self_hash` round-trips per §2.4.

### 7.5 `policy_resolution.json` schema

```rust
// gbf-report

pub struct PolicyResolutionReportBody {
    pub artifact_identity: ArtifactIdentitySection,
    pub compile_request: CompileRequestSection,
    pub result: Option<PolicyResolutionSuccessSection>,
    pub hint_consumption: HintConsumptionSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct PolicyResolutionSuccessSection {
    pub resolved: ResolvedSection,
    pub compile_knobs: CompileKnobsSection,
    pub provenance: PolicyProvenanceSection,
}

pub struct HintConsumptionSection {
    pub facts_used: Vec<FactUse>,
    pub preferences_honored: Vec<PreferenceUse>,
    pub preferences_ignored: Vec<IgnoredPreference>,
    pub constraints_enforced: Vec<ConstraintEnforcement>,
}

pub struct ArtifactIdentitySection {
    pub artifact_core_hash: Hash256,
    pub artifact_manifest_hash: Hash256,
    pub semantic_lineage: LineageId,
    pub lowering_manifest_hash: Hash256,
    pub hint_bundle_hash: Hash256,
    pub workload_refs: Vec<WorkloadId>,
    pub golden_vector_refs: Vec<GoldenVectorId>,
}

pub struct CompileRequestSection {
    pub compile_request_hash: Hash256,
    pub target: TargetProfileId,
    pub target_profile_hash: Hash256,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub required_features: BTreeSet<CompilerFeature>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub calibration_set_ref: CalibrationSetRef,
    pub calibration_hash: Hash256,
}

pub struct ResolvedSection {
    pub effective_constraints: EffectiveConstraints,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub repair: RepairPolicy,
}

pub struct CompileKnobsSection {
    pub global: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub locks: KnobLockSet,
    pub overrides: CompileKnobOverrides,
    pub provenance: Vec<CompileKnobProvenanceEntry>,
}

pub struct CompileKnobProvenanceEntry {
    pub path: CompileKnobPath,
    /// Ordered chain of operations that produced the final value or bound at
    /// `path`, e.g. target default -> profile tighten -> request override.
    pub chain: Vec<ConstraintProvenance>,
}

pub struct PolicyProvenanceSection {
    pub target_defaults: Hash256,
    pub profile_defaults: Hash256,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub calibration_hash: Hash256,
}

pub struct ValidationDiagnosticRecord {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}
```

Semantic invariants (validated by `PolicyResolutionReport::validate_semantics`):

* `schema == "policy_resolution.v1"`;
* `compile_request.target == resolved` target invariant matches selected target profile id;
* `compile_knobs.provenance` is sorted by `CompileKnobPath`;
* every `CompileKnobPath` produced by the resolver appears exactly once in
  `compile_knobs.provenance`;
* every provenance chain is non-empty;
* every `CompileKnobValues` and `CompileKnobBounds` subfield references a
  provenance chain whose `PolicySource` values are all in
  `TargetDefault | ProfileDefault | CompileRequestOverride | HintBundle | Calibration`
  during F-B2/F-B4. Stage 0.5 forbids `RepairProposal(_)`;
* no provenance chain may contain `ConstraintOperation::AuthorizedRelaxation`;
* `compile_knobs.bounds` is monotonically tighter than the target's `CompileKnobBounds` defaults;
* `compile_knobs.locks` ⊆ `CompileKnobId`;
* `report_self_hash` round-trips per §2.4;
* `outcome == Passed` iff `result.is_some()` and no `Hard` diagnostic appears.
* `outcome == Failed` iff `result.is_none()` and at least one `Hard` diagnostic appears.
* no diagnostic has severity `Soft`;
* `policy_resolution.json` never contains a partial `ResolvedCompilePolicy`.
  Resolution failures are reports with diagnostics, not half-resolved policies.
* every `HintBundle::preferences` entry appears exactly once in either
  `hint_consumption.preferences_honored` or `hint_consumption.preferences_ignored`;
  every `HintBundle::constraints` entry appears in
  `hint_consumption.constraints_enforced` or produces a hard diagnostic.

**Deferred (F-B16):**

* `compile_knobs.provenance` may reference `RepairProposal(RepairProposalId)`.
* `repair_report.json` emission.

### 7.6 Stage 2 — `StaticBudgetReport` types

The Stage 2 input shape:

```rust
// gbf-codegen::stages::budget

pub struct BudgetInputs<'a, Q: QuantGraphBudgetSource + ?Sized> {
    pub policy: &'a ResolvedPolicyProduct,
    pub quant_graph: &'a Q,
    pub runtime_chrome_budget: Option<&'a RuntimeChromeBudget>,
    pub target_profile: &'a TargetProfile,
}

/// F-B4-owned seam consumed by the budget pass.
///
/// Synthetic fixtures implement this trait in F-B4. F-B3 later implements it
/// for the real `QuantGraph`.
pub trait QuantGraphBudgetSource {
    fn quant_graph_hash(&self) -> Hash256;
    fn semantic_core_hash(&self) -> Hash256;
    fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError>;
}

/// Serializable Stage-2 input view. F-B4 owns this view; F-B3 owns construction
/// of the real QuantGraph that can produce it.
pub struct QuantGraphBudgetView {
    pub semantic_core_hash: Hash256,
    pub quant_graph_hash: Hash256,
    pub experts: Vec<ExpertProjection>,
    pub shared_kernels: Vec<SharedKernelProjection>,
    pub shared_luts: Vec<SharedLutProjection>,
    pub reduction_sites: Vec<ReductionSiteProjection>,
    pub sequence_state: SequenceStateProjection,
    pub routing: RoutingProjection,
}

pub struct ExpertProjection {
    pub layer: LayerId,
    pub expert: ExpertId,
    pub rows: u32,
    pub cols: u32,
    pub metadata_bytes: ByteBudget,
    pub plan: TernaryWeightPlan,
}

pub struct SharedKernelProjection { /* ... */ }
pub struct SharedLutProjection { /* ... */ }

pub struct ReductionSiteProjection {
    pub site: ReductionSiteId,
    pub layer: Option<LayerId>,
    pub expert: Option<ExpertId>,
    pub term_count: u32,
    pub input_max_abs_q: u32,
    pub weight_max_abs_q: u32,
    pub bias_max_abs_q: Option<u32>,
    pub accumulator_domain: AccumulatorDomain,
}

pub enum AccumulatorDomain {
    RawIntegerProducts,
    PostScaleQ8_8,
    PostScaleQ16_16,
}

pub struct SequenceStateProjection { /* ... */ }
pub struct RoutingProjection { /* ... */ }
```

`QuantGraphBudgetView` semantic validator:

* `experts` sorted by `(layer, expert)`;
* `shared_kernels` sorted by `KernelSpecId`;
* `shared_luts` sorted by LUT id;
* `reduction_sites` sorted by `ReductionSiteId`;
* every `ExpertProjection` references a known layer and has non-zero shape;
* every projection quantity uses checked integer arithmetic;
* `quant_graph_hash` is the canonical hash of the full F-B3 QuantGraph, not
  only this budget view.

Stage 2 must call `to_budget_view()`, validate the returned
`QuantGraphBudgetView`, and use that view for all byte math. Stage 2 must not
trust producer-supplied payload byte totals.

F-B4 does not recompute the full QuantGraph hash; F-B3 owns that canonical hash
rule. F-B4 validates the view body and records the supplied `quant_graph_hash`
as the identity of the full QuantGraph.

If `runtime_chrome_budget == None`, Stage 2 fails before calling
`to_budget_view()`.

If `to_budget_view()` fails or the returned view fails its semantic validator,
Stage 2 emits `BudgetQuantGraphViewMalformed` and no generic string error.

Expert payload byte math is owned by F-B4, not trusted from the QuantGraph
producer.

`gbf-artifact::TernaryWeightPlan::compute_byte_cost` remains scoped to
target-independent artifact/model diagnostics. It is not the canonical deployed
F-B4 payload formula because it uses the historical flattened-element
`PerGroup` scale count, has no target-profile override for `Pow2` scale widths,
does not include Stage 2 metadata bytes, and saturates into `ByteCost`.

```text
weight_count = rows * cols        // checked u64

Ternary2:
  weight_bytes = ceil(weight_count / 4)

Binary1:
  weight_bytes = ceil(weight_count / 8)

SparseTernaryBitplanes:
  weight_bytes = positive_bitplane_bytes
               + negative_bitplane_bytes
               + sparse_metadata_bytes
  where the sparse metadata layout is named in `TernaryWeightPlan`.

Scale format:
  Q8_8 => 2 bytes per scale
  Q4_4 => 1 byte per scale
  Pow2 => 1 byte per scale unless overridden by target profile

Scale count:
  PerTensor       => 1
  PerOutputRow    => rows
  PerGroup(group) => ceil(rows / group)

payload_bytes = weight_bytes + scale_bytes + metadata_bytes
```

All intermediate math uses checked `u64`. Report-visible fields reject values
that cannot fit into their declared width.

Accumulator projection:

```text
raw_product_bound = input_max_abs_q * weight_max_abs_q
sum_bound = term_count * raw_product_bound
projected_max_abs = sum_bound + bias_max_abs_q.unwrap_or(0)
```

All math uses checked `u128` internally. The report rejects projections that
cannot be represented in `u64`.

`i16_safe = projected_max_abs <= i16::MAX`
`i32_safe = projected_max_abs <= i32::MAX`

Stage 2 fails hard on `!i32_safe`. `!i16_safe` is recorded for RangePlan but is
not by itself a Stage-2 hard failure unless the active policy locks
`ReductionPlanCeiling::SingleI16Only`.

Stage 2 uses a deterministic static placement model for each
`PlacementProfile`. This is not final layout; it is a necessary fit check.

```rust
pub enum StaticPlacementModel {
    StrictOnePerBank,
    BudgetedFirstFit,
    PackedExpertsFirstFitDecreasing,
}
```

Mapping from resolved `PlacementProfile` to static model is fixed in v1:

| `PlacementProfile`        | `StaticPlacementModel`                 |
|---------------------------|----------------------------------------|
| `StrictOnePerBank`        | `StrictOnePerBank`                     |
| `Budgeted`                | `BudgetedFirstFit`                     |
| `PackedExperts`           | `PackedExpertsFirstFitDecreasing`      |

Rules:

* `StrictOnePerBank`: each expert must fit in a distinct eligible
  `ExpertBank` slot.
* `BudgetedFirstFit`: experts are considered in `(layer, expert)` order and
  assigned to the first eligible slot with enough effective cap; the model
  records residual bytes for every slot.
* `PackedExpertsFirstFitDecreasing`: experts are sorted by descending payload
  bytes, then `(layer, expert)`, and assigned by deterministic first fit.
* common kernels, shared LUTs, and optional shared dense FFN payloads are
  assigned only to slots whose class/caps allow common-bank residency.
* `Bank0Free` is used only for components explicitly marked Bank0-compatible.
* every assignment decision is recorded in `per_bank_occupancy`.

The active model is recorded in `static_budget.json`.

The Stage 2 typed product:

```rust
pub struct StaticBudgetReport {
    pub per_expert_payload: BTreeMap<(LayerId, ExpertId), ByteBudget>,
    pub per_bank_occupancy: BTreeMap<BudgetSlotId, BankOccupancySummary>,
    pub common_bank_footprint: CommonBankFootprint,
    pub accumulator_maxima: BTreeMap<ReductionSiteId, AccumulatorBound>,
    pub projected_wram: ProjectedSize,
    pub projected_sram: ProjectedSize,
    pub projected_hram: ProjectedSize,
    pub projected_bank_switches_per_token: ProjectedSwitchCount,
    pub projected_sram_page_switches_per_token: ProjectedSwitchCount,
    pub fits: bool,
    pub interpretation: StaticFitInterpretation,
    pub failures: Vec<BudgetFailure>,
}

pub struct ProjectedSwitchCount {
    pub upper_bound: u16,
    pub expected_q16_16: Option<u32>,
    pub decision_value: u16,
    pub source: SwitchProjectionSource,
}

pub enum SwitchProjectionSource {
    ConservativeStaticUpperBound,
    HintWeightedExpectedWithStaticCap,
    CalibrationClosedFormWithStaticCap,
}

pub enum StaticFitInterpretation {
    /// All Stage-2 necessary static checks passed. Later placement/layout
    /// passes remain authoritative for final deployability.
    PassesNecessaryStaticChecks,
    /// At least one necessary static check failed, so later passes cannot make
    /// the build valid without a policy change.
    FailsNecessaryStaticChecks,
}

pub struct BankOccupancySummary {
    pub slot: BudgetSlotId,
    pub class: BudgetSlotClass,
    pub usable_bytes: u32,
    pub assigned_bytes: u32,
    pub residual_bytes: i32,   // signed: negative under busted profiles
    pub assigned_components: Vec<AssignedComponent>,
    pub placement_caps: BTreeSet<PlacementProfile>,
}

pub struct CommonBankFootprint {
    pub kernel_bytes: ByteBudget,
    pub lut_bytes: ByteBudget,
    pub shared_dense_ffn_bytes: Option<ByteBudget>, // per bd-33q handoff
    pub aggregate_bytes: ByteBudget,
}

pub struct AccumulatorBound {
    pub site: ReductionSiteId,
    pub projected_max_abs: u64,
    pub i16_safe: bool,
    pub i32_safe: bool,
}

pub struct ProjectedSize {
    pub peak_bytes: u32,
    pub source: ProjectedSizeSource,
}

pub enum ProjectedSizeSource {
    StaticGraphProjection,
    HintBundleConstraint,
    CalibrationSamplingClosedForm,
}

pub enum BudgetFailure {
    MissingRuntimeChromeBudget,
    QuantGraphBudgetViewMalformed { field: FieldPath },
    ExpertExceedsSlot {
        layer: LayerId,
        expert: ExpertId,
        slot: BudgetSlotId,
        payload_bytes: u32,
        cap_bytes: u32,
        excess_bytes: u32,
    },
    CommonBankExceedsCap {
        assigned_bytes: u32,
        cap_bytes: u32,
        excess_bytes: u32,
    },
    WramPeakExceedsCap { peak: u32, cap: u32 },
    SramPeakExceedsCap { peak: u32, cap: u32 },
    HramPeakExceedsCap { peak: u32, cap: u32 },
    AccumulatorExceedsI32 { site: ReductionSiteId, projected_max_abs: u64 },
    BankSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    SramPageSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    PlacementProfileInfeasible { profile: PlacementProfile, reason: PlacementInfeasibilityReason },
}
```

Validation:

* every expert appears in `per_expert_payload`;
* `per_bank_occupancy` covers every slot in the active `RuntimeChromeBudget`;
* `common_bank_footprint.aggregate_bytes` equals the sum of its components;
* `fits` is a function of `failures.is_empty()`;
* every `failure` corresponds to a recorded `ValidationDiagnostic` in the diagnostics list of the emitted JSON;
* `BankOccupancySummary::residual_bytes` is signed so a busted profile reports `−N` rather than clamping;
* `projected_bank_switches_per_token` is computed under a documented routing model named in `static_budget.json`.

Budget decisions compare caps against `decision_value`.

Default rule:

* `decision_value = upper_bound`.

A profile may permit `decision_value` to use an expected value only when:

* the profile explicitly allows expected-value static budget decisions;
* the expectation source is named;
* the static upper bound is still reported;
* the report records the risk policy that permitted this choice.

Bringup, Default, Trace, and Recovery v1 use the upper bound.

`fits = true` means "passes Stage-2 necessary static checks." It does not mean
the final ROM is proven placeable. `RomWindowPlan`, `ArenaPlan`,
`ResourceStateValidation`, and backend layout remain authoritative for final
deployability. `fits = false` is a hard stop because at least one necessary
budget condition is already violated.

Amends planv0: this RFC deliberately tightens the wording around
`StaticBudgetReport`; Stage 2 is an early necessary-condition gate, not a
substitute for later physical planning certificates.

**Deferred (later chunks):**

* The actual `RomWindowPlan` (F-B10) decision that consumes these projections.
* The actual `ScheduleCostAnalysis` (F-B14) cycle envelope.
* `OverlayPlan` interaction (F-B11).
* `SramPagePlan` interaction (F-B9).

### 7.7 `static_budget.json` schema

```rust
// gbf-report

pub struct StaticBudgetReportBody {
    pub identity: BudgetIdentitySection,
    pub policy: BudgetPolicySection,
    pub runtime_chrome_budget: Option<RuntimeChromeBudgetSection>,
    pub projections: BudgetProjectionSection,
    pub decision: BudgetDecisionSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct BudgetIdentitySection {
    pub artifact_core_hash: Hash256,
    pub quant_graph_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Option<Hash256>,
    pub target_profile_hash: Hash256,
}

pub struct BudgetPolicySection {
    pub placement_profile: PlacementProfile,
    pub objective_hash: Hash256,
}

pub struct RuntimeChromeBudgetSection {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub runtime_nucleus_hash: Hash256,
    pub rom_slots: Vec<RomBudgetSlotEntry>,
    pub memory_caps: RuntimeMemoryCapSection,
    pub wram_reserved: u16,
    pub sram_reserved: u32,
}

pub struct RuntimeMemoryCapSection {
    pub wram_usable_bytes: u32,
    pub sram_usable_bytes: u32,
    pub hram_usable_bytes: u32,
    pub source_target_profile_hash: Hash256,
}

pub struct BudgetProjectionSection {
    pub per_expert_payload: Vec<PerExpertEntry>,
    pub per_bank_occupancy: Vec<PerBankEntry>,
    pub common_bank_footprint: CommonBankFootprintSection,
    pub accumulator_maxima: Vec<AccumulatorEntry>,
    pub projected_wram: ProjectedSizeSection,
    pub projected_sram: ProjectedSizeSection,
    pub projected_hram: ProjectedSizeSection,
    pub projected_bank_switches_per_token: ProjectedSwitchCountSection,
    pub projected_sram_page_switches_per_token: ProjectedSwitchCountSection,
    pub routing_model: RoutingModelSection,
}

pub struct ProjectedSwitchCountSection {
    pub upper_bound: u16,
    pub expected_q16_16: Option<u32>,
    pub decision_value: u16,
    pub source: SwitchProjectionSource,
}

pub struct BudgetDecisionSection {
    pub fits: bool,
    pub interpretation: StaticFitInterpretation,
    pub placement_model: StaticPlacementModel,
    pub failures: Vec<BudgetFailureRecord>,
}
```

Semantic invariants:

* `schema == "static_budget.v1"`;
* `report_self_hash` round-trips per §2.4;
* `runtime_chrome_budget_hash.is_none()` iff
  `runtime_chrome_budget.is_none()` iff the report has `outcome == Failed`
  and contains exactly one `BudgetMissingRuntimeChromeBudget` diagnostic
  and exactly one `BudgetFailure::MissingRuntimeChromeBudget`;
* all other Stage 2 reports must include `runtime_chrome_budget_hash` and
  `runtime_chrome_budget`;
* when `runtime_chrome_budget.is_some()`, the embedded
  `runtime_chrome_budget` section is a canonical review excerpt and must hash
  to `runtime_chrome_budget_hash`; the excerpt is not the source of truth;
* `per_expert_payload` is sorted by `(LayerId, ExpertId)`;
* `per_bank_occupancy` is sorted by `BudgetSlotId`;
* when `runtime_chrome_budget.is_some()`, `per_bank_occupancy` covers every
  slot in the active `RuntimeChromeBudget`;
* when `decision.fits == true`, every entry in `per_expert_payload` has a
  corresponding `BudgetSlotId` referenced in
  `per_bank_occupancy.assigned_components`;
* when `decision.fits == false`, unassigned experts must either appear in a
  placement/budget failure or in an explicit `UnassignedBecause` field in the
  corresponding projection entry;
* `decision.fits == decision.failures.is_empty()`;
* when `decision.fits == true`, `interpretation` must be
  `PassesNecessaryStaticChecks`;
* when `decision.fits == false`, `interpretation` must be
  `FailsNecessaryStaticChecks`;
* every `BudgetFailure` corresponds to exactly one `ValidationDiagnostic` in `diagnostics` with matching `(layer, expert, slot, ...)`;
* no diagnostic has severity `Soft`;
* if `BudgetMissingRuntimeChromeBudget` is present, no `QuantGraphBudgetView`
  conversion was attempted;
* `routing_model.kind` names the static routing assumption (top-1, top-2 +
  chosen tie-break, deterministic-once-per-token, etc.) used to derive
  `projected_bank_switches_per_token`;
* budget decisions compare switch caps against
  `projected_*_switches_per_token.decision_value`;
* `upper_bound` is always present even when a profile permits an expected-value
  decision;
* projection-section sizes may be zero when the corresponding resource is
  unused. Zero is valid only when the `ProjectedSizeSource` is explicit and the
  relevant target/runtime capacity is present by hash or excerpt.

Amends planv0: this RFC splits early static feasibility from later measured
budget reporting.

* `static_budget.json` is emitted by Stage 2 and contains only static
  necessary-condition projections.
* later `budget.json` is emitted by the reporting envelope after scheduling,
  layout, and measurements exist.
* fields that require schedule, emulator, hardware, or calibration measurement
  evidence must not appear in `static_budget.json`.

**Deferred:**

* Observed-vs-projected fields (those live in `budget.json` once the runtime has measurements per `planv0.md` line 2776).
* Cycle envelopes (F-B14 `schedule_cost.json`).

### 7.8 `StageCache` keys

For `gbf-store::StageCache` lookup:

| Stage  | Key inputs                                                                                                                                                                                                                                                       | Cached output                                                                                  |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| 0      | `artifact_source_hash`, `artifact_effective_core_hash?`, `artifact_manifest_hash?`, `artifact_aux_hash?`, `lowering_manifest_hash?`, `hint_bundle_hash`, `compile_request_hash`, `target_profile_hash`, `compile_profile_hash`, `calibration_hash?`, `compatibility_adapter_registry_hash`, `pass_version_validate`, `crate_feature_set_hash`, `artifact_validation_schema_hash` | success: `ValidationProduct`; failure memo: `artifact_validation.json` + diagnostics |
| 0.5    | `artifact_validation_self_hash`, `ValidatedInputHashes`, `target_defaults_hash`, `compile_profile_hash`, `profile_defaults_hash`, `compile_objective_hash`, `pass_version_resolve`, `crate_feature_set_hash`, `policy_resolution_schema_hash` | success: `ResolvedPolicyProduct` + report; failure memo: `policy_resolution.json` + diagnostics |
| 2      | `policy_resolution_self_hash`, `quant_graph_hash`, `runtime_chrome_budget_hash`, `target_profile_hash`, `pass_version_budget`, `crate_feature_set_hash`, `static_budget_schema_hash` | success: `StaticBudgetReport` + report; failure memo: `static_budget.json` + diagnostics |

Stage 0 cache lookup occurs after a cheap normalization prelude computes the
source artifact hash, schema header, possible compatibility-adapter identity,
and any identity hashes available without trusting unverified artifact content.
When a hash cannot be computed safely, the key records explicit `None`; it must
not use an all-zero sentinel as a fake valid hash.

`pass_version_*` is a per-stage version constant in `gbf-codegen` that bumps when the stage's logic changes; it forms part of the cache key so an old build's cached output cannot be served against a new pass implementation.

`crate_feature_set_hash` is the canonical hash of compile-time feature flags
that can affect type layout, serde shape, pass behavior, or validator behavior.

`report_schema_hash` changes whenever a report schema, canonicalization rule,
or semantic validator changes, even if pass logic is unchanged.

Per §2.6, the cache stores success outputs only as authoritative products and
may memoize failure diagnostics under exact input-hash match.

On a cache hit, `gbf-codegen` materializes the cached canonical report bytes
into the current build output unless the caller explicitly requested no
sidecar emission. A cache hit must preserve the cached `report_self_hash`; it
must not recompute or pretty-print the report.

### 7.9 Worked examples

The abstract types and rules in §7 become concrete when you trace a build
through the chunk. The five examples below are written at the level a reviewer
or implementer needs: each names the inputs, what each gate decides, and what
shows up in each report. All examples assume a tiny synthetic fixture (one
dense-int kernel, no MoE, DMG/MBC5 target).

#### 7.9.1 Successful Bringup compile (happy path)

Inputs:
* `ArtifactCore` with `semantic_core_hash = sha256:aaaa…` (canonical).
* `ArtifactManifest` whose recorded hash matches; declares
  `required_features = {DenseI8}`, schema epoch current.
* `ArtifactAux` minimal (one workload ref, one golden vector ref).
* `HintBundle` empty (no facts, no preferences, no constraints).
* `CompileRequest { target=DMG/MBC5, profile=Bringup, calibration=Some(BootstrapCalibrationBundle ref) }`.
* `TargetProfile` DMG/MBC5 default. Bringup reserved slack is expressed only by the selected `bringup-*.chrome_budget.json` input.
* `CalibrationBundleSet` resolves the bootstrap bundle; declared confidence `None`.
* `RuntimeChromeBudget` from `bringup-dmg-mbc5.chrome_budget.json` (tiny
  reserved_slack, one ExpertBank slot, one Bank0Free slot).
* `QuantGraphBudgetSource` synthetic impl yielding one dense expert.

Stage 0 walks classes 1–10 in order. Each passes:

```text
class 1 schema epoch                    ok
class 2 semantic core hash              ok (recomputed == manifest.recorded)
class 3 manifest invariants             ok (feature set ⊆ target capabilities)
class 4 artifact semantic payload       ok (canonical tensor present, LUT well-formed)
class 5 artifact aux sidecars           ok (golden vector resolves)
class 6 target-data lowering            ok (round-trip under packer_version)
class 7 calibration binding             ok (Bringup profile RiskPolicy::calibration_confidence_requirement == NoMinimumConfidence;
                                            BootstrapCalibrationBundle confidence == None ⇒ pass)
class 8 hint provenance                 ok (empty)
class 9 workload/golden refs            ok
class 10 compile-request admissibility  ok
```

Stage 0 emits `artifact_validation.json` with `outcome = Passed`, no Hard
diagnostics, full identity hashes. `report_self_hash` computed and round-tripped.

Stage 0.5 collects ConstraintFrames in order:

```text
frame 1 (TargetDefault):     seeds CompileKnobs.global, bounds, target_caps
frame 2 (ProfileDefault):    Bringup applies ObservabilityMode=Invariant, TraceBudget=minimal,
                             RepairPolicy={max_refinement_iters=1, all-false flags}
frames 3–4 (HintBundle):     no-op (empty)
frame 5 (CompileRequestOverride): no overrides
frame 6 (Calibration):       no calibration-driven knobs (bootstrap bundle has no measurements)
```

Monotonicity verified. Per-knob path-level provenance recorded. Emits
`policy_resolution.json` with `outcome = Passed`, `result = Some(…)`,
`hint_consumption = { all empty }`.

Stage 2:

* Calls `to_budget_view()` → validated `QuantGraphBudgetView`.
* Per-expert byte math: rows × cols / 4 (Ternary2) + scales + metadata.
* `StaticPlacementModel::BudgetedFirstFit` assigns the single expert to the
  first `ExpertBank` slot.
* Common bank footprint: just the kernel bytes.
* `projected_bank_switches_per_token = ProjectedSwitchCount { upper_bound: 1, expected_q16_16: None, decision_value: 1, source: ConservativeStaticUpperBound }`.
* `decision = { fits: true, interpretation: PassesNecessaryStaticChecks, placement_model: BudgetedFirstFit, failures: [] }`.
* Emits `static_budget.json` with `outcome = Passed`.

All three reports' `report_self_hash` round-trip. The chunk is closed.

#### 7.9.2 Stale calibration (Stage 0 hard fail)

Inputs same as 7.9.1 but the calibration bundle's recorded
`target_profile_hash` is for a CGB target, while the active build is DMG.

Stage 0 walks classes 1–6 successfully, then class 7 emits
`CalibrationStale { class: PlatformCalibrationLayer, declared:
sha256:bbbb…, observed: sha256:aaaa… }`. Class 7 does not short-circuit
classes 8–10 because they don't dereference unverified state, so Stage 0
also runs those (they pass independently).

Stage 0 emits `artifact_validation.json` with `outcome = Failed`, exactly one
Hard diagnostic (CalibrationStale), and full identity hashes. The
`compile_request_hash`, `target_profile_hash`, and `calibration_hash` are all
present so reviewers can diff. `report_self_hash` is stable.

Stage 0.5 and Stage 2 do **not** run. The build cannot close.

If a re-run happens with byte-identical inputs (same stale bundle), the
`StageCache` failure memo replays the same `artifact_validation.json` with
the original `report_self_hash` (per §2.6).

#### 7.9.3 Locked knob override (Stage 0.5 hard fail)

Inputs same as 7.9.1, but `CompileRequest.constraint_overrides` attempts to
set `KernelResidencyBias = PreferWramOverlay`. The Bringup profile spec locks
`RomKernelResidencyBias`.

Stage 0 passes (the override is well-typed; admissibility checks the *target*,
not the *profile-locked* set).

Stage 0.5 collects frames. When applying frame 5
(CompileRequestOverride), it sees the override against a locked knob.
Resolution emits `PolicyKnobLockedAndOverridden { knob: RomKernelResidencyBias }`.
`resolve_policy` returns `PolicyResolutionStageFailure` carrying a report
envelope with `outcome = Failed`, `result = None`, one Hard diagnostic.

Stage 0.5 emits `policy_resolution.json` with `outcome = Failed`. Stage 0's
`artifact_validation.json` is **not** mutated (per §2.1). Stage 2 does not
run.

#### 7.9.4 Expert exceeds bank (Stage 2 hard fail)

Inputs same as 7.9.1, but the synthetic expert's rows × cols implies more
weight bytes than the chrome budget's single ExpertBank slot can hold.

Stage 0 and Stage 0.5 succeed. Stage 2:

* Computes per-expert payload bytes via the byte-math formulas (§7.6).
* Under `StaticPlacementModel::StrictOnePerBank`, assignment fails: the only
  ExpertBank slot's `effective_cap_bytes < payload_bytes`.
* Records `BudgetFailure::ExpertExceedsSlot { layer: 0, expert: 0, slot: expert_slot_0, payload_bytes: 17000, cap_bytes: 16128, excess_bytes: 872 }`.
* `decision = { fits: false, interpretation: FailsNecessaryStaticChecks, placement_model: StrictOnePerBank, failures: [<the failure>] }`.
* Emits `static_budget.json` with `outcome = Failed`, full
  `runtime_chrome_budget` section present (since the budget *was* available,
  per §7.7's invariant).

The diagnostic carries the concrete byte counts so a future fix is mechanical:
either reduce the expert (training-side change) or change the chrome budget
(runtime-shell change).

#### 7.9.5 Missing chrome budget (Stage 2 hard fail with the only "no-budget" report)

Inputs same as 7.9.1 but `CompileInvocationInputs.runtime_chrome_budget == None`.
This is the head-of-line case where the runtime shell has not been built yet.

Stage 0 and Stage 0.5 succeed (neither requires the chrome budget).

Stage 2 immediately rejects with `BudgetMissingRuntimeChromeBudget` (Hard).
The emitted `static_budget.json` has:

* `outcome = Failed`,
* `identity.runtime_chrome_budget_hash = None`,
* `runtime_chrome_budget = None` (the only Stage-2 report shape that allows
  this — see §7.7's invariant binding it to outcome=Failed plus exactly one
  `BudgetMissingRuntimeChromeBudget` diagnostic and one
  `BudgetFailure::MissingRuntimeChromeBudget`),
* `decision.fits = false`,
* `decision.failures = [MissingRuntimeChromeBudget]`,
* `placement_model` recorded but no per-bank occupancy,
* exactly one Hard diagnostic.

This shape exists for a reason: it gives the runtime-shell team a stable
diff target while their build product becomes available. Once the shell ships
its first chrome budget, byte-identical reruns with the same fixture produce
a successful report; the failure memo in `StageCache` is invalidated by the
new `runtime_chrome_budget_hash` input.

## 8. Pass plan

Unlike F-B1's L0–L5 layer story, this chunk has no integration milestones. The pass plan is **per-stage**, with explicit acceptance gates at each stage.

### 8.1 Stage 0 — `ArtifactValidationAndUpgrade`

Purpose: head-of-pipeline schema/identity/calibration filter.

Acceptance gates (unit tests, in-process):

```text
cargo test -p gbf-codegen -- f_b2_validate_accepts_canonical_fixture
cargo test -p gbf-report -- f_b2_artifact_validation_v1_self_hash_round_trip
cargo test -p gbf-report -- f_b2_artifact_validation_v1_rejects_unknown_fields
cargo test -p gbf-codegen -- f_b2_validate_emits_failure_report_for_bad_schema
cargo test -p gbf-codegen -- f_b2_validate_accepts_lossless_in_memory_schema_adapter
cargo test -p gbf-codegen -- f_b2_validate_lossless_adapter_preserves_semantic_hash
cargo test -p gbf-codegen -- f_b2_validate_lossless_adapter_records_source_and_effective_hashes
cargo test -p gbf-codegen -- f_b2_validate_rejects_lossy_schema_adapter
cargo test -p gbf-codegen -- f_b2_validate_rejects_unregistered_schema_adapter
cargo test -p gbf-codegen -- f_b2_validate_rejects_schema_epoch_unsupported
cargo test -p gbf-codegen -- f_b2_validate_rejects_semantic_core_hash_mismatch
cargo test -p gbf-codegen -- f_b2_validate_rejects_manifest_invariant_violated
cargo test -p gbf-codegen -- f_b2_validate_rejects_lowering_round_trip_failure
cargo test -p gbf-codegen -- f_b2_validate_rejects_lowering_packer_version_mismatch
cargo test -p gbf-codegen -- f_b2_validate_rejects_calibration_stale
cargo test -p gbf-codegen -- f_b2_validate_rejects_missing_calibration
cargo test -p gbf-codegen -- f_b2_validate_accepts_bootstrap_calibration_when_profile_requires_none
cargo test -p gbf-codegen -- f_b2_validate_rejects_bootstrap_calibration_under_default_profile
cargo test -p gbf-codegen -- f_b2_validate_rejects_hint_provenance_inconsistent
cargo test -p gbf-codegen -- f_b2_validate_rejects_workload_ref_unresolved
cargo test -p gbf-codegen -- f_b2_validate_rejects_golden_vector_missing
cargo test -p gbf-codegen -- f_b2_validate_rejects_golden_vector_digest_mismatch
cargo test -p gbf-codegen -- f_b2_validate_rejects_artifact_payload_blob_digest_mismatch
cargo test -p gbf-codegen -- f_b2_validate_rejects_transport_manifest_mismatch
cargo test -p gbf-codegen -- f_b2_validate_rejects_compile_request_unsupported_feature
cargo test -p gbf-codegen -- f_b2_validate_rejects_compile_request_profile_forbids_objective
cargo test -p gbf-codegen -- f_b2_validate_returns_typed_validated_inputs_handle
cargo test -p gbf-codegen -- f_b2_validate_validated_inputs_cannot_be_constructed_outside_module
cargo test -p gbf-codegen -- f_b2_validate_records_canonical_input_hashes
cargo test -p gbf-codegen -- f_b2_validate_returns_all_diagnostics_in_one_pass
cargo test -p gbf-codegen -- f_b2_validate_short_circuits_when_continuing_is_unsafe
```

### 8.2 Stage 0.5 — `ResolvedCompilePolicy`

Purpose: resolve `CompileRequest` into `ResolvedCompilePolicy` with per-knob provenance and emit `policy_resolution.json`.

Acceptance gates:

```text
cargo test -p gbf-codegen -- f_b2_resolve_policy_target_defaults_seed_global
cargo test -p gbf-codegen -- f_b2_resolve_policy_profile_defaults_tighten_bounds
cargo test -p gbf-codegen -- f_b2_resolve_policy_hints_apply_within_bounds
cargo test -p gbf-codegen -- f_b2_resolve_policy_constraints_tighten_bounds
cargo test -p gbf-codegen -- f_b2_resolve_policy_overrides_only_tighten
cargo test -p gbf-codegen -- f_b2_resolve_policy_calibration_data_drives_pressure_thresholds
cargo test -p gbf-codegen -- f_b2_resolve_policy_rejects_locked_knob_override
cargo test -p gbf-codegen -- f_b2_resolve_policy_rejects_out_of_bounds_value
cargo test -p gbf-codegen -- f_b2_resolve_policy_rejects_unsatisfiable_bound_meet
cargo test -p gbf-codegen -- f_b2_resolve_policy_rejects_authorized_relaxation_operation
cargo test -p gbf-codegen -- f_b2_resolve_policy_records_per_knob_provenance
cargo test -p gbf-codegen -- f_b2_resolve_policy_records_path_level_provenance
cargo test -p gbf-codegen -- f_b2_resolve_policy_records_hint_consumption
cargo test -p gbf-codegen -- f_b2_resolve_policy_ignores_out_of_bounds_preferences_with_record
cargo test -p gbf-codegen -- f_b2_resolve_policy_no_repair_proposal_provenance_in_chunk
cargo test -p gbf-codegen -- f_b2_resolve_policy_failure_emits_policy_resolution_failure_report
cargo test -p gbf-codegen -- f_b2_resolve_policy_failure_does_not_mutate_artifact_validation_report
cargo test -p gbf-codegen -- f_b2_resolve_policy_no_profile_relaxation_field
cargo test -p gbf-report -- f_b2_policy_resolution_v1_schema_accepts_canonical_fixture
cargo test -p gbf-report -- f_b2_policy_resolution_v1_rejects_missing_required_fields
cargo test -p gbf-report -- f_b2_policy_resolution_v1_rejects_repair_proposal_provenance
cargo test -p gbf-report -- f_b2_policy_resolution_v1_self_hash_round_trip
cargo test -p gbf-report -- f_b2_policy_resolution_v1_failure_report_self_hash_round_trip
cargo test -p gbf-codegen -- f_b2_resolve_policy_emits_canonical_json
cargo test -p gbf-codegen -- f_b2_resolve_policy_is_deterministic_for_same_inputs
cargo test -p gbf-codegen -- f_b2_resolve_policy_stage_cache_key_is_stable
```

### 8.3 Stage 2 — `StaticBudgetReport`

Purpose: pre-lowering static fit check.

Acceptance gates:

```text
cargo test -p gbf-codegen -- f_b4_budget_accepts_canonical_fixture
cargo test -p gbf-codegen -- f_b4_budget_validates_quant_graph_budget_view_ordering
cargo test -p gbf-codegen -- f_b4_budget_computes_ternary2_payload_bytes_from_shape
cargo test -p gbf-codegen -- f_b4_budget_computes_scale_bytes_by_granularity
cargo test -p gbf-codegen -- f_b4_budget_uses_reserved_slack_in_effective_cap
cargo test -p gbf-codegen -- f_b4_budget_records_static_placement_model
cargo test -p gbf-codegen -- f_b4_budget_switch_decision_uses_upper_bound_by_default
cargo test -p gbf-codegen -- f_b4_budget_recovery_switch_decision_uses_upper_bound
cargo test -p gbf-codegen -- f_b4_budget_fits_means_necessary_checks_only
cargo test -p gbf-codegen -- f_b4_budget_rejects_missing_runtime_chrome_budget
cargo test -p gbf-codegen -- f_b4_budget_missing_runtime_chrome_budget_records_budget_failure
cargo test -p gbf-codegen -- f_b4_budget_missing_runtime_chrome_budget_emits_failure_report_without_budget_hash
cargo test -p gbf-codegen -- f_b4_budget_rejects_malformed_quant_graph_budget_view
cargo test -p gbf-codegen -- f_b4_budget_rejects_expert_exceeds_slot
cargo test -p gbf-codegen -- f_b4_budget_rejects_common_bank_exceeds_cap
cargo test -p gbf-codegen -- f_b4_budget_rejects_wram_peak_exceeds_cap
cargo test -p gbf-codegen -- f_b4_budget_rejects_sram_peak_exceeds_cap
cargo test -p gbf-codegen -- f_b4_budget_rejects_hram_peak_exceeds_cap
cargo test -p gbf-codegen -- f_b4_budget_rejects_accumulator_exceeds_i32
cargo test -p gbf-codegen -- f_b4_budget_rejects_switches_per_token_over_cap
cargo test -p gbf-codegen -- f_b4_budget_rejects_sram_page_switches_per_token_over_cap
cargo test -p gbf-codegen -- f_b4_budget_rejects_placement_profile_infeasible
cargo test -p gbf-codegen -- f_b4_budget_includes_shared_dense_ffn_in_common_bank_footprint
cargo test -p gbf-codegen -- f_b4_budget_per_expert_payload_covers_every_expert
cargo test -p gbf-codegen -- f_b4_budget_routing_model_named_in_report
cargo test -p gbf-codegen -- f_b4_budget_uses_quant_graph_budget_source_trait_stub_until_f_b3
cargo test -p gbf-report -- f_b4_static_budget_v1_schema_accepts_canonical_fixture
cargo test -p gbf-report -- f_b4_static_budget_v1_rejects_missing_required_fields
cargo test -p gbf-report -- f_b4_static_budget_v1_self_hash_round_trip
cargo test -p gbf-report -- f_b4_static_budget_v1_failure_report_self_hash_round_trip
cargo test -p gbf-report -- f_b4_static_budget_v1_rejects_float_values
cargo test -p gbf-codegen -- f_b4_budget_emits_canonical_json
cargo test -p gbf-codegen -- f_b4_budget_is_deterministic_for_same_inputs
cargo test -p gbf-codegen -- f_b4_budget_stage_cache_key_is_stable
```

### 8.4 Chunk-level integration

```text
cargo test -p gbf-codegen -- f_b2_f_b4_chunk_pipeline_runs_in_order
cargo test -p gbf-codegen -- f_b2_f_b4_chunk_failures_short_circuit_correctly
cargo test -p gbf-codegen -- f_b2_f_b4_chunk_reports_are_byte_identical_across_runs
cargo test -p gbf-codegen -- f_b2_f_b4_chunk_stage_cache_hit_materializes_cached_report
./scripts/review/f-b2-f-b4/regen.sh
./scripts/review/f-b2-f-b4/verify-packet.sh
```

### 8.5 Implementation order recommendation

The task graph in §12 is a DAG, but the *useful* implementation order trades
unblocking against thrashing. Recommended sequencing:

**Wave 0 — foundational schema absorption (parallel; Wave 1 cannot start
without it):**

These tasks introduce the type definitions that every later wave's "Inputs"
section relies on. Per §2.14, the chunk absorbs schema halves of upstream
stubs in `gbf-policy`, `gbf-artifact`, and `gbf-workload` because no other
feature owns them. Five of the seven are independent and parallel-able;
T-B2.0d depends on T-B2.0 (uses `CompileKnobId`, `ConstraintValue`); T-B2.0f
depends on T-B2.0c (uses `PackerVersion`).

a. T-B2.0  `gbf-policy` core schema (compile + objective + repair + budget).
b. T-B2.0a `gbf-artifact::manifest` schema (`ArtifactManifest`,
   `ManifestInvariant`, ...).
c. T-B2.0b `gbf-artifact::aux` schema (`ArtifactAux`, `SidecarKind`).
d. T-B2.0c `gbf-artifact::lowerings` schema (`TargetDataLoweringArtifact`,
   `PackerVersion`, ...).
e. T-B2.0d `gbf-artifact::HintBundle` assembly (`HintBundle`,
   `BuildConstraints`, `EvidenceScope`). Depends on T-B2.0.
f. T-B2.0e `gbf-workload::manifest` schema (`WorkloadManifestRef`,
   `WorkloadId`, ...).
g. T-B2.0f `gbf-policy::calibration` schema (`CalibrationBundleSet`,
   `CalibrationBundle`, `BootstrapCalibrationBundle` factory). Depends on T-B2.0c.

After Wave 0, every later Wave's "Inputs" section is satisfied at the type
level; Wave-1 work can compile against real `gbf-artifact` / `gbf-workload`
schema rather than module stubs.

**Wave 1 — chunk-shared infrastructure (no surprises here, but everything
else blocks on it):**

1. T-B2.1 `ValidationDiagnostic` taxonomy in `gbf-policy::diagnostics`. The
   closed `ValidationCode` enum is the canary; once stable, every later test
   names a code.
2. T-B2.2 `ReportEnvelope<R>` + canonical JSON + `compute_self_hash` +
   `round_trip_self_hash` in `gbf-report`. This is the fixed point that all
   three reports share.
3. T-B2.3 `artifact_validation.v1` schema + `ArtifactValidationReportBody` +
   semantic validator, plus the success/failure emission contract.

After Wave 1, schema work lands; the chunk has its first checked-in golden
shape.

**Wave 2 — Stage 0 (parallelizable internally):**

4. T-B2.4 `ValidateInputs` / `ValidatedInputs` / `ValidationProduct` (private
   constructor witness).
5. In parallel: T-B2.5 (schema/core/manifest), T-B2.6 (payload/aux),
   T-B2.7 (lowering round-trip), T-B2.8 (calibration), T-B2.9 (hint provenance
   + workload/golden refs), T-B2.10 (CompileRequest admissibility).
6. T-B2.11 ties these together: full diagnostic set in one pass, fixed
   ordering, accumulating within a class.

After Wave 2, Stage 0 is testable end-to-end with synthetic fixtures.

**Wave 3 — Stage 0.5 (depends on Wave 1+2 plus profile fixtures):**

7. T-B2.12 `CompileProfileSpec` fixtures for Bringup/Default/Trace/Recovery.
   These are checked-in TOML files that pin every bound, lock, and default.
   Without these, the merge resolver has no inputs to test.
8. T-B2.18 `BootstrapCalibrationBundle` + `bringup-*.chrome_budget.json`
   fixtures. (Pulled forward from "near the end" because Stage 0 calibration
   tests need them.)
9. T-B2.13 ConstraintFrame merge resolver. Implements the merge rules.
10. T-B2.14 path-level `ConstraintProvenance` recorder.
11. T-B2.15 lock/bound enforcement.
12. T-B2.16 forbid `RepairProposal(_)` provenance in this chunk.
13. T-B2.17 `HintConsumptionSection` wiring.
14. T-B2.19 `policy_resolution.v1` schema + semantic validator + tests.
15. T-B2.20 StageCache keys for Stage 0 and Stage 0.5 (success + failure
    memo).

After Wave 3, F-B2 is feature-complete. PR-1 of the chunk lands here.

**Wave 4 — Stage 2 (depends on Wave 3 close):**

16. T-B4.1 `QuantGraphBudgetSource` trait + `QuantGraphBudgetView` schema.
17. T-B4.2 per-expert byte math (with the canonical formulas from §7.6).
18. In parallel: T-B4.3 (per-bank occupancy under each placement model),
    T-B4.4 (common bank footprint), T-B4.5 (accumulator maxima),
    T-B4.6 (WRAM/SRAM/HRAM peaks), T-B4.7 (switch counts).
19. T-B4.8 deterministic static placement models.
20. T-B4.8a `StaticFitInterpretation` semantics (small task; a clarity gate).
21. T-B4.9 `BudgetFailure` taxonomy + diagnostics wiring.
22. T-B4.10 `RuntimeChromeBudget` binding, including the missing-budget hard
    reject.
23. T-B4.11 `static_budget.v1` schema + semantic validator + tests.
24. T-B4.12 StageCache key for Stage 2.

After Wave 4, F-B4 is feature-complete. PR-2 of the chunk lands here.

**Wave 5 — review packet (last):**

25. T-B2.21 F-B2 review-packet sub-bundle.
26. T-B4.13 F-B4 review-packet sub-bundle.

Reviewers see goldens for all three reports plus the failure goldens.
PR-3 (or appended to PR-2) lands the review packet, and the chunk closes.

Three-PR cadence (one per feature plus one for the review packet) is the
recommended default. A single bundled PR is acceptable if the diff stays
under ~3000 lines and review can keep up; for this chunk's expected size
(~5000 lines including tests and goldens), three is cleaner.

## 9. Report shapes

### 9.1 `policy_resolution.json` top-level

```json
{
  "schema": "policy_resolution.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed",
  "artifact_identity": {
    "artifact_core_hash": "sha256:...",
    "artifact_manifest_hash": "sha256:...",
    "semantic_lineage": "...",
    "lowering_manifest_hash": "sha256:...",
    "hint_bundle_hash": "sha256:...",
    "workload_refs": ["..."],
    "golden_vector_refs": ["..."]
  },
  "compile_request": {
    "compile_request_hash": "sha256:...",
    "target": "...",
    "target_profile_hash": "sha256:...",
    "profile": "Bringup",
    "objective": { "...": "..." },
    "required_features": ["..."],
    "requested_runtime_modes": ["Interactive"],
    "calibration_set_ref": { "kind": "Bootstrap", "id": "bootstrap-dmg-mbc5" },
    "calibration_hash": "sha256:..."
  },
  "result": {
    "resolved": {
      "effective_constraints": { "...": "..." },
      "observability": "Invariant",
      "trace_budget": { "...": "..." },
      "repair": { "...": "..." }
    },
    "compile_knobs": {
      "global":     { "placement": { "profile": "Budgeted" }, "...": "..." },
      "bounds":     { "max_placement_profile": "PackedExperts", "...": "..." },
      "locks":      { "locked": [] },
      "overrides":  { "...": "..." },
      "provenance": [
        {
          "path": { "knob": "PlacementProfile", "selector": null, "field": null },
          "chain": [
            {
              "source": "ProfileDefault",
              "operation": "SeedDefault",
              "evidence": [{ "kind": "ProfileFile", "ref": "Bringup.toml" }]
            }
          ]
        }
      ]
    },
    "provenance": {
      "target_defaults": "sha256:...",
      "profile_defaults": "sha256:...",
      "hint_bundle_hash": "sha256:...",
      "compile_request_hash": "sha256:...",
      "calibration_hash": "sha256:..."
    }
  },
  "hint_consumption": {
    "facts_used": [],
    "preferences_honored": [],
    "preferences_ignored": [],
    "constraints_enforced": []
  },
  "diagnostics": [],
  "report_self_hash": "sha256:..."
}
```

### 9.2 `static_budget.json` top-level

```json
{
  "schema": "static_budget.v1",
  "schema_version": "1.0.0",
  "outcome": "Passed",
  "identity": {
    "artifact_core_hash": "sha256:...",
    "quant_graph_hash": "sha256:...",
    "policy_resolution_self_hash": "sha256:...",
    "runtime_chrome_budget_hash": "sha256:...",
    "target_profile_hash": "sha256:..."
  },
  "policy": {
    "placement_profile": "Budgeted",
    "objective_hash": "sha256:..."
  },
  "runtime_chrome_budget": {
    "target": "...",
    "profile": "Bringup",
    "runtime_nucleus_hash": "sha256:...",
    "rom_slots": [
      { "id": "expert_slot_0", "class": "ExpertBank", "usable_bytes": 16384,
        "reserved_slack": 256, "placement_caps": ["StrictOnePerBank", "Budgeted"] }
    ],
    "memory_caps": {
      "wram_usable_bytes": 8192,
      "sram_usable_bytes": 32768,
      "hram_usable_bytes": 127,
      "source_target_profile_hash": "sha256:..."
    },
    "wram_reserved": 0,
    "sram_reserved": 0
  },
  "projections": {
    "per_expert_payload": [],
    "per_bank_occupancy": [],
    "common_bank_footprint": {
      "kernel_bytes": 0,
      "lut_bytes": 0,
      "shared_dense_ffn_bytes": null,
      "aggregate_bytes": 0
    },
    "accumulator_maxima": [],
    "projected_wram": { "peak_bytes": 0, "source": "StaticGraphProjection" },
    "projected_sram": { "peak_bytes": 0, "source": "StaticGraphProjection" },
    "projected_hram": { "peak_bytes": 0, "source": "StaticGraphProjection" },
    "projected_bank_switches_per_token": {
      "upper_bound": 0,
      "expected_q16_16": null,
      "decision_value": 0,
      "source": "ConservativeStaticUpperBound"
    },
    "projected_sram_page_switches_per_token": {
      "upper_bound": 0,
      "expected_q16_16": null,
      "decision_value": 0,
      "source": "ConservativeStaticUpperBound"
    },
    "routing_model": { "kind": "Top1Deterministic" }
  },
  "decision": {
    "fits": true,
    "interpretation": "PassesNecessaryStaticChecks",
    "placement_model": "BudgetedFirstFit",
    "failures": []
  },
  "diagnostics": [],
  "report_self_hash": "sha256:..."
}
```

### 9.3 Canonical JSON, self-hash, determinism

* Canonical JSON rule per §2.5.
* Self-hash rule per §2.4.
* All three reports must regenerate byte-identical output across two consecutive runs on a clean checkout.
* All three reports use `gbf-report::canonical_json` and `gbf-report::compute_self_hash`. No per-stage canonicalizer.
* Floating-point fields are forbidden in `policy_resolution.json`,
  `artifact_validation.json`, and `static_budget.json` v1.

Canonical ordering:

| Field family            | Order                                                                          |
| ----------------------- | ------------------------------------------------------------------------------ |
| diagnostics             | validation order, then origin, then code, then primary selector/provenance     |
| workload refs           | `WorkloadId` ascending                                                         |
| golden vector refs      | `GoldenVectorId` ascending                                                     |
| compile knob provenance | `CompileKnobPath` ascending                                                    |
| per expert payload      | `(LayerId, ExpertId)` ascending                                                |
| per bank occupancy      | `BudgetSlotId` ascending                                                       |
| assigned components     | `(component_class, layer, expert, component_id)` ascending                     |
| accumulator maxima      | `ReductionSiteId` ascending                                                    |
| budget failures         | failure class, then primary ids ascending                                      |
| hint consumption        | source order from canonical hint bundle, with stable tie-breaker by selector  |

No report may depend on `HashMap` iteration order.

## 10. Validation strategy

### 10.1 Schema correctness

Claims:

* every report shape has a Rust type with `serde::Serialize + serde::Deserialize`;
* round-tripping a canonicalized report through `serde_json` produces byte-identical output;
* `report_self_hash` is computed after canonicalization with the field zero-sentinel;
* unknown variants in any enum field are hard-rejected by the deserializer.
* F-B2/F-B4 reports reject `DiagnosticSeverity::Soft`.
* nullable fields are allowed only where explicitly enumerated.

Gates:

```text
gbf-report::policy_resolution_v1_round_trip
gbf-report::static_budget_v1_round_trip
gbf-report::policy_resolution_v1_rejects_unknown_policy_source
gbf-report::policy_resolution_v1_rejects_unknown_compile_knob_id
gbf-report::static_budget_v1_rejects_unknown_budget_failure
gbf-report::canonical_json_emitter_is_byte_stable
gbf-report::canonical_json_rejects_float_values_in_f_b2_f_b4_reports
gbf-report::f_b2_f_b4_reports_reject_soft_diagnostics
gbf-report::f_b2_f_b4_reports_reject_unlisted_null_fields
gbf-report::self_hash_round_trip
```

### 10.2 Stage 0 correctness

Claims:

* every reject class in §7.3 is detected;
* successful inputs produce a `ValidatedInputs` handle whose constructor is private;
* `ValidatedInputs::input_hashes` is canonical;
* successful inputs produce no `Hard` diagnostics;
* `Bringup` profile + `BootstrapCalibrationBundle` (declared `CalibrationConfidenceClass::None`) is accepted; `Default`/`Trace`/`Recovery` reject it via `CalibrationConfidenceTooLow`;
* `Default` profile + missing calibration produces `Hard`;
* missing calibration never produces a passing `ValidatedInputs` handle;
* Stage 0 returns *all* diagnostics for a class before short-circuiting.

Gates: see §8.1.

### 10.3 Stage 0.5 correctness

Claims:

* defaults seed the global knob set;
* profile defaults tighten bounds without exceeding target capabilities;
* `HintBundle::preferences` are applied only inside bounds;
* `HintBundle::constraints` tighten bounds;
* `CompileRequest::constraint_overrides` cannot loosen bounds;
* `CalibrationBundle` data may set pressure thresholds;
* per-knob `ConstraintProvenance` records the value's source;
* a locked knob with an override is hard-rejected;
* an out-of-bounds value is hard-rejected;
* an unsatisfiable bound meet is hard-rejected with `PolicyConstraintUnsatisfiable`;
* `RepairProposal(_)` provenance values are unreachable in this chunk.
* `ConstraintOperation::AuthorizedRelaxation` is unreachable in this chunk.

Gates: see §8.2.

### 10.4 Stage 2 correctness

Claims:

* per-expert payload includes every expert in the QuantGraph;
* per-bank occupancy includes every slot in the `RuntimeChromeBudget`;
* `common_bank_footprint.aggregate_bytes` equals the sum of components;
* `fits` matches `failures.is_empty()`;
* projected sizes use a documented source (`StaticGraphProjection`, `HintBundleConstraint`, or `CalibrationSamplingClosedForm`);
* `routing_model` is named in the report;
* missing `RuntimeChromeBudget` is a hard reject and records both
  `BudgetMissingRuntimeChromeBudget` and `BudgetFailure::MissingRuntimeChromeBudget`;
* malformed `QuantGraphBudgetView` is a hard reject with `BudgetQuantGraphViewMalformed`;
* tests run against synthetic `QuantGraphBudgetSource` impls; F-B3 lands the real impl later.

Gates: see §8.3.

### 10.5 StageCache correctness

Claims:

* StageCache key construction is deterministic;
* same inputs produce same key;
* a single byte change in any input produces a different key;
* `pass_version_*` participates in the key;
* failed passes do not enter the success-product cache;
* failure memos may enter the diagnostic cache only under exact key match;

Gates:

```text
gbf-codegen::stage_cache_key_validate_is_deterministic
gbf-codegen::stage_cache_key_validate_changes_with_inputs
gbf-codegen::stage_cache_key_validate_changes_with_pass_version
gbf-codegen::stage_cache_key_resolve_policy_is_deterministic
gbf-codegen::stage_cache_key_budget_is_deterministic
gbf-codegen::stage_cache_failed_pass_does_not_enter_success_cache
gbf-codegen::stage_cache_failure_memo_replays_only_on_exact_input_match
gbf-codegen::stage_cache_validate_allows_partial_failure_key_without_fake_hashes
gbf-codegen::stage_cache_hit_materializes_cached_report_bytes
```

### 10.6 Chunk-level integration

Claims:

* the chunk runs in order: validate → resolve → (Stage 1 stub or real) → budget;
* failures short-circuit correctly;
* successful runs emit byte-identical reports across two regenerations;
* `verify-packet.sh` rejects a stale checked-in report;
* `regen.sh` succeeds on a clean checkout.

Gates: see §8.4.

## 11. Claim-to-gate matrix

| Claim                                                                       | Gate                                                                                  |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Stage 0 rejects unsupported schema epoch                                    | `gbf-codegen::f_b2_validate_rejects_schema_epoch_unsupported`                         |
| Stage 0 rejects mismatched semantic core hash                               | `gbf-codegen::f_b2_validate_rejects_semantic_core_hash_mismatch`                      |
| Stage 0 rejects manifest invariant violations                               | `gbf-codegen::f_b2_validate_rejects_manifest_invariant_violated`                      |
| Stage 0 rejects packer round-trip failure                                   | `gbf-codegen::f_b2_validate_rejects_lowering_round_trip_failure`                      |
| Stage 0 rejects packer version mismatch                                     | `gbf-codegen::f_b2_validate_rejects_lowering_packer_version_mismatch`                 |
| Stage 0 rejects stale calibration                                           | `gbf-codegen::f_b2_validate_rejects_calibration_stale`                                |
| Stage 0 rejects missing calibration                                         | `gbf-codegen::f_b2_validate_rejects_missing_calibration`                              |
| Stage 0 accepts BootstrapCalibrationBundle when profile requires None       | `gbf-codegen::f_b2_validate_accepts_bootstrap_calibration_when_profile_requires_none` |
| Stage 0 rejects BootstrapCalibrationBundle under Default profile            | `gbf-codegen::f_b2_validate_rejects_bootstrap_calibration_under_default_profile`      |
| Stage 0 rejects unsupported features                                        | `gbf-codegen::f_b2_validate_rejects_compile_request_unsupported_feature`              |
| Stage 0 records canonical input hashes                                      | `gbf-codegen::f_b2_validate_records_canonical_input_hashes`                           |
| Stage 0 returns all diagnostics in one pass                                 | `gbf-codegen::f_b2_validate_returns_all_diagnostics_in_one_pass`                      |
| Stage 0.5 rejects locked-knob overrides                                     | `gbf-codegen::f_b2_resolve_policy_rejects_locked_knob_override`                       |
| Stage 0.5 rejects out-of-bounds knob values                                 | `gbf-codegen::f_b2_resolve_policy_rejects_out_of_bounds_value`                        |
| Stage 0.5 rejects unsatisfiable bound meet                                  | `gbf-codegen::f_b2_resolve_policy_rejects_unsatisfiable_bound_meet`                   |
| Stage 0.5 records per-knob provenance                                       | `gbf-codegen::f_b2_resolve_policy_records_per_knob_provenance`                        |
| Stage 0.5 forbids `RepairProposal` provenance                               | `gbf-codegen::f_b2_resolve_policy_no_repair_proposal_provenance_in_chunk`             |
| Stage 0.5 forbids authorized relaxation operations                          | `gbf-codegen::f_b2_resolve_policy_rejects_authorized_relaxation_operation`            |
| Stage 0.5 has no profile relaxation field                                   | `gbf-codegen::f_b2_resolve_policy_no_profile_relaxation_field`                        |
| `policy_resolution.v1` round-trips                                          | `gbf-report::f_b2_policy_resolution_v1_self_hash_round_trip`                          |
| `policy_resolution.v1` rejects RepairProposal provenance                    | `gbf-report::f_b2_policy_resolution_v1_rejects_repair_proposal_provenance`            |
| Stage 2 rejects missing RuntimeChromeBudget                                 | `gbf-codegen::f_b4_budget_rejects_missing_runtime_chrome_budget`                      |
| Stage 2 missing RuntimeChromeBudget records matching budget failure         | `gbf-codegen::f_b4_budget_missing_runtime_chrome_budget_records_budget_failure`        |
| Stage 2 rejects malformed QuantGraphBudgetView                              | `gbf-codegen::f_b4_budget_rejects_malformed_quant_graph_budget_view`                  |
| Stage 2 rejects expert exceeding slot                                       | `gbf-codegen::f_b4_budget_rejects_expert_exceeds_slot`                                |
| Stage 2 rejects common bank exceeding cap                                   | `gbf-codegen::f_b4_budget_rejects_common_bank_exceeds_cap`                            |
| Stage 2 rejects WRAM peak overflow                                          | `gbf-codegen::f_b4_budget_rejects_wram_peak_exceeds_cap`                              |
| Stage 2 rejects accumulator overflow                                        | `gbf-codegen::f_b4_budget_rejects_accumulator_exceeds_i32`                            |
| Stage 2 rejects bank-switches-per-token over cap                            | `gbf-codegen::f_b4_budget_rejects_switches_per_token_over_cap`                        |
| Stage 2 rejects placement-profile infeasibility                             | `gbf-codegen::f_b4_budget_rejects_placement_profile_infeasible`                       |
| Stage 2 includes shared dense FFN in common bank footprint                  | `gbf-codegen::f_b4_budget_includes_shared_dense_ffn_in_common_bank_footprint`         |
| Stage 2 covers every expert                                                 | `gbf-codegen::f_b4_budget_per_expert_payload_covers_every_expert`                     |
| Stage 2 names routing model in report                                       | `gbf-codegen::f_b4_budget_routing_model_named_in_report`                              |
| Stage 2 Recovery profile uses upper-bound switch decisions in v1            | `gbf-codegen::f_b4_budget_recovery_switch_decision_uses_upper_bound`                  |
| Stage 2 consumes the QuantGraphBudgetSource trait until F-B3 lands          | `gbf-codegen::f_b4_budget_uses_quant_graph_budget_source_trait_stub_until_f_b3`       |
| `static_budget.v1` round-trips                                              | `gbf-report::f_b4_static_budget_v1_self_hash_round_trip`                              |
| StageCache keys are deterministic                                           | `gbf-codegen::stage_cache_key_*_is_deterministic`                                     |
| Failed passes do not enter cache                                            | `gbf-codegen::stage_cache_failed_pass_does_not_enter_success_cache`                   |
| Chunk runs in order                                                         | `gbf-codegen::f_b2_f_b4_chunk_pipeline_runs_in_order`                                 |
| Reports are byte-identical across runs                                      | `gbf-codegen::f_b2_f_b4_chunk_reports_are_byte_identical_across_runs`                 |
| Review packet regenerates cleanly                                           | `./scripts/review/f-b2-f-b4/regen.sh` then clean diff                                 |
| Review packet verifier rejects staleness                                    | `./scripts/review/f-b2-f-b4/verify-packet.sh`                                         |

## 12. Task graph

```text
Chunk 1: Pipeline entry & validation
├── F-B2 ArtifactValidationAndUpgrade + ResolvedCompilePolicy   (bd-2fj)
│   ├── T-B2.0  gbf-policy core schema (compile + objective + repair + budget)
│   ├── T-B2.0a gbf-artifact::manifest schema (ArtifactManifest, ManifestInvariant)
│   ├── T-B2.0b gbf-artifact::aux schema (ArtifactAux, SidecarKind)
│   ├── T-B2.0c gbf-artifact::lowerings schema (TargetDataLoweringArtifact, PackerVersion)
│   ├── T-B2.0d gbf-artifact::HintBundle assembly (HintBundle, BuildConstraints, EvidenceScope)
│   ├── T-B2.0e gbf-workload::manifest schema (WorkloadManifestRef, WorkloadId)
│   ├── T-B2.0f gbf-policy::calibration schema (CalibrationBundleSet, CalibrationBundle, BootstrapCalibrationBundle)
│   ├── T-B2.1 Shared diagnostic taxonomy in gbf-policy::diagnostics
│   ├── T-B2.2 ReportEnvelope + canonical JSON + self-hash in gbf-report
│   ├── T-B2.3 artifact_validation.v1 schema + success/failure report emission
│   ├── T-B2.4 ValidateInputs / ValidatedInputsProduct (private constructor)
│   ├── T-B2.5 Stage 0 schema compatibility + semantic-core-hash + manifest invariants
│   ├── T-B2.6 Stage 0 payload/aux sidecar validation
│   ├── T-B2.7 Stage 0 lowering round-trip + packer version
│   ├── T-B2.8 Stage 0 calibration binding (resolution + freshness + confidence gate, all Hard)
│   ├── T-B2.9 Stage 0 hint-bundle provenance + workload/golden refs
│   ├── T-B2.10 Stage 0 CompileRequest admissibility
│   ├── T-B2.11 Stage 0 returns full diagnostic set in one pass
│   ├── T-B2.12 CompileProfileSpec fixtures for Bringup/Default/Trace/Recovery
│   ├── T-B2.13 Stage 0.5 constraint-frame merge resolver
│   ├── T-B2.14 Stage 0.5 path-level ConstraintProvenance recorder
│   ├── T-B2.15 Stage 0.5 lock/bound enforcement
│   ├── T-B2.16 Stage 0.5 forbid RepairProposal provenance in chunk
│   ├── T-B2.17 HintConsumptionSection wiring
│   ├── T-B2.18 BootstrapCalibrationBundle fixture + bringup-*.chrome_budget.json fixture
│   ├── T-B2.19 policy_resolution.v1 schema + semantic validator + tests
│   ├── T-B2.20 StageCache success + failure-memo keys for Stage 0 and Stage 0.5
│   └── T-B2.21 F-B2 review-packet sub-bundle
└── F-B4 StaticBudgetReport                                    (bd-2ps)
    ├── T-B4.1 QuantGraphBudgetView schema in gbf-codegen
    ├── T-B4.2 Per-expert payload byte math from shape + TernaryWeightPlan
    ├── T-B4.3 Per-bank occupancy + slot assignment
    ├── T-B4.4 Common bank footprint (kernels, LUTs, shared dense FFN)
    ├── T-B4.5 Accumulator maxima projection (i16/i32 safety)
    ├── T-B4.6 Projected WRAM/SRAM/HRAM peaks
    ├── T-B4.7 Projected bank-switch + SRAM-page-switch upper bounds and optional expected values
    ├── T-B4.8 Deterministic static placement models by PlacementProfile
    ├── T-B4.8a Static-fit interpretation and necessary-condition decision semantics
    ├── T-B4.9 BudgetFailure taxonomy + diagnostics wiring
    ├── T-B4.10 RuntimeChromeBudget binding + missing-budget hard reject
    ├── T-B4.11 static_budget.v1 schema + semantic validator + tests
    ├── T-B4.12 StageCache key for Stage 2
    └── T-B4.13 F-B4 review-packet sub-bundle
```

Cross-feature ordering:

* T-B2.1 and T-B2.2 are prerequisites for every other task in the chunk.
* T-B4.* depends on T-B2.13 (`ResolvedPolicyProduct`) and T-B4.1 (`QuantGraphBudgetSource` trait + `QuantGraphBudgetView` schema).
* T-B4 unit tests use synthetic `QuantGraphBudgetSource` impls. F-B3 (Stage 1) lands the real impl later.
* The chunk's merged PR sequence is F-B2 first, F-B4 second. F-B4 must not be merged before F-B2.

### 12.1 Bead inventory

Every task in §12 has a corresponding bead in `.beads/issues.jsonl`. The
authoritative mapping (preserved on disk for `future-self` to query via
`br show <id>`):

| Task     | Bead    | Title (short)                                                       |
| -------- | ------- | ------------------------------------------------------------------- |
| F-B2     | bd-2fj  | F-B2: ArtifactValidationAndUpgrade + ResolvedCompilePolicy           |
| F-B4     | bd-2ps  | F-B4: StaticBudgetReport (Stage 2)                                  |
| T-B2.0   | bd-558z | gbf-policy core schema (compile + objective + repair + budget)       |
| T-B2.0a  | bd-1bkx | gbf-artifact::manifest schema (ArtifactManifest, ManifestInvariant)  |
| T-B2.0b  | bd-39xv | gbf-artifact::aux schema (ArtifactAux, SidecarKind)                  |
| T-B2.0c  | bd-34zu | gbf-artifact::lowerings schema (TargetDataLoweringArtifact, PackerVersion) |
| T-B2.0d  | bd-3p3l | gbf-artifact::HintBundle assembly (HintBundle, BuildConstraints, EvidenceScope) |
| T-B2.0e  | bd-2o2c | gbf-workload::manifest schema (WorkloadManifestRef, WorkloadId)      |
| T-B2.0f  | bd-2sab | gbf-policy::calibration schema (CalibrationBundleSet, CalibrationBundle, BootstrapCalibrationBundle) |
| T-B2.1   | bd-ulvb | Shared diagnostic taxonomy in gbf-policy::diagnostics                |
| T-B2.2   | bd-2xlj | ReportEnvelope + canonical JSON + self-hash in gbf-report            |
| T-B2.3   | bd-1dvs | artifact_validation.v1 schema + success/failure report emission      |
| T-B2.4   | bd-3j65 | ValidateInputs / ValidatedInputs / ValidationProduct                 |
| T-B2.5   | bd-18ut | Stage 0 schema compatibility + semantic-core-hash + manifest invs    |
| T-B2.6   | bd-2ipf | Stage 0 payload/aux sidecar validation                               |
| T-B2.7   | bd-2n01 | Stage 0 lowering round-trip + packer version                         |
| T-B2.8   | bd-6zi1 | Stage 0 calibration binding (resolution + freshness + confidence)    |
| T-B2.9   | bd-2bo9 | Stage 0 hint-bundle provenance + workload/golden refs                |
| T-B2.10  | bd-2mfr | Stage 0 CompileRequest admissibility                                 |
| T-B2.11  | bd-26zc | Stage 0 returns full diagnostic set in one pass                      |
| T-B2.12  | bd-3mqn | CompileProfileSpec fixtures for Bringup/Default/Trace/Recovery       |
| T-B2.13  | bd-1hob | Stage 0.5 constraint-frame merge resolver                            |
| T-B2.14  | bd-3kj1 | Stage 0.5 path-level ConstraintProvenance recorder                   |
| T-B2.15  | bd-ja8r | Stage 0.5 lock/bound enforcement                                     |
| T-B2.16  | bd-106k | Stage 0.5 forbid RepairProposal provenance in chunk                  |
| T-B2.17  | bd-3sjj | HintConsumptionSection wiring                                        |
| T-B2.18  | bd-19by | BootstrapCalibrationBundle + bringup chrome-budget fixtures          |
| T-B2.19  | bd-2aua | policy_resolution.v1 schema + semantic validator + tests             |
| T-B2.20  | bd-30ul | StageCache success + failure-memo keys for Stage 0 and Stage 0.5     |
| T-B2.21  | bd-2uvs | F-B2 review-packet sub-bundle                                        |
| T-B4.1   | bd-gv2w | QuantGraphBudgetSource trait + QuantGraphBudgetView schema           |
| T-B4.2   | bd-25r0 | Per-expert payload byte math (Ternary2/Binary1/SparseTernaryBitplanes) |
| T-B4.3   | bd-1bax | Per-bank occupancy + slot assignment                                 |
| T-B4.4   | bd-gt5g | Common bank footprint (kernels, LUTs, shared dense FFN)              |
| T-B4.5   | bd-29m5 | Accumulator maxima projection (i16/i32 safety)                       |
| T-B4.6   | bd-ywh0 | Projected WRAM/SRAM/HRAM peaks                                       |
| T-B4.7   | bd-1fbz | Projected bank-switch + SRAM-page-switch upper bounds                |
| T-B4.8   | bd-3cfz | Deterministic static placement models by PlacementProfile            |
| T-B4.8a  | bd-2edz | Static-fit interpretation and necessary-condition decision semantics |
| T-B4.9   | bd-1rf5 | BudgetFailure taxonomy + diagnostics wiring                          |
| T-B4.10  | bd-23vr | RuntimeChromeBudget binding + missing-budget hard reject             |
| T-B4.11  | bd-3euv | static_budget.v1 schema + semantic validator + tests                 |
| T-B4.12  | bd-jmns | StageCache key for Stage 2                                           |
| T-B4.13  | bd-3fug | F-B4 review-packet sub-bundle + chunk regen/verify scripts           |

Each bead's description carries the full self-documenting context (purpose,
rationale, inputs, outputs, pitfalls, acceptance gates, references). The
chunk-level label `f-b2-f-b4,chunk1` selects the entire set; per-wave labels
(`wave1`, `wave2`, `wave3`, `wave4`, `wave5`) match the §8.5 implementation
order.

`br ready` shows the unblocked work. After F-B2 closes, F-B4's interior
opens; after F-B4 closes, the review-packet wave (T-B2.21, T-B4.13) is
ready.

## 13. Review packet

Required path:

```text
docs/review/f-b2-f-b4/
```

Required files:

```text
scope.md
architecture.md
claim-to-gate.md
reviewer-checklist.md
known-debt.md
reproducibility.md
generated-artifacts.md
artifacts/artifact_validation.golden.json
artifacts/policy_resolution.golden.json
artifacts/static_budget.golden.json
artifacts/artifact_validation.failure.golden.json
artifacts/policy_resolution.failure.golden.json
artifacts/static_budget.failure.golden.json
artifacts/artifact_validation.fixture.toml
artifacts/policy_resolution.fixture.toml
artifacts/static_budget.fixture.toml
artifacts/artifact_validation.golden.sha256
artifacts/policy_resolution.golden.sha256
artifacts/static_budget.golden.sha256
artifacts/artifact_validation.failure.golden.sha256
artifacts/policy_resolution.failure.golden.sha256
artifacts/static_budget.failure.golden.sha256
```

Required scripts:

```text
scripts/review/f-b2-f-b4/regen.sh
scripts/review/f-b2-f-b4/verify-packet.sh
```

Hard rule (lifted from F-B1 §14):

> A fresh checkout regenerates the packet with one command, and staleness fails loudly.

The review packet is markedly smaller than F-B1's because no emulator artifacts
are produced. The checked-in goldens are the JSON outputs against fixed
synthetic success and failure fixtures: they exist to give reviewers a stable
diff target when the schema, diagnostic taxonomy, or resolution logic changes.

The fixture inputs (`*.fixture.toml`) are deterministic and small (a single dense-int kernel under `Bringup`, a single experts-disabled topology, no MoE, a synthetic `bringup-*.chrome_budget.json` with one Bank0Free slot and one ExpertBank slot, and a synthetic `BootstrapCalibrationBundle` with `CalibrationConfidenceClass::None`). The fixture is intentionally tiny so reviewers can read the report in full.

`regen.sh`:

1. cleans `docs/review/f-b2-f-b4/artifacts/` of generated reports;
2. runs the fixture through Stage 0 → Stage 0.5 → synthetic
   `QuantGraphBudgetView` → Stage 2;
3. canonicalizes `artifact_validation.json`, `policy_resolution.json`, and
   `static_budget.json`;
4. writes them and their SHA-256 sidecars into `artifacts/`;
5. exits non-zero if any step fails.

`verify-packet.sh`:

1. recomputes all three success reports from the fixture;
2. compares byte-for-byte against the checked-in goldens;
3. validates each golden against its semantic validator;
4. validates each golden's `report_self_hash` round-trip;
5. runs intentionally failing Stage 0, Stage 0.5, and Stage 2 fixtures;
6. compares their failure reports byte-for-byte against the checked-in failure
   goldens;
7. exits non-zero on any mismatch.

`cargo test` runs the unit tests but does not run the regen scripts. CI runs `verify-packet.sh` on every PR.

### 13.1 Review packet markdown contents

Each `.md` file in `docs/review/f-b2-f-b4/` answers a specific reviewer
question and follows the same shape as F-B1's review packet. The section
checklists below are the minimum content; richer is fine, but each section
must exist and be non-empty.

**`scope.md`** — what is and isn't in this PR sequence.

* What this chunk delivers (one paragraph from §0 TL;DR).
* What it intentionally defers (cross-link to §4 non-goals).
* The three reports and what each one answers.
* Two checked-in feature beads (bd-2fj, bd-2ps) and their relationship to
  Epic B (bd-2bw).

**`architecture.md`** — how the three stages fit together.

* The chunk's slot in the headline pipeline (cross-link to §6).
* Visual or textual data flow: `ImportedArtifactView → validate → resolve_policy → QuantGraphBudgetSource → budget`.
* Where the code lives (cross-link to §2.12) — exact crate/module paths.
* The `ValidatedInputs`/`ResolvedPolicyProduct` private-constructor token
  pattern, so reviewers know the identity discipline is enforced by the type
  system.
* Where the `BootstrapCalibrationBundle` and `bringup-*.chrome_budget.json`
  fixtures live and how Bringup builds reference them.

**`claim-to-gate.md`** — every load-bearing claim mapped to its test.

* Reproduce the §11 claim-to-gate matrix verbatim.
* For each claim, link to the test file and a short prose explanation of
  what the assertion proves.
* List any claims that are checked only by review-packet scripts
  (e.g. byte-identical regeneration).

**`reviewer-checklist.md`** — what a reviewer should manually inspect.

* Open `policy_resolution.golden.json` and check: every `compile_knobs`
  field has a non-empty provenance chain. No `RepairProposal(_)` source.
* Open `static_budget.golden.json` and check: every expert appears in
  `per_expert_payload`; every slot in the runtime chrome budget appears in
  `per_bank_occupancy`; `decision.fits` matches `failures.is_empty()`.
* Open all three `.failure.golden.json`s and check: each has exactly one
  expected Hard diagnostic, `outcome = Failed`, and stable `report_self_hash`.
* Run `verify-packet.sh` and confirm clean exit + clean diff.
* Diff `policy_resolution.golden.json` against `policy_resolution.failure.golden.json`
  to confirm the failure shape preserves identity sections (so reviewers can
  diagnose without re-running).

**`known-debt.md`** — what we know is incomplete and where it goes next.

* `RepairProposal(_)` provenance values: forbidden in this chunk; F-B16
  unblocks them.
* `QuantGraphBudgetSource` real impl: F-B3 owns it.
* `RuntimeChromeBudget` production: lives in the runtime-shell build, not
  here.
* `gbf-migrate` (lossy migration): deferred to F-A6b.
* Cycle envelopes in budgets: F-B14 owns `schedule_cost.json`.
* Soft diagnostics: not used in this chunk; the variant exists for downstream.

**`reproducibility.md`** — the deterministic-build story.

* The canonical JSON rule (cross-link to §2.5).
* The self-hash convention (cross-link to §2.4).
* The domain-separated object hash form (`gbf:<crate>:<type>:<schema>:<ver>\0…`).
* What changes when `crate_feature_set_hash` changes; what changes when
  `*_schema_hash` changes; how the StageCache key isolates them.
* The path through `regen.sh` to verify clean diff.
* Note: floating-point fields are forbidden in v1 reports; if a future bump
  adds them, the canonical formatter rule must be specified before the bump
  lands.

**`generated-artifacts.md`** — what each artifact under `artifacts/` is.

* `artifact_validation.golden.json` — Stage 0 success report against the
  canonical synthetic fixture.
* `policy_resolution.golden.json` — Stage 0.5 success report.
* `static_budget.golden.json` — Stage 2 success report.
* `*.failure.golden.json` — paired failure reports for each stage's primary
  failure mode.
* `*.fixture.toml` — the synthetic input fixture for the success path and
  each failure path.
* `*.golden.sha256` — SHA-256 of the corresponding `.json`, used by
  `verify-packet.sh` for fast staleness detection.

**`bead-map.md`** — task → bead ID lookup table.

* Each task `T-B2.X` and `T-B4.Y` paired with its bead ID.
* The chunk-level features (bd-2fj, bd-2ps).
* The Epic parent (bd-2bw).
* Closure status as of merge.

This file is a small but important reproducibility aid: future readers can
walk the JSONL log and link work back to the design.

## 14. Risks and what we want to learn

### 14.1 Risk table

| Risk                                                                                           | Mitigation                                                                                                |
| ---------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| F-B3 (`QuantGraph`) lands later than F-B4                                                      | F-B4 consumes a `QuantGraphBudgetSource` trait that yields a validated `QuantGraphBudgetView`; F-B3 implements the trait later. Tests against synthetic impls. |
| `RuntimeChromeBudget` source-of-truth not yet baselined for M1                                 | Hard-reject `BudgetMissingRuntimeChromeBudget`; document where the budget comes from in `architecture.md` |
| F-B16 (`FeasibilityRefinementLoop`) lands later than F-B2                                      | F-B2 emits the `compile_knobs` section but forbids `RepairProposal(_)` provenance values                  |
| `AuthorizedRelaxation` sneaks back into Stage 0.5                                               | F-B2 has no `AuthorizedRelaxation` operation; semantic validators reject it if a future enum accidentally serializes it |
| `gbf-migrate` is deferred, so Stage 0 must error loudly on unsupported schema drift            | F-A6 already establishes this; Stage 0 produces `SchemaEpochUnsupported` or an adapter-specific failure unless §7.0 admits a tested lossless in-memory adapter |
| `QuantGraphBudgetView` malformedness gets reported as an opaque budget bust                    | Add `BudgetQuantGraphViewMalformed` and a dedicated fixture before other Stage-2 math runs                 |
| `CompileKnobs` schema churns during M1                                                         | Pin schema in `gbf-policy`; bump `pass_version_resolve` whenever it changes                               |
| Calibration confidence rules vary by kernel set                                                | `CalibrationBundleSet` carries per-layer confidence; Stage 0 validates per layer, not as a single scalar  |
| Floating-point in `static_budget.json` leaks non-determinism                                   | Forbid floats in v1; encode routing expectations as fixed-point integers such as `expected_q16_16`       |
| Cycle-budget temptation in Stage 2                                                             | Stage 2 reports only static counts; cycles live in F-B14's `schedule_cost.json`                           |
| `bd-w80` model-side byte math drifts from F-B4 byte math                                       | Stage 2 `expert_payload_bytes` is the canonical deployed-byte owner. `TernaryWeightPlan::compute_byte_cost` remains artifact/model diagnostic math and has an explicit divergence regression. |
| Bringup builds quietly accept stale or weak calibration                                        | No profile-time relaxation: Bringup uses an explicit `BootstrapCalibrationBundle` with declared `CalibrationConfidenceClass::None`; same `CalibrationConfidenceTooLow` gate fires for every other profile (§2.8, §2.13) |
| StageCache key includes too much / too little                                                  | Pin key inputs in `stage_cache.rs`; explicit regression tests for key stability                           |
| Reviewers cannot tell what a knob's source is                                                  | Per-knob `ConstraintProvenance` is mandatory in `compile_knobs.provenance`                                |
| `.golden.json` rot from incidental schema changes                                              | `regen.sh` is the only source of truth; staleness is loud                                                 |
| Validation runtime adopts `String` errors out of expedience                                    | `ValidationCode` is a closed enum; PR review rejects `String`-only paths                                  |

### 14.2 What we want to learn from the chunk

* Whether the validation taxonomy is complete enough that no later Epic B chunk needs to add a new `ValidationCode` for input-side issues. (If F-B5..F-B15 keep adding new `ValidationOrigin` values, this taxonomy needs revision.)
* Whether `RuntimeChromeBudget` needs further evolution before F-B4 can be useful. (If M1 model topologies routinely need a chrome budget that doesn't exist yet, the budget production pipeline — not F-B4 — needs work.)
* Whether `CompileKnobs` is granular enough for the refinement loop F-B16 will plug in. (If F-B16 needs additional `KnobDelta` variants, those land via amendment, not redesign.)
* Whether the `QuantGraphBudgetSource` + `QuantGraphBudgetView` seam cleanly survives F-B3's real implementation. (If F-B3 needs to break the trait or the view, the seam was wrong.)
* Whether shipping a `BootstrapCalibrationBundle` and a `bringup-*.chrome_budget.json` per target profile is sufficient for first-light builds, or whether further explicit-input variants are needed (e.g. a "no-MoE" topology budget). The answer must remain explicit-input-shaped, not profile-time relaxation.

## 15. Resolved seed open questions

| Seed question                                                                | Resolution                                                                                              |
| ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Should F-B2 own `repair_report.json`?                                        | No. F-B16 owns the refinement loop and `repair_report.json`; F-B2 only emits the `compile_knobs` schema |
| Should F-B4 emit cycle envelopes?                                            | No. Cycle envelopes live in F-B14's `schedule_cost.json`. Stage 2 reports static counts only.           |
| Should `RuntimeChromeBudget` be optional?                                    | Hard required for closure-eligible builds; missing budget is `BudgetMissingRuntimeChromeBudget`         |
| Should Stage 0 attempt schema migration?                                     | No lossy or on-disk migration. Stage 0 may use the narrow §7.0 lossless in-memory adapter path; otherwise it fails closed on schema mismatch. |
| Where does `RuntimeChromeBudget` live as a type?                             | `gbf-policy` (per `planv0.md` line 142, 144)                                                            |
| Where does Stage 0 implementation live?                                      | `gbf-codegen::stages::validate` (per `planv0.md` line 206)                                              |
| Where does Stage 2 implementation live?                                      | `gbf-codegen::stages::budget`                                                                           |
| Where do the JSON schemas live?                                              | `gbf-report`                                                                                            |
| Where does the shared diagnostic taxonomy live?                              | `gbf-policy::diagnostics`                                                                               |
| Should profiles carry a relaxation surface at all?                          | No. Bringup is a profile selection that pairs with explicit `BootstrapCalibrationBundle` and `bringup-*.chrome_budget.json` inputs; no profile-time latitude exists in F-B2/F-B4 (§2.13). |
| Should F-B4 consume a real `QuantGraph` before F-B3 lands?                   | No. F-B4 consumes the `QuantGraphBudgetSource` trait yielding a `QuantGraphBudgetView`; F-B3 lands the impl. |
| Should `policy_resolution.json` include runtime knobs?                       | No. Runtime knobs (yield quantum, scheduler profile) live in F-B1's `realism_report.v1.json`.           |
| Should `bd-w80`'s model-side byte math be reused?                            | Only as a target-independent artifact/model diagnostic. F-B4 deployed payload sizing uses Stage 2 `expert_payload_bytes`; future bd-w80 references should point there for canonical fit decisions. |
| Should the chunk introduce a new crate?                                      | No. All code lives in existing crates per `planv0.md` line 142–206.                                     |
| Is missing calibration ever a passing build state?                           | No. Passing Stage 0 always has a `CalibrationBundleSet`; Bringup uses an explicit `BootstrapCalibrationBundle`. |
| Is `CalibrationConfidenceClass::None` the same thing as no confidence requirement? | No. Bundle confidence `None` is distinct from profile requirement `NoMinimumConfidence`. |
| Should `policy_resolution.json` put `resolved`/`compile_knobs` at top level? | No. They live under `result`; failure reports set `result = None`. |
| Should missing `RuntimeChromeBudget` be only a diagnostic?                   | No. It is both `BudgetMissingRuntimeChromeBudget` and `BudgetFailure::MissingRuntimeChromeBudget` so `fits == failures.is_empty()` remains true. |
| Can F-B2 accept cross-major lossless schema adapters?                         | No. Cross-major schema changes are migration and wait for `gbf-migrate`. |
| Can Recovery use expected switch counts for v1 static decisions?             | No. Bringup, Default, Trace, and Recovery all use static upper bounds in v1. |

## 16. End state

After this chunk, every later Epic B feature starts with:

* a typed `ValidatedInputs` handle whose constructor is private to Stage 0;
* a `ResolvedCompilePolicy` with per-knob provenance;
* a deterministic `policy_resolution.json` emitted to `gbf-report`;
* a `StaticBudgetReport` whose `fits` flag is the static-fit precondition for the rest of the pipeline;
* a deterministic `static_budget.json` emitted to `gbf-report`;
* a `StageCache` shape every later stage can copy;
* a shared diagnostic taxonomy every later stage can extend;
* a self-hash convention every later report obeys;
* a known reject set: schema epoch, semantic core hash, manifest invariants, forbidden build-identity fields, lowering round-trip, packer version, missing/stale/low-confidence calibration, hint provenance, workload/golden refs, request admissibility, unsatisfiable knob bounds, locked-knob overrides, out-of-bounds knobs, malformed QuantGraph budget view, missing chrome budget, expert/common-bank/peak overflows, accumulator overflow, switches-per-token over cap, placement infeasibility.

The chunk does not make the pipeline transform anything. It makes the pipeline **reject the wrong inputs early, with provenance, in a report a reviewer can read**.

### 16.1 Forward compatibility — what later chunks inherit

This section is a contract for future RFCs. Every promise here either
already exists in this chunk's surface or will exist when the chunk closes.
A later RFC that wants to change one of these promises must explicitly amend
this RFC.

**For F-B3 (`QuantGraph`, Stage 1):**

* `QuantGraphBudgetSource` is the seam that F-B4 consumes. F-B3 must
  implement this trait against the real `QuantGraph`.
* `to_budget_view()` returns a serializable, canonicalizable
  `QuantGraphBudgetView`. F-B3 may add validators in the *production* of the
  view but may not change the view's public schema without amending this
  RFC.
* `quant_graph_hash` is the canonical hash of the full `QuantGraph` per
  F-B3's hashing rule. F-B4 records it and validates the budget view body; it
  does not recompute the full QuantGraph hash.

**For F-B5 (`GbInferIR`, Stage 3) through F-B15 (Backend, Stage 12):**

* Every later stage receives a `ResolvedPolicyProduct` rather than re-resolving
  policy. The product carries `policy_resolution_self_hash`,
  `artifact_validation_self_hash`, and `input_hashes`.
* Every later stage's report must use the `ReportEnvelope<R>` + canonical-JSON
  + domain-separated self-hash convention defined here.
* Every later stage's diagnostics must use `ValidationDiagnostic` with a
  closed `ValidationCode` enum extension. A new code per gate; never a free-form
  `String` detail.
* Every later stage's `StageCache` key must include `crate_feature_set_hash`
  and the relevant `*_schema_hash` constants.
* Stage 0/0.5/2 having validated and projected means later stages may
  *consume* hashes (e.g. `runtime_chrome_budget_hash`) but must not *re-derive*
  them.

**For F-B14 (`ScheduleCostAnalysis`, Stage 11):**

* `static_budget.json` is the static counterpart; it never claims cycles or
  measured costs. F-B14's `schedule_cost.json` is where those go.
* `ProjectedSwitchCount.upper_bound` may inform F-B14's risk policy. F-B14
  may also produce expected-value cost estimates against the upper bound.

**For F-B16 (`FeasibilityRefinementLoop`, blocked):**

* `compile_knobs` schema is wired in `policy_resolution.json` with
  `PolicySource` ⊆ `{TargetDefault, ProfileDefault, CompileRequestOverride,
  HintBundle, Calibration}` until F-B16 lands.
* When F-B16 unblocks, it adds `RepairProposal(RepairProposalId)` as a sixth
  legal `PolicySource`. It may also add
  `ConstraintOperation::AuthorizedRelaxation`. Until that amendment lands, both
  are rejected by F-B2/F-B4 semantic validators. Existing reports remain valid
  because they contain only the F-B2 source/operation set.
* F-B16 also adds `repair_report.json`. This RFC does not own that schema,
  but `policy_resolution.json` and `repair_report.json` are read together.

**For `gbf-migrate` (deferred to F-A6b):**

* When `gbf-migrate` lands, Stage 0's compatibility surface (§7.0) becomes a
  consumer of registered migration adapters rather than a hand-curated list.
  The shape of `ArtifactCompatibilityDecision` is forward-compatible, but
  cross-major compatibility remains forbidden until the migration RFC explicitly
  amends this section.

**For `gbf-train preflight`:**

* `DeployabilityEnvelope` and `RuntimeChromeBudget` remain the contract
  between trainer-side preflight and compiler-side validation. This chunk
  *consumes* both; it does not modify either.
* If preflight passes but Stage 0 fails on the same model, the divergence is
  diagnostic of either (a) drift between training-time and compile-time
  inputs (e.g. calibration), or (b) a `DeployabilityEnvelope` that
  underspecifies what Stage 0 actually checks. The latter is a feedback
  signal to the deployability layer.

### 16.2 Schema versioning strategy

This RFC introduces three v1 report schemas and one trait/view shape. Their
evolution rules:

* Each report schema (`artifact_validation.v1`, `policy_resolution.v1`,
  `static_budget.v1`) has its `schema_version` in the envelope. A v1.x
  bump may add optional fields; a v2 bump requires a new RFC and a migration
  story for `StageCache` keys.
* `ValidationCode` is a closed enum. Adding a variant is a backward-compatible
  schema-level change *only when* downstream consumers handle the new variant
  (typed match exhaustiveness). It never silently degrades.
  Unknown diagnostic variants remain hard deserialization failures for checked-in
  reports until the consuming schema is bumped.
* `QuantGraphBudgetView` is owned by F-B4. F-B3 produces it; F-B4 evolves it.
  A schema change here requires both features to land together.
* `ResolvedCompilePolicy` is owned by `gbf-policy`. Changes propagate
  automatically through `ResolvedPolicyProduct`.
* `CompileKnobs` shape changes bump `pass_version_resolve` and invalidate
  cached resolve outputs. This is a feature, not a cost: it forces
  recomputation when knob semantics change.

T-B6.C amendment (2026-05-14): `CompileProfileSpec` moved from
`1.0.0` to `2.0.0` by adding required `range_caps` and
`observation_caps` fields. Stage 0.5 pins
`compile_profile_spec_version = "2.0.0"` in `policy_resolution.json`, and
`pass_version_resolve` is bumped to `2.0.0` so cached resolve outputs are
invalidated across the profile-spec break.

### 16.3 Operating posture once the chunk closes

After merge, this chunk is **dormant**: no active development, no expected
churn. The next changes happen when:

1. F-B16 lands and adds `RepairProposal(_)` provenance.
2. F-B3 lands and replaces synthetic `QuantGraphBudgetSource` impls in tests.
3. A new `ValidationCode` is needed because a downstream stage discovered an
   input-side gap.
4. A target profile gains new capability flags that affect
   `CompileRequestUnsupportedFeature` or `CompileRequestTargetIncompatible`.

Any of those changes follows the amendment rule (§-1): record it as a typed
schema change with a migration note, never as folklore.

## 17. References

* `history/planv0.md` — §"The compiler pipeline" stages 0, 0.5, 2; §"The compile-request boundary" (`CompileRequest`, `ResolvedCompilePolicy`, `PolicyProvenance`); §"Deployability envelope" (`RuntimeChromeBudget`, `RomBudgetSlot`, `BudgetSlotClass`); §"Reports and artifacts" (`policy_resolution.json`, `repair_report.json`, `budget.json`); §"Engineering rules".
* `history/rfcs/F-B1-compute-bringup.md` — pass-shape rhetoric, canonical JSON rule, self-hash convention, review-packet pattern, claim-to-gate matrix shape.
* `history/rfcs/F-A1-gbf-asm.md` — `MachineEffect`, `PrivilegeClass`, `SystemCallKind` (referenced by downstream stages, not this chunk).
* `history/rfcs/F-A2-gbf-hw.md` — `TargetProfileId`, `TargetProfile`, calibration schema (consumed by Stage 0).
* `history/rfcs/F-A3-gbf-abi.md` — `BuildIdentityBlock`, `CompatibilityEnvelope` (Stage 0 records hashes; Stage 12 produces them).
* `history/rfcs/F-A4-banklease-banking.md` — `BankLease`/`BankGuard` (downstream; not consumed by this chunk).
* `history/rfcs/F-A6-gbf-store-migrate.md` — `gbf-migrate` deferral (F-A6b); Stage 0 fails closed on schema mismatch.
* `history/glossary.md` — artifact stratum, denotational stratum, policy provenance, calibration, hint bundle, deployability envelope, runtime chrome budget, placement profile, residency, common bank.
* `CONSTITUTION.md` — Doctrine of Correctness (§I), Velocity of Tooling (§II), Shifting Left (§III), Immutable Runtime (§IV), Observability (§V), Knowledge Graph (§VI).
* `CLAUDE.md` — beads workflow, pre-commit hook, session protocol, project skills.
