# Formal spec pack: F-S5 Pick and Fit — sequence-state A/B + Game Boy fit

> **DRAFT.** This is a scientific/experimental RFC for the combined Slice S5
> of the training-contract epic. It is structured to be defensible to a
> skeptical reviewer (in particular P5 Proof-of-Work Detective and P6 RFC
> Scope Sentinel). Predictions in this document are **pre-registered**; the
> R-Predictions ancestry rule of S1 §10 carries over unchanged.
>
> **Merge 2026-05-19 — F-S5 + F-S6 -> F-S5 "Pick and Fit".**
> Originally drafted as two slices (F-S5-sequence-state-ab.md picked the
> sequence-state variant; F-S6-gameboy-fit.md fit it onto Game Boy), now
> merged into a single slice with the same combined scope. The two source
> RFCs remain on disk with SUPERSEDED headers for historical reference;
> this file is the only normative source for the merged slice.
>
> Closure beads: bd-36y1 (originally S5) and bd-1cdu (originally S6).
> Both close together; one may be retired as duplicate in a follow-up
> bead-graph op (owner: user).
>
> Schema namespace: all `s6_*.v1` schemas from the original F-S6 RFC
> have been renamed to `s5_*.v1` in this merged file. The merged slice
> closes F2 + F8 + F11 (originally S6) and F12 (originally S5).

This is the fifth scientific/experimental RFC in the training-contract epic.
Like S1..S4, its deliverable is **verified knowledge**, not just code.
S5 "Pick and Fit" is a two-act slice with one closure boundary:

  Act I  — Pick: train all three sequence-state variants (BoundedKv,
           LinearState Fixed(0.5), LinearState MultiTimescale) end-to-end
           through the F4 phase scheduler, produce a defensible side-by-
           side CheckpointFrontierPoint report, and emit a deterministic
           FrontierRecommendation in {A, B, Tie} per a pinned rubric.

  Act II — Fit: take the picked variant (BoundedKv as the conformance
           baseline always; LinearState as an optional second build) and
           drive it through the F-B* compiler pipeline + F-A* runtime
           stack into an EncodedRom that the gbf-emu deterministic
           execution policy can run for a one-token agreement check
           against the ArtifactOracle.

Important interpretation:
  S5 is **explicitly not** the slice that picks a long-term production
  winner between LinearState and BoundedKv. Act I produces a defensible
  measurement and a frontier recommendation; Act II proves that the
  conformance-baseline variant fits real Game Boy chrome end-to-end.
  A `Pass-with-A-frontier`, `Pass-with-B-frontier`, or `Pass-with-tie`
  outcome at the frontier is equally valid as a frontier verdict — but
  Act II always integration-tests BoundedKv as primary because BoundedKv
  is the only variant with a deterministic attention-oracle reference,
  which is what makes the emulator one-token agreement check a true
  falsification. If both variants pass the frontier and the picked
  variant fails the emulator harness, the slice fails on the emulator
  harness, not on the frontier.

```text
Spec:
  F-S5 Pick and Fit — sequence-state A/B + Game Boy fit
  Slice S5 of the training-contract epic (bd-1rb)
  Closure beads: bd-36y1 (originally S5) AND bd-1cdu (originally S6)
  Closes features:
    F12 / bd-144  (Dual-Path Sequence-State Experimentation; originally S5)
    F2  / bd-1hv  (RuntimeChromeBudget;                       originally S6)
    F8  / bd-2am  (Shadow Export/Compile;                     originally S6)
    F11 / bd-1i8  (CompileProfile + WRAM Layout;              originally S6)

Hypothesis-under-test (combined):
  (Pick) Three sequence-state variants — BoundedKv with K cap pinned to
  128, LinearState with DecayPolicy::Fixed(0.5), and LinearState with
  DecayPolicy::MultiTimescale([0.5, 0.75, 0.875, 0.9375]) partitioned
  into four equal-width state bands — each train end-to-end on the
  Project Gutenberg charset_v1 corpus through the F4 Phase A->D scaffold,
  produce finite val bpc per pinned seed, pass the v0_success workload,
  and surface as a single side-by-side CheckpointFrontierPoint frontier
  report. BoundedKv additionally agrees with a deterministic
  attention-oracle reference within pinned numeric tolerance.

  (Fit) The BoundedKv per-seed checkpoints, when preflighted against a
  real RuntimeChromeBudget emitted by the gbf-runtime shell build under
  the BringUp CompileProfile, shadow-compiled at pinned cadence during
  training with the real compiler pipeline (not the Act I stub),
  hardened through Phase E HardenAndSelect, compiled through the full
  F-B* pipeline into an EncodedRom whose certs validate, and loaded
  into gbf-emu under DeterminismPolicy::default(), emit exactly one
  token to the runtime's video commit queue, and that emitted token
  equals the ArtifactOracle's predicted token for the pinned prompt
  fixture byte-for-byte under canonical token comparison.

Owns:
  hypothesis statements H1..H17 (10 Pick + 7 Fit; see §3)
  pre-registered prediction tables (multi-variant, multi-axis)
  BoundedKv block training contract (K cap, attention-oracle agreement)
  LinearState DecayPolicy contract (Fixed, MultiTimescale; Learned out
    of scope per D3)
  CheckpointFrontierPoint side-by-side schema (s5_frontier.v1)
  RuntimeChromeBudget end-to-end contract (synthetic + real, agreement,
    per-export re-validation, runtime_nucleus_hash drift gate)
  CompileProfile + WRAM Layout end-to-end contract (BringUp profile
    registry entry, OverlayReload, WramLayoutPolicy, WramReserved,
    CompileRequest binding)
  Shadow compile pipeline (real, not stub; Act II makes Act I real)
  Pareto frontier + selection rubric over CheckpointFrontierPoint
  compiler_feedback.json feedback loop into training (safe_bound update
    rule + empty-affinity no-op)
  EncodedRom end-to-end build integration (ArtifactValidationAndUpgrade
    through PlacedRom through EncodedRom)
  Emulator one-token harness contract (gbf-emu invocation under
    DeterminismPolicy::default())
  Logging-overhead benchmark gate (< 1% per CONSTITUTION §II.1)
  attention-oracle reference implementation + fixture suite (Pick side)
  Burn adapter + gradient smoke for BoundedKv (T12.3b semantics)
  Burn adapter extensions for DecayPolicy MultiTimescale gradient smoke
  all s5_*.v1 artifact schemas (the largest set of any slice in the
    epic; combined from the original s5_*.v1 and s6_*.v1 namespaces)
  S5 reproducibility laws (extends S1 with per-variant + per-export
    determinism, frontier byte-equality, emulator deterministic
    execution)
  S5 falsification suite (fifteen deliberately-broken substitutes;
    current in-repo wrapper is substrate-only until bd-q3zo wires live
    gbf-experiments::s5 producer execution)

Does not own:
  charset_v1 (S3)
  Gutenberg corpus / KN-5 baseline (S4)
  v0_success workload manifest (S3 owns; S5 invokes per-variant)
  ReferenceModelBundle / ArtifactOracle contract (S3; S5 invokes for
    deployable-weights resolution + token prediction)
  Phase scheduler semantics (S1..S2; S5 inherits Phase A->D as closed)
  AdamW pinning (S1)
  S1CpuDeterministic device profile (S1)
  Toy0 ModelSizeProfile reference instance (S1 / T14.1)
  gbf-asm typed eDSL (F-A1, M0)
  gbf-emu adapter, DeterminismPolicy, trap dispatcher, harness plumbing
    (F-A7, M0; S5 consumes)
  gbf-debug agent CLI (F-A8; S5 may use scripted CLI for replay
    artifacts but does not modify F-A8)
  Bank0 cooperative runtime skeleton (F-A5, M0)
  BankLease / BankGuard ABI (F-A4, M0)
  Compiler pipeline stage definitions (F-B1..F-B15; S5 binds them but
    does not redefine them)
  ReachabilityValidation contract (F-B*; S5 invokes + asserts cert
    validity)
  PlacedRom / EncodedRom emission semantics (F-B*; S5 invokes)
  MoE / router / λ_balance / λ_zrouter / λ_switch (S7)
  UpperBankCandidate production-scale runs on Gutenberg /
    StructuredWidthGates / λ_shape / λ_overflow (S8 / M6)
  Multi-mode SchedulePack switching + Recovery profile end-to-end
    (S8 / post-closure)
  Steady-state generation past the first emitted token (post-closure)
  Cartridge hardware bring-up (post-closure)
```

---

# 1. Core notation

The notation below combines and deduplicates §3 of the original F-S5 and
§3 of the original F-S6. Primitives shared with S1..S4 (Hash256, Seed,
SemVer, BpcValue, GradNorm) are inherited unchanged from S1 §1 by
reference.

```text
Hash256          := /^sha256:[0-9a-f]{64}$/
RuntimeNucleusHash :=
    Hash256
  | /^SYNTHETIC_REFERENCE:sha256:[0-9a-f]{64}$/
Seed             := u64
TrainStep        := u32         ; valid training steps are 1..=optimizer_steps
EvalStep         := u32         ; eval points at steps 0, 2000, 4000, ..., 20000
ShadowStep       := u32         ; shadow_compile sampled at 4000, 8000, 12000, 16000, 20000
ShadowEmissionId := ShadowStep | PhaseEFinal
LossNatsPerByte  := f32         ; finite natural-log cross entropy per target token
BpcValue         := f64         ; required finite, >= 0; all pass/fail gates compare f64
GradNorm         := f32         ; required finite, >= 0; per-parameter L2 norm
Verdict          := Confirmed | Refuted

Variant          := BoundedKv | L_FIX1 | L_MT4
VariantId        := "boundedkv" | "linearstate_fixed_0_5" | "linearstate_mt4"

HypothesisStatus :=
    Confirmed
  | Refuted
  | NotEvaluatedDueToPriorGate(reason: String)

FailureKind :=
    Substrate
  | Capacity
  | Suspicious
  | Phase
  | Metric
  | AttentionOracle
  | FrontierIncomplete
  | ShadowCompileWiring
  | RuntimeBudget
  | CompileProfile
  | EncodedRom
  | EmulatorHarness
  | FeedbackLoop
  | LoggingOverhead

Hypothesis :=
  H1 | H2 | H3 | H4 | H5 | H6 | H7 | H8 | H9 | H10       ; "Pick" axis
| H11 | H12 | H13 | H14 | H15 | H16 | H17                  ; "Fit" axis

CharVocab        := charset_v1 token id ∈ [0, 79]   ; from S3
NGramOrder       := 5                                 ; KN-5 baseline from S4

ClockCycles      := u64
TickFrames       := u32

; --- BoundedKv types ----------------------------------------------------
BoundedKvKCap := { max_context: NonZeroU16, kv_bytes_per_token: NonZeroU16 }

S5 canonical BoundedKv instance (per D2):
  max_context        = 128
  kv_bytes_per_token = 16
  ; Record layout: 1 f32 valid flag + 3 f32 tied-KV payload slots
  ; per entry; see gbf-model::sequence::bounded_kv source.

BoundedKvSpec := SequenceSemanticsSpec::BoundedKv(
                   BoundedKvSemantics::new(128, 16)
                 )

; --- DecayPolicy types --------------------------------------------------
DecayPolicy :=
    Fixed(decay: f32)
  | MultiTimescale(decays: Vec<f32>, layout: BandLayout)
  | Learned                ; OUT of S5 scope per D3

BandLayout :=
    EqualBandsByOrder      ; partition state_slots into decays.len()
                             contiguous equal-width bands.

DecayPolicy invariants (validated by LinearStateBlockConfig::new):
  Fixed(d):
    d is finite AND d in (0.0, 1.0)
  MultiTimescale(ds, EqualBandsByOrder):
    ds.len() >= 2
    ds.len() <= state_slots
    state_slots % ds.len() = 0
    ∀ d in ds. d is finite AND d in (0.0, 1.0)
  Learned:
    Reject at LinearStateBlockConfig::new with an explicit
    "DecayPolicy::Learned is out of S5 scope per F-S5 D3" error.

S5 canonical instances:
  L_FIX1_decay : DecayPolicy::Fixed(0.5)
  L_MT4_decays : DecayPolicy::MultiTimescale(
                   decays = [0.5, 0.75, 0.875, 0.9375],
                   layout = EqualBandsByOrder
                 )

; --- Attention-oracle types --------------------------------------------
AttentionOracleSpec :=
  {
    schema:               "s5_attention_oracle_spec.v1"
    fixture_id:           "AOF-1" | ... | "AOF-5"
    fixture_token_count:  u32
    k_cap:                u32                ; pinned 128
    payload_slots:        u32                ; pinned 3
    mask:                 "causal_with_fifo_eviction"
    accumulator_dtype:    "f64"
    output_dtype:         "f32"
  }

AttentionOracleFixture :=
  {
    fixture_id:               "AOF-1" | ... | "AOF-5"
    source_corpus_sha:        Hash256        ; gutenberg val
    source_byte_offset:       u64
    token_ids:                List[CharVocab]
    fixture_self_hash:        Hash256
  }

AttentionOracleResult :=
  {
    fixture_id:                "AOF-1" | ... | "AOF-5"
    position:                  u32
    oracle_logits_sha256:      Hash256
    boundedkv_logits_sha256:   Hash256
    max_abs_diff:              f32
    agreement:                 Bool          ; max_abs_diff <= 1e-4
  }

AttentionOracleReport :=
  {
    schema:                    "s5_attention_oracle.v1"
    seed:                      Seed
    phase_a_checkpoint_sha:    Hash256
    projection_tensors_sha:    Hash256
    quant_spec_sha:            Hash256
    activation_clip_sha:       Hash256
    fixture_suite_sha:         Hash256
    spec_sha:                  Hash256
    per_fixture_results:       List[AttentionOracleResult]
    aggregate_max_abs_diff:    f32
    aggregate_p99_max_abs_diff:f32
    aggregate_agreement:       Bool
    oracle_self_hash:          Hash256
  }

; --- Shadow compile / frontier types -----------------------------------
ShadowCompileSampleStub :=    ; produced by training-scaffold (Act I)
  {
    schema:                 "s5_shadow_compile_sample_stub.v1"
    variant:                VariantId
    seed:                   Seed
    step:                   ShadowStep
    shadow_byte_cost:       u32
    shadow_kernel_count:    u32
    shadow_compile_ok:      Bool
    shadow_compile_skipped: Null | SkipReason
    sample_self_hash:       Hash256
  }

ShadowCompileSampleReal :=    ; produced by Act II full pipeline
  {
    schema:                 "s5_shadow_compile_sample.v1"
    variant:                VariantId
    seed:                   Seed
    emission_id:            ShadowEmissionId
    stages_executed:        List[String]  ; must equal S5_SHADOW_PIPELINE_STAGES
    shadow_byte_cost:       u32           ; from StaticBudgetReport
    shadow_kernel_count:    u32
    shadow_compile_ok:      Bool
    shadow_compile_skipped: Null | SkipReason
    fits_envelope:          Bool          ; from DeployabilityReport
    reachability_cert_valid:    Bool
    resource_state_cert_valid:  Bool
    shadow_latency_proxy_cycles: u64
    shadow_energy_proxy_units:   u64
    compiler_feedback_sha:  Hash256       ; sha of compiler_feedback.json
    sample_self_hash:       Hash256
  }

SkipReason :=
    EarlyDivergence
  | FixtureCorruption
  | UnsupportedVariantOnThisStep

CheckpointFrontierPoint (canonical JSON shape) :=
  {
    schema:                 "s5_checkpoint_frontier.v1"
    variant:                VariantId
    seed:                   Seed
    cadence_step:           Null | ShadowStep      ; Null = Phase E final
    checkpoint_phase_a_sha: Hash256                ; dense fp teacher
    checkpoint_phase_d_sha: Hash256                ; ternary student
    val_bpc_fp:             BpcValue
    val_bpc_ternary:        BpcValue
    ternary_gap:            BpcValue
    v0_success_pass:        Bool
    v0_success_score:       f64
    param_count:            u64
    projected_deployed_bytes:        u64
    shadow_compile_ok_at_end:        Bool
    shadow_byte_cost_at_end:         u32
    shadow_kernel_count_at_end:      u32
    latency_proxy_cycles:            u64
    long_range_repetition_penalty:   f64
    encoded_rom_byte_cost:           Null | u64     ; Act II only
    fits_envelope:                   Null | Bool
    reachability_cert_valid:         Null | Bool
    resource_state_cert_valid:       Null | Bool
    point_self_hash:        Hash256
  }

; --- Runtime budget / compile profile types ---------------------------
TargetProfileId  := "DmgMbc5BringUp" | ...
CompileProfileId := "BringUp" | "Default" | "Trace" | "Recovery"
SequenceSemanticsRefSpec := { kind: "BoundedKv" | "LinearState",
                              ; with kind-specific params }

WramLayoutPolicy :=
  {
    overlay_bytes:       u32
    continuation_bytes:  u32
    stack_bytes:         u32
    hot_arena_bytes_min: u32
    reserve_bytes:       u32
  }

WramReserved :=
  {
    overlay:         u32
    hot_arena_floor: u32
    total:           u32
  }

RomBudgetSlot :=
  {
    id:              BudgetSlotId
    class:           BudgetSlotClass    ; "Bank0Free" | "CommonBank" | "ExpertBank"
    usable_bytes:    u32
    reserved_slack:  u32
    placement_caps:  List[PlacementCap]
  }

ReferenceShellModule :=
  "Boot" | "Interrupts" | "Scheduler" | "Banking" | "Joypad"
  | "Text" | "Keyboard" | "VideoCommit"

RuntimeChromeBudget (JSON) :=
  {
    schema:                  "s5_runtime_chrome_budget.v1"
    target:                  TargetProfileId
    profile:                 CompileProfileId
    runtime_nucleus_hash:    RuntimeNucleusHash
    reference_shell_modules: List[ReferenceShellModule]
    reference_shell_spec:    ReferenceShellSpec
    rom_slots:               List[RomBudgetSlot]   ; sorted by (class, id)
    wram_reserved:           WramReserved
    sram_reserved:           u32
    chrome_budget_self_hash: Hash256
  }

; --- Emulator harness types -------------------------------------------
DeterminismPolicy := (opaque; F-A7 owns; S5 consumes default())
BootMode          := PreBoot | PostBootDmg | PostBootCgb

EmuHarnessOutcome :=
  {
    schema:                  "s5_emulator_harness.v1"
    seed:                    Seed
    prompt_id:               String                ; "S5-P2-token-zero-state"
    encoded_rom_sha:         Hash256
    token_emitted:           CharVocab             ; charset_v1 id
    ticks_consumed:          ClockCycles
    ticks_exhausted:         Bool
    agreement:               Bool                  ; vs ArtifactOracle
    oracle_predicted_token:  CharVocab
    determinism_policy_hash: Hash256
    harness_self_hash:       Hash256
  }

; --- Compiler feedback types ------------------------------------------
CompilerFeedback :=
  {
    range_hotspots:   List[{ layer_id: u32, max_abs: f32 }]
    affinity_hints:   List[ExpertSlotAffinity]    ; empty for dense
    feedback_self_hash: Hash256
  }

FeedbackApplyResult :=
  {
    schema:               "s5_feedback_apply.v1"
    seed:                 Seed
    cadence_step:         TrainStep
    safe_bound_in:        Vec<f32>
    safe_bound_out:       Vec<f32>
    affinity_was_no_op:   Bool
    feedback_input_sha:   Hash256
    apply_self_hash:      Hash256
  }

; --- Domain hashes and canonical JSON ---------------------------------
DomainHash(crate, type, schema_id, schema_version, canonical_json_bytes) =
  "sha256:" ++ hex(sha256(
    "gbf:" ++ crate ++ ":" ++ type ++ ":" ++ schema_id ++ ":" ++ schema_version
    ++ "\0" ++ canonical_json_bytes
  ))

Self-hash rule:
  For any artifact containing field *_self_hash, canonical_json_bytes
  are computed with that field omitted. Hashing an artifact including
  its own self-hash is forbidden. (Inherited verbatim from S1.)

S5CanonicalJson:
  UTF-8, sorted object keys, no insignificant whitespace, arrays in
  declared order (per D5/D11 axis orders for frontier_axes), finite
  floats encoded by shortest round-trip decimal representation, -0.0
  normalized to 0.0. (Inherited verbatim from S1.)

Prediction status rule:
  Entries under a hypothesis's Predicted block are pre-registered
  expectations. They affect the verdict only when repeated under that
  hypothesis's Falsification block. Otherwise, out-of-range observations
  are reported as Surprises, not automatic Refutations. (Inherited
  verbatim from S1.)
```

