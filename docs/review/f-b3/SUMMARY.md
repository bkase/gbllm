# F-B3 Review Packet

This packet is the feature-level review bundle for F-B3 QuantGraph, generated
from the fixed `fixtures/quant_graph` outputs. It is intentionally narrow: it
proves the Stage 1 QuantGraph surface, fixture hashes, reject taxonomy, and
review routing. The chunk-level F-B3 + F-B5 closure remains owned by `bd-d06f`.

## Review Commands

```bash
scripts/review/f-b3/regen.sh
scripts/review/f-b3/verify.sh
```

`regen.sh` runs the Stage 1 fixture pipeline test and rebuilds
`docs/review/f-b3/golden` from `fixtures/quant_graph`. `verify.sh` regenerates
to a temporary directory, diffs the committed golden bundle, checks the
36-row reject table, and verifies the reduction-site scheme doc. Both scripts
are deterministic and do not use network-capable commands.

## Passing Fixture Goldens

| Fixture | Shape | QuantGraph self hash | Canonical bytes hash | Report self hash |
| --- | --- | --- | --- | --- |
| `dense_toy0` | Minimal single-layer dense FFN | `sha256:252ca7705c96cc33e093c00e6e8018e5c7254cf656e10c750b55e0173450883a` | `sha256:e2417abd44701796b7d18cea04a62264134367409c9f378f1ce69b6cb31fa039` | `sha256:748829a4408196e797691ab3ed28323ecc4e313872bfd12137daa2a80df204de` |
| `dense_toy1_tied` | Two-layer dense FFN with tied classify head | `sha256:23eb111087869ffa7f99d1df3dc466562e1fba72ecabd48f388ac5e7dc72d0ee` | `sha256:af58be544486442feea712435a53c8155641cfd4f58c387a32f2e90a4d947be3` | `sha256:af256312489abaf560610546c7027365524ff6746b3342503a7e966a9d21018b` |
| `dense_toy1_untied` | Two-layer dense FFN with untied classify head | `sha256:7f91759239f378aeb1c8ba96202346d64a84f1c2714089cb87763597904bacdc` | `sha256:0932dbad5c15ce61adc6c63d92c6c0488a1d0aeaebcc3ab420ab2f22d86d5b04` | `sha256:da324e641626844c81021117ad9f1ad6fac7c3a34a8463b02a0b0d4498b09e66` |
| `routed_basic_one` | Single routed layer with unit router gate weight | `sha256:d1a991e9cadcaafb1a63bb0110e39cea30cec76795919b37dc2802c08addfa2d` | `sha256:1daea51cb6401cc3ccc91b3589ee913b1c5854f9320da2f6c186054d4d9bdcd1` | `sha256:67bbeb072d7fccce330d4f2dedc491bde0a607a5daa8ce668a49b0b351b2473f` |
| `routed_basic_selected_score` | Single routed layer with selected-score router gate weight | `sha256:1d41dbe49fdf2d4ad71ad844a18e4d2dceba518549c74fa423ce879288b52da6` | `sha256:86b1553d106d026cf2cea4a98eb482b6ee91f93173e097273f76608e63efd03b` | `sha256:c9278ea4210d146fc08c7623e8a2ffe815e54315e64b801c54a9db7b4667b326` |
| `mixed_topology` | One dense layer and one routed layer | `sha256:6dc62d1d13266ae5b4b39f79004b7a300fa71b44d39788c61eb3c17a2794e4f5` | `sha256:35ad10ee1fca6662be7a4b1643ca713b6239bd1cc3fc4c767d41ecefa91d5e94` | `sha256:1adaa96a6e3fdffced8815c29dc2485e23d7d00598e2369982a5e60118d14abb` |

The routed pair is deliberate: `routed_basic_one` covers the constant-one
gate-weight path, and `routed_basic_selected_score` covers the selected router
score path without importing a real exported model.

## Binding Outputs

