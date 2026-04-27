---
name: model-contract-bead-closure
description: Use when implementing, reviewing, or closing gbllm model topology or scalar gbf-model semantics beads, especially config validation, topology choices, parameter sharing, and non-Burn model behavior.
---

# Model Contract Bead Closure

Use this for `gbf-model` topology/config beads and scalar model-semantics beads that do not primarily belong to QAT, sequence-state, fixtures, or artifact export.

## Topology Beads

- State in closure whether the bead owns only shape/topology or executable forward behavior.
- Do not derive `Deserialize` for constructor-validated model config types unless deserialization goes through the same validation boundary.
- If the bead explicitly names TOML/serde config, either implement a validated parse path with a parse test or create/name the owner bead before closing.
- Prefer enum-backed topology choices over bool flags when invalid combinations must be unrepresentable.
- Public strategy/selection enums with payloads should use private-field payload structs or an equivalent constructor-only boundary; do not expose unchecked public fields for invalid ranges, empty lists, or zero strides.
- Test both dense and routed paths directly when a topology switch supports both.
- For block/depth selection config, test every advertised selector variant plus zero-depth, out-of-range, duplicate/empty, and default-on-small-depth behavior.
- For feature closures that aggregate topology primitives, either add and run one construction gate that ties topology, embeddings, expert MLP, and budget choices together, or close explicitly as model-core primitives and name downstream owners for artifact, preflight, and runtime boundaries.

## Scalar Semantics Beads

- For scalar `gbf-model` semantics that do not own Burn, export, artifact, or budget paths, name those unsupported boundaries in closure and do not claim them complete.
- When claiming parameter sharing or parameter-count reduction, add an owned-layer alias/count test.
- If export, artifact, or budget sharing is not implemented, name the follow-up owner before closing.
- If a config only represents a shared prototype, do not claim runtime/common-bank/export aliasing unless a bank/owner/count test proves it.
- Keep enum or named constructors as the primary API for mode choices; bools should be derived queries or edge-adapter inputs.
- When a fixed byte formula backs an executable model type, reject unsupported public tensors or modes at the model boundary unless the formula and tests account for them.

## Closure Evidence

- Closure claims must be guarded by behavior tests, not only constructor smoke tests.
- If a bead exposes public config accepted today but executable behavior is deferred, reject that config at the executable boundary or name the owner bead in the closure.
- If a model/test bead claims gradient flow or parameter sharing under Burn, prove it through a real Burn adapter path and `.backward()`/`.grad()` assertions. Manual gradient arithmetic is only an oracle after an executable gradient has been observed.
- For F10 model coverage beads, reuse the project tiny fixtures for concrete model/expert construction. Build custom topologies only where the selector matrix itself is the behavior under test.
- Treat structured config diagnostics and tracing logs as different surfaces. If the implementation owner returns a structured warning event, assert that event contract and state that logger adoption is not owned by the model test bead.
- For multi-path feature closures, include a support matrix that separates model core, Burn adapter, export/artifact, budget estimate, and preflight/compiler status; every moved or unsupported surface needs a named owner bead.
