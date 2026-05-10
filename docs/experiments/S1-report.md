---
{"baseline_self_hash":"sha256:ab10244caffbdedf7727b08a17edc970f392f258a05c8b2de486b1a20d8e731c","decision":{"kind":"Investigate","reason":"propose-Toy1"},"first_result_commit":"c4170d9a6a316f210a6a4b8d74bd93f827c4a8f9","generated_at":"2026-05-09T23:32:46Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:57229bbaeae60c19057c00eacaf1f2079c200be14f1c17c77eaaa57139733898","checkpoint_self_hash":"sha256:468b78bd28070e9fdc3ab4382a3c8a91d773c8ae2b5667a32367295bed2c2b8c","completion":{"kind":"completed"},"negative_self_hash":"sha256:f33ca53e3f4e387b0b5c802b12a98555f56b2521cb742fa2268e21fe6b1fdc9a","run_log_self_hash":"sha256:b709918eddbd41b54e9074b039a932d8be552d1131be009336db99a485b07c68","score_self_hash":"sha256:dbf543b2aa237c12d8936ef4b7b62f7ad5d433a8bf5098b8faaa9350b364ec03","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:152ccc04946cd5562b1ac8f3dd614240b6afa2930e79a42e0902014493a1ded3","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:38083ec648d08aedf57df299d137e603e2deed5f5f338e1d1ccf671c3150edc3","score_self_hash":"sha256:77b2f1df965cbb87be2b6e5fe6e361fbbc5e041dbb33d99d5da455f163c2447b","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:26d82622d15ae35d1edd1233cf1f2a574ca355aa243d497f09989ee87fa1b5e8","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:8aa1e41223aab730337e17debd2700d11170b970bc6d5c52573bc7b1bf92820f","score_self_hash":"sha256:4efb4c7ae1759fe516ce04f1e2841a0504e7b0f4b69232fe0062de13cf2d5220","seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:7e42b52f28e3416c172bd11620eae469f3bb07dc618f97ee341d84e2b556c97d","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:e36d30dea8c4896412fd986c5bf6e6de3377847750da46bfe5fc7d051616364b","score_self_hash":"sha256:c8729444352d3e6a1f29576f83de9a580a79dcba663bc41a78fed07232ed24ea","seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:c2e7b137e80a8dce0ef00c72458958a5b55049bbf76f2461c69baf734c0536f6","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:62ad2e23ca53be23dc3db4c9805f73df67d1053d022f7d1cada08e30d3f4ed0f","score_self_hash":"sha256:b6973c1bcc8c135d579a4fe9528039e47168cf508826de793b23374e2d1628d9","seed":4}],"predictions_commit":"ffb26a6823799e9fb55eaa5fd0f26779afa26f2e","predictions_section_hash":"sha256:a05b43620bd26cb2cb79b37c041bd7cbed57508321f9bd81b775940c33370b3f","report_self_hash":"sha256:fa6241b705886dc02c81a2fd3b96462cba533068c9c66b489d4164c09108088d","rfc_revision":"7852fd17328e0631123df016baa88a4c0e48b601","s1_outcome":"Fail-capacity","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

### H1 Plumbing

Statement: for every seed, the training loop produces finite losses and finite gradient norms, and early training loss decreases over the pre-registered windows.

Predicted observables:

- mean_train_loss(seed, steps 1..10) is in [4.0, 6.5] nats per byte.
- mean_train_loss(seed, steps 91..100) is less than mean_train_loss(seed, steps 1..10) - 0.5.
- Every recorded grad_norm(seed, step) is finite and >= 0.
- At least one recorded grad_norm(seed, step) is > 0.

Falsification rules:

- Any non-finite loss at any seed or step refutes H1.
- Any seed with mean_train_loss(91..100) >= mean_train_loss(1..10) - 0.5 refutes H1.
- Any non-finite grad_norm at any seed or step refutes H1.
- Any seed whose recorded grad_norm is zero at every step refutes H1.

Surprise, not refutation:

- max_step grad_norm(seed, step) >= 1e3 is reported as a surprise unless a falsification rule also fires.

Verdict mapping:

- H1 is Refuted if any H1 falsification rule fires.
- H1 is Confirmed otherwise.

Consequence of Refuted:

- S2..S8 cannot proceed. Non-finite loss, non-finite grad norm, or all-zero gradients are investigated as Burn, autodiff, optimizer, or RNG substrate failures. An early-loss-window-only refutation is treated as a smoke-learning failure before concluding the substrate is broken.

### H2 Capacity

Statement: Toy0 (d_model=16, d_ff=32, one block, vocab=256) has enough representational power to model TinyStories n-gram structure better than the fixed 3-gram baseline by a margin strictly greater than 0.05 bpc for every seed.

Predicted observables:

- bpc_3gram_baseline is in [1.7, 2.0]. This is a sanity range only.
- median(val_bpc(seed)) is in [1.4, 1.8]. This is a sanity range only.
- For every seed, val_bpc(seed) < bpc_3gram_baseline - 0.05. This is the actual gate.

Falsification rules:

- Any seed with val_bpc(seed) >= bpc_3gram_baseline - 0.05 refutes H2.
- median(val_bpc) < 0.5 refutes H2 and triggers the suspicious-low-bpc halt path.

Verdict mapping:

- H2 is Refuted if any H2 falsification rule fires.
- H2 is Confirmed otherwise.

Consequence of Refuted:

- For the non-suspicious path, Toy0 may be undersized; open a follow-up bead proposing Toy1 (d_model=32, d_ff=64) as the actual S1 model and re-run.
- For median(val_bpc) < 0.5, Halt: audit train/val leakage, the bpc accumulator, and the corpus loader before any later slice proceeds.

### H3 Sequence-state utility

