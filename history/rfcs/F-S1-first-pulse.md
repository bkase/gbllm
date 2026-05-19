# Formal spec pack: F-S1 First Pulse

This is the first scientific/experimental RFC in the training-contract epic. Its
deliverable is **verified knowledge**, not just code. Sections that are normally
"Stage X transforms input I to output O" become "Hypothesis H_n predicts P; the
experiment either Confirms or Refutes H_n with these observable consequences."

Important interpretation:
  A `Fail-capacity` result is a successful scientific falsification, not an
  implementation failure. It means S1 retired substrate risk only if H1, H4,
  and H5 confirmed; it does not retire Toy0-size risk. Closure of bd-12pl
  remains blocked because this RFC chooses Toy0 sufficiency as a mandatory
  gate.

```text
Spec:
  F-S1 First Pulse
  Slice S1 of the training-contract epic (bd-1rb)
  Closure bead: bd-12pl

Hypothesis-under-test:
  A Toy0 dense fp byte model, trained on TinyStories raw byte stream
  through the F4 phase scheduler in Phase A only, produces a checkpoint
  whose held-out val bpc is strictly more than 0.05 bpc below a
  fixed-spec 3-gram baseline, for every one of five fixed seeds.

Owns:
  hypothesis statements H1..H5
  pre-registered prediction tables
  Toy0 ModelSizeProfile reference instance
  TinyStories raw-byte loader (S1 stub form)
  3-gram baseline trainer + scorer (linear-interp + add-α)
  bpc scoring math (S1 instance, vocab=256)
  AdamW Phase A run protocol
  s1_checkpoint.v1, s1_run_log.v1, s1_score.v1,
  s1_negative_test.v1, s1_ablation.v1, s1_baseline.v1, s1_report.v1
  S1 reproducibility laws
  S1 metric-falsification negative test

Does not own:
  ternary QAT (S2)
  charset_v1 enforcement (S3)
  ReferenceModelBundle export (S3)
  ArtifactOracle round-trip (S3)
  Project Gutenberg corpus (S4); production-scale UpperBankCandidate
    runs on Gutenberg (S8)
  multi-timescale LinearState (T12.5; arrives in S5 unless promoted)
  Game Boy ROM build (S6)
  MoE / router (S7)
  v0_success workload manifest (S3)
  RuntimeChromeBudget preflight (S6)
```

## Decisions

```text
D1 raw bytes, not charset_v1
   S1 trains on TinyStories raw byte stream with vocab = 256.
   No NFC, no charset folding, no <unk>, no <bos>/<eos>.
   Charset_v1 enforcement is owned by S3.

D2 fixed seed list
   seeds = [0, 1, 2, 3, 4]
   Five seeds are mandatory. No more, no fewer.

D3 fixed train budget
   optimizer_steps   = 10000
   batch_size        = 32
   sequence_length   = 128
   eval_every_steps  = 1000
   eval_subset_size  = 4096 sequences

   eval_subset_size applies only to progress eval_points in s1_run_log.v1.
   Final gate bpc is always computed over the full val byte sequence.

   These values are part of this RFC. Changing any invalidates prior
   comparisons and constitutes a new experiment.

D3a deterministic batch/eval sampling
   Training batch sampler:
     start_offset(step, batch_index) is drawn from BatchRng(seed) uniformly
     over the inclusive integer interval
     0..=(byte_length(corpus_train) - sequence_length), using rejection
     sampling to avoid modulo bias.
     The sampled byte sequence is
     corpus_train[start_offset .. start_offset + sequence_length].

     BatchRng(seed) is initialized exactly once per run, before optimizer
     step 1. Draws occur in lexicographic order:

       for step in 1..=optimizer_steps:
         for batch_index in 0..batch_size:
           draw one start_offset

     Evaluation, scoring, baseline fitting, negative testing,
     initialization, and ablation must not consume from BatchRng.
     InitRng, BatchRng, and ShuffleRng are disjoint streams.

   Training objective:
     For each sampled sequence x[0..sequence_length), the model scores all
     sequence_length target bytes under the same reset-context semantics used
     by S1 bpc:

       - x[0] is predicted from the deterministic zero initial state.
       - for j > 0, x[j] is predicted from the state after consuming x[0..j).
       - state is reset between batch elements.
       - state is reset between optimizer steps.

     The per-step train loss is the mean natural-log cross entropy over
     batch_size * sequence_length target bytes. No padding token, BOS token,
     EOS token, or next-byte outside the sampled slice is used.

   Progress eval subset:
     progress_eval_bytes = val[0 .. min(byte_length(val), 4096 * 128)].
     This prefix is scored with the same non-overlapping 128-byte chunking
     and final-short-chunk rule as final gate scoring.

   Gate scoring:
     full val bytes, including a final short chunk if present.

D4 fixed 3-gram baseline math
   linear interpolation:
     λ_3 = 3/5
     λ_2 = 3/10
     λ_1 = 1/10

   add-α smoothing per n-gram order:
     α = 1/100

   These constants have rational semantics. Implementations materialize
   them as f64 at the point of probability computation; f32 rounding is
   forbidden in baseline probability, bpc, and oracle computations.
   vocabulary matches the model: 256.

D5 fixed split
   TinyStories canonical train/val split, hash-pinned in
   fixtures/corpora/tinystories.toml. No on-the-fly resampling.
   The manifest pins the exact post-decompression train and val byte streams.
   No Unicode normalization, newline normalization, archive reserialization,
   document-boundary insertion, or text decoding is permitted before hashing
   or training.

D6 strict pass criterion
   ∀ seed s. val_bpc(s) < bpc_3gram − 0.05
   median, max, min are reported, but the pass test is per-seed.

D7 mandatory measurement-oracle falsification
   H5 is tested by deterministic metric fixtures independent of Toy0:

   O-metric-0 rejection-sampler adversarial fixture:
     uniform_u64_inclusive must be tested with an injected deterministic
     u64 stream that first returns a value in the rejection zone for a
     non-power-of-two interval, then returns an accepted value. The
     fixture must prove that the rejected value is not reduced modulo
     the interval.

     Example obligation:
       For interval [0, 9], the test harness injects a u64 r such that
       r ≥ floor(2^64 / 10) * 10. The implementation must reject r and
       consume the next u64 output. A modulo-reduction implementation
       must fail this fixture.

   O-metric-1 uniform logits:
     A fixture model that emits exactly uniform logits over 256 bytes must
     score bpc = 8.0 on any non-empty byte sequence, within tolerance
     1e-12 in f64.

   O-metric-2 hand-counted n-gram:
     A fixed tiny corpus with hand-computed unigram, bigram, and trigram
     counts must produce the exact expected smoothed probabilities and bpc
     within tolerance 1e-12 in f64.

   O-metric-3 reset boundary:
     A spy fixture scorer records the context length observed before every
     scored byte. For an input of length 129 and chunk_size = 128, the
     recorded context lengths must be exactly:

       [0, 1, 2, ..., 127, 0]

     This oracle proves reset behavior directly. A value-based fixture
     alone is insufficient because it can pass accidentally if the model
     assigns the same probability under both contexts.

   O-metric-4 shuffle permutation:
     V_shuffled is produced by Fisher-Yates over byte indices using
     ShuffleRng(0xDEADBEEF). The shuffled byte sequence must have exactly
     the same byte multiset as V_original and, for the canonical val split,
     must not be byte-identical to V_original.

     Additionally, the canonical TinyStories manifest pins:

       val_shuffle_deadeef_sha256: Hash256

     O-metric-4 must assert sha256(V_shuffled) equals this pinned value.
     Multiset preservation and non-identity are necessary but not
     sufficient: a modulo-biased Fisher-Yates implementation can preserve
     the multiset while permuting incorrectly.

   The trained-model shuffle delta is still recorded:
     model_neg_test_delta = bpc(model, V_shuffled) − bpc(model, V_original)
   but it is a model context-sensitivity observation (H3), not by itself
   proof that the metric is correct.

D8 strict reproducibility
   Same seed + same corpus_train_sha + same corpus_val_sha +
   same train_config_hash + same model_config_hash + same gbf-train
   pass_version + same dependency lockfile + same rust_toolchain_hash +
   same build_config_hash + same device_profile
   ⇒ bit-identical safetensors checkpoint.

D9 fail-closed on NaN / divergence
   Any seed producing non-finite loss or non-finite gradient norm at any
   step fails the entire S1.
   No partial pass.

D10 optimizer pinned
   AdamW { lr=1e-3, β1=0.9, β2=0.999, eps=1e-8, weight_decay=0.0 }
   No schedule. No warmup.
```

---

# 1. Core notation