| Wave | Class | Bound output or contract | Fixture evidence |
| --- | --- | --- | --- |
| 1 | QG core, tensors, `ResolvedBlobIndex`, provenance | `QuantGraph`, `QuantTensorRef`, `ResolvedBlobIndex`, `TensorProvenanceMap`; storage metadata remains outside the product | Passing fixture hashes; QG-Reject-6, 8, 9, 17, 18, 19, 22 |
| 1 | Norm plans and layer norms | `NormPlanRecord`, `NormSite`, `LayerNorms`, deterministic `NormPlanId` assignment | QG-Reject-10, 26, 27, 28 |
| 1 | Routing, experts, FFN topology | `RoutingTable`, `RouterLayer`, `ExpertSection`, `ExpertWeightSlot`, `FfnPlan`, dense/routed/mixed topology tags | Routed and mixed fixtures; QG-Reject-3, 4, 5, 11, 25, 33, 34 |
| 1 | Classify and decode | `ClassifyHead`, `DecodeSpecRecord`, decode capability checks | Dense tied/untied fixtures; QG-Reject-12, 13, 14, 30 |
| 2 | IdentityBinding | Audit-parent hashes, semantic core hash, determinism class, model summary | QG-Reject-7, 20, 35 |
| 2 | SequenceSemanticsBinding | v1 identity sequence semantics and no state-slot rebind into QuantGraph | QG-Reject-15 |
| 2 | NormPlanIdPreBinding | Canonical pre-allocation of layer sequence, layer FFN, and final norm ids | Reduction-site scheme doc |
| 2 | TensorBinding | Tensor roles, formats, resolved blobs, aux refs, and payload sizes | QG-Reject-2, 6, 17, 18, 19, 29 |
| 2 | NormPlanBinding | One record per norm site, final norm required, unresolved references rejected | QG-Reject-10, 27, 28 |
| 2 | LayerNormsBinding | Every layer has sequence and FFN norm references | QG-Reject-26 |
| 2 | RoutingBinding | Routed layers require routing; dense layers reject routing; expert counts match | QG-Reject-3, 4, 5 |
| 2 | ExpertBinding | Expert sections cover required weights, gate presence matches FFN plan, coverage gaps/extras reject | QG-Reject-11, 25, 33, 34 |
| 2 | ResidualPlanBinding | Residual activation format and named clamp boundary are explicit | QG-Reject-32 |
| 2 | DecodeBinding | Decode spec is explicit or hash-bound and inside capabilities | QG-Reject-14, 30 |
| 2 | ClassifyHeadBinding | Tied/untied classify semantics and logit format are checked | QG-Reject-12, 13, 23, 24 |
| 2 | ProvenanceBinding | Tensor export image is total and injective | QG-Reject-8, 9 |
| 2 | CanonicalSort | Product vectors are sorted before hashing/reporting | Passing canonical bytes hashes |
| 2 | SelfConsistency | RFC Section 15.1 hard reject taxonomy, no short-circuit on accumulated diagnostics | `reject-class-table.md` and fixture pipeline test |
| 2 | RouterSemantics v1 | Only `Top1Hard` with gate weight `One` or `SelectedScore` and tie-break `LowestExpertId` | `routed_basic_one`, `routed_basic_selected_score`, QG-Reject-31, 36 |

## Proposal Diff

Section 8.3 storage-freeness is implemented as a product-shape rule plus an
upstream fact gate. QuantGraph has no residency, offset, path, mmap, or storage
metadata field; QG-Reject-22 proves forbidden storage metadata is a hard input
diagnostic rather than a serialized QuantGraph field.

Section 8.5 self-consistency now matches RFC Section 15.1 exactly. Earlier
temporary fixture rows such as `QuantGraphSequenceSemanticsUnsupportedV1`,
`QuantGraphSequenceSemanticsSidecarRebind`, `QuantGraphDecodeSpecUnboundDefault`,
and `QuantGraphMissingLayerNorms` remain useful internal diagnostics, but they
are no longer counted as the 36 closure reject classes.

Section 8.7 hashing/reporting is pinned by the six passing fixture hashes above:
`quant_graph_self_hash`, `quant_graph_canonical_bytes_hash`, and
`report_self_hash`. The review scripts rebuild those pins from the fixture
inputs and fail on any byte-level drift in the review bundle.

## Cross-Stage Handshake

F-B4 placeholder retirement evidence: Stage 1 exposes `QuantGraphBudgetSource`
and a `QuantGraphBudgetView`; dense shared-FFN placeholder output is explicitly
`None` in v1. Stage 1 cache keys include the full policy-resolution report hash
and resolved blob index hash, so cache hits replay the same report bytes without
an S3-style audit rewrap.

F-C2 readiness checklist: the packet supplies tiny dense, tied/untied classify,
routed `Top1Hard`, and mixed-topology fixtures, all generated without a real
exported model. F-C2 still owns op-for-op oracle and runtime equivalence; this
packet only proves the QuantGraph contract F-C2 will consume.

F-B5 input-shape guarantees: reduction sites are canonicalized as documented in
`reduction-site-id-scheme.md`; router, expert, norm, classify, residual, decode,
and provenance contracts have typed reject coverage before InferIR consumes the
QuantGraph.

## Persona Routing

Recommended routing for bd-2vx3: P5 Proof-of-Work Detective and P6 RFC Scope
Sentinel always run. Add P1 Architecture and Boundary Steward, P2 Code
Cleanliness / Idiomatic Rust, P4 QA / Test Engineer, and P8 Public Contract /
Schema Stability. Do not run ACPX from this bead implementation pass.
