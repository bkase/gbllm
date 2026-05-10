# S1 Implementation Guide

This guide is the contributor-facing companion to the
[F-S1 First Pulse RFC](../../history/rfcs/F-S1-first-pulse.md). The RFC is the
normative contract; this document explains where the implementation lives, how
to run the local checks, and how to extend the S1 surface without blurring
fixtures into production evidence.

Status caveat: this guide is not the final S1 report and does not claim final
experiment results. The full 5-seed TinyStories run and seed-0 ablation
artifacts are owned by `bd-1ehz`. Final `S1-report.md` assembly, Decision
binding, and closure evidence are owned by `bd-1261`. IntegrationFixture and
tiny-fixture artifacts are useful development evidence, but they are not S1
closure artifacts.

## Overview

F-S1 is the "first pulse" experiment: run the dense floating-point Toy0 model
on raw-byte TinyStories, prove that the measurement and replay machinery is
honest, then decide whether the slice can proceed to S2. The main risk is not
whether a tiny model can improve at all; it is whether the code can separate
real training evidence from fixture smoke tests, accidental phase leaks,
measurement bugs, or suspicious data handling.

The closure hypotheses from RFC Section 3 are:

| Hypothesis | Meaning | Closure role |
| --- | --- | --- |
| H1 | Substrate: dense fp training, replay, logs, and checkpoint artifacts behave deterministically enough to trust. | Required for closure. |
| H2 | Capacity: Toy0 beats the registered baseline on canonical validation data. | Required for closure. |
| H3 | Sequence-state: optional warning path; failure does not block closure but creates the T12.5 prerequisite. | Non-blocking warning. |
| H4 | Phase cleanliness: phase-A weights and ablation/QAT boundaries do not leak later-phase behavior. | Required for closure. |
| H5 | Measurement: scoring, shuffle pins, oracles, and report inputs are internally consistent. | Required for closure. |

Outcome dispatch is total: every evaluated combination of H1 through H5 maps to
an `S1Outcome` and `Decision`. Closure/proceed decisions must not be produced
when a closure-required hypothesis is `NotEvaluatedDueToPriorGate`.

## Crate And File Map

| Area | Primary files | Bead references | Notes |
| --- | --- | --- | --- |
| RFC contract | [history/rfcs/F-S1-first-pulse.md](../../history/rfcs/F-S1-first-pulse.md) | `bd-12pl` epic | Normative source for hypotheses, artifacts, and amendments. |
| Corpus manifests | `gbf-data`, [fixtures/corpora/tinystories.toml](../../fixtures/corpora/tinystories.toml) | `bd-ifwg`, `bd-2who`, `bd-3bqk` | Manifest paths resolve relative to the manifest file; canonical TinyStories bytes may be absent locally. |
| S1 experiment modules | [gbf-experiments/src/s1](../../gbf-experiments/src/s1) | many S1 beads | Public experiment support modules. |
| Run/checkpoint artifacts | [gbf-experiments/src/s1/run.rs](../../gbf-experiments/src/s1/run.rs) | `bd-1xo5`, `bd-14f2` | IntegrationFixture path exists; production Burn optimizer loop is owned separately. |
| Baseline | [gbf-experiments/src/s1/baseline.rs](../../gbf-experiments/src/s1/baseline.rs) | baseline beads | Fits and emits `s1_baseline.v1`. |
| Scoring | [gbf-experiments/src/s1/score.rs](../../gbf-experiments/src/s1/score.rs) | scorer beads | Keep fixture scorer artifacts distinct from production scorer evidence. |
| Negative test | [gbf-experiments/src/s1/neg_test.rs](../../gbf-experiments/src/s1/neg_test.rs) | `bd-2tlx` | Fisher-Yates shuffle, delta checks, and `s1_negative_test.v1`. |
| Ablation | [gbf-experiments/src/s1/ablation.rs](../../gbf-experiments/src/s1/ablation.rs) | `bd-3b3l` | Compares trainable canonical tensor payloads and reports first mismatch. |
| Oracles | [gbf-experiments/src/s1/oracle.rs](../../gbf-experiments/src/s1/oracle.rs) | `bd-2jta`, `bd-3u93`, `bd-2hs7` | Produces `s1_oracle.v1` and H5 status. |
| Outcome/report algebra | [gbf-experiments/src/s1/report.rs](../../gbf-experiments/src/s1/report.rs), [gbf-experiments/src/s1/schema.rs](../../gbf-experiments/src/s1/schema.rs) | `bd-2ei7`, `bd-3v7y`, `bd-1261` | Owns `S1Outcome`, `Decision`, report JSON, and final report assembly. |
| CLI support | [gbf-experiments/src/s1/cli.rs](../../gbf-experiments/src/s1/cli.rs), `gbf-cli` | `bd-7ljt`, `bd-tq7w`, `bd-17gk`, `bd-2n4s` | Implements `gbf s1 ...` subcommands and diagnostics. |
| Structured logging | [gbf-experiments/src/s1/logging.rs](../../gbf-experiments/src/s1/logging.rs) | logging/review beads | Use S1 event names and subscriber-capture tests for shape changes. |
| Scripts | [scripts](../../scripts) | `bd-ah6o`, prereg/e2e beads | CI and local workflows should call these scripts rather than duplicating protocol logic. |
| CI workflows | [.github/workflows/s1-pr.yml](../../.github/workflows/s1-pr.yml), [.github/workflows/s1-nightly.yml](../../.github/workflows/s1-nightly.yml), [.github/workflows/s1-on-demand.yml](../../.github/workflows/s1-on-demand.yml) | `bd-ah6o` | Boundary-aware PR, nightly, and on-demand workflows. |
| Final report | [docs/experiments/S1-report.md](../../docs/experiments/S1-report.md) | `bd-1261` | Report scaffold and final evidence live here, not in this guide. |

