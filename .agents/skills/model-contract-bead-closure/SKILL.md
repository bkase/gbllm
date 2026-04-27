---
name: model-contract-bead-closure
description: Use when implementing, reviewing, or closing gbllm model topology or scalar gbf-model semantics beads, especially config validation, topology choices, parameter sharing, and non-Burn model behavior.
---

# Model Contract Bead Closure

Use this for `gbf-model` topology/config beads and scalar model-semantics beads that do not primarily belong to QAT, sequence-state, fixtures, or artifact export.

## Topology Beads

- State in closure whether the bead owns only shape/topology or executable forward behavior.
- Do not derive `Deserialize` for constructor-validated model config types unless deserialization goes through the same validation boundary.
- Prefer enum-backed topology choices over bool flags when invalid combinations must be unrepresentable.
- Test both dense and routed paths directly when a topology switch supports both.

## Scalar Semantics Beads

- For scalar `gbf-model` semantics that do not own Burn, export, artifact, or budget paths, name those unsupported boundaries in closure and do not claim them complete.
- When claiming parameter sharing or parameter-count reduction, add an owned-layer alias/count test.
- If export, artifact, or budget sharing is not implemented, name the follow-up owner before closing.
- Keep enum or named constructors as the primary API for mode choices; bools should be derived queries or edge-adapter inputs.

## Closure Evidence

- Closure claims must be guarded by behavior tests, not only constructor smoke tests.
- If a bead exposes public config accepted today but executable behavior is deferred, reject that config at the executable boundary or name the owner bead in the closure.