```text
Hash256        := /^sha256:[0-9a-f]{64}$/
Seed           := u64
TrainStep      := u32      ; valid training steps are 1..=optimizer_steps
EvalStep       := u32      ; valid eval steps are 0, eval_every_steps, ...
Step           := u32      ; generic step only when train/eval distinction is irrelevant
LossNatsPerByte := f32     ; finite natural-log cross entropy per target byte
BpcValue       := f64      ; required finite, ≥ 0; all pass/fail gates compare f64
GradNorm       := f32      ; required finite, ≥ 0
                       ; global L2 norm over all trainable parameter
                       ; gradients, computed after backward and before
                       ; AdamW update and gradient clearing

Verdict     := Confirmed | Refuted
HypothesisStatus :=
    Confirmed
  | Refuted
  | NotEvaluatedDueToPriorGate(reason: String)

FailureKind := Substrate | Capacity | Suspicious | Phase | Metric

; S1 does not emit a top-level generic Outcome. Use S1Outcome in §8.
;
; "Inconclusive" is not a legal S1 value.
;
; Closure-candidate reports (Decision ∈ {ProceedToS2,
; ProceedToS2-with-T12.5-prereq}) must give binary Verdict values for all
; of H1..H5. Early-failure reports — emitted before downstream evidence
; exists, e.g. the T2b divergence short-circuit before scoring or
; ablation — must mark unreachable downstream hypotheses as
; NotEvaluatedDueToPriorGate(reason) rather than asserting Confirmed
; or Refuted on evidence that does not exist.

Hypothesis  := H1 | H2 | H3 | H4 | H5

PredictedRange     := { low: BpcValue, high: BpcValue }   ; low ≤ high
ObservedStatistic  := { median: BpcValue, min: BpcValue, max: BpcValue, stddev: f64 }

CharVocab256       := byte ∈ [0, 255]
NGramOrder         := 1 | 2 | 3
SmoothingScheme    := { alpha: f64, lambdas: [f64; 3] }     ; pinned by D4

CorpusManifestRef  := { sha256: Hash256, path: String, schema_version: SemVer }

TinyStoriesManifest :=
  {
    schema: "tinystories_manifest.v1",
    source_name: String,
    source_url: String,
    source_archive_sha256: Hash256,
    decompression: String,
    train_path: String,
    val_path: String,
    train_sha256: Hash256,
    val_sha256: Hash256,
    val_shuffle_deadeef_sha256: Hash256,   ; sha256 of Fisher-Yates(V_val,
                                           ; ShuffleRng(0xDEADBEEF)); pinned
                                           ; at fixture creation; consumed
                                           ; by D7 O-metric-4
    raw_byte_policy: "post-decompression bytes; no normalization"
  }

S1 canonical TinyStories instance (concrete):
  source_name:     "roneneldan/TinyStories"
  source_url:      "https://huggingface.co/datasets/roneneldan/TinyStories"
  train_file:      "TinyStoriesV2-GPT4-train.txt"
  val_file:        "TinyStoriesV2-GPT4-valid.txt"
  train_url:       "https://huggingface.co/datasets/roneneldan/TinyStories/resolve/main/TinyStoriesV2-GPT4-train.txt"
  val_url:         "https://huggingface.co/datasets/roneneldan/TinyStories/resolve/main/TinyStoriesV2-GPT4-valid.txt"
  decompression:   "none"
  raw_byte_policy: "post-decompression bytes; no normalization. Stories are
                   separated by the literal ASCII string <|endoftext|>; this
                   separator is treated as ordinary input bytes (14 bytes)
                   and is NOT stripped, replaced, or interpreted as a
                   semantic boundary token."
  local_path:      "corpus/tinystories/raw/" (gitignored via /corpus/ rule)
  train_byte_length: 2227753162
  val_byte_length:    22502601
  train_sha256:    sha256:6418d412de72888f52b5142c761ac21a582f7d1166f0bfbdb5f03ccfdec90443
  val_sha256:      sha256:6874bae9a4c1a4e7edcf0e53b86c17817e9cf881fc75ff2368da457b80c0585d
  train_story_count: 2717699   ; counted by occurrences of <|endoftext|>
  val_story_count:    27630
  downloaded_at:   2026-05-09
  fixture_pin:     fixtures/corpora/tinystories.toml records both sha256
                   values verbatim and is the authoritative source consulted
                   by the downloader at replay time.

DomainHash(crate, type, schema_id, schema_version, canonical_json_bytes) =
  "sha256:" ++ hex(sha256(
    "gbf:" ++ crate ++ ":" ++ type ++ ":" ++ schema_id ++ ":" ++ schema_version
    ++ "\0" ++ canonical_json_bytes
  ))

Self-hash rule:
  For any artifact containing field *_self_hash, canonical_json_bytes are
  computed with that field omitted. Hashing an artifact including its own
  self-hash is forbidden.

CanonicalTensorPayloadHash:
  Hash over the ordered sequence of trainable tensors sorted by tensor name.
  The stream is explicitly framed. Hash:
    tensor count as little-endian u64,
    then for each tensor:
      tensor_name byte length as little-endian u64,
      tensor_name UTF-8 bytes,
      dtype tag as one byte (`Float32 = 0`, `TernaryI2 = 1`, `Q8_8 = 2`),
      rank as little-endian u64,
      shape as little-endian u64 dimensions,
      payload byte length as little-endian u64,
      raw tensor payload bytes in row-major order.
  The name length, rank, and payload length frames are normative; omitting them
  makes variable-length field boundaries ambiguous and is forbidden.
  Adversarial tests
  `canonical_tensor_payload_hash_frames_tensor_count_and_payload_length` and
  `canonical_tensor_payload_hash_frames_shape_rank_before_payload` witness the
  post-amendment boundary-collision cases that these frames reject.
  SafeTensors container metadata is excluded from this hash.

CanonicalCheckpointWrite:
  For any checkpoint byte-equality claim (Rep-1, O2, O9 seed-pair
  reordering), tensors are serialized in ascending tensor-name order,
  tensor metadata is deterministic, and no timestamp, path, host, build
  duration, or nondeterministic map iteration order may appear in the
  SafeTensors file.

S1CanonicalJson:
  UTF-8, sorted object keys, no insignificant whitespace, arrays in declared
  order, finite floats encoded by shortest round-trip decimal representation,
  and -0.0 normalized to 0.0.

Prediction status rule:
  Entries under a hypothesis's Predicted block are pre-registered
  expectations. They affect the verdict only when repeated under that
  hypothesis's Falsification block. Otherwise, out-of-range observations
  are reported as Surprises, not automatic Refutations.
```

bpc:

```text
For a model M and validation byte sequence V containing N bytes:

  Let chunk(i) = floor(i / 128) and start(i) = 128 * chunk(i).
  Let ctx(i) = V[start(i) .. i], the prefix within the current chunk only.

  bpc(M, V) = (1 / N) * Σ_{i=0}^{N-1} -log2(P_M(V[i] | ctx(i)))

P_M is computed by numerically stable log_softmax.
Logits are produced from the model state before consuming V[i].
For ctx(i)=ε, logits are produced from the deterministic zero initial state.

Required:
  - log2_sum is accumulated in f64; final division by N happens once.
  - N equals byte_length(V) exactly. No padding tokens included.
  - V is consumed in non-overlapping chunks of length 128; the final chunk
    may be shorter. State resets to zero at each chunk boundary.
  - The first byte of each chunk is scored from empty context.
  - This is the S1 reset-context bpc, not full-stream autoregressive bpc.
    The 3-gram baseline uses the same reset-context scoring semantics.
```

3-gram bpc (D4-pinned form):

```text
P_3gram(c | c_{-2}, c_{-1}) =
    λ_3 * P_smoothed(c | c_{-2}, c_{-1})
  + λ_2 * P_smoothed(c | c_{-1})
  + λ_1 * P_smoothed(c)

P_smoothed(c | context) =
  (count_train(context, c) + α) / (count_train(context) + α * |Σ|)

where:
  |Σ| = 256
  count_train(context) = Σ_{c∈Σ} count_train(context, c)

Count extraction:
  corpus_train is treated as one contiguous raw byte sequence.
  No synthetic <bos>, <eos>, padding, document-boundary token, or chunk-boundary
  token is inserted.

  P1 counts:
    count_train(ε, c) counts every occurrence of byte c in corpus_train.

  P2 counts:
    count_train(a, c) counts every adjacent pair (a, c) fully contained in
    corpus_train.

  P3 counts:
    count_train((a, b), c) counts every adjacent triple (a, b, c) fully
    contained in corpus_train.

  Reset-context semantics applies at scoring time: when a validation chunk
  starts, the scorer queries P1 for the first byte and P2 for the second byte.
  Training counts themselves are not chunked.

For a position i within its 128-byte chunk:
  if ctx(i)=ε:
    P(c | ctx) = P1(c)
  if ctx(i) has length 1:
    P(c | ctx) = (λ_3 + λ_2) * P2(c | c_{-1}) + λ_1 * P1(c)
  if ctx(i) has length ≥ 2:
    P(c | ctx) =
        λ_3 * P3(c | c_{-2}, c_{-1})
      + λ_2 * P2(c | c_{-1})
      + λ_1 * P1(c)

bpc_3gram(V_val) follows the same N-byte reset-context average as bpc above.

2-gram and unigram baseline bpc:

  P_unigram(c | ctx) = P1(c)

  P_2gram(c | ctx) =
    if ctx=ε:
      P1(c)
    else:
      P2(c | c_{-1})

  bpc_2gram and bpc_unigram follow the same N-byte reset-context average,
  final-short-chunk rule, and f64 accumulation requirements as bpc_3gram.
```

---

# 2. Authority rules

```text
Scope(F-S1) =
  {
    H1, H2, H3, H4, H5,
    Toy0 reference instance,
    TinyStories raw-byte loader (S1 stub form),
    3-gram baseline math,
    bpc math (S1 instance),
    AdamW Phase A run protocol,
    s1_run_log.v1, s1_score.v1, s1_negative_test.v1,
    s1_ablation.v1, s1_baseline.v1, s1_checkpoint.v1, s1_report.v1
  }

Rule Authority:
  ∀ behavior b ∈ Scope(F-S1) ∧ this RFC specifies b
  ⇒ SourceOfTruth(b) = this RFC.

Rule PlanContext:
  Behavior outside Scope informed by planv0 amendments and bd-1rb comments.
  Closed features F1, F3, F4, F6, F12
  (including LinearStateBlock at Fixed(0.5)) and the T14.1 Toy0
  ModelSizeProfile reference instance provide the substrate; their contracts
  are not amended by this RFC.

Rule CrateOwnership:
  Every behavior in Scope(F-S1) is implemented in exactly one of:
    - gbf-experiments       (NEW workspace crate; hosts s1_* operations,
                              D7 oracle suite, schema encoders, replay CLI
                              entrypoints, and the falsification suite)
    - gbf-policy            (Toy0 ModelSizeProfile reference instance)
    - gbf-model             (LinearStateBlock with Fixed(0.5))
    - gbf-train             (Phase scheduler, Burn adapter, AdamW config,
                              `qat` and `qat-ablation` feature flags)
    - gbf-data              (TinyStoriesManifest reader, raw-byte loader)
    - gbf-foundation        (Hash256, sha256 helper)
    - gbf-artifact          (CanonicalTensor, CanonicalTensorPayloadHash)
    - gbf-cli               (`gbf s1` subcommand for replay)
  No S1-specific code lives outside this set. The crate-level ownership
  table is normative; module names within each crate are illustrative
  unless explicitly tagged Required in §15.

Rule Amendment:
  Later slice changes any of:
    Toy0 dim caps
    bpc math
    3-gram baseline math
    seed list
    train budget
    pass criterion
  ⇒ Later slice's RFC must explicitly amend this RFC.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation produces
  the expected Refuted verdict on the appropriate hypothesis. Falsification
  sensitivity is a first-class proof obligation (§13 O5).
```

---

# 3. Hypothesis algebra

Every hypothesis carries a statement, predicted observables, falsification
rule, verdict mapping, and downstream consequence. H1, H2, H4, H5 are
**mandatory closure gates**. H3 is **non-closure-gating**: it still has a
binary verdict, and that verdict controls whether S2 gains the T12.5
prerequisite, but H3 Refuted does not by itself block bd-12pl closure.

## H1 Plumbing

