---
{"baseline_self_hash":"sha256:5ac855077d78dd1e69edec76d5cbd92b8ad795e3cb11d22b73f2656a50b8d034","decision":{"kind":"Halt","reason":"audit-split-and-bpc"},"first_result_commit":"cccccccccccccccccccccccccccccccccccccccc","generated_at":"2026-05-09T12:00:00Z","per_seed_artifacts":[{"ablation_self_hash":"sha256:1c062aa3cfdf233a3b5e403f6b9befa7657c2cad797058f4d02ea0a8361989d8","checkpoint_self_hash":"sha256:0e942c2028d311e532e676da9b246a746c0ea7df9bfdb94e2e8a4f5db2b2abf0","completion":{"kind":"completed"},"negative_self_hash":"sha256:0c6185dc7acd19a4a328dcb0c83de3ca90194c5dedd82b56bc2552ba8b247279","run_log_self_hash":"sha256:1894a4ee1044df711b774793cb98564c6c00715e5f99de66db52dcd67efb6558","score_self_hash":"sha256:727d883c2b18af60cf8aab9f722149e2e67224b4420febbc299d75e981c7f080","seed":0},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:186755da5bc7a100904d70b81f1dbdae81ddf2384e761e5dc1c2ad81253f1b74","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:33235669454d09fd872cd2aed81da592317b13b3c594b85e3f8a6bc9d625f15a","score_self_hash":"sha256:c5baec023e03cd9e1e0ea6fe375d92d8201049de9a340ab494a546cf39d19a03","seed":1},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:4bef3d9c01785373d2d48ee14daf6de2a68d4990dcbbebb0130964a574e5c0cd","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:63c3ce6d912ffe41dffaabce98a01b460eae0149dc5dc6ab6dc65dfb74641a80","score_self_hash":"sha256:749568ad631db0bb733c2315c78832cd6a02df30acdb2de69b5f9da4106f05b6","seed":2},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:20efd2ea949acc1f22307016a0b9cc6ca823db2ccf712928614aa8a22ab8d17a","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:402fafe58af9d87ca2cb75da301bc4aa391915b784fa563251d64adb260bd35f","score_self_hash":"sha256:a13db88a0d6204c0e04cd5f92370f5ecd12ffaf1bc6dcf0326931f022a9f3e9e","seed":3},{"ablation_self_hash":null,"checkpoint_self_hash":"sha256:3f037ab1b6e79eda5ddb0a5bb6759aae0c41a8e3cf9fba05c6c6f8ccd726879d","completion":{"kind":"completed"},"negative_self_hash":null,"run_log_self_hash":"sha256:b50d70c9a91adbc83d2fc316bbd99269d67f5a05aa0d392edf8af71a8cb6f1d3","score_self_hash":"sha256:d8fc901535c85375ea712a06547576fb50edf4b09a75e5494c037ebcabd451b5","seed":4}],"predictions_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","predictions_section_hash":"sha256:9668ca13dc6d85414e8f4d9efe06eee9f7a509a034f6ac607011ab0029124f1a","report_self_hash":"sha256:1f1d2dfb8567eb96070c59f921381d08686fd5ec85fc869538d9574e396d4890","rfc_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","s1_outcome":"Fail-suspicious","schema":"s1_report.v1"}
---
# S1 Report

## Pre-registered predictions

Fixture predictions are pre-registered for the S1 E2E harness.

## Observed

| seed | completion | val_bpc | neg_test_delta | ablation_eq |
| --- | --- | --- | --- | --- |
| 0 | Completed | 0.400000 | 2.100000 | true |
| 1 | Completed | 0.410000 | NA | NA |
| 2 | Completed | 0.420000 | NA | NA |
| 3 | Completed | 0.430000 | NA | NA |
| 4 | Completed | 0.440000 | NA | NA |

## Hypothesis verdicts

| hypothesis | status | observation |
| --- | --- | --- |
| H1 | Confirmed | all fixture seeds reached the expected substrate state |
| H2 | Confirmed | fixture val_bpc 0.400000 was evaluated against baseline 2.300000 |
| H3 | Confirmed | fixture shuffle delta 2.100000 was evaluated against the sensitivity threshold |
| H4 | Confirmed | phase_a_eq_ablation=true |
| H5 | Confirmed | all metric oracle fixture checks passed |

## Falsification analysis

fail_suspicious_low_bpc fixture exercised the existing S1 outcome/report composition path.

## Surprises

Suspicious-low-bpc sentinel fired in the synthetic fixture.

## Decision

`Halt(audit-split-and-bpc)`. Decision follows RFC section 8 dispatch.

## Reproducibility statement

- command: `scripts/s1_e2e.sh --scenario fail_suspicious_low_bpc --fixture tiny`
- pass_version: `0.1.0`
- manifests: train_sha=sha256:b9373820fc2c6959bfdeb648b732f82f6781f1aef2a4f2db08ed817650e5e37f val_sha=sha256:30a987e41df83b5d02817d662760e59e27788f1235a17737ad048637a6deaaf4
