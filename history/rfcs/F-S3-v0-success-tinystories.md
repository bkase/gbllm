# Formal spec pack: F-S3 v0_success on TinyStories — real export + oracle round-trip

> DRAFT. This is the third scientific/experimental RFC in the training-contract
> epic. It inherits the pre-registration discipline, falsification suite
> pattern, and report self-hash regime of F-S1 First Pulse and the QAT phase
> closure of F-S2 QAT Survives. It introduces, for the first time in this
> project, real cross-epic surface area: the corpus pipeline (F-G1), the
> charset_v1 lexical contract (F-G2), the denotational stratum
> (F-C1 `DenotationalOracle`), the artifact stratum (F-C2 `ArtifactOracle`),
> the durable `ReferenceModelBundle` export, and the v0_success
> `WorkloadManifest`. Its deliverable is **verified knowledge** that the
> training output, the exported reference bundle, and the artifact oracle
> agree on a pinned workload — not merely code that runs.

Important interpretation:

```text
A `Fail-oracle-agreement` result is a successful scientific falsification, not
merely an implementation failure. It means S3 did *not* retire
denotation/artifact agreement risk under the workload. Only a passing
agreement verdict produced by the real F-C1/F-C2 oracle backends, together
with H1, H2, H3, H5, H6, and H7, retires real-oracle denotation/artifact
agreement risk for the v0_success workload. It still does not retire
emulator end-to-end risk (S6) or cross-corpus generalization risk (S4).

When a named S3 fallback evaluator is used, bd-3k8o may close only as
Pass-with-fallback-oracle / ProceedToS4-with-deferred-clause. That closure
retires the S3-local export/fallback-evaluator agreement risk, but explicitly
does not retire real F-C1/F-C2 implementation risk. The deferred clause
remains a mandatory gate before any later slice may claim real-oracle risk
retirement.
```

```text
Spec:
  F-S3 v0_success on TinyStories — real export + oracle round-trip
  Slice S3 of the training-contract epic (bd-1rb)
  Closure bead: bd-3k8o

Hypothesis-under-test:
  A Toy0 model trained through the F4 phase scheduler (Phase A through
  Phase D) on charset_v1-normalized TinyStories produces, at the end of
  Phase D, a frozen dense teacher checkpoint and a hard ternary student
  checkpoint such that:
    (a) the dense teacher exports as a deterministic ReferenceModelBundle
        whose ReferenceProgram, evaluated by DenotationalOracle on the
        v0_success workload prompt set, agrees with the live training
        teacher's forward pass within the H4 tolerance band;
    (b) the hard ternary student exports as a ModelArtifact whose
        ArtifactOracle observations agree with the live training student
        within the H4 tolerance band;
    (c) the v0_success WorkloadManifest passes for the dense baseline:
        per-seed bpc beats the 5-gram Kneser-Ney baseline on the pinned
        post-normalization validation sequence, the model survives ternary QAT with bpc(ternary) - bpc(fp)
        <= 0.5 bpc, generation respects the v1 charset, no immediate
        repetition collapse occurs, and the artifact fits a conservative
        RuntimeChromeBudget estimate.

Owns:
  hypothesis statements H1..H7
  pre-registered prediction tables (S3 instance)
  charset_v1 normalization contract (S3 schema instance; bd-2ym0 schema is
    upstream owner of LexicalSpec, F-G1 owns the deterministic pipeline)
  TinyStories.v2 manifest with charset_v1 normalization sha256
  5-gram Kneser-Ney baseline math (S3 instance)
  bpc-per-character primitive (S3 instance, vocab=80)
  v0_success WorkloadManifest schema instance and per-prompt acceptance
  ReferenceModelBundle export protocol (S3 instance; T4.3b)
  ModelArtifact export protocol (S3 instance; Phase-D hard ternary student)
  DenotationalOracle replay contract (S3 instance; F-C1)
  ArtifactOracle replay contract (S3 instance; F-C2)
  Phase-specific exported-surface agreement gate (train ~ bundle, train ~ artifact)
  ConformanceEnvelope emission (S3 instance; F-C4 schema upstream)
  s3_charset_v1.v1, s3_baseline_kn5.v1, s3_bundle.v1,
  s3_artifact.v1,
  s3_oracle_agreement.v1, s3_v0_success.v1, s3_conformance.v1,
  s3_oracle_re_run.v1, s3_report.v1
  S3 reproducibility laws (extend Rep-1..Rep-8 from S1)
  S3 falsification suite (nine broken substitutes)
  Closure of F4 (Phased Training with Dense Teacher)

Does not own:
  Project Gutenberg corpus + progression schedule (S4)
  cross-corpus contamination report (S4)
  BoundedKv attention-oracle conformance (S5)
  LinearState multi-timescale A/B (S5)
  RuntimeChromeBudget end-to-end real measurement (S6)
  shadow compile + EncodedRom + emulator harness end-to-end (S6)
  emulator-runs-at-least-one-token clause of v0 success (deferred to S6)
  MoE / router (S7)
  UpperBankCandidate production-scale runs on Gutenberg +
    StructuredWidthGates (S8)
```

## Decisions

```text
D1 charset_v1 normalization is mandatory, not raw bytes
   S3 trains on charset_v1-normalized TinyStories with vocab = 80.
   The S1-era raw-byte loader is retired for S3 and beyond. The S1
   raw-byte stream remains valid for re-running F-S1 against a fixed
   pass_version, but s1_score.v1 numbers and s3_v0_success.v1 numbers
   are not directly comparable: one is bpc-per-byte, the other is
   bpc-per-character.

   Normalization order is hashed into LexicalSpec and is part of the
   ArtifactCore identity hash. The order is, exactly:

     1. Unicode NFC normalize.
     2. Strip combining accents via NFD-decompose-and-filter.
     3. Preserve case as-is (mixed case is load-bearing).
     4. Fold quotation/dash/ellipsis variants to ASCII:
          U+201C U+201D U+00AB U+00BB        -> '"'
          U+2018 U+2019 U+201A               -> '\''
          U+2014 (em dash)                   -> "--"
          U+2013 (en dash)                   -> "-"
          U+2026 (ellipsis)                  -> "..."
     5. Whitespace normalization, in this exact order:
          CRLF and CR -> LF;
          tab -> single space;
          trim trailing ASCII spaces before LF and at end of example;
          collapse runs of two or more internal ASCII spaces to one;
          preserve LF characters exactly after the previous rewrites.
     6. Unmappable codepoint handling:
          if any character outside the 76-printable-codepoint set survives
          steps 1..5, replace with the <unk> control token id 79.
          For each raw example e, let unk_fraction(e) be:

            count(<unk> tokens after normalization) /
            max(1, post-normalization token count before example dropping)

          If unk_fraction(e) > 0.02, drop the example.

          The manifest records both:
            unmappable_example_drop_rate =
              dropped_example_count / raw_example_count
            unmappable_char_drop_rate =
              post_normalization_token_count_of_dropped_examples /
              post_normalization_character_count_before_example_dropping

          H1's hard gate applies to unmappable_example_drop_rate unless
          otherwise stated.

   The 76-printable set is, exactly:

     A..Z  (26 codepoints, ids 0..25)
     a..z  (26 codepoints, ids 26..51)
     0..9  (10 codepoints, ids 52..61)
     punctuation (13 codepoints, ids 62..74):
       ' '   (space, id 62)
       '.'   (id 63)
       ','   (id 64)
       '!'   (id 65)
       '?'   (id 66)
       '-'   (id 67)
       '\''  (id 68)
       ':'   (id 69)
       ';'   (id 70)
       '('   (id 71)
       ')'   (id 72)
       '"'   (id 73)
       '/'   (id 74)
     newline '\n' (id 75)
     [reserved] (id 76)               ; not printable; reserved for forward charset
                                      ; expansion in v1.1; never appears in
                                      ; a v1 stream and is rejected by the
                                      ; loader if seen in input
     control tokens (ids 77..79):
       <bos>  (id 77)
       <eos>  (id 78)
       <unk>  (id 79)

   vocab_size = 80. Tied embedding/classifier sharing is mandatory at
   this vocab size; see bd-3bf1 and §7.

   The normalization API is typed:

     normalize_raw(raw_example_bytes) -> TextCharSeqWithStats
       Applies steps 1..6 below to raw UTF-8 source text and emits token ids.

     normalize_tokens(text_char_seq) -> TextCharSeq
       Canonicalizes an already-tokenized TextCharSeq. It accepts ids
       {0..75} ∪ {79}, rejects ids 76, 77, and 78, and returns the same
       sequence unchanged.

   Idempotence means:

     normalize_tokens(normalize_raw(x).tokens) == normalize_raw(x).tokens

   The literal source string "<unk>" in raw bytes is not a control token.
   It is normalized as ordinary source text under steps 1..6. Only the
   already-tokenized id 79 is treated as <unk>.

   The literal ASCII string "<|endoftext|>" present in the raw
   TinyStories stream is, after step 6, mapped to character tokens:
   '<', each '|', and '>' each become <unk> because they are unmappable;
   the letters in "endoftext" remain printable. The S3 loader does NOT
   interpret this literal as a
   semantic boundary token. Document boundary insertion is reserved
   for S4+ once a corpus governance policy defines it.

   Raw example boundaries are determined before charset_v1 normalization
   by the TinyStories.v2 manifest reader. The literal "<|endoftext|>"
   MUST NOT create, delete, or merge examples in S3; if it appears inside
   a raw example, it is treated as ordinary text under the
   unmappable-codepoint rule above.

D2 fixed seed list (inherited from S1)
   seeds = [0, 1, 2, 3, 4]
   Five seeds are mandatory. No more, no fewer. S3 reuses this list.

D3 fixed train budget (S3 instance; PhaseBudget_S3 = PhaseBudget_S2)
   Per seed s and per phase p ∈ {A, B, C, D}:
     optimizer_steps   exactly as listed in PhaseBudget_S3 below
     batch_size        = 32                   ; inherited from F-S1 D3 / F-S2 D1
     sequence_length   = 128                  ; inherited from F-S1 D3 / F-S2 D1
     eval_every_steps  = 1000                 ; inherited from F-S1 D3 / F-S2 D1
     eval_subset_size  = 4096 sequences

   Phase B has phase_mode = DegenerateDenseNoop for S3 (Toy0 carries no
   router; PhaseKindS2::PhaseB consumes router_train_mode = NoRouter as
   already pinned in F-S2 D1). The phase id remains PhaseKindS2::PhaseB
   everywhere in schemas, logs, hashes, and reports; PhaseKindFixture::PhaseE
   is reserved for fixture-only D8 transition tests and is not part of an
   S3 production run.

   PhaseBudget_S3 (inherited verbatim from F-S2 D1; values cited from
   gbf-experiments/src/s2/schema.rs):

     A.optimizer_steps = 4_000     ; S2_TEACHER_FREEZE_STEP        (steps 1..4000)
     B.optimizer_steps = 1_000     ;                                (steps 4001..5000)
     C.optimizer_steps = 3_000     ; S2_PHASE_C_END_STEP - 5000    (steps 5001..8000)
     D.optimizer_steps = 2_000     ; S2_OPTIMIZER_STEPS - 8000     (steps 8001..10000)
     total             = 10_000    ; S2_OPTIMIZER_STEPS = S1_OPTIMIZER_STEPS

   These four integers are not S3 estimates; they are the F-S2-pinned
   constants `S2_TEACHER_FREEZE_STEP = 4_000`, `S2_PHASE_B_END_STEP = 5_000`,
   `S2_PHASE_C_END_STEP = 8_000`, `S2_OPTIMIZER_STEPS = 10_000`. Changing
   any of them invalidates the S3 train_config_hash and the inherited S2
   `pass_version_S2`; both effects require a coordinated F-S2 amendment.

   The teacher_freeze event fires at the boundary between step 4000 and
   step 4001 (S2 S2-Run-Ok-8); the bundle export contract (§7) consumes
   the snapshot taken AFTER step 4000's optimizer update. The
   end-of-Phase-D student snapshot is taken AFTER step 10000's optimizer
   update; the artifact export contract (§7) consumes that snapshot.

   Phase E (HardenAndSelect) is replaced for S3 closure by §7 Bundle and
   Artifact Export Contract; shadow compile is deferred to S6.

   The deterministic batch/eval sampling rule, training objective, and
   gate scoring rule of F-S1 D3a are re-affirmed verbatim, with one
   amendment:

     S3 amendment to F-S1 D3a — bpc-per-character substitution
       The per-step train loss is the mean natural-log cross entropy
       over batch_size * sequence_length target *characters* (not
       bytes). progress_eval_chars = val[0 .. min(len(val_chars),
       4096 * 128)]. Gate scoring is over the full normalized val
       character sequence, including a final short chunk if present.

   Eval cost on Toy0 is negligible at vocab=80; full val is scored.

D4 fixed 5-gram Kneser-Ney baseline math (replaces F-S1 D4)
   Modified Kneser-Ney smoothing, order n = 5, three discounts D_1,
   D_2, D_3+ as defined by Chen and Goodman (1998) eq. 28 with the
   formulas pinned in §6.

   Discounts are computed by D-rule from the 5-gram count-of-counts
   on corpus_train (charset_v1-normalized) at fit time:

     Y      = n_1 / (n_1 + 2 * n_2)
     D_1    = 1 - 2 * Y * (n_2 / n_1)
     D_2    = 2 - 3 * Y * (n_3 / n_2)
     D_3+   = 3 - 4 * Y * (n_4 / n_3)

   where n_k is the number of distinct 5-grams whose count in
   corpus_train equals k.

   For interpolated orders k ∈ {2, 3, 4, 5}, the same D-rule is
   applied using the count-of-counts of the effective count table C_k
   consumed by P_KN_k, as defined in §3 KnEffectiveCounts. P_KN_1 uses
   continuation counts and has no discount parameters.

   The D-rule is defined only when n_1^{(k)}, n_2^{(k)}, and
   n_3^{(k)} are all non-zero for each k ∈ {2,3,4,5}. n_4^{(k)} may be
   zero, in which case the D_3+ formula contributes zero from n_4/n_3.
   If the required n_1/n_2/n_3 condition
   fails, baseline fitting aborts with Fail-baseline. No add-alpha,
   discount clipping, linear-interpolation fallback, or guessed default
   discount is permitted in S3.

   All discounts are computed in f64 from the integer counts at the
   point of probability computation; f32 rounding is forbidden in
   baseline probability, bpc-per-character, and oracle computations.

   Vocabulary matches the model: |Sigma| = 80.

   No add-alpha smoothing. No linear interpolation override of the
   D-rule discounts. No <bos>/<eos> insertion at corpus_train edges
   when computing counts; the corpus is treated as one contiguous
   normalized character sequence for count extraction. Reset-context
   semantics applies at scoring time exactly as in F-S1 D4 P1/P2/P3,
   extended to orders 4 and 5: when a validation chunk starts, the
   scorer queries P_k for the k-th character (k = 1..5) until the
   intra-chunk context reaches length 4, after which the full 5-gram
   conditional is queried.

D5 fixed split (amends F-S1 D5)
   TinyStories canonical train/val split, hash-pinned in
   fixtures/corpora/tinystories.v2.toml. The S3 manifest pins the
   exact post-charset_v1-normalization train and val character
   sequences. The pre-normalization train and val byte sha256s of
   F-S1's tinystories.toml are also recorded for cross-revision
   traceability, but S3 scoring uses only the post-normalization
   character sequences.

   No on-the-fly resampling. No archive reserialization. The S3
   manifest also pins the v0_success workload prompt set
   (held-out chapter sha256, the chapter character count, and the
   fixed prompt offsets; see §9).

D6 strict pass criterion (composite, per seed)
   The S3 closure gate is a *composite* per-seed predicate:
     ∀ seed s. all of the following hold for the Phase-A frozen dense
       teacher and the Phase-D hard ternary student artifact:

       (Q1) val_bpc_char_fp(s) < bpc_kn5_baseline - 0.05
       (Q2) val_bpc_char_ternary(s) - val_bpc_char_fp(s) <= 0.5
       (Q3) generated_token_charset_validity_rate(s) = 1.0 over all decoded
            generated text characters for all workload prompts, where valid
            generated text tokens are ids {0..75} ∪ {79}; id 78 <eos>
            is permitted only as a terminal stop token and is not counted
            as a generated character; ids 76 and 77 are always invalid
            in decoded text.
       (Q4) max over prompts of max_consecutive_same_token(s, p) <= 8
       (Q5) min over prompts of generated_char_count(s, p) >= 128
       (Q6) artifact_deployable_bytes(s) <= conservative_chrome_budget_bytes

   median, max, min are reported, but the pass test is per-seed.

D7 mandatory surface-agreement gate (S3-only)
   Surface agreement is a closure gate across the three exported/evaluated
   surfaces, but the gated comparison is phase-specific:

     Phase A: live frozen dense teacher ↔ ReferenceModelBundle
     Phase D: live hard ternary student ↔ ModelArtifact

   The Phase-A bundle and the Phase-D artifact are not required to be
   mutually bitwise equal. Their difference is the quantization/distillation
   gap and is reported in the ConformanceEnvelope.

   For each seed s and each prompt p ∈ v0_success.prompts:

     A_train_A[s, p]  = live frozen Phase-A dense teacher output sequence on p
     A_train_D[s, p]  = live Phase-D hard ternary student output sequence on p
                          (per LexicalSpec, recorded at the
                          PostLogits and PostDecode SemanticCheckpoints)
     A_bundle[s, p]   = DenotationalOracle.evaluate(
                          ReferenceModelBundle_seed_s, p, observation_policy)
     A_artifact[s, p] = ArtifactOracle.evaluate(
                          ModelArtifact_seed_s, p, observation_policy)

   Agreement modes (pinned by phase):

     Phase A (dense teacher / fp32):
       PostLogits:     |A_train_A.logits - A_bundle.logits|_inf <= 4.0e-6
       PostDecode:     argmax token equality, every checked step

       No Phase-A artifact comparison is required unless this RFC also
       defines and exports a separate Phase-A fp32 ModelArtifact. The S3
       artifact otherwise denotes the Phase-D hard ternary student.

     Phase D (hard ternary):
       PostLogits:     |A_train_D.logits - A_artifact.logits|_inf = 0.0
                       bitwise equality under QuantSpec_S3's pinned
                       Ternary2 reduction policy
       PostDecode:     argmax token equality, every checked step

       A_bundle vs A_artifact is recorded as a non-gating
       quantization/distillation-gap metric.

   The surface-agreement check is run on a pinned subset of
   v0_success.prompts (the first three prompts, ordered as written
   in the manifest) at each of two SemanticCheckpoints per prompt:
   PostLogits for each checked generated step and PostDecode for the
   corresponding selected token. At minimum, steps 1..16 are checked;
   step 1's PostLogits row is the prompt-final logits row.

   Agreement traces are forced-length traces: stop_on_eos = false for
   the first 16 checked steps. If id 78 (<eos>) is selected before step
   16, it is compared as an ordinary selected token for D7 purposes and
   decoding continues for the remaining agreement-trace steps. The
   v0_success quality generation procedure in §9 remains stop_on_eos =
   true and uses Q5 to fail premature terminal generation.

   |.|_inf is the elementwise absolute maximum after reducing each
   logits vector to a canonical f32 representation as defined by
   ReferenceNumericProfile. Argmax token equality compares the
   selected token id under the workload's pinned DecodeMode
   (Argmax, no temperature, no sampling) at every generated step
   for at least 16 generated steps per prompt.

D8 strict reproducibility (extends F-S1 D8 and F-S2 Rep-S2-1..4)
   Same seed + same corpus_train_sha + same corpus_val_sha +
   same charset_v1_sha + same s2_train_config_hash + same
   s3_train_config_hash + same model_config_hash + same gbf-train
   pass_version + same pass_version_S2 + same pass_version_S3 +
   same S2EnvironmentHash + same S3EnvironmentHash (which packs
   build_config_hash, rust_toolchain_hash, dependency_lockfile_hash,
   and oracle_backend_identity per §3) + same device_profile (pinned
   to S1CpuDeterministic) + same export_visitor_hash
   ⇒ bit-identical canonical checkpoint payload sha at every
     PhaseKindS2 boundary (steps 4000, 5000, 8000, 10000) AND, when
     the canonical SafeTensors writer is used, bit-identical
     SafeTensors file bytes AND
     bit-identical ReferenceModelBundle bytes AND
     bit-identical ModelArtifact bytes (excluding ArtifactAux mutable
     sidecars, which are compared by canonical_aux_payload_sha as
     defined in §11).

   ReferenceModelBundle and ModelArtifact byte-equality is asserted
   under canonical write rules (§11). SafeTensors container metadata,
   timestamps, host paths, and nondeterministic map iteration order
   must either be absent from the canonical writer or excluded before
   computing canonical checkpoint payload sha. Raw file-byte equality
   may be claimed only for canonical SafeTensors files.

D9 fail-closed on NaN / divergence (re-affirmed from F-S1 D9 and F-S2)
   Any seed producing non-finite loss or non-finite gradient norm at
   any step in any phase fails the entire S3. No partial pass.

D10 optimizer pinned (re-affirmed from F-S1 D10, with F-S2 phase plan)
   AdamW { lr=1e-3, β1=0.9, β2=0.999, eps=1e-8, weight_decay=0.0 }
   No schedule. No warmup. The training phase scheduler from F-S2 governs
   QuantHardness ramp and loss term activation at phase boundaries via
   `gbf_train::scheduler::TrainingPhaseSchedule` consuming
   `gbf_train::phase::TrainPhaseSpec` for each PhaseKindS2 variant.
   S3 does not amend the optimizer; S3 amends only the data, the
   baseline, and the export+oracle contract.

   Phase boundary effects (re-affirmed from F-S2 D2 and the boundary
   projection rule recorded in `PhaseBoundaryHardnessProjection` of
   gbf-experiments::s2::schema): on a A→B transition no QAT is active;
   on B→C the expert_qat soak window opens; on C→D the expert_qat is
   already Hard and activation/norm transitions begin per D2.

D11 export visitor identity is normative
   The exported ReferenceModelBundle and the exported ModelArtifact
   share a single ExportVisitor identity, recorded in
   export_visitor_hash. Bumping the ExportVisitor version invalidates
   prior s3_bundle.v1 and s3_artifact.v1 artifacts.

   Tied embedding/classifier sharing (bd-3bf1) is represented in both
   ReferenceModelBundle and ModelArtifact as a single CanonicalTensor
   referenced twice (once as input embedding, once as output
   classifier weight) with explicit alias metadata. Silent
   duplication of the classifier payload is forbidden and is the
   subject of falsification F6 (§14).

D12 deployable weights resolve through QuantSpec, not by name
   ArtifactOracle MUST resolve deployable logical weights via
   QuantSpec::weight_quant. Tensor-id naming conventions
   ("linear_0_weight" vs "linear_0_weight_shadow_fp32", etc.) are NOT
   a substitute. Falsification F4 pins this rule.

D13 bpc-per-character semantics
   bpc_char on a normalized character sequence V_char with N characters:

     bpc_char(M, V_char) = (1/N) * Σ_{i=0}^{N-1} -log2(P_M(V_char[i] | ctx_char(i)))

   chunk_size = 128 *characters* (not bytes); state resets between
   chunks.

   Empty-context scoring rule:
     The first target character of each chunk is scored from the model's
     canonical initial state. No <bos> or <eos> token is inserted into
     val_post, and no boundary token is counted as a target. If an
     evaluator internally represents the initial state with <bos>, that
     token is non-scored and must produce the same distribution as the
     canonical empty-context state recorded in SequenceSemanticsSpec.

   The KN baseline's corresponding empty-context distribution is P_KN_1.

   This is the S3 reset-context bpc-per-character primitive. Both the
   model scorer and the 5-gram KN baseline scorer use this primitive.

D14 v0_success workload defers emulator end-to-end to S6
   The eighth clause of the planv0 2026-05-06 amendment ("runs at
   least one token through the emulator harness end-to-end") is
   relaxed at S3 to:
     "compiles to a verifiable artifact whose ArtifactOracle and
      DenotationalOracle agree with the live training model on the
      pinned three-way agreement subset, and whose serialized bytes
      fit a conservative_chrome_budget_bytes estimate."

   The emulator-runs-at-least-one-token clause is pinned to S6 (Slice
   S6: Game Boy ROM build) and recorded in the ambiguity ledger as
   A6.

D15 RuntimeChromeBudget estimate is conservative and synthetic at S3
   conservative_chrome_budget_bytes is computed from a synthetic
   RuntimeChromeBudget where:
     ExpertBank usable_bytes := smallest_expert_bank_default_bytes * 0.90
     CommonBank usable_bytes := common_bank_default_bytes * 0.90
     Bank0Free  usable_bytes := bank0_free_default_bytes * 0.90
   The defaults are pinned in fixtures/runtime/chrome_budget.synthetic.toml.
   The real RuntimeChromeBudget produced by a UI/runtime shell build
   is owned by S6.
```

