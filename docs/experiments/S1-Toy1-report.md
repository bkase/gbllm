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
  "generated_at": "2026-05-10T00:00:00Z",
  "rfc_revision": "92b6cb53a489558e9f39e36a05d160e45750e190",
  "predictions_section_hash": "sha256:a2a1f33df69e97b3296f592fefd311503f29e75bff10036cd81855cc96964e1e",
  "predictions_commit": null,
  "first_result_commit": null,
  "report_self_hash": "sha256:419cc61e052accfe2da05cc4c5b7202dacfb0754b0756438b10dcacc11ed1eda"
}
---
# S1 Toy1 Successor Report

This report is the pre-result registration for the `bd-2ibf` Toy1 successor
run created after the committed Toy0 `Fail-capacity` result. It does not
rewrite `docs/experiments/S1-report.md`.

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

Toy1 has not run yet. No Toy1 result artifacts are claimed by this report.

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | NotEvaluatedDueToPriorGate(Toy1 not yet run) | No Toy1 run logs exist yet |
| H2 | NotEvaluatedDueToPriorGate(Toy1 not yet run) | No Toy1 scores exist yet |
| H3 | NotEvaluatedDueToPriorGate(Toy1 not yet run) | No Toy1 negative-test artifact exists yet |
| H4 | NotEvaluatedDueToPriorGate(Toy1 not yet run) | No Toy1 ablation artifact exists yet |
| H5 | NotEvaluatedDueToPriorGate(Toy1 runtime support pending) | No Toy1 oracle/report input has been bound yet |

## Falsification analysis

No Toy1 falsification result exists yet.

## Surprises

None yet.

## Decision

Not run.

## Reproducibility statement

- preregistration: `scripts/s1_preregistration_check.sh --report docs/experiments/S1-Toy1-report.md --artifact-dir experiments/S1-toy1`
- artifact directory: `experiments/S1-toy1`
