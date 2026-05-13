# Formal spec pack: F-S2 QAT Survives — DRAFT

> **Status: DRAFT.** Numeric values tagged `[ESTIMATE]` are unresolved and
> must be hardened before the first S2 result artifact is committed.
>
> There are two kinds:
> - **Prediction estimates**: expected ranges or tolerances used only for
>   hypothesis interpretation. Hardening these requires a new
>   `predictions_section_hash` and `rfc_revision`; it does not by itself
>   require `pass_version_S2`.
> - **Run-config estimates**: values that affect training, such as
>   `lambda_range` or `lambda_zero`. Hardening these changes
>   `train_config_hash` and requires a new `pass_version_S2` if any prior
>   S2 artifact exists.
>
> After the first result artifact commit, neither kind may be weakened
> without an explicit RFC amendment.

This is the second scientific/experimental RFC in the training-contract epic,
following F-S1 First Pulse (`history/rfcs/F-S1-first-pulse.md`). Its
deliverable is **verified knowledge** about whether a Toy0 ternary student,
distilled from a seed-matched frozen fp teacher produced inside the same S2
binary at the Phase A boundary, survives the
B → C → D hardening transitions on the TinyStories raw-byte corpus to within
a pre-registered ternary-vs-fp bpc gap.

Important interpretation:
  A `Fail-gap` result is a successful scientific falsification, not an
  implementation failure. An H3 Refuted verdict maps to
  `Pass-with-distill-warn`, not to a failure outcome: it says the chosen
  distillation policy hurt or failed to help Toy0 beyond the pre-registered
  tolerance, while H2 may still establish QAT survival. S2 retires QAT
  substrate risk only if H1, H4, H5, and H6 confirm; it retires Toy0 QAT
  survival risk only if H2 also confirms. It does not retire QAT quality
  risk on larger models. Closure of bd-1xqf remains blocked because this
  RFC chooses Toy0 ternary sufficiency under the matched-protocol gate as
  a mandatory condition.

```text
Spec:
  F-S2 QAT Survives
  Slice S2 of the training-contract epic (bd-1rb)
  Closure bead: bd-1xqf
  Predecessor RFC: F-S1 First Pulse (history/rfcs/F-S1-first-pulse.md)

Hypothesis-under-test:
  A Toy0 ternary student model, trained on TinyStories raw byte stream
  through the F4 phase scheduler in Phases A → B → C → D with logit
  distillation from a frozen fp teacher checkpoint produced by the same
  S2 run at the seed-matched Phase A boundary, scores
  a held-out val bpc that exceeds the matched fp val bpc by no more than
  0.5 bits per byte, for every one of five fixed seeds, under the same
  S1CpuDeterministic device profile and the same chunked-reset bpc math.

Owns:
  hypothesis statements H1..H6
  pre-registered prediction tables for S2
  Phase B/C/D run protocol on Toy0 (Phase A inherited from S1 byte-equality)
  QuantHardness ramp schedule
  Per-row threshold initialization rule for Toy0 ternary FFN linears
  Distillation temperature, lambda_distill default, teacher checkpoint
    binding rule
  Inert-loss policy for Toy0 (router-side lambdas present-but-zero)
  Standard loss-term gradient flow contract (lambda_zrouter, lambda_balance,
    lambda_range, lambda_zero) under Toy0 dense topology, with router-side
    terms exercised against synthetic router fixtures rather than Toy0
  Burn LinearStateBlock gradient smoke contract over a tiny fixture
  Phase transition integration test contract (5-phase schedule)
  s2_phase_log.v1, s2_score.v1, s2_distillation_log.v1,
    s2_loss_grad_flow.v1, s2_linearstate_grad_smoke.v1,
    s2_phase_transition_integration.v1, s2_ablation.v1,
    s2_oracle_re_run.v1, s2_report.v1
  S2 reproducibility laws
  S2 falsification suite

Does not own:
  charset_v1 enforcement                    (S3)
  Tier 2 vocab                              (S3)
  5-gram KN baseline                        (S3)
  ReferenceModelBundle export               (S3)
  ArtifactOracle round-trip                 (S3)
  v0_success workload manifest              (S3)
  Project Gutenberg / progression           (S4)
  BoundedKv vs LinearState A/B comparison   (S5)
  multi-timescale LinearState variants      (S5)
  RuntimeChromeBudget preflight             (S6)
  shadow_compile path                       (S6)
  Game Boy ROM build                        (S6)
  MoE / router (lambda_balance, lambda_zrouter, lambda_switch, low-rank
    router, expert dropout, switch stats export) on real model topology   (S7)
  StructuredWidthGates / lambda_shape / lambda_overflow                    (S8)

Inert in S2 (carried but not owned):
  Toy0 has no router and no MoE, so router-side lambdas
  (lambda_balance, lambda_zrouter, lambda_switch) are present in the
  configuration surface but enforced to zero on Toy0 by phase-effective
  composition. The standard loss-term gradient flow test exercises those
  terms against minimal synthetic router fixtures (not Toy0) to satisfy
  bd-1j7 without claiming router operation on Toy0.
```

## Inheritance from F-S1

S2 silently inherits from S1 (no re-derivation, no amendment):

```text
F-S1 §1   Hash256, Seed, TrainStep, EvalStep, BpcValue, GradNorm,
          Verdict, HypothesisStatus.
F-S1 §1   DomainHash construction and self-hash rule.
F-S1 §1   CanonicalTensorPayloadHash and CanonicalCheckpointWrite.
F-S1 §1   S1CanonicalJson encoding.
F-S1 §1   bpc reset-context formula at chunk_size = 128, vocab = 256.
F-S1 §1   3-gram baseline math (used in S2 only as a sanity floor in
          s2_report.v1 verbatim from S1, never as the S2 gate).
F-S1 §3   Toy0 ModelSizeProfile reference instance
          (d_model=16, d_ff=32, n_blocks=1, vocab=256, tied embeddings).
F-S1 §5   TrainConfig pinned fields:
            optimizer_steps   = 10000
            batch_size        = 32
            sequence_length   = 128
            eval_every_steps  = 1000
            eval_subset_size  = 4096
            optimizer         = AdamW { lr=1e-3, beta1=0.9, beta2=0.999,
                                        eps=1e-8, weight_decay=0.0 }
            rng_kind          = Pcg64Mcg
            device_profile    = S1CpuDeterministic
          and S1CpuDeterministic env_exact, thread_count, deterministic
          reductions, GPU/network/host-clock prohibitions.
F-S1 §5   InitRng / BatchRng / ShuffleRng disjoint streams; rejection
          sampling for uniform integer draw; Fisher-Yates definition.
F-S1 §6   3-gram baseline numbers (sanity floor only, not gate).
F-S1 §10  Reproducibility laws Rep-1..Rep-8 (with Rep-5 amended in §10
          below to cover the Phase B/C/D code paths).
F-S1 §16  Build-A "Phase A run" semantics for the fp teacher branch.
F-S1 §13  D7 measurement-oracle definitions are inherited unchanged.
          S2 re-runs the D7 measurement-oracle suite under the S2 binary
          and records metric_oracle_passed in s2_report.v1 because
          Phase B/C/D bpc computations rely on the same primitive.
```

S2 also inherits F-S1's test + observability infrastructure (now landed):

```text
bd-2i00  (F-S1.31) Test scaffolding + tests/common/ utilities. S2
         extends this with s2-specific fixtures and helpers in
         F-S2.23; it does not re-implement the dev-deps (proptest,
         insta, assert_cmd, predicates, trybuild, tempfile, criterion,
         tracing-test, pretty_assertions), the ScriptedRng pattern,
         the canonical_json_byte_eq / self_hash_excludes_field
         assertions, or the tracing_capture helper.

bd-16mx  (F-S1.36) End-to-end test harness with structured logs +
         golden artifacts. S2 adopts this pattern verbatim:
           - per-step structured `tracing` events with stable field
             schemas;
           - tracing_capture in tests for log-shape assertions
             (event name + level + structured fields);
           - golden artifact files under
             gbf-experiments/tests/snapshots/s2_*.json regenerated by
             `cargo insta accept` and stored in the repo;
           - CI captures the structured log stream + the artifact set
             as build artifacts so any regression is forensically
             reproducible.
         Every S2 producer bead (scheduler, train run, verifiers,
         report emitter) emits structured logs to a uniform event
         schema. Every S2 schema bead ships an insta golden artifact.

bd-i5tz  (F-S1.24) `falsify` test-only feature gate on
         gbf-experiments. S2 reuses the same feature name; S2's six
         broken-impl tests under tests/falsification_s2/ are
         compiled in only when --features falsify is enabled. The
         harness Guard pattern + falsification_s2_suite_hash
         construction lives in F-S2.16.

bd-7ljt  (F-S1.25) gbf-cli `gbf s1` subcommand surface. F-S2.20 adds
         `gbf s2` as a sibling subcommand with the same logging /
         structured-output / exit-code conventions.

bd-ah6o  (F-S1.32) GitHub Actions s1-pr / s1-nightly / s1-on-demand
         workflows. F-S2.22 adds s2-* workflows that mirror the
         structure (cargo fmt + clippy + test on PR; full TinyStories
         run nightly; parameterized on-demand) and call into the s2
         closure scripts owned by F-S2.21.
```

S2 amends S1 only in the following narrow ways:

```text
A-S2-1   Build configuration set is extended (§16): four S2 build
         configurations (`s2-ternary`, `s2-fp`, `s2-ternary-nodistill`,
         `s2-ablation`) join the existing S1 `phase_a` and `ablation`
         builds. The S1 builds remain normative for S1 closure and are
         reused, not re-defined.

A-S2-2   pass_version (Rep-5) is bumped because Phase B/C/D code paths
         are now compiled into the S2 binaries; the Phase A
         byte-equality proven in S1 H4 is re-asserted in S2 H4 against
         the bumped binary to detect regressions introduced by Phase
         B/C/D code presence.

A-S2-3   gbf-experiments grows a new module subtree `s2::*` parallel to
         `s1::*`. S1 modules are imported by reference, not duplicated.
         The s1_* schemas are unchanged and not re-emitted by S2.

A-S2-4   `gbf-experiments/falsify` feature gates an S2 falsification
         test set distinct from S1's; both can co-exist in the same
         test build and must each pass independently.
```

---

## Decisions

```text
D1  Phase plan and step budget on Toy0
    The S2 budget is the S1 budget unchanged at 10000 optimizer steps,
    redistributed across phases A → B → C → D as follows:

      Phase A  DenseTeacherWarmup     steps        1..=4000   (40 percent)
      Phase B  RouterWarmup           steps     4001..=5000   (10 percent)
      Phase C  ExpertTernaryQat       steps     5001..=8000   (30 percent)
      Phase D  FullNumericQat         steps     8001..=10000  (20 percent)

    Phase E HardenAndSelect is not run in S2 (it owns export and
    shadow_compile, both deferred). The phase scheduler must accept the
    A → B → C → D prefix as a valid run mode and assert the absence of
    Phase E artifacts in s2_phase_log.v1.

    Rationale:
      Phase A is allocated 4000 steps because it must produce a usable
      in-protocol teacher while leaving budget for the QAT phases. We
      explicitly accept a quality drop relative to S1's 10000-step
      Phase A baseline; the gate is matched-protocol
      (S1 fp baseline at 10000 steps is not the comparator for the H2
      gap). The matched comparator is the s2-fp build, which uses the
      same 4000-step Phase A and then continues through B/C/D with
      QuantHardness=Off while preserving the same scheduler transitions
      and distillation policy (see D6).

      Phase B at 1000 steps is short because Toy0 has no router; B is a
      stability checkpoint and a place for the scheduler to attach the
      teacher freeze boundary, not a router-warmup workload.

      Phase C at 3000 steps is the load-bearing QAT phase: ternary
      hardness ramps from Off to Hard via the ramp schedule in D2, and
      distillation is on. This is where the gap is paid down.

      Phase D at 2000 steps adds activation and norm fake-quant; in Toy0
      this is small surface area, but the integration is what S2 verifies.

    These step boundaries are part of this RFC. Changing any invalidates
    prior comparisons and constitutes a new experiment. They are also
    constants in s2_phase_log.v1.

D2  QuantHardness ramp schedule
    The full S2 ternary HardnessTriple is:

      Phase A, global steps 1..=4000:
        expert_qat     = Off
        activation_qat = Off
        norm_qat       = Off

      Phase B, global steps 4001..=5000:
        expert_qat     = Off
        activation_qat = Off
        norm_qat       = Off

      Phase C, global steps 5001..=8000, with k = global_step - 5000:
        if k <= 1000:          expert_qat = Off    (Phase C "soak" sub-window)
        if 1000 < k <= 2000:   expert_qat = Soft
        if k > 2000:           expert_qat = Hard
        activation_qat = Off
        norm_qat       = Off

      Phase D, global steps 8001..=10000, with k = global_step - 8000:
        expert_qat = Hard
        if k <= 500:
          activation_qat = Off
          norm_qat       = Off
        if 500 < k <= 1000:
          activation_qat = Soft
          norm_qat       = Soft
        if k > 1000:
          activation_qat = Hard
          norm_qat       = Hard

    This is a piecewise schedule, not linear or cosine. Rationale:
      - Soft and Hard are explicit enum variants in
        gbf-model::qat::QuantHardness with discrete semantics
        (Soft = sigmoid-based STE projection at the configured
        temperature; Hard = sign-based STE projection). A linear or
        cosine ramp between two discrete enum variants is undefined in
        the QAT module surface; only a piecewise step-boundary schedule
        is meaningful.
      - The 1000-step soak inside Phase C lets distillation lock onto
        the teacher logits before any ternary noise is introduced.
      - The 1000-step Soft window supplies a smooth gradient region
        for thresholds to settle.
      - The 1000-step Hard window verifies survival under deployable
        ternary semantics.

    These ramps are part of s2_phase_log.v1 and are recorded per step.

D3  Distillation policy
    lambda_distill                  = 1.0     (Phase C and D only)
    distillation_temperature        = 2.0     (default in
                                              gbf-train::loss::distillation,
                                              referenced by name in the
                                              s2_distillation_log.v1)
    distillation_loss_form          = T^2 * KL(softmax(teacher / T) ||
                                              softmax(student / T))

    The teacher checkpoint MUST be the seed-matched Phase A snapshot
    from this S2 run, frozen at Phase A end (step 4000). It is NOT the
    S1 closure checkpoint at step 10000.

    Rationale for not reusing S1 closure checkpoints:
      - S1 closure pinned a 10000-step Phase A budget. Reusing it as a
        teacher here would change the protocol because the S1 teacher
        was trained with no Phase B/C/D code in the binary; binding S2
        student gradients to the S1 teacher would conflate two
        binaries' Phase A semantics.
      - S2 H4 (Phase A cleanliness) proves that the S2 binary's Phase A
        byte-equals an ablation Phase A. Once H4 confirms, the S2
        Phase A teacher at step 4000 is itself a defensible artifact.
      - Per-seed teachers are required because the gap measurement in
        H2 is per-seed.

    The teacher freeze must satisfy gbf-train::teacher::FrozenTeacher:
      - detach_for_teacher invoked exactly once at step 4000 boundary;
      - teacher_requires_grad = false thereafter;
      - teacher_weight_fingerprint and teacher_storage_fingerprint
        recorded in s2_distillation_log.v1.

D4  Threshold plan
    Toy0 ternary FFN linears use a per-output-row threshold:

      ScaleGranularity       = PerOutputRow
      ScaleFormat            = Q8.8
      ThresholdPlan          = OneThresholdPerOutputRow
      WeightEncoding         = Ternary2

    Scale value semantics are inherited unchanged from the closed
    gbf-model::qat::ternary contract: <INSERT EXACT BEAD/RFC SECTION>.
    This placeholder is closure-blocking. S2 may not run until the
    inherited contract is cited, or until this RFC defines scale
    initialization, update, clipping, and serialization semantics.

    Per CLAUDE.md "Training Loss Beads" rule:
      For ternary zero/sparsity losses, matrix thresholds mirror the
      QAT ternary model contract: one global threshold or one threshold
      per output row. Do not expose per-weight thresholds.

    S2 chooses one-per-output-row to match the deployment plan
    (PerOutputRow scales already pinned in §F4 / planv0 line 951)
    rather than a single global threshold. Falsification
    F4_threshold_per_weight_structural_mask_fixture (§13 O5)
    deliberately uses a structural per-weight-threshold mask fixture
    and must Refute H5.4 without claiming a real helper/Burn-adapter
    mutation.

    Threshold initialization occurs after the step-5000 optimizer update
    and Phase B-end checkpoint write, and before the first forward pass
    of global step 5001, i.e. before Phase C local step 1:
      For each ternary FFN linear M, traversed in canonical model
      tensor order, and for each output row r in ascending row index:
        sum_abs_r = f64 sum over columns c = 0..cols-1 in ascending
                    column order of abs(f64(M.weight[r, c]))
        mean_abs_r = sum_abs_r / f64(cols)
        threshold_r = f32(0.7 * mean_abs_r)
      The f32 materialization uses the platform-independent canonical
      f64-to-f32 rounding rule inherited from CanonicalTensor encoding.
      The 0.7 multiplier matches the BitNet / 1-bit LLM literature
      convention; alternative values are A-blocked in §17 A-S2-7.

    In S2, thresholds are fixed non-trainable buffers after
    initialization. They are not trainable parameters and do not
    participate in CanonicalTensorPayloadHash for H4 before Phase C
    initialization. After initialization, threshold buffers are included
    in full checkpoint serialization, final checkpoint self-hashes, and
    threshold_stats in s2_score.v1, but they remain excluded from the
    H4 Phase A trainable-tensor payload comparison.

D5  Standard loss-term gradient flow contract
    Each of {lambda_zrouter, lambda_balance, lambda_range, lambda_zero}
    is exercised via gradient-flow tests under the SHARED protocol
    defined in §3 H5 and §13 O5. The gradient-flow tests are NOT run
    on Toy0 for the router-side terms because Toy0 has no router; they
    are run on a synthetic minimal router fixture defined in
    gbf-experiments::s2::loss_grad_flow::synthetic. This satisfies
    bd-1j7 acceptance without polluting the S2 train-run protocol.

    Per-term default values for S2:
      lambda_distill   = 1.0     (Phase C, D)
      lambda_balance   = 0.0     (Toy0 dense; tested on synthetic fixture
                                  with non-zero value)
      lambda_zrouter   = 0.0     (same)
      lambda_switch    = 0.0     (same)
      lambda_range     = 0.01
      lambda_zero      = 0.0001

    These two values are run-config constants, not prediction estimates.
    Changing either value after the first S2 result artifact requires:
      - a new train_config_hash;
      - a new pass_version_S2 if any prior S2 artifact exists;
      - an RFC amendment if the change weakens an already-committed run.
      lambda_shape     = 0.0     (S8 owns)
      lambda_overflow  = 0.0     (S8 owns)

    Per CLAUDE.md "Training Loss Beads":
      Tests for scalar hyperparameters such as safe bounds, temperatures,
      and loss weights must include a non-default/non-1.0 value.
    The synthetic-fixture gradient-flow test for each of
    {lambda_balance, lambda_zrouter, lambda_range, lambda_zero}
    therefore exercises at least one non-zero, non-1.0 value
    (see §3 H5 predictions).

D6  Matched-protocol gate
    The H2 gap is measured between two builds:

      s2-ternary  : the QAT-on student under D1's A→B→C→D schedule.
      s2-fp       : the same A→B→C→D schedule with QuantHardness=Off
                    pinned across all phases (i.e. the s2-fp model
                    sees Phase B/C/D scheduler transitions and
                    distillation losses but no quantization).

    The gap is computed as:

      gap(s) = bpc_ternary(s) - bpc_fp(s)
      Pass:   for all s in {0,1,2,3,4}. gap(s) <= 0.5 bpc.

    Both builds use:
      - the same TinyStoriesManifest (S1 fixture),
      - the same val byte sequence,
      - the same chunked-reset bpc primitive (S1 §7),
      - the same S1CpuDeterministic device profile,
      - the same seed list [0, 1, 2, 3, 4],
      - the same 10000 step budget redistributed per D1,
      - distinct teacher checkpoints (each build's own Phase A end).

    The fp build's distillation step receives its own Phase A teacher,
    so the fp build is not "no distillation"; it is "no quantization".
    The no-distillation control is owned by H3 below as a separate
    build (`s2-ternary-nodistill`), not by H2.

D7  Strict per-seed pass criterion
    H2 pass:  for all s. gap(s) <= 0.5 bpc. (D6.)
    H1 pass:  for all s. all four phases A,B,C,D Completed without
              divergence on the s2-ternary build.
    H4 pass:  Phase A on the s2-ternary build is bit-identical (in
              CanonicalTensorPayloadHash) to Phase A on the s2-ablation
              build for seed 0.
    H5 pass:  every standard loss-term gradient flow test passes for
              its expected parameter set and the corresponding
              stop-gradient set is verified zero.
    H6 pass:  Burn LinearStateBlock gradient smoke produces finite,
              nonzero, deterministic gradients on the pinned fixture.

D8  Phase transition integration test (T10.7 / bd-14k)
    A 5-phase fixture model, using PhaseKindFixture, with half-open
    zero-indexed phase intervals verifies:

      Phase A: fixture steps [0, 10)
      Phase B: fixture steps [10, 20)
      Phase C: fixture steps [20, 30)
      Phase D: fixture steps [30, 40)
      Phase E: fixture steps [40, 50)

    The fixture emits transition events at fixture steps 10, 20, 30,
    and 40, i.e. the first step of the new phase, mirroring the S2
    production convention where A->B is recorded at global step 4001.

    It verifies:
      - QuantHardness changes at exactly the step boundaries;
      - phase_transition log events fire exactly once per boundary;
      - teacher freeze fires at the A → B boundary, exactly once;
      - teacher_requires_grad becomes false at A → B and stays false;
      - skipped phase (start-from-C) edge case fires correct
        QuantHardness;
      - empty phase list and overlapping phases produce errors.
    This integration test is REQUIRED for closure but does NOT consume
    the 10000-step budget; it runs on a tiny model fixture
    (gbf-test::tiny_model) on the order of seconds.

D9  Burn LinearState gradient smoke (T12.2b / bd-2bm4)
    A Burn autodiff path through gbf-model::sequence::LinearStateBlock
    at Fixed(0.5) is exercised on a tiny synthetic input
    (sequence_length=8, hidden_dim=4, batch=1) and asserted to:
      - produce finite forward output;
      - produce finite, nonzero gradients on every trainable
        parameter and on the input tensor;
      - produce IDENTICAL gradient bytes (bitwise) on a second
        invocation with the same seed under S1CpuDeterministic.

    This is a smoke test, not a unit-correctness proof of the
    recurrence. The recurrence semantics are owned by bd-tnb (closed).
    S2's contract is that QuantHardness controls reach the LinearState
    boundary without breaking autodiff.

D10 Inert-loss policy on Toy0
    On the s2-ternary and s2-fp Toy0 builds:
      lambda_balance, lambda_zrouter, lambda_switch are present in
      LossConfig but ENFORCED to zero by phase-effective composition.
    On the synthetic router fixture used for H5:
      lambda_balance and lambda_zrouter are non-zero in their assigned
      gradient-flow sub-tests; lambda_switch remains 0 in S2 (S7 owns).

    Per CLAUDE.md "Training Loss Beads":
      Loss config helpers must distinguish raw TOML config from
      phase-effective config. Scalar diagnostic totals/logging helpers
      are not differentiable Burn training-loss composers.
    s2_distillation_log.v1 records phase-effective lambdas at every
    eval point; raw TOML config is recorded once in the run header.

D11 Strict reproducibility (extension of S1 D8)
    Same seed + same corpus_train_sha + same corpus_val_sha +
    same train_config_hash + same model_config_hash + same gbf-train
    pass_version_S2 + same dependency lockfile + same rust_toolchain_hash +
    same build_config_hash + same device_profile + same
    teacher_freeze_step + same ThresholdInitRng domain definition
    => bit-identical safetensors checkpoint AT EACH PHASE BOUNDARY
    AND at the final step.

    Phase-boundary snapshots are mandatory for all full S2 builds:
    s2_ternary_full, s2_fp_full, and s2_ternary_nodistill emit
    checkpoints at steps 4000, 5000, 8000, and 10000. The s2_ablation
    build emits only the step-4000 Phase A checkpoint.

D12 Fail-closed on NaN / divergence in any phase
    Any seed producing non-finite loss, non-finite gradient norm, or
    non-finite distillation loss at any step where distillation is
    computed fails the
    entire S2. No partial pass.

    Per CLAUDE.md "Training Loss Beads":
      Burn loss helpers must validate computed tensor losses, including
      weighted outputs, for finite values before returning.
    The S2 train loop records non-finiteness as a DivergedRunProduct
    with a divergence_event distinguishing
      NonFiniteLoss | NonFiniteGradNorm | NonFiniteDistillLoss
    and aborts the seed cleanly without serializing NaN/Inf.

D13 Optimizer pinned (inherited from S1 D10, plus scheduler clarification)
    AdamW { lr=1e-3, beta1=0.9, beta2=0.999, eps=1e-8, weight_decay=0.0 }.
    No learning rate schedule, no warmup. The phase scheduler does NOT
    modify optimizer state; it only flips QuantHardness, phase-effective
    loss lambdas according to the table in §1, and the teacher freeze.
    In particular, lambda_distill, lambda_range, and lambda_zero may
    change their phase-effective values without changing raw TOML config.
    This is part of D11's hash surface.
```