---

# 1. Hypothesis algebra

Every hypothesis carries a statement, predicted observables, falsification
rule, verdict mapping, and downstream consequence. H1, H2, H3, H4, H5, H6
are **mandatory closure gates**. H7 is also closure-gating because its
verdict carries the F4 closure decision.

## H1 Charset and loader integrity

```text
Statement:
  The charset_v1 normalization pipeline produces a deterministic,
  idempotent, and round-trip-stable post-normalization stream from the
  pinned TinyStories raw bytes; the manifest sha256s match; the
  unmappable drop rate is within the pre-registered bound.

Predicted:
  unmappable_example_drop_rate(train) ∈ [0.0, 0.005]        ; ≤ 0.5% [ESTIMATE]
  unmappable_example_drop_rate(val)   ∈ [0.0, 0.005]        ; ≤ 0.5% [ESTIMATE]
  charset_validity_rate(train_post) = 1.0                    ; every char ∈ {0..75} ∪ {79}
  charset_validity_rate(val_post)   = 1.0
  idempotence_holds(train_pre_sample) = true
  idempotence_holds(val_pre_sample)   = true
  manifest.train_sha256              = sha256(train_post)    ; verified at load
  manifest.val_sha256                = sha256(val_post)
  manifest.charset_v1_sha256         = sha256(LexicalSpec_v1) ; identity-hashed

Falsification:
  ∃ x. normalize_tokens(normalize_raw(x).tokens)
       ≠ normalize_raw(x).tokens                            ⇒ Refuted
  any post_token ∉ {0..75} ∪ {79}                            ⇒ Refuted
  any <bos> or <eos> token in train_post or val_post          ⇒ Refuted
  unmappable_example_drop_rate(train) > 0.02                 ⇒ Refuted
  unmappable_example_drop_rate(val)   > 0.02                 ⇒ Refuted
  manifest sha256 mismatch                                   ⇒ Refuted
  any occurrence of reserved id 76 in train_post or val_post  ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Halt. Every later slice's gate numbers are unreliable until charset_v1
  is fixed. Open follow-up bead under F-G1/F-G2.
```

## H2 5-gram Kneser-Ney baseline soundness

```text
Statement:
  The 5-gram KN baseline math, as pinned by D4, produces the exact
  expected probabilities and exact expected bpc_char on a hand-counted
  fixture corpus, within 1e-12 in f64; the D-rule discounts are
  computed from the corpus_train count-of-counts as specified.

Predicted:
  bpc_kn5_oracle_fixture     = expected_bpc_char_oracle ± 1.0e-12
  D_1 ∈ [0,1], D_2 ∈ [0,2], D_3+ ∈ [0,3], all finite          ; D-rule sanity
  Y                          ∈ (0.0, 1.0)                    ; D-rule sanity
  bpc_kn5_baseline(val_post) ∈ [1.7, 2.6]                    ; sanity range, [ESTIMATE]
  bpc_kn5_baseline(val_post) is reported beside the S1 3-gram
  byte-level baseline for traceability only. No ordering prediction is
  made because the metrics are not directly comparable.

Falsification:
  |bpc_kn5_oracle_fixture - expected_bpc_char_oracle| > 1.0e-12
                                                              ⇒ Refuted
  any required count-of-count n_1, n_2, or n_3 is zero        ⇒ Refuted
  any D_k < 0 or non-finite                                  ⇒ Refuted
  D_1 > 1 or D_2 > 2 or D_3+ > 3                              ⇒ Refuted
  Y not in (0.0, 1.0)                                        ⇒ Refuted
  any zero probability assigned to an actually scored target
    character in val_post at any context length 1..5          ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  bpc_kn5_baseline is unreliable; H3's per-seed Q1 predicate is
  unverifiable. Halt; open KN math bead.
```

## H3 v0_success quality gate (per-seed)

```text
Statement:
  For every seed s ∈ {0, 1, 2, 3, 4}, the dense teacher trained
  from seed s through F4 Phase A and the hard ternary student exported
  after Phase D jointly produce, on the v0_success workload,
  observations that satisfy each of D6 Q1..Q6.

Predicted:
  Q1 val_bpc_char_fp(s)         < bpc_kn5_baseline - 0.05
  Q2 val_bpc_char_ternary(s) - val_bpc_char_fp(s) <= 0.5
  Q3 generated_token_charset_validity_rate(s) = 1.0
  Q4 max_consecutive_same_token(s) <= 8
  Q5 generated_chars_per_prompt(s) >= 128
  Q6 artifact_deployable_bytes(s) <= conservative_chrome_budget_bytes

Falsification:
  ∃ s. ¬(Q1 ∧ Q2 ∧ Q3 ∧ Q4 ∧ Q5 ∧ Q6)                       ⇒ Refuted
  median(val_bpc_char_fp) < 0.5                              ⇒ Refuted (suspicious)

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  v0 is not "working enough" for the dense baseline. Decision dispatch
  in §10 routes to Investigate(quality) or Halt(suspicious-low-bpc).
```

## H4 Surface oracle agreement

```text
Statement:
  For each seed s ∈ {0, 1, 2, 3, 4} and each prompt p in the pinned
  agreement subset of v0_success.prompts:
    - the live frozen Phase-A dense teacher agrees with the
      DenotationalOracle output on the exported ReferenceModelBundle;
    - the live Phase-D hard ternary student agrees with the ArtifactOracle
      output on the exported ModelArtifact.

  The Phase-A bundle and Phase-D artifact are not required to agree with
  each other; their difference is reported as quantization/distillation gap.

Predicted:
  ∀ s, p ∈ subset.
    PostLogits Phase A:
      |A_train_A.logits - A_bundle.logits|_inf <= 4.0e-6
    PostLogits Phase D:
      A_train_D.logits == A_artifact.logits                   ; bitwise
    PostDecode (both phases):
      Phase A: A_train_A.token == A_bundle.token
      Phase D: A_train_D.token == A_artifact.token

  At least 16 generated steps per prompt in the subset are checked.

Falsification:
  any |.|_inf tolerance breach at Phase A                    ⇒ Refuted
  any bitwise inequality at Phase D                          ⇒ Refuted
  any argmax token mismatch at any of the checked steps      ⇒ Refuted
  ArtifactOracle resolves deployable weights by tensor-id name
    rather than via QuantSpec::weight_quant                  ⇒ Refuted
  s3_conformance.v1 is missing, self-hash-invalid, or computed using
    prompt-wide softmax aggregation rather than per-token/per-vocab-row
    aggregation                                                ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Either export drift, oracle implementation drift, or
  quantization-resolution drift. Block S6 and any future
  ROM-emitting bead per the planv0 2026-05-06 ordering rule.
```

## H5 Bundle and artifact export determinism

```text
Statement:
  Given identical training inputs (corpus + charset_v1 + train_config +
  pass_version + device_profile + export_visitor_hash) and identical
  frozen teacher / hard ternary student weights, the exported
  ReferenceModelBundle and ModelArtifact bytes are bit-identical across
  replays under their canonical write rules.

Predicted:
  ∀ s. bundle_self_hash_replay_1(s) = bundle_self_hash_replay_2(s)
  ∀ s. canonical_bundle_payload_sha_replay_1(s)
       = canonical_bundle_payload_sha_replay_2(s)
  ∀ s. artifact_self_hash_replay_1(s) = artifact_self_hash_replay_2(s)
  ∀ s. canonical_artifact_payload_sha_replay_1(s)
       = canonical_artifact_payload_sha_replay_2(s)
  ∀ s. tied_embedding_classifier_alias.canonicaltensor_id is identical
       across the embedding and the classifier reference in both bundle
       and artifact (one CanonicalTensor, two references; not two tensors
       with equal bytes).

Falsification:
  any replay-pair hash mismatch                              ⇒ Refuted
  any tied embedding represented as two separate
    CanonicalTensors with equal bytes                        ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Bundle export is non-deterministic or silently duplicates payload.
  Either way, denotational truth is unstable; H4 is contaminated.
```

## H6 Artifact deployable weights resolve through QuantSpec

```text
Statement:
  ArtifactOracle's evaluation pipeline resolves deployable
  logical weights through QuantSpec::weight_quant on every
  Linear/Embedding/Classifier op in the canonical logical form, and
  produces the same logits as a reference evaluator that resolves
  weights only by tensor-id name when the artifact's tensor-id naming
  is canonical, BUT produces strictly different logits when the
  artifact's tensor-id naming is intentionally adversarial (e.g. a
  shadow tensor "linear_0_weight_naive_fp32" is present alongside the
  real one).

Predicted:
  on the canonical_naming fixture artifact:
    artifact_oracle_logits == name_resolver_logits          ; bitwise,
                                                            ; sanity-only
  on the adversarial_naming fixture artifact (test-only):
    artifact_oracle_logits != name_resolver_logits          ; difference > 0
    artifact_oracle_logits == quant_spec_resolver_logits    ; bitwise

  The adversarial fixture MUST choose the shadow tensor values and prompt
  so that the name-resolver and QuantSpec-resolver logits differ by a
  nonzero amount. Accidental numerical equality invalidates the fixture,
  not the hypothesis.

Falsification:
  artifact_oracle_logits == name_resolver_logits on the
    adversarial_naming fixture                               ⇒ Refuted
  artifact_oracle_logits != quant_spec_resolver_logits on
    the adversarial_naming fixture                           ⇒ Refuted

Sanity-only surprise:
  artifact_oracle_logits != name_resolver_logits on the
    canonical_naming fixture                                 ⇒ Surprise,
                                                               not Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  ArtifactOracle is silently using a brittle fallback path. Per D12
  this is a closure-blocking implementation defect. Block S6.

Note:
  H6 is a deliberate implementation-direction predicate; the
  adversarial fixture is owned by gbf-experiments::s3::oracle and
  gated by the test-only `s3-oracle-adversarial` feature.
```

## H7 F4 phase scheduler closure (carry-through)

```text
Statement:
  Re-affirms the F-S2 H4 Phase-A cleanliness verdict and the F-S2 H1
  substrate-survival verdict for all six F-S2 hypotheses
  (S2Hypothesis::{H1, H2, H3, H4, H5, H6}), and additionally asserts
  that Phases A, B (degenerate for the dense baseline), C, D produce,
  for every seed s ∈ {0,1,2,3,4}, the required Phase-A frozen dense
  teacher checkpoint at step 4000 and the Phase-D hard ternary student
  checkpoint at step 10000 such that:
    (a) both checkpoints reproduce bit-identically across replay (D8);
    (b) the run uses the F-S2-pinned phase scheduler semantics
        (phase_plan, HardnessRampS2::PhaseCRampD2PlusPhaseDRampD2,
        per-component HardnessTriple recorded at each phase boundary),
        as witnessed by s2_train_config_hash, the loss-term activation
        log, and the QuantHardness ramp log;
    (c) the s2_distillation_log.v1 records non-empty distill_loss
        histograms during Phase C (steps 5001..8000) and Phase D
        (steps 8001..10000), and the s2_phase_log.v1 emits the
        teacher_freeze event at exactly the boundary between step
        4000 and step 4001;
    (d) the QuantHardness ramp recorded in the run log matches the
        pinned D2 schedule:
          Phase C k = global_step - 5000:
            k ≤ 1000:               expert_qat = Off  (soak)
            1000 < k ≤ 2000:        expert_qat = Soft
            k > 2000:               expert_qat = Hard
            activation_qat = Off,   norm_qat = Off
          Phase D k = global_step - 8000:
            expert_qat = Hard
            k ≤ 500:                activation_qat = Off,  norm_qat = Off
            500 < k ≤ 1000:         activation_qat = Soft, norm_qat = Soft
            k > 1000:               activation_qat = Hard, norm_qat = Hard.

Predicted:
  ∀ s. teacher_checkpoint_sha(step=4000, s)  reproducible across replay
  ∀ s. student_checkpoint_sha(step=10000, s) reproducible across replay
  for all seeds:
    train_config_hash(s)           == s3_train_config_hash    ; D3 / Rep-S2-4
    s2_environment_hash(s)         == s3_environment_hash     ; D8 / D11
    distill_loss_histogram(phase=C, s).sum > 0
    distill_loss_histogram(phase=D, s).sum > 0
    quanthardness_ramp_recorded(s) == HardnessRampS2::PhaseCRampD2PlusPhaseDRampD2
    teacher_freeze_event_recorded(s).step == 4001            ; S2 S2-Run-Ok-8

Falsification:
  any seed's replay produces a checkpoint sha mismatch       ⇒ Refuted
  any seed's train_config_hash disagrees with s3_train_config_hash
                                                              ⇒ Refuted
  any seed's S2EnvironmentHash field disagrees across replay  ⇒ Refuted
  any phase's distillation loss histogram is empty when the
    phase contract requires it to be active                  ⇒ Refuted
  the recorded HardnessRampS2 variant ≠ PhaseCRampD2PlusPhaseDRampD2
                                                              ⇒ Refuted
  the recorded HardnessTriple at any boundary deviates from the
    D2 schedule above                                         ⇒ Refuted
  teacher_freeze event fires more than once or at a step ≠ 4001
                                                              ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  F4 cannot close at S3. Open follow-up under F4 phase contract;
  block S6.
```

Hypothesis composition rules are formalized in §10 (Outcome algebra).

---

# 2. Authority rules

```text
Scope(F-S3) =
  {
    H1, H2, H3, H4, H5, H6, H7,
    charset_v1 normalization (S3 schema instance; pipeline owner is F-G1),
    TinyStories.v2 manifest with charset_v1 normalization,
    5-gram Kneser-Ney baseline math,
    bpc_char primitive (S3 instance, vocab=80),
    v0_success WorkloadManifest schema instance (S3 instance; F-C4 schema upstream),
    ReferenceModelBundle export operation,
    ModelArtifact export operation,
    DenotationalOracle replay operation,
    ArtifactOracle replay operation,
    Phase-specific exported-surface agreement gate procedure,
    ConformanceEnvelope emission procedure,
    s3_charset_v1.v1, s3_baseline_kn5.v1, s3_bundle.v1,
    s3_artifact.v1,
    s3_oracle_agreement.v1, s3_v0_success.v1, s3_conformance.v1,
    s3_oracle_re_run.v1, s3_report.v1
  }

Rule Authority:
  ∀ behavior b ∈ Scope(F-S3) ∧ this RFC specifies b
  ⇒ SourceOfTruth(b) = this RFC.

Rule InheritanceFromS1:
  All F-S1 Authority rules, Reproducibility laws Rep-1..Rep-8,
  pre-registration discipline, falsification suite pattern, S1CanonicalJson
  encoding, DomainHash function, CanonicalTensorPayloadHash function, and
  the per-seed isolation property are inherited by F-S3 unchanged EXCEPT
  where this RFC explicitly amends:

    F-S1 D1   raw bytes              → AMENDED by F-S3 D1 (charset_v1)
    F-S1 D3   train budget bytes     → AMENDED by F-S3 D3 (chars; phase plan
                                          taken from F-S2 D3 with shortened
                                          Phase B for dense baseline)
    F-S1 D3a  bpc per byte           → AMENDED by F-S3 D3 (bpc per char)
    F-S1 D4   3-gram add-α           → AMENDED by F-S3 D4 (5-gram KN, D-rule)
    F-S1 D5   raw byte split         → AMENDED by F-S3 D5 (post-normalization
                                          char split; old byte-sha256 retained
                                          for traceability)
    F-S1 D7   metric-oracle suite    → AMENDED by F-S3 D7 (three-way oracle
                                          agreement; the F-S1 metric oracle
                                          suite remains required for the bpc
                                          primitive at vocab=80)
    F-S1 D6   strict per-seed bpc    → AMENDED by F-S3 D6 (composite
                                          per-seed predicate Q1..Q6)

  All other F-S1 Authority rules are re-affirmed:
    D2 fixed seed list, D8 strict reproducibility, D9 fail-closed on NaN,
    D10 optimizer pinned.

Rule InheritanceFromS2:
  All F-S2 Authority rules governing the F4 phase scheduler, QuantHardness
  ramp pinning, ternary student tracks teacher (≤0.5 bpc gap on Toy0),
  Burn LinearState gradient smoke, standard loss term gradient flow,
  measurement-oracle re-run discipline, falsification-suite + api-drift
  + preregistration discipline, RngStreams disjointness (S2RngStreams =
  { InitRng, BatchRng, ShuffleRng, ThresholdInitRng } via
  seed128(domain, seed)), and structured logging adoption in producers
  are inherited by F-S3 unchanged.

  PhaseBudget_S3 = PhaseBudget_S2 (D3 above). S3 does not introduce a
  separate "phase_schedule_hash" object; instead, S3 reuses F-S2's
  `train_config_hash` (Rep-S2-4) for the inherited training substrate
  and chains in the S3-specific inputs:

    s3_train_config_hash =
      DomainHash("s3_train_config.v1",
        {
          s2_train_config_hash,                ; binds D1+D3+D5+D10+D13 of F-S2
                                               ; — phase_plan, hardness_ramp_id =
                                               ; HardnessRampS2::PhaseCRampD2PlusPhaseDRampD2,
                                               ; distill_temp = 2.0,
                                               ; lambda_distill_default = 1.0,
                                               ; lambda_range = 0.01,
                                               ; lambda_zero = 0.0001,
                                               ; range_safe_lo = -1.0, range_safe_hi = 1.0,
                                               ; threshold_init_multiplier = 0.7,
                                               ; teacher_freeze_step = 4000,
                                               ; per-build hardness/lambda overrides,
                                               ; phase-effective lambda table.
          charset_v1_sha256,                   ; LexicalSpec_v1 hash (D1 / §3)
          workload_self_hash,                  ; v0_success.toml (D5 / §6.2)
          export_visitor_hash,                 ; ExportVisitor identity (D11)
          quant_spec_hash,                     ; QuantSpec_S3 (D12 / §3)
          observation_policy_hash              ; ObservationPolicy_S3 (§6.2)
        })

  S2 environment hashes are also inherited verbatim:
    S3EnvironmentHash := S2EnvironmentHash extended only by oracle_backend_identity
      = { build_config_hash, rust_toolchain_hash, dependency_lockfile_hash,
          oracle_backend_identity }

  HardnessRampS2 currently has exactly one variant:
  PhaseCRampD2PlusPhaseDRampD2. S3 may NOT introduce a new ramp variant;
  doing so requires a coordinated F-S2 amendment.

  S3 amends only what's needed to add charset_v1, the 5-gram KN baseline,
  the v0_success workload, the bundle/artifact export operation, the
  phase-specific exported-surface oracle agreement gate, and the S3
  observation/QuantSpec hashes.

Rule PlanContext:
  Behavior outside Scope informed by planv0.md as of `0349b50` plus
  the 2026-05-06 amendment items 6 (v0 success envelope), 7 (artifact
  oracle gate scoped forward), and 8 (constant rename). Closed
  features F1, F3, F6, F12 (LinearStateBlock at Fixed(0.5)),
  T14.1 Toy0 ModelSizeProfile, F-B1 compute bringup, F-S1 First
  Pulse, and F-S2 QAT Survives (commit ca21eb4 "F-S2 QAT survives
  closure") provide the substrate; their contracts are not amended by
  this RFC except as listed under Rule InheritanceFromS1 and
  Rule InheritanceFromS2 above. The landed F-S2 surface — gbf-experiments
  s2 module tree (schema, scheduler glue, run, score, ablation, oracle
  re-run, falsify, environment, rng, distill, gap, verifiers, report,
  cli), gbf-train phase scheduler / FrozenTeacher / Burn LinearState
  block / loss composer / ternary QAT / range loss / zero loss, and
  the s2_*.v1 artifact schemas — is the concrete substrate S3 inherits.
  F4's phase-scheduler substrate is inherited from F-S2, but full F4
  closure is owned by H7 in this RFC and is not assumed before S3.

Rule CrateOwnership:
  Every behavior in Scope(F-S3) is implemented in exactly one of:
    - gbf-experiments       (s3_* operations, S3 falsification suite, schema
                              encoders, replay CLI entrypoints. The s3 module
                              tree mirrors the landed s2 module shape:
                              `s3::{schema, run, score, ablation,
                              oracle_re_run, falsify, environment, rng,
                              gap, verifiers, report, cli, manifest}` plus
                              S3-specific `charset`, `baseline`, `bundle`,
                              `artifact`, `oracle`, `conformance`. F-S1 +
                              F-S2 module trees continue to live here too.)
    - gbf-policy            (Toy0 ModelSizeProfile reference instance)
    - gbf-model             (LinearStateBlock with Fixed(0.5);
                              CHARSET_V1_VOCAB_TIE_DEFAULT_LIMIT constant
                              renamed per planv0 amendment item 8;
                              tied embedding/classifier sharing per bd-3bf1;
                              re-exports `gbf_model::qat::QuantHardness`
                              consumed by gbf-train and gbf-experiments::s2)
    - gbf-train             (TrainingPhaseSchedule, TrainPhaseSpec, AdamW
                              config, ExportVisitor, `qat`, `qat-ablation`,
                              `burn-adapter` feature flags, the
                              `freeze_teacher` operation already landed in
                              `gbf_train::teacher`, and a NEW
                              `freeze_student_as_artifact` operation that
                              S3 introduces for the step-10000 boundary;
                              the loss composer / distillation / range /
                              zero loss helpers landed in F-S2 are reused
                              by S3 unchanged)
    - gbf-data              (TinyStoriesManifest reader, charset_v1
                              normalization pipeline (F-G1), CorpusManifest)
    - gbf-artifact          (LexicalSpec, ReferenceModelBundle,
                              ReferenceProgram, ReferenceNumericProfile,
                              ArtifactCore, QuantSpec, CanonicalTensor,
                              ConformanceEnvelope schema, SemanticCheckpointSchema)
    - gbf-workload          (WorkloadManifest schema; v0_success.toml lives in
                              fixtures/workloads/, schema in this crate)
    - gbf-oracle            (DenotationalOracle, ArtifactOracle,
                              ReferenceScorer, ArtifactScorer,
                              phase-specific surface comparator helpers)
    - gbf-foundation        (Hash256, sha256 helper, SemVer; re-affirmed
                              from F-S2)
    - gbf-cli               (`gbf s3` subcommand for replay; S3Command verbs
                              mirror the F-S2 S2Command shape — see §16.4)
    - gbf-test              (cross-crate end-to-end pipeline tests, including
                              the F-S2-landed phase_tests.rs and loss_tests.rs;
                              S3 contributes a v0_success E2E test owned by
                              bd-1wd / T10.11)
  No S3-specific code lives outside this set. The crate-level ownership
  table is normative; module names within each crate are illustrative
  unless explicitly tagged Required in §16.

  Until gbf-oracle is staffed (F-C1 + F-C2 closure), gbf-experiments
  may host stub-quality forwarding shims that defer to in-crate
  fallbacks; CI fail-fast checks that any such shim is gated behind
  the `s3-oracle-fallback` feature, the fallback's resolution
  behavior is exercised by the H6 adversarial fixture, and the bead
  s3_report.v1 explicitly records `oracle_owner_bead = bd-1rcc /
  bd-c4wg` rather than asserting the real oracle is implemented.

Rule Amendment:
  Later slice changes any of:
    Toy0 dim caps
    bpc_char math
    5-gram KN baseline math
    seed list
    train budget (per phase)
    composite pass criterion Q1..Q6
    three-way agreement tolerance bands
    charset_v1 normalization order or vocab id assignment
    ExportVisitor identity
  ⇒ Later slice's RFC must explicitly amend this RFC.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation
  produces the expected Refuted verdict on the appropriate hypothesis.
  Falsification sensitivity is a first-class proof obligation
  (§14 O5). The S3 falsification suite has nine broken substitutes
  covering H1, H2, H3, H4, H5, H6, and H7.

Rule OracleOwnerNaming:
  Per CLAUDE.md "Oracle And Conformance Beads": when a real oracle is
  unavailable and a fixture-local fallback evaluator is used, the
  fallback MUST be named as a fallback in tests and closure, AND the
  s3_report.v1 must record the real oracle owner bead. For S3 the
  real owner beads are bd-1rcc (F-C1 DenotationalOracle) and
  bd-c4wg (F-C2 ArtifactOracle). A run using a named fallback may close
  S3 only as Pass-with-fallback-oracle / ProceedToS4-with-deferred-clause;
  it must not be reported as Pass-clean.

Rule QuantSpecResolution:
  ArtifactOracle MUST resolve deployable full-precision weights via
  QuantSpec::weight_quant. Tensor-id naming conventions are not a
  substitute. Per CLAUDE.md "Artifact evaluators must resolve
  deployable full-precision weights through QuantSpec::weight_quant,
  not by assuming tensor-id naming conventions." Falsification F4 (§14)
  pins this rule.

Rule LogitsAggregation:
  Quantization-gap and oracle-agreement metrics over token logits MUST
  aggregate per token / per vocab row. Softmaxing a whole prompt's
  concatenated logits as one distribution is a falsifiable defect
  (F8 in §14), per CLAUDE.md "Quantization-gap metrics over token
  logits must aggregate per token/vocab row; do not softmax a whole
  prompt's concatenated logits as one distribution."

Rule ConformanceEnvelopeEmission:
  Per CLAUDE.md "In-memory metric JSON shape tests do not prove
  conformance.json emission. Name the report/conformance owner bead
  when report plumbing is not implemented." S3's
  s3_conformance.v1 IS the conformance.json emission for the
  v0_success workload, owned by this RFC; the report plumbing in
  gbf-report is owned upstream by F-C4 (bd-35l3) and any feature gap
  must be recorded as a moved-owner bead in the report.
```

