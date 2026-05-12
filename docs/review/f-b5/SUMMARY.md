# F-B5 Review Packet Summary

This packet covers the `GbInferIR` review surface for RFC F-B3/F-B5 canonical IRs. It is scoped to F-B5 Stage 3 construction, typed IIR rejection coverage, op signatures, reduction-site binding, and the fixture-level BitExact closure gate.

The packet is intentionally fixture-sized. It does not include production model outputs.

## Contents

| Artifact | Purpose |
| --- | --- |
| `golden/<fixture>/infer_ir.json` | Product-bearing canonical Stage 3 report emitted by the Stage 3 driver. |
| `golden/<fixture>/hashes.toml` | Pinned `infer_ir_self_hash`, canonical bytes hash, `topological_order_hash`, report self hash, and anchor count. |
| `golden/driver_evidence.toml` | Stage 3 driver-backed evidence for product-bearing report emission, fixture export, and cache audit rewrap. |
| `golden/<fixture>/anchor_ids.toml` | Per-fixture `NodeAnchorMap` sidecar keyed by node id and op tag. |
| `reject-class-table.md` | All 36 IIR reject classes, each mapped to a fixture and typed diagnostic. |
| `op-signature-table.md` | Closed 13-variant `InferOp` signature table with value kinds, effect kinds, and reduction-site bearing. |
| `reduction-site-join.md` | Stage 2 <-> Stage 3 reduction-site key alignment and canonical id patterns. |
| `bit_exact_equivalence.toml` | Fixture-level BitExact equivalence golden and feature-gated verification command. |
| `scripts/review/f-b5/regen.sh` | Regenerates the derived golden files from `fixtures/infer_ir`. |
| `scripts/review/f-b5/verify.sh` | Verifies generated goldens and table coverage. Set `GBF_REVIEW_F_B5_RUN_CARGO=1` to also run the Stage 3 e2e driver gate and focused S3 cargo gates when the shared worktree compiles. |

## Passing Fixtures

| Fixture | Topology | Self hash | Topological order hash | BitExact status |
| --- | --- | --- | --- | --- |
| `dense_toy0` | Dense | `sha256:cf7f891c7f2f25baabf282cb587a72f5a5166a38439163094ea50c06ee1887c3` | `sha256:077b5c7a5cd377f70bbf2bf683a26377d876389818127e646400c86ae01cc541` | Verified by `semantic_equivalence_check` fixture gate. |
| `routed_basic` | Routed | `sha256:fa6437611f341bd4b59dd6e7a1c28c33bbde6a856fc0cd80c68bf0bca2f1d68e` | `sha256:89a15db53783fb722058a97a342dfb27d9017976812a2b716426578a1df5cf8b` | Verified by `semantic_equivalence_check` fixture gate. |
| `mixed_topology` | Mixed | `sha256:e35d886f4d93aee31b7c458cf8c344a6bd6bf3064ea1794b3007cd961cffd27d` | `sha256:7446809c6dc45d3cd36417bd1811c37bb44b76bc91404f9b618c3464aab955e4` | Verified by `semantic_equivalence_check` fixture gate. |

## IIR Class Outputs

| IIRClass | Binding output | Packet evidence |
| --- | --- | --- |
| 1 IdentityBinding | `InferIrIdentity` binds QG, policy, static-budget, runtime-mode, and determinism identity. | S3 focused gate plus fixture self-hash goldens. |
| 2 TokenInputBinding | Single v1 token input and allowed ingress modes. | S3 focused gate and reject rows for ingress ambiguity and token value mismatch. |
| 3 ValueAllocation | Stable value ids for token, activations, routing, experts, logits, and decoded token. | S3 focused gate and op-signature table value-kind columns. |
| 4 EffectAllocation | Edge-token effect chains, decode RNG only when required, no v1 fault boundary emission. | S3 focused gate and reject rows for effect chain, RNG, and fault boundary. |
| 5 NodeBuilding | Dense/routed/mixed node construction before canonical sort. | Passing fixtures and op-signature table. |
| 6 ReductionSiteBinding | `GbNode.reduction_site` on router, expert, qualifying norm, and classify nodes only. | `reduction-site-join.md` and `InferIrReductionSiteMissing` reject row. |
| 7 ProvenanceBinding | Total node/value/effect provenance maps. | S3 focused gate and semantic-anchor reject coverage. |
| 8 AnchorBinding | `NodeAnchorMap` from domain-hashed semantic anchors. | `golden/<fixture>/infer_ir.json` embeds the product anchors and `golden/<fixture>/anchor_ids.toml` pins each node anchor id. |
| 9 CanonicalSort | Canonical ordering and post-sort `NodeId` assignment. | Topological-order hash goldens and S3 idempotence tests. |
| 10 SelfConsistency | SC-1..SC-18 cross-class checks. | 36 reject fixtures and focused S3 gate. |
| 11 SemanticEquivalenceCheck | Fixture-only BitExact equivalence against canonical reference semantics. | `bit_exact_equivalence.toml` and feature-enabled cargo gate. |

## Provenance Wire Shape

RFC snippets such as `ValueProducerRef::Node(NodeId)` and
`SemanticAnchor(Hash256)` are Rust-level shorthand for the closed identity
domain, not the `infer_ir.v1` JSON encoding. The public review packet uses
serde tagged objects for provenance refs and an object wrapper for anchors:
`{"kind":"Node","node":0}`, `{"kind":"External","token_input":0}`, and
`{"anchor_id":"sha256:..."}`. The fixture goldens pin those field names, and
the focused S3 test `provenance_refs_public_json_shape_uses_tagged_objects`
asserts representative tuple-like RFC cases directly.

## Proof Obligations

| Obligation | Packet evidence |
| --- | --- |
| O4 IIR rejection completeness | `reject-class-table.md` has 36 rows and `verify.sh` cross-checks every `fixtures/infer_ir/reject/*/expected.toml` diagnostic. |
| O10 Stable topological order | `golden/*/hashes.toml` pins `topological_order_hash` for dense, routed, and mixed fixtures. |
| O11 FixtureSemanticEquivalence | `bit_exact_equivalence.toml` records the feature-enabled BitExact gate and two sample trace hashes per fixture. |
| O13 Cache soundness | `golden/driver_evidence.toml` names the `run_stage3_cache_hit_replays_with_audit_rewrap` driver gate. `verify.sh` runs it with `GBF_REVIEW_F_B5_RUN_CARGO=1`. |
| O15 F-C2 readiness | `op-signature-table.md` and `reduction-site-join.md` expose the op and reduction-site contracts downstream stages consume. |

## Review Routing

Recommended persona routing: P5 Proof-of-Work Detective and P6 RFC Scope Sentinel always; P3 AI Researcher for canonical reference semantics; P4 QA for fixture and reject coverage; P7 Numerical and Determinism for BitExact; P9 Performance for reduction-site alignment.

## Export Path

`scripts/review/f-b5/regen.sh` runs the deterministic Stage 3 export test with `semantic_equivalence_check` enabled. The export test calls `run_stage3` for `dense_toy0`, `routed_basic`, and `mixed_topology`, writes product-bearing `infer_ir.json` reports into the packet, and derives the hash, anchor, and BitExact sidecars from those reports.

## Local Verification

```bash
./scripts/review/f-b5/regen.sh
./scripts/review/f-b5/verify.sh
```

Optional cargo-backed verification:

```bash
GBF_REVIEW_F_B5_RUN_CARGO=1 ./scripts/review/f-b5/verify.sh
```
