# Claim To Gate

F-B2 rows copied from RFC §11:

| Claim | Gate | Review note |
| --- | --- | --- |
| Stage 0 rejects unsupported schema epoch | `gbf-codegen::f_b2_validate_rejects_schema_epoch_unsupported` | Fails closed before policy resolution. |
| Stage 0 rejects mismatched semantic core hash | `gbf-codegen::f_b2_validate_rejects_semantic_core_hash_mismatch` | Protects artifact identity. |
| Stage 0 rejects manifest invariant violations | `gbf-codegen::f_b2_validate_rejects_manifest_invariant_violated` | Keeps manifest structure load-bearing. |
| Stage 0 rejects packer round-trip failure | `gbf-codegen::f_b2_validate_rejects_lowering_round_trip_failure` | Proves lowering bytes are reversible. |
| Stage 0 rejects packer version mismatch | `gbf-codegen::f_b2_validate_rejects_lowering_packer_version_mismatch` | Prevents runtime/compiler packer drift. |
| Stage 0 rejects stale calibration | `gbf-codegen::f_b2_validate_rejects_calibration_stale` | Calibration must bind to the active target. |
| Stage 0 rejects missing calibration | `gbf-codegen::f_b2_validate_rejects_missing_calibration` | Non-Bootstrap flows cannot proceed without calibration. |
| Stage 0 accepts BootstrapCalibrationBundle when profile requires None | `gbf-codegen::f_b2_validate_accepts_bootstrap_calibration_when_profile_requires_none` | Bringup can use bootstrap calibration. |
| Stage 0 rejects BootstrapCalibrationBundle under Default profile | `gbf-codegen::f_b2_validate_rejects_bootstrap_calibration_under_default_profile` | Default requires transferred confidence. |
| Stage 0 rejects unsupported features | `gbf-codegen::f_b2_validate_rejects_compile_request_unsupported_feature` | Target/compiler feature checks are explicit. |
| Stage 0 records canonical input hashes | `gbf-codegen::f_b2_validate_records_canonical_input_hashes` | StageCache keys and reports share the same identity material. |
| Stage 0 returns all diagnostics in one pass | `gbf-codegen::f_b2_validate_returns_all_diagnostics_in_one_pass` | Reviewers get a full diagnostic set when safe. |
| Stage 0.5 rejects locked-knob overrides | `gbf-codegen::f_b2_resolve_policy_rejects_locked_knob_override` | Profile locks cannot be bypassed. |
| Stage 0.5 rejects out-of-bounds knob values | `gbf-codegen::f_b2_resolve_policy_rejects_out_of_bounds_value` | Overrides cannot escape the bound meet. |
| Stage 0.5 rejects unsatisfiable bound meet | `gbf-codegen::f_b2_resolve_policy_rejects_unsatisfiable_bound_meet` | Conflicting constraints stop the pipeline. |
| Stage 0.5 records per-knob provenance | `gbf-codegen::f_b2_resolve_policy_records_per_knob_provenance` | Every resolved knob is explainable. |
| Stage 0.5 forbids `RepairProposal` provenance | `gbf-codegen::f_b2_resolve_policy_no_repair_proposal_provenance_in_chunk` | F-B16 owns repair proposal adoption. |
| Stage 0.5 forbids authorized relaxation operations | `gbf-codegen::f_b2_resolve_policy_rejects_authorized_relaxation_operation` | Relaxation cannot sneak into F-B2. |
| Stage 0.5 has no profile relaxation field | `gbf-codegen::f_b2_resolve_policy_no_profile_relaxation_field` | The schema does not expose F-B16 behavior early. |
| `policy_resolution.v1` round-trips | `gbf-report::f_b2_policy_resolution_v1_self_hash_round_trip` | Self-hash and canonical JSON are stable. |
| `policy_resolution.v1` rejects RepairProposal provenance | `gbf-report::f_b2_policy_resolution_v1_rejects_repair_proposal_provenance` | Public schema enforces the F-B2 source allowlist. |
| StageCache keys are deterministic | `gbf-codegen::stage_cache_key_*_is_deterministic` | Stage 0/0.5 success and failure memo keys are content-addressed. |
| Failed passes do not enter cache | `gbf-codegen::stage_cache_failed_pass_does_not_enter_success_cache` | Failure memoization cannot masquerade as success. |
| Review packet regenerates cleanly | `./scripts/review/f-b2-f-b4/regen.sh` then clean diff | Regenerates Stage 0, Stage 0.5, and Stage 2 success/failure goldens plus sidecars. |
| Review packet verifier rejects staleness | `./scripts/review/f-b2-f-b4/verify-packet.sh` | Compares regenerated Stage 0/0.5/2 goldens/fixtures byte-for-byte and runs exposed report invariant gates. |
| Static-budget missing-runtime-budget failure is reviewable | `artifacts/static_budget.failure.golden.json` and `gbf-report::f_b2_f_b4_reports_reject_unlisted_null_fields` | The failure golden carries both the expected Hard diagnostic and `BudgetFailure::MissingRuntimeChromeBudget`. |

F-B4 rows from RFC §11 are out of scope for `bd-2uvs` and remain with `bd-3fug`.
