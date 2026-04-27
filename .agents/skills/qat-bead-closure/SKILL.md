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

## Harness-Compatible Closure Skeleton

Use these exact fragments in QAT close reasons or closure comments; `scripts/qat_harness_checks.py` scans bead metadata case-sensitively:

- `QAT Closure Checklist`
- `Artifact contract`
- `differentiable Burn path`
- `Tests proving it`
- `Support Matrix`
- `Claim-To-Gate`
- `| Closure claim | Guarding test or command | Feature gate | Notes or deviation |`
- `No-future`

Before rerunning the harness after a corrective close, search the bead's existing close reason and comments for stale support-matrix rows. The harness scans comments as well as the current close reason, so an old closure packet can keep failing even after the bead is reclosed with corrected ownership.

## QAT Test Beads

- Before creating a moved-acceptance owner, search existing open beads and enrich the existing owner when one already names the behavior. Router/expert Burn adapter work is owned by `bd-1ptv` unless that bead is explicitly superseded.
- Keep scalar module tests, Burn adapter gradient tests, artifact byte agreement, and phase-hardness scheduling as separate claims.
- If a QAT test bead only proves scalar or pre-export behavior, move artifact agreement to `bd-g90`/`bd-12c`/`bd-22o`, router/expert Burn gradients to `bd-1ptv`, and Off/Soft/Hard annealing semantics to `bd-2uw`.
- Test oracles should be independent of the production helper under review. Prefer literal expected values or separately computed reference formulas over calling the same projection/export helper the test is meant to verify.
- For independent reference packer/oracle beads, keep projection, quantization, and byte packing as separate claims unless the public plan carries the concrete values needed for all three. Byte packing should consume canonical artifact payloads; mark production-vs-reference byte agreement as moved until a production materializer emits comparable bytes.
- Do not call a pre-export `export_canonical` reconstruction an artifact round trip. Artifact round trips require `ArtifactCore` or serialized artifact bytes and a dedicated gate.
- For F4 phase/config beads, state whether the type is a canonical five-phase schedule or a generic timeline. Canonical schedules must reject wrong phase count, non-zero start, noncanonical order, gaps, overlaps, zero-length ranges, and step overflow with focused tests.
- For phase hardness/mode beads, prove live phase transitions on already-constructed model or Burn adapter state. Construction-time initialization tests are not enough for a claim that a scheduler can change modes at phase boundaries.

## Claim Discipline