---

# 1. Core notation

S2 inherits Hash256, Seed, TrainStep, BpcValue, GradNorm, Verdict,
HypothesisStatus, Hypothesis, PredictedRange, ObservedStatistic,
CharVocab256, NGramOrder, CorpusManifestRef, TinyStoriesManifest,
DomainHash, Self-hash rule, CanonicalTensorPayloadHash,
CanonicalCheckpointWrite, S1CanonicalJson, and the Prediction status rule
verbatim from F-S1 §1. They are not re-derived here.

S2 introduces:

```text
PhaseKindS2     := PhaseA | PhaseB | PhaseC | PhaseD
                ; PhaseE is forbidden in S2 train-run artifacts.

PhaseKindFixture := PhaseA | PhaseB | PhaseC | PhaseD | PhaseE
                ; Used only by the D8 phase-transition integration
                ; fixture. PhaseE fixture events MUST NOT appear in
                ; s2_phase_log.v1 for TinyStories S2 runs.

QuantHardness   := Off | Soft | Hard           ; from gbf-model::qat
RouterTrainMode := NoRouter | SoftTop1 | HardTop1
                ; Toy0 S2 records NoRouter. Synthetic H5 router fixtures
                ; may record SoftTop1 or HardTop1.

GlobalStep      := u64
                ; absolute optimizer step counter, 1-indexed, monotonic
                ; across the entire run (NOT per-phase). Phase boundaries
                ; from D1 partition the global counter.

PhaseLocalStep  := u64
                ; 1-indexed step counter relative to the current phase.
                ; For Phase C: local_step = global_step - 5000.
                ; For Phase D: local_step = global_step - 8000.

PhaseStep       := GlobalStep

HardnessTriple  := { expert_qat: QuantHardness,
                     activation_qat: QuantHardness,
                     norm_qat: QuantHardness }

PhaseEffectiveLambda :=
  { lambda_distill: f32,    ; non-negative finite per loss-config validate
    lambda_balance: f32,
    lambda_zrouter: f32,
    lambda_switch: f32,
    lambda_range: f32,
    lambda_zero: f32,
    lambda_shape: f32,
    lambda_overflow: f32 }

DistillTemperature  := f32  ; finite, > 0; default 2.0 per D3
DistillLossNats     := f32  ; finite, recorded value >= 0
DistillLossNats64   := f64  ; promoted accumulator only; reported as f32

GapBpc          := f64       ; bpc_ternary - bpc_fp; required finite
ThresholdScalar := f32       ; per-output-row threshold value
Q8_8Scale       := u16       ; Q8.8 raw payload, per gbf-model::qat::ternary

S2BuildKind     := s2_ternary_full    ; s2-ternary build, A→B→C→D run
                | s2_fp_full         ; s2-fp build, A→B→C→D with hardness Off
                | s2_ternary_nodistill ; H3 control: no-distillation ternary
                | s2_ablation        ; QAT codepaths compiled out (mirror S1)

Phase-A teacher checkpoints are not a distinct build kind. They are
step-4000 checkpoints emitted by the parent build kind
(`s2_ternary_full`, `s2_fp_full`, or `s2_ternary_nodistill`).

Verdict     := Confirmed | Refuted     ; reused
HypothesisStatus :=
    Confirmed
  | Refuted
  | NotEvaluatedDueToPriorGate(reason: String)
  ; same dispatch discipline as F-S1 §1.

Hypothesis  := H1 | H2 | H3 | H4 | H5 | H6
            ; H3 is non-closure-gating in S2; see §3.

FailureKindS2   := Substrate | Gap | Distill | Phase | Metric | Suspicious
                | LinearState | LossGradFlow | PhaseIntegration
                | ApiDrift | Preregistration | Artifact | Incomplete
                ; Used only in verifier diagnostics and report body.

S2 does not emit a top-level generic Outcome. Use S2Outcome in §8.

PredictedRange     := { low: BpcValue, high: BpcValue }   ; low <= high
ObservedStatistic  := { median: BpcValue, min: BpcValue,
                        max: BpcValue, stddev: f64 }

PhaseEntry :=
  { step: PhaseStep,
    phase: PhaseKindS2,
    hardness: HardnessTriple,
    router_mode: RouterTrainMode,
    lambda_effective: PhaseEffectiveLambda,
    teacher_frozen: Bool,
    train_loss: f32,
    grad_norm: f32,
    distill_loss: DistillLossNats | null }
  ; one entry per optimizer step.

S2 reuses BatchRng, InitRng, ShuffleRng, seed128, uniform_u64_inclusive,
Fisher-Yates, and the S1CpuDeterministic device profile verbatim from S1.

Additionally:
  ThresholdInitRng(seed) = Pcg64Mcg(seed128("threshold_init", seed))
  This stream is consumed exactly once per ternary FFN linear at the
  Phase B → C boundary if any threshold initialization randomization is
  ever introduced. In v1 (D4), threshold init is deterministic from the
  Phase B-end weights, so this stream consumes zero u64 draws but is
  declared so future randomized init does not silently expand the rng
  domain set.
```

Distillation loss form (canonical per D3):

```text
For a batch B of token positions, with
  student_logits[b, c]   in R, vocab dim V = 256
  teacher_logits[b, c]   in R, same shape, no autodiff (frozen teacher)
  T                      = 2.0                                (D3)

  scaled_student[b, c] = student_logits[b, c] / T
  scaled_teacher[b, c] = teacher_logits[b, c] / T

  q[b, c] = softmax(scaled_teacher)[b, c]    ; computed under no_grad
  log_p[b, c] = log_softmax(scaled_student)[b, c]
  log_q[b, c] = log_softmax(scaled_teacher)[b, c]   ; under no_grad

  KL_per_pos[b] = sum_c q[b, c] * (log_q[b, c] - log_p[b, c])
                ; non-negative within KL_NEGATIVE_TOLERANCE = 1.0e-6
                ; per gbf-train::loss::distillation contract.

  pre_clamp_kl_loss = T^2 * mean_b KL_per_pos[b]
  if pre_clamp_kl_loss < -KL_NEGATIVE_TOLERANCE:
    error DistillationLossError::NegativeKlBeyondTolerance
  distill_loss_raw = max(pre_clamp_kl_loss, 0.0)
                ; this is the raw loss consumed by the training
                ; composer. The optional pre-clamp diagnostic may be
                ; recorded separately but is not the training loss.

Class axis is named explicitly: dim 1 ("vocab"). Reduction axis is
batch (dim 0); reduction is mean over batch positions. Per CLAUDE.md
"Training Loss Beads": logits reduction axis is named.

Gradient flow:
  d(distill_loss)/d(student_logits) flows in.
  d(distill_loss)/d(teacher_logits) MUST be exactly zero by construction
  (teacher is frozen; teacher tensor is detached). H5 verifies this.

Promotion rule:
  KL_per_pos accumulator is computed in f32 element-wise; the mean over
  batch is computed in f32 because the existing
  gbf-train::loss::distillation API returns f32. The reported
  distill_loss is f32. For across-step aggregation in
  s2_distillation_log.v1, an f64 running sum may be used; the per-step
  recorded value remains f32.

Reduction order:
  The vocab-axis sum is evaluated in ascending vocab index. The
  batch-position mean is evaluated in ascending batch-position index.
  No tree reduction or backend-dependent reduction order may be used
  under S1CpuDeterministic.

Finite-validation:
  Any non-finite distill_loss MUST trigger D12 fail-closed semantics.
  Per CLAUDE.md "Training Loss Beads":
    Loss helpers must validate computed tensor losses, including
    weighted outputs, for finite values before returning.
```

Training loss composer:

```text
For every train step, the scalar training loss is:

  total_loss =
      lm_loss_next_byte
    + lambda_distill_effective  * distill_loss_raw
    + lambda_balance_effective  * balance_loss_raw
    + lambda_zrouter_effective  * zrouter_loss_raw
    + lambda_range_effective    * range_loss_raw
    + lambda_zero_effective     * zero_loss_raw
    + lambda_shape_effective    * shape_loss_raw
    + lambda_overflow_effective * overflow_loss_raw

where:
  lm_loss_next_byte is inherited from S1's next-byte objective but is
    consumed by the S2 training composer in natural-log cross-entropy
    units, i.e. nats per byte/token. The bpc scorer remains a separate
    evaluation-only primitive. No log2/bpc value may be summed into
    total_loss during training.

  distill_loss_raw is also in nats. Therefore lm_loss_next_byte and
    lambda_distill_effective * distill_loss_raw are unit-compatible
    without conversion. This unit choice is part of train_config_hash.

  Raw losses for phase-effective lambda = 0 are handled according to
    the inert-loss discipline: router-side losses on Toy0 are not computed
    and are recorded as null; distillation raw diagnostics are computed
    in Phase C/D even when lambda_distill_effective = 0 for the
    nodistill build.

  The exact per-build and per-phase effective lambdas are defined in
    the PhaseEffectiveLambda table below and are part of train_config_hash.
```

Phase-effective lambda table for Toy0:

```text
Build s2_ternary_full:
  Phase A/B: lambda_distill = 0.0
  Phase C/D: lambda_distill = 1.0
  lambda_range = 0.01 iff activation_qat != Off or norm_qat != Off.
    Therefore lambda_range = 0.01 for Phase D local steps 501..=2000
    and 0.0 otherwise.
  lambda_zero = 0.0001 for global steps 5001..=10000, after threshold
    initialization at the Phase B -> C boundary. It is 0.0 before
    global step 5001.
  router-side lambdas are 0.0 in all phases.

Build s2_ternary_nodistill:
  Same as s2_ternary_full except lambda_distill = 0.0 in every phase.
  Raw distillation diagnostics are still computed in Phase C/D.

Build s2_fp_full:
  lambda_distill follows s2_ternary_full.
  QuantHardness is all-Off.
  lambda_zero = 0.0 because there is no ternary threshold surface.
  lambda_range = 0.0.
  Rationale: the matched fp build isolates quantization by preserving
  the same phase schedule and distillation policy while disabling
  QuantHardness-dependent QAT regularizers. Range regularization is
  therefore treated as QAT-side and is not applied to the fp comparator.

Build s2_ablation:
  Phase A only. All QAT-side lambdas are 0.0.
```

Range and zero loss forms (referenced by H5 only):

```text
range_loss(activations, safe_lo, safe_hi) =
  (1 / batch) * sum_b sum_axis [
      max(0, activations[b, axis] - safe_hi)^2
    + max(0, safe_lo - activations[b, axis])^2
  ]
  ; per CLAUDE.md "Training Loss Beads":
  ; "For activation/range losses, name the batch and per-sample
  ;  activation axes."
  ; Batch axis: dim 0. Per-sample activation axis: dim 1.
  ; Implementation must use a checked value object that names both
  ; widths; not a flat slice.

zero_loss(weights, threshold_per_row) =
  mean_r ((1 / cols) * sum_c indicator(|weights[r, c]| < threshold_r) *
                              |weights[r, c]|)
  ; ternary zero regularizer, L1-style toward zero on entries below
  ; the per-row threshold. NOT differentiable through the indicator
  ; in the strict sense; the ternary surface emits the indicator under
  ; stop-gradient, and the |weights[r,c]| factor carries the gradient.
  ; Per CLAUDE.md "Training Loss Beads":
  ; "For ternary zero/sparsity losses, matrix thresholds mirror the QAT
  ;  ternary model contract."
  ; Per-row threshold is the only legal granularity here.
```

bpc, 3-gram baseline, unigram, and 2-gram baseline definitions are
inherited from F-S1 §1, used unchanged.

---

# 2. Authority rules

