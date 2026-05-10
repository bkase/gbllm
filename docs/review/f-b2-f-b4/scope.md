# Scope

This F-B2 packet covers Stage 0 `ArtifactValidationAndUpgrade` and Stage 0.5 `ResolvedCompilePolicy` for RFC F-B2-F-B4. The chunk makes imported artifacts fail closed before compile policy is resolved, emits durable `artifact_validation.v1` reports for Stage 0 pass/fail, resolves static compile policy with explicit per-knob provenance, and emits `policy_resolution.v1` reports for Stage 0.5 pass/fail.

This packet covers the F-B2/F-B4 chunk review surface: Stage 0 artifact validation, Stage 0.5 policy resolution, and Stage 2 static budget report goldens with shared regeneration and verification scripts.

The three chunk reports answer different review questions:

| Report | Stage | Question |
| --- | --- | --- |
| `artifact_validation.json` | Stage 0 | Is the imported artifact current, hash-consistent, losslessly adaptable if needed, and admissible for this compile request? |
| `policy_resolution.json` | Stage 0.5 | Which compile policy was resolved, what provenance produced every compile knob, and did any policy diagnostic stop the pipeline? |
| `static_budget.json` | Stage 2 | Included in this packet; it answers whether the resolved policy and QuantGraph budget view fit the runtime chrome budget. |

F-B2 is the feature bead `bd-2fj` under Epic B (`bd-2bw`). F-B4 is the sibling feature bead `bd-2ps`; its packet completion and static-budget artifacts are deliberately left to `bd-3fug`.

Deferrals follow RFC §4: no `RepairProposal(_)` policy provenance until F-B16, no lossy/on-disk migration until `gbf-migrate`/F-A6b, no real F-B3 `QuantGraphBudgetSource` implementation here, and no F-B14 cycle envelope in these reports.