---

# 3. Core notation

```text
Hash256        := /^sha256:[0-9a-f]{64}$/                 ; inherited from F-S1
Seed           := u64                                       ; inherited from F-S1
TrainStep      := u32
EvalStep       := u32
PhaseId        := PhaseKindS2  ; one of phase-a | phase-b | phase-c | phase-d
                                ; (kebab-case serde from gbf-experiments::s2::schema)
LossNatsPerTokenLegacyName := f32                          ; raw natural-log CE per byte;
                                                              retained only as a legacy
                                                              emitter field name for
                                                              backward compatibility
                                                              with run_log emitters; the
                                                              S3 per-character/token training
                                                              loss is computed in nats and
                                                              converted to bpc_char only
                                                              at score time
BpcCharValue   := f64                                      ; required finite, ≥ 0;
                                                              all S3 pass/fail gates
                                                              compare f64
GradNorm       := f32                                      ; finite, ≥ 0

Verdict     := Confirmed | Refuted
HypothesisStatus :=
    Confirmed
  | Refuted
  | NotEvaluatedDueToPriorGate(reason: String)

S3FailureKind :=
    Charset
  | Baseline
  | Quality
  | OracleAgreement
  | Bundle
  | QuantSpec
  | Substrate
  | Phase
  | Suspicious

S3Outcome (anticipated; full enumeration in §10):
  Pass-clean | Pass-with-fallback-oracle | Fail-charset | Fail-baseline
  | Fail-quality | Fail-oracle-agreement | Fail-bundle | Fail-quantspec
  | Fail-substrate | Fail-phase | Fail-suspicious | Fail-falsification
  | Fail-api-drift | Fail-metric | Fail-preregistration | Fail-artifact
  | Fail-incomplete

S3BuildKind (kebab-case, mirrors F-S2 S2BuildKind shape):
    s3_v0_success_real_oracle      ; bundle + artifact export + real oracle backends
  | s3_v0_success_fallback_oracle  ; same training, but S3DenotationalFallback /
                                   ; S3ArtifactFallback evaluator
  | s3_oracle_adversarial          ; test-only; H6 adversarial fixture only

S3Decision (mirrors F-S2 S2Decision tagged-enum shape):
    ProceedToS4
  | ProceedToS4-with-deferred-clause   ; fallback oracle usage; see §10
  | Investigate { reason: String }
  | Halt { reason: String }

S3RngStreams (re-affirmed from F-S2 S2RngStreams; S3 declares no new streams):
  init           = InitRng(seed)            ; Pcg64Mcg(seed128("init", seed))
  batch          = BatchRng(seed)           ; Pcg64Mcg(seed128("batch", seed))
  shuffle        = ShuffleRng(seed)         ; Pcg64Mcg(seed128("shuffle", seed))
  threshold_init = ThresholdInitRng(seed)   ; Pcg64Mcg(seed128("threshold_init", seed))

  Per F-S2 D11, declaring (and only declaring) keeps the rng-stream
  contract explicit. S3 MUST NOT add new streams (e.g. for charset
  augmentation or prompt sampling) without an RFC amendment that
  appends a domain tag to S2RngStreams.

S3EnvironmentHash := S2EnvironmentHash extended only by oracle_backend_identity
  = { build_config_hash, rust_toolchain_hash, dependency_lockfile_hash,
      oracle_backend_identity }
```

```text
Charset_v1 (vocab = 80, identity-hashed):

  CharId         := u8 ∈ [0, 79]
  PrintableId    := CharId ∈ [0, 75]                       ; 76 printable (A..Z,a..z,0..9,
                                                              13 punctuation) + '\n' (id 75)
  ReservedId     := 76                                      ; reserved for v1.1; rejected
                                                              if seen in input
  ControlId      := CharId ∈ {77, 78, 79}                   ; <bos>=77, <eos>=78, <unk>=79

  TextCharSeq    := Vec<CharId> where every id ∈ {0..75} ∪ {79}
                   ; normalized corpus and prompt text. <bos>, <eos>,
                   ; and reserved id 76 are forbidden.

  ModelTokenSeq  := Vec<CharId> where every id ∈ {0..75} ∪ {77,78,79}
                   ; model-side sequences may contain <bos>/<eos>.
                   ; reserved id 76 remains forbidden.

  CharSeq        := TextCharSeq unless explicitly stated otherwise.
  CharLen        := u64                                     ; byte_length is NOT a synonym

  LexicalSpec_v1 :=
    {
      schema:           "lexical_spec.v1"
      charset:          [Char; 80]                           ; ordered table; printable
                                                              entries are ASCII codepoints,
                                                              control entries carry their
                                                              token name
      normalization:    NormalizationSpec_v1
      control_tokens:   { bos: 77, eos: 78, unk: 79 }
      lexical_self_hash: Hash256                              ; over canonical encoding;
                                                              part of ArtifactCore identity
    }

  NormalizationSpec_v1 :=
    {
      schema:           "normalization_spec.v1"
      order:            ["nfc", "strip_combining_accents", "preserve_case",
                          "fold_quotes_and_dashes", "whitespace", "unmappable"]
                                                              ; pinned, not configurable
      max_unmappable_pct_per_example: 2.0                      ; %, drop above
      reserved_id_in_input_policy:    "reject"                 ; CharId 76 is forbidden
                                                              ; in input; emitting it
                                                              ; aborts the loader
      normalization_self_hash:         Hash256
    }
```

```text
KnEffectiveCounts:
  C_5(h, w) is the raw surface 5-gram count c(h, w).

  For k ∈ {2, 3, 4}, C_k(h, w) is the modified-KN continuation count
  used by P_KN_k, i.e. the number of distinct left contexts that precede
  the k-gram (h, w) in corpus_train_post. Context marginal counts and
  N_r continuation counts for order k are derived from C_k, not from raw
  surface k-gram counts.

  This convention is normative: every D-rule count-of-count n_j^{(k)}
  is computed over the effective count table C_k used by P_KN_k.

KnDiscounts (D-rule, per interpolated order k ∈ {2, 3, 4, 5}):
  Y_k     = n_1^{(k)} / (n_1^{(k)} + 2 * n_2^{(k)})
  D_1^{(k)}  = 1 - 2 * Y_k * (n_2^{(k)} / n_1^{(k)})
  D_2^{(k)}  = 2 - 3 * Y_k * (n_3^{(k)} / n_2^{(k)})
  D_3p^{(k)} = 3 - 4 * Y_k * (n_4^{(k)} / n_3^{(k)})

  where n_j^{(k)} is the number of distinct effective k-gram entries
  whose C_k count is exactly j.

KnConditional (modified KN, order n = 5):
  Notation:
    c(w_{i-n+1..i})            = count of the n-gram in corpus_train_post
    N1+(•w_{i-n+1..i-1}•)      = number of distinct preceding contexts of
                                  the (n-1)-gram (left continuation count)
    N1+(w_{i-n+1..i-1}•)       = number of distinct following words after
                                  the (n-1)-gram (right continuation count)
    N1+(••)                    = total number of distinct bigrams (used at
                                  the unigram level)

  P_KN_5(w_i | w_{i-4..i-1}) = max(c(w_{i-4..i}) - D[c(w_{i-4..i})], 0)
                                / c(w_{i-4..i-1})
                              + γ_5(w_{i-4..i-1}) * P_KN_4(w_i | w_{i-3..i-1})

  If c(w_{i-k+1..i-1}) = 0 for any queried order k ∈ {2,3,4,5},
  the scorer MUST back off directly to P_KN_{k-1}(w_i | suffix(w_{i-k+1..i-1}))
  with interpolation weight 1.0. Division by zero is forbidden.

  γ_5(w_{i-4..i-1}) = (D_1 * N1(w_{i-4..i-1}•) + D_2 * N2(w_{i-4..i-1}•)
                        + D_3+ * N3+(w_{i-4..i-1}•)) / c(w_{i-4..i-1})

  Lower-order P_KN_k(w_i | w_{i-k+1..i-1}) for k ∈ {2, 3, 4} use the
  continuation-count form appropriate to modified Kneser-Ney. For each
  k < 5, the "count" and context-count terms are continuation counts
  over distinct left contexts of the corresponding k-gram, not raw
  surface k-gram counts. The implementation MUST expose these derived
  counts in the hand-counted fixture report.

  P_KN_1(w_i) = N1+(•w_i) / N1+(••).

  D[c] = D_1 if c == 1
       = D_2 if c == 2
       = D_3+ if c >= 3

  All probabilities are computed in f64. f32 rounding is forbidden in the
  baseline.

KnReset:
  At a chunk boundary (chunk_size = 128 chars; reset between chunks),
  the scorer queries P_KN_k(w_i | ...) where k is min(intra_chunk_position
  + 1, 5). The first character in a chunk is scored with P_KN_1; the
  second with P_KN_2; ...; the fifth-and-after with P_KN_5.
```

```text
ReferenceModelBundle (S3 instance shape; full schema in §11):

  ReferenceModelBundle :=
    {
      schema:                      "reference_model_bundle.v1"
      manifest:                    ReferenceManifest
      numeric:                     ReferenceNumericProfile
      lexical:                     LexicalSpec_v1
      model:                       ReferenceModelSpec
      program:                     ReferenceProgram
      tensors:                     Vec<ReferenceTensor>
      decode:                      DecodeSpec
      tied_embedding_alias:        Option<TiedEmbeddingAlias>
      bundle_self_hash:            Hash256
    }

  TiedEmbeddingAlias :=
    {
      embedding_canonical_id:      CanonicalTensorId
      classifier_canonical_id:     CanonicalTensorId      ; equal to
                                                            ; embedding_canonical_id
                                                            ; when shared
      shared:                      bool
      classifier_view:             SameTensor | TransposedView
    }

  ReferenceProgram :=
    {
      opset:                       ReferenceOpsetId       ; pinned: opset_v1
      graph:                       ReferenceEvalGraph
      checkpoint_schema_hash:      Hash256
    }

  ReferenceNumericProfile :=
    {
      scalar_format:               ReferenceScalarFormat  ; pinned: F32
      reduction_order:             Some(ReductionOrderCanonical)
      reduction_order_policy:      Enforced
      rng:                         ReferenceRngProfile    ; pinned: NoRng
                                                            ; (Argmax decode)
      determinism:                 BitExact
    }

ModelArtifact (S3 instance shape):

  ModelArtifact :=
    {
      schema:                      "model_artifact.s3.v1"
      core:                        ArtifactCore
      lowerings:                   Vec<TargetDataLoweringArtifact>  ; empty at S3
      aux:                         ArtifactAux                      ; sparse at S3
      reference:                   Some(ReferenceLink)
      artifact_self_hash:          Hash256
      canonical_aux_payload_sha:   Hash256
    }

  ArtifactCore (S3 instance):
    {
      manifest:                    ArtifactManifest
      lexical:                     LexicalSpec_v1
      model:                       ModelSpec_S3
      quant:                       QuantSpec_S3
      sequence:                    SequenceSemanticsSpec
      tensors:                     Vec<CanonicalTensor>
      luts:                        Vec<LogicalLutSpec>      ; empty at S3
      decode_caps:                 DecodeCapabilitySet      ; { Argmax }
      tied_embedding_alias:        Option<TiedEmbeddingAlias>
    }

  QuantSpec_S3 owns weight_quant : Map<CanonicalTensorId, WeightQuant>.
  weight_quant(t) returns:
    Fp32                          only for explicitly non-deployable
                                  reference tensors, if such tensors are
                                  present as auxiliary payloads
    Ternary2 { row_scale, threshold, accumulator: I32,
               reduction_order: CanonicalIntegerThenScale }
                                  for every deployable post-Phase-D tensor

  ArtifactOracle MUST consult QuantSpec::weight_quant for every deployable
  tensor it dereferences. See D12 and Rule QuantSpecResolution.
```

```text
Phase-specific surface agreement procedure (formal):

  Inputs:
    seed s ∈ {0..4}
    prompt p ∈ first three of v0_success.prompts (in manifest order)
    phase φ ∈ {A, D}
    semantic_checkpoint sc ∈ {PostLogits, PostDecode}
    generated_step_count k ∈ {1..16}

  Procedure:
    if φ == A:
      a_train_A = run_live(frozen_teacher_seed_s_phase_A, prompt p, sc, k)
      a_bundle  = DenotationalOracle.evaluate(
                    bundle_seed_s_phase_A, prompt p, sc, k)

    if φ == D:
      a_train_D  = run_live(hard_ternary_student_seed_s_phase_D, prompt p, sc, k)
      a_artifact = ArtifactOracle.evaluate(
                     artifact_seed_s_phase_D, prompt p, sc, k)

  Comparison:
    if sc == PostLogits and φ == A:
      assert |a_train_A - a_bundle|_inf <= 4.0e-6
    if sc == PostLogits and φ == D:
      assert a_train_D == a_artifact            ; bitwise canonical reduction
    if sc == PostDecode:
      if φ == A:
        assert a_train_A.token == a_bundle.token
      if φ == D:
        assert a_train_D.token == a_artifact.token

  Aggregation policy (Rule LogitsAggregation):
    Comparisons are performed elementwise per logits row at each token
    position. No prompt-wide softmax-then-compare. ConformanceEnvelope
    metrics record per-token max-abs-diff and per-token KL only when
    both sides are aligned to the same canonical reduction order.

  Output: AgreementProduct (§11). The product records three surfaces, but
  closure gates only the phase-specific pairs:
    Phase A: live teacher ↔ bundle
    Phase D: live student ↔ artifact
  Bundle ↔ artifact is report-only.
```

```text
DomainHash, Self-hash rule, CanonicalTensorPayloadHash, CanonicalCheckpointWrite,
S1CanonicalJson, Prediction status rule:
  Inherited verbatim from F-S1 §1, exactly as F-S2 inherited them
  (F-S2 §1 line 561 explicitly does NOT introduce S2CanonicalJson;
  S3 follows the same discipline and does NOT introduce S3CanonicalJson).
  All s3_*.v1 artifacts are encoded using S1CanonicalJson. S3 introduces
  four additional
  canonical write rules:

  CanonicalBundleWrite:
    For any ReferenceModelBundle byte-equality claim, tensors are
    serialized in ascending CanonicalTensorId order, manifest fields are
    encoded with S1CanonicalJson, ReferenceProgram graph nodes are
    serialized in topological-sort order with deterministic tie-breaking
    on op_id, no timestamp/host path/build duration/iteration order may
    appear, and tied_embedding_alias's classifier_canonical_id is
    written as a CanonicalTensorId reference, never as a duplicated
    tensor payload.

  CanonicalArtifactWrite:
    Identical to CanonicalBundleWrite, applied to ModelArtifact bytes;
    ArtifactAux mutable sidecars are excluded from the artifact_self_hash
    and instead summarized by canonical_aux_payload_sha.

  CanonicalConformanceWrite:
    For s3_conformance.v1, per-prompt agreement records are written in
    manifest prompt order; per-checkpoint metric maps are written with
    sorted keys; per-metric float values are encoded using S1CanonicalJson
    (shortest round-trip decimal, -0.0 normalized).

  CanonicalKnCountsWrite:
    For s3_baseline_kn5.v1's counts_blob_sha256, effective count tables
    C_2, C_3, C_4, and C_5 are serialized in ascending order of:
      (order, context_token_tuple_lexicographic, target_token_id).
    Counts are encoded as unsigned little-endian u64. No hash-map
    iteration order, host path, timestamp, build duration, or compression
    metadata may appear in the counts blob.
```

---

# 4. Authority rules — explicit S1 amendment summary

This section is normative and exists so a reviewer can verify, line by line,
which F-S1 invariants S3 inherits and which it amends.

```text
| F-S1 rule / decision      | S3 status      | Amendment                                                    |
| ------------------------- | -------------- | ------------------------------------------------------------ |
| Authority                 | Re-affirmed    | Scope swapped to Scope(F-S3); rule is the same.              |
| PlanContext               | Re-affirmed    | Plan context block extended to include 2026-05-06 amendment. |
| CrateOwnership            | Extended      | Adds gbf-artifact, gbf-workload, gbf-oracle.                 |
| Amendment                 | Re-affirmed   | Same rule, fresh trigger list (§2 Rule Amendment).           |
| Falsification             | Re-affirmed   | Same rule; nine substitutes instead of six (§14 O5).         |
| Rep-1 Seed determinism    | Re-affirmed + | Adds bundle and artifact byte-equality (D8).                  |
| Rep-2 Cross-machine       | Re-affirmed   | Single-machine only.                                          |
| Rep-3 Corpus pinning      | Re-affirmed + | Adds charset_v1_sha to every artifact.                        |
| Rep-4 Train-config pinning| Re-affirmed + | Adds export_visitor_hash to the pinning set.                  |
| Rep-5 Pass-version pinning| Re-affirmed   | Same.                                                         |
| Rep-6 RFC revision pinning| Re-affirmed   | Same.                                                         |
| Rep-7 Per-seed isolation  | Re-affirmed   | Same.                                                         |
| Rep-8 No hidden inputs    | Re-affirmed   | Same.                                                         |
| D1 raw bytes              | AMENDED       | Replaced by F-S3 D1 (charset_v1).                             |
| D2 fixed seed list        | Re-affirmed   | Same five seeds.                                              |
| D3 fixed train budget     | AMENDED       | Adopts F-S2 phase budget; bpc-per-character substitution.     |
| D3a det batch sampling    | Re-affirmed + | Same plus the per-character substitution.                     |
| D4 3-gram add-α           | AMENDED       | Replaced by F-S3 D4 (5-gram KN, D-rule).                      |
| D5 fixed split            | AMENDED       | Post-normalization char split; old byte sha256 retained.      |
| D6 strict per-seed        | AMENDED       | Composite per-seed Q1..Q6.                                    |
| D7 metric oracle suite    | AMENDED       | Extended to three-way oracle agreement (Q1..Q4 of D7 still    |
|                           |               | required for the bpc primitive at vocab=80).                  |
| D8 strict reproducibility | AMENDED       | Adds bundle and artifact byte-equality.                       |
| D9 fail-closed on NaN     | Re-affirmed   | Same.                                                         |
| D10 optimizer pinned      | Re-affirmed   | Same; phase scheduler from F-S2 governs ramp.                 |

S2 inheritance summary (decision IDs from `history/rfcs/F-S2-qat-survives.md`):

| F-S2 surface                                | S3 status     | Amendment                                                                                |
| ------------------------------------------- | ------------- | ---------------------------------------------------------------------------------------- |
| D1 PhaseBudget (4000/1000/3000/2000=10000)  | Re-affirmed   | PhaseBudget_S3 := PhaseBudget_S2; integers cited verbatim in S3 D3.                      |
| D2 HardnessRampS2::PhaseCRampD2PlusPhaseDRampD2 | Re-affirmed | Pinned ramp consumed by H7.                                                              |
| D3 distill_temp=2.0, lambda_distill=1.0     | Re-affirmed   | Inherited via s2_train_config_hash.                                                      |
| D4 threshold_init_multiplier=0.7            | Re-affirmed   | Inherited via s2_train_config_hash.                                                      |
| D5 lambda_range=0.01, lambda_zero=0.0001,   | Re-affirmed   | Inherited via s2_train_config_hash.                                                      |
|     range_safe_lo=-1.0, range_safe_hi=1.0   |               |                                                                                          |
| D6 S2BuildKind matrix                       | Extended      | S3 introduces S3BuildKind (real/fallback/adversarial) with same kebab-case style.        |
| D7 metric oracle re-run                     | Re-affirmed + | S3 re-runs the inherited S1 + F-S2 oracle suites under the S3 binary; emits              |
|                                             |               | s3_oracle_re_run.v1 in the same shape as s2_oracle_re_run.v1.                            |
| D8 PhaseBoundaryHardnessProjection fixture  | Re-affirmed   | Substrate; not re-tested in S3 (D8 phase-transition integ already proves the boundary).  |
| D9 LinearState fixture (Fixed(0.5))         | Re-affirmed   | Substrate; not re-tested in S3.                                                          |
| D10 Inert-loss policy                       | Re-affirmed   | S3 charset_v1 / KN baseline / oracle-agreement helpers must follow the inert-loss rule:  |
|                                             |               | raw diagnostics are computed and validated even when configured weight = 0.              |
| D11 RngStreams declaration                  | Re-affirmed   | S3RngStreams = S2RngStreams; S3 declares no new streams.                                 |
| D13 Distillation form (forward KL, T^2 *)   | Re-affirmed   | Inherited via s2_train_config_hash.                                                      |
| Rep-S2-4 train_config_hash binding          | Extended      | s3_train_config_hash chains s2_train_config_hash with charset/workload/export inputs.    |
| Rep-S2-5 pass_version_S2                    | Re-affirmed   | s3_report.v1 records both pass_version_S2 and pass_version_S3.                           |
| F-S2 H4 (Phase A cleanliness)               | Carry-through | H7 statement re-affirms the F-S2 H4 verdict for every seed at S3.                        |
| F-S2 falsification (F1-broken-S2..F6)       | Re-affirmed   | Defense-in-depth; S3 does not re-run, but S3 falsification numbers F1-broken-S3 onwards. |
| Burn LinearState gradient                   | Re-affirmed   | Substrate; not re-tested in S3.                                                          |
| Standard loss-term gradient flow            | Re-affirmed   | Substrate; not re-tested in S3.                                                          |
| Structured logging                          | Re-affirmed + | Producer adoption gated by bd-2sd7; S3 reports adoption proof for                        |
|                                             |               | ExportVisitor, oracle-replay, and bundle-export producers.                               |
| `gbf_train::scheduler::TrainingPhaseSchedule` | Re-affirmed | S3 consumes this scheduler verbatim; no S3 fork.                                         |
| `gbf_train::teacher::FrozenTeacher` /         | Re-affirmed | S3 ReferenceModelBundle export consumes the existing FrozenTeacher snapshot at the      |
|     `freeze_teacher`                         |               | step-4000 boundary; no S3 freeze re-implementation.                                       |
| F-S2 measurement-oracle / api-drift /        | Extended      | S3 mirrors the same gate template with `s3_*` script and test-target names.               |
|     preregistration / falsification CI scripts |             |                                                                                          |
```

