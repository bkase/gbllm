---
{
  "schema": "s1_report.v1",
  "s1_outcome": null,
  "decision": "NotYetRun",
  "baseline_self_hash": null,
  "per_seed_artifacts": [
    {
      "seed": 0,
      "completion": {
        "kind": "not_reached"
      },
      "checkpoint_self_hash": null,
      "run_log_self_hash": null,
      "score_self_hash": null,
      "negative_self_hash": null,
      "ablation_self_hash": null
    },
    {
      "seed": 1,
      "completion": {
        "kind": "not_reached"
      },
      "checkpoint_self_hash": null,
      "run_log_self_hash": null,
      "score_self_hash": null,
      "negative_self_hash": null,
      "ablation_self_hash": null
    },
    {
      "seed": 2,
      "completion": {
        "kind": "not_reached"
      },
      "checkpoint_self_hash": null,
      "run_log_self_hash": null,
      "score_self_hash": null,
      "negative_self_hash": null,
      "ablation_self_hash": null
    },
    {
      "seed": 3,
      "completion": {
        "kind": "not_reached"
      },
      "checkpoint_self_hash": null,
      "run_log_self_hash": null,
      "score_self_hash": null,
      "negative_self_hash": null,
      "ablation_self_hash": null
    },
    {
      "seed": 4,
      "completion": {
        "kind": "not_reached"
      },
      "checkpoint_self_hash": null,
      "run_log_self_hash": null,
      "score_self_hash": null,
      "negative_self_hash": null,
      "ablation_self_hash": null
    }
  ],
  "generated_at": "2026-05-09T18:15:00Z",
  "rfc_revision": "7852fd17328e0631123df016baa88a4c0e48b601",
  "predictions_section_hash": "sha256:a05b43620bd26cb2cb79b37c041bd7cbed57508321f9bd81b775940c33370b3f",
  "predictions_commit": null,
  "first_result_commit": null,
  "report_self_hash": "sha256:b7244d90d7bb10b34d633a3ddc27e4126950c4e5ac0ff3ec1c9d5147599112bd"
}
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

Populated by F-S1.29 after the run completes.

## Hypothesis verdicts

Populated by F-S1.29 after the run completes.

## Falsification analysis

Populated by F-S1.29 after the run completes.

## Surprises

Populated by F-S1.29 after the run completes.

## Decision

Populated by F-S1.29 after the run completes.

## Reproducibility statement

Populated by F-S1.29 after the run completes.