bpc (S5 reset-context, charset_v1):

```text
For a variant v's checkpoint M and validation token sequence T containing
N tokens of charset_v1:

  Let chunk(i) = floor(i / 128) and start(i) = 128 * chunk(i).
  Let ctx(i)   = T[start(i) .. i].

  bpc(v, M, T) = (1 / N) * sum_{i=0}^{N-1} -log2(P_M(T[i] | ctx(i)))

Required:
  - log2_sum is accumulated in f64; final division by N happens once.
  - State resets to zero at each chunk boundary.
  - For BoundedKv, the KV slab is also reset at each chunk boundary;
    KV occupancy never exceeds K_cap = 128.
  - The KN-5 baseline from S4 is scored under the same reset-context
    semantics so the H4 comparison is consistent.
```

KN-5 baseline reference (loaded, not refit):

```text
S5 does not refit the KN-5 character n-gram baseline. The S4-pinned
bpc_kn5_baseline value, baseline_self_hash, and counts_blob_sha256 are
loaded from artifacts/S4/baseline/kn5-report.json by their pinned
sha256. Refitting is forbidden in S5 even on the same corpus.
```

---

# 2. Decisions

The decisions below are the binding choices for the merged slice. They
combine the two original Decisions blocks (S5 had D1..D18, S6 had D1..D17)
into a pruned set of twelve. The merge resolution rules from the merge
operation are folded in where they alter pre-merge choices (notably the
primary-integration-variant rule in D8).

```text
D1 BoundedKv as conformance baseline; LinearState beside (Pick axis)
   BoundedKv lands first as the conformance baseline because bounded
   causal attention has well-understood oracle semantics. LinearState
   (both Fixed(0.5) and MultiTimescale variants) trains beside BoundedKv
   through the same scaffold but is not held to an attention-oracle
   agreement obligation. This asymmetry is intentional: BoundedKv carries
   the attention-oracle conformance burden; LinearState carries only the
   training-end-to-end + frontier-emission burden.

D2 Variant configs pinned for S5
   BoundedKv:
     max_context           = 128
     kv_bytes_per_token    = 16
     The K=128 cap matches the S1 chunk_size = 128 boundary so the
     reset-context bpc primitive established in S1 §7 applies unchanged.
   LinearState Fixed:
     Variant L_FIX1 = DecayPolicy::Fixed(0.5)
     Inherits the closed bd-tnb behavior.
   LinearState MultiTimescale:
     Variant L_MT4 = DecayPolicy::MultiTimescale(
       decays = [0.5, 0.75, 0.875, 0.9375],
       layout = EqualBandsByOrder
     )
     state_slots % 4 must equal 0.
   DecayPolicy::Learned is OUT of scope. Pinning any of these values
   constitutes a new experiment.

D3 BoundedKv attention-oracle agreement
   ∀ fixture seq f ∈ AttentionOracleFixtureSuite,
     ∀ token position t in f,
       max_abs_diff(
         model_output_logits(f, t),
         attention_oracle_logits(f, t)
       ) <= 1e-4   in f32 under S1CpuDeterministic.
   "max_abs_diff" is elementwise L_inf over the d_model pre-softmax
   output channel of the BoundedKv block.

   AttentionOracleFixtureSuite contains exactly five fixture sequences:
     fixture_id   len_tokens
     AOF-1                 1   (single token: zero-context edge case)
     AOF-2                32   (sub-K_cap)
     AOF-3               128   (exactly K_cap)
     AOF-4               192   (1.5x K_cap; exercises FIFO eviction)
     AOF-5               512   (4x K_cap; exercises sustained eviction)
   Reference attention oracle is a deterministic non-Burn Rust
   implementation, evaluated in f64 internally and downcast to f32 only
   at output time.

D4 seeds, train budget, and phase schedule (inherited / pinned)
   seeds                = [0, 1, 2, 3, 4]            (inherited from S1)
   optimizer_steps      = 20000                       (per variant; 15 runs total)
   batch_size           = 32
   sequence_length      = 128
   eval_every_steps     = 2000
   eval_subset_size     = 4096 sequences

   Phase ladder (F4):
     Phase A  DenseTeacherWarmup           steps    1..  6000
     Phase B  RouterWarmup                 steps 6001..  6001   (no-op for dense)
     Phase C  ExpertTernaryQat             steps 6002.. 12000
     Phase D  FullNumericQat               steps 12001..20000
     Phase E  HardenAndSelect              once at end

   S5 inherits S2's ternary QAT contract unchanged (per-row Q8.8 scales,
   AnnealedGlobalThenPerOutputRow threshold plan, hard ternary at Phase
   C entry, activation fake quant at Phase D entry); ternary gap target
   bpc(ternary, val) - bpc(fp, val) <= 0.5 bpc per variant per seed
   (inherited from planv0 amendment 2026-05-06 item 6).

   S5 inherits S4's Gutenberg charset_v1 corpus, KN-5 baseline,
   v0_success workload, and ReferenceModelBundle/ArtifactOracle
   contracts unchanged.

D5 frontier scoring rubric — pinned axes (Pick axis)
   The s5_frontier.v1 frontier_axes array, in this order:
     1.  val_bpc_fp                  : BpcValue
     2.  val_bpc_ternary             : BpcValue
     3.  ternary_gap                 : BpcValue
     4.  v0_success_pass             : Bool
     5.  v0_success_score            : f64
     6.  param_count                 : u64
     7.  projected_deployed_bytes    : u64
     8.  shadow_compile_ok_at_end    : Bool
     9.  shadow_byte_cost_at_end     : u32
    10.  shadow_kernel_count_at_end  : u32
    11.  latency_proxy_cycles        : u64
   None of these axes is by itself the "winner" predicate.

   FrontierRecommendation := A | B | Tie
   FrontierLeaderVariant := Null | VariantId
     A := "BoundedKv-leading"
          BoundedKv beats both LinearState variants strictly on
          val_bpc_ternary by >= 0.05 bpc, AND BoundedKv passes
          v0_success, AND BoundedKv shadow_compile_ok_at_end = true.
     B := "LinearState-leading"
          at least one LinearState variant beats BoundedKv strictly on
          val_bpc_ternary by >= 0.05 bpc, passes v0_success, and has
          shadow_compile_ok_at_end = true. frontier_leader_variant records
          the best LinearState variant by val_bpc_ternary, tie-broken by
          encoded_rom_byte_cost if present, then lexicographically.
     Tie := neither A nor B holds, but all three variants pass
          v0_success and emit valid CheckpointFrontierPoints.

   FrontierRecommendation is a license, not a winner. The picked variant
   may still fail Act II's emulator harness; if so the slice fails on
   the harness, not on the frontier (this is the merge-binding rule
   that resolves the original S6 D1 / D17 with the S5 frontier choice;
   see also D8 below).

D6 strict per-variant pass criterion
   ∀ variant v in {BoundedKv, L_FIX1, L_MT4}.
     ∀ seed s in {0..4}.
       run(v, s).completion = Completed
       AND val_bpc_ternary(v, s) is finite
       AND val_bpc_ternary(v, s) < bpc_kn5_baseline(charset_v1) - 0.05
   Per-variant strict: one bad seed in any variant fails the Pick axis.

D7 BringUp profile is the closure profile (Fit axis)
   All Fit-axis closure obligations are evaluated under
   CompileProfile::BringUp. Default, Trace, and Recovery profiles are
   out of scope for closure.

   BringUp profile defaults (pinned; changing any constitutes a new
   experiment and bumps pass_version):
     wram_layout:
       overlay_bytes        = 4096
       continuation_bytes   = 256
       stack_bytes          = 256
       hot_arena_bytes_min  = 2048
       reserve_bytes        = 1536
     overlay_reload         = OverlayReloadPolicy::PerExpertSwitch
     max_bank_switches_per_token = 8
     sequence_state         = SequenceSemanticsRef::BoundedKv { k_cap: 128 }
     placement_profile      = PlacementProfile::StrictOnePerBank
     max_refinement_iters   = 1
     allow_placement_profile_fallback = false
     allow_trace_demotion             = false
     allow_overlay_promotion          = false
     allow_recompute_promotion        = false

D8 Primary integration variant (Fit axis) — MERGE-BINDING RULE
   Act II compiles BoundedKv as the primary integration test
   regardless of which frontier recommendation (A/B/Tie) Act I emits,
   because BoundedKv is the only variant with a deterministic
   attention-oracle reference and the H15 emulator one-token agreement
   check needs that oracle.

   If Act I emitted FrontierRecommendation = B or Tie, Act II may run
   the optional LinearState build alongside the primary BoundedKv
   build and report it as informational. The closure gate is
   BoundedKv only.

   **Merge resolution**: this is the merge-binding override of the
   original S5 §D11's "let frontier decide" with the original S6 §D17's
   "BoundedKv unconditionally". The merged rule is: both variants train;
   the frontier recommendation is preserved as data; Act II picks
   BoundedKv as primary and may run LinearState as a second informational
   build. If both variants pass the frontier and the picked variant
   fails Act II's emulator, the slice fails on the emulator (not on
   the frontier).

D9 RuntimeChromeBudget pinned shape — synthetic and real
   The reference RuntimeChromeBudget under BringUp pins:
     target                  = TargetProfileId::DmgMbc5BringUp
     profile                 = CompileProfileId::BringUp
     reference_shell_modules = { Boot, Interrupts, Scheduler, Banking,
                                 Joypad, Text, Keyboard, VideoCommit }

   future_reservations (subtracted from rom_slots[Bank0Free]):
     Persistence: { rom_bytes_per_bank0:  256, wram_bytes:  64, sram_bytes:  256 }
     Trace      : { rom_bytes_per_bank0:  512, wram_bytes:  64, sram_bytes: 1024 }
     Harness    : { rom_bytes_per_bank0:  256, wram_bytes:  32, sram_bytes:   64 }
     Panic      : { rom_bytes_per_bank0:  128, wram_bytes:   0, sram_bytes:    0 }

   rom_slots (BringUp; pinned):
     Bank0Free   :  usable_bytes =  6_144  reserved_slack = 256
     CommonBank  :  usable_bytes = 15_872  reserved_slack = 512
     ExpertBank  :  usable_bytes = 15_872  reserved_slack = 384
   wram_reserved:
     overlay         = 4_096
     hot_arena_floor = 2_048
     total           = 8_192
   sram_reserved   = 4_096
   These are [ESTIMATE]; tunable once the real shell ships its first
   measured emission. See A4 in the ambiguity ledger.

   D9-tol synthetic-vs-real tolerance:
     For every RomBudgetSlot s:
       |real.usable_bytes(s) - synthetic.usable_bytes(s)| <= 256
       real.reserved_slack(s) >= synthetic.reserved_slack(s)
       real.placement_caps(s) == synthetic.placement_caps(s)
     For wram_reserved: equality on all three fields.
     For sram_reserved: equality.
     For runtime_nucleus_hash: NOT compared by tolerance; see D10.

D10 runtime_nucleus_hash — synthetic vs real
   Synthetic budget:
     runtime_nucleus_hash starts with literal ASCII
     "SYNTHETIC_REFERENCE:" prefix on a sha256.
   Real budget:
     runtime_nucleus_hash = sha256(assembled_runtime_nucleus_bytes);
     does NOT carry the prefix.
   The trainer MUST log whether it is on synthetic or real at every
   preflight invocation. CI gate (T2.5 / bd-177) enforces that, after
   the first real shell build is committed, every new training run
   records the REAL hash and that hash matches the live shell build's
   emission.

D11 Shadow compile cadence and pipeline (real, not stub) (Fit axis)
   shadow_every_n_steps     = 4000
   shadow_keep_frontier     = 3
   shadow_workloads         = [v0_success_subset_S5]
   shadow_requests          = [BringUp]
   shadow_full_compile      = true
   Cadence: 5 samples per seed at steps {4000, 8000, 12000, 16000, 20000}
   plus once at Phase E HardenAndSelect = 6 ShadowCompileSampleReal
   records per seed * 5 seeds = 30 real shadow records per PR.

   The Act II shadow compile MUST exercise (not skip) every stage in
   S5_SHADOW_PIPELINE_STAGES =
     [ QuantGraph, StaticBudgetReport, GbInferIR, ObservationPlan,
       RangePlan, StoragePlan, SramPagePlan, RomWindowPlan,
       OverlayPlan, ArenaPlan, GbSchedIR, ResourceStateValidation,
       AsmIR, ReachabilityValidation, PlacedRom, EncodedRom ]
   The list MUST appear verbatim in shadow.stages_executed and is
   asserted equal to S5_SHADOW_PIPELINE_STAGES in gbf-policy.

   Pareto dominance:
     lower-is-better:
       val_bpc_fp,
       val_bpc_ternary,
       ternary_gap,
       param_count,
       projected_deployed_bytes,
       shadow_byte_cost_at_end,
       shadow_kernel_count_at_end,
       latency_proxy_cycles,
       encoded_rom_byte_cost when present.
     higher-is-better:
       v0_success_score.
     boolean must-be-true:
       v0_success_pass,
       shadow_compile_ok_at_end,
       fits_envelope when present,
       reachability_cert_valid when present,
       resource_state_cert_valid when present.
   Selection: filter out points where any of
   {shadow_compile_ok, fits_envelope, reachability_cert_valid,
   resource_state_cert_valid} is false; from the remaining, pick the
   point with minimum val_bpc_ternary, breaking ties by minimum
   encoded_rom_byte_cost only when both tied candidates have a present
   value. If exactly one tied candidate has Null encoded_rom_byte_cost,
   the encoded-ROM tie-break is skipped for that pair and selection
   falls through to the lexicographic tie-breaker.

   v0_success_subset_S5 is a strict 3-prompt subset of S3's v0_success
   workload, pinned in fixtures/workloads/v0_success_s5.toml:
     P1  "S5-P1-coherence"             64 charset_v1 tokens
     P2  "S5-P2-token-zero-state"       1 charset_v1 token (zero context)
     P3  "S5-P3-bounded-kv-fill"      128 charset_v1 tokens (= K_cap)
   The emulator one-token harness (D12) uses P2 only.

D12 Emulator one-token harness — pinned execution contract (Fit axis)
   variant      = BoundedKv (D8)
   seed         = 0
   prompt       = v0_success_subset_S5.P2
   determinism  = DeterminismPolicy::default()      (per F-A7)
   boot_mode    = BootMode::PostBootDmg             (per F-A7 §0.1)
   budget       = ClockCycles(DMG_FRAME_CLOCK_CYCLES * S5_TICK_FRAMES)
   S5_TICK_FRAMES = 240                              ; [ESTIMATE] = 4 s @ 60fps
   stop_predicate = first commit to runtime's video commit queue
                    carrying a charset_v1 token

   The harness loads the EncodedRom into gbf-emu, executes via
   run_fast_for(budget) with one PC trap at VIDEO_COMMIT_TOKEN_TRAP_PC
   (resolved at link time from gbf-runtime's .sym map), captures the
   token byte, and returns { token_emitted, ticks_consumed, agreement }.

   agreement is computed against ArtifactOracle::predict_first_token(
     P2, ema_checkpoint, charset_v1) byte-for-byte under canonical
   token comparison (single u8 equality after charset_v1 token-id
   lookup).

D13 Compiler feedback loop — pinned consumer rule (Fit axis)
   Two channels, both consumed at PHASE BOUNDARIES ONLY (steps 6000,
   6001, 12000, 20000):

     channel A: ActFakeQuant.safe_bound updates
       Rule:
         if max_abs > current_safe_bound:
           new = old + min(0.10 * old, 0.5 * (max_abs - old))
           clamped to safe_bound_max = 16.0.
         elif max_abs <= 0.5 * current_safe_bound:
           new = old * 0.95
           clamped to safe_bound_min = 0.5.
         else: new = old.
       Result recorded in s5_feedback_apply.v1 per phase boundary.

     channel B: ExpertSlotAffinity hints (S7 territory; for S5 the
       channel is plumbed but the dense baseline has no experts. The
       hint vector is empty and the consumer is a no-op. H16 verifies
       the consumer handles the empty-affinity case as a byte-identical
       no-op, NOT as a NaN.)

D14 logging-overhead gate — pinned threshold (Fit axis)
   Per CONSTITUTION §II.1, the structured-logging path through gbf-train
   producers must add < 1% overhead:
     workload          : pinned tiny preflight + shadow_compile invocation
     baseline_kind     : same workload with logging compiled out
                         (`--no-default-features --features "s5-no-log,qat,burn-adapter"`)
     measurement_env   : CPU governor fixed, single worker thread,
                         warm cache after warmup, baseline/instrumented
                         runs interleaved or randomized by pair
     warmup_iterations : 5
     measured_iterations: 50
     metric            : median wall-clock per iteration
     gate_predicate    :
       (median(instrumented) - median(baseline)) / median(baseline) < 0.01
   Threshold: 0.01 (1.0%). [ESTIMATE]; see A16 in §19.
   Implementation: scripts/s5_logging_overhead_check.sh as the D14
   gate/report substrate. The full §13.18 self-hashed report emission
   remains owned by bd-zy3j.

D15 fail-closed on NaN / divergence (per-variant)
   Any seed of any variant producing non-finite loss or non-finite
   gradient norm at any step fails the entire slice with
   Fail-substrate(variant=v, seed=s, step=k). One bad seed in any
   variant fails the slice.

D16 strict reproducibility (per-variant + per-export)
   Same seed + same variant config + same corpus_*_sha + same
   charset_v1_sha + same train_config_hash + same model_config_hash
   + same gbf-train pass_version + same gbf-codegen pass_version +
   same gbf-runtime pass_version + same dependency lockfile + same
   rust_toolchain_hash + same build_config_hash + same device_profile
   + same compile_profile_hash + same runtime_chrome_budget_hash
   ==> bit-identical safetensors per (variant, seed)
   AND bit-identical (.gb, .sym, .lst) per (seed, export_pass).

   Additionally:
     same (variant, seed) ScoreReports
     + same (variant, seed, cadence_step) ShadowCompileSample-real
     + same v0_success outcomes
     + same AttentionOracleReports
     + same Pareto inputs
     ==> bit-identical s5_frontier.v1 JSON.

D17 emulator deterministic execution policy
   For S5 closure runs:
     DeterminismPolicy::default()  per F-A7 §0.1
     BootMode::PostBootDmg
     PowerOnRamPolicy::GameroyDefault
     audio_output_enabled        : false
     real_time_cartridge_rtc     : false
     save_state_metadata_timestamp : fixed
     joypad_input_stream         : EMPTY for the harness window
     seed_for_internal_rng       : pinned via gbf-emu DeterminismPolicy
   Replays MUST produce byte-identical (token_emitted, ticks_consumed,
   harness_self_hash) per (seed, EncodedRom_sha).

D18 per-export re-validation gate
   Every Phase E export emission re-runs the preflight against the
   CURRENT RuntimeChromeBudget. Outcomes:
     - both runtime_nucleus_hashes match              -> Pass
     - hashes differ but per-slot byte deltas within
       D9-tol AND fits_envelope = true                -> Warn
     - hashes differ AND any fits_envelope check
       fails                                          -> BlockExport
                                                       with Fail-runtime-budget
     - synthetic-vs-real mismatch (one sentinel)      -> BlockExport per D10
   The export is blocked at the export pipeline stage, NOT after ROM
   emission.

D19 amendment to S3 v0_success workload — per-variant invocation
   S3 owns the v0_success WorkloadManifest; S5 invokes it per variant
   without amending the manifest's pass criteria. S5 records three
   independent v0_success run results in s5_v0_success_per_variant.v1
   sidecars and surfaces pass/fail bit and aggregate score in
   s5_frontier.v1.

D20 attention-oracle independence from Burn/runtime state
   The attention-oracle reference implementation does not depend on
   any Burn module handle, live trainer state, host runtime state, or
   random number stream. It is a deterministic numeric routine over
   canonical projection tensors extracted from the checkpoint/artifact
   under test, plus the pinned activation-fake-quant range. If the
   trained block disagrees with the oracle, the trained block is
   wrong (or the oracle is wrong), not "they happen to differ."
```

