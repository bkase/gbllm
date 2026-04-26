

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

### Model Topology Beads

- For topology/config beads, state in closure whether the bead owns only shape/topology or executable forward behavior.
- Do not derive `Deserialize` for constructor-validated model config types unless deserialization goes through the same validation boundary.
- Prefer enum-backed topology choices over bool flags when invalid combinations must be unrepresentable; test both dense and routed paths directly.

### Model Semantics Beads

- For scalar `gbf-model` semantics that do not own Burn, export, artifact, or budget paths, name those unsupported boundaries in closure and do not claim them complete.
- When claiming parameter sharing or parameter-count reduction, add an owned-layer alias/count test and name a follow-up owner for export/artifact/budget sharing if that layer is not implemented.
- Keep enum or named constructors as the primary API for model mode choices; bools should be derived queries or edge-adapter inputs.

### Fixture Beads

- Keep one source of truth for tiny fixtures. If a tiny model config names layers or paths, the model fixture should own the state that artifact/workload factories consume.
- Name placeholders explicitly when real policy, workload, manifest, compiler, or runtime contracts are still stubs, and create a follow-up owner before closing.
- Scope artifact assertions to the artifact type that exists. `ArtifactCore` helpers must check core tensor/quant invariants, including tensor content-hash self-consistency; manifest validation needs a real manifest contract and test.

### Structured Logging Beads

- Closure must distinguish a logging schema/helper contract from adoption by real training, data, model, CLI, export, or runtime producers. If producer adoption is incomplete, create a named follow-up bead before closing.
- Tests for logging event shape should include at least one subscriber-level capture of actual `tracing` fields, not only a mirrored test collector or source grep.
- Canonical event names used by downstream tests should be constants in code. Do not introduce direct `tracing::*` call sites with ad hoc event names or load-bearing message strings.
- Do not claim observability performance targets, such as logging overhead percentage, unless a benchmark or explicit gate measures them.

<!-- end-br-agent-instructions -->