```text
Statement:
  For every seed s, the training loop produces finite losses and finite
  gradient norms, and the early training loss decreases over pre-registered
  windows.

Predicted:
  mean_train_loss(s, steps 1..10)    ∈ [4.0, 6.5]             ; nats; uniform over 256 = ln(256)
  mean_train_loss(s, steps 91..100)  < mean_train_loss(s, steps 1..10) − 0.5
  ∀ step. grad_norm(s, step) is finite and ≥ 0
  ∃ step. grad_norm(s, step) > 0

Falsification:
  ∃ s, step. loss(s, step) is non-finite                     ⇒ Refuted
  ∃ s. mean_train_loss(s, 91..100) ≥ mean_train_loss(s, 1..10) − 0.5
                                                              ⇒ Refuted
  ∃ s. ∃ step. grad_norm(s, step) is non-finite              ⇒ Refuted
  ∃ s. ∀ step. grad_norm(s, step) = 0                        ⇒ Refuted

Surprise, not falsification:
  ∃ s. max_step grad_norm(s, step) ≥ 1e3

Mean computation:
  mean_train_loss over a step window is computed in f64 from the recorded
  per-step LossNatsPerByte values converted to f64. The comparison is
  exact under the recorded decimal values; no epsilon or tolerance is
  applied.

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  S2..S8 cannot proceed.

  If refuted by non-finite loss, non-finite grad norm, or all-zero
  gradients, investigate Burn integration, autodiff path, optimizer step,
  or RNG seeding as a substrate failure.

  If refuted only by the early-loss decrease window, treat it as a
  smoke-learning failure: investigate LR, initialization, data sampling,
  Toy0 capacity, and loss wiring before concluding the substrate is
  broken.
```

## H2 Capacity

```text
Statement:
  Toy0 (d_model=16, d_ff=32, 1 block, vocab=256) has enough representational
  power to model n-gram structure of TinyStories better than the fixed
  3-gram baseline by a margin strictly greater than 0.05 bpc, for every seed.

Predicted:
  bpc_3gram_baseline      ∈ [1.7, 2.0]                        ; sanity range only
  median(val_bpc(seed))   ∈ [1.4, 1.8]                        ; sanity range only
  ∀ s. val_bpc(s)         < bpc_3gram_baseline − 0.05         ; the actual gate

Falsification:
  ∃ s. val_bpc(s) ≥ bpc_3gram_baseline − 0.05                 ⇒ Refuted
  median(val_bpc) < 0.5                                        ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted (non-suspicious):
  Toy0 may be undersized. Open follow-up bead proposing Toy1 (d_model=32,
  d_ff=64) as the actual S1 model and re-run.

Consequence when median(val_bpc) < 0.5:
  Halt. Audit train/val split for leakage, audit bpc accumulator, audit
  corpus loader. Do not proceed to any later slice.
```

## H3 Sequence-state utility (non-closure-gating)

```text
Statement:
  The Toy0 model as a whole uses byte context enough to beat a unigram
  baseline on the same val by strictly more than 0.5 bpc.

Non-claim:
  This does not isolate LinearStateBlock's causal contribution. Isolating
  that requires a state-disabled ablation, which S1 does not require.

Predicted:
  bpc_unigram_val        ∈ [3.5, 5.0]
  ∀ s. val_bpc(s)        < bpc_unigram_val − 0.5
  model_neg_test_delta(seed=0) > 2.0

Falsification:
  ∃ s. val_bpc(s) ≥ bpc_unigram_val − 0.5                      ⇒ Refuted
  model_neg_test_delta(seed=0) ≤ 2.0                            ⇒ Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  The S1 model did not demonstrate sufficient byte-context utility at this
  scale. Do not attribute causality to LinearStateBlock without a
  state-disabled or state-varied ablation.

  For S1 planning purposes, Multi-timescale (T12.5 / bd-1y1s) becomes the
  conservative prerequisite for S2: add `blocks` edge
  S2.closure ← T12.5.
  S1 still passes if H1, H2, H4, H5 confirm. H3 is non-closure-gating,
  but it is not merely observational because it determines whether S2
  receives the T12.5 prerequisite edge.
```

## H4 Phase A cleanliness

```text
Statement:
  The phase scheduler at hardness=(Off, Off, Off) for (expert_qat,
  activation_qat, norm_qat) produces results bit-identical to an ablation
  build in which all QAT codepaths are compiled out.

Predicted:
  canonical_tensor_payload_sha(seed=0, phase_a_run)
    = canonical_tensor_payload_sha(seed=0, ablation_run)
  Whole-file safetensors byte equality is non-normative and may be
  reported separately only if the writer is canonicalized. H4 compares
  trainable tensor payloads only; checkpoint metadata, build_kind,
  SafeTensors metadata, and artifact paths must not participate in the
  H4 equality decision.
  Seeds 1..4 may be compared optionally and reported as observational.

Falsification:
  phase_a_tensor_payload_sha ≠ ablation_tensor_payload_sha
  ⇒ Refuted

Verdict:
  Confirmed if seed 0 produces bit-identical checkpoints between phase_a
  and ablation modes. Refuted otherwise.

Consequence of Refuted:
  Phase A is contaminated by later-phase code. This is a Phase contract
  bug, not a numerical issue. Block S2 until F4's phase scheduler is fixed.
```

## H5 Measurement

```text
Statement:
  bpc scoring, reset-boundary handling, 3-gram baseline math, and validation
  shuffling are implemented according to this RFC.

Predicted:
  metric_oracle_passed = true
  shuffle_multiset_preserved = true

Falsification:
  metric_oracle_passed = false                                ⇒ Refuted
  shuffle_multiset_preserved = false                          ⇒ Refuted

Verdict:
  Confirmed if all D7 measurement-oracle checks pass.
  Refuted otherwise.

Consequence of Refuted:
  bpc math or val construction is wrong. Halt. Every later slice's gate
  numbers are unreliable until this is fixed.
```

Hypothesis composition rules are formalized in §8 (Outcome algebra).

---

# 4. Experiment state machine

```text
State :=
    Configured(corpus, model_config, train_config, baseline_config)
  | BaselineFitted(state, bpc_3gram, bpc_unigram)
  | TrainAttempted(state, run_products[5])
  | Trained(state, completed_run_products[5])
  | Scored(state, val_bpc[5], grad_logs[5], weight_stats[5])
  | NegTested(state, neg_test_delta[seed_0_required])
  | AblationCompared(state, phase_a_eq_ablation[seed_0_required])
  | Reported(state, report)
  | Decided(state, decision: ProceedToS2
                          | ProceedToS2-with-T12.5-prereq
                          | Investigate(reason)
                          | Halt(reason))
```

Transitions:

```text
T0 configure:
  ∅ → Configured(c)

T1 baseline:
  Configured(c) → BaselineFitted(c, fit_3gram(c), fit_unigram(c))

T2 train:
  BaselineFitted(c, _, _) → TrainAttempted(c, [s1_train_run(c, s) for s in seeds])

T2a all completed:
  TrainAttempted(c, runs) ∧ ∀ r ∈ runs. r.completion = Completed
  → Trained(c, runs)

T2b divergence short-circuit:
  TrainAttempted(c, runs) ∧ ∃ r ∈ runs. r.completion = DivergedAt(_)
  → Reported(state, build_fail_substrate_report(state))

T3 score:
  Trained(c, runs) → Scored(c, [s1_score_bpc(runs[s], V_val) for s in seeds],
                            grads, weights)

T4 negative test (seed 0 mandatory):
  Scored(...) → NegTested(c, s1_negative_test(runs[0]))

T5 ablation (seed 0 mandatory):
  NegTested(...) → AblationCompared(c, ablation_eq(runs[0]))

T6 report:
  AblationCompared(...) → Reported(state, build_report(state))

T7 decide:
  Reported(state, r) → Decided(state, decide(r))
```

Invariants:

```text
I-S1-1
  T2 must not run until T1 has produced bpc_3gram and bpc_unigram.

I-S1-2
  T3 must score against the val split named by C and hash-pinned by manifest.

I-S1-3
  T4 must produce neg_test_delta strictly after T3 — not on partial checkpoints.

I-S1-4
  T5's ablation checkpoint for seed 0 must use the same seed,
  train_config_hash, corpus_*_sha, model_config_hash, device_profile, and
  rng stream definitions. Only the QAT code paths differ.

I-S1-5
  T6 emits exactly one s1_report.v1 instance per S1 PR. Re-runs after RFC
  amendment produce a new report with bumped rfc_revision.

I-S1-6
  Decided is final: closure of bd-12pl is gated on
  Decision ∈ {ProceedToS2, ProceedToS2-with-T12.5-prereq}.
```

---

# 5. Run protocol contract

```text
RunInputs :=
  {
    corpus_train: ByteSeq        ; sha256-pinned via manifest
    corpus_val:   ByteSeq        ; sha256-pinned via manifest
    model_config: Toy0Config     ; from ModelSizeProfile::Toy0 (T14.1)
    train_config: TrainConfig    ; pinned by D3, D10
    seed:         Seed
  }

TrainConfig :=
  {
    optimizer_steps:   10000
    batch_size:        32
    sequence_length:   128
    eval_every_steps:  1000
    eval_subset_size:  4096
    optimizer:         AdamW { lr: 1e-3, beta1: 0.9, beta2: 0.999,
                               eps: 1e-8, weight_decay: 0.0 }
    phase:             Phase::A   ; QuantHardness all Off
    rng_kind:          Pcg64Mcg
    device_profile:    S1CpuDeterministic
  }

S1CpuDeterministic :=
  {
    backend:                   Burn CPU backend pinned by dependency lockfile
    thread_count:              1
    deterministic_reductions:  true
    gpu_allowed:               false
    network_allowed:           false
    host_clock_allowed_for_training_artifacts: false
    env_exact: {
      BURN_NDARRAY_NUM_THREADS: "1",
      BURN_DETERMINISTIC:       "1",
      OMP_NUM_THREADS:          "1",
      RAYON_NUM_THREADS:        "1"
    }
    env_forbidden_unless_listed: true
  }

Rng streams:
  InitRng(seed)     = Pcg64Mcg(seed128("init", seed))
  BatchRng(seed)    = Pcg64Mcg(seed128("batch", seed))
  ShuffleRng(seed)  = Pcg64Mcg(seed128("shuffle", seed))

  seed128(domain, seed) = little_endian_u128(
    sha256("gbf:s1:" ++ domain ++ ":" ++ decimal(seed))[0..16]
  )

Uniform integer draw:
  uniform_u64_inclusive(rng, lo, hi) uses rejection sampling from u64 output
  so that every integer in [lo, hi] has exactly equal probability. Modulo
  reduction without rejection is forbidden.

Fisher-Yates shuffle:
  for i from N-1 down to 1:
    j = uniform_u64_inclusive(ShuffleRng(0xDEADBEEF), 0, i)
    swap(V[i], V[j])
  The loop order, integer draw algorithm, and byte-index basis are pinned.

RunProduct :=
    CompletedRunProduct | DivergedRunProduct

CompletedRunProduct :=
  {
    seed:                 Seed
    final_checkpoint:     SafeTensors blob
    final_checkpoint_sha: Hash256
    metadata:             CheckpointMetadata
    run_log:              RunLog
    weight_stats:         WeightStats[per eval_every_steps]
    grad_log:             GradLog[per optimizer step]
    completion:           Completed
  }

DivergedRunProduct :=
  {
    seed:                 Seed
    run_log:              RunLog
    weight_stats:         WeightStats[recorded until divergence]
    grad_log:             GradLog[recorded until divergence]
    completion:           DivergedAt(TrainStep)
    divergence_event:     {
                            step: TrainStep,
                            observed: NonFiniteLoss | NonFiniteGradNorm,
                            last_finite_loss: Null | LossNatsPerByte
                          }
  }
```