## Quickstart

Start with model-free and fixture-local checks. They give fast signal without
pretending that canonical TinyStories artifacts exist:

```sh
cargo test -p gbf-experiments --test tiny_fixture
cargo test -p gbf-experiments --test run
cargo test -p gbf-experiments --test oracle
cargo test -p gbf-experiments --features falsify --test falsification
```

The CLI smoke path lives behind the `gbf s1` subcommands. Build the CLI first,
then use the tiny fixture manifest when canonical data is not present:

```sh
cargo build -p gbf-cli
target/debug/gbf-cli s1 doctor \
  --manifest gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml \
  --json
```

For hermetic checks, use a minimal environment and keep cache/output paths
outside the worktree:

```sh
env -i \
  HOME="$HOME" \
  PATH="$PATH" \
  TMPDIR="${TMPDIR:-/tmp}" \
  RUST_BACKTRACE=1 \
  target/debug/gbf-cli s1 doctor \
    --manifest gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml \
    --json
```

Useful script-level smoke gates:

```sh
scripts/s1_determinism_check.sh --fast
scripts/s1_isolation_check.sh --fast
scripts/s1_e2e_cli.sh --fixture tiny --out-dir /tmp/gbf-s1-e2e
scripts/s1_preregistration_check.sh --report docs/experiments/S1-report.md
```

If a command emits structured logs, check the event name first, then the
machine-readable fields. S1 logging tests intentionally assert field names so
dashboards and report assembly do not silently drift.

## CLI And Debugging Cookbook

The `gbf s1` family is the normal operator surface. Depending on the subcommand
and the current production boundary, a command may operate on canonical inputs,
fixture inputs, or metadata-only fixture artifacts. The command help should say
which case applies; if it does not, treat that as a bug.

Common commands:

| Command | Use |
| --- | --- |
| `gbf s1 doctor` | Check environment, manifest availability, pins, and diagnostic metadata. |
| `gbf s1 print-config` | Print the effective S1 configuration for review or CI logs. |
| `gbf s1 inspect` | Inspect emitted S1 artifacts and self-hashes. |
| `gbf s1 diff-checkpoints` | Compare checkpoint tensor payloads and metadata. |
| `gbf s1 replay` | Replay a run. Normative replay is production-only; fixture replay must be explicit. |
| `gbf s1 verify-determinism` | Re-run and compare deterministic artifacts, including mismatch diagnostics. |
| `gbf s1 fit-baseline` | Fit the registered baseline artifact. |
| `gbf s1 score` | Score a checkpoint. Fixture scorer output must not masquerade as production closure evidence. |
| `gbf s1 negative-test` | Run the validation-shuffle negative test and emit `s1_negative_test.v1`. |
| `gbf s1 ablation` | Compare phase-A and ablation checkpoint tensor payloads. |
| `gbf s1 oracle` | Emit `s1_oracle.v1` and H5 input. |
| `gbf s1 report` | Assemble report artifacts; final closure is owned by `bd-1261`. |

Failure playbook:

