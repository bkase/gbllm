# Architecture

F-B2 occupies the front of the RFC §6 compiler pipeline:

```text
ImportedArtifactView
  -> validate_artifact_and_request
  -> ValidationProduct / artifact_validation.v1
  -> resolve_policy
  -> ResolvedPolicyProduct / policy_resolution.v1
  -> QuantGraphBudgetSource
  -> budget (F-B4, deferred to bd-3fug for this packet)
```

The Stage 0 code path lives in `gbf-codegen/src/validate.rs`. It takes `ValidateInputs`, checks schema compatibility, manifest/semantic hashes, payload and aux sidecars, lowering pack/unpack, calibration, hints, workloads, golden vectors, and compile-request admissibility, then returns `ValidationProduct` with a private-constructor `ValidatedInputs` token. That private token is the type-system boundary that prevents later stages from accepting unvalidated inputs.

The Stage 0.5 code path lives in `gbf-codegen/src/policy.rs`. `resolve_policy` consumes `ValidationProduct`, builds ordered constraint frames, rejects locked or out-of-bounds knob changes, records `CompileKnobProvenanceEntry` paths, and returns `ResolvedPolicyProduct`. F-B2 explicitly rejects `PolicySource::RepairProposal(_)`; F-B16 owns that future source.

The report schemas and deterministic envelope logic live in `gbf-report/src/report_schemas/artifact_validation_v1.rs`, `gbf-report/src/report_schemas/policy_resolution_v1.rs`, and `gbf-report/src/canonical_json.rs`. The F-B2 review goldens are regenerated and verified by `gbf-report/src/bin/f_b2_review_artifacts.rs`.

Fixture locations used by the implemented F-B2 path:

| Fixture | Location | Use |
| --- | --- | --- |
| Compile profiles | `gbf-policy/fixtures/compile-profiles/*.profile.toml` | Bringup/Default/Trace/Recovery policy defaults and locks. |
| Bootstrap calibration | `fixtures/calibration/bootstrap-dmg-mbc5.calibration.json` | Bootstrap bundle for Bringup/DMG review flows. |
| Runtime chrome budget | `fixtures/runtime-chrome-budget/bringup-dmg-mbc5.chrome_budget.json` | F-B4 input, listed here because Bringup builds reference it; static-budget packet materialization is deferred to `bd-3fug`. |
| Review fixture TOML | `docs/review/f-b2-f-b4/artifacts/*.fixture.toml` | Human-readable summary of the generated F-B2 success/failure cases. |
