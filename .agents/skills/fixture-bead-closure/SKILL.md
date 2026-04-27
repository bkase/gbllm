---
name: fixture-bead-closure
description: Use when implementing, reviewing, or closing gbllm fixture beads, especially tiny model/artifact/workload fixtures, deterministic helper data, and artifact assertion helpers.
---

# Fixture Bead Closure

Use this for tiny fixtures, deterministic fixture tensors, artifact/workload factories, and assertion helpers.

## Fixture Ownership

- Keep one source of truth for tiny fixtures.
- If a tiny model config names layers or paths, the model fixture should own the state that artifact/workload factories consume.
- Name placeholders explicitly when real policy, workload, manifest, compiler, or runtime contracts are still stubs.
- Create or cite a follow-up owner before closing if a fixture intentionally stops at placeholders.

## Artifact Assertions

- Scope artifact assertions to the artifact type that exists.
- `ArtifactCore` helpers must check core tensor/quant invariants, including tensor content-hash self-consistency.
- Manifest validation needs a real manifest contract and test; do not claim manifest correctness from an `ArtifactCore` helper.

## Closure Evidence

- Prefer deterministic fixture-repeat tests and literal expected IDs/paths over self-comparison.
- If factories share a tiny config, test that model, artifact, and workload factories stay consistent with that source.