| Symptom | First checks | Re-run |
| --- | --- | --- |
| A seed diverged at step K. | Inspect `s1_run_log.v1`, `run.divergence` events, final gradient norms, and checkpoint metadata. | `cargo test -p gbf-experiments --test run` |
| Determinism check fails. | Use `scripts/s1_determinism_check.sh --json`; compare `safetensors` bytes, `run_log_self_hash`, and checkpoint metadata. | `cargo test -p gbf-cli --test s1_cli` |
| Ablation reports `first_mismatch`. | Confirm shared seed/config/corpus/device metadata, then use the tensor name and byte offset to inspect payload drift. | `cargo test -p gbf-experiments --test ablation` |
| Pre-registration check fails. | Do not edit predictions after result artifacts exist; inspect expected/observed commit labels in the script output. | `scripts/tests/preregistration_check_test.sh` |
| O-metric-3 reset-boundary oracle fails. | Check scorer context-length reset behavior at split boundaries. | `cargo test -p gbf-experiments --test oracle` |
| Doctor reports missing canonical corpus. | Either download/verify TinyStories for production work or switch to the tiny fixture for local smoke tests. | `target/debug/gbf-cli s1 doctor --json` |
| Fixture artifacts appear under `experiments/S1`. | Keep generated result artifacts untracked until predictions are committed. | `scripts/tests/s1_result_gitignore_test.sh` |

## Running The Full S1 Experiment

This section is a protocol checklist, not a claim that the protocol has already
been completed in this checkout. `bd-1ehz` owns the real full-run artifacts.
`bd-1261` owns final report binding and closure. Until those close with real
evidence, use this checklist as the operator path only.

1. Confirm the pre-registration state:

   ```sh
   scripts/s1_preregistration_check.sh --report docs/experiments/S1-report.md
   git status --short experiments/S1 docs/experiments/S1-report.md
   ```

2. Download or mount canonical TinyStories bytes, then verify the manifest and
   shuffle pin. If canonical data is absent, stop the production path rather
   than substituting the tiny fixture.

   ```sh
   python3 scripts/download_tinystories.py --split all
   scripts/s1_canonical_o_metric_4_check.sh
   ```

3. Build the phase-A and ablation configurations. Use the RFC build-feature
   definitions; do not infer phase cleanliness from unrelated features.

4. Run `gbf s1 doctor` in a clean environment and save its JSON output next to
   the experiment logs.

5. Fit the registered baseline, then run the canonical 5-seed replay for seeds
   0 through 4. Artifacts should land under the configured `experiments/S1`
   output tree and remain untracked until the prediction commit is fixed.

6. Score every seed with the production scorer. Fixture scorer output is valid
   only for fixture smoke tests.

7. Run the seed-0 negative test with `ShuffleRng(0xDEADBEEF)` and verify
   `s1_negative_test.v1`.

8. Run the seed-0 ablation build and comparator. A clean ablation report must
   compare trainable canonical tensor payload bytes, not just metadata hashes.

9. Emit `s1_oracle.v1` and feed H5 into report/outcome dispatch.

10. Run full determinism and per-seed isolation checks.

11. Assemble the final report and bind the final Decision. This is the
    `bd-1261` responsibility. Closure of `bd-12pl` should cite final artifacts,
    not this guide.

The S1 CI workflows call the current owner scripts:
[s1-pr.yml](../../.github/workflows/s1-pr.yml),
[s1-nightly.yml](../../.github/workflows/s1-nightly.yml), and
[s1-on-demand.yml](../../.github/workflows/s1-on-demand.yml). Nightly and
on-demand workflows are allowed to surface current production blockers; they
must not paper over missing full-run support.

## Toy1 Successor Run

The committed Toy0 production report ended in `FailCapacity` /
`InvestigateProposeToy1`. The Toy1 follow-up is a successor run, not a rewrite
of [S1-report.md](../../docs/experiments/S1-report.md). Keep the Toy0 report
and artifacts as predecessor evidence, then pre-register and run Toy1 through
its own paths:

```sh
S1_TOY1_REPORT=docs/experiments/S1-Toy1-report.md
S1_TOY1_ARTIFACT_DIR=experiments/S1-toy1
scripts/s1_preregistration_check.sh \
  --report "$S1_TOY1_REPORT" \
  --artifact-dir "$S1_TOY1_ARTIFACT_DIR"
```

Until Rust runtime/CLI support accepts `ModelSizeProfile::Toy1` for the S1 run
surface, stop after the preregistration check. When that support lands, every
Toy1-producing command must select Toy1 explicitly and write all generated
baseline, checkpoint, score, negative-test, ablation, oracle, and report inputs
under `experiments/S1-toy1`. Do not mix Toy0 `experiments/S1` artifacts into
the Toy1 report, even for the model-independent baseline; regenerate or copy
with a Toy1 result commit so the successor `first_result_commit` is auditable.