```text
Scope(F-S2) =
  {
    H1, H2, H3, H4, H5, H6,
    Phase B/C/D run protocol on Toy0,
    QuantHardness ramp schedule (D2),
    Per-row threshold init rule (D4),
    Distillation lambda + temperature defaults (D3),
    Inert-loss policy on Toy0 dense (D10),
    Standard loss-term gradient flow contract (D5),
    Burn LinearState gradient smoke contract (D9),
    Phase transition integration test contract (D8),
    Matched-protocol gate (D6, D7),
    s2_phase_log.v1, s2_score.v1, s2_distillation_log.v1,
    s2_loss_grad_flow.v1, s2_linearstate_grad_smoke.v1,
    s2_phase_transition_integration.v1, s2_ablation.v1,
    s2_oracle_re_run.v1, s2_report.v1
  }

Rule Authority:
  for all behavior b in Scope(F-S2) and this RFC specifies b
  => SourceOfTruth(b) = this RFC.

Rule InheritanceFromS1:
  Behavior listed in "Inheritance from F-S1" above is sourced from F-S1
  unchanged. S2 cites by reference; it does not re-derive.

Rule PlanContext:
  Behavior outside Scope informed by planv0 amendments
  (especially the 2026-05-06 dense-baseline amendment around line 1065
  and the F4 phased-training material around lines 997-1045) and the
  bd-1rb thread. Closed features F1, F3, F4, F6, F12 (LinearStateBlock
  Fixed(0.5)) and Toy0 ModelSizeProfile (T14.1) provide the substrate;
  their contracts are not amended by this RFC.

Rule CrateOwnership:
  Every behavior in Scope(F-S2) is implemented in exactly one of:
    - gbf-experiments       (s2_* operations, S2 falsification suite,
                              S2 schema encoders, S2 replay CLI entrypoints,
                              synthetic router fixture for H5)
    - gbf-policy            (Toy0 ModelSizeProfile, unchanged)
    - gbf-model             (qat::ternary, qat::activation, qat::norm,
                              sequence::LinearStateBlock. Any public
                              API insertion required by the
                              QuantHardness control surface must be
                              listed explicitly in this RFC and added
                              to the O11 allowed-drift snapshot.)
    - gbf-train             (phase scheduler, teacher freeze, distillation
                              loss composer, Burn adapter, AdamW config,
                              `qat`/`qat-ablation` features as in S1, plus
                              new feature `qat-fp-only` per §16)
    - gbf-data              (TinyStoriesManifest reader; unchanged)
    - gbf-foundation        (Hash256, sha256 helper; unchanged)
    - gbf-artifact          (CanonicalTensor, CanonicalTensorPayloadHash,
                              QuantSpec, TernaryWeightPlan, ScaleGranularity,
                              ScaleFormat, ThresholdPlan, WeightEncoding;
                              unchanged)
    - gbf-cli               (`gbf s2` subcommand for replay)
    - gbf-test              (tiny_model fixture for D8 integration test;
                              unchanged from bd-mov / T10.1)
  No S2-specific code lives outside this set. The crate-level ownership
  table is normative; module names within each crate are illustrative
  unless explicitly tagged Required in §15.

Rule Amendment:
  Later slice changes any of:
    Phase boundaries (D1)
    QuantHardness ramp schedule (D2)
    Distillation form / temperature / lambda (D3)
    Threshold initialization rule (D4)
    Matched-protocol gate (D6)
    Pass criterion (D7)
    Inert-loss policy (D10)
  => Later slice's RFC must explicitly amend this RFC.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation
  produces the expected Refuted verdict on the appropriate hypothesis.
  Falsification sensitivity is a first-class proof obligation
  (§13 O5) and gates closure.

Rule InertLossDiscipline:
  Per CLAUDE.md "Training Loss Beads":
  "Do not give raw per-term diagnostic collections an implicit all-zero
  default; enabled lambdas can otherwise hide missing raw loss
  computation."

  S2 distinguishes:

    ComputedDisabled:
      The raw helper is intentionally invoked even though the
      phase-effective lambda is zero. The raw field MUST be finite and
      the weighted field MUST be exactly 0.0. This applies to
      distillation diagnostics in s2_ternary_nodistill during Phase C/D
      and to explicit H5 raw-helper diagnostic subchecks.

    StructurallyInert:
      The term is not meaningful on the Toy0 topology, e.g. router-side
      losses on a dense no-router model. The raw field MUST be null and
      the weighted field MUST be null.

    Enabled:
      The raw field MUST be finite and the weighted field MUST be
      finite.

  A literal 0.0 raw value is allowed only when the helper was actually
  computed and the mathematical raw loss is zero.
```

---

# 3. Hypothesis algebra

Every hypothesis carries a statement, predicted observables, falsification
rule, verdict mapping, and downstream consequence. H1, H2, H4, H5, H6 are
**mandatory closure gates**. H3 is **non-closure-gating**: it still has a
binary verdict, and that verdict shapes the s2_report.v1 narrative and any
follow-up bead, but H3 Refuted does not by itself block bd-1xqf closure.

```text
H1  Phase scheduler integrity
H2  Ternary capacity gap (closure gate)
H3  Distillation effectiveness (non-closure-gating)
H4  Phase A cleanliness preservation under S2 binary
H5  Standard loss gradient flow
H6  Burn LinearState gradient smoke
```

## H1 Phase scheduler integrity

```text
Statement:
  For every seed s in {0,1,2,3,4}, the s2-ternary build runs all four
  phases A, B, C, D end-to-end without divergence: every recorded
  per-step loss, gradient norm, and distill_loss is finite, the
  phase-boundary transitions fire at exactly the steps pinned by D1,
  the QuantHardness values at every step match the ramp schedule
  in D2, and the phase-effective loss configuration matches D3/D5/D10
  exactly.

Predicted:
  for all s, for all step in 1..=10000.
    train_loss(s, step) is finite
  for all s, for all step in 1..=10000.
    grad_norm(s, step) is finite and >= 0
  for all s, for all step in 5001..=10000.
    distill_loss(s, step) is finite and >= 0
  for all s.
    phase_transition events at steps {4001, 5001, 8001} fire exactly
    once each, in order.
  for all s.
    teacher_freeze event fires exactly once at the boundary after the
    step-4000 optimizer update and is recorded on global step 4001,
    the first step of Phase B.
  Number of optimizer steps with hardness expert_qat=Hard equals 3000:
    1000 steps in late Phase C plus all 2000 steps of Phase D.
  Number of optimizer steps with activation_qat=Hard equals 1000
    (the Phase D step k > 1000 sub-window).
  Number of optimizer steps with norm_qat=Hard equals 1000
    (the Phase D step k > 1000 sub-window).
  mean_train_loss(s, steps 1..10) is in [4.0, 6.5] (S1 H1 inheritance,
    sanity range only; an out-of-range observation is a Surprise).

Falsification:
  exists s, step. loss(s, step) non-finite                  => Refuted
  exists s, step. grad_norm(s, step) non-finite             => Refuted
  exists s, step in 5001..10000. distill_loss(s, step) non-finite
                                                            => Refuted
  exists s. phase_transition event count != 3               => Refuted
  exists s. teacher_freeze event count != 1                 => Refuted
  exists s. teacher_frozen flag is false at any step k > 4000
                                                            => Refuted
  exists s, step. recorded HardnessTriple at step does not match
    D2 ramp formula for that step                           => Refuted

Surprise, not falsification:
  exists s, step. grad_norm(s, step) >= 1e3 (extreme spike)
  exists s. mean_train_loss in 91..100 increases by > 0.5 over
    the same window. (Note: S1 H1's "decrease by 0.5" rule is NOT
    inherited because S2 Phase A budget is 4000 not 10000 and the
    early-loss profile may be slower; weakening this is intentional.)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S3..S8 cannot proceed.
  If Refuted by non-finite loss / grad / distill, investigate ternary
    STE, distillation autodiff, or scheduler off-by-one.
  If Refuted by missing phase_transition or teacher_freeze, investigate
    F4 phase scheduler integration; this is a Phase contract bug, not
    a numerical issue.
  If Refuted by wrong HardnessTriple at a step, investigate D2
    ramp implementation against the recorded schedule.
```

## H2 Ternary capacity gap (closure gate)

```text
Statement:
  Toy0 ternary survives QAT to within 0.5 bpc of the matched fp build
  on the same val under the matched protocol pinned by D6, for every
  seed.

Predicted:
  bpc_fp(s)              in [1.4, 1.9]                ; sanity range [ESTIMATE]
                                                      ; (S1 H2 informs but
                                                      ; does not constrain;
                                                      ; S2 Phase A is shorter)
  bpc_ternary(s)         in [1.6, 2.4]                ; sanity range [ESTIMATE]
  median(gap)            in [0.05, 0.40]              [ESTIMATE]
  for all s. gap(s) <= 0.5 bpc                        ; the actual gate

Quality sanity gate:
  for all s. bpc_fp(s) <= 2.5
  for all s. bpc_ternary(s) <= 3.0
  These are not claims of larger-model quality. They prevent the
  matched-protocol gap from passing when both models are obviously
  noncompetitive under the S1 TinyStories fixture.

Falsification:
  exists s. gap(s) > 0.5                              => Refuted
  exists s. bpc_fp(s) > 2.5                            => Refuted
  exists s. bpc_ternary(s) > 3.0                       => Refuted
  median(bpc_fp) < 0.5                                => Refuted
                                                      ; suspicious: same
                                                      ; floor as S1 H2
                                                      ; suspicion gate;
                                                      ; H2 cascades
                                                      ; Fail-suspicious.
  median(bpc_ternary) < 0.5                           => Refuted
                                                      ; same.

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted (non-suspicious):
  Open follow-up bead: investigate threshold init multiplier (D4),
  Phase C soak length (D2), distillation temperature (D3), or
  Toy0 capacity. Do NOT increase the 0.5 bpc gate without an RFC
  amendment.

Consequence when the outcome-level suspicious sentinel fires for any
scored full build:
  Halt. Audit train/val split for leakage, audit bpc accumulator,
  audit corpus loader. Same disposition as S1 H2 suspicious branch.
```

## H3 Distillation effectiveness (non-closure-gating)

```text
Statement:
  Distillation from the frozen Phase A teacher narrows the ternary-vs-fp
  bpc gap relative to a no-distillation ternary control: for every seed,
  the distilled gap is no larger than the no-distillation gap by more
  than a small tolerance (i.e. distillation does not strictly hurt).

Pre-registered weak form:
  for all s. gap_distill(s) <= gap_nodistill(s) + 0.10  bpc  [ESTIMATE]
  where gap_nodistill is computed against the same s2-fp run.

Pre-registered strong form (informational, not gating):
  median(gap_nodistill - gap_distill) >= 0.05 bpc       [ESTIMATE]

Builds:
  s2-ternary           : lambda_distill = 1.0 in Phase C and D
  s2-ternary-nodistill : lambda_distill = 0.0 throughout
  s2-fp                : the matched-protocol fp build (D6)

Predicted:
  gap_nodistill(s) in [0.10, 0.80]   ; informational range [ESTIMATE]
  gap_distill(s)   in [0.05, 0.45]   ; informational range [ESTIMATE]

Falsification:
  exists s. gap_distill(s) > gap_nodistill(s) + 0.10    => Refuted

Verdict:
  Refuted if falsification hits.
  Confirmed otherwise.

Why this is non-closure-gating:
  H2 is the substantive QAT-survival gate. H3 is a methodological
  check that distillation is not actively harmful; if it is, S5+
  may revisit distillation policy, but Toy0 may simply be too small
  to benefit from distillation, and that is not a substrate failure.

  However, the s2_ternary_nodistill control run is still a required
  S2 artifact. H3 Refuted does not block closure, but H3
  NotEvaluatedDueToPriorGate, missing nodistill artifacts, or nodistill
  divergence blocks closure because the methodological comparison was
  not actually performed.

Consequence of Refuted:
  Mark s2_report.v1 with Decision::ProceedToS3-with-distill-review
  (see §8 dispatch). Open follow-up bead in F4 epic to revisit
  distillation defaults (lambda_distill, temperature, or schedule).
```

## H4 Phase A cleanliness preservation under S2 binary

```text
Statement:
  The s2-ternary build's Phase A run, at QuantHardness=(Off, Off, Off),
  produces a step-4000 checkpoint whose CanonicalTensorPayloadHash
  equals the step-4000 checkpoint produced by an s2-ablation build
  in which all QAT codepaths are compiled out via `qat-ablation`,
  for seed 0.

  This re-asserts the S1 H4 invariant against the S2 binary, which
  contains additional Phase B/C/D code (distillation composer,
  ternary STE wiring, activation fake-quant, norm fake-quant). H4
  detects regressions caused by the mere PRESENCE of that code.

  During Phase A, QAT-only tensors such as ternary thresholds, ternary
  scales, activation fake-quant calibration buffers, or norm fake-quant
  buffers MUST either be absent or non-trainable and excluded from
  CanonicalTensorPayloadHash. The H4 comparison is over the dense Toy0
  trainable tensor set inherited from S1.

Predicted:
  canonical_tensor_payload_sha(s2-ternary, seed=0, step=4000)
    = canonical_tensor_payload_sha(s2-ablation, seed=0, step=4000)
  Whole-file safetensors byte equality is non-normative and may be
  reported separately only if the writer is canonicalized. H4 compares
  trainable tensor payloads only; checkpoint metadata, build_kind,
  SafeTensors metadata, and artifact paths must not participate in
  the H4 equality decision.
  Seeds 1..4 may be compared optionally and reported as observational.

Falsification:
  s2-ternary phase_a_tensor_payload_sha != s2-ablation
    phase_a_tensor_payload_sha => Refuted

Verdict:
  Confirmed if seed 0 produces matching canonical tensor payloads.
  Refuted otherwise.

Consequence of Refuted:
  Phase A is contaminated by Phase B/C/D code IN THE BUILD that S2
  closes against. Block S2 until F4's phase scheduler is fixed. This
  is independent of S1 H4 because the S1 binary did not contain
  distillation, ternary STE, or fake-quant codepaths at all.
```

## H5 Standard loss gradient flow

```text
Statement:
  Each of {lambda_zrouter, lambda_balance, lambda_range, lambda_zero}
  produces gradients only on the parameter set declared by the loss
  module's contract (the "in-scope" set), and zero gradients on the
  declared stop-gradient set. The tests are run on synthetic minimal
  fixtures (NOT on Toy0) per D5 and D10.

  Additionally, lambda_distill produces a non-zero gradient on
  student_logits and EXACTLY ZERO gradient on teacher_logits when
  the teacher is frozen.

Sub-hypotheses:
  H5.1  lambda_zrouter:
        non-zero grad on router_logits.
        zero grad on expert FFN weights, embeddings, sequence-state.
        Tested with router_logits magnitude in {1.0, 100.0} (D5
        non-default values).
        Per CLAUDE.md: zero point is uncentered. The z-loss is computed
        as stable_logsumexp(logits)^2, not by materializing
        sum_e exp(logits) in f32. The training
        lambda_zrouter loss is distinct from any QAT/router aux-loss
        proxy; only the training form is tested here.

  H5.2  lambda_balance:
        non-zero grad on router_logits via stop-gradient hard top-1
        dispatch provenance (the gradient reaches the routing
        probabilities through the soft selection, not through the
        hard assignment).
        zero grad on expert FFN weights.
        Per CLAUDE.md: hard top-1 assignments are stop-gradient
        dispatch provenance. The gradient reaches router_logits only
        through the soft routing probabilities used in soft_usage; it
        MUST NOT flow through the hard top-1 assignment tensor.

  H5.3  lambda_range:
        non-zero grad on the activation tensor at expert input/output
        boundary, proportional to distance outside [safe_lo, safe_hi].
        zero grad on parameters NOT named in the safe-range surface.
        Tests use a checked value object naming batch axis (dim 0)
        and per-sample activation axis (dim 1).
        Tested with safe_hi in {1.0, 8.0} and an activation magnitude
        of 16.0 (so the loss is non-trivial; D5 non-default values).

  H5.4  lambda_zero:
        non-zero grad on pre-threshold expert weights of magnitude
        below the per-row threshold.
        zero grad on expert weights of magnitude above the threshold
        (within an epsilon tolerance for the indicator boundary).
        Per CLAUDE.md: matrix thresholds mirror the QAT ternary
        model contract; this test uses a per-output-row threshold.
        The S2 falsification suite maps F4-broken to
        F4_threshold_per_weight_structural_mask_fixture (§13 O5),
        a structural mask fixture expected to Refute H5.4 without
        claiming a real helper/Burn-adapter mutation.
        It maps the H5.4b raw-diagnostic honesty fallback to
        F5_zero_loss_diagnostic_runner_fallback (§13 O5), a diagnostic
        runner sensitivity check rather than a real zero_loss helper or
        Burn-adapter mutation.

  H5.5  lambda_distill:
        non-zero grad on student_logits.
        EXACTLY ZERO grad on teacher_logits (teacher is detached).
        Tested with lambda_distill in {0.5, 1.0} and temperature in
        {1.0, 2.0} (D5 non-default values; T=1.0 is non-default).

Predicted:
  All five sub-hypotheses pass on the synthetic fixtures.
  Numerical-stability sub-checks pass:
    very large router logits (magnitude 100):
      H5.1 z-loss is finite, gradient is finite.
    very small expert usage (one expert at 99 percent):
      H5.2 balance loss is finite and large, gradient is finite.

Falsification:
  exists sub_hypothesis H5.k. expected non-zero gradient norm == 0
                                                            => Refuted
  exists sub_hypothesis H5.k. expected zero gradient norm > epsilon
                                                            => Refuted
  exists sub_hypothesis H5.k. forward or backward produces NaN/Inf
                                                            => Refuted
  H5.5 specifically: nonzero grad on teacher_logits         => Refuted

  epsilon for "expected zero" comparisons:
    1e-6 in f32; gradient norms below this are treated as zero.
    Boundary cases for H5.4 (weights exactly at threshold) are
    excluded from the zero-gradient test surface by construction;
    the test fixture uses weights at threshold +/- 1e-3.

Verdict:
  Refuted if any sub-hypothesis falsifies.
  Confirmed otherwise.

Consequence of Refuted:
  Block S2 closure. The failing sub-hypothesis identifies the
  specific loss module to repair. lambda_distill failure is the most
  serious because it directly corrupts H2.
```

## H6 Burn LinearState gradient smoke

```text
Statement:
  The Burn autodiff path through gbf-model::sequence::LinearStateBlock
  at Fixed(0.5) decay produces finite, nonzero, deterministic
  gradients on the pinned tiny fixture defined in
  gbf-experiments::s2::linearstate_smoke::FIXTURE_V1, under the
  S1CpuDeterministic device profile.

Fixture V1:
  sequence_length         = 8
  hidden_dim              = 4
  batch                   = 1
  input_init_seed         = LinearStateSmokeRng("linearstate_input_v1")
  parameter_init_seed     = LinearStateSmokeRng("linearstate_params_v1")
  parameter_init_policy   = deterministic_non_degenerate
                           ; forbids all-zero tensors, repeated rows
                           ; that make a trainable parameter unused,
                           ; and initial values known to make the
                           ; mean-output loss gradient exactly zero.
  loss                    = mean over output[t, h] multiplied by a fixed
                           deterministic nonzero coefficient
                           coeff[t, h] = 1 + t + 17*h, normalized by
                           sum_t,h coeff[t,h].
                           ; This avoids accidental cancellation that can
                           ; make a trainable parameter appear unused in
                           ; the tiny fixture. No labels are needed.

  LinearStateSmokeRng(domain_tag: String) =
    Pcg64Mcg(seed128(concat("linearstate_smoke/", domain_tag), 0))
  ; domain_tag MUST be one of {"linearstate_input_v1",
  ; "linearstate_params_v1"} in FIXTURE_V1.

Predicted:
  forward_output is finite for every element.
  for all trainable parameters p. grad(p) is finite.
  for all trainable parameters p declared active by FIXTURE_V1.
    ||grad(p)||_2 > 0.
  ||grad(input)||_2 > 0   (input gradient should also flow because
                          the recurrence couples timesteps).
  Re-running with the same seed produces a byte-identical gradient
  bytestream (per-tensor, in canonical order), under
  S1CpuDeterministic.

Falsification:
  exists p in trainable. grad(p) non-finite                 => Refuted
  exists p in trainable. ||grad(p)||_2 == 0                 => Refuted
  ||grad(input)||_2 == 0                                    => Refuted
  Re-run gradient bytes != original gradient bytes          => Refuted

  The S2 falsification suite maps F6-broken to
  F6_linearstate_structural_smoke_fallback (§13 O5), a structural
  smoke fallback expected to Refute H6 without claiming a mutation of
  the public LinearState Burn adapter.

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Block S5 (BoundedKv vs LinearState A/B) until the autodiff path is
  fixed. S2 closure is also blocked: H6 is a closure gate because
  later slices assume a working LinearState autodiff.
```