---

# 5. Experiment state machine

```text
State :=
    Configured(corpus, model_config, train_config, baseline_config, workload, lexical_spec)
  | CharsetVerified(state)                              ; H1 inputs
  | BaselineFitted(state, kn5_baseline_product)          ; H2 inputs
  | TrainAttempted(state, run_products[5][phase])
  | Trained(state, completed_run_products[5][phase])
  | TeacherFrozen(state, completed_run_products[5][phase],
                  frozen_teacher[5])                    ; F4 mid-flow
  | Exported(state, bundles[5], artifacts[5])            ; H5 inputs
  | LiveObserved(state, train_observations[5])
  | OracleReplayed(state, denotational_runs[5], artifact_runs[5])
  | ThreeWayCompared(state, agreement_products[5][prompt_subset])
                                                          ; H4 + H6 inputs
  | Scored(state, val_bpc_fp[5], val_bpc_ternary[5],
                  generation_products[5][prompt])         ; H3 inputs
  | Reported(state, conformance_envelope, report)
  | Decided(state, decision: ProceedToS4
                          | ProceedToS4-with-deferred-clause
                          | Investigate(reason)
                          | Halt(reason))
```

Transitions:

```text
T0 configure:
  ∅ → Configured(c)

T1 charset:
  Configured(c) → CharsetVerified(c)
    requires: O-charset-roundtrip (§14 O-1) passes;
              manifest sha256 verifications hold;
              unmappable_example_drop_rate within [0, 0.02]

T2 baseline:
  CharsetVerified(c) → BaselineFitted(c, fit_kn5(c))
    requires: O-kn-oracle (§14 O-2) passes on the hand-counted fixture

T3 train:
  BaselineFitted(c, _) → TrainAttempted(c, [s3_train_run(c, s, all_phases) for s in seeds])

T3a all completed:
  TrainAttempted(c, runs) ∧ ∀ r ∈ runs. ∀ φ ∈ {A,B,C,D}.
    r[φ].completion = Completed
  → Trained(c, runs)

T3b divergence short-circuit:
  TrainAttempted(c, runs) ∧ ∃ r, φ. r[φ].completion = DivergedAt(_)
  → Reported(state, build_fail_substrate_report(state))

T3c teacher freeze (carry-through from F-S2 protocol):
  Trained(c, runs) → TeacherFrozen(c, runs,
                                   [freeze_teacher(runs[s], at = end_of_phase_A)
                                    for s in seeds])
    requires: H7 distillation_loss_histogram non-empty for Phase C and Phase D

T4 export:
  TeacherFrozen(c, runs, frozen) →
    Exported(c,
      bundles    = [export_reference_bundle(frozen[s])      for s in seeds],
      artifacts  = [export_model_artifact(runs[s], phase=D) for s in seeds])
    notes: O-bundle/O-artifact determinism replay-pairs (§14 O-3/O-3')
           are evaluated after export by replaying the canonical write
           operation; they are not preconditions for producing the first
           export.

T4a live observations:
  Exported(c, bundles, artifacts) →
    LiveObserved(c,
      train_observations = [capture_live_observations(runs[s],
                              phases = {A, D},
                              prompts = v0_success.prompts[0..3],
                              steps = 1..16)
                            for s in seeds])

T5 oracle replay:
  LiveObserved(c, train_observations) →
    OracleReplayed(c,
      denot      = [DenotationalOracle.evaluate(bundles[s], workload, observation_policy)
                    for s in seeds],
      artif      = [ArtifactOracle.evaluate(artifacts[s], workload, observation_policy)
                    for s in seeds])

T6 three-way compare:
  OracleReplayed(...) →
    ThreeWayCompared(...,
      agreement = [for s, p ∈ v0_success.prompts[0..3].
                     three_way_agreement(s, p, A and D)])
    requires: O-oracle-agreement-tolerance (§14 O-4) passes

T7 score:
  ThreeWayCompared(...) →
    Scored(...,
      val_bpc_fp      = [score_bpc_char(ReferenceScorer(bundles[s]),    val_post) for s in seeds],
      val_bpc_ternary = [score_bpc_char(ArtifactScorer(artifacts[s]),   val_post) for s in seeds],
      generation      = [for s. for p.
                           generate(ArtifactDecoder(artifacts[s]),
                                    prompt p,
                                    max_chars=256,
                                    stop_on_eos=true)])

T8 report:
  Scored(...) → Reported(state, build_envelope_and_report(state))

T9 decide:
  Reported(state, r) → Decided(state, decide(r))
```

Invariants:

```text
I-S3-1
  T1 must precede T2; T2 must precede T3; T3 must complete before T3c.

I-S3-2
  T3c must produce a frozen teacher checkpoint at the end of Phase A
  for every seed; export_reference_bundle takes the frozen teacher,
  not a Phase-D dense snapshot.

I-S3-3
  T4 emits exactly one ReferenceModelBundle and exactly one
  ModelArtifact per seed. Both go through canonical write rules
  (CanonicalBundleWrite, CanonicalArtifactWrite).

I-S3-4
  T5 must run DenotationalOracle on the bundle (NOT on live training
  weights) and ArtifactOracle on the artifact (NOT on the bundle).
  Confusing the two is a falsifiable defect.

I-S3-5
  T6 must check at least the first three prompts in v0_success.prompts
  in manifest order, at PostLogits and PostDecode, for both Phase A
  (dense teacher) and Phase D (hard ternary student).

I-S3-6
  T7's val_bpc_ternary is computed by s3_score_bpc_char using the
  ArtifactScorer/ArtifactOracle evaluator over the val_post character
  sequence, NOT by re-running the live training model with hard ternary.
  This proves the deployed artifact's quality, not the training model's.

I-S3-7
  T8 emits exactly one s3_conformance.v1 instance and one s3_report.v1
  instance per S3 PR. Re-runs after RFC amendment produce a new report
  with bumped rfc_revision.

I-S3-8
  Decided is final: closure of bd-3k8o is gated on
  Decision ∈ {ProceedToS4, ProceedToS4-with-deferred-clause}.
```

---

# 6. Workload + corpus contract

## 6.1 charset_v1 normalization (S3 schema instance)

```text
CharsetInputs :=
  {
    raw_train_bytes:  ByteSeq                  ; sha256 pinned to F-S1 manifest
    raw_val_bytes:    ByteSeq                  ; sha256 pinned to F-S1 manifest
    spec:             LexicalSpec_v1            ; D1 / §3
  }

CharsetProduct :=
  {
    train_post:                  CharSeq
    val_post:                    CharSeq
    train_post_sha256:           Hash256
    val_post_sha256:             Hash256
    charset_v1_sha256:           Hash256        ; lexical_self_hash
    unmappable_example_drop_rate_train: f64
    unmappable_example_drop_rate_val:   f64
    unmappable_char_drop_rate_train:    f64
    unmappable_char_drop_rate_val:      f64
    drop_log:                    Vec<DropEvent> ; per-example reason codes
    charset_self_hash:           Hash256
  }

operation s3_charset_v1
  input:  CharsetInputs
  output: CharsetProduct

Preconditions:
  Ch-Pre-1  raw bytes sha256 matches the F-S1 tinystories.toml manifest.
  Ch-Pre-2  spec equals D1's pinned LexicalSpec_v1 instance exactly.

Postconditions:
  Ch-Ok-1   ∀ c ∈ train_post ∪ val_post. c ∈ {0..75} ∪ {79}.
            ids 76, 77, and 78 are forbidden in normalized corpus
            streams.
  Ch-Ok-2   normalize_tokens(normalize_raw(x).tokens)
              = normalize_raw(x).tokens
            on every fixture input listed in
            fixtures/corpora/charset_v1_idempotence/*.
  Ch-Ok-3   sha256(train_post) = train_post_sha256
            sha256(val_post)   = val_post_sha256
            and these match tinystories.v2.toml's pins.
  Ch-Ok-4   unmappable_example_drop_rate_train ≤ 0.02
            unmappable_example_drop_rate_val   ≤ 0.02
            (D1 hard cap; H1 falsification at > 0.02).

Failure:
  Ch-Fail-1 reserved id 76 emitted into post stream:
            abort the loader before any tensor allocation.
  Ch-Fail-2 sha256 mismatch:
            abort the loader before scoring.
```

## 6.2 v0_success workload

```text
WorkloadManifest_v0 (S3 schema instance) :=
  {
    schema:               "workload_manifest.v1"
    id:                   "v0_success"
    class:                Conformance
    prompts:              Vec<PromptCase>      ; pinned, see fixture
    seeds:                [0, 1, 2, 3, 4]
    session:              SessionProfile_S3
    observation:          ObservationPolicy_S3
    execution:            ExecutionMatrix_S3
    acceptance:           AcceptanceMatrix_S3
    workload_self_hash:   Hash256
  }

PromptCase :=
  {
    id:                   PromptId
    prompt_chars:         CharSeq               ; 64..128 chars after charset_v1
    held_out_chapter_sha: Hash256               ; chapter that the prompt was sliced from
    expected_min_gen:     u32                   ; >= 128
    expected_max_repeat:  u32                   ; <= 8
    decode_mode:          Argmax
    rng_spec:             NoRng
  }

SessionProfile_S3 :=
  {
    decode:               { mode: Argmax }
    decode_transforms:    None
    transcript_policy:    None
  }

ObservationPolicy_S3 :=
  {
    checkpoints:          { PostEmbedding, PostLogits, PostDecode }
    trace_level:          Standard
    compare_domain:       LogitsF32CanonicalReduction
    determinism_requirement: BitExact
    agreement_trace:     { generated_steps: 16, stop_on_eos: false }
    checkpoint_roles:     {
      PostEmbedding: ObservationOnly,
      PostLogits:    AgreementGated,
      PostDecode:    AgreementGated
    }
  }

ExecutionMatrix_S3 :=
  {
    denotational:         true
    artifact:             true
    schedule:             false   ; deferred to S6
    harness:              false   ; deferred to S6 per D14
    hardware:             false   ; deferred to S8
  }

AcceptanceMatrix_S3 :=
  {
    live_phase_a_vs_bundle: Some(EnvelopeGate {
      max_per_token_logit_abs_diff_phase_A: 4.0e-6,         ; [ESTIMATE]
      argmax_token_must_match: true
    })
    live_phase_d_vs_artifact: Some(EnvelopeGate {
      max_per_token_logit_abs_diff_phase_D: 0.0,             ; bitwise
      argmax_token_must_match: true
    })
    bundle_vs_artifact:       ReportOnly                     ; quantization gap
    artifact_vs_schedule:     None                           ; deferred to S6
    schedule_vs_runtime:      None                           ; deferred to S6
    performance:              None
    experience:               None
    recovery:                 None
  }

Pinned prompt set:
  fixtures/workloads/v0_success.toml lists exactly N_v0 = 8 prompts
  drawn from the held-out chapter sha256-pinned by v0_success.toml.
  The chapter is NOT in train_post; the contamination check (§14 O-7)
  asserts no prompt's char sequence appears in train_post.

The first three prompts (in manifest order) are the surface oracle
agreement subset (D7).
```

## 6.3 5-gram Kneser-Ney baseline

```text
KnBaselineInputs :=
  {
    train_post:  CharSeq                       ; sha256-pinned
    val_post:    CharSeq                       ; sha256-pinned
    order:       5                              ; pinned
  }

KnBaselineProduct :=
  {
    bpc_kn5_val:                BpcCharValue
    bpc_kn4_val:                BpcCharValue   ; reported, not gating
    bpc_kn3_val:                BpcCharValue   ; reported, not gating
    bpc_kn2_val:                BpcCharValue   ; reported, not gating
    bpc_kn1_val:                BpcCharValue   ; reported, not gating
    discounts:                  Map<order ∈ {2,3,4,5}, KnDiscounts>
    counts_summary:             CountsSummary
    counts_blob_sha256:         Hash256
    baseline_self_hash:         Hash256
  }

operation s3_fit_kn5
  input:  KnBaselineInputs
  output: KnBaselineProduct

Preconditions:
  Bk-Pre-1  train_post sha256 matches tinystories.v2.toml.
  Bk-Pre-2  order = 5 exactly.
  Bk-Pre-3  char_length(train_post) ≥ 5.
  Bk-Pre-4  char_length(val_post)   > 0.

Postconditions:
  Bk-Ok-1   bpc_kn{1..5}_val are finite, ≥ 0.
  Bk-Ok-2   discounts.values() are finite and obey D-rule pre-registration:
              Y ∈ (0, 1);
              0 ≤ D_1 ≤ 1;
              0 ≤ D_2 ≤ 2;
              0 ≤ D_3+ ≤ 3.
  Bk-Ok-3   counts_summary is reproducible: same train_post sha256
              ⇒ same counts.
  Bk-Ok-4   bpc_kn5_oracle_fixture matches the hand-counted expected
              value within 1.0e-12 in f64 (H2).

Reported sanity, not invariants:
  bpc_kn5_val ≤ bpc_kn4_val ≤ bpc_kn3_val ≤ bpc_kn2_val ≤ bpc_kn1_val
```

## 6.4 bpc_char primitive (S3 instance)

```text
ScoreCharInputs :=
  {
    evaluator:    Evaluator                     ; either model under D7 reduction or
                                                  ; KN baseline; both are supplied with
                                                  ; the same chunk-reset semantics
    val_post:     CharSeq                        ; canonical val post-normalization
    chunk_size:   128                            ; characters
  }

ScoreCharProduct :=
  {
    bpc_char:         BpcCharValue
    char_count:       u64
    log2_sum:         f64
    score_self_hash:  Hash256
  }

operation s3_score_bpc_char
  input:  ScoreCharInputs
  output: ScoreCharProduct

Postconditions:
  Sc-Ok-1   bpc_char = log2_sum / char_count
  Sc-Ok-2   char_count = char_length(val_post) exactly
  Sc-Ok-3   log2_sum is computed in f64 over the entire val_post,
            then divided once at the end; no per-chunk rounding
  Sc-Ok-4   score is deterministic: same checkpoint + same val_post
            ⇒ same bpc_char.

Sliding-window evaluation:
  val_post is split into non-overlapping chunks of chunk_size = 128
  characters; sequence state resets to zero between chunks. The first
  character of each chunk is scored without context.

  The same chunked-reset semantics is used by the model scorer and by
  the KN baseline scorer, ensuring identical reset-context semantics on
  both sides of the H3 Q1 pass criterion.
```

---

# 7. Bundle export contract

```text
BundleExportInputs :=
  {
    frozen_teacher:        FrozenTeacherCheckpoint  ; safetensors blob with
                                                      ; canonical_tensor_payload_sha
    lexical_spec:          LexicalSpec_v1
    sequence_semantics:    SequenceSemanticsSpec
    decode_caps:           DecodeCapabilitySet       ; { Argmax }
    export_visitor_id:     ExportVisitorId
    determinism_class:     DeterminismClass          ; pinned: BitExact
  }

BundleExportProduct :=
  {
    bundle:                ReferenceModelBundle
    bundle_self_hash:      Hash256
    canonical_bundle_payload_sha:  Hash256
    program_validation:    ProgramValidationReport
  }

operation s3_export_reference_bundle
  input:  BundleExportInputs
  output: BundleExportProduct

Preconditions:
  Be-Pre-1  frozen_teacher is a `gbf_train::teacher::FrozenTeacher<M>`
            obtained from the existing `gbf_train::teacher::freeze_teacher`
            (or `TeacherFreezeGuard::freeze_with_logging`) at the
            S2_TEACHER_FREEZE_STEP = 4000 boundary; the S3 export does
            NOT re-implement freezing. The frozen snapshot's
            canonical_tensor_payload_sha matches the end-of-Phase-A
            teacher checkpoint hash recorded in the S3 RunProduct for
            the associated seed and s3_train_config_hash; the
            teacher_freeze event recorded in s3_phase_log.v1 fires at
            exactly the boundary between step 4000 and step 4001
            (S2 S2-Run-Ok-8 carry-through).
  Be-Pre-2  lexical_spec equals D1's pinned LexicalSpec_v1 instance.
  Be-Pre-3  sequence_semantics matches the model_config used during
            training (recorded in train_config_hash).
  Be-Pre-4  decode_caps = { Argmax }.

Postconditions:
  Be-Ok-1   bundle.lexical = lexical_spec.
  Be-Ok-2   bundle.numeric.scalar_format = F32 and
            bundle.numeric.reduction_order_policy = Enforced.
  Be-Ok-3   bundle.program is a valid ReferenceProgram in opset_v1
            and the program_validation report records exact agreement
            (within Phase A tolerance D7) between the program's
            evaluation on the v0_success three-way agreement subset
            and the live frozen_teacher's forward pass on the same
            prompts.
  Be-Ok-4   bundle.tied_embedding_alias.shared = true and
            bundle.tied_embedding_alias.embedding_canonical_id =
            bundle.tied_embedding_alias.classifier_canonical_id.
            (Required because vocab=80 ≤ tied-embedding limit;
            bd-3bf1 owns this representation.)
  Be-Ok-5   canonical_bundle_payload_sha is computed under
            CanonicalBundleWrite (§3); two replays with identical
            inputs produce identical canonical_bundle_payload_sha.
  Be-Ok-6   bundle_self_hash is computed by DomainHash over the
            canonical encoding of the bundle with bundle_self_hash
            omitted.

ArtifactExportProduct (sibling, same procedure but for the hard
ternary student post-Phase-D):
  artifact:                       ModelArtifact
  artifact_self_hash:             Hash256
  canonical_artifact_payload_sha: Hash256
  artifact_validation:            ArtifactValidationReport
                                    ; covers QuantSpec resolution
                                    ; and tied-embedding alias preservation

operation s3_export_model_artifact
  input:  ArtifactExportInputs (analogous to BundleExportInputs)
  output: ArtifactExportProduct

Preconditions:
  Ae-Pre-1  frozen_student is a snapshot produced by S3's NEW
            `gbf_train::teacher::freeze_student_as_artifact` operation
            at the S2_OPTIMIZER_STEPS = 10000 boundary; the snapshot is
            taken AFTER step 10000's optimizer update completes,
            mirroring the F-S2 teacher_freeze convention. The
            corresponding student_freeze event recorded in
            s3_phase_log.v1 fires at the boundary after step 10000
            (S3 analog of S2 S2-Run-Ok-8).
  Ae-Pre-2  artifact.core.quant matches s3_train_config_hash's
            quant_spec_hash input.
  Ae-Pre-3  decode_caps = { Argmax }.

Postconditions:
  Ae-Ok-1   artifact.core.lexical = lexical_spec.
  Ae-Ok-2   artifact.core.quant.weight_quant covers every Linear and
            Embedding/Classifier tensor in the canonical logical form.
  Ae-Ok-3   artifact.core.tensors include both the row-scale (Q8.8) and
            ternary payload tensors per TernaryWeightPlan; no dual
            "naive_fp32" companion tensor is emitted unless the test-only
            adversarial fixture is active.
  Ae-Ok-4   tied embedding/classifier sharing: artifact.core.tied_embedding_alias
            records one CanonicalTensor referenced by both the input
            embedding op and output classifier op in the artifact's
            canonical logical graph; not duplicated in payload.
```

---

# 8. Oracle contract

## 8.1 DenotationalOracle (S3 instance)

```text
DenotationalOracleInputs :=
  {
    bundle:                ReferenceModelBundle
    workload:              WorkloadManifest_v0
    observation_policy:    ObservationPolicy_S3
  }

DenotationalOracleProduct :=
  {
    observations:          ReferenceObservations
                             ; map (prompt_id, semantic_checkpoint, step) → Observation
    determinism_class:     BitExact
    oracle_self_hash:      Hash256
  }

operation gbf_oracle::denotational::evaluate
  input:  DenotationalOracleInputs
  output: DenotationalOracleProduct

Preconditions:
  Do-Pre-1  bundle.numeric.determinism = BitExact.
  Do-Pre-2  bundle.numeric.reduction_order_policy = Enforced.
  Do-Pre-3  workload.session.decode.mode = Argmax (no sampling).

Postconditions:
  Do-Ok-1   For every (prompt, checkpoint, step), observations contain
            an Observation whose required fields depend on checkpoint:
              PostEmbedding: hidden_state or embedding vector when requested
              PostLogits:    logits : Vec<f32> (length = vocab_size = 80)
              PostDecode:    token  : CharId
            Optional fields may be present, but agreement-gated fields
            MUST be present at their checkpoint.
  Do-Ok-2   Determinism: same bundle + same workload ⇒ byte-identical
            ReferenceObservations encoding under
            ReferenceObservationsCanonical (S1CanonicalJson with
            sorted keys and shortest f32 round-trip decimals).
  Do-Ok-3   Source of truth: observations are produced by evaluating
            bundle.program; the live training code path is not consulted.

Fallback (required by Rule OracleOwnerNaming):
  If gbf-oracle::denotational is not implemented, gbf-experiments may
  use a fixture-local fallback evaluator gated by the
  `s3-oracle-fallback` feature. The fallback evaluator MUST:
    - be named "S3DenotationalFallback" in tests and in the report
    - record real_owner_bead = bd-1rcc in s3_report.v1
    - exercise the bundle.program graph end-to-end (no shortcut to
      live training weights)
```

## 8.2 ArtifactOracle (S3 instance)

```text
ArtifactOracleInputs :=
  {
    artifact:              ModelArtifact
    workload:              WorkloadManifest_v0
    observation_policy:    ObservationPolicy_S3
  }

ArtifactOracleProduct :=
  {
    observations:          ArtifactObservations
                             ; same shape as ReferenceObservations
    determinism_class:     BitExact
    oracle_self_hash:      Hash256
    weight_resolution_log: Vec<{ tensor_id: CanonicalTensorId,
                                   resolved_via: "QuantSpec::weight_quant" }>
  }

operation gbf_oracle::artifact::evaluate
  input:  ArtifactOracleInputs
  output: ArtifactOracleProduct

Preconditions:
  Ao-Pre-1  artifact.core.quant.weight_quant is total over the
            CanonicalTensors that the ReferenceProgram graph consumes.
  Ao-Pre-2  workload.execution.artifact = true.
  Ao-Pre-3  workload.session.decode.mode = Argmax.

Postconditions:
  Ao-Ok-1   For every CanonicalTensor consumed by the canonical logical
            form, the resolution path goes through QuantSpec::weight_quant.
            weight_resolution_log records the mapping per tensor.
            Falsification F4 (§14) asserts that bypassing this path is
            detected.
  Ao-Ok-2   Determinism: same artifact + same workload ⇒ byte-identical
            ArtifactObservations encoding.
  Ao-Ok-3   No tiling, bank, or layout assumption is made; the oracle
            consumes the canonical logical form (cf. planv0 §299).
  Ao-Ok-4   Tied-embedding sharing is honored: the embedding tensor and
            classifier tensor are resolved from the same CanonicalTensor
            (alias check via artifact.core.tensors lookup +
             ReferenceProgram graph alias metadata).

Fallback (required by Rule OracleOwnerNaming):
  If gbf-oracle::artifact is not implemented, gbf-experiments may use
  the fixture-local fallback artifact evaluator already mentioned in
  bd-c4wg's handoff comment. The fallback MUST:
    - be named "S3ArtifactFallback" in tests and in the report
    - record real_owner_bead = bd-c4wg in s3_report.v1
    - resolve weights through QuantSpec::weight_quant (this is exactly
      what bd-c4wg's handoff comment instructs)
    - preserve activation passthrough semantics
    - keep checkpoint observations at PostEmbedding / PostLogits /
      PostDecode (PostRouter and PostExpertDowncast are absent from
      the dense baseline; reserved for S7)
```

