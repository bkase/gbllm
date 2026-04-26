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

## Support Matrix

Every QAT bead closure must include a path support matrix. Use `supported`, `rejected`, `moved`, or `not applicable`; do not leave a path implicit.

```markdown
| Public behavior or variant | Scalar/model core | Burn training path | Export/artifact path | Guard |
| --- | --- | --- | --- | --- |
| <variant/API/claim> | <supported/rejected/moved> | <supported/rejected/moved> | <supported/rejected/moved> | `<exact test or bead id>` |
```

Examples:
- If `gbf-model` accepts an enum variant but `gbf-train` cannot train it yet, the Burn adapter must reject it and a test must prove the rejection.
- If export materialization is not in the current bead, mark export as `moved` and name the bead that owns it.

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

Path-specific rule: a public variant is only accepted on paths where it works today. For every public variant, strategy, or schedule, one of these must be true for each advertised path (`scalar/model`, `Burn training`, `export/artifact`):

- The path implements the behavior and has a guarding test.
- The path rejects the variant at its boundary and has a guarding rejection test.
- The closure matrix marks the path as moved and names the owning bead.

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

Use explicit bead ids in moved-acceptance text. The acceptance-owner harness checks that referenced bead ids exist and that the owning bead mentions the named concept.

## Bead Comment Workflow

For non-trivial closure comments or corrective review notes, write the markdown to a temporary file and use:

```bash
br comments add <id> --file <path>
br close <id> --reason "$(cat <path>)"
```

Do not inline markdown containing backticks in a shell `--message`; command substitution can corrupt the comment.

## Local Sensors

Before closing QAT beads, run:

```bash
python3 scripts/qat_harness_checks.py
cargo test -p gbf-test --test architecture
```

The harness checks enforce QAT closure shape, moved-acceptance ownership, and absence of test-only backend seams in QAT modules.