Hypothesis composition rules are formalized in §8 (Outcome algebra).

---

# 4. Experiment state machine

```text
State :=
    Configured(corpus, model_config, train_config_s2, baseline_ref)
  | LoadedBaselines(state, bpc_3gram_S1, bpc_unigram_S1)   ; carried from
                                                            ; S1 artifacts
                                                            ; for sanity
                                                            ; reporting only
  | LinearStateSmokeRun(state, smoke_result)                ; H6 gate
  | LossGradFlowRun(state, grad_flow_results)               ; H5 gate
  | PhaseTransitionIntegRun(state, integ_result)            ; D8 gate
  | OracleReRun(state, oracle_result)                         ; O3 gate
  | ApiDriftChecked(state, api_drift_result)                  ; O11 gate
  | FalsificationChecked(state, falsification_result)         ; O5 gate
  | TrainAttempted(state, ternary_run_products[5],
                          fp_run_products[5],
                          nodistill_run_products[5])
  | AblationAttempted(state, ablation_run_product_seed_0)
  | Trained(state, completed_run_products_per_build)
  | Scored(state, val_bpc_ternary[5], val_bpc_fp[5],
                  val_bpc_nodistill[5])
  | GapComputed(state, gap_ternary_vs_fp[5],
                       gap_nodistill_vs_fp[5])
  | AblationCompared(state, phase_a_eq_ablation_seed_0)
  | Reported(state, report)
  | Decided(state, decision: ProceedToS3
                          | ProceedToS3-with-distill-review
                          | Investigate(reason)
                          | Halt(reason))
```

Transitions:

```text
T0 configure:
  empty -> Configured(c)

T1a baseline carry:
  Configured(c) -> LoadedBaselines(c, S1.bpc_3gram, S1.bpc_unigram)
  ; S2 does NOT re-fit the baseline; it reads s1_baseline.v1 by
  ; baseline_self_hash. If the recorded hash does not match the on-disk
  ; baseline, the run aborts before T2.

T1b LinearState smoke:
  LoadedBaselines(c, _, _) ->
    LinearStateSmokeRun(state, run_linearstate_smoke())
  ; H6 gate is run early so a broken autodiff path fails fast before
  ; the 5-seed train phase consumes hours.

  LinearStateSmokeRun(state, smoke) and smoke.smoke_passed = false ->
    Reported(state, build_fail_linearstate_report(state))

T1c LossGradFlow:
  LinearStateSmokeRun(state, smoke) ->
    LossGradFlowRun(state, run_loss_grad_flow_suite())
  ; H5 gate, also run early for fast failure.

  LossGradFlowRun(state, grad) and grad.overall_passed = false ->
    Reported(state, build_fail_loss_grad_flow_report(state))

T1d PhaseTransitionInteg:
  LossGradFlowRun(state, _) ->
    PhaseTransitionIntegRun(state, run_phase_transition_integration())
  ; D8 / bd-14k integration test. Runs on the tiny test fixture; cheap.

  PhaseTransitionIntegRun(state, integ) and integ.integ_passed = false ->
    Reported(state, build_fail_phase_integration_report(state))

T1e oracle re-run:
  PhaseTransitionIntegRun(state, integ) and integ.integ_passed = true ->
    OracleReRun(state, run_s1_oracle_re_run_under_s2_binary())

  OracleReRun(state, oracle) and oracle.metric_oracle_passed = false ->
    Reported(state, build_fail_metric_report(state))

T1f API drift:
  OracleReRun(state, oracle) and oracle.metric_oracle_passed = true ->
    ApiDriftChecked(state, run_s2_api_drift_check())

  ApiDriftChecked(state, api) and api.api_drift_check_passed = false ->
    Reported(state, build_fail_api_drift_report(state))

T1g falsification suite:
  ApiDriftChecked(state, api) and api.api_drift_check_passed = true ->
    FalsificationChecked(state, run_s2_falsification_suite())

  FalsificationChecked(state, falsify) and
  falsify.falsification_s2_passed = false ->
    Reported(state, build_fail_falsification_report(state))

T2 train (three parallel build types, each over 5 seeds):
  FalsificationChecked(state, _) ->
    TrainAttempted(state,
                   [s2_train_run(c, s, S2BuildKind::s2_ternary_full)
                      for s in seeds],
                   [s2_train_run(c, s, S2BuildKind::s2_fp_full)
                      for s in seeds],
                   [s2_train_run(c, s, S2BuildKind::s2_ternary_nodistill)
                      for s in seeds])

T2a all completed:
  TrainAttempted(c, t, f, nd) and
  for all r in (t ++ f ++ nd). r.completion = Completed
  -> Trained(c, t, f, nd)

T2b divergence short-circuit:
  TrainAttempted(c, t, f, nd) and
  exists r in (t ++ f ++ nd). r.completion = DivergedAt(_)
  -> Reported(state, build_fail_substrate_report(state))

T3 score:
  Trained(c, t, f, nd) ->
    Scored(c, [s1_score_bpc(t[s], V_val) for s in seeds],
              [s1_score_bpc(f[s], V_val) for s in seeds],
              [s1_score_bpc(nd[s], V_val) for s in seeds])
  ; bpc primitive is the SAME function as S1; this is enforced
  ; by reuse, not by re-implementation.

T4 compute gap:
  Scored(c, vt, vf, vnd) ->
    GapComputed(c,
                [vt[s] - vf[s] for s in seeds],
                [vnd[s] - vf[s] for s in seeds])

T5 ablation (seed 0 mandatory):
  GapComputed(...) ->
    AblationAttempted(state,
      s2_train_run(c, seed=0, S2BuildKind::s2_ablation))

  AblationAttempted(state, ablation_run) and
  ablation_run.completion = Completed ->
    AblationCompared(c, ablation_eq(t[0], ablation_run))

  AblationAttempted(state, ablation_run) and
  ablation_run.completion = DivergedAt(_) ->
    Reported(state, build_fail_phase_report(state))

T6 report:
  AblationCompared(...) -> Reported(state, build_report(state))

T7 decide:
  Reported(state, r) -> Decided(state, decide(r))
```

Invariants:

```text
I-S2-1
  T1b, T1c, T1d MUST run before T2; their failure blocks T2 entry.

I-S2-2
  T2 issues 15 full train runs (5 seeds * 3 builds). T5 issues one
  additional ablation run for seed 0. Run isolation per
  S1 Rep-7 applies: no shared mutable state between any pair of
  runs.

I-S2-3
  T3 reuses s1_score_bpc verbatim; the scorer is parameterized by
  checkpoint and val_bytes only. No S2-specific scorer.

I-S2-4
  T4 gap is computed only after all three builds have produced
  finite val_bpc. A divergence on any build path fails T2b.

I-S2-5
  T5's ablation checkpoint for seed 0 must use the same seed,
  PhaseAEffectiveConfigHash, corpus_*_sha,
  model_config_hash, device_profile, and rng stream definitions.
  Only the QAT code paths differ.

I-S2-6
  T6 emits exactly one s2_report.v1 instance per S2 PR. Re-runs
  after RFC amendment produce a new report with bumped
  rfc_revision.

I-S2-7
  Decided is final: closure of bd-1xqf is gated on
  Decision in {ProceedToS3, ProceedToS3-with-distill-review}.
```

---

# 5. Run protocol contract

```text
RunInputs :=
  {
    corpus_train: ByteSeq        ; sha256-pinned via manifest
    corpus_val:   ByteSeq        ; sha256-pinned via manifest
    model_config: Toy0Config     ; from ModelSizeProfile::Toy0 (T14.1)
    train_config: TrainConfigS2  ; pinned by D1, D3, D13
    seed:         Seed
    build_kind:   S2BuildKind    ; selects ternary, fp, nodistill, or
                                  ; ablation
  }

TrainConfigS2Full :=
  {
    optimizer_steps:   10000             ; (S1 inheritance)
    batch_size:        32                ; (S1 inheritance)
    sequence_length:   128               ; (S1 inheritance)
    eval_every_steps:  1000              ; (S1 inheritance)
    eval_subset_size:  4096              ; (S1 inheritance)
    optimizer:         AdamW { lr: 1e-3, beta1: 0.9, beta2: 0.999,
                               eps: 1e-8, weight_decay: 0.0 }
                                          ; (S1 inheritance, D13)
    phase_plan:        [ PhaseA(1..=4000),
                         PhaseB(4001..=5000),
                         PhaseC(5001..=8000),
                         PhaseD(8001..=10000) ]   ; (D1)
    hardness_ramp:     PhaseCRampD2 + PhaseDRampD2              ; (D2)
    distill_temp:      2.0                                       ; (D3)
    lambda_distill_default: 1.0                                  ; (D3)
    threshold_init_multiplier: 0.7                               ; (D4)
    range_safe_lo:      -1.0
    range_safe_hi:       1.0
                         ; Used only when lambda_range_effective > 0.
                         ; These are train-config constants and are
                         ; included in train_config_hash.
    teacher_freeze_step: 4000                                    ; (D3)
    rng_kind:          Pcg64Mcg
    device_profile:    S1CpuDeterministic                        ; (S1)
  }

TrainConfigS2PhaseAOnly :=
  {
    optimizer_steps:   4000
    batch_size:        32
    sequence_length:   128
    eval_every_steps:  1000
    eval_subset_size:  4096
    optimizer:         same AdamW pins as TrainConfigS2Full
    phase_plan:        [ PhaseA(1..=4000) ]
    rng_kind:          Pcg64Mcg
    device_profile:    S1CpuDeterministic
  }

PhaseAEffectiveConfigHash :=
  Hash over the fields that can affect Phase A execution:
    optimizer hyperparameters, batch_size, sequence_length,
    eval settings, rng streams, model_config, corpus hashes,
    device_profile, and PhaseA boundary.
  It excludes future Phase B/C/D schedule fields.
  This exclusion is only for H4 Phase A cleanliness comparison. Full
  S2 reproducibility and all full-run artifacts use train_config_hash,
  which includes the complete A->B->C->D plan and every build_kind
  override.

For S2BuildKind::s2_fp_full only:
  hardness_ramp is OVERRIDDEN to all-Off across every phase.
  lambda_distill_default is unchanged (the fp build still distills
  from its own Phase A teacher; this isolates distillation from
  quantization in the H2 comparison per D6).

For S2BuildKind::s2_ternary_nodistill only:
  lambda_distill_default is OVERRIDDEN to 0.0.
  hardness_ramp is identical to s2_ternary_full.

For S2BuildKind::s2_ablation only:
  Phases B/C/D are NOT EXECUTED. Only Phase A 1..=4000 runs. The
  resulting checkpoint is compared in H4 against the s2-ternary
  Phase A end checkpoint.

Rng streams:
  InitRng(seed)              = Pcg64Mcg(seed128("init", seed))
  BatchRng(seed)             = Pcg64Mcg(seed128("batch", seed))
  ShuffleRng(seed)           = Pcg64Mcg(seed128("shuffle", seed))
  ThresholdInitRng(seed)     = Pcg64Mcg(seed128("threshold_init", seed))
                              ; declared per D4; consumes 0 draws in v1
  ; All four streams are disjoint by domain prefix.

Operation:

operation s2_train_run
  input:  RunInputs
  output: RunProductS2 (CompletedRunProductS2 | DivergedRunProductS2)

Preconditions:
  S2-Pre-1  input.corpus_*.sha256 matches manifest (S1 fixture).
  S2-Pre-2  input.model_config equals Toy0 reference instance exactly.
  S2-Pre-3  input.train_config equals TrainConfigS2Full pinned values
            exactly, modulo build_kind overrides above.
  S2-Pre-4  input.seed is in {0, 1, 2, 3, 4}.
  S2-Pre-5  byte_length(corpus_train) >= sequence_length.
  S2-Pre-6  byte_length(corpus_val) > 0.
  S2-Pre-7  device_profile is S1CpuDeterministic and the env_exact
            check passes BEFORE any tensor allocation.
  S2-Pre-8  For build_kind in {s2_ternary_full, s2_fp_full,
            s2_ternary_nodistill}: phase_plan is the full D1 plan.
            For build_kind = s2_ablation: train_config is
            TrainConfigS2PhaseAOnly.

Postconditions:
  S2-Run-Ok-1  completion = Completed
               => for all step in 1..=optimizer_steps.
                  run_log.loss(step) is finite.
  S2-Run-Ok-1a completion = Completed
               => for all step in 1..=optimizer_steps.
                  run_log.grad_norm(step) is finite and >= 0.
  S2-Run-Ok-2  completion = Completed
               => run_log records optimizer_steps train losses (10000
                  for full builds, 4000 for ablation), plus eval points
                  every eval_every_steps inclusive of step 0 and the
                  final step.
  S2-Run-Ok-3  completion = Completed
               => phase_log records exactly one phase_transition event
                  per D1 boundary that the run actually crosses
                  (3 events for full builds, 0 for ablation).
  S2-Run-Ok-4  completion = Completed and build_kind is in
               {s2_ternary_full, s2_ternary_nodistill}
               => phase_log records HardnessTriple at every step matching
                  D2's ramp formula.
  S2-Run-Ok-5  completion = Completed and build_kind = s2_fp_full
               => phase_log records HardnessTriple = (Off, Off, Off) at
                  every step.
  S2-Run-Ok-6  completion = Completed
               => final_checkpoint deserializes back to a Toy0 model.
                  For full builds, intermediate phase-boundary
                  checkpoints exist at steps 4000, 5000, 8000, 10000.
                  For s2_ablation, only the step-4000 checkpoint exists.
  S2-Run-Ok-7  completion = Completed and build_kind in
               {s2_ternary_full, s2_fp_full, s2_ternary_nodistill}
               => for all step in 5001..=10000.
                  distill_loss(step) is finite and >= 0 within
                  the gbf-train::loss::distillation tolerance.
                  For s2_ternary_nodistill, lambda_distill = 0.0 by D5
                  override, but the raw distill_loss diagnostic is
                  STILL computed and recorded with the literal value
                  written to s2_distillation_log.v1; it is NOT
                  multiplied into the training loss. Per CLAUDE.md
                  "Training Loss Beads": "Keep raw weighted-loss
                  helpers honest: they must validate finite/non-negative
                  raw diagnostics even when the configured weight is
                  zero."
  S2-Run-Ok-8  completion = Completed and build_kind in
               {s2_ternary_full, s2_fp_full, s2_ternary_nodistill}
               => teacher_freeze fires at step 4000 boundary, exactly
                  once. teacher_storage_fingerprint and
                  teacher_weight_fingerprint are recorded.
  S2-Run-Fail-1
                completion = DivergedAt(k)
                => divergence_event.step = k and
                   divergence_event.observed records the first
                   non-finite loss, gradient norm, OR distill_loss
                   without serializing NaN or Inf.
  S2-Run-Fail-2
                exists s. completion(s) = DivergedAt(_)
                => S2Outcome = Fail-substrate (per D12).
  S2-Run-Warn-1
                A 10-step mean train-loss increase greater than 2.0 is
                recorded as a Surprise, not DivergedAt, unless it also
                produces non-finite loss.
```

---

# 6. Distillation contract

```text
DistillInputs :=
  {
    student_logits: Tensor[batch_positions, vocab=V]
    teacher_logits: Tensor[batch_positions, vocab=V]   ; under no_grad
    temperature:    DistillTemperature         ; > 0, finite
    lambda_distill: f32                        ; >= 0, finite (composer
                                                 applies; the raw helper
                                                 ignores)
  }

DistillProduct :=
  {
    distill_loss_raw:        DistillLossNats   ; clamped T^2 * KL form
    pre_clamp_kl_loss:       f32 | null         ; optional diagnostic
    distill_loss_weighted:   DistillLossNats   ; lambda_distill * raw
  }
```

Operation:

```text
operation s2_distill_step
  input:  DistillInputs
  output: DistillProduct

Preconditions:
  Di-Pre-1   student_logits.shape[0] = teacher_logits.shape[0]
  Di-Pre-2   student_logits.shape[1] = teacher_logits.shape[1] = V
             and V >= 2.
  Di-Pre-2a  In the S2 TinyStories train/scoring runs, V MUST equal 256.
             Synthetic H5.5 fixtures may use smaller V to make exact
             gradient checks tractable.
  Di-Pre-3   temperature > 0 and finite
  Di-Pre-4   lambda_distill >= 0 and finite
  Di-Pre-5   For each element of student_logits and teacher_logits:
             |logit / T| is representable in f32 without overflow at
             log_softmax. Per
             gbf-train::loss::distillation::DistillationLossError::
             ScaledLogitOverflow contract.

Postconditions:
  Di-Ok-1    distill_loss_raw is finite and >= 0.0 after tolerance
             handling. A pre-clamp KL estimate below
             -KL_NEGATIVE_TOLERANCE is an error.
  Di-Ok-2    distill_loss_weighted = lambda_distill * distill_loss_raw,
             validated finite per CLAUDE.md "Training Loss Beads"
             ("Burn loss helpers must validate computed tensor losses,
             including weighted outputs, for finite values before
             returning").
  Di-Ok-3    teacher_logits is consumed through a detached/no_grad path.
             The exact-zero teacher gradient property is verified by H5.5,
             not by this forward-only operation.
  Di-Ok-4    Output is deterministic: same inputs (same bytes, same
             dtype, same axis order) => same outputs and same
             tape topology.

No failure mode:
  All preconditions are checked; precondition violation is a
  DistillationLossError, not an experiment outcome. NaN/Inf in OUTPUT
  is escalated to a DivergedRunProductS2 with
  observed = NonFiniteDistillLoss.
```

---

# 7. Bpc scoring contract

S2 reuses the operation contract `s1_score_bpc` from F-S1 §7 unchanged:

```text
ScoreInputs :=
  {
    checkpoint:   SafeTensors blob
    val_bytes:    ByteSeq    ; canonical val split
    chunk_size:   128
  }

ScoreProduct :=
  {
    bpc:              BpcValue
    token_count:      u64
    log2_sum:         f64
    score_self_hash:  Hash256
  }
```

Application:

```text
The same primitive scores ALL THREE build types' final checkpoints
against the SAME val_bytes. This is normative for D6 matched-protocol:
the gap is computed by subtracting f64 bpc values produced by the same
function on the same bytes with byte-equal chunking. No per-build
scorer fork.
The H2/H3 final bpc values are computed on the canonical held-out
validation byte sequence used by s1_score_bpc. They are not computed
from the training-loop eval_subset_size = 4096 diagnostics.

Per CLAUDE.md "Oracle and Conformance Beads":
  Quantization-gap metrics over token logits must aggregate per
  token/vocab row; do not softmax a whole prompt's concatenated logits
  as one distribution.
The S2 gap is computed at the bpc-aggregate level (not per-token
quantization gap), but the underlying scorer respects per-token reset-
context aggregation; chunk-boundary resets are inherited from S1 §7
and are not violated by S2.
```

---

# 8. Outcome algebra