Operation:

```text
operation s1_train_run
  input:  RunInputs
  output: RunProduct

Preconditions:
  S1-Pre-1  input.corpus_*.sha256 matches manifest.
  S1-Pre-2  input.model_config equals Toy0 reference instance exactly.
  S1-Pre-3  input.train_config equals TrainConfig pinned values exactly.
  S1-Pre-4  input.seed ∈ {0, 1, 2, 3, 4}.
  S1-Pre-5  byte_length(corpus_train) ≥ sequence_length.
  S1-Pre-6  byte_length(corpus_val) > 0.

Postconditions:
  S1-Run-Ok-1
    completion = Completed
    ⇒ ∀ step ∈ 1..=optimizer_steps. run_log.loss(step) is finite.
  S1-Run-Ok-2
    completion = Completed
    ⇒ run_log records 10000 train losses for optimizer steps 1..10000,
       plus 11 eval points at steps 0, 1000, 2000, ..., 10000.
  S1-Run-Ok-3
    completion = Completed
    ⇒ final_checkpoint deserializes back to a Toy0 model with weight
       tensors identical to the in-memory model at termination.
  S1-Run-Fail-1
    completion = DivergedAt(k)
    ⇒ divergence_event.step = k and divergence_event.observed records the
       first non-finite loss or gradient norm without serializing NaN or Inf.
  S1-Run-Fail-2
    ∃ s. completion(s) = DivergedAt(_)
    ⇒ S1Outcome = Fail-substrate (per D9).
  S1-Run-Warn-1
    A 10-step mean train-loss increase greater than 2.0 is recorded as
    a surprise, not DivergedAt, unless it also produces non-finite loss.
```

---

# 6. Baseline contract

```text
BaselineInputs :=
  {
    corpus_train: ByteSeq        ; sha256-pinned
    corpus_val:   ByteSeq        ; sha256-pinned
    smoothing:    SmoothingScheme  ; pinned by D4
  }

BaselineProduct :=
  {
    bpc_3gram:           BpcValue
    bpc_2gram:           BpcValue
    bpc_unigram:         BpcValue
    counts_summary:      CountsSummary
    baseline_self_hash:  Hash256
  }
```

Operation:

```text
operation s1_fit_3gram
  input:  BaselineInputs
  output: BaselineProduct

Preconditions:
  B-Pre-1  input.corpus_*.sha256 matches manifest.
  B-Pre-2  input.smoothing exactly equals D4 values.
  B-Pre-3  byte_length(corpus_train) ≥ 3
  B-Pre-4  byte_length(corpus_val) > 0

Postconditions:
  B-Ok-1   bpc_3gram is finite, ≥ 0.
  B-Ok-2   bpc_2gram is finite, ≥ 0.
  B-Ok-3   bpc_unigram is finite, ≥ 0.
  B-Ok-4   counts_summary is reproducible: same train sha256 → same counts.
  B-Ok-5   baseline_self_hash is canonical hash of bpc_*, counts_summary,
           counts_blob_sha256, and pinned smoothing.

Reported sanity checks, not invariants:
  bpc_3gram ≤ bpc_2gram ≤ bpc_unigram

No failure mode:
  Baseline math is total. NaN/inf in baseline ⇒ implementation bug, not
  experiment outcome.
```

---

# 7. Bpc scoring contract

```text
ScoreInputs :=
  {
    checkpoint:   SafeTensors blob
    val_bytes:    ByteSeq    ; canonical val split
    chunk_size:   128         ; sliding window for sequence-state evaluation
  }

ScoreProduct :=
  {
    bpc:              BpcValue
    token_count:      u64
    log2_sum:         f64       ; reported in f64 to bound rounding
    score_self_hash:  Hash256
  }
```

Operation:

```text
operation s1_score_bpc
  input:  ScoreInputs
  output: ScoreProduct

Preconditions:
  Sc-Pre-1  checkpoint deserializes to a Toy0 model.
  Sc-Pre-2  val_bytes sha256 matches manifest.
  Sc-Pre-3  byte_length(val_bytes) > 0.

Postconditions:
  Sc-Ok-1   bpc = log2_sum / token_count
  Sc-Ok-2   token_count = byte_length(val_bytes)
  Sc-Ok-3   log2_sum is computed in f64 over the entire val,
            then divided once at the end (no per-chunk rounding).
  Sc-Ok-4   score is deterministic: same checkpoint + same val ⇒ same bpc.

Sliding-window evaluation:
  val is split into non-overlapping chunks of chunk_size = 128 bytes;
  sequence state resets to zero between chunks. The first byte of each
  chunk is scored without context.

  This is the S1 reset-context bpc, not full-stream autoregressive bpc.
  The 3-gram baseline uses the same reset-context scoring semantics, so
  the comparison is consistent.
```

---

# 8. Outcome algebra

```text
S1Outcome :=
    Pass-clean         ; H1 ∧ H2 ∧ H3 ∧ H4 ∧ H5  all Confirmed
  | Pass-with-warning  ; H1 ∧ H2 ∧ H4 ∧ H5 Confirmed; H3 Refuted
  | Fail-substrate     ; H1 Refuted, or any seed diverged
  | Fail-capacity      ; H2 Refuted (non-suspicious)
  | Fail-suspicious    ; median(val_bpc) < 0.5
  | Fail-phase         ; H4 Refuted
  | Fail-metric        ; H5 Refuted
```

Combination (mandatory checks first):

```text
if ∃ seed s. completion(s) = DivergedAt(_)             ⇒ Fail-substrate
elif H1 verdict = Refuted                              ⇒ Fail-substrate
elif H5 verdict = Refuted                              ⇒ Fail-metric
elif H4 verdict = Refuted                              ⇒ Fail-phase
elif median(val_bpc) < 0.5                             ⇒ Fail-suspicious
elif H2 verdict = Refuted                              ⇒ Fail-capacity
elif H3 verdict = Refuted                              ⇒ Pass-with-warning
else                                                   ⇒ Pass-clean
```

Decision dispatch:

```text
Pass-clean         → Decision::ProceedToS2
Pass-with-warning  → Decision::ProceedToS2-with-T12.5-prereq
Fail-capacity      → Decision::Investigate(propose-Toy1)
Fail-substrate     → Decision::Investigate(burn-or-autodiff)
Fail-phase         → Decision::Investigate(F4-phase-contract)
Fail-metric        → Decision::Halt(measurement-broken)
Fail-suspicious    → Decision::Halt(audit-split-and-bpc)
```

`Halt` blocks bd-12pl closure unconditionally. `Investigate` creates a
follow-up bead and may extend this RFC's scope or seed list.

## 8.1 Amendment A1: Toy1 successor run

This amendment is activated by the committed Toy0 result:
`S1Outcome = Fail-capacity` and `Decision = Investigate(propose-Toy1)`.
The Toy0 report remains immutable evidence for that falsification. The Toy1
successor run is a new pre-registered run identity in the same F-S1 PR, not an
edit of the Toy0 predictions or result history.

```text
Successor identity:
  model_config:  ModelSizeProfile::Toy1
                 d_model = 32
                 d_ff = 64
                 n_blocks = 2
                 vocab = 256
  report_path:   docs/experiments/S1-Toy1-report.md
  artifact_dir:  experiments/S1-toy1/

Preregistration gate:
  scripts/s1_preregistration_check.sh \
    --report docs/experiments/S1-Toy1-report.md \
    --artifact-dir experiments/S1-toy1
```

For A1 only, §5 `RunInputs.model_config` and S1-Pre-2 are amended from
`Toy0` to `Toy1`. All other D1..D10 decisions remain unchanged: raw bytes,
seed list `[0, 1, 2, 3, 4]`, train budget, optimizer, deterministic sampling,
baseline math, split, strict per-seed pass criterion, measurement oracles, and
phase-A cleanliness rules are identical to the Toy0 run.

The A1 H2 hypothesis is:

```text
Toy1 (d_model=32, d_ff=64, n_blocks=2, vocab=256) has enough
representational power to model TinyStories n-gram structure better than the
fixed 3-gram baseline by a margin strictly greater than 0.05 bpc for every
seed.
```

H1, H3, H4, and H5 retain their original meanings with `Toy1` substituted for
`Toy0` where the model identity appears. The A1 successor report may support
bd-12pl closure only if H1, A1-H2, H4, and H5 are Confirmed under the Toy1
artifacts. The original Toy0 `Fail-capacity` report must still be cited as
predecessor evidence and must not be rewritten to look like a Toy1 result.
If A1-H2 is also Refuted, the successor report remains `Fail-capacity` but its
investigation target becomes `propose-Toy2` because the `Toy1` successor has
already been executed.

## 8.2 Amendment A2: Toy1 narrow H2 waiver

This amendment is activated by the committed Toy1 result and the human
decision recorded on 2026-05-10: "that's narrow enough, we can just move ahead
then." The Toy1 report must not relabel H2 as Confirmed. It records
`S1Outcome = Fail-capacity`, `H2 = Refuted`, and a closure decision of
`ProceedToS2-with-H2-waiver(toy1-narrow-h2-miss)`.

The waiver is valid only for this exact Toy1 result class:

```text
  model_config = ModelSizeProfile::Toy1
  H1, H3, H4, H5 are Confirmed
  all five seeds Completed
  all five seeds have val_bpc < bpc_3gram
  exactly one seed misses val_bpc < bpc_3gram - 0.05
  that miss is <= 0.05 bpc beyond the H2 threshold
```