Statement: the Toy0 model as a whole uses byte context enough to beat a unigram baseline on the same validation bytes by strictly more than 0.5 bpc.

Non-claim: H3 does not isolate LinearStateBlock causality. That requires a state-disabled or state-varied ablation outside S1.

Predicted observables:

- bpc_unigram_val is in [3.5, 5.0].
- For every seed, val_bpc(seed) < bpc_unigram_val - 0.5.
- model_neg_test_delta(seed=0) > 2.0.

Falsification rules:

- Any seed with val_bpc(seed) >= bpc_unigram_val - 0.5 refutes H3.
- model_neg_test_delta(seed=0) <= 2.0 refutes H3.

Verdict mapping:

- H3 is Refuted if any H3 falsification rule fires.
- H3 is Confirmed otherwise.

Consequence of Refuted:

- S1 can still pass if H1, H2, H4, and H5 confirm, because H3 is non-closure-gating.
- Add the conservative prerequisite edge S2.closure <- T12.5 (bd-1y1s) before S2 closure.
- Do not attribute causality to LinearStateBlock without a state-disabled or state-varied ablation.

### H4 Phase A cleanliness

Statement: the phase scheduler at hardness=(Off, Off, Off) for (expert_qat, activation_qat, norm_qat) produces results bit-identical to an ablation build in which all QAT code paths are compiled out.

Predicted observables:

- canonical_tensor_payload_sha(seed=0, phase_a_run) equals canonical_tensor_payload_sha(seed=0, ablation_run).
- Whole-file safetensors byte equality is non-normative and may be reported only as an observation if the writer is canonicalized.
- H4 compares trainable tensor payloads only; checkpoint metadata, build_kind, SafeTensors metadata, and artifact paths do not participate in the H4 equality decision.
- Seeds 1..4 may be compared optionally and reported as observational.

Falsification rule:

- phase_a_tensor_payload_sha != ablation_tensor_payload_sha refutes H4.

Verdict mapping:

- H4 is Confirmed if seed 0 produces bit-identical trainable tensor payloads between phase_a and ablation modes.
- H4 is Refuted otherwise.

Consequence of Refuted:

- Phase A is contaminated by later-phase code. This is a phase-contract bug, not a numerical issue. Block S2 until F4's phase scheduler is fixed.

### H5 Measurement

Statement: bpc scoring, reset-boundary handling, 3-gram baseline math, and validation shuffling are implemented according to the F-S1 RFC.

Predicted observables:

- metric_oracle_passed = true.
- shuffle_multiset_preserved = true.

Falsification rules:

- metric_oracle_passed = false refutes H5.
- shuffle_multiset_preserved = false refutes H5.

Verdict mapping:

- H5 is Confirmed if all D7 measurement-oracle checks pass.
- H5 is Refuted otherwise.

Consequence of Refuted:

- Halt. bpc math or validation construction is wrong, and every later slice's gate numbers are unreliable until this is fixed.

### Pass criterion and prediction-status rule

D6 per-seed strict pass criterion:

- The S1 closure candidate requires all five seeds {0, 1, 2, 3, 4} to be covered exhaustively.
- For closure-candidate decisions ProceedToS2 and ProceedToS2-with-T12.5-prereq, checkpoint_self_hash, run_log_self_hash, and score_self_hash must be non-null for all five seeds.
- For closure-candidate decisions, negative_self_hash and ablation_self_hash must be non-null for seed 0.
- H1, H2, H4, and H5 are mandatory closure gates. H3 is non-closure-gating but determines whether S2 receives the T12.5 prerequisite.

Prediction-status rule:

- Entries listed under a hypothesis's Predicted block are pre-registered expectations.
- A predicted-range miss affects the verdict only when the same condition is repeated under that hypothesis's Falsification block.
- Otherwise, out-of-range observations are reported as Surprises, not automatic Refutations.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 3.111780 | 7.428885 | true |
| 1 | Completed | 3.148044 | NA | NA |
| 2 | Completed | 3.081588 | NA | NA |
| 3 | Completed | 3.137616 | NA | NA |
| 4 | Completed | 3.115104 | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Confirmed | Production seeds 0..4 completed; minimum H1 loss-window drop was 2.332301 and final grad norms were finite, nonnegative, and nonzero |
| H2 | Refuted | Production seed 0 val_bpc 3.111780 failed H2 threshold 2.570544; best=3.081588 median=3.115104 worst=3.148044 |
| H3 | Confirmed | Production all-seed val_bpc beat H3 unigram threshold 3.950948; best=3.081588 median=3.115104 worst=3.148044; negative delta 7.428885 sensitive=true |
| H4 | Confirmed | Production ablation phase_a_eq_ablation=true |
| H5 | Confirmed | Production oracle metric_oracle_passed=true failed_ids=[] |

## Falsification analysis

Production S1 CLI report collected canonical TinyStories artifacts from disk.
H2 falsification: Production seed 0 val_bpc 3.111780 failed H2 threshold 2.570544; best=3.081588 median=3.115104 worst=3.148044

## Surprises

- bpc_3gram_baseline 2.620544 was outside preregistered sanity range [1.7, 2.0]; this range miss is reported as a Surprise, not a verdict change.
- median(val_bpc) 3.115104 was outside preregistered sanity range [1.4, 1.8]; this range miss is reported as a Surprise, not a verdict change.

## Decision

`Investigate(propose-Toy1)`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `gbf s1 replay --manifest fixtures/corpora/tinystories.toml --seed-list 0,1,2,3,4`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:6418d412de72888f52b5142c761ac21a582f7d1166f0bfbdb5f03ccfdec90443 val_sha=sha256:6874bae9a4c1a4e7edcf0e53b86c17817e9cf881fc75ff2368da457b80c0585d