- Do not claim exact compiler/runtime lowering agreement unless the closure cites a compiler, oracle, or codegen gate that exercises that lowering. If the lowering gate does not exist yet, mark the claim as `moved` and name the owning bead, typically `bd-g90` for ExportVisitor materialization or `bd-12c` for ArtifactOracle agreement.
- Do not call inline floats, structs, or `Vec<f32>` "first-class tensors". First-class tensor claims require `CanonicalTensor` handles or an explicit moved-acceptance owner such as `bd-g90`/`bd-209`.
- Distinguish artifact metadata/facts from canonical tensors. Activation ranges, range digests, and scalar export records are not tensors unless the artifact carries them as `CanonicalTensor`; sequence-state semantics belong to F12 unless the current bead explicitly owns them.
- Treat public quantization plan variants as unsupported on deployable artifact paths until `ArtifactCore` validates the exact tensor encoding and executable path. Byte-cost math or verifier-only pack/unpack support is not artifact support by itself.
- Treat threshold schedules as projection provenance unless concrete threshold state is exported and tested. If a learned threshold variant has no artifact state, reject it at the artifact boundary and name the owner bead.
- If a bead's literal acceptance wants a gradient proof in `gbf-model`, satisfy it through `gbf-train --features burn-adapter` only when the closure states the architectural move, references the Burn-adapter boundary, and names the exact Burn test.
- Scope Burn/scalar parity claims to what the Burn tests actually prove. If scalar code rejects non-finite inputs or computed state but the Burn adapter does not guard that path, either add focused Burn rejection tests or say the Burn path is supported for finite inputs and finite adapter state.
- A deployable approximation's training forward must match the exported behavior for every supported path, or the support matrix must mark the mismatch as moved/rejected and name the owning bead.
- If QAT forward semantics add or change a nonlinearity, activation clip, phase behavior, or other non-weight operation, encode it in export/artifact identity or reject that path with a focused test. Do not let scalar forward behavior and exported artifact semantics diverge silently.
- Keep one authoritative range owner for activation/nonlinearity behavior. If a pre-quant nonlinearity and fake-quant step both use a range, validate or derive one from the other and document which one exports.
- Artifact schema summaries must be bidirectionally consistent with their detailed records. If one field says a weight is ternary, another record must provide the required scale/bias metadata, and tests must prove both missing-summary and missing-detail rejection paths.
- When a bead is mostly proving behavior implemented by an earlier bead, state the earlier owner and add a stronger regression oracle rather than reclosing the same claim.
- Artifact tensor encoding must match the declared quant plan. If a public plan exposes formats or granularities that the current tensor model cannot encode, reject them at the artifact boundary and create or cite the owner bead for support.
- Default plan, mode, or schedule claims must prove where the default is applied, not only that a helper returns the expected value. Add a constructor/export test, or mark the default as schema-only and name the bead that wires it into behavior.
- When adding a serde-default field to public artifact data, add a compatibility test for deserializing the previous shape and a round-trip test for the new shape before claiming migration safety.
- Burn adapters must not hide stale learnable state inside embedded scalar cores. Split non-learnable plan shape from learnable tensors, or document one authoritative source per field and guard it with an export-from-trained-state test.
- A moved Burn training path must name a concrete adapter owner for the exact public behavior. Do not cite a closed generic adapter-containment bead as the owner for newly introduced router/expert/quantizer adapters; create or follow a live owner bead first, and cite that bead in the support matrix.
- Do not call a scalar single-token router term `balance_loss` or `load_balance_loss` if the standard batch/token MoE objective is elsewhere. Name it as a proxy and cite the bead that owns the standard loss.
- If a QAT forward mutates sequence, temporal, EMA, or cache state, add tests proving failure does not advance state and that the stored state has the documented semantics. For routers, keep soft routing probabilities separate from hard dispatch weights.
- Thread phase/activation/hardness options through every branch, including optional shared branches. Add a test that exercises the optional branch, not only the default path.
- Keep router dispatch mode and numeric quantization hardness separate unless the public API explicitly unifies them. If router behavior stays on `RouterTrainMode`, the support matrix must say so instead of implying `QuantHardness::Off/Soft/Hard` applies to the router.
- For optional QAT branch or stability-profile beads, prove both surfaces named by the bead: the architecture/config/topology surface must carry the `Option` and default it off, while executable QAT state must prove the branch math. A concrete `Option<BranchState>` on the module alone is not a config gate.
- If an optional branch initializes with zero scale/gate/alpha, add a Burn gradient test proving the trainable scale/gate/alpha still receives gradient at initialization. Do not claim branch weights receive gradient at step zero unless the test proves it.
- If a branch is described as small or common-bank resident, enforce a relative-width/budget boundary or mark common-bank accounting as moved with a named owner.
- If a bead mentions F4 phased hardness, either implement the exact `Off`/`Soft`/`Hard` contract or explicitly move it to the phase-hardness owner bead. Local two-state shortcuts must be documented as local execution modes, not as F4 completion.
- If a training config names executable model modes, reuse the model-owned enum where possible. If a separate config enum is necessary, add explicit conversion tests so the two vocabularies cannot drift silently.
- Document deterministic router conventions that affect artifacts or training traces, including top-1 tie break and default-rank clamping for tiny expert sets.
- Budget and preflight beads must distinguish provisional estimates from artifact/compiler-backed exact costs. Name estimated metadata as estimated, state the exact owner when final packing is elsewhere, and add tests for both the formula and the fit/reject boundary. Do not create shadow `RuntimeChromeBudget` or `DeployabilityEnvelope` schemas in `gbf-model`; the model layer computes byte math, while policy/train/compiler layers own budget-source selection and per-bank diagnostics.
- Preflight budget APIs must reject invalid model dimensions instead of reporting zero-sized experts or routers as fitting. Keep raw arithmetic helpers separate from validated preflight/report entry points.
- Unsupported QAT variants may expose config-only diagnostics, but must not expose public canonical budget/export/training helpers unless the executable path exists or the helper is explicitly rejected/moved in the support matrix.

## Support Matrix

Every QAT bead closure must include a path support matrix. Use `supported`, `rejected`, `moved`, or `not applicable`; do not leave a path implicit.
Keep these status cells exact. Put qualifiers such as "finite-input", "config-only", or "metadata-only" in the behavior name or guard text, not by inventing new status values.

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
cargo clippy -p gbf-train --features burn-adapter -- -D warnings
cargo test --workspace --all-features
```

For Burn version pinning or adapter-containment claims, cite the project pin check directly:

```bash
./scripts/check_burn_pin.sh
```

If no exact gate exists, add one before closing the bead or leave the bead open.

## No Future Variant Acceptance

Do not accept enum variants, plan variants, strategies, or schedules that imply future behavior until the implementation and tests exist. Examples include annealing modes, learned thresholds, future artifact encodings, or unsupported lowering plans. Config-only variants must be named as config-only in the support matrix and must be rejected at executable/export/Burn boundaries with tests unless those paths are implemented.

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

If a moved-acceptance sentence contains backticked API names or enum variants, verify the target bead text contains those exact names before closing. Enrich the target bead first when the ownership is correct but not machine-checkable.

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

After closing any QAT bead with a substantial closure reason or moved-acceptance comments, run `python3 scripts/qat_harness_checks.py` again. The close reason itself becomes bead metadata and can introduce new harness failures.