## 8.3 Three-way agreement comparator

```text
operation gbf_oracle::three_way::compare
  input:  TrainObservations,
          DenotationalOracleProduct,
          ArtifactOracleProduct,
          AgreementPolicy
  output: AgreementProduct

AgreementPolicy :=
  {
    phase:                                     PhaseId
    max_logit_abs_diff:                        f32       ; per (prompt, checkpoint, step)
    require_argmax_token_match:                bool
    aggregation:                               PerTokenPerVocabRow  ; Rule LogitsAggregation
  }

AgreementProduct :=
  {
    per_record:               Vec<{ prompt_id: PromptId,
                                    checkpoint: SemanticCheckpoint,
                                    step:       u32,
                                    phase:      PhaseId,
                                    train_vs_bundle_max_abs_diff:    Option<f32>,
                                    train_vs_artifact_max_abs_diff:  Option<f32>,
                                    bundle_vs_artifact_max_abs_diff: Option<f32>,
                                    train_vs_bundle_argmax_match:    Option<bool>,
                                    train_vs_artifact_argmax_match:  Option<bool>,
                                    bundle_vs_artifact_argmax_match: Option<bool> }>
    overall_pass:                              bool
    agreement_self_hash:                       Hash256
  }

Postconditions:
  Tw-Ok-1   For the Phase A subset:
              ∀ record. record.train_vs_bundle_max_abs_diff   <= 4.0e-6
              ∀ record. record.train_vs_bundle_argmax_match   = true
              (Phase A does not gate train_vs_artifact; the artifact
              denotes the Phase-D hard ternary student.)
  Tw-Ok-2   For the Phase D subset:
              ∀ record. record.train_vs_artifact_max_abs_diff = 0.0
                          (bitwise; canonical reduction order)
              ∀ record. record.train_vs_artifact_argmax_match = true
              (bundle_vs_artifact_max_abs_diff is the
              quantization/distillation gap and is NOT a closure gate;
              it feeds ConformanceEnvelope.)
  Tw-Ok-3   Aggregation: per-token, per-vocab-row absolute differences
            are reported. No prompt-wide softmax-then-compare.
            (Rule LogitsAggregation; F8 in §14.)
  Tw-Ok-4   overall_pass = true iff all per-record pre-registered
            tolerances hold.
```

---

# 9. v0_success workload contract

```text
operation s3_run_v0_success
  input:  artifacts: Vec<ModelArtifact>[5]
          bundles:   Vec<ReferenceModelBundle>[5]
          workload:  WorkloadManifest_v0
          val_post:  TextCharSeq
          baseline:  KnBaselineProduct
          chrome_budget: ConservativeChromeBudget
  output: V0SuccessProduct

V0SuccessProduct :=
  {
    per_seed:                  Vec<V0SuccessPerSeed>
    overall_pass:              bool
    v0_success_self_hash:      Hash256
  }

V0SuccessPerSeed :=
  {
    seed:                                Seed
    val_bpc_char_fp:                     BpcCharValue
    val_bpc_char_ternary:                BpcCharValue
    bpc_gain_vs_kn5:                     f64       ; bpc_kn5_val - val_bpc_char_fp
    bpc_quant_gap:                       f64       ; val_bpc_char_ternary - val_bpc_char_fp
    per_prompt_generation:               Vec<GenerationRecord>
    artifact_deployable_bytes:           u64
    fits_chrome_budget:                  bool

    Q1_holds:                            bool      ; bpc_gain_vs_kn5 > 0.05
    Q2_holds:                            bool      ; bpc_quant_gap <= 0.5
    Q3_holds:                            bool      ; charset validity = 1.0
    Q4_holds:                            bool      ; max_consecutive_same_token <= 8
    Q5_holds:                            bool      ; gen_chars >= 128 for every prompt
    Q6_holds:                            bool      ; artifact_deployable_bytes <= chrome_budget

    pass:                                bool      ; Q1 ∧ Q2 ∧ Q3 ∧ Q4 ∧ Q5 ∧ Q6
  }

GenerationRecord :=
  {
    prompt_id:                           PromptId
    generated_chars:                     TextCharSeq
    generated_char_count:                u32
    max_consecutive_same_token:          u32
    charset_validity_rate:               f64        ; expected 1.0 over
                                                     ; decoded text chars,
                                                     ; excluding terminal eos
    terminal_eos_seen:                   bool
    decode_mode:                         Argmax
    decode_log:                          Vec<{ step: u32, token: CharId, logit_max: f32 }>
  }

Procedure:
  For each seed s:
    For each prompt p:
      generated = artifact[s].decode_argmax(prompt p, max_chars = 256, stop_on_eos = true)
      record = GenerationRecord { ... }
    val_bpc_char_fp(s)      = s3_score_bpc_char(bundle[s].program, val_post)
    val_bpc_char_ternary(s) = s3_score_bpc_char(artifact[s].deployable, val_post)
    artifact_deployable_bytes(s) =
      sum byte_length(t.payload) for every artifact.core.tensor t whose
      role is DeployableWeight or DeployableQuantParam, plus canonical
      metadata bytes required to resolve those tensors through QuantSpec.
      ArtifactAux sidecars and non-deployable reference tensors are excluded.
    Q1..Q6 evaluated; pass = ∧.

Conservative chrome budget (D15):
  fixtures/runtime/chrome_budget.synthetic.toml records default_bytes for
  each synthetic RomBudgetSlot. S3 computes:

    conservative_chrome_budget_bytes =
      sum over RomBudgetSlot floor(0.90 * default_bytes)

  The 0.90 factor is applied exactly once.
  Real RuntimeChromeBudget consumption is owned by S6.

Acceptance:
  overall_pass = ∀ s. per_seed[s].pass = true
                  ∧ median(val_bpc_char_fp) ≥ 0.5      ; suspicious sentinel
```

---

# 10. Outcome algebra

```text
S3Outcome (PascalCase Rust enum; serde rename to the kebab-cased tags
shown below; full enumeration mirrors the F-S2 S2Outcome shape so the
S3 dispatcher reuses the same dispatch idiom):

    Pass-clean                 ; H1 ∧ H2 ∧ H3 ∧ H4 ∧ H5 ∧ H6 ∧ H7 all Confirmed
                                ; AND fallback_used is empty
                                ; AND every methodological-controls verifier
                                ; in the S3VerifierBundle (§10.1) passed
  | Pass-with-fallback-oracle  ; all closure-gating hypotheses Confirmed but
                                ; at least one oracle backend was the named
                                ; S3 fallback rather than the real F-C1/F-C2
                                ; oracle (§10 / Rule OracleOwnerNaming)
  | Fail-charset               ; H1 Refuted
  | Fail-baseline              ; H2 Refuted
  | Fail-quality               ; H3 Refuted, non-suspicious
  | Fail-suspicious            ; median(val_bpc_char_fp) < 0.5
  | Fail-oracle-agreement      ; H4 Refuted
  | Fail-bundle                ; H5 Refuted: bundle or artifact export
                                ; nondeterministic, malformed, or payload-
                                ; duplicating
  | Fail-quantspec             ; H6 Refuted (adversarial direction;
                                ; ArtifactOracle relied on naming)
  | Fail-substrate             ; any seed diverged in any phase
  | Fail-phase                 ; H7 Refuted (F4 cannot close at S3)
  | Fail-falsification         ; S3 falsification suite failed
                                ; (any of F1-broken-S3..F9-broken-S3 did
                                ; NOT produce its expected Refuted verdict)
  | Fail-api-drift             ; gbf-artifact / gbf-oracle / gbf-workload
                                ; public symbol drift
  | Fail-metric                ; inherited S1 + F-S2 oracle re-run regressed
                                ; under the S3 binary
  | Fail-preregistration       ; preregistration proof failed (R-Predictions
                                ; ancestry, predictions_section_hash mismatch,
                                ; or first_result_commit ordering violation)
  | Fail-artifact              ; required s3_*.v1 artifact missing or
                                ; self-hash invalid
  | Fail-incomplete            ; required non-gating artifact missing

Pass-with-fallback-oracle is reserved for runs where all closure-gating
hypotheses are Confirmed but at least one oracle backend was the named S3
fallback rather than the real F-C1/F-C2 oracle. It permits S3 closure only
with an explicit deferred clause and does not claim that the real oracle
implementation risk has been retired.

Therefore:
  Pass-clean retires v0_success denotation/artifact agreement risk against
  the real F-C1/F-C2 oracle implementations.

  Pass-with-fallback-oracle retires only the S3-local export/fallback-
  evaluator agreement risk. It does not retire real-oracle implementation
  risk and must be carried forward as a named deferred clause.
```

Combination (mandatory checks first):

```text
if not preregistration_passed                                  ⇒ Fail-preregistration
elif not artifact_integrity_passed                             ⇒ Fail-artifact
elif not falsification_s3_passed                               ⇒ Fail-falsification
elif not api_drift_check_passed                                ⇒ Fail-api-drift
elif not oracle_re_run_passed                                  ⇒ Fail-metric
elif ∃ seed s, phase φ. completion(s, φ) = DivergedAt(_)       ⇒ Fail-substrate
elif H1 verdict = Refuted                                      ⇒ Fail-charset
elif H2 verdict = Refuted                                      ⇒ Fail-baseline
elif H7 verdict = Refuted                                      ⇒ Fail-phase
elif H5 verdict = Refuted                                      ⇒ Fail-bundle
elif H6 verdict = Refuted (adversarial direction)              ⇒ Fail-quantspec
elif H4 verdict = Refuted                                      ⇒ Fail-oracle-agreement
elif suspicious_low_bpc                                        ⇒ Fail-suspicious
elif H3 verdict = Refuted                                      ⇒ Fail-quality
elif methodological_controls_present is false                  ⇒ Fail-incomplete
elif oracle_fallback_used is non-empty                         ⇒ Pass-with-fallback-oracle
else                                                            ⇒ Pass-clean
```

Decision dispatch:

```text
Pass-clean                  → Decision::ProceedToS4
Pass-with-fallback-oracle   → Decision::ProceedToS4-with-deferred-clause
                                (records fallback oracle usage; real
                                F-C1/F-C2 oracle implementation remains
                                an explicit deferred clause)
Fail-charset                → Decision::Halt { reason: "charset-broken" }
Fail-baseline               → Decision::Halt { reason: "baseline-broken" }
Fail-quality                → Decision::Investigate { reason: "quality-gap" }
Fail-suspicious             → Decision::Halt { reason: "audit-split-and-bpc-char" }
Fail-oracle-agreement       → Decision::Halt { reason: "oracle-disagreement" }
Fail-bundle                 → Decision::Halt { reason: "bundle-nondeterministic" }
Fail-quantspec              → Decision::Halt { reason: "quantspec-resolution-broken" }
Fail-substrate              → Decision::Investigate { reason: "burn-or-autodiff-or-phase" }
Fail-phase                  → Decision::Investigate { reason: "F4-phase-contract" }
Fail-falsification          → Decision::Halt { reason: "s3-falsification-suite" }
Fail-api-drift              → Decision::Halt { reason: "public-api-drift" }
Fail-metric                 → Decision::Halt { reason: "oracle-re-run-regressed" }
Fail-preregistration        → Decision::Halt { reason: "preregistration-proof" }
Fail-artifact               → Decision::Halt { reason: "artifact-self-hash" }
Fail-incomplete             → Decision::Investigate { reason: "missing-controls" }
```

`Halt` blocks bd-3k8o closure unconditionally. `Investigate` creates a
follow-up bead and may extend this RFC's scope.

## 10.1 S3VerifierBundle (mirrors F-S2 S2VerifierBundle)

The §10 dispatch ladder consumes a structured verifier bundle:

```text
S3VerifierBundle :=
  {
    preregistration_passed:           bool   ; scripts/s3_preregistration_check.sh
    artifact_integrity_passed:        bool   ; every required s3_*.v1 self-hash valid
    oracle_re_run_passed:             bool   ; inherited S1 + S2 oracle suites pass under
                                             ; the S3 binary (s3_oracle_re_run.v1)
    api_drift_check_passed:           bool   ; gbf-artifact + gbf-oracle + gbf-workload
                                             ; public-symbol snapshot match
    falsification_s3_passed:          bool   ; F1-broken-S3..F9-broken-S3 produce
                                             ; expected Refuted verdicts
    bundle_determinism_passed:        bool   ; O-3 (all five seeds)
    artifact_determinism_passed:      bool   ; O-3' (all five seeds)
    charset_idempotence_passed:       bool   ; O-1 + O-1'
    kn_oracle_passed:                 bool   ; O-2
    oracle_agreement_passed:          bool   ; O-4
    quantspec_resolution_passed:      bool   ; O-4'
    methodological_controls_present:  bool   ; every required non-gating artifact recorded
    suspicious_low_bpc:               bool   ; sentinel; gates Fail-suspicious
    completions:                      Vec<S3Completion>  ; per (seed, build_kind) cell
    hypothesis_statuses:              Map<S3Hypothesis, HypothesisStatus>
    oracle_fallback_used:             Vec<OracleFallbackTag>
  }

S3Hypothesis := H1 | H2 | H3 | H4 | H5 | H6 | H7  ; canonical closure order
S3Completion := Completed | DivergedAt { step: GlobalStep } | NotReached
```

A "closure-candidate" S3VerifierBundle has every `*_passed` true, every
hypothesis status Confirmed, and `suspicious_low_bpc = false`. The §10
dispatcher consumes this bundle deterministically; the same bundle bytes
always produce the same S3Outcome and S3Decision tag pair.

---

# 11. Artifact schemas

The S3 artifact set extends the F-S1 set. F-S1 schemas continue to live
under `experiments/S1/`; S3 schemas live under `experiments/S3/`.

## 11.1 s3_charset_v1.v1

```text
Path:
  experiments/S3/charset/charset-product.json

CharsetProductRecord (JSON) :=
  {
    schema:                       "s3_charset_v1.v1"
    raw_train_sha256:             Hash256
    raw_val_sha256:               Hash256
    train_post_sha256:            Hash256
    val_post_sha256:              Hash256
    charset_v1_sha256:            Hash256
    train_post_char_count:        u64
    val_post_char_count:          u64
    unmappable_example_drop_rate_train: f64
    unmappable_example_drop_rate_val:   f64
    unmappable_char_drop_rate_train:    f64
    unmappable_char_drop_rate_val:      f64
    drop_log_summary:             { reasons: Map<String, u64>,
                                    total_examples_dropped_train: u64,
                                    total_examples_dropped_val:   u64 }
    charset_self_hash:            Hash256
  }

Invariants:
  ChR-1  reserved id 76 never appears in either post stream.
  ChR-2  unmappable_example_drop_rate_{train,val} ≤ 0.02.
  ChR-3  charset_self_hash round-trips.
```

## 11.2 s3_baseline_kn5.v1

```text
Path:
  experiments/S3/baseline/kn5.bin
  experiments/S3/baseline/kn5-report.json

BaselineKnReport (JSON) :=
  {
    schema:                       "s3_baseline_kn5.v1"
    train_post_sha256:            Hash256
    val_post_sha256:              Hash256
    order:                        5
    discounts:                    Map<order, KnDiscounts>
    bpc_kn1_val:                  BpcCharValue
    bpc_kn2_val:                  BpcCharValue
    bpc_kn3_val:                  BpcCharValue
    bpc_kn4_val:                  BpcCharValue
    bpc_kn5_val:                  BpcCharValue
    counts_summary:               CountsSummary
    counts_blob_sha256:           Hash256
    baseline_self_hash:           Hash256
  }

Invariants:
  Bk-1   baseline_self_hash round-trips.
  Bk-2   ∀ k ∈ {1..5}. bpc_kn{k}_val finite, ≥ 0.

Reported (not invariant):
  bpc_kn5_val ≤ bpc_kn4_val ≤ bpc_kn3_val ≤ bpc_kn2_val ≤ bpc_kn1_val
```

## 11.3 s3_bundle.v1

```text
Path:
  experiments/S3/bundles/seed-{seed}/bundle.bin                  (binary canonical encoding)
  experiments/S3/bundles/seed-{seed}/bundle-metadata.json

BundleMetadata (JSON) :=
  {
    schema:                       "s3_bundle.v1"
    seed:                         Seed
    frozen_teacher_sha:           Hash256
    lexical_self_hash:            Hash256
    sequence_semantics_hash:      Hash256
    decode_caps:                  ["Argmax"]
    export_visitor_id:            String
    export_visitor_hash:          Hash256
    determinism_class:            "BitExact"
    bundle_self_hash:             Hash256
    canonical_bundle_payload_sha: Hash256
    program_validation:           {
      prompt_subset_pass:         bool
      max_logit_abs_diff:         f32
      argmax_token_all_match:     bool
    }
    tied_embedding_alias:         {
      shared:                     true
      embedding_canonical_id:     CanonicalTensorId
      classifier_canonical_id:    CanonicalTensorId  ; equals embedding id
    }
  }

Invariants:
  Bn-1   tied_embedding_alias.shared = true and the two canonical ids
         are equal.
  Bn-2   canonical_bundle_payload_sha is deterministic across replays.
  Bn-3   bundle_self_hash round-trips.
```

## 11.4 s3_artifact.v1 (sibling of bundle)

```text
Path:
  experiments/S3/artifacts/seed-{seed}/artifact.bin
  experiments/S3/artifacts/seed-{seed}/artifact-metadata.json

ArtifactMetadata (JSON) :=
  {
    schema:                       "s3_artifact.v1"
    seed:                         Seed
    student_checkpoint_sha:       Hash256
    lexical_self_hash:            Hash256
    quant_spec_hash:              Hash256
    decode_caps:                  ["Argmax"]
    export_visitor_id:            String
    export_visitor_hash:          Hash256
    artifact_self_hash:           Hash256
    canonical_artifact_payload_sha: Hash256
    canonical_aux_payload_sha:    Hash256
    artifact_deployable_bytes:    u64
    weight_resolution_summary: {
      total_tensors:                u32
      tensors_resolved_via_quant_spec: u32
      tensors_resolved_via_naming:     u32         ; must be 0
    }
    tied_embedding_alias: {
      shared:                       true
      embedding_canonical_id:       CanonicalTensorId
      classifier_canonical_id:      CanonicalTensorId  ; equals embedding id
      classifier_view:              SameTensor | TransposedView
    }
  }

Invariants:
  Ar-1   weight_resolution_summary.tensors_resolved_via_naming = 0.
  Ar-2   canonical_artifact_payload_sha deterministic across replays.
  Ar-3   tied_embedding_alias.shared = true and embedding_canonical_id =
         classifier_canonical_id. The payload is represented once.
```

## 11.5 s3_oracle_agreement.v1

```text
Path:
  experiments/S3/oracle/seed-{seed}/agreement.json

OracleAgreementReport (JSON) :=
  {
    schema:                       "s3_oracle_agreement.v1"
    seed:                         Seed
    workload_self_hash:           Hash256
    bundle_self_hash:             Hash256
    artifact_self_hash:           Hash256
    phase_a_records:              Vec<AgreementRecord>
    phase_d_records:              Vec<AgreementRecord>
    phase_a_pass:                 bool
    phase_d_pass:                 bool
    overall_pass:                 bool
    agreement_self_hash:          Hash256
    real_owner_bead_denotational: "bd-1rcc"
    real_owner_bead_artifact:     "bd-c4wg"
    fallback_used:                Vec<OracleFallbackTag>
                                                     ; [] when real oracles are wired
  }

OracleFallbackTag :=
    "S3DenotationalFallback"
  | "S3ArtifactFallback"

AgreementRecord :=
  {
    prompt_id:                            PromptId
    checkpoint:                           SemanticCheckpoint
    step:                                 u32
    train_vs_bundle_max_abs_diff:         Null | f32
    train_vs_artifact_max_abs_diff:       Null | f32
    bundle_vs_artifact_max_abs_diff:      Null | f32  ; expected quantization gap
    train_vs_bundle_argmax_match:         Null | bool
    train_vs_artifact_argmax_match:       Null | bool
    bundle_vs_artifact_argmax_match:      Null | bool
  }

Invariants:
  Oa-1   phase_a_pass = true requires every phase_a_record's tolerance
         to hold for non-null train_vs_bundle fields and every non-null
         train_vs_bundle argmax token to match. train_vs_artifact fields
         are null in Phase A unless a separate Phase-A artifact is
         explicitly exported.
  Oa-2   phase_d_pass = true requires every phase_d_record's
         non-null train_vs_artifact_max_abs_diff = 0.0 and every non-null
         train_vs_artifact argmax token to match. train_vs_bundle fields
         are null in Phase D unless a separate Phase-D bundle is explicitly
         exported.
  Oa-3   real_owner_bead_* fields are non-null even when the real
         oracle is wired (they document ownership).
```

## 11.6 s3_v0_success.v1

```text
Path:
  experiments/S3/v0_success/v0-success-report.json

V0SuccessReport (JSON) :=
  {
    schema:                       "s3_v0_success.v1"
    workload_self_hash:           Hash256
    baseline_self_hash:           Hash256
    chrome_budget_self_hash:      Hash256
    per_seed:                     Vec<V0SuccessPerSeed>
    overall_pass:                 bool
    v0_success_self_hash:         Hash256
  }
```

## 11.7 s3_conformance.v1

```text
Path:
  experiments/S3/conformance/conformance.json

ConformanceEnvelope (JSON, hierarchical) :=
  {
    schema:                       "s3_conformance.v1"
    workload_self_hash:           Hash256
    per_seed:                     Vec<SeedConformanceEnvelope>
    overall:                      EnvelopeGate    ; aggregate over all seeds
    quantization_gap_summary: {
      mean_per_token_max_abs_diff_phase_A: f32
      mean_per_token_max_abs_diff_phase_D: f32      ; ≈ quantization gap
      mean_per_token_kl:                   f32      ; per-token KL,
                                                       not prompt-wide
    }
    real_owner_bead:              "bd-35l3"      ; F-C4 ConformanceEnvelope
    conformance_self_hash:        Hash256
  }

SeedConformanceEnvelope :=
  {
    seed:                         Seed
    bundle_self_hash:             Hash256
    artifact_self_hash:           Hash256
    overall:                      EnvelopeGate
    per_checkpoint:               Map<SemanticCheckpoint, EnvelopeGate>
    per_metric:                   Map<MetricId, EnvelopeGate>
  }

Invariants:
  Co-1   every per_seed[*].per_checkpoint covers PostLogits and PostDecode.
         PostEmbedding is included only when the observation policy requests
         it and a tolerance is defined.
  Co-2   every per_seed[*].per_metric includes MaxAbsLogitDiff. PerTokenKL
         is included only for aligned logits rows where KL is well-defined.
  Co-3   overall.tolerance ≥ max per_seed[*].overall.tolerance.
  Co-4   conformance_self_hash round-trips.

Per Rule ConformanceEnvelopeEmission, this artifact IS the
conformance.json emission for v0_success at S3. The gbf-report
plumbing for hierarchical roll-up across multiple workloads is owned
upstream by F-C4 (bd-35l3); S3 does not own that roll-up but does
own this single-workload emission.
```

