> FINAL - F-S7 MoE Beats Dense at Matched Bytes
> Status: result recorded. The production telemetry exists for bd-2v9r and
> resolves the pre-registered review estimates. The observed S7 result is
> Fail-parity / ProceedToS8DenseOnly.

# Formal spec pack: F-S7 MoE Beats Dense at Matched Bytes

This is the seventh scientific/experimental RFC in the training-contract
epic. Like F-S1, its deliverable is **verified knowledge**, not just code.
The hypothesis is concrete: at MoeTiny size, on Project Gutenberg, under the
S5 (Pick and Fit)-validated training scaffold, sparse top-1 MoE beats a matched-deployed-
bytes dense baseline on bits-per-character — for every one of five fixed
seeds — and the F8 Pareto frontier picks MoE over dense.

S7 is where MoE goes from "ambition" to "load-bearing." A `Fail-parity`
result is a successful scientific falsification, not an implementation
failure: it means the hypothesis "MoE wins at matched bytes" was rejected,
and S8 must inherit a dense-only production track.

```text
Spec:
  F-S7 MoE Beats Dense at Matched Bytes
  Slice S7 of the training-contract epic (bd-1rb)
  Closure bead: bd-2v9r

Hypothesis-under-test:
  At MoeTiny (d_model=64, d_ff=128, 4 blocks, n_experts=4) trained on
  Project Gutenberg under the inherited S2->S5 (Pick and Fit) phase scheduler, the trained
  MoE artifact beats a matched-deployed-bytes dense baseline (MoeTinyDense-
  Matched) on validation bpc by strictly more than 0.05 bpc, for every one
  of five fixed seeds, and the F8 Pareto frontier picks the MoE
  CheckpointFrontierPoint over the dense one on the (val_bpc, deployed
  bytes) plane.

Owns:
  hypothesis statements H1..H10
  pre-registered prediction tables for the MoE/dense pair
  MoeTiny ModelSizeProfile reference instance (n_experts=4 default; n_experts=2 alternate)
  MoeTinyDenseMatched dense profile (matched-bytes scaling formula pinned)
  Top1RouterQat (top-1, hard dispatch with stop-gradient routing)
  LowRankRouter (rank pinned)
  ExpertBlockQat two-matrix expert (clipped activation; no GLU)
  Burn adapter for ExpertBlockQat (mirrors S2 LinearState gradient smoke)
  Temporal smoothness regularization (L_switch_router; window pinned)
  Differentiable temporal switch loss L_switch (T5.1; gradient provenance pinned)
  Expert dropout + Gaussian router-logit jitter
  Loss composition: lambda_distill, lambda_balance, lambda_zrouter, lambda_switch
  Router collapse guardrail: lambda_switch sweep grid + entropy floor
  Matched-deployed-bytes parity gate
  Dense-vs-MoE comparison report (s7_dense_vs_moe.v1)
  TemporalSwitchDigest, ClipSaturationDigest, ExpertPayloadDigest
    schema completion (LayerId-scoped) and producer collection contract
  Standard router observability metrics (per-step) and lambda_switch sweep
  RouterRng disjoint stream + per-step seeding
  s7_*.v1 artifact schemas (run_log, score, frontier, switch_stats,
    router_collapse_sweep, dense_vs_moe, report)
  S7 reproducibility law extensions

Does not own:
  UpperBankCandidate (d_model in {96, 128}) production-scale runs
    on Gutenberg (S8)
  ExpertShapePolicy::StructuredWidthGates supernet (S8)
  M6 adaptive shapes / lambda_shape / lambda_overflow (T5.5; S8)
  Non-Ternary2 weight encodings, non-Q8_8 scales (F15 post-closure; S8)
  Per-weight learned thresholds (F15 post-closure; S8)
  Top-k routing for k >= 2 (experimental compile profile; out of scope)
  GatedLinearUnit (three-matrix) experts (deferred; bd-2c8z explicitly rejects)
  Real ROM emulation of the MoE artifact at scale (carry-through from S5
    (Pick and Fit); one-token harness only, per the S5 (Pick and Fit) contract)
  v0 success workload (closed at S3; carry-through, not amended)
  Charset_v1 / KN-5 baseline (closed at S3; carry-through, not amended)
  Project Gutenberg promotion gate (closed at S4; carry-through)
  RuntimeChromeBudget end-to-end (closed at S5 (Pick and Fit); carry-through)

Inherits unchanged from S1..S5 (Pick and Fit):
  Fixed seed list [0, 1, 2, 3, 4]                                          (S1 D2)
  Deterministic batch sampling and disjoint Rng streams                    (S1 D3a)
  Strict reproducibility law (Rep-1, Rep-7)                                (S1 D8, S5 D16)
  Fail-closed on NaN / divergence                                          (S1 D9)
  AdamW pinning (lr/beta1/beta2/eps/weight_decay)                           (S1 D10)
  S1CpuDeterministic device profile                                         (S1 D8)
  Canonical JSON discipline + DomainHash                                    (S1)
  Pre-registration CI rule                                                  (S1 O1)
  Charset_v1 lexical contract (vocab=80)                                    (S3)
  KN-5 character n-gram baseline scoring math                               (S3)
  Project Gutenberg manifest + contamination report                         (S4)
  ReferenceModelBundle export contract                                      (S3)
  ArtifactOracle three-way agreement law (train ~ bundle ~ artifact)        (S3)
  v0_success WorkloadManifest                                               (S3)
  ConformanceEnvelope schema                                                (S3 / S5 "Pick and Fit")
  Phase A->D scheduler                                                      (S2)
  ternary QAT survival contract (gap <= 0.5 bpc)                            (S2)
  LinearState gradient smoke                                                (S2)

From S5 (Pick and Fit) — single combined inheritance block (post 2026-05-19 merge):
  BoundedKv attention-oracle agreement                                      (S5)
  shadow_compile A/B wiring (training-scaffold-level)                       (S5)
  CheckpointFrontierPoint side-by-side schema                               (S5)
  RuntimeChromeBudget + CompileProfile + EncodedRom + emulator one-token    (S5)
  compiler_feedback feedback loop                                           (S5)
```

S7 amends the model topology: switches from dense Toy0/Toy1 to MoeTiny
(plus a matched-byte dense control), introduces the router and routing
losses, and validates that MoE at matched ROM beats dense. The training
scaffold is unchanged. The corpus is Gutenberg, inherited unchanged from
S4. The bpc primitive, oracle agreement, frontier emission, and emulator
one-token harness are inherited unchanged from earlier slices.

---

## Decisions

```text
D1 Topology: MoeTiny default; dense matched control mandatory
   The S7 experimental subject is MoeTiny:
     d_model    = 64
     d_ff       = 128
     n_blocks   = 4
     n_experts  = 4              ; production default (D2)
     vocab      = 80             ; charset_v1, inherited from S3
     ffn_path   = MoE on every block
     router     = Top1RouterQat with LowRankRouter projection (D7)
     expert     = ExpertBlockQat two-matrix (clipped activation; NOT GLU)
     embedding  = tied input/output (charset_v1 default)
     sequence   = LinearState multi-timescale (S5 default)
     norm       = AffineClipLut (S2 default)

   The S7 control subject is MoeTinyDenseMatched:
     ffn_path   = Dense everywhere (FfnPathConfig::Dense on every block)
     d_model    = 64                              ; equal to MoeTiny
     d_ff       = solve d_ff_dense per D6         ; matched deployed bytes
     other dims = identical to MoeTiny
     uses       = identical training scaffold (D8)
     run        = identical to MoeTiny in every other respect

   Both subjects MUST run all five seeds. Both MUST complete Phase A->E
   under the S2 scheduler. Both MUST emit a CheckpointFrontierPoint.

D2 Number of experts: 4 (default), 2 (mandatory ablation lane)
   n_experts = 4 is the production decision. The ModelSizeProfile registry
   admits both n_experts ∈ {2, 4} per planv0 amendment item 1. S7 trains
   the n_experts = 4 lane through the full closure pipeline (parity gate,
   frontier, switch stats, oracle, emulator). The n_experts = 2 lane runs
   the same training/scoring pipeline but is reported as observational only.
   Its artifacts are written under:

     experiments/S7/ablations/n_experts-2/

   The n_experts = 2 lane:
     - does not contribute to H1..H10 verdicts,
     - does not contribute to S7Outcome,
     - does not affect bd-2v9r closure,
     - records its parity and switch-stats observations in s7_report.v1
       under "Ablations".
   Rationale: 4 experts gives enough specialization headroom for the parity
   gate while still fitting strictly inside one ExpertBank. Two experts is
   the literature minimum and is reported as a sanity floor.

D3 Routing: top-1, hard dispatch with stop-gradient
   Token routing is hard top-1. The expert dispatch tensor is the
   one-hot indicator of argmax(routing_probs). The dispatch tensor is
   stop-gradient: its gradient is the zero tensor by construction.
   Differentiable signal into the router flows through:
     - the soft routing distribution p_{l,t} (used by L_switch, balance loss)
     - the raw router logits z_{l,t} (pre-jitter; used by z-loss)
   No straight-through estimator on the dispatch indicator. No Gumbel
   sampling. Top-2 routing is explicitly out of scope and forbidden by
   §6.2 falsification F1.

D4 Phase scheduler: full Phase A->E
   The S2 phase scheduler runs through Phase A (DenseTeacherWarmup),
   Phase B (RouterWarmup), Phase C (ExpertTernaryQat), Phase D
   (FullNumericQat), and Phase E (HardenAndSelect). Phase boundaries
   in optimizer steps:
     Phase A end:   step 4000          ; quality + stable specialization
     Phase B end:   step 8000          ; router commits before quant noise
     Phase C end:   step 14000         ; expert ternary annealed in
     Phase D end:   step 18000         ; activation fake quant on
     Phase E end:   step 20000         ; HardenAndSelect; shadow compile dense
   Total optimizer_steps = 20000. (Inherits S5 budget; see D9.)

   For the MoeTinyDenseMatched run, Phase B is a no-op (no router exists)
   and lambda_balance / lambda_zrouter / lambda_switch are gated to 0 in
   the dense-effective loss config. Phase C ternarizes the dense FFN's two
   linears via the same QatHardnessControl trait used by ExpertBlockQat.

D5 Loss composition: F5 partial (T5.1; T5.5 deferred)
   Composed loss (training):

     L_total =
         lm_loss
       + lambda_distill  * logit_distillation_loss(student, frozen_teacher)
       + lambda_balance  * expert_load_balance_loss(p, dispatch_sg)
       + lambda_zrouter  * router_z_loss(z)
       + lambda_switch   * temporal_switch_penalty(p, sequence_mask)

   Defaults (production):
     lambda_distill   = 1.0       ; on from Phase C
     lambda_balance   = 0.01
     lambda_zrouter   = 1e-3      ; centered z-loss; baseline = 0
     lambda_switch    = 0.05      ; production value (D11); below collapse threshold

   Mandatory non-default (anti-1.0) test values:
     lambda_balance_alt   = 0.1
     lambda_zrouter_alt   = 1e-2
     lambda_switch_alt    = 0.5    ; also a sweep grid point (D11)

   GATED OFF in S7 (T5.5 / S8):
     lambda_shape    = 0.0  ; meaningless for fixed-shape Ternary2
     lambda_overflow = 0.0  ; meaningless for fixed-shape Ternary2
     lambda_range    = 0.0  ; out of S7 scope; S2 closed activation phase
     lambda_zero     = 0.0  ; out of S7 scope; S2 closed ternary survival

   Phase-effective gating (per CLAUDE.md raw-vs-phase config bullet):
     Phase A: only lm_loss is on; all lambdas multiplied by 0.
     Phase B: lambda_balance, lambda_zrouter, lambda_switch on.
     Phase C: lambda_distill turns on; lambda_balance, lambda_zrouter,
              lambda_switch remain on.
     Phase D: identical to Phase C except activation fake quant.
     Phase E: identical to Phase D except no parameter updates from
              shadow-compile passes.

   Per CLAUDE.md "raw vs weighted" bullet: every per-term raw diagnostic
   loss is logged unconditionally and validated for finiteness BEFORE
   multiplication by the (possibly zero) lambda. The raw helpers must
   return a finite value even when the configured lambda is 0; if a helper
   intentionally skips computation (e.g. to avoid graph cost in Phase A),
   it MUST be named a *contribution* helper rather than a raw helper, and
   the omission MUST be explicit.

D6 Matched-deployed-bytes formula
   Linear shape convention:
     Linear[out_rows, in_cols]

   compute_weight_byte_cost(linear[out_rows, in_cols]) =
       ceil(out_rows * in_cols / 4)         ; ternary packing (2 bits/wt)
     + out_rows * 2                          ; per-output-row Q8_8 scale (2 bytes)
     + ternary_metadata_bytes                ; format-fixed; pinned in F-A4

   Bias deployment policy:
     S7 experts and dense FFNs use biases during training.
     Bias bytes MUST be accounted for by one of the following explicit
     policies, pinned in matched_bytes.json:
       - bias_policy = "not_deployed"
       - bias_policy = "folded"
       - bias_policy = "q8_8_per_output"
       - bias_policy = "fp16_per_output"
    The canonical S7 policy is: bias_policy = "q8_8_per_output".

   compute_linear_deployed_byte_cost(linear) =
     compute_weight_byte_cost(linear) + bias_byte_cost(linear, bias_policy)

   Let:
     B_experts_total = sum over all MoE blocks of:
                   sum over experts of:
                     compute_linear_deployed_byte_cost(up   [out=d_ff,   in=d_model])
                   + compute_linear_deployed_byte_cost(down [out=d_model,in=d_ff])

     B_router_overhead = total deployed bytes consumed by the LowRankRouter
       projections in MoeTiny that have no analogue in dense.

   S7 uses TWO byte measures:
     B_ffn_payload_total
       Raw FFN/expert payload bytes only. Used for diagnostics.

     B_deployed_total
       The byte count used by H3 and H4. Includes FFN/expert payload,
       router overhead, quantization metadata, bias payloads, and any
       deployment-bank allocation required by the artifact format.

   Matched-deployed-bytes means:
     B_deployed_total(MoeTiny) ≈ B_deployed_total(MoeTinyDenseMatched)
   within D6 tolerance.

   Solve d_ff_dense such that:
     B_deployed_total(MoeTinyDenseMatched)
       matches B_deployed_total(MoeTiny), where the MoE side includes
       B_router_overhead.

   The dense FFN contribution is computed by exact calls to
   compute_linear_deployed_byte_cost for each dense linear (which itself
   wraps TernaryWeightPlan::compute_byte_cost plus the pinned bias policy),
   not by a fractional byte-per-weight approximation:

     B_dense_ffn_block =
       compute_linear_deployed_byte_cost(DenseFFN.up   [out=d_ff_dense, in=d_model])
     + compute_linear_deployed_byte_cost(DenseFFN.down [out=d_model,    in=d_ff_dense])

     B_dense_ffn_total = n_blocks * B_dense_ffn_block

     B_deployed_total(Dense) =
       B_common_total + B_dense_ffn_total

     B_deployed_total(MoE) =
       B_common_total + B_experts_total + B_router_overhead

   Tolerance:
     |B_deployed_total(MoE) - B_deployed_total(Dense)|
       <= max(0.10 * B_deployed_total(MoE), 4 * one_bank_bytes)
   per planv0 amendment item 2 (parity gate per bd-2zv4: ±10% of the
   deployed MoE reference bytes, with a small absolute slack to handle
   integer rounding across four blocks).

   For the canonical S7 instance with MoeTiny (d_model=64, d_ff=128,
   n_blocks=4, n_experts=4):
     d_ff_dense is intentionally NOT specified in prose.
     It is resolved only by experiments/S7/profile/matched_bytes.json and
     verified by O11's standalone matched-bytes CI test.

   Non-normative note:
     Any prose estimate of d_ff_dense is forbidden because changes to
     ternary metadata, bias policy, or bank-allocation semantics can change
     the exact integer solution.

   Both d_ff_dense and the resulting B_dense_ffn_total are pinned in
   experiments/S7/profile/matched_bytes.json and hashed into every
   s7_*.v1 artifact.

D7 LowRankRouter projection
   Router parameters factor as:
     proj_down: Linear[d_model -> rank]      ; high precision
     proj_up:   Linear[rank   -> n_experts]  ; high precision
   For MoeTiny (d_model=64, n_experts=4):
     rank = max(1, min(ceil(n_experts / 4), 8)) = 1  ; non-S7 default formula
   This collapses to a degenerate rank=1 router. Rank=1 is mathematically
   well-defined (a single direction in d_model space scored against n_experts
   scales), but it is a special case: it caps the router's expressivity.
   Therefore:
     S7 OVERRIDES the default formula:
       n_experts = 4 production lane: router_rank = 4
       n_experts = 2 ablation lane:  router_rank = 2
     Rationale: rank = n_experts makes the factorized linear map capable
     of representing any full d_model -> n_experts router matrix while
     preserving the LowRankRouter abstraction used by later slices.
     For MoeTiny, this is not a parameter-saving choice:
       full dense router params = 64 * 4 = 256
       factorized params        = 64 * 4 + 4 * 4 = 272
     The load-bearing claim is the abstraction/regularization behavior,
     not parameter-count savings.
   The default formula's rank = max(1, min(ceil(n_experts / 4), 8)) is preserved for
   non-S7 use of the LowRankRouter.

D8 Training scaffold parity (D5 of bd-do2j re-stated)
   The MoE and dense runs share, byte-identically, the following
   substrate:
     - phase scheduler                                      (S2 / F4)
     - teacher freeze logic                                 (S2 / F4)
     - shadow_compile pipeline                              (S5 "Pick and Fit" / F8)
     - frontier emission                                    (S5 / F8)
     - optimizer + AdamW config                             (S1 D10)
     - device profile                                       (S1 D8 -> S7CpuDeterministic, identical pinning)
     - rng kind                                             (S1 D3a -> Pcg64Mcg)
     - corpus + manifest                                    (S4 Gutenberg)
     - charset_v1                                           (S3)
     - bpc primitive (vocab=80)                             (S3)
     - canonical JSON + DomainHash                          (S1)

   The ONLY differences are:
     - ModelTopologyConfig.ffn_path (MoE vs Dense)
     - n_blocks-many d_ff values (128 in MoE, d_ff_dense in dense)
     - presence of LowRankRouter
     - phase-effective lambda_balance, lambda_zrouter, lambda_switch
       which are 0 in the dense run (Phase B no-op)

   Any other divergence is a contract violation that invalidates the
   parity comparison.

D9 Train budget
     optimizer_steps   = 20000             ; inherits S5 D7
     batch_size        = 32                ; inherits S1 D3
     sequence_length   = 256               ; bumped from S1's 128 to give
                                           ; the temporal smoothness window
                                           ; (32) room to act on contiguous
                                           ; trajectories.
     eval_every_steps  = 1000
     eval_subset_size  = 4096

   Both MoE and dense MUST use these exact values. Changing any
   invalidates the parity comparison.

D10 Temporal smoothness window
   smoothness_window = 32 tokens          ; per bd-122 default
   Window size of 1 reduces L_switch to an adjacent-token penalty. It is
   mathematically valid but too weak for S7's temporal-smoothness claim.
   S7 forbids smoothness_window = 1 as a scope decision, not because the
   loss is identically zero (see falsification F7).
   Pair-set behavior:
     - the backend-independent model helper enumerates every valid
       (t, u) pair in the smoothness_window.
     - invalid sequence_mask positions and explicit sequence boundaries
       reset the window.
     - first token of each sequence contributes 0 to L_switch.
     - executable Burn/ring-buffer realization and gradient assertions
       are owned by O13 / bd-1kkf.

D11 lambda_switch sweep + router collapse guardrail
   Sweep grid (mandatory; per bd-3sp0):
     lambda_switch ∈ {0.0, 0.05, 0.1, 0.5, 1.0, 5.0}
   Production value:
     lambda_switch_production = 0.05            ; D5
   Collapse threshold (production must be strictly below):
     lambda_switch_collapse_threshold = 1.0     ; pinned by §10
   Guardrail assertions:
     A. bpc(MoE @ lambda_switch_production, val_gutenberg)
          <= bpc(MoE @ lambda_switch=0.0, val_gutenberg) + 0.05
       Resolved by production sweep: production lambda_switch did not regress
       quality by more than 0.05 bpc relative to the unregularized router.
     B. expert_usage_entropy_bits(MoE @ lambda_switch_production)
          >= 0.85 * log2(n_experts)
        For n_experts=4, log2(4) = 2.0; floor = 1.7 bits.
       Resolved by production sweep: seed-0 production-lambda mean entropy was
       1.7679 bits, above the 1.7-bit floor.
     C. expert_usage_entropy_bits(MoE @ lambda_switch=5.0)
          < expert_usage_entropy_bits(MoE @ lambda_switch_production) - 0.3
        i.e. the high-lambda variant must demonstrably collapse.
     D. bpc(MoE @ lambda_switch=5.0)
          > bpc(MoE @ lambda_switch_production) + 0.3
        i.e. the high-lambda variant must demonstrably regress quality.
   Failure of any of A, B, C, or D means the lambda_switch sweep itself is
   broken (it failed to demonstrate either non-collapse at production or
   collapse at the high end), which is a Refute on H6.
   The sweep runs at seed 0 only; n_experts = 4. The sweep-local
   production-lambda bpc is used only for H6 guardrail comparisons.
   H3 uses the final production training run's validation bpc, not the
   sweep-local retrain.

D12 Matched-bytes parity gate margin
   ∀ seed s. bpc(MoE_seed=s, val_gutenberg)
              < bpc(dense_matched_seed=s, val_gutenberg) - 0.05
   The 0.05 bpc margin mirrors S1 D6 / H2. Per-seed strict.
   No median rule, no aggregate threshold, no relaxation by variance.
   In addition:
     |B_deployed_total(MoE) - B_deployed_total(Dense)|
       <= max(0.10 * B_deployed_total(MoE), 4 * one_bank_bytes)
   per D6 tolerance. A bytes-mismatch above tolerance is NotEvaluable
   for H3; the run records S7Outcome = Fail-bytes (invalid-experiment),
   not Fail-parity (scientific falsification).

D13 Pareto dominance contract
   The F8 Pareto frontier (closed at S5 (Pick and Fit) via bd-1cdu) is re-evaluated on
   the (MoE, dense_matched) pair using the (val_bpc, deployed_bytes_total)
   axes. MoE wins the frontier iff:
     val_bpc(MoE)         <= val_bpc(dense_matched)
     deployed_bytes(MoE)  <= deployed_bytes(dense_matched)
     and at least one inequality is strict.
   Otherwise the points are Pareto-incomparable (in which case S7 reports
   `Fail-pareto`) or dense dominates (in which case S7 reports
   `Fail-parity` because H3 must have failed too).
   The frontier comparison uses the median-over-seeds val_bpc per
   variant; per-seed Pareto verdicts are recorded as observations.

D14 RouterRng disjoint stream
   RouterRng(seed) := Pcg64Mcg(seed128("router", seed))
   Disjoint from InitRng, BatchRng, ShuffleRng (S1 D3a) and from any
   future S2..S5 (Pick and Fit) stream. Consumed by:
     - LowRankRouter parameter initialization (under InitRng deterministic
       seeding, with RouterRng providing the per-step Gaussian jitter
       only).
     - Expert dropout mask sampling at each forward pass (per CLAUDE.md
       routing/expert dropout determinism: dropout RNG derived from step
       number for reproducibility).
     - Gaussian jitter on router logits during training (D5 and bd-1oc).
   At step k, expert dropout consumes RouterRng with sub-seed
     dropout_sub_seed(seed, step) =
       Pcg64Mcg(seed128("router-dropout", seed) XOR (step as u128))
   so dropout masks are reconstructable from (seed, step) without
   replaying earlier draws.
   Eval/export modes: NO RouterRng draws. Router is deterministic at
   eval/export per bd-1oc.

D15 Strict reproducibility (extends S1 D8)
   Same seed + same corpus_train_sha + same corpus_val_sha (Gutenberg) +
   same manifest charset_v1_sha + same train_config_hash + same
   model_topology_hash + same router_config_hash + same loss_config_hash +
   same gbf-train pass_version + same dependency lockfile + same
   rust_toolchain_hash + same build_config_hash + same device_profile
   ⇒ bit-identical safetensors checkpoint AND bit-identical
       s7_switch_stats.v1, s7_router_collapse_sweep.v1,
       s7_dense_vs_moe.v1, s7_frontier.v1, s7_report.v1.

D16 Fail-closed on collapse
   Collapse is evaluated only after:
     router_collapse_grace_steps = 500 steps after Phase B begins.

   Let entropy_floor_bits = 0.5 * log2(n_experts).
   Let entropy_window_steps = 100.
   Let entropy_for_step = min over layers of expert_usage_entropy_bits.

   If, after the grace period, the rolling mean of entropy_for_step over
   entropy_window_steps is:

     rolling_mean(entropy_for_step) < entropy_floor_bits

   the run halts with completion = CollapsedAt(step), and the variant
   is recorded as Fail-router-collapse. This is distinct from the
   sweep guardrail (D11) which is a deliberate falsification probe.

D17 No retroactive promotion
   If H3 (matched-bytes parity) fails, S7 closes with Fail-parity. S8
   inherits a dense-only production track. There is no "we'll tune
   lambdas and re-run" loop within S7. Re-runs after RFC amendment bump
   pass_version per S1 Rep-5.

D18 Oracle agreement carries through
   The S3 ArtifactOracle three-way agreement law (train ~ bundle ~
   artifact-oracle within S3-pinned tolerance) MUST hold on the MoE
   artifact. The artifact evaluator MUST resolve deployable full-precision
   weights through QuantSpec::weight_quant per CLAUDE.md "Oracle and
   Conformance Beads" — not by tensor-id naming convention, and not by
   assuming dense-FFN tensor names.
   For the routed FFN path specifically, the oracle test fixture MUST
   exercise at least one token whose argmax-route differs across two
   layers, ruling out the "router stuck on expert 0" trivial case.

D19 Standard producer telemetry, every step
   Per planv0 amendment item 5 + bd-3pg session note:
     expert_usage_entropy_bits       ; Shannon entropy of per-expert assignment counts, base-2
     same_expert_rate                ; per layer; already in TemporalSwitchDigest
     router_confidence_distribution  ; max-softmax-mass per token: mean, p10, p50, p90
     tokens_per_expert               ; raw count per (layer, expert) per step
     bank_switches_per_token         ; deploy metric, computed from consecutive argmax-routes
   Recorded as gbf-train::logging structured tracing events (per CLAUDE.md
   logging-bead bullet). In the post-Phase-D sweep harness, each
   lambda_switch grid point adds:
     quality_delta_per_lambda_switch ; bpc(MoE @ lambda) - bpc(MoE @ production lambda)
   on a held-out 4096-sequence eval subset.
   Subscriber-level capture is the proof obligation; producer-side
   collection in gbf-train is owned by S7. Real dashboard / report
   adoption is owned by F-C4 and is out of S7 scope per CLAUDE.md
   logging-bead bullet.
```

