

<!-- br-agent-instructions-v1 -->

---

## Beads Workflow Integration

This project uses [beads_rust](https://github.com/Dicklesworthstone/beads_rust) (`br`/`bd`) for issue tracking. Issues are stored in `.beads/` and tracked in git.

### Essential Commands

```bash
# View ready issues (unblocked, not deferred)
br ready              # or: bd ready

# List and search
br list --status=open # All open issues
br show <id>          # Full issue details with dependencies
br search "keyword"   # Full-text search

# Create and update
br create --title="..." --type=task --priority=2
br update <id> --status=in_progress
br close <id> --reason="Completed"
br close <id1> <id2>  # Close multiple issues at once

# Sync with git
br sync --flush-only  # Export DB to JSONL
br sync --status      # Check sync status
```

### Workflow Pattern

1. **Start**: Run `br ready` to find actionable work
2. **Claim**: Use `br update <id> --status=in_progress`
3. **Work**: Implement the task
4. **Complete**: Use `br close <id>`
5. **Sync**: Always run `br sync --flush-only` at session end

### Key Concepts

- **Dependencies**: Issues can block other issues. `br ready` shows only unblocked work.
- **Priority**: P0=critical, P1=high, P2=medium, P3=low, P4=backlog (use numbers 0-4, not words)
- **Types**: task, bug, feature, epic, question, docs
- **Blocking**: `br dep add <issue> <depends-on>` to add dependencies

### Pre-Commit Hook

Before the first commit in a fresh clone, run `./scripts/install-hooks.sh`.

A pre-commit hook automatically runs on every `git commit`. Do NOT run it manually — just commit and it gates you. The hook runs (fail-fast):
1. `cargo fmt --check --all`
2. `cargo clippy --workspace --all-features -- -D warnings`
3. `cargo test --workspace --all-features`

There is no escape hatch. If tests fail, fix them.

### Project Skills

- For QAT bead implementation, review, or closure, use `.agents/skills/qat-bead-closure/SKILL.md` before closing. Closure comments must include the QAT checklist, a claim-to-gate matrix, and the no future variant acceptance rule.
- For ASM/ISA beads, also use `.agents/skills/asm-bead-closure/SKILL.md`.
- For model topology or scalar model-contract beads, also use `.agents/skills/model-contract-bead-closure/SKILL.md`.
- For sequence-state beads, also use `.agents/skills/sequence-state-bead-closure/SKILL.md`.
- For tiny fixture beads, also use `.agents/skills/fixture-bead-closure/SKILL.md`.
- For structured logging beads, also use `.agents/skills/logging-bead-closure/SKILL.md`.

### Session Protocol

**Before ending any session, run this checklist:**

```bash
git status              # Check what changed
git add <files>         # Stage code changes
br sync --flush-only    # Export beads changes to JSONL
git commit -m "..."     # Commit everything (pre-commit hook runs automatically)
git push                # Push to remote
```

### Best Practices

- Check `br ready` at session start to find available work
- Update status as you work (in_progress → closed)
- Create new issues with `br create` when you discover tasks
- Use descriptive titles and set appropriate priority/type
- Always sync before ending session

### Training Loss Beads

- Separate raw diagnostic loss from weighted total-loss contribution. Log raw per-term losses; apply configured loss weights in the composition owner.
- Define logits reduction explicitly: name the class/vocab axis, then state whether remaining batch/token dimensions are summed or averaged.
- For activation/range losses, name the batch and per-sample activation axes. Use a checked value object when a flat slice would hide sample width or boundary semantics.
- Tests for scalar hyperparameters such as safe bounds, temperatures, and loss weights must include a non-default/non-1.0 value.
- For router z-loss, name the zero point/baseline (uncentered versus centered) and distinguish training `lambda_zrouter` losses from QAT/router aux-loss proxies.
- For router load-balance losses, name hard top-1 assignments as stop-gradient dispatch provenance; gradient claims must identify whether the proof reaches routing probabilities, router logits, or full router parameters.
- Burn loss helpers must validate computed tensor losses, including weighted outputs, for finite values before returning.
- Burn loss helpers should not host-copy entire differentiable tensors for routine validation; validate scalar config/shape before tensor math and validate the computed loss after tensor math unless host inspection is required by the contract.
- For capped tensor losses, do not express `min`/`max` by subtracting large nearly equal tensors. Use scalar `clamp` or tensor mask selection and add a large finite scalar/Burn parity regression.
- For ternary zero/sparsity losses, matrix thresholds mirror the QAT ternary model contract: one global threshold or one threshold per output row. Do not expose per-weight thresholds unless a model/artifact bead defines that public behavior.
- Keep raw weighted-loss helpers honest: they must validate finite/non-negative raw diagnostics even when the configured weight is zero. If a helper intentionally skips raw computation for a disabled config term, name it as a contribution/composer helper rather than a raw weighted-loss helper.
- If a loss claim depends on Burn autodiff, closure must cite a feature-enabled gate such as `cargo test -p gbf-train --features burn-adapter -- <loss_test>`.
- When a filtered test target is introduced by the patch, report the number of tests run and avoid claiming red-before-green unless you actually ran the pre-patch check.
- Do not claim phase-boundary adoption or training-loop logging from a standalone loss helper. Name the integration bead that owns the real caller.
- Loss config helpers must distinguish raw TOML config from phase-effective config. Scalar diagnostic totals/logging helpers are not differentiable Burn training-loss composers.
- Do not give raw per-term diagnostic collections an implicit all-zero default; enabled lambdas can otherwise hide missing raw loss computation. If zeros are intentional, require explicit fields or a named contribution helper.
- Logging helper closure must cite subscriber-level capture for event shape and move real producer/dashboard adoption to a named owner bead when no executable producer exists.

### Artifact Export Fact Beads

- Export fact schema beads must separate schema support from producer collection, compiler consumption, and dashboard/report adoption. Name the owner bead for any moved producer or consumer path.
- Rate or probability newtypes must reject impossible ratios before quantization and must use overflow-safe arithmetic for conversion helpers.
- Distribution-like vectors must validate aggregate invariants in constructors and deserialization, not just per-entry scalar bounds.
- Unordered artifact hint pairs must canonicalize their stored representation in constructors and deserialization before deriving equality or hashing.
- Public artifact JSON shape tests should pin downstream field names with explicit `serde_json::json!` assertions, not only serde round-trips.
- Expert-scoped export facts must state whether `ExpertId` is global or layer-local. If the model uses layer-local expert indexes, include `LayerId` or an artifact path in the fact.

<!-- end-br-agent-instructions -->