## 11.7a s3_oracle_re_run.v1 (mirror of s2_oracle_re_run.v1)

```text
Path:
  experiments/S3/oracle_re_run/oracle-re-run.json

OracleReRunReport (JSON) :=
  {
    schema:                       "s3_oracle_re_run.v1"
    s1_oracle_re_run_passed:      bool
    s2_oracle_re_run_passed:      bool
    per_metric:                   Map<MetricId, { s1_baseline: f64,
                                                  s2_baseline: f64,
                                                  s3_observed: f64,
                                                  delta_vs_s1:  f64,
                                                  delta_vs_s2:  f64,
                                                  passed:       bool }>
    oracle_re_run_self_hash:      Hash256
  }

Invariants:
  Or-1   s1_oracle_re_run_passed = true requires every per_metric
         delta_vs_s1 within the F-S1 D7 tolerance band for that metric.
  Or-2   s2_oracle_re_run_passed = true requires every per_metric
         delta_vs_s2 within the F-S2 O3 tolerance band for that metric.
  Or-3   oracle_re_run_self_hash round-trips.

Per F-S2's O3 discipline ("Re-run D7 oracle suite under S2 binary"),
S3 re-runs the inherited S1 + F-S2 oracle suites under the S3 binary.
A regression here surfaces as Fail-metric in the §10 dispatcher,
catching a class of contamination H4 alone may not (S2 RFC ambiguity
ledger AS2-16).
```

## 11.8 s3_report.v1

```text
Path:
  docs/experiments/S3-report.md

Front-matter (YAML, hashed into report):
  ---
  schema:                       "s3_report.v1"
  s3_outcome:                   S3Outcome
  decision:                     Decision
  charset_self_hash:            Hash256
  baseline_self_hash:           Hash256
  workload_self_hash:           Hash256
  conformance_self_hash:        Hash256
  v0_success_self_hash:         Hash256
  per_seed_artifacts:
    List[{
      seed: Seed,
      teacher_completion: Completed | DivergedAt(TrainStep) | NotReached,
      student_completion: Completed | DivergedAt(TrainStep) | NotReached,
      phase_completion: {
        A: Completed | DivergedAt(TrainStep) | NotReached,
        B: Completed | DivergedAt(TrainStep) | NotReached,
        C: Completed | DivergedAt(TrainStep) | NotReached,
        D: Completed | DivergedAt(TrainStep) | NotReached
      },
      teacher_checkpoint_self_hash: Null | Hash256,
      student_checkpoint_self_hash: Null | Hash256,
      bundle_self_hash:             Null | Hash256,
      artifact_self_hash:           Null | Hash256,
      agreement_self_hash:          Null | Hash256,
      generation_log_self_hash:     Null | Hash256
    }]
  oracle_owner_beads: { denotational: "bd-1rcc", artifact: "bd-c4wg" }
  oracle_fallback_used: List["S3DenotationalFallback" | "S3ArtifactFallback"]
                        ; empty list when no fallback was used
  oracle_re_run_self_hash:       Null | Hash256          ; from s3_oracle_re_run.v1
  conformance_owner_bead: "bd-35l3"
  e2e_test_owner_bead:    "bd-1wd"          ; bd-1wd remains open at S3 if
                                              ; the full E2E pipeline test is
                                              ; not yet adopted into the S3 PR
  structured_logging_owner_bead: "bd-2sd7"
  pass_version_S1:               String     ; inherited; reaffirmed by s3 oracle re-run
  pass_version_S2:               String     ; inherited; from F-S2 Rep-S2-5
  pass_version_S3:               String     ; new; bumped by §1 amendments per
                                            ; S3 analog of F-S2 Rep-S2-5
  s2_train_config_hash:          Hash256    ; from F-S2 Rep-S2-4
  s3_train_config_hash:          Hash256    ; from S3 Rule InheritanceFromS2
  s2_environment_hash:           S2EnvironmentHash
  s3_environment_hash:           S3EnvironmentHash
  s2_pinned_phase_schedule_hash: Hash256    ; HardnessRampS2 + boundaries
  generated_at_commit_time:      RFC3339 UTC timestamp of first_result_commit,
                                  informational only, excluded from report hash
  rfc_revision:                  GitCommitId | Hash256
  predictions_section_hash:      Hash256
  predictions_commit:            GitCommitId
  first_result_commit:           GitCommitId
  report_self_hash:              Hash256
  ---

Required sections (markdown body):
  ## Pre-registered predictions
    Predicted ranges, tolerance bands (D7), Q1..Q6 thresholds, and the
    H6 adversarial-direction expectation as committed before any S3
    result artifact commit. This section's content must appear in git
    history strictly before the first S3 result artifact commit.

  ## Observed
    Per-seed table:
      val_bpc_char_fp, val_bpc_char_ternary, bpc_quant_gap, Q1..Q6 results,
      teacher_completion, student_completion, agreement_pass_phase_A,
      agreement_pass_phase_D, fits_chrome_budget.
    Plus baseline numbers, charset summary, and aggregate statistics.

  ## Hypothesis verdicts
    H1, H2, H3, H4, H5, H6, H7 each as HypothesisStatus, with the
    concrete observation that drove each verdict.
    Closure-candidate reports must use only Confirmed | Refuted.
    Early-failure reports may use NotEvaluatedDueToPriorGate(reason)
    for hypotheses whose required observations do not exist because an
    earlier mandatory gate failed (e.g. T3b divergence short-circuit
    bypasses export, oracle replay, three-way comparison, and scoring).

  ## Falsification analysis
    Direct citation of which prediction or falsification rule fired for
    each Refuted hypothesis. The nine S3 broken substitutes (§14 O5)
    are referenced with their commit ids in gbf-experiments/tests.

  ## Surprises
    Anything outside predicted ranges, even if not a verdict change.
    Per-token quantization gap distributions go here.

  ## Decision
    Exactly one Decision tag, justified in ≤3 sentences.

  ## Reproducibility statement
    Exact command + manifest hashes + pass_version + ExportVisitor id
    + oracle owner beads + fallback usage + conformance owner bead.

Invariants:
  R-Decision         Exactly one Decision tag in front-matter.
  R-AllSeeds         per_seed_artifacts and the observed per-seed table
                     cover all 5 seeds in {0,1,2,3,4}.
  R-ClosureArtifacts For Decision ∈ {ProceedToS4,
                     ProceedToS4-with-deferred-clause}, every
                     teacher_checkpoint_self_hash, student_checkpoint_self_hash,
                     bundle_self_hash, artifact_self_hash, and
                     agreement_self_hash is non-null for all five seeds.
  R-Self-Hash        report_self_hash is computed over front-matter
                     (with generated_at_commit_time and report_self_hash omitted) and
                     markdown body bytes exactly as committed, using
                     S1CanonicalJson for front-matter normalization.
  R-Predictions      The commit introducing the exact "Pre-registered
                     predictions" section, identified by
                     predictions_section_hash, is a strict ancestor of
                     first_result_commit. first_result_commit is the
                     earliest commit introducing any of: charset_self_hash,
                     baseline_self_hash, bundle_self_hash, artifact_self_hash,
                     agreement_self_hash, or v0_success_self_hash derived
                     from S3 execution.
  R-AllHypotheses    All seven hypotheses have an explicit HypothesisStatus.
                     For Decision ∈ {ProceedToS4,
                     ProceedToS4-with-deferred-clause}, every status must
                     be a binary Verdict, not NotEvaluatedDueToPriorGate.
  R-OwnerBeads       oracle_owner_beads, conformance_owner_bead,
                     e2e_test_owner_bead, and structured_logging_owner_bead
                     are non-null whether or not the upstream owner is
                     fully implemented. They document ownership.
```

The pre-registration timestamp is itself a load-bearing artifact: predictions
written after-the-fact are not pre-registered, even if textually identical.

---

# 12. Reproducibility laws (S3 extension)

```text
Inherited from F-S1 §10:
  Rep-1 Seed determinism                                   (extended below)
  Rep-2 Cross-machine determinism is NOT required for v1   (re-affirmed)
  Rep-3 Corpus pinning                                     (extended below)
  Rep-4 Train-config pinning                               (extended below)
  Rep-5 Pass-version pinning                               (re-affirmed)
  Rep-6 RFC revision pinning                               (re-affirmed)
  Rep-7 Per-seed isolation                                 (re-affirmed)
  Rep-8 No hidden semantic inputs                          (re-affirmed)

Extended for S3:

Rep-1' Seed determinism + bundle/artifact byte-equality
  ∀ s ∈ {0,1,2,3,4}. replay(s, manifest) byte-identical to original(s, manifest)
       AND bundle_self_hash and canonical_bundle_payload_sha identical
       AND artifact_self_hash and canonical_artifact_payload_sha identical
       (under canonical write rules; ArtifactAux mutable sidecars
        excluded as defined in §11).

Rep-3' Corpus pinning + charset_v1
  Every s3_*.v1 artifact records:
    raw_train_sha256, raw_val_sha256                     ; pre-normalization
    train_post_sha256, val_post_sha256                   ; post-normalization
    charset_v1_sha256                                    ; LexicalSpec_v1 hash
  Replay validates all five sha256s against the on-disk manifest
  before proceeding.

Rep-4' Train-config + ExportVisitor pinning
  s3_train_config_hash chains s2_train_config_hash (binds F-S2
  D1+D3+D5+D10+D13 exactly per Rep-S2-4) with charset_v1_sha256,
  workload_self_hash, export_visitor_hash, quant_spec_hash, and
  observation_policy_hash; export_visitor_hash binds the ExportVisitor
  identity. Bumping any of these invalidates s3_bundle.v1 and
  s3_artifact.v1 instances. pass_version_S3 is bumped per the same
  discipline as F-S2 Rep-S2-5: any change to optimizer step semantics,
  Phase scheduler behavior, sequence-state forward, initialization rng,
  distillation form, threshold init formula, teacher freeze semantics,
  OR addition/removal of a phase, OR change to D2's ramp formula, OR
  addition of a new RNG sub-stream, OR change to the bundle/artifact
  export contract, OR change to the oracle agreement gate. pass_version_S3
  is independent of pass_version_S1 and pass_version_S2; all three are
  recorded in s3_report.v1.

New for S3:

Rep-9 Workload pinning
  v0_success.toml's workload_self_hash is recorded in every s3 artifact
  that consumes it (s3_oracle_agreement.v1, s3_v0_success.v1,
  s3_conformance.v1). Replay validates it against fixtures/workloads/.

Rep-10 Oracle ownership pinning
  Every s3_oracle_agreement.v1 records real_owner_bead for both
  denotational and artifact oracle, and the fallback_used field. A
  deferred-clause pass is permitted with a fallback only when the
  fallback evaluator's contract (§8.1, §8.2) is satisfied; the report
  must surface the fallback as a known limitation. Pass-clean requires
  real oracle backends.

Rep-11 Conformance emission ownership
  s3_conformance.v1 records conformance_owner_bead = "bd-35l3" so the
  hierarchical roll-up plumbing in F-C4 has a named handoff.
```

---

# 13. Decision protocol

```text
S3 closure (bd-3k8o) requires:
  1. All 5 seed runs Completed through Phase D (D9).
  2. s3_report.v1 emitted with R-Predictions verified by git history.
  3. Decision ∈ {ProceedToS4, ProceedToS4-with-deferred-clause}.
  4. charset_self_hash, baseline_self_hash, workload_self_hash,
     conformance_self_hash, v0_success_self_hash recorded in
     front-matter.
  5. Phase-specific exported-surface agreement gate (D7) passes for
     every seed at Phase A (tolerance) and Phase D (bitwise) on the
     pinned three-prompt subset.
  6. v0_success per-seed composite (D6 Q1..Q6) holds for every seed.
  7. ReferenceModelBundle and ModelArtifact replay byte-equality
     proven for all five seeds (Rep-1').
  8. Oracle owner beads recorded; fallback usage explicit if any.
  9. Tied embedding sharing preserved in both bundle and artifact
     (bd-3bf1 contract).
  10. F4 closure decision recorded (H7 Confirmed); any seed deviation
      from the S3-pinned train_config_hash, S2EnvironmentHash, or
      HardnessRampS2 ramp opens an investigation.

S3 closure is forbidden when:
  Any of:
    Decision::Halt(_), Decision::Investigate(_),
    missing pre-registration,
    any seed completion = DivergedAt(_),
    H1, H2, or H7 Refuted,
    H4 or H5 Refuted,
    median(val_bpc_char_fp) < 0.5 (suspicious sentinel),
    any required artifact missing or self-hash invalid,
    fallback used without satisfying the fallback evaluator's
      §8 contract,
    weight_resolution_summary.tensors_resolved_via_naming > 0
      in any artifact metadata.

If Decision = ProceedToS4-with-deferred-clause:
  Record the exact fallback oracle backend(s) used and open or retain
  follow-up beads under F-C1/F-C2 as appropriate:
    - bd-1rcc when S3DenotationalFallback was used;
    - bd-c4wg when S3ArtifactFallback was used.
  H6 canonical-fixture surprises are reported in the Surprises section
  but do not by themselves produce ProceedToS4-with-deferred-clause.
```

---

# 14. Proof obligations

```text
O-charset O-1 charset_v1 round-trip
  fixtures/corpora/charset_v1_idempotence/ contains a hand-curated
  set of inputs covering: NFC composed/decomposed, accented Latin,
  smart quotes, em/en dashes, ellipsis, tabs, CRLF, internal-space
  runs, leading/trailing whitespace, unmappable codepoints
  (Cyrillic, CJK, emoji), and a pre-normalization stream containing
  the literal "<|endoftext|>" separator. For every such input x:
    normalize_tokens(normalize_raw(x).tokens)
      == normalize_raw(x).tokens                       ; idempotence
    every output char ∈ {0..75} ∪ {79}                 ; corpus charset validity
    <bos> and <eos> never appear in normalized corpus streams
    reserved id 76 never appears                        ; D1
  Test target: gbf-experiments/tests/charset/idempotence.rs

  The idempotence fixture MUST include at least one example whose first
  normalization pass emits <unk>, to prove that normalize() accepts the
  already-tokenized <unk> representation without treating the literal
  string "<unk>" as source text on the second pass.

O-charset O-1' manifest sha256 verification
  Replay verifies tinystories.v2.toml's train_post_sha256 and
  val_post_sha256 against re-running normalize() on the pinned raw
  bytes. Any mismatch aborts loading.
  Test target: gbf-experiments/tests/charset/manifest_sha.rs

O-kn O-2 KN math oracle
  fixtures/baselines/kn_oracle/ contains a tiny corpus
  (~512 chars) with hand-counted unigram, bigram, ..., 5-gram counts
  and hand-computed continuation counts. The hand-computed expected
  bpc_char under D4's modified KN with D-rule discounts is recorded
  to 16 significant decimal digits. The implementation must match
  within 1.0e-12 in f64.
  Test target: gbf-experiments/tests/baseline/kn5_oracle.rs

O-bundle O-3 ReferenceModelBundle determinism
  Replay-pair: export the same frozen teacher checkpoint twice with
  the same ExportVisitor; assert canonical_bundle_payload_sha and
  bundle_self_hash byte-equal across replays for all five seeds.
  Test target: gbf-experiments/tests/bundle/determinism.rs

O-artifact O-3' ModelArtifact determinism
  Same as O-3 but for the hard ternary student artifact;
  canonical_artifact_payload_sha byte-equal across replays for all five seeds.
  ArtifactAux mutable sidecars excluded as documented.
  Test target: gbf-experiments/tests/artifact/determinism.rs

O-tied O-3'' Tied embedding alias preservation
  Both bundle and artifact must represent tied embedding/classifier
  as one CanonicalTensor referenced twice. Test enumerates tensor
  payload byte counts and asserts: total payload <= one_copy_payload
  + small_metadata_overhead.
  Test target: gbf-experiments/tests/bundle/tied_embedding_alias.rs

O-oracle O-4 three-way oracle agreement on pinned subset
  Per (seed s, prompt p ∈ first three of v0_success.prompts, phase φ ∈ {A, D}):
    agreement_record satisfies D7's tolerance. The test exercises both
    real-oracle and fallback-oracle code paths under the
    `s3-oracle-real` and `s3-oracle-fallback` features.
  Test target: gbf-experiments/tests/oracle/three_way.rs

O-quantspec O-4' QuantSpec resolution
  ArtifactOracle's adversarial fixture (test-only,
  `s3-oracle-adversarial` feature):
    fixture artifact contains a shadow tensor "linear_0_weight_naive_fp32"
    alongside the canonical "linear_0_weight". A name-resolver
    implementation (test-only) returns the shadow; the
    QuantSpec::weight_quant resolver returns the canonical. The test
    asserts:
      artifact_oracle_logits == quant_spec_resolver_logits
      artifact_oracle_logits != name_resolver_logits
  Test target: gbf-experiments/tests/oracle/quantspec_resolution.rs

O-v0success O-5 v0_success totality
  Every observable combination of per-seed binary Q1..Q6 verdicts maps
  to exactly one V0SuccessPerSeed.pass value and exactly one
  V0SuccessProduct.overall_pass value. The suspicion sentinel is
  recorded as a separate report field and is consumed by the §10
  S3Outcome dispatcher.
  Test target: gbf-experiments/tests/v0_success/outcome_totality.rs

O-conformance O-5' conformance.json round-trip
  s3_conformance.v1 round-trips through canonical JSON with self-hash
  equality, sorted keys, and per-token aggregation preserved.
  Per Rule LogitsAggregation, the test asserts metric values were not
  computed from a prompt-wide softmax.
  Test target: gbf-experiments/tests/conformance/round_trip.rs

O-falsification O-5'' Falsification suite (nine broken substitutes)
  Nine deliberately-broken implementations must each produce the
  expected Refuted verdict on the corresponding hypothesis. The
  identifiers `F{n}-broken-S3` mirror F-S2's `F{n}-broken-S2` shape
  and live in the unified `falsify`-feature suite alongside S1's
  `F1..F6` and S2's `F1-broken-S2..F6-broken-S2`:

    F1-broken-S3: charset_v1_lossy_normalization
                    (e.g. case-fold to lower) → H1 Refuted
    F2-broken-S3: five_gram_smoothing_uniform
                    (replace KN with uniform-1/|Sigma|) → H2 Refuted
    F3-broken-S3: model_emits_invalid_charset
                    (decode allows ids ∉ {0..75} ∪ {79}, including ids
                     76, 77, 78) → H3 Refuted
    F4-broken-S3: artifact_oracle_dropped_quant_resolve
                    (resolve weights by tensor-id name only) → H4 + H6 Refuted
    F5-broken-S3: bundle_export_nondeterministic_map_iter
                    (HashMap iteration order in graph serialize) → H5 Refuted
    F6-broken-S3: tied_embedding_export_split
                    (write classifier as a separate CanonicalTensor with
                     identical bytes; payload doubles) → H5 Refuted
    F7-broken-S3: v0_success_repetition_collapse
                    (decode loop disables max_consecutive_same_token check;
                     Q4 fires) → H3 Refuted
    F8-broken-S3: oracle_softmax_over_concat_logits
                    (compute KL over softmax of [prompt_len * vocab]
                     flattened logits, violating Rule LogitsAggregation)
                                                       → H4 Refuted because
                                                         the agreement evidence
                                                         was produced by an
                                                         invalid comparator /
                                                         conformance pipeline
    F9-broken-S3: phase_scheduler_wrong_ramp
                    (record HardnessRampS2 ramp Off → Soft → Soft → Hard
                     for expert_qat, or emit empty Phase C distill_loss
                     histogram in s2_distillation_log.v1)
                                                       → H7 Refuted

  Required test files (F-S2's `falsification_s2/{f1..f6}.rs` layout
  is the template; S3 files live under `falsification_s3/`):
    gbf-experiments/tests/falsification_s3.rs                    ; gate harness
    gbf-experiments/tests/falsification_s3/f1.rs                 ; charset lossy
    gbf-experiments/tests/falsification_s3/f2.rs                 ; KN uniform
    gbf-experiments/tests/falsification_s3/f3.rs                 ; invalid-charset decode
    gbf-experiments/tests/falsification_s3/f4.rs                 ; oracle drops QuantSpec
    gbf-experiments/tests/falsification_s3/f5.rs                 ; bundle nondet map iter
    gbf-experiments/tests/falsification_s3/f6.rs                 ; tied-embed split
    gbf-experiments/tests/falsification_s3/f7.rs                 ; repetition collapse
    gbf-experiments/tests/falsification_s3/f8.rs                 ; oracle softmax concat
    gbf-experiments/tests/falsification_s3/f9.rs                 ; phase-scheduler wrong ramp

  These tests are gated by the unified test-only `falsify` feature on
  gbf-experiments — the same feature that already gates F-S1's F1..F6
  and F-S2's F1-broken-S2..F6-broken-S2 broken substitutes. The S3
  test target `falsification_s3` selects only the F*-broken-S3 tests
  for the gate `cargo test -p gbf-experiments --features falsify
  --test falsification_s3`. gbf-experiments MUST compile_error! if
  `falsify` is enabled outside `cfg(test)` builds.

O-rep O-6 Hash round-trip
  Every emitted s3_*.v1 artifact round-trips through canonical JSON
  with self-hash equality (S1 O6 generalized). Tested per-schema in
  gbf-experiments/tests/canonical_json/s3/*.rs

O-totality O-7 Outcome algebra totality
  Every observable combination of binary H1..H7 verdicts, per-seed
  per-phase completion states, the suspicion threshold, and the H6
  direction split maps to exactly one S3Outcome variant under §10.
  Test target: gbf-experiments/tests/outcome/totality.rs

O-contam O-8 Workload-corpus contamination check
  No prompt's char sequence appears in train_post. The check uses a
  rolling hash over train_post; runtime budgeted ≤ 60 s on the
  pinned TinyStories train_post.
  Test target: gbf-experiments/tests/workload/contamination.rs
  (Per F-G1's closure plan, the cross-corpus contamination check is
   stubbed in v1 and named as moved to bd-tmaw / bd-pso7 in the
   report.)

O-noinputs O-9 No hidden inputs
  s3 artifacts depend only on:
    raw_train_sha256, raw_val_sha256
    train_post_sha256, val_post_sha256
    charset_v1_sha256
    workload_self_hash
    model_config (Toy0 from T14.1)
    train_config (D3 + D10 + F-S2 phase plan)
    seed
    pass_version
    export_visitor_hash
    chrome_budget_self_hash
    rust_toolchain_hash
    build_config_hash
    device_profile
    oracle_backend_identity
    gbf-train + gbf-artifact + gbf-oracle pinned dependency set
  No env-var, no host-clock, no network, no stdin. Report timestamps, when
  present, are derived from git commit metadata already named in the report,
  not from the wall clock at replay time.

O-isolation O-10 Per-seed isolation
  Seed s and seed s' produce independent run products and
  independent bundles and artifacts. Smoke checks identical to
  S1's O9, plus:
    bundle_self_hash differs across at least two of the five seeds;
    artifact_self_hash differs across at least two of the five seeds.

O-closure O-11 Closure gate
  bd-3k8o close is reachable iff Decision ∈ {ProceedToS4,
  ProceedToS4-with-deferred-clause}.

O-f4 O-12 F4 carry-through
  The s3_report.v1's H7 verdict, when Confirmed, is also recorded
  on the bd-3w2 closure comment via the QAT-bead-closure skill. This
  is the operational closure of F4 (Phased Training with Dense
  Teacher).
```