---

# 1. Hypothesis algebra

Every hypothesis carries a statement, predicted observables, falsification
rule, verdict mapping, and downstream consequence. H1, H2, H3, H4, H6,
H7, H8 are **mandatory closure gates**. H5, H9, H10 are **closure-gating
in inherited form** (i.e. they are inherited unchanged from earlier
slices and S7's job is to prove they still hold under MoE).

## H1 MoeTiny trains end-to-end

```text
Statement:
  For every seed s, the MoeTiny training loop produces finite losses and
  finite gradient norms across all five phases (A->E), the early Phase A
  loss decreases over pre-registered windows, and no seed triggers the
  D16 collapse halt.

Predicted:
  ∀ s, step ∈ 1..=20000. loss(s, step) is finite, grad_norm(s, step) is finite
  mean_train_loss(s, steps 1..10)    ∈ [3.5, 5.0]    ; nats; warm starts below ln(80) = 4.382
  mean_train_loss(s, steps 491..500) < mean_train_loss(s, steps 1..10) − 0.3
  ∀ s, step ∈ Phase A. expert_usage_entropy_bits is reported but not asserted (router not warmed)
  ∀ s, step ∈ Phase B onward. expert_usage_entropy_bits ≥ 0.5 * log2(n_experts)
                              (D16; non-collapse during normal training)

Falsification:
  ∃ s, step. loss(s, step) is non-finite                           ⇒ Refuted
  ∃ s. mean_train_loss(s, 491..500) ≥ mean_train_loss(s, 1..10) − 0.3
                                                                    ⇒ Refuted
  ∃ s, step. grad_norm(s, step) is non-finite                      ⇒ Refuted
  ∃ s. ∀ step. grad_norm(s, step) = 0                              ⇒ Refuted
  ∃ s, step ∈ Phase B onward.
       expert_usage_entropy_bits(s, step) < 0.5 * log2(n_experts)        ⇒ Refuted
                                                                       (D16 fired;
                                                                        S7Outcome = Fail-router-collapse)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S7 cannot proceed; bd-2v9r blocked.
  If Refuted by collapse, the lambda_switch_production value (D5) is too
  high or expert dropout / jitter is mistuned. Investigate before re-run.
```

## H2 Dense matched-bytes baseline trains end-to-end

```text
Statement:
  For every seed s, the MoeTinyDenseMatched training loop produces finite
  losses and finite gradient norms across Phase A, C, D, E (Phase B is
  no-op), and the early Phase A loss decreases over pre-registered windows.

Predicted:
  ∀ s, step ∈ 1..=20000. loss(s, step) is finite, grad_norm(s, step) is finite
  mean_train_loss(s, steps 1..10)    ∈ [3.5, 5.0]
  mean_train_loss(s, steps 491..500) < mean_train_loss(s, steps 1..10) − 0.3
  All Phase B logging events for the dense run carry router_present = false
                                                                    (D8)

Falsification:
  ∃ s, step. loss(s, step) is non-finite                           ⇒ Refuted
  ∃ s. mean_train_loss(s, 491..500) ≥ mean_train_loss(s, 1..10) − 0.3
                                                                    ⇒ Refuted
  ∃ s, step. grad_norm(s, step) is non-finite                      ⇒ Refuted
  ∃ s. completion(s, dense) = DivergedAt(_)                        ⇒ Refuted
  ∃ s, step. dense run records router_present = true               ⇒ Refuted
                                                                       (training-scaffold-parity violation)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S7 cannot proceed; the parity gate has no valid baseline. Investigate
  the dense topology constructor (T13.1 / bd-ne6t) and the Phase B no-op
  contract (bd-do2j).
```

## H3 Matched-bytes parity gate (mandatory closure gate)

```text
Statement:
  At equal deployed bytes (within D6 tolerance), MoeTiny beats
  MoeTinyDenseMatched on Gutenberg val bpc by strictly more than 0.05 bpc,
  for every one of the five seeds.

Predicted:
  bpc(MoeTinyDenseMatched, val_gutenberg)   ∈ [1.7, 2.4]   ; sanity range only
  median(bpc(MoeTiny, val_gutenberg))        ∈ [1.5, 2.2]   ; sanity range only
  ∀ s. bpc(MoeTiny_seed=s, val_gutenberg)
        < bpc(MoeTinyDenseMatched_seed=s, val_gutenberg) − 0.05
                                                            ; the actual gate (D12)
  |B_deployed_total(MoE) - B_deployed_total(Dense)|
        <= max(0.10 * B_deployed_total(MoE), 4 * one_bank_bytes)
                                                            ; bytes parity (D6)

Falsification:
  ∃ s. bpc(MoeTiny_seed=s) ≥ bpc(MoeTinyDenseMatched_seed=s) − 0.05  ⇒ Refuted
  |B_deployed_total(MoE) - B_deployed_total(Dense)| > tolerance       ⇒ NotEvaluable;
                                                                        S7Outcome = Fail-bytes
  median(bpc(MoeTiny)) < 0.5                                          ⇒ Refuted (suspicious)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S7Outcome = Fail-parity. S8 inherits a dense-only production track
  per D17. bd-2v9r closes with Fail-parity (a successful scientific
  falsification of the MoE-wins claim, not an implementation failure).
  Note: planv0 amendment item 2 explicitly admits this outcome as a
  legitimate research result.
```

## H4 Pareto dominance (mandatory closure gate)

```text
Statement:
  On the (val_bpc, deployed_bytes_total) plane, the MoeTiny
  CheckpointFrontierPoint dominates the MoeTinyDenseMatched
  CheckpointFrontierPoint per D13.

Predicted:
  median_val_bpc(MoE)        <= median_val_bpc(dense_matched)
  deployed_bytes_total(MoE)  <= deployed_bytes_total(dense_matched)
                                                  ; equal within D6 tolerance
  At least one inequality is strict.

Falsification:
  median_val_bpc(MoE)        > median_val_bpc(dense_matched)            ⇒ Refuted (Pareto-incomparable)
  deployed_bytes_total(MoE)  > deployed_bytes_total(dense_matched)
                                + tolerance                              ⇒ Refuted
  Neither inequality is strict (exact tie on both)                       ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S7Outcome = Fail-pareto. (Distinct from Fail-parity: a per-seed
  win on bpc with median equality, or a marginal byte overhead, can
  trigger Fail-pareto without Fail-parity.) bd-2v9r blocked.
```

## H5 Router switch-awareness (mandatory closure gate)

```text
Statement:
  TemporalSwitchDigest, ClipSaturationDigest, ExpertPayloadDigest are
  emitted in the exported MoeTiny ExportFacts with the schema and
  invariants pinned in §3.3, §3.4, §3.5; the per-step standard producer
  telemetry of D19 is captured under structured tracing.

Predicted:
  for every layer L in MoeTiny:
    TemporalSwitchDigest_L.same_expert_rate_q8_8 ∈ [0, 256]
    sum(TemporalSwitchDigest_L.transition_mass) <= 256
    ClipSaturationDigest_L.saturation_rate_q8_8 ∈ [0, 256]
    ExpertPayloadDigest_L lists exactly n_experts entries
    each entry's byte_count > 0
  number of layers in TemporalSwitchDigest = n_blocks = 4
  ExpertSlotAffinity is a canonicalized unordered hint per bd-2pe
  per_step_log captures all 5 metrics of D19 every step
  lambda_switch_sweep_log captures one record per grid point, after 1000
    additional training steps from the same base checkpoint

Falsification:
  any digest field violates its invariant                              ⇒ Refuted
  number of TemporalSwitchDigest layers ≠ 4                            ⇒ Refuted
  ExpertSlotAffinity hint pair ordering is direction-dependent         ⇒ Refuted (canonicalization broken)
  per_step_log misses any of the 5 metrics                              ⇒ Refuted
  lambda_switch_sweep_log does not contain exactly one 1000-extra-step
    record per grid point                                                ⇒ Refuted
  ExpertId scoping is global (not LayerId-scoped) in the export schema  ⇒ Refuted
                                                                          (per CLAUDE.md export-fact bullet)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S7Outcome = Fail-switch-stats. The compiler's ExpertSlotAffinity hints
  and the F-C4 conformance dashboard cannot trust the export. bd-2v9r
  blocked.
```

## H6 Router collapse guardrail (mandatory closure gate)

```text
Statement:
  The lambda_switch sweep over D11's grid demonstrates that:
    A. production lambda_switch (0.05) does not regress bpc more than
       0.05 bpc relative to lambda_switch = 0.0;
    B. production lambda_switch maintains expert_usage_entropy_bits >= 0.85 *
       log2(n_experts);
    C. high-lambda lambda_switch (5.0) demonstrably collapses entropy
       (drops by >= 0.3 bits relative to production);
    D. high-lambda lambda_switch (5.0) demonstrably regresses bpc (rises
       by >= 0.3 bpc relative to production).

Predicted: as in D11.

Falsification: as in D11; failure of A, B, C, or D ⇒ Refuted.

Note on the asymmetry of C and D:
  Both C and D must hold. C without D would mean "collapse without
  measurable quality cost" (suspicious; the entropy metric may be
  miscomputed). D without C would mean "quality cost without measurable
  collapse" (suspicious; the entropy floor may be too low). Requiring
  both pins the collapse phenomenon to both quality and entropy axes.

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S7Outcome = Fail-router-collapse-guardrail. The production
  lambda_switch may be in a region where collapse is undetectable, OR
  the sweep itself is not exercising a wide enough range. bd-2v9r blocked.
```

## H7 Loss gradient provenance (mandatory closure gate)

```text
Statement:
  Each loss term's gradient flow matches its declared provenance, on a
  fixed tiny fixture batch.
  Specifically:
    z-loss            (router_z_loss):
      gradient reaches raw_router_logits z
      gradient does NOT reach the LowRankRouter parameters via any
        path other than through z (i.e. the z-loss is uncentered/centered
        as declared in D5; centered baseline = 0)
    balance_loss      (expert_load_balance_loss):
      gradient reaches the soft routing distribution p
      gradient is exactly 0 on the dispatch indicator (stop-gradient
        dispatch per D3)
      gradient does NOT reach the expert parameters
    switch_loss       (temporal_switch_penalty L_switch):
      gradient reaches the soft routing distribution p (both p_{l,t}
        and p_{l,t-1} via autodiff)
      gradient does NOT reach across sequence boundaries (sequence_mask
        zeroes the cross-boundary contribution)
      gradient does NOT reach expert parameters from L_switch alone
    distill_loss      (logit_distillation_loss):
      gradient reaches student_logits and through them selected expert
        parameters, embeddings, sequence-state, and norm.
      Does NOT reach LowRankRouter parameters through dispatch, because
        dispatch_indicator is stop-gradient by D3.
      gradient does NOT reach the frozen teacher parameters
    lm_loss           (cross-entropy on charset_v1):
      gradient reaches student_logits and through them selected expert
        parameters, embeddings, sequence-state, and norm.
      Does NOT reach LowRankRouter parameters through dispatch, because
        dispatch_indicator is stop-gradient by D3.

  Router task-coupling note:
    In S7, the router is trained only by router_z_loss, balance_loss, and
    temporal_switch_penalty. The LM/distillation objective does not directly
    train routing decisions. This is an explicit scientific risk of the
    hard-dispatch/no-STE design and is recorded as an observed limitation if
    H3 fails.

Predicted:
  on a fixture batch of (batch=2, seq=8, n_experts=4, n_blocks=1):
    each declared "reaches" gradient is finite and at least one entry
    has |grad| >= 1e-6
    each declared "does NOT reach" gradient is exactly the zero tensor
    centered z-loss baseline: when all router logits = 0,
      router_z_loss = 0 within tolerance 1e-12 in f64

Falsification:
  any declared reach has all-zero gradient                       ⇒ Refuted
  any declared not-reach has nonzero gradient                    ⇒ Refuted
  centered z-loss baseline ≠ 0 within 1e-12                       ⇒ Refuted
  switch_loss gradient flows across a sequence boundary          ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Loss math is dishonest per CLAUDE.md "training loss bullets". One of:
    - L_switch's stop-gradient discipline is broken
    - balance_loss is back-propagating through hard dispatch
    - z-loss is uncentered when declared centered
    - the teacher freeze is leaking gradient
  Halt. S7 closure forbidden until the offending term is corrected.
```

## H8 ExpertBlockQat Burn gradient smoke (mandatory closure gate)

```text
Statement:
  The Burn adapter for ExpertBlockQat (bd-2c8z) produces deterministic,
  finite, nonzero gradients into the intended parameter sets and exactly-
  zero gradients into stop-gradient sets, on a fixed tiny fixture batch.
  Mirrors S2's LinearState gradient smoke (bd-1y1s closed at S2).

Predicted:
  Fixture: batch=2, seq=4, d_model=8, d_ff=16, n_experts=2, two-matrix
  expert with clipped activation (bd-x75 default).
  After one backward pass under loss = sum(expert_output**2), for each
  supported clipped activation (relu, gelu_clip, silu_clip):
    grad(up.weight)         finite, sum(|grad|) > 0
    grad(down.weight)       finite, sum(|grad|) > 0
    activation range mode           fixed range only at the Burn boundary;
                                      learned/EMA activation ranges remain
                                      rejected until state ownership exists
    expert projection biases         unsupported by the model contract;
                                      construction with projection bias
                                      remains rejected, not silently trained
    grad(GatedLinearUnit gate)       does not exist; bd-2c8z rejects GLU
                                       at construction time
  Determinism: replay with identical inputs ⇒ bit-identical gradients.

Falsification:
  any required gradient is non-finite                              ⇒ Refuted
  any required nonzero gradient is identically zero                ⇒ Refuted
  any rejected variant (GLU) silently constructs                   ⇒ Refuted
  replay produces non-bit-identical gradients                      ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  The Burn ExpertBlockQat adapter (bd-2c8z) is broken. H1 results are
  meaningless because the trained parameters cannot be trusted to have
  received correct gradients. bd-2v9r blocked.

Closure citation:
  This hypothesis MUST be backed by:
    cargo test -p gbf-train --features burn-adapter -- expert_block_qat_grad
  per CLAUDE.md "If a loss claim depends on Burn autodiff, closure must
  cite a feature-enabled gate".
```

## H9 ArtifactOracle three-way agreement on the routed FFN (carry-through)

```text
Statement:
  The S3 ArtifactOracle three-way agreement law
    train_output ≈ exported_reference_bundle ≈ artifact_oracle_output
  holds on the MoeTiny artifact within S3-pinned tolerance, on a routed
  FFN fixture per D18.

Predicted:
  for the test fixture defined in §6.5:
    pairwise_logit_max_abs_diff(train, bundle)     <= S3_tol
    pairwise_logit_max_abs_diff(bundle, artifact)  <= S3_tol
    pairwise_logit_max_abs_diff(train, artifact)   <= 2 * S3_tol
  fixture exercises at least one token whose argmax-route differs
  across two different MoE layers (per D18).

Falsification:
  any pairwise diff exceeds tolerance                              ⇒ Refuted
  fixture has all tokens routing to the same expert in every layer ⇒ Refuted
                                                                      (D18 violated)
  artifact oracle resolves dense-FFN tensor names instead of routed
    paths                                                           ⇒ Refuted
                                                                      (per CLAUDE.md oracle bullet)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Either the export is incorrect for routed FFNs, or the artifact oracle
  cannot resolve the routed weight stack. bd-2v9r blocked. The S3
  closure law must continue to hold for every model topology that emits
  a ROM (planv0 amendment item 7).
```

## H10 EncodedRom + emulator one-token harness (carry-through)

```text
Statement:
  The S5 (Pick and Fit) EncodedRom + emulator one-token harness preserves on the MoeTiny
  artifact: a single forward token from a fixed prompt produces the
  expected logits modulo the S5 (Pick and Fit) pinned tolerance, AND the runtime
  records exactly one MBC5 bank-switch event per actual expert change
  (no spurious switches; no missed switches).

Predicted:
  emulator one-token output for the canonical S5 (Pick and Fit) prompt under the
    MoeTiny-derived EncodedRom matches the artifact-oracle one-token
    output within the S5 (Pick and Fit) pinned tolerance.
  observed_bank_switches_per_token == bank_switches_per_token
    computed by the artifact-oracle route tracer on the same fixed prompt
    (within 1; off-by-one for the prompt prefix is permitted).

Falsification:
  emulator one-token output diverges from artifact-oracle output
    beyond S5 (Pick and Fit) tolerance                                   ⇒ Refuted
  observed_bank_switches_per_token differs from artifact-oracle recorded
    value by > 1 (after prefix correction)                                ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Either the routed FFN's compile path lost a route, or the emulator's
  bank-switch accounting is wrong. bd-2v9r blocked. The S5 (Pick and Fit) emulator
  contract is incompatible with MoE artifacts; investigate the routed
  EncodedRom path.
```

Hypothesis composition rules are formalized in §10 (Outcome algebra).

---

# 2. Authority rules

```text
Scope(F-S7) =
  {
    H1, H2, H3, H4, H5, H6, H7, H8, H9, H10,
    MoeTiny ModelSizeProfile reference instance,
    MoeTinyDenseMatched ModelSizeProfile reference instance,
    matched-deployed-bytes formula (D6),
    Top1RouterQat with stop-gradient dispatch (D3),
    LowRankRouter (D7) with router_rank = 4 override,
    ExpertBlockQat two-matrix Burn adapter (bd-2c8z),
    Temporal smoothness regularization (D10),
    L_switch differentiable temporal switch loss (T5.1; D5; D10),
    Loss composition (D5),
    Router collapse guardrail (D11; D16),
    Switch statistics export (D19; §3.3-3.5),
    RouterRng disjoint stream (D14),
    Matched-bytes parity gate margin (D12),
    Pareto dominance contract (D13),
    s7_run_log.v1, s7_score.v1, s7_switch_stats.v1,
    s7_router_collapse_sweep.v1, s7_dense_vs_moe.v1,
    s7_frontier.v1, s7_burn_grad_smoke.v1,
    s7_oracle_routed.v1, s7_emulator_one_token.v1,
    s7_report.v1
  }

Rule Authority:
  ∀ behavior b ∈ Scope(F-S7) ∧ this RFC specifies b
  ⇒ SourceOfTruth(b) = this RFC.

Rule PlanContext:
  Behavior outside Scope informed by planv0 amendments and bd-2v9r
  comments. Closed features (F1, F3, F4, F6, F12, F2, F8, F11) and the
  T14.1 ModelSizeProfile registry provide the substrate; their contracts
  are not amended by this RFC except where explicitly noted (D7 router
  rank override; D2 n_experts default).

Rule CrateOwnership:
  Every behavior in Scope(F-S7) is implemented in exactly one of:
    - gbf-experiments    (S7 namespace; hosts s7_* operations,
                          falsification suite, schema encoders, replay CLI)
    - gbf-policy         (MoeTiny + MoeTinyDenseMatched ModelSizeProfile
                          instances; matched-bytes formula constants)
    - gbf-model          (Top1RouterQat, LowRankRouter, ExpertBlockQat,
                          temporal smoothness pair-set helper, expert dropout,
                          jitter, switch statistics collection)
    - gbf-train          (loss composer extensions for lambda_distill,
                          lambda_balance, lambda_zrouter, lambda_switch;
                          Burn adapter for ExpertBlockQat;
                          per-step producer telemetry; lambda_switch
                          sweep harness; phase-effective gating;
                          matched-bytes parity gate runner)
    - gbf-data           (Gutenberg loader carry-through from S4;
                          unchanged)
    - gbf-foundation     (carry-through; unchanged)
    - gbf-artifact       (TemporalSwitchDigest, ClipSaturationDigest,
                          ExpertPayloadDigest schemas; LayerId scoping;
                          ExpertSlotAffinity canonicalization; carry-through)
    - gbf-report         (s7_dense_vs_moe.v1 emitter per bd-12b9)
    - gbf-cli            (`gbf s7` subcommand for replay)
  No S7-specific code lives outside this set.

Rule Amendment:
  Later slice changes any of:
    MoeTiny dim caps
    matched-bytes formula
    parity gate margin
    n_experts default
    router rank override
    lambda defaults or sweep grid
    L_switch gradient provenance
    seed list (S1 D2)
    phase boundaries (D4)
    train budget (D9)
  ⇒ Later slice's RFC must explicitly amend this RFC.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation
  produces the expected Refuted verdict on the appropriate hypothesis.
  Falsification sensitivity is a first-class proof obligation (§16 O5).
```

---

# 3. Core notation

S7 inherits the S1 base types (Hash256, Seed, TrainStep, EvalStep,
LossNatsPerByte, BpcValue, GradNorm, Verdict, HypothesisStatus, FailureKind,
PredictedRange, ObservedStatistic, DomainHash, Self-hash rule, Canonical-
TensorPayloadHash, S1CanonicalJson, Prediction status rule), the S2 phase
types (TrainPhaseSpec, QuantHardness, RouterTrainMode), the S3 charset
types (CharsetV1, CharsetVocab=80, KN5SmoothingScheme, ConformanceEnvelope),
the S4 corpus types (GutenbergManifest), the S5 frontier types (Decay-
Policy, CheckpointFrontierPoint, AttentionOracle), and the S5 (Pick and Fit) deployment
types (RuntimeChromeBudget, CompileProfile, EncodedRom).

S7 introduces the following NEW types.

## 3.1 MoeTiny topology

```text
ExpertCount    := u8      ; valid in {2, 4} for MoeTiny per D2
ExpertId       := u8      ; LAYER-LOCAL per CLAUDE.md export-fact bullet;
                            disambiguated by carrying LayerId alongside.
LayerId        := u8      ; 0..n_blocks-1
RouterRank     := u8      ; valid >= 1 and <= n_experts
SmoothnessWindow := u16   ; tokens; >= 2 (window=1 forbidden per §6.2 F7)
LambdaSwitch   := f32     ; finite, >= 0
LambdaBalance  := f32     ; finite, >= 0
LambdaZRouter  := f32     ; finite, >= 0
LambdaDistill  := f32     ; finite, >= 0

FfnPathConfig :=
    Dense
  | MoE { n_experts: ExpertCount, router: RouterConfig }

RouterConfig :=
  {
    rank:                  RouterRank
    smoothness_window:     SmoothnessWindow
    expert_dropout_rate:   f32          ; in [0, 1)
    jitter_stddev:         f32          ; >= 0; Gaussian on logits
    train_mode:            RouterTrainMode  ; SoftTop1 in Phase A; HardTop1 from B onward
    centered_z_loss:       bool         ; true for S7 (D5)
  }

ExpertBlockConfig :=
  {
    activation_kind:       ActivationKind   ; ClippedRelu | ClippedGelu (bd-x75 default)
    activation_clip:       f32              ; finite, > 0
    no_glu:                bool = true      ; bd-2c8z explicitly rejects GLU
  }

ModelTopologyConfig (S7 instance, MoeTiny):
  {
    profile:               "MoeTiny"
    d_model:               u16 = 64
    d_ff:                  u16 = 128
    n_blocks:              u8  = 4
    vocab:                 u16 = 80          ; charset_v1
    embedding_tied:        bool = true
    sequence:              SequenceSemanticsSpec  ; LinearState multi-timescale (S5)
    norm:                  NormPlan          ; AffineClipLut (S2 default)
    ffn_path_per_block:    Vec[FfnPathConfig] of length n_blocks
                                              ; for MoeTiny: all MoE
    expert_block:          ExpertBlockConfig
    router:                RouterConfig
  }

ModelTopologyConfig (S7 instance, MoeTinyDenseMatched):
  {
    profile:               "MoeTinyDenseMatched"
    d_model:               u16 = 64
    d_ff:                  u16 = d_ff_dense   ; per D6; pinned in matched_bytes.json
    n_blocks:              u8  = 4
    vocab:                 u16 = 80
    embedding_tied:        bool = true
    sequence:              SequenceSemanticsSpec  ; LinearState multi-timescale (S5)
    norm:                  NormPlan          ; AffineClipLut
    ffn_path_per_block:    Vec[FfnPathConfig] = [Dense, Dense, Dense, Dense]
    expert_block:          (none; dense)
    router:                (none; dense)
  }
```

## 3.2 LowRankRouter

```text
LowRankRouter :=
  {
    proj_down:  Linear[d_model, rank]    ; high precision (NOT ternary)
    proj_up:    Linear[rank, n_experts]  ; high precision
  }

Forward semantics (training):
  z = proj_up(proj_down(x)) + jitter(RouterRng, jitter_stddev)
  p = softmax(z)
  dispatch = onehot(argmax(z))            ; stop-gradient on dispatch
  expert_output = sum_e dispatch[e] * Expert_e(x)

Forward semantics (eval/export):
  z = proj_up(proj_down(x))               ; no jitter (D14)
  p = softmax(z)                           ; computed only for digest export
  dispatch = onehot(argmax(z))
  expert_output = Expert_argmax(x)         ; single expert called

Parameter count:
  full_rank   d_model * n_experts                = 64 * 4 = 256
  low_rank    d_model * rank + rank * n_experts  = 64 * 4 + 4 * 4 = 272
  delta       +6.25%   (negative; low-rank with rank = n_experts adds bias)
  This is intentional. The implicit-regularization claim is the load-
  bearing rationale, not the parameter savings. See A4.

Centered z-loss (D5):
  z_loss_uncentered = (1/B) sum_b log(sum_e exp(z_{b,e}))^2
  z_loss_centered   = (1/B) sum_b (log(sum_e exp(z_{b,e})) - mu)^2
                                    where mu = log(n_experts).
  S7 uses CENTERED z-loss with a constant mu = log(n_experts), not a
  batch running mean. When all router logits are 0, log(sum_e exp(0)) =
  log(n_experts), so the centered z-loss baseline is 0 by construction.
  This is the "centered" variant declared by D5 and falsifiable by
  F5-z-uncentered (§16 O5).
```

## 3.3 TemporalSwitchDigest (export-fact schema)

```text
TemporalSwitchDigest :=
  {
    schema_version:        SemVer = "1.0"
    layer_id:              LayerId            ; per CLAUDE.md: LayerId-scoped
    n_experts:             ExpertCount
    same_expert_rate_q8_8: u16                ; in [0, 256] = [0.0, 1.0] in Q8.8
    transition_mass:       Vec[TransitionEntry]  ; top-K entries; K bounded by export config
    digest_self_hash:      Hash256
  }

TransitionEntry :=
  {
    from_expert: ExpertId                    ; layer-local
    to_expert:   ExpertId                    ; layer-local
    mass_q8_8:   u16                          ; in [0, 256]; per-pair transition
                                              ; probability under the layer's
                                              ; observation window
  }

UnorderedExpertPair :=
  {
    lo: ExpertId
    hi: ExpertId
    invariant: lo <= hi                       ; canonicalized at construction
                                              ; AND in deserialization
  }

Invariants (per CLAUDE.md export-fact bullets):
  TSD-1   same_expert_rate_q8_8 <= 256                  (rate is a probability)
  TSD-2   sum_{e in transition_mass} e.mass_q8_8 <= 256 (aggregate distribution invariant
                                                         in CONSTRUCTOR and DESERIALIZATION;
                                                         not just per-entry)
  TSD-3   no two TransitionEntry with the same
          (from_expert, to_expert) pair                 (uniqueness)
  TSD-4   ExpertId values are layer-local: an ExpertId
          in TemporalSwitchDigest_{layer=L} need not
          equal an ExpertId in TemporalSwitchDigest_{layer=L'}
  TSD-5   from_expert and to_expert are directional and are NOT canonicalized.
          Canonicalization occurs only when constructing ExpertSlotAffinity.
```

### 3.3.1 ExpertSlotAffinity canonicalization

```text
ExpertSlotAffinity :=
  {
    schema_version:    SemVer = "1.0"
    layer_id:          LayerId                    ; ExpertId is layer-local
    affinities:        Vec[CanonicalizedAffinity]
    affinity_self_hash: Hash256
  }

CanonicalizedAffinity :=
  {
    pair:              UnorderedExpertPair        ; lo <= hi enforced
    affinity_score:    AffinityScore               ; aggregation rule pinned (per bd-3pg comment)
  }

Aggregation rule (per bd-2pe handoff comment on bd-3pg):
  The directional TemporalSwitchDigest.transition_mass entries are
  aggregated INTO the unordered ExpertSlotAffinity by the SUM rule:
    aff(a, b) = transition_mass(a -> b) + transition_mass(b -> a)
                  for a < b
    aff(a, a) = transition_mass(a -> a)             ; same-expert
  This rule is pinned by S7 and recorded in s7_switch_stats.v1.
  Alternative rules (max, max-symmetric) are forbidden in S7.
```

## 3.4 ClipSaturationDigest

```text
ClipSaturationDigest :=
  {
    schema_version:        SemVer = "1.0"
    layer_id:              LayerId
    saturation_rate_q8_8:  u16          ; in [0, 256]; fraction of activation
                                          ; entries that hit the clip bound
                                          ; during the observation window
    clip_bound_observed:   f32          ; the runtime clip value used
    digest_self_hash:      Hash256
  }

Invariants:
  CSD-1  saturation_rate_q8_8 <= 256
  CSD-2  clip_bound_observed is finite and > 0
```

## 3.5 ExpertPayloadDigest

```text
ExpertPayloadDigest :=
  {
    schema_version:    SemVer = "1.0"
    layer_id:          LayerId
    artifact_path:     String                  ; path within ModelArtifact
                                                ; per CLAUDE.md export-fact bullet:
                                                ; "include LayerId or ArtifactPath"
                                                ; (we include both)
    entries:           Vec[ExpertPayloadEntry]  ; length = n_experts
    digest_self_hash:  Hash256
  }

ExpertPayloadEntry :=
  {
    expert_id:         ExpertId                 ; layer-local
    byte_count:        u32                      ; > 0; matches
                                                ; TernaryWeightPlan::compute_byte_cost
                                                ; for this expert
    weight_quant:      QuantSpec                ; per CLAUDE.md oracle bullet:
                                                ; "resolve deployable weights through
                                                ; QuantSpec::weight_quant"
  }

Invariants:
  EPD-1  entries.length = n_experts
  EPD-2  every byte_count > 0
  EPD-3  artifact_path is non-empty
  EPD-4  expert_id values exhaust 0..n_experts-1 (no missing or duplicate)
```

## 3.6 Standard producer telemetry (per-step)

```text
RouterStepTelemetry :=
  {
    schema_version:                 SemVer = "1.0"
    seed:                           Seed
    train_step:                     TrainStep
    layer_id:                       LayerId
    expert_usage_entropy_bits:      f32       ; bits; in [0, log2(n_experts)]
    same_expert_rate:               f32       ; in [0, 1]
    router_confidence_distribution: ConfidenceDist     ; mean, p10, p50, p90 of max-softmax
    tokens_per_expert:              Vec[u32]  ; length = n_experts
    bank_switches_per_token:        f32       ; in [0, n_blocks]; deploy metric
    telemetry_self_hash:            Hash256
  }

ConfidenceDist :=
  { mean: f32, p10: f32, p50: f32, p90: f32 }
  invariant: 0 <= p10 <= p50 <= p90 <= 1 ; max-softmax is bounded
  invariant: mean is finite and in [0, 1]

LambdaSwitchSweepStep :=
  {
    schema_version:                 SemVer = "1.0"
    seed:                           Seed
    train_step:                     TrainStep
    lambda_switch:                  LambdaSwitch
    completion:                     Completed | DivergedAt(TrainStep)
    bpc_eval_subset:                Null | BpcValue
    expert_usage_entropy_bits_mean: f32          ; averaged across layers (bits)
    quality_delta_per_lambda_switch: Null | f32  ; bpc_eval_subset(lambda) -
                                                  ;   bpc_eval_subset(lambda_production);
                                                  ;   null iff completion = DivergedAt(_)
    sweep_self_hash:                Hash256
  }

Invariants:
  RST-1  expert_usage_entropy_bits in [0, log2(n_experts)]
  RST-2  sum(tokens_per_expert) > 0
  RST-3  bank_switches_per_token in [0, n_blocks]
  LSS-1  lambda_switch in D11 sweep grid
  LSS-2  exactly one record per grid point; each record produced after
         1000 additional training steps from the same base checkpoint
         (post-Phase-D sweep harness; D11 / D19 / §9.3)
  LSS-Diverged
         bpc_eval_subset and quality_delta_per_lambda_switch are null iff
         completion = DivergedAt(_); expert_usage_entropy_bits_mean is the
         last finite observed value in bits
```

## 3.7 RouterRng and seed derivation

```text
RouterRng(seed)   = Pcg64Mcg(seed128("router", seed))
                                              ; disjoint from S1 streams (D14)

DropoutSubRng(seed, step) =
  Pcg64Mcg(seed128("router-dropout", seed) XOR (step as u128))

JitterSubRng(seed, step, layer_id) =
  Pcg64Mcg(seed128("router-jitter", seed) XOR (step as u128) XOR ((layer_id as u128) << 32))

Eval/export: NO RouterRng draws.

Reconstructability rule (per CLAUDE.md routing/dropout determinism):
  Given (seed, step, layer_id), the dropout mask and jitter values are
  recomputable without replaying earlier draws.
```

## 3.8 Domain hashes (S7 additions)

```text
S7DomainHashKeys (canonical-JSON object; sorted keys):
  - matched_bytes_formula_version: SemVer
  - moe_tiny_topology_hash:        Hash256
  - moe_tiny_dense_matched_topology_hash: Hash256
  - low_rank_router_config_hash:   Hash256
  - expert_block_config_hash:      Hash256
  - loss_config_hash:              Hash256
  - phase_schedule_hash:           Hash256
  - lambda_switch_sweep_grid_hash: Hash256

  Each is a DomainHash over the canonical JSON of its source struct,
  with self_hash fields omitted (Self-hash rule from S1).
```

---

# 4. Authority delta from S1..S5 (Pick and Fit)

This section explicitly enumerates the deltas S7 introduces over S1..S5
(Pick and Fit) so the reviewer does not have to diff six prior RFCs.

```text
Delta-1   New variant axis (MoE | dense_matched)
  S1..S5 (Pick and Fit) ran a single topology per slice (S5 had three sequence-block
  variants; that axis is orthogonal). S7 introduces two PARITY-COUPLED
  topology variants under a shared training scaffold. Every per-seed
  artifact schema gains a `topology` field in {"MoeTiny",
  "MoeTinyDenseMatched"}. Every closure obligation other than H1 vs
  H2 is per-pair (per-seed pair across the two variants).

Delta-2   Topology profile change
  S1..S5 used Toy0 / Toy1 / dense baselines. S7 uses MoeTiny (4 blocks,
  4 experts, MoE every block) as the experimental subject and
  MoeTinyDenseMatched as the matched-bytes control. Both sit at
  d_model=64, d_ff=128 (MoE) / d_ff_dense (dense). Profile registry
  per planv0 amendment item 1 (F14 / bd-rq46) is the source of truth.

Delta-3   Sequence length bump
  S1 used sequence_length=128. S5 onward kept that. S7 bumps to
  sequence_length=256 (D9) so the temporal smoothness window of 32
  has room to act on contiguous trajectories of >= 8 windows per
  sequence. No prior gate is renormalized; the bpc primitive's
  reset-context semantics extends naturally to chunk_size=256.

Delta-4   Train budget
  Inherits S5's optimizer_steps=20000. Phase boundaries (D4) split this
  budget across A (4000) / B (4000) / C (6000) / D (4000) / E (2000).

Delta-5   New loss terms
  S1..S5 had only lm_loss (and S2's QAT-internal helpers). S7 adds:
    lambda_distill * logit_distillation_loss   (Phase C onward)
    lambda_balance * expert_load_balance_loss   (Phase B onward)
    lambda_zrouter * router_z_loss              (Phase B onward)
    lambda_switch  * temporal_switch_penalty    (Phase B onward)
  Each with its declared gradient provenance per H7. T5.5 (lambda_shape /
  lambda_overflow) is GATED OFF (D5).

Delta-6   New RNG stream
  S7 adds RouterRng disjoint from S1's InitRng/BatchRng/ShuffleRng (D14).
  Eval mode consumes NO RouterRng draws. Dropout / jitter sub-streams
  derive from RouterRng XOR step (per CLAUDE.md determinism pattern).

Delta-7   New export-fact schemas
  TemporalSwitchDigest, ClipSaturationDigest, ExpertPayloadDigest enter
  ExportFacts (per planv0 model-side amendment + bd-3pg). LayerId-scoped
  ExpertId per CLAUDE.md export-fact bullet. ExpertSlotAffinity is a
  canonicalized unordered hint per bd-2pe.

Delta-8   Matched-bytes parity gate
  Brand-new contract introduced by S7 (D6, D12, bd-2zv4). Has no analogue
  in S1..S5 (Pick and Fit).

Delta-9   Pareto frontier dominance
  S5 (Pick and Fit) introduced the F8 frontier with a single dense topology. S7 makes
  the frontier comparison meaningful by introducing a second topology
  in the same training run, then asserting Pareto dominance. The
  frontier emission contract (bd-9m2 / bd-2gp) is unchanged; the
  comparison contract (D13) is new.

Delta-10  Router collapse guardrail
  S7 introduces the lambda_switch sweep at seed 0 (D11) and the
  per-step entropy floor (D16). These are first-class proof obligations.

Delta-11  Standard producer telemetry
  S7 specifies five per-step metrics under structured tracing (D19).
  S5 had ad-hoc diagnostics; S7 pins the exact set. Per CLAUDE.md
  logging-bead bullet, S7 owns producer collection; report adoption is
  named to F-C4.

Delta-12  Closure outcome variants
  S7 expands the Outcome algebra to include Fail-parity, Fail-pareto,
  Fail-router-collapse, Fail-router-collapse-guardrail, Fail-switch-stats,
  Fail-grad-provenance, Fail-burn-grad, Fail-oracle-routed,
  Fail-emulator-routed (§10).

Delta-13  Inheritance preservation
  S1 D2 (seed list), D3a (deterministic batch sampling), D8 (strict
  reproducibility), D9 (fail-closed on NaN), D10 (AdamW) carry forward
  unchanged. S2 phase scheduler, S3 charset_v1 + KN-5 baseline + oracle
  three-way agreement, S4 Gutenberg manifest + contamination report,
  S5 (Pick and Fit) LinearState multi-timescale + frontier emission +
  RuntimeChromeBudget + EncodedRom + emulator one-token harness all carry
  through unchanged. S7 does not amend any of these.
```

---

# 5. Experiment state machine

```text
State :=
    Configured(corpus, charset_v1, topologies, train_config, baseline_ref,
               matched_bytes_pin)
  | BaselineMatched(state, d_ff_dense, byte_parity_check)
  | TrainAttempted(state, topology, seed, phase_products)
  | MoeTrainAttempted(state, run_products[5])
  | DenseTrainAttempted(state, run_products[5])         ; parallel with MoE
  | MoeTrained(state, completed_runs[5])
  | DenseTrained(state, completed_runs[5])
  | Scored(state, val_bpc[2 topologies][5 seeds], grad_logs[2][5],
           weight_stats[2][5])
  | ParityChecked(state, parity_per_seed[5], bytes_diff)
  | ParetoEvaluated(state, frontier_verdict)
  | RouterCollapseSwept(state, sweep_results[seed=0])
  | SwitchStatsExported(state, switch_stats_per_layer[4])
  | OracleAgreement(state, oracle_routed_result)
  | EncodedRomBuilt(state, encoded_rom_moe, encoded_rom_dense)
  | EmulatorOneToken(state, emulator_result_moe, emulator_result_dense)
  | BurnGradSmoke(state, grad_smoke_result)
  | Reported(state, report)
  | Decided(state, decision: ProceedToS8
                          | ProceedToS8-DenseOnly
                          | Investigate(reason)
                          | Halt(reason))
```

Transitions:

```text
T0 configure:
  ∅ → Configured(c)

T1 baseline-match:
  Configured(c) → BaselineMatched(c, solve_d_ff_dense(c), check_bytes(c))

T2 train-with-internal-teacher-freeze:
  BaselineMatched(c, _, _) → TrainAttempted(c, topology, seed, phase_products)

  Within each s7_train_run(topology, seed):
    - Phase A runs inside the same run.
    - At the Phase A boundary, the teacher checkpoint for that same
      (topology, seed) is frozen.
    - Phases C, D, and E use that frozen same-topology, same-seed teacher
      for distillation.
    - The top-level experiment state does not contain TeacherFrozen; the
      freeze is a run-internal boundary recorded as
      frozen_teacher_checkpoint_sha in the run provenance.

T3 moe-train (parallel with T4):
  BaselineMatched(c, _, _) → MoeTrainAttempted(c,
    [s7_train_run(c, "MoeTiny", s) for s in seeds])

T4 dense-train (parallel with T3):
  BaselineMatched(c, _, _) → DenseTrainAttempted(c,
    [s7_train_run(c, "MoeTinyDenseMatched", s) for s in seeds])

T3a all completed (MoE):
  MoeTrainAttempted(c, runs) ∧ ∀ r ∈ runs. r.completion = Completed
  → MoeTrained(c, runs)

T3b divergence short-circuit (MoE):
  MoeTrainAttempted(c, runs) ∧ ∃ r ∈ runs. r.completion = DivergedAt(_)
  → Reported(state, build_fail_substrate_report(state, "MoeTiny"))

T3c collapse short-circuit (MoE):
  MoeTrainAttempted(c, runs) ∧ ∃ r ∈ runs. r.completion = CollapsedAt(_)
  → Reported(state, build_fail_router_collapse_report(state))

T4a all completed (dense):
  DenseTrainAttempted(c, runs) ∧ ∀ r ∈ runs. r.completion = Completed
  → DenseTrained(c, runs)

T4b divergence short-circuit (dense):
  DenseTrainAttempted(c, runs) ∧ ∃ r ∈ runs. r.completion = DivergedAt(_)
  → Reported(state, build_fail_substrate_report(state, "MoeTinyDenseMatched"))

T5 score (after both T3a and T4a):
  MoeTrained(c, m_runs) ∧ DenseTrained(c, d_runs)
  → Scored(c,
           {moe: [s7_score_bpc(m_runs[s], V_val_gutenberg) for s in seeds],
            dense: [s7_score_bpc(d_runs[s], V_val_gutenberg) for s in seeds]})

T6 parity (per-seed):
  Scored(...) → ParityChecked(...,
    [parity_seed(moe_bpc[s], dense_bpc[s]) for s in seeds],
    bytes_parity(B_deployed_total_moe, B_deployed_total_dense))

T7 pareto:
  ParityChecked(...) → ParetoEvaluated(..., frontier_verdict(...))

T8 router-collapse-sweep (seed 0):
  ParetoEvaluated(...) → RouterCollapseSwept(..., sweep_at_seed_0(c))

T9 switch-stats-export (production-lambda MoE artifact, all seeds):
  RouterCollapseSwept(...) → SwitchStatsExported(...,
    [export_switch_stats(m_runs[s]) for s in seeds])

T10 oracle-routed (production-lambda MoE artifact, seed 0):
  SwitchStatsExported(...) → OracleAgreement(...,
    oracle_three_way_routed(m_runs[0]))

T11 encoded-rom (both topologies, seed 0):
  OracleAgreement(...) → EncodedRomBuilt(...,
    encoded_rom(m_runs[0]), encoded_rom(d_runs[0]))

T12 emulator-one-token (both topologies, seed 0):
  EncodedRomBuilt(...) → EmulatorOneToken(...,
    emulator_one_token_moe(...), emulator_one_token_dense(...))

T13 burn-grad-smoke (independent fixture):
  Configured(c) → BurnGradSmoke(c, grad_smoke_expert_block_qat(c))

T14 report (after T12 and T13 both):
  EmulatorOneToken(...) ∧ BurnGradSmoke(...)
  → Reported(state, build_report(state))

T15 decide:
  Reported(state, r) → Decided(state, decide(r))
```

Invariants:

```text
I-S7-1
  T1 must produce d_ff_dense BEFORE T2 (teacher freeze depends on the
  resolved dense topology).

I-S7-2
  T3 and T4 use byte-identical training scaffold per D8. Any divergence
  in optimizer config, phase scheduler, RNG kind, device profile, or
  scaffolding code path is a contract violation that aborts both runs
  with a non-zero exit before any tensor allocation.

I-S7-3
  T6 must use the SAME val byte sequence for both topologies (Gutenberg
  V_val per S4 manifest; sha256 enforced).

I-S7-4
  T8 (sweep) at seed 0 uses the same teacher checkpoint as the production
  MoE seed-0 run (no re-warmup). Only lambda_switch varies across the
  sweep grid.

I-S7-5
  T9 (switch-stats export) uses the production-lambda checkpoint, not
  any swept-lambda variant.

I-S7-6
  T13 (Burn grad smoke) is a fixture test; it does not consume any of
  the five seeds. It runs against a distinct fixture seed = 0xFEED.

I-S7-7
  Decided is final: closure of bd-2v9r is gated on
  Decision ∈ {ProceedToS8, ProceedToS8-DenseOnly}.

I-S7-8
  ProceedToS8-DenseOnly is reachable iff S7Outcome = Fail-parity.
  Per D17, Fail-parity is a successful scientific falsification that
  permits S8 to inherit a dense-only production track; it does not
  block bd-2v9r closure.
```

---

# 6. MoeTiny + dense matched-bytes contract

## 6.1 MoeTiny topology contract

```text
operation s7_construct_moe_tiny
  input:  ModelSizeProfile reference instance
  output: ModelTopologyConfig

Preconditions:
  M-Pre-1  profile = ModelSizeProfile::MoeTiny in F14 registry (bd-rq46)
  M-Pre-2  n_experts ∈ {2, 4} per F14
  M-Pre-3  d_model = 64, d_ff = 128, n_blocks = 4 per F14
  M-Pre-4  Construction goes through ModelTopologyConfig::from_profile
           (per planv0 amendment item 1; raw constructor forbidden in
           new code paths).

Postconditions:
  M-Ok-1   Every block in ffn_path_per_block carries FfnPathConfig::MoE
           with the same n_experts.
  M-Ok-2   The router config is LowRankRouter with rank = 4 (D7 override).
  M-Ok-3   The expert config is two-matrix; expert.no_glu = true.
           Construction MUST reject GLU experts (bd-2c8z).
  M-Ok-4   Sequence semantics is LinearState multi-timescale (S5).
  M-Ok-5   Norm is AffineClipLut (S2 default).
  M-Ok-6   Embeddings are tied (charset_v1 default; vocab=80).
  M-Ok-7   moe_tiny_topology_hash deterministically derives from
           canonical JSON of the topology config (no host clock,
           no path).
```

## 6.2 MoeTinyDenseMatched topology contract

```text
operation s7_construct_moe_tiny_dense_matched
  input:  ModelTopologyConfig (MoeTiny) AND
          DenseMatchedBytesPolicy
  output: ModelTopologyConfig (MoeTinyDenseMatched)

Preconditions:
  D-Pre-1  Input MoeTiny config validated by §6.1.
  D-Pre-2  DenseMatchedBytesPolicy carries the D6 formula version,
           the F-A4 ternary metadata constants, and the tolerance
           (±10% or 4 banks).

Postconditions:
  D-Ok-1   d_ff_dense = solve_d6(MoeTiny)
           solved deterministically via integer search over admissible
           d_ff values, picking the d_ff that minimizes:

             abs(B_deployed_total(Dense[d_ff]) - B_deployed_total(MoE))

           subject to the D6 tolerance.

           Tie-break:
             prefer the candidate with B_deployed_total(Dense) >=
             B_deployed_total(MoE); if still tied, prefer smaller d_ff.
  D-Ok-2   |B_deployed_total(MoE) - B_deployed_total(Dense)| <= D6 tolerance
           (tolerance check at construction; failure aborts construction).
  D-Ok-3   ffn_path_per_block = [Dense, Dense, Dense, Dense]
  D-Ok-4   No router config; routing structures are absent at the type
           level.
  D-Ok-5   All other dims (d_model, n_blocks, sequence, norm, vocab,
           embeddings) match MoeTiny exactly.
  D-Ok-6   moe_tiny_dense_matched_topology_hash derives from canonical
           JSON of the resulting config; pinned in matched_bytes.json
           alongside d_ff_dense.

Failure modes:
  D-Fail-1  No admissible d_ff_dense exists in [d_model, 4096]
            (i.e. dense FFN cannot be sized within tolerance).
            ⇒ MatchedBytesInfeasible; aborts before T2.
            Reported as a halt; bd-2v9r blocked.
  D-Fail-2  D6 tolerance fails after solve.
            ⇒ MatchedBytesToleranceViolation; aborts before T2.

Pinned values (canonical S7 instance):
  d_ff_dense    is resolved by O11's standalone CI test against the
                pinned F-A4 metadata and bias policy; prose estimates
                are forbidden per D6.
  B_experts_total      resolved at first commit; pinned in matched_bytes.json
  B_dense_ffn_total    resolved at first commit; pinned in matched_bytes.json
  B_deployed_total_moe / B_deployed_total_dense within D6 tolerance.
```

## 6.3 Training scaffold parity contract

```text
operation s7_assert_scaffold_parity
  input:  ScaffoldFingerprint(MoE_run), ScaffoldFingerprint(dense_run)
  output: ScaffoldParityVerdict

ScaffoldFingerprint :=
  {
    optimizer_config_hash:       Hash256
    phase_schedule_hash:         Hash256
    rng_kind:                    "Pcg64Mcg"
    device_profile_hash:         Hash256
    corpus_train_sha:            Hash256
    corpus_val_sha:              Hash256
    charset_v1_sha:              Hash256
    bpc_chunk_size:              256
    sequence_length:             256
    batch_size:                  32
    optimizer_steps:             20000
    eval_every_steps:            1000
    eval_subset_size:            4096
    burn_pinned_version:         String
    dependency_lockfile_sha:     Hash256
    rust_toolchain_hash:         Hash256
    build_config_hash:           Hash256
    pass_version:                SemVer
  }

Postcondition:
  ScaffoldFingerprint(MoE) = ScaffoldFingerprint(dense)
  with the SOLE exception of: model_topology_hash, router_config_hash,
  expert_block_config_hash (these are necessarily different).

  Any other mismatch ⇒ ScaffoldParityViolation; aborts both runs before
  optimizer step 1 with non-zero exit.
```

## 6.4 Falsification probes (topology side)

```text
F0-topology   MoeTiny constructed via raw constructor (bypassing
              from_profile)            ⇒ S7 setup error; aborts at
                                          construction.
F1-topology   GLU expert configured     ⇒ bd-2c8z rejection at construction.
F2-topology   d_ff_dense unscaled       ⇒ matched-bytes formula not honored;
                                          §16 O5 falsification F2-bytes-
                                          unscaled fires; H3 Refuted.
F3-topology   Top-2 routing             ⇒ rejected at construction; D3
                                          forbids; §16 O5 falsification
                                          F1-router-top-k-ge-2 fires;
                                          H1 Refuted.
F7-topology   smoothness_window = 1     ⇒ rejected at construction; D10
                                          forbids; §16 O5 falsification
                                          F7-window-one fires; H5 Refuted.
```

## 6.5 Routed FFN oracle fixture

```text
RoutedFfnOracleFixture (per H9 / D18):
  prompt:        a fixed UTF-8 byte sequence of length 64 chars (charset_v1)
  expected:
    at least one token whose argmax-route differs across two MoE layers
    at least one token whose argmax-route differs from the previous token
      in some layer (i.e. exercises bank_switches_per_token > 0)
    at least one token whose argmax-route stays the same as the previous
      token in some layer (i.e. exercises same_expert_rate > 0)

  The fixture prompt is pinned in
    fixtures/oracle/s7_routed_ffn_prompt.bin
  with a sha256 in fixtures/oracle/s7_routed_ffn_manifest.toml.

Pre-condition for H9:
  RouteCoverage(prompt, MoeTiny artifact) satisfies all three
  requirements above, ELSE H9 is NotEvaluatedDueToPriorGate("oracle
  fixture lacks route coverage") and S7 emits a Halt(audit-fixture).
```

---

# 7. Router contract

## 7.1 Top1RouterQat dispatch

```text
operation s7_top1_dispatch
  input:  raw_router_logits: Tensor[batch, seq, n_experts]
          training: bool
  output: dispatch_indicator: Tensor[batch, seq, n_experts]   ; one-hot, stop-gradient
          routing_probs:      Tensor[batch, seq, n_experts]   ; softmax(effective logits)
          raw_router_logits:  Tensor[batch, seq, n_experts]   ; for z-loss
          effective_logits:   Tensor[batch, seq, n_experts]   ; for p and argmax

Forward semantics:
  if training:
    effective_logits = raw_router_logits + jitter(JitterSubRng, jitter_stddev)
  else:
    effective_logits = raw_router_logits
  routing_probs   = softmax(effective_logits)
  argmax_e        = argmax(effective_logits, axis=-1)
  dispatch_indicator = one_hot(argmax_e, n_experts)
                       (stop-gradient by construction; tape disabled)

Gradient provenance:
  ∂ routing_probs / ∂ raw_router_logits       : autodiff through softmax (alive)
  ∂ dispatch_indicator / ∂ raw_router_logits  : EXACTLY ZERO (stop-gradient)
  ∂ jitter / ∂ raw_router_logits              : ZERO (jitter is sample, not parameter)
  ∂ jitter / ∂ jitter_stddev                  : ZERO at eval; alive at training only
                                                if jitter_stddev is a learnable
                                                parameter (S7: it is NOT; it is
                                                a fixed scalar config)

Determinism:
  In training mode, jitter is reproducible from (seed, step, layer_id)
  per D14. Eval/export draws no Rng samples.

Top-2 explicitly forbidden (D3):
  Construction asserts n_top1_routes = 1; any other value is rejected
  with a compile-time error at config validation.
```

## 7.2 LowRankRouter

Per §3.2. Forward semantics, parameter count, and centered z-loss
baseline are pinned in §3.2.

## 7.3 Temporal smoothness regularization

```text
operation s7_temporal_smoothness
  input:  routing_probs: Tensor[batch, seq, n_layers, n_experts]
          sequence_mask: Tensor[batch, seq]      ; 1 = valid token, 0 = padding/boundary
          smoothness_window: SmoothnessWindow    ; pinned 32 (D10)
  output: L_switch_router: Tensor scalar (per-batch reduction declared below)

Reduction semantics (per CLAUDE.md "logits reduction" bullet):
  Class/expert axis: n_experts                       ; inner product
  Layer axis: n_layers                                ; SUMMED then divided by n_layers
  Token axis t: n_token_pairs (within window, masked); SUMMED then divided by
                                                     n_token_pairs (effective)
  Batch axis b: BATCH-MEAN reduction (per CLAUDE.md: name whether remaining
                                       axes are summed or averaged)

  L_switch_router(b) = (1 / (n_layers * n_pairs(b))) *
                         sum_l sum_{(t,u) in pairs(b)}
                           (1 - <p_{b,t,l}, p_{b,u,l}>)
  L_switch_router    = mean_b L_switch_router(b)

Where pairs(b) =
  {
    (t, u) :
      t in [1, seq),
      u in [max(0, t - smoothness_window), t),
      sequence_mask[b, v] = 1 for every v in [u, t],
      no explicit boundary-before marker occurs at any v in [u + 1, t]
        (a marker at t starts a fresh sequence and excludes (t, u))
  }

Pair-count sanity check:
  For an all-valid sequence with no boundaries, this range gives
    n_pairs = sum_{t=1}^{seq-1} min(t, smoothness_window).
  When seq > smoothness_window, this is
    seq * smoothness_window
      - smoothness_window * (smoothness_window + 1) / 2.
  Therefore D9's seq=256 and D10's smoothness_window=32 give
    n_pairs = 256*32 - 32*33/2 = 7664.
  The alternative 256*32 - 32*(32-1)/2 would count one extra
  full-window current-token slot not present in the u < t pair set.

Window semantics:
  - smoothness_window = 1 is forbidden by D10; construction rejects it.
  - smoothness_window = 32 (D10 default) means t can pair with any t' in
    [t - 32, t - 1] within the same window; a streaming implementation
    may realize this by holding distributions for the last 32 valid tokens.
  - At a sequence boundary (an invalid sequence_mask gap or an explicit
    boundary-before marker), the candidate window resets to empty; the next
    valid token starts a fresh window.

Gradient provenance (per H7):
  ∂ L_switch_router / ∂ routing_probs[b, t, l, *] : alive (autodiff through inner product)
  ∂ L_switch_router / ∂ routing_probs[b, u, l, *] : alive for every
                                                     u in the valid window
  ∂ L_switch_router / ∂ <anything across sequence boundary> : ZERO (mask)
  ∂ L_switch_router / ∂ <expert parameters> : ZERO at this loss term alone
                                                (expert parameters reached
                                                 via lm_loss / distill, not L_switch)
  The lines above are the formula-level provenance. The executable
  Burn/autodiff proof that every valid u receives nonzero gradient remains
  the O13 / bd-1kkf gradient-assertion owner.

Finite-value guard (per CLAUDE.md "burn loss helpers must validate
finite values before returning"):
  The helper validates L_switch_router is finite AFTER tensor math but
  BEFORE returning. The helper validates n_layers > 0, smoothness_window
  >= 2 BEFORE tensor math (scalar config validation per CLAUDE.md
  "validate scalar config/shape before tensor math").
```

## 7.4 Expert dropout

```text
operation s7_expert_dropout
  input:  expert_outputs: Tensor[batch, seq, n_experts, d_model]
          dropout_rate:   f32 in [0, 1)
          training:       bool
          seed:           Seed
          step:           TrainStep
          layer_id:       LayerId
  output: dropped_outputs: Tensor[batch, seq, n_experts, d_model]
          dropout_mask:    Tensor[batch, n_experts]    ; per (batch, expert)

Forward semantics:
  if not training: return expert_outputs (no draws)
  rng = DropoutSubRng(seed, step) XOR layer_id
  for each (b, e):
    dropout_mask[b, e] = bernoulli(rng, 1 - dropout_rate)
  dropped_outputs = expert_outputs * dropout_mask  (broadcast over seq, d_model)

Phase-effective dropout_rate (per bd-1oc):
  Phase A: 0.0   ; teacher warmup; no dropout
  Phase B: 0.1   ; router warmup
  Phase C: 0.1   ; expert ternary QAT
  Phase D: 0.05  ; full numeric QAT (slight reduction)
  Phase E: 0.0   ; HardenAndSelect; no dropout (consistent with eval)

Determinism (per CLAUDE.md "Dropout RNG state is not persistent — seed
from step number for reproducibility"):
  Reconstructable from (seed, step, layer_id); no persistent state.
```

## 7.5 Gaussian jitter

```text
operation s7_router_jitter
  input:  raw_router_logits: Tensor[batch, seq, n_experts]
          jitter_stddev:  f32 >= 0
          training:       bool
          seed:           Seed
          step:           TrainStep
          layer_id:       LayerId
  output: effective_logits: Tensor[batch, seq, n_experts]

Forward semantics:
  if not training or jitter_stddev = 0:
    return raw_router_logits
  rng = JitterSubRng(seed, step, layer_id)
  jitter = gaussian(rng, mean=0, stddev=jitter_stddev,
                    shape=[batch, seq, n_experts])
  return raw_router_logits + jitter

Phase-effective jitter_stddev:
  Phase A: 0.0
  Phase B: 0.5    ; router exploration
  Phase C: 0.3
  Phase D: 0.1
  Phase E: 0.0

Determinism: per D14 reconstructable from (seed, step, layer_id).
```

---

# 8. Loss composition contract

## 8.1 Composed loss

```text
L_total(step, batch) =
    lm_loss(student_logits, target)
  + lambda_distill_eff(step) * logit_distillation_loss(student_logits, teacher_logits)
  + lambda_balance_eff(step) * expert_load_balance_loss(routing_probs, dispatch_indicator)
  + lambda_zrouter_eff(step) * router_z_loss(raw_router_logits)
  + lambda_switch_eff(step)  * temporal_switch_penalty(routing_probs, sequence_mask)

where lambda_*_eff(step) is the phase-effective lambda per D5
(0 in Phase A; on per per-term Phase boundary).
```

## 8.2 Per-term raw vs weighted helpers

Per CLAUDE.md "Keep raw weighted-loss helpers honest" and "Loss config
helpers must distinguish raw TOML config from phase-effective config":

```text
RawLossDiagnostics :=
  {
    lm_loss_raw:                    LossNatsPerByte
    distill_loss_raw:               DistillRawDiagnostic
    balance_loss_raw:               f32       ; finite, >= 0
    zrouter_loss_raw:               f32       ; finite, >= 0; zero baseline by D5
    switch_loss_raw:                f32       ; finite, in [0, 1]
    diagnostics_self_hash:          Hash256
  }

DistillRawDiagnostic :=
    NotAvailable { reason: "no_frozen_teacher", phase: TrainPhase }
  | Value { loss: LossNatsPerByte }

WeightedLossContribution :=
  {
    lm_contribution:                LossNatsPerByte                ; lm_loss_raw * 1.0
    distill_contribution:           LossNatsPerByte                ; distill_loss_raw * lambda_distill_eff
    balance_contribution:           f32
    zrouter_contribution:           f32
    switch_contribution:            f32
    contribution_self_hash:         Hash256
  }

Helper invariants (per CLAUDE.md):
  RH-1   Each raw helper validates finiteness AFTER tensor math BEFORE
         returning, regardless of whether lambda_eff = 0.
  RH-2   Each raw helper validates scalar config/shape BEFORE tensor math
         (e.g. n_experts > 0, smoothness_window >= 2).
  RH-3   Raw helpers do NOT host-copy the entire differentiable tensor
         for validation; they validate the resulting scalar loss only.
  RH-4   If a raw helper intentionally skips computation in some phase
         (e.g. Phase A skips distill because no teacher exists yet),
         the helper is named a *contribution* helper, not a raw helper,
         and the omission is explicit. Skipping in a raw helper is
         a contract violation.
  RH-5   No implicit zero default for missing raw entries; if a raw
         entry is omitted, RawLossDiagnostics deserialization fails.
         (Per CLAUDE.md "Do not give raw per-term diagnostic collections
         an implicit all-zero default".)
         Distillation before the Phase A teacher-freeze boundary MUST be
         represented as DistillRawDiagnostic::NotAvailable, never as 0.0.

Phase-effective config helper distinction:
  RawTomlLossConfig    -> the values pinned in train_config.toml
  PhaseEffectiveLossConfig(step) -> RawTomlLossConfig with lambdas
                                     gated to 0 outside the per-term
                                     Phase window per D5.
  These are DISTINCT types in gbf-train; they are not interchangeable.
  Construction asserts the rule: every PhaseEffectiveLossConfig value
  is the corresponding raw value or 0; no other value is admissible.
```

## 8.3 Per-term contracts

### 8.3.1 lm_loss

```text
lm_loss = (1 / (B * T)) * sum_{b, t} -log_softmax(student_logits[b, t])[target[b, t]]

Reduction:
  Class axis: vocab=80 (charset_v1)        ; log_softmax inner reduction
  Token axis t: SUM
  Batch axis b: SUM
  Final divide once by (B * T)             ; equivalent to mean

Gradient provenance:
  gradient reaches student_logits and through them selected expert
    parameters, embeddings, sequence-state, and norm.
  Does NOT reach LowRankRouter parameters through dispatch, because
    dispatch_indicator is stop-gradient by D3.
```

### 8.3.2 logit_distillation_loss

```text
distill_loss = KL(softmax(teacher_logits / T_d) || softmax(student_logits / T_d))
T_d = 1.0  ; pinned distillation temperature for S7

Reduction:
  Class axis: vocab=80                     ; softmax inner reduction
  Token axis t: SUM then mean over (B * T)
  Batch axis b: SUM then mean over (B * T)

Gradient provenance:
  gradient reaches student_logits and through them selected expert
    parameters, embeddings, sequence-state, and norm.
  Does NOT reach LowRankRouter parameters through dispatch, because
    dispatch_indicator is stop-gradient by D3.
  ∂ distill_loss / ∂ teacher_logits         : ZERO (teacher frozen at end of Phase A)
  ∂ distill_loss / ∂ teacher_parameters     : ZERO (teacher frozen)

Phase-effective:
  lambda_distill_eff = lambda_distill (1.0) * indicator(step in Phase C+)
  In Phase A, B: lambda_distill_eff = 0; raw helper still computes raw value
                                          (will be 0 because no teacher yet,
                                          but the helper may output a sentinel
                                          when no teacher is loaded; sentinel
                                          must be an explicit Option, not 0).
```

### 8.3.3 expert_load_balance_loss

```text
balance_loss = n_experts * sum_e f_e * P_e
  where:
    f_e = (1/T) sum_t indicator(dispatch_indicator[t, e] = 1)   ; usage fraction
    P_e = (1/T) sum_t routing_probs[t, e]                        ; mean prob
  (standard MoE balance loss; Switch Transformer formulation)

Reduction:
  Class axis: n_experts                     ; sum
  Token axis t: averaged inside f_e and P_e
  Batch axis b: BATCH-MEAN
  Layer axis l: SUMMED then divided by n_layers

Gradient provenance:
  ∂ balance_loss / ∂ routing_probs (P_e term)   : alive (autodiff through softmax)
  ∂ balance_loss / ∂ dispatch_indicator (f_e)   : ZERO (stop-gradient dispatch per D3)
                                                  (per CLAUDE.md "name hard top-1
                                                  assignments as stop-gradient
                                                  dispatch provenance")
  ∂ balance_loss / ∂ expert_parameters          : ZERO at this term alone
  Reach claim per CLAUDE.md "gradient claims must identify whether the proof
  reaches routing probabilities, router logits, or full router parameters":
    Reaches: routing_probs (and through them, effective_logits via softmax,
             and through them, the LowRankRouter parameters)
    Does NOT reach: dispatch_indicator, expert parameters, embeddings,
                    sequence-state, norm.

Phase-effective:
  lambda_balance_eff = lambda_balance (0.01) * indicator(step in Phase B+)
```

### 8.3.4 router_z_loss (centered)

```text
z_loss_centered = (1/B) sum_b (log(sum_e exp(z_{b, e})) - mu)^2
  with mu = log(n_experts)                  ; pinned baseline (D5; §3.2)

Properties:
  When all z = 0, log(sum_e exp(0)) = log(n_experts) = mu, so
  z_loss_centered = 0 within f64 tolerance 1e-12. This is the
  "centered z-loss with zero baseline" claim falsifiable by F-z-uncentered
  (§16 O5).

Reduction:
  Class axis: n_experts                     ; logsumexp inner reduction
  Token axis t: averaged BEFORE the squared term
  Batch axis b: BATCH-MEAN

Gradient provenance:
  ∂ z_loss / ∂ raw_router_logits z          : alive (autodiff through logsumexp)
  ∂ z_loss / ∂ LowRankRouter parameters     : alive ONLY through z; no other path
                                              (per H7 falsification rule)

Phase-effective:
  lambda_zrouter_eff = lambda_zrouter (1e-3) * indicator(step in Phase B+)
```

### 8.3.5 temporal_switch_penalty (L_switch)

Per §7.3. Differentiable through routing_probs as declared. Gradient
provenance asserted by H7. Sequence boundary handling per the
sequence_mask discipline.

Phase-effective:
  lambda_switch_eff = lambda_switch (0.05) * indicator(step in Phase B+)

```text
Note on swept-vs-production confusion (per CLAUDE.md "router z-loss,
name the zero point/baseline ... and distinguish training lambda_zrouter
losses from QAT/router aux-loss proxies"):
  S7's lambda_switch is an UNAMBIGUOUS training loss weight on a
  differentiable temporal switch penalty. It is NOT a QAT proxy
  loss. The same scalar appears in:
    - production training (D5; pinned at 0.05)
    - the swept lambda_switch grid (D11; pinned values 0.0..5.0)
  Both consume the same code path; only the scalar value differs.
  This is documented in s7_router_collapse_sweep.v1.
```

---

# 9. Switch statistics export contract

## 9.1 Producer collection

```text
operation s7_collect_switch_stats
  input:  trained MoeTiny artifact (production lambda_switch)
          val_eval_subset:    Vec[byte] (Gutenberg val prefix)
  output: SwitchStatsBundle

SwitchStatsBundle :=
  {
    schema_version:                "s7_switch_stats.v1"
    seed:                          Seed
    artifact_path:                 String
    temporal_switch_digest:        Vec[TemporalSwitchDigest]   ; one per layer
    clip_saturation_digest:        Vec[ClipSaturationDigest]   ; one per layer
    expert_payload_digest:         Vec[ExpertPayloadDigest]    ; one per layer
    expert_slot_affinity:          Vec[ExpertSlotAffinity]     ; one per layer
    bundle_self_hash:              Hash256
  }

Preconditions:
  E-Pre-1  artifact loaded from a frontier-selected MoE checkpoint at
           production lambda_switch.
  E-Pre-2  val_eval_subset.sha256 matches manifest pin.
  E-Pre-3  artifact is in eval mode (no RouterRng draws per D14).

Postconditions:
  E-Ok-1  Each digest list has length n_blocks = 4.
  E-Ok-2  All TSD-, CSD-, EPD- invariants from §3.3-3.5 hold.
  E-Ok-3  Aggregate-distribution invariants validated in BOTH
          construction and deserialization (per CLAUDE.md
          "distribution-like vectors must validate aggregate
          invariants in constructors and deserialization").
  E-Ok-4  bundle_self_hash deterministic from canonical JSON of all
          digests with self-hashes omitted.
  E-Ok-5  ExpertSlotAffinity entries are canonicalized (lo <= hi)
          BEFORE hashing (per CLAUDE.md unordered-pair bullet).
```

## 9.2 Per-step producer telemetry

```text
operation s7_emit_router_step_telemetry
  input:  raw_router_logits, effective_logits, routing_probs, dispatch_indicator
          training_step, seed, layer_id
  output: RouterStepTelemetry (subscriber-captured)

Emitter contract:
  - Emitted under structured tracing event "s7.router.step" at INFO level.
  - One event per (training_step, layer_id).
  - v0.2 schema/helper home: gbf_experiments::s7::schema. This helper
    is the O12 subscriber-proof surface; production training-loop
    adoption or a later artifact-schema re-export must be claimed by
    its owner bead, not inferred from the helper alone.
  - The tracing event carries flat subscriber fields for the D19 scalar
    checks plus telemetry_canonical_json so the captured event can be
    deserialized as RouterStepTelemetry and self-hash verified.
  - Subscriber-level capture is the proof obligation per CLAUDE.md
    logging-bead bullet ("subscriber-level capture for event shape").
  - Real dashboard / report adoption is owned by F-C4
    (named owner per CLAUDE.md).

Test obligation (per H5):
  Test that captures emitted events at subscriber level and asserts:
    - exactly n_blocks events per training_step
    - all five RST-* invariants hold
    - all five D19 metrics present in each event
```

## 9.3 lambda_switch sweep telemetry

```text
operation s7_emit_lambda_switch_sweep
  input:  trained checkpoint (seed=0, end of Phase D)
          lambda_switch_grid: Vec[LambdaSwitch]    ; D11 grid
          val_eval_subset:    Vec[byte]
  output: SweepRecord per grid point

For each lambda in grid:
  - load checkpoint
  - re-train for 1000 additional steps with lambda_switch = lambda
    (other lambdas held at production values)
  - score bpc on val_eval_subset
  - record expert_usage_entropy_bits_mean across all layers
  - emit LambdaSwitchSweepStep event
  - persist as one entry in s7_router_collapse_sweep.v1

If a swept run diverges:
  - record completion = DivergedAt(step)
  - bpc_eval_subset = null
  - expert_usage_entropy_bits_mean = last finite observed value
  - GuardrailVerdict = InconclusiveDiverged unless the last finite route
    telemetry independently satisfies the high-lambda collapse criteria.

Cadence:
  Sweep runs once at the END of training (after Phase D).
  Per-step in-loop sweeps are NOT performed (too expensive).
  The sweep produces exactly |grid| = 6 records, one per grid point.
  Each record is taken from the same base checkpoint that has been
  trained for an additional 1000 steps at the swept lambda. The
  "1000 steps" delta pins the cost; it is NOT an in-loop cadence.
```

---

# 10. Router collapse guardrail contract

```text
operation s7_router_collapse_guardrail
  input:  SweepRecords for D11 grid
          production_lambda: 0.05
          collapse_threshold: 1.0
  output: GuardrailVerdict

GuardrailVerdict :=
    Pass
  | FailA(reason: "production lambda regresses bpc by > 0.05")
  | FailB(reason: "production lambda's entropy < 0.85 * log2(n_experts)")
  | FailC(reason: "high-lambda 5.0 entropy drop < 0.3 bits")
  | FailD(reason: "high-lambda 5.0 bpc rise < 0.3")
  | InconclusiveDiverged(lambda_switch, step)

Decision:
  bpc_baseline       = sweep[lambda_switch=0.0].bpc_eval_subset
  bpc_production     = sweep[lambda_switch=0.05].bpc_eval_subset
  ent_production     = sweep[lambda_switch=0.05].expert_usage_entropy_bits_mean
  bpc_high           = sweep[lambda_switch=5.0].bpc_eval_subset
  ent_high           = sweep[lambda_switch=5.0].expert_usage_entropy_bits_mean
  log2_n_experts     = log2(4) = 2.0

  if any non-5.0 sweep point completion = DivergedAt(step):          InconclusiveDiverged(lambda, step)
  elif bpc_production - bpc_baseline > 0.05:                         FailA
  elif ent_production < 0.85 * log2_n_experts (= 1.7):               FailB
  elif sweep[lambda_switch=5.0].completion = DivergedAt(step):
       if (ent_production - ent_high) >= 0.3:                        Pass
       else:                                                         InconclusiveDiverged(5.0, step)
  elif (ent_production - ent_high) < 0.3:                             FailC
  elif (bpc_high - bpc_production) < 0.3:                             FailD
  else:                                                              Pass

Closure rule:
  Pass                   ⇒ H6 Confirmed
  FailA|B|C|D            ⇒ H6 Refuted; S7Outcome = Fail-router-collapse-guardrail
  InconclusiveDiverged   ⇒ H6 Refuted; S7Outcome = Fail-router-collapse-guardrail
```

---

# 11. Matched-bytes parity gate + Pareto dominance contract

## 11.1 Per-seed parity check

```text
operation s7_parity_seed
  input:  production_moe_score: S7ScoreReport
          production_dense_matched_score: S7ScoreReport
          ; from experiments/S7/scores/{topology}/seed-{seed}/score.json
          ; (§13.2 production-run score artifacts, not sweep-local records)
          margin: 0.05
  output: ParityVerdict in {Pass, Fail}

Decision:
  bpc_moe_seed_s = production_moe_score.bpc
  bpc_dense_matched_seed_s = production_dense_matched_score.bpc
  if bpc_moe_seed_s < bpc_dense_matched_seed_s - margin: Pass
  else:                                                   Fail
```

## 11.2 Aggregate parity verdict

```text
operation s7_parity_aggregate
  input:  ParityVerdict per seed; bytes_diff
  output: AggregateParityVerdict in {Pass-clean, Fail-parity, Fail-bytes}

Decision:
  if |bytes_diff| > D6 tolerance:                Fail-bytes
  elif ∀ seed s. ParityVerdict(s) = Pass:        Pass-clean
  else:                                          Fail-parity
```

## 11.3 Pareto dominance

```text
operation s7_pareto_verdict
  input:  CheckpointFrontierPoint(MoE), CheckpointFrontierPoint(dense_matched)
          (each carries quality.median_val_bpc and projected_fit.deployed_bytes_total)
  output: ParetoVerdict in {
    MoE-dominates,
    dense-dominates,
    MoE-wins-under-byte-equivalence,
    Dense-wins-under-byte-equivalence,
    Incomparable,
    Tied
  }

Decision (per D13):
  bpc_moe   = MoE.quality.median_val_bpc
  bpc_dense = dense_matched.quality.median_val_bpc
  by_moe    = MoE.projected_fit.deployed_bytes_total
  by_dense  = dense_matched.projected_fit.deployed_bytes_total

  bytes_equivalent := abs(by_moe - by_dense) <= D6_tolerance
  bpc_le_moe       := bpc_moe   <= bpc_dense
  bpc_le_dense     := bpc_dense <= bpc_moe
  by_le_moe        := by_moe   <= by_dense
  by_le_dense      := by_dense <= by_moe

  if bpc_le_moe and by_le_moe and (bpc_moe < bpc_dense or by_moe < by_dense):
    MoE-dominates
  elif bpc_le_dense and by_le_dense and (bpc_dense < bpc_moe or by_dense < by_moe):
    dense-dominates
  elif bpc_moe == bpc_dense and by_moe == by_dense:
    Tied
  elif bytes_equivalent and bpc_moe < bpc_dense:
    MoE-wins-under-byte-equivalence
  elif bytes_equivalent and bpc_dense < bpc_moe:
    Dense-wins-under-byte-equivalence
  else:
    Incomparable

Closure rule:
  MoE-dominates                       ⇒ H4 Confirmed
  MoE-wins-under-byte-equivalence     ⇒ H4 Confirmed
                                        (matched-bytes is the scientific
                                         claim; a strict-bpc win under D6
                                         byte-equivalence satisfies H4
                                         per the "matched bytes" framing)
  dense-dominates                     ⇒ H4 Refuted; H3 also Refuted (parity gate
                                         inconsistency is impossible if dense
                                         dominates the median);
                                         S7Outcome = Fail-parity
  Dense-wins-under-byte-equivalence   ⇒ H4 Refuted; H3 also Refuted (dense beats
                                         MoE on bpc under byte-equivalence);
                                         S7Outcome = Fail-parity
  Tied                                ⇒ H4 Refuted (no strict inequality);
                                         S7Outcome = Fail-pareto
  Incomparable                        ⇒ H4 Refuted; S7Outcome = Fail-pareto
```

---

# 12. Outcome algebra

```text
S7Outcome :=
    Pass-clean                       ; H1 ∧ H2 ∧ H3 ∧ H4 ∧ H5 ∧ H6 ∧ H7 ∧ H8 ∧ H9 ∧ H10
  | Fail-moe-train                   ; H1 Refuted (substrate or loss decrease)
  | Fail-router-collapse             ; H1 Refuted via D16 collapse halt at production lambda
  | Fail-dense-baseline              ; H2 Refuted
  | Fail-parity                      ; H3 Refuted (per-seed parity miss)
  | Fail-bytes                       ; matched-deployed-bytes tolerance violated
  | Fail-pareto                      ; H4 Refuted (incomparable or tied)
  | Fail-switch-stats                ; H5 Refuted
  | Fail-router-collapse-guardrail   ; H6 Refuted
  | Fail-grad-provenance             ; H7 Refuted
  | Fail-burn-grad                   ; H8 Refuted
  | Fail-oracle-routed               ; H9 Refuted
  | Fail-emulator-routed             ; H10 Refuted
  | Fail-suspicious                  ; median(MoE val_bpc) < 0.5
```

Combination (mandatory checks first):

```text
if ∃ seed s. completion(MoE, s) = DivergedAt(_)            ⇒ Fail-moe-train
elif ∃ seed s. completion(MoE, s) = CollapsedAt(_)         ⇒ Fail-router-collapse
elif H1 verdict = Refuted (non-collapse)                   ⇒ Fail-moe-train
elif ∃ seed s. completion(dense, s) = DivergedAt(_)        ⇒ Fail-dense-baseline
elif H2 verdict = Refuted                                  ⇒ Fail-dense-baseline
elif H7 verdict = Refuted                                  ⇒ Fail-grad-provenance
elif H8 verdict = Refuted                                  ⇒ Fail-burn-grad
elif H5 verdict = Refuted                                  ⇒ Fail-switch-stats
elif H6 verdict = Refuted                                  ⇒ Fail-router-collapse-guardrail
elif median(MoE val_bpc) < 0.5                             ⇒ Fail-suspicious
elif aggregate_parity_verdict = Fail-bytes                 ⇒ Fail-bytes
elif H3 verdict = Refuted                                  ⇒ Fail-parity
elif H4 verdict = Refuted                                  ⇒ Fail-pareto
elif H9 verdict = Refuted                                  ⇒ Fail-oracle-routed
elif H10 verdict = Refuted                                 ⇒ Fail-emulator-routed
else                                                       ⇒ Pass-clean
```

Decision dispatch:

```text
Pass-clean                          → Decision::ProceedToS8
Fail-parity                         → Decision::ProceedToS8-DenseOnly
                                      (per D17; successful scientific
                                       falsification of the MoE-wins claim;
                                       does NOT block bd-2v9r closure)
Fail-bytes                          → Decision::Halt(matched-bytes-invalid;
                                                     comparison-not-scientific)
Fail-pareto                         → Decision::Investigate(pareto-incomparable;
                                                           inspect bytes
                                                           accounting and per-seed
                                                           variance)
Fail-moe-train                      → Decision::Investigate(burn-or-loss-substrate)
Fail-router-collapse                → Decision::Investigate(reduce-lambda-switch-or-tune-dropout)
Fail-router-collapse-guardrail      → Decision::Investigate(sweep-grid-or-thresholds)
Fail-dense-baseline                 → Decision::Investigate(dense-topology-constructor)
Fail-switch-stats                   → Decision::Halt(export-schema-broken)
Fail-grad-provenance                → Decision::Halt(loss-math-dishonest)
Fail-burn-grad                      → Decision::Halt(burn-adapter-broken)
Fail-suspicious                     → Decision::Halt(audit-split-and-bpc)
Fail-oracle-routed                  → Decision::Halt(oracle-cannot-resolve-routed-FFN)
Fail-emulator-routed                → Decision::Halt(routed-encoded-rom-broken)
```

`Halt` blocks bd-2v9r closure unconditionally. `Investigate` creates a
follow-up bead and may extend this RFC's scope. `ProceedToS8-DenseOnly`
is the unique non-clean closure variant: it closes bd-2v9r AND amends
S8's epic to inherit a dense-only production track per D17.

---

# 13. Artifact schemas

All s7_*.v1 artifacts use S1CanonicalJson (UTF-8, sorted keys, no
insignificant whitespace, finite floats encoded by shortest round-trip
decimal, -0.0 normalized to 0.0). All carry a *_self_hash field
computed via DomainHash with the self-hash field omitted.

## 13.1 s7_run_log.v1

```text
Path:
  experiments/S7/runs/{topology}/seed-{seed}/run-log.json
  experiments/S7/runs/{topology}/seed-{seed}/grad-log.jsonl
  experiments/S7/runs/{topology}/seed-{seed}/router-step-telemetry.jsonl

RunLog (JSON) :=
  {
    schema:                  "s7_run_log.v1"
    seed:                    Seed
    topology:                "MoeTiny" | "MoeTinyDenseMatched"
    train_config_hash:       Hash256
    model_topology_hash:     Hash256
    router_config_hash:      Hash256          ; null for dense
    expert_block_config_hash: Hash256          ; null for dense
    loss_config_hash:        Hash256
    phase_schedule_hash:     Hash256
    frozen_teacher_checkpoint_sha:
                              Null | Hash256        ; null only before the
                                                     ; Phase A boundary in an
                                                     ; in-progress log;
                                                     ; otherwise the
                                                     ; same-topology,
                                                     ; same-seed Phase A teacher
    losses:                  List[(TrainStep, RawLossDiagnostics)]
    grad_norms:              List[(TrainStep, GradNormSummary)]
    eval_points:             List[(EvalStep, BpcValue)]
    final_grad_norms:        GradNormSummary
    completion:              Completed | DivergedAt(TrainStep) | CollapsedAt(TrainStep)
    run_log_self_hash:       Hash256
  }

Invariants:
  RL-Length     If completion = Completed:
                  losses.length = train_config.optimizer_steps = 20000
                Else:
                  losses.length = last_completed_step
  RL-Eval       If completion = Completed:
                  eval_points.length = optimizer_steps / eval_every_steps + 1 = 21
                Else:
                  eval_points.length = number of eval points reached before
                  DivergedAt/CollapsedAt
  RL-Finite     every recorded value is finite (else completion = DivergedAt
                                                  or CollapsedAt)
  RL-Teacher    after the Phase A boundary, frozen_teacher_checkpoint_sha is
                non-null, scoped to this exact (topology, seed), and is the
                only teacher hash Phases C/D/E may use for distillation
  RL-Topology   topology field matches the model_topology_hash exactly.
  RL-Router-Null
                For topology = "MoeTinyDenseMatched", router_config_hash and
                expert_block_config_hash MUST be JSON null (not omitted).
```

## 13.2 s7_score.v1

```text
Path:
  experiments/S7/scores/{topology}/seed-{seed}/score.json

ScoreReport (JSON) :=
  {
    schema:               "s7_score.v1"
    seed:                 Seed
    topology:             "MoeTiny" | "MoeTinyDenseMatched"
    checkpoint_sha:       Hash256
    corpus_val_sha:       Hash256
    chunk_size:           256
    token_count:          u64
    log2_sum:             f64
    bpc:                  BpcValue
    score_self_hash:      Hash256
  }

Invariants:
  S-Bpc      bpc = log2_sum / token_count
  S-Tokens   token_count = length(charset_v1_encode(normalize(val_bytes)))
  S-Det      score is deterministic per (checkpoint, val_bytes).
```

## 13.3 s7_burn_grad_smoke.v1

```text
Path:
  experiments/S7/burn-grad-smoke/expert_block_qat.json

BurnGradSmokeReport (JSON) :=
  {
    schema:                       "s7_burn_grad_smoke.v1"
    fixture_seed:                 0xFEED
    burn_adapter_version:         String
    fixture_input_sha:            Hash256
    grad_up_weight_sum_abs:       f64
    grad_down_weight_sum_abs:     f64
    supported_clipped_activation_count: u64   ; must be 3
    learned_activation_range_unsupported: Bool
    projection_biases_unsupported: Bool
    glu_construction_rejected:    Bool
    replay_byte_identical:        Bool
    smoke_self_hash:              Hash256
  }

Invariants:
  BG-Finite           every grad_*_sum_abs is finite
  BG-Nonzero          every required grad_*_sum_abs > 0
                      (per H8 declared reach set)
  BG-Activations      supported_clipped_activation_count = 3
  BG-RangeContract    learned_activation_range_unsupported = true
  BG-BiasContract     projection_biases_unsupported = true
  BG-Glu              glu_construction_rejected = true
  BG-Replay           replay_byte_identical = true

Closure citation (per CLAUDE.md):
  cargo test -p gbf-train --features burn-adapter -- expert_block_qat_grad
```

## 13.4 s7_switch_stats.v1

```text
Path:
  experiments/S7/switch-stats/seed-{seed}/switch-stats.json

SwitchStatsReport (JSON) :=
  {
    schema:                       "s7_switch_stats.v1"
    seed:                         Seed
    artifact_path:                String
    temporal_switch_digest:       Vec[TemporalSwitchDigest]   ; len = 4
    clip_saturation_digest:       Vec[ClipSaturationDigest]   ; len = 4
    expert_payload_digest:        Vec[ExpertPayloadDigest]    ; len = 4
    expert_slot_affinity:         Vec[ExpertSlotAffinity]     ; len = 4
    aggregation_rule:             "SUM"                        ; per §3.3.1
    bundle_self_hash:             Hash256
  }

Invariants:
  All TSD-, CSD-, EPD- invariants from §3.3-3.5.
  SS-Layers     all four lists have length n_blocks = 4.
  SS-LayerLocal Every digest carries layer_id; ExpertId values are
                layer-local. (CLAUDE.md export-fact bullet enforced
                in deserialization.)
  SS-Pinned-Json
                Public JSON shape pinned by an explicit serde_json::json!
                assertion, NOT by serde round-trip alone (per CLAUDE.md
                "Public artifact JSON shape tests should pin downstream
                field names with explicit serde_json::json! assertions").
```

## 13.5 s7_router_collapse_sweep.v1

```text
Path:
  experiments/S7/router-collapse/seed-0/sweep.json

RouterCollapseSweepReport (JSON) :=
  {
    schema:                          "s7_router_collapse_sweep.v1"
    seed:                            0
    base_checkpoint_sha:             Hash256          ; end of Phase D
    producer_kind:                   "production_closure_retrain_score"
                                                       ; not fixture
    grid:                            Vec[LambdaSwitch]   ; pinned by D11
    records:                         Vec[LambdaSwitchSweepStep]
    production_lambda:               LambdaSwitch       ; 0.05 (D5)
    collapse_threshold:              LambdaSwitch       ; 1.0 (D11)
    guardrail_verdict:               GuardrailVerdict   ; per §10
    sweep_self_hash:                 Hash256
  }

Invariants:
  RCS-Grid      grid = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0]    ; exact (D11)
  RCS-Producer  producer_kind = "production_closure_retrain_score";
                deterministic_fixture is invalid for bd-2v9r closure.
  RCS-Records   records.length = grid.length
  RCS-Cadence   each record's training-extra step delta = 1000 (D11/§9.3)
  RCS-Diverged  bpc_eval_subset may be null iff record completion = DivergedAt(_)
  RCS-Verdict   guardrail_verdict deterministically derived from records
                per §10 decision table.
```

## 13.6 s7_dense_vs_moe.v1

```text
Path:
  experiments/S7/dense-vs-moe/comparison.json

DenseVsMoeComparisonReport (JSON) :=
  {
    schema:                          "s7_dense_vs_moe.v1"
    moe_topology_hash:               Hash256
    dense_matched_topology_hash:     Hash256
    matched_bytes_pin:               MatchedBytesPin
    per_seed:                        Vec[PerSeedComparison]    ; len = 5
    median_val_bpc_moe:              BpcValue
    median_val_bpc_dense:            BpcValue
    deployed_bytes_total_moe:        u64
    deployed_bytes_total_dense:      u64
    bytes_diff:                      i64
    bytes_within_tolerance:          Bool
    aggregate_parity_verdict:        AggregateParityVerdict     ; §11.2
    pareto_verdict:                  ParetoVerdict              ; §11.3
    switch_stats_summary:            SwitchStatsSummary
    sweep_summary:                   SweepSummary
    comparison_self_hash:            Hash256
  }

PerSeedComparison :=
  {
    seed:                  Seed
    val_bpc_moe:           BpcValue
    val_bpc_dense:         BpcValue
    delta:                 f64                         ; val_bpc_dense - val_bpc_moe
    parity_verdict:        ParityVerdict               ; per §11.1
  }

MatchedBytesPin :=
  {
    formula_version:                  SemVer
    d_ff_dense_resolved:              u16
    bias_policy:                      String
    b_experts_total:                  u64
    b_router_overhead_total:          u64
    b_dense_ffn_total:                u64
    b_deployed_total_moe:             u64
    b_deployed_total_dense:           u64
    tolerance_bytes:                  u64
    matched_bytes_self_hash:          Hash256
  }

SwitchStatsSummary :=
  {
    same_expert_rate_per_layer_q8_8:  Vec[u16]            ; len = 4
    expert_usage_entropy_bits_mean:   f32
    bank_switches_per_token_mean:     f32
  }

SweepSummary :=
  {
    bpc_at_lambda:        Map[LambdaSwitch, BpcValue]
    entropy_at_lambda:    Map[LambdaSwitch, f32]
    guardrail_verdict:    GuardrailVerdict
  }

Invariants:
  DvM-PerSeed        per_seed.length = 5
  DvM-Bytes          bytes_within_tolerance ⟺ |bytes_diff| <= D6 tolerance
  DvM-Aggregate      aggregate_parity_verdict deterministically derived
                     from per_seed and bytes_within_tolerance per §11.2
  DvM-Pareto         pareto_verdict deterministically derived from
                     median_val_bpc_* and deployed_bytes_total_* per §11.3
  DvM-Self-Hash      comparison_self_hash deterministic and round-trip stable.
  DvM-Pinned-Json    Public JSON shape pinned by explicit json! assertions
                     for downstream consumer field names (F-C4 conformance,
                     v0_success envelope's downstream-MoE gate per bd-12b9).
```

## 13.7 s7_frontier.v1

Inherits S5's s5_frontier.v1 schema with a topology axis:

```text
Path:
  experiments/S7/frontier/frontier.json

FrontierReport (JSON) :=
  {
    schema:                "s7_frontier.v1"
    points:                Vec[FrontierPoint]    ; one MoE + one dense, both at production lambda
    pareto_verdict:        ParetoVerdict
    frontier_self_hash:    Hash256
  }

FrontierPoint :=
  {
    topology:              "MoeTiny" | "MoeTinyDenseMatched"
    checkpoint_sha:        Hash256
    quality:               { median_val_bpc: BpcValue, per_seed_val_bpc: Vec[BpcValue] }
    conformance:           ConformanceSummary
    projected_fit:         { deployed_bytes_total: u64, deployed_bytes_per_block: Vec[u64] }
    schedule_cost:         Option[EstimatedCostDelta]
  }
```

## 13.8 s7_oracle_routed.v1

```text
Path:
  experiments/S7/oracle-routed/seed-0/oracle.json

OracleRoutedReport (JSON) :=
  {
    schema:                       "s7_oracle_routed.v1"
    seed:                         0
    topology:                     "MoeTiny"
    fixture_prompt_sha:           Hash256
    train_logits_sha:             Hash256
    bundle_logits_sha:            Hash256
    artifact_logits_sha:          Hash256
    frozen_teacher_checkpoint_sha: Hash256      ; equals
                                                ; RunLog.frozen_teacher_checkpoint_sha
                                                ; for this (topology, seed)
    pairwise_max_abs_diff_train_bundle:  f64
    pairwise_max_abs_diff_bundle_artifact: f64
    pairwise_max_abs_diff_train_artifact: f64
    s3_tolerance:                 f64                  ; pinned by S3 RFC
    route_coverage:               RouteCoverage
    weight_quant_resolution:      "QuantSpec::weight_quant"   ; per CLAUDE.md
    oracle_self_hash:             Hash256
  }

RouteCoverage :=
  {
    cross_layer_route_difference:    Bool
    consecutive_token_route_change:  Bool
    consecutive_token_route_same:    Bool
  }

Invariants:
  OR-Coverage    All three RouteCoverage fields = true (D18 / §6.5).
  OR-Tolerance   pairwise diffs <= S3 tolerance per H9.
  OR-WeightResolve weight_quant_resolution = "QuantSpec::weight_quant" exactly
                  (per CLAUDE.md oracle bullet).
```

## 13.9 s7_emulator_one_token.v1

```text
Path:
  experiments/S7/emulator-one-token/seed-0/{topology}/result.json

EmulatorOneTokenReport (JSON) :=
  {
    schema:                          "s7_emulator_one_token.v1"
    seed:                            0
    topology:                        "MoeTiny" | "MoeTinyDenseMatched"
    encoded_rom_sha:                 Hash256
    prompt_sha:                      Hash256
    artifact_oracle_logits_sha:      Hash256
    emulator_logits_sha:             Hash256
    pairwise_max_abs_diff:           f64
    s5_tolerance:                    f64
    observed_bank_switches_per_token: f32
    oracle_recorded_bank_switches:    f32
    bank_switch_diff:                 f32        ; |observed - recorded|
    bank_switch_within_one:           Bool       ; <= 1 per H10
    emulator_self_hash:              Hash256
  }

Invariants:
  EO-Tolerance       pairwise diff <= S5 (Pick and Fit) tolerance.
  EO-Switch          bank_switch_diff <= 1 (D17 / H10 prefix correction).
  EO-Topology        For dense topology, bank_switches accounting is
                     trivially 0; the field is recorded but the
                     bank_switch_within_one assertion is N/A (forced true).
```

## 13.10 s7_report.v1

```text
Path:
  docs/experiments/S7-report.md

Front-matter (YAML, hashed into report):
  ---
  schema:                "s7_report.v1"
  s7_outcome:            S7Outcome
  decision:              Decision
  matched_bytes_self_hash:        Hash256
  per_seed_artifacts:
    List[{
      seed: Seed,
      topology: "MoeTiny" | "MoeTinyDenseMatched",
      completion: Completed | DivergedAt(TrainStep) | CollapsedAt(TrainStep) | NotReached,
      checkpoint_self_hash: Null | Hash256,
      run_log_self_hash:    Null | Hash256,
      score_self_hash:      Null | Hash256
    }]
  switch_stats_self_hash:         Null | Hash256
  router_collapse_sweep_self_hash: Null | Hash256
  dense_vs_moe_self_hash:         Null | Hash256
  frontier_self_hash:             Null | Hash256
  burn_grad_smoke_self_hash:      Null | Hash256
  oracle_routed_self_hash:        Null | Hash256
  emulator_one_token_moe_self_hash:   Null | Hash256
  emulator_one_token_dense_self_hash: Null | Hash256
  generated_at:          RFC3339 UTC, informational only, excluded from report hash.
  rfc_revision:          GitCommitId | Hash256
  predictions_section_hash: Hash256
  predictions_commit:    GitCommitId
  first_result_commit:   GitCommitId
  report_self_hash:      Hash256
  ---

Required sections (markdown body):
  ## Pre-registered predictions
    Predicted ranges and pass criteria as committed before any training run.
    Must appear in git history strictly before the first S7 result artifact
    commit (per O1).

  ## Observed (per-seed, per-topology table)
    val_bpc, completion, parity_verdict, deployed_bytes_total per (seed, topology).

  ## Hypothesis verdicts
    H1..H10 each as HypothesisStatus, with the concrete observation that
    drove each verdict. Closure-candidate reports MUST use only
    Confirmed | Refuted; early-failure reports may use
    NotEvaluatedDueToPriorGate(reason).

  ## Falsification analysis
    Direct citation of which prediction or falsification rule fired for
    each Refuted hypothesis. For Fail-parity, the per-seed bpc table is
    cited explicitly.

  ## Switch statistics summary
    Per-layer same_expert_rate, expert_usage_entropy_bits_mean, bank_
    switches_per_token_mean. Cite s7_switch_stats.v1.

  ## lambda_switch sweep summary
    Cite s7_router_collapse_sweep.v1; show grid and verdict.

  ## Pareto verdict
    MoE vs dense_matched on (median val_bpc, deployed bytes total).

  ## Surprises
    Anything outside predicted ranges, even if not a verdict change.

  ## Decision
    Exactly one Decision tag, justified in <= 3 sentences.

  ## Reproducibility statement
    Exact command + manifest hashes + pass_version to replay.

Invariants:
  R-Decision         Exactly one Decision tag in front-matter.
  R-AllSeeds         per_seed_artifacts covers 10 entries
                     (5 seeds × 2 topologies).
  R-ClosureArtifacts For Decision ∈ {ProceedToS8, ProceedToS8-DenseOnly},
                     all artifact self-hashes are non-null.
  R-Self-Hash        report_self_hash computed over front-matter
                     (with generated_at and report_self_hash omitted)
                     plus markdown body bytes exactly as committed.
  R-Predictions      predictions_commit is a strict ancestor of
                     first_result_commit; first_result_commit is the
                     earliest commit introducing any S7-derived
                     self-hash.
  R-AllHypotheses    All ten hypotheses have an explicit HypothesisStatus.
                     Closure-candidate verdicts must be binary.
```

---

# 14. Reproducibility laws

S7 inherits S1 Rep-1..Rep-8 unchanged and adds the following S7-specific
extensions.

```text
Rep-S7-1 Per-(topology, seed) determinism
  ∀ topology t, seed s. replay(t, s, manifest) is byte-identical to
  original(t, s, manifest), under same Burn version + same dependency
  lockfile + S7CpuDeterministic device profile.

Rep-S7-2 Per-(topology, seed) router determinism
  For topology = MoeTiny, ∀ seed s. RouterRng(seed) draws are
  deterministic per (seed, step, layer_id) per D14. Replay of the same
  (seed, step) produces bit-identical:
    - dropout masks
    - jitter samples
    - routing logits, routing probs, dispatch indicator

Rep-S7-3 Cross-topology scaffold parity
  ScaffoldFingerprint(MoE) and ScaffoldFingerprint(dense_matched) differ
  only in the explicitly-permitted fields per §6.3. Any other
  difference is a contract violation that aborts both runs.

Rep-S7-4 Sweep determinism
  s7_router_collapse_sweep.v1 replays byte-identically given the same
  base_checkpoint_sha + same grid + same RouterRng seed.

Rep-S7-5 Bytes parity is reproducible
  matched_bytes_pin.{d_ff_dense_resolved, b_experts_total,
  b_dense_ffn_total, tolerance_bytes} are deterministic functions of
  the matched_bytes_formula_version + the F-A4 ternary metadata
  constants. They MUST not depend on host clock, network, or any
  non-pinned input.

Rep-S7-6 Switch stats are reproducible
  s7_switch_stats.v1 byte-identical under replay given the same
  artifact + same val_eval_subset.

Rep-S7-7 Pareto verdict is total
  Given two FrontierPoints, ParetoVerdict is a total function (no
  ambiguity). No floating-point tiebreak: equality compares f64 bit
  patterns of canonical-JSON-encoded values.

Rep-S7-8 Pre-registration carries through (extends S1 O1)
  The "Pre-registered predictions" section of S7-report.md must appear
  in git history strictly before the first S7 result artifact commit,
  including dense_matched runs and switch stats and sweep records.
```

---

# 15. Decision protocol

```text
S7 closure (bd-2v9r) requires:
  1. All 5 seeds × 2 topologies = 10 runs Completed (or, for the dense
     topology under Fail-parity outcome, all 5 dense Completed plus all
     5 MoE Completed; the parity verdict drives the closure variant,
     not the run completion).
  2. s7_report.v1 emitted with R-Predictions verified by git history.
  3. Decision ∈ {ProceedToS8, ProceedToS8-DenseOnly}.
  4. matched_bytes_self_hash recorded; |bytes_diff| within D6 tolerance.
  5. s7_switch_stats.v1 emitted with all four-layer digests passing
     §3.3-3.5 invariants. (Mandatory regardless of outcome variant
     because H5 is a closure gate.)
  6. s7_router_collapse_sweep.v1 emitted with guardrail_verdict = Pass.
     (Mandatory regardless of outcome variant because H6 is a closure
     gate.)
  7. s7_burn_grad_smoke.v1 emitted with all H8 invariants satisfied.
     (Mandatory; H8 closure gate.)
  8. s7_oracle_routed.v1 emitted with all H9 invariants satisfied.
     (Mandatory; H9 closure gate.)
  9. s7_emulator_one_token.v1 (MoE, seed 0) emitted with all H10
     invariants satisfied. For Decision = ProceedToS8-DenseOnly, the
     dense emulator one-token harness is also required.
 10. Loss gradient provenance suite (H7) passed with explicit
     gradient assertions for each declared "reaches" and "does not
     reach" set.

S7 closure is forbidden when:
  Any of:
    Decision::Halt(_), Decision::Investigate(_),
    missing pre-registration,
    any seed completion = DivergedAt(_),
    any seed completion = CollapsedAt(_) (for MoE),
    any required artifact missing or self-hash invalid,
    matched_bytes outside tolerance,
    H5 / H6 / H7 / H8 / H9 / H10 Refuted (these are unconditional gates).

Closes:
  bd-2v9r           Slice S7 closure
  bd-do2j (F13)     Dense Baseline Track
  bd-19u  (F7)      Router Switch-Awareness
  bd-2ky  (F5)      Honest Loss Function — partial:
                       T5.1 (L_switch) closes here
                       T5.5 (lambda_shape / lambda_overflow) defers to S8
                    F5 itself remains open until T5.5 closes at S8.

Adds blocking edge for S8 (bd-218w):
  If Decision = ProceedToS8-DenseOnly:
    S8 inherits a dense-only production track per D17.
    The S8 RFC must explicitly amend its scope to drop MoE.
  Else (Decision = ProceedToS8):
    S8 retains both MoE and dense baselines per the original epic plan.
```

---

# 16. Proof obligations

```text
O1  Pre-registration provability (extends S1 O1)
    "Pre-registered predictions" section of S7-report.md must appear
    in git history strictly before any S7 result artifact commit.
    CI script asserts:
      1. predictions_section_hash matches the exact normalized markdown
         section in predictions_commit;
      2. predictions_commit is a strict ancestor of first_result_commit;
      3. first_result_commit is the earliest commit that introduces any
         S7-derived self-hash (run, score, switch_stats, sweep, dense_vs_moe,
         frontier, oracle, emulator, burn_grad_smoke, report).

O2  Determinism (extends S1 O2 / Rep-S7-1)
    Same (topology, seed) + same scaffold pinning ⇒ bit-identical
    safetensors AND bit-identical s7_*.v1 artifacts.
    v1 CI closure test:
      run (MoeTiny, seed 0) twice; assert byte equality.
      run (MoeTinyDenseMatched, seed 0) twice; assert byte equality.
      run swept lambda at lambda=0.05 twice; assert byte equality.

O3  Switch stats schema correctness
    All TSD-, CSD-, EPD-, RST-, LSS- invariants enforced in BOTH
    construction and deserialization. Round-trip tests for every
    digest type. Public JSON shape pinned by explicit serde_json::json!
    assertions per CLAUDE.md export-fact bullet.

O4  Burn adapter gradient smoke (H8)
    cargo test -p gbf-train --features burn-adapter -- expert_block_qat_grad
    Reports the number of tests run; does NOT claim red-before-green
    unless the pre-patch check was actually run (per CLAUDE.md
    "When a filtered test target is introduced by the patch").

O5  Falsification suite
    Nine deliberately-broken implementations must each produce the
    expected Refuted verdict on the corresponding hypothesis:

      F1-router-top-k-ge-2:
        Top-2 routing silently constructs                  → H1 Refuted
        (D3 forbids top-k >= 2)
      F2-bytes-unscaled:
        Dense matched-bytes uses MoE's d_ff (=128)
        instead of d_ff_dense                              → H3 Refuted
        (gate fires on accidentally-too-small dense)
      F3-pareto-unequal-bytes:
        Pareto verdict ignores tolerance and compares
        unequal byte budgets                                → H4 Refuted
      F4-switch-grad-router-only:
        L_switch backward path stops at routing_probs and
        does NOT extend to LowRankRouter parameters         → H7 Refuted
        (L_switch must reach the router parameters via
         the routing_probs softmax chain)
      F5-z-uncentered:
        z-loss is implemented uncentered when D5 declares
        centered (mu = log(n_experts))                       → H7 Refuted
        (centered baseline = 0 fixture fails)
      F6-balance-no-stop-grad:
        balance_loss back-propagates through dispatch
        indicator (no stop-gradient)                         → H7 Refuted
        (gradient leaks to expert parameters via dispatch)
      F7-window-one:
        smoothness_window = 1 silently constructs            → H5 Refuted
        (D10 / §6.4 forbids as too weak for the S7 claim)
      F8-sweep-constant-lambda:
        lambda_switch sweep grid contains only
        the production value (no actual sweep)               → H6 Refuted
        (FailC fires: high-lambda entropy drop = 0)
      F9-expert-block-qat-grad-dead:
        Burn adapter for ExpertBlockQat returns zero
        gradients into the up.weight tensor                  → H8 Refuted

    Required test files:
      gbf-experiments/tests/falsification/f1_router_top_k_ge_2.rs
      gbf-experiments/tests/falsification/f2_bytes_unscaled.rs
      gbf-experiments/tests/falsification/f3_pareto_unequal_bytes.rs
      gbf-experiments/tests/falsification/f4_switch_grad_router_only.rs
      gbf-experiments/tests/falsification/f5_z_uncentered.rs
      gbf-experiments/tests/falsification/f6_balance_no_stop_grad.rs
      gbf-experiments/tests/falsification/f7_window_one.rs
      gbf-experiments/tests/falsification/f8_sweep_constant_lambda.rs
      gbf-experiments/tests/falsification/f9_expert_block_qat_grad_dead.rs
    Gated by the test-only `falsify` feature on gbf-experiments so
    broken substitutes cannot leak into a release build.

O6  Hash round-trip
    Every emitted s7_*.v1 artifact round-trips through canonical JSON
    with self-hash equality. Aggregate-distribution invariants
    (TSD-2, EPD-1) are enforced in BOTH construction and
    deserialization paths.

O7  Outcome algebra totality
    Every observable combination of binary H1..H10 verdicts,
    per-seed-per-topology completion states, suspicion thresholds,
    bytes parity, parity verdict, and pareto verdict maps to exactly
    one S7Outcome variant under §12.

O8  No hidden inputs (extends S1 O8)
    s7 artifacts depend only on:
      corpus_train, corpus_val (Gutenberg, sha256-pinned per S4)
      charset_v1 manifest (sha256-pinned per S3)
      MoeTiny + MoeTinyDenseMatched topology configs
        (deterministically resolved from F14 profile registry)
      train_config (D9 pinned)
      loss_config (D5 pinned)
      router_config (D7 pinned)
      phase_schedule (D4 pinned)
      lambda_switch_sweep_grid (D11 pinned)
      seeds [0, 1, 2, 3, 4]
      pass_version
      gbf-train pinned dependency set
    No env-var, no host-clock, no network, no stdin.

O9  Per-(topology, seed) isolation (extends S1 O9)
    The 10 runs (5 seeds × 2 topologies) are independent. No shared
    mutable state. CI smoke checks:
      1. (MoeTiny, seed 0) and (MoeTiny, seed 1) produce different
         final_checkpoint_sha;
      2. (MoeTiny, seed 0) and (MoeTinyDenseMatched, seed 0) produce
         different final_checkpoint_sha (different topology);
      3. running ((MoeTiny, [0,1])) and ((MoeTiny, [1,0])) produces the
         same per-seed hashes.

O10 Closure gate
    bd-2v9r close is reachable iff Decision ∈ {ProceedToS8,
    ProceedToS8-DenseOnly}.

O11 Matched-bytes formula CI
    A standalone CI test (independent of any training run) computes
    d_ff_dense from the canonical MoeTiny instance and asserts:
      d_ff_dense_resolved = the value pinned in matched_bytes.json
      |b_deployed_total_moe - b_deployed_total_dense| <= D6 tolerance
      formula_version matches matched_bytes_pin.formula_version
    Any drift fails CI before any training begins.

O12 Standard producer telemetry coverage
    A subscriber-level test (per CLAUDE.md logging-bead bullet) captures
    "s7.router.step" events and asserts:
      - exactly n_blocks events per training_step
      - all five D19 metrics present in each event
      - none missing; no sentinel zeros
    Real dashboard / report adoption is named to F-C4 (out of S7 scope).

O13 Loss gradient provenance assertions (H7)
    A fixture test computes gradients on a tiny batch and asserts:
      - every declared "reaches" relation has nonzero gradient
      - every "does NOT reach" relation has exactly-zero gradient
      - lm_loss and distill_loss have exactly-zero gradient on
        LowRankRouter parameters under hard top-1 stop-gradient dispatch
      - centered z-loss baseline: f64 absolute value <= 1e-12 when
        all router logits = 0

O14 Aggregate distribution invariants in deserialization
    Deserialization tests for TemporalSwitchDigest, ClipSaturationDigest,
    and ExpertPayloadDigest reject malformed JSON whose per-entry fields
    are individually valid but whose aggregate violates invariants
    (e.g. transition_mass entries summing to > 256). Per CLAUDE.md
    "distribution-like vectors must validate aggregate invariants in
    constructors and deserialization".

O15 Unordered pair canonicalization
    Tests for ExpertSlotAffinity construction and deserialization
    canonicalize unordered pairs (lo <= hi) BEFORE deriving equality
    or hashing. Per CLAUDE.md "Unordered artifact hint pairs must
    canonicalize their stored representation in constructors and
    deserialization".
```

---

# 17. Minimal end-to-end theorem

```text
Theorem S7Soundness:

Given:
  Gutenberg manifest with valid sha256 (S4)
  charset_v1 manifest (S3)
  MoeTiny + MoeTinyDenseMatched ModelSizeProfile reference instances
    (F14 / bd-rq46; resolved via from_profile)
  matched_bytes.json with d_ff_dense_resolved within D6 tolerance
  TrainConfig pinned per D9
  RouterConfig pinned per D7
  ExpertBlockConfig pinned per D5/D8 (no GLU; clipped activation)
  Phase schedule pinned per D4
  LossConfig pinned per D5 (with phase-effective gating)
  lambda_switch sweep grid pinned per D11
  pass_version V_S7 fixed by gbf-train HEAD at S7 PR merge

If for every (topology t, seed s) ∈ {MoeTiny, MoeTinyDenseMatched} × {0,1,2,3,4}:
  s7_train_run(t, s)        returns Completed RunProduct
                            (no DivergedAt, no CollapsedAt for MoeTiny)
  s7_score_bpc(t, s)        returns finite val_bpc
And for seed 0 specifically:
  s7_router_collapse_sweep returns SweepReport with guardrail_verdict = Pass
  s7_switch_stats          returns valid SwitchStatsBundle for MoeTiny
  s7_oracle_routed         returns OracleReport with all H9 invariants
  s7_emulator_one_token    returns reports for both topologies satisfying H10
And:
  s7_burn_grad_smoke       returns BurnGradSmokeReport with all H8 invariants
  Loss gradient provenance suite (O13) passes for all H7 declarations
  s7_dense_vs_moe          returns DenseVsMoeComparisonReport with
                            aggregate_parity_verdict and pareto_verdict
                            deterministically derived
  s7_report.v1             contains pre-registered predictions in pre-run
                            git history

Then:
  Each of H1, H2, H3, H4, H5, H6, H7, H8, H9, H10 has a defined verdict
  in {Confirmed, Refuted}.

  S7Outcome is exactly one of:
    Pass-clean
    Fail-moe-train       (H1 Refuted)
    Fail-router-collapse (H1 Refuted via D16)
    Fail-dense-baseline  (H2 Refuted)
    Fail-grad-provenance (H7 Refuted)
    Fail-burn-grad       (H8 Refuted)
    Fail-switch-stats    (H5 Refuted)
    Fail-router-collapse-guardrail  (H6 Refuted)
    Fail-suspicious      (median MoE bpc < 0.5)
    Fail-bytes           (matched-deployed-bytes invalid)
    Fail-parity          (H3 Refuted)
    Fail-pareto          (H4 Refuted)
    Fail-oracle-routed   (H9 Refuted)
    Fail-emulator-routed (H10 Refuted)

  Decision is unique under the dispatch rule of §12.

  If S7Outcome = Pass-clean, S7 has produced these verified knowledge
  claims:
    – MoeTiny trains end-to-end through Phase A->E on Gutenberg without
      router collapse, for all five seeds.
    – MoeTinyDenseMatched trains end-to-end on Gutenberg, for all five
      seeds, under the SAME training scaffold.
    – At equal deployed bytes (within D6 tolerance), MoE beats
      dense by > 0.05 bpc on Gutenberg val, for every seed.
    – On the (val_bpc, deployed_bytes_total) Pareto plane, MoE
      dominates dense.
    – Switch statistics export schemas are correct, complete, and
      LayerId-scoped.
    – The lambda_switch sweep guardrail demonstrates non-collapse at
      production lambda AND demonstrable collapse at lambda = 5.0.
    – Each loss term's gradient provenance matches its declared reach
      set, in particular L_switch reaches routing_probs (and via
      softmax, the LowRankRouter parameters), balance_loss does NOT
      reach the dispatch indicator (stop-gradient holds), and z-loss
      is centered with mu = log(n_experts) baseline 0.
    – The Burn ExpertBlockQat adapter produces deterministic, finite,
      nonzero gradients into all required parameter sets.
    – ArtifactOracle three-way agreement holds on the routed FFN.
    – EncodedRom + emulator one-token harness preserves on the MoE
      artifact.

  If S7Outcome = Fail-parity, S7 has produced this verified knowledge
  claim:
    – At MoeTiny size on Gutenberg under the inherited training
      scaffold, sparse top-1 MoE does NOT beat a matched-deployed-bytes
      dense baseline by 0.05 bpc per-seed. The MoE-wins hypothesis is
      falsified at this scale and corpus. S8 inherits a dense-only
      production track per D17.

  If S7Outcome = Fail-pareto, S7 has produced this verified knowledge
  claim:
    – At MoeTiny size on Gutenberg, MoE and dense_matched are Pareto-
      incomparable on (val_bpc, deployed_bytes_total). The strict
      dominance claim is falsified. Investigate bytes accounting
      and per-seed variance before re-running.

Not proven:
  UpperBankCandidate (d_model=128) production-scale viability
    on Gutenberg                          (S8)
  StructuredWidthGates supernet           (S8)
  lambda_shape / lambda_overflow          (T5.5; S8)
  Top-2 routing                           (forbidden in S7; experimental)
  Three-matrix GLU experts                (forbidden in S7; bd-2c8z rejection)
  Multi-token emulator harness            (S8)
  Real-corpus deployment                  (S8 / production beads)
```

---

# 18. Implementation crate layout

S7 hosts new modules in `gbf-experiments::s7::*` together with
contributions to existing crates that provide its substrate.

## 18.1 Crate map

```text
gbf-policy
  Required  ModelSizeProfile::MoeTiny reference instance (F14 / bd-rq46).
  Required  ModelSizeProfile::MoeTinyDenseMatched reference instance with
            d_ff_dense_resolved pinned in matched_bytes.json.
  Required  DenseMatchedBytesPolicy with D6 formula version constant and
            tolerance pinning.
  Required  Lock-down: ModelTopologyConfig::from_profile validates dim
            caps and is the ONLY constructor admitted by S7. Raw
            constructor is forbidden in new code paths (planv0
            amendment item 1).
  Notes     d_ff_dense_resolved is computed at first pre-registration
            commit and pinned; it is NOT recomputed at runtime. The
            standalone CI test (O11) verifies the pinned value matches
            the formula.

gbf-model
  Required  Top1RouterQat with stop-gradient hard dispatch (D3) and
            top-k = 1 hardcoded.
  Required  LowRankRouter with router_rank parameter; S7 pins rank = 4
            via configuration; default formula
            `max(1, min(ceil(n_experts/4), 8))`
            preserved for non-S7 callers.
  Required  ExpertBlockQat two-matrix module (bd-x75) with explicit
            GLU rejection at construction (bd-2c8z).
  Required  Temporal smoothness pair-set helper with sequence-mask reset,
            boundary exclusion, and window pinning (D10; bd-2llp/bd-295u).
  Required  Expert dropout (bd-1oc) with phase-effective rates per
            §7.4 and step-derived RNG seeding.
  Required  Gaussian jitter on router logits (bd-1oc) with phase-
            effective stddev per §7.5.
  Required  Switch statistics collection: TemporalSwitchDigest,
            ClipSaturationDigest, ExpertPayloadDigest construction
            with all §3.3-3.5 invariants enforced in constructors;
            ExpertSlotAffinity canonicalization with SUM aggregation
            rule per §3.3.1.
  Required  Per-step RouterStepTelemetry emitter under structured
            tracing event "s7.router.step".

gbf-train
  Required  Burn adapter for ExpertBlockQat (bd-2c8z), behind the
            `burn-adapter` feature gate. Exports the required
            test target `expert_block_qat_grad`.
  Required  Loss composer extension: lambda_distill, lambda_balance,
            lambda_zrouter, lambda_switch.
  Required  RawLossDiagnostics and WeightedLossContribution structs
            with the helper-invariant suite (§8.2 RH-1..RH-5).
  Required  PhaseEffectiveLossConfig distinct from RawTomlLossConfig;
            no implicit zero defaults.
  Required  Phase scheduler extension: gates lambda_balance,
            lambda_zrouter, lambda_switch on Phase B+; gates
            lambda_distill on Phase C+.
  Required  Matched-bytes parity gate runner (bd-2zv4): reads
            CheckpointFrontierPoint pair, computes deployed bytes via
            TernaryWeightPlan::compute_byte_cost, asserts D6 tolerance,
            asserts D12 per-seed margin.
  Required  lambda_switch sweep harness (bd-3sp0): consumes the
            production checkpoint at end of Phase D, re-trains for
            1000 steps at each grid point, emits LambdaSwitchSweepStep
            and computes GuardrailVerdict.
  Required  Router collapse halt: D16 entropy floor check, every step
            in Phase B+, halts with completion = CollapsedAt(step).
  Required  Cargo features: `qat`, `qat-ablation`, `burn-adapter`
            inherited; new `s7-moe`, `s7-dense-matched`,
            `s7-router-collapse-sweep` (test-only) added per §19.

gbf-data
  Required  Gutenberg loader carry-through from S4 (unchanged).

gbf-foundation
  Required  Hash256, sha256, DomainHash carry-through (unchanged).

gbf-artifact
  Required  TemporalSwitchDigest, ClipSaturationDigest, ExpertPayload-
            Digest schemas with LayerId scoping per CLAUDE.md.
  Required  ExpertSlotAffinity unordered-pair canonicalization with
            SUM aggregation rule per §3.3.1; both construction and
            deserialization enforce the canonicalization invariant.
  Required  Aggregate-invariant validation in BOTH construction and
            deserialization paths (TSD-2, EPD-1).

gbf-report
  Required  s7_dense_vs_moe.v1 emitter (bd-12b9). Public JSON shape
            pinned by explicit serde_json::json! assertions for
            downstream F-C4 consumer field names (per CLAUDE.md
            export-fact bullet).

gbf-experiments::s7::*
  Required  Owns Scope(F-S7) end-to-end. Required modules:

    gbf_experiments::s7::manifest
      Gutenberg manifest reader (delegates to gbf-data); charset_v1
      manifest reader (delegates to gbf-data); matched_bytes.json
      reader.

    gbf_experiments::s7::rng
      RouterRng disjoint stream per D14; DropoutSubRng,
      JitterSubRng per §3.7. Reuses S1's seed128 helper.

    gbf_experiments::s7::device_profile
      S7CpuDeterministic = byte-identical clone of S1CpuDeterministic.

    gbf_experiments::s7::run
      s7_train_run operation. Composes:
        - phase scheduler (gbf-train)
        - loss composer (gbf-train)
        - shadow_compile (gbf-train, S5 "Pick and Fit" carry-through)
        - per-step telemetry emission (gbf-model)
        - D16 collapse halt
      Emits s7_run_log.v1.

    gbf_experiments::s7::baseline_match
      Solves d_ff_dense per §6.2; emits matched_bytes.json with
      MatchedBytesPin.

    gbf_experiments::s7::score
      s7_score_bpc operation; uses S5's bpc primitive (vocab=80,
      chunk_size=256).

    gbf_experiments::s7::parity
      s7_parity_seed and s7_parity_aggregate (§11.1, §11.2).

    gbf_experiments::s7::pareto
      s7_pareto_verdict (§11.3). Reads CheckpointFrontierPoints from
      gbf-train's frontier emission.

    gbf_experiments::s7::collapse_sweep
      Drives the lambda_switch sweep at seed 0; emits
      s7_router_collapse_sweep.v1.

    gbf_experiments::s7::switch_stats
      Collects switch stats from a trained MoE artifact; emits
      s7_switch_stats.v1.

    gbf_experiments::s7::oracle_routed
      Runs the routed FFN three-way agreement test per H9; emits
      s7_oracle_routed.v1. Resolves weights via QuantSpec::weight_quant
      per CLAUDE.md oracle bullet.

    gbf_experiments::s7::emulator_one_token
      One-token emulator harness for both MoE and dense topologies
      per H10; emits s7_emulator_one_token.v1 (one per topology).

    gbf_experiments::s7::burn_grad_smoke
      Burn adapter gradient smoke test fixture (bd-2c8z); emits
      s7_burn_grad_smoke.v1.

    gbf_experiments::s7::loss_provenance
      Loss-term gradient provenance assertion suite (H7 / O13).

    gbf_experiments::s7::dense_vs_moe
      Composes per-seed comparison records into
      s7_dense_vs_moe.v1 (delegates JSON shape to gbf-report).

    gbf_experiments::s7::schema
      Type definitions, S7CanonicalJson encoder (re-export of
      S1CanonicalJson), DomainHash function (S1 carry-through), and
      self-hash round-trip helpers for all s7_*.v1 schemas.

    gbf_experiments::s7::report
      s7_report.v1 emitter and outcome-algebra dispatcher implementing
      §12. Authors front-matter, validates R-Decision, R-AllSeeds,
      R-ClosureArtifacts, R-Self-Hash, R-Predictions, R-AllHypotheses,
      and binds the pre-registration commit history per O1.

    gbf_experiments::s7::cli
      Public entrypoint(s) for replay. The CLI surface is the canonical
      invocation point referenced by §14 Rep-S7-1 and §15 closure.

gbf-cli
  Required  Subcommand `gbf s7 ...` dispatching into
            gbf_experiments::s7::cli.
```

## 18.2 Test layout

```text
gbf-experiments/tests/falsification.rs
gbf-experiments/tests/falsification/*.rs
  Root harness plus nine module files required by §16 O5; gated by the
  test-only `falsify` feature.

gbf-experiments/tests/oracle_s7.rs
gbf-experiments/tests/oracle_s7/*.rs
  Routed FFN oracle suite per §6.5 / D18.

gbf-experiments/tests/canonical_json_s7.rs
  Round-trip tests for every s7_*.v1 schema (O6). Each artifact must
  serialize, hash, deserialize, re-serialize, re-hash, and produce
  byte-identical output and self-hash equality.
  Includes aggregate-invariant deserialization tests (O14) and unordered-
  pair canonicalization tests (O15).

gbf-experiments/tests/integration_s7.rs
  End-to-end smoke run on a tiny in-repo fixture corpus (NOT Gutenberg).
  Used in CI to gate determinism (O2) and per-(topology, seed) isolation
  (O9). The full Gutenberg run is gated behind a separate CI job.

gbf-experiments/tests/loss_provenance_s7.rs
  H7 / O13 gradient provenance assertions for all five loss terms.

gbf-experiments/tests/lambda_switch_sweep_smoke.rs
  Tiny-fixture sweep smoke test (gated by feature
  `s7-router-collapse-sweep`) verifying GuardrailVerdict logic.

gbf-experiments/tests/matched_bytes_formula.rs
  Standalone CI test per O11; computes d_ff_dense from the canonical
  MoeTiny instance and asserts it matches matched_bytes.json. Runs
  WITHOUT requiring any training artifact.

gbf-experiments/tests/router_step_telemetry.rs
  Subscriber-level capture test per O12; asserts all five D19 metrics
  are emitted at every step under structured tracing.

gbf-train/tests/expert_block_qat_grad.rs
  Burn-adapter gradient smoke test (H8 / O4); behind the
  `burn-adapter` feature.
```

## 18.3 Artifact paths

All run artifacts under repository-root `experiments/S7/` tree. The
report under `docs/experiments/S7-report.md`. The matched-bytes pin
under `experiments/S7/profile/matched_bytes.json` (committed; pinned
as part of pre-registration).

## 18.4 Canonical replay command

```text
cargo run --release -p gbf-cli --features s7-moe -- s7 replay \
  --gutenberg-manifest fixtures/corpora/gutenberg.toml \
  --charset fixtures/charsets/charset_v1.toml \
  --matched-bytes experiments/S7/profile/matched_bytes.json \
  --pass-version <pass_version_pinned_in_report> \
  --topology MoeTiny \
  --seed-list 0,1,2,3,4 \
  --device-profile S7CpuDeterministic

cargo run --release -p gbf-cli --features s7-dense-matched -- s7 replay \
  --gutenberg-manifest fixtures/corpora/gutenberg.toml \
  --charset fixtures/charsets/charset_v1.toml \
  --matched-bytes experiments/S7/profile/matched_bytes.json \
  --pass-version <pass_version_pinned_in_report> \
  --topology MoeTinyDenseMatched \
  --seed-list 0,1,2,3,4 \
  --device-profile S7CpuDeterministic
```

`scripts/s7_isolation_check.sh --self-test` pins this command shape and
the MoE-then-dense order without running training. Live replay execution
and final end-to-end adoption remain owned by bd-1ryn until the split
S7 replay binaries/features are available.

Optional non-normative subcommands:

```text
gbf s7 verify-matched-bytes  computes d_ff_dense and checks pin
gbf s7 sweep-lambda-switch   runs the D11 sweep at seed 0
gbf s7 burn-grad-smoke       runs the H8 fixture
gbf s7 oracle-routed         runs the H9 oracle suite at seed 0
gbf s7 emulator-one-token    runs the H10 one-token harness
gbf s7 verify-determinism    replays (MoeTiny, seed 0) and asserts byte equality
```

## 18.5 Workspace registration

Cargo.toml workspace `members` already includes `gbf-experiments` (S1).
The crate's `Cargo.toml` declares (at minimum) workspace dependencies
on `gbf-policy`, `gbf-model`, `gbf-train`, `gbf-data`, `gbf-foundation`,
`gbf-artifact`, `gbf-report`, with workspace-pinned versions
(`= ` syntax already enforced workspace-wide per A18).

---

# 19. Build configurations and feature flags

Three build configurations participate in the S7 contract.

## 19.1 S7-build-A — "MoE production run"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments --features s7-moe
Active features (workspace-resolved):
  gbf-experiments/s7-moe
  gbf-experiments/phase-a (inherited)
  gbf-train/qat
  gbf-train/burn-adapter
Behavior:
  Builds the MoeTiny topology run path. Used for all 5 MoE seeds.
Build identity tag (recorded in s7_run_log.v1):
  build_kind = "s7_moe"
```

## 19.2 S7-build-B — "Dense matched-bytes baseline run"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments --features s7-dense-matched
Active features:
  gbf-experiments/s7-dense-matched
  gbf-experiments/phase-a (inherited)
  gbf-train/qat
  gbf-train/burn-adapter
Behavior:
  Builds the MoeTinyDenseMatched topology run path. Phase B is no-op.
  Used for all 5 dense seeds.
Build identity tag (recorded in s7_run_log.v1):
  build_kind = "s7_dense_matched"
```

## 19.3 S7-build-C — "Falsification harness"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --features falsify,s7-router-collapse-sweep
Active features:
  gbf-experiments/falsify
  gbf-experiments/s7-router-collapse-sweep
  gbf-experiments/s7-moe (inherited by sweep)
Behavior:
  Test-only. Compiles the nine deliberately-broken substitutes (§16 O5)
  and the lambda_switch sweep harness with smoke-fixture variants.
  NOT for production use.
```

## 19.4 Feature flag contract

```text
gbf-experiments/s7-moe                      gates the MoeTiny run path
gbf-experiments/s7-dense-matched            gates the dense baseline run path
gbf-experiments/s7-router-collapse-sweep    test-only; gates sweep harness +
                                              fixture-variant LambdaSwitchSweepStep
                                              emitters
gbf-experiments/falsify                     test-only; gates F1..F9 broken
                                              substitutes used by §16 O5
gbf-train/burn-adapter                      gates Burn-backed loss / module paths
                                              (already exists; H8 closure
                                              cites this gate per CLAUDE.md)

Mutual exclusion enforcement:
  gbf-experiments must compile_error! if both `s7-moe` and
  `s7-dense-matched` are enabled simultaneously in a single build, to
  prevent a misconfigured CI from silently building an indeterminate
  binary that runs both topologies in one process and mixes RNG state.

Feature ordering for the canonical replay command:
  cargo build --features s7-moe          THEN
  cargo build --features s7-dense-matched
  Two separate binaries, run in sequence (parallel allowed by isolation
  per O9).
```

## 19.5 Determinism budgets

```text
Both MoE and dense runs use S7CpuDeterministic. The runner sets each
variable in env_exact to its pinned value, and unsets every variable
not present in env_exact, before any tensor allocation:

  BURN_NDARRAY_NUM_THREADS=1
  BURN_DETERMINISTIC=1
  OMP_NUM_THREADS=1
  RAYON_NUM_THREADS=1

S7 adds NO new env_exact entries beyond S1's. RouterRng draws are
in-process; no environment variable controls them.

Violation aborts the run with non-zero exit before training begins.
```

## 19.6 Pre-registration CI

```text
scripts/s7_preregistration_check.sh implements §16 O1:
  1. predictions_section_hash matches the markdown section in
     predictions_commit, recomputed using S1CanonicalJson normalization;
  2. predictions_commit is a strict ancestor of first_result_commit;
  3. first_result_commit is the earliest commit introducing any S7-
     derived self-hash (run, score, switch_stats, sweep, dense_vs_moe,
     frontier, oracle, emulator, burn_grad_smoke, report).
Exit non-zero on any violation. Closure of bd-2v9r is forbidden while
this script exits non-zero.
```

## 19.7 CI gates that block bd-2v9r closure

```text
cargo test -p gbf-experiments --features s7-moe
cargo test -p gbf-experiments --features s7-dense-matched
cargo test -p gbf-experiments --features falsify --test falsification
cargo test -p gbf-experiments --test oracle_s7
cargo test -p gbf-experiments --test canonical_json_s7
cargo test -p gbf-experiments --test integration_s7
cargo test -p gbf-experiments --test loss_provenance_s7
cargo test -p gbf-experiments --features s7-router-collapse-sweep --test lambda_switch_sweep_smoke
cargo test -p gbf-experiments --test matched_bytes_formula
cargo test -p gbf-experiments --test router_step_telemetry
cargo test -p gbf-train --features burn-adapter -- expert_block_qat_grad
cargo build -p gbf-experiments --features s7-moe
cargo build -p gbf-experiments --features s7-dense-matched
scripts/s7_preregistration_check.sh
scripts/s7_determinism_check.sh
  (replays (MoeTiny, seed 0) and (MoeTinyDenseMatched, seed 0); asserts
   byte equality of safetensors and run_log_self_hash; satisfies O2)
scripts/s7_isolation_check.sh
  (asserts O9 conditions across the 10-run matrix)
scripts/s7_matched_bytes_check.sh
  (computes d_ff_dense from canonical formula; asserts equality with
   matched_bytes.json pin; satisfies O11; runs WITHOUT any training)
```

---

# 20. Ambiguity ledger

|  ID | Ambiguity                                                                              | Chosen path                                                                | Clarifying question                                                                | Suggested final decision                                                                                                              |
| --: | -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
|  A1 | n_experts choice (2 vs 4 vs 8)                                                          | n_experts = 4 production; n_experts = 2 ablation lane (D2)                  | Why not 8?                                                                          | F14 registry caps MoeTiny at {2, 4} per planv0 amendment item 1. 8 experts would push expert payload into multiple banks; outside MoeTiny scope. 2 is reported as a sanity floor; 4 is the parity-gate subject. |
|  A2 | Top-1 vs top-2 routing                                                                   | Top-1 only (D3)                                                            | Doesn't top-2 help quality?                                                         | Top-2 doubles bank-switch cost on MBC5 deployment. The parity gate is at MoeTiny size where expert capacity is the bottleneck, not routing capacity. Top-2 is forbidden in S7 and only admissible behind an explicit experimental compile profile (planv0 model-side recs). |
|  A3 | LowRankRouter rank choice                                                                | router_rank = 4 (D7 override; n_experts equality)                          | The default formula gives rank=1; isn't that simpler?                              | Rank=1 is degenerate (single direction in d_model). Saturating rank to n_experts=4 preserves the LowRankRouter as a real factorization while costing only 16 extra params. The implicit-regularization claim is the load-bearing rationale. |
|  A4 | LowRankRouter parameter savings vs full-rank                                              | Slight increase (272 vs 256 params) for rank = n_experts                    | Why call it "low-rank" if it has more params?                                       | Naming is conventional. The LowRankRouter abstraction admits any rank; S7 picks rank = n_experts to saturate while staying inside the same data type. The abstraction is what survives to S8 with larger n_experts where the savings matter. |
|  A5 | Matched-bytes tolerance: ±10% vs strict equality                                          | ±10% with 4-bank absolute slack (D6)                                       | Why not strict equality?                                                            | Integer rounding across four blocks and packed-ternary metadata bytes makes strict equality infeasible without artificial padding. The 10% margin per bd-2zv4 admits realistic d_ff_dense values; the per-seed 0.05 bpc margin (D12) absorbs the residual fairness uncertainty. |
|  A6 | Per-seed strict parity (D12) vs aggregate margin                                         | Per-seed strict, mirroring S1 D6                                            | Aggregate would be more lenient; why not?                                           | Per-seed catches pathological seeds. A median-pass with one-seed-fails could be a fragile MoE win. Per-seed is the honest reading of "MoE beats dense at matched bytes". |
|  A7 | Pareto verdict tie semantics                                                              | Strict dominance required; ties = Refute H4 (D13)                          | What if it's an exact tie on both axes?                                             | A tie on both axes means MoE bought no advantage at matched bytes. It is not a Fail-parity (the per-seed margin may have been met) but it does fail H4. Closure variant: Fail-pareto, not Fail-parity. |
|  A8 | lambda_switch sweep grid choice                                                          | {0.0, 0.05, 0.1, 0.5, 1.0, 5.0} (D11)                                      | Why not log-uniform 0.0..10.0?                                                      | The grid covers four decades centered on the production value (0.05) and includes both 0.0 (no regularization, baseline) and 5.0 (high enough to demonstrably collapse). bd-3sp0 originally specified [0.0, 0.1, 0.5, 1.0, 5.0]; S7 adds 0.05 as the production point. |
|  A9 | Router collapse threshold (lambda_switch_collapse_threshold = 1.0)                         | Pinned at 1.0 (D11)                                                        | Reviewer-tunable?                                                                  | Pinned. The high-lambda guardrail point (5.0) is well above; the production point (0.05) is well below. Tuning would invalidate the H6 falsifiability claim. If 1.0 is wrong, the high-lambda probe (5.0) will demonstrate or fail to demonstrate collapse cleanly. |
| A10 | expert_usage_entropy_bits floor (0.85 * log2(n_experts))                                 | Pinned per D11; production sweep observed 1.7679 bits at lambda_switch=0.05 | What if observed entropy is naturally lower at MoeTiny?                             | Resolved by the bd-2v9r production bundle: the observed mean is above the 1.7-bit floor, so no S7 re-pin is needed. |
| A11 | Centered z-loss baseline (mu = log(n_experts))                                            | Pinned (D5; §3.2)                                                          | Why not running mean?                                                               | Constant mu makes the baseline analytically zero (when all logits are 0). Running mean introduces a hidden statistic that breaks per-step replay determinism. Constant mu is reproducible and falsifiable; F5-z-uncentered tests it directly. |
| A12 | Stop-gradient on dispatch indicator                                                       | Yes (D3); declared explicitly per CLAUDE.md routing/expert-loss bullet      | Wouldn't a straight-through estimator help expert specialization?                   | STE would let balance_loss leak through dispatch into expert parameters, breaking the H7 declared provenance and making the MoE win attributable to a phantom path. Stop-gradient is the honest semantics. STE would be admissible in S8 only if explicitly amended. |
| A13 | Temporal smoothness window = 32                                                           | Pinned (D10; bd-122 default)                                                | Why not 64 or 128?                                                                  | 32 = eighth of sequence_length=256, giving 8 windows per sequence — enough to amortize the boundary cost while still giving the regularizer a meaningful pair set per sequence. Window=1 is mathematically valid as an adjacent-token penalty, but too weak for S7 and rejected at construction; window=128 would over-couple distant tokens. |
| A14 | sequence_length = 256                                                                    | Bumped from S1's 128 (D9)                                                  | Doesn't this break inheritance?                                                     | The bpc reset-context primitive extends naturally; chunk_size matches sequence_length. The bump is justified by the temporal smoothness window's need for ≥ 8 windows per sequence. Documented in Delta-3. |
| A15 | RouterRng disjoint stream                                                                 | New stream per D14                                                         | Couldn't dropout reuse BatchRng?                                                   | Reusing BatchRng would couple dropout sampling to batch indexing, breaking O9 isolation when seeds are run in different orders. RouterRng is disjoint, with sub-streams for dropout/jitter derived from (seed, step, layer_id) for reconstructability. |
| A16 | ExpertId scoping: global vs layer-local                                                  | Layer-local + LayerId carried alongside (per CLAUDE.md export-fact bullet)  | Global is simpler; why layer-local?                                                | At MoeTiny n_experts=4 / n_blocks=4, global ExpertIds (0..15) are technically possible but conflate "the expert in the same slot in different layers". CLAUDE.md mandates layer-local + LayerId. The cost is minimal and the schema is self-describing. |
| A17 | ExpertSlotAffinity aggregation rule                                                      | SUM (a→b + b→a) per §3.3.1                                                | Why not max or directional?                                                         | bd-2pe handoff mandated unordered canonical pairs; SUM is the simplest aggregation that preserves total mass and admits a clean canonicalization. Max would lose information; directional pairs were rejected by bd-2pe. SUM is pinned by S7. |
| A18 | Burn-adapter for ExpertBlockQat: feature-gate or default                                  | Feature-gated (`burn-adapter`); H8 closure cites it                        | Default would simplify CI                                                          | CLAUDE.md training-loss bullet requires the gate citation: "If a loss claim depends on Burn autodiff, closure must cite a feature-enabled gate". The gate also lets gbf-train compile under non-Burn backends in the future. |
| A19 | Phase B no-op semantics for dense run                                                    | Phase B literal pass-through; lambdas gated to 0 (D5)                       | What about distillation?                                                            | Distillation depends on the frozen teacher from Phase A; for the dense run, this is itself a dense teacher. Phase C onward distillation is meaningful (dense student distilling from dense teacher); it matches S1's Phase A semantics minus the LR scheduler. |
| A20 | Frozen-teacher provenance for the dense run                                              | Same Phase A run produces the dense teacher AND seeds the dense student     | Couldn't we share the MoE teacher?                                                  | No. The matched-bytes parity is a TOPOLOGY parity, not a teacher parity. The dense student must be distilled from a dense teacher of the SAME topology so the comparison is over architectural choice alone, not teacher-quality differential. |
| A21 | One-token emulator harness for dense (carry-through)                                     | Required for Decision = ProceedToS8-DenseOnly (§15)                         | If MoE is the subject, why test dense?                                              | Under Fail-parity outcome, the dense path is the production track; the emulator must work on it. The MoE one-token harness is mandatory regardless of outcome variant because H10 is a closure gate; the dense one is conditional. |
| A22 | What if the matched-bytes formula is unsolvable (D-Fail-1)?                              | Halt; bd-2v9r blocked; investigation bead                                  | Could MoE be too small?                                                             | At MoeTiny dimensions the formula has solutions in [d_model, 4096]; D-Fail-1 would only fire on a future profile change. If it fires at S7, the F-A4 metadata constants likely changed; investigate before running. |
| A23 | What if the lambda_switch sweep itself diverges at lambda=5.0?                            | Recorded as DivergedAt; sweep continues at the next grid point             | Doesn't divergence preclude entropy comparison?                                     | A divergent sweep point is logged with completion=DivergedAt and bpc_eval_subset=null. GuardrailVerdict is InconclusiveDiverged unless the last finite route telemetry independently satisfies the high-lambda entropy-collapse criterion; in that recovery case divergence supplies the quality-regression evidence. |
| A24 | Why both H3 and H4 (parity AND Pareto)?                                                  | Both required; H3 is per-seed strict, H4 is median-Pareto                   | Aren't they redundant?                                                              | H3 catches per-seed misses that median-Pareto would average away. H4 catches Pareto-incomparable cases (MoE wins bpc but pays bytes, or vice versa) that H3 alone wouldn't surface. They are complementary closure gates. |
| A25 | Why include H8 (Burn grad smoke) when S2 already proved LinearState gradient flow?       | ExpertBlockQat is a NEW Burn module; bd-2c8z is OPEN                       | Doesn't S2's smoke generalize?                                                      | No. S2 proved gradient flow for LinearState only. ExpertBlockQat's two-matrix expert with clipped activation is structurally distinct (clip threshold, no GLU rejection); it requires its own gradient smoke. CLAUDE.md mandates the burn-adapter feature gate citation. |
| A26 | Why include H7 (loss gradient provenance) when individual loss tests cover each term?    | Provenance is a closure-level invariant; per-term tests cover correctness   | Isn't this duplicative?                                                             | Per-term tests prove the loss VALUE; H7 proves the loss GRADIENT REACH set. CLAUDE.md training-loss bullets require the distinction explicitly: "gradient claims must identify whether the proof reaches routing probabilities, router logits, or full router parameters". |
| A27 | What if Decision = ProceedToS8-DenseOnly and S8 already started planning MoE?            | S7 closure amends S8's epic to drop MoE; planv0 amendment item 2 admits this | Isn't this disruptive?                                                              | Yes, intentionally. The matched-bytes parity gate exists precisely so that the project does not waste S8 capacity on MoE if MoE doesn't earn its bytes. Fail-parity is a successful scientific result, not an implementation failure. |
| A28 | Does S7 prove ROM emission for MoE?                                                       | No; only the one-token harness (H10) is proven                              | What about full-corpus emulation?                                                  | Out of S7 scope. S5 (Pick and Fit) closed the EncodedRom + emulator one-token contract. Full-corpus emulator gating is an S8 concern. Per planv0 amendment item 7, every NEW ROM-emitting bead must add a blocks edge from F-C2 closure; S7 emits ROM only for the one-token harness. |
| A29 | Why not include lambda_range and lambda_zero in S7?                                       | They are S2-closed; their integration is a separate concern                 | Wouldn't they help router stability?                                                | S2 closed lambda_range/lambda_zero in their own context (ternary survival). Re-adding them at S7 would conflate the F5-partial scope. T5.5 (lambda_shape/lambda_overflow) is the sole pending F5 work and is gated to S8. |
| A30 | Why pin lambda_distill = 1.0 instead of a non-default?                                   | Phase-effective gating + explicit non-default test value (D5)               | Doesn't 1.0 violate CLAUDE.md "non-default value" rule?                              | The CLAUDE.md rule mandates a non-default test for SCALAR HYPERPARAMETERS such as safe bounds, temperatures, and loss weights. S7 satisfies this with lambda_balance_alt=0.1, lambda_zrouter_alt=1e-2, and lambda_switch_alt=0.5; lambda_distill = 1.0 is the strength scalar (KL temperature) and is independently tested under T_d != 1 in a follow-up bead. |
| A31 | Are TemporalSwitchDigest's same_expert_rate and the per-step same_expert_rate the same?   | Same metric, different cadences (D19; §3.3)                                | Where's the proof of agreement?                                                    | Per-step metric is computed live during training; export-fact metric is computed during HardenAndSelect over a held-out eval slice. The two values converge in late Phase E; their agreement within numerical tolerance is recorded in s7_switch_stats.v1 vs the last per-step capture, but not as a closure gate (it's an observation). |
| A32 | What if H9 (oracle routed) Refutes due to a fixture coverage gap, not an oracle bug?     | Pre-condition check fires; H9 marked NotEvaluatedDueToPriorGate; halt        | Wouldn't this just fail closure?                                                    | Yes, with a clear diagnostic (Halt(audit-fixture)). The fixture coverage check is in §6.5 as a pre-condition explicitly to avoid this confusion. The fixture is committed; if route coverage degrades after a code change, the CI fails fast before the oracle suite runs. |

---

# 21. Final concise contract

```text
F-S7 MoE Beats Dense at Matched Bytes is correct when:

1.  Five seeded MoeTiny runs and five seeded MoeTinyDenseMatched runs
    on Project Gutenberg complete Phase A->E without divergence and
    without router collapse, under byte-identical training scaffold
    other than the topology itself.

2.  Per-seed parity gate fires:
    ∀ s. bpc(MoeTiny_seed=s, val_gutenberg)
          < bpc(MoeTinyDenseMatched_seed=s, val_gutenberg) − 0.05
    AND deployed-byte parity holds within ±10% (or 4 banks).

3.  Pareto frontier dominance: median MoeTiny dominates median
    MoeTinyDenseMatched on (val_bpc, deployed_bytes_total) with at
    least one strict inequality.

4.  Switch statistics export schemas are correct, complete, LayerId-
    scoped, and round-trip stable. ExpertSlotAffinity unordered pairs
    are canonicalized in both construction and deserialization.

5.  Standard producer telemetry (D19) is captured at every step under
    structured tracing for all five metrics.

6.  lambda_switch sweep over the D11 grid demonstrates non-collapse at
    production lambda (0.05) AND demonstrable collapse at high lambda
    (5.0); GuardrailVerdict = Pass.

7.  Each loss term's gradient flow matches its declared provenance.
    L_switch reaches routing probabilities (and via softmax, the
    LowRankRouter parameters); balance_loss does not reach the
    stop-gradient dispatch indicator; z-loss is centered with
    mu = log(n_experts) and zero baseline; teacher is frozen.

8.  Burn adapter for ExpertBlockQat (bd-2c8z) produces deterministic,
    finite, nonzero gradients into all required parameter sets, with
    GLU construction explicitly rejected.

9.  ArtifactOracle three-way agreement holds on the routed FFN within
    S3 tolerance; weights resolved via QuantSpec::weight_quant.

10. EncodedRom + emulator one-token harness preserves on the MoE
    artifact; observed bank_switches_per_token agrees with the artifact-
    oracle recorded value within ±1.

11. s7_report.v1 emits pre-registered predictions in git history
    strictly before the first S7 result artifact commit, and concludes
    with exactly one Decision value chosen by §12 dispatch.

12. Decision is one of {ProceedToS8, ProceedToS8-DenseOnly}; any other
    Decision blocks bd-2v9r closure. ProceedToS8-DenseOnly is
    permitted iff S7Outcome = Fail-parity caused by per-seed bpc parity
    failure under valid matched-deployed-byte accounting. Bytes mismatch,
    missing artifacts, gradient-provenance failure, oracle failure, emulator
    failure, or guardrail failure are invalid-experiment outcomes and MUST
    NOT close bd-2v9r as DenseOnly.

13. Every JSON artifact (s7_run_log, s7_score, s7_switch_stats,
    s7_router_collapse_sweep, s7_dense_vs_moe, s7_frontier,
    s7_burn_grad_smoke, s7_oracle_routed, s7_emulator_one_token,
    s7_report) is canonical, deterministic, and self-hash-valid.
    Aggregate-distribution invariants validated in BOTH construction
    and deserialization.

14. All ten hypotheses have explicit verdicts in the falsification
    analysis section, with concrete observations cited.

15. The nine-test falsification suite passes: deliberately-broken
    implementations produce the expected Refuted verdicts.

16. S7 retires MoE-vs-dense risk at MoeTiny size on Gutenberg only.
    It does not claim UpperBankCandidate production-scale viability
    on Gutenberg, StructuredWidthGates supernet, top-2 routing, GLU
    experts, or
    full-corpus ROM emission — those are S8 / experimental / out-of-
    scope concerns.
```