The Toy1 report uses the same H1, H3, H4, and H5 rules as Toy0. Its H2 capacity
gate substitutes the registered `ModelSizeProfile::Toy1` profile
(`d_model=32`, `d_ff=64`, `n_blocks=2`) for Toy0 and keeps the exact same
per-seed threshold: `val_bpc(seed) < bpc_3gram - 0.05` for seeds 0 through 4.

## Extending Schemas

Create `s1_*.v2` only when the RFC contract changes or a backward-incompatible
consumer needs a new shape. Cosmetic producer refactors should keep the current
version and its JSON shape stable.

Schema extension checklist:

1. Amend the RFC first when the field changes the experiment contract.
2. Add or update the Rust type in
   [schema.rs](../../gbf-experiments/src/s1/schema.rs) or the owning module.
3. Preserve canonical JSON behavior: sorted fields where required, stable
   float/string forms, and deterministic self-hash zeroing.
4. Add serde shape tests using `serde_json::json!`, not only round-trip tests.
5. Add consumer migration tests so old v1 artifacts either keep working or fail
   with a typed diagnostic.
6. Update producer comments, CLI help, and this guide if operator behavior
   changes.

Do not mutate v1 semantics to make a new producer convenient. S1 artifacts are
audit evidence; old artifacts must remain interpretable.

## Adding A Falsification Variant

Falsification tests live under
[gbf-experiments/tests/falsification](../../gbf-experiments/tests/falsification)
and are compiled with the `falsify` feature. Production/default builds must not
accidentally include falsification substitutes.

Checklist for a new variant:

1. Name the intended risk and the hypothesis it should fail.
2. Add the substitute under the `falsify` feature and keep the production path
   unchanged.
3. Assert the expected `S1Outcome`, `Decision`, and any structured event fields.
4. Add dispatcher coverage in
   [gbf-experiments/tests/falsification.rs](../../gbf-experiments/tests/falsification.rs).
5. If the variant represents a new measurement risk rather than a local harness
   check, amend the RFC and add an oracle or owner bead.

Falsification closure should say whether it is unit-level, fixture-level, or
full-pipeline coverage. Those are different claims.

## Adding An Oracle

Add an O-metric only when a new measurement risk is identified. The normal path
is an RFC amendment, then implementation in
[oracle.rs](../../gbf-experiments/src/s1/oracle.rs), then focused tests in
[oracle.rs tests](../../gbf-experiments/tests/oracle.rs).

Oracle checklist:

1. Keep the oracle model-free unless the RFC explicitly requires model
   evaluation.
2. Define deterministic inputs and expected outputs.
3. Emit a per-oracle boolean, include it in `failed_oracle_ids`, and recompute
   the aggregate `metric_oracle_passed`.
4. Wire the result into `MetricOracleResults::h5_status` so H5 is report input,
   not only a cargo-test property.
5. Add subscriber-capture tests for any new telemetry.
6. If canonical TinyStories bytes are optional locally, provide an ignored or
   skipped canonical test with a clear message and a tiny-fixture analog where
   allowed.

## Structured Logging

S1 logs are part of the debugging surface. Use
[logging.rs](../../gbf-experiments/src/s1/logging.rs) constants where available
and prefer event names under `s1.*`. New telemetry should have:

1. A stable event name.
2. Machine-readable fields with explicit units.
3. No raw corpus bytes, checkpoint payloads, or gradient tensors.
4. A subscriber-capture test for event shape.
5. A note in the producer or CLI help if the event is fixture-only.

Errors should include enough structure for scripts to propagate diagnostics in
stderr and `--json` output.

## Cross-Reference Index

RFC-to-implementation map:

| RFC area | Bead IDs | Implementation/docs |
| --- | --- | --- |
| Section 2 amendments and ownership rules | `bd-3rei`, future amendment beads | This guide and the RFC. |
| Section 3 hypotheses | `bd-2ei7`, `bd-3v7y`, `bd-1261` | [report.rs](../../gbf-experiments/src/s1/report.rs) |
| Section 5 run/checkpoint protocol | `bd-1xo5`, `bd-14f2`, `bd-1ehz` | [run.rs](../../gbf-experiments/src/s1/run.rs) |
| Section 6 baseline | baseline beads | [baseline.rs](../../gbf-experiments/src/s1/baseline.rs) |
| Section 7 scoring | scorer beads, `bd-1ehz` for production run use | [score.rs](../../gbf-experiments/src/s1/score.rs) |
| Section 8 outcome algebra | `bd-2ei7` | [report.rs](../../gbf-experiments/src/s1/report.rs) |
| Section 9 report artifacts | `bd-3v7y`, `bd-1261` | [schema.rs](../../gbf-experiments/src/s1/schema.rs), [S1-report.md](../../docs/experiments/S1-report.md) |
| Section 11 negative test | `bd-2tlx` | [neg_test.rs](../../gbf-experiments/src/s1/neg_test.rs) |
| Section 12 decision binding | `bd-1261` | Final report assembly. |
| Section 13 oracles/falsification | `bd-2jta`, `bd-3u93`, `bd-2hs7`, `bd-i5tz`, `bd-3l75` | [oracle.rs](../../gbf-experiments/src/s1/oracle.rs), [falsification tests](../../gbf-experiments/tests/falsification.rs) |
| Section 15 implementation layout | `bd-7ljt`, `bd-3rei` | CLI and this guide. |
| Section 16 builds/CI | `bd-ah6o` | [S1 workflows](../../.github/workflows/s1-pr.yml) |