---

# 15. Minimal end-to-end theorem

```text
Theorem S3Soundness:

Given:
  charset_v1 LexicalSpec_v1 instance (D1)
  TinyStories.v2 manifest with charset_v1 normalization (D5)
  Toy0 reference instance (T14.1 closed, bd-1r6k)
  TrainConfig pinned per D3 + D10 + F-S2 phase plan
  pass_version V_S3 fixed by gbf-train HEAD at S3 PR merge
  ExportVisitor identity pinned by export_visitor_hash
  v0_success.toml WorkloadManifest pinned in fixtures/workloads/
  conservative chrome budget pinned in
    fixtures/runtime/chrome_budget.synthetic.toml

If for every seed s ∈ {0, 1, 2, 3, 4} and every phase φ ∈ {A, B, C, D}:
  s3_train_run(...)            returns Completed RunProduct
  s3_export_reference_bundle(s) returns BundleExportProduct
  s3_export_model_artifact(s)   returns ArtifactExportProduct
  DenotationalOracle.evaluate    returns DenotationalOracleProduct
  ArtifactOracle.evaluate         returns ArtifactOracleProduct
  three_way_agreement(s, p, A and D, prompt subset) returns
    AgreementProduct with overall_pass = true

And for the dense baseline per seed:
  s3_score_bpc_char(bundle, val_post)   returns finite val_bpc_char_fp
  s3_score_bpc_char(artifact, val_post) returns finite val_bpc_char_ternary
  per-prompt generation produces GenerationRecords satisfying Q3..Q5

And:
  s3_fit_kn5(...)                 returns finite KnBaselineProduct
  s3_charset_v1(...)              passes O-1 and O-1'
  KN math oracle O-2              passes
  bundle determinism O-3 + tied alias O-3'' pass
  artifact determinism O-3'        passes
  three-way oracle O-4 + QuantSpec O-4' pass
  v0_success totality O-5 + conformance round-trip O-5' + falsification
    suite O-5''                    pass
  s3_report.v1                     contains pre-registered predictions
    in pre-run git history
  H7 carry-through proof O-12      records F4 closure on bd-3w2

Then:
  Each of H1, H2, H3, H4, H5, H6, H7 has a defined verdict in
  {Confirmed, Refuted}.

  S3Outcome is exactly one of:
    Pass-clean
    Pass-with-fallback-oracle
    Fail-charset, Fail-baseline, Fail-quality, Fail-suspicious,
    Fail-oracle-agreement, Fail-bundle, Fail-quantspec,
    Fail-substrate, Fail-phase

  Decision is unique under the dispatch rule of §10.

  If S3Outcome ∈ {Pass-clean, Pass-with-fallback-oracle}, S3 has produced
  these verified knowledge claims:
    – charset_v1 normalization is deterministic, idempotent, and
      manifest-pinned at vocab=80; the locked Tier 2 charset is in
      effect.
    – 5-gram Kneser-Ney baseline math under D-rule discounts agrees
      with hand-counted oracle to f64 1e-12.
    – The dense Toy0 teacher trained from each of five seeds beats
      the 5-gram KN baseline by > 0.05 bpc on the pinned
      post-normalization validation sequence.
    – The hard ternary student survives QAT with a quantization gap
      ≤ 0.5 bpc on the same val.
    – Generation under Argmax decode produces only valid v1 charset
      tokens, no immediate repetition collapse, and ≥128 chars per
      prompt for every prompt in the v0_success workload.
    – The frozen teacher exports as a deterministic
      ReferenceModelBundle preserving tied embedding/classifier
      sharing.
    – The hard ternary student exports as a deterministic
      ModelArtifact whose ArtifactOracle resolves deployable weights
      through QuantSpec::weight_quant.
    – On the pinned three-prompt subset, the live frozen Phase-A dense
      teacher and the DenotationalOracle output on the bundle agree within
      the Phase-A tolerance band.
    – On the same subset, the live Phase-D hard ternary student and the
      ArtifactOracle output on the artifact agree bitwise under the Phase-D
      canonical reduction policy.
    – The bundle-vs-artifact difference is reported as a
      quantization/distillation gap, not treated as a bitwise agreement gate.
    – F4 (Phased Training with Dense Teacher) closes.

  If S3Outcome = Pass-with-fallback-oracle, S3 additionally records that
  at least one real oracle backend was unavailable and that a named S3
  fallback evaluator satisfied the fallback contract. Real F-C1/F-C2
  oracle implementation risk remains deferred and is not retired by S3.

  If S3Outcome = Fail-charset, S3 verifies that charset_v1 is
  broken; gate numbers downstream are unreliable.

  If S3Outcome = Fail-baseline, S3 verifies that the KN math is
  broken; H3's per-seed Q1 predicate is unverifiable.

  If S3Outcome = Fail-quality, S3 verifies that v0 is not
  "working enough" for the dense baseline; the F4 phase plan is
  not at fault by itself, but the workload + Toy0 sizing did not
  clear the gate.

  If S3Outcome = Fail-suspicious, S3 verifies the suspicious
  low-bpc sentinel fired; audit train/val split, bpc accumulator,
  and the charset normalization pipeline.

  If S3Outcome = Fail-oracle-agreement, S3 verifies that the live
  training output, the bundle, and the artifact disagree under D7's
  tolerance; per planv0 2026-05-06 amendment item 7, no future
  ROM-emitting bead may close until this is fixed.

  If S3Outcome = Fail-bundle, S3 verifies that ReferenceModelBundle or
  ModelArtifact export is non-deterministic, malformed, or silently
  duplicates payload; H4's agreement claim is contaminated.

  If S3Outcome = Fail-quantspec, S3 verifies that ArtifactOracle is
  using a brittle name-resolution path; per D12 this is a closure-
  blocking implementation defect.

  If S3Outcome = Fail-substrate, S3 verifies that the F4 phase plan
  destabilized under the v2 manifest (charset_v1) or that Burn
  numerics drifted; investigate per the F-S1 substrate playbook
  before concluding.

  If S3Outcome = Fail-phase, S3 verifies that F4 cannot close
  because per-seed reproducibility or per-phase distillation
  histograms violated H7.

Not proven:
  Project Gutenberg generalization (S4)
  cross-corpus contamination (S4)
  BoundedKv attention-oracle conformance (S5)
  multi-timescale LinearState A/B (S5)
  RuntimeChromeBudget end-to-end real measurement (S6)
  emulator harness end-to-end (S6)
  EncodedRom build (S6)
  MoE benefit (S7)
  UpperBankCandidate production-scale generalization on Gutenberg (S8)
  StructuredWidthGates (S8)
```

---

# 16. Implementation crate layout

Scope(F-S3) is hosted in `gbf-experiments` together with the existing
crates that provide its substrate and the new gbf-artifact / gbf-workload
/ gbf-oracle surfaces. This section pins the public surface that the
hypotheses and proof obligations rely on. Module names within each
crate are illustrative; only items tagged **Required** are normative.

## 16.1 Crate map

```text
gbf-policy
  Required  ModelSizeProfile::Toy0 reference instance (T14.1 closed,
            bd-1r6k). Re-affirmed from F-S1.

gbf-model
  Required  LinearStateBlock with Fixed(0.5) decay (bd-tnb closed).
  Required  CHARSET_V1_VOCAB_TIE_DEFAULT_LIMIT constant; renamed from
            BYTE_LEVEL_TIED_VOCAB_LIMIT per planv0 2026-05-06 amendment
            item 8. Constant value unchanged at 256.
  Required  Tied embedding/classifier sharing implementation (bd-2v4
            T6.3). The export-side representation of the alias is
            owned by gbf-artifact + gbf-train per bd-3bf1.

gbf-train
  Required  `gbf_train::scheduler::TrainingPhaseSchedule` consuming
            `gbf_train::phase::TrainPhaseSpec` for each PhaseKindS2
            variant (already landed via F-S2). Phase E HardenAndSelect
            is replaced for S3 closure by §7 export operations + §8
            oracle replay.
  Required  AdamW config helper with D10 hyperparameters (already landed
            via F-S2; re-affirmed by S3).
  Required  Loss composer / distillation / range / zero loss helpers
            (already landed via F-S2; re-affirmed by S3 inert-loss
            discipline per CLAUDE.md "Training Loss Beads").
  Required  ExportVisitor implementation (T1.6 / bd-g90) — the same
            visitor produces both the ReferenceModelBundle and the
            ModelArtifact; export_visitor_hash binds the identity.
  Required  freeze_teacher_as_reference operation (T4.3b / bd-7lu):
            on Phase A end (step 4000 boundary), the existing
            `gbf_train::teacher::freeze_teacher` returns a
            `FrozenTeacher<M>` snapshot; `s3_export_reference_bundle`
            runs ExportVisitor on that snapshot and emits the bundle.
            S3 does NOT re-implement the freeze; it consumes the
            FrozenTeacher landed by F-S2.
  Required  freeze_student_as_artifact operation (NEW in S3): on
            Phase D end (step 10000 boundary), freeze the hard ternary
            student checkpoint AFTER step 10000's optimizer update,
            mirroring the F-S2 teacher_freeze convention. The
            corresponding event fires at the boundary after step 10000
            and is recorded in s3_phase_log.v1 (S3-Run-Ok-N analog of
            S2 S2-Run-Ok-8). This is the source for `s3_export_model_artifact`.
  Required  Cargo features `qat` (default-on), `qat-ablation`
            (mutually exclusive with `qat`), and `burn-adapter`
            (transitive). See §17.
  Required  Structured logging adoption proof for ExportVisitor and
            oracle-replay producers (bd-2sd7). Subscriber-level tests
            must drive the producer entrypoints and assert the full
            required field set per the logging-bead-closure skill;
            helper-only emitter tests do not count. The F-S2 landed
            structured-logging surface (gbf-train/tests/structured_logging.rs,
            gbf-experiments/tests/phase_log_emitter_s2.rs,
            distillation_log_s2.rs) is the template S3 mirrors.

gbf-data
  Required  TinyStoriesManifest reader (S3 instance, v2 manifest).
  Required  charset_v1 normalization pipeline (F-G1 / bd-tmaw):
            implements the deterministic order of D1 step 1..6;
            emits CharsetProduct under O-1.
  Required  CorpusManifest schema (F-G1 / bd-tmaw). The S3 instance
            references TinyStories.v2 only; cross-corpus and
            contamination work is deferred to S4.
  Required  Canonical manifest paths:
              fixtures/corpora/tinystories.toml         (S1, raw bytes)
              fixtures/corpora/tinystories.v2.toml      (S3, charset_v1)

gbf-foundation
  Required  Hash256, sha256 helper. Re-affirmed from F-S1.

gbf-artifact
  Required  LexicalSpec_v1 (D1 / §3) as a serde + DomainHash type;
            lexical_self_hash participates in ArtifactCore identity.
  Required  ReferenceModelBundle, ReferenceProgram,
            ReferenceNumericProfile, ReferenceManifest types.
  Required  ArtifactCore, ModelSpec, QuantSpec (with weight_quant
            map), SequenceSemanticsSpec, CanonicalTensor.
  Required  TiedEmbeddingAlias type and CanonicalBundleWrite +
            CanonicalArtifactWrite encoders. These encoders are the
            only normative implementations for bundle/artifact canonical
            bytes.
  Required  ConformanceEnvelope type (S3 instance shape; F-C4
            schema upstream).
  Required  SemanticCheckpointSchema with PostEmbedding, PostLogits,
            PostDecode IDs.

gbf-workload
  Required  WorkloadManifest, WorkloadClass, ObservationPolicy,
            ExecutionMatrix, AcceptanceMatrix types.
  Required  fixtures/workloads/v0_success.toml schema reader and
            workload_self_hash computer.

gbf-oracle  (NEW workspace crate; may be initialized as a stub
             at S3 if F-C1 / F-C2 are not yet implemented; the
             stub MUST gate real-vs-fallback behavior behind
             the `s3-real` and `s3-fallback` features)
  Required  DenotationalOracle public API per §8.1.
  Required  ArtifactOracle public API per §8.2.
  Required  ArtifactScorer public API implementing the Evaluator interface
            consumed by s3_score_bpc_char for teacher-forced scoring over
            TextCharSeq with S3 chunk-reset semantics.
  Required  ReferenceScorer public API implementing the Evaluator interface
            consumed by s3_score_bpc_char for teacher-forced scoring of the
            exported ReferenceModelBundle over TextCharSeq with S3
            chunk-reset semantics.
  Required  three_way::compare comparator per §8.3.
  Notes     gbf-oracle MUST NOT depend on gbf-experiments. If the real
            oracle is not yet implemented, either:
              (a) gbf-oracle hosts the named fallback evaluators behind
                  `s3-fallback`, or
              (b) gbf-experiments hosts fallback evaluators locally and
                  calls them without routing through gbf-oracle.
            The workspace must not introduce a gbf-oracle ↔
            gbf-experiments dependency cycle.

gbf-experiments  (extends F-S1 + F-S2 ownership)
  Owns Scope(F-S3) end-to-end. Required modules:

    gbf_experiments::s3::manifest
      TinyStoriesManifest.v2 reader; verifies raw and post sha256s
      against the manifest before bytes flow.

    gbf_experiments::s3::charset
      Implements charset_v1 (delegating the pipeline to gbf-data
      under bd-tmaw), produces CharsetProduct, emits
      s3_charset_v1.v1.

    gbf_experiments::s3::baseline
      s3_fit_kn5 operation per §6.3. Emits s3_baseline_kn5.v1.

    gbf_experiments::s3::score
      s3_score_bpc_char operation per §6.4. Shared between the model
      scorer and the KN baseline scorer.

    gbf_experiments::s3::workload
      v0_success WorkloadManifest reader + GenerationRecord runner
      per §9. Emits s3_v0_success.v1.

    gbf_experiments::s3::export
      Wraps gbf-train::ExportVisitor for both bundle and artifact
      flows; emits s3_bundle.v1 and s3_artifact.v1 metadata.

    gbf_experiments::s3::oracle
      Three-way agreement runner. Drives gbf-oracle (real) or
      fallback evaluators behind feature flags. Emits
      s3_oracle_agreement.v1.
      Hosts the H6 adversarial fixture for QuantSpec resolution
      (§14 O-4').

    gbf_experiments::s3::conformance
      Builds ConformanceEnvelope from the AgreementProduct and the
      bundle/artifact pair. Emits s3_conformance.v1.

    gbf_experiments::s3::schema
      Type definitions for S3 report/product records, S1CanonicalJson
      encoder reuse, DomainHash function, CanonicalConformanceWrite,
      and self-hash round-trip helpers
      for s3_charset_v1.v1, s3_baseline_kn5.v1, s3_bundle.v1,
      s3_artifact.v1, s3_oracle_agreement.v1, s3_v0_success.v1,
      s3_conformance.v1, and s3_report.v1. Bundle/artifact canonical
      bytes are delegated to gbf-artifact's CanonicalBundleWrite and
      CanonicalArtifactWrite.

    gbf_experiments::s3::report
      s3_report.v1 emitter and outcome-algebra dispatcher
      implementing §10. Authors front-matter, validates R-Decision,
      R-AllSeeds, R-Self-Hash, R-Predictions, R-AllHypotheses,
      R-OwnerBeads, and binds the pre-registration commit history
      per §17.6 / O-rep generalization.

    gbf_experiments::s3::cli
      Public entrypoint(s) for replay. The S3Command enum mirrors the
      F-S2 S2Command shape (§16.4). The CLI surface is the canonical
      invocation point referenced by §17 closure.

    gbf_experiments::s3::environment
      S3EnvironmentHash producer mirroring `s2::environment`
      (build_config_hash, rust_toolchain_hash, dependency_lockfile_hash,
      oracle_backend_identity).

    gbf_experiments::s3::rng
      RNG-stream audit mirroring `s2::rng` (S2RngStreams reaffirmed;
      no new S3 stream domains).

    gbf_experiments::s3::oracle_re_run
      Re-runs the inherited S1 + S2 oracle suites under the S3 binary;
      emits s3_oracle_re_run.v1 in the same shape as
      s2_oracle_re_run.v1.

    gbf_experiments::s3::falsify
      Test-only; gated by the unified `falsify` feature. Hosts
      F1-broken-S3..F9-broken-S3.

gbf-cli
  Required  Subcommand `gbf s3 …` dispatching into
            gbf_experiments::s3::cli. The pre-registration check, the
            determinism check, the bundle/artifact replay check, and
            the closure script all shell into this surface.
```

## 16.2 Test layout

```text
Test target naming convention mirrors F-S2 (which uses `*_s2` suffixes
on integration test files; see `gbf-experiments/tests/falsification_s2.rs`,
`oracle_re_run_s2.rs`, `outcome_totality_s2.rs`, etc.). S3 test integration
files follow the same `*_s3` suffix shape:

gbf-experiments/tests/charset_idempotence_s3.rs
  O-1 + O-1' idempotence + manifest sha verification

gbf-experiments/tests/baseline_kn5_s3.rs
  O-2 KN math oracle on hand-counted fixture

gbf-experiments/tests/bundle_determinism_s3.rs
gbf-experiments/tests/artifact_determinism_s3.rs
  O-3 + O-3' determinism, O-3'' tied alias preservation

gbf-experiments/tests/oracle_agreement_s3.rs
  O-4 phase-specific exported-surface agreement on pinned subset

gbf-experiments/tests/oracle_quantspec_s3.rs
  O-4' QuantSpec resolution (adversarial fixture, gated by
       `s3-oracle-adversarial`)

gbf-experiments/tests/v0_success_outcome_totality_s3.rs
  O-5 outcome totality

gbf-experiments/tests/conformance_round_trip_s3.rs
  O-5' canonical conformance.json round-trip

gbf-experiments/tests/falsification_s3.rs
gbf-experiments/tests/falsification_s3/{f1,f2,...,f9}.rs
  O-5'' nine broken substitutes; gated by the unified `falsify` feature,
        selected via `--test falsification_s3`. File layout mirrors
        `gbf-experiments/tests/falsification_s2/{f1..f6}.rs`.

gbf-experiments/tests/canonical_json_s3.rs
  O-6 round-trip per schema (mirrors canonical_json_s2.rs)

gbf-experiments/tests/outcome_dispatch_s3.rs
  O-7 S3Outcome dispatch totality (mirrors outcome_totality_s2.rs +
      outcome_dispatch_s2.rs)

gbf-experiments/tests/contamination_s3.rs
  O-8 stub contamination check

gbf-experiments/tests/oracle_re_run_s3.rs
  Inherited S1 + S2 oracle suite re-run under the S3 binary;
  mirrors gbf-experiments/tests/oracle_re_run_s2.rs.

gbf-experiments/tests/integration_s3.rs
  End-to-end smoke run against a tiny in-repo fixture corpus
  (NOT TinyStories) used in CI to gate determinism (O-3 / O-3') and
  per-seed isolation (O-10). The fixture corpus is sized so a 5-seed
  S3 pass completes within the project's standard test timeout.
  The fixture corpus MUST be constructed so the KN D-rule preconditions
  hold for effective orders 2, 3, 4, and 5: n_1, n_2, and n_3 are all
  non-zero. Otherwise the smoke test is testing Fail-baseline rather than
  the intended end-to-end path.

  The full TinyStories.v2 run is gated behind a separate CI job, but
  bd-3k8o closure requires that job's artifacts and s3_report.v1, not
  merely the tiny-fixture smoke run. The full E2E pipeline test
  (bd-1wd / T10.11) is owned by gbf-test, not gbf-experiments; if it
  is not yet adopted into the S3 PR, e2e_test_owner_bead in the
  s3_report.v1 records the handoff.
```

## 16.3 Artifact paths

Unchanged from §11. All run artifacts are written under the repository-root
`experiments/S3/` tree. The report is written to
`docs/experiments/S3-report.md`. The conformance file is written to
`experiments/S3/conformance/conformance.json`.

## 16.4 Canonical replay command

The S3 CLI surface (`gbf s3 …`) mirrors the landed F-S2 S2Command
shape (`replay-full`, `replay-ablation`, `verify-determinism`,
`grad-flow`, `linearstate-smoke`, `phase-integ`, `oracle-re-run`,
`report`, `distill-once`). S3 introduces analogous verbs:

```text
S3Command :=
  | ReplayFull(args)              ; full S3 run for a build/seed cell
  | ReplayFallback(args)          ; same training, fallback oracle backend
  | VerifyDeterminism(args)       ; replay seed/build twice and byte-compare
  | NormalizeCorpus(args)         ; runs s3_charset_v1 only
  | FitBaseline(args)             ; runs s3_fit_kn5 only
  | ExportBundle(args)            ; runs s3_export_reference_bundle for one seed
  | ExportArtifact(args)          ; runs s3_export_model_artifact for one seed
  | OracleAgreement(args)         ; runs phase-specific surface agreement
  | OracleReRun(args)             ; re-runs S1 + S2 oracle suites under S3 binary
  | Report(args)                  ; emits s3_report.v1
```

Canonical full-pipeline replay:

```text
cargo run --release -p gbf-cli --no-default-features \
  --features "s3,s3-phase-d,s3-oracle-real" -- \
  s3 replay-full \
  --manifest fixtures/corpora/tinystories.v2.toml \
  --workload fixtures/workloads/v0_success.toml \
  --chrome-budget fixtures/runtime/chrome_budget.synthetic.toml \
  --pass-version <pass_version_S3_pinned_in_report> \
  --seed-list 0,1,2,3,4 \
  --build-kind s3_v0_success_real_oracle \
  --device-profile S1CpuDeterministic \
  --export-visitor-id <export_visitor_id_pinned_in_report>
```

Under the same machine + OS + pinned Burn version + pinned dependency
lockfile + S1CpuDeterministic, this command reproduces `experiments/S3/**`
byte-for-byte per Rep-1' and Rep-2.

Optional non-normative subcommands:

```text
gbf s3 normalize-corpus      runs s3_charset_v1 only
gbf s3 fit-baseline          runs s3_fit_kn5 only
gbf s3 export-bundle         runs s3_export_reference_bundle for one seed
gbf s3 export-artifact       runs s3_export_model_artifact for one seed
gbf s3 oracle-agreement      runs phase-specific surface agreement for one seed/prompt
gbf s3 oracle-re-run         re-runs S1 + S2 oracle suites under the S3 binary
gbf s3 verify-determinism    replays seed 0 and asserts byte-equality
                              of safetensors at every PhaseKindS2 boundary,
                              bundle, and artifact
```

## 16.5 Workspace registration

`Cargo.toml` workspace `members` is amended to include `gbf-workload`
and `gbf-oracle` (if not already present from F-C1 / F-C2). The
`gbf-experiments` crate's `Cargo.toml` declares additional workspace
dependencies on `gbf-artifact`, `gbf-workload`, and `gbf-oracle`, with
workspace-pinned versions (`= ` syntax already enforced workspace-wide
per A18 of F-S1).

---

# 17. Build configurations and feature flags

Three build configurations participate in the S3 contract. The first two
are inherited from F-S1 / F-S2; the third is new.

## 17.1 S3-build-A — "Phase D run with real oracle"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --no-default-features \
    --features "s3,s3-phase-d,s3-oracle-real"
Active features (workspace-resolved):
  gbf-experiments/s3 expands to:
    gbf-artifact/s3-schemas, gbf-workload/s3-schemas
  gbf-experiments/s3-phase-d expands to:
    gbf-train/qat, gbf-train/burn-adapter
  gbf-experiments/s3-oracle-real expands to:
    gbf-oracle/s3-real

Behavior:
  Full F4 phase plan A→B→C→D runs (PhaseBudget_S3 = PhaseBudget_S2).
  Real DenotationalOracle and ArtifactOracle are invoked. All H1..H7
  verdicts are reachable.

