# Reproducibility

F-B2 reports use the `gbf-report` canonical JSON contract: object keys are sorted, whitespace is omitted, integers are base-10, unknown fields are denied, and floating-point JSON is rejected for `artifact_validation.v1` and `policy_resolution.v1`.

Each report stores `report_self_hash`. The hash is computed with `report_self_hash` set to zero, canonicalized, and then hashed with the report domain separator:

```text
gbf:<crate>:<type>:<schema>:<version>\0<canonical-json>
```

For these reports the domains are `gbf:gbf-report:ArtifactValidationReport:artifact_validation.v1:1.0.0\0...` and `gbf:gbf-report:PolicyResolutionReport:policy_resolution.v1:1.0.0\0...`.

Changing `crate_feature_set_hash`, schema hashes, compile profile hashes, calibration hashes, or input artifact hashes changes the StageCache key material. Stage 0 and Stage 0.5 therefore isolate success products from byte-identical failure memos and from stale hand edits.

Regeneration path for this bead:

```bash
./scripts/review/f-b2-f-b4/regen.sh
./scripts/review/f-b2-f-b4/verify-packet.sh
git diff -- docs/review/f-b2-f-b4 scripts/review/f-b2-f-b4
```

`verify-packet.sh` validates sidecars, canonical bytes, semantic validators, `report_self_hash` round trips, byte-identical regeneration for the Stage 0/0.5/2 goldens, and the exposed Soft-diagnostic/nullability allow-list gates.
