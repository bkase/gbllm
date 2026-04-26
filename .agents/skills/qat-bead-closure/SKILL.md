---
name: qat-bead-closure
description: Use when implementing, reviewing, or closing gbllm QAT beads; enforces artifact-contract, differentiable Burn path, proof-test, acceptance-criterion movement, claim-to-gate, and no-future-variant checks.
---

# QAT Bead Closure

Use this before marking any QAT bead complete, and when addressing review feedback on QAT training, fake quantization, export, or artifact-lowering work.

## Required Questions

Every QAT bead closure comment must answer these explicitly:

- What is the artifact contract?
- Where is the differentiable Burn path?
- Which tests prove it?
- Is any acceptance criterion intentionally moved?

If the artifact contract is not implemented yet, do not claim artifact agreement. State the narrower pre-export contract and create or follow the bead that owns the artifact contract.

## Claim-To-Gate Matrix

Every closure claim must name the exact test, integration test, or CI command that protects it. Use this template in bead closure comments:

```markdown
| Closure claim | Guarding test or command | Feature gate | Notes or deviation |
| --- | --- | --- | --- |
| <specific behavior now guaranteed> | `<exact cargo test/CI command>` | `<none/all-features/feature name>` | <why this proves the claim> |
```

Acceptable gates include focused tests for development and the final all-features gate:

```bash
cargo test -p gbf-model -- <module_or_test_name>
cargo test -p gbf-train --features burn-adapter -- <module_or_test_name>
cargo test --workspace --all-features
```

If no exact gate exists, add one before closing the bead or leave the bead open.

## No Future Variant Acceptance

Do not accept enum variants, plan variants, strategies, or schedules that imply future behavior until the implementation and tests exist. Examples include annealing modes, learned thresholds, future artifact encodings, or unsupported lowering plans.

When a future variant is needed but not implemented:

- Reject it in validators or constructors today.
- Create or follow a bead that owns the variant.
- Add tests proving the current rejection.
- Do not close the current bead by saying future work will handle behavior that current public APIs already accept.

## Acceptance Movement

If an acceptance criterion is intentionally moved, renamed, or satisfied in a different crate than the bead text says, the closure comment must state:

- The original acceptance criterion.
- The new location or command.
- Why the move is semantically equivalent or why the bead scope changed.
- The gate that proves the moved criterion.