Build identity tag (S3BuildKind, kebab-case, recorded in s3_artifact.v1
metadata; mirrors S2BuildKind shape):
  build_kind = s3_v0_success_real_oracle
```

## 17.2 S3-build-B — "Phase D run with fallback oracle"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --no-default-features \
    --features "s3,s3-phase-d,s3-oracle-fallback"

Behavior:
  Same as S3-build-A but routes oracle calls through the
  S3DenotationalFallback and S3ArtifactFallback evaluators hosted in
  gbf-experiments::s3::oracle (or gbf-oracle when wired). The fallback
  evaluators must satisfy §8.1 / §8.2 fallback contracts (notably:
  weight resolution through QuantSpec::weight_quant; bundle program
  graph end-to-end).

  Closure with this build is permitted ONLY when s3_report.v1 records
  oracle_fallback_used as a non-empty list (one or both of
  "S3DenotationalFallback", "S3ArtifactFallback") AND the report's
  Decision is ProceedToS4-with-deferred-clause.

Build identity tag:
  build_kind = s3_v0_success_fallback_oracle
```

## 17.3 S3-build-C — "Adversarial oracle fixture"

```text
Cargo invocation:
  cargo test -p gbf-experiments \
    --features "s3,s3-oracle-adversarial,falsify" \
    --test oracle_quantspec_s3

Behavior:
  Test-only. Enables the H6 adversarial fixture (shadow tensor
  "linear_0_weight_naive_fp32") and the nine broken-S3 substitutes
  (F1-broken-S3..F9-broken-S3). Cannot leak into a release build.

Build identity tag:
  build_kind = s3_oracle_adversarial
```

## 17.4 Feature flag contract

```text
gbf-experiments/s3                  gates all S3 schemas, the s3 module
                                    tree, and the s3 CLI subcommand. It
                                    does not select a training/QAT path or
                                    an oracle backend.
gbf-experiments/s3-phase-d           enables the Phase-D training/export
                                    runtime path and forwards to
                                    gbf-train/qat and gbf-train/burn-adapter.
gbf-experiments/s3-oracle-real       enables the real-oracle code path
                                    (forwards to gbf-oracle/s3-real).
gbf-experiments/s3-oracle-fallback   enables the fallback evaluators.
gbf-experiments/s3-oracle-adversarial test-only; enables the H6
                                    adversarial fixture artifact.
gbf-experiments/falsify              test-only; UNIFIED feature; gates
                                    F-S1 F1..F6 + F-S2 F1-broken-S2..F6-broken-S2 +
                                    F-S3 F1-broken-S3..F9-broken-S3.
                                    The `falsification_s3` test target selects
                                    only the S3 broken substitutes.
gbf-experiments/qat-ablation         forwards to gbf-train/qat-ablation
                                    and is mutually exclusive with
                                    gbf-train/qat.
gbf-train/qat                       default-on; gates all QAT codepaths.
gbf-train/qat-ablation              mutually exclusive with `qat`.
gbf-artifact/s3-schemas             enables LexicalSpec_v1, ReferenceModelBundle,
                                    ConformanceEnvelope serde implementations
                                    in their S3 instance shape.
gbf-workload/s3-schemas             enables WorkloadManifest serde in S3
                                    instance shape and the v0_success.toml
                                    schema reader.
gbf-oracle/s3-real                  enables the real DenotationalOracle and
                                    ArtifactOracle implementations; absent
                                    when F-C1 / F-C2 are not yet wired.
gbf-oracle/s3-fallback              enables named fallback evaluators when
                                    they are hosted in gbf-oracle rather than
                                    gbf-experiments.

Mutual exclusion enforcement:
  gbf-experiments must compile_error! at the crate root when both
  `s3-oracle-real` and `s3-oracle-fallback` are enabled.

  The replay/oracle-agreement entrypoints must compile_error! when neither
  `s3-oracle-real` nor `s3-oracle-fallback` is enabled. Schema-only tests,
  canonical JSON tests, outcome algebra tests, and adversarial fixture tests
  may run with `s3` alone.

  gbf-experiments must also compile_error! when `s3-oracle-real` is
  enabled but gbf-oracle/s3-real is absent at link time.
```

## 17.5 Determinism budgets

```text
S3 inherits S1CpuDeterministic from F-S1 §5 unchanged. The runner sets
the same env_exact and rejects any unset env_exact entry, value mismatch,
or other variable still set in the process environment. The S3 export
operations (BundleExportProduct, ArtifactExportProduct) and the oracle
operations are subject to the same determinism budget as training.
```

## 17.6 Pre-registration CI

```text
scripts/s3_preregistration_check.sh implements the S3 instance of
F-S1's O1:
  1. predictions_section_hash matches the markdown section in
     predictions_commit, recomputed using S1CanonicalJson normalization;
  2. predictions_commit is a strict ancestor of first_result_commit;
  3. first_result_commit is the earliest commit introducing any
     charset_self_hash, baseline_self_hash, bundle_self_hash,
     artifact_self_hash, agreement_self_hash, v0_success_self_hash,
     or conformance_self_hash derived from S3 execution.
Exit non-zero on any violation. Closure of bd-3k8o is forbidden while
this script exits non-zero.
```

## 17.7 CI gates that block bd-3k8o closure

```text
cargo test -p gbf-experiments --no-default-features \
  --features "s3,s3-phase-d,s3-oracle-real"
cargo test -p gbf-experiments --no-default-features \
  --features "s3,s3-phase-d,s3-oracle-fallback"
cargo test -p gbf-experiments --features "s3,falsify" \
  --test falsification_s3
cargo test -p gbf-experiments --features "s3,s3-oracle-adversarial" \
  --test oracle_quantspec_s3
cargo test -p gbf-experiments --features s3 --test charset_idempotence_s3
cargo test -p gbf-experiments --features s3 --test baseline_kn5_s3
cargo test -p gbf-experiments --features s3 --test bundle_determinism_s3
cargo test -p gbf-experiments --features s3 --test artifact_determinism_s3
cargo test -p gbf-experiments --features s3 --test conformance_round_trip_s3
cargo test -p gbf-experiments --features s3 --test v0_success_outcome_totality_s3
cargo test -p gbf-experiments --features s3 --test outcome_dispatch_s3
cargo test -p gbf-experiments --features s3 --test canonical_json_s3
cargo test -p gbf-experiments --features s3 --test integration_s3
cargo test -p gbf-experiments --features s3 --test oracle_re_run_s3
cargo build -p gbf-experiments --no-default-features \
  --features "s3,qat-ablation"
  (re-affirms F-S1 §16.6 / F-S2 §16.6 ablation build still builds with
   the S3 tree compiled in; the runtime path stays guarded behind
   `s3-phase-d`. Note: s3 alone must not enable gbf-train/qat — that is
   gated by s3-phase-d, exactly mirroring F-S2's split between
   `gbf-experiments/s2-full` (runtime) and `gbf-experiments/s2-ablation`.)
cargo test -p gbf-train --features burn-adapter -- \
  teacher::freeze bundle_export student_freeze
  (Burn-feature-enabled gate per CLAUDE.md "loss claim depends on Burn
   autodiff"; export traverses Burn-resident tensors. The
   `teacher::freeze` filter re-affirms the F-S2 inherited
   FrozenTeacher snapshot used as the bundle source.)
scripts/s3_preregistration_check.sh
  (mirrors scripts/s2_preregistration_check.sh; satisfies O1)
scripts/s3_determinism_check.sh
  (CI smoke: replays seed 0 of s3_v0_success_real_oracle and asserts
   byte-equality of safetensors at every PhaseKindS2 boundary
   (4000/5000/8000/10000), bundle, and artifact. This is a smoke gate
   only and does NOT satisfy O-3 + O-3' for closure. Mirrors
   scripts/s2_determinism_check.sh.)

scripts/s3_full_determinism_check.sh
  (closure gate: replays all five seeds of s3_v0_success_real_oracle
   and asserts byte-equality of safetensors at every phase boundary,
   bundle, and artifact for each seed; satisfies O-3 + O-3'.)
scripts/s3_isolation_check.sh
  (asserts at least two of the five seeds produce different
   bundle_self_hash and artifact_self_hash, AND that the
   s3_v0_success_real_oracle vs s3_v0_success_fallback_oracle build
   pair produces identical per-seed bundle / artifact hashes (only the
   oracle backend changes, not the train output); satisfies O-10.
   Mirrors scripts/s2_isolation_check.sh.)
scripts/s3_api_drift_check.sh
  (greps gbf-artifact, gbf-workload, gbf-oracle public symbols against
   pinned snapshot files; satisfies the api-drift gate; mirrors
   scripts/s2_api_drift_check.sh.)
scripts/s3_oracle_re_run_check.sh
  (re-runs the inherited S1 + S2 oracle suites under the S3 binary;
   emits s3_oracle_re_run.v1 and gates Fail-metric.)
scripts/s3_no_naming_resolution_check.sh
  (greps every emitted s3_artifact.v1 metadata file for
   weight_resolution_summary.tensors_resolved_via_naming and asserts
   the value is 0; satisfies the closure-forbidden clause of §13)
```

---

# 18. Ambiguity ledger

|  ID | Ambiguity                                                                                 | Chosen path                                                                | Clarifying question                                                                  | Suggested final decision                                                                                                                              |
| --: | ----------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
|  A1 | charset_v1 vocab arithmetic discrepancy (planv0 says "76 + \n + 2 control = 79; round to 80 with <unk> fallback" but bd-2ym0 lock says "76 printable + newline = 77; +3 control = 80") | vocab_size = 80 with id assignment 0..75 printable, 76 reserved (forbidden in input), 77/78/79 = bos/eos/unk | Why is id 76 reserved? planv0 numbering is inconsistent.       | Pin id 76 reserved for v1.1 forward expansion; reject if seen in input. Alternative (collapse reserved) would silently change the Tier 2 charset table. Open follow-up bead under bd-2ym0 to confirm. |
|  A2 | Constant rename value preservation (planv0 amendment item 8 says value unchanged at 256)  | Rename to CHARSET_V1_VOCAB_TIE_DEFAULT_LIMIT, value = 256                  | Should the limit drop to 80 to match the locked vocab?                               | Keep at 256 per planv0 amendment item 8; tied embedding applies whenever vocab ≤ 256, and 80 ≤ 256.                                                   |
|  A3 | bpc-per-byte vs bpc-per-character regression                                              | bpc-per-character at vocab=80 supersedes for S3+; S1 bpc-per-byte numbers are not directly comparable | Should we report both?                                              | Report bpc-per-character as the gate; S1's bpc-per-byte is recorded in cross-revision traceability only. The 0.05 margin in Q1 is per character.       |
|  A4 | 5-gram KN discount D-rule on small per-order count-of-counts                              | No fallback. If any required n_1, n_2, or n_3 is zero for orders 2..5, abort baseline fit. | What if the corpus is too small to estimate discounts? | Abort with Fail-baseline. TinyStories train_post is expected to be large enough; if not, the baseline is not pre-registered well enough for S3. |
|  A5 | "<\|endoftext\|>" handling under charset_v1                                                | Pipes are unmappable → <unk>; not a semantic token                         | Should we re-introduce <bos>/<eos> at story boundaries?                              | No at S3. Document boundary insertion is reserved for S4+ once corpus governance defines it. The <unk>-from-pipe is a known artifact, recorded in drop_log_summary. |
|  A6 | v0_success eighth clause (emulator runs ≥1 token end-to-end)                              | Deferred to S6 per D14                                                     | Should S3 attempt a stub emulator run?                                               | No. Stub would be misleading. S3's gate is artifact + oracle agreement, not emulator. Recorded in s3_report.v1 as a pinned deferral.                  |
|  A7 | Phase A vs Phase D oracle tolerance                                                       | Phase A: 4.0e-6 abs (fp32 elementwise); Phase D: bitwise under canonical reduction order | Why 4.0e-6 and not 1e-7 or 1e-5?                                          | 4.0e-6 [ESTIMATE] is approximately 32x f32 epsilon; tolerates one round of canonical reduction on Toy0's small vocabulary. Should be re-examined when real oracle ships; flag for P7. |
|  A8 | Three-way agreement subset size                                                           | First three prompts (manifest order); ≥16 generated steps each              | Why three prompts and not all of v0_success?                                          | Three is enough to detect agreement-direction defects; all-eight runs are too slow under S1CpuDeterministic. Tightening to all eight is a follow-up.   |
|  A9 | Bundle Phase D logits comparison                                                          | At Phase D the bundle stays the dense teacher (Phase A); train_vs_bundle uses Phase A tolerance | Should we also export a Phase D bundle?                                | No. The bundle's purpose is denotational truth, which is the dense teacher. Quantization gap (bundle vs artifact at Phase D) is reported in conformance, not gated. |
| A10 | Tied embedding alias representation in safetensors                                        | One CanonicalTensor referenced twice via TiedEmbeddingAlias metadata        | Why not two tensors with byte-equal payloads?                                         | Byte-equal duplicate would silently double payload + classifier-bank cost. F6 falsification asserts the alias is preserved. See bd-3bf1.              |
| A11 | Real oracle vs fallback oracle for closure                                                | Both permitted at S3; fallback usage explicit in s3_report.v1               | Should closure require real oracle?                                                   | No, per Rule OracleOwnerNaming and bd-c4wg's handoff comment. S3 may close with fallback if and only if the fallback satisfies §8 contract. Real-oracle requirement is a strict S6 prerequisite. |
| A12 | conservative_chrome_budget_bytes derivation                                                | 90% of pinned synthetic defaults (D15)                                      | Why 0.90 not 0.50?                                                                    | 0.90 [ESTIMATE] is conservative-but-realistic — assumes the runtime shell takes ≤10% of each bank. Tighten when S6 produces real RuntimeChromeBudget. |
| A13 | Q5 expected_min_gen vs decode_max                                                         | expected_min_gen = 128 chars; decode budget max_chars = 256                 | What if model emits <eos> before 128?                                                 | Q5 fails for that prompt. Argmax + small Toy0 may produce premature <eos>; the gate is per-prompt strict by design.                                    |
| A14 | Generation determinism for Argmax decode                                                   | Argmax is deterministic; rng_spec = NoRng                                   | Should we also pin a sampling decode for completeness?                                | Not at S3. TopKTemperature is reserved for later workloads (e.g. S5+).                                                                                |
| A15 | reserved_id 76 forbidden in input                                                         | Reject + abort loader on encounter                                          | Should we map to <unk>?                                                                | No. Reject preserves the v1.1 forward-expansion contract. <unk> mapping would silently corrupt v1.1 streams in a future migration.                     |
| A16 | bd-1wd (T10.11 E2E test) ownership at S3                                                  | Recorded as e2e_test_owner_bead; not strictly required to be in the S3 PR  | Should bd-1wd be a hard blocker?                                                      | Soft blocker. The S3 closure gate is the s3_report.v1 + the §17 CI matrix. bd-1wd remains open if the cross-crate E2E in gbf-test is not adopted into the S3 PR; the report records the handoff. |
| A17 | bd-2sd7 (structured logging adoption) at S3                                                | Required for ExportVisitor and oracle-replay producers; recorded as structured_logging_owner_bead | What if logging is missing for one producer?                          | Subscriber-level integration tests must drive the producer entrypoint and assert the full required field set per the logging-bead-closure skill. Helper-only emitter tests do not count as adoption proof; in that case, bd-2sd7 remains open. |
| A18 | F-G1 contamination check                                                                   | Stub at S3 (named as moved to bd-tmaw / bd-pso7 in the report)              | Should S3 require real cross-corpus check?                                             | No. S3 has only TinyStories.v2; cross-corpus is an S4 concern. Stub asserts no prompt's char sequence appears in train_post.                          |
| A19 | gbf-oracle crate existence at S3                                                          | NEW workspace crate; may begin as a stub                                    | Why not host the oracle in gbf-experiments?                                            | Per Rule CrateOwnership and planv0's three-stratum design (line 151), gbf-oracle is the long-term home. Hosting in gbf-experiments would conflate framework with experiment. The stub is a temporary shim. |
| A20 | Per-token KL aggregation for ConformanceEnvelope                                          | Per-token, per-vocab-row only (Rule LogitsAggregation)                      | What about prompt-level metrics?                                                       | None at S3. Prompt-level summaries are aggregations of per-token records; they are reported in s3_conformance.v1 but never computed by softmaxing concatenated logits. F8 falsification pins this. |
| A21 | F4 closure decision recording                                                             | H7 Confirmed → close bd-3w2 with QAT-bead-closure skill checklist          | Why not close F4 at S2?                                                                | F-S2's H4 closes only Phase A cleanliness. F4 requires Phase A through Phase D plus the export contract; only S3 produces all three. Closing F4 at S3 follows the slice-closure pattern.                 |
| A22 | Multi-Phase phase-budget alignment                                                        | Inherit F-S2's per-phase optimizer_steps, sequence_length, batch_size       | Should S3 grow the phase budget?                                                       | No. S3 amends only the data, baseline, and export+oracle contract. Growing the phase budget would be a separate amendment with bumped pass_version.    |
| A23 | Real-oracle ReductionOrderPolicy default                                                  | Enforced at S3 (D7 / §3 ReferenceNumericProfile)                            | What if the real oracle ships with Advisory?                                          | Per planv0 §366..369, both are valid policies; at S3 we enforce. If gbf-oracle ships with Advisory by default, S3 overrides via ReferenceNumericProfile construction. Flag for P7.                       |
| A24 | bpc_kn5_val sanity range [1.7, 2.6] [ESTIMATE]                                             | Range is sanity-only; H2 gate is the oracle equality, not the range         | What if real bpc_kn5 is outside [1.7, 2.6]?                                            | Report as a Surprise; H2 verdict is unaffected. The 5-gram KN on charset_v1 TinyStories has no published number; range is a cautious estimate.       |
| A25 | unmappable_drop_rate predicted bound 0.005 [ESTIMATE]                                      | Sanity prediction; H1 hard gate is 0.02 (D1)                                 | What if real drop rate exceeds 0.005?                                                  | Surprise (P3 signal); not a Refutation unless > 0.02.                                                                                                  |
| A26 | conservative_chrome_budget_bytes synthetic table at S3                                    | Pinned in fixtures/runtime/chrome_budget.synthetic.toml                     | Why not derive from gbf-hw at S3?                                                      | gbf-hw integration is S6 territory. The synthetic budget is conservative (0.90 of small ExpertBank default) so a Toy0 artifact safely fits.            |
| A27 | What if all five seeds' bundle_self_hash are *identical*?                                  | Suspicious; O-10 asserts at least two seeds differ                          | Could mean ExportVisitor is seed-independent.                                          | Add explicit O-10 assertion; investigate ExportVisitor RNG dependence if it fires. The dense teacher's parameters are seed-dependent so bundles must differ.                                              |
| A28 | gbf-train ExportVisitor identity bump policy                                              | Pinned by export_visitor_hash (D11); bumping invalidates s3_bundle.v1 + s3_artifact.v1 | What if a trivial whitespace fix is rolled into ExportVisitor?    | Bump pass_version + export_visitor_hash; re-run S3. The closure script verifies export_visitor_hash matches the report's pin.                          |

---

# 19. Final concise contract

```text
F-S3 v0_success on TinyStories is correct when:

1.  charset_v1 normalization on TinyStories is deterministic, idempotent,
    manifest-pinned at vocab=80, and produces train_post and val_post
    streams with unmappable drop rate ≤ 2.0% and zero occurrences of
    the reserved id 76.

2.  The 5-gram Kneser-Ney baseline math under D-rule discounts agrees
    with the hand-counted oracle to within 1.0e-12 in f64 on the
    pinned fixture corpus.

3.  For every seed s ∈ {0..4}, the F-S2-landed phase scheduler
    (`gbf_train::scheduler::TrainingPhaseSchedule`) runs PhaseKindS2
    PhaseA → PhaseB → PhaseC → PhaseD over the inherited
    PhaseBudget_S3 = (4000, 1000, 3000, 2000) optimizer steps without
    divergence; the dense teacher is frozen at the step-4000 boundary
    via the inherited `gbf_train::teacher::freeze_teacher`; the hard
    ternary student is frozen at the step-10000 boundary via the new
    `freeze_student_as_artifact`; and the per-seed composite quality
    predicate (Q1..Q6) holds on the v0_success WorkloadManifest:
      Q1: val_bpc_char_fp(s) < bpc_kn5_baseline - 0.05
      Q2: val_bpc_char_ternary(s) - val_bpc_char_fp(s) ≤ 0.5
      Q3: charset validity rate of generation = 1.0
      Q4: max consecutive same token ≤ 8
      Q5: ≥128 generated chars per prompt
      Q6: artifact bytes ≤ conservative chrome budget

4.  For every seed s and the first three prompts of v0_success.prompts:
      - the live frozen Phase-A dense teacher and the DenotationalOracle
        output on the exported ReferenceModelBundle agree at PostLogits
        and PostDecode under the Phase-A tolerance band
        (4.0e-6 fp32 elementwise; argmax tokens must match);
      - the live Phase-D hard ternary student and the ArtifactOracle
        output on the exported ModelArtifact agree at PostLogits and
        PostDecode under the Phase-D bitwise canonical reduction rule
        (argmax tokens must match).
    Bundle-vs-artifact differences are reported as quantization gap,
    not used as a bitwise equality gate.

5.  ReferenceModelBundle and ModelArtifact bytes are bit-identical
    across replays under the canonical write rules; tied
    embedding/classifier sharing is represented as one CanonicalTensor
    referenced twice; the artifact's deployable weights resolve through
    QuantSpec::weight_quant on every tensor (no tensor-id name
    fallback).

6.  s3_report.v1 emits pre-registered predictions in git history strictly
    before the first S3 result artifact commit, records oracle owner
    beads (bd-1rcc, bd-c4wg), conformance owner bead (bd-35l3),
    structured-logging owner bead (bd-2sd7), and concludes with exactly
    one Decision value chosen by §10 dispatch.

7.  Decision is one of {ProceedToS4, ProceedToS4-with-deferred-clause};
    any other Decision blocks bd-3k8o closure. Decision::Halt is
    forbidden as a closure outcome; Decision::Investigate routes to
    follow-up beads.

8.  Every JSON artifact (s3_charset_v1, s3_baseline_kn5, s3_bundle,
    s3_artifact, s3_oracle_agreement, s3_v0_success, s3_conformance,
    s3_oracle_re_run, s3_report) is canonical (S1CanonicalJson;
    no S3CanonicalJson is introduced), deterministic, and
    self-hash-valid. Binary blobs (frozen teacher safetensors, KN
    counts, bundle bytes, artifact bytes) are bound by recorded
    Hash256 fields. The S3 binary's S2EnvironmentHash and
    S3EnvironmentHash are recorded in s3_report.v1 for replay.

9.  All seven hypotheses have explicit verdicts in the falsification
    analysis section, with concrete observations cited.

10. The nine-test S3 falsification suite passes: the deliberately-broken
    substitutes for charset, KN math, charset-violating decode,
    QuantSpec-bypassing oracle, non-deterministic bundle export, tied-
    embedding split, repetition collapse, prompt-wide softmax
    aggregation, and a wrong-ramp phase scheduler each produce the
    expected Refuted verdict on the corresponding hypothesis.

11. F4 (Phased Training with Dense Teacher) closes via H7's
    carry-through verdict. The QAT-bead-closure skill records the
    closure on bd-3w2.

12. S3 retires the cross-epic surface-area risk (corpus pipeline +
    charset + denotational stratum + artifact stratum + workload manifest
    + reference bundle export) for the v0_success workload only. It
    does not claim cross-corpus generalization (S4), attention-oracle
    conformance or multi-timescale state (S5), emulator end-to-end or
    EncodedRom (S6), MoE benefit (S7), or UpperBankCandidate production-
    scale generalization on Gutenberg (S8) — those are later slices'
    proof obligations.
```