```text
S2Outcome :=
    Pass-clean              ; H1 ^ H2 ^ H3 ^ H4 ^ H5 ^ H6 all Confirmed
  | Pass-with-distill-warn  ; H1 ^ H2 ^ H4 ^ H5 ^ H6 Confirmed; H3 Refuted
  | Fail-substrate          ; H1 Refuted, or any seed diverged
  | Fail-gap                ; H2 Refuted (non-suspicious)
  | Fail-suspicious         ; median(bpc_fp) < 0.5 or median(bpc_ternary)
                            ;   < 0.5
  | Fail-phase              ; H4 Refuted
  | Fail-loss-grad-flow     ; H5 Refuted
  | Fail-linearstate        ; H6 Refuted
  | Fail-phase-integration  ; D8 phase-transition integration failed
  | Fail-falsification      ; O5 S2 falsification suite failed
  | Fail-api-drift          ; O11 public API non-drift check failed
  | Fail-metric             ; D7 oracle suite (carried from S1) regresses
                            ;   under the S2 binary
  | Fail-preregistration    ; O1 pre-registration proof failed
  | Fail-artifact           ; required artifact missing or self-hash invalid
  | Fail-incomplete         ; required non-gating artifact missing, e.g. H3
                            ;   control absent, while no earlier failure
                            ;   justifies NotEvaluatedDueToPriorGate
```

Combination (mandatory checks first):

```text
if preregistration_check_passed = false                      => Fail-preregistration
elif any reached required artifact has invalid self_hash      => Fail-artifact
elif closure-candidate report has missing required artifact   => Fail-artifact
elif H6 verdict = Refuted                                    => Fail-linearstate
elif H5 verdict = Refuted                                    => Fail-loss-grad-flow
elif phase_transition_integ.integ_passed = false             => Fail-phase-integration
elif falsification_s2_passed = false                         => Fail-falsification
elif s1_oracle_re_run was reached
  and s1_oracle_re_run.metric_oracle_passed = false           => Fail-metric
elif api_drift_check was reached
  and api_drift_check_passed = false                          => Fail-api-drift
elif exists seed s, build b. completion(s, b) = DivergedAt(_) => Fail-substrate
elif H1 verdict = Refuted                                    => Fail-substrate
elif H4 verdict = Refuted                                    => Fail-phase
elif median(bpc_fp) < 0.5
  or median(bpc_ternary) < 0.5
  or median(bpc_nodistill) < 0.5                             => Fail-suspicious
elif H2 verdict = Refuted                                    => Fail-gap
elif H3 status = NotEvaluatedDueToPriorGate(_)
  and no prior failure outcome explains that status          => Fail-incomplete
elif any required verifier record is NotReached
  and no prior failure outcome explains that status           => Fail-incomplete
elif H3 verdict = Refuted                                    => Pass-with-distill-warn
else                                                          => Pass-clean
```

Decision dispatch:

```text
Pass-clean                  -> Decision::ProceedToS3
Pass-with-distill-warn      -> Decision::ProceedToS3-with-distill-review
Fail-gap                    -> Decision::Investigate(propose-tighten-D2-ramp-or-D3-temp)
Fail-substrate              -> Decision::Investigate(burn-or-distill-substrate)
Fail-phase                  -> Decision::Investigate(F4-phase-contract)
Fail-loss-grad-flow         -> Decision::Investigate(loss-module-of-failing-sub-hyp)
Fail-linearstate            -> Decision::Investigate(linearstate-autodiff-or-burn-adapter)
Fail-phase-integration      -> Decision::Investigate(F4-phase-transition-integration)
Fail-falsification          -> Decision::Investigate(S2-verifier-insensitive)
Fail-api-drift              -> Decision::Investigate(public-api-drift-requires-amendment)
Fail-metric                 -> Decision::Halt(measurement-broken)
Fail-suspicious             -> Decision::Halt(audit-split-and-bpc)
Fail-preregistration        -> Decision::Halt(preregistration-invalid)
Fail-artifact               -> Decision::Halt(artifact-missing-or-self-hash-invalid)
Fail-incomplete             -> Decision::Halt(required-methodological-control-missing)
```

`Halt` blocks bd-1xqf closure unconditionally. `Investigate` creates a
follow-up bead and may extend this RFC's scope or seed list.

---

# 9. Artifact schemas

## 9.1 s2_phase_log.v1

```text
Path:
  Header:
    experiments/S2/runs/{build_kind}/seed-{seed}/phase-log.json
  Entries:
    experiments/S2/runs/{build_kind}/seed-{seed}/phase-log.jsonl

PhaseLog (JSON, header) :=
  {
    schema:                "s2_phase_log.v1"
    seed:                  Seed
    build_kind:            S2BuildKind
    train_config_hash:     Hash256
    full_s2_phase_boundaries: [4000, 5000, 8000, 10000] ; D1
    executed_checkpoint_steps: List[u64]
                              ; full builds: [4000, 5000, 8000, 10000]
                              ; ablation: [4000]
    hardness_ramp_id:      "PhaseCRampD2+PhaseDRampD2"
    teacher_freeze_step:   4000
    phase_log_self_hash:   Hash256
  }

PhaseEntry (JSONL, one per optimizer step) :=
  {
    step:                  PhaseStep
    phase:                 PhaseKindS2
    hardness:              HardnessTriple
    router_mode:           RouterTrainMode  ; NoRouter on Toy0
    lambda_effective:      PhaseEffectiveLambda
    teacher_frozen:        Bool
    train_loss:            f32
                            ; finite scalar total_loss after all enabled
                            ; weighted terms are composed.
    grad_norm:             f32
                            ; finite, >= 0, global norm after backward
                            ; and before optimizer update.
    distill_loss:          DistillLossNats | null
    events:                List[PhaseEvent]
                           ; Empty list when no event fires.
                           ;
                           ; distill_loss is null in Phase A and B
                           ; and finite f32 in Phase C and D for full
                           ; builds. Per D-Inert discipline: null,
                           ; not 0.0, means "not computed".
  }

PhaseEvent :=
    PhaseTransition { from: PhaseKindS2, to: PhaseKindS2 }
  | TeacherFreeze { teacher_checkpoint_sha: Hash256 }

Invariants:
  PL-0  phase_log_self_hash is computed over:
          1. PhaseLog header JSON with phase_log_self_hash omitted,
          2. the exact ordered JSONL PhaseEntry bytes after canonical
             per-line JSON normalization,
        concatenated with a domain separator
        "s2_phase_log.v1/header+entries".
  PL-1  Number of JSONL entries equals optimizer_steps for full builds
        (10000) and 4000 for ablation.
  PL-2  Number of PhaseTransition events embedded in JSONL = 3 for full
        builds, 0 for ablation.
  PL-3  teacher_frozen is false for steps 1..=4000 and true for steps
        4001..=10000 (full builds).
  PL-4  HardnessTriple at every step matches D2 for s2_ternary_full and
        s2_ternary_nodistill; (Off,Off,Off) for s2_fp_full and
        s2_ablation.
  PL-5  distill_loss is finite for steps 5001..=10000 in
        s2_ternary_full, s2_fp_full; recorded but not multiplied for
        s2_ternary_nodistill (still finite); null in steps 1..=5000.
        For s2_ablation, distill_loss is null for every step.
  PL-6  Transition events are recorded on the first optimizer step of
        the new phase. Therefore A->B is recorded at step 4001,
        B->C at step 5001, and C->D at step 8001.
  PL-7  train_loss is finite for every JSONL entry.
  PL-8  grad_norm is finite and >= 0 for every JSONL entry.
```

## 9.2 s2_score.v1

```text
Path:
  experiments/S2/scores/{build_kind}/seed-{seed}/score.json

S2ScoreReport (JSON) :=
  {
    schema:                "s2_score.v1"
    seed:                  Seed
    build_kind:            S2BuildKind
    checkpoint_sha:        Hash256
    corpus_val_sha:        Hash256
    chunk_size:            128
    token_count:           u64
    log2_sum:              f64
    bpc:                   BpcValue
    threshold_stats:       Null | ThresholdStatsSummary
                            ; non-null only for s2_ternary_full and
                            ; s2_ternary_nodistill
    scale_stats:           Null | ScaleStatsSummary
                            ; same nullability
    score_self_hash:       Hash256
  }

ThresholdStatsSummary :=
  {
    matrices:        u32              ; count of ternary linears (Toy0 = 2)
    threshold_min:   f32
    threshold_max:   f32
    threshold_mean:  f32
    threshold_count: u32
                      ; per-row count, summed across matrices
  }

ScaleStatsSummary :=
  {
    matrices:        u32
    scale_count:     u32
    scale_min:       Q8_8Scale
    scale_max:       Q8_8Scale
    scale_mean_f32:  f32
                      ; mean of f32-decoded Q8.8 scales
  }
```

## 9.3 s2_distillation_log.v1

```text
Path:
  experiments/S2/distillation/{build_kind}/seed-{seed}/distill-log.json

DistillationLog (JSON) :=
  {
    schema:                       "s2_distillation_log.v1"
    seed:                         Seed
    build_kind:                   S2BuildKind
    teacher_checkpoint_sha:       Hash256
                                   ; the seed's own Phase A end checkpoint;
                                   ; never an S1 checkpoint.
    teacher_weight_fingerprint:   String  ; hex of TeacherWeightFingerprint
    teacher_storage_fingerprint:  String
    teacher_freeze_step:          4000
    distill_temperature:          DistillTemperature
    lambda_distill_default:       f32
    distill_loss_per_eval_point:  List[(EvalStep, DistillLossNats | null)]
                                   ; null entries for eval points before
                                   ; Phase C onset (step 5001).
    phase_log_self_hash:           Hash256
                                   ; points to the authoritative
                                   ; per-step phase-effective lambda log.
    loss_terms_per_eval_point:     List[LossTermEvalPoint]
                                   ; raw and weighted diagnostics for
                                   ; enabled loss terms at eval points.
    distill_log_self_hash:        Hash256
  }

LossTermEvalPoint :=
  {
    eval_step: EvalStep,
    lambda_effective: PhaseEffectiveLambda,
    raw_losses: {
      distill: DistillLossNats | null,
      balance: f32 | null,
      zrouter: f32 | null,
      range: f32 | null,
      zero: f32 | null,
      shape: f32 | null,
      overflow: f32 | null
    },
    weighted_losses: {
      distill: f32 | null,
      balance: f32 | null,
      zrouter: f32 | null,
      range: f32 | null,
      zero: f32 | null,
      shape: f32 | null,
      overflow: f32 | null
    }
  }

Invariants:
  DL-1  teacher_checkpoint_sha matches the s2_checkpoint at step 4000
        for the same seed and build_kind.
  DL-2  For build_kind = s2_ternary_nodistill, lambda_distill_default
        = 0.0. Eval points before Phase C onset record distill_loss
        as null. Eval points in Phase C/D record finite raw
        distill_loss values, not null, because the raw helper is still
        invoked. For build_kind in {s2_ternary_full, s2_fp_full},
        lambda_distill_default = 1.0.
  DL-3  Per CLAUDE.md "Training Loss Beads":
        "Loss config helpers must distinguish raw TOML config from
        phase-effective config." DistillationLog records BOTH the
        configured lambda_distill_default and phase-effective lambdas
        per eval point, and it links to the authoritative per-step
        phase log via phase_log_self_hash.
```

## 9.4 s2_loss_grad_flow.v1

```text
Path:
  experiments/S2/loss-grad-flow/results.json

LossGradFlowReport (JSON) :=
  {
    schema:                "s2_loss_grad_flow.v1"
    fixtures:              List[FixtureResult]
    overall_passed:        Bool
    loss_grad_flow_self_hash: Hash256
  }

FixtureResult :=
  {
    sub_hypothesis:        "H5.1" | "H5.2" | "H5.3" | "H5.4" | "H5.5"
    loss_term:             "lambda_zrouter" | "lambda_balance" |
                           "lambda_range" | "lambda_zero" |
                           "lambda_distill"
    in_scope_grad_norms:          Map<String, f32>
                                   ; key = tensor or parameter name,
                                   ; value = ||grad||_2
    stop_gradient_grad_norms:     Map<String, f32>
                                   ; key = tensor or parameter name;
                                   ; values must be 0
                                   ; within epsilon = 1e-6.
    non_default_value_used:        Bool
                                    ; per CLAUDE.md "Training Loss Beads":
                                    ; tests for scalar hyperparameters
                                    ; must include a non-default/non-1.0
                                    ; value. This flag MUST be true for
                                    ; every fixture.
    numerical_stability_passed:    Bool
    diagnostic_subchecks:          List[DiagnosticSubcheckResult]
    detached_grad_absence:         Map<String, Bool>
                                   ; true means the backend omitted the
                                   ; gradient entry because the tensor
                                   ; was detached. Used by H5.5.
    sub_passed:                    Bool
  }

DiagnosticSubcheckResult :=
  {
    name: String,
    lambda_value: f32,
    raw_loss_computed: Bool,
    raw_loss_finite: Bool,
    weighted_loss_value: f32 | null,
    passed: Bool
  }

Invariants:
  LGF-1  fixtures.length = 5 (one per sub-hypothesis).
  LGF-2  for all f in fixtures. f.non_default_value_used = true.
  LGF-3  overall_passed = AND over fixtures of f.sub_passed.
  LGF-4  H5.5's stop_gradient_grad_norms MUST contain
         "teacher_logits" with value exactly 0.0 (not within epsilon;
         exact zero, because teacher is detached).
  LGF-5  The H5.4 fixture MUST include diagnostic_subcheck
         "lambda_zero_raw_honesty_at_zero_weight" with lambda_value = 0.0.
         This subcheck is exempt from LGF-2 because its purpose is to
         test zero-weight raw diagnostic computation.
```

## 9.5 s2_linearstate_grad_smoke.v1

```text
Path:
  experiments/S2/linearstate-smoke/result.json

LinearStateSmokeReport (JSON) :=
  {
    schema:                       "s2_linearstate_grad_smoke.v1"
    fixture_id:                   "FIXTURE_V1"
    fixture_seq_len:              8
    fixture_hidden_dim:           4
    fixture_batch:                1
    forward_finite:               Bool
    param_grad_norms:             Map<String, f32>
    input_grad_norm:              f32
    determinism_byte_equal:       Bool
                                   ; true iff a re-run produces
                                   ; bitwise-equal gradient bytes.
    smoke_passed:                 Bool
    smoke_self_hash:              Hash256
  }

Invariants:
  LS-1  forward_finite = true.
  LS-2  for all p in param_grad_norms. p.value finite and > 0.
  LS-3  input_grad_norm finite and > 0.
  LS-4  determinism_byte_equal = true.
  LS-5  smoke_passed = LS-1 ^ LS-2 ^ LS-3 ^ LS-4.
```

## 9.6 s2_phase_transition_integration.v1

```text
Path:
  experiments/S2/phase-transition-integ/result.json

PhaseTransitionIntegReport (JSON) :=
  {
    schema:                       "s2_phase_transition_integration.v1"
    fixture_id:                   "tiny_model_T10.1"
    fixture_phase_boundaries:     [0, 10, 20, 30, 40, 50]
    transition_event_count:       u32
    teacher_freeze_event_count:   u32
    hardness_at_boundary:         Map<u32, HardnessTriple>
    skip_phase_test_passed:       Bool
    overlap_phase_error_raised:   Bool
    empty_phase_error_raised:     Bool
    integ_passed:                 Bool
    integ_self_hash:              Hash256
  }

Invariants:
  PT-1  transition_event_count = 4
        (5 phases means 4 inter-phase transitions, A->B, B->C, C->D, D->E).
  PT-2  teacher_freeze_event_count = 1.
  PT-3  hardness_at_boundary records the hardness on the first step of
        the new phase:
          hardness_at_boundary[10] = hardness for Phase B
          hardness_at_boundary[20] = hardness for Phase C
          hardness_at_boundary[30] = hardness for Phase D
          hardness_at_boundary[40] = hardness for Phase E
        The fixture schedule pins these values explicitly:
          Phase B = (Off, Off, Off)
          Phase C = (Soft, Off, Off)
          Phase D = (Hard, Soft, Soft)
          Phase E = (Hard, Hard, Hard)
  PT-4  skip_phase_test_passed = true.
  PT-5  overlap_phase_error_raised = true (the fixture deliberately
        constructs an overlapping schedule and asserts the scheduler
        rejects it).
  PT-6  empty_phase_error_raised = true (same idea).
  PT-7  integ_passed = AND of PT-1..PT-6.
```

## 9.7 s2_ablation.v1

```text
Path:
  experiments/S2/ablation/seed-0/ablation-report.json

S2AblationReport (JSON) :=
  {
    schema:                       "s2_ablation.v1"
    seed:                         0
    s2_ternary_phase_a_checkpoint_sha:  Hash256   ; step 4000
    s2_ablation_phase_a_checkpoint_sha: Hash256   ; step 4000
    s2_ternary_tensor_payload_sha:      Hash256
    s2_ablation_tensor_payload_sha:     Hash256
    phase_a_eq_ablation:                Bool
    first_mismatch:                     Null |
                                        { tensor: String,
                                          byte_offset: u64 }
    ablation_self_hash:                 Hash256
  }

Invariants:
  AB-1  phase_a_eq_ablation =
          (s2_ternary_tensor_payload_sha
           = s2_ablation_tensor_payload_sha).
  AB-2  Closure requires phase_a_eq_ablation = true.
```

## 9.8 s2_oracle_re_run.v1

```text
Path:
  experiments/S2/oracle-re-run/result.json

S2OracleReRunReport (JSON) :=
  {
    schema:                 "s2_oracle_re_run.v1"
    s1_oracle_suite_version: String
    metric_oracle_passed:   Bool
    oracle_cases:           List[String]
    oracle_re_run_self_hash: Hash256
  }

Invariants:
  OR-1  metric_oracle_passed = true is required for closure.
  OR-2  oracle_re_run_self_hash is included in s2_report.v1.
```

## 9.9 s2_report.v1

