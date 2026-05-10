---
{"baseline_self_hash":"sha256:5ac855077d78dd1e69edec76d5cbd92b8ad795e3cb11d22b73f2656a50b8d034","decision":{"kind":"ProceedToS2"},"first_result_commit":"cccccccccccccccccccccccccccccccccccccccc","generated_at":"2026-05-09T12:00:00Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1c062aa3cfdf233a3b5e403f6b9befa7657c2cad797058f4d02ea0a8361989d8","checkpoint_self_hash":"sha256:0e942c2028d311e532e676da9b246a746c0ea7df9bfdb94e2e8a4f5db2b2abf0","completion":{"kind":"completed"},"negative_self_hash":"sha256:27bc0b9ac1c8672e929d0a66630c8ee2a0396feb7c19dd36c21e6b84fd3f9339","run_log_self_hash":"sha256:1894a4ee1044df711b774793cb98564c6c00715e5f99de66db52dcd67efb6558","score_self_hash":"sha256:6527aa38d1b00b0def6f074cceb94e2370a6c710b40c12a6a5139857187811b4","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:186755da5bc7a100904d70b81f1dbdae81ddf2384e761e5dc1c2ad81253f1b74","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:33235669454d09fd872cd2aed81da592317b13b3c594b85e3f8a6bc9d625f15a","score_self_hash":"sha256:7824d959e88e404f0e50cacf9fc91277531b9d5c42156effc2b2f6bd7215d44f","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:4bef3d9c01785373d2d48ee14daf6de2a68d4990dcbbebb0130964a574e5c0cd","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:63c3ce6d912ffe41dffaabce98a01b460eae0149dc5dc6ab6dc65dfb74641a80","score_self_hash":"sha256:d58933ea164b051e5938430f760f30bddedcf6c35ee869e715563ea9206a8471","seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:20efd2ea949acc1f22307016a0b9cc6ca823db2ccf712928614aa8a22ab8d17a","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:402fafe58af9d87ca2cb75da301bc4aa391915b784fa563251d64adb260bd35f","score_self_hash":"sha256:3e75f779d8e1d30681020df815437f34b8a9911f311538b11928b7438f9d68db","seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:3f037ab1b6e79eda5ddb0a5bb6759aae0c41a8e3cf9fba05c6c6f8ccd726879d","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:b50d70c9a91adbc83d2fc316bbd99269d67f5a05aa0d392edf8af71a8cb6f1d3","score_self_hash":"sha256:7bda982a8ca58968d400ebdfb255735bdbab3b05cfa3e36956a14484edd799e7","seed":4}],"predictions_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","predictions_section_hash":"sha256:9668ca13dc6d85414e8f4d9efe06eee9f7a509a034f6ac607011ab0029124f1a","report_self_hash":"sha256:599f99333681a02e096535f961be34c2977272095f6a341c2356c980ed2baaee","rfc_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","s1_outcome":"Pass-clean","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

Fixture predictions are pre-registered for the S1 E2E harness.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 1.700000 | 2.500000 | true |
| 1 | Completed | 1.710000 | NA | NA |
| 2 | Completed | 1.720000 | NA | NA |
| 3 | Completed | 1.730000 | NA | NA |
| 4 | Completed | 1.740000 | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Confirmed | all fixture seeds reached the expected substrate state |
| H2 | Confirmed | fixture val_bpc 1.700000 was evaluated against baseline 2.300000 |
| H3 | Confirmed | fixture shuffle delta 2.500000 was evaluated against the sensitivity threshold |
| H4 | Confirmed | phase_a_eq_ablation=true |
| H5 | Confirmed | all metric oracle fixture checks passed |

## Falsification analysis

pass_clean fixture exercised the existing S1 outcome/report composition path.

## Surprises

None.

## Decision

`ProceedToS2`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `scripts/s1_e2e.sh --scenario pass_clean --fixture tiny`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f val_sha=sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4
