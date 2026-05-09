# Scope

This F-B2 packet covers Stage 0 `ArtifactValidationAndUpgrade` and Stage 0.5 `ResolvedCompilePolicy` for RFC F-B2-F-B4. The chunk makes imported artifacts fail closed before compile policy is resolved, emits durable `artifact_validation.v1` reports for Stage 0 pass/fail, resolves static compile policy with explicit per-knob provenance, and emits `policy_resolution.v1` reports for Stage 0.5 pass/fail.

This bead intentionally does not deliver F-B4 `static_budget.v1` substance, the final shared `regen.sh`/`verify-packet.sh` behavior, or Stage 2 static-budget goldens. T-B4.13 (`bd-3fug`) owns that final sub-bundle and extends the scripts added here from F-B2 scaffolding to full chunk verification.

The three chunk reports answer different review questions:

| Report | Stage | Question |
| --- | --- | --- |
| `artifact_validation.json` | Stage 0 | Is the imported artifact current, hash-consistent, losslessly adaptable if needed, and admissible for this compile request? |
| `policy_resolution.json` | Stage 0.5 | Which compile policy was resolved, what provenance produced every compile knob, and did any policy diagnostic stop the pipeline? |
| `static_budget.json` | Stage 2 | Deferred to T-B4.13 for this packet; it answers whether the resolved policy and QuantGraph budget view fit the runtime chrome budget. |

F-B2 is the feature bead `bd-2fj` under Epic B (`bd-2bw`). F-B4 is the sibling feature bead `bd-2ps`; its packet completion and static-budget artifacts are deliberately left to `bd-3fug`.

Deferrals follow RFC §4: no `RepairProposal(_)` policy provenance until F-B16, no lossy/on-disk migration until `gbf-migrate`/F-A6b, no real F-B3 `QuantGraphBudgetSource` implementation here, and no F-B14 cycle envelope in these reports.