```text
Path:
  docs/experiments/S2-report.md

Front-matter (YAML, hashed into report) :=
  ---
  schema:                "s2_report.v1"
  s2_outcome:            S2Outcome
  decision:              Decision
  baseline_self_hash_carried_from_s1: Hash256
  oracle_re_run_passed:  Bool
                          ; D7 oracle re-run under the S2 binary.
  oracle_re_run_self_hash: Hash256
  api_drift_check_passed: Bool
                          ; O11 public API snapshot check.
  qat_public_api_snapshot_hash: Hash256
  linearstate_public_api_snapshot_hash: Hash256
  per_seed_artifacts:
    List[{
      seed: Seed,
      build_kind: S2BuildKind,
      completion: Completed | DivergedAt(TrainStep) | NotReached,
      checkpoint_self_hashes: { phase_a: Hash256 | null,
                                 phase_b: Hash256 | null,
                                 phase_c: Hash256 | null,
                                 final:   Hash256 | null },
      phase_log_self_hash:    Hash256 | null,
      score_self_hash:        Hash256 | null,
      distill_log_self_hash:  Hash256 | null
    }]
  ablation_self_hash:    Hash256 | null
  loss_grad_flow_self_hash: Hash256
  linearstate_smoke_self_hash: Hash256
  phase_transition_integ_self_hash: Hash256
  falsification_s2_passed: Bool
  falsification_s2_suite_hash: Hash256
  generated_at:          RFC3339 UTC, informational only, excluded from
                         report hash. Same exclusion rule as S1 R-Self-Hash.
  rfc_revision:          GitCommitId | Hash256
  predictions_section_hash: Hash256
  predictions_commit:    GitCommitId
  first_result_commit:   GitCommitId
  pass_version_S2:       SemVer
  report_self_hash:      Hash256
  ---

Required sections (markdown body):
  ## Pre-registered predictions
    Predicted ranges and pass criteria as committed before any S2
    result artifact commit. Same R-Predictions discipline as S1.
    Includes the 0.5 bpc gap gate, the H3 weak form, and all H5/H6
    sub-hypothesis predictions.

  ## Observed
    Per-seed table per build_kind: val_bpc, gap, ablation_eq (seed 0
    only), completion. Plus aggregate statistics min/median/max/stddev.
    Includes carried-S1 baseline numbers for reference (NOT a gate).

  ## Hypothesis verdicts
    H1, H2, H3, H4, H5, H6 each as HypothesisStatus, with the concrete
    observation that drove each verdict.
    Closure-candidate reports must use only Confirmed | Refuted.
    Early-failure reports may use NotEvaluatedDueToPriorGate.

  ## Falsification analysis
    Direct citation of which prediction or falsification rule fired
    for each Refuted hypothesis.

  ## Surprises
    Anything outside predicted ranges, even if not a verdict change.
    Includes the H1 inheritance-weakening note (see H1 Surprises).

  ## Decision
    Exactly one Decision tag, justified in <= 3 sentences.

  ## Reproducibility statement
    Exact commands + manifest hashes + pass_version_S2 to replay all
    three build types (s2_ternary_full, s2_fp_full,
    s2_ternary_nodistill) and the s2_ablation Phase A run.

Invariants:
  R-S2-Decision     Exactly one Decision tag in front-matter.
  R-S2-AllSeeds     per_seed_artifacts and the observed per-seed table
                    cover all 5 seeds for s2_ternary_full and s2_fp_full;
                    s2_ternary_nodistill participates in H3 only and
                    must also cover all 5 seeds.
  R-S2-ClosureArtifacts
                    For Decision in {ProceedToS3,
                    ProceedToS3-with-distill-review}, every required
                    self-hash is non-null.
  R-S2-Self-Hash    report_self_hash is computed over:
                      1. the parsed YAML front-matter object, rendered
                         through S1CanonicalJson with generated_at and
                         report_self_hash omitted;
                      2. a domain separator
                         "s2_report.v1/frontmatter+body";
                      3. the markdown body bytes exactly as committed.
                    The original YAML key order is non-normative.
  R-S2-Predictions  predictions_commit is a strict ancestor of
                    first_result_commit. first_result_commit is the
                    earliest commit introducing any non-null S2 result
                    artifact self-hash, completed/diverged run-product
                    field, score value, or hypothesis verdict derived
                    from an S2 result. Schema templates, null
                    placeholders, and pre-registration-only report
                    drafts do not count as result artifacts.
  R-S2-AllHypotheses All six hypotheses have an explicit
                    HypothesisStatus. For Decision in {ProceedToS3,
                    ProceedToS3-with-distill-review}, every status
                    must be a binary Verdict, not
                    NotEvaluatedDueToPriorGate.
```

The pre-registration timestamp is itself a load-bearing artifact, same
as S1.

---

# 10. Reproducibility laws

```text
Rep-S2-1 Seed determinism (per-build)
  for all s, b. replay(s, b, manifest, pass_version_S2) byte-identical
  to original(s, b, manifest, pass_version_S2). At every phase
  boundary AND at the final step.

Rep-S2-2 Cross-machine determinism is NOT required for v1.
  Inherited from F-S1 Rep-2.

Rep-S2-3 Corpus pinning
  Inherited from F-S1 Rep-3.

Rep-S2-4 Train-config pinning
  train_config_hash binds D1 + D3 + D5 + D10 + D13 values exactly.
  Changing any pinned value invalidates prior s2 artifacts. The hash
  includes phase_plan, hardness_ramp_id, distill_temp,
  lambda_distill_default, lambda_range, lambda_zero, range_safe_lo,
  range_safe_hi, threshold_init_multiplier, teacher_freeze_step,
  build_kind overrides, and the phase-effective lambda table.

Rep-S2-5 Pass-version pinning (extension of S1 Rep-5)
  pass_version_S2 is bumped by any change to: optimizer step
  semantics, Phase A QAT branch behavior, sequence-state forward,
  initialization rng, distillation form, threshold init formula,
  teacher freeze semantics, OR by adding/removing a phase or by
  changing D2's ramp formula. pass_version_S2 is independent of S1's
  pass_version; both are recorded in s2_report.v1.

Rep-S2-6 RFC revision pinning
  s2_report.v1 records the git sha of this RFC at report generation.
  Same discipline as S1 Rep-6.

Rep-S2-7 Per-seed isolation
  Inherited from F-S1 Rep-7. Additionally: per-build isolation. The
  s2_ternary_full, s2_fp_full, and s2_ternary_nodistill runs for the
  same seed share NO mutable state; they run as independent processes
  or as serially-isolated in-process passes that re-create model and
  optimizer state from RngStreams.

Rep-S2-8 No hidden semantic inputs
  Inherited from F-S1 Rep-8. The teacher checkpoint is NOT a hidden
  input because it is bound by teacher_checkpoint_sha and is itself
  the seed's own Phase A end snapshot, derivable from the same RngStreams.

Rep-S2-9 Threshold-init determinism
  Per D4, threshold init at the Phase B->C boundary is computed as
  0.7 * mean_abs(M.weight[r, :]) in f64 on the Phase B-end weights.
  The result is materialized as f32 and bit-equal across replays
  given the same Phase B-end checkpoint. ThresholdInitRng exists
  but consumes 0 draws; this is recorded in the run header so the
  rng-stream contract remains explicit.
```

---

# 11. Loss-term gradient-flow contract (for §3 H5 and bd-1j7 acceptance)

```text
operation s2_loss_grad_flow_suite
  input:   none (driven by fixed synthetic fixtures defined in
                gbf-experiments::s2::loss_grad_flow::synthetic)
  output:  LossGradFlowReport per §9.4

Per sub-hypothesis fixture:

  H5.1 lambda_zrouter:
    Synthetic router with 4 experts, 8 token positions, batch=1.
    router_logits initialized to two values: magnitude 1.0 and
    magnitude 100.0 (latter is the numerical-stability sub-check
    referenced in H5 prediction).
    Loss = lambda_zrouter * router_z_loss(router_logits)
    lambda_zrouter = 0.1 (D5 non-default; not 1.0).
    Expected: grad on router_logits non-zero; grad on a synthetic
    expert weight (not part of the loss) is zero.
    Stability sub-check at logit magnitude 100: forward and
    gradient finite.

  H5.2 lambda_balance:
    Synthetic router with 4 experts.
    The main gradient-flow fixture uses 8 token positions, batch=1.
    Soft top-1 routing, with a per-token soft
    distribution p_t computed under autodiff. Hard top-1 dispatch
    a_t = onehot(argmax(p_t)) is computed under stop-gradient.
    expert_usage[e] = mean_t a_t[e] (stop-gradient).
    soft_usage[e]   = mean_t p_t[e] (autodiff).
    L_balance = N_experts * sum_e expert_usage[e] * soft_usage[e]
    lambda_balance = 0.05 (D5 non-default).
    Expected:
      grad on router_logits non-zero (through soft_usage path);
      grad on the synthetic expert weight is zero;
      gradient flow reaches router_logits through soft routing
      probabilities, not through hard dispatch.
    Imbalance stability sub-check: either
      (a) use 400 token positions so hard top-1 expert_usage can be
          represented as [0.99, 0.005, 0.0025, 0.0025], or
      (b) directly inject this stop-gradient expert_usage vector into
          the balance-loss helper while separately testing hard top-1
          provenance in the 8-token fixture.
    The chosen path MUST be recorded in the fixture metadata.

  H5.3 lambda_range:
    Synthetic activation tensor of shape [batch=2, axis=8].
    safe_lo = -1.0, safe_hi = 1.0.
    Activation tensor:
      row 0 = [-2.0, -0.5, 0.0, 0.5, 2.0, 16.0, -16.0, 1.0]
      row 1 = [ 0.25, -0.25, 1.5, -1.5, 8.0, -8.0, 0.75, -0.75]
    L_range = checked range loss per §1 form.
    lambda_range = 0.1 (D5 non-default).
    Expected:
      grad on activation non-zero only at out-of-range positions;
      grad EXACTLY zero (within 1e-6) at positions inside [-1, 1];
      a checked value object is used (the test exercises both
      width-checks; the test fixture asserts a flat-slice
      implementation would FAIL the per-sample axis check).

  H5.4 lambda_zero:
    Synthetic ternary FFN matrix of shape [4, 8].
    Per-row threshold = 0.5 for every row (one-per-output-row;
    NOT global, NOT per-weight).
    Weights initialized so each row has 4 entries below threshold
    (magnitudes 0.1) and 4 entries above (magnitudes 0.7), placed
    at threshold +/- 0.2 (boundary safety per H5 epsilon rule).
    L_zero = ternary zero regularizer per §1 form.
    lambda_zero = 0.001 (D5 non-default).
    Expected:
      grad non-zero on the 4 below-threshold entries per row;
      grad EXACTLY zero on the 4 above-threshold entries per row;
      no gradient expectation is made for threshold values. In S2,
      per-row thresholds are fixed buffers initialized by D4, not
      trainable parameters. The stop-gradient indicator means threshold
      gradients are outside the H5.4 contract.

  H5.4b lambda_zero raw-diagnostic honesty:
    This is a diagnostic_subcheck inside the H5.4 FixtureResult, not
    a separate FixtureResult.
    F5_zero_loss_diagnostic_runner_fallback (§13 O5) exercises this
    report-sensitivity surface as a diagnostic-runner fallback; it is
    not a real zero_loss helper or Burn-adapter mutation.
    Same synthetic matrix and thresholds as H5.4.
    lambda_zero = 0.0.
    Expected:
      raw zero_loss is still computed, finite, and non-negative;
      weighted zero_loss is exactly 0.0;
      no non-zero gradient expectation is made for the weighted loss.

  H5.5 lambda_distill:
    Synthetic batch=2, vocab=4 student/teacher logits.
    teacher_logits is an independently initialized tensor with
    requires_grad = false. Its values differ from student_logits by a
    fixed offset pattern chosen so KL is non-trivial.
    lambda_distill = 0.5 (D5 non-default; not 1.0).
    distill_temperature = 1.0 (D5 non-default; not 2.0).
    Expected:
      grad on student_logits non-zero;
      grad on teacher_logits EXACTLY 0.0. If the autodiff backend
      represents detached tensors by omitting a gradient entry, the
      artifact encoder records that absence as 0.0 and additionally
      sets teacher_grad_entry_present = false.

Termination:
  s2_loss_grad_flow_suite is total. NaN/Inf during forward or
  backward of any fixture is escalated to the LossGradFlowReport
  field FixtureResult.numerical_stability_passed = false, and
  overall_passed = false.

Failure mode:
  overall_passed = false => H5 verdict Refuted.
```

---

# 12. Decision protocol

```text
S2 closure (bd-1xqf) requires:
  0. D4's inherited scale semantics citation is resolved. The literal
     placeholder "<INSERT EXACT BEAD/RFC SECTION>" MUST NOT appear in the
     committed closure RFC.
  1. All 5 seeds, all three full-build types {s2_ternary_full,
     s2_fp_full, s2_ternary_nodistill}: run completion = Completed
     (D12). Plus s2_ablation seed 0.
  2. s2_report.v1 emitted with R-S2-Predictions verified by git history.
  3. Decision in {ProceedToS3, ProceedToS3-with-distill-review}.
  4. baseline_self_hash_carried_from_s1 present and matching the on-disk
     S1 baseline.
  5. oracle_re_run_passed = true (D7 oracle suite re-run under S2 binary).
  6. ablation phase_a_eq_ablation = true for seed 0.
  7. loss_grad_flow_self_hash present and overall_passed = true.
  8. linearstate_smoke_self_hash present and smoke_passed = true.
  9. phase_transition_integ_self_hash present and integ_passed = true.
  10. api_drift_check_passed = true, with snapshot hashes recorded.
  11. falsification_s2_passed = true for all six deliberately-broken
      S2 implementations in §13 O5.

S2 closure is forbidden when:
  Any of:
    Decision::Halt(_), Decision::Investigate(_),
    missing pre-registration,
    any seed completion = DivergedAt(_),
    oracle_re_run_passed = false,
    ablation phase_a_eq_ablation = false,
    loss_grad_flow overall_passed = false,
    linearstate_smoke smoke_passed = false,
    phase_transition_integ integ_passed = false,
    api_drift_check_passed = false,
    any required artifact missing or self-hash invalid.

If Decision = ProceedToS3-with-distill-review:
  Open follow-up bead in the F4 epic to revisit lambda_distill,
  distillation_temperature, or the Phase C ramp; cite the H3 Refuted
  evidence. This does NOT add a structural slice-graph blocker
  between S2 and S3 (unlike S1's optional T12.5 prereq). H3 Refuted
  on Toy0 is consistent with successful QAT survival; S3 should
  proceed.
```

---

# 13. Proof obligations

```text
O1  Pre-registration provability
    "Pre-registered predictions" section content of S2-report.md must
    appear in git history strictly before any S2 result artifact
    commit. CI script asserts:
      1. predictions_section_hash matches the exact normalized markdown
         section in predictions_commit;
      2. predictions_commit is a strict ancestor of first_result_commit;
      3. first_result_commit is the earliest commit that introduces any
         non-null S2 result artifact self-hash or completed/diverged
         run-product field. Schema templates, empty placeholders, and
         pre-registration-only report drafts do not count as result
         artifacts.

O2  Determinism (per build, per seed)
    Same seed + same corpus_*_sha + same train_config_hash + same
    pass_version_S2 + same device_profile + same dependency lockfile
    + same teacher_freeze_step
    => bit-identical safetensors AT THE FINAL STEP and at every
    phase boundary checkpoint.

    v1 CI closure test:
      run seed 0 of s2_ternary_full twice and assert byte equality
      at all four phase boundary snapshots and at the final step.

O3  Measurement-oracle correctness (re-run under S2 binary)
    The S1 D7 measurement-oracle suite (O-metric-0..O-metric-4) is
    re-run under the S2 binary and must produce metric_oracle_passed
    = true. This guards against a Phase B/C/D code path silently
    altering the bpc primitive or the shuffle. (Required for closure.)

O4  Ablation match (S2-binary edition)
    For seed 0 of s2_ternary_full vs s2_ablation:
    phase_a_eq_ablation = true. (Required for closure.)
    This is the H4 invariant; it is a strict re-assertion under the
    S2 binary, distinct from S1 H4.

O5  Falsification suite (S2-specific)
    Six deliberately-broken implementations must each produce the
    expected verifier result. Some are hypothesis-level Refuted
    verdicts; config-violating cases must be rejected before training
    and must not rely on empirical gap changes.

      F1-broken-S2  phase_b_skips_ternary:
                    The phase scheduler advances from B to C but never
                    flips expert_qat to Soft/Hard. Phase log records
                    HardnessTriple.expert_qat = Off throughout C and D.
                    Expected: H1 Refuted (PL-4 invariant violated).

      F2-broken-S2  phase_d_unfreezes_teacher:
                    The teacher freeze is reverted at the C->D
                    boundary. teacher_requires_grad becomes true
                    after step 8000.
                    Expected: H1 Refuted (teacher_frozen invariant
                    PL-3 violated).

      F3-broken-S2  distill_temperature_inverted:
                    distill_temperature = 1/2.0 = 0.5 instead of 2.0.
                    Expected: the S2 train-config validator rejects
                    the run before training because D3 pins
                    distillation_temperature = 2.0. The falsification
                    suite must not rely on an empirical H2/H3 gap
                    failure for this case.

      F4-broken-S2  F4_threshold_per_weight_structural_mask_fixture:
                    Structural fallback fixture for
                    ThresholdPlan = OneThresholdPerWeight (forbidden
                    by D4 and CLAUDE.md "Training Loss Beads").
                    The fixture assigns non-uniform per-weight
                    thresholds chosen so that at least one weight is
                    below the illegal per-weight threshold but above the
                    legal row threshold, and at least one weight is
                    above the illegal per-weight threshold but below the
                    legal row threshold. Expected: H5.4 sub-hypothesis
                    Refuted because the observed structural mask
                    deviates from the legal per-row mask. This is not a
                    real zero_loss helper or Burn-adapter mutation.

      F5-broken-S2  F5_zero_loss_diagnostic_runner_fallback:
                    Diagnostic-runner fallback for the case where the
                    zero-loss helper would short-circuit and return 0.0
                    without computing the L1 sum when lambda_zero = 0.
                    Such behavior would violate the "raw weighted-loss
                    helpers must validate finite/non-negative raw
                    diagnostics even when the configured weight is zero"
                    rule (CLAUDE.md "Training Loss Beads"). Expected:
                    the raw-diagnostic zero-loss fixture is Refuted
                    because raw_loss is missing or incorrectly reported
                    as computed. This row proves H5.4b report
                    sensitivity; it is not a real zero_loss helper or
                    Burn-adapter mutation, and it is not a
                    weighted-gradient failure: with lambda_zero = 0,
                    weighted gradients are expected to be zero.

      F6-broken-S2  F6_linearstate_structural_smoke_fallback:
                    Structural smoke fallback for a dead
                    LinearState recurrence/readout gradient. Expected:
                    H6 Refuted (LS-2 violated). This row proves H6
                    smoke-report sensitivity to the structural fixture;
                    it is not a mutation of the public LinearState Burn
                    adapter.

    These are unit tests against the S2 framework, not actual S2
    runs.
    Required test files:
      gbf-experiments/tests/falsification_s2/f1.rs
      gbf-experiments/tests/falsification_s2/f2.rs
      gbf-experiments/tests/falsification_s2/f3.rs
      gbf-experiments/tests/falsification_s2/f4.rs
      gbf-experiments/tests/falsification_s2/f5.rs
      gbf-experiments/tests/falsification_s2/f6.rs
    Gated by the test-only `falsify` feature on gbf-experiments so
    the broken substitutes cannot leak into a release build.

O6  Hash round-trip
    Every emitted s2_*.v1 artifact round-trips through canonical JSON
    with self-hash equality.

    For JSONL artifacts, each line is canonicalized independently using
    S1CanonicalJson, line order is preserved, and the artifact self-hash
    covers the ordered canonical line byte sequence plus the header.

O7  Outcome algebra totality
    Every observable combination of binary H1..H6 verdicts, per-seed
    completion states across all three build types, and suspicion
    thresholds maps to exactly one S2Outcome variant under §8.

O8  No hidden inputs
    s2 artifacts depend only on:
      corpus_train, corpus_val (sha256-pinned)
      model_config (Toy0 pinned by T14.1 reference instance)
      train_config_S2 (D1, D3, D13 pinned)
      seed
      build_kind
      pass_version_S2
      build_config_hash
      rust_toolchain_hash
      dependency lockfile hash
      gbf-train + gbf-model + gbf-experiments pinned dependency set
    No env-var, no host-clock, no network, no stdin.

O9  Per-seed and per-build isolation
    Seed s and seed s' produce independent run products; same for
    different build_kind values.

    CI smoke checks:
      1. at least two of the five seeds in s2_ternary_full produce
         different final_checkpoint_sha;
      2. running s2_ternary_full and s2_fp_full sequentially in
         either order produces identical per-build per-seed hashes
         (no in-process state leak).

O10 Closure gate
    bd-1xqf close is reachable iff Decision in {ProceedToS3,
    ProceedToS3-with-distill-review}.

O11 Public API non-drift
    gbf-model::qat public symbols and gbf-model::sequence::LinearStateBlock
    public symbols are unchanged from S1 closure (bd-12pl). A CI grep
    asserts the symbol list against a pinned snapshot file under
    gbf-experiments/snapshots/s1_qat_public_api.txt and
    gbf-experiments/snapshots/s1_linearstate_public_api.txt. Any
    drift requires either:
      1. an explicit S2 allowed-drift entry naming the new/changed symbol,
         plus a bumped snapshot hash; or
      2. an upstream feature bead.

    S2AllowedApiDriftV1 := {}

    Because S2AllowedApiDriftV1 is empty, any gbf-model::qat or
    gbf-model::sequence::LinearStateBlock public symbol drift in v1
    fails O11 unless this RFC is amended.

O12 Distillation determinism
    Same student_logits + teacher_logits + temperature + lambda_distill
    => bit-identical distill_loss bytes (under S1CpuDeterministic).
    The s2 distill helper is wrapped in a CI smoke that re-invokes
    on the same byte-pinned inputs and asserts byte equality.

O13 Gradient-flow non-default value enforcement
    The s2 loss-grad-flow suite (§11) MUST set
    FixtureResult.non_default_value_used = true for every H5.1..H5.5
    fixture. Diagnostic subchecks whose purpose is specifically to test
    zero-weight raw-loss honesty, such as H5.4b, are exempt from this
    fixture-level rule but MUST be attached to a fixture that also has
    a non-default sweep of the same loss formula.
```

