---
{"baseline_self_hash":"sha256:5ac855077d78dd1e69edec76d5cbd92b8ad795e3cb11d22b73f2656a50b8d034","decision":{"kind":"Investigate","reason":"burn-or-autodiff"},"first_result_commit":"cccccccccccccccccccccccccccccccccccccccc","generated_at":"2026-05-09T12:00:00Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1c062aa3cfdf233a3b5e403f6b9befa7657c2cad797058f4d02ea0a8361989d8","checkpoint_self_hash":"sha256:0e942c2028d311e532e676da9b246a746c0ea7df9bfdb94e2e8a4f5db2b2abf0","completion":{"kind":"completed"},"negative_self_hash":"sha256:f2d57d9c53f1f93beeb167739ba35e2e0557c4f589d2976c57e5067f3b0a63cb","run_log_self_hash":"sha256:1894a4ee1044df711b774793cb98564c6c00715e5f99de66db52dcd67efb6558","score_self_hash":"sha256:32960b832d30bcf2b0534c882b9493836702fb230513cfcad9447baa94511525","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:ec7409c9e698b018a455304cd527a3b538f199ba262ee6a13d5aa10ef94cf17a","completion":{"kind":"diverged_at","step":23},"negative_self_hash":null,"run_log_self_hash":"sha256:85ed08dd1c5847d83f992830846950c45e94469609ad8eee3f82a3af6de8296a","score_self_hash":null,"seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":null,"completion":{"kind":"not_reached"},"negative_self_hash":null,"run_log_self_hash":null,"score_self_hash":null,"seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":null,"completion":{"kind":"not_reached"},"negative_self_hash":null,"run_log_self_hash":null,"score_self_hash":null,"seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":null,"completion":{"kind":"not_reached"},"negative_self_hash":null,"run_log_self_hash":null,"score_self_hash":null,"seed":4}],"predictions_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","predictions_section_hash":"sha256:9668ca13dc6d85414e8f4d9efe06eee9f7a509a034f6ac607011ab0029124f1a","report_self_hash":"sha256:bd1561a07005e5c615bf6a96194f2200b5a904fb7d9768a46dcf45f0c2a54df0","rfc_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","s1_outcome":"Fail-substrate","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

Fixture predictions are pre-registered for the S1 E2E harness.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 1.810000 | 2.200000 | true |
| 1 | DivergedAt(23) | NA | NA | NA |
| 2 | NotReached | NA | NA | NA |
| 3 | NotReached | NA | NA | NA |
| 4 | NotReached | NA | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Refuted | seed 1 diverged at step 23 with observed=zero_grad |
| H2 | NotEvaluatedDueToPriorGate(H1 zero-gradient hook stopped scoring) | fixture val_bpc 1.810000 was evaluated against baseline 2.300000 |
| H3 | NotEvaluatedDueToPriorGate(H1 zero-gradient hook stopped negative testing) | fixture shuffle delta 2.200000 was evaluated against the sensitivity threshold |
| H4 | NotEvaluatedDueToPriorGate(H1 zero-gradient hook stopped ablation) | phase_a_eq_ablation=true |
| H5 | Confirmed | all metric oracle fixture checks passed |

## Falsification analysis

fail_substrate_zero_grad fixture exercised the existing S1 outcome/report composition path.

## Surprises

None.

## Decision

`Investigate(burn-or-autodiff)`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `scripts/s1_e2e.sh --scenario fail_substrate_zero_grad --fixture tiny`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f val_sha=sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4
