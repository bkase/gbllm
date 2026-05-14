# F-B5 Reduction-Site Join

F-B5 does not independently invent range-analysis reduction sites. Stage 2 exposes `ReductionSiteProjection` records through the static-budget report, and Stage 3 binds reduction-bearing `GbNode`s by reconstructing the same canonical id strings.

## Bearing Predicate

The closed reduction-site-bearing set is:

| Infer op | Site required when | Canonical id pattern |
| --- | --- | --- |
| `RouterMatVec { layer }` | Always | `router.<layer>` |
| `ExpertMatVec { layer, expert, slot }` | Always | `expert.<layer>.<expert>.<slot>` where slot is `gate`, `up`, or `down`. |
| `Norm { plan }` | The norm plan is `TileRmsThenAffineClip`. | `norm.<plan>` |
| `Classify` | Always | `classify` |

Every other `InferOp` must carry `GbNode.reduction_site = None`.

## Stage 2 <-> Stage 3 Alignment

| Stage | Responsibility |
| --- | --- |
| F-B3 / QuantGraph budget view | Mints the opaque `ReductionSiteId` strings using the canonical patterns above. |
| F-B4 / StaticBudget | Records ordered, unique `ReductionSiteProjection` entries and validates the budget product. |
| F-B5 / GbInferIR | Reconstructs the canonical id for each reduction-bearing node and looks it up in the Stage 2 site set. |
| F-B7 / RangePlan | Consumes `GbNode.reduction_site` to choose per-site range strategy. |

Missing or duplicate Stage 2 site matches reject with `InferIrReductionSiteMissing`.

## Review Checks

Reviewers should confirm:

| Check | Evidence |
| --- | --- |
| The bearing set is closed. | `op-signature-table.md` marks only `RouterMatVec`, `ExpertMatVec`, qualifying `Norm`, and `Classify` as required. |
| F-B5 does not carry storage or range policy. | `GbNode.reduction_site` is only an opaque `ReductionSiteId`. |
| F-B5 remains F-B7-ready. | Every reduction-bearing node has a join id that F-B7 can map back through the static-budget product. |
| The id scheme is reviewable. | The four canonical patterns are documented here and verified by `scripts/review/f-b5/verify.sh`. |
