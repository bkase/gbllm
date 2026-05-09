---
{"baseline_self_hash":"sha256:5ac855077d78dd1e69edec76d5cbd92b8ad795e3cb11d22b73f2656a50b8d034","decision":{"kind":"Investigate","reason":"burn-or-autodiff"},"first_result_commit":"cccccccccccccccccccccccccccccccccccccccc","generated_at":"2026-05-09T12:00:00Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1c062aa3cfdf233a3b5e403f6b9befa7657c2cad797058f4d02ea0a8361989d8","checkpoint_self_hash":"sha256:0e942c2028d311e532e676da9b246a746c0ea7df9bfdb94e2e8a4f5db2b2abf0","completion":{"kind":"completed"},"negative_self_hash":"sha256:1bdd2c7256e5f30ba3fb8fe1116eab8b110a60284e14afdcb1a9a766cd83844e","run_log_self_hash":"sha256:1894a4ee1044df711b774793cb98564c6c00715e5f99de66db52dcd67efb6558","score_self_hash":"sha256:ac09abcb64ccc77b81370edcfdb4e909721deb51aaea0ffac5ba11fad557051f","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:186755da5bc7a100904d70b81f1dbdae81ddf2384e761e5dc1c2ad81253f1b74","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:33235669454d09fd872cd2aed81da592317b13b3c594b85e3f8a6bc9d625f15a","score_self_hash":"sha256:7f57eaf4b44720f506d47fc85cbbd94cf8394ac4b708546863ba8b088dd66f3a","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:171e64543a013cb5e8e558baaf177ab2a14cef2ec69a4c3242fa401f1942f5f9","completion":{"kind":"diverged_at","step":17},"negative_self_hash":null,"run_log_self_hash":"sha256:1f4fc7029e6d8bc8838261232b6f3e8679907b03a00848a1b18ef7251d2ab9e7","score_self_hash":null,"seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":null,"completion":{"kind":"not_reached"},"negative_self_hash":null,"run_log_self_hash":null,"score_self_hash":null,"seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":null,"completion":{"kind":"not_reached"},"negative_self_hash":null,"run_log_self_hash":null,"score_self_hash":null,"seed":4}],"predictions_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","predictions_section_hash":"sha256:9668ca13dc6d85414e8f4d9efe06eee9f7a509a034f6ac607011ab0029124f1a","report_self_hash":"sha256:b59c9f2894757393d71426300f650cab64aab402fc641f2bb5601aee6069bba0","rfc_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","s1_outcome":"Fail-substrate","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

Fixture predictions are pre-registered for the S1 E2E harness.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 1.800000 | 2.250000 | true |
| 1 | Completed | 1.810000 | NA | NA |
| 2 | DivergedAt(17) | NA | NA | NA |
| 3 | NotReached | NA | NA | NA |
| 4 | NotReached | NA | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Refuted | seed 2 diverged at step 17 with observed=non_finite_loss |
| H2 | NotEvaluatedDueToPriorGate(H1 diverged before scoring) | fixture val_bpc 1.800000 was evaluated against baseline 2.300000 |
| H3 | NotEvaluatedDueToPriorGate(H1 diverged before negative testing) | fixture shuffle delta 2.250000 was evaluated against the sensitivity threshold |
| H4 | NotEvaluatedDueToPriorGate(H1 diverged before ablation) | phase_a_eq_ablation=true |
| H5 | Confirmed | all metric oracle fixture checks passed |

## Falsification analysis

fail_substrate_nan fixture exercised the existing S1 outcome/report composition path.

## Surprises

None.

## Decision

`Investigate(burn-or-autodiff)`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `scripts/s1_e2e.sh --scenario fail_substrate_nan --fixture tiny`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f val_sha=sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4