---

# 3. Hypothesis algebra

Every hypothesis carries a statement, predicted observables,
falsification rule, verdict mapping, and downstream consequence.

The combined slice carries 17 hypotheses split across two axes:
  - Pick axis (Act I, originally S5):   H1..H10
  - Fit  axis (Act II, originally S6):  H11..H17

Mandatory closure gates:
  Pick: H1, H2, H3, H4, H6, H7, H9, H10
  Fit:  H11, H12, H13, H15, H16, H17
Non-closure-gating (binary verdict still required, but Refuted does not
  by itself block closure):
  Pick: H5 (multi-timescale quality direction)
  Pick: H8 (BoundedKv-vs-LinearState parity direction)
  Fit:  H14 (Pareto frontier soundness; H14 Refuted demotes to
        manual-override per §12)

Quantifier convention: hash-bearing hypotheses range over seeds = {0..4}
and (where applicable) variant in {BoundedKv, L_FIX1, L_MT4}. The
emulator one-token harness (H15) runs seed 0 only.

## H1 BoundedKv attention-oracle agreement (mandatory)

```text
Statement:
  ∀ s, ∀ f in AttentionOracleFixtureSuite, ∀ t in f.
    max_abs_diff(boundedkv_block_output(s, phase_a_ckpt, f, t),
                 attention_oracle_output(f, t)) <= 1e-4
  in f32 under S1CpuDeterministic.
  Asserted on Phase A (dense fp) checkpoints, not on Phase D ternary.

Predicted (aggregate sanity):
  median over (s, f, t) max_abs_diff <= 5e-5
  p99    over (s, f, t) max_abs_diff <= 1e-4

Falsification:
  ∃ s, f, t. max_abs_diff(...) > 1e-4           ⇒ Refuted
  attention_oracle_self_hash mismatch on replay ⇒ Refuted
  AttentionOracleFixtureSuite hash drift        ⇒ Refuted

Verdict: Refuted if any falsification fires, else Confirmed.
Consequence of Refuted: BoundedKv loses conformance-baseline role;
  Act II cannot rely on the oracle agreement for H15. Block slice.
```

## H2 BoundedKv Burn gradient smoke (mandatory)

```text
Statement:
  In Burn autodiff at QuantHardness::Off, a forward+backward pass
  through BoundedKv on a tiny pinned fixture produces finite, nonzero,
  deterministic gradients into every intended trainable parameter set
  and zero gradients into every stop-gradient set.

Trainable: query/kv/output projection weights; per-row Q8.8 scales;
  AffineParams of input/output_norm.
Stop-gradient: TernaryThreshold values (Phase A); state buffer bytes;
  frozen Q8.8 scales.

Predicted:
  ∀ p in trainable. grad_norm(p) is finite AND > 0
  ∀ p in stop_gradient. grad_norm(p) = 0 exactly
  Replay determinism: bit-identical Hash256 over sorted gradient bytes.

Falsification: any of the above violated ⇒ Refuted.
Consequence: BoundedKv is not a usable training-scaffold target. Block
  every dependent H. Investigate T12.3b (bd-3arn).
```

## H3 LinearState multi-timescale trains end-to-end (mandatory)

```text
Statement:
  Variant L_MT4 trains end-to-end through Phase A->D for every seed
  without divergence and produces a valid CheckpointFrontierPoint.

Predicted:
  ∀ s. run(L_MT4, s).completion = Completed
  ∀ s. val_bpc_ternary(L_MT4, s) is finite
  ∀ s, b in 0..4. mean(state band b at every eval point) is finite

Falsification:
  any divergence, non-finite bpc, missing frontier point, or band
  collapse to non-finite ⇒ Refuted.
Consequence: T12.5 not deployment-ready. Because H3 is a mandatory
  Pick-axis closure gate and D6 requires all three variants to pass
  per seed, block the slice and open a follow-up bead.
```

## H4 Three-variant smoke (mandatory)

```text
Statement:
  Each of {BoundedKv, L_FIX1, L_MT4} produces finite val bpc and
  passes v0_success per seed on Gutenberg val.

Predicted:
  bpc_kn5_baseline(charset_v1) in [1.5, 2.5]                ; sanity
  ∀ v, s. val_bpc_fp(v, s)        in [1.0, 2.2]              ; sanity
  ∀ v, s. val_bpc_ternary(v, s)   in [1.0, 2.5]              ; sanity
  ∀ v, s. val_bpc_ternary(v, s) < bpc_kn5_baseline - 0.05   ; gate
  ∀ v, s. v0_success_pass(v, s) = true

Falsification: any per-(v,s) gate fails; or median bpc < 0.5
  (suspicious; mirrors S1 H2) ⇒ Refuted.
Consequence: Pick axis blocked. Investigate per-variant.
```

## H5 LinearState multi-timescale advantage (non-closure-gating)

```text
Statement:
  L_MT4 beats L_FIX1 on val_bpc_ternary by >= 0 (seed-aggregate) AND
  reduces long_range_repetition_penalty by >= 0.10 / generated token.

Falsification:
  median_seed val_bpc_ternary(L_MT4) > median_seed val_bpc_ternary(L_FIX1) + 0.05
                                                            ⇒ Refuted
  long_range_repetition_penalty(L_MT4) >
    long_range_repetition_penalty(L_FIX1) + 0.05            ⇒ Refuted

Verdict: binary, but Refuted does NOT block closure. Demotes
  FrontierRecommendation to whichever non-LinearState path leads;
  L_FIX1 becomes the LinearState representative for Fit-axis purposes.
```

## H6 shadow_compile A/B wiring — Act I stub level (mandatory)

```text
Statement:
  Every shadow_compile sample after step 4000 produces a well-formed
  ShadowCompileSampleStub:
    shadow_byte_cost finite (not u32::MAX)
    shadow_kernel_count > 0
    shadow_compile_ok recorded as a real bool (not constant)
  BoundedKv records shadow_compile_ok = true on every cadence sample
  after step 4000.

Falsification:
  any cadence sample missing, sentinel byte cost, all-constant ok,
  BoundedKv ok=false at any cadence, or any LinearState variant with
  no ok=true cadence sample ⇒ Refuted.
Consequence: Act II cannot trust the shadow API surface. Block.
```

## H7 frontier emission completeness — Act I (mandatory)

```text
Statement:
  Exactly one s5_frontier.v1 instance is emitted per PR; it contains
  one variant_record per variant in {BoundedKv, L_FIX1, L_MT4} with
  every D5 axis filled per seed and per per-variant aggregate, and a
  self-hash that round-trips through canonical JSON.

Falsification:
  variant_records.length != 3, any axis null, missing aggregate,
  self-hash drift after canonical round-trip ⇒ Refuted.
Consequence: closure deliverable does not exist; block.
```

## H8 BoundedKv-vs-LinearState parity (non-closure-gating)

```text
Statement:
  Either BoundedKv beats both LinearState variants on val_bpc_ternary
  by >= 0.05 bpc (Recommendation = A), or the gap is recorded honestly
  as Tie or B per D5.

Predicted (pre-registered direction):
  median_seed val_bpc_ternary(BoundedKv)
    <= median_seed val_bpc_ternary(L_FIX1) - 0.05
  median_seed val_bpc_ternary(BoundedKv)
    <= median_seed val_bpc_ternary(L_MT4)  - 0.05

Falsification:
  median_seed val_bpc_ternary(BoundedKv) >
    min(median_seed val_bpc_ternary(L_FIX1), median_seed val_bpc_ternary(L_MT4))
    + 0.05  ⇒ Refuted

Verdict: binary; Refuted does NOT block. Drives FrontierRecommendation
  to B or Tie; closure document records the surprise honestly.
```

## H9 Reset boundary preservation (mandatory)

```text
Statement:
  Every variant's val bpc scoring uses chunk_size = 128 unchanged.
  State resets between chunks; first byte of each chunk scored from
  the zero state. BoundedKv's KV slab is also reset between chunks.

Falsification:
  context-length spy fixture records context length != expected
  sequence [0, 1, ..., 127, 0]; BoundedKv KV occupancy fails to
  reset at chunk boundary; KV occupancy > K_cap = 128 ⇒ Refuted.
Consequence: bpc unreliable across variants; block.
```

## H10 per-variant determinism — Act I (mandatory)

```text
Statement:
  ∀ (variant, seed). replay produces bit-identical safetensors,
  s5_run_log.v1 self-hash, s5_score.v1 self-hash. Additionally
  s5_frontier.v1 is byte-identical under replay.

Falsification: any replay disagreement on any pinned hash ⇒ Refuted.
Consequence: S5 cannot make scientific claims. Investigate per-variant
  determinism (Burn backend, BoundedKv FIFO order, MultiTimescale band
  partitioning, gradient reduction order).
```

## H11 RuntimeChromeBudget integrity (mandatory)

```text
Statement:
  Synthetic RuntimeChromeBudget under T2.6 and real RuntimeChromeBudget
  emitted by the runtime shell build under T2.2 agree on schema shape
  and per-slot byte counts within D9-tol, AND the runtime_nucleus_hash
  CI drift gate (T2.5) fires on a deliberately mismatched build.

Falsification:
  ∃ slot. |real - synthetic| usable_bytes > 256                       ⇒ Refuted
  ∃ slot. real.reserved_slack < synthetic.reserved_slack              ⇒ Refuted
  wram_reserved / sram_reserved unequal                                ⇒ Refuted
  synthetic missing SYNTHETIC_REFERENCE: prefix                        ⇒ Refuted
  drift-gate on altered nucleus exits 0                                ⇒ Refuted
  trainer self-check fails to fire on mid-run hash mismatch            ⇒ Refuted

Consequence: RuntimeChromeBudget is not a faithful contract; the
  trainer was reasoning about a budget that does not match the shell.
  Block. Investigate T2.4 / T2.2 / T2.5.
```

## H12 CompileProfile binding (mandatory)

```text
Statement:
  BringUp CompileProfile flows through CompileRequest into gbf-train
  preflight; preflight rejects an over-budget artifact deterministically
  with a clear diagnostic AND passes the in-budget BoundedKv artifact.

Predicted:
  preflight(in_budget,   BringUp).fits_envelope    = true,  hard_failures = []
  preflight(over_budget, BringUp).fits_envelope    = false, hard_failures != []
  ∃ f in hard_failures. f.diagnostic contains:
    - the offending dim/quantity
    - the slot id and slot class
    - usable_bytes - reserved_slack
    - a suggested fix (e.g. "reduce d_ff from 224 to 208")
  in_budget preflight under profile=Default succeeds AND records BringUp-
    specific WRAMFitReport NOT present in Default output.

over_budget fixture: hand-pinned ModelArtifact with declared expert
  dimensions exceeding ExpertBank.usable_bytes - reserved_slack by
  exactly +512 bytes. Pinned in fixtures/preflight/s5_over_budget.json.

Falsification:
  in_budget preflight fits_envelope = false                ⇒ Refuted
  over_budget preflight fits_envelope = true               ⇒ Refuted
  diagnostic missing slot id / class / over-by              ⇒ Refuted
  CompileRequest.profile absent or ignored downstream      ⇒ Refuted

Consequence: CompileProfile is not actually consumed. Block.
  Investigate T11.3 / T11.2 / WramFitReport emitters.
```

## H13 Shadow compile correctness — Act II real pipeline (mandatory)

```text
Statement:
  At every cadence step, the real shadow compile pipeline produces a
  CheckpointFrontierPoint whose:
    - shadow_compile_ok is honestly computed from the full pipeline
      result (not constant);
    - shadow EncodedRom byte cost is within tolerance of a final
      EncodedRom built from the same EMA checkpoint at Phase E;
    - compiler_feedback.json fields are identical (modulo noise) to
      the final-export feedback when built from the same checkpoint.

Pinned tolerances:
  |shadow.shadow_byte_cost - final.encoded_rom_byte_cost| <= 1024 bytes
                                                       ; [ESTIMATE] A8
  shadow_kernel_count vs final kernel_count: equality required.
  compiler_feedback.json equality EXCEPT
    { generated_at_us, build_id_uuid, observation_seq }.

Predicted:
  shadow.stages_executed = S5_SHADOW_PIPELINE_STAGES (verbatim)
  if shadow.shadow_compile_ok = true:
    shadow.fits_envelope                = true
    shadow.reachability_cert_valid      = true
    shadow.resource_state_cert_valid    = true

Falsification:
  stages_executed != S5_SHADOW_PIPELINE_STAGES                         ⇒ Refuted
  shadow_compile_ok constant true (on broken substitute)               ⇒ Refuted
  shadow_compile_ok constant false on healthy checkpoint               ⇒ Refuted
  ∃ s. |shadow_at_20000 - final.encoded_rom_byte_cost| > 2048          ⇒ Refuted
  kernel_count mismatch                                                ⇒ Refuted
  feedback diff (modulo noise) non-empty between shadow and final      ⇒ Refuted

Consequence: shadow compile is not the full pipeline; the byte-cost
  reported during training is fiction. Block. Investigate T8.3 / T8.2 /
  Phase E export determinism.

Warning band: 1024 < |shadow - final| <= 2048 ⇒ Pass-with-shadow-gap-warning
  (other H13 predictions still required to confirm).
```

## H14 Pareto frontier soundness (non-closure-gating)

```text
Statement:
  The Pareto frontier emitter computes dominance and selection per D11
  exactly on a pinned fixture vector with hand-computed dominance.

Predicted (given S5_FRONTIER_FIXTURE_V1 pinned in
fixtures/frontier/s5_frontier_fixture.toml):
  frontier_emitter(fixture).frontier = S5_FRONTIER_EXPECTED_FRONTIER_V1
  frontier_emitter(fixture).selected = S5_FRONTIER_EXPECTED_SELECTED_V1
  Empty-frontier: frontier=[], selected=None
  Single-point:   frontier=[P], selected = if any-of false-gates: None else: Some(P)
  All-fail:       selected = None

Falsification: any of the fixture invariants violated ⇒ Refuted.

Non-closure-gating rule:
  H14 Refuted does NOT block closure by itself. It downgrades the
  s5_frontier.v1 report to record selection_authority = "manual-override"
  and the s5_report.v1 must cite the manual override in §Decision.
  H13/H15 still gate closure as normal.
```

## H15 Emulator harness one-token agreement (mandatory)

```text
Statement:
  Under D12, the BoundedKv seed-0 ROM emits exactly one token t_emit
  within the pinned budget; t_emit equals the ArtifactOracle's
  predicted first token for prompt P2 byte-for-byte; AND replays
  produce bit-identical (t_emit, ticks_consumed, harness_self_hash).

Predicted:
  harness.token_emitted   = t_emit (single u8 charset_v1 id)
  harness.ticks_exhausted = false
  harness.ticks_consumed  in (0, ClockCycles(DMG_FRAME_CLOCK_CYCLES * 240)]
  harness.token_emitted   = artifact_oracle.predict_first_token(
                              P2, ema_checkpoint_seed_0, charset_v1)
  harness.agreement       = true

Falsification:
  ticks_exhausted = true                                          ⇒ Refuted
  token_emitted != oracle                                         ⇒ Refuted
  replay drift on (token, ticks, self_hash)                       ⇒ Refuted
  first video-commit event contains zero tokens or more than one
  charset_v1 token                                                ⇒ Refuted

Consequence: the integration path is broken end-to-end. Distinguish:
  - ticks_exhausted: cooperative scheduler / video_commit / cold-start
  - oracle disagreement: numeric divergence (ternary projection on
    export, activation fake quant, tile/residency, integer accumulator)
  - replay drift: gbf-emu DeterminismPolicy escape (audio, RTC, ...)
```

## H16 Compiler feedback loop convergence (mandatory)

```text
Statement:
  apply_feedback (D13) applies the deterministic safe_bound update
  rule and produces finite, non-zero scalar updates on a pinned
  synthetic fixture; the empty ExpertSlotAffinity case is a pure
  no-op (byte-identical router_state).

Predicted (S5_FEEDBACK_FIXTURE_V1, pinned in
fixtures/feedback/s5_feedback_fixture.toml):
  apply_feedback(in=4.0,  max_abs=5.0  fixture).out = 4.4
  apply_feedback(in=8.0,  low-range    fixture).out = 7.6
  apply_feedback(in=16.0, max-bound    fixture).out = 16.0
  apply_feedback(in=0.5,  low-bound    fixture).out = 0.5

  apply_affinity_hints(router_state=R, hints=[]).router_state = R
    (byte-identical, no-op)

Falsification:
  output != expected; non-finite; empty case mutates router_state;
  empty case panics or yields NaN; non-determinism across replays
  ⇒ Refuted.
Consequence: shadow -> training feedback loop is broken. Block.
  Investigate T8.5 / fixture parsing / range_hotspots plumbing.
```

## H17 Logging overhead gate (mandatory)

```text
Statement:
  Logging-overhead measurement (D14) reports median_overhead < 0.01
  on the pinned tiny workload, AND the gate fires when overhead is
  artificially inflated (e.g. by injecting a synthetic 5 ms sleep
  into the logging path).

Predicted:
  measured_overhead      < 0.01
  inflated_overhead     >= 0.01
  gate_fires_on_inflated = true

Falsification:
  measured_overhead >= 0.01                                            ⇒ Refuted
  inflated_overhead < 0.01 (test failed to actually inflate)           ⇒ Refuted
  gate does not fail CI on inflated build                              ⇒ Refuted
  procedure does not record warmup/measured/metric/predicate           ⇒ Refuted

Consequence: real perf regression OR gate is silent. Block.
```

Hypothesis composition rules are formalized in §12 (Outcome algebra).

---

# 4. Authority rules