---

# 14. Minimal end-to-end theorem

```text
Theorem S2OutcomeTotalityAndSoundness:

Given:
  corpus manifest with valid sha256 pinned in
    fixtures/corpora/tinystories.toml (S1 inheritance)
  Toy0 reference instance (T14.1 closed, bd-1r6k)
  TrainConfigS2 pinned per D1 + D3 + D4 + D5 + D10 + D13 and the
    training-loss unit choice in §1
  pass_version_S2 fixed by gbf-experiments HEAD at S2 PR merge
  S1 baseline artifact present with valid baseline_self_hash

If the S2 verifier is given a complete or early-failure S2 report with:
  - run products or explicit NotEvaluatedDueToPriorGate statuses,
  - H1..H6 verdict/status records,
  - loss-grad-flow, LinearState smoke, phase-integration, metric-oracle,
    API-drift, ablation, score, and preregistration verifier records,
  - self-hash-valid artifacts for every artifact that was reached,
  - pre-registered predictions in pre-run git history when any result
    artifact exists,

Then:
  Each of H1, H2, H3, H4, H5, H6 has a defined HypothesisStatus.

  For closure-candidate outcomes in
  {Pass-clean, Pass-with-distill-warn}, every hypothesis status is
  binary: Confirmed or Refuted.

  For early-failure outcomes, hypotheses not reached by the state
  machine may be NotEvaluatedDueToPriorGate(reason), but the prior
  gate that prevented evaluation must itself have a binary Refuted
  status or an explicit failed verifier record.

  S2Outcome is exactly one of:
    Pass-clean
    Pass-with-distill-warn  (H3 Refuted; remaining gates pass)
    Fail-substrate          (H1 Refuted or any seed diverged on any
                            of the three full-build types)
    Fail-gap                (H2 Refuted, non-suspicious)
    Fail-suspicious         (median bpc below 0.5 floor on any of
                            the three scoring builds)
    Fail-phase              (H4 Refuted)
    Fail-loss-grad-flow     (H5 Refuted)
    Fail-linearstate        (H6 Refuted)
    Fail-phase-integration  (D8 phase-transition integration regressed)
    Fail-falsification      (O5 falsification suite failed)
    Fail-api-drift          (O11 public API drift detected)
    Fail-metric             (D7 oracle re-run regressed)
    Fail-preregistration    (O1 pre-registration proof failed)
    Fail-artifact           (required artifact missing or self-hash invalid)
    Fail-incomplete         (required non-gating artifact missing without
                            an explaining prior failure)

  Decision is unique under the dispatch rule of §8.

Corollary S2PassSoundness:

  If S2Outcome in {Pass-clean, Pass-with-distill-warn}, S2 has
  produced these verified knowledge claims:
    - The F4 phase scheduler advances Toy0 through A->B->C->D without
      divergence under the pinned protocol.
    - Toy0 ternary survives QAT: bpc(ternary) - bpc(fp) <= 0.5 bpc
      per seed under the matched protocol.
    - Phase A under the S2 binary is uncontaminated by Phase B/C/D
      code (canonical tensor payload byte-equal to ablation).
    - Standard loss-term gradients flow into intended parameter sets
      and stop at intended boundaries; teacher logits receive zero
      gradient under distillation.
    - Burn LinearStateBlock has finite, nonzero, deterministic
      gradients on the pinned tiny fixture; later slices may rely
      on this autodiff path.

  If S2Outcome = Pass-with-distill-warn, S2 verifies that H3 was
  refuted: distillation worsened at least one seed beyond the
  pre-registered tolerance, while H2 still passed. A follow-up bead is
  created in the F4 epic.

  If S2Outcome = Fail-gap, S2 verifies that the ternary-vs-fp gap
  exceeded the 0.5 bpc gate on at least one seed under the pinned
  protocol; it does not verify that ternary is unviable in general.

  If S2Outcome = Fail-substrate, S2 verifies that the S2 training
  substrate or the QAT/distillation pipeline failed; no downstream
  capacity, phase, distillation, or gradient-flow claim is licensed
  unless that hypothesis has an explicit binary verdict in the report.

  If S2Outcome = Fail-phase, S2 verifies that Phase A is not clean
  with respect to the seed-0 ablation comparison under the S2 binary.

  If S2Outcome = Fail-loss-grad-flow, S2 verifies that at least one
  of the standard loss terms violates its declared gradient contract.

  If S2Outcome = Fail-linearstate, S2 verifies that the Burn autodiff
  path through LinearStateBlock fails the smoke test; S5 is blocked.

  If S2Outcome = Fail-metric, S2 verifies that the D7 oracle suite
  regressed under the S2 binary; no reported bpc gap should be trusted.

  If S2Outcome = Fail-suspicious, S2 verifies that the suspicious-low-bpc
  sentinel fired on at least one of the three scored full builds and that
  split/leakage/metric audit is required.

Not proven:
  charset_v1 normalization (S3)
  ReferenceModelBundle export (S3)
  ArtifactOracle round-trip (S3)
  v0_success workload pass (S3)
  Project Gutenberg generalization (S4)
  multi-timescale LinearState (S5)
  BoundedKv comparison (S5)
  Game Boy ROM fit (S6)
  MoE benefit (S7)
  StructuredWidthGates (S8)
```

---

# 15. Implementation crate layout

Scope(F-S2) is hosted inside the existing `gbf-experiments` workspace
crate alongside `s1::*`, plus extensions in `gbf-train` and tests in
`gbf-test`. This section pins the public surface that the hypotheses
and proof obligations rely on. Module names within each crate are
illustrative; only items tagged **Required** are normative.

## 15.1 Crate map

```text
gbf-policy
  Required  ModelSizeProfile::Toy0 reference instance (T14.1, bd-1r6k).
            Unchanged from S1.

gbf-model
  Required  qat::ternary::TernaryLinearQat with PerOutputRow scale
            granularity, Q8.8 scale format, OneThresholdPerOutputRow
            threshold plan, Ternary2 weight encoding (D4).
  Required  qat::activation::ActFakeQuant under QuantHardness control.
  Required  qat::norm::NormApproxQat under QuantHardness control.
  Required  sequence::LinearStateBlock with Fixed(0.5) decay (bd-tnb,
            unchanged from S1).
  Required  Public API non-drift snapshot (O11) covering qat::* and
            sequence::LinearStateBlock symbols.

gbf-train
  Required  Phase scheduler covering Phase A, B, C, D transitions
            with QuantHardness ramping per D2.
  Required  AdamW config helper exposing the D13 hyperparameters
            unchanged.
  Required  loss::distillation module with the canonical T^2 KL form
            per D3 / §6 / §1.
  Required  loss::config::LossConfig with TOML round-trip and per-phase
            effective lambda computation per D5 + D10. Inert lambdas
            (lambda_balance, lambda_zrouter, lambda_switch) on Toy0
            are zeroed by phase-effective composition.
  Required  teacher::FrozenTeacher detach + fingerprint surface per D3.
  Required  Burn adapter feature `burn-adapter`.
            No `qat-fp-only` feature participates in S2 closure.
            The s2_fp_full logical build is selected by runtime
            S2BuildKind and applies QuantHardnessOverride::AllOff
            inside the same compiled binary as s2_ternary_full.
  Required  logging::TrainingLogEmitter must emit
            phase_transition, teacher_freeze, loss_step, and
            distill_step events that s2_phase_log.v1 consumes.

gbf-data
  Required  TinyStoriesManifest reader (S1 inheritance, unchanged).

gbf-foundation
  Required  Hash256, sha256 helper (S1 inheritance).

gbf-artifact
  Required  CanonicalTensor, CanonicalTensorPayloadHash, QuantSpec,
            TernaryWeightPlan, ScaleGranularity, ScaleFormat,
            ThresholdPlan, WeightEncoding (S1 inheritance + D4 pins).

gbf-experiments
  Owns Scope(F-S2) end-to-end. Required modules:

    gbf_experiments::s2::manifest
      Re-export of s1 manifest reader (no duplication).

    gbf_experiments::s2::rng
      Re-export of s1 RngStreams + ThresholdInitRng definition.

    gbf_experiments::s2::device_profile
      Re-export of s1::device_profile S1CpuDeterministic enforcement.

    gbf_experiments::s2::run
      s2_train_run operation per §5. Emits CompletedRunProductS2 or
      DivergedRunProductS2. Produces s2_phase_log.v1 entries,
      checkpoints at phase boundaries, distillation log entries.

    gbf_experiments::s2::distill
      s2_distill_step operation per §6. Wraps
      gbf_train::loss::distillation with the S2 invariant checks
      (Di-Ok-* postconditions).

    gbf_experiments::s2::score
      Re-export of s1::score::s1_score_bpc with S2 wrapper that
      tags the output as s2_score.v1 and adds
      threshold_stats/scale_stats sidecars for ternary builds.

    gbf_experiments::s2::ablation
      Seed-0 ablation comparator over CanonicalTensorPayloadHash;
      emits s2_ablation.v1.

    gbf_experiments::s2::gap
      Computes per-seed gap_ternary_vs_fp and gap_nodistill_vs_fp
      from the three score artifacts.

    gbf_experiments::s2::loss_grad_flow
      The five sub-hypothesis fixtures and the
      s2_loss_grad_flow_suite operation per §11. Submodule
      `synthetic` hosts the minimal router/expert fixtures used
      for H5.1 and H5.2.

    gbf_experiments::s2::linearstate_smoke
      The FIXTURE_V1 definition and the
      s2_linearstate_grad_smoke operation per D9 / §3 H6.

    gbf_experiments::s2::phase_transition_integ
      The s2_phase_transition_integration operation per D8.

    gbf_experiments::s2::oracle_re_run
      Wrapper that invokes s1::oracle (D7 measurement-oracle suite)
      under the S2 binary and produces oracle_re_run_passed.

    gbf_experiments::s2::schema
      Type definitions, S1CanonicalJson encoder reuse, DomainHash
      reuse, and self-hash round-trip helpers for:
        s2_phase_log.v1, s2_score.v1,
        s2_distillation_log.v1, s2_loss_grad_flow.v1,
        s2_linearstate_grad_smoke.v1,
        s2_phase_transition_integration.v1, s2_ablation.v1,
        s2_report.v1.

    gbf_experiments::s2::report
      s2_report.v1 emitter and outcome-algebra dispatcher
      implementing §8. Authors front-matter, validates R-S2-Decision,
      R-S2-AllSeeds, R-S2-Self-Hash, R-S2-Predictions,
      R-S2-AllHypotheses, and binds the pre-registration commit
      history per O1.

    gbf_experiments::s2::cli
      Public entrypoint(s) for replay. The CLI surface is the
      canonical invocation point referenced by §10 Rep-S2-1 and
      §12 closure.

gbf-cli
  Required  Subcommand `gbf s2 ...` dispatching into
            gbf_experiments::s2::cli. The pre-registration check,
            the determinism check, and the closure script all shell
            into this surface.

gbf-test
  Required  tiny_model fixture (T10.1 / bd-mov, unchanged from S1
            inheritance; S2 reuses for D8 phase transition integration
            test).
```

## 15.2 Test layout

```text
gbf-experiments/tests/falsification_s2.rs
gbf-experiments/tests/falsification_s2/*.rs
  Six S2-specific files per §13 O5; gated by the existing
  `falsify` feature. Co-resides with the S1 falsification suite;
  each must independently pass.

gbf-experiments/tests/loss_grad_flow_s2.rs
gbf-experiments/tests/loss_grad_flow_s2/*.rs
  Per-sub-hypothesis fixtures for H5.1..H5.5.

gbf-experiments/tests/linearstate_smoke_s2.rs
  FIXTURE_V1 forward + backward determinism gate.

gbf-experiments/tests/phase_transition_integ_s2.rs
  D8 / bd-14k integration on the tiny_model fixture.

gbf-experiments/tests/canonical_json_s2.rs
gbf-experiments/tests/canonical_json_s2/*.rs
  Round-trip tests for every s2_*.v1 schema (O6).

gbf-experiments/tests/integration_s2.rs
  End-to-end smoke run against the in-repo tiny fixture corpus
  (NOT TinyStories) used in CI to gate determinism (O2) and
  per-seed/per-build isolation (O9). Sized so a 5-seed * 3-build
  run completes within the project standard test timeout.

gbf-experiments/tests/oracle_re_run_s2.rs
  Invokes the S1 D7 oracle suite under the S2 binary; satisfies O3.

The full TinyStories run is gated behind a separate CI job, but
bd-1xqf closure requires that job's artifacts and s2_report.v1, not
merely the tiny-fixture smoke run.
```

## 15.3 Artifact paths

All run artifacts are written under the repository-root
`experiments/S2/` tree, partitioned by `{build_kind}/seed-{seed}/`.
The report is written to `docs/experiments/S2-report.md`.

## 15.4 Canonical replay commands

```text
cargo run --release -p gbf-cli --features s2-full -- s2 replay-full \
  --manifest fixtures/corpora/tinystories.toml \
  --pass-version <pass_version_S2_pinned_in_report> \
  --seed-list 0,1,2,3,4 \
  --builds s2_ternary_full,s2_fp_full,s2_ternary_nodistill \
  --device-profile S1CpuDeterministic

cargo run --release -p gbf-cli \
  --no-default-features \
  --features s2-ablation \
  -- s2 replay-ablation \
  --manifest fixtures/corpora/tinystories.toml \
  --pass-version <pass_version_S2_pinned_in_report> \
  --seed-list 0 \
  --device-profile S1CpuDeterministic
```

Under the same machine + OS + pinned Burn version + pinned dependency
lockfile + S1CpuDeterministic, this command reproduces
`experiments/S2/**` byte-for-byte per Rep-S2-1.

Optional non-normative subcommands:

```text
gbf s2 oracle-re-run        runs the D7 oracle suite under the S2 binary
gbf s2 verify-determinism   replays seed 0 of s2_ternary_full and
                            asserts byte equality at all phase
                            boundaries
gbf s2 grad-flow            runs the §11 loss-grad-flow suite
gbf s2 linearstate-smoke    runs the H6 LinearState smoke
gbf s2 phase-integ          runs the D8 integration test
```

## 15.5 Workspace registration

No new workspace members. `gbf-experiments` already exists from S1 and
is amended to add the `s2::*` module subtree. No `qat-fp-only` feature
is added for S2 closure; the fp comparator is selected exclusively by
runtime `S2BuildKind::s2_fp_full` and `QuantHardnessOverride::AllOff`.
No other Cargo.toml changes are required except feature forwarding for
`gbf-experiments/s2-full` and `gbf-experiments/s2-ablation` as specified
in §16.

---

# 16. Build configurations and feature flags

Four S2 build configurations participate in the S2 contract. The S1 builds
remain registered for replay of S1 artifacts but are not S2 closure builds
unless explicitly named.

## 16.1 S2-build-T — "s2-ternary"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments
Active features (workspace-resolved):
  gbf-experiments/default
    expands to gbf-experiments/phase-a + gbf-experiments/s2-full
  gbf-train/qat (default-on, S1 inheritance)
  gbf-train/burn-adapter
Behavior:
  All QAT codepaths present and active per D2. Phase scheduler runs
  the full A->B->C->D plan. Distillation enabled in C and D.
Build identity tag (recorded in s2_phase_log.v1.build_kind):
  build_kind = "s2_ternary_full"
```

## 16.2 S2-build-F — "s2-fp"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments
Active features:
  gbf-experiments/default (same as s2-ternary)
Behavior:
  Same compiled binary as s2-ternary. The s2_fp_full build is
  selected by runtime BuildKind, which sets
  QuantHardnessOverride::AllOff. The QAT codepaths are compiled in
  but exercise the all-Off branch.
Build identity tag:
  build_kind = "s2_fp_full"
```

## 16.3 S2-build-N — "s2-ternary-nodistill"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments
Active features:
  gbf-experiments/default (same as s2-ternary)
Behavior:
  Same compiled binary as s2-ternary. The s2_ternary_nodistill build
  is selected by runtime BuildKind, which sets the LossConfig override
  lambda_distill_default = 0.0. The raw distill_loss diagnostic is
  STILL computed and recorded (per S2-Run-Ok-7 + DL-2). The teacher
  is still frozen at step 4000 (keeping the run protocol identical
  except for the lambda); this isolates "does distillation help" from
  "does freezing perturb training".
Build identity tag:
  build_kind = "s2_ternary_nodistill"
```

## 16.4 S2-build-A — "s2-ablation"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --no-default-features \
    --features s2-ablation
Active features:
  gbf-experiments/s2-ablation
    expands to gbf-train/qat-ablation + gbf-train/burn-adapter
Behavior:
  QAT codepaths are compiled out via `qat-ablation`. Run executes
  Phase A only (steps 1..=4000) and produces a step-4000 checkpoint
  for H4 comparison.
Build identity tag:
  build_kind = "s2_ablation"
```

## 16.5 S1 builds (carried)

```text
S1-build-A "phase_a"     unchanged from F-S1 §16.1; not referenced by
                         S2 closure but kept available for replay of
                         S1 closure artifacts.
S1-build-B "ablation"    unchanged from F-S1 §16.2.
```

## 16.6 Feature flag contract

