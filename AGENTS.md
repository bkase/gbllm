

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
- If a loss claim depends on Burn autodiff, closure must cite a feature-enabled gate such as `cargo test -p gbf-train --features burn-adapter -- <loss_test>`.
- Do not claim phase-boundary adoption or training-loop logging from a standalone loss helper. Name the integration bead that owns the real caller.

<!-- end-br-agent-instructions -->
