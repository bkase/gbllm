---
name: sequence-state-bead-closure
description: Use when implementing, reviewing, or closing gbllm sequence-state beads, including LinearState, BoundedKv, SequenceBlock traits, exported sequence facts, byte-backed state, and recurrent state tests.
---

# Sequence-State Bead Closure

Use this for sequence semantics, sequence blocks, state layout, exported state facts, and byte-backed recurrent/cache behavior.

## Contract Ownership

- Keep one authoritative sequence semantics contract. If crate dependencies require durable schema in `gbf-artifact`, re-export it from `gbf-model` and state that ownership movement explicitly in bead closure.
- Export paths must consume `SequenceExportFacts` derived from model topology or a sequence block, not a free-standing enum supplied at finish time.
- Add a test that `ExportVisitor` carries sequence facts into both `ArtifactCore` and `ExportFacts` when that path is owned.
- A sequence dispatch trait must use project-native activation/state types with shape and finiteness validation. Do not satisfy acceptance with unconstrained associated `Input`/`Output`/`State` types.
- Policy/profile selector enums such as `SequenceSemanticsRef` are configuration references only until compile profiles consume them. Avoid derived ordering unless ordering has executable policy meaning.

## Executable State Rules

- Define batch/state semantics before closure. If `SequenceState` is one shared buffer, reject `batch > 1` and test the rejection.
- Do not put artifact path naming or fabricated tensor handles in runtime block config. Sequence parameter handles must come from an `ExportVisitor` path that emits matching `ArtifactCore` tensors.
- Fixed recurrence/update behavior needs a literal value-level oracle test over at least two tokens.
- If the update law is a placeholder or research variant, narrow the closure and avoid claiming numeric semantics beyond scaffolding.
- Keep scalar sequence kernels, Burn/autodiff gradient paths, `TrainPhaseSpec` hardness scheduling, and shadow-compile A/B fixture adoption as separate claims with separate gates or moved-to beads.

## Byte-Backed State

- When a sequence block imposes an executable byte-record layout, align model topology validation and tiny fixtures with that layout.
- Durable artifact schema may stay broader, but model config must reject layouts the executable block cannot instantiate.
- Byte-backed sequence state must validate canonical persisted form before mutation.
- For cache-like records, test validity flags, contiguous live records, empty-record zeroing, sliding/truncation behavior, and failed-forward atomicity.
- If a v1 sequence block uses a simplified mechanism such as tied key/value payloads or record metadata inside `*_bytes_per_token`, name that explicitly in docs, tests, and closure.