The committed Toy1 result satisfies the waiver predicate: seed 1 observed
`val_bpc = 2.6143710626756853` against H2 threshold
`2.5705440233457097`, while the 3-gram baseline itself was
`2.6205440233457096`. The miss beyond the preregistered margin is
`0.0438270393299756` bpc, and every seed still beats the 3-gram baseline.

This waiver is not a future general relaxation of H2. Wider Toy1 failures,
multiple margin misses, any seed at or above the 3-gram baseline, or any
non-Toy1 capacity failure still dispatch to `Investigate(propose-Toy2)` or the
applicable §8 failure decision.

A1 artifact history is scoped to `artifact_dir` plus `report_path`. Existing
Toy0 artifacts under `experiments/S1/` and the original
`docs/experiments/S1-report.md` are not prior Toy1 result artifacts. The first
committed Toy1 artifact hash under `experiments/S1-toy1/`, or the first Toy1
report commit containing a populated `checkpoint_self_hash`,
`score_self_hash`, `negative_self_hash`, `ablation_self_hash`, or
`baseline_self_hash`, is the A1 `first_result_commit`.

---

# 9. Artifact schemas

## 9.1 s1_checkpoint.v1

```text
Path:
  experiments/S1/checkpoints/seed-{seed}/checkpoint.safetensors
  experiments/S1/checkpoints/seed-{seed}/checkpoint.metadata.json

CheckpointMetadata (JSON) :=
  {
    schema:                  "s1_checkpoint.v1"
    seed:                    Seed
    corpus_train_sha:        Hash256
    corpus_val_sha:          Hash256
    model_config_hash:       Hash256
    train_config_hash:       Hash256
    build_kind:              "phase_a" | "ablation"
    build_config_hash:       Hash256
    dependency_lockfile_sha: Hash256
    rust_toolchain_hash:     Hash256
    device_profile_hash:     Hash256
    pass_version:            SemVer
    final_step:              TrainStep
    final_train_loss:        LossNatsPerByte
    completion:              Completed
    checkpoint_self_hash:    Hash256
  }

Invariants:
  C-Self-Hash      DomainHash(...) round-trips.
  C-Determinism    Replay with same seed + same hashes ⇒ identical safetensors bytes.
  C-NoLeakage      Replay must not depend on host clock, network, or stdin.
                   The runner enforces S1CpuDeterministic.env_exact before
                   any tensor allocation: every variable in env_exact is
                   set to its pinned value, and every other environment
                   variable is unset (env_forbidden_unless_listed = true).
                   Violation aborts the run with a non-zero exit before
                   training begins.
```

## 9.2 s1_run_log.v1

```text
Path:
  experiments/S1/runs/seed-{seed}/run-log.json
  experiments/S1/runs/seed-{seed}/grad-log.jsonl
  experiments/S1/runs/seed-{seed}/weight-stats.jsonl

RunLog (JSON) :=
  {
    schema:               "s1_run_log.v1"
    seed:                 Seed
    train_config_hash:    Hash256
    losses:               List[(TrainStep, LossNatsPerByte)] ; one per optimizer step
    eval_points:          List[(EvalStep, BpcValue)]   ; includes step 0
    final_grad_norms:     GradNormSummary
    run_log_self_hash:    Hash256
  }

Invariants:
  RL-Length     losses.length = train_config.optimizer_steps
  RL-Eval       eval_points.length = optimizer_steps / eval_every_steps + 1 = 11
  RL-Finite     every recorded value is finite (else completion = DivergedAt)
```

## 9.3 s1_score.v1

```text
Path:
  experiments/S1/scores/seed-{seed}/score.json

ScoreReport (JSON) :=
  {
    schema:               "s1_score.v1"
    seed:                 Seed
    checkpoint_sha:       Hash256
    corpus_val_sha:       Hash256
    chunk_size:           128
    token_count:          u64
    log2_sum:             f64
    bpc:                  BpcValue
    score_self_hash:      Hash256
  }
```

## 9.4 s1_negative_test.v1

```text
Path:
  experiments/S1/negative-test/seed-0.json

NegativeTestReport (JSON) :=
  {
    schema:               "s1_negative_test.v1"
    seed:                 0
    checkpoint_sha:       Hash256
    corpus_val_sha:       Hash256
    shuffle_seed:         Seed
    bpc_original:         BpcValue
    bpc_shuffled:         BpcValue
    shuffled_val_sha256:  Hash256       ; must equal manifest
                                        ; val_shuffle_deadeef_sha256
    delta:                f64
    sensitive:            Bool
    negative_self_hash:   Hash256
  }
```

## 9.5 s1_ablation.v1

```text
Path:
  experiments/S1/ablation/seed-0/ablation-report.json

AblationReport (JSON) :=
  {
    schema:                       "s1_ablation.v1"
    seed:                         0
    phase_a_checkpoint_sha:       Hash256
    ablation_checkpoint_sha:      Hash256
    phase_a_tensor_payload_sha:   Hash256
    ablation_tensor_payload_sha:  Hash256
    phase_a_eq_ablation:          Bool
    first_mismatch:               Null | { tensor: String, byte_offset: u64 }
    ablation_self_hash:           Hash256
  }
```

## 9.6 s1_baseline.v1

```text
Path:
  experiments/S1/baseline/3gram.bin
  experiments/S1/baseline/3gram-report.json
  experiments/S1/baseline/unigram-report.json

BaselineReport (JSON) :=
  {
    schema:               "s1_baseline.v1"
    corpus_train_sha:     Hash256
    corpus_val_sha:       Hash256
    smoothing:            SmoothingScheme
    bpc_3gram:            BpcValue
    bpc_2gram:            BpcValue
    bpc_unigram:          BpcValue
    counts_summary:       CountsSummary
    counts_blob_sha256:   Hash256
    baseline_self_hash:   Hash256
  }

Invariants:
  B-Self-Hash   round-trips.

Reported (not invariant):
  bpc_3gram ≤ bpc_2gram ≤ bpc_unigram
```

## 9.7 s1_report.v1

```text
Path:
  docs/experiments/S1-report.md

Front-matter (YAML, hashed into report):
  ---
  schema:                "s1_report.v1"
  s1_outcome:            S1Outcome
  decision:              Decision
  baseline_self_hash:    Hash256
  per_seed_artifacts:
    List[{
      seed: Seed,
      completion: Completed | DivergedAt(TrainStep) | NotReached,
      checkpoint_self_hash: Null | Hash256,
      run_log_self_hash: Null | Hash256,
      score_self_hash: Null | Hash256,
      negative_self_hash: Null | Hash256,
      ablation_self_hash: Null | Hash256
    }]
  generated_at:          RFC3339 UTC, informational only, excluded from report hash.
                         Report generation may read the host clock only for
                         this field. Training, scoring, baseline, ablation,
                         negative-test, and oracle artifacts must not depend
                         on host clock.
  rfc_revision:          GitCommitId | Hash256
  predictions_section_hash: Hash256
  predictions_commit:    GitCommitId
  first_result_commit:   GitCommitId
  report_self_hash:      Hash256
  ---

Required sections (markdown body):
  ## Pre-registered predictions
    Predicted ranges and pass criteria as committed before any training run.
    This section's content must appear in git history strictly before the
    first S1 result artifact commit, including baseline artifacts.

  ## Observed
    Per-seed table: val_bpc, neg_test_delta (seed 0), ablation_eq (seed 0),
    completion. Plus baseline numbers and aggregate statistics.

  ## Hypothesis verdicts
    H1, H2, H3, H4, H5 each as HypothesisStatus, with the concrete
    observation that drove each verdict.
    Closure-candidate reports must use only Confirmed | Refuted.
    Early-failure reports may use NotEvaluatedDueToPriorGate(reason)
    for hypotheses whose required observations do not exist because an
    earlier mandatory gate failed (e.g. T2b divergence short-circuit
    bypasses scoring, negative test, and ablation).

  ## Falsification analysis
    Direct citation of which prediction or falsification rule fired for
    each Refuted hypothesis.

  ## Surprises
    Anything outside predicted ranges, even if not a verdict change.

  ## Decision
    Exactly one Decision tag, justified in ≤3 sentences.

  ## Reproducibility statement
    Exact command + manifest hashes + pass_version to replay.

Invariants:
  R-Decision        Exactly one Decision tag in front-matter.
  R-AllSeeds        per_seed_artifacts and the observed per-seed table cover
                    all 5 seeds in {0,1,2,3,4}.
  R-ClosureArtifacts
                    For Decision ∈ {ProceedToS2,
                    ProceedToS2-with-T12.5-prereq}, checkpoint_self_hash,
                    run_log_self_hash, and score_self_hash are non-null for
                    all five seeds, and negative_self_hash plus
                    ablation_self_hash are non-null for seed 0.
  R-Self-Hash       report_self_hash is computed over:
                      - front-matter with generated_at and report_self_hash omitted
                      - markdown body bytes exactly as committed
                    using S1CanonicalJson for front-matter normalization.
  R-Predictions     The commit introducing the exact "Pre-registered
                    predictions" section, identified by
                    predictions_section_hash, is a strict ancestor of
                    first_result_commit. first_result_commit is the earliest
                    commit introducing any checkpoint_self_hash,
                    score_self_hash, negative_self_hash, ablation_self_hash,
                    or baseline_self_hash derived from an S1 run.
  R-AllHypotheses   All five hypotheses have an explicit HypothesisStatus.
                    For Decision ∈ {ProceedToS2,
                    ProceedToS2-with-T12.5-prereq}, every status must be a
                    binary Verdict, not NotEvaluatedDueToPriorGate.
```

The pre-registration timestamp is itself a load-bearing artifact: predictions
written after-the-fact are not pre-registered, even if textually identical.

---

# 10. Reproducibility laws