```text
gbf-train/qat              default-on; gates all QAT codepaths
                           (S1 inheritance).
gbf-train/qat-ablation     mutually exclusive with `qat`; replaces
                           QAT codepaths with stubs that compile to
                           a no-op (or compile_error! if invoked).
                           (S1 inheritance.)
gbf-train/qat-fp-only      Not defined by S2. If a later experiment adds
                           such a feature, it MUST NOT be used by any
                           S2 closure build or replay command.
gbf-train/burn-adapter     gates Burn autodiff backend wiring.
                           Required for all three S2 full builds and
                           the S2 ablation build.
gbf-experiments/phase-a    forwards to gbf-train/qat and burn-adapter.
                           (S1 inheritance.)
gbf-experiments/s2-full    NEW. Activates s2::* modules and the
                           s2 CLI subcommand.
gbf-experiments/s2-fp-only Not used for S2 closure. The fp logical build
                           is selected by runtime S2BuildKind.
gbf-experiments/s2-nodistill Not used for S2 closure. The nodistill
                           logical build is selected by runtime
                           S2BuildKind.
gbf-experiments/s2-ablation NEW. Forwards to gbf-train/qat-ablation
                           + gbf-train/burn-adapter; sets the build
                           identity tag.
                           The gbf-train dependency MUST be declared
                           with default-features = false for this
                           feature path, so gbf-train/qat is not
                           enabled accidentally.
gbf-cli/s2-full            NEW. Forwards to gbf-experiments/s2-full and
                           exposes `gbf s2 replay-full`.
gbf-cli/s2-ablation        NEW. Forwards to gbf-experiments/s2-ablation
                           and exposes `gbf s2 replay-ablation`.
gbf-experiments/ablation   S1's ablation feature (carried unchanged).
gbf-experiments/falsify    test-only; gates the F1..F6 (S1) and
                           F1-broken-S2..F6-broken-S2 broken substitutes
                           used by both falsification suites.

Mutual exclusion enforcement:
  gbf-train must compile_error! when both `qat` and `qat-ablation`
  are enabled. S2 closure full builds use `qat`; the S2 ablation
  build uses `qat-ablation`.

  gbf-experiments must compile_error! if `s2-full` and `s2-ablation`
  are both enabled. Full-build identity is runtime data, not a Cargo
  feature.
```

## 16.7 Determinism budgets

```text
All five builds run under S1CpuDeterministic (S1 §5). The runner sets
each variable in env_exact to its pinned value, and unsets every
variable not present in env_exact (env_forbidden_unless_listed = true),
before any tensor allocation:

  BURN_NDARRAY_NUM_THREADS=1
  BURN_DETERMINISTIC=1
  OMP_NUM_THREADS=1
  RAYON_NUM_THREADS=1

Violation aborts the run with a non-zero exit before training begins.
```

## 16.8 Pre-registration CI

```text
scripts/s2_preregistration_check.sh implements §13 O1:
  1. predictions_section_hash matches the markdown section in
     predictions_commit, recomputed using S1CanonicalJson normalization
     and exact byte equality of body markdown;
  2. predictions_commit is a strict ancestor of first_result_commit;
  3. first_result_commit is the earliest commit introducing any non-null
     S2 result artifact self-hash, completed/diverged run-product field,
     score value, or hypothesis verdict derived from an S2 result.
     Schema templates, null placeholders, and pre-registration-only
     report drafts do not count as result artifacts.
Exit non-zero on any violation. Closure of bd-1xqf is forbidden while
this script exits non-zero.
```

## 16.9 CI gates that block bd-1xqf closure

```text
cargo test -p gbf-experiments
cargo test -p gbf-experiments --features falsify --test falsification_s2
cargo test -p gbf-experiments --test loss_grad_flow_s2
cargo test -p gbf-experiments --test linearstate_smoke_s2
cargo test -p gbf-experiments --test phase_transition_integ_s2
cargo test -p gbf-experiments --test canonical_json_s2
cargo test -p gbf-experiments --test integration_s2
cargo test -p gbf-experiments --test oracle_re_run_s2

# Per CLAUDE.md "Training Loss Beads":
#   "If a loss claim depends on Burn autodiff, closure must cite a
#   feature-enabled gate such as
#   cargo test -p gbf-train --features burn-adapter -- <loss_test>."
cargo test -p gbf-train --features burn-adapter -- linear_state::gradient
cargo test -p gbf-train --features burn-adapter -- phase::linear_state_hardness
cargo test -p gbf-train --features burn-adapter -- distillation
cargo test -p gbf-train --features burn-adapter -- loss::config

cargo build -p gbf-experiments --no-default-features --features s2-ablation
cargo build -p gbf-experiments --features s2-full

scripts/s2_preregistration_check.sh
scripts/s2_determinism_check.sh
  (replays seed 0 of s2_ternary_full and asserts byte equality of
   safetensors at every phase boundary AND of phase_log_self_hash;
   satisfies O2)
scripts/s2_isolation_check.sh
  (asserts at least two of the five seeds in s2_ternary_full produce
   different final_checkpoint_sha, and that running [ternary, fp]
   vs [fp, ternary] produces identical per-build per-seed hashes;
   satisfies O9)
scripts/s2_api_drift_check.sh
  (greps gbf-model::qat and gbf-model::sequence::LinearStateBlock
   public symbols against pinned snapshot files; satisfies O11)
scripts/s2_distill_determinism_check.sh
  (re-invokes s2_distill_step on byte-pinned inputs and asserts
   output byte equality; satisfies O12)
```

---

# 17. Ambiguity ledger

|  ID | Ambiguity | Chosen path | Clarifying question | Suggested final decision |
| --: | --- | --- | --- | --- |
| AS2-1 | Phase budget split: should the 10000 steps be evenly distributed (2500 each) or weighted toward Phase A and Phase C? | A=4000, B=1000, C=3000, D=2000 (D1) | Why not give Phase A more, since it produces the teacher? | The S1 closure showed Toy0 quality saturates well before 10000 steps in Phase A; 4000 steps is sufficient teacher quality, leaving budget where ternarization actually risks divergence. Per-seed variance in Phase A teacher quality is recorded and reported. |
| AS2-2 | Use the S1 closure checkpoint (10000-step Phase A) as the teacher vs. a fresh Phase A teacher inside the S2 binary | Fresh per-seed Phase A teacher inside S2 binary | Wouldn't the S1 teacher be higher quality and save budget? | Yes, but it conflates two binaries' Phase A semantics and weakens H4. The S2 binary contains Phase B/C/D code; only an in-binary teacher proves the S2 binary's Phase A is clean enough to bind student gradients to. |
| AS2-3 | QuantHardness ramp shape inside Phase C: linear vs cosine vs piecewise step | Piecewise step at 1000 / 2000 boundaries (D2) | Why not anneal smoothly? | QuantHardness is a discrete enum with three variants (Off/Soft/Hard) at the gbf-model::qat level. A continuous ramp between two discrete values is undefined at the model surface. A piecewise schedule is honest about the discrete transitions, and the soak-then-ramp-then-hard pattern is a known-good QAT cadence. |
| AS2-4 | lambda_distill default value | 1.0 (D3) | Why not 0.5? | 1.0 keeps distill_loss directly comparable across builds (no implicit scaling). The H3 weak-form gate accommodates the case where distillation does not strictly help on Toy0. |
| AS2-5 | distillation_temperature default | 2.0 (D3) | Why not 1.0 or 4.0? | 2.0 is the default already pinned in gbf-train::loss::distillation::DEFAULT_DISTILLATION_TEMPERATURE and matches the existing distillation contract. Falsification F3-broken (T=0.5) is expected to be rejected by the train-config validator before training, not to rely on empirical H2/H3 behavior. |
| AS2-6 | Threshold init: per-row vs per-matrix vs per-weight | One per output row (D4) | Per-weight gives finer control. | Forbidden by CLAUDE.md "Training Loss Beads" and by the deployed PerOutputRow scale granularity. Falsification F4-broken is represented by `F4_threshold_per_weight_structural_mask_fixture`, a structural mask fixture expected to Refute H5.4 without claiming a real zero-loss helper/Burn-adapter mutation. |
| AS2-7 | Threshold init multiplier: 0.7 vs other values like 0.5 or mean_abs * sqrt(2 / pi) | 0.7 * mean_abs(row) (D4) | Why exactly 0.7? | Matches the 1-bit LLM literature convention; alternative values are an A-block follow-up bead (F4 epic), not an S2 amendment. The exact value is part of train_config_hash so a change forces a new pass_version_S2. |
| AS2-8 | s2-fp build: should it really run distillation from its own teacher, or be a strict no-distill fp baseline? | Run distillation from its own Phase A teacher (D6) | Doesn't this conflate "fp matters" with "distillation matters"? | The H2 gap measures the QUANTIZATION impact alone. To isolate quantization from distillation, both ternary and fp builds must distill from comparable teachers. The no-distill ternary control (s2_ternary_nodistill, H3) isolates distillation impact separately. |
| AS2-9 | qat-fp-only feature: compile out QAT or guard at runtime? | Guard at runtime (16.2, 16.6) | A compile-out is more honest. | A compile-out makes the s2-fp binary smaller and structurally different from s2-ternary, breaking the "same binary surface, different config" property. Runtime guard preserves binary comparability. The s2-ablation build is the place for compile-out (16.4). |
| AS2-10 | LinearState smoke: full S5 multi-timescale vs. Fixed(0.5) only | Fixed(0.5) only (D9) | Multi-timescale is also load-bearing for S5. | Multi-timescale is owned by T12.5 / S5; including it here would expand S2's scope and risk binding S5 to an immature decay-policy contract. The smoke covers the autodiff path through the existing closed bd-tnb implementation, which is sufficient evidence that QuantHardness controls reach the LinearState boundary. |
| AS2-11 | Phase transition integration test on tiny_model fixture vs Toy0 | tiny_model fixture (T10.1, D8) | Why not run the integration test on Toy0 itself? | Cost: D8 is a fast unit-style integration test (boundaries at 0/10/20/30/40/50 steps), not a 10000-step run. Reusing Toy0 would conflate the integration test (D8) with H1 (which already proves phase transitions over the real budget). |
| AS2-12 | H3 (distillation effectiveness) closure-gating vs non-closure-gating | Non-closure-gating (H3 statement) | If distillation hurts, isn't the F4 contract broken? | Distillation may simply not help on Toy0 because Toy0 is too small for the teacher's logit signal to add meaningful information beyond the lm_loss. That is a quality observation about Toy0, not a substrate failure. The H3 weak form catches "distillation actively hurts"; the strong form is informational. |
| AS2-13 | Should H6 LinearState smoke be a closure gate or a sanity check? | Closure gate (H6 in §3) | Aren't sequence-state correctness tests already in bd-tnb? | bd-tnb closed against scalar guards. H6 specifically asserts the autodiff path under the S2 binary, which adds Phase B/C/D code that may inadvertently disturb Burn autodiff registration. S5's BoundedKv vs LinearState A/B requires this autodiff path; failing fast on H6 saves S5 from chasing a downstream symptom. |
| AS2-14 | s2_ternary_nodistill: include all 5 seeds or only seed 0? | All 5 seeds (R-S2-AllSeeds) | Doesn't this triple the compute cost? | H3 uses a per-seed delta comparison; reducing to seed 0 would force aggregate-only verdicts, weakening the H3 invariant. The cost is acceptable for Toy0 (the budget is small in absolute terms). |
| AS2-15 | Inert-loss policy: zero out the lambdas or omit the loss terms entirely? | Present in config, with explicit ComputedDisabled vs StructurallyInert logging semantics (D10, S2-Run-Ok-7) | Wouldn't omitting be cleaner? | CLAUDE.md "Training Loss Beads" rule: "Keep raw weighted-loss helpers honest: they must validate finite/non-negative raw diagnostics even when the configured weight is zero." Omitting hides the helper invocation; zeroing keeps the diagnostic. The s2_distillation_log.v1 records phase-effective lambdas alongside raw values. |
| AS2-16 | Re-run D7 oracle suite under S2 binary vs trust S1's prior pass | Re-run (O3, §12 closure) | The oracle is independent of the trained model; why re-run? | The oracle implementation lives in gbf-experiments::s1::oracle and consumes the bpc primitive. The S2 binary's compilation may inadvertently perturb that primitive (e.g., via an inlined fake-quant noise leak from Phase B/C/D code if H4 fails subtly in a way the canonical-tensor-payload check misses). Re-running is cheap and catches a class of contamination H4 alone may not. |
| AS2-17 | Distillation form: T^2 * KL(softmax(t/T) \|\| softmax(s/T)) vs T^2 * KL(softmax(s/T) \|\| softmax(t/T)) (forward vs reverse KL) | KL(t \|\| s) per gbf-train::loss::distillation existing contract (§1) | Reverse KL is mode-seeking; forward is mean-seeking. | The existing gbf-train implementation pins forward KL. Changing it would break the closed bd that produced that implementation and would force a re-run of any prior distillation tests. S2 inherits forward KL; S5+ may revisit. |
| AS2-18 | Phase A teacher freeze: at step 4000 (Phase A end) or at step 4001 (Phase B start) | Step 4000, with the teacher_freeze event firing on transition into step 4001 (S2-Run-Ok-8) | Off-by-one matters for reproducibility. | The teacher snapshot is taken AFTER step 4000's optimizer update completes (so the teacher reflects 4000 completed updates). The teacher_freeze event is emitted at the boundary between step 4000 and step 4001, which is the same boundary where Phase A transitions to Phase B in D1. Recorded explicitly in s2_phase_log.v1 as a single boundary. |
| AS2-19 | Should the S2 binary share the same `pass_version` as S1 or have its own? | Independent pass_version_S2 (Rep-S2-5) | Sharing is simpler. | Independent versions let S1 closure artifacts remain valid even after S2 amendments. Both are recorded in s2_report.v1 for cross-reference. |
| AS2-20 | Build naming: `s2_ternary_full` vs `s2_ternary` | `s2_ternary_full` | The "full" suffix is verbose. | Distinguishes from a future S2.x where a partial schedule (e.g., Phase A+C only) might be added; reserves the shorter name. The build_kind is a stable identifier in s2_phase_log.v1 and changing it forces a pass_version_S2 bump. |
| AS2-21 | Should the S2 closure also re-run the S1 falsification suite (F1..F6)? | No; only re-run D7 oracle suite | Defense in depth. | F1..F6 test S1-specific dense-fp behaviors; they don't depend on S2 binary code. Re-running them would conflate S1 substrate with S2 substrate verdicts. The S2 falsification suite (F1-broken-S2..F6-broken-S2) is the S2-specific defense. |
| AS2-22 | Threshold init RNG declaration when v1 consumes 0 draws (D4): declare or omit? | Declare (D11, §1, §5 RngStreams) | Adding a never-used RNG is overhead. | Declaring keeps the rng-stream contract explicit and prevents a future randomized-init implementation from silently expanding the RNG domain set. Per F-S1's discipline of disjoint RngStreams. |
| AS2-23 | Final concise contract count: keep the closure checklist compact vs expand it? | Keep 10 statements (§18) unless a blocker cannot be expressed clearly. | Newer slices need more. | A compact closure-readiness checklist is useful, but count should not override precision. Anything omitted must be deferred, inherited, or covered by proof obligations §13. |

---

# 18. Final concise contract

```text
F-S2 QAT Survives is correct when:

1.  Five seeded Toy0 ternary runs of the s2_ternary_full build complete
    Phases A->B->C->D end-to-end without divergence, with phase_transition
    events at exactly steps 4001, 5001, 8001 and a single teacher_freeze
    event at step 4001, on the same TinyStories raw-byte fixture used by
    S1 under S1CpuDeterministic. Replay produces bit-identical
    safetensors at every phase boundary and at the final step.

2.  For every seed, the matched-protocol gap satisfies
    bpc_ternary(seed) - bpc_fp(seed) <= 0.5 bpc, where both bpc values
    are produced by the s1_score_bpc primitive on the same val bytes
    with the same chunked-reset semantics, and the matched fp build
    (s2_fp_full) runs the same A->B->C->D phase plan with
    QuantHardness=Off enforced runtime-side via the
    QuantHardnessOverride::AllOff selected by runtime BuildKind.

3.  The s2_ternary_full Phase A end checkpoint (step 4000) has the same
    CanonicalTensorPayloadHash as the s2_ablation Phase A end checkpoint,
    for seed 0. Whole-file safetensors byte equality is non-normative.
    This re-asserts the S1 H4 invariant under the S2 binary.

4.  The s2_ternary_nodistill build runs to completion on all five seeds,
    and H3 receives an explicit binary verdict. If for every seed
    gap_distill(seed) <= gap_nodistill(seed) + 0.10 bpc, H3 is Confirmed.
    Otherwise H3 is Refuted and the outcome maps to
    `Pass-with-distill-warn`, provided H1, H2, H4, H5, and H6 all confirm.
    H3 Refuted does not block closure; H3 NotEvaluated does.

5.  The standard loss-term gradient flow suite (H5.1..H5.5) passes on
    the synthetic fixtures for {lambda_zrouter, lambda_balance,
    lambda_range, lambda_zero, lambda_distill}: every in-scope
    parameter receives a non-zero gradient, every stop-gradient
    parameter or tensor receives a zero gradient within the H5 epsilon
    rule, except that the H5.5 teacher-logits case is enforced as exact
    zero or backend-reported detached absence, and every
    fixture exercises a non-default lambda value.

6.  The Burn LinearStateBlock gradient smoke (H6) on FIXTURE_V1
    (seq_len=8, hidden_dim=4, batch=1) produces finite, nonzero
    gradients on every trainable parameter and on the input tensor,
    and re-running with the same seed produces a byte-identical
    gradient bytestream.

7.  The phase-transition integration test (D8 / bd-14k) passes on the
    tiny_model fixture: 4 transition events, 1 teacher_freeze event,
    correct HardnessTriple at each boundary, skip-phase test passes,
    overlap-phase and empty-phase errors are raised.

8.  The D7 measurement-oracle suite re-runs under the S2 binary with
    metric_oracle_passed = true (O3). The gbf-model::qat and
    gbf-model::sequence::LinearStateBlock public APIs match the S1
    closure snapshot (O11).

9.  s2_report.v1 emits pre-registered predictions in git history
    strictly before the first S2 result artifact commit, and concludes
    with exactly one Decision value chosen by §8 dispatch. Every JSON
    artifact (s2_phase_log, s2_score, s2_distillation_log,
    s2_loss_grad_flow, s2_linearstate_grad_smoke,
    s2_phase_transition_integration, s2_ablation, s2_oracle_re_run,
    s2_report) is
    canonical, deterministic, and self-hash-valid.

10. The six-test S2 falsification suite passes: deliberately-broken
    implementations (F1-broken-S2..F6-broken-S2) produce the expected
    Refuted verdicts, gated by the test-only `falsify` feature. F4/F5/F6
    use self-describing fallback labels
    (`F4_threshold_per_weight_structural_mask_fixture`,
    `F5_zero_loss_diagnostic_runner_fallback`,
    `F6_linearstate_structural_smoke_fallback`) so structural mask,
    diagnostic-runner, and smoke-fixture sensitivity are not read as
    stronger real helper or Burn-adapter mutations. S2
    retires QAT survival risk on Toy0 only; it does not claim ROM,
    MoE, multi-timescale state, oracle round-trip, charset_v1, or
    v0_success readiness — those are later slices' proof obligations.
```
