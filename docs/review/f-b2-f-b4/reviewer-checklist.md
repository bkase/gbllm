# Reviewer Checklist

- [ ] Open `artifacts/policy_resolution.golden.json` and check every `compile_knobs.provenance[*].chain` is non-empty and no source has `kind = "RepairProposal"`.
- [ ] Check the `policy_resolution.golden.json` `compile_knobs` section carries populated `global`, `bounds`, `locks`, `overrides`, and path-level `provenance`.
- [ ] Open `artifacts/policy_resolution.failure.golden.json` and confirm `outcome = "Failed"`, `result = null`, exactly one expected Hard diagnostic, and stable `report_self_hash`.
- [ ] Open `artifacts/artifact_validation.failure.golden.json` and confirm exactly one expected Hard diagnostic, `outcome = "Failed"`, and stable `report_self_hash`.
- [ ] Confirm neither F-B2 golden contains a Soft diagnostic; Soft/nullability report checks are schema-level invariants, not fixture conventions.
- [ ] Confirm nullable fields match RFC §2.5: F-B2 `policy_resolution.v1` failure may set only `result = null`; Stage 0 failure may null only the listed identity/compatibility fields.
- [ ] Run `./scripts/review/f-b2-f-b4/verify-packet.sh` and confirm clean exit.
- [ ] Note for T-B4.13: `static_budget.failure.golden.json` missing-budget must include both `BudgetMissingRuntimeChromeBudget` Hard diagnostic and `BudgetFailure::MissingRuntimeChromeBudget`; every static-budget failure golden must use the same expected Hard diagnostic wording.