```text
Rep-1 Seed determinism
  ∀ s. replay(s, manifest) byte-identical to original(s, manifest).

Rep-2 Cross-machine determinism is NOT required for v1.
  Bit-identicality is asserted within a single machine + OS + pinned Burn
  version + pinned dependency lockfile + S1CpuDeterministic device profile.
  Cross-platform reproducibility is a future concern.

Rep-3 Corpus pinning
  Every s1_*.v1 artifact records corpus_train_sha and corpus_val_sha.
  Replay validates these sha256s against the on-disk manifest before
  proceeding.

Rep-4 Train-config pinning
  train_config_hash binds D3 + D10 values exactly. Changing any pinned
  value invalidates prior s1 artifacts.

Rep-5 Pass-version pinning
  pass_version is bumped by any change to: optimizer step semantics,
  Phase A QAT branch behavior, sequence-state forward, or initialization
  rng. Bump invalidates checkpoints.

Rep-6 RFC revision pinning
  s1_report.v1 records the git sha of this RFC at report generation. A
  re-run after this RFC is amended produces a new report with a new
  rfc_revision; old reports remain valid for their revision.

Rep-7 Per-seed isolation
  No global mutable state is shared across seeds. Seed s and seed s'
  are independent runs; no rng leakage, no shared tensor cache, no
  static mutable model registry.

Rep-8 No hidden semantic inputs
  Informational report fields such as generated_at are excluded from
  semantic hashes and closure predicates.
```

---

# 11. Negative test contract

```text
operation s1_negative_test
  input:   { checkpoint: SafeTensors, val_bytes: ByteSeq }
  output:  NegTestResult

NegTestResult :=
  {
    bpc_original:    BpcValue
    bpc_shuffled:    BpcValue
    delta:           f64     ; bpc_shuffled − bpc_original
    shuffle_seed:    Seed    ; pinned: 0xDEADBEEF for v1
    sensitive:       Bool    ; delta > 2.0
  }

Preconditions:
  N-Pre-1  shuffle_seed is fixed to 0xDEADBEEF and recorded.
  N-Pre-2  shuffle is uniform over the val byte sequence (not over chunks)
           via Fisher-Yates as defined in core notation.
  N-Pre-3  shuffle_multiset_preserved = true   (from D7 O-metric-4)

Postconditions:
  N-Ok-1   delta = bpc_shuffled − bpc_original
  N-Ok-2   sensitive = (delta > 2.0)
  N-Ok-3   sensitive = (model_neg_test_delta(seed=0) > 2.0)
           feeds H3 (model context-utility), not H5.
  N-Ok-4   The H5 metric-oracle suite of D7 is independent of this operation
           and does not depend on the trained model.

Why this matters:
  An honest model that uses byte context produces much higher loss on a
  shuffled sequence. If it doesn't, the model is context-insensitive at
  this scale — H3 fires, not H5. H5 is reserved for genuine measurement
  bugs and is tested by deterministic fixtures in D7.
```

---

# 12. Decision protocol

```text
S1 closure (bd-12pl) requires:
  1. All 5 seed runs Completed (D9).
  2. s1_report.v1 emitted with R-Predictions verified by git history.
  3. Decision ∈ {ProceedToS2, ProceedToS2-with-T12.5-prereq,
     ProceedToS2-with-H2-waiver(toy1-narrow-h2-miss)}.
  4. baseline_self_hash and per_seed_artifacts recorded in front-matter.
  5. H5 measurement-oracle checks recorded with metric_oracle_passed = true.
     The trained-model shuffle delta for seed 0 is recorded and participates
     in H3, not H5.
  6. Ablation comparison recorded for at least seed 0 with phase_a_eq_ablation = true.

S1 closure is forbidden when:
  Any of:
    Decision::Halt(_), Decision::Investigate(_),
    missing pre-registration,
    any seed completion = DivergedAt(_),
    metric_oracle_passed = false,
    ablation phase_a_eq_ablation = false,
    any required artifact (checkpoints, run_logs, baseline, report) missing
    or self-hash invalid.

If Decision = ProceedToS2-with-T12.5-prereq:
  Add `blocks` edge S2.closure (bd-1xqf) ← T12.5 (bd-1y1s).
  This is the only structural slice-graph amendment S1 may make.
```

---

# 13. Proof obligations

```text
O1  Pre-registration provability
    "Pre-registered predictions" section content of S1-report.md must
    appear in git history strictly before any S1 result artifact commit.
    CI script asserts:
      1. predictions_section_hash matches the exact normalized markdown
         section in predictions_commit;
      2. predictions_commit is a strict ancestor of first_result_commit;
      3. first_result_commit is the earliest commit that introduces any
         checkpoint_self_hash, score_self_hash, negative_self_hash,
         ablation_self_hash, or baseline_self_hash derived from S1 execution.

    This proves repository pre-registration order. It does not claim to
    prove that no uncommitted local run occurred before predictions_commit.

O2  Determinism
    Same seed + same corpus_*_sha + same train_config_hash + same
    pass_version + same device_profile + same dependency lockfile
    → bit-identical safetensors.

    v1 CI closure test:
      run seed 0 twice and assert byte equality.

    v1 law:
      all five seeds are expected to satisfy the same replay property.

O3  Measurement-oracle correctness
    metric_oracle_passed = true. (Required for closure.)
    The trained-model shuffle delta for seed 0 is recorded as an H3 input,
    not as part of H5.

O4  Ablation match
    For seed 0: phase_a_eq_ablation = true. (Required for closure.)

O5  Falsification suite
    Six deliberately-broken implementations must each produce the
    expected Refuted verdict on the corresponding hypothesis:
      F1-broken: NaN-injecting forward            → H1 Refuted
      F2-broken: gradient-zeroing hook            → H1 Refuted (loss flat)
      F3-broken: scorer consumes byte before scoring
                 or fails to reset at 128-byte boundary       → H5 Refuted
      F4-broken: phase-A leaks soft ternary       → H4 Refuted
      F5-broken: ToyTiny with d_model=2, admitted only inside the
                 falsification harness                    → H2 Refuted
      F6-broken: Fisher-Yates uses modulo-biased draw or fails to preserve
                 byte multiset                         → H5 Refuted
    These are unit tests against the s1 framework, not actual S1 runs.
    Required test files:
      gbf-experiments/tests/falsification/f1_nan_forward.rs
      gbf-experiments/tests/falsification/f2_zero_grad.rs
      gbf-experiments/tests/falsification/f3_no_reset_scorer.rs
      gbf-experiments/tests/falsification/f4_phase_a_leaks_ternary.rs
      gbf-experiments/tests/falsification/f5_toytiny_undersized.rs
      gbf-experiments/tests/falsification/f6_modulo_biased_shuffle.rs
    These tests are gated by the test-only `falsify` feature on
    gbf-experiments so the broken substitutes cannot leak into a
    release build.

O6  Hash round-trip
    Every emitted s1_*.v1 artifact round-trips through canonical JSON
    with self-hash equality.

O7  Outcome algebra totality
    Every observable combination of binary H1..H5 verdicts, per-seed
    completion states, and suspicion thresholds maps to exactly one
    S1Outcome variant under §8.

O8  No hidden inputs
    s1 artifacts depend only on:
      corpus_train, corpus_val (sha256-pinned)
      model_config (Toy0 pinned by T14.1 reference instance)
      train_config (D3, D10 pinned)
      seed
      pass_version
      gbf-train pinned dependency set
    No env-var, no host-clock, no network, no stdin.

O9  Per-seed isolation
    Seed s and seed s' produce independent run products. No shared
    mutable state.

    CI smoke checks, not a complete proof:
      1. at least two of the five seeds produce different final_checkpoint_sha;
      2. running seeds [0, 1] and [1, 0] produces the same per-seed hashes.

O10 Closure gate
    bd-12pl close is reachable iff Decision ∈ {ProceedToS2,
    ProceedToS2-with-T12.5-prereq,
    ProceedToS2-with-H2-waiver(toy1-narrow-h2-miss)}.
```

---

# 14. Minimal end-to-end theorem

```text
Theorem S1Soundness:

Given:
  corpus manifest with valid sha256 pinned in fixtures/corpora/tinystories.toml
  Toy0 reference instance (T14.1 closed, bd-1r6k)
  TrainConfig pinned per D3 + D10
  pass_version V_S1 fixed by gbf-train HEAD at S1 PR merge

If for every seed s ∈ {0, 1, 2, 3, 4}:
  s1_train_run(...)       returns Completed RunProduct
  s1_score_bpc(...)       returns finite val_bpc
And for seed 0 specifically:
  s1_negative_test(...)   records model_neg_test_delta (feeds H3)
  ablation comparison     returns phase_a_eq_ablation = true
And:
  s1_fit_3gram(...)       returns finite bpc_3gram
  D7 measurement-oracle suite returns metric_oracle_passed = true
  s1_report.v1            contains pre-registered predictions in pre-run git history

Then:
  Each of H1, H2, H4, H5 has a defined verdict in {Confirmed, Refuted}.
  H3 has a defined verdict in {Confirmed, Refuted}.

  S1Outcome is exactly one of:
    Pass-clean
    Pass-with-warning   (H3 Refuted; T12.5 prereq)
    Fail-capacity       (H2 Refuted, non-suspicious)
    Fail-substrate      (H1 Refuted or seed diverged)
    Fail-phase          (H4 Refuted)
    Fail-metric         (H5 Refuted)
    Fail-suspicious     (median bpc < 0.5)

  Decision is unique under the dispatch rule of §8.

  If S1Outcome ∈ {Pass-clean, Pass-with-warning}, S1 has produced these
  verified knowledge claims:
    – Burn substrate trains a tiny dense fp model end-to-end without
      divergence under the pinned S1 protocol.
    – Toy0 sizing is sufficient for the S1 TinyStories raw-byte 3-gram
      margin gate.
    – Phase A is uncontaminated by later-phase QAT code for the seed-0
      canonical tensor payload comparison.
    – bpc + 3-gram baseline numbers are sensitive and reproducible under
      the D7 oracle suite.

  If S1Outcome = Pass-with-warning, S1 additionally verifies that H3's
  context-utility criterion was not met and that S2 must gain the T12.5
  prerequisite edge.

  If S1Outcome = Fail-capacity, S1 verifies that Toy0 failed the
  mandatory S1 margin gate under the pinned protocol; it does not
  verify Toy0 sufficiency.

  If S1Outcome = Fail-substrate, S1 verifies that the S1 training
  substrate or smoke-learning criterion failed; no downstream capacity,
  phase, or context-utility claim is licensed unless that hypothesis has
  an explicit binary verdict in the report.

  If S1Outcome = Fail-phase, S1 verifies that Phase A is not clean with
  respect to the seed-0 ablation comparison.

  If S1Outcome = Fail-metric, S1 verifies that the measurement oracle
  failed; no reported bpc margin should be trusted.

  If S1Outcome = Fail-suspicious, S1 verifies that the suspicious-low-bpc
  sentinel fired and that split/leakage/metric audit is required.

Not proven:
  ternary survival (S2)
  charset_v1 normalization (S3)
  ArtifactOracle round-trip (S3)
  Game Boy ROM fit (S6)
  MoE benefit (S7)
  v0_success workload pass (S3)
```