```text
Scope(F-S5) =
  {
    H1..H17,
    BoundedKv K cap pinning (D2),
    DecayPolicy::Fixed and DecayPolicy::MultiTimescale variants (D2),
    AttentionOracleFixtureSuite (D3),
    BringUp CompileProfile defaults (D7),
    RuntimeChromeBudget pinned shape (D9, D9-tol, D10),
    Shadow compile cadence + pipeline (D11) including
      S5_SHADOW_PIPELINE_STAGES,
    Pareto frontier rubric (D5 axes + D11 selection),
    Emulator one-token harness (D12, D17),
    Compiler feedback safe_bound rule (D13),
    Logging-overhead gate (D14),
    per-export re-validation (D18),
    all s5_*.v1 artifact schemas (see §13),
    S5 reproducibility laws (extension of S1 Rep-1..Rep-8),
    S5 falsification suite
  }

Rule Authority:
  ∀ behavior b in Scope(F-S5) and this RFC specifies b
  ⇒ SourceOfTruth(b) = this RFC.

Rule PlanContext:
  Behavior outside Scope informed by planv0 amendments and bd-36y1 /
  bd-1cdu comments. Closed features and closed S1..S4 RFCs provide
  substrate; S5 does not amend their contracts; it consumes them.

Rule InheritedFromS1:
  S1's seed list (D2), deterministic batch sampling (D3a), strict
  reproducibility (D8), fail-closed on NaN (D9), AdamW pinning (D10),
  measurement-oracle obligation (in S5-adapted form §3), and
  reset-context chunk semantics (chunk_size = 128) are inherited.

Rule InheritedFromS2:
  Ternary QAT contract (per-row Q8.8, AnnealedGlobalThenPerOutputRow,
  hard ternary at Phase C entry, activation fake quant at Phase D
  entry) inherited unchanged.

Rule InheritedFromS3:
  charset_v1 token table, KN-5 baseline contract, three-way oracle
  agreement (Denotational + Artifact + Schedule), v0_success
  WorkloadManifest, ConformanceEnvelope schema, ReferenceModelBundle
  + ArtifactOracle export inherited unchanged. S5 invokes v0_success
  per variant per D19.

Rule InheritedFromS4:
  Project Gutenberg corpus, promotion gate, contamination report,
  corpus-progression schedule, KN-5 baseline inherited from S4
  unchanged. S5 does not refit the baseline.

Rule InheritedFromF-A7:
  gbf-emu adapter, DeterminismPolicy, BootMode, PowerOnRamPolicy,
  trap dispatcher, run_fast_for budget semantics inherited unchanged.
  S5 invokes DeterminismPolicy::default() per D17.

Rule InheritedFromF-B*:
  Compiler pipeline stage definitions (ArtifactValidationAndUpgrade
  through EncodedRom), ReachabilityValidation, ResourceStateValidation,
  PlacedRom, EncodedRom, RangePlan/ArenaPlan/RomWindowPlan certificates,
  BuildIdentityBlock inherited unchanged. S5 binds and asserts; does
  not redefine.

Rule CrateOwnership:
  Every behavior in Scope(F-S5) is implemented in exactly one of:
    gbf-experiments  (NEW S5 modules in gbf_experiments::s5::*; absorbs
                       what was originally gbf_experiments::s6::*)
    gbf-policy       (BringUp CompileProfile registry entry; pinned
                       S5_SHADOW_PIPELINE_STAGES constant)
    gbf-model        (DecayPolicy enum + MultiTimescale partitioning;
                       attention-oracle reference impl)
    gbf-train        (Burn adapter for BoundedKv gradient smoke;
                       shadow_compile A/B + real-pipeline wiring;
                       SequenceBlock dispatch; preflight; feedback
                       consumer)
    gbf-codegen      (consumes; no new types beyond what F-B* owns)
    gbf-runtime      (consumes; chrome_budget.json emitter + nucleus
                       hash drift gate live here per T2.2 / T2.5,
                       which are F-A epic deliverables S5 binds)
    gbf-data         (consumes Gutenberg manifest)
    gbf-foundation   (no new behavior)
    gbf-artifact     (CheckpointFrontierPoint canonical encoding)
    gbf-cli          (`gbf s5` subcommand for replay, regress, oracle,
                       verify-determinism)
  No S5-specific code lives outside this set.

Rule Amendment:
  Later slice changes any of:
    BoundedKv K cap (D2)
    DecayPolicy variants (D2)
    Attention-oracle tolerance (D3)
    Attention-oracle fixture suite (D3)
    Frontier scoring rubric (D5, D11)
    BringUp profile defaults (D7)
    RuntimeChromeBudget pinned shape (D9, D9-tol, D10)
    Shadow cadence (D11)
    Emulator harness contract (D12, D17)
    Compiler feedback consumer rule (D13)
    Logging-overhead threshold (D14)
    Per-export re-validation gate (D18)
  ⇒ Later slice's RFC must explicitly amend this RFC.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation
  produces the expected Refuted verdict on the appropriate hypothesis.
  Falsification sensitivity is a first-class proof obligation (§16).

Rule ScopeBoundary (the merge boundary):
  Act I (Pick) and Act II (Fit) close together as one slice. There is
  no closure boundary between them; both bd-36y1 and bd-1cdu close
  on the same PR merge. The two-act decomposition is a presentation
  device; the hypotheses, schemas, and falsification suite form a
  single algebra.
```

---

# 5. Experiment state machine

The combined slice executes a single state machine. Act I (Pick) runs
first; Act II (Fit) starts when the Pick artifacts (per-seed checkpoints
and frontier inputs) are emitted. The two acts share trainer process
state where possible (shadow_compile cadence emits records in both acts;
the difference is which API surface the cadence step calls into).

```text
S5StateMachine :=
  initial state: PreRegistered
  states:
    PreRegistered
    PickRunning(variant, seed)
    PickEvalDone(variant, seed)
    PickScoreDone(variant, seed)
    PickAttentionOracleDone(variant=BoundedKv, seed)
    PickShadowDone(variant, seed, cadence_step)
    PickFrontierInputsFrozen
    FitPreflightDone(seed)
    FitShadowRealDone(seed, cadence_step)
    FitFeedbackApplied(seed, phase_boundary)
    FitExportDone(seed)
    FitRevalidationDone(seed)
    FitEncodedRomEmitted(seed)
    FitHarnessRun(seed=0)
    FrontierEmitted
    LoggingOverheadMeasured
    ReportEmitted
    Completed

  transitions (in declared order; deterministic):
    PreRegistered
      -> PickRunning(BoundedKv, 0)
      -> ... (per (variant, seed) cross product, 15 runs total)
      -> PickFrontierInputsFrozen

    PickFrontierInputsFrozen
      -> FitPreflightDone(0)
      -> FitShadowRealDone(0, 4000)
      -> FitFeedbackApplied(0, 6000)
      -> FitFeedbackApplied(0, 6001)
      -> FitShadowRealDone(0, 8000)
      -> FitFeedbackApplied(0, 12000)
      -> FitShadowRealDone(0, 12000)
      -> FitShadowRealDone(0, 16000)
      -> FitShadowRealDone(0, 20000)
      -> FitFeedbackApplied(0, 20000)
      -> FitExportDone(0)
      -> FitRevalidationDone(0)
      -> FitEncodedRomEmitted(0)
      -> (repeat FitPreflight/Shadow/Feedback/Export/Revalidation/
          EncodedRom for seeds 1..4; emulator harness is seed-0 only
          and runs exactly once)
      -> FitHarnessRun(0)
      -> FrontierEmitted
      -> LoggingOverheadMeasured
      -> ReportEmitted
      -> Completed

  early-exit transitions:
    DivergedAt(step) in any PickRunning -> Fail-substrate (D15)
    AnyAttentionOracleAgreementFalse -> Fail-attention-oracle (H1)
    AnyMissingFrontierAxis -> Fail-frontier-incomplete (H7)
    RuntimeNucleusHashDrift mid-run -> Fail-runtime-budget (H11)
    ReVal blocked -> Fail-runtime-budget (D18 / H11)
    AnyShadowStageMissing -> Fail-shadow-compile (H13)
    AnyEncodedRomCertInvalid -> Fail-encoded-rom (H15 / Act II §9)
    HarnessTicksExhausted OR HarnessOracleDisagree -> Fail-emulator-harness
    InflatedLoggingGatePassed -> Fail-logging-overhead (H17)

  every state transition emits an immutable artifact whose hash binds
  the previous state hash chain; replay verifies the chain.
```

State-to-hypothesis traceability:

```text
PickAttentionOracleDone(BoundedKv, s)  binds H1, H9 (BoundedKv KV reset)
PickRunning(v, s) completion           binds H2 (BoundedKv autodiff),
                                              H3 (L_MT4 stability),
                                              H4 (three-variant smoke),
                                              H10 (per-variant determinism)
PickShadowDone(v, s, k) at stub level  binds H6
PickFrontierInputsFrozen               binds H7, and the Pick-side of
                                              H5 / H8 / H10's frontier hash
FitPreflightDone(0)                    binds H11, H12
FitShadowRealDone(s, k) at real level  binds H13
FitFeedbackApplied(s, b)               binds H16
FitExportDone(s) + FitRevalidationDone binds H11 (re-val), D18
FitEncodedRomEmitted(s)                binds the encoded-ROM cert chain
FitHarnessRun(0)                       binds H15
FrontierEmitted                        binds H14
LoggingOverheadMeasured                binds H17
ReportEmitted                          binds the full hypothesis verdict set
```

---

# 6. Sequence-state-variant contract

This section combines the BoundedKv contract (originally S5 §6),
LinearState DecayPolicy contract (S5 §7), and the attention-oracle
contract (S5 §6.2-§6.5).

## 6.1 BoundedKv block contract

```text
BoundedKvBlock (S5 instance) consumes BoundedKvSpec (D2) and:
  - holds an opaque KV slab of max_context * kv_bytes_per_token bytes
    per layer (1 layer for Toy0; n_blocks * slab_bytes overall);
  - on each forward step, evicts FIFO when the slab is at capacity,
    appends the current (Q-projected key, V-projected value) entry,
    and computes scaled-dot-product over the live entries with causal
    mask;
  - holds a "valid" flag per entry; eviction is by oldest valid flag;
  - produces d_model output via the output projection;
  - resets at chunk boundary (chunk_size = 128) per H9.

Forbidden behaviors:
  - retaining KV entries across chunk boundaries (violates H9)
  - any RNG access during forward (violates H10)
  - any host-clock read during forward
  - any tensor allocation per step (slab is pre-allocated)

Pin: the executable record layout (1 f32 valid + 3 f32 tied-KV payload)
matches gbf-model::sequence::bounded_kv. Bump the gbf-model crate
version to change the record layout.

LongRangeRepetitionPenalty (§3 of original S5):
  For a generated token sequence g of length N >= 64:
    let R := { (i, j) : 0 <= i < j < N, g[i] = g[j], j - i >= 64 }
    pair_weighted_sum(g) := sum_{(i,j) in R} 1 / (j - i)
    LongRangeRepetitionPenalty(g) := pair_weighted_sum(g) / N
  This per-generated-token value is non-negative; larger means more
  long-range repetition.
  H5 compares
  L_MT4 vs L_FIX1 on this quantity over the v0_success generation
  prompt set; threshold 0.10 pair-weighted sum / generated token
  (Predicted) and 0.05 pair-weighted sum / generated token
  (Falsification).
```

## 6.2 Attention-oracle reference contract

```text
AttentionOracleReference :=
  pure Rust impl of bounded causal attention with FIFO eviction at K_cap,
  evaluated in f64 internally, downcast to f32 only at output.

Properties (all required):
  AO-1  No Burn dependency.
  AO-2  No Burn module handle, live trainer state, host runtime state,
        or checkpoint object dependency: the oracle takes canonical
        projection tensors extracted from the checkpoint/artifact under
        test and the activation-fake-quant clip bound as inputs.
  AO-3  No RNG: the oracle is a pure function of (fixture, serialized
        projections, clip bound).
  AO-4  Bit-identical output across runs on the same platform.
  AO-5  oracle_self_hash binds the canonical f64 weights, the f32
        activation clip, and the fixture inputs; replay reproduces it.

Inputs:
  - charset_v1 token-id sequence (AOF-1..AOF-5)
  - Phase A teacher checkpoint's canonical projection tensors, resolved via
    QuantSpec::weight_quant when the checked path is quantized, otherwise
    the exact fp projection tensors used by the Phase A model (NOT by
    tensor-id naming convention; see the conformance/oracle bead-closure
    skill)
  - activation-fake-quant clip bound (from PhaseScheduler at Phase A
    exit; pinned in fixtures/attention_oracle/s5_oracle.toml)

Outputs:
  per (fixture, position) f32 logits over d_model.

Domain: f64 internally; downcast at output. The trained BoundedKv
block computes in f32. The agreement check (H1) is in f32.
```

## 6.3 Attention-oracle fixture suite

```text
Pinned at fixtures/attention_oracle/s5_oracle.toml:
  fixture_suite_self_hash : Hash256
  spec_self_hash          : Hash256

  AOF-1 : len=1   source_corpus=gutenberg_val, byte_offset=<pinned>
  AOF-2 : len=32
  AOF-3 : len=128
  AOF-4 : len=192
  AOF-5 : len=512

Each fixture's token_ids are recorded inline (no hidden offset rewrite).
Fixture suite hash drift (any token id or any byte offset changing
between RFC commit and run) is an H1 Refuted condition. Regenerating
the suite is an RFC-amendment step.
```

## 6.4 Latency proxy formula (Pick-axis; informational for Fit)

```text
latency_proxy_cycles(v) =
    sum over forward steps of
      (variant kernel cycles per step)
  + (state-update cycles per step)
  + (KV-eviction cycles per step)
  ; pinned constants per variant in fixtures/proxies/s5_latency.toml

For Act II, the Pareto axis shadow_latency_proxy_cycles uses the real
ScheduleCostAnalysis estimate plus:
  + (max_bank_switches_per_token * BANK_SWITCH_PROXY_CYCLES)
  + (overlay_installs_per_token  * OVERLAY_INSTALL_PROXY_CYCLES)

BANK_SWITCH_PROXY_CYCLES     = 24    ; [ESTIMATE]; see A14 in §19
OVERLAY_INSTALL_PROXY_CYCLES = 256   ; [ESTIMATE]; see A14 in §19

Both are analytic proxies, not measured cycle counts. F4-broken
substitute (§16) rejects a build that omits the bank-switch contribution.
```

## 6.5 LinearState DecayPolicy contract changes

```text
DecayPolicy::MultiTimescale partitions state_slots into decays.len()
contiguous equal-width bands. For decays = [0.5, 0.75, 0.875, 0.9375]
and state_slots = 16, the bands are slots 0..4 (decay 0.5), 4..8 (0.75),
8..12 (0.875), 12..16 (0.9375).

Forward-pass decay application:
  state_b_new[i] = decays[b] * state_b_old[i] + linear_update[i]
  for each band b and slot i within band b.

Gradient smoke (extension of S2's H6):
  trainable: decay_state weights, projection weights, per-row Q8.8 scales.
  stop-gradient: decays[b] values (constants in S5), state buffer bytes,
    TernaryThreshold values (Phase A).
  H3 + the Pick-axis gradient smoke (T12.5 / bd-1y1s) asserts finite,
  nonzero grads in trainable and exact zero in stop-gradient, with
  bit-identical Hash256 over the gradient bytes per parameter.

Phase A teacher checkpoint contract (L_MT4):
  At Phase A exit, the L_MT4 teacher's state must contain finite values
  in every band at every recorded eval step. Any band collapsing to
  non-finite is an H3 Refuted condition.
```

## 6.6 BoundedKv contract operation (Pick-axis)

```text
operation s5_pick_run
  input:   { variant: VariantId, seed: Seed,
             corpus_manifest: CorpusManifestRef,
             charset_v1: CharsetTableRef,
             phase_scheduler: PhaseSchedulerConfig,
             quant_spec: QuantSpec,
             device_profile: S1CpuDeterministic,
             pass_version: SemVer }
  output:  { checkpoint_phase_a: SafetensorsRef,
             checkpoint_phase_d: SafetensorsRef,
             run_log: s5_run_log.v1,
             score: s5_score.v1,
             attention_oracle: Null | s5_attention_oracle.v1,
                                       ; Some only for BoundedKv
             shadow_samples_stub: List[s5_shadow_compile_sample_stub.v1],
             v0_success: s5_v0_success_per_variant.v1 }

Invariants:
  Pick-1  variant in {BoundedKv, L_FIX1, L_MT4}; rejecting Learned at
          construction (D2).
  Pick-2  seeds = [0..4]; one run per (variant, seed).
  Pick-3  reset-context chunk semantics preserved (H9).
  Pick-4  BoundedKv K cap = 128, kv_bytes_per_token = 16 (D2).
  Pick-5  Attention-oracle run is BoundedKv-only; the per-(variant,
          seed) emitter MUST skip oracle production for LinearState
          variants and assert oracle=None in the score record.
  Pick-6  shadow_compile_sample emission is per-cadence-step; missing
          a cadence sample is an H6 Refuted condition.
  Pick-7  v0_success invocation is per-variant per D19; the workload
          manifest is loaded by sha and not mutated.
```

---

# 7. Runtime + CompileProfile contract

This section combines RuntimeChromeBudget end-to-end (originally S6 §6)
with the CompileProfile + WRAM Layout contract (S6 §7).

## 7.1 RuntimeChromeBudget operation

```text
operation s5_runtime_chrome_budget_emit
  input:   { source: SyntheticReference | RealShellBuild }
  output:  RuntimeChromeBudget (per §1 type)

Invariants:
  CB-1  Synthetic emission carries runtime_nucleus_hash with
        "SYNTHETIC_REFERENCE:" prefix per D10.
  CB-2  Real emission carries runtime_nucleus_hash =
        sha256(assembled_runtime_nucleus_bytes); does NOT carry the
        prefix.
  CB-3  Synthetic emission is deterministic given the pinned ReferenceShellSpec.
  CB-4  Real emission is deterministic given the same runtime shell build
        (same rustc, same Cargo.lock, same gbf-runtime pass_version).
  CB-5  rom_slots is sorted by (class, id) before hashing.
  CB-6  chrome_budget_self_hash round-trips through canonical JSON
        with chrome_budget_self_hash omitted.
```

## 7.2 Synthetic-vs-real agreement contract

```text
operation s5_budget_agreement_check
  input:   { synthetic: RuntimeChromeBudget, real: RuntimeChromeBudget }
  output:  BudgetAgreementReport (s5_budget_agreement.v1; see §13)

Invariants (the D9-tol expansion):
  Ag-Ok-1  For every RomBudgetSlot s:
             |real.usable_bytes(s) - synthetic.usable_bytes(s)| <= 256
             real.reserved_slack(s)               >= synthetic.reserved_slack(s)
             real.placement_caps(s)               == synthetic.placement_caps(s)
  Ag-Ok-2  real.wram_reserved == synthetic.wram_reserved (all fields)
  Ag-Ok-3  real.sram_reserved == synthetic.sram_reserved
  Ag-Ok-4  agreement = (all of Ag-Ok-1..3); nucleus_hash_match is
           informational only (per D10 synthetic-vs-real are NOT equal).

Failure mode:
  any Ag-Ok-* false ⇒ H11 Refuted.
```

## 7.3 runtime_nucleus_hash CI drift gate

```text
scripts/check-nucleus-drift.sh (provided by gbf-runtime, S5 binds):
  reads the live shell build's runtime_nucleus_hash from the emitted
  chrome_budget.json; compares against the hash pinned in
  fixtures/runtime/nucleus_pin.toml. Exits non-zero on mismatch.

Trainer self-check (T2.5 plumbing):
  At every preflight invocation, the trainer reads the live
  RuntimeChromeBudget and compares runtime_nucleus_hash against the
  one pinned at the start of the run. Mismatch mid-run ⇒ refuse to
  advance past the next step, log Fail-runtime-budget(
    reason="nucleus-hash-drift-mid-run", expected=..., actual=...).

CI gate:
  After the first real chrome_budget.json is committed:
    - any new training run that records the SYNTHETIC_REFERENCE: hash
      is CI-rejected;
    - any training run whose recorded nucleus hash differs from the
      live shell's emission is CI-rejected.
  Before the first real chrome_budget.json: synthetic-only runs are
  legal and the gate is dormant.
```

## 7.4 Training preflight wiring (Fit-axis)

```text
operation s5_preflight
  input:   { artifact: ModelArtifact, profile: CompileProfileId,
             budget: RuntimeChromeBudget }
  output:  DeployabilityReport

DeployabilityReport :=
  {
    schema:              "s5_deployability_report.v1"
    artifact_sha:        Hash256
    profile_sha:         Hash256
    budget_sha:          Hash256
    fits_envelope:       Bool
    hard_failures:       List[HardFailure]
    soft_warnings:       List[SoftWarning]
    wram_fit_report:     WramFitReport          ; BringUp-specific
    switch_budget_report: SwitchBudgetReport    ; BringUp-specific
    diagnostic_log:      List[DiagnosticEntry]
    report_self_hash:    Hash256
  }

HardFailure :=
  {
    check_name:       String
    slot_id:          BudgetSlotId
    slot_class:       BudgetSlotClass
    over_by_bytes:    u32
    usable_minus_slack: u32
    suggested_fix:    String          ; e.g. "reduce d_ff from 224 to 208"
  }

Invariants:
  PF-1  preflight(in_budget, BringUp).fits_envelope = true,
        hard_failures = [].
  PF-2  preflight(over_budget, BringUp).fits_envelope = false; every
        hard_failure has slot_id + slot_class + over_by_bytes +
        suggested_fix populated.
  PF-3  preflight(in_budget, profile=Default) succeeds and does not
        include BringUp-specific WramFitReport fields. preflight(in_budget,
        profile=BringUp) succeeds and does include the BringUp-specific
        WramFitReport fields.
  PF-4  diagnostic_log captures every check_name + status + numeric
        value + threshold per bd-1u1.
  PF-5  CompileRequest.profile field is byte-identical to the profile
        forwarded into every downstream pipeline stage (asserted via
        canonical JSON of resolved CompileProfile in PlacedRom report).
```

## 7.5 Per-export re-validation (D18)

