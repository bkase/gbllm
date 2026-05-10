---
{"baseline_self_hash":"sha256:5ac855077d78dd1e69edec76d5cbd92b8ad795e3cb11d22b73f2656a50b8d034","decision":{"kind":"Halt","reason":"measurement-broken"},"first_result_commit":"cccccccccccccccccccccccccccccccccccccccc","generated_at":"2026-05-09T12:00:00Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1c062aa3cfdf233a3b5e403f6b9befa7657c2cad797058f4d02ea0a8361989d8","checkpoint_self_hash":"sha256:0e942c2028d311e532e676da9b246a746c0ea7df9bfdb94e2e8a4f5db2b2abf0","completion":{"kind":"completed"},"negative_self_hash":"sha256:037156a55be6d7db6d40941bab02c8708512f4060ca8f28d128c33ca96666c5c","run_log_self_hash":"sha256:1894a4ee1044df711b774793cb98564c6c00715e5f99de66db52dcd67efb6558","score_self_hash":"sha256:93e9bc57e626fa4fe1c042c4b71284e6d170b003d18336f5b0a1ac5d7250e32b","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:186755da5bc7a100904d70b81f1dbdae81ddf2384e761e5dc1c2ad81253f1b74","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:33235669454d09fd872cd2aed81da592317b13b3c594b85e3f8a6bc9d625f15a","score_self_hash":"sha256:b284fa9a775c7d21202b13ad91487a99498cbbf608557bcbb736c64739b1b1c3","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:4bef3d9c01785373d2d48ee14daf6de2a68d4990dcbbebb0130964a574e5c0cd","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:63c3ce6d912ffe41dffaabce98a01b460eae0149dc5dc6ab6dc65dfb74641a80","score_self_hash":"sha256:355128884dc5ec50d8619ec11b8751ba2b5cc0d203199af3192152a24771fd75","seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:20efd2ea949acc1f22307016a0b9cc6ca823db2ccf712928614aa8a22ab8d17a","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:402fafe58af9d87ca2cb75da301bc4aa391915b784fa563251d64adb260bd35f","score_self_hash":"sha256:e18720b6fe33fe8cb7d365937faeb898b232194341e7856b7746a3bc1aa5fa52","seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:3f037ab1b6e79eda5ddb0a5bb6759aae0c41a8e3cf9fba05c6c6f8ccd726879d","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:b50d70c9a91adbc83d2fc316bbd99269d67f5a05aa0d392edf8af71a8cb6f1d3","score_self_hash":"sha256:e522c2b7f4efa506c143c3ffba891d300d0c320b8c6316b5215abb01124f1122","seed":4}],"predictions_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","predictions_section_hash":"sha256:9668ca13dc6d85414e8f4d9efe06eee9f7a509a034f6ac607011ab0029124f1a","report_self_hash":"sha256:f73228ccec703f85c211f35cb98659cba45063bb63a64785aa455b625e6217a2","rfc_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","s1_outcome":"Fail-metric","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

Fixture predictions are pre-registered for the S1 E2E harness.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 1.760000 | 2.050000 | true |
| 1 | Completed | 1.770000 | NA | NA |
| 2 | Completed | 1.780000 | NA | NA |
| 3 | Completed | 1.790000 | NA | NA |
| 4 | Completed | 1.800000 | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Confirmed | all fixture seeds reached the expected substrate state |
| H2 | Confirmed | fixture val_bpc 1.760000 was evaluated against baseline 2.300000 |
| H3 | Confirmed | fixture shuffle delta 2.050000 was evaluated against the sensitivity threshold |
| H4 | Confirmed | phase_a_eq_ablation=true |
| H5 | Refuted | O-metric-4 modulo-shuffle fixture returned false |

## Falsification analysis

fail_metric_modulo_shuffle fixture exercised the existing S1 outcome/report composition path.

## Surprises

None.

## Decision

`Halt(measurement-broken)`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `scripts/s1_e2e.sh --scenario fail_metric_modulo_shuffle --fixture tiny`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f val_sha=sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4