---

# 15. Implementation crate layout

Scope(F-S1) is hosted in a dedicated workspace crate `gbf-experiments`
together with the existing crates that provide its substrate. This section
pins the public surface that the hypotheses and proof obligations rely on.
Module names within each crate are illustrative; only items tagged
**Required** are normative.

## 15.1 Crate map

```text
gbf-policy
  Required  ModelSizeProfile::Toy0 reference instance (T14.1, bd-1r6k).
            Dim caps d_model=16, d_ff=32, n_blocks=1, vocab=256.
  Notes     Toy0 is not redefined elsewhere; gbf-experiments imports
            ModelSizeProfile::Toy0 from this crate by reference, never by
            inline literal.

gbf-model
  Required  LinearStateBlock with Fixed(0.5) decay (bd-tnb closed). S1
            does not amend this contract.

gbf-train
  Required  Phase scheduler with Phase::A semantics, QuantHardness all
            Off (F4 closed).
  Required  AdamW config helper exposing the D10 hyperparameters
            { lr=1e-3, beta1=0.9, beta2=0.999, eps=1e-8, weight_decay=0.0 }
            as constants (no schedule, no warmup).
  Required  Burn backend aliases for CPU and CPU autodiff under the
            existing `burn-adapter` feature.
  Required  Cargo features `qat` (default-on) and `qat-ablation`
            (mutually exclusive with `qat`). See §16.

gbf-data
  Required  TinyStoriesManifest reader and raw-byte loader. The loader
            verifies train_sha256 and val_sha256 against the on-disk
            archive before yielding bytes. No NFC, no charset folding,
            no document-boundary insertion (D5).
  Required  Canonical manifest path: fixtures/corpora/tinystories.toml
            at repository root. This path is shared across S1..S8
            experiments and is not crate-local.

gbf-foundation
  Required  Hash256 type with prefix-aware parsing; sha256 helper used
            by DomainHash and every *_self_hash field.

gbf-artifact
  Required  CanonicalTensor type and CanonicalTensorPayloadHash function
            per §1 core notation. Used by the seed-0 H4 ablation
            comparator. SafeTensors container metadata is excluded from
            this hash by contract.

gbf-experiments  (NEW workspace crate)
  Owns Scope(F-S1) end-to-end. Required modules:

    gbf_experiments::s1::manifest
      TinyStoriesManifest reader; delegates to gbf-data and asserts
      manifest sha256 verification before bytes flow.

    gbf_experiments::s1::rng
      Pcg64Mcg, seed128, InitRng/BatchRng/ShuffleRng, and
      uniform_u64_inclusive (rejection sampling per D3a; modulo
      reduction is forbidden and statically audited by a deny-list
      clippy lint or hand-rolled CI grep).

    gbf_experiments::s1::device_profile
      S1CpuDeterministic enforcement: thread-count, deterministic
      reductions, env_exact application, env_forbidden_unless_listed
      enforcement, and network/host-clock/GPU rejection. The runner
      aborts with a non-zero exit before any tensor allocation if the
      environment is non-conforming.

    gbf_experiments::s1::run
      s1_train_run operation per §5. Emits CompletedRunProduct or
      DivergedRunProduct. Produces s1_checkpoint.v1, s1_run_log.v1,
      and the per-step grad and weight-stats sidecars.

    gbf_experiments::s1::baseline
      s1_fit_3gram operation per §6. Emits s1_baseline.v1 covering
      bpc_3gram, bpc_2gram, and bpc_unigram. Count extraction follows
      §6 P1/P2/P3 semantics (no synthetic BOS/EOS/padding tokens).

    gbf_experiments::s1::score
      s1_score_bpc operation per §7. The chunk-reset bpc primitive is
      shared between the model scorer and the 3-gram baseline scorer,
      ensuring identical reset-context semantics on both sides of the
      H2 pass criterion.

    gbf_experiments::s1::neg_test
      s1_negative_test operation per §11. Fisher-Yates over byte
      indices using ShuffleRng(0xDEADBEEF). Multiset preservation
      check is exercised in s1::oracle as O-metric-4 (D7).

    gbf_experiments::s1::ablation
      Seed-0 ablation comparator. Reads two checkpoints (S1-build-A
      and S1-build-B per §16), computes canonical_tensor_payload_sha
      for both, asserts byte equality of trainable-tensor payloads,
      and emits s1_ablation.v1.

    gbf_experiments::s1::oracle
      D7 measurement-oracle fixtures O-metric-1..O-metric-4. These
      are deterministic and run without a trained model.

    gbf_experiments::s1::schema
      Type definitions, S1CanonicalJson encoder, DomainHash function
      (per §1), and self-hash round-trip helpers for:
        s1_checkpoint.v1, s1_run_log.v1, s1_score.v1,
        s1_negative_test.v1, s1_ablation.v1, s1_baseline.v1,
        s1_report.v1.

    gbf_experiments::s1::report
      s1_report.v1 emitter and outcome-algebra dispatcher implementing
      §8. Authors front-matter, validates R-Decision, R-AllSeeds,
      R-Self-Hash, R-Predictions, R-AllHypotheses, and binds the
      pre-registration commit history per O1.

    gbf_experiments::s1::cli
      Public entrypoint(s) for replay. The CLI surface is the canonical
      invocation point referenced by §10 Rep-1 and §12 closure.

gbf-cli
  Required  Subcommand `gbf s1 …` dispatching into
            gbf_experiments::s1::cli. The pre-registration check, the
            determinism check, and the closure script all shell into
            this surface.
```

## 15.2 Test layout

```text
gbf-experiments/tests/falsification.rs
gbf-experiments/tests/falsification/*.rs
  Root harness plus six module files required by §13 O5; gated by the
  test-only `falsify` feature so broken substitutes cannot leak into
  release builds. Rust integration tests must be reachable from a
  root tests/*.rs target.

gbf-experiments/tests/oracle.rs
gbf-experiments/tests/oracle/*.rs
  D7 O-metric-0..O-metric-4 executed deterministically without a
  trained model.

gbf-experiments/tests/canonical_json.rs
gbf-experiments/tests/canonical_json/*.rs
  Round-trip tests for every s1_*.v1 schema (O6). Each artifact must
  serialize, hash, deserialize, re-serialize, re-hash, and produce
  byte-identical output and self-hash equality.

gbf-experiments/tests/integration.rs
gbf-experiments/tests/integration/*.rs
  End-to-end smoke run against a tiny in-repo fixture corpus (NOT
  TinyStories) used in CI to gate determinism (O2) and per-seed
  isolation (O9). The fixture corpus is sized so a 5-seed run
  completes within the project's standard test timeout.

  The full TinyStories run is gated behind a separate CI job, but
  bd-12pl closure requires that job's artifacts and s1_report.v1, not
  merely the tiny-fixture smoke run.
```

## 15.3 Artifact paths

Unchanged from §9. All run artifacts are written under the repository-root
`experiments/S1/` tree. The report is written to `docs/experiments/S1-report.md`.

## 15.4 Canonical replay command

```text
cargo run --release -p gbf-cli -- s1 replay \
  --manifest fixtures/corpora/tinystories.toml \
  --pass-version <pass_version_pinned_in_report> \
  --seed-list 0,1,2,3,4 \
  --device-profile S1CpuDeterministic
```

Under the same machine + OS + pinned Burn version + pinned dependency
lockfile + S1CpuDeterministic, this command reproduces `experiments/S1/**`
byte-for-byte per Rep-1 and Rep-2.

Optional non-normative subcommands:

```text
gbf s1 fit-baseline       runs s1_fit_3gram only
gbf s1 oracle             runs the D7 measurement-oracle suite
gbf s1 verify-determinism replays seed 0 and asserts byte equality
```

## 15.5 Workspace registration

Cargo.toml workspace `members` is amended to include `gbf-experiments`.
The crate's `Cargo.toml` declares (at minimum) workspace dependencies on
`gbf-policy`, `gbf-model`, `gbf-train`, `gbf-data`, `gbf-foundation`, and
`gbf-artifact`, with workspace-pinned versions (`= ` syntax already
enforced workspace-wide per A18).

---

# 16. Build configurations and feature flags

Two build configurations participate in the S1 contract. Both are pinned
here so downstream CI scripts and the H4 ablation comparison can refer to
them by name.

## 16.1 S1-build-A — "Phase A run"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments
Active features (workspace-resolved):
  gbf-experiments/default

gbf-experiments/default expands to:
  gbf-experiments/phase-a

gbf-experiments/phase-a expands to:
  gbf-train/qat
  gbf-train/burn-adapter

Behavior:
  QAT codepaths are present in the binary, but Phase A configures
  QuantHardness all Off. This build produces the five seeded
  checkpoints used for H1..H3 and H5 verdicts.
Build identity tag (recorded in s1_checkpoint.v1.metadata):
  build_kind = "phase_a"
```

## 16.2 S1-build-B — "Ablation"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --no-default-features \
    --features ablation
Active features:
  gbf-experiments/ablation
  gbf-train/qat-ablation
  gbf-train/burn-adapter
Behavior:
  QAT codepaths are compiled out via `qat-ablation`. Used only for the
  seed-0 H4 ablation checkpoint compared in §3 H4.
Build identity tag (recorded in s1_ablation.v1):
  build_kind = "ablation"
```

## 16.3 Feature flag contract

```text
gbf-train/qat              default-on; gates all QAT codepaths.
gbf-train/qat-ablation     mutually exclusive with `qat`; replaces
                           QAT codepaths with stubs that compile to a
                           no-op (or compile_error! if invoked).
gbf-experiments/phase-a    forwards to gbf-train/qat and
                           gbf-train/burn-adapter.
gbf-experiments/ablation   forwards to gbf-train/qat-ablation and
                           gbf-train/burn-adapter; sets the build
                           identity tag in CheckpointMetadata.
gbf-experiments/falsify    test-only; gates the F1..F6 broken
                           substitutes used by the falsification suite.

Mutual exclusion enforcement:
  gbf-train must compile_error! at the crate root when both `qat` and
  `qat-ablation` are enabled. This prevents a misconfigured CI from
  silently building an indeterminate binary that would invalidate the
  H4 ablation comparison.
```

## 16.4 Determinism budgets

