---
{"baseline_self_hash":"sha256:ab10244caffbdedf7727b08a17edc970f392f258a05c8b2de486b1a20d8e731c","decision":{"kind":"ProceedToS2-with-H2-waiver","reason":"toy1-narrow-h2-miss"},"first_result_commit":"6ccde91e69df039e9399e63aaf72196ccd4d7ce0","generated_at":"2026-05-10T18:56:05Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1bffbb8ec4d8c47579366474e2edbbb19b842844c84f117d8558b86ad74d5a7b","checkpoint_self_hash":"sha256:00c9a068576eb851e1297cf172db872ab53274ec074f468491e4d3ec10345978","completion":{"kind":"completed"},"negative_self_hash":"sha256:b5f402e02e4d8f1fa78c891608af7085b147805c8d90c2a18a0949d86071f2f0","run_log_self_hash":"sha256:4aa91c97999fc3dee98f13feea656ef0ced50866a1bfa80983b708ad9026e10a","score_self_hash":"sha256:bf3be861da68da4fa7fd1be6be3b77449e114f49611866fa296f361c4aed58aa","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:f7921a8e941a81942ce2fd43c69e51ba1704f2853ab187e53dce2d7af9ce6d55","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:d1e5bc208db49afbf7204eae6a3415ae8b5792c00f05c991cf6937b05086a2ea","score_self_hash":"sha256:a30e43a566fb7f5f2dadc0d491ba537ab158723297322c0c190622a0f11c7592","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:e1ad975d58b1b43ad51b1ac54fee0aca352e23fcf1857aad63b5e4d43bd3afa0","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:2862ff0b04cd09e897f21d232228f3028895880a4a3f1d56de5a567ee5682250","score_self_hash":"sha256:2ca2f3ff331fd22f2551c96b890f16d26a630f6b5daa99a8346b13803c7e19f9","seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:48cc66009ba5a2fdb6bd0c9d7311dbc162d4059304e14fe56a618ce5d9b7f9c2","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:c8e16ba672f064cb5820055ced73e8e8c44b9408fe8b606be99ea87361e5d51a","score_self_hash":"sha256:de9b0dbb2d362f05e90f2a52e7172a0bce80b2f073bc1b53862f915f6deb8c99","seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:abd36611d0537530eddf31c256292e969a9554dc4527c948107735ba76afdf5b","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:8de783399846e982b6e8ed6418a50752ea592ba49f719355f9e987bfab394c0d","score_self_hash":"sha256:deba3db548f7b3d35dc3b1ffc7e830e2a319b1c7c8dfb43cd99093c8591a1a77","seed":4}],"predictions_commit":"8814daef35d688db4edb5cb0efa63d358afa2814","predictions_section_hash":"sha256:a2a1f33df69e97b3296f592fefd311503f29e75bff10036cd81855cc96964e1e","report_self_hash":"sha256:33995c0d6a1f8b6f3257b1faf4b4663537db7809630c3190a94dc578a4490467","rfc_revision":"248c514f968d5ef4a109785ed66ecbb3c985617f","s1_outcome":"Fail-capacity","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

### H1 Plumbing

Statement: for every seed, the Toy1 training loop produces finite losses and
finite gradient norms, and early training loss decreases over the
pre-registered windows.

Predicted observables:

- mean_train_loss(seed, steps 1..10) is in [4.0, 6.5] nats per byte.
- mean_train_loss(seed, steps 91..100) is less than
  mean_train_loss(seed, steps 1..10) - 0.5.
- Every recorded grad_norm(seed, step) is finite and >= 0.
- At least one recorded grad_norm(seed, step) is > 0.

Falsification rules:

- Any non-finite loss at any seed or step refutes H1.
- Any seed with mean_train_loss(91..100) >=
  mean_train_loss(1..10) - 0.5 refutes H1.
- Any non-finite grad_norm at any seed or step refutes H1.
- Any seed whose recorded grad_norm is zero at every step refutes H1.

### H2 Toy1 capacity

Statement: Toy1 (`ModelSizeProfile::Toy1`: d_model=32, d_ff=64, n_blocks=2,
vocab=256) has enough representational power to model TinyStories n-gram
structure better than the fixed 3-gram baseline by a margin strictly greater
than 0.05 bpc for every seed.

Predicted observables:

- bpc_3gram_baseline is in [1.7, 2.0]. This is a sanity range only.
- median(val_bpc(seed)) is in [1.4, 1.8]. This is a sanity range only.
- For every seed, val_bpc(seed) < bpc_3gram_baseline - 0.05. This is the
  actual gate.

Falsification rules:

- Any seed with val_bpc(seed) >= bpc_3gram_baseline - 0.05 refutes H2.
- median(val_bpc) < 0.5 refutes H2 and triggers the suspicious-low-bpc halt
  path.

### H3 Sequence-state utility

Statement: the Toy1 model as a whole uses byte context enough to beat a unigram
baseline on the same validation bytes by strictly more than 0.5 bpc.

Non-claim: H3 does not isolate LinearStateBlock causality. That requires a
state-disabled or state-varied ablation outside S1.

