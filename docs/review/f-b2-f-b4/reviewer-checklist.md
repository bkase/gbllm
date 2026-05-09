# Reviewer Checklist

- [ ] Open `artifacts/policy_resolution.golden.json` and check every `compile_knobs.provenance[*].chain` is non-empty and no source has `kind = "RepairProposal"`.
- [ ] Check the `policy_resolution.golden.json` `compile_knobs` section carries populated `global`, `bounds`, `locks`, `overrides`, and path-level `provenance`.
- [ ] Diff `artifacts/policy_resolution.golden.json` against `artifacts/policy_resolution.failure.golden.json` and confirm the failure shape preserves the `artifact_identity`, `compile_request`, and `hint_consumption` sections while setting only `result = null` and adding the expected Hard diagnostic.
- [ ] Open `artifacts/policy_resolution.failure.golden.json` and confirm `outcome = "Failed"`, `result = null`, exactly one expected Hard diagnostic, and stable `report_self_hash`.
- [ ] Open `artifacts/artifact_validation.failure.golden.json` and confirm exactly one expected Hard diagnostic, `outcome = "Failed"`, and stable `report_self_hash`.
- [ ] Open `artifacts/static_budget.golden.json` and confirm the synthetic expert payload is assigned to a runtime chrome budget slot and `decision.fits = true`.
- [ ] Open `artifacts/static_budget.failure.golden.json` and confirm `outcome = "Failed"`, `runtime_chrome_budget = null`, one expected Hard diagnostic `BudgetMissingRuntimeChromeBudget`, and `decision.failures` includes `BudgetFailure::MissingRuntimeChromeBudget`.
- [ ] Confirm no F-B2/F-B4 golden contains a Soft diagnostic; Soft/nullability report checks are schema-level invariants, not fixture conventions.
- [ ] Confirm nullable fields match RFC §2.5: F-B2 `policy_resolution.v1` failure may set only `result = null`; Stage 0 failure may null only the listed identity/compatibility fields; Stage 2 missing-budget failure may null only the runtime chrome budget fields.
- [ ] Run `./scripts/review/f-b2-f-b4/verify-packet.sh` and confirm clean exit.