```text
operation s5_re_validation
  input:   { export_pass: ExportPass, current_budget: RuntimeChromeBudget }
  output:  ReValidationReport

ReValidationReport :=
  {
    schema:                          "s5_re_validation.v1"
    export_pass_sha:                 Hash256
    training_time_budget_sha:        Hash256
    current_budget_sha:              Hash256
    runtime_nucleus_hashes_match:    Bool
    per_slot_byte_deltas:            List[{
                                       slot_id: BudgetSlotId,
                                       slot_class: BudgetSlotClass,
                                       delta_bytes: i64
                                     }]
    fits_envelope:                   Bool
    outcome:                         "Pass" | "Warn" | "BlockExport"
    diagnostic:                      String
    revalidation_self_hash:          Hash256
  }

Invariants (the D18 expansion):
  RV-1  outcome = Pass     iff runtime_nucleus_hashes_match AND fits_envelope
  RV-2  outcome = Warn     iff !runtime_nucleus_hashes_match AND
                                fits_envelope AND
                                (∀ s. |delta_bytes(s)| <= D9-tol).
  RV-3  outcome = BlockExport iff any fits_envelope check fails OR
                                synthetic-vs-real mismatch (D10) OR
                                (!runtime_nucleus_hashes_match AND
                                 any |delta_bytes(s)| > D9-tol).
  RV-4  diagnostic on BlockExport MUST contain both runtime nucleus
        hashes AND per-slot byte deltas AND the offending slot id.
  RV-5  re_validation report is written BEFORE EncodedRom bytes are
        written (debugging aid, not a normative hash invariant).
```

## 7.6 BringUp CompileProfile registry entry

```text
CompileProfileSpec for BringUp (canonical JSON shape; pinned in
gbf-policy):

CompileProfileSpec :=
  {
    schema:                            "s5_compile_profile.v1"
    id:                                "BringUp"
    wram_layout:
      { overlay_bytes:        4096,
        continuation_bytes:    256,
        stack_bytes:           256,
        hot_arena_bytes_min:  2048,
        reserve_bytes:        1536 }
    overlay_reload:                    "PerExpertSwitch"
    max_bank_switches_per_token:       8
    sequence_state:
      { kind:      "BoundedKv",
        k_cap:     128 }
    placement_profile:                 "StrictOnePerBank"
    max_refinement_iters:              1
    allow_placement_profile_fallback:  false
    allow_trace_demotion:              false
    allow_overlay_promotion:           false
    allow_recompute_promotion:         false
    profile_self_hash:                 Hash256
  }

Invariants:
  PR-Self-Hash    profile_self_hash round-trips through canonical JSON
                  with profile_self_hash omitted.
  PR-BringUp      For id = "BringUp", every field equals D7.
  PR-Determinism  Two CompileProfileSpec instances with the same field
                  values produce the same profile_self_hash.
```

## 7.7 CompileRequest threading

```text
CompileRequestSpec :=
  {
    schema:             "s5_compile_request.v1"
    artifact_ref:       Hash256
    profile:            CompileProfileId
    target:             TargetProfileId
    objectives:         CompileObjectiveSpec
    risk_policy:        RiskPolicySpec
    chrome_budget_ref:  Hash256
    request_self_hash:  Hash256
  }

Invariants:
  CR-Self-Hash    round-trips.
  CR-ProfileFlow  CompileRequest.profile is byte-identical to the profile
                  forwarded into every downstream pipeline stage.
  CR-BudgetBound  CompileRequest.chrome_budget_ref equals the
                  RuntimeChromeBudget that the artifact was preflighted
                  against; mismatch is an H12 Refuted condition.
```

---

# 8. Shadow compile + Pareto contract

This section combines the Pick-axis shadow_compile API surface
exercise (originally S5 §8) with the Act II real shadow compile
pipeline (originally S6 §8) and the Pareto frontier emitter.

## 8.1 ShadowCompileSampleStub (Pick-axis)

```text
operation s5_shadow_compile_stub_sample
  input:   { trainer_state: TrainerState, cadence_step: ShadowStep,
             variant: VariantId, seed: Seed }
  output:  s5_shadow_compile_sample_stub.v1

This call exercises the shadow_compile API surface on the deployable
ternary export of the current checkpoint. It does NOT exercise the
full ROM emission pipeline. It records:
  shadow_byte_cost       : u32   (from API surface, not a constant)
  shadow_kernel_count    : u32   (from API surface)
  shadow_compile_ok      : bool  (from API surface call result)
  shadow_compile_skipped : Option<SkipReason>

Invariants:
  SHST-1  shadow_byte_cost is finite (not u32::MAX sentinel).
  SHST-2  shadow_kernel_count > 0.
  SHST-3  shadow_compile_ok is the raw bool from the API call.
  SHST-4  BoundedKv: shadow_compile_ok = true at every cadence step
                     >= 4000.
  SHST-5  Each LinearState variant: at least one cadence sample with
                     shadow_compile_ok = true.
```

## 8.2 ShadowCompileSampleReal (Fit-axis)

```text
operation s5_shadow_compile_real_sample
  input:   { ema_checkpoint: EmaCheckpoint, profile: CompileProfileId,
             budget: RuntimeChromeBudget, workload: PinnedWorkload }
  output:  s5_shadow_compile_sample.v1 (ShadowCompileSampleReal)

This call exercises the FULL F-B* pipeline; stages_executed MUST equal
S5_SHADOW_PIPELINE_STAGES verbatim (per D11).

Invariants:
  SHR-1  For healthy closure samples, stages_executed =
         S5_SHADOW_PIPELINE_STAGES and shadow_compile_ok = true.
         For deliberately broken substitutes, stages_executed records
         every stage attempted before failure and failure_stage records
         the first missing or failed stage.
  SHR-2  shadow_byte_cost is the StaticBudgetReport projection (not a
         constant; not a measurement of the final EncodedRom).
  SHR-3  shadow_kernel_count is the count after StaticBudgetReport.
  SHR-4  fits_envelope = DeployabilityReport.fits_envelope on the
         shadow EncodedRom.
  SHR-5  reachability_cert_valid / resource_state_cert_valid from
         the certs emitted by the shadow pipeline.
  SHR-6  shadow_latency_proxy_cycles = ScheduleCostAnalysis estimate
         + bank-switch + overlay-install proxy contributions (per §6.4).
  SHR-7  shadow_energy_proxy_units = sum of weighted kernel cycles
         per slice; pinned formula in gbf-policy.
  SHR-8  compiler_feedback_sha = sha256 over the canonical JSON of the
         compiler_feedback.json emitted at this cadence step.
  SHR-9  When built from the same checkpoint, compiler_feedback.json
         fields are byte-identical to the final-export feedback
         EXCEPT { generated_at_us, build_id_uuid, observation_seq }.
  SHR-10 |shadow_at_step_20000.shadow_byte_cost -
          final.encoded_rom_byte_cost| <= 1024 bytes (per H13 strict);
         warning band 1024..2048 yields Pass-with-shadow-gap-warning;
         > 2048 ⇒ H13 Refuted.
  SHR-11 shadow_at_step_20000.shadow_kernel_count == final.kernel_count.
```

## 8.3 EMA weight export (T8.2)

```text
operation s5_ema_export
  input:   { live_weights: LiveWeights, ema_state: EmaState,
             cadence_step: ShadowStep }
  output:  EmaCheckpoint

EmaCheckpoint :=
  {
    schema:          "s5_ema_checkpoint.v1"
    seed:            Seed
    cadence_step:    ShadowStep
    weights_sha:     Hash256
    ema_alpha:       f32                  ; pinned per S2
    ema_self_hash:   Hash256
  }

Invariants:
  EMA-1  ema_self_hash round-trips.
  EMA-2  weights_sha = sha256(canonical safetensors payload).
  EMA-3  EMA export is deterministic given (live_weights, ema_state,
         cadence_step).
```

## 8.4 Pareto frontier emitter

```text
operation s5_frontier_emit
  input:   { points: List[CheckpointFrontierPoint],
             keep_frontier: u32 (default 3),
             axes: List[String] (pinned to D5 verbatim) }
  output:  FrontierReport (s5_frontier.v1; see §13)

Dominance predicate:
  Point A dominates B iff A is >= on every "higher-is-better" axis
  (v0_success_pass, shadow_compile_ok_at_end, fits_envelope,
  reachability_cert_valid, resource_state_cert_valid) AND <= on every
  "lower-is-better" axis (val_bpc_fp, val_bpc_ternary, ternary_gap,
  projected_deployed_bytes, shadow_byte_cost_at_end,
  shadow_kernel_count_at_end, latency_proxy_cycles, encoded_rom_byte_cost).

Selection:
  Filter out points where any of {shadow_compile_ok_at_end,
  fits_envelope, reachability_cert_valid, resource_state_cert_valid}
  is false; from the remaining, pick min val_bpc_ternary; break ties
  by min encoded_rom_byte_cost only when both candidates have a
  present value; if one candidate has Null and the other has Some,
  encoded_rom_byte_cost is not treated as better or worse and the
  comparison falls through to lex (variant, seed, cadence_step).

Empty-input invariant: emitter returns frontier=[], selected=None.
Single-point: frontier=[P], selected = (Some(P) if all gates true else None).
All-fail:     selected = None.

Invariants:
  PF-1  Same input points + same axes -> byte-identical FrontierReport.
  PF-2  frontier.len() <= keep_frontier.
  PF-3  selected ∈ frontier or None.
  PF-4  selection_authority = "automated" unless H14 Refuted, in which
        case the report records "manual-override" per §12 dispatch.

The pinned fixture S5_FRONTIER_FIXTURE_V1 lives in
fixtures/frontier/s5_frontier_fixture.toml with hand-computed
dominance relations; S5_FRONTIER_EXPECTED_FRONTIER_V1 and
S5_FRONTIER_EXPECTED_SELECTED_V1 are committed before any real run.
H14's falsification suite asserts that a broken emitter (e.g. one
that always picks the first point) fails the fixture.
```

## 8.5 FrontierRecommendation computation (Pick-axis)

```text
operation s5_frontier_recommendation
  input:   { per_variant_aggregates: VariantAggregates }
  output:  {
    frontier_recommendation: FrontierRecommendation in {A, B, Tie},
    frontier_leader_variant: Null | VariantId
  }

Rule (mirrors D5):
  if BoundedKv beats both L_FIX1 and L_MT4 on aggregate val_bpc_ternary
     by >= 0.05 AND BoundedKv passes v0_success AND
     shadow_compile_ok_at_end is true:
    A
  elif any LinearState variant in {L_FIX1, L_MT4} beats BoundedKv on
       aggregate val_bpc_ternary by >= 0.05 AND passes v0_success:
    B, with frontier_leader_variant set to the best LinearState
    variant by val_bpc_ternary, tie-broken by encoded_rom_byte_cost
    when both candidate costs are present, then by VariantId
  else:
    Tie, with frontier_leader_variant = Null

Invariants:
  FR-1  Recommendation is a license, not a winner. Decision dispatch
        (§12) reads it to label Pass-with-A-frontier /
        Pass-with-B-frontier / Pass-with-tie.
  FR-2  Recommendation does NOT override D8's primary integration
        choice for Act II (BoundedKv always).
  FR-3  frontier_leader_variant is populated only for B, and may be
        either L_FIX1 or L_MT4. A and Tie record Null.
```

---

# 9. EncodedRom + ReachabilityValidation contract

This section is the Act II end-to-end ROM build contract (originally
S6 §9). S5 binds the F-B* pipeline contracts; it does not redefine
them.

## 9.1 EncodedRom build operation

```text
operation s5_encoded_rom_build
  input:   { artifact: ModelArtifact, profile: CompileProfileId,
             budget: RuntimeChromeBudget }
  output:  EncodedRomBundle

EncodedRomBundle :=
  {
    schema:                    "s5_encoded_rom.v1"
    artifact_sha:              Hash256
    compile_profile_sha:       Hash256
    chrome_budget_sha:         Hash256
    encoded_rom_sha:           Hash256  ; CanonicalRomPayloadHash
    gb_sha:                    Hash256
    sym_sha:                   Hash256
    lst_sha:                   Hash256
    cert_shas: {
      range_cert_sha:          Hash256
      arena_cert_sha:          Hash256
      window_cert_sha:         Hash256
      reachability_cert_sha:   Hash256
      resource_state_cert_sha: Hash256
    }
    placed_rom_report_sha:     Hash256
    build_identity_block: {
      abi_version:              SemVer
      artifact_core_hash:       Hash256
      lowering_hash:            Hash256
      compile_request_hash:     Hash256
      runtime_nucleus_hash:     Hash256
      target_profile:           TargetProfileId
    }
    bundle_self_hash:          Hash256
  }

CanonicalRomPayloadHash:
  encoded_rom_sha = sha256(rom.gb || "\n--SYM--\n" || rom.sym ||
                           "\n--LST--\n" || rom.lst)

Invariants:
  ER-1  bundle_self_hash round-trips through canonical JSON with
        bundle_self_hash omitted.
  ER-2  Every cert_sha matches the certs/*.cert.json sha on disk.
  ER-3  build_identity_block.runtime_nucleus_hash equals
        chrome_budget.runtime_nucleus_hash of the budget that was
        preflighted against.
  ER-4  encoded_rom.byte_length(.gb) > 0 AND <= 524288 (conservative
        MBC5 budget; H15-style sanity).
  ER-5  Every ReferenceShellSpec.included module has at least one
        binding in the .sym map.
  ER-6  Determinism: same (artifact, profile, budget) -> bit-identical
        (.gb, .sym, .lst).
  ER-7  build_identity_block.artifact_core_hash equals the artifact
        hash used by ArtifactOracle::predict_first_token for H15.
```

## 9.2 ReachabilityValidation binding

```text
ReachabilityReport :=
  {
    schema:                                  "s5_reachability.v1"
    cert_sha:                                Hash256
    isr_reachable_outside_bank0:             List[SectionId]
    forbidden_mbc_writes_outside_banklease:  List[SectionId]
    illegal_machine_effects:                 List[SectionId]
    switchable_bank_dependencies_on_isr:     List[SectionId]
    unreachable_continuation_targets:        List[SectionId]
    classification_summary: {
      isr_reachable_count:        u32
      yield_resume_reachable_count: u32
      fault_path_reachable_count: u32
      harness_entry_reachable_count: u32
      bank_lease_protected_count: u32
      normal_only_count:          u32
    }
    reachability_self_hash:                  Hash256
  }

Invariants:
  RC-1  reachability_self_hash round-trips.
  RC-2  cert_sha matches certs/reachability.cert.json sha recorded in
        s5_encoded_rom.v1.
  RC-3  For closure (H15 Predicted), all five violation lists are
        empty.
  RC-4  classification_summary counts derive deterministically from
        the section partition; replay reproduces them.
```

## 9.3 ResourceStateValidation binding

```text
ResourceStateValidation cert is consumed and asserted as valid
per H15 Predicted. The cert structure is owned by F-B* / bd-2pl;
this RFC asserts cert validity and binds resource_state_cert_sha
into the EncodedRomBundle.

For closure:
  resource_state_cert_valid = true on the seed-0 final EncodedRom
  AND on every cadence step's shadow EncodedRom that has
  shadow_compile_ok = true.
```

---

# 10. Emulator harness contract

This section is the Act II one-token harness (originally S6 §10).
gbf-emu, DeterminismPolicy, BootMode, and run_fast_for are F-A7 contracts
inherited unchanged.

## 10.1 One-token harness operation

```text
operation s5_emulator_one_token_harness
  input:   { encoded_rom: EncodedRomBundle,
             ema_checkpoint_seed_0: SafetensorsRef,
             prompt: PinnedPrompt (P2),
             determinism: DeterminismPolicy::default(),
             boot_mode: BootMode::PostBootDmg,
             budget: ClockCycles(DMG_FRAME_CLOCK_CYCLES * 240) }
  output:  EmuHarnessOutcome

Procedure:
  1. Load EncodedRom into gbf-emu under the pinned DeterminismPolicy
     and BootMode (per D17).
  2. Install a single PC trap at VIDEO_COMMIT_TOKEN_TRAP_PC, resolved
     at link time from the .sym map (no hard-coded addresses; see
     §19 A22).
  3. Execute run_fast_for(budget).
  4. Capture the first token byte committed to the video commit queue
     that carries a charset_v1 token.
  5. Invoke ArtifactOracle::predict_first_token(P2,
     ema_checkpoint_seed_0, charset_v1) to obtain the oracle's
     predicted first token.
  6. Compute agreement = (token_emitted == oracle_predicted_token)
     under canonical token comparison (single u8 equality after
     charset_v1 token-id lookup).
  7. If the budget is exhausted without a video commit, return
     ticks_exhausted = true, agreement = false.
  8. Emit EmuHarnessOutcome.

Invariants:
  EH-1  Single PC trap, no other harness intrusion.
  EH-2  No host-clock read during harness execution (the
        determinism_policy_hash is deterministic).
  EH-3  No RNG access outside DeterminismPolicy's pinned seed.
  EH-4  Replay under the same (seed, EncodedRom_sha,
        DeterminismPolicy, BootMode) produces byte-identical
        (token_emitted, ticks_consumed, harness_self_hash).
  EH-5  determinism_policy_hash = sha256(canonical JSON of
        DeterminismPolicy::default() under S5CanonicalJson).
```

## 10.2 ArtifactOracle binding (oracle round-trip)

```text
ArtifactOracle::predict_first_token (S3-owned, F-C2 closed) is invoked
with:
  - the prompt (P2 = single charset_v1 token, zero-context)
  - the ema_checkpoint_seed_0 safetensors (deployable weights resolved
    via QuantSpec::weight_quant, NOT by tensor-id naming convention)
  - charset_v1 token table

Pin: oracle output MUST be re-derived each invocation; caching is
forbidden (would mask numeric drift). The oracle's own self-hash is
recorded; replay reproduces it.

If the oracle and the harness disagree, the integration is broken.
Distinguishing the failure mode (see H15 Consequence) is a debugging
exercise, not a closure escape hatch.
```

## 10.3 Determinism Policy hash

```text
DeterminismPolicy::default() (F-A7 §0.1) carries:
  power_on_ram_policy:            GameroyDefault
  audio_output_enabled:           false
  real_time_cartridge_rtc:        false
  save_state_metadata_timestamp:  fixed (zeroed)
  joypad_input_stream:            EMPTY
  seed_for_internal_rng:          pinned

The canonical JSON of this struct under S5CanonicalJson is hashed
into determinism_policy_hash and recorded in s5_emulator_harness.v1.
Customizing the policy invalidates closure.
```

---

# 11. Feedback loop contract

This section is the compiler-feedback-into-training consumer
(originally S6 §11). The rule is pinned in D13; this section is the
operational shape.

## 11.1 apply_feedback operation

```text
operation s5_apply_feedback
  input:   { current: PhaseSchedulerState, feedback: CompilerFeedback,
             config: FeedbackApplyConfig }
  output:  FeedbackApplyResult (s5_feedback_apply.v1; see §13)

FeedbackApplyConfig (pinned):
  safe_bound_min:      0.5
  safe_bound_max:      16.0
  grow_alpha:          0.10
  shrink_alpha:        0.95
  shrink_threshold:    0.5     ; max_abs <= 0.5 * current ⇒ shrink

Channel A rule (per D13):
  for each layer_id l with feedback.range_hotspots[l]:
    cur = current.safe_bound[l]
    ma  = feedback.range_hotspots[l].max_abs
    if ma > cur:
      new = cur + min(grow_alpha * cur, 0.5 * (ma - cur))
    elif ma <= shrink_threshold * cur:
      new = cur * shrink_alpha
    else:
      new = cur
    new = clamp(new, safe_bound_min, safe_bound_max)
    record (cur, new) in safe_bound_in / safe_bound_out vectors.

Channel B rule:
  if feedback.affinity_hints.is_empty():
    affinity_was_no_op = true
    do NOT mutate router_state at all (byte-identical no-op).
  else:
    (S7 territory; in S5 this branch is unreachable for the dense
    baseline. Any attempt to take this branch in S5 is an H16
    Refuted condition: empty-affinity case mutated router_state.)

Invariants:
  FA-1  All updates are finite f64, deterministic, reproducible.
  FA-2  Empty-affinity case is a byte-identical no-op.
  FA-3  apply records only at phase boundaries: steps 6000, 6001,
        12000, 20000 (FA-PhaseBoundary).
  FA-4  apply_self_hash round-trips.
  FA-5  On the pinned fixture S5_FEEDBACK_FIXTURE_V1, apply produces
        byte-identical FeedbackApplyResult.
```