Predicted observables:

- bpc_unigram_val is in [3.5, 5.0].
- For every seed, val_bpc(seed) < bpc_unigram_val - 0.5.
- model_neg_test_delta(seed=0) > 2.0.

Falsification rules:

- Any seed with val_bpc(seed) >= bpc_unigram_val - 0.5 refutes H3.
- model_neg_test_delta(seed=0) <= 2.0 refutes H3.

### H4 Phase A cleanliness

Statement: the phase scheduler at hardness=(Off, Off, Off) for (expert_qat,
activation_qat, norm_qat) produces Toy1 results bit-identical to an ablation
build in which all QAT code paths are compiled out.

Predicted observables:

- canonical_tensor_payload_sha(seed=0, phase_a_run) equals
  canonical_tensor_payload_sha(seed=0, ablation_run).
- H4 compares trainable tensor payloads only; checkpoint metadata, build_kind,
  SafeTensors metadata, and artifact paths do not participate in the H4
  equality decision.

Falsification rule:

- phase_a_tensor_payload_sha != ablation_tensor_payload_sha refutes H4.

### H5 Measurement

Statement: bpc scoring, reset-boundary handling, 3-gram baseline math, and
validation shuffling are implemented according to the F-S1 RFC and the Toy1
successor amendment.

Predicted observables:

- metric_oracle_passed = true.
- shuffle_multiset_preserved = true.

Falsification rules:

- metric_oracle_passed = false refutes H5.
- shuffle_multiset_preserved = false refutes H5.

### Pass criterion and prediction-status rule

D6 per-seed strict pass criterion:

- The Toy1 closure candidate requires all five seeds {0, 1, 2, 3, 4} to be
  covered exhaustively.
- For closure-candidate decisions ProceedToS2 and
  ProceedToS2-with-T12.5-prereq, checkpoint_self_hash, run_log_self_hash, and
  score_self_hash must be non-null for all five seeds.
- For closure-candidate decisions, negative_self_hash and ablation_self_hash
  must be non-null for seed 0.
- H1, H2, H4, and H5 are mandatory closure gates. H3 is non-closure-gating but
  determines whether S2 receives the T12.5 prerequisite.

Prediction-status rule:

- Entries listed under a hypothesis's Predicted block are pre-registered
  expectations.
- A predicted-range miss affects the verdict only when the same condition is
  repeated under that hypothesis's Falsification block.
- Otherwise, out-of-range observations are reported as Surprises, not automatic
  Refutations.

Artifact isolation rule:

- Toy1 result artifacts live under `experiments/S1-toy1`.
- This report is checked with
  `scripts/s1_preregistration_check.sh --report docs/experiments/S1-Toy1-report.md --artifact-dir experiments/S1-toy1`.
- Existing Toy0 artifacts under `experiments/S1` and the original
  `docs/experiments/S1-report.md` are predecessor evidence, not Toy1 result
  artifacts.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 2.474073 | 8.499022 | true |
| 1 | Completed | 2.614371 | NA | NA |
| 2 | Completed | 2.493123 | NA | NA |
| 3 | Completed | 2.481167 | NA | NA |
| 4 | Completed | 2.538161 | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Confirmed | Production seeds 0..4 completed; minimum H1 loss-window drop was 2.442829 and final grad norms were finite, nonnegative, and nonzero |
| H2 | Refuted | Production seed 1 val_bpc 2.614371 failed H2 threshold 2.570544; best=2.474073 median=2.493123 worst=2.614371 |
| H3 | Confirmed | Production all-seed val_bpc beat H3 unigram threshold 3.950948; best=2.474073 median=2.493123 worst=2.614371; negative delta 8.499022 sensitive=true |
| H4 | Confirmed | Production ablation phase_a_eq_ablation=true |
| H5 | Confirmed | Production oracle metric_oracle_passed=true failed_ids=[] |

## Falsification analysis

Production S1 CLI report collected canonical TinyStories artifacts from disk.
H2 falsification: Production seed 1 val_bpc 2.614371 failed H2 threshold 2.570544; best=2.474073 median=2.493123 worst=2.614371

## Surprises

- bpc_3gram_baseline 2.620544 was outside preregistered sanity range [1.7, 2.0]; this range miss is reported as a Surprise, not a verdict change.
- median(val_bpc) 2.493123 was outside preregistered sanity range [1.4, 1.8]; this range miss is reported as a Surprise, not a verdict change.

## Decision

`ProceedToS2-with-H2-waiver(toy1-narrow-h2-miss)`. Decision records a human-approved Toy1 narrow H2 miss waiver: H2 remains Refuted, but all Toy1 seeds beat the 3-gram baseline and only one seed missed the 0.05 bpc margin by at most 0.05 bpc.

## Reproducibility statement

- command: `gbf s1 replay --manifest fixtures/corpora/tinystories.toml --seed-list 0,1,2,3,4`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:6418d412de72888f52b5142c761ac21a582f7d1166f0bfbdb5f03ccfdec90443 val_sha=sha256:6874bae9a4c1a4e7edcf0e53b86c17817e9cf881fc75ff2368da457b80c0585d