```text
Both builds run under S1CpuDeterministic (§5). The runner sets each
variable in env_exact to its pinned value, and unsets every variable
not present in env_exact (env_forbidden_unless_listed = true), before
any tensor allocation:

  BURN_NDARRAY_NUM_THREADS=1
  BURN_DETERMINISTIC=1
  OMP_NUM_THREADS=1
  RAYON_NUM_THREADS=1

Violation — any unset env_exact entry, any value mismatch, or any
other variable still set in the process environment — aborts the run
with a non-zero exit before training begins.
```

## 16.5 Pre-registration CI

```text
scripts/s1_preregistration_check.sh implements §13 O1:
  1. predictions_section_hash matches the markdown section in
     predictions_commit, recomputed using S1CanonicalJson normalization
     of the report front-matter and exact byte equality of body markdown;
  2. predictions_commit is a strict ancestor of first_result_commit;
  3. first_result_commit is the earliest commit introducing any
     checkpoint_self_hash, score_self_hash, negative_self_hash,
     ablation_self_hash, or baseline_self_hash derived from S1
     execution.
Exit non-zero on any violation. Closure of bd-12pl is forbidden while
this script exits non-zero.
```

## 16.6 CI gates that block bd-12pl closure

```text
cargo test -p gbf-experiments
cargo test -p gbf-experiments --features falsify --test falsification
cargo test -p gbf-experiments --test oracle
cargo test -p gbf-experiments --test canonical_json
cargo test -p gbf-experiments --test integration
cargo build -p gbf-experiments --no-default-features --features ablation
scripts/s1_preregistration_check.sh
scripts/s1_determinism_check.sh
  (replays seed 0 and asserts byte equality of safetensors and
   run_log_self_hash; satisfies O2)
scripts/s1_isolation_check.sh
  (asserts at least two of the five seeds produce different
   final_checkpoint_sha, and that seed-pairs [0,1] and [1,0] produce
   identical per-seed hashes; satisfies O9)
```

---

# 17. Ambiguity ledger

|  ID | Ambiguity                                                                          | Chosen path                                                                | Clarifying question                                                              | Suggested final decision                                                                                                              |
| --: | ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
|  A1 | TinyStories raw bytes vs charset_v1                                                | Raw bytes (vocab=256). Defer charset_v1 to S3.                             | Should S1 use the locked 80-token charset?                                       | No. The point of S1 is to retire substrate risk before S3 brings F-G1/F-G2 cross-epic surface area in.                                |
|  A2 | Number of seeds                                                                    | 5                                                                          | Why not 3 or 10?                                                                 | 5 bounds variance well enough without 5× CI time. Pinned in D2.                                                                       |
|  A3 | bpc evaluation: full autoregressive vs windowed                                    | Windowed at chunk_size=128, state reset between chunks                     | Will this match deployment behavior?                                             | Approximately yes. Sequence state resets at boundaries in deploy. Document the upper-bound semantic explicitly.                       |
|  A4 | 3-gram smoothing scheme                                                            | Linear interp + add-α (D4)                                                 | Why not Kneser-Ney?                                                              | KN is S3's job (5-gram). S1's 3-gram is a sanity floor, not the v0_success gate.                                                      |
|  A5 | Pass criterion: per-seed strict vs aggregate                                       | Per-seed strict (D6)                                                       | Why not median-based?                                                            | Per-seed catches pathological seeds. Aggregate hides them.                                                                            |
|  A6 | Negative-test threshold: 2.0                                                       | 2.0 bpc (D7)                                                               | What if delta ≈ 1.5?                                                             | That signals metric weakness even if technically positive. 2.0 is a generous floor.                                                   |
|  A7 | Ablation comparison: byte-equality vs ulp-equality                                 | Byte-equality of safetensors                                               | Should small numerical drift be tolerated?                                       | No. Phase A is supposed to be a no-op; any drift is a contract violation.                                                             |
|  A8 | Cross-machine reproducibility                                                      | Single-machine only (Rep-2)                                                | Should CI machine match developer machine bit-exactly?                           | Eventually yes; out of scope for S1.                                                                                                  |
|  A9 | LinearStateBlock at Fixed(0.5) vs no-state                                         | Use existing closed bd-tnb implementation                                  | Should S1 ship multi-timescale (T12.5) up front?                                 | No. Multi-timescale is a quality bump for S5. S1 retires plumbing; quality is later slices' concern.                                  |
| A10 | Optimizer schedule                                                                 | Constant 1e-3 AdamW (D10)                                                  | Why no warmup/decay?                                                             | Toy0 is small enough that schedule sensitivity is low. Schedule decisions are S2/S3 concerns.                                         |
| A11 | Sequence-state initialization                                                      | Zero-init at run start; reset between chunks                               | Random-init?                                                                     | Zero is deterministic and matches deployment. Random adds an rng source for no clear gain.                                            |
| A12 | What if bpc_3gram is outside predicted [1.7, 2.0]?                                 | Report it as a surprise; do not auto-fail                                  | Should H2 be normalized against actual 3-gram?                                   | Yes. H2's pass condition is `val_bpc < bpc_3gram − 0.05`, not against the predicted range. Predicted is for sanity only.              |
| A13 | bpc as bits per byte vs bits per char                                              | Bits per byte (vocab=256)                                                  | TinyStories is mostly ASCII, so bytes ≈ chars                                    | Use bytes; matches loader exactly. S3 switches to bits per Tier 2 token.                                                              |
| A14 | Total token count for evaluation                                                   | All val bytes                                                              | Subset for speed?                                                                | Full val. Eval cost on Toy0 is negligible.                                                                                            |
| A15 | Reporting variance bands                                                           | min/median/max/stddev across 5 seeds                                       | Confidence intervals?                                                            | Skip CI; variance is a sanity check only.                                                                                             |
| A16 | Pre-registration enforcement                                                       | Git history check (R-Predictions, O1)                                      | What if predictions section is edited after runs?                                | Reject the report. CI script compares git blame of predictions section to checkpoint metadata commits.                                |
| A17 | What if all five seeds produce *identical* checkpoint bytes?                       | Suspicious. O9 asserts at least two seeds differ.                          | Could mean RNG isn't seeded per-run.                                             | Add explicit O9 assertion in CI.                                                                                                      |
| A18 | Burn version drift between writing RFC and running S1                              | Pin via Cargo.toml `=` syntax (already enforced workspace-wide)            | What if Burn ships a fix that changes numerics?                                  | Bump pass_version, re-run, document.                                                                                                  |
| A19 | RuntimeChromeBudget linkage                                                        | Not used in S1                                                             | Toy0 has byte cost; should we preflight?                                         | No. RuntimeChromeBudget is S6's concern. Toy0 byte cost is a static check that doesn't gate S1 outcome.                               |
| A20 | F0 / F1 / F4 closure dependencies                                                  | Treat as substrate; assert versions in metadata                            | What if F4 phase scheduler has a known bug?                                      | Block S1 by adding the bug bead to bd-12pl's blockers and fixing first.                                                               |
| A21 | "Decision-decided" git commit vs "Decision-honored" PR merge                       | Both required; closure is gated on PR merge AND honored Decision           | Could a Halt PR be merged with a "we'll fix later" justification?                | No. Halt blocks merge of bd-12pl's closure commit.                                                                                    |
| A22 | What if H1 confirms but loss only barely decreased (e.g. by 0.6 over 100 steps)?   | The H1 falsification rule uses 0.5 — values just above are Confirmed       | Is "barely confirmed" a Pass-with-warning?                                       | No. H1 is binary. Surprises section may flag this for follow-up, but it does not change Outcome.                                      |
| A23 | What if seed 0 ablation succeeds but seeds 1-4 don't?                              | Closure requires only seed 0; others informational                         | Should we require all five?                                                      | No for v1; ablation is expensive. Future tightening is a follow-up bead.                                                              |
| A24 | gbf-experiments dedicated crate vs. spreading code across gbf-train / gbf-data     | Dedicated crate (§15)                                                      | Why not extend gbf-train?                                                        | gbf-train is substrate (Burn adapter, phase scheduler). S1..S8 are eight falsifiable experiments; collocating them in gbf-train balloons the substrate's API and conflates "framework" with "experiment". A dedicated crate matches the slice graph and lets S2..S8 reuse the scaffolding. |
| A25 | qat-ablation feature flag vs. `--no-default-features` alone                        | Explicit `qat-ablation`, mutex with `qat` via compile_error! (§16.3)       | Why not just `--no-default-features`?                                            | --no-default-features composes poorly across workspace dependencies and gives no compile-time guarantee QAT was actually compiled out. An explicit mutually-exclusive flag is unambiguous, CI-checkable, and round-trips through the H4 ablation comparator. |
| A26 | fixtures/corpora/tinystories.toml at repo root vs. crate-local                     | Top-level fixtures/corpora/ (§15.1)                                        | Should the manifest live inside gbf-experiments?                                 | No. Corpora are shared across S1..S8; crate-local fixtures would force duplication when S4 (Project Gutenberg) arrives. Keep manifests at repo root, owned by gbf-data's loader contract.                                                  |

---

# 18. Final concise contract

```text
F-S1 First Pulse is correct when:

1.  Five seeded Toy0 dense fp Phase-A runs on TinyStories raw bytes complete
    without divergence and produce bit-identical checkpoints under replay.

2.  Every seed's val bpc beats the fixed 3-gram baseline by more than 0.05:
    val_bpc(seed) < bpc_3gram − 0.05.

3.  Phase A run is bit-identical to a no-QAT ablation for seed 0.

4.  The H5 metric-oracle suite passes. The trained-model shuffled-val delta
    is recorded for seed 0 and contributes to H3's context-utility verdict.

5.  s1_report.v1 emits pre-registered predictions in git history strictly
    before the first checkpoint commit, and concludes with exactly one
    Decision value chosen by §8 dispatch.

6.  Decision is one of {ProceedToS2, ProceedToS2-with-T12.5-prereq,
    ProceedToS2-with-H2-waiver(toy1-narrow-h2-miss)}; any other Decision
    blocks bd-12pl closure.

7.  Every JSON artifact
    (s1_checkpoint metadata, s1_run_log, s1_score, s1_negative_test,
    s1_ablation, s1_baseline, s1_report) is canonical, deterministic, and
    self-hash-valid. Binary blobs such as checkpoint.safetensors and
    3gram.bin are bound by recorded Hash256 fields.

8.  All five hypotheses have explicit verdicts in the falsification analysis
    section, with concrete observations cited.

9.  The six-test falsification suite passes: deliberately-broken
    implementations produce the expected Refuted verdicts.

10. S1 retires substrate risk only. It does not claim ternary, charset,
    oracle, ROM, MoE, or v0_success readiness — those are later slices'
    proof obligations.
```
