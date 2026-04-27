---
name: asm-bead-closure
description: Use when implementing, reviewing, or closing gbllm ASM/ISA beads in gbf-asm, including instruction modeling, sections, symbols, builders, layout, relaxation, effects, privileges, encoders, cycle models, and listings.
---

# ASM Bead Closure

Use this before closing `gbf-asm` beads and when addressing review feedback on assembly IR, ISA modeling, symbolic sections, builders, effect typing, layout, or lowering boundaries.

## Scope Discipline

- State whether the type is symbolic pre-layout IR, post-relaxation machine IR, or an adapter between them.
- Keep symbolic labels, relocations, pseudo-ops, branch relaxation, far-call thunking, final align padding, and byte lowering in their owner beads.
- Distinguish legal CPU encodings from canonical project encodings. If canonical forms reject legal CPU encodings, document and test that boundary.
- When a bead supersedes an older `planv0.md` sketch, state the mapping in code docs and closure.

## Type Boundaries

- Do not derive `Deserialize` for constructor-validated newtypes unless serde goes through the same validation boundary, for example `#[serde(try_from = ...)]`.
- Add at least one negative deserialization test for every private-field newtype whose constructor rejects values.
- Symbol names must be built from validated segments, not by joining raw caller strings and validating afterward. Add collision tests for dotted helper arguments.
- Symbol tables should allow address aliases unless the bead explicitly owns a primary-symbol-only table. Reverse lookups must return all names for an address.
- JSON-facing schema with maps must verify JSON serialization directly. Avoid non-string map keys in JSON output, or provide a stable representation.

## Builder And Section Rules

- Builder beads emit symbolic pre-layout section IR. They may record concrete `Instr`s, labels, alignment directives, pseudo-op intent, and raw escape hatches.
- Raw byte escape hatches must not be publicly constructible through multiple paths. Keep raw constructors crate-private or guarded, keep section mutation behind builder APIs, and add a raw-specific test or closure note.
- Do not model unknown-width symbolic items as zero-byte fixed size. Size APIs for alignments, pseudo-ops, relocations, or runtime-lowered markers should return `None`/unknown or be explicitly named as lower bounds.
- Pseudo-op tests must assert exact payloads, not only that pseudo-op calls do not panic. If a builder tracks lease-like state, test duplicate, unknown, released, and range-error paths.
- Provenance-scope helpers that temporarily mutate builder state must restore that state on normal return and caught panic.

## Effect And Privilege Rules

- Effect classifiers must not collapse stack-touching instructions into pure compute. `PUSH`/`POP`, calls, returns, `RST`, and two-byte stores need explicit stack/control or mixed-region tests.
- Full-address classifiers must test memory-map boundaries, including `$FF00..=$FFFF` high memory and two-byte writes that cross regions.
- Section privilege is a durable section invariant. Any API that changes privilege after emission must revalidate existing items, and tests must cover downgrade rejection.
- Raw bytes are opaque privileged effects unless a bead explicitly narrows the claim to data-only sections.
- Dynamic-address load/store effects must be named as reachability obligations; do not silently call them fixed-region effects without a proof from a later pass.

## Closure Evidence

- When citing filtered cargo-test commands, confirm the command actually ran tests in the current patch. A passing filter with `running 0 tests` is not evidence.
- Include a claim-to-gate matrix for non-trivial ASM beads, especially when code has symbolic, layout, and byte-lowering boundaries.
