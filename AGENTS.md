

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

### QAT Test Beads

- Before creating a moved-acceptance owner, search existing open beads and enrich the existing owner when one already names the behavior. Router/expert Burn adapter work is owned by `bd-1ptv` unless that bead is explicitly superseded.
- Keep scalar module tests, Burn adapter gradient tests, artifact byte agreement, and phase-hardness scheduling as separate claims. If a QAT test bead only proves scalar or pre-export behavior, move artifact agreement to `bd-g90`/`bd-12c`/`bd-22o`, router/expert Burn gradients to `bd-1ptv`, and Off/Soft/Hard annealing semantics to `bd-2uw`.
- Test oracles should be independent of the production helper under review. Prefer literal expected values or separately computed reference formulas over calling the same projection/export helper the test is meant to verify.
- Do not call a pre-export `export_canonical` reconstruction an artifact round trip. Artifact round trips require `ArtifactCore` or serialized artifact bytes and a dedicated gate.

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

### ASM / ISA Beads

- State whether an `Instr`/AsmIR type is symbolic pre-layout IR, post-relaxation concrete machine IR, or a separate adapter between them. Move symbolic labels, relocations, pseudo-ops, and branch relaxation to their owning bead instead of letting concrete instruction types imply that lifecycle.
- Distinguish legal CPU encodings from canonical project encodings. If a shorter/canonical form is required, document the rejected non-canonical legal form and test the constructor or boundary that enforces it.
- Do not derive `Deserialize` for constructor-validated newtypes unless serde is routed through the same validation boundary, for example with `#[serde(try_from = ...)]`. Add at least one negative deserialization test for every private-field newtype whose constructor rejects values.
- When citing filtered cargo-test commands in closure, confirm the command actually ran tests in the current patch. A passing filter with `running 0 tests` is not evidence.
- When a bead supersedes an older `planv0.md` sketch, state the supersession or mapping in code docs and closure. Do not let a self-referential enum list stand in for plan alignment.
- Symbol names must be built from validated segments, not by joining raw caller strings and validating afterward. Add collision tests for dotted helper arguments.
- Symbol tables should allow address aliases unless the bead explicitly owns a primary-symbol-only table. Reverse lookup APIs must return all names for an address.
- If a report/schema type derives serde and contains maps, verify JSON serialization directly. Avoid non-string map keys in JSON-facing structures, or provide an explicit stable representation.
- Builder beads emit symbolic pre-layout section IR. They may record concrete `Instr`s, labels, alignment directives, pseudo-op intent, and raw escape hatches, but closure must move relocation, branch relaxation, far-call thunking, final align padding, and byte lowering to their owner beads. Effect/privilege claims belong to `bd-1bw` or a named successor.
- Effect classifiers must not collapse stack-touching instructions into pure compute. `PUSH`/`POP`, calls, returns, `RST`, and two-byte stores need explicit stack/control or mixed-region tests.
- Full-address classifiers must test memory-map boundaries, including `$FF00..=$FFFF` high memory and two-byte writes that cross regions.
- Section privilege is a durable section invariant. Any API that changes privilege after emission must revalidate existing items, and tests must cover downgrade rejection.
- Raw bytes are opaque privileged effects unless a bead explicitly narrows the claim to data-only sections. Add a raw-byte rejection or audit-surface test whenever builder-boundary privilege enforcement is claimed.
- Dynamic-address load/store effects must be named as reachability obligations; do not silently call them fixed-region effects without a proof from a later pass.
- Raw byte escape hatches must not be publicly constructible through multiple paths. Keep raw constructors crate-private or guarded, keep section mutation behind builder APIs, and add a raw-specific test or closure note for the audit surface.
- Do not model unknown-width symbolic items as zero-byte fixed size. Size APIs for alignments, pseudo-ops, relocations, or runtime-lowered markers should return `None`/unknown or be explicitly named as lower bounds.
- Pseudo-op tests must assert exact payloads, not only that pseudo-op calls do not panic. If a builder tracks lease-like state, test duplicate, unknown, released, and range-error paths.
- Provenance-scope helpers that temporarily mutate builder state must restore that state on normal return and caught panic.

### Sequence-State Beads

- Keep one authoritative sequence semantics contract. If crate dependencies require durable schema in `gbf-artifact`, re-export it from `gbf-model` and state that ownership movement explicitly in bead closure.
- Export paths must consume `SequenceExportFacts` derived from model topology or a sequence block, not a free-standing enum supplied at finish time. Add a test that `ExportVisitor` carries those facts into both `ArtifactCore` and `ExportFacts`.
- A sequence dispatch trait must use project-native activation/state types with shape and finiteness validation. Do not satisfy trait acceptance with fully unconstrained associated `Input`/`Output`/`State` types.
- Policy/profile selector enums such as `SequenceSemanticsRef` are configuration references only until compile profiles consume them. Avoid derived ordering unless an ordering has executable policy meaning.
- For executable sequence blocks, define batch/state semantics before closure. If `SequenceState` is one shared buffer, reject `batch > 1` and test the rejection; only accept batches after state is explicitly batch-shaped.
- Do not put artifact path naming or fabricated tensor handles in runtime block config. Sequence parameter handles must come from an `ExportVisitor` path that actually emits matching `ArtifactCore` tensors, with a test that every handle resolves to a tensor id.
- Fixed recurrence/update behavior needs a literal value-level oracle test over at least two tokens. If the update law is only a placeholder or research variant, narrow the closure and avoid claiming numeric semantics beyond scaffolding.
- Keep scalar sequence kernels, Burn/autodiff gradient paths, `TrainPhaseSpec` hardness scheduling, and shadow-compile A/B fixture adoption as separate claims with separate gates or explicit moved-to beads.
- When a sequence block imposes an executable byte-record layout, align model topology validation and tiny fixtures with that layout. Durable artifact schema may stay broader, but model config must reject layouts the executable block cannot instantiate.
- Byte-backed sequence state must validate canonical persisted form before mutation. For cache-like records, test validity flags, contiguous live records, empty-record zeroing, sliding/truncation behavior, and failed-forward atomicity.
- If a v1 sequence block uses a simplified mechanism such as tied key/value payloads or record metadata inside `*_bytes_per_token`, name that explicitly in docs, tests, and closure; do not let the public wording imply a richer KV layout.

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
