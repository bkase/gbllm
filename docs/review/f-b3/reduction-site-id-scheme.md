# F-B3 Reduction-Site ID Scheme

F-B3 mints `ReductionSiteId` values for every reduction-bearing QuantGraph
projection consumed by F-B5. The IDs are strings with stable numeric components;
they do not include storage paths, tensor order, node ids, or allocation ids.

| Site kind | Canonical form | Example | Notes |
| --- | --- | --- | --- |
| Router matvec | `router.<layer>` | `router.2` | One site per routed layer in `RoutingTable.layers`. |
| Expert matvec | `expert.<layer>.<expert>.<slot>` | `expert.3.5.down` | Slot is one of `gate`, `up`, `down` in canonical expert slot order. |
| Norm | `norm.<norm_plan_id>` | `norm.7` | Uses the pre-bound `NormPlanId`, not vector position after sorting. |
| Classify | `classify` | `classify` | Single classify-head reduction site. |

## Emission Order

The projection is sorted by the site string before it is handed to budget and
InferIR consumers. The source enumeration is deterministic before that sort:
router layers come from `RoutingTable.layers`, expert sections come from
`ExpertSection` records with `gate`, `up`, `down` slot expansion, norm sites
come from `NormPlanRecord`, and classify is appended once.

## F-B5 Reconstruction Contract

F-B5 must reconstruct the same IDs from QuantGraph facts:

- Use `LayerId` and `ExpertId` numeric values directly.
- Use expert slot labels exactly: `gate`, `up`, `down`.
- Use the bound `NormPlanId` integer for norm sites.
- Use the literal classify id `classify`.
- Treat a missing reduction site on a reduction-bearing op as a hard F-B5
  diagnostic, not as a chance to mint a fresh id.

The pinned Stage 1 table test covers these examples: `router.2`,
`expert.3.5.gate`, `expert.3.5.up`, `expert.3.5.down`, `norm.7`, and
`classify`.
