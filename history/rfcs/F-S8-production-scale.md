# Formal spec pack: F-S8 Production-scale + research + epic closure

> **Changelog 2026-07-04 — quality-first re-framing; matched-cycles
> architecture selection supersedes the dense-only lane.** Three inputs
> landed after the 2026-05-17 revision and change S8's premise:
>   1. **Recorded S7 outcome:** `FailParity` / `ProceedToS8DenseOnly`
>      (dense beat MoE on all 5 seeds at matched deployed **bytes**).
>   2. **Measured kernel costs** (bd-rzq5n, `docs/experiments/kernel-bakeoff/`):
>      weights-as-code ≈ 5.4 M-cycles/MAC (~4.4 B/weight of ROM code);
>      threaded dispatch ≈ 10.3 M-cycles/MAC (weights stay 0.25 B/weight
>      data). The matched-bytes framing S7 used does not model either
>      deployment: at the deployed constraint surface, bytes and cycles
>      decouple.
>   3. **UX budget (bkase, 2026-07-04): up to ~30 s/char; maximize
>      intelligence.** At 70% CPU that is ~22M M-cycles/token — so the
>      cycle budget admits ~2.1M MACs/token even on the slower 0.25 B/w
>      dispatch kernel, and **ROM capacity (8 MiB MBC5) is the binding
>      constraint**. Top-1 MoE stores k experts while spending one
>      expert's cycles, so at matched cycles/token MoE trades abundant
>      ROM for capacity — the S7 verdict does not carry over.
> Consequences (owner bead bd-3771m; a fuller protocol revision must be
> pre-registered before any S8 result artifact):
>   * The "dense-only lane" inherited from S7 is **superseded** as a
>     premise; architecture selection becomes an explicit S8 deliverable:
>     a matched-cycles sweep over dense-V3, dense-V2, MoE-V2, and mixed
>     lowerings at ≈2–4M MACs/token and ≤ ~7 MiB deployable ROM, gated by
>     val bpc on `gutenberg_val` against the KN-5 baseline (bd-2nca) plus
>     committed readable samples.
>   * Distillation (dense teacher → ternary student, already specified in
>     planv0's training design) is authorized as a primary quality lever
>     and gets an explicit arm in the sweep.
>   * `UpperBankCandidate` bring-up remains, but as one point in the
>     sweep, not the headline; the strict one-bank-per-expert regime is
>     no longer load-bearing under weights-as-code/dispatch lowerings.
>   * Deployed numeric semantics are pinned by planv0 session amendment
>     2026-07-04 §3 (u8@zp128 activations, RangePlan-proven i16
>     accumulators, Pow2-preferred scales, Argmax decode).

> **DRAFT.** This is a pre-merge scientific/experimental RFC for Slice S8 of
> the training-contract epic (bd-1rb). It is the **eighth and final** slice
> RFC and doubles as the closure-readiness checklist for the entire
> training-contract epic. It is structured to be defensible to a skeptical
> reviewer, in particular P5 Proof-of-Work Detective and P6 RFC Scope
> Sentinel. Predictions in this document are **pre-registered**; the
> R-Predictions ancestry rule of S1 §10 carries over to S8 unchanged. The
> final concise contract in §23 IS the bd-1rb closure-readiness checklist.

> **Changelog 2026-05-17 — enwiki8 dropped; Gutenberg becomes the
> production-scale corpus.** The Project Gutenberg corpus turned out
> larger than originally planned. enwiki8 has been removed from S8's
> scope; S8 now exercises the production-scale gates on Project
> Gutenberg directly. High-level structural changes:
>   1. The S4 manifest `gutenberg_manifest.v1` (90/10 train/val, test
>      partition deferred to S8) is amended in this RFC to
>      `gutenberg_manifest.v2` with an 80/10/10 train/val/test split. The
>      val partition is byte-identical to v1; the test partition is
>      pulled from books that v1 had assigned to train, using the same
>      per-book deterministic split rule with a newly pinned
>      `test_split_seed_u128`. New `train_sha256` and `test_sha256`;
>      `val_sha256` is unchanged from S4 (Option A; see Ambiguity A12).
>   2. The KN-5 baseline is rebuilt over `gutenberg_manifest.v2.train`
>      (since train shrank). Numeric ranges marked `[ESTIMATE]`.
>   3. The cross-corpus contamination check is extended to cover
>      Gutenberg-test vs TinyStories-train (in addition to S4's
>      Gutenberg-val vs TinyStories-train direction).
>   4. The test-split-once-per-pass-version discipline (D17) now binds
>      `gutenberg_manifest.v2.test_sha256`.
>   5. F16 (Multi-Corpus Training Data Preparation) is NO LONGER closed
>      at S8 — F16's scope (TinyStories + Gutenberg) was already
>      satisfied at S4, so F16 closes at S4. S8's closure set is
>      F5 (T5.5), F9, F10 (T10.15), F15 (creation), and bd-1rb itself.
>   6. REMOVED: every reference to bd-59af (T16.3 enwiki8 corpus
>      preparation) and the prior closure claim on F16 / bd-1lin.
>      bd-59af and bd-1lin must be retired or re-scoped in a separate
>      bead-graph operation; this RFC simply stops referencing them.
>   7. Falsification entry F1 renamed `gutenberg_v2_test_overlaps_train`
>      (book-level split rule prevents straddling; falsification fires
>      when the split rule is broken).
>   8. Implementation crate paths: `gbf_experiments::s8::enwiki8_manifest`
>      becomes `gbf_experiments::s8::gutenberg_manifest_v2`. The
>      `s8-enwiki8` feature is dropped; `s8-fixed`,
>      `s8-structured-width-gates`, `s8-regression` are unchanged.

This is the eighth scientific/experimental RFC in the training-contract
epic. Like S1..S7, its deliverable is **verified knowledge**, not just
code. S8 is the slice that:

  * pushes to production scale on Project Gutenberg as the canonical
    production-scale corpus, amending S4's `gutenberg_manifest.v1` to
    `gutenberg_manifest.v2` with an 80/10/10 train/val/test split,
  * brings up the `UpperBankCandidate` `ModelSizeProfile` (the "risky upper"
    profile from the planv0 amendment 2026-05-06 item 1, ~13.0 KiB/expert
    at d_model=128, d_ff=192),
  * implements the M6 adaptive-shapes research mode
    (`ExpertShapePolicy::StructuredWidthGates` supernet plus
    hardening/pruning export, F9 / bd-1ql, T9.1..T9.3),
  * gates `lambda_shape` and `lambda_overflow` on `ExpertShapePolicy` so
    they are inert under fixed `Ternary2` and active under
    `StructuredWidthGates` (T5.5 / bd-3i5),
  * names — but does not implement — the F15 post-closure follow-ups for
    non-`Ternary2` weight encodings (bd-nyen), non-Q8_8 ternary scale
    formats (bd-38om), and learned per-group ternary thresholds
    (bd-2pg2),
  * ships the full regression test script (T10.15 / bd-180) that re-runs
    every closure gate from S1..S7 plus this slice's gates under one
    deterministic CLI,
  * closes F5 (T5.5), F9, F10 (T10.15), F15 (creation), AND the
    bd-1rb training-contract epic itself. (F16 closed at S4 and is no
    longer on S8's closure list.)

S8 is not a slice that retires "one new risk." It is the slice that
re-validates every previously-retired contract at production scale and
on the largest profile, then certifies the epic.

Important interpretation:
  S8 is **the epic-closing slice**. A `Pass-clean` outcome certifies that
  every training-contract feature shipped, every cross-slice invariant
  holds at production scale, and the M6 research mode is implementable
  (whether or not it beats the fixed baseline). A
  `Pass-with-research-tail` outcome is the legal outcome when M6
  StructuredWidthGates trains and exports honestly but the hardened
  artifact does not strictly dominate the fixed `Ternary2` baseline on
  the F8 Pareto frontier — the M6 mode is then recorded as
  research-mode-only and the production default remains the fixed
  Ternary2 path. Both outcomes close bd-1rb.

```text
Spec:
  F-S8 Production-scale + research + epic closure
  Slice S8 of the training-contract epic (bd-1rb)
  Closure bead:               bd-218w
  Closes features:            F5 (T5.5 only), F9, F10 (T10.15 only),
                              F15 (creation)
                              (F16 closed at S4; not on S8's list.)
  Closes parent epic:         bd-1rb (the entire training-contract
                              revision pass)

Hypothesis-under-test:
  Five seeded UpperBankCandidate-MoE training runs on Gutenberg
  (gutenberg_manifest.v2) charset_v1 through the F4 Phase A->E ladder
  (a) pass v0_success on the gutenberg_manifest.v2 val split (val is
  byte-identical to S4's gutenberg_manifest.v1 val split), (b) beat
  the matched-deployed-bytes dense baseline on the same val split,
  (c) survive the three-way oracle agreement and the EncodedRom +
  emulator one-token harness re-validation, (d) preflight cleanly
  against UpperBankCandidate's BringUp RuntimeChromeBudget, (e) fit
  the 16 KiB ExpertBank slot per expert, AND, on a separate pinned
  ExpertShapePolicy::StructuredWidthGates supernet run, the supernet
  trains end-to-end with per-expert width selectors converging to
  one-hot, the hardening/pruning export is deterministic, the
  hardened artifact passes every gate above (a)..(e), and either
  strictly beats the fixed Ternary2 baseline on the F8 Pareto frontier
  by >= 0.02 bpc (M6 research-mode-promoting outcome) or honestly
  records parity within the same margin (M6 research-mode-only outcome).
  The new gutenberg_manifest.v2 test partition (the books reassigned
  from v1's train set into the held-out test set) is evaluated EXACTLY
  ONCE per pass_version. The full regression test script `gbf s8
  regress` re-runs every closure gate from F-S1..F-S8 in one
  deterministic CLI invocation and emits a clean
  s8_regression_summary.v1.

Owns:
  hypothesis statements H1..H13
  pre-registered prediction tables (multi-mode, multi-seed)
  gutenberg_manifest v1 -> v2 amendment contract:
    re-derived per-book split with a newly pinned
    test_split_seed_u128, 80/10/10 train/val/test fractional split,
    val_sha256 byte-identical to v1, new train_sha256 + new
    test_sha256, charset_v1 unmappable-rate bounds carried through
    from S4 D5, KN-5 baseline rebuilt over gutenberg_manifest.v2
    train, contamination check vs TinyStories (extending S4's
    Gutenberg-val direction to also cover Gutenberg-test).
  UpperBankCandidate `ModelSizeProfile` reference instance
    (d_model = 128, d_ff = 192, n_blocks = 4, n_experts = 4, n_active = 1)
  matched-deployed-bytes dense baseline at UpperBankCandidate
    (carry-through from S7's matched-bytes formula, re-instantiated at
    UpperBankCandidate)
  ExpertShapePolicy enum public surface (T9.1 / bd-3vu)
  StructuredWidthGates supernet training contract (T9.2 / bd-2oo)
  hardening/pruning export contract (T9.3 / bd-3nj)
  lambda_shape / lambda_overflow gating contract (T5.5 / bd-3i5);
    inert-under-Fixed via named contribution helper, active-under-
    StructuredWidthGates with pinned defaults plus pinned non-default
  full regression test script (T10.15 / bd-180):
    `gbf s8 regress --pass-version <v>` runs every closure gate from
    every slice, every falsification suite, every oracle suite, every
    CI gate; emits s8_regression_summary.v1 with per-slice and
    per-test verdicts.
  F15 post-closure follow-up beads:
    bd-38om (non-Q8_8 scale formats),
    bd-nyen (non-Ternary2 encodings),
    bd-2pg2 (learned per-group thresholds);
    NAMED with explicit dependency edges, NOT implemented.
  s8_*.v1 artifact schemas:
    s8_corpus_manifest.v1 (gutenberg_manifest.v2 amendment record),
    s8_baseline_kn5.v1 (Gutenberg-v2-train KN-5),
    s8_run_log.v1 (per mode, per seed),
    s8_score.v1 (val + test, where test is the once-per-pass-version
                 split),
    s8_matched_bytes_parity.v1 (UpperBankCandidate matched-bytes
                                gate),
    s8_oracle_agreement.v1 (three-way carry-through),
    s8_emulator_harness.v1 (S5 (Pick and Fit) carry-through at UpperBankCandidate),
    s8_supernet_run.v1 (StructuredWidthGates supernet log),
    s8_hardened_export.v1 (deterministic hardening artifact + manifest),
    s8_pareto_frontier.v1 (fixed-Ternary2 vs hardened-StructuredWidth),
    s8_regression_summary.v1 (T10.15 output),
    s8_followup_beads.v1 (F15 named follow-up beads),
    s8_epic_closure.v1 (bd-1rb closure-readiness checklist),
    s8_report.v1.
  S8 reproducibility laws (extends S1..S7 with epic-closure
    determinism: full regression script must produce a stable summary
    across replays, and the hardening/pruning export must be
    deterministic byte-for-byte under replay).
  S8 falsification suite (>= 10 deliberately-broken substitutes).

Does not own:
  Anything further inside the training-contract epic — S8 closes it.
  F15 implementation. F15 is created in S8 with three named children
  (bd-38om, bd-nyen, bd-2pg2). Their implementation is post-closure
  work tracked outside bd-1rb. S8 carries the F15 *creation* and
  *dependency-wiring* obligation only.
  bpc primitive; carried through unchanged from S1 §7.
  TinyStories corpus; carried through unchanged from S1 / S3.
  Gutenberg book catalog, source-format selection, header/footer
    stripping, charset_v1 per-document drop policy, dedup policy, and
    per-book split rule: carried through unchanged from S4. S8 only
    amends S4's split-fraction triple (0.90/0.10/0.00) to
    (0.80/0.10/0.10) by re-applying S4's same per-book split rule
    with a newly pinned test_split_seed_u128; book identity, body
    stripping, and per-book charset_v1 normalization are unchanged.
  F16 (Multi-Corpus Training Data Preparation): closed at S4 (T16.1
    TinyStories at S1/S3; T16.2 Gutenberg at S4). S8 does not close
    F16.
  charset_v1; carried through unchanged from S3.
  ReferenceModelBundle / ArtifactOracle / DenotationalOracle / three-
    way oracle agreement; carried through unchanged from S3.
  RuntimeChromeBudget / CompileProfile / WRAM layout / shadow_compile
    pipeline / EncodedRom / emulator harness; carried through unchanged
    from S5 (Pick and Fit). S8 re-validates them at UpperBankCandidate scale; it does
    not amend their contracts.
  MoE arch restrictions (FFN-only, two-matrix, tied embeddings); F6
    closed. S8 consumes.
  Router switch-awareness, L_switch, router collapse guardrail, dense-
    vs-MoE matched-bytes parity gate; F7 + F13 closed at S7. S8
    re-validates at UpperBankCandidate.
  The ten standard non-shape loss terms (lambda_distill, lambda_balance,
    lambda_zrouter, lambda_switch, lambda_range, lambda_zero, plus
    lm_loss + lambda_distill); F5 closed except T5.5. S8 closes T5.5.
  The Toy0/Toy1/MoeTiny ModelSizeProfile entries; F14 closed at S1/S7.
    S8 adds UpperBankCandidate to the registry as a new entry, not as
    an amendment to existing entries.
```

## Decisions

```text
D1 gutenberg_manifest v1 -> v2 amendment (test partition added)
   S4 closed `gutenberg_manifest.v1` with the 90/10/0 split fractions
   (train_fraction = 0.90, val_fraction = 0.10, test_fraction = 0.00)
   and explicitly reserved the test partition for S8 (see S4 D2,
   `test_fraction = 0.00 ; S4 does not allocate a test split;
   reserved for S8`). S8 amends the manifest to
   `gutenberg_manifest.v2` with an 80/10/10 fractional split.

   The catalog snapshot, the deterministic book selection filter, the
   1500-book target, the per-book source-format selection, the
   header/footer stripping, the charset_v1 per-document drop policy,
   the dedup policy, and the per-book split rule are all consumed
   verbatim from S4 D1 / D2 / D3 / D4 / D5. S8 does not change book
   identity, body stripping, per-book normalization, or the
   high_53_bits_as_f64 split-hash function. S8 only changes:

     1. the three split fractions, from (0.90, 0.10, 0.00) to
        (0.80, 0.10, 0.10);
     2. the split-seed string, by adding a `test_split_seed_u128`
        pinned alongside (not replacing) S4's `split_seed_u128`. The
        S4 split_seed_u128 is preserved verbatim so the val partition
        emerges byte-identical from the new procedure (see D2).

   New manifest field block (additions to the S4 schema; existing
   fields are unchanged):

     schema:               "gutenberg_manifest.v2"
     split_seed_u128:      <PRESERVED FROM S4>      ; selects the
                                                    ; train/val cut
     test_split_seed_u128: <NEW; pinned at S8 fixture creation; first
                            16 digest bytes of
                            sha256(ascii("gbf:s8:gutenberg-test-split:2026-05-17")),
                            interpreted as little-endian u128>
     split_train_fraction: 0.80
     split_val_fraction:   0.10
     split_test_fraction:  0.10
     test_path:            String                   ; on-disk path
     test_sha256:          Hash256                  ; post-strip,
                                                    ; post-charset_v1,
                                                    ; concatenated test
                                                    ; byte stream
     test_byte_length:     u64                       [ESTIMATE; pinned at
                                                    ; fixture creation
                                                    ; once train shrinks]
     test_book_count:      u32

   All other S4 GutenbergManifest fields are inherited unchanged
   (book_ids, sources, header_regex_pattern, footer_regex_pattern,
   normalization_spec_self_hash, dedup_policy, drop_count_*, etc.).

   v1 -> v2 is a STRICT AMENDMENT, not a rebuild: S4's
   gutenberg_manifest.v1 is the ancestor manifest; gutenberg_manifest.v2
   is produced by re-running S4's per-book split with the new
   fractional cut and the new test_split_seed_u128, over the exact
   same retained, non-duplicate book set. The compatibility properties
   between v1 and v2 are pinned in D2.

   v1 (the S4 closure record) remains a valid historical artifact;
   v2 is the canonical manifest consumed by S8.

   No re-archiving, no re-fetching of the Gutenberg catalog, no
   header/footer regex change.

D2 gutenberg_manifest.v2 split rule — byte-identical val, redistributed
   train + test
   The per-book split rule (S4 D2) is unchanged for the train-vs-val
   decision. For each retained, non-duplicate book id b:

     split_hash = sha256(ascii("gbf:s4:book-split:v1")
                          || split_seed_bytes
                          || le_u32(b))
     u          = high_53_bits_as_f64(split_hash)

   v1 (S4) rule:
     if u < 0.90 -> train; else val.

   v2 (S8) rule (two-stage; val partition deliberately byte-identical
   to v1):

     Stage 1 (val vs everything-else; UNCHANGED from v1):
       if u >= 0.90 -> val
       else         -> train-pool-v1                       (defer to
                                                            stage 2)

     Stage 2 (split the v1 train pool into v2 train and v2 test using
              a second deterministic hash):

       test_split_hash = sha256(ascii("gbf:s8:book-test-split:v1")
                                 || test_split_seed_bytes
                                 || le_u32(b))
       u_test          = high_53_bits_as_f64(test_split_hash)
       test_membership_function(b) =
         if u_test < (0.10 / 0.90)  ≈ 0.111111...   -> test
         else                                        -> train

   Equivalent set definition:

     val_v2   = val_v1                                     ; same books
     test_v2  = { b : b in train_v1 AND
                      test_membership_function(b) = test }
     train_v2 = train_v1 \ test_v2

   The 0.10 / 0.90 ratio re-allocates 1/9 of the v1 train pool into
   the v2 test partition, yielding the target 0.10 + 0.10 = 0.20
   total non-train fraction with the val partition unchanged.
   Equivalently, the marginal v2 fractions are 0.80 train + 0.10 val
   + 0.10 test up to per-book hash quantization noise that scales as
   1/sqrt(retained_book_count) (~1500 books).

   Compatibility invariants (must hold under replay):

     val_v2.book_ids       = val_v1.book_ids               (byte-identical)
     val_v2.train_sha256   N/A (val is val)
     val_v2.val_sha256     = val_v1.val_sha256             (SAME; pinned
                                                            ; in v2 manifest
                                                            ; from v1)
     train_v2.book_ids ∪ test_v2.book_ids = train_v1.book_ids
     train_v2.book_ids ∩ test_v2.book_ids = empty
     train_v2.train_sha256 != train_v1.train_sha256        (train shrank)
     test_v2.test_sha256   distinct, pinned at fixture creation.

   Per-split sha256 hashes pinned in
   fixtures/corpora/gutenberg.toml under the v2 amendment block:
     train_sha256:  sha256:<NEW v2 train>
     val_sha256:    sha256:<UNCHANGED from v1>
     test_sha256:   sha256:<NEW v2 test>

   No book straddles splits. Dropped books are dropped from v2 with
   the same drop reasons that applied in v1.

D3 charset_v1 normalization on gutenberg_manifest.v2 — unmappable bound
   inherited from S4 D5
   The S3 charset_v1 token table is consumed unchanged. Per S4 D4 /
   D5, the per-document unmappable_density bound is 0.02 and the
   aggregate corpus unmappable_rate bound is 0.005. These bounds are
   PROPERTIES OF THE BOOK SET, not of the split, so they continue to
   hold for the v2 corpus (the retained, non-duplicate book set is
   the same as v1). For per-split reporting in s8_corpus_manifest.v1:

     gutenberg train unmappable_rate <= 0.005   (carry-through from S4)
     gutenberg val   unmappable_rate <= 0.005   (carry-through from S4)
     gutenberg test  unmappable_rate <= 0.005   (carry-through from S4;
                                                  NEW split has the same
                                                  per-book bound)

   Unmappable byte runs are mapped to the charset_v1 <unmappable>
   sentinel token per the S3 contract. The aggregate corpus
   unmappable_rate_corpus is recorded in gutenberg_manifest.v2 as a
   v1 carry-through. Per-split rates are recorded in
   s8_corpus_manifest.v1 as unmappable_rate_train / _val / _test and
   checked against the S4-pinned aggregate bound at runtime;
   violation aborts the run with a non-zero exit before training
   begins.

D4 KN-5 baseline rebuilt over gutenberg_manifest.v2 train
   The S3-pinned 5-gram Kneser-Ney baseline math is consumed unchanged
   (S3 §6; carried through by S4 §6.2). Counts are extracted over
   the gutenberg_manifest.v2 train split (D2) after charset_v1
   token mapping. Because v2 train is a STRICT SUBSET of v1 train
   (some v1-train books were reassigned to v2 test), the v2 KN-5
   baseline is a NEW artifact distinct from the S4 baseline.

   The fitted KN-5 produces:
     bpc_kn5_gutenberg_v2_val_predicted   in [1.20, 1.60]   [ESTIMATE;
                                                              v1 baseline
                                                              hit a range
                                                              in this
                                                              band; v2
                                                              train shrank
                                                              by ~11%,
                                                              widening
                                                              uncertainty
                                                              modestly]
     bpc_kn5_gutenberg_v2_test_predicted  in [1.20, 1.60]   [ESTIMATE]

   These predicted ranges are sanity bounds only; the actual values
   are pinned at fixture creation in s8_baseline_kn5.v1 and
   referenced by hash thereafter. Predicted ranges feed H1's
   sanity checks; the closure gate (H2) compares val_bpc strictly to
   the recorded bpc_kn5_gutenberg_v2_val.

D5 contamination check vs TinyStories — extended to v2 test
   The S4-pinned contamination contract (S4 D6) is consumed unchanged
   in mechanism (13-gram window, sha256_high_u64 fingerprint, exact
   byte-window confirmation on hit, per-document stratified sampling
   for diagnostic directions). S8 EXTENDS the closure-gated direction
   set so that the new v2 test partition is also checked against
   TinyStories train:

     S4 closure-gated directions (carry-through, unchanged):
       TS_train_contains_GB_val      (S4 D6)
       GB_train_contains_TS_val      (S4 D6)
                                     ; under v2, GB_train is the
                                     ; v2 train pool

     S8-added closure-gated directions:
       TS_train_contains_GB_test     (v2 test vs TinyStories train)
       GB_test_contains_TS_val       (TinyStories val vs v2 test
                                      pool; mirror direction)

   Pinned thresholds are inherited from S4 D6:
     overlap_threshold_hard_fail    = 0.0010    [carry-through; S4]
     overlap_threshold_warn         = 0.0005    [carry-through; S4]

   Diagnostic (non-closure-gated) directions remain as defined in
   S4. The contamination sub-block in s8_corpus_manifest.v1 records
   shared_13gram_rate for every pair, including the new v2-test
   directions. Aborts via D24 if any closure-gated direction exceeds
   `overlap_threshold_hard_fail`.

D6 UpperBankCandidate `ModelSizeProfile` reference instance
   Adds one new `ModelSizeProfile` registry entry to F14 (gbf-policy):

     pub const UPPER_BANK_CANDIDATE: ModelSizeProfile = ModelSizeProfile {
         id:           "UpperBankCandidate",
         d_model:      128,
         d_ff:         192,
         n_blocks:     4,
         n_experts:    4,
         n_active:     1,
         shape_policy: ExpertShapePolicy::Fixed,
         vocab:        CHARSET_V1_VOCAB_TIE_DEFAULT_LIMIT,  // 256
         tied_io:      true,
     };

   Justification for d_model = 128 (rather than 96):
     planv0 amendment 2026-05-06 item 1 lists UpperBankCandidate as
     "d_model in {96, 128}, d_ff = 192, n_blocks = small, n_experts = 4".
     The bead bd-218w explicitly recommends d_model = 128. The
     d_model = 128 / d_ff = 192 instance is the harder budget test
     (~13.0 KiB/expert vs ~9.8 KiB at d_model = 96), and S8 is the
     epic-closing slice — it must exercise the harder profile. The
     d_model = 96 instance is not registered by S8; if it is ever
     needed, a follow-up bead (post-closure, post-F15) would add it.

   Justification for n_blocks = 4:
     "small" is informal in the planv0 amendment. Pin n_blocks = 4
     to match MoeTiny's n_blocks (S7 / F14) so the matched-bytes
     dense baseline calculation re-uses S7's per-block formula
     unchanged. n_blocks = 4 also keeps total ROM bounded so the S5
     (Pick and Fit) RuntimeChromeBudget BringUp profile (single 16 KiB ExpertBank
     class active) accommodates four experts plus the dense path.

   Justification for n_experts = 4 / n_active = 1:
     planv0 amendment item 1 pins n_experts = 4. n_active = 1
     matches S7 and the deployment default ("top-1 routing", planv0
     §"Model-side recommendations"). Top-2 routing remains
     experimental and is not in S8 scope.

   Per-expert deployed byte cost (TernaryWeightPlan formula
   ceil(rows*cols/4) + per-row Q8_8 scales + metadata):

     two-matrix expert: W_up [128 x 192], W_down [192 x 128]
       W_up   weights: 128 * 192 = 24576    -> ceil(24576/4) = 6144 bytes
       W_down weights: 192 * 128 = 24576    -> ceil(24576/4) = 6144 bytes
       W_up   scales:  192 rows * 2 bytes (Q8_8) = 384 bytes
       W_down scales:  128 rows * 2 bytes (Q8_8) = 256 bytes
       per-expert metadata: <= 64 bytes (TernaryWeightPlan-pinned)
       per-expert payload total: 6144 + 6144 + 384 + 256 + 64
                                = 12992 bytes
                                ~= 12.69 KiB

   The "13.0 KiB/expert" figure in the planv0 amendment is the
   informal upper estimate; the honest computed value is
   12992 bytes per expert. Both fit a 16 KiB ExpertBank slot
   (16384 bytes) with 16384 - 12992 = 3392 bytes of headroom for
   per-bank metadata, slot guard bytes, and reserved slack.

D7 fixed seed list
   seeds = [0, 1, 2, 3, 4]
   Inherited from S1 §D2 unchanged. All five seeds run for every
   training mode in {fixed_ternary2, structured_width_gates}. Total
   training runs per S8 PR: 5 seeds * 2 modes + 5 seeds (matched-bytes
   dense baseline at UpperBankCandidate) = 15 runs.

D8 fixed train budget per mode per seed
   optimizer_steps    = 30000
   batch_size         = 32
   sequence_length    = 128
   eval_every_steps   = 3000
   eval_subset_size   = 4096 sequences

   Justification for 30000 (vs S5's 20000, S1's 10000):
     UpperBankCandidate has ~5x the parameter count of MoeTiny
     (S7's profile) and is trained on ~30x more data than
     TinyStories. 30000 steps is the smallest power-of-three
     multiple of S5's 20000 that gives every variant past the
     30000-step QAT-hardness anneal completion. Doubling beyond
     30000 makes the 15-run cross product cost-prohibitive.
     [ESTIMATE — bump only if H2/H3 fail because of insufficient
     training time; the Surprises section is the right place for
     "almost-passed" reporting.]

D9 phase scheduler pinned for S8
   The full F4 Phase A->E ladder is exercised:
     Phase A  DenseTeacherWarmup   steps     1..  6000
     Phase B  RouterWarmup         steps  6001..  9000
     Phase C  ExpertTernaryQat     steps  9001.. 18000
     Phase D  FullNumericQat       steps 18001.. 27000
     Phase E  HardenAndSelect      steps 27001.. 30000

   Phase B is non-trivial in S8 because S8 uses a real router (n_experts
   = 4, n_active = 1). lambda_balance, lambda_zrouter, lambda_switch
   are all active in Phase B onwards per S7's contract. Phase E is
   responsible for emitting the per-mode CheckpointFrontierPoint
   record consumed by the s8_pareto_frontier.v1 emitter.

   Under ExpertShapePolicy::StructuredWidthGates, Phase E is
   *additionally* responsible for invoking the hardening/pruning
   export pass per §9.

D10 ExpertShapePolicy public surface (T9.1 / bd-3vu)
   The enum closes the public surface T9.1 owes:

     pub enum ExpertShapePolicy {
         Fixed,                                  // M0..M5; default
         StructuredWidthGates {
             row_group: u16,                     // > 0; pinned 8 in S8
             col_group: u16,                     // > 0; pinned 8 in S8
         },
     }

     impl Default for ExpertShapePolicy {
         fn default() -> Self { ExpertShapePolicy::Fixed }
     }

   row_group = col_group = 8 is the S8-pinned value; smaller groupings
   produce more selectors but inflate per-step training cost; larger
   groupings collapse the supernet to a single-axis search. 8 divides
   d_ff = 192 (24 col groups) and d_model = 128 (16 row groups)
   exactly. The grouping constants are recorded in
   s8_supernet_run.v1.

D11 StructuredWidthGates supernet training contract
   At ExpertShapePolicy::StructuredWidthGates, each expert is trained
   as a supernet over the maximum d_ff = 192. For each expert e in
   {0..4} and each col group g in {0..24}, the model carries one
   learnable selector alpha[e, g] in R. The col-group activation is
   multiplied by sigmoid(alpha[e, g] * tau(step)) where tau(step) is
   a temperature schedule:

     tau(step) =
       if step <= 18000 (end of Phase C):
         1.0 + (step / 18000) * 9.0          // 1.0 .. 10.0 linear ramp
       else if step <= 27000 (end of Phase D):
         10.0 + ((step - 18000) / 9000) * 90.0  // 10.0 .. 100.0 linear ramp
       else (Phase E):
         100.0                               // sharp; near-one-hot

   At Phase E entry (step 27001), the per-expert selectors are nearly
   one-hot. At Phase E exit (step 30000), the hardening pass
   argmax-collapses each selector to a hard one-hot mask (with the
   tiebreak rule of D13). The chosen col groups remain; the unchosen
   col groups are pruned. The hardened expert is the deployable
   Ternary2 expert. Row-group selectors operate identically with
   row_group = 8 over d_model = 128 = 16 row groups; row pruning
   produces variable-width experts whose hardened W_up rows and
   W_down columns are the chosen row groups.

   Lambda terms specifically active in StructuredWidthGates mode:
     lambda_shape    = 0.05          [ESTIMATE; pinned default]
     lambda_overflow = 0.20          [ESTIMATE; pinned default]
   plus the standard S5/S7 inherited lambdas (lambda_distill,
   lambda_balance, lambda_zrouter, lambda_switch, lambda_range,
   lambda_zero) at their inherited values.

   Per CLAUDE.md training-loss bullet "Tests for scalar
   hyperparameters such as safe bounds, temperatures, and loss weights
   must include a non-default/non-1.0 value", the S8 training-loss
   tests must additionally exercise:
     lambda_shape    = 0.10          (non-default)
     lambda_overflow = 0.40          (non-default)
   on at least one fixture seed-equivalent test. Recorded in
   gbf-train tests as `loss::shape_overflow::non_default_values`.

D12 lambda_shape / lambda_overflow gating contract (T5.5 / bd-3i5)
   Per planv0.md §"Model-side recommendations":
     "lambda_shape and lambda_overflow are disabled for fixed-shape
     Ternary2 experts, because bank fit is then a geometry/export
     property, not a differentiable training property."

   Per CLAUDE.md training-loss bullets:
     "Keep raw weighted-loss helpers honest: they must validate
     finite/non-negative raw diagnostics even when the configured
     weight is zero. If a helper intentionally skips raw computation
     for a disabled config term, name it as a contribution/composer
     helper rather than a raw weighted-loss helper."
     "Do not give raw per-term diagnostic collections an implicit
     all-zero default; enabled lambdas can otherwise hide missing
     raw loss computation. If zeros are intentional, require explicit
     fields or a named contribution helper."

   The gating implementation MUST therefore satisfy:

     1. For ExpertShapePolicy::Fixed:
          - validate_loss_config(config, &Fixed) returns
            ValidatedLossConfig with lambda_shape = 0.0 AND
            lambda_overflow = 0.0 AND inert_shape_overflow = true.
          - if the user-supplied LossConfig has lambda_shape > 0 or
            lambda_overflow > 0 under Fixed, validate_loss_config emits
            a structured warning event
            (loss.config.shape_overflow_inert_under_fixed) and forces
            the values to 0.0.
          - the loss composer uses the named contribution helper
            `inert_shape_overflow_contribution(...)` which returns
            ContributionRecord {
                raw_value:       0.0,
                weighted_value:  0.0,
                inert:           true,
                inert_reason:    "ExpertShapePolicy::Fixed",
            }
            This is NOT a raw-weighted-loss helper; it is an explicit
            contribution helper, named per the CLAUDE.md bullet.
          - the loss composer's per-term diagnostic collection has
            EXPLICIT fields for shape_contribution and
            overflow_contribution; there is NO implicit all-zero
            default.

     2. For ExpertShapePolicy::StructuredWidthGates:
          - validate_loss_config(config, &StructuredWidthGates {..})
            returns ValidatedLossConfig with the user-configured
            lambda_shape and lambda_overflow values, asserts they are
            finite and >= 0, and sets inert_shape_overflow = false.
          - if the user-supplied lambda_shape == 0 AND
            lambda_overflow == 0 under StructuredWidthGates,
            validate_loss_config emits a structured warning
            (loss.config.shape_overflow_zero_under_supernet) but does
            not abort.
          - the loss composer invokes the differentiable raw-weighted-
            loss helpers `shape_penalty_raw_weighted(...)` and
            `overflow_penalty_raw_weighted(...)`. Both validate
            finite/non-negative raw diagnostics per CLAUDE.md.

     3. CI test obligations (per CLAUDE.md "tests for scalar
        hyperparameters such as safe bounds ... must include a
        non-default/non-1.0 value"):
          - `loss::shape_overflow::fixed_inert_records_zero_with_flag`
            asserts that under Fixed with user lambdas (0.05, 0.20),
            ContributionRecord.inert = true AND raw_value = 0.0 AND
            weighted_value = 0.0 AND inert_reason = "ExpertShapePolicy::Fixed".
          - `loss::shape_overflow::structured_active_nonzero_grad`
            asserts that under StructuredWidthGates with lambdas
            (0.10, 0.40), the gradients into expert width selectors
            alpha[e, g] are finite, nonzero, and deterministic.
          - `loss::shape_overflow::sweep_inert_under_fixed`
            sweeps lambda_shape over {0.0, 0.05, 0.10, 0.50, 1.0}
            under Fixed and asserts the loss composition is
            byte-identical for every value (because every value is
            inert). This is the falsification entry F7-broken in §13.

D13 Hardening / pruning export contract — deterministic argmax with
    pinned tiebreak rule (T9.3 / bd-3nj)
   At the start of Phase E (step 27001), the model checkpoint enters a
   "pre-hardening" state. At the end of Phase E (step 30000), the
   hardening pass:

     for each expert e in {0..4}:
       for each col group g in {0..24}:
         hard_mask_col[e, g] = (alpha[e, g] == max_g' alpha[e, g'])
                               with deterministic argmax tiebreak:
                                 lowest g' wins on tie.
       for each row group r in {0..16}:
         hard_mask_row[e, r] = (alpha_row[e, r] == max_r' alpha_row[e, r'])
                               with the same tiebreak rule.

       chosen_cols(e) = { g : hard_mask_col[e, g] = 1 }
       chosen_rows(e) = { r : hard_mask_row[e, r] = 1 }

       prune W_up[e]:   keep only chosen_rows(e) (output rows) and
                          chosen_cols(e) (input cols beyond d_model)
       prune W_down[e]: symmetric

       recompute TernaryWeightPlan with the pruned (rows', cols')
       per-expert TernaryWeightPlan byte cost recomputed via
       gbf-artifact::TernaryWeightPlan::compute_byte_cost(rows', cols')

     export the hardened expert via the standard ExportVisitor;
     hardened_artifact.expert_payload_digests are emitted with the
     ACTUAL pruned dimensions, not the supernet max dimensions.

   Determinism obligations (per CLAUDE.md "Deterministic export — same
   gates + threshold produce identical output"):

     R-S8-Hard-1
       Same supernet checkpoint (canonical_tensor_payload_sha) +
       same hardening rule + same tiebreak rule
       ==> same hardened CanonicalTensor payloads byte-for-byte.

     R-S8-Hard-2
       The argmax tiebreak rule is "lowest index wins on tie." No
       random tiebreak. CI test
       `model::supernet::hardening::tiebreak_lowest_index_wins`
       asserts this on a hand-crafted fixture where two selectors
       are exactly equal.

     R-S8-Hard-3
       The hardened artifact must pass the matched-bytes parity gate
       AND fit the RuntimeChromeBudget for UpperBankCandidate's
       BringUp profile. If hardening produces an expert larger than
       16 KiB after pruning (which can happen if no col groups are
       pruned), the export aborts with a non-zero exit and S8
       Outcome = Fail-hardening.

D14 matched-deployed-bytes dense baseline at UpperBankCandidate
    (carry-through from S7)
   The S7 matched-bytes formula is consumed unchanged. Under
   UpperBankCandidate-MoE (n_experts = 4, n_active = 1), the active
   per-token expert byte cost is 12992 bytes (per D6). The dense
   baseline at the same matched-deployed-bytes target uses
   FfnPathConfig::Dense with d_ff scaled to consume the same
   per-token deployed bytes:

     dense_d_ff_matched =
       ((4 * 12992) - 64 - 256) / (2 * 128 / 4 + 2)
       (back-solving: dense W_up and W_down each cost
        ceil(d_model * d_ff / 4) + d_ff * 2 (Q8_8) + 64 metadata;
        target is sum of 4 expert payloads, less metadata.)

     Pinned closed-form value:
       dense_d_ff_matched = 760    [ESTIMATE — pin at fixture
                                    creation by recomputing exactly
                                    in s8_matched_bytes_parity.v1.
                                    The number above uses the same
                                    back-solve algebra as S7 §7;
                                    record the actual closed-form
                                    integer that satisfies the
                                    matched-bytes equality within
                                    +/- 64 bytes.]

   The dense baseline run uses ExpertShapePolicy::Fixed (the
   StructuredWidthGates supernet only applies to MoE experts). All
   five seeds. Same Phase A->E ladder, Phase B is a literal no-op
   for dense (no router; lambda_balance / lambda_zrouter /
   lambda_switch all forced to 0.0 in Phase B).

D15 closure-blocking matched-bytes parity gate at UpperBankCandidate
   ∀ s in {0..4}.
     bpc(UpperBankCandidate_MoE_fixed_seed=s, gutenberg_v2_val) <
     bpc(UpperBankCandidate_dense_matched_seed=s, gutenberg_v2_val) - 0.05

   This is a re-validation of the S7 matched-bytes parity gate at the
   larger UpperBankCandidate scale. If the gate fails on every seed,
   the larger profile does not justify MoE — Outcome = Fail-parity.

D16 closure-blocking Gutenberg v0_success gate
   ∀ s in {0..4}. ∀ mode in {fixed_ternary2, structured_width_gates_hardened}.
     v0_success_per_mode_per_seed(mode, s, gutenberg_v2_val) = Pass

   The S3-pinned v0_success WorkloadManifest is consumed unchanged.
   The eight sub-criteria from planv0 amendment 2026-05-06 item 6
   are evaluated against gutenberg_manifest.v2 val (the val partition
   is byte-identical to S4's gutenberg_manifest.v1 val; the test
   partition is reserved for D17). The "fits_runtime_chrome_estimate"
   sub-criterion is tightened from the S3 "conservative estimate" to
   the S5 (Pick and Fit) full RuntimeChromeBudget preflight at
   UpperBankCandidate's BringUp profile.

D17 gutenberg_manifest.v2 test-split discipline — used EXACTLY ONCE
    per pass_version
   The gutenberg_manifest.v2 test split (the books reassigned from
   v1 train into v2 test by the per-book test_membership_function in
   D2) is the benchmark "held-out" split. It is used exactly ONCE per
   pinned pass_version, and the result is committed to the S8 report
   *before* the closure PR is merged. After committing, the test
   split must NOT be re-evaluated against the same pass_version.
   Re-running training with new hyperparameters and re-reading the
   test split is a NEW pass_version (forced by Rep-S8-1). This
   discipline matches the held-out evaluation practice for any
   benchmark whose published number is the point.

   Operational realization:
     - the test split is loaded only inside `gbf s8 test-eval`, a
       separate CLI subcommand from `gbf s8 train` and `gbf s8
       val-eval`.
     - `gbf s8 test-eval --pass-version <v>` records the pass_version
       AND the gutenberg_manifest.v2.test_sha256 in a write-once log
       file experiments/S8/test_eval_pass_versions.jsonl. If the same
       (pass_version, test_sha256) pair appears twice in the log, the
       second invocation aborts with a non-zero exit before any test
       bytes are read.
     - `gbf s8 regress` (T10.15) does NOT invoke `gbf s8 test-eval`;
       it runs only val gates. The test gate is its own closure
       step, performed once.

D18 oracle agreement re-validation at UpperBankCandidate
   The S3 three-way oracle agreement (training output ~= ArtifactOracle
   ~= DenotationalOracle on tiny fixtures) is re-validated on the
   hardened UpperBankCandidate artifact, exactly the same numeric
   tolerance contract:
     max_abs_diff(training_logits, artifact_oracle_logits) <= 1e-4
     max_abs_diff(training_logits, denotational_oracle_logits) <= 1e-4
   on the S3-pinned tiny-fixture suite (not gutenberg_manifest.v2).
   S8 does not amend the S3 contract; it re-runs the agreement test
   on the new artifact.

D19 EncodedRom + emulator one-token harness re-validation
   The S5 (Pick and Fit) EncodedRom + emulator one-token harness is re-run on the
   hardened UpperBankCandidate artifact. The harness produces one
   token under the live ROM; that token must match the training-
   side logits within the S5 (Pick and Fit) pinned numeric tolerance for at least
   one prompt drawn from the v0_success WorkloadManifest. S8 does
   not amend the S5 (Pick and Fit) harness contract.

D20 F8 Pareto frontier — fixed-Ternary2 vs hardened-StructuredWidth
   The S5 (Pick and Fit) / F8 Pareto frontier is re-emitted at UpperBankCandidate
   scale with TWO points:
     - point P_fixed:  (UpperBankCandidate-MoE, ExpertShapePolicy::Fixed)
     - point P_hard:   (UpperBankCandidate-MoE, hardened StructuredWidthGates)
   Each point carries the standard CheckpointFrontierPoint axes
   (val_bpc_ternary, projected_deployed_bytes, conformance summary,
   shadow_compile_ok, schedule_cost_estimate). The frontier emitter
   is the F8-closed selection logic; S8 does not amend it.

   FrontierRecommendationS8 := M6Promote | M6ResearchOnly | M6Reject

     M6Promote :=
       bpc(P_hard) < bpc(P_fixed) - 0.02
       AND projected_deployed_bytes(P_hard) <= projected_deployed_bytes(P_fixed)
       AND P_hard passes every gate above (D15..D19)

     M6Reject :=
       P_hard fails any gate above
       OR bpc(P_hard) > bpc(P_fixed) + 0.05    (hardened is meaningfully worse)
       OR projected_deployed_bytes(P_hard) > projected_deployed_bytes(P_fixed) + 1024
                                              (hardened is meaningfully larger)

     M6ResearchOnly :=
       neither M6Promote nor M6Reject; P_hard is honest but not
       strictly dominant. Recorded as M6 research-mode-only.

   FrontierRecommendationS8 is a license for post-closure work; it
   does NOT block bd-1rb closure. M6Promote, M6ResearchOnly, and
   M6Reject are all legal closure-completion outcomes (see §14).

D21 F15 follow-up beads NAMED, not implemented
   S8 creates F15 (bd-stu4 already adopted under bd-1rb at
   2026-05-06; S8 wires three NEW dependency edges) and asserts that
   three named children exist with explicit closure conditions:

     bd-38om "Represent non-Q8_8 ternary scale tensor formats"
       Closure conditions: ScaleFormat::Q4_4 + ScaleFormat::Pow2 each
       have a CanonicalTensor representation, ArtifactOracle
       agreement under the new format, and the
       artifact_core_rejects_declared_non_q8_8_scale_formats_until_tensor_encoding_exists
       guard is removed or replaced. Owner: F15.

     bd-nyen "Implement non-Ternary2 artifact weight encodings"
       Closure conditions: WeightEncoding::SparseTernaryBitplanes and
       WeightEncoding::Binary1 each have a CanonicalTensor
       representation, ArtifactOracle agreement, Binary1 zero-rejection
       semantics, and the
       artifact_core_rejects_declared_non_ternary2_weight_encodings_until_tensor_encoding_exists
       guard is removed or replaced. Owner: F15.

     bd-2pg2 "Implement learned per-group ternary threshold state"
       Closure conditions: ThresholdPlan::LearnedPerGroup has concrete
       learned threshold state, artifact/export representation, Burn
       training behavior, reconstruction tests, and the
       artifact_core_rejects_learned_per_group_thresholds_until_state_is_exported
       guard is removed or replaced. Owner: F15.
       Per CLAUDE.md: "matrix thresholds mirror the QAT ternary model
       contract: one global threshold or one threshold per output row.
       Do not expose per-weight thresholds unless a model/artifact
       bead defines that public behavior." bd-2pg2's closure must
       carry this constraint into ThresholdPlan::LearnedPerGroup
       semantics: the learned thresholds are per output row OR one
       global, NOT per weight.

   S8 does not implement these. The s8_followup_beads.v1 artifact
   asserts these three beads exist under F15 (bd-stu4) with
   `blocks` edges to S8's epic-closure step removed (the beads are
   no longer blocking; bd-1rb closes without them).

D22 full regression test script (T10.15 / bd-180)
   `gbf s8 regress --pass-version <v>` is a single deterministic CLI
   that re-runs every closure gate from F-S1..F-S8. Specifically:

     for slice in [S1, S2, S3, S4, S5, S7, S8]:
       run slice.unit_test_suite           (cargo test -p ... per slice)
       run slice.falsification_suite       (cargo test ... --features falsify-<slice>)
       run slice.oracle_suite              (cargo test ... --test oracle_<slice>)
       run slice.canonical_json_suite      (cargo test ... --test canonical_json_<slice>)
       run slice.integration_suite         (tiny-fixture integration test)
       run slice.preregistration_check     (scripts/<slice>_preregistration_check.sh)
       run slice.determinism_check         (scripts/<slice>_determinism_check.sh)
       run slice.isolation_check           (scripts/<slice>_isolation_check.sh)
       record per-test pass/fail/skip with timing into
       s8_regression_summary.v1.

   Performance budget (per CONSTITUTION.md §II.1, hard wall):
     full regression script must complete in < 5 minutes on the
     pinned CI device profile. Individual slice budgets:
       S1, S2: < 30s each
       S3: < 60s
       S4: < 60s
       S5: < 90s
       S7: < 60s
       S8: < 60s
     The S5 (Pick and Fit) budget is slightly larger because EncodedRom + emulator
     replay is included.

   Exit code 0 if every per-slice block returns Pass; 1 otherwise.
   No partial-pass: any single test failure in any slice blocks
   bd-218w closure.

   The script does NOT consume the gutenberg_manifest.v2 test split
   (D17). The test gate is invoked separately, exactly once per
   pass_version, via `gbf s8 test-eval`.

D23 strict reproducibility (per mode + per seed)
   Same seed + same mode + same corpus_*_sha + same charset_v1_sha +
   same train_config_hash + same model_config_hash +
   same gbf-train pass_version + same dependency lockfile +
   same rust_toolchain_hash + same build_config_hash +
   same device_profile + same expert_shape_policy
   ==> bit-identical safetensors per (mode, seed).

   Additionally:
     same supernet checkpoint + same hardening rule + same tiebreak
     ==> bit-identical hardened ExportVisitor output.

   Additionally:
     same set of slice test outputs + same regression script
     invocation
     ==> bit-identical s8_regression_summary.v1 JSON.

D24 fail-closed on NaN / divergence / hardening overflow
   Any seed of any mode producing non-finite loss or non-finite
   gradient norm at any step fails the entire S8 with
   Fail-substrate(mode = m, seed = s, step = k).

   Additionally: any hardening pass producing a hardened expert
   payload exceeding the smallest ExpertBank slot (16384 bytes after
   reserved slack of 256 bytes) fails the entire S8 with
   Fail-hardening(seed = s).

   Additionally: any test-split eval invoked twice for the same
   pass_version fails the entire S8 with Fail-test-discipline.

D25 optimizer pinned (S1 carry-through)
   AdamW { lr = 1e-3, beta1 = 0.9, beta2 = 0.999, eps = 1e-8,
           weight_decay = 0.0 }
   No schedule. No warmup. Inherited from S1 §D10 unchanged.
```

---

# 1. Hypothesis algebra

Every hypothesis carries a statement, predicted observables,
falsification rule, verdict mapping, and downstream consequence.
H1, H2, H3, H4, H5, H6, H7, H8, H9, H11, H12, H13 are **mandatory
closure gates** for bd-218w AND for bd-1rb. H10 (M6 Pareto frontier
direction) is **non-closure-gating**: it has a binary verdict and
that verdict controls FrontierRecommendationS8 in §14, but neither
M6Promote nor M6ResearchOnly nor M6Reject blocks bd-218w closure
(they are all legal closure outcomes).

S8 is a multi-mode, multi-seed slice. Quantifiers are written
explicitly. The mode set is

  ModeSet := { fixed_ternary2,
               structured_width_gates_supernet,
               structured_width_gates_hardened,
               dense_matched_bytes }

where:
  - fixed_ternary2: UpperBankCandidate-MoE with ExpertShapePolicy::Fixed
  - structured_width_gates_supernet: UpperBankCandidate-MoE with
    ExpertShapePolicy::StructuredWidthGates, pre-hardening (Phase A->E
    entry)
  - structured_width_gates_hardened: the deployable artifact produced
    by the Phase E hardening pass over structured_width_gates_supernet
  - dense_matched_bytes: UpperBankCandidate-dense with d_ff scaled to
    consume matched deployed bytes per D14

## H1 gutenberg_manifest.v2 manifest integrity + test partition derivation

```text
Statement:
  The amended Gutenberg manifest (fixtures/corpora/gutenberg.toml,
  schema gutenberg_manifest.v2) verifies on load:
    - every book in the v1-inherited book_ids resolves on the content-
      addressed mirror/cache;
    - re-applying the S4 D2 per-book split rule with v1's preserved
      split_seed_u128 plus v2's newly pinned test_split_seed_u128
      reproduces a partition (train_v2, val_v2, test_v2) such that
      val_v2 is byte-identical to S4's val_v1 (same book set, same
      val_sha256) and test_v2 is a subset of train_v1 (no v1-val book
      reassigned to v2 test);
    - train_sha256_v2, val_sha256_v2, and test_sha256_v2 match the
      pinned values in the manifest (val_sha256_v2 = val_sha256_v1);
    - charset_v1 normalization satisfies the S4 D5 per-document and
      per-corpus unmappable bounds across train_v2 + val_v2 + test_v2;
    - the contamination check (D5) satisfies the S4-pinned thresholds
      for the closure-gated directions, including the new v2-test
      directions added by D5;
    - the KN-5 baseline rebuilt over gutenberg_manifest.v2 train
      produces bpc_kn5_gutenberg_v2_val and bpc_kn5_gutenberg_v2_test
      within the D4 sanity ranges.

Predicted:
  every_book_id_resolves_on_mirror      = true
  val_v2_book_ids = val_v1_book_ids     = true   (byte-identical)
  val_sha256_v2   = val_sha256_v1                (preserved across v1->v2)
  train_v2 ∪ test_v2 = train_v1                  (closed partition)
  train_v2 ∩ test_v2 = empty
  train_sha256_v2 != train_sha256_v1             (train shrank ~11%)
  test_sha256_v2 distinct and round-trip-stable
  unmappable_rate_train                 in [0.000, 0.005]   (S4 D5)
  unmappable_rate_val                   in [0.000, 0.005]   (S4 D5)
  unmappable_rate_test                  in [0.000, 0.005]   (S4 D5)
  shared_13gram_rate(gb_v2_*, tinystories_*)   in [0.000, 0.001] (each pair)
  bpc_kn5_gutenberg_v2_val              in [1.20, 1.60]   [ESTIMATE]
  bpc_kn5_gutenberg_v2_test             in [1.20, 1.60]   [ESTIMATE]

Falsification:
  any v1 retained book missing from v2                       ⇒ Refuted
  val_v2 differs from val_v1 in any book id                  ⇒ Refuted
  val_sha256_v2 != val_sha256_v1                             ⇒ Refuted
  test_v2 contains any book also in val_v1                   ⇒ Refuted
  train_v2 and test_v2 overlap on any book id                ⇒ Refuted
  any per-split sha256 (v2) mismatch on replay               ⇒ Refuted
  unmappable_rate_X > S4 D5 bound for X in {train,val,test}  ⇒ Refuted
  shared_13gram_rate exceeds D5 bound for any closure-gated
    pair                                                     ⇒ Refuted
  bpc_kn5_gutenberg_v2_val outside [1.00, 1.80] (hard band)  ⇒ Refuted
  bpc_kn5_gutenberg_v2_test outside [1.00, 1.80] (hard band) ⇒ Refuted

Surprise, not falsification:
  bpc_kn5_gutenberg_v2_val outside [1.20, 1.60] but inside [1.00, 1.80]

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  gutenberg_manifest.v2 manifest integrity (or test-partition
  derivation, or KN-5 baseline) is broken. Halt. Every subsequent
  gate is unreliable. Block bd-218w AND bd-1rb closure until the
  manifest is corrected. No re-running the test split under a new
  pass_version is allowed if H1 is refuted (D17 discipline still
  holds).
```

## H2 Gutenberg-v2-val v0_success on UpperBankCandidate-MoE

```text
Statement:
  ∀ s in {0..4}.
    ∀ mode in {fixed_ternary2, structured_width_gates_hardened}.
      run(mode, s) on gutenberg_manifest.v2 produces a Phase D
      ternary checkpoint whose val bpc beats
      bpc_kn5_gutenberg_v2_val by > 0.05 and whose v0_success
      per-mode-per-seed run on gutenberg_v2_val passes all eight
      sub-criteria, AND the ternary gap vs the Phase A teacher is
      bounded by 0.5 bpc.

Predicted:
  ∀ s, mode.
    val_bpc_ternary(mode, s, gutenberg_v2_val) < bpc_kn5_gutenberg_v2_val - 0.05
    val_bpc_ternary(mode, s, gutenberg_v2_val) - val_bpc_fp(mode, s, gutenberg_v2_val)
                                                                 <= 0.5
    v0_success_pass(mode, s, gutenberg_v2_val) = true

  Sanity range only:
    median over (s, mode) of val_bpc_ternary in [1.00, 1.40]
                                                  [ESTIMATE]

Falsification:
  ∃ s, mode. val_bpc_ternary(mode, s, gutenberg_v2_val)
              >= bpc_kn5_gutenberg_v2_val - 0.05                ⇒ Refuted
  ∃ s, mode. v0_success_pass(mode, s, gutenberg_v2_val) = false ⇒ Refuted
  ∃ s, mode. ternary_gap(mode, s, gutenberg_v2_val) > 0.5       ⇒ Refuted
  median over (s, mode) of val_bpc_ternary < 0.5                ⇒ Refuted
                                                                   (Suspicious)

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted (non-suspicious):
  UpperBankCandidate may be undersized for Gutenberg, OR ternary QAT
  may not survive the larger profile. Investigate phase scheduler,
  ternary projection, distillation. Open follow-up bead. Do not
  attempt Promote-to-bigger-profile without a new RFC.

Consequence when median(val_bpc_ternary) < 0.5:
  Halt. Audit train/val splits for leakage, audit bpc accumulator,
  audit corpus loader. Same suspicious-low-bpc sentinel as S1 §H2.
```

## H3 UpperBankCandidate matched-bytes parity gate

```text
Statement:
  ∀ s in {0..4}.
    bpc(UpperBankCandidate_MoE_fixed_ternary2_seed=s, gutenberg_v2_val) <
    bpc(UpperBankCandidate_dense_matched_seed=s, gutenberg_v2_val) - 0.05

  i.e. UpperBankCandidate-MoE strictly beats the matched-deployed-
  bytes dense baseline by > 0.05 bpc for every seed at the new
  larger scale.

Predicted:
  ∀ s. delta(s) = bpc(dense_matched, s) - bpc(MoE_fixed, s) > 0.05
  median over s of delta in [0.07, 0.20]   [ESTIMATE]

Falsification:
  ∃ s. delta(s) <= 0.05                                    ⇒ Refuted
  ∀ s. delta(s) <  0                                       ⇒ Refuted
                                                              (dense
                                                              strictly
                                                              wins;
                                                              MoE not
                                                              justified
                                                              at this
                                                              scale)

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  At UpperBankCandidate scale on Gutenberg, MoE does not justify its
  bank-switch cost. Investigate router collapse, expert specialization,
  matched-bytes formula. Open follow-up bead. The S7 parity gate at
  MoeTiny still holds; this Refutation says scaling MoE up to
  UpperBankCandidate did not preserve the win. May trigger a new
  RFC to investigate.
```

## H4 RuntimeChromeBudget fits at UpperBankCandidate's BringUp profile

```text
Statement:
  The S5 (Pick and Fit) RuntimeChromeBudget preflight at UpperBankCandidate's BringUp
  profile passes for both fixed_ternary2 and
  structured_width_gates_hardened modes. Specifically, every
  per-bank slot byte count is within budget AND the runtime_nucleus_hash
  CI drift gate (S5 "Pick and Fit") is clean against the UpperBankCandidate
  CompileProfile.

Predicted:
  preflight_ok(fixed_ternary2)                  = true
  preflight_ok(structured_width_gates_hardened) = true
  per_expert_payload_bytes(fixed_ternary2)      = 12992
  per_expert_payload_bytes(structured_width_gates_hardened, e)
    in [4096, 12992] for every expert e
                                                 (hardening prunes;
                                                 max retains all
                                                 groups; min keeps
                                                 only one row group
                                                 + one col group +
                                                 metadata)
  runtime_nucleus_hash_drift                    = false

Falsification:
  preflight_ok(fixed_ternary2) = false                       ⇒ Refuted
  preflight_ok(structured_width_gates_hardened) = false      ⇒ Refuted
  ∃ e. per_expert_payload_bytes(structured_width_gates_hardened, e)
       > 16384 - 256 = 16128                                  ⇒ Refuted
                                                                (hardening
                                                                overflow;
                                                                also
                                                                triggers
                                                                D24
                                                                Fail-hardening)
  runtime_nucleus_hash_drift = true                           ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  UpperBankCandidate exceeds the BringUp profile's RuntimeChromeBudget.
  Investigate metadata accounting, scale tensor packing, or reduce
  d_ff. The S5 (Pick and Fit) contract is not amended; the profile may need to be
  amended in a follow-up RFC.
```

## H5 StructuredWidthGates supernet trains end-to-end

```text
Statement:
  ∀ s in {0..4}.
    run(structured_width_gates_supernet, s) trains end-to-end without
    divergence. At the end of Phase D (step 27000), per-expert width
    selectors alpha[e, g] for every expert e and every col group g
    have converged to a near-one-hot distribution: for each expert
    e, max_g sigmoid(alpha[e, g] * tau(27000)) > 0.90 AND
    sigmoid(second_max_g) < 0.50. The same holds for row-group
    selectors.

Predicted:
  ∀ s, e.
    max_g    sigmoid(alpha_col[e, g, s] * tau(27000)) > 0.90
    second_g sigmoid(alpha_col[e, g, s] * tau(27000)) < 0.50
    max_r    sigmoid(alpha_row[e, r, s] * tau(27000)) > 0.90
    second_r sigmoid(alpha_row[e, r, s] * tau(27000)) < 0.50

  Sanity (not falsifying):
    final-step entropy per expert < 0.5 nats over the col-group
    selector distribution.

Falsification:
  ∃ s. completion(structured_width_gates_supernet, s)
        = DivergedAt(_)                                         ⇒ Refuted
                                                                  (also
                                                                  triggers
                                                                  D24)
  ∃ s, e.
       max_g sigmoid(alpha_col[e, g, s] * tau(27000)) <= 0.90  ⇒ Refuted
                                                                  (selectors
                                                                  did not
                                                                  converge)

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  The supernet did not train under the pinned schedule. Investigate
  tau(step) ramp, lambda_shape, lambda_overflow. M6 mode is
  recorded as not-implementable-under-S8; bd-1ql cannot close.
  However, fixed_ternary2 may still pass if H2/H3/H4 confirm —
  bd-218w may still close with M6 marked as Fail-supernet.
```

## H6 Hardening / pruning export determinism

```text
Statement:
  ∀ s in {0..4}.
    HardeningExport(structured_width_gates_supernet_checkpoint(s))
    is deterministic in the strong sense: replaying the hardening
    pass on the same supernet checkpoint sha produces a
    canonical_tensor_payload_sha for the hardened artifact that is
    bit-identical to the original. The argmax tiebreak rule (D13)
    is the lowest-index-wins rule, asserted on a hand-crafted
    fixture.

Predicted:
  ∀ s.
    canonical_tensor_payload_sha(hardened_artifact_replay(s))
      = canonical_tensor_payload_sha(hardened_artifact_original(s))
  tiebreak_test_passes = true
  per_expert_pruned_dims(s, e) is recorded in s8_hardened_export.v1
    with explicit (rows', cols') tuples per expert.

Falsification:
  ∃ s. canonical_tensor_payload_sha mismatch on replay         ⇒ Refuted
  tiebreak_test_passes = false                                  ⇒ Refuted
  ∃ s, e. per_expert_pruned_dims missing or null               ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  Hardening is non-deterministic OR the tiebreak rule is wrong OR
  the export pipeline is missing per-expert dimension records.
  Block bd-1ql closure. Investigate ExportVisitor for variable-width
  experts, deterministic argmax implementation. The fixed_ternary2
  path is unaffected.
```

## H7 lambda_shape / lambda_overflow gating contract

```text
Statement:
  Per D12:
    Under ExpertShapePolicy::Fixed:
      validate_loss_config returns lambda_shape = 0.0 AND
      lambda_overflow = 0.0 AND inert_shape_overflow = true,
      regardless of user-supplied values; the named contribution
      helper inert_shape_overflow_contribution returns
      ContributionRecord with raw_value = 0.0, weighted_value = 0.0,
      inert = true, inert_reason = "ExpertShapePolicy::Fixed".
    Under ExpertShapePolicy::StructuredWidthGates:
      validate_loss_config returns user-configured lambda_shape and
      lambda_overflow values (validated finite/non-negative), AND
      inert_shape_overflow = false; the differentiable raw-weighted
      helpers shape_penalty_raw_weighted and overflow_penalty_raw_weighted
      validate finite/non-negative raw diagnostics AND produce finite,
      nonzero, deterministic gradients into expert width selectors
      alpha[e, g] and alpha_row[e, r] under autodiff.

Predicted:
  fixed_inert_records_zero_with_flag                  = Pass
  structured_active_nonzero_grad                      = Pass
  sweep_inert_under_fixed                             = Pass
  raw_helpers_validate_finite_under_zero_lambda       = Pass
  contribution_collection_has_explicit_fields         = Pass

Falsification:
  fixed_inert_records_zero_with_flag = Fail                  ⇒ Refuted
  structured_active_nonzero_grad = Fail                      ⇒ Refuted
  sweep_inert_under_fixed = Fail                             ⇒ Refuted
                                                                (Fixed
                                                                composition
                                                                drifts
                                                                with
                                                                lambda_shape;
                                                                gating
                                                                broken)
  raw_helpers_validate_finite_under_zero_lambda = Fail       ⇒ Refuted
                                                                (CLAUDE.md
                                                                bullet
                                                                violated)
  contribution_collection_has_explicit_fields = Fail         ⇒ Refuted
                                                                (CLAUDE.md
                                                                "implicit
                                                                all-zero
                                                                default"
                                                                bullet
                                                                violated)

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  T5.5 / bd-3i5 cannot close. Block bd-218w closure. Block bd-1rb
  closure. The gating policy is the load-bearing contract that
  separates honest M6 research from cargo-culted phantom-gradient
  loss terms.
```

## H8 Three-way oracle agreement re-validated on hardened
        UpperBankCandidate artifact

```text
Statement:
  The S3-pinned three-way oracle agreement test
  (training_logits ≈ ArtifactOracle ≈ DenotationalOracle within
  max_abs_diff <= 1e-4 in f32) passes on the hardened UpperBankCandidate
  Phase D ternary artifact. The oracle agreement is asserted on the
  S3-pinned tiny-fixture suite, NOT on gutenberg_manifest.v2 (the
  fixture suite is already big enough; the production-scale corpus is
  a quality benchmark, not an oracle benchmark).

Predicted:
  ∀ s in {0..4}.
    max_abs_diff(training_logits(s), artifact_oracle_logits(s))
      <= 1e-4 in f32 on every fixture
    max_abs_diff(training_logits(s), denotational_oracle_logits(s))
      <= 1e-4 in f32 on every fixture

Falsification:
  ∃ s, fixture. max_abs_diff > 1e-4                          ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  S3 contract is broken on UpperBankCandidate scale. Block bd-218w
  closure. Investigate ExportVisitor at the new scale, hardening
  pass artifact representation, or oracle numeric profile.
```

## H9 EncodedRom + emulator one-token harness re-validated

```text
Statement:
  The S5 (Pick and Fit) pinned EncodedRom + emulator one-token harness produces at
  least one valid token under live ROM execution on the hardened
  UpperBankCandidate artifact, matching the training-side logits
  within the S5 (Pick and Fit) pinned numeric tolerance for at least one v0_success
  prompt.

Predicted:
  ∀ s in {0..4}. ∃ prompt p in v0_success WorkloadManifest.
    emulator_token(s, p) is well-formed AND
    max_abs_diff(emulator_logits(s, p), training_logits(s, p))
      <= S6_PINNED_TOLERANCE  (carry-through; do not amend;
                               constant now owned by
                               gbf_experiments::s5)

Falsification:
  ∃ s. ∀ prompt p. max_abs_diff exceeds S6_PINNED_TOLERANCE ⇒ Refuted
  ∃ s. emulator harness aborts with FAULT                  ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  S5 (Pick and Fit) emulator harness contract broken at UpperBankCandidate scale.
  Investigate ROM placement, bank switching, or the hardened
  artifact's BankPlan. Block bd-218w closure.
```

## H10 M6 Pareto frontier direction (non-closure-gating)

```text
Statement:
  The F8 Pareto frontier at UpperBankCandidate scale either:
    (a) strictly favors structured_width_gates_hardened over
        fixed_ternary2 by >= 0.02 bpc with no deployed-bytes
        increase (M6Promote), OR
    (b) reports parity within +/- 0.05 bpc and within +/- 1024
        deployed bytes (M6ResearchOnly), OR
    (c) strictly favors fixed_ternary2 over hardened by >= 0.05
        bpc OR by >= 1024 deployed bytes (M6Reject).

  The frontier emitter (F8-closed) categorizes which branch fires.
  This hypothesis is non-closure-gating: all three outcomes are
  legal closure outcomes.

Predicted:
  FrontierRecommendationS8 in {M6Promote, M6ResearchOnly, M6Reject}
  No prediction on which branch fires. M6 is a research mode; we
  pre-register that it will produce ONE of the three outcomes,
  not which one.

Falsification:
  FrontierRecommendationS8 not in {M6Promote, M6ResearchOnly, M6Reject}
                                                              ⇒ Refuted
                                                                (frontier
                                                                emitter
                                                                broken)
  s8_pareto_frontier.v1 missing P_fixed or P_hard            ⇒ Refuted
  s8_pareto_frontier.v1 produces inconsistent outcome on
    replay (different branch on same data)                   ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise (regardless of which of the three branches
  fired).

Consequence of Refuted:
  F8 frontier emitter is broken at UpperBankCandidate scale.
  Block bd-1ql closure. Investigate F8 Pareto selection logic.
  Other modes unaffected.

Consequence of Confirmed (per branch):
  M6Promote      -> M6 promoted to default-on for future profiles;
                    bd-1ql closes with M6 = production-ready.
  M6ResearchOnly -> M6 recorded as research-mode-only; bd-1ql
                    closes with M6 = research-only.
  M6Reject       -> M6 honestly tried and did not pay off at
                    UpperBankCandidate scale; bd-1ql closes with
                    M6 = research-rejected (the enum still exists;
                    StructuredWidthGates is still implemented;
                    the recommendation is "do not deploy").
```

## H11 Full regression script clean

```text
Statement:
  `gbf s8 regress --pass-version <pass_version_pinned_in_S8_RFC>`
  re-runs every closure gate from F-S1..F-S8 in the order pinned
  by D22 and reports per-slice and per-test verdicts. Every per-slice
  block returns Pass.

Predicted:
  s8_regression_summary.v1.per_slice = {
    S1: Pass, S2: Pass, S3: Pass, S4: Pass,
    S5: Pass, S7: Pass, S8: Pass
  }
  s8_regression_summary.v1.total_tests >= TOTAL_KNOWN_LOWER_BOUND
                                          [ESTIMATE: >= 200]
  s8_regression_summary.v1.skipped = 0
  total_runtime_seconds <= 300            (D22 5-minute hard wall)

Falsification:
  ∃ slice X in [S1..S8]. per_slice[X] = Fail                 ⇒ Refuted
  s8_regression_summary.v1.skipped > 0                        ⇒ Refuted
                                                                (no
                                                                skips
                                                                allowed
                                                                under
                                                                pinned
                                                                pass_version)
  total_runtime_seconds > 300                                 ⇒ Refuted
                                                                (perf
                                                                budget
                                                                violation;
                                                                Halt)
  s8_regression_summary.v1 missing or self-hash invalid       ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  T10.15 cannot close. F10 cannot close. bd-218w cannot close.
  bd-1rb cannot close. Investigate the failing slice; do not
  attempt to skip the failing test.
```

## H12 F15 follow-up beads created

```text
Statement:
  Three F15 sub-beads exist with the IDs and contents pinned in
  D21:
    bd-38om (non-Q8_8 scale formats)
    bd-nyen (non-Ternary2 weight encodings)
    bd-2pg2 (learned per-group thresholds)
  Each is parented under bd-stu4 (F15) AND has its `blocks` edge to
  bd-218w REMOVED (the beads no longer block S8 closure; they are
  follow-up work). Each carries an explicit "closure conditions"
  comment per D21.

Predicted:
  for bead_id in [bd-38om, bd-nyen, bd-2pg2]:
    br show bead_id reports parent = bd-stu4
    br show bead_id reports no `blocks` edge to bd-218w
    br show bead_id contains the D21-pinned closure conditions text

Falsification:
  any of the three beads is missing                          ⇒ Refuted
  any of the three beads is not parented under bd-stu4        ⇒ Refuted
  any of the three beads still has a `blocks` edge to bd-218w ⇒ Refuted
                                                                (would
                                                                make
                                                                bd-218w
                                                                un-closable
                                                                until
                                                                F15 is
                                                                done;
                                                                violates
                                                                the
                                                                "named,
                                                                not
                                                                implemented"
                                                                rule)
  any bead is missing the D21-pinned closure conditions text  ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  F15 creation contract is broken. Block bd-218w closure. Fix the
  bead-graph and re-verify; this is purely a tracking-graph
  obligation, not a code change.
```

## H13 Epic closure-readiness checklist clean

```text
Statement:
  bd-1rb's closure-readiness checklist (the §23 final concise
  contract) is all green: every direct feature child of bd-1rb is
  closed (F0..F16, with F15 explicitly named-not-implemented and
  bd-1rb's parent-child edge to F15 NOT closure-blocking; F16 closed
  at S4 carry-through and is verified-Closed, not re-closed, here),
  cargo check / clippy / test --workspace --all-features all pass
  on the pinned dependency lockfile, and the s8_epic_closure.v1
  artifact records every feature child's closure_id.

Predicted:
  for feature in [F0, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10,
                  F11, F12, F13, F14, F16]:
    feature.status = Closed   (F16 verified-Closed at S4 carry-through)
  F15.status = Open  (named, not implemented, with three children
                      tracked and not blocking bd-1rb)
  cargo_check_workspace_all_features_pass     = true
  cargo_clippy_workspace_all_features_pass    = true
  cargo_test_workspace_all_features_pass      = true
  s8_epic_closure.v1.feature_records.length   = 17
  s8_epic_closure.v1.bd_1rb_closure_eligible  = true

Falsification:
  any feature in [F0..F16] except F15 is not Closed           ⇒ Refuted
  F15 is Closed (means F15 was implemented in S8; violates
    D21 named-not-implemented)                                 ⇒ Refuted
  cargo check / clippy / test fails                            ⇒ Refuted
  s8_epic_closure.v1.bd_1rb_closure_eligible = false           ⇒ Refuted

Verdict:
  Refuted if any falsification fires.
  Confirmed otherwise.

Consequence of Refuted:
  bd-1rb cannot close. The PR that closes bd-218w must be revised
  before merge; bd-1rb closure is gated on H13 Confirmed.
```

Hypothesis composition rules are formalized in §14 (Outcome algebra).

---

# 2. Authority rules

```text
Scope(F-S8) =
  {
    H1, H2, H3, H4, H5, H6, H7, H8, H9, H10, H11, H12, H13,
    gutenberg_manifest v1 -> v2 amendment contract,
    Gutenberg-v2-train KN-5 baseline,
    UpperBankCandidate ModelSizeProfile reference instance,
    UpperBankCandidate matched-deployed-bytes dense baseline,
    ExpertShapePolicy enum public surface (T9.1),
    StructuredWidthGates supernet training contract (T9.2),
    Hardening / pruning export contract (T9.3),
    lambda_shape / lambda_overflow gating contract (T5.5),
    full regression test script `gbf s8 regress` (T10.15),
    F15 follow-up bead creation contract,
    s8_*.v1 schema family,
    S8 reproducibility laws,
    S8 falsification suite
  }

Rule Authority:
  ∀ behavior b ∈ Scope(F-S8) ∧ this RFC specifies b
  ⇒ SourceOfTruth(b) = this RFC.

Rule InheritanceFromS1S7:
  Behavior outside Scope(F-S8) is governed by:
    - S1 (F-S1-first-pulse.md): bpc primitive, AdamW pinning,
      Pcg64Mcg + uniform_u64_inclusive, S1CpuDeterministic device
      profile, Toy0 ModelSizeProfile reference instance, TinyStories
      manifest, falsification-suite discipline, pre-registration CI,
      DomainHash + canonical JSON.
    - S2 (F-S2-qat-survives.md): ternary QAT contract (per-row Q8.8
      scales, AnnealedGlobalThenPerOutputRow threshold plan, hard
      ternary projection at Phase C entry, activation fake quant at
      Phase D entry, ternary gap budget bpc(ternary) - bpc(fp) <= 0.5).
    - S3 (F-S3-v0-success-tinystories.md): charset_v1 normalization,
      v0_success WorkloadManifest, ReferenceModelBundle, three-way
      oracle agreement (training ≈ ArtifactOracle ≈ DenotationalOracle),
      KN-5 baseline math (consumed unchanged on Gutenberg-v2 train).
    - S4 (F-S4-gutenberg-promotion.md): `gutenberg_manifest.v1` (S4
      closure record): book selection, per-book source-format
      selection, header/footer stripping, charset_v1 per-document drop
      policy, dedup policy, per-book split rule, split_seed_u128,
      contamination contract (D6), KN-5 baseline math at
      corpus-promotion scale. S8 amends this manifest to
      `gutenberg_manifest.v2` per S8 D1 / D2; everything else
      inherited from S4 stays the same.
    - S5 "Pick and Fit" (F-S5-pick-and-fit.md): BoundedKv K cap = 128
      (matches chunk_size = 128), LinearState DecayPolicy variants,
      shadow_compile_sample API surface, frontier emission discipline,
      per-variant determinism, RuntimeChromeBudget end-to-end,
      CompileProfile + WRAM Layout, BringUp profile, full shadow
      compile pipeline, EncodedRom emission, emulator one-token harness,
      runtime_nucleus_hash CI drift gate.
    - S7 (F-S7-moe-beats-dense.md): MoeTiny matched-deployed-bytes
      parity gate formula, router switch-awareness (low-rank router,
      smoothness reg, expert dropout, switch stats export),
      L_switch differentiable temporal switch penalty (T5.1),
      router collapse guardrail.

  S8 inherits ALL of the above unchanged, except for the explicit S4
  amendment (`gutenberg_manifest.v1` -> `gutenberg_manifest.v2`)
  pinned in S8 D1 / D2. S8 is the FIRST slice that exercises every
  contract simultaneously at production scale on the full Gutenberg
  corpus with a held-out test partition.

Rule CrateOwnership:
  Every behavior in Scope(F-S8) is implemented in exactly one of:
    - gbf-experiments       (NEW S8 modules under gbf_experiments::s8::*;
                              re-uses the gbf-experiments crate per
                              S1 §15.5)
    - gbf-policy            (UpperBankCandidate ModelSizeProfile entry;
                              CompileProfile UpperBankCandidate-BringUp
                              entry)
    - gbf-model             (ExpertShapePolicy enum (T9.1);
                              StructuredWidthGates supernet (T9.2);
                              hardening / pruning export (T9.3))
    - gbf-train             (lambda_shape / lambda_overflow gating
                              (T5.5); supernet training schedule;
                              tau(step) ramp; loss config validation;
                              named contribution helper
                              `inert_shape_overflow_contribution`)
    - gbf-data              (gutenberg_manifest.v2 amendment reader;
                              charset_v1 normalization on the
                              Gutenberg book set inherited from S4;
                              v2 per-split sha256 validator;
                              v2 test-partition derivation)
    - gbf-artifact          (TernaryWeightPlan::compute_byte_cost
                              re-used; hardened expert payload digest;
                              s8_*.v1 schemas)
    - gbf-test              (T10.15 full regression script entrypoint;
                              cross-slice harness per D22)
    - gbf-cli               (`gbf s8 train`, `gbf s8 val-eval`,
                              `gbf s8 test-eval`, `gbf s8 regress`,
                              `gbf s8 supernet`, `gbf s8 harden`,
                              `gbf s8 frontier`, `gbf s8 epic-closure`)
  No S8-specific code lives outside this set.

Rule Amendment:
  Later activity (post-bd-1rb-closure) that changes any of:
    gutenberg_manifest.v2 split fractions, split-seeds, or per-split
      sha256s
    UpperBankCandidate dim caps
    StructuredWidthGates row_group / col_group / tau(step) schedule
    hardening tiebreak rule
    lambda_shape / lambda_overflow defaults or non-default test values
    Pareto frontier rubric (D20)
    test-split-once-per-pass-version discipline (D17)
    F15 follow-up bead naming
  ⇒ Must explicitly amend this RFC and bump pass_version. Because
    bd-1rb is closed at S8 PR merge, such amendments cannot reopen
    the epic; they must take the form of a new epic or a new RFC.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation
  produces the expected Refuted verdict on the appropriate hypothesis.
  Falsification sensitivity is a first-class proof obligation
  (§18 O5).

Rule TestSplitOncePerPassVersion:
  The gutenberg_manifest.v2 test split (the books reassigned from
  v1 train into v2 test per D2; identified by
  gutenberg_manifest.v2.test_sha256) is read EXACTLY ONCE per
  pass_version. Re-reading aborts. Re-running with new training
  hyperparameters bumps pass_version per Rep-S8-1 before the test
  split may be read again. This rule is enforced at CLI level (D17)
  and at filesystem level (the
  experiments/S8/test_eval_pass_versions.jsonl write-once log).

Rule EpicClosure:
  bd-1rb closure is reachable iff §17 Decision protocol returns
  EpicClosurePass. EpicClosurePass requires every mandatory hypothesis
  Confirmed AND s8_epic_closure.v1 emitted with bd_1rb_closure_eligible
  = true.
```

---

# 3. Core notation

```text
Inherited from S1 §1 unchanged:
  Hash256, Seed, TrainStep, EvalStep, LossNatsPerByte, BpcValue,
  GradNorm, Verdict, HypothesisStatus, FailureKind, PredictedRange,
  ObservedStatistic, CharVocab256 (S1; S8 uses Tier 2 charset_v1),
  NGramOrder, SmoothingScheme (D4 KN-5 inherited from S3/S4),
  CorpusManifestRef, DomainHash, S1CanonicalJson (renamed
  S8CanonicalJson but byte-identical; see Rep-S8 §16).

Renamed for S8:
  S8CanonicalJson := exact same encoder as S1CanonicalJson; renamed
                     so per-slice CanonicalJson hashes are namespaced.
                     The encoder bytes are identical.

S8-new types:

GutenbergManifestV2 :=
  {
    schema:                "gutenberg_manifest.v2",
    ; All v1 GutenbergManifest fields are inherited verbatim from
    ; S4 §5 (catalog_snapshot_url, catalog_snapshot_sha256,
    ; selection_filter_canonical_json, selection_filter_sha256,
    ; book_ids, sources, header_regex_pattern, footer_regex_pattern,
    ; normalization_spec_self_hash, dedup_policy,
    ; split_seed_u128 (preserved across v1->v2),
    ; drop_count_*, unmappable_rate_corpus,
    ; raw_byte_policy, retained_book_count_min). The amended fields
    ; relative to v1 are:
    split_train_fraction:            0.80                       ; was 0.90 in v1
    split_val_fraction:              0.10
    split_test_fraction:             0.10                       ; was 0.00 in v1
    test_split_seed_u128:            String                     ; hex 32 chars (NEW)
    train_path:                      String
    val_path:                        String
    test_path:                       String                     ; NEW
    train_sha256:                    Hash256                    ; NEW v2 value
    val_sha256:                      Hash256                    ; PRESERVED from v1
    test_sha256:                     Hash256                    ; NEW
    train_byte_length:               u64                        ; NEW v2 value
                                                                 ; [ESTIMATE; pinned
                                                                 ; at fixture
                                                                 ; creation once
                                                                 ; v2 train materializes]
    val_byte_length:                 u64                        ; PRESERVED from v1
    test_byte_length:                u64                        ; NEW
                                                                 ; [ESTIMATE; pinned
                                                                 ; at fixture creation]
    train_book_count:                u32                        ; NEW v2 value
    val_book_count:                  u32                        ; PRESERVED from v1
    test_book_count:                 u32                        ; NEW
    v1_ancestor_manifest_self_hash:  Hash256                    ; sha of the
                                                                 ; closed S4
                                                                 ; gutenberg_manifest.v1
                                                                 ; (load-bearing
                                                                 ; provenance)
    charset_v1_sha:                  Hash256                    ; carry-through from S3
    contamination:                   ContaminationSubBlock
    manifest_self_hash:              Hash256
  }

ContaminationSubBlock :=
  {
    schema:                          "gutenberg_v2_contamination.v1",
    shared_13gram_rate_with_tinystories:
                                     Map<(GutenbergV2Split, TinyStoriesSplit), f64>
    contamination_self_hash:         Hash256
  }

GutenbergV2Split := Train | Val | Test

ModeId         := "fixed_ternary2"
                | "structured_width_gates_supernet"
                | "structured_width_gates_hardened"
                | "dense_matched_bytes"

ExpertShapePolicy ::= enum (T9.1; gbf-model)
  Fixed
  StructuredWidthGates { row_group: u16, col_group: u16 }

  Default = Fixed.

UpperBankCandidateProfile :=
  {
    id:           "UpperBankCandidate",
    d_model:      128,
    d_ff:         192,
    n_blocks:     4,
    n_experts:    4,
    n_active:     1,
    shape_policy: ExpertShapePolicy,        ; Fixed for fixed_ternary2 +
                                            ; dense_matched_bytes;
                                            ; StructuredWidthGates {8, 8}
                                            ; for supernet/hardened
    vocab:        256,                       ; CHARSET_V1_VOCAB_TIE_DEFAULT_LIMIT
    tied_io:      true,
  }

WidthSelectorParams :=
  {
    alpha_col:   [[f32; 24]; 4]            ; per expert e in 0..4,
                                            ; per col group g in 0..24
    alpha_row:   [[f32; 16]; 4]            ; per expert e, per row group r in 0..16
    tau_at_step: f32                        ; pinned by D11 schedule
  }

HardeningRule :=
  {
    schema:           "s8_hardening_rule.v1",
    argmax_tiebreak:  "lowest_index_wins",
    row_group:        u16                  ; 8
    col_group:        u16                  ; 8
    rule_self_hash:   Hash256
  }

HardenedExpertPayload :=
  {
    expert_id:                 ExpertId,        ; (LayerId, local_idx)
                                                ; per CLAUDE.md export
                                                ; fact bullet
    pruned_d_model_rows:       u16
    pruned_d_ff_cols:          u16
    chosen_row_groups:         Vec<u16>        ; canonicalized ascending
    chosen_col_groups:         Vec<u16>        ; canonicalized ascending
    canonical_tensor_payload_sha:  Hash256
    byte_cost_bytes:           u32
  }

ContributionRecord :=                        ; per CLAUDE.md "named
                                              ; contribution helper"
  {
    raw_value:      f64,
    weighted_value: f64,
    inert:          Bool,
    inert_reason:   Option<String>,           ; e.g. "ExpertShapePolicy::Fixed"
  }

ValidatedLossConfig :=
  {
    lambda_distill:      f32,
    lambda_balance:      f32,
    lambda_zrouter:      f32,
    lambda_switch:       f32,
    lambda_range:        f32,
    lambda_zero:         f32,
    lambda_shape:        f32,
    lambda_overflow:     f32,
    inert_shape_overflow:Bool,                 ; true under Fixed
                                              ; false under StructuredWidthGates
    config_self_hash:    Hash256,
  }

MatchedBytesPolicy := S7-pinned formula at UpperBankCandidate scale
                     (D14). dense_d_ff_matched is recorded explicitly
                     in s8_matched_bytes_parity.v1.

ParetoFrontierPointS8 :=
  {
    point_id:                ModeId,           ; "fixed_ternary2" or
                                              ; "structured_width_gates_hardened"
    val_bpc_ternary_per_seed:[BpcValue; 5],
    val_bpc_ternary_median:  BpcValue,
    projected_deployed_bytes:u64,
    shadow_compile_ok:       Bool,
    runtime_chrome_budget_ok:Bool,
    oracle_agreement_pass:   Bool,
    emulator_harness_pass:   Bool,
    schedule_cost_estimate:  EstimatedCostDelta
  }

FrontierRecommendationS8 :=
    M6Promote
  | M6ResearchOnly
  | M6Reject

S8Outcome (see §14 for full algebra).

EpicClosureCertificate :=
  {
    schema:                       "s8_epic_closure.v1",
    bd_1rb_closure_eligible:      Bool,
    feature_records:              [FeatureClosureRecord; 17],
                                  ; F0..F16 inclusive
    workspace_check_passes:       Bool,
    workspace_clippy_passes:      Bool,
    workspace_test_passes:        Bool,
    pass_version:                 SemVer,
    epic_closure_self_hash:       Hash256
  }

FeatureClosureRecord :=
  {
    feature_id:        String,                   ; "F0".."F16"
    bead_id:           String,                   ; "bd-..."
    status:            "Closed" | "Open"
    closure_pr:        Option<GitCommitId>,
    closure_observed_at:RFC3339String            ; informational only
                                                ; excluded from closure_self_hash
  }

PredictionStatusRule (carry-through from S1):
  Entries under a hypothesis's Predicted block are pre-registered
  expectations. They affect the verdict only when repeated under
  that hypothesis's Falsification block. Otherwise, out-of-range
  observations are reported as Surprises, not automatic Refutations.
```

bpc (carry-through from S1 §7, charset_v1 vocab):

```text
For a model M and validation byte sequence V containing N tokens
(charset_v1 tokens, vocab = 256 with the Tier 2 80-token Accelerando
charset tied):

  Let chunk(i) = floor(i / 128) and start(i) = 128 * chunk(i).
  Let ctx(i) = V[start(i) .. i].

  bpc(M, V) = (1 / N) * Sum_{i=0}^{N-1} -log2(P_M(V[i] | ctx(i)))

P_M is computed by numerically stable log_softmax. State resets at
chunk boundaries. Final-short-chunk rule applies. f64 accumulation;
divide once at the end.

This is the S1 reset-context bpc exactly. The 5-gram KN baseline
from S3 is scored under the same reset-context semantics over the
gutenberg_manifest.v2 val and test splits. The matched-bytes dense
baseline is scored under the same semantics.
```

---

# 4. Authority delta from S1..S7

This section enumerates every contract S8 inherits and explicitly states
"unchanged" or "carry-through with re-validation at UpperBankCandidate
scale." Reviewers (P5, P6) MUST verify there is no silent amendment.

```text
S8-Inheritance-Map :=

  S1 -> S8:
    bpc primitive (S1 §7)                          : unchanged
    AdamW pinning (S1 §D10)                        : unchanged (D25)
    Pcg64Mcg + uniform_u64_inclusive (S1 §5)       : unchanged
    S1CpuDeterministic device profile (S1 §5)      : unchanged
    DomainHash + canonical JSON (S1 §1)            : unchanged
                                                     (S8CanonicalJson is
                                                     a renaming, byte-
                                                     identical)
    Toy0 ModelSizeProfile (T14.1 / bd-1r6k)        : unchanged
    falsification-suite discipline (S1 §13 O5)     : carry-through
                                                     (>= 10 substitutes
                                                     for S8; see §18)
    pre-registration CI (S1 §13 O1)                : carry-through
                                                     (S8-specific script
                                                     scripts/s8_preregistration_check.sh)

  S2 -> S8:
    ternary QAT contract                           : unchanged
    per-row Q8.8 scales (default)                  : unchanged
    AnnealedGlobalThenPerOutputRow threshold plan  : unchanged
    hard ternary projection at Phase C entry       : unchanged
    activation fake quant at Phase D entry         : unchanged
    ternary gap budget bpc(ternary)-bpc(fp)<= 0.5  : unchanged
                                                     (re-validated under
                                                     H2 on Gutenberg-v2-val)

  S3 -> S8:
    charset_v1 token table (Tier 2 80-token)       : unchanged
                                                     (consumed on Gutenberg
                                                     under D3 carry-through
                                                     from S4)
    v0_success WorkloadManifest (eight sub-criteria): unchanged
                                                     (re-validated per-mode
                                                     per-seed under H2 on
                                                     gutenberg_v2_val)
    ReferenceModelBundle export                    : unchanged
                                                     (carry-through to
                                                     hardened
                                                     UpperBankCandidate
                                                     artifact)
    three-way oracle agreement                     : unchanged
                                                     (re-validated under
                                                     H8 on hardened
                                                     UpperBankCandidate
                                                     artifact)
    KN-5 baseline math                             : unchanged
                                                     (rebuilt over
                                                     gutenberg_manifest.v2
                                                     train under D4)

  S4 -> S8:
    gutenberg_manifest.v1 (S4 closure record)      : AMENDED to
                                                     gutenberg_manifest.v2
                                                     per S8 D1/D2 (test
                                                     partition added;
                                                     val byte-identical).
                                                     Book identity,
                                                     stripping, dedup,
                                                     and per-book split
                                                     rule unchanged.
    contamination contract (S4 §7 / D6)            : MECHANISM unchanged;
                                                     EXTENDED with new
                                                     closure-gated
                                                     directions covering
                                                     gutenberg_manifest.v2
                                                     test (S8 D5).
    KN-5 baseline scoring under reset-context      : unchanged
                                                     (baseline REBUILT
                                                     over the new v2
                                                     train per S8 D4)

  S5 -> S8:
    BoundedKv K cap = 128                          : unchanged
                                                     (S8 does not amend;
                                                     S8 selects neither
                                                     BoundedKv nor LinearState
                                                     for headline runs.
                                                     See A20 in §22.)
    LinearState DecayPolicy variants               : unchanged
                                                     (S8 inherits S5
                                                     (Pick and Fit)'s
                                                     selected variant
                                                     from the frontier
                                                     recommendation; see
                                                     §22 A20.)
    shadow_compile_sample API surface              : unchanged
                                                     (carry-through;
                                                     re-exercised at
                                                     UpperBankCandidate)
    frontier emission discipline                   : unchanged
                                                     (s8_pareto_frontier.v1
                                                     consumes the F8
                                                     emitter)
    per-variant determinism                        : unchanged

  S5 (Pick and Fit, deployment carry-through) -> S8:
    RuntimeChromeBudget end-to-end                 : unchanged
                                                     (re-validated under H4
                                                     at UpperBankCandidate's
                                                     BringUp profile)
    CompileProfile + WRAM Layout                   : unchanged
                                                     (UpperBankCandidate-
                                                     BringUp registered as
                                                     a new entry in the
                                                     CompileProfile registry;
                                                     reuses the BringUp
                                                     defaults from S5
                                                     (Pick and Fit))
    full shadow compile pipeline                   : unchanged
                                                     (re-exercised on
                                                     hardened
                                                     UpperBankCandidate
                                                     artifact)
    EncodedRom emission                            : unchanged
                                                     (re-built from
                                                     hardened
                                                     UpperBankCandidate
                                                     artifact)
    emulator one-token harness                     : unchanged
                                                     (re-validated under H9)
    runtime_nucleus_hash CI drift gate             : unchanged
                                                     (carry-through;
                                                     UpperBankCandidate-
                                                     BringUp must not drift
                                                     against pinned shell)

  S7 -> S8:
    matched-deployed-bytes parity formula          : unchanged
                                                     (re-instantiated at
                                                     UpperBankCandidate
                                                     under D14)
    router switch-awareness                        : unchanged
                                                     (low-rank router,
                                                     smoothness reg, expert
                                                     dropout all carried
                                                     through)
    L_switch differentiable temporal switch penalty: unchanged
                                                     (T5.1 carry-through;
                                                     S8 does not amend)
    router collapse guardrail                      : unchanged
                                                     (T10.6c re-runs
                                                     under H11 regression
                                                     script)
    F8 Pareto frontier emitter                     : unchanged
                                                     (s8_pareto_frontier.v1
                                                     consumes it; D20)

  Loss composition (F5 closed except T5.5):
    lambda_distill, lambda_balance, lambda_zrouter,
    lambda_switch, lambda_range, lambda_zero       : unchanged
                                                     (S5/S7-pinned
                                                     defaults; re-used
                                                     at UpperBankCandidate)
    lambda_shape, lambda_overflow                  : NEW gating contract
                                                     (D12); inert under
                                                     Fixed via named
                                                     contribution helper;
                                                     active under
                                                     StructuredWidthGates
```

S8 explicitly amends or extends the following surfaces:

```text
S8-Amendments-To-Closed-Contracts:

  F14 ModelSizeProfile registry (gbf-policy):
    AMENDMENT: add UpperBankCandidate entry per D6. F14 itself remains
    closed; this is a registry expansion, not a contract amendment to
    F14 itself. The registry's "open for extension" property is the
    closed contract; new entries are non-amending.

  F11 CompileProfile registry (gbf-policy):
    AMENDMENT: add UpperBankCandidate-BringUp entry by reusing the
    closed BringUp defaults. Same "open for extension" property.

  F5 Honest Loss Function:
    CLOSED EXCEPT T5.5: T5.5 (bd-3i5) closes as part of S8 per D12.
    The lambda_shape / lambda_overflow gating contract is a new
    public surface inside F5; it does not amend the closed
    lambda_distill / lambda_balance / lambda_zrouter / lambda_switch /
    lambda_range / lambda_zero terms.

  F9 Adaptive Expert Shapes:
    CLOSES AT S8: T9.1 (enum), T9.2 (supernet), T9.3 (hardening) all
    close at S8. ExpertShapePolicy gains StructuredWidthGates as a
    real implemented variant.

  F10 Training-Contract Test Suite and Diagnostic Logging:
    CLOSES AT S8 via T10.15 (full regression script). Other F10 tasks
    closed at S5 (Pick and Fit)/S7.

  F15 Post-closure follow-ups:
    CREATED-NOT-CLOSED at S8: bd-stu4 was adopted under bd-1rb at
    2026-05-06; S8 wires three named children (D21) and removes their
    blocking edges to bd-218w.

  F16 Multi-Corpus Training Data Preparation:
    CLOSED AT S4. T16.1 (TinyStories) closed at S1/S3 and T16.2
    (Gutenberg) closed at S4 satisfied F16's full scope. S8 does
    NOT re-close F16; it consumes F16's closed contract and amends
    the S4-closed gutenberg_manifest.v1 to v2 (S8 D1/D2) via the
    "open for extension" property of the manifest schema family.

  bd-1rb Training-Contract Revision Pass (PARENT EPIC):
    CLOSES AT S8 PR merge, gated on H13 + s8_epic_closure.v1
    bd_1rb_closure_eligible = true.
```

---

# 5. Experiment state machine

```text
State :=
    Configured(corpus_manifest, model_config_per_mode, train_config,
               loss_config_per_mode)
  | CorpusReady(state, GutenbergManifestV2)
  | ContaminationClean(state, ContaminationSubBlock)
  | ProfileBound(state, UpperBankCandidateProfile)
  | BudgetPreflightPassed(state, RuntimeChromeBudget,
                                 PreflightReport_per_mode)
  | SupernetTrained(state, supernet_run_products[5])
  | HardeningExported(state, hardened_artifact_per_seed[5])
  | DeployableTrained(state, fixed_run_products[5],
                              dense_matched_run_products[5])
  | ScoredOnVal(state, val_bpc[per_mode][per_seed])
  | ScoredOnTest(state, test_bpc[per_mode][per_seed])      ; once per pass_version
  | ParityGateChecked(state, matched_bytes_parity[5])
  | ParetoEvaluated(state, ParetoFrontierPointS8[2],
                            FrontierRecommendationS8)
  | OracleAgreement(state, oracle_agreement_per_seed[5])
  | EncodedRomAndEmulator(state, emulator_results_per_seed[5])
  | RegressionScriptRun(state, s8_regression_summary)
  | F15Named(state, followup_beads_certificate)
  | EpicClosureChecked(state, EpicClosureCertificate)
  | Reported(state, s8_report)
  | Decided(state, decision: EpicClosurePass(FrontierRecommendationS8)
                          | Halt(reason)
                          | Investigate(reason))
```

Transitions:

```text
T0 configure:
  empty -> Configured(c, model_config_per_mode, train_config,
                       loss_config_per_mode)

T1 corpus-ready:
  Configured(c, ...) -> CorpusReady(state, load_gutenberg_manifest_v2(c))
  Aborts on D24 if v1_ancestor_manifest_self_hash mismatch, any per-
  split sha256 mismatch, val_sha256 not byte-identical to v1, train+test
  partition not equal to v1 train, or per-split unmappable_rate exceeds
  S4 D5 bounds.

T2 contamination-clean:
  CorpusReady(state, m) -> ContaminationClean(state, contamination_check(m,
                            tinystories_manifest))
  Aborts on D24 if any shared_13gram_rate exceeds D5 bound on any
  closure-gated direction (including the new gutenberg_v2_test
  directions added in S8 D5).

T3 profile-bound:
  ContaminationClean(state, _) -> ProfileBound(state,
                                  UPPER_BANK_CANDIDATE_PROFILE)

T4 budget-preflight:
  ProfileBound(state, profile) -> BudgetPreflightPassed(state, budget,
                                   preflight_report_per_mode)
  Aborts on H4 Refuted (preflight_ok = false for either active mode).

T5 supernet-train:
  BudgetPreflightPassed(...) ->
    SupernetTrained(state,
                    [supernet_run(profile, s, loss_config) for s in {0..4}])
  Aborts per D24 on divergence; partial-pass not legal.

T6 harden-export:
  SupernetTrained(state, supernet_runs) ->
    HardeningExported(state,
                      [harden(supernet_runs[s].final_checkpoint, hardening_rule)
                       for s in {0..4}])
  Aborts on D24 if any hardened expert payload exceeds 16128 bytes
  (16384 - 256 reserved).

T7 deployable-train:
  HardeningExported(state, hardened) ->
    DeployableTrained(state,
                      [fixed_run(profile, s) for s in {0..4}],
                      [dense_matched_run(profile, s) for s in {0..4}])
  Aborts per D24 on divergence.
  Note: structured_width_gates_hardened mode does NOT re-train;
  it is the hardened artifact emitted at T6, scored as a deployable
  ternary model.

T8 score-val:
  DeployableTrained(state, ...) ->
    ScoredOnVal(state, score_per_mode_per_seed_on_gutenberg_v2_val(...))

T9 parity-check:
  ScoredOnVal(state, val_bpc) -> ParityGateChecked(state,
                                   matched_bytes_parity_per_seed)
  Computes delta(s) = bpc(dense_matched, s) - bpc(MoE_fixed, s)
  per D14/D15.

T10 pareto-evaluate:
  ParityGateChecked(...) -> ParetoEvaluated(state,
                              [P_fixed, P_hard],
                              compute_recommendation(P_fixed, P_hard))
  Per D20.

T11 oracle-agree:
  ParetoEvaluated(...) -> OracleAgreement(state,
                            three_way_oracle_agreement_per_seed)
  Per D18, on hardened UpperBankCandidate artifact AND on fixed
  UpperBankCandidate artifact.

T12 encoded-rom-emulator:
  OracleAgreement(...) -> EncodedRomAndEmulator(state,
                           emulator_one_token_harness_per_seed)
  Per D19, on hardened UpperBankCandidate artifact AND on fixed
  UpperBankCandidate artifact.

T13 regression-script:
  EncodedRomAndEmulator(...) -> RegressionScriptRun(state,
                                 gbf_s8_regress(pass_version))
  Per D22.

T14 score-test (ONCE per pass_version, gated by D17):
  RegressionScriptRun(...) -> ScoredOnTest(state,
                                test_bpc_per_mode_per_seed)
  Aborts via D17 write-once log if pass_version already used.

T15 f15-named:
  ScoredOnTest(...) -> F15Named(state, f15_followup_beads_certificate)
  Per D21. Verifies bd-38om, bd-nyen, bd-2pg2 exist with correct
  parent + closure conditions text + no `blocks` edges to bd-218w.

T16 epic-closure-check:
  F15Named(...) -> EpicClosureChecked(state, EpicClosureCertificate)
  Verifies every F0..F16 (except F15) is Closed; F15 is Open with
  three children; cargo check / clippy / test --workspace --all-features
  pass.

T17 report:
  EpicClosureChecked(...) -> Reported(state, s8_report)

T18 decide:
  Reported(state, r) -> Decided(state, decide(r))
```

Invariants:

```text
I-S8-1
  T1 must abort on D24 BEFORE T2 runs.
  Implementation: load_gutenberg_manifest_v2 verifies all per-split
  sha256s, the v1 ancestor self-hash, the byte-identical val
  invariant (val_sha256_v2 = val_sha256_v1, val book_ids unchanged),
  and the train+test partition closure (train_v2 ∪ test_v2 = train_v1,
  disjoint) before returning a CorpusReady value.

I-S8-2
  T4 must run BEFORE any T5/T7 training. Preflight is a build-time
  check, not a runtime check.

I-S8-3
  T6 (hardening) must read ONLY the supernet checkpoint payload sha;
  it must NOT inspect training loss or any optimizer state. Determinism
  requires a pure function of (supernet_checkpoint_sha, hardening_rule_sha).

I-S8-4
  T8 must score on gutenberg_manifest.v2 val (the byte-identical-to-v1
  val partition; D2) for every mode AND every seed. Cross-mode mixing
  is forbidden.

I-S8-5
  T9 parity must use the SAME (mode_fixed, seed s) and
  (mode_dense_matched, seed s) pairing for every s. Cross-seed comparisons
  invalidate the matched-bytes argument.

I-S8-6
  T10 must include exactly two ParetoFrontierPointS8 entries: P_fixed
  and P_hard. Adding a third would contaminate the recommendation
  rule; omitting one means the frontier is incomplete (Refutes H10).

I-S8-7
  T11 must use the S3-pinned tiny-fixture suite for oracle agreement
  testing; using Gutenberg-derived fixtures would be an oracle-suite
  scope creep and Refutes H8 by construction (the S3 oracle suite is
  pinned; changing fixtures changes the contract).

I-S8-8
  T12 (emulator harness) must use the S5 (Pick and Fit) pinned emulator binary +
  ROM build pipeline. Using a newer emulator constitutes a contract
  amendment to S5 (Pick and Fit) and is forbidden in S8.

I-S8-9
  T13 (regression script) must run BEFORE T14 (test eval). The
  regression script is the gate that proves every prior slice still
  passes; running test eval first would risk consuming the test split
  on a broken pass_version.

I-S8-10
  T14 must write to experiments/S8/test_eval_pass_versions.jsonl
  exactly once per pass_version. Re-invocation aborts.

I-S8-11
  T15 must NOT close bd-38om, bd-nyen, or bd-2pg2. Closing them
  would mean implementing F15, which violates D21.

I-S8-12
  T16 must verify every F0..F16 except F15 is Closed. T16 emits
  EpicClosureCertificate with bd_1rb_closure_eligible = true if and
  only if all checks pass.

I-S8-13
  T17 emits exactly one s8_report.v1 per S8 PR. Re-runs after RFC
  amendment produce a new report with bumped rfc_revision and
  pass_version.

I-S8-14
  Decided is final: closure of bd-218w is gated on
  Decision = EpicClosurePass(FrontierRecommendationS8 in
  {M6Promote, M6ResearchOnly, M6Reject}). Closure of bd-1rb is gated
  on the same Decision AND s8_epic_closure.v1.bd_1rb_closure_eligible
  = true.
```

---

# 6. gutenberg_manifest.v2 amendment + contamination contract

## 6.1 Corpus loading and validation

```text
operation s8_load_gutenberg_manifest_v2
  input:  manifest_path: Path
  output: GutenbergManifestV2

Preconditions:
  E-Pre-1   manifest_path resolves to fixtures/corpora/gutenberg.toml
  E-Pre-2   the file exists and parses as TOML
  E-Pre-3   gbf-data crate version matches the F16 (S4-closed) version
            recorded in the manifest's v1 ancestor block
  E-Pre-4   v1_ancestor_manifest_self_hash matches the closed S4
            gutenberg_manifest.v1 sha
  E-Pre-5   split_seed_u128 (v2) byte-equals split_seed_u128 (v1)
  E-Pre-6   test_split_seed_u128 (v2) is well-formed hex 32 chars and
            matches the D1-pinned digest derivation

Postconditions:
  E-Ok-1    every retained book id in v1 is retained in v2 (no
            silent re-fetch)
  E-Ok-2    re-applying the S4 D2 split rule with v1 split_seed_u128
            reproduces the v1 val_book_ids exactly
  E-Ok-3    re-applying the S8 D2 test_membership_function with
            test_split_seed_u128 over v1-train books produces test_v2
            and remaining-train_v2 deterministically
  E-Ok-4    val_sha256_v2 = val_sha256_v1 (preserved byte-identical)
  E-Ok-5    train_v2 byte stream sha256 matches manifest train_sha256
  E-Ok-6    test_v2 byte stream sha256 matches manifest test_sha256
  E-Ok-7    train_v2.book_ids ∩ test_v2.book_ids = empty AND
            train_v2.book_ids ∪ test_v2.book_ids = train_v1.book_ids
  E-Ok-8    after charset_v1 normalization, unmappable_rate per
            split is <= S4 D5 bound (D3 carry-through)
  E-Ok-9    contamination check vs TinyStories returns
            shared_13gram_rate satisfying D5 on every closure-gated
            direction (including the S8-added v2-test directions)
  E-Ok-10   manifest_self_hash round-trips under S8CanonicalJson

Failure modes (per D24, all abort with non-zero exit before any
training begins):
  E-Fail-1  any per-split sha256 mismatch (train, val, or test)
  E-Fail-2  val_sha256_v2 != val_sha256_v1 (val partition drift)
  E-Fail-3  train_v2 ∪ test_v2 != train_v1 (partition not closed)
  E-Fail-4  unmappable_rate exceeds S4 D5 bound for any split
  E-Fail-5  contamination shared_13gram_rate exceeds D5 bound on any
            closure-gated direction
  E-Fail-6  manifest_self_hash mismatch under round-trip
  E-Fail-7  v1_ancestor_manifest_self_hash mismatch (v2 not derivable
            from the recorded S4 ancestor)
```

## 6.2 KN-5 baseline operation

```text
operation s8_fit_kn5_gutenberg_v2
  input:   { gutenberg_v2_train_bytes: ByteSeq,    ; charset_v1-mapped tokens
              charset_v1_sha:      Hash256,
              kn5_smoothing:       SmoothingScheme }   ; from S3
  output:  S8BaselineKn5

S8BaselineKn5 :=
  {
    schema:                       "s8_baseline_kn5.v1",
    corpus_train_sha:             Hash256                  ; gutenberg_v2 train
    corpus_val_sha:               Hash256                  ; gutenberg_v2 val
    corpus_test_sha:              Hash256                  ; gutenberg_v2 test
    charset_v1_sha:               Hash256
    bpc_kn5_gutenberg_v2_val:     BpcValue
    bpc_kn5_gutenberg_v2_test:    BpcValue
    counts_blob_sha256:           Hash256
    baseline_self_hash:           Hash256
  }

Preconditions:
  K-Pre-1   inherited from S3 KN-5 contract: smoothing parameters
            match S3 §6 exactly
  K-Pre-2   gutenberg_v2_train_bytes sha256 matches manifest
            train_sha256_v2

Postconditions:
  K-Ok-1    bpc_kn5_gutenberg_v2_val and bpc_kn5_gutenberg_v2_test
            are finite
  K-Ok-2    counts_blob_sha256 is reproducible (same train sha => same
            counts)
  K-Ok-3    baseline_self_hash round-trips

Reported sanity checks (NOT closure-blocking):
  bpc_kn5_gutenberg_v2_val  in [1.20, 1.60]   [ESTIMATE]
  bpc_kn5_gutenberg_v2_test in [1.20, 1.60]   [ESTIMATE]
```

## 6.3 Contamination contract

```text
ContaminationCheckOperation:
  input:   { gutenberg_v2_manifest: GutenbergManifestV2,
              tinystories_manifest:  TinyStoriesManifest } ; S1/S3-closed
  output:  ContaminationSubBlock

  for X in {Train, Val, Test}:                  ; gutenberg_v2 splits
    for Y_t in {Train, Val}:                    ; TinyStories splits
      shared_13gram_rate[(X, Y_t)] :=
        |{13-grams that appear in both gutenberg_v2.X and tinystories.Y_t}|
        / |{13-grams in gutenberg_v2.X}|

  Closure-gated directions (per S4 D6 mechanism, extended in S8 D5):
    TS_train_contains_GB_val      (S4 carry-through; val unchanged)
    GB_train_contains_TS_val      (S4 carry-through; GB train shrank
                                   under v2 but mechanism unchanged)
    TS_train_contains_GB_test     (NEW; S8 D5)
    GB_test_contains_TS_val       (NEW; S8 D5)

  Per D5 bounds (inherited from S4 D6):
    overlap_threshold_hard_fail = 0.0010
    overlap_threshold_warn      = 0.0005
  Aborts via D24 if any closure-gated pair exceeds the hard-fail
  threshold.
  contamination_self_hash round-trips under S8CanonicalJson.
```

## 6.4 Test-split discipline

```text
operation s8_test_eval_once_per_pass_version
  input:   { gutenberg_v2_test_bytes: ByteSeq,    ; v2 test partition,
                                                   ; pinned by
                                                   ; test_sha256
              pass_version:           SemVer }
  output:  Map<ModeId, Map<Seed, BpcValue>>

Preconditions:
  TE-Pre-1  experiments/S8/test_eval_pass_versions.jsonl exists
            (created empty if absent at first run)
  TE-Pre-2  (pass_version, gutenberg_manifest.v2.test_sha256) not
            already present in the log
  TE-Pre-3  every (mode, seed) checkpoint exists with valid sha
            (T7 + T6 must have completed)

Postconditions:
  TE-Ok-1   appends one line {pass_version, test_sha256, observed_at:
            RFC3339} to experiments/S8/test_eval_pass_versions.jsonl
            using O_APPEND atomic semantics
  TE-Ok-2   returns test_bpc per (mode, seed)
  TE-Ok-3   recorded in s8_score.v1 as a separate field test_bpc
            distinct from val_bpc

Failure modes:
  TE-Fail-1 (pass_version, test_sha256) already present in log =>
            abort with non-zero exit; no test bytes are loaded into
            memory
  TE-Fail-2 any required checkpoint missing => abort
```

---

# 7. UpperBankCandidate profile + matched-bytes baseline

## 7.1 UpperBankCandidate profile registration

```text
gbf-policy::ModelSizeProfile registry entry (D6):

  pub const UPPER_BANK_CANDIDATE: ModelSizeProfile = ModelSizeProfile {
      id:           "UpperBankCandidate",
      d_model:      128,
      d_ff:         192,
      n_blocks:     4,
      n_experts:    4,
      n_active:     1,
      shape_policy: ExpertShapePolicy::Fixed,    ; default; supernet
                                                  ; mode overrides at
                                                  ; construction time
      vocab:        256,                          ; charset_v1 tied
      tied_io:      true,
  };

  impl ModelSizeProfile {
      pub fn upper_bank_candidate_with_shape_policy(
          shape_policy: ExpertShapePolicy
      ) -> ModelSizeProfile {
          let mut p = UPPER_BANK_CANDIDATE;
          p.shape_policy = shape_policy;
          p
      }
  }

Dimension cap validation (T14.2 carry-through):
  ModelTopologyConfig::from_profile(UPPER_BANK_CANDIDATE) MUST:
    - call TernaryWeightPlan::compute_byte_cost(d_model, d_ff)
      twice (W_up, W_down) and verify sum + scale_bytes + metadata
      <= 16384 - 256 (reserved slack)
    - reject d_model > 128 with a structured error
    - reject d_ff > 192 with a structured error
    - reject n_experts > 4 with a structured error
    - accept n_active in {1, 2}; default 1
```

## 7.2 Per-expert byte-cost computation (computed honestly)

```text
TernaryWeightPlan::compute_byte_cost(rows, cols, encoding) for
encoding = WeightEncoding::Ternary2 with PerOutputRow Q8.8 scales:

  weight_count          = rows * cols
  packed_weight_bytes   = ceil(weight_count / 4)              ; 2 bits per weight
  per_row_scale_bytes   = rows * 2                            ; Q8.8 = 2 bytes per row
  per_tensor_metadata   = 32                                  ; canonical fixed
  total_bytes           = packed_weight_bytes + per_row_scale_bytes
                          + per_tensor_metadata

UpperBankCandidate two-matrix expert (D6 honest computation):
  W_up:   [d_model=128 input cols, d_ff=192 output rows]
    weight_count          = 128 * 192 = 24_576
    packed_weight_bytes   = ceil(24_576 / 4) = 6_144
    per_row_scale_bytes   = 192 * 2 = 384
    per_tensor_metadata   = 32
    W_up_total            = 6_144 + 384 + 32 = 6_560 bytes

  W_down: [d_ff=192 input cols, d_model=128 output rows]
    weight_count          = 192 * 128 = 24_576
    packed_weight_bytes   = 6_144
    per_row_scale_bytes   = 128 * 2 = 256
    per_tensor_metadata   = 32
    W_down_total          = 6_144 + 256 + 32 = 6_432 bytes

  per_expert_payload    = 6_560 + 6_432 = 12_992 bytes
                                       ~= 12.69 KiB

  ExpertBank slot       = 16_384 bytes
  reserved slack        = 256 bytes  (per-bank metadata, slot guard)
  effective per-bank    = 16_128 bytes
  headroom per expert   = 16_128 - 12_992 = 3_136 bytes  (~ 3.06 KiB)

The ~13.0 KiB/expert in planv0 amendment is the informal upper estimate;
the honest computed value is 12,992 bytes. Both fit the 16 KiB
ExpertBank slot with measurable headroom.
```

## 7.3 Matched-deployed-bytes dense baseline at UpperBankCandidate (D14)

```text
Dense baseline target: consume the same per-token deployed bytes as
the active expert payload (n_active = 1 means one expert per token,
12_992 bytes). The dense FFN has W_up [d_model, d_ff_dense] and
W_down [d_ff_dense, d_model] with the same TernaryWeightPlan.

We solve for d_ff_dense such that:
  W_up_dense_total  + W_down_dense_total  = 4 * 12_992 - 256 - 64
                                          = 51_968 - 320
                                          = 51_648 bytes

  (target = sum of 4 expert payloads (the MoE budget over 4 experts),
   minus reserved slack and per-block metadata for the dense path)

  W_up_dense_total  = ceil(d_model * d_ff_dense / 4) + d_ff_dense * 2
                      + 32
  W_down_dense_total= ceil(d_ff_dense * d_model / 4) + d_model * 2
                      + 32

  sum               = 2 * ceil(128 * d_ff_dense / 4) + d_ff_dense * 2
                      + 128 * 2 + 64
                    = 2 * 32 * d_ff_dense + 2 * d_ff_dense + 320
                    = 66 * d_ff_dense + 320

  Solving 66 * d_ff_dense + 320 = 51_648:
    d_ff_dense = floor((51_648 - 320) / 66) = floor(51_328 / 66)
               = 777

  Per-block dense byte cost at d_ff_dense = 777:
    66 * 777 + 320 = 51_402 bytes
    delta vs target: 51_648 - 51_402 = 246 bytes (within +/- 64 margin
                                                  is desirable; +/- 256 is
                                                  acceptable per S7
                                                  contract)

Pin dense_d_ff_matched = 777                         [ESTIMATE; pin
                                                     the actual integer
                                                     value at fixture
                                                     creation by
                                                     recomputing the
                                                     closed-form back-
                                                     solve. Update if
                                                     metadata sizes
                                                     change between
                                                     S7 close and S8
                                                     PR.]

s8_matched_bytes_parity.v1 records the closed-form integer used.
The S7 +/- 64 byte tolerance still applies; if the back-solve cannot
land within +/- 256 bytes of target, the dense_d_ff_matched
computation is invalid and S8 aborts via D24.

Note: the original RFC body's earlier-pinned guess of "760" is a
[ESTIMATE] that should be replaced with the closed-form 777 integer
computed here at fixture creation time. The ground truth is the
algebra above.
```

## 7.4 Matched-bytes parity gate operation (D15)

```text
operation s8_matched_bytes_parity_gate
  input:   { fixed_ternary2_run_products: [RunProduct; 5],
              dense_matched_run_products:  [RunProduct; 5],
              corpus_val_sha:              Hash256 }   ; gutenberg_v2 val
  output:  S8MatchedBytesParityReport

S8MatchedBytesParityReport :=
  {
    schema:                       "s8_matched_bytes_parity.v1",
    profile:                      "UpperBankCandidate",
    dense_d_ff_matched:           u16,                  ; 777 (D14)
    target_per_token_bytes:       u32,                  ; 51_648 (D14)
    actual_dense_per_block_bytes: u32,                  ; 51_402 (D14)
    moe_per_token_active_bytes:   u32,                  ; 12_992 (D6)
    delta_per_seed:               [BpcValue; 5]
                                  ; delta(s) = bpc(dense_matched, s)
                                  ;          - bpc(MoE_fixed, s)
    parity_pass_per_seed:         [Bool; 5]
                                  ; parity_pass(s) = delta(s) > 0.05
    median_delta:                 BpcValue
    matched_bytes_parity_self_hash: Hash256
  }
```

---

# 8. StructuredWidthGates supernet contract

## 8.1 Supernet model surface

```text
gbf-model::qat::expert::StructuredWidthGatesExpert :=
  {
    base:                  TwoMatrixExpert<TernaryWeights>,
                                                ; max-d_ff (192) supernet
    alpha_col:             [[f32; 24]; 4]      ; per (expert, col_group)
    alpha_row:             [[f32; 16]; 4]      ; per (expert, row_group)
    row_group:             u16                  ; pinned 8 (D10)
    col_group:             u16                  ; pinned 8 (D10)
  }

  Forward pass:
    for each expert e in 0..n_experts:
      mask_col[e, c] := sigmoid(alpha_col[e, c / col_group] * tau)
                       for c in 0..d_ff
      mask_row[e, r] := sigmoid(alpha_row[e, r / row_group] * tau)
                       for r in 0..d_model

      h := base.W_up[e].matmul(input)           ; [d_ff]
      h := h * mask_col[e, :]                    ; col masking
      h := nonlinearity(h)
      out := base.W_down[e].matmul(h)            ; [d_model]
      out := out * mask_row[e, :]                ; row masking
      return out

  Trainable parameters:
    base.W_up, base.W_down                      ; ternary weights
    base.scales_W_up, base.scales_W_down         ; Q8.8 per-row scales
    alpha_col, alpha_row                         ; supernet selectors
    base.thresholds                              ; from S2 ternary contract

  Stop-gradient sets:
    Phase A: alpha_col, alpha_row are stop-gradient (selectors do not
             learn during dense teacher warmup)
    Phase B: alpha_col, alpha_row are stop-gradient (router warmup; do
             not perturb supernet structure yet)
    Phase C onwards: alpha_col, alpha_row are gradient-on; ternary
             projection is hard
    Phase D onwards: alpha_col, alpha_row are gradient-on; tau ramp is
             10..100
    Phase E: alpha_col, alpha_row are stop-gradient again at the
             hardening step (T6 transition); no further alpha update
             is permitted between hardening and export.
```

## 8.2 Tau temperature schedule (D11 reproduced)

```text
fn tau(step: u32) -> f32 {
    if step <= 18000 {       // end of Phase C
        1.0 + (step as f32 / 18000.0) * 9.0     // 1.0 .. 10.0 linear
    } else if step <= 27000 { // end of Phase D
        10.0 + ((step - 18000) as f32 / 9000.0) * 90.0  // 10.0 .. 100.0
    } else {                  // Phase E
        100.0
    }
}
```

CI obligations:
  - `model::supernet::tau::endpoints_match_pinned_values`
    asserts tau(0) = 1.0, tau(18000) = 10.0, tau(27000) = 100.0,
    tau(30000) = 100.0.
  - `model::supernet::tau::ramp_monotone_nondecreasing`
    asserts tau(s) <= tau(s+1) for s in 1..29999.

## 8.3 Lambda penalties for supernet (D11 reproduced)

```text
shape_penalty(alpha_col, alpha_row) :=
  // Encourages per-expert selector distributions to be near-one-hot.
  // Specifically: penalize entropy of softmax over each expert's
  // selectors. Lower entropy <=> more concentrated <=> more decided
  // selector.
  shape_loss := 0.0
  for e in 0..n_experts:
    p_col[e] := softmax(alpha_col[e] * tau(step))
    p_row[e] := softmax(alpha_row[e] * tau(step))
    shape_loss += entropy(p_col[e]) + entropy(p_row[e])
  return shape_loss / n_experts

overflow_penalty(alpha_col, alpha_row, profile) :=
  // Penalizes the projected hardened expert byte cost exceeding
  // 16128 - delta_safety bytes. Differentiable via the soft mask
  // sums (sum of sigmoid masks approximates the count of kept rows
  // and cols).
  overflow_loss := 0.0
  for e in 0..n_experts:
    soft_kept_rows := sum_r sigmoid(alpha_row[e, r / row_group] * tau(step))
    soft_kept_cols := sum_c sigmoid(alpha_col[e, c / col_group] * tau(step))
    soft_byte_cost := 2 * ceil(soft_kept_rows * soft_kept_cols / 4)
                      + (soft_kept_rows + soft_kept_cols) * 2
                      + 64                                   ; metadata
    overflow_loss += relu(soft_byte_cost - 16_128.0) ^ 2
  return overflow_loss / n_experts

Default lambdas (D11):
  lambda_shape    = 0.05
  lambda_overflow = 0.20

Non-default for tests (CLAUDE.md "non-default/non-1.0 value" bullet):
  lambda_shape    = 0.10
  lambda_overflow = 0.40

Per CLAUDE.md training-loss bullets:
  - shape_penalty and overflow_penalty are differentiable w.r.t.
    alpha_col, alpha_row (gradients flow through softmax / sigmoid).
  - the loss helpers must validate finite/non-negative raw diagnostics
    even when lambda is zero (tested under
    `loss::shape_overflow::raw_helpers_validate_finite_under_zero_lambda`).
  - the contribution helper (D12) explicitly distinguishes raw-weighted
    helpers from inert contribution helpers.
```

## 8.4 Supernet run product

```text
SupernetRunProduct :=
  {
    seed:                            Seed,
    final_supernet_checkpoint:       SafeTensors blob
    final_supernet_checkpoint_sha:   Hash256
    pre_hardening_checkpoint_sha:    Hash256                  ; at step 27001
    metadata:                        SupernetCheckpointMetadata
    run_log:                         SupernetRunLog
    selector_evolution:              SelectorEvolutionLog
                                     ; alpha_col[e, g] and
                                     ; alpha_row[e, r] sampled at
                                     ; eval_every_steps boundaries
    per_expert_selector_state_at_27000:
                                     [(ExpertId, [f32; 24], [f32; 16]); 4]
    completion:                      Completed
                                     | DivergedAt(TrainStep)
                                     | ConvergenceFailureAt(step, e)
                                     ; per H5 falsification: any expert
                                     ; whose max selector at step 27000
                                     ; is <= 0.90 fails completion
  }
```

---

# 9. Hardening / pruning export contract

## 9.1 Hardening operation

```text
operation s8_harden_export
  input:   { supernet_run_product: SupernetRunProduct,
              hardening_rule:       HardeningRule }
  output:  HardenedArtifact

HardenedArtifact :=
  {
    schema:                            "s8_hardened_export.v1",
    seed:                              Seed,
    source_supernet_checkpoint_sha:    Hash256
    hardening_rule_sha:                Hash256
    per_expert_payloads:               [HardenedExpertPayload; 4]
    canonical_artifact_payload_sha:    Hash256
    static_budget_report:              StaticBudgetReport
                                       ; from gbf-codegen at hardened
                                       ; dimensions
    expert_payload_digest:             [ExpertPayloadDigest; 4]
                                       ; carry-through from S7 export
                                       ; facts; ExpertId and (LayerId,
                                       ; local_idx) per CLAUDE.md
                                       ; export-fact bullet
    hardened_export_self_hash:         Hash256
  }

Algorithm:
  for each expert e in 0..4:
    let alpha_col_e = supernet_run_product.per_expert_selector_state_at_27000[e].1
    let alpha_row_e = supernet_run_product.per_expert_selector_state_at_27000[e].2

    chosen_g_for_e :=
      let max_alpha = max(alpha_col_e)
      let candidates = {g : alpha_col_e[g] == max_alpha}
      argmin(candidates)              // tiebreak: lowest index wins (D13)

    chosen_r_for_e := same algorithm over alpha_row_e

    chosen_col_groups[e] := [chosen_g_for_e]   // hardened to single group
    chosen_row_groups[e] := [chosen_r_for_e]

    chosen_cols_for_e := [chosen_g_for_e * 8 + 0,
                          chosen_g_for_e * 8 + 1,
                          ..,
                          chosen_g_for_e * 8 + 7]
                         // 8 cols per col group
    chosen_rows_for_e := same with row_group = 8

    pruned_W_up[e]   := W_up[e][chosen_rows_for_e, :]
    pruned_W_down[e] := W_down[e][:, chosen_rows_for_e]
                        with col-pruning over chosen_cols_for_e
                        applied to the inner d_ff dimension

    Note: a "harder" hardening rule that retains the top-K col groups
    per expert is possible and may produce variable-width experts that
    fit better; the S8-pinned rule retains exactly ONE col group per
    expert (the argmax) for determinism and clarity. A future RFC
    (post-closure) may introduce a top-K hardening rule.

  recompute TernaryWeightPlan::compute_byte_cost(rows', cols') per
  expert using pruned dimensions.

  if any expert's pruned byte cost > 16_128 bytes:
    abort with D24 Fail-hardening(seed)

  emit HardenedExpertPayload per expert with:
    expert_id              := ExpertId(layer_id, local_idx)
                              (per CLAUDE.md export-fact bullet
                               "Expert-scoped export facts must state
                               whether ExpertId is global or layer-local.
                               If the model uses layer-local expert
                               indexes, include LayerId or an artifact
                               path in the fact.")
    pruned_d_model_rows    := |chosen_rows_for_e|     // = 8 with single-group
    pruned_d_ff_cols       := |chosen_cols_for_e|     // = 8 with single-group
    chosen_row_groups      := canonical ascending [chosen_r_for_e]
    chosen_col_groups      := canonical ascending [chosen_g_for_e]
    canonical_tensor_payload_sha := DomainHash(...) over the pruned
                                    W_up + W_down + scales
    byte_cost_bytes        := from compute_byte_cost

  Emit canonical_artifact_payload_sha covering all four
  HardenedExpertPayload entries plus the shared dense / router /
  embedding tensors (which are NOT subject to hardening).

Determinism obligations (per D13):
  R-S8-Hard-1   same supernet checkpoint sha + same hardening rule
                sha => same canonical_artifact_payload_sha
  R-S8-Hard-2   tiebreak rule "lowest index wins" asserted on a
                hand-crafted fixture with two equal selectors
  R-S8-Hard-3   per-expert pruned byte cost <= 16_128 (16384 - 256
                reserved); else Fail-hardening
```

## 9.2 Hardened artifact ExportVisitor compatibility

```text
The standard ExportVisitor (S3-closed) must accept variable-dimension
expert payloads. Per planv0 amendment item 1 and bd-3nj acceptance
criteria:
  - ExportVisitor::visit_expert(expert_id, W_up_dims, W_down_dims, ...)
    must NOT assume W_up_dims = W_down_dims = (d_model, d_ff_max).
  - ExpertPayloadDigest must record actual dimensions, not max
    dimensions.
  - The downstream compiler's StaticBudgetReport must accept variable
    expert sizes; bank packing logic accommodates per-expert payload
    sizes.

S8 does NOT amend the ExportVisitor public surface. It exercises a
pre-existing capability (variable-dim per-expert support); if that
capability is missing because S3/S5 (Pick and Fit) did not implement it for the
fixed-shape default case, T9.3 / bd-3nj is responsible for adding it,
NOT S8. S8 merely consumes the capability.

In practice (S7 closed at MoeTiny with fixed n_experts = 2 each at
identical dimensions), the variable-dim capability has not been
exercised end-to-end before S8. The CI obligation
`export::variable_dim_experts::accepted_by_visitor`
asserts this capability under a tiny fixture before any
UpperBankCandidate hardening run.
```

## 9.3 Hardened artifact downstream consumption

```text
After hardening:
  - the hardened artifact is the deployable Ternary2 expert per
    planv0 §"Model-side recommendations"
  - the matched-bytes parity gate (D14/D15) is re-run on the hardened
    artifact (not the supernet); the matched-deployed-bytes target
    uses the actual hardened per-token byte cost
  - the three-way oracle agreement (D18) is re-run on the hardened
    artifact
  - the EncodedRom + emulator harness (D19) is re-run on the hardened
    artifact
  - the F8 Pareto frontier (D20) compares the hardened artifact to the
    fixed_ternary2 baseline

The hardened artifact's per-token deployed bytes may be SMALLER than
the fixed_ternary2 artifact (because some col/row groups are pruned).
This is the M6Promote case: smaller bytes AND lower bpc. The
M6ResearchOnly case is parity within margins. The M6Reject case is
larger bytes OR meaningfully worse bpc.
```

---

# 10. lambda_shape / lambda_overflow gating contract (T5.5 / bd-3i5)

## 10.1 Surface obligations (per D12)

```text
gbf-train::loss::config:

  pub fn validate_loss_config(
      config:       &LossConfig,
      shape_policy: &ExpertShapePolicy
  ) -> ValidatedLossConfig {
      match shape_policy {
          ExpertShapePolicy::Fixed => {
              if config.lambda_shape != 0.0 || config.lambda_overflow != 0.0 {
                  emit_warning_event!(
                      target: "loss.config.shape_overflow_inert_under_fixed",
                      lambda_shape:    config.lambda_shape,
                      lambda_overflow: config.lambda_overflow,
                      forced_to:       0.0_f32
                  );
              }
              ValidatedLossConfig {
                  lambda_shape:        0.0,
                  lambda_overflow:     0.0,
                  inert_shape_overflow:true,
                  ..config.copy_other_lambdas()
              }
          }
          ExpertShapePolicy::StructuredWidthGates {..} => {
              assert!(config.lambda_shape.is_finite() &&
                      config.lambda_shape >= 0.0);
              assert!(config.lambda_overflow.is_finite() &&
                      config.lambda_overflow >= 0.0);
              if config.lambda_shape == 0.0 && config.lambda_overflow == 0.0 {
                  emit_warning_event!(
                      target: "loss.config.shape_overflow_zero_under_supernet",
                      message: "structured width gates active but both lambdas zero"
                  );
              }
              ValidatedLossConfig {
                  lambda_shape:        config.lambda_shape,
                  lambda_overflow:     config.lambda_overflow,
                  inert_shape_overflow:false,
                  ..config.copy_other_lambdas()
              }
          }
      }
  }

gbf-train::loss::compose:

  pub fn inert_shape_overflow_contribution(
      validated: &ValidatedLossConfig
  ) -> ContributionRecord {
      // NAMED CONTRIBUTION HELPER per CLAUDE.md bullet
      // "If a helper intentionally skips raw computation for a disabled
      //  config term, name it as a contribution/composer helper rather
      //  than a raw weighted-loss helper."
      ContributionRecord {
          raw_value:      0.0,
          weighted_value: 0.0,
          inert:          true,
          inert_reason:   Some(String::from("ExpertShapePolicy::Fixed")),
      }
  }

  pub fn shape_penalty_raw_weighted<B: Backend>(
      alpha_col: &Tensor<B, 2>,
      alpha_row: &Tensor<B, 2>,
      tau:       f32,
      lambda_shape: f32,
  ) -> ContributionRecord {
      // RAW WEIGHTED LOSS HELPER per CLAUDE.md bullet
      // "Keep raw weighted-loss helpers honest: they must validate
      //  finite/non-negative raw diagnostics even when the configured
      //  weight is zero."
      let raw = compute_shape_loss_raw(alpha_col, alpha_row, tau);
      assert!(raw.is_finite() && raw >= 0.0,
              "shape_penalty_raw must be finite and non-negative; got {}",
              raw);
      let weighted = lambda_shape * raw;
      assert!(weighted.is_finite() && weighted >= 0.0,
              "shape_penalty_weighted must be finite and non-negative");
      ContributionRecord {
          raw_value:      raw as f64,
          weighted_value: weighted as f64,
          inert:          false,
          inert_reason:   None,
      }
  }

  pub fn overflow_penalty_raw_weighted<B: Backend>(
      alpha_col:    &Tensor<B, 2>,
      alpha_row:    &Tensor<B, 2>,
      tau:          f32,
      profile:      &UpperBankCandidateProfile,
      lambda_overflow: f32,
  ) -> ContributionRecord {
      // RAW WEIGHTED LOSS HELPER (same discipline as above)
      let raw = compute_overflow_loss_raw(alpha_col, alpha_row,
                                           tau, profile);
      assert!(raw.is_finite() && raw >= 0.0);
      let weighted = lambda_overflow * raw;
      assert!(weighted.is_finite() && weighted >= 0.0);
      ContributionRecord {
          raw_value:      raw as f64,
          weighted_value: weighted as f64,
          inert:          false,
          inert_reason:   None,
      }
  }

gbf-train::loss::compose::compose_total_loss:

  pub fn compose_total_loss<B: Backend>(
      ...
      validated: &ValidatedLossConfig,
      shape_policy: &ExpertShapePolicy,
      alpha_col: Option<&Tensor<B, 2>>,
      alpha_row: Option<&Tensor<B, 2>>,
      profile:   &UpperBankCandidateProfile,
      step:      u32,
  ) -> (Tensor<B, 0>, ContributionsCollection) {
      // Per CLAUDE.md "Do not give raw per-term diagnostic collections
      //  an implicit all-zero default; enabled lambdas can otherwise
      //  hide missing raw loss computation. If zeros are intentional,
      //  require explicit fields or a named contribution helper."

      let shape_contribution = match shape_policy {
          ExpertShapePolicy::Fixed => {
              inert_shape_overflow_contribution(validated)
          }
          ExpertShapePolicy::StructuredWidthGates { .. } => {
              shape_penalty_raw_weighted(
                  alpha_col.expect("StructuredWidthGates requires alpha_col"),
                  alpha_row.expect("StructuredWidthGates requires alpha_row"),
                  tau(step),
                  validated.lambda_shape,
              )
          }
      };

      let overflow_contribution = match shape_policy {
          ExpertShapePolicy::Fixed => {
              inert_shape_overflow_contribution(validated)
          }
          ExpertShapePolicy::StructuredWidthGates { .. } => {
              overflow_penalty_raw_weighted(
                  alpha_col.expect("..."),
                  alpha_row.expect("..."),
                  tau(step),
                  profile,
                  validated.lambda_overflow,
              )
          }
      };

      // ... compose with the other six lambdas (all carry-through from S5/S7)
      // Total loss is the sum of weighted contributions; the
      // ContributionsCollection record explicitly contains all eight
      // lambda terms with named fields, NOT an implicit map keyed on
      // lambda name.
      ...
  }

ContributionsCollection :=
  {
      lm_contribution:        ContributionRecord,
      distill_contribution:   ContributionRecord,
      balance_contribution:   ContributionRecord,
      zrouter_contribution:   ContributionRecord,
      switch_contribution:    ContributionRecord,
      range_contribution:     ContributionRecord,
      zero_contribution:      ContributionRecord,
      shape_contribution:     ContributionRecord,
      overflow_contribution:  ContributionRecord,
  }
  // EXPLICIT named fields, NOT a HashMap<String, ContributionRecord>
  // per CLAUDE.md bullet on implicit all-zero default.
```

## 10.2 CI test obligations (per D12 #3)

```text
The following test names appear in gbf-train tests under
`loss::shape_overflow::*`:

  fixed_inert_records_zero_with_flag
    Setup: ExpertShapePolicy::Fixed, user lambdas (0.05, 0.20)
    Assert: inert_shape_overflow_contribution returns
            { raw_value: 0.0, weighted_value: 0.0, inert: true,
              inert_reason: Some("ExpertShapePolicy::Fixed") }

  structured_active_nonzero_grad
    Setup: ExpertShapePolicy::StructuredWidthGates {8, 8},
           lambdas (0.10, 0.40) (non-default per CLAUDE.md scalar
           bullet)
    Assert: gradients into alpha_col and alpha_row are finite,
            nonzero, and deterministic over two replays.
    Cite: cargo test -p gbf-train --features burn-adapter --
          loss::shape_overflow::structured_active_nonzero_grad
    (per CLAUDE.md "If a loss claim depends on Burn autodiff,
     closure must cite a feature-enabled gate".)

  sweep_inert_under_fixed
    Setup: ExpertShapePolicy::Fixed
    Loop: for lambda_shape in [0.0, 0.05, 0.10, 0.50, 1.0]:
            for lambda_overflow in [0.0, 0.20, 0.40]:
              compute the loss composer output on a fixed alpha_col,
              alpha_row, profile.
    Assert: the loss composer output is byte-identical across all
            (lambda_shape, lambda_overflow) pairs (because under Fixed,
            all are inert and produce 0.0 contribution regardless).

  raw_helpers_validate_finite_under_zero_lambda
    Setup: ExpertShapePolicy::StructuredWidthGates {8, 8},
           lambda_shape = 0.0, lambda_overflow = 0.0
    Assert: shape_penalty_raw_weighted and overflow_penalty_raw_weighted
            both validate finite/non-negative raw_value AND weighted_value
            even when the lambda is zero.
    Cite: cargo test -p gbf-train -- loss::shape_overflow::raw_helpers_validate_finite_under_zero_lambda

  contribution_collection_has_explicit_fields
    Static structural test: assert ContributionsCollection has nine
    explicit named fields (one per lambda term), and that the type
    is NOT a HashMap or Vec.
    Cite: cargo test -p gbf-train -- loss::shape_overflow::contribution_collection_has_explicit_fields

  non_default_values_test
    Setup: lambda_shape = 0.10, lambda_overflow = 0.40
    Assert: shape_penalty_raw_weighted and overflow_penalty_raw_weighted
            produce values consistent with non-default lambdas. Per
            CLAUDE.md "Tests for scalar hyperparameters such as safe
            bounds, temperatures, and loss weights must include a
            non-default/non-1.0 value."
    Cite: cargo test -p gbf-train --features burn-adapter -- loss::shape_overflow::non_default_values_test
```

---

# 11. Carry-through validation suite

S8 re-runs three previously-retired validation suites at the new
UpperBankCandidate scale + on the hardened artifact. None of the
underlying contracts is amended; the suites are re-instantiated.

## 11.1 S3 three-way oracle agreement (carry-through)

```text
operation s8_oracle_agreement_check
  input:   { hardened_artifact: HardenedArtifact,
              fixed_artifact:    FixedTernary2Artifact,
              s3_tiny_fixture_suite_sha: Hash256 }
  output:  S8OracleAgreementReport per (mode, seed)

S8OracleAgreementReport :=
  {
    schema:                          "s8_oracle_agreement.v1",
    mode:                            ModeId,
    seed:                            Seed,
    artifact_payload_sha:            Hash256
    s3_fixture_suite_sha:            Hash256
    per_fixture_max_abs_diff_artifact_oracle:    Map<FixtureId, f32>
    per_fixture_max_abs_diff_denotational_oracle:Map<FixtureId, f32>
    artifact_oracle_pass:            Bool
    denotational_oracle_pass:        Bool
    three_way_pass:                  Bool       ; both above true
    oracle_agreement_self_hash:      Hash256
  }

Per D18 numeric tolerance: max_abs_diff <= 1e-4 in f32 under
S1CpuDeterministic. Inherited from S3 unchanged.
```

## 11.2 S5 (Pick and Fit) EncodedRom + emulator harness (carry-through)

```text
operation s8_emulator_harness_check
  input:   { hardened_artifact: HardenedArtifact,
              fixed_artifact:    FixedTernary2Artifact,
              v0_success_workload_manifest_sha: Hash256,
              s5_emulator_binary_sha: Hash256,
              s5_compile_profile:    "UpperBankCandidate-BringUp" }
  output:  S8EmulatorHarnessReport per (mode, seed)

S8EmulatorHarnessReport :=
  {
    schema:                          "s8_emulator_harness.v1",
    mode:                            ModeId,
    seed:                            Seed,
    artifact_payload_sha:            Hash256
    encoded_rom_sha:                 Hash256
    emulator_run_log_sha:            Hash256
    per_prompt_first_token:          Vec<EmulatorTokenRecord>
                                     ; one record per v0_success
                                     ; prompt
    at_least_one_prompt_passed:      Bool
                                     ; H9 falsification predicate
    emulator_harness_self_hash:      Hash256
  }

EmulatorTokenRecord :=
  {
    prompt_id:                String,
    training_logits_sha:      Hash256
    emulator_logits_sha:      Hash256
    max_abs_diff:             f32
    within_pinned_tolerance:  Bool       ; per S6_PINNED_TOLERANCE
                                         ; (code constant; owner is
                                         ; now gbf_experiments::s5)
    emulator_no_fault:        Bool
  }

S6_PINNED_TOLERANCE is inherited from F-S5-pick-and-fit.md unchanged.
```

## 11.3 S7 matched-bytes parity gate (re-validated)

Already specified in §7.4. The S7 parity formula is re-instantiated
at UpperBankCandidate scale with dense_d_ff_matched = 777 (D14).

---

# 12. Regression script contract (T10.15 / bd-180)

## 12.1 CLI surface

```text
gbf s8 regress
  --pass-version <SemVer>
  --output       <Path>             ; default: experiments/S8/regression/
  --device-profile S1CpuDeterministic
  --json         <Bool>             ; default: true; emit s8_regression_summary.v1

Behavior:
  1. Load pinned per-slice test manifests from
     fixtures/regression/per-slice/ (one TOML per slice S1..S8)
     listing every CI gate command.
  2. For each slice X in [S1, S2, S3, S4, S5, S7, S8] in the
     pinned dispatch order:
       Run, in sequence:
         (a) cargo test -p gbf-experiments --test <slice>           ; unit
         (b) cargo test -p gbf-experiments --features falsify-<slice>
                                            --test falsification_<slice>
         (c) cargo test -p gbf-experiments --test oracle_<slice>
         (d) cargo test -p gbf-experiments --test canonical_json_<slice>
         (e) cargo test -p gbf-experiments --test integration_<slice>
         (f) scripts/<slice>_preregistration_check.sh
         (g) scripts/<slice>_determinism_check.sh
         (h) scripts/<slice>_isolation_check.sh
       For each command:
         record exit code, runtime in seconds, captured stdout/stderr
         tail (last 8 KiB).
       The slice-block returns Pass iff every command exits 0.
  3. Emit s8_regression_summary.v1 (see §15.10).
  4. Total runtime budget: 300 seconds (D22). If exceeded, the script
     still completes (does not kill commands mid-run) but records the
     budget overshoot in the summary; H11 falsifies on overshoot.
  5. Exit code 0 iff every slice-block returned Pass AND total runtime
     <= 300 seconds AND skipped_count = 0.

The script does NOT consume the gutenberg_manifest.v2 test split
(D17). That is a separate manual `gbf s8 test-eval` invocation.

Composability obligation:
  Each per-slice command list (fixtures/regression/per-slice/<slice>.toml)
  is owned by that slice's RFC. S8 does NOT redefine any prior slice's
  per-slice manifest. The S8 per-slice manifest is owned by this RFC.

CI-runnable form for human reviewers:
  cargo run --release -p gbf-cli -- s8 regress \
    --pass-version $(cat experiments/S8/PASS_VERSION) \
    --device-profile S1CpuDeterministic
```

## 12.2 Per-slice manifest schema

```text
fixtures/regression/per-slice/<slice>.toml :=
  schema = "regression_per_slice.v1"
  slice  = "<slice>"
  commands = [
    { kind = "unit",            cmd = "cargo test -p gbf-experiments --test <slice>" },
    { kind = "falsification",   cmd = "cargo test -p gbf-experiments --features falsify-<slice> --test falsification_<slice>" },
    { kind = "oracle",          cmd = "cargo test -p gbf-experiments --test oracle_<slice>" },
    { kind = "canonical_json",  cmd = "cargo test -p gbf-experiments --test canonical_json_<slice>" },
    { kind = "integration",     cmd = "cargo test -p gbf-experiments --test integration_<slice>" },
    { kind = "preregistration", cmd = "scripts/<slice>_preregistration_check.sh" },
    { kind = "determinism",     cmd = "scripts/<slice>_determinism_check.sh" },
    { kind = "isolation",       cmd = "scripts/<slice>_isolation_check.sh" },
  ]
  expected_total_runtime_seconds_max = <int>     ; per-slice budget
  manifest_self_hash = "sha256:..."
```

## 12.3 Skip semantics

```text
Per H11 falsification: `skipped > 0` Refutes H11.

A skip occurs when:
  - a command's exit code is 77 (cargo test convention for skipped)
  - the test manifest declares an explicit `skip = true` flag

S8 explicitly forbids any skip in any per-slice manifest. The
manifest validator
  `gbf s8 regress --validate-manifest`
asserts no skip flags are present. Re-running the script with skip
flags present aborts before any test is run.

Rationale: the regression script is the bd-1rb closure-readiness
gate. A skip leaves a contract claim unverified; closing the epic
on unverified claims would violate P5 Proof-of-Work Detective.
```

---

# 13. F15 follow-up bead creation contract

## 13.1 Bead-graph obligations (per D21)

```text
S8 PR must perform the following bead-graph operations BEFORE merge,
verified by `gbf s8 epic-closure --check-followup-beads`:

  1. Verify bd-stu4 (F15) is parented under bd-1rb. (Already true at
     2026-05-06 adoption.)

  2. Verify bd-38om, bd-nyen, bd-2pg2 are parented under bd-stu4.
     (Already true at 2026-05-06 adoption.)

  3. Remove any `blocks` edge from {bd-38om, bd-nyen, bd-2pg2} to
     bd-218w. As of 2026-05-09 these edges exist (per `br show
     bd-218w`). They MUST be removed in the same PR that closes
     bd-218w; otherwise the closure is impossible.

  4. Verify each of bd-38om, bd-nyen, bd-2pg2 carries a top-level
     comment with the closure conditions text from D21. The comment
     must be an exact-match for the D21 text (modulo bead ID
     formatting).

  5. Emit s8_followup_beads.v1 recording the bead state observed at
     PR-creation time and at PR-merge time (two snapshots).

s8_followup_beads.v1 :=
  {
    schema:                       "s8_followup_beads.v1",
    snapshot_at_pr_creation:      [BeadSnapshot; 3]
    snapshot_at_pr_merge:         [BeadSnapshot; 3]
    blocks_edges_removed:         [BlockEdgeRemoval]
    closure_conditions_present:   [(BeadId, Bool)]
    followup_beads_self_hash:     Hash256
  }

BeadSnapshot :=
  {
    bead_id:               String,        ; "bd-38om" etc
    parent:                String,        ; "bd-stu4"
    status:                "open" | "closed",
    blocks_edges:          Vec<String>    ; current blockers
    has_closure_conditions:Bool
  }

BlockEdgeRemoval :=
  {
    bead_id:        String,
    removed_blocks: String,                ; "bd-218w"
    removed_at:     RFC3339String
  }
```

## 13.2 Why the edges must be removed

```text
As of 2026-05-09:
  bd-38om, bd-nyen, bd-2pg2 are listed in `br show bd-218w` as
  blockers. If those edges remain, S8 closure (bd-218w close) is
  blocked on F15 implementation, which is exactly the opposite of
  D21 (F15 is named-not-implemented).

  Removing the edges captures the architectural decision that F15 is
  *post-closure* work — extension work that does not block the epic.
  The beads themselves remain Open under bd-stu4, tracked for
  future implementation.

  The PR that closes bd-218w MUST include three `br dep rm` (or
  equivalent) operations:
    br dep rm bd-218w bd-38om
    br dep rm bd-218w bd-nyen
    br dep rm bd-218w bd-2pg2
  followed by `br sync --flush-only` to record the change in
  .beads/issues.jsonl.

  s8_followup_beads.v1 records both snapshots (pre-removal and
  post-removal) as proof-of-work that the edges were removed
  intentionally and in this PR.
```

---

# 14. Outcome algebra

```text
S8Outcome :=
    Pass-clean
      ; H1..H9 ∧ H11..H13 all Confirmed; H10 Confirmed with
      ; FrontierRecommendationS8 = M6Promote
  | Pass-with-research-tail
      ; H1..H9 ∧ H11..H13 all Confirmed; H10 Confirmed with
      ; FrontierRecommendationS8 = M6ResearchOnly
  | Pass-with-research-rejected
      ; H1..H9 ∧ H11..H13 all Confirmed; H10 Confirmed with
      ; FrontierRecommendationS8 = M6Reject
  | Fail-corpus              ; H1 Refuted
  | Fail-quality             ; H2 Refuted (non-suspicious)
  | Fail-suspicious          ; median(val_bpc_ternary) < 0.5
  | Fail-parity              ; H3 Refuted (matched-bytes parity)
  | Fail-budget              ; H4 Refuted (RuntimeChromeBudget)
  | Fail-supernet            ; H5 Refuted (selectors did not converge)
  | Fail-hardening           ; H6 Refuted (non-deterministic export
                             ;             OR overflow)
  | Fail-gating              ; H7 Refuted (lambda gating broken)
  | Fail-oracle              ; H8 Refuted (three-way oracle disagreement)
  | Fail-emulator            ; H9 Refuted (EncodedRom + emulator)
  | Fail-pareto              ; H10 Refuted (frontier emitter broken)
  | Fail-regression          ; H11 Refuted (regression script)
  | Fail-followup-naming     ; H12 Refuted (F15 bead-graph)
  | Fail-epic-closure        ; H13 Refuted (epic checklist)
  | Fail-substrate           ; ∃ mode, seed. completion = DivergedAt(_)
  | Fail-test-discipline     ; D17 violation (test split re-used)
```

Combination (mandatory checks first):

```text
if ∃ mode, seed. completion(mode, seed) = DivergedAt(_)         ⇒ Fail-substrate
elif test_eval_pass_versions_log shows duplicate pass_version   ⇒ Fail-test-discipline
elif H1  verdict = Refuted                                      ⇒ Fail-corpus
elif median over (mode, seed) of val_bpc_ternary < 0.5          ⇒ Fail-suspicious
elif H4  verdict = Refuted                                      ⇒ Fail-budget
elif H6  verdict = Refuted                                      ⇒ Fail-hardening
elif H7  verdict = Refuted                                      ⇒ Fail-gating
elif H8  verdict = Refuted                                      ⇒ Fail-oracle
elif H9  verdict = Refuted                                      ⇒ Fail-emulator
elif H10 verdict = Refuted                                      ⇒ Fail-pareto
elif H11 verdict = Refuted                                      ⇒ Fail-regression
elif H12 verdict = Refuted                                      ⇒ Fail-followup-naming
elif H13 verdict = Refuted                                      ⇒ Fail-epic-closure
elif H2  verdict = Refuted                                      ⇒ Fail-quality
elif H3  verdict = Refuted                                      ⇒ Fail-parity
elif H5  verdict = Refuted                                      ⇒ Fail-supernet
else
  // all mandatory hypotheses Confirmed; H10 always Confirmed (the
  // three branches of D20 are all Confirmed-classes)
  let rec = compute_frontier_recommendation_s8(P_fixed, P_hard)
  match rec:
    M6Promote        ⇒ Pass-clean
    M6ResearchOnly   ⇒ Pass-with-research-tail
    M6Reject         ⇒ Pass-with-research-rejected
```

Decision dispatch:

```text
Pass-clean                       -> Decision::EpicClosurePass(M6Promote)
Pass-with-research-tail          -> Decision::EpicClosurePass(M6ResearchOnly)
Pass-with-research-rejected      -> Decision::EpicClosurePass(M6Reject)
Fail-corpus                      -> Decision::Halt(corpus-broken)
Fail-quality                     -> Decision::Investigate(upperbank-undersized-or-qat-broken-on-gutenberg-v2)
Fail-suspicious                  -> Decision::Halt(audit-charset-and-bpc)
Fail-parity                      -> Decision::Investigate(matched-bytes-parity-at-upperbank)
Fail-budget                      -> Decision::Investigate(runtime-chrome-budget-at-upperbank)
Fail-supernet                    -> Decision::Investigate(structured-width-gates-supernet)
Fail-hardening                   -> Decision::Investigate(hardening-export-deterministic-or-overflow)
Fail-gating                      -> Decision::Halt(t5_5-lambda-gating-broken)
Fail-oracle                      -> Decision::Investigate(s3-oracle-at-upperbank)
Fail-emulator                    -> Decision::Investigate(s6-emulator-at-upperbank)
Fail-pareto                      -> Decision::Investigate(f8-frontier-at-upperbank)
Fail-regression                  -> Decision::Halt(regression-script-broken)
Fail-followup-naming             -> Decision::Investigate(f15-bead-graph)
Fail-epic-closure                -> Decision::Halt(epic-checklist-fail)
Fail-substrate                   -> Decision::Investigate(burn-or-autodiff-at-upperbank)
Fail-test-discipline             -> Decision::Halt(test-split-discipline-violated)
```

`Halt` blocks bd-218w AND bd-1rb closure unconditionally.
`Investigate` creates a follow-up bead and may extend this RFC's
scope or trigger a new epic.

`EpicClosurePass(M6Promote | M6ResearchOnly | M6Reject)` are ALL
legal closure outcomes for both bd-218w and bd-1rb. The M6 frontier
recommendation is metadata about the M6 research mode's payoff; it
is not a closure precondition.

---

# 15. Artifact schemas (s8_*.v1)

## 15.1 s8_corpus_manifest.v1

```text
Path:
  fixtures/corpora/gutenberg.toml         ; canonical TOML source
                                            ; (amended in S8 to record
                                            ; the v2 split block on
                                            ; top of the S4 v1 fields)
  experiments/S8/corpus/gutenberg-manifest-v2.json
                                            ; canonical JSON copy of
                                            ; the v2 manifest emitted
                                            ; at corpus-load time

GutenbergManifestV2 schema as in §3.
```

## 15.2 s8_baseline_kn5.v1

```text
Path:
  experiments/S8/baseline/kn5-gutenberg-v2.bin
  experiments/S8/baseline/kn5-gutenberg-v2-report.json

S8BaselineKn5 schema as in §6.2.
```

## 15.3 s8_run_log.v1

```text
Path:
  experiments/S8/{mode}/seed-{seed}/run-log.json
  experiments/S8/{mode}/seed-{seed}/grad-log.jsonl
  experiments/S8/{mode}/seed-{seed}/weight-stats.jsonl
  experiments/S8/structured_width_gates_supernet/seed-{seed}/selector-evolution.jsonl

S8RunLog (JSON) :=
  {
    schema:                "s8_run_log.v1",
    mode:                  ModeId
    seed:                  Seed
    train_config_hash:     Hash256
    losses:                List[(TrainStep, LossNatsPerByte)]
                           ; length = optimizer_steps = 30000
    contributions:         List[(TrainStep, ContributionsCollection)]
                           ; recorded at eval_every_steps boundaries
    eval_points:           List[(EvalStep, BpcValue)]
                           ; length = 11 (steps 0, 3000, 6000, ..., 30000)
    phase_boundary_steps:  { phase_a_end: 6000, phase_b_end: 9000,
                             phase_c_end: 18000, phase_d_end: 27000,
                             phase_e_end: 30000 }
    final_grad_norms:      GradNormSummary
    run_log_self_hash:     Hash256
  }
```

## 15.4 s8_score.v1

```text
Path:
  experiments/S8/{mode}/seed-{seed}/score-val.json    ; always emitted
  experiments/S8/{mode}/seed-{seed}/score-test.json   ; emitted ONCE
                                                        ; per pass_version

S8Score (JSON) :=
  {
    schema:                "s8_score.v1",
    mode:                  ModeId
    seed:                  Seed
    checkpoint_phase_a_sha: Hash256              ; teacher (fp)
    checkpoint_phase_d_sha: Hash256              ; ternary (deployable)
    corpus_split:          "val" | "test"
    corpus_split_sha:      Hash256
    charset_v1_sha:        Hash256
    chunk_size:            128
    token_count:           u64
    log2_sum_fp:           f64
    log2_sum_ternary:      f64
    val_bpc_fp:            BpcValue              ; teacher
    val_bpc_ternary:       BpcValue              ; deployable
    ternary_gap:           BpcValue              ; val_bpc_ternary - val_bpc_fp
    pass_version:          SemVer                ; required for split = "test"
    score_self_hash:       Hash256
  }

Invariants:
  S-Score-1   For corpus_split = "test", pass_version is non-null
              and matches a single line in
              experiments/S8/test_eval_pass_versions.jsonl.
  S-Score-2   For corpus_split = "val", pass_version may be null.
```

## 15.5 s8_matched_bytes_parity.v1

Schema as in §7.4.

## 15.6 s8_oracle_agreement.v1

Schema as in §11.1.

## 15.7 s8_emulator_harness.v1

Schema as in §11.2.

## 15.8 s8_supernet_run.v1

```text
Path:
  experiments/S8/structured_width_gates_supernet/seed-{seed}/supernet-run.json

S8SupernetRun (JSON) :=
  {
    schema:                          "s8_supernet_run.v1",
    seed:                            Seed
    final_supernet_checkpoint_sha:   Hash256
    pre_hardening_checkpoint_sha:    Hash256
    per_expert_max_alpha_col_at_27000:    [f32; 4]
    per_expert_second_alpha_col_at_27000: [f32; 4]
    per_expert_max_alpha_row_at_27000:    [f32; 4]
    per_expert_second_alpha_row_at_27000: [f32; 4]
    per_expert_chosen_col_group_at_27001: [u16; 4]
    per_expert_chosen_row_group_at_27001: [u16; 4]
    convergence_pass:                Bool         ; H5 falsification
                                                  ; max > 0.90 AND
                                                  ; second_max < 0.50
    selector_evolution_jsonl_sha:    Hash256
    supernet_run_self_hash:          Hash256
  }
```

## 15.9 s8_hardened_export.v1

Schema as in §9.1.

## 15.10 s8_pareto_frontier.v1

```text
Path:
  experiments/S8/frontier/s8-pareto-frontier.json

S8ParetoFrontier (JSON) :=
  {
    schema:                          "s8_pareto_frontier.v1",
    profile:                         "UpperBankCandidate",
    points:                          [ParetoFrontierPointS8; 2]
                                     ; [P_fixed, P_hard]
    recommendation:                  FrontierRecommendationS8
    recommendation_rationale:        String
                                     ; one of three pinned strings:
                                     ; "M6Promote: hardened beats fixed by >= 0.02 bpc..."
                                     ; "M6ResearchOnly: parity within margins..."
                                     ; "M6Reject: hardened worse than fixed..."
    pareto_frontier_self_hash:       Hash256
  }

ParetoFrontierPointS8 as in §3.

Invariant:
  PF-1   points.length = 2
  PF-2   points[0].point_id = "fixed_ternary2"
  PF-3   points[1].point_id = "structured_width_gates_hardened"
  PF-4   recommendation matches the algorithm in D20 applied to the
         two points (deterministic; replay must produce the same
         recommendation).
```

## 15.11 s8_regression_summary.v1

```text
Path:
  experiments/S8/regression/s8-regression-summary.json
  experiments/S8/regression/per-slice/{slice}/raw-stdout.txt
  experiments/S8/regression/per-slice/{slice}/raw-stderr.txt

S8RegressionSummary (JSON) :=
  {
    schema:                          "s8_regression_summary.v1",
    pass_version:                    SemVer
    rfc_revision:                    GitCommitId
    invoked_at:                      RFC3339String         ; informational
                                                            ; only;
                                                            ; excluded
                                                            ; from
                                                            ; self-hash
    per_slice:                       Map<SliceId, PerSliceSummary>
    total_runtime_seconds:           u64
    total_tests:                     u32
    total_passed:                    u32
    total_failed:                    u32
    total_skipped:                   u32                    ; must be 0
    overall_pass:                    Bool
    regression_summary_self_hash:    Hash256
  }

PerSliceSummary :=
  {
    slice_id:                        SliceId               ; "S1".."S8"
    slice_block_pass:                Bool
    runtime_seconds:                 u64
    runtime_seconds_budget:          u64                    ; per D22
    runtime_overshoot:               Bool
    commands:                        Map<CommandKind, CommandResult>
    per_slice_self_hash:             Hash256
  }

CommandResult :=
  {
    cmd:                             String
    exit_code:                       i32
    runtime_seconds:                 u64
    stdout_tail_sha256:              Hash256
    stderr_tail_sha256:              Hash256
  }

Invariants:
  RS-1   total_skipped = 0 (per §12.3)
  RS-2   overall_pass = true iff every per_slice[X].slice_block_pass
         AND total_runtime_seconds <= 300 AND total_skipped = 0
  RS-3   regression_summary_self_hash round-trips under S8CanonicalJson
```

## 15.12 s8_followup_beads.v1

Schema as in §13.1.

## 15.13 s8_epic_closure.v1

```text
Path:
  experiments/S8/epic-closure/s8-epic-closure.json

EpicClosureCertificate schema as in §3 (already pinned).

Invariants:
  EC-1   feature_records.length = 17 (F0..F16 inclusive)
  EC-2   for f in {F0, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10,
                   F11, F12, F13, F14, F16}:
           feature_records[f].status = "Closed"
  EC-3   feature_records[F15].status = "Open"
  EC-4   feature_records[F15] has three children:
           {bd-38om, bd-nyen, bd-2pg2} all Open
  EC-5   workspace_check_passes = workspace_clippy_passes =
         workspace_test_passes = true
  EC-6   bd_1rb_closure_eligible = true iff EC-1..EC-5 all hold
  EC-7   epic_closure_self_hash round-trips
```

## 15.14 s8_report.v1

```text
Path:
  docs/experiments/S8-report.md

Front-matter (YAML, hashed into report):
  ---
  schema:                            "s8_report.v1"
  s8_outcome:                        S8Outcome
  decision:                          Decision
  pareto_frontier_self_hash:         Hash256
  regression_summary_self_hash:      Hash256
  followup_beads_self_hash:          Hash256
  epic_closure_self_hash:            Hash256
  baseline_self_hash:                Hash256              ; from §6.2
  charset_v1_sha:                    Hash256              ; from S3
  gutenberg_manifest_v2_self_hash:   Hash256
  gutenberg_manifest_v1_ancestor_sha:Hash256
  per_mode_per_seed_artifacts:
    List[{
      mode:                          ModeId,
      seed:                          Seed,
      completion:                    Completed | DivergedAt(TrainStep)
                                     | NotReached,
      checkpoint_phase_a_sha:        Null | Hash256,
      checkpoint_phase_d_sha:        Null | Hash256,
      run_log_self_hash:             Null | Hash256,
      score_val_self_hash:           Null | Hash256,
      score_test_self_hash:          Null | Hash256,
      oracle_agreement_self_hash:    Null | Hash256,
      emulator_harness_self_hash:    Null | Hash256,
      supernet_run_self_hash:        Null | Hash256
                                     ; only for structured_width_gates_supernet
      hardened_export_self_hash:     Null | Hash256
                                     ; only for structured_width_gates_supernet
                                     ; entries
    }]
  generated_at:                      RFC3339 UTC; informational only;
                                     excluded from report_self_hash
  rfc_revision:                      GitCommitId | Hash256
  predictions_section_hash:          Hash256
  predictions_commit:                GitCommitId
  first_result_commit:               GitCommitId
  pass_version:                      SemVer
  test_eval_pass_versions_log_sha:   Hash256
  report_self_hash:                  Hash256
  ---

Required sections (markdown body):
  ## Pre-registered predictions
    Per-hypothesis predicted ranges and pass criteria as committed
    before any S8 result artifact commit. Repeats the §1 Predicted
    blocks verbatim. Must appear in git history strictly before
    first_result_commit.

  ## Observed
    Per (mode, seed) table: val_bpc_fp, val_bpc_ternary, ternary_gap,
    test_bpc_ternary (if test eval has been performed), v0_success_pass,
    completion. Plus aggregate stats. Plus the matched-bytes parity
    deltas. Plus the per-expert hardened payload byte costs. Plus
    the three-way oracle agreement results. Plus the emulator harness
    results.

  ## Hypothesis verdicts
    H1..H13 each as HypothesisStatus, with the concrete observation
    that drove each verdict. Closure-candidate reports must use
    only Confirmed | Refuted (per S1-style discipline).

  ## Falsification analysis
    Direct citation of which prediction or falsification rule fired
    for each Refuted hypothesis. References each relevant
    falsification test in the S8 falsification suite by file path.

  ## Surprises
    Anything outside predicted ranges, even if not a verdict change.

  ## Frontier
    Reproduce the §15.10 frontier as a markdown table. Cite
    pareto_frontier_self_hash. State FrontierRecommendationS8 in
    one line.

  ## Regression script summary
    Per-slice block pass/fail. Cite regression_summary_self_hash.

  ## F15 follow-up beads
    Reproduce the §13.1 bead snapshots. Cite followup_beads_self_hash.

  ## Epic closure certificate
    Reproduce the §15.13 feature_records table. Cite
    epic_closure_self_hash. State bd_1rb_closure_eligible in one line.

  ## Decision
    Exactly one Decision tag, justified in <= 3 sentences.

  ## Reproducibility statement
    Exact command + manifest hashes + pass_version to replay all 15
    runs PLUS the supernet run PLUS the regression script PLUS the
    test eval. Include both the `gbf s8 train` invocation and the
    `gbf s8 regress` invocation.

Invariants:
  R-Decision         Exactly one Decision tag in front-matter.
  R-AllModesAllSeeds per_mode_per_seed_artifacts covers all
                     {fixed_ternary2, structured_width_gates_supernet,
                      structured_width_gates_hardened, dense_matched_bytes}
                     x {0, 1, 2, 3, 4} = 20 entries.
                     (structured_width_gates_hardened is a derived
                     mode produced from structured_width_gates_supernet's
                     supernet checkpoint; checkpoint_phase_a_sha is
                     null for it; checkpoint_phase_d_sha is the
                     hardened artifact sha.)
  R-ClosureArtifacts For Decision = EpicClosurePass(_),
                     checkpoint_phase_a_sha, checkpoint_phase_d_sha,
                     run_log_self_hash, score_val_self_hash,
                     oracle_agreement_self_hash, and
                     emulator_harness_self_hash are non-null for every
                     non-derived (mode, seed). score_test_self_hash
                     is non-null for every non-derived (mode, seed)
                     UNLESS the test eval is deferred to a separate
                     post-RFC-merge invocation per D17 (in which case
                     pass_version on test_eval_pass_versions_log_sha
                     must already include the pinned pass_version
                     before merge).
  R-Self-Hash        report_self_hash is computed over:
                       - front-matter with generated_at and
                         report_self_hash omitted
                       - markdown body bytes exactly as committed
                     using S8CanonicalJson for front-matter
                     normalization.
  R-Predictions      The commit introducing the exact "Pre-registered
                     predictions" section, identified by
                     predictions_section_hash, is a strict ancestor
                     of first_result_commit. first_result_commit is
                     the earliest commit introducing any per-(mode,
                     seed) self-hash, supernet run self-hash, hardened
                     export self-hash, or epic closure self-hash
                     derived from S8 execution.
  R-AllHypotheses    All thirteen hypotheses have an explicit
                     HypothesisStatus. For Decision =
                     EpicClosurePass(_), every status must be a
                     binary Verdict, not NotEvaluatedDueToPriorGate.
  R-PassVersion      pass_version is recorded in front-matter and
                     matches the value in the closing PR's git
                     metadata.
```

The pre-registration timestamp is itself a load-bearing artifact:
predictions written after-the-fact are not pre-registered, even if
textually identical.

---

# 16. Reproducibility laws

```text
Rep-S8-1 Per-mode per-seed determinism
  ∀ mode in ModeSet \ {structured_width_gates_hardened}.
    ∀ s. replay(mode, s, manifest) byte-identical to original(mode, s, manifest).
  Hardened mode is a derived artifact: bit-identicality is asserted
  via the hardened_export deterministic function (Rep-S8-3).

Rep-S8-2 Cross-machine determinism is NOT required for v1.
  Bit-identicality is asserted within a single machine + OS + pinned
  Burn version + pinned dependency lockfile + S1CpuDeterministic
  device profile. Cross-platform reproducibility is a future concern.

Rep-S8-3 Hardening export determinism (NEW for S8)
  Same supernet checkpoint sha + same hardening rule sha + same
  argmax tiebreak rule
  ==> bit-identical hardened ExportVisitor output (asserted on
      canonical_artifact_payload_sha).

Rep-S8-4 Frontier byte-equality (carry-through from S5 Rep-S5-2 form)
  Same set of per-mode per-seed score reports + same supernet selector
  evolution + same hardened export shas + same matched-bytes parity
  report
  ==> bit-identical s8_pareto_frontier.v1 JSON.

Rep-S8-5 Regression summary byte-equality
  Same set of per-slice command results + same pass_version + same
  rfc_revision
  ==> bit-identical s8_regression_summary.v1 JSON
      (modulo `invoked_at` field, which is excluded from
      regression_summary_self_hash).

Rep-S8-6 Corpus pinning
  Every s8_*.v1 artifact records corpus_train_sha, corpus_val_sha,
  corpus_test_sha (where applicable), and charset_v1_sha. Replay
  validates these sha256s against the on-disk manifest before
  proceeding.

Rep-S8-7 Train-config + loss-config + shape-policy pinning
  train_config_hash binds D8 + D25 values exactly. loss_config_hash
  binds the lambda values for each mode (different defaults under
  Fixed vs StructuredWidthGates). shape_policy_hash pins the
  ExpertShapePolicy variant (Fixed or StructuredWidthGates {8, 8}).
  Changing any pinned value invalidates prior s8 artifacts.

Rep-S8-8 Pass-version pinning
  pass_version is bumped by any change to: optimizer step semantics,
  Phase A->E QAT branch behavior, supernet tau(step) ramp, hardening
  argmax tiebreak rule, lambda_shape / lambda_overflow gating
  contract, or matched-bytes parity formula. Bump invalidates
  checkpoints AND requires the test split to be re-eligible for
  evaluation under the new pass_version.

Rep-S8-9 RFC revision pinning
  s8_report.v1 records the git sha of this RFC at report generation.
  A re-run after this RFC is amended produces a new report with a
  new rfc_revision; old reports remain valid for their revision.

Rep-S8-10 Per-mode per-seed isolation
  No global mutable state is shared across modes or seeds. Mode m,
  seed s and mode m', seed s' are independent runs; no rng leakage,
  no shared tensor cache, no static mutable model registry.

Rep-S8-11 No hidden semantic inputs
  Informational report fields such as generated_at and invoked_at
  are excluded from semantic hashes and closure predicates.

Rep-S8-12 Test-split write-once log
  experiments/S8/test_eval_pass_versions.jsonl is APPEND-ONLY under
  O_APPEND atomic semantics. The log is committed to git as part of
  the closing PR; once committed, it MUST NOT be re-edited (history
  rewrites would invalidate the once-per-pass-version discipline).
```

---

# 17. Decision protocol

```text
S8 closure (bd-218w) requires:
  1. All training runs (5 modes x 5 seeds plus supernet x 5 seeds)
     Completed (D24); no DivergedAt; no hardening overflow.
  2. s8_report.v1 emitted with R-Predictions verified by git history.
  3. Decision = EpicClosurePass(M6Promote | M6ResearchOnly | M6Reject).
  4. gutenberg_manifest_v2_self_hash,
     gutenberg_manifest_v1_ancestor_sha, baseline_self_hash,
     charset_v1_sha, pareto_frontier_self_hash,
     regression_summary_self_hash, followup_beads_self_hash,
     epic_closure_self_hash, and test_eval_pass_versions_log_sha
     all recorded in front-matter.
  5. matched_bytes_parity_pass on every seed (D15).
  6. RuntimeChromeBudget preflight passes for fixed_ternary2 AND
     structured_width_gates_hardened modes (D14 / H4).
  7. Three-way oracle agreement passes on every seed for the hardened
     UpperBankCandidate artifact (D18 / H8).
  8. EncodedRom + emulator one-token harness passes at least one prompt
     per seed (D19 / H9).
  9. F15 follow-up bead naming complete; the three blocks edges from
     {bd-38om, bd-nyen, bd-2pg2} to bd-218w have been removed in this
     PR (D21 / H12).
 10. s8_regression_summary.v1.overall_pass = true (D22 / H11).
 11. s8_epic_closure.v1.bd_1rb_closure_eligible = true (H13).
 12. The gutenberg_manifest.v2 test split has been evaluated EXACTLY
     ONCE for the pinned pass_version, and the result is committed
     to s8_score.v1 test-split files BEFORE PR merge (D17).

bd-1rb closure (the parent epic) requires:
  All of the above S8 closure requirements PLUS:
 13. cargo check --workspace --all-features passes
 14. cargo clippy --workspace --all-features -- -D warnings passes
 15. cargo test --workspace --all-features passes (subset of #10's
     regression script invocation; recorded as a separate proof-of-work
     line in s8_epic_closure.v1)
 16. Every direct feature child of bd-1rb (F0..F14, F16) is Closed.
     F15 is Open with three named children whose `blocks` edges to
     bd-218w have been removed.
 17. The s8_report.v1 markdown body's "Epic closure certificate"
     section reproduces the EpicClosureCertificate verbatim.

S8 closure is forbidden when:
  Any of:
    Decision::Halt(_)
    Decision::Investigate(_)
    missing pre-registration
    any (mode, seed) completion = DivergedAt(_)
    matched_bytes_parity_pass(s) = false for any s
    H4..H13 verdict = Refuted (any one)
    test split re-evaluated for the same pass_version
    {bd-38om, bd-nyen, bd-2pg2} still listed as `blocks` of bd-218w
    s8_regression_summary.v1.overall_pass = false
    cargo check / clippy / test --workspace --all-features fails
    any required artifact missing or self-hash invalid

bd-1rb closure is forbidden when:
  Any of the S8 closure forbiddens, OR:
    s8_epic_closure.v1.bd_1rb_closure_eligible = false
    any feature child F0..F14 / F16 not Closed
    F15 missing or its three children missing

If Decision = EpicClosurePass(M6Promote):
  M6 (StructuredWidthGates) is recommended for production by future
  profile work. Add follow-up bead "post-bd-1rb: promote M6 to default
  on next profile" (post-closure tracking, NOT under bd-1rb).

If Decision = EpicClosurePass(M6ResearchOnly):
  M6 is recorded as research-mode-only. Default remains
  ExpertShapePolicy::Fixed. Add follow-up bead "post-bd-1rb: revisit
  M6 with stronger lambda or different row/col grouping".

If Decision = EpicClosurePass(M6Reject):
  M6 is honestly rejected at UpperBankCandidate scale. The enum +
  supernet + hardening pass remain in the codebase but are not the
  recommended path. Add follow-up bead "post-bd-1rb: re-evaluate M6
  if/when a profile larger than UpperBankCandidate is introduced".
```

---

# 18. Proof obligations

```text
O1  Pre-registration provability
    "Pre-registered predictions" section content of S8-report.md
    must appear in git history strictly before any S8 result
    artifact commit. CI script asserts:
      1. predictions_section_hash matches the exact normalized
         markdown section in predictions_commit;
      2. predictions_commit is a strict ancestor of first_result_commit;
      3. first_result_commit is the earliest commit that introduces
         any per-(mode, seed) self-hash, supernet run self-hash,
         hardened export self-hash, pareto frontier self-hash,
         regression summary self-hash, followup beads self-hash,
         or epic closure self-hash derived from S8 execution.
    Implementation:
      scripts/s8_preregistration_check.sh

O2  Per-mode per-seed determinism (Rep-S8-1)
    Same seed + same mode + same hashes => bit-identical safetensors.
    v1 CI closure tests:
      - replay (fixed_ternary2, seed 0) twice; assert byte equality
        of safetensors AND run_log_self_hash AND score_val_self_hash.
      - replay (structured_width_gates_supernet, seed 0) twice;
        assert byte equality of supernet checkpoint AND supernet_run_self_hash.
      - replay (dense_matched_bytes, seed 0) twice; assert byte equality.
    v1 law: all five seeds for each mode are expected to satisfy the
            same replay property.
    Implementation:
      scripts/s8_determinism_check.sh

O3  Hardening export determinism (Rep-S8-3)
    Same supernet checkpoint sha + same hardening rule sha
    => bit-identical hardened ExportVisitor output.
    CI test:
      cargo test -p gbf-model -- supernet::hardening::deterministic_replay
    plus the tiebreak fixture:
      cargo test -p gbf-model -- supernet::hardening::tiebreak_lowest_index_wins

O4  lambda_shape / lambda_overflow gating soundness (D12 / H7)
    The five test names of D12 #3 plus the H7 falsification entries
    in §13 must all pass.
    CI tests:
      cargo test -p gbf-train -- loss::shape_overflow::fixed_inert_records_zero_with_flag
      cargo test -p gbf-train --features burn-adapter -- loss::shape_overflow::structured_active_nonzero_grad
      cargo test -p gbf-train -- loss::shape_overflow::sweep_inert_under_fixed
      cargo test -p gbf-train -- loss::shape_overflow::raw_helpers_validate_finite_under_zero_lambda
      cargo test -p gbf-train -- loss::shape_overflow::contribution_collection_has_explicit_fields
      cargo test -p gbf-train --features burn-adapter -- loss::shape_overflow::non_default_values_test
    (Per CLAUDE.md "If a loss claim depends on Burn autodiff, closure
     must cite a feature-enabled gate such as `cargo test -p gbf-train
     --features burn-adapter -- <loss_test>`.")

O5  Falsification suite (S1 §13 O5 carry-through; >= 10 substitutes)
    Ten or more deliberately-broken implementations must each produce
    the expected Refuted verdict. See §18 falsification table below.
    Required test files:
      gbf-experiments/tests/falsification_s8/f1_gutenberg_v2_test_overlaps_train.rs
      gbf-experiments/tests/falsification_s8/f2_d_model_silently_clipped.rs
      gbf-experiments/tests/falsification_s8/f3_matched_bytes_inverted.rs
      gbf-experiments/tests/falsification_s8/f4_runtime_chrome_skipped.rs
      gbf-experiments/tests/falsification_s8/f5_no_gumbel_or_temperature.rs
      gbf-experiments/tests/falsification_s8/f6_random_tiebreak_argmax.rs
      gbf-experiments/tests/falsification_s8/f7_lambda_shape_active_under_fixed.rs
      gbf-experiments/tests/falsification_s8/f8_lambda_overflow_silent_skip.rs
      gbf-experiments/tests/falsification_s8/f9_test_split_used_repeatedly.rs
      gbf-experiments/tests/falsification_s8/f10_regression_skips_falsification.rs
    Gated by the test-only `falsify-s8` feature on gbf-experiments.

O6  Hash round-trip
    Every emitted s8_*.v1 artifact round-trips through canonical JSON
    with self-hash equality.
    CI test target:
      cargo test -p gbf-experiments --test canonical_json_s8

O7  Outcome algebra totality
    Every observable combination of binary H1..H13 verdicts,
    per-(mode, seed) completion states, suspicion thresholds,
    test-discipline log states, and FrontierRecommendationS8 branches
    maps to exactly one S8Outcome variant under §14.
    CI test target:
      cargo test -p gbf-experiments -- s8::outcome::totality

O8  No hidden inputs
    s8 artifacts depend only on:
      gutenberg_manifest.v2 train, val, test partitions (sha256-pinned;
        val partition is byte-identical to S4 v1 val per D2)
      gutenberg_manifest.v1 ancestor sha (v1_ancestor_manifest_self_hash;
        carry-through provenance from S4)
      tinystories manifest (for contamination check; sha256-pinned)
      charset_v1 token table (sha256-pinned)
      UpperBankCandidate ModelSizeProfile (D6 pinned)
      UpperBankCandidate-BringUp CompileProfile (D8 pinned)
      ExpertShapePolicy variants (D10 pinned)
      train_config (D8 + D25 pinned)
      loss_config (per mode; D11 + D12 pinned)
      hardening_rule (D13 pinned)
      pass_version
      gbf-train + gbf-model + gbf-data + gbf-policy + gbf-codegen
        pinned dependency set
    No env-var, no host-clock (except RFC3339 informational fields),
    no network, no stdin.

O9  Per-mode per-seed isolation
    Mode m, seed s and mode m', seed s' produce independent run
    products. No shared mutable state.
    CI smoke checks:
      1. at least two of the (mode, seed) combos produce different
         final_checkpoint_sha;
      2. running mode-pairs in (forward, reverse) order produces the
         same per-(mode, seed) hashes;
      3. variant_seed128("fixed_ternary2", "init", 0) !=
         variant_seed128("structured_width_gates_supernet", "init", 0).
    Implementation:
      scripts/s8_isolation_check.sh

O10 Closure gate
    bd-218w close is reachable iff Decision = EpicClosurePass(_).
    bd-1rb close is reachable iff bd-218w closes AND
    s8_epic_closure.v1.bd_1rb_closure_eligible = true.

O11 F15 follow-up bead naming (D21 / H12)
    The three F15 sub-beads (bd-38om, bd-nyen, bd-2pg2) exist with
    correct parent and closure conditions text, and their `blocks`
    edges to bd-218w have been removed in this PR.
    CI test:
      gbf s8 epic-closure --check-followup-beads
        (validates against the bead JSONL; exits 0 iff D21 holds)

O12 Test-split discipline (D17 / Rep-S8-12)
    The gutenberg_manifest.v2 test split is read EXACTLY ONCE per
    pinned pass_version, recorded in
    experiments/S8/test_eval_pass_versions.jsonl as APPEND-ONLY,
    bound to gutenberg_manifest.v2.test_sha256.
    CI test:
      gbf s8 test-eval --pass-version <duplicate>
        (asserts non-zero exit when called with a (pass_version,
         test_sha256) pair already in the log)

O13 Regression script overall pass + budget (D22 / H11)
    s8_regression_summary.v1.overall_pass = true AND
    total_runtime_seconds <= 300 AND total_skipped = 0.
    CI invocation:
      gbf s8 regress --pass-version <pinned> --json
        (checks exit 0)

O14 Epic closure certificate (H13)
    s8_epic_closure.v1.bd_1rb_closure_eligible = true AND every
    F0..F16 (except F15) is Closed AND F15 has the three named
    children Open AND workspace check/clippy/test pass.
    CI invocation:
      gbf s8 epic-closure --check-all
        (exits 0 iff every check passes)
```

## 18.1 Falsification suite table (>= 10 substitutes per O5)

|  ID  | Broken implementation                                          | Targets   | Expected Verdict |
| ---: | -------------------------------------------------------------- | --------- | ---------------- |
|  F1  | `gutenberg_v2_test_overlaps_train`: a book is simultaneously   | H1        | Refuted          |
|      | assigned to v2 train AND v2 test (split rule broken)           |           |                  |
|  F2  | `upperbank_d_model_silently_clipped`: profile constructor      | H4        | Refuted          |
|      | accepts d_model = 256 by silently clipping to 128              |           |                  |
|  F3  | `matched_bytes_formula_inverted`: dense_d_ff_matched is        | H3        | Refuted          |
|      | computed as -(rows*cols/4) instead of +(rows*cols/4)           |           |                  |
|  F4  | `runtime_chrome_budget_skipped_at_upperbank`: preflight        | H4        | Refuted          |
|      | step skipped when d_model > MoeTiny's d_model                  |           |                  |
|  F5  | `structured_width_gates_no_gumbel`: tau(step) is constant      | H5        | Refuted          |
|      | at 1.0; selectors never sharpen; convergence_pass = false      |           |                  |
|  F6  | `hardening_pick_argmax_with_random_tiebreak`: tiebreak         | H6        | Refuted          |
|      | uses an InitRng-derived random pick instead of lowest-index    |           |                  |
|  F7  | `lambda_shape_active_under_fixed`: validate_loss_config        | H7        | Refuted          |
|      | does NOT zero out lambda_shape under Fixed                     |           |                  |
|  F8  | `lambda_overflow_silent_skip_raw`: raw_weighted helper         | H7        | Refuted          |
|      | silently returns 0.0 instead of validating finite (CLAUDE.md   |           |                  |
|      | "do not give raw per-term diagnostic collections an implicit   |           |                  |
|      | all-zero default")                                             |           |                  |
|  F9  | `test_split_used_repeatedly`: `gbf s8 test-eval` invoked       | H1        | Refuted          |
|      | twice for the same pass_version; second call must abort        |           | (Fail-test-disc) |
| F10  | `regression_script_skips_falsification_suites`: per-slice      | H11       | Refuted          |
|      | manifest declares falsification suite as `skip = true`         |           |                  |
| F11  | `oracle_uses_gutenberg_fixtures`: oracle agreement is asserted | H8        | Refuted          |
|      | against a Gutenberg-derived fixture instead of S3-pinned       |           | (I-S8-7)         |
|      | tiny-fixture suite                                             |           |                  |
| F12  | `f15_blocks_edges_not_removed`: PR closes bd-218w but leaves   | H12       | Refuted          |
|      | the three blocks edges intact                                  |           |                  |

These twelve substitutes exceed the required >= 10 from S1 §13 O5.
Each is a unit test against the s8 framework, NOT an actual S8 run.
All gated by the `falsify-s8` feature.

---

# 19. Minimal end-to-end theorem (the EPIC theorem)

```text
Theorem S8Soundness (EPIC closure):

Given:
  - gutenberg_manifest.v2 (amendment of S4 gutenberg_manifest.v1) with
    valid per-split sha256s pinned in fixtures/corpora/gutenberg.toml;
    val partition byte-identical to S4 v1 val; test partition derived
    deterministically from v1 train via D2
  - charset_v1 token table pinned by S3
  - UpperBankCandidate ModelSizeProfile reference instance (D6,
    registered in F14)
  - UpperBankCandidate-BringUp CompileProfile (registered in F11)
  - TrainConfig pinned per D8 + D25
  - LossConfig per mode pinned per D11 + D12
  - HardeningRule pinned per D13
  - pass_version V_S8 fixed by gbf-train HEAD at S8 PR merge
  - Every closed feature F0, F1, F2, F3, F4, F6, F7, F8, F11, F12, F13,
    F14 is at its closed contract (S1 §15 + S2..S7 inheritance)

If for every mode in {fixed_ternary2, structured_width_gates_supernet,
                       structured_width_gates_hardened, dense_matched_bytes}
   and every seed s in {0, 1, 2, 3, 4}:
  - run(mode, s) returns Completed RunProduct (where structured_width_gates_hardened
    is the derived artifact from harden(structured_width_gates_supernet, s))
  - score_val(mode, s) returns finite val_bpc on gutenberg_v2_val
  - score_test(mode, s) returns finite test_bpc on gutenberg_v2_test
    (recorded EXACTLY ONCE per V_S8 in
    experiments/S8/test_eval_pass_versions.jsonl, bound to
    gutenberg_manifest.v2.test_sha256)
And:
  - matched_bytes_parity_per_seed satisfies D15 for every s
  - oracle_agreement_per_seed satisfies D18 for every (mode, seed) on
    the hardened UpperBankCandidate artifact
  - emulator_harness_per_seed satisfies D19 for every (mode, seed) on
    the hardened UpperBankCandidate artifact
  - s8_pareto_frontier.v1 is emitted with two ParetoFrontierPointS8
    entries and a deterministic FrontierRecommendationS8 in
    {M6Promote, M6ResearchOnly, M6Reject}
  - s8_regression_summary.v1.overall_pass = true with
    total_runtime_seconds <= 300 and total_skipped = 0
  - s8_followup_beads.v1 records the three F15 children with
    `blocks` edges to bd-218w removed
  - s8_epic_closure.v1.bd_1rb_closure_eligible = true with all 16
    other features Closed
  - s8_report.v1 contains pre-registered predictions in pre-run git
    history

Then:
  Each of H1, H2, H3, H4, H5, H6, H7, H8, H9, H10, H11, H12, H13 has
  a defined verdict in {Confirmed, Refuted}.

  S8Outcome is exactly one of:
    Pass-clean                        (H10 Confirmed; M6Promote)
    Pass-with-research-tail           (H10 Confirmed; M6ResearchOnly)
    Pass-with-research-rejected       (H10 Confirmed; M6Reject)
    Fail-corpus                       (H1 Refuted)
    Fail-quality                      (H2 Refuted, non-suspicious)
    Fail-suspicious                   (median val_bpc_ternary < 0.5)
    Fail-parity                       (H3 Refuted)
    Fail-budget                       (H4 Refuted)
    Fail-supernet                     (H5 Refuted)
    Fail-hardening                    (H6 Refuted)
    Fail-gating                       (H7 Refuted)
    Fail-oracle                       (H8 Refuted)
    Fail-emulator                     (H9 Refuted)
    Fail-pareto                       (H10 Refuted)
    Fail-regression                   (H11 Refuted)
    Fail-followup-naming              (H12 Refuted)
    Fail-epic-closure                 (H13 Refuted)
    Fail-substrate                    (any seed diverged)
    Fail-test-discipline              (D17 violation)

  Decision is unique under the dispatch rule of §14.

  If S8Outcome in {Pass-clean, Pass-with-research-tail,
                    Pass-with-research-rejected}, S8 has produced
  these verified knowledge claims (the EPIC claims):

    – The training-contract substrate (Burn front-end + gbf-model
      deployed numerics + gbf-train phase scheduler + gbf-codegen
      shadow compile + emulator harness) is proven to train, export,
      compile, and deploy a UpperBankCandidate-MoE model end-to-end
      on the production-scale Gutenberg corpus
      (gutenberg_manifest.v2), beating both the n-gram baseline AND
      the matched-deployed-bytes dense baseline on the val split
      (which is byte-identical to the S4 val partition), with the
      test split (newly derived in S8 D2) evaluated exactly once per
      pass_version.

    – The MoE architecture restrictions (FFN-only, two-matrix,
      tied embeddings, top-1 routing, low-rank router, temporal
      smoothness, expert dropout) scale honestly to UpperBankCandidate.

    – The ternary QAT contract (per-row Q8.8 scales,
      AnnealedGlobalThenPerOutputRow thresholds, hard ternary
      projection at Phase C entry, activation fake quant at Phase D
      entry) survives at UpperBankCandidate scale on Gutenberg with
      ternary gap <= 0.5 bpc.

    – RuntimeChromeBudget preflight at UpperBankCandidate's BringUp
      profile passes; per-bank slot byte counts are within budget;
      runtime_nucleus_hash CI drift gate is clean.

    – The three-way oracle agreement (training ≈ ArtifactOracle ≈
      DenotationalOracle) carries through to the UpperBankCandidate
      hardened artifact.

    – The EncodedRom + emulator one-token harness carries through
      to the UpperBankCandidate hardened artifact.

    – The M6 adaptive-shape research mode (StructuredWidthGates
      supernet + hardening / pruning export) is implementable; the
      supernet trains end-to-end with selectors converging to
      one-hot, the hardening export is byte-deterministic, and the
      hardened artifact passes every gate above. Its production
      payoff vs the fixed Ternary2 baseline is recorded as one of
      M6Promote / M6ResearchOnly / M6Reject; all three are honest
      closure outcomes.

    – The lambda_shape / lambda_overflow gating contract is sound:
      under ExpertShapePolicy::Fixed both lambdas are inert via the
      named contribution helper (raw = 0.0, weighted = 0.0,
      inert = true) per CLAUDE.md training-loss bullets; under
      ExpertShapePolicy::StructuredWidthGates both lambdas produce
      finite, nonzero, deterministic gradients into the per-expert
      width selectors.

    – The full regression test script (T10.15 / `gbf s8 regress`)
      re-runs every closure gate from F-S1..F-S8 in one
      deterministic CLI invocation with overall pass under the
      300-second budget.

    – Three F15 post-closure follow-up beads (bd-38om non-Q8_8 scale
      formats, bd-nyen non-Ternary2 weight encodings, bd-2pg2
      learned per-group thresholds) are NAMED with explicit closure
      conditions and dependency edges, NOT implemented; they do not
      block bd-1rb closure.

    – The training-contract revision-pass epic (bd-1rb) is verified
      end-to-end. Every direct feature child F0..F14, F16 is Closed.
      F15 is Open with three children. cargo check / clippy / test
      --workspace --all-features all pass on the pinned dependency
      lockfile.

  If S8Outcome = Pass-with-research-tail, S8 additionally verifies
  that StructuredWidthGates is implementable but does not strictly
  dominate the fixed Ternary2 baseline at UpperBankCandidate scale;
  M6 is recorded as research-mode-only.

  If S8Outcome = Pass-with-research-rejected, S8 additionally
  verifies that StructuredWidthGates is implementable but is
  honestly rejected for production at UpperBankCandidate scale; the
  enum + supernet + hardening pass remain in the codebase but are
  not the recommended path.

  If S8Outcome = Fail-corpus, S8 verifies that gutenberg_manifest.v2
  amendment integrity (val byte-identicality, train+test partition
  closure, KN-5 sanity range, or contamination thresholds) is broken;
  bd-218w cannot close; bd-1rb cannot close.

  If S8Outcome = Fail-quality, S8 verifies that UpperBankCandidate-
  MoE failed the v0_success or KN-5 margin gate on gutenberg_v2_val;
  no claim about MoE benefit at the larger scale is licensed.

  If S8Outcome = Fail-parity, S8 verifies that MoE does not justify
  bank-switch cost at UpperBankCandidate scale; the S7 parity claim
  at MoeTiny scale still holds, but does not extrapolate.

  If S8Outcome = Fail-budget, S8 verifies that UpperBankCandidate
  exceeds the BringUp profile's RuntimeChromeBudget; the profile
  may need amendment in a follow-up RFC.

  If S8Outcome = Fail-supernet, S8 verifies that
  StructuredWidthGates does not converge under the pinned schedule;
  M6 is not implementable under S8; bd-1ql cannot close. However,
  fixed_ternary2 may still pass; bd-218w may still close with M6
  marked as Fail-supernet IF H10 is then re-evaluated to
  M6 = research-rejected, BUT only if bd-1ql's closure conditions
  permit a partial M6 close, which they do not. In practice
  Fail-supernet blocks bd-1ql which blocks bd-218w; remediation is
  to investigate the supernet schedule and try again.

  If S8Outcome = Fail-hardening, S8 verifies that hardening export
  is non-deterministic OR overflows; M6 is not exportable; bd-1ql
  cannot close; bd-218w cannot close.

  If S8Outcome = Fail-gating, S8 verifies that T5.5 gating is broken;
  the lambda_shape / lambda_overflow contract is unsound; bd-3i5
  cannot close; F5 cannot close; bd-218w cannot close. This is the
  most CLAUDE.md-bullet-load-bearing failure: it indicates that the
  honest-loss contract is broken.

  If S8Outcome = Fail-oracle, S8 verifies that S3 three-way oracle
  agreement does not carry through to UpperBankCandidate scale;
  bd-218w cannot close.

  If S8Outcome = Fail-emulator, S8 verifies that the S5 (Pick and Fit) emulator harness
  does not carry through to UpperBankCandidate scale; bd-218w cannot
  close.

  If S8Outcome = Fail-pareto, S8 verifies that F8 frontier emitter
  does not produce a deterministic recommendation at UpperBankCandidate
  scale; bd-1ql cannot close; bd-218w cannot close.

  If S8Outcome = Fail-regression, S8 verifies that some prior slice's
  closure gate broke under the current pinned dependency set; bd-218w
  cannot close. The failing slice's bead must be re-opened.

  If S8Outcome = Fail-followup-naming, S8 verifies that the F15
  bead-graph operations were not performed correctly in this PR;
  bd-218w cannot close. Remediation is to perform the correct
  br dep rm operations in the same PR.

  If S8Outcome = Fail-epic-closure, S8 verifies that some epic-level
  invariant broke (a feature unexpectedly Open, workspace check failed,
  etc.); bd-1rb cannot close. The blocking feature must be re-closed
  before bd-218w.

  If S8Outcome = Fail-substrate, S8 verifies that some seed diverged;
  no later inference is licensed.

  If S8Outcome = Fail-test-discipline, S8 verifies that the test split
  was re-evaluated for a single pass_version; the test result is
  scientifically tainted. Halt; bd-218w cannot close until pass_version
  is bumped.

Not proven (post-S8 work):
  – F15 implementations (non-Q8_8 scales, non-Ternary2 encodings,
    learned per-group thresholds)
  – any profile larger than UpperBankCandidate
  – cross-machine reproducibility (Rep-S8-2)
  – production deployment on real Game Boy hardware (gbf-bench
    measurement, not S8 territory)
  – multi-corpus joint training (S8 trains on Gutenberg alone, not
    a mixture)
```

---

# 20. Implementation crate layout

Scope(F-S8) is hosted in the existing gbf-experiments crate (per S1
§15.5 commitment) under a new s8 module subtree. The closed substrate
crates are extended only by adding new entries to existing registries
or by adding the F9/T5.5 implementations to gbf-model and gbf-train.

## 20.1 Crate map

```text
gbf-policy
  Required  ModelSizeProfile::UpperBankCandidate registry entry (D6).
  Required  CompileProfile::UpperBankCandidate-BringUp registry entry
            (reuses BringUp defaults from the S5 (Pick and Fit) CompileProfile registry).
  Notes     UpperBankCandidate is added to the existing F14 registry,
            not redefined elsewhere. Closed F14 contract permits
            registry expansion as non-amending.

gbf-model
  Required  ExpertShapePolicy enum public surface (T9.1 / bd-3vu) per
            §3 + §10. Default = Fixed.
  Required  StructuredWidthGatesExpert struct + forward pass (T9.2 /
            bd-2oo) per §8.1. Includes alpha_col, alpha_row,
            tau-modulated sigmoid masks, stop-gradient phase rules.
  Required  Hardening / pruning export (T9.3 / bd-3nj) per §9.
            Includes deterministic argmax with lowest-index tiebreak,
            per-expert pruned dimensions, ExportVisitor::visit_expert
            with variable dims.
  Notes     The closed S2 ternary contract (per-row Q8.8 scales,
            AnnealedGlobalThenPerOutputRow thresholds) is consumed
            unchanged. The new supernet expert composes existing
            TernaryLinearQat modules.

gbf-train
  Required  validate_loss_config(config, shape_policy) -> ValidatedLossConfig
            per §10.1 (T5.5 / bd-3i5).
  Required  inert_shape_overflow_contribution helper per §10.1 (named
            contribution helper per CLAUDE.md bullet).
  Required  shape_penalty_raw_weighted, overflow_penalty_raw_weighted
            helpers per §10.1 (raw-weighted helpers; validate finite/
            non-negative even at lambda = 0).
  Required  ContributionsCollection struct with explicit named fields
            per §10.1 (no implicit all-zero default per CLAUDE.md
            bullet).
  Required  compose_total_loss extension that branches on
            ExpertShapePolicy and selects the right contribution
            helper per term.
  Required  tau(step) ramp per §8.2 (Phase A->E temperature schedule).
  Required  Supernet training loop integration: per-step alpha_col +
            alpha_row stop-gradient toggling per phase (D11).
  Required  Phase E hardening trigger: at step 27001..30000, the
            phase scheduler invokes harden_export AFTER the final
            optimizer step (or at step 27001 if no further training
            is desired in Phase E).
  Notes     The closed F4 phase scheduler is consumed unchanged. The
            phase boundary semantics are inherited.

gbf-data
  Required  GutenbergManifestV2 reader and v2 partition byte-stream
            loader (D1, D2). The loader verifies the v1 ancestor sha,
            the byte-identical-val invariant, the train+test partition
            closure, per-split sha256s, and per-split unmappable_rate
            before yielding bytes (E-Pre / E-Ok per §6.1).
  Required  charset_v1 normalization on the v1-inherited Gutenberg
            book set (consumes S3-pinned charset_v1 unchanged; S4
            carry-through).
  Required  per-split unmappable_rate validator per D3 (carry-through
            of S4 D5 bounds).
  Required  contamination check operation per §6.3 (vs TinyStories
            manifest; closure-gated direction set extended for v2
            test per S8 D5).
  Required  Canonical manifest path:
            fixtures/corpora/gutenberg.toml at repository root
            (amended in S8 with the v2 split block on top of the
            S4-pinned v1 fields). Same shared-across-S1..S8
            discipline as TinyStories.

gbf-foundation
  Required  Hash256, sha256 helper (S1 carry-through, unchanged).
  Required  RFC3339 helper (carry-through; informational fields only).

gbf-artifact
  Required  TernaryWeightPlan::compute_byte_cost re-used at
            UpperBankCandidate dimensions and at hardened pruned
            dimensions (per §7.2 + §9.1).
  Required  HardenedExpertPayload type per §3.
  Required  s8_*.v1 schemas: s8_corpus_manifest, s8_baseline_kn5,
            s8_run_log, s8_score, s8_matched_bytes_parity,
            s8_oracle_agreement, s8_emulator_harness,
            s8_supernet_run, s8_hardened_export,
            s8_pareto_frontier, s8_regression_summary,
            s8_followup_beads, s8_epic_closure, s8_report.
  Required  ExpertPayloadDigest with explicit ExpertId carrying
            (LayerId, local_idx) per CLAUDE.md export-fact bullet.

gbf-test
  Required  T10.15 full regression script entrypoint:
            gbf_test::regression_s8::main per §12.
  Required  Per-slice manifest schema per §12.2.
  Required  CommandResult / PerSliceSummary / S8RegressionSummary
            types per §15.10.

gbf-cli
  Required  Subcommands `gbf s8 train`, `gbf s8 supernet`,
            `gbf s8 harden`, `gbf s8 val-eval`, `gbf s8 test-eval`,
            `gbf s8 frontier`, `gbf s8 regress`,
            `gbf s8 epic-closure`, `gbf s8 replay`. Pre-registration,
            determinism, isolation, and closure scripts shell into
            this surface.
  Required  `gbf s8 epic-closure --check-followup-beads` validates
            D21 against the bead JSONL.
  Required  `gbf s8 epic-closure --check-all` validates every
            obligation in §17 #13..#17.

gbf-experiments  (existing crate, S1 §15.5)
  New module subtree gbf_experiments::s8::*. Required modules:

    gbf_experiments::s8::gutenberg_manifest_v2
      GutenbergManifestV2 reader; delegates to gbf-data. (See the
      2026-05-17 changelog note at the top of this RFC for the
      module rename rationale.)

    gbf_experiments::s8::profile
      UpperBankCandidate ModelSizeProfile binding; constructs
      ModelTopologyConfig via from_profile.

    gbf_experiments::s8::run_fixed
      s8_run(mode = fixed_ternary2, seed) operation; emits
      s8_run_log.v1 + s8_score.v1.

    gbf_experiments::s8::run_supernet
      s8_supernet_run(seed) operation; emits s8_supernet_run.v1
      AND s8_run_log.v1.

    gbf_experiments::s8::run_dense_matched
      s8_dense_matched_run(seed) operation; emits the dense
      matched-bytes baseline.

    gbf_experiments::s8::harden
      s8_harden_export(supernet_run_product, hardening_rule)
      operation per §9.1; emits s8_hardened_export.v1.

    gbf_experiments::s8::matched_bytes_parity
      s8_matched_bytes_parity_gate operation per §7.4; emits
      s8_matched_bytes_parity.v1.

    gbf_experiments::s8::oracle_agreement
      s8_oracle_agreement_check operation per §11.1; emits
      s8_oracle_agreement.v1.

    gbf_experiments::s8::emulator_harness
      s8_emulator_harness_check operation per §11.2; emits
      s8_emulator_harness.v1.

    gbf_experiments::s8::pareto_frontier
      Builds s8_pareto_frontier.v1 from the per-mode artifacts;
      computes FrontierRecommendationS8 per D20.

    gbf_experiments::s8::regression
      Reads per-slice manifests; runs each command; collects
      timing; emits s8_regression_summary.v1. Implementation
      backend for `gbf s8 regress`.

    gbf_experiments::s8::followup_beads
      Validates and snapshots the F15 bead-graph state; emits
      s8_followup_beads.v1. Implementation backend for
      `gbf s8 epic-closure --check-followup-beads`.

    gbf_experiments::s8::epic_closure
      Validates every F0..F16 status; runs cargo check / clippy /
      test --workspace --all-features; emits s8_epic_closure.v1.
      Implementation backend for `gbf s8 epic-closure --check-all`.

    gbf_experiments::s8::report
      s8_report.v1 emitter and outcome-algebra dispatcher per §14.
      Authors front-matter, validates R-Decision, R-AllModesAllSeeds,
      R-Self-Hash, R-Predictions, R-AllHypotheses, R-PassVersion,
      R-ClosureArtifacts, and binds the pre-registration commit
      history per O1.

    gbf_experiments::s8::schema
      Type definitions, S8CanonicalJson encoder, DomainHash function
      (carry-through from S1), and self-hash round-trip helpers for
      every s8_*.v1 schema.

    gbf_experiments::s8::cli
      Public entrypoint(s) for replay. The CLI surface is the
      canonical invocation point referenced by §16 Rep-S8-1 and
      §17 closure.

  New module subtree gbf_experiments::s8::oracle (D7-style metric
  oracle suite, but most metric primitives are inherited from S1):

    gbf_experiments::s8::oracle::contamination_oracle
      Hand-counted shared-13gram fixture for D5 cross-corpus
      contamination check.

    gbf_experiments::s8::oracle::matched_bytes_oracle
      Hand-computed back-solve fixture asserting D14
      dense_d_ff_matched algebra is correct.

    gbf_experiments::s8::oracle::tau_oracle
      Hand-computed tau(step) endpoint fixture asserting §8.2.
```

## 20.2 Test layout

```text
gbf-experiments/tests/falsification_s8.rs
gbf-experiments/tests/falsification_s8/*.rs
  Root harness plus twelve module files required by §18 O5; gated
  by the test-only `falsify-s8` feature so broken substitutes
  cannot leak into release builds. (Twelve substitutes for the
  >= 10 obligation; see §18.1 table.)

gbf-experiments/tests/oracle_s8.rs
gbf-experiments/tests/oracle_s8/*.rs
  Contamination, matched-bytes, tau oracle fixtures. Run
  deterministically without a trained model.

gbf-experiments/tests/canonical_json_s8.rs
gbf-experiments/tests/canonical_json_s8/*.rs
  Round-trip tests for every s8_*.v1 schema (O6). Each artifact
  must serialize, hash, deserialize, re-serialize, re-hash, and
  produce byte-identical output and self-hash equality.

gbf-experiments/tests/integration_s8.rs
gbf-experiments/tests/integration_s8/*.rs
  End-to-end smoke run against a tiny in-repo fixture corpus (NOT
  the full gutenberg_manifest.v2) used in CI to gate determinism
  (O2) and per-(mode, seed) isolation (O9). Sized so a 5-mode-x-5-seed
  run completes within the project's standard test timeout.

  The full gutenberg_manifest.v2 run is gated behind a separate CI
  job, but bd-218w closure requires that job's artifacts and
  s8_report.v1, not merely the tiny-fixture smoke run.
```

## 20.3 Artifact paths

Unchanged from §15. All run artifacts are written under the
repository-root `experiments/S8/` tree. The report is written to
`docs/experiments/S8-report.md`. The test-eval log is written to
`experiments/S8/test_eval_pass_versions.jsonl`.

## 20.4 Canonical replay command

```text
# Train all 15 base runs (3 modes x 5 seeds; supernet counts as a
# fourth mode, hardened is derived):
cargo run --release -p gbf-cli -- s8 train \
  --manifest fixtures/corpora/gutenberg.toml \
  --pass-version <pass_version_pinned_in_report> \
  --mode-list fixed_ternary2,structured_width_gates_supernet,dense_matched_bytes \
  --seed-list 0,1,2,3,4 \
  --device-profile S1CpuDeterministic

# Harden the supernet checkpoints into deployable artifacts:
cargo run --release -p gbf-cli -- s8 harden \
  --pass-version <pass_version_pinned_in_report> \
  --seed-list 0,1,2,3,4

# Score val on every (mode, seed):
cargo run --release -p gbf-cli -- s8 val-eval \
  --pass-version <pass_version_pinned_in_report> \
  --device-profile S1CpuDeterministic

# Score test on every (mode, seed) — EXACTLY ONCE per pass_version:
cargo run --release -p gbf-cli -- s8 test-eval \
  --pass-version <pass_version_pinned_in_report> \
  --device-profile S1CpuDeterministic

# Build the Pareto frontier:
cargo run --release -p gbf-cli -- s8 frontier \
  --pass-version <pass_version_pinned_in_report>

# Run the regression script:
cargo run --release -p gbf-cli -- s8 regress \
  --pass-version <pass_version_pinned_in_report> \
  --device-profile S1CpuDeterministic

# Validate epic closure readiness:
cargo run --release -p gbf-cli -- s8 epic-closure --check-all \
  --pass-version <pass_version_pinned_in_report>

# Authoritative single-shot replay (composes all of the above in
# the correct order; fails fast on any error):
cargo run --release -p gbf-cli -- s8 replay \
  --manifest fixtures/corpora/gutenberg.toml \
  --pass-version <pass_version_pinned_in_report> \
  --device-profile S1CpuDeterministic
```

Under the same machine + OS + pinned Burn version + pinned dependency
lockfile + S1CpuDeterministic, `gbf s8 replay` reproduces
`experiments/S8/**` byte-for-byte per Rep-S8-1, and reproduces
s8_pareto_frontier.v1 and s8_regression_summary.v1 byte-for-byte
per Rep-S8-4 + Rep-S8-5.

## 20.5 Workspace registration

Cargo.toml workspace `members` already includes `gbf-experiments`
(per S1 §15.5). S8 adds no new workspace members. The crate's
`Cargo.toml` adds workspace dependencies on `gbf-test` (for the
T10.15 entrypoint integration) and on the closed S5 (Pick and Fit) crates needed
for emulator harness invocation (gbf-codegen, gbf-store, gbf-emu,
or whichever crate exposes the S5 (Pick and Fit) emulator harness public surface).

---

# 21. Build configurations and feature flags

Two production build configurations participate in the S8 contract,
plus the falsification gate.

## 21.1 S8-build-A — "Fixed Ternary2 (default)"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments
Active features (workspace-resolved):
  gbf-experiments/default

gbf-experiments/default expands to:
  gbf-experiments/s8-fixed
  (plus the previously-defined per-slice features for S1..S7)

gbf-experiments/s8-fixed expands to:
  gbf-train/qat
  gbf-train/burn-adapter
  gbf-model/expert-shape-policy
  gbf-train/lambda-shape-overflow

Behavior:
  ExpertShapePolicy::Fixed is the default. Phase A->E ladder runs
  through fixed_ternary2 + dense_matched_bytes modes. Loss composer
  uses inert_shape_overflow_contribution per D12.
Build identity tag (recorded in s8_run_log.v1.metadata):
  build_kind = "s8_fixed"
```

## 21.2 S8-build-B — "Structured Width Gates supernet (M6 research)"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --no-default-features \
    --features s8-structured-width-gates,burn-adapter,qat
Active features:
  gbf-experiments/s8-structured-width-gates
  gbf-train/qat
  gbf-train/burn-adapter
  gbf-train/lambda-shape-overflow
  gbf-model/expert-shape-policy
  gbf-model/structured-width-gates-supernet
  gbf-model/structured-width-gates-hardening

Behavior:
  ExpertShapePolicy::StructuredWidthGates {8, 8} is the active
  shape policy. Supernet training enabled. Hardening pass enabled.
  Loss composer uses raw-weighted shape + overflow helpers per D12.
Build identity tag (recorded in s8_supernet_run.v1.metadata):
  build_kind = "s8_structured_width_gates"
```

## 21.3 Feature flag contract

```text
gbf-train/qat                          default-on; gates all QAT
                                       codepaths.
gbf-train/qat-ablation                 mutually exclusive with
                                       `qat`; replaces QAT codepaths
                                       with stubs (S1 contract).
gbf-train/burn-adapter                 enables Burn autodiff
                                       integration; required by
                                       loss::shape_overflow::structured_active_nonzero_grad.
gbf-train/lambda-shape-overflow        gates the lambda_shape and
                                       lambda_overflow surface; new
                                       in S8 (T5.5 / bd-3i5). Must
                                       be present for any
                                       StructuredWidthGates training
                                       run; under Fixed it is still
                                       present but the contribution
                                       helper is inert.
gbf-model/expert-shape-policy          gates the ExpertShapePolicy
                                       enum public surface; new in
                                       S8 (T9.1 / bd-3vu). Inert
                                       under default = Fixed.
gbf-model/structured-width-gates-supernet
                                       gates the supernet
                                       implementation (T9.2 / bd-2oo).
                                       Off by default; on under
                                       S8-build-B.
gbf-model/structured-width-gates-hardening
                                       gates the hardening / pruning
                                       export (T9.3 / bd-3nj). Off
                                       by default; on under
                                       S8-build-B (or whenever a
                                       hardened export is required).
gbf-experiments/s8-fixed               forwards to gbf-train/qat,
                                       gbf-train/burn-adapter,
                                       gbf-train/lambda-shape-overflow,
                                       gbf-model/expert-shape-policy
gbf-experiments/s8-structured-width-gates
                                       forwards to all of the above
                                       PLUS
                                       gbf-model/structured-width-gates-supernet,
                                       gbf-model/structured-width-gates-hardening
gbf-experiments/falsify-s8             test-only; gates the F1..F12
                                       broken substitutes used by
                                       the S8 falsification suite.
gbf-experiments/s8-regression          test-only; gates the
                                       regression script entrypoint
                                       and per-slice manifest
                                       loaders.

Mutual exclusion enforcement:
  gbf-train must compile_error! at the crate root when both `qat`
  and `qat-ablation` are enabled (S1 carry-through).
  gbf-experiments must compile_error! at the crate root when more
  than one of {s8-fixed, s8-structured-width-gates, falsify-s8}
  is enabled. This prevents a misconfigured CI from silently
  building an indeterminate binary that would invalidate the H7
  gating-soundness assertion or the H10 frontier-recommendation
  determinism.
```

## 21.4 Determinism budgets

```text
All builds run under S1CpuDeterministic (S1 §5; inherited unchanged).

  BURN_NDARRAY_NUM_THREADS=1
  BURN_DETERMINISTIC=1
  OMP_NUM_THREADS=1
  RAYON_NUM_THREADS=1

Violation aborts the run before any tensor allocation, per S1 §5
unchanged. The S8 supernet adds NO new env-var requirements; the
tau(step) ramp is deterministic by construction.
```

## 21.5 Pre-registration CI

```text
scripts/s8_preregistration_check.sh implements §18 O1, parameterized
on s8_*.v1 result artifacts:
  1. predictions_section_hash matches the markdown section in
     predictions_commit, recomputed using S8CanonicalJson
     normalization of the report front-matter and exact byte
     equality of body markdown;
  2. predictions_commit is a strict ancestor of first_result_commit;
  3. first_result_commit is the earliest commit introducing any
     per-(mode, seed) artifact hash, supernet run hash, hardened
     export hash, pareto frontier hash, regression summary hash,
     followup beads hash, or epic closure hash derived from S8
     execution.
Exit non-zero on any violation. Closure of bd-218w (and bd-1rb) is
forbidden while this script exits non-zero.
```

## 21.6 CI gates that block bd-218w + bd-1rb closure

```text
cargo test -p gbf-experiments
cargo test -p gbf-experiments --features falsify-s8 --test falsification_s8
cargo test -p gbf-experiments --test oracle_s8
cargo test -p gbf-experiments --test canonical_json_s8
cargo test -p gbf-experiments --test integration_s8
cargo test -p gbf-train --features burn-adapter -- loss::shape_overflow::structured_active_nonzero_grad
cargo test -p gbf-train --features burn-adapter -- loss::shape_overflow::non_default_values_test
cargo test -p gbf-train -- loss::shape_overflow::fixed_inert_records_zero_with_flag
cargo test -p gbf-train -- loss::shape_overflow::sweep_inert_under_fixed
cargo test -p gbf-train -- loss::shape_overflow::raw_helpers_validate_finite_under_zero_lambda
cargo test -p gbf-train -- loss::shape_overflow::contribution_collection_has_explicit_fields
cargo test -p gbf-model -- supernet::tau::endpoints_match_pinned_values
cargo test -p gbf-model -- supernet::tau::ramp_monotone_nondecreasing
cargo test -p gbf-model -- supernet::hardening::deterministic_replay
cargo test -p gbf-model -- supernet::hardening::tiebreak_lowest_index_wins
cargo test -p gbf-model -- export::variable_dim_experts::accepted_by_visitor
cargo build -p gbf-experiments --no-default-features --features s8-structured-width-gates,burn-adapter,qat
cargo build -p gbf-experiments
scripts/s8_preregistration_check.sh
scripts/s8_determinism_check.sh
  (replays {(fixed_ternary2, 0), (structured_width_gates_supernet, 0),
            (dense_matched_bytes, 0)} and asserts byte equality of
   safetensors, run_log_self_hash, score_val_self_hash; replays
   harden(structured_width_gates_supernet, 0) twice and asserts
   byte equality of canonical_artifact_payload_sha; satisfies O2 + O3)
scripts/s8_isolation_check.sh
  (asserts at least two of the (mode, seed) combos produce different
   final_checkpoint_sha; mode-pairs in (forward, reverse) order
   produce same per-(mode, seed) hashes; satisfies O9)
scripts/s8_test_split_discipline_check.sh
  (asserts that re-invoking `gbf s8 test-eval --pass-version <P>`
   when <P> is already in the log exits non-zero; satisfies O12)
gbf s8 epic-closure --check-followup-beads
gbf s8 epic-closure --check-all
gbf s8 regress --pass-version <pinned> --json
  (satisfies O13)
cargo check --workspace --all-features
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
  (satisfies bd-1rb closure step 13..15 in §17)
```

The pre-commit hook (CLAUDE.md) already runs cargo fmt + clippy +
test on every commit; the S8-specific gates above are additional
CI gates the closure PR must pass.

---

# 22. Ambiguity ledger

|  ID | Ambiguity                                                                          | Chosen path                                                                                       | Clarifying question                                                                          | Suggested final decision                                                                                                                                  |
| --: | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
|  A1 | UpperBankCandidate d_model: 96 vs 128                                              | 128 (D6); the harder budget test                                                                  | The planv0 amendment lists both as in-scope; why not 96 first?                               | bd-218w explicitly recommends 128. S8 is the EPIC closing slice; it must exercise the harder profile. d_model = 96 can be added post-closure if needed.   |
|  A2 | UpperBankCandidate n_blocks: "small"                                               | 4 (D6, matches MoeTiny n_blocks)                                                                  | "small" is informal in the planv0 amendment                                                  | n_blocks = 4 reuses S7's per-block formula unchanged, keeps total ROM bounded, and aligns with the BringUp profile's available bank classes.              |
|  A3 | StructuredWidthGates row_group / col_group                                         | row_group = col_group = 8 (D10)                                                                   | Why 8? smaller produces more selectors                                                       | 8 divides d_ff = 192 (24 col groups) and d_model = 128 (16 row groups) exactly. Smaller (e.g. 4) inflates per-step training cost; larger (e.g. 16) collapses search. |
|  A4 | tau(step) ramp shape                                                               | Two linear segments: 1.0->10.0 by Phase C end, then 10.0->100.0 by Phase D end (D11)              | Why not a sigmoid or exponential ramp?                                                       | Two-segment linear matches the F4 phase structure exactly: every phase boundary is a temperature breakpoint. Easier to debug and reason about.            |
|  A5 | lambda_shape = 0.05, lambda_overflow = 0.20 (defaults)                             | Pinned per D11; non-default test values 0.10, 0.40 per CLAUDE.md scalar bullet                    | Are these sensitive to the exact value?                                                      | Likely; the test grid in O4 covers a 4-way sweep. Bump pass_version if production tuning chooses different defaults.                                      |
|  A6 | Hardening tiebreak: lowest-index vs random                                         | Lowest-index wins (D13); deterministic                                                            | Random tiebreak might be more "fair" across runs                                              | Random would Refute H6 by construction. Determinism trumps "fairness" for an export step. Lowest-index is the simplest deterministic rule.                  |
|  A7 | Hardening retains exactly 1 col group per expert                                   | Yes (single-group hardening; §9.1)                                                                | Could retain top-K; would produce variable-width experts                                     | Single-group is the simplest deterministic rule and produces the smallest deployable artifact. Top-K is post-closure work in a future RFC.                |
|  A8 | dense_d_ff_matched: 760 vs 777 vs other                                            | 777 (closed-form back-solve in §7.3); pin actual integer at fixture creation                      | Should we record both the algebra and the integer?                                            | Yes. s8_matched_bytes_parity.v1 records the integer; this RFC records the algebra. The integer must satisfy the +/- 256 bytes margin per S7.            |
|  A9 | UpperBankCandidate per-expert byte cost: 12992 honest vs ~13.0 KiB informal        | 12992 honest (§7.2)                                                                               | The planv0 amendment says ~13.0 KiB; why the discrepancy?                                    | "~13.0 KiB" is the informal upper estimate; 12992 bytes is the honest computed value via TernaryWeightPlan formula. Both fit a 16 KiB ExpertBank slot.    |
| A10 | Reserved slack at 256 bytes per ExpertBank                                         | 256 (§7.2)                                                                                        | Why not 512?                                                                                 | 256 leaves the maximum headroom for the 12992-byte expert (3136 bytes). 512 would leave 2880 bytes; both are >> 64 metadata. 256 chosen for headroom.       |
| A11 | Test-split-once-per-pass-version discipline at CLI vs at filesystem                | Both (D17); write-once log + separate CLI subcommand                                              | Belt-and-suspenders is overkill?                                                              | This is the LOAD-BEARING scientific discipline of S8. Belt-and-suspenders is precisely the right amount of caution for a benchmark whose published value is the point. |
| A12 | gutenberg_manifest amendment approach: v1->v2 (Option A) vs separate test catalog (Option B) | v1 -> v2 with byte-identical val (Option A; D1/D2)                                              | Why amend v1 instead of fetching a fresh disjoint test catalog?                              | Option A preserves S4 -> S8 ancestry exactly: same book selection, same per-book split rule, same stripping, same charset_v1 drops, val partition byte-identical (val_sha256_v2 = val_sha256_v1). Test partition is a deterministic subset of v1 train, derived from a newly pinned test_split_seed_u128. Option B (separate catalog) would require re-fetching Gutenberg, picking a new book set, and breaking the S4 -> S8 manifest ancestry. Option A is the minimum drift consistent with adding a held-out test partition. |
| A13 | KN-5 baseline rebuild: rebuild from scratch on gutenberg_v2 train vs reuse S4 v1 baseline | Rebuild on gutenberg_v2 train (D4)                                                          | Reuse would be faster and use the closed S4 artifact                                          | Reuse would be wrong: v2 train is a strict subset of v1 train (some v1-train books were reassigned to v2 test). The S4 KN-5 counts include the v2-test books, which would leak test text into the v2 KN-5 baseline. Rebuild produces an honest comparison floor that is split-aware. |
| A12b | F16 closure point: S4 vs S8                                                       | S4 (T16.1 TinyStories at S1/S3; T16.2 Gutenberg at S4; F16 fully satisfied by S4)               | An earlier draft had F16 closing at S8 via a third corpus               | See the 2026-05-17 changelog note at the top of this RFC. T16.1 and T16.2 alone fully satisfy F16's multi-corpus contract; S4 is the legitimate F16 closure point. S8's closure list drops F16. The retired third-corpus bead and the F16 owning bead must be retired or re-scoped in a separate bead-graph operation; this RFC simply stops referencing them. |
| A14 | Train budget at 30000 steps                                                        | 30000 (D8) [ESTIMATE]                                                                             | UpperBankCandidate may need more                                                              | Pin at 30000; bump only if H2/H3 fail because of insufficient training. Surprises section is the right place for "almost-passed" reporting.               |
| A15 | Phase E hardening trigger: at step 27001 vs at step 30000                          | At step 30000 (final step); alpha frozen 27001..30000 (D11)                                       | Could harden earlier and skip Phase E                                                         | Phase E is the standard EMA + shadow_compile + frontier-emission window; preserving it keeps the F4 contract intact. Hardening AFTER the last step keeps tau at maximum sharpness. |
| A16 | F15 follow-up bead removal of `blocks` edges: at PR creation vs at PR merge        | At PR creation, snapshot at both points (D21 / §13.1)                                             | Should the edges still be present until merge?                                                | No. The edges must be removed in the same PR that closes bd-218w; they would otherwise prevent the closure operation itself. Snapshot at both points proves intent. |
| A17 | s8_followup_beads.v1 vs br-direct verification                                     | s8_followup_beads.v1 (committed artifact)                                                         | Why not just rely on `br show` at review time?                                                | Reviewers (P5, P6) need a self-hashed artifact to verify. `br show` reads the JSONL but doesn't produce a re-hashable record. The artifact is the proof. |
| A18 | Regression script: cargo test --workspace vs explicit per-slice list               | Explicit per-slice list (D22 / §12)                                                               | --workspace is simpler                                                                       | --workspace conflates slice ownership; explicit per-slice manifests give clearer failure attribution. The workspace cargo test is ALSO required (§17 #15). |
| A19 | Regression script perf budget: 5 minutes hard wall                                 | 300 seconds (D22)                                                                                  | What if a slice has a long E2E test?                                                          | 300 seconds is the slice-cumulative budget; per-slice budgets in D22 sum to 240 seconds (60 spare). Bump only if a future slice's E2E test legitimately exceeds 90s. |
| A20 | S5 BoundedKv vs LinearState for S8 sequence-state                                  | Inherit S5 (Pick and Fit) selection; S8 does not pick                                             | Which sequence-state variant runs in the UpperBankCandidate-MoE?                             | S5 (Pick and Fit) emitted a FrontierRecommendation and selected one variant for the EncodedRom build. S8 inherits that selection. The s8_run_log.v1 records which variant was used. If S5 (Pick and Fit) selected a tie or a research-only variant, S8 may need to pick explicitly; pin in the s8_corpus_manifest comment block at PR creation. |
| A21 | M6 Pareto frontier branch labels: M6Promote / M6ResearchOnly / M6Reject            | All three are legal closure outcomes (§14)                                                        | Should M6Reject block bd-1ql closure?                                                         | No. M6Reject means M6 was honestly tried and the data does not support production deployment. The enum + supernet + hardening are still implemented; bd-1ql closes with a "research-rejected" annotation. The whole point of M6 was to find out, not to win. |
| A22 | Should bd-1rb closure require all 5 seeds to pass test-eval?                       | Yes; H2 quantifies over all 5 seeds; H6/H8/H9 also (§17 #12)                                       | What if 4/5 seeds pass test-eval cleanly?                                                    | One bad seed at production scale is a substrate or capacity failure; per-seed strict mirrors S1 §D6. Aggregate pass would hide bad seeds.                |
| A23 | Test-split bpc target band                                                         | No predicted band; test bpc is reported, not gated against the predicted band                     | Should we pre-register a test-bpc range?                                                     | No. The val gate is the closure gate; test bpc is reported as the published number. Pre-registering test bpc would invite Goodhart's law on the test split. |
| A24 | F15 implementation: who picks them up post-closure?                                | Outside bd-1rb scope (post-closure work)                                                          | Who tracks them?                                                                             | They remain Open under bd-stu4 in the bead-graph indefinitely. A future PR may close them individually with their own RFCs.                                 |
| A25 | s8_epic_closure.v1 is also under bd-218w closure scope                             | Yes (§13.1 / §17 #11)                                                                              | Is it sufficient to commit it post-merge?                                                     | No. The certificate is part of the closure-PR's diff, asserting the workspace is in the closure-eligible state at PR-merge time.                          |
| A26 | F0..F16 are 17 features; bd-1rb has 17 children; F15 stays Open                    | Yes; s8_epic_closure.v1.feature_records has 17 entries with F15 as the only Open one              | What if F15 also closes accidentally?                                                        | EC-3 fails; bd_1rb_closure_eligible = false; bd-1rb cannot close. Forces the PR author to revert any accidental F15 closure.                              |
| A27 | EncodedRom + emulator harness can be slow to run at UpperBankCandidate              | Inherited tolerance + budget from S5 (Pick and Fit)                                               | What if it exceeds CI runtime?                                                                | The emulator harness obligation is "at least one prompt per seed"; one prompt per seed at UpperBankCandidate scale is bounded. If the S5 (Pick and Fit) harness budget is exceeded, the S5 (Pick and Fit) contract was wrong and a new RFC is needed. |
| A28 | The S8 RFC is itself ~5000 lines; reviewer attention budget                        | Mirrors S5 (3078 lines); S8 is larger because epic-closure                                        | Could we slim down by referring to S1..S7?                                                    | The §4 inheritance map is the slimming mechanism; everything S8 actively owns is inline. P5/P6 review will be on the inheritance + new-contract diff, not the inherited contract surface. |
| A29 | Why do M6Promote / M6ResearchOnly / M6Reject all close bd-1ql?                     | The bd-1ql contract is "M6 implementable", not "M6 wins"                                          | Could we add a "wins" bead?                                                                   | A future RFC may. bd-1ql closes when ExpertShapePolicy::StructuredWidthGates is implementable end-to-end; the production-deployment decision is separate. |
| A30 | Hardened artifact must pass matched-bytes parity gate?                             | Yes (§9.3)                                                                                        | What if hardened is smaller bytes AND lower bpc — is parity even meaningful?                  | If hardened is smaller bytes and lower bpc than the dense baseline at the matched-bytes target, parity passes trivially. The gate is robust to "hardened wins extra hard."   |
| A31 | F15 sub-beads have priority labels P2/P3                                           | Inherited from existing bead-graph                                                                 | Should S8 bump them to P1?                                                                    | No; S8 is naming, not implementing. Priority is an implementation-cadence signal; it stays at the existing values. Future implementers may re-prioritize. |
| A32 | Does S8 remove gbf-experiments/falsify (S1 falsify feature)?                       | No; S8 adds falsify-s8 alongside it                                                                | Will the regression script run both?                                                          | Yes; per D22 the regression script runs every per-slice falsification suite. Each slice's falsify-<slice> feature is invoked separately.                  |

---

# 23. Final concise contract (THE bd-1rb CLOSURE-READINESS CHECKLIST)

```text
F-S8 Production-scale + research + epic closure is correct, AND
bd-1rb (the entire training-contract revision-pass epic) is ready
to close, when:

1.  All 25 base training runs cover the cross product
    {fixed_ternary2, structured_width_gates_supernet,
     dense_matched_bytes} x {0, 1, 2, 3, 4} (15 runs) plus the
    derived structured_width_gates_hardened artifact per seed (5
    derived); the structured_width_gates_hardened mode produces no
    additional training runs but is scored on val and test as a
    fourth mode. All training runs complete Phase A->E on
    gutenberg_manifest.v2 charset_v1 without divergence and produce
    bit-identical safetensors per (mode, seed) under replay.

2.  Every (mode, seed) val_bpc_ternary on gutenberg_v2_val is finite,
    beats the rebuilt KN-5 baseline (over gutenberg_v2 train) by
    > 0.05 bpc, and passes the S3-pinned v0_success WorkloadManifest's
    eight sub-criteria (per-mode, per-seed). Ternary gap
    bpc(ternary) - bpc(fp) <= 0.5 on every (mode, seed). The
    gutenberg_manifest.v2 test split is evaluated EXACTLY ONCE per
    pass_version (D17) and the test bpc is recorded in s8_score.v1
    BEFORE the closure PR merges.

3.  The matched-deployed-bytes parity gate fires at UpperBankCandidate
    scale on every seed: bpc(MoE_fixed_ternary2, s) <
    bpc(dense_matched, s) - 0.05 for every s in {0..4}. The dense
    baseline d_ff equals the closed-form value from D14 +/- 256 bytes
    of matched per-token deployed bytes.

4.  RuntimeChromeBudget preflight at UpperBankCandidate's BringUp
    CompileProfile passes for both fixed_ternary2 and
    structured_width_gates_hardened; per-bank slot byte counts are
    within budget; runtime_nucleus_hash CI drift gate is clean;
    every hardened expert payload is <= 16128 bytes (16384 - 256
    reserved slack).

5.  StructuredWidthGates supernet trains end-to-end without divergence
    on every seed; per-expert width selectors converge to near-one-hot
    at end of Phase D (max selector > 0.90, second selector < 0.50);
    hardening / pruning export is byte-deterministic across replays
    of the same supernet checkpoint sha + same hardening rule sha;
    argmax tiebreak rule is "lowest index wins" asserted on a
    hand-crafted fixture; per-expert hardened payload byte costs
    recorded explicitly in s8_hardened_export.v1.

6.  lambda_shape and lambda_overflow gating contract (T5.5 / bd-3i5)
    is sound: under ExpertShapePolicy::Fixed both lambdas are inert
    via the named contribution helper inert_shape_overflow_contribution
    (raw_value = 0.0, weighted_value = 0.0, inert = true,
    inert_reason = "ExpertShapePolicy::Fixed"); under
    ExpertShapePolicy::StructuredWidthGates both lambdas produce
    finite, nonzero, deterministic gradients into per-expert width
    selectors via the raw-weighted helpers shape_penalty_raw_weighted
    and overflow_penalty_raw_weighted (which validate finite/non-
    negative raw diagnostics even at lambda = 0). The
    ContributionsCollection has explicit named fields per term, no
    implicit all-zero default. CI tests
    `loss::shape_overflow::{fixed_inert_records_zero_with_flag,
                             structured_active_nonzero_grad,
                             sweep_inert_under_fixed,
                             raw_helpers_validate_finite_under_zero_lambda,
                             contribution_collection_has_explicit_fields,
                             non_default_values_test}` all pass; the
    Burn-autodiff-dependent tests cite the burn-adapter feature
    gate per CLAUDE.md.

7.  The S3-pinned three-way oracle agreement re-validates on the
    hardened UpperBankCandidate artifact AND the fixed UpperBankCandidate
    artifact: max_abs_diff(training_logits, artifact_oracle_logits)
    <= 1e-4 in f32 AND max_abs_diff(training_logits,
    denotational_oracle_logits) <= 1e-4 in f32 on every seed on the
    S3-pinned tiny-fixture suite.

8.  The S5 (Pick and Fit) pinned EncodedRom + emulator one-token harness re-validates
    on the hardened UpperBankCandidate artifact AND the fixed
    UpperBankCandidate artifact: at least one v0_success prompt per
    seed produces a well-formed first token within S6_PINNED_TOLERANCE
    (code constant; owner is now gbf_experiments::s5)
    of the training-side logits.

9.  s8_pareto_frontier.v1 is emitted with exactly two
    ParetoFrontierPointS8 entries (P_fixed, P_hard); all axes are
    populated; FrontierRecommendationS8 in {M6Promote, M6ResearchOnly,
    M6Reject} is computed deterministically from the two points per
    D20; replay produces the same recommendation. All three branches
    are legal closure outcomes.

10. `gbf s8 regress --pass-version <pinned>` re-runs every closure
    gate from F-S1..F-S8 in the order pinned by D22; per-slice
    block returns Pass for every slice; s8_regression_summary.v1
    has overall_pass = true, total_skipped = 0, and
    total_runtime_seconds <= 300.

11. F15 (bd-stu4) is Open with three NAMED-NOT-IMPLEMENTED children:
    bd-38om (non-Q8_8 ternary scale tensor formats), bd-nyen
    (non-Ternary2 artifact weight encodings), bd-2pg2 (learned
    per-group ternary threshold state). Each child is parented under
    bd-stu4, has its `blocks` edge to bd-218w REMOVED in this PR
    (verified by s8_followup_beads.v1 snapshot at PR-merge time),
    and carries the D21-pinned closure conditions text as a top-level
    comment on the bead.

12. s8_epic_closure.v1 records every F0..F16 status: F0..F14 + F16
    are all Closed; F15 is Open with the three children. Workspace
    cargo check / clippy / test --workspace --all-features all pass
    on the pinned dependency lockfile. bd_1rb_closure_eligible = true.

bd-218w closure is reachable iff statements 1..12 all hold AND
s8_report.v1 emits pre-registered predictions in git history strictly
before the first per-(mode, seed) artifact commit AND concludes with
exactly one Decision = EpicClosurePass(M6Promote | M6ResearchOnly |
M6Reject).

bd-1rb closure (the parent epic) is reachable iff bd-218w closes AND
s8_epic_closure.v1.bd_1rb_closure_eligible = true.

Closure of bd-1rb retires the entire training-contract revision-pass
epic. The training stack is verified end-to-end at production scale
on the full Gutenberg corpus (gutenberg_manifest.v2) with the
largest pinned profile. M6 adaptive-shape research mode is
implementable; whether it ships depends on the FrontierRecommendationS8
branch but does not block closure. F15 post-closure follow-ups
remain Open under bd-stu4 for indefinite future work.

S8 retires:
  – production-scale + Gutenberg-with-held-out-test risk
  – UpperBankCandidate profile fit + train risk
  – M6 adaptive-shape research-mode implementability risk
  – T5.5 lambda_shape / lambda_overflow gating-soundness risk
  – T10.15 full-regression-script-completeness risk
  – F15 post-closure follow-up bead-naming risk
  – bd-1rb epic-closure-readiness risk

S8 does NOT retire:
  – F15 implementation (post-closure work, indefinite timeline)
  – cross-machine reproducibility (Rep-S8-2; future concern)
  – production deployment on real Game Boy hardware (gbf-bench
    measurement; not training-contract territory)
  – multi-corpus joint training (S8 trains on Gutenberg alone)
  – any profile larger than UpperBankCandidate (would require a
    new RFC and likely a new epic)
```