Outcome-to-decision recovery map:

| `S1Outcome` | Decision | Recovery |
| --- | --- | --- |
| `PassClean` | `ProceedToS2` | Close S1 only after `bd-1261` binds real artifacts. |
| `PassWithWarning` | `ProceedToS2WithT12_5Prereq` | Add or confirm the T12.5 prerequisite owner before S2. |
| `FailSubstrate` | `InvestigateBurnOrAutodiff` | Start from run logs, divergence events, determinism diagnostics, and Burn integration. |
| `FailCapacity` | `InvestigateProposeToy1` | Confirm scoring and baseline first, then consider Toy1. |
| `FailPhase` | `InvestigateF4PhaseContract` | Inspect ablation payload mismatch and build-feature isolation. |
| `FailMetric` | `HaltMeasurementBroken` | Fix oracles/scorer before any closure claim. |
| `FailSuspicious` | `HaltAuditSplitAndBpc` | Audit corpus split, shuffle, BPC, and suspiciously low validation loss. |

Self-hash map:

| Field | Producer | Notes |
| --- | --- | --- |
| `checkpoint_self_hash` | [run.rs](../../gbf-experiments/src/s1/run.rs) | Checkpoint metadata hash; tensor bytes need separate payload checks. |
| `run_log_self_hash` | [run.rs](../../gbf-experiments/src/s1/run.rs) | Run log hash for determinism and replay. |
| `baseline_self_hash` | [baseline.rs](../../gbf-experiments/src/s1/baseline.rs) | Baseline artifact. |
| `score_self_hash` | [score.rs](../../gbf-experiments/src/s1/score.rs) | Ensure the scorer path matches fixture vs production claims. |
| `negative_self_hash` | [neg_test.rs](../../gbf-experiments/src/s1/neg_test.rs) | Seeded validation-shuffle negative test. |
| `ablation_self_hash` | [ablation.rs](../../gbf-experiments/src/s1/ablation.rs) | Payload comparator artifact. |
| `oracle_self_hash` | [oracle.rs](../../gbf-experiments/src/s1/oracle.rs) | `s1_oracle.v1` producer and H5 input. |
| `report_self_hash` | [report.rs](../../gbf-experiments/src/s1/report.rs) | Final report binding is `bd-1261`. |

## Glossary

Toy0: the dense floating-point raw-byte model used for S1.

Phase A: the dense fp training phase before later QAT/ablation phases.

IntegrationFixture: a deterministic local harness for smoke tests. It is not a
production closure artifact.

Tiny fixture: the checked-in tiny corpus used for fast tests and CI smoke
coverage.

Canonical TinyStories: the production corpus bytes registered by the S1
manifest and hash pins.

`S1CpuDeterministic`: the deterministic CPU profile expected by the S1
production protocol.

`BatchRng`, `InitRng`, `ShuffleRng`: separated random streams. Validation
shuffle pins use `ShuffleRng(0xDEADBEEF)`.

`DomainHash`: domain-separated hash helper used to avoid cross-artifact hash
ambiguity.

`S1CanonicalJson`: canonical JSON encoding used for S1 artifact self-hashes.

`metric_oracle_passed`: aggregate H5 boolean; it is true only when all
registered oracles pass.

Sensitive: the negative-test verdict that validation shuffle hurts enough to
support the measurement claim.

`first_mismatch`: the first tensor name and byte offset where ablation payloads
diverge.

`pass_version`: a pinned version constant for checks whose definition can
change only by explicit contract update.

Pre-registration: the rule that predictions and report scaffolding must be
committed before result artifacts influence the final report.

Closure artifact: evidence that can support S1 closure. Fixture smoke outputs,
metadata-only placeholders, and skipped canonical tests are not closure
artifacts.