## 11.2 When apply runs

```text
apply_feedback is invoked exactly at:
  step 6000   : Phase A -> Phase B boundary
  step 6001   : Phase B -> Phase C boundary
  step 12000  : Phase C -> Phase D boundary
  step 20000  : Phase D -> Phase E boundary (final apply)

Mid-phase apply is forbidden: changing activation range targets while
QAT hardness is also changing confounds the two motions. Any
FeedbackApplyResult at any step OTHER than the four above is an H16
Refuted condition.
```

---

# 12. Outcome algebra

The merged slice's outcome algebra has 18 variants. Names align with
the merge resolution rules (see merge instructions §7 "Outcome algebra
variants"). The dispatch order mirrors the state machine: substrate
failures first, then frontier-incompleteness, then Pick-axis specific,
then Fit-axis specific, with non-blocking variants (H5/H8/H14) folded
into Pass variants.

```text
S5Outcome :=
    Pass-clean
      ; Alias for Pass-with-A-frontier when all mandatory hypotheses are
      ; Confirmed, H5/H8/H14 are Confirmed, and no warning band applies.

  | Pass-with-A-frontier
      ; All mandatory Confirmed. FrontierRecommendation = A. H5 may
      ; be Confirmed or Refuted; H8 Confirmed (BoundedKv leads).

  | Pass-with-B-frontier
      ; All mandatory Confirmed. FrontierRecommendation = B. H8
      ; Refuted (LinearState beats BoundedKv on bpc); the surprise
      ; is recorded honestly. Act II still runs BoundedKv as primary
      ; per D8 and Confirmed H11..H17.

  | Pass-with-tie
      ; All mandatory Confirmed. FrontierRecommendation = Tie. H5 + H8
      ; each may be Refuted; closure proceeds.

  | Pass-with-frontier-warning
      ; All mandatory Confirmed. H14 Refuted; report records
      ; selection_authority = "manual-override".

  | Pass-with-shadow-gap-warning
      ; All mandatory Confirmed. H13's shadow-vs-final byte-cost gap
      ; in the warning band (1024 < gap <= 2048).

  | Fail-frontier-incomplete
      ; H7 Refuted (closure deliverable does not exist).

  | Fail-attention-oracle
      ; H1 Refuted. BoundedKv's conformance role is lost; integration
      ; (H15) cannot ride on the oracle agreement.

  | Fail-bounded-kv-grad
      ; H2 Refuted. Burn autodiff path through BoundedKv is broken.

  | Fail-linearstate-grad
      ; H3 Refuted (L_MT4 unstable or band collapse).

  | Fail-runtime-budget
      ; H11 Refuted OR D18 BlockExport fired OR trainer self-check on
      ; mid-run nucleus drift fired.

  | Fail-compile-profile
      ; H12 Refuted.

  | Fail-shadow-compile
      ; H13 Refuted (other than the shadow-gap warning band).

  | Fail-encoded-rom
      ; The seed-0 EncodedRom failed any cert validity check or any
      ; ReferenceShellSpec module had bind_count = 0. (Folded from
      ; the original S6 H5 here.)

  | Fail-emulator-harness
      ; H15 Refuted (ticks exhausted, oracle disagreement, or replay
      ; drift).

  | Fail-feedback-loop
      ; H16 Refuted.

  | Fail-logging-overhead
      ; H17 Refuted.

  | Fail-substrate
      ; D15 fired during any (variant, seed) Pick run (NaN or
      ; divergence). Dominates all other outcomes.
```

Dispatch (fail-fast; first matching rule wins):

```text
if any (v, s) raised DivergedAt(_) OR non-finite loss/grad
                                                       ⇒ Fail-substrate
elif H1  Refuted                                       ⇒ Fail-attention-oracle
elif H2  Refuted                                       ⇒ Fail-bounded-kv-grad
elif H3  Refuted                                       ⇒ Fail-linearstate-grad
elif H7  Refuted                                       ⇒ Fail-frontier-incomplete
elif H4  Refuted                                       ⇒ Fail-substrate
                                                          (capacity failure;
                                                           bpc/v0_success gate
                                                           failed)
elif H6  Refuted                                       ⇒ Fail-shadow-compile
elif H9  Refuted                                       ⇒ Fail-substrate
                                                          (reset-boundary leak;
                                                           bpc unreliable)
elif H10 Refuted                                       ⇒ Fail-substrate
                                                          (per-variant determinism)
elif H11 Refuted OR D18 BlockExport OR self-check fired
                                                       ⇒ Fail-runtime-budget
elif H12 Refuted                                       ⇒ Fail-compile-profile
elif H17 Refuted                                       ⇒ Fail-logging-overhead
elif H16 Refuted                                       ⇒ Fail-feedback-loop
elif any EncodedRom cert invalid OR bind_count = 0 on
     a ReferenceShellSpec module                       ⇒ Fail-encoded-rom
elif H13 Refuted (outside warning band)                ⇒ Fail-shadow-compile
elif H13 in shadow-warning band only                   ⇒ Pass-with-shadow-gap-warning
elif H15 Refuted                                       ⇒ Fail-emulator-harness
elif H14 Refuted AND no manual override                ⇒ Fail-frontier-incomplete
                                                          (Pareto-broken sub-variant;
                                                           see §15 protocol)
elif H14 Refuted AND manual override provided          ⇒ Pass-with-frontier-warning
elif FrontierRecommendation = A AND H5 Confirmed AND H8 Confirmed
     AND H14 Confirmed                                 ⇒ Pass-clean
elif FrontierRecommendation = A                        ⇒ Pass-with-A-frontier
elif FrontierRecommendation = B                        ⇒ Pass-with-B-frontier
elif FrontierRecommendation = Tie                      ⇒ Pass-with-tie
```

Decision dispatch:

```text
Pass-clean                       → Decision::ProceedToS7
Pass-with-A-frontier             → Decision::ProceedToS7
Pass-with-B-frontier             → Decision::ProceedToS7-with-warning(
                                     "B-frontier surprise; LinearState-MT
                                      beat BoundedKv on bpc; Act II still
                                      ran BoundedKv per D8")
Pass-with-tie                    → Decision::ProceedToS7
Pass-with-frontier-warning       → Decision::ProceedToS7-with-warning(
                                     "frontier emitter required manual
                                      override; H14 Refuted")
Pass-with-shadow-gap-warning     → Decision::ProceedToS7-with-warning(
                                     "shadow-vs-final byte-cost gap in
                                      warning band 1024..2048")
Fail-runtime-budget              → Decision::Investigate(
                                     "runtime-budget-or-revalidation")
Fail-compile-profile             → Decision::Investigate(
                                     "compile-profile-binding")
Fail-shadow-compile              → Decision::Investigate(
                                     "shadow-pipeline")
Fail-frontier-incomplete         → Decision::Investigate(
                                     "frontier-or-pareto")
Fail-attention-oracle            → Decision::Investigate(
                                     "attention-oracle-or-boundedkv-forward")
Fail-bounded-kv-grad             → Decision::Investigate("boundedkv-autodiff")
Fail-linearstate-grad            → Decision::Investigate("l_mt4-stability")
Fail-encoded-rom                 → Decision::Investigate(
                                     "encoded-rom-or-reachability")
Fail-emulator-harness            → Decision::Investigate(
                                     "emulator-or-runtime-or-numeric")
Fail-feedback-loop               → Decision::Investigate(
                                     "feedback-consumer-rule")
Fail-substrate                   → Decision::Investigate(
                                     "substrate-determinism-or-capacity")
Fail-logging-overhead            → Decision::Halt(
                                     "logging-overhead-violation")
```

`Halt` blocks bd-36y1 AND bd-1cdu closure unconditionally.
`Investigate` creates a follow-up bead and may extend this RFC's
scope or fixture set.

---

# 13. Artifact schemas (all `s5_*.v1`)

S5 emits the largest artifact set of any slice. Each schema is
`s5_*.v1`, canonical-JSON-serialized under S5CanonicalJson, and
self-hash-bound. Binary blobs (.gb, .sym, .lst, certs/*.cert.json,
safetensors) are bound by recorded Hash256 fields.

All s6_*.v1 schemas from the original F-S6 RFC are renamed to s5_*.v1
in this merged file (merge resolution rule 2).

## 13.1 s5_run_log.v1

```text
Path: experiments/S5/runs/{variant}/seed-{seed}/run_log.json
Per S1's run-log shape extended with `variant` field; per-seed; finite
loss and grad-norm trace; per-eval-step bpc; canonical-JSON +
self-hash.

Invariants:
  RL-Self-Hash    round-trips.
  RL-Variant      variant field is one of the three VariantId strings.
  RL-Determinism  Same (variant, seed, config) -> identical
                  run_log_self_hash.
```

## 13.2 s5_score.v1

```text
Path: experiments/S5/runs/{variant}/seed-{seed}/score.json
Per S1's score shape, per-variant. Carries val_bpc_fp, val_bpc_ternary,
ternary_gap, attention_oracle_self_hash (Null for LinearState),
context-length spy fixture results (for H9), self-hash.

Invariants:
  SC-Self-Hash    round-trips.
  SC-OracleField  attention_oracle_self_hash is Null iff variant !=
                  BoundedKv (Pick-5).
```

## 13.3 s5_attention_oracle.v1 (BoundedKv only)

```text
Path: experiments/S5/runs/boundedkv/seed-{seed}/attention_oracle.json
Per §1 AttentionOracleReport.

Invariants:
  AO-Self-Hash    oracle_self_hash round-trips.
  AO-FixtureBound fixture_suite_sha = pinned hash from
                  fixtures/attention_oracle/s5_oracle.toml.
  AO-Spec         spec_sha = pinned hash from same file.
  AO-Coverage     per_fixture_results covers all five AOF-1..AOF-5 at
                  every position.
  AO-Pre-2        Replay under S1CpuDeterministic reproduces every
                  Hash256 in per_fixture_results.
```

## 13.4 s5_shadow_compile_sample_stub.v1 (Pick-axis)

```text
Path: experiments/S5/runs/{variant}/seed-{seed}/shadow_stub/step-{step}.json
Per §1 ShadowCompileSampleStub.

Invariants:
  SHST-Self-Hash  round-trips.
  SHST-Cadence    one file per (variant, seed, cadence_step); 75 files
                  total per slice (3 variants * 5 seeds * 5 cadence).
```

## 13.5 s5_shadow_compile_sample.v1 (Fit-axis, real)

```text
Path: experiments/S5/runs/seed-{seed}/shadow/{step-{cadence_step}.json | phase-e-final.json}
Per §1 ShadowCompileSampleReal.

Invariants:
  SH-Self-Hash    round-trips.
  SH-Cadence      One file per (seed, cadence_step); 30 files total
                  per S5 PR (5 seeds * 6 cadence emissions: 5 mid +
                  Phase E final).
  SH-StagesEqual  stages_executed = S5_SHADOW_PIPELINE_STAGES iff
                  shadow_compile_ok = true (SHR-1).
```

## 13.6 s5_v0_success_per_variant.v1

```text
Path: experiments/S5/runs/{variant}/seed-{seed}/v0_success.json
Records per-variant invocation of the S3 v0_success workload (D19).
Carries workload_manifest_sha, per-prompt outcomes, aggregate score,
pass bit, self-hash.

Invariants:
  V0-Self-Hash      round-trips.
  V0-ManifestPin    workload_manifest_sha matches S3's pinned manifest.
  V0-NoManifestEdit S5 does NOT mutate the workload; CI rejects PRs
                    that edit the S3 manifest.
```

## 13.7 s5_frontier.v1

```text
Path: experiments/S5/frontier/s5_frontier.json
The single combined frontier report for both Pick and Fit axes.

FrontierReport (JSON) :=
  {
    schema:                "s5_frontier.v1"
    frontier_axes:         List[String]     ; D5 array, fixed order
    variant_records:       List[VariantRecord]   ; one per VariantId
    pick_points:           List[CheckpointFrontierPoint]
                                              ; exactly one final Pick point per
                                              ; (variant, seed)
    fit_points:            List[CheckpointFrontierPoint]
                                              ; BoundedKv-only closure Fit points unless
                                              ; optional LinearState Fit is run
    frontier:              List[CheckpointFrontierPoint]
                                              ; bounded Pareto frontier
    selected:              Null | CheckpointFrontierPoint
    selection_authority:   "automated" | "manual-override"
    frontier_recommendation: "A" | "B" | "Tie"
    frontier_leader_variant: Null | VariantId
                          ; set to L_FIX1 or L_MT4 only when
                          ; frontier_recommendation = "B"
    dominance_log_sha:     Hash256
    frontier_self_hash:    Hash256
  }

VariantRecord :=
  {
    variant:               VariantId
    per_seed:              List[{ seed: Seed, axes: AxisMap }]
                                              ; one entry per seed
    aggregate:             AxisMap
    record_self_hash:      Hash256
  }

Invariants:
  FR-Self-Hash    frontier_self_hash round-trips.
  FR-AxisOrder    frontier_axes equals the literal D5 array order;
                  reordering invalidates Rep-S5-2.
  FR-VariantCount variant_records.length = 3 (per H7).
  FR-Coverage     ∀ vr, ∀ axis in frontier_axes, ∀ s in {0..4}.
                  vr.per_seed[s].axes[axis] is non-null.
  FR-FitNullRule  Nullable Fit-only fields are forbidden in fit_points
                  and allowed only in pick_points.
  FR-FrontierBound frontier.len() <= keep_frontier (default 3 per D11).
  FR-SelectedSubset selected ∈ frontier or None.
  FR-DeterministicEmission Same input points -> byte-identical FrontierReport.
```

## 13.8 s5_runtime_chrome_budget.v1

```text
Path: experiments/S5/budget/{synthetic | real}.runtime_chrome_budget.json
Per §1 RuntimeChromeBudget.

Invariants:
  CB-Self-Hash    DomainHash round-trips with chrome_budget_self_hash omitted.
  CB-SyntheticTag synthetic instance ⇒ runtime_nucleus_hash starts with
                  "SYNTHETIC_REFERENCE:".
  CB-RealTag      real instance ⇒ no prefix AND
                  = sha256(assembled_runtime_nucleus_bytes).
  CB-Determinism  Replay with same shell build ⇒ identical bytes.
```

## 13.9 s5_budget_agreement.v1

```text
Path: experiments/S5/budget/agreement.json

BudgetAgreementReport (JSON) :=
  {
    schema:                "s5_budget_agreement.v1"
    synthetic_self_hash:   Hash256
    real_self_hash:        Hash256
    per_slot_deltas:       List[{
                              id: BudgetSlotId,
                              class: BudgetSlotClass,
                              usable_delta_bytes: i64,
                              slack_delta_bytes: i64
                            }]
    wram_match:            Bool
    sram_match:            Bool
    nucleus_hash_match:    Bool
    nucleus_kind:          "synthetic-vs-real" | "real-vs-real"
                                              | "synthetic-vs-synthetic"
    agreement:             Bool
    report_self_hash:      Hash256
  }

Invariants:
  BA-Self-Hash    round-trips.
  BA-AgreementRule agreement = (Ag-Ok-1 AND Ag-Ok-2 AND Ag-Ok-3) per §7.2.
```

## 13.10 s5_compile_profile.v1

```text
Path: experiments/S5/profile/bringup.compile_profile.json
Per §7.6 CompileProfileSpec.
```

## 13.11 s5_compile_request.v1

```text
Path: experiments/S5/runs/seed-{seed}/compile_request.json
Per §7.7 CompileRequestSpec.
```

## 13.12 s5_deployability_report.v1

```text
Path: experiments/S5/runs/seed-{seed}/deployability_report.json
Per §7.4 DeployabilityReport.

Invariants:
  DP-Self-Hash    round-trips.
  DP-DiagnosticLog diagnostic_log captures every check_name + status
                   + numeric_value + threshold per bd-1u1.
```

## 13.13 s5_re_validation.v1

```text
Path: experiments/S5/runs/seed-{seed}/re_validation.json
Per §7.5 ReValidationReport.
```

## 13.14 s5_encoded_rom.v1

```text
Path: experiments/S5/runs/seed-{seed}/encoded_rom/
       bundle.json
       rom.gb
       rom.sym
       rom.lst
       certs/range.cert.json
       certs/arena.cert.json
       certs/window.cert.json
       certs/reachability.cert.json
       certs/resource_state.cert.json
       placed_rom_report.json
       build_identity_block.json

Per §9.1 EncodedRomBundle; emitted as bundle.json.

Invariants:
  ER-Self-Hash    bundle.json round-trips.
  ER-RomHash      encoded_rom_sha = CanonicalRomPayloadHash.
  ER-Certs        Every cert file referenced in bundle.json exists at
                  the listed path AND its sha256 matches the recorded
                  *_cert_sha field.
  ER-BuildIdentity build_identity_block.runtime_nucleus_hash = chrome
                   budget that was preflighted against.
```

## 13.15 s5_reachability.v1

```text
Path: experiments/S5/runs/seed-{seed}/reachability_report.json
Per §9.2 ReachabilityReport.
```

## 13.16 s5_emulator_harness.v1

```text
Path: experiments/S5/runs/seed-0/emulator_harness/p2.json
Per §1 EmuHarnessOutcome.

Invariants:
  EH-Self-Hash         round-trips.
  EH-Replay            Replays under D17 produce byte-identical outcome
                       modulo on-disk path of the EncodedRom.
  EH-DeterminismHash   determinism_policy_hash = sha256(canonical JSON
                       of DeterminismPolicy::default() under S5CanonicalJson).
```

## 13.17 s5_feedback_apply.v1

```text
Path: experiments/S5/runs/seed-{seed}/feedback/step-{cadence_step}.json
Per §11.1 FeedbackApplyResult.

Invariants:
  FA-Self-Hash    round-trips.
  FA-PhaseBoundary cadence_step ∈ {6000, 6001, 12000, 20000}
                   (per §11.2); apply at any other step ⇒ H16 Refuted.
  FA-DeterminismOnFixture On the pinned synthetic fixture (H16
                   Predicted), apply produces byte-identical
                   FeedbackApplyResult.
```

## 13.18 s5_logging_overhead.v1

```text
Path: experiments/S5/logging_overhead/report.json

LoggingOverheadReport (per §12 of original F-S6):
  {
    schema:                "s5_logging_overhead.v1"
    workload_id:           String
    workload_self_hash:    Hash256
    warmup_iterations:     u32                 ; 5
    measured_iterations:   u32                 ; 50
    median_baseline_ns:    u64
    median_instrumented_ns: u64
    overhead:              f64                 ; (instr - base) / base
    threshold:             f64                 ; 0.01
    pass:                  Bool
    constitution_section:  String              ; "II.1"
    measurement_self_hash: Hash256
  }

Invariants:
  LO-Self-Hash    round-trips.
  LO-Threshold    threshold = 0.01 (D14).
  LO-Pass         pass = (overhead < threshold).
```

## 13.19 s5_report.v1

```text
Path: docs/experiments/S5-report.md

Front-matter (YAML, hashed into report):
  ---
  schema:                       "s5_report.v1"
  s5_outcome:                   S5Outcome
  decision:                     Decision
  primary_variant:              "boundedkv"      ; per D8
  primary_seed:                 0                ; emulator harness seed
  frontier_recommendation:      "A" | "B" | "Tie"
  per_variant_artifacts:
    List[{
      variant: VariantId,
      per_seed: List[{
        seed: Seed,
        run_log_sha:                  Hash256,
        score_sha:                    Hash256,
        attention_oracle_sha:         Null | Hash256,
        shadow_stub_shas:             List[Hash256],
        v0_success_sha:               Hash256,
        frontier_point_sha:           Hash256
      }]
    }]
  per_seed_fit_artifacts:
    List[{
      seed: Seed,
      preflight_report_sha:       Null | Hash256,
      shadow_real_shas:           List[Hash256],
      compile_request_sha:        Null | Hash256,
      compile_profile_sha:        Hash256,
      encoded_rom_bundle_sha:     Null | Hash256,
      reachability_report_sha:    Null | Hash256,
      resource_state_cert_sha:    Null | Hash256,
      re_validation_sha:          Null | Hash256,
      feedback_apply_shas:        List[Hash256]
    }]
  emulator_harness_sha:           Null | Hash256      ; seed 0 only
  budget_agreement_sha:           Hash256
  frontier_self_hash:             Hash256
  logging_overhead_sha:           Hash256
  generated_at:                   RFC3339 UTC, informational only,
                                   excluded from report hash.
  rfc_revision:                   GitCommitId | Hash256
  predictions_section_hash:       Hash256
  predictions_commit:             GitCommitId
  first_result_commit:            GitCommitId
  pass_version:                   SemVer
  report_self_hash:               Hash256
  ---

Required sections (markdown body):
  ## Pre-registered predictions
  ## Pick axis observations (H1..H10)
  ## Fit axis observations (H11..H17)
  ## Falsification analysis (per hypothesis verdict)
  ## Surprise log
  ## Decision

Invariants:
  RP-Self-Hash             report_self_hash round-trips with the field
                           omitted from canonical JSON.
  RP-Predictions-Ancestry  predictions_commit precedes first_result_commit
                           in git history (per S1 R-Predictions).
  RP-Decision-Single       Exactly one Decision value, computed by §12
                           dispatch.
  RP-FrontierMirror        report.frontier_recommendation =
                           s5_frontier.v1.frontier_recommendation
                           (bitwise equal).
```

---

# 14. Reproducibility laws

These laws extend S1's Rep-1..Rep-8 with per-variant determinism,
per-export determinism, and emulator deterministic-execution
guarantees.

```text
Rep-S5-1  per-(variant, seed) bit-identical safetensors under replay
          given the inputs in D16.

Rep-S5-2  per-(variant, seed) bit-identical run_log, score, oracle,
          shadow-stub, and v0_success-per-variant JSON self-hashes
          under replay.

Rep-S5-3  variant-namespaced RNG: every random draw in S5 derives from
          ChaCha20 seeded by sha256("gbf:s5:{variant_id}:{namespace}:{seed}").
          No global RNG access from any variant.

Rep-S5-4  per-(seed, cadence_step) bit-identical real shadow records
          under replay given the same ema_checkpoint and the same
          (profile, budget, workload).

Rep-S5-5  per-(seed, export_pass) bit-identical (.gb, .sym, .lst)
          under replay given the inputs in D16.

Rep-S5-6  cross-build attention-oracle re-derivation: the
          AttentionOracleReport is identical when produced by S5-build-A
          and S5-build-B (the cross-build oracle harness; see §18).

Rep-S5-7  frontier byte-equality under replay: same
          (run_log + score + oracle + shadow-stub + shadow-real +
           v0_success-per-variant + Pareto inputs) ⇒ byte-identical
          s5_frontier.v1.

Rep-S5-8  emulator deterministic execution: ∀ seed=0 replay r1, r2.
          r1.token_emitted = r2.token_emitted
          r1.ticks_consumed = r2.ticks_consumed
          r1.harness_self_hash = r2.harness_self_hash
          under DeterminismPolicy::default() + BootMode::PostBootDmg.

Rep-S5-9  per-export re-validation idempotence: re-running
          s5_re_validation against the same (export_pass, current_budget)
          inputs produces byte-identical ReValidationReport.

Rep-S5-10 logging-overhead measurement reproducibility: median is
          stable to within 0.5% across two consecutive runs of the
          pinned workload (this is a sanity check; CI does not gate
          on it).

Rep-S5-11 feedback-apply determinism: on the pinned synthetic fixture,
          apply produces byte-identical FeedbackApplyResult across
          replays.

Rep-S5-12 reverse-order replay: replaying any per-(variant, seed) Pick
          run or per-seed Fit export in REVERSE order produces the same
          hashes as forward-order replay. (Catches hidden global mutable
          state; mirrors S1's O9 in the multi-variant + multi-seed
          setting.)
```

S1's Rep-1..Rep-8 (single-machine, no host clock for non-informational
fields, no network during runs, deterministic batch sampling, etc.)
are inherited unchanged and apply to every S5 process.

---

# 15. Decision protocol (closure of bd-36y1 + bd-1cdu)

This section is the human-facing closure protocol. Both bd-36y1 and
bd-1cdu close on the SAME PR merge per the merge resolution rules.
One may be retired as duplicate in a follow-up bead-graph op (owner:
user); the RFC itself does not prescribe which.

```text
Closure protocol:

1. Pre-registration commit must precede the first per-(variant, seed)
   or per-seed result-artifact commit on the closure PR. The
   predictions section of s5_report.v1 (markdown body) is committed
   first; its sha is pinned in predictions_commit and asserted by CI
   against the git history of the predictions section.

2. The closure PR contains:
   a. Three variant_records' worth of Pick-axis artifacts (run_log,
      score, attention_oracle for BoundedKv, shadow_stub, v0_success,
      frontier_point) per (variant, seed) for 15 (variant, seed) pairs.
   b. Five seeds' worth of Fit-axis artifacts (preflight, shadow_real,
      compile_request, compile_profile, encoded_rom, reachability,
      re_validation, feedback_apply) PLUS one seed-0 emulator_harness.
   c. One s5_frontier.v1.
   d. One s5_budget_agreement.v1 + one synthetic + one real
      s5_runtime_chrome_budget.v1.
   e. One s5_logging_overhead.v1.
   f. One s5_report.v1 with Outcome + Decision.

3. CI gates (must all pass before bd-36y1 + bd-1cdu can close):
   - cargo fmt --check --all
   - cargo clippy --workspace --features "s5-default,qat,burn-adapter" -- -D warnings
   - cargo test --workspace --features "s5-default,qat,burn-adapter"
   - scripts/s5_feature_matrix_check.sh
   - scripts/check-nucleus-drift.sh
   - scripts/s5_logging_overhead_check.sh
   - scripts/s5_predictions_ancestry.sh (RP-Predictions-Ancestry)
   - scripts/s5_falsification_suite.sh  (substrate-only F13/F14/F15 policy
       checks + dry-run feature matrix; live F1..F15 producer loop owner:
       bd-q3zo)
   - scripts/s5_reproducibility_smoke.sh (Rep-S5-1, Rep-S5-2, Rep-S5-5,
       Rep-S5-7, Rep-S5-8, Rep-S5-12 on the tiny in-repo fixture)

4. The Decision value MUST be one of:
     { ProceedToS7, ProceedToS7-with-warning(_) }
   Any other Decision (Investigate, Halt) blocks closure.

5. Both bd-36y1 and bd-1cdu MUST be closed together. The closure
   comment on each cites:
   - the merge changelog header
   - the s5_outcome and Decision
   - the QAT checklist (per .agents/skills/qat-bead-closure/SKILL.md)
   - the asm-bead-closure checklist (for the .gb / .sym / .lst
     payload, per .agents/skills/asm-bead-closure/SKILL.md)
   - the model-contract-bead-closure checklist (for the BoundedKv +
     LinearState contracts)
   - the sequence-state-bead-closure checklist
   - the fixture-bead-closure checklist (for AttentionOracleFixtureSuite,
     S5_FRONTIER_FIXTURE_V1, S5_FEEDBACK_FIXTURE_V1, v0_success_subset_S5)
   - the logging-bead-closure checklist (for s5_logging_overhead)

6. Feature closures: F2 (bd-1hv), F8 (bd-2am), F11 (bd-1i8) (all
   originally S6) and F12 (bd-144) (originally S5) close alongside
   bd-36y1 and bd-1cdu.

7. F-A7 (bd-3mxe) MUST be closed before bd-1cdu can close: the
   emulator harness depends on F-A7's pinned API. F-B1 (compute
   bringup) is already merged on main (per planv0 amendment 7).

8. Follow-up beads:
   - Decay::Learned variant (out of D2 scope)
   - L_MT4 band layout alternatives (Interleaved, RandomInit)
   - feedback consumer config sweep (alternate grow_alpha / shrink_alpha)
   - real cycle measurement (S6-bench / M2-M6 territory)
   - duplicate-bead retirement (one of bd-36y1 / bd-1cdu)
```

---

# 16. Proof obligations (combined falsification suite)

Each numbered O-obligation is a CI-enforced test or script. The
F1..F15 falsification cases (down from S5's 7 + S6's 9 = 16 by
folding one same-substrate duplicate per merge rule 8) are the
deliberately-broken substitutes that must each produce the expected
Refuted verdict.

```text
O1  Pre-registration ancestry
    predictions_commit precedes first_result_commit on the closure
    branch. Enforced by scripts/s5_predictions_ancestry.sh.

O2  Per-(variant, seed) determinism smoke
    Two replays of (BoundedKv, 0) produce identical safetensors hash
    + run_log_self_hash + score_self_hash. Tiny-fixture variant runs
    in CI; full Gutenberg in closure run.

O3  Cross-build attention-oracle re-derivation
    AttentionOracleReport produced by S5-build-A and S5-build-B are
    byte-identical (Rep-S5-6).

O4  Frontier byte-equality under replay
    s5_frontier.v1 is byte-identical across two end-to-end replays
    given the same per-(variant, seed) inputs.

O5  Per-seed Fit determinism smoke
    Two replays of seed=0 export produce identical (.gb, .sym, .lst)
    sha + cert shas + re_validation_self_hash + emulator_harness_self_hash.

O6  RuntimeChromeBudget synthetic-vs-real round-trip
    BudgetAgreementReport pass per Ag-Ok-1..3 on the live shell build.

O7  CI drift gate (nucleus hash)
    scripts/check-nucleus-drift.sh exits non-zero on an altered
    nucleus build.

O8  Reverse-order replay
    Replaying (variant, seed) tuples in REVERSE order yields the
    same per-pair hashes (Rep-S5-12).

O9  Cross-seed difference sanity
    For every variant, at least two seeds produce different safetensors
    hashes. (Catches ignored seed / non-seeded RNG; mirrors S1 O9.)

O10 Shadow-vs-final byte cost
    For seed in {0..4},
      |shadow_at_20000.shadow_byte_cost - final.encoded_rom_byte_cost|
        <= 1024 (strict pass) or <= 2048 (warning band).

O11 EncodedRom cert validity
    range, arena, window, reachability, resource_state certs all
    valid on every final EncodedRom emitted for seeds {0..4}.

O12 ReferenceShellSpec module binding
    .sym map has bind_count > 0 for each of the 8 modules in D9's
    reference_shell_modules.

O13 Emulator one-token determinism
    Two replays under D17 produce identical (token_emitted,
    ticks_consumed, harness_self_hash).

O14 Oracle agreement on harness
    The seed-0 harness's token_emitted equals
    ArtifactOracle::predict_first_token output.

O15 Feedback apply on synthetic fixture
    apply_feedback on S5_FEEDBACK_FIXTURE_V1 produces the four
    pinned (in, out) pairs of D13.

O16 Oracle is not cached
    Calling ArtifactOracle::predict_first_token twice with the same
    inputs produces the same output AND the oracle's internal hash
    proves re-derivation (no memoized cache between invocations).

O17 Logging overhead measured + inflated
    measured_overhead < 0.01 AND inflated_overhead >= 0.01 AND gate
    fires on inflated build.

O18 Falsification sensitivity (all F-cases below)
    Each F-case substitution produces the expected Refuted verdict.

F-cases (deliberately-broken substitutes for §16 O18):

F1  oracle-equality-tampered
    Modify the attention-oracle reference to add a constant 5e-4 to
    every logit. Expected: H1 Refuted (max_abs_diff > 1e-4).

F2  boundedkv-autodiff-broken
    Stop-gradient ALL parameters of BoundedKv in the Burn adapter.
    Expected: H2 Refuted (grad_norm = 0 on trainable).

F3  l_mt4-band-collapse
    Substitute MultiTimescale decays = [NaN, NaN, NaN, NaN].
    Expected: H3 Refuted (band mean non-finite).

F4  capacity-undersized
    Set Toy0 d_model to a value too small to beat KN-5 by 0.05 bpc.
    Expected: H4 Refuted.

F5  shadow-ok-constant-true
    Hardcode shadow_compile_ok = true in the API surface (Pick) AND
    in the real pipeline (Fit). Expected: H6 Refuted (constant-ok
    suspect) AND H13 Refuted (constant true).

H6 negative-control fixture:
  At least one Pick-axis shadow_compile API call MUST be run on a pinned
  broken deployable artifact that is known to fail. The expected result is
  shadow_compile_ok = false with a non-null diagnostic. This is the
  ordinary-run proof that shadow_compile_ok is not a constant true.

F6  frontier-missing-axis
    Drop axis 11 (latency_proxy_cycles) from the frontier emitter.
    Expected: H7 Refuted (axis null) AND, transitively, dispatch to
    Fail-frontier-incomplete.

F7  reset-boundary-leak
    Carry BoundedKv KV state across chunk boundaries (skip the
    chunk-boundary reset). Expected: H9 Refuted (occupancy != expected
    sequence; > K_cap eventually).

F8  per-seed-non-determinism
    Inject a host-clock read into the BoundedKv forward path
    Expected: H10 Refuted (replay disagreement) AND Rep-S5-8 fails.

F9  runtime-budget-tolerance-violation
    Emit a real RuntimeChromeBudget that over-reports usable_bytes by
    +400 vs synthetic (> 256-byte tol). Expected: H11 Refuted.

F10 compile-profile-not-threaded
    Strip the CompileRequest.profile field from the request before
    pipeline dispatch (silently fall back to Default). Expected: H12
    Refuted (in_budget preflight under "BringUp" does not record
    BringUp-specific WRAMFitReport).

F11 shadow-stages-missing
    Stub out OverlayPlan in the real shadow pipeline. Expected: H13
    Refuted (stages_executed != S5_SHADOW_PIPELINE_STAGES) AND
    Fail-shadow-compile.

F12 encoded-rom-cert-corrupted
    Emit an EncodedRom with an invalid reachability cert.
    Expected: O11 fails ⇒ Fail-encoded-rom.

F13 emulator-oracle-disagree
    Patch the BoundedKv export path to bias a non-oracle token above
    the P2 prompt's predicted token, or bias the oracle-predicted token
    downward below the runner-up. Expected: H15 Refuted (oracle
    disagrees with harness).

F14 pareto-broken
    Replace the Pareto emitter with one that always picks the first
    point. Expected: H14 Refuted and selection_authority must be
    "manual-override" for any allowed closure.

F15 feedback-broken
    Mutate router_state on empty ExpertSlotAffinity or change one
    safe_bound update constant. Expected: H16 Refuted.
```

Current in-repo support for bd-233u is the bounded policy substrate
for F13/F14/F15 plus Cargo feature-mutex/dry-run feature-matrix
coverage for `s5-falsify-{N}`. It does not claim live
gbf-experiments::s5 producer-side substitutions or real F1..F15
feature-loop execution. That live producer loop, fixtures, and
one-feature-at-a-time execution are owned by bd-q3zo.

---

# 17. Minimal end-to-end theorem

The minimal end-to-end theorem proves that the merged slice's
combined hypothesis set is necessary and sufficient for closure.

```text
Theorem (S5 closure).
  bd-36y1 AND bd-1cdu close ⇔
    (1) H1, H2, H3, H4, H6, H7, H9, H10 all Confirmed (Pick mandatory),
    (2) H11, H12, H13, H15, H16, H17 all Confirmed (Fit mandatory),
    (3) H5, H8 each have an explicit Confirmed | Refuted verdict
        (non-blocking),
    (4) H14 either Confirmed OR Refuted-with-manual-override,
    (5) All schemas in §13 emitted, self-hash-valid, canonical,
        binary blobs bound,
    (6) FrontierRecommendation ∈ {A, B, Tie} recorded honestly,
    (7) Decision ∈ {ProceedToS7, ProceedToS7-with-warning(_)},
    (8) Pre-registration ancestry holds (O1),
    (9) Falsification suite §16 passes all 13 F-cases (O18),
    (10) Reproducibility laws Rep-S5-1..Rep-S5-12 hold on the closure
         artifacts.

Proof sketch.
  (⇒) Suppose bd-36y1 + bd-1cdu close. Then §15 step 3 was satisfied
      (all CI gates green), §15 step 4 selected one of the two
      allowed Decision values, and §15 step 2 emitted all required
      artifacts. By RP-Decision-Single + dispatch in §12, the Decision
      value implies the predicate (1)-(7) above. CI gates §15 step 3
      enforce (8), (9), (10) directly.
  (⇐) Suppose (1)-(10) hold. Then §12 dispatch yields one of the
      allowed Decision values, every schema in §13 is present + valid,
      pre-registration ancestry holds (O1), the falsification suite
      passes, and the reproducibility smoke succeeds. §15's closure
      protocol then permits both bead closures.

Corollary 1 (frontier honesty).
  FrontierRecommendation = A ⇒ Pass-with-A-frontier; FrontierRecommendation
  = B ⇒ Pass-with-B-frontier (with H8 Refuted recorded honestly);
  FrontierRecommendation = Tie ⇒ Pass-with-tie. Each is a legal
  closure result.

Corollary 2 (the merge boundary).
  Act II always integration-tests BoundedKv as primary regardless
  of FrontierRecommendation. If both variants pass the frontier and
  BoundedKv fails the emulator harness, the slice fails on
  Fail-emulator-harness (not on the frontier). This is the
  merge-binding resolution of the original S5/S6 contradiction.
```

---

# 18. Crate layout + build configurations

## 18.1 Crate map

```text
gbf-experiments  (NEW + EXTEND)
  src/
    s1/                      (closed)
    s2/                      (closed)
    s3/                      (closed)
    s4/                      (closed)
    s5/                      ; NEW; absorbs the originally-planned s6/* tree
      mod.rs
      pick/
        attention_oracle.rs  ; reference impl + fixture loader
        bounded_kv.rs        ; BoundedKv contract operation
        linearstate.rs       ; LinearState DecayPolicy contract
        shadow_stub.rs       ; Pick-axis shadow_compile API surface call
      fit/
        chrome_budget.rs     ; synthetic + real emitters + agreement
        compile_profile.rs   ; BringUp registry entry
        preflight.rs         ; DeployabilityReport emitter
        revalidation.rs      ; D18 contract
        shadow_real.rs       ; Act II real shadow_compile pipeline
        encoded_rom.rs       ; EncodedRomBundle assembler
        emulator_harness.rs  ; one-token harness
        feedback_apply.rs    ; D13 consumer rule
        logging_overhead.rs  ; bench driver
      frontier.rs            ; Pareto emitter + recommendation
      report.rs              ; s5_report.v1 emitter
      replay.rs              ; CLI driver
  fixtures/                  (S5-owned)
    attention_oracle/s5_oracle.toml
    frontier/s5_frontier_fixture.toml
    feedback/s5_feedback_fixture.toml
    runtime/nucleus_pin.toml
    workloads/v0_success_s5.toml
    preflight/s5_over_budget.json
    benches/s5_log_bench.toml
    proxies/s5_latency.toml

gbf-policy
  src/
    s5/
      shadow_pipeline_stages.rs   ; pinned S5_SHADOW_PIPELINE_STAGES
      bringup_profile_defaults.rs ; pinned BringUp constants

gbf-model
  src/
    sequence/
      decay_policy.rs          ; DecayPolicy enum + MultiTimescale layout
      bounded_kv.rs            ; (existing) — record layout pin
      attention_oracle_ref.rs  ; deterministic reference impl

gbf-train
  src/
    burn_adapter/
      bounded_kv_smoke.rs      ; T12.3b
      multi_timescale_smoke.rs ; T12.5
    shadow/
      stub.rs                  ; Pick-axis API surface
      real_pipeline.rs         ; Act II full pipeline
    preflight.rs               ; DeployabilityReport emitter
    feedback_consumer.rs       ; D13 consumer

gbf-cli
  src/
    s5_cmd.rs                  ; `gbf s5` subcommand tree:
                               ;   gbf s5 replay {--variant V --seed S | --all}
                               ;   gbf s5 regress
                               ;   gbf s5 oracle {--fixture AOF-N}
                               ;   gbf s5 verify-determinism
                               ;   gbf s5 emit-frontier
                               ;   gbf s5 logging-overhead

gbf-artifact
  src/
    s5/
      checkpoint_frontier_point.rs  ; canonical encoding
```

## 18.2 Test layout

```text
gbf-experiments/tests/
  s5_pick_attention_oracle_agreement.rs
  s5_pick_boundedkv_gradient_smoke.rs
  s5_pick_l_mt4_stability.rs
  s5_pick_three_variant_smoke.rs
  s5_pick_shadow_stub_wiring.rs
  s5_pick_frontier_emission.rs
  s5_pick_reset_boundary.rs
  s5_pick_per_variant_determinism.rs
  s5_fit_chrome_budget_agreement.rs
  s5_fit_compile_profile_binding.rs
  s5_fit_shadow_real_correctness.rs
  s5_fit_pareto_frontier_fixture.rs
  s5_fit_encoded_rom_end_to_end.rs
  s5_fit_emulator_harness_seed0.rs
  s5_fit_per_export_revalidation.rs
  s5_fit_feedback_apply_fixture.rs
  s5_fit_logging_overhead_gate.rs
  s5_falsification_F1.rs ... s5_falsification_F15.rs

gbf-experiments/tests/fixtures/
  integration_s5/             ; tiny in-repo fixture for CI smoke
    tiny_gutenberg.txt
    tiny_charset_v1.toml
    tiny_seed_0_checkpoint.safetensors
    tiny_runtime_chrome_budget.json
```

## 18.3 Artifact paths

```text
experiments/S5/
  runs/
    {variant}/seed-{seed}/
      run_log.json
      score.json
      attention_oracle.json     ; BoundedKv only
      shadow_stub/step-{step}.json
      v0_success.json
  runs/seed-{seed}/
    compile_request.json
    deployability_report.json
    shadow/step-{cadence_step}.json
    feedback/step-{cadence_step}.json
    re_validation.json
    encoded_rom/
      bundle.json
      rom.gb
      rom.sym
      rom.lst
      certs/*.cert.json
      placed_rom_report.json
      build_identity_block.json
    reachability_report.json
  runs/seed-0/emulator_harness/p2.json
  budget/
    synthetic.runtime_chrome_budget.json
    real.runtime_chrome_budget.json
    agreement.json
  profile/bringup.compile_profile.json
  frontier/s5_frontier.json
  logging_overhead/report.json

docs/experiments/S5-report.md
```

## 18.4 Canonical replay command

```text
gbf s5 replay \
  --rfc-revision <git-sha-or-hash> \
  --variant {boundedkv | linearstate_fixed_0_5 | linearstate_mt4 | all} \
  --seed {0..4 | all} \
  --device-profile S1CpuDeterministic \
  --device-profile-sha <sha> \
  --pass-version =<semver> \
  --include-fit                                  ; default true; opts out for
                                                 ; Pick-only replays
  --include-emulator-harness                     ; default true for seed 0
```

## 18.5 Workspace registration

```text
[workspace]
members = [
  ...,
  "crates/gbf-experiments",
  "crates/gbf-policy",
  "crates/gbf-model",
  "crates/gbf-train",
  "crates/gbf-codegen",
  "crates/gbf-runtime",
  ...
]
```

## 18.6 Build configurations

```text
S5-build-A — "Standard S5 closure run"
  features:
    qat (default)
    s5-default
    burn-adapter
  toolchain: pinned (Cargo.lock)
  binary: gbf-experiments

S5-build-B — "Attention-oracle cross-build"
  features:
    qat
    s5-default
    burn-adapter
    s5-oracle-cross-build       ; alternate code path that builds the
                                ; oracle reference from scratch (no
                                ; cached crate artifacts)
  Purpose: prove Rep-S5-6 (cross-build oracle re-derivation).

S5-build-C — "Falsification harness"
  features:
    qat
    s5-default
    burn-adapter
    s5-falsify-{N}              ; mutually exclusive; N in {1..13}
  Compile-time mutex enforced via compile_error!.
  Purpose: drive the §16 F-cases.

S5-build-D — "Logging baseline"
  features:
    qat
    s5-no-log                   ; logging compiled out
    burn-adapter
  Purpose: drive D14 baseline measurement (H17).
```

## 18.7 Feature flag contract

```text
[features]
default                  = ["s5-default", "qat", "burn-adapter"]
s5-default               = []
s5-no-log                = []
s5-oracle-cross-build    = []
s5-falsify-1             = []
s5-falsify-2             = []
... (s5-falsify-3..13)

# mutually-exclusive guards:
# - qat XOR qat-ablation (inherited from S1)
# - s5-default XOR s5-no-log
# - at most one s5-falsify-N at a time
# Enforced via compile_error! macros in lib.rs.
```

## 18.8 Determinism budgets

```text
- log2_sum accumulation in f64 (bpc).
- Gradient reductions in the order pinned by the Burn adapter
  (deterministic axis order; no atomic-add reductions).
- BoundedKv FIFO eviction order is by valid-flag scan; the order is
  pinned by record layout.
- MultiTimescale band partitioning is by contiguous slot index range;
  the order is pinned by EqualBandsByOrder.
- Tile, residency, integer accumulator ranges in EncodedRom export
  are pinned by gbf-codegen pass_version.
- gbf-emu DeterminismPolicy::default() controls audio, RTC, PowerOnRamPolicy,
  save-state metadata, joypad input stream, internal RNG seed.
```

## 18.9 Pre-registration CI

```text
scripts/s5_predictions_ancestry.sh:
  1. Read s5_report.v1 predictions_commit.
  2. Read first_result_commit (earliest commit on the closure branch
     that adds a per-(variant, seed) result artifact or per-seed Fit
     artifact).
  3. Assert predictions_commit is an ancestor of first_result_commit
     in git log --topo-order.
  4. Exit non-zero on failure.
```

## 18.10 CI gates that block bd-36y1 + bd-1cdu closure

```text
- cargo fmt --check --all
- cargo clippy --workspace --features "s5-default,qat,burn-adapter" -- -D warnings
- cargo test --workspace --features "s5-default,qat,burn-adapter"
- scripts/s5_feature_matrix_check.sh
- scripts/check-nucleus-drift.sh
- scripts/s5_logging_overhead_check.sh
- scripts/s5_predictions_ancestry.sh
- scripts/s5_falsification_suite.sh
- scripts/s5_reproducibility_smoke.sh
- scripts/s5_pareto_fixture_check.sh
- scripts/s5_feedback_fixture_check.sh
- scripts/s5_attention_oracle_fixture_check.sh
- scripts/s5_encoded_rom_cert_check.sh
- scripts/s5_emulator_harness_seed0_check.sh
```

---

# 19. Ambiguity ledger

The combined ledger has 25 entries (down from S5's 28 + S6's 38 = 66
by dropping items resolved by the merge itself and folding duplicates).

|  ID | Ambiguity                                                                   | Chosen path                                                                   | Clarifying question                                                              | Suggested final decision                                                                                                                |
| --: | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
|  A1 | Pick vs Fit closure boundary                                                | Both close together as one slice (merge-binding)                              | Could Act I close first and Act II later?                                        | No. The merge resolution makes the two acts one closure. If Act II fails, the slice fails; Act I outputs remain on disk.               |
|  A2 | Primary integration variant (when frontier picks B or Tie)                  | BoundedKv always (D8)                                                         | Should the frontier winner drive Act II?                                         | No. The oracle reference exists only for BoundedKv; H15 needs it. LinearState may run informationally.                                  |
|  A3 | Attention-oracle tolerance numerics                                         | max_abs_diff per logit <= 1e-4 in f32 (D3)                                    | Should the tolerance be ulp-based?                                               | No. f32 max-abs-diff is a strict, easy-to-explain criterion. ulp tolerance hides hardware-dependent rounding.                          |
|  A4 | BringUp profile WRAM split (4096/256/256/2048/1536)                         | Pinned per D7                                                                 | Why this split?                                                                  | [ESTIMATE] from F11 bead refinement notes. Tighten once gbf-bench supplies measured kernel sizes. Marked A_S5_WRAM.                    |
|  A5 | max_bank_switches_per_token cap = 8                                         | 8 (D7)                                                                        | Why 8?                                                                           | [ESTIMATE] A_S5_SWITCH_CAP. Hand-picked for BringUp; calibration-derived once gbf-bench is online.                                     |
|  A6 | Bank0Free 6144 bytes (D9)                                                   | 6144 usable + 256 slack                                                       | Why not 8192?                                                                    | [ESTIMATE] A_S5_BUDGET. Synthetic future-reservations are conservative.                                                                |
|  A7 | Synthetic-vs-real budget tolerance (256 bytes per slot)                     | D9-tol                                                                        | Why 256?                                                                         | One bank tile; small enough to catch material drift, large enough for honest shell variation.                                          |
|  A8 | Shadow-vs-final byte-cost tolerance (1024 strict / 2048 warning)            | Two-band gate (H13)                                                           | Why two thresholds?                                                              | StaticBudgetReport is approximate. A_S5_SHADOW_GAP.                                                                                    |
|  A9 | DecayPolicy::Learned scope                                                  | Out of S5 (D2)                                                                | Why defer?                                                                       | Learned adds a learnable scalar that confounds the L_MT4 vs L_FIX1 comparison. Follow-up bead.                                          |
| A10 | MultiTimescale band layout (EqualBandsByOrder)                              | [0.5, 0.75, 0.875, 0.9375] x equal bands                                      | Why not random-init bands or interleaved?                                        | Equal bands are the simplest interpretable partition. Other layouts add a second hyperparameter S5 cannot honestly compare.            |
| A11 | Attention-oracle fixture suite size (5 fixtures)                            | AOF-1..AOF-5 (D3)                                                             | Why not 3 or 20?                                                                 | Five covers {single, sub-K, at-K, 1.5x K, 4x K}. Twenty is redundant; three misses a regime.                                            |
| A12 | BoundedKv K cap = 128                                                       | K = 128 (D2)                                                                  | Why match chunk_size?                                                            | Matching chunk_size keeps the S1 reset-context bpc primitive applicable unchanged.                                                      |
| A13 | Train budget at 20000 vs S1's 10000                                         | 20000 per variant                                                             | Is this enough for BoundedKv?                                                    | Empirically risky; doubling beyond 20000 makes the 15-run cross product expensive. Bump only if H4 fails.                              |
| A14 | Latency proxy formula constants (BANK_SWITCH_PROXY_CYCLES = 24, OVERLAY_INSTALL_PROXY_CYCLES = 256) | Pinned per §6.4                                       | Why these constants?                                                             | [ESTIMATE]. F4-broken substitute rejects builds that omit the bank-switch contribution.                                                |
| A15 | Pareto rubric: lex order vs full Pareto                                     | Full Pareto dominance + lex tie-break                                         | Lex order is simpler                                                             | Pareto is honest about multi-objective trade-offs. Lex tie-break keeps it deterministic.                                                |
| A16 | Logging-overhead threshold = 1%                                             | 0.01 per CONSTITUTION §II.1                                                   | Why exactly 1%?                                                                  | [ESTIMATE] A_S5_LOG; constitution names this number.                                                                                    |
| A17 | Emulator tick budget = 240 frames                                           | 240 (D12)                                                                     | Why 240?                                                                         | [ESTIMATE] A_S5_TICK = 4 s at 60fps. Generous for cooperative scheduler + cold start; tight enough that a hung runtime is detectable. |
| A18 | Emulator harness prompt = 1-token P2                                        | Single charset_v1 token, zero-context (D11/D12)                               | Why a 1-token prompt?                                                            | Simplest possible falsifier; longer prompts add multi-token state out of scope for the merged closure.                                  |
| A19 | Compiler feedback safe_bound constants (grow 0.10, shrink 0.95)             | D13                                                                           | Why these?                                                                       | [ESTIMATE] A_S5_FEEDBACK. Converges without oscillation; wider sweep is a follow-up.                                                    |
| A20 | Compiler feedback timing (phase boundaries only)                            | Steps 6000, 6001, 12000, 20000 (§11.2)                                        | Why not every cadence step?                                                      | Mid-phase apply confounds activation range and QAT hardness changes. Phase-boundary isolates.                                          |
| A21 | runtime_nucleus_hash sentinel format ("SYNTHETIC_REFERENCE:")               | Prefix on sha256 (D10)                                                        | Why a prefixed string?                                                           | Single-field discriminator keeps the schema flat. CI scripts can grep the prefix without parsing JSON.                                  |
| A22 | gbf-runtime symbol resolution for harness                                   | Read from .sym map; fail with diagnostic if missing                           | Hard-coded constants?                                                            | No. Hard-coded addresses would silently drift; .sym lookup is honest and runtime_nucleus_hash binds.                                    |
| A23 | gbf-emu DeterminismPolicy customization                                     | DeterminismPolicy::default() per F-A7 §0.1 (D17)                              | Could S5 customize?                                                              | No. Customization invalidates F-A7's "every consumer shares one substrate" rationale.                                                  |
| A24 | Tiny in-repo fixture for integration_s5 vs full Gutenberg run               | Tiny fixture for CI smoke; full Gutenberg for closure                         | Should CI run all 15 (variant, seed) on full Gutenberg?                          | No. Full Gutenberg is the closure deliverable; CI gates on tiny-fixture smoke. Closure requires both.                                  |
| A25 | LinearState as informational second Act II build                            | Allowed; closure stays BoundedKv-only (D8)                                    | Should LinearState build be required if frontier picked Tie?                     | No. Absence of attention-oracle reference for LinearState makes its emulator agreement non-falsifiable.                                |

[ESTIMATE] markers worth a human review:
  A4 (BringUp WRAM split), A5 (switch cap), A6 (Bank0Free byte count),
  A8 (shadow-vs-final tolerance bands), A14 (latency proxy constants),
  A16 (logging-overhead threshold), A17 (tick budget), A19 (feedback
  rule constants).

---

# 20. Final concise contract

```text
F-S5 Pick and Fit is correct when:

1.  All 15 (variant, seed) Pick runs in the cross product
    {BoundedKv, L_FIX1, L_MT4} x {0..4} complete Phase A->D on
    Gutenberg charset_v1 without divergence and produce bit-identical
    safetensors per (variant, seed) under replay. Every variant's
    val_bpc_ternary is finite and beats the S4-pinned KN-5 baseline
    by > 0.05 bpc per seed; every variant passes the S3-pinned
    v0_success workload per seed.

2.  BoundedKv's Phase A checkpoint agrees with the deterministic
    attention-oracle reference within max_abs_diff <= 1e-4 in f32 on
    every position of every fixture in AttentionOracleFixtureSuite
    (AOF-1..AOF-5), for every seed. Burn autodiff through BoundedKv
    produces finite, nonzero, deterministic gradients into intended
    trainable parameters and zero gradients into stop-gradient sets.
    LinearState DecayPolicy::MultiTimescale (decays = [0.5, 0.75,
    0.875, 0.9375], EqualBandsByOrder over four bands) trains
    end-to-end without divergence and without any band collapsing
    to non-finite.

3.  s5_frontier.v1 is emitted with three variant_records, every D5
    axis populated for every (variant, seed), and is byte-identical
    under replay. FrontierRecommendation in {A, B, Tie} is computed
    deterministically from per-variant aggregates per §8.5. The
    reset-context chunk semantics (chunk_size = 128) is preserved by
    every variant; BoundedKv's KV slab resets at every chunk boundary
    and never exceeds K_cap = 128.

4.  The five BoundedKv per-seed checkpoints preflight under
    CompileProfile::BringUp against a real RuntimeChromeBudget
    emitted by the gbf-runtime shell build, with hard-failure
    diagnostics that include slot id, slot class, over-by bytes, and
    suggested fix. The synthetic and real RuntimeChromeBudget instances
    agree on schema shape and per-slot byte counts within 256 bytes;
    the runtime_nucleus_hash CI drift gate fires on a deliberately
    altered nucleus build; the trainer self-check fires on mid-run
    hash drift.

5.  The BringUp CompileProfile (overlay 4096, hot_arena 2048,
    PerExpertSwitch reload, max_bank_switches_per_token = 8,
    StrictOnePerBank) flows through CompileRequest into preflight and
    every downstream pipeline stage; over-budget artifacts are
    rejected; in-budget artifacts pass.

6.  Real shadow compile runs the full F-B* pipeline
    (S5_SHADOW_PIPELINE_STAGES) at every cadence step (4000, 8000,
    12000, 16000, 20000 plus Phase E final), producing 30
    ShadowCompileSampleReal records (5 seeds * 6 cadence emissions)
    with non-constant shadow_compile_ok bits, real
    compiler_feedback.json, and shadow_byte_cost matching final
    encoded_rom_byte_cost within 1024 bytes (warning band 1024..2048).

7.  The seed-0 EncodedRom built by the full F-B* pipeline has a
    non-zero byte length within the 524288-byte budget, valid
    range/arena/window/reachability/resource_state certs, an
    isr_reachable_outside_bank0 list of [], every ReferenceShellSpec
    module with bind_count > 0 in the .sym map, and a
    BuildIdentityBlock whose runtime_nucleus_hash equals the chrome
    budget that was preflighted against.

8.  The seed-0 emulator one-token harness, under
    DeterminismPolicy::default() and BootMode::PostBootDmg with a
    240-frame budget, emits exactly one charset_v1 token to the
    runtime's video commit queue, and that token equals the
    ArtifactOracle's predicted first token for prompt P2 byte-for-byte;
    harness replays produce bit-identical (token_emitted,
    ticks_consumed, harness_self_hash). Act II runs BoundedKv as
    primary regardless of FrontierRecommendation per D8.

9.  Phase E HardenAndSelect re-runs the preflight against the current
    RuntimeChromeBudget; healthy exports yield Pass, hash-drift +
    within-tolerance exports yield Warn, bloated exports yield
    BlockExport with diagnostics citing both nucleus hashes and
    per-slot deltas; synthetic-vs-real mismatch is BlockExport per
    D10. apply_feedback applies the safe_bound update rule per D13
    on the pinned synthetic feedback fixture, producing finite,
    deterministic, non-zero scalar updates and a byte-identical no-op
    on empty ExpertSlotAffinity hints.

10. The structured-logging overhead bench measures
    (median_instrumented - median_baseline) / median_baseline < 0.01
    on the pinned tiny preflight + shadow workload; the gate fires
    on an artificially inflated build.

11. s5_report.v1 emits pre-registered predictions in git history
    strictly before the first per-(variant, seed) or per-seed result
    artifact commit, and concludes with exactly one Decision value
    chosen by §12 dispatch; the Decision is one of
    {ProceedToS7, ProceedToS7-with-warning(_)}. F2 (bd-1hv), F8
    (bd-2am), F11 (bd-1i8), and F12 (bd-144) close alongside bd-36y1
    and bd-1cdu. Every JSON artifact (s5_run_log, s5_score,
    s5_attention_oracle, s5_shadow_compile_sample_stub,
    s5_shadow_compile_sample, s5_v0_success_per_variant, s5_frontier,
    s5_runtime_chrome_budget, s5_budget_agreement, s5_compile_profile,
    s5_compile_request, s5_deployability_report, s5_re_validation,
    s5_encoded_rom, s5_reachability, s5_emulator_harness,
    s5_feedback_apply, s5_logging_overhead, s5_report) is canonical,
    deterministic, and self-hash-valid. Binary blobs (.gb, .sym,
    .lst, certs/*, safetensors) are bound by recorded Hash256 fields.

12. All seventeen hypotheses have explicit verdicts in the
    falsification analysis section, with concrete observations cited.
    The fifteen F1..F15 deliberately-broken substitutes each produce
    the expected Refuted verdict once the live producer loop owned by
    bd-q3zo is wired; bd-233u provides only the policy substrate and
    feature-mutex scaffolding for this claim. S5 retires sequence-state-
    architecture comparison risk AND the integration risk between
    training (S1..S4 + Pick) and the compiler+runtime stack
    (F-A*/F-B*). It does NOT claim MoE benefit (S7),
    UpperBankCandidate production-scale quality on Gutenberg (S8),
    steady-state generation correctness past the first token,
    measured (vs analytic) cycle counts, multi-mode SchedulePack
    switching, or cartridge hardware behavior. Those are later
    slices' or post-closure proof obligations.
```
