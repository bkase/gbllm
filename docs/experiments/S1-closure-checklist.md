# S1 Closure Checklist

This checklist is for `bd-1261`, the final S1 report assembly and Decision
binding bead. It is intentionally not a closure artifact by itself.

Do not close `bd-1261` or `bd-12pl` from IntegrationFixture output, fixture
goldens, report-only scenario selection, placeholder hashes, or dummy commit
ids. Closure requires the real F-S1.28 production run artifacts from `bd-1ehz`
and the final `docs/experiments/S1-report.md` assembled from those artifacts.

Current draft status: local artifacts under `experiments/S1-production-local`
bind `S1Outcome = Fail-capacity` and `Decision = Investigate(propose-Toy1)`.
The report is still blocked from final CLI emission by missing real
`predictions_commit` and `first_result_commit` history values.

Toy1 successor status: `bd-2ibf` extends the closure path with a separate
pre-registered report at `docs/experiments/S1-Toy1-report.md` and a separate
artifact tree at `experiments/S1-toy1`. Use this checklist against the Toy1
paths after runtime/CLI support can actually produce `ModelSizeProfile::Toy1`
S1 artifacts; substitute `experiments/S1-toy1` for `experiments/S1` in the
production-input paths below. The original Toy0 report remains predecessor
evidence and must not be rewritten as if it were the Toy1 run.

## Required Production Inputs

- [ ] `bd-1ehz` is closed with five completed TinyStories Production seed runs.
- [ ] `experiments/S1/checkpoints/seed-0/metadata.json` exists and is
      `budget_profile = "production"`.
- [ ] `experiments/S1/checkpoints/seed-1/metadata.json` exists and is
      `budget_profile = "production"`.
- [ ] `experiments/S1/checkpoints/seed-2/metadata.json` exists and is
      `budget_profile = "production"`.
- [ ] `experiments/S1/checkpoints/seed-3/metadata.json` exists and is
      `budget_profile = "production"`.
- [ ] `experiments/S1/checkpoints/seed-4/metadata.json` exists and is
      `budget_profile = "production"`.
- [ ] `experiments/S1/runs/seed-0/run_log.json` exists.
- [ ] `experiments/S1/runs/seed-1/run_log.json` exists.
- [ ] `experiments/S1/runs/seed-2/run_log.json` exists.
- [ ] `experiments/S1/runs/seed-3/run_log.json` exists.
- [ ] `experiments/S1/runs/seed-4/run_log.json` exists.
- [ ] `experiments/S1/seed-0/s1_score.v1.json` exists.
- [ ] `experiments/S1/seed-1/s1_score.v1.json` exists.
- [ ] `experiments/S1/seed-2/s1_score.v1.json` exists.
- [ ] `experiments/S1/seed-3/s1_score.v1.json` exists.
- [ ] `experiments/S1/seed-4/s1_score.v1.json` exists.
- [ ] `experiments/S1/s1_baseline.v1.json` exists.
- [ ] `experiments/S1/seed-0/s1_negative_test.v1.json` exists.
- [ ] `experiments/S1/seed-0/s1_ablation.v1.json` exists.
- [ ] `experiments/S1/s1_oracle.v1.json` exists.

## Report Assembly Inputs

- [ ] RFC revision is a real git commit id.
- [ ] `predictions_commit` is a real git commit id.
- [ ] `first_result_commit` is a real git commit id.
- [ ] `predictions_commit` is a strict ancestor of `first_result_commit`.
- [ ] `predictions_section_hash` matches the pre-result
      `## Pre-registered predictions` section.
- [ ] `baseline_self_hash` is populated from the production baseline artifact.
- [ ] Every seed has `checkpoint_self_hash`, `run_log_self_hash`, and
      `score_self_hash` in report front matter.
- [ ] Seed 0 has `negative_self_hash` and `ablation_self_hash` in report front
      matter.
- [ ] No final report field uses fixture-only constants, zero hashes, null
      closure hashes, IntegrationFixture report output, or dummy commit ids.

For the Toy1 successor report, run the preregistration gate with the successor
artifact directory so Toy0 result history does not count as Toy1 history:

```sh
scripts/s1_preregistration_check.sh \
  --report docs/experiments/S1-Toy1-report.md \
  --artifact-dir experiments/S1-toy1
```

## Hypothesis And Decision Checks

- [ ] H1 is computed from production run-log completion, finite losses, finite
      gradient norms, and loss-window checks.
- [ ] H2 is computed from every seed satisfying
      `val_bpc < bpc_3gram - 0.05` plus the median-bound check.
- [ ] H3 is computed from validation bpc versus `bpc_unigram - 0.5` and
      `neg_test_delta(seed=0) > 2.0`.
- [ ] H4 is computed from seed-0 canonical tensor payload hash equality.
- [ ] H5 is computed from `metric_oracle_passed`.
- [ ] The F-S1.19 dispatcher is the only source of `S1Outcome` and `Decision`.
- [ ] `Decision` is exactly one of `ProceedToS2`,
      `ProceedToS2-with-T12.5-prereq`, or
      `ProceedToS2-with-H2-waiver(toy1-narrow-h2-miss)`.
- [ ] If `Decision = ProceedToS2-with-T12.5-prereq`, the `bd-1xqf <- bd-1y1s`
      dependency edge has been added.

## Closure Gates

- [ ] `cargo test -p gbf-experiments`
- [ ] `cargo test -p gbf-experiments --features falsify --test falsification`
- [ ] `cargo test -p gbf-experiments --test oracle`
- [ ] `cargo test -p gbf-experiments --test canonical_json`
- [ ] `cargo test -p gbf-experiments --test integration`
- [ ] `cargo build -p gbf-experiments --no-default-features --features ablation`
- [ ] `scripts/s1_preregistration_check.sh`
- [ ] `scripts/s1_determinism_check.sh`
- [ ] `scripts/s1_isolation_check.sh`
- [ ] `s1_report.v1` R-Decision, R-AllSeeds, R-ClosureArtifacts,
      R-Self-Hash, R-Predictions, and R-AllHypotheses validators pass.
- [ ] `gbf s1 doctor` reports all PASS for the closure environment.
- [ ] F-S1 CI closure jobs are green.

## Forbidden Closure States

Closure is forbidden if any of these are true:

- Any seed is `DivergedAt` or `NotReached`.
- Any production artifact listed above is missing or has an invalid self-hash.
- Any checkpoint metadata has `budget_profile = "integration_fixture"`.
- `metric_oracle_passed = false`.
- `phase_a_eq_ablation = false`.
- `Decision` is `Halt(...)` or `Investigate(...)`.
- Pre-registration cannot prove `predictions_commit` precedes
  `first_result_commit`.
- The final report relies on fixture goldens, dummy commit ids, placeholder
  hashes, or scenario-key-only verdicts.
