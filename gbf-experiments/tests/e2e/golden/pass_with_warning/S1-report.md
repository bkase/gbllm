---
{"baseline_self_hash":"sha256:5ac855077d78dd1e69edec76d5cbd92b8ad795e3cb11d22b73f2656a50b8d034","decision":{"kind":"ProceedToS2-with-T12.5-prereq"},"first_result_commit":"cccccccccccccccccccccccccccccccccccccccc","generated_at":"2026-05-09T12:00:00Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1c062aa3cfdf233a3b5e403f6b9befa7657c2cad797058f4d02ea0a8361989d8","checkpoint_self_hash":"sha256:0e942c2028d311e532e676da9b246a746c0ea7df9bfdb94e2e8a4f5db2b2abf0","completion":{"kind":"completed"},"negative_self_hash":"sha256:073567ee0f8eb3692f75fcf6b50ac4afb91c8a0943e0b8f02079fc1cd8c869af","run_log_self_hash":"sha256:1894a4ee1044df711b774793cb98564c6c00715e5f99de66db52dcd67efb6558","score_self_hash":"sha256:4110f5b8244e965a3649f24f0aaa635dd8d3bf9029ef4a5d3cdde1765070769c","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:186755da5bc7a100904d70b81f1dbdae81ddf2384e761e5dc1c2ad81253f1b74","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:33235669454d09fd872cd2aed81da592317b13b3c594b85e3f8a6bc9d625f15a","score_self_hash":"sha256:e9167924df53ff850d77566646a5f9822ff68d005c8d2db1f45bc454f95f1e76","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:4bef3d9c01785373d2d48ee14daf6de2a68d4990dcbbebb0130964a574e5c0cd","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:63c3ce6d912ffe41dffaabce98a01b460eae0149dc5dc6ab6dc65dfb74641a80","score_self_hash":"sha256:7cd8463f3b5194e989a9e8f26a896e86e3b23f61e9872ff6f1c72883928e8544","seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:20efd2ea949acc1f22307016a0b9cc6ca823db2ccf712928614aa8a22ab8d17a","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:402fafe58af9d87ca2cb75da301bc4aa391915b784fa563251d64adb260bd35f","score_self_hash":"sha256:d743337aaed733787bfe917782283f442cb6ce19c4eb484aaca2de0ae1383fa2","seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:3f037ab1b6e79eda5ddb0a5bb6759aae0c41a8e3cf9fba05c6c6f8ccd726879d","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:b50d70c9a91adbc83d2fc316bbd99269d67f5a05aa0d392edf8af71a8cb6f1d3","score_self_hash":"sha256:5a27f7fa0fb03cd91f9e31a6908d49f85ac5f4f88cd5b57faa563c53c9303376","seed":4}],"predictions_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","predictions_section_hash":"sha256:9668ca13dc6d85414e8f4d9efe06eee9f7a509a034f6ac607011ab0029124f1a","report_self_hash":"sha256:7ce386b41e23b5333dec39a2174737d02d3742a5498d494ba07f92c84e794a40","rfc_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","s1_outcome":"Pass-with-warning","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

Fixture predictions are pre-registered for the S1 E2E harness.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 1.720000 | 0.250000 | true |
| 1 | Completed | 1.730000 | NA | NA |
| 2 | Completed | 1.740000 | NA | NA |
| 3 | Completed | 1.750000 | NA | NA |
| 4 | Completed | 1.760000 | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Confirmed | all fixture seeds reached the expected substrate state |
| H2 | Confirmed | fixture val_bpc 1.720000 was evaluated against baseline 2.300000 |
| H3 | Refuted | shuffle delta 0.250000 did not exceed the 2.0 bpc sensitivity threshold |
| H4 | Confirmed | phase_a_eq_ablation=true |
| H5 | Confirmed | all metric oracle fixture checks passed |

## Falsification analysis

pass_with_warning fixture exercised the existing S1 outcome/report composition path.

## Surprises

None.

## Decision

`ProceedToS2-with-T12.5-prereq`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `scripts/s1_e2e.sh --scenario pass_with_warning --fixture tiny`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f val_sha=sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4
