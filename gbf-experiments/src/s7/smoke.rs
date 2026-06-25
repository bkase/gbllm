//! Deterministic tiny-fixture S7 smoke harness.
//!
//! This module proves that the S7 closure helpers compose over one tiny
//! in-repo fixture. It intentionally does not claim production Gutenberg
//! training adoption; the real producer/report envelope remains owned by
//! bd-2v9r.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use gbf_artifact::{
    DistillRawDiagnostic, GradNormSummary, GuardrailVerdict as ArtifactGuardrailVerdict,
    LambdaSwitch, MatchedBytesPin, ParetoVerdict, RawLossDiagnostics, S7AggregateParityVerdict,
    S7Completion, S7DenseVsMoeComparisonReport, S7ParityVerdict, S7PerSeedComparison,
    S7ScoreReport, S7Topology, SweepSummary, SwitchStatsSummary, TrainPhase,
};
use gbf_foundation::{Hash256, sha256};
use serde::{Deserialize, Serialize};

use crate::S7_LOG_TARGET;
use crate::s7::baseline_match::canonical_s7_matched_bytes_pin;
use crate::s7::collapse_sweep::{
    D11_LAMBDA_SWITCH_SWEEP_SEED, DeterministicFixtureSweepProducer, GuardrailVerdict,
    LambdaSwitchSweepInput, RouterCollapseSweepReport, h6_guardrail_verdict,
    run_lambda_switch_sweep,
};
use crate::s7::outcome::{
    AggregateParityVerdict, S7Decision, S7Outcome, S7OutcomeDispatchInput, decision_for_s7_outcome,
    dispatch_s7_outcome,
};
use crate::s7::pareto::{s7_pareto_closure_signals, s7_pareto_verdict_from_matched_bytes_pin};
use crate::s7::parity::s7_parity_aggregate;
use crate::s7::report::{
    S7_REQUIRED_CLOSURE_ARTIFACTS, S7ArtifactHashStatus, S7ClosureArtifactKind,
    S7ClosureArtifactStatus, S7ClosureGateStatus, S7ClosureValidationInput,
    S7PerSeedClosureArtifacts, validate_s7_closure,
};
use crate::s7::schema::s7_score_report_from_val_bytes;

/// `s7.bytes.validate` smoke event.
pub const S7_SMOKE_BYTES_EVENT: &str = "s7.bytes.validate";
/// `s7.parity.aggregate` smoke event.
pub const S7_SMOKE_PARITY_EVENT: &str = "s7.parity.aggregate";
/// `s7.pareto.verdict` smoke event.
pub const S7_SMOKE_PARETO_EVENT: &str = "s7.pareto.verdict";
/// `s7.guardrail.decision` smoke event.
pub const S7_SMOKE_GUARDRAIL_EVENT: &str = "s7.guardrail.decision";
/// `s7.switch_stats.summary` smoke event.
pub const S7_SMOKE_SWITCH_STATS_EVENT: &str = "s7.switch_stats.summary";
/// `s7.outcome.dispatch` smoke event.
pub const S7_SMOKE_OUTCOME_EVENT: &str = "s7.outcome.dispatch";
/// `s7.closure.validate` smoke event.
pub const S7_SMOKE_CLOSURE_EVENT: &str = "s7.closure.validate";
/// State-transition event used as the high-level smoke narrative.
pub const S7_SMOKE_TRANSITION_EVENT: &str = "s7.prereg.transition";
/// Debug diagnostic emitted for parity failure roots.
pub const S7_SMOKE_PARITY_DIAGNOSTIC_EVENT: &str = "s7.parity.diagnostic";
/// Debug diagnostic emitted for matched-byte failure roots.
pub const S7_SMOKE_BYTES_DIAGNOSTIC_EVENT: &str = "s7.bytes.diagnostic";
/// Debug diagnostic emitted for collapse failure roots.
pub const S7_SMOKE_COLLAPSE_DIAGNOSTIC_EVENT: &str = "s7.guardrail.diagnostic";

/// Fixture schema id for the committed smoke transcript.
pub const S7_SMOKE_TRANSCRIPT_SCHEMA: &str = "s7_smoke_transcript.v1";
/// Fixture name pinned by bd-1ryn.
pub const S7_SMOKE_FIXTURE: &str = "tiny_v1";
/// Smoke transcript schema version carried by tracing events.
pub const S7_SMOKE_SCHEMA_VERSION: &str = "1";
/// Owner for real production S7 run/report adoption that this fixture does not claim.
pub const S7_SMOKE_REAL_PRODUCER_OWNER: &str = "bd-2v9r";

const TRAIN_STEP: u64 = 20_000;
const BASE_TRAIN_STEP: u64 = 16_000;
const FAIL_COLLAPSED_AT_STEP: u64 = 1_337;
const N_SEEDS: u64 = 5;
const FIXTURE_CORPUS: &[u8] = include_bytes!("../../tests/fixtures/s7_tiny_corpus.txt");
const FIXTURE_TOPOLOGY: &[u8] = include_bytes!("../../tests/fixtures/s7_tiny_topology.toml");

/// Result type used by the smoke harness.
pub type S7SmokeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

/// Tiny smoke scenario used by positive and negative-path tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S7SmokeScenario {
    /// Full pass-clean closure composition.
    PassClean,
    /// Valid matched bytes with one per-seed parity failure.
    FailParity,
    /// Matched-byte tolerance violation.
    FailBytes,
    /// Production-lambda router collapse.
    FailCollapse,
}

impl S7SmokeScenario {
    fn name(self) -> &'static str {
        match self {
            Self::PassClean => "pass_clean",
            Self::FailParity => "fail_parity",
            Self::FailBytes => "fail_bytes",
            Self::FailCollapse => "fail_collapse",
        }
    }
}

/// Built deterministic smoke run.
#[derive(Debug, Clone)]
pub struct S7SmokeRun {
    transcript: S7SmokeTranscript,
    report: String,
    dense_vs_moe: S7DenseVsMoeComparisonReport,
    collapse_sweep: RouterCollapseSweepReport,
}

impl S7SmokeRun {
    /// Return the transcript payload.
    #[must_use]
    pub const fn transcript(&self) -> &S7SmokeTranscript {
        &self.transcript
    }

    /// Return the human-readable smoke report.
    #[must_use]
    pub fn human_report(&self) -> &str {
        &self.report
    }

    /// Return the dense-vs-MoE artifact used by the smoke transcript.
    #[must_use]
    pub const fn dense_vs_moe(&self) -> &S7DenseVsMoeComparisonReport {
        &self.dense_vs_moe
    }

    /// Return the router-collapse sweep artifact used by the smoke transcript.
    #[must_use]
    pub const fn collapse_sweep(&self) -> &RouterCollapseSweepReport {
        &self.collapse_sweep
    }

    /// Pretty JSON transcript bytes with a trailing newline.
    pub fn transcript_json_pretty(&self) -> S7SmokeResult<Vec<u8>> {
        pretty_json_bytes(&self.transcript)
    }

    /// Canonical dense-vs-MoE JSON bytes with a trailing newline.
    pub fn dense_vs_moe_json_bytes(&self) -> S7SmokeResult<Vec<u8>> {
        bytes_with_newline(self.dense_vs_moe.canonical_json_bytes()?)
    }

    /// Canonical router-collapse-sweep JSON bytes with a trailing newline.
    pub fn collapse_sweep_json_bytes(&self) -> S7SmokeResult<Vec<u8>> {
        bytes_with_newline(self.collapse_sweep.canonical_json_bytes()?)
    }
}

/// Committed smoke transcript payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S7SmokeTranscript {
    /// Schema literal.
    pub schema: String,
    /// Fixture id.
    pub fixture: String,
    /// Scenario id.
    pub scenario: String,
    /// Claim-boundary owner for real production adoption.
    pub moved_real_producer_scope_to: String,
    /// Matched byte pin hash prefix.
    pub matched_bytes_self_hash_prefix: String,
    /// Dense-vs-MoE report hash prefix.
    pub dense_vs_moe_self_hash_prefix: String,
    /// Router-collapse sweep report hash prefix.
    pub collapse_sweep_self_hash_prefix: String,
    /// Aggregate parity verdict string.
    pub aggregate_parity: String,
    /// Pareto verdict string.
    pub pareto: String,
    /// H6 guardrail verdict string.
    pub guardrail: String,
    /// Production-lambda collapse step surfaced by the fixture failure path.
    pub collapsed_at_step: Option<u64>,
    /// Outcome string.
    pub outcome: String,
    /// Decision string.
    pub decision: String,
    /// Whether §15 closure validation accepted this tiny fixture envelope.
    pub closure_valid: bool,
    /// Human-readable closure validation summary.
    pub closure_reason: String,
    /// Transcript hash prefix used by log-shape tests.
    pub transcript_self_hash_prefix: String,
}

/// Build a deterministic smoke run and emit the structured tracing transcript.
pub fn run_s7_smoke(scenario: S7SmokeScenario) -> S7SmokeResult<S7SmokeRun> {
    let run = build_s7_smoke(scenario)?;
    emit_smoke_trace(&run)?;
    Ok(run)
}

/// Build a deterministic smoke run without emitting tracing events.
pub fn build_s7_smoke(scenario: S7SmokeScenario) -> S7SmokeResult<S7SmokeRun> {
    let mut pin = canonical_s7_matched_bytes_pin()?;
    if scenario == S7SmokeScenario::FailBytes {
        pin.b_deployed_total_dense = pin
            .b_deployed_total_moe
            .checked_add(pin.tolerance_bytes)
            .and_then(|value| value.checked_add(1))
            .ok_or("matched-byte fixture overflow")?;
        pin.b_dense_ffn_total = pin.b_deployed_total_dense;
        pin = pin.with_computed_self_hash()?;
    }

    let scores = score_reports(scenario)?;
    let per_seed = per_seed_comparisons(&scores)?;
    let per_seed_passes = per_seed
        .iter()
        .map(|entry| entry.parity_verdict.passed())
        .collect::<Vec<_>>();
    let bytes_diff = pin
        .b_deployed_total_dense
        .abs_diff(pin.b_deployed_total_moe);
    let aggregate_parity =
        s7_parity_aggregate(&per_seed_passes, bytes_diff, pin.tolerance_bytes)
            .map_err(|error| format!("s7 smoke parity aggregate failed: {error:?}"))?;
    let pareto = s7_pareto_verdict_from_matched_bytes_pin(
        median_bpc(&per_seed, |entry| entry.val_bpc_moe),
        median_bpc(&per_seed, |entry| entry.val_bpc_dense),
        &pin,
    )?;
    let collapse_sweep = collapse_sweep_report(scenario)?;
    let dense_vs_moe = dense_vs_moe_report(
        pin.clone(),
        per_seed,
        aggregate_parity,
        pareto,
        &collapse_sweep,
    )?;
    let guardrail = if scenario == S7SmokeScenario::FailCollapse {
        GuardrailVerdict::FailB
    } else {
        h6_guardrail_verdict(&collapse_sweep.records)?
    };

    let pareto_signals = s7_pareto_closure_signals(pareto);
    let outcome = dispatch_s7_outcome(S7OutcomeDispatchInput {
        moe_collapsed: scenario == S7SmokeScenario::FailCollapse,
        aggregate_parity_verdict: aggregate_parity,
        h3_refuted: matches!(aggregate_parity, AggregateParityVerdict::FailParity)
            || pareto_signals.h3_refuted,
        h4_refuted: pareto_signals.h4_refuted,
        h6_refuted: !matches!(guardrail, GuardrailVerdict::Pass),
        ..S7OutcomeDispatchInput::default()
    });
    let decision = decision_for_s7_outcome(outcome);
    let closure_input = closure_input(
        outcome,
        decision,
        &scores,
        &dense_vs_moe,
        &collapse_sweep,
        scenario,
    );
    let closure_result = validate_s7_closure(&closure_input);
    let (closure_valid, closure_reason) = match closure_result {
        Ok(_) => (true, "validated".to_owned()),
        Err(error) => (false, error.to_string()),
    };

    let transcript_hash_source = sha256(format!(
        "{}:{}:{}:{}:{}",
        scenario.name(),
        dense_vs_moe.comparison_self_hash,
        collapse_sweep.sweep_self_hash,
        outcome_name(outcome),
        decision_name(decision)
    ));
    let transcript = S7SmokeTranscript {
        schema: S7_SMOKE_TRANSCRIPT_SCHEMA.to_owned(),
        fixture: S7_SMOKE_FIXTURE.to_owned(),
        scenario: scenario.name().to_owned(),
        moved_real_producer_scope_to: S7_SMOKE_REAL_PRODUCER_OWNER.to_owned(),
        matched_bytes_self_hash_prefix: hash_prefix(pin.matched_bytes_self_hash),
        dense_vs_moe_self_hash_prefix: hash_prefix(dense_vs_moe.comparison_self_hash),
        collapse_sweep_self_hash_prefix: hash_prefix(collapse_sweep.sweep_self_hash),
        aggregate_parity: aggregate_parity_name(aggregate_parity).to_owned(),
        pareto: pareto_name(pareto).to_owned(),
        guardrail: guardrail_name(guardrail).to_owned(),
        collapsed_at_step: collapsed_at_step_for_scenario(scenario),
        outcome: outcome_name(outcome).to_owned(),
        decision: decision_name(decision).to_owned(),
        closure_valid,
        closure_reason,
        transcript_self_hash_prefix: hash_prefix(transcript_hash_source),
    };
    let report = human_report(&transcript);

    Ok(S7SmokeRun {
        transcript,
        report,
        dense_vs_moe,
        collapse_sweep,
    })
}

/// Write committed deterministic smoke artifacts under `root`.
pub fn write_s7_smoke_artifacts(root: &Path) -> S7SmokeResult<()> {
    let pass = build_s7_smoke(S7SmokeScenario::PassClean)?;
    std::fs::create_dir_all(root)?;
    std::fs::write(root.join("S7-smoke-report.md"), pass.human_report())?;
    std::fs::write(
        root.join("transcript.v1.json"),
        pass.transcript_json_pretty()?,
    )?;
    std::fs::write(
        root.join("s7_dense_vs_moe.v1.json"),
        pass.dense_vs_moe_json_bytes()?,
    )?;
    std::fs::write(
        root.join("s7_router_collapse_sweep.v1.json"),
        pass.collapse_sweep_json_bytes()?,
    )?;
    Ok(())
}

fn score_reports(scenario: S7SmokeScenario) -> S7SmokeResult<Vec<(S7ScoreReport, S7ScoreReport)>> {
    let corpus_sha = sha256(FIXTURE_CORPUS);
    let mut reports = Vec::new();
    for seed in 0..N_SEEDS {
        let moe_bpc = match (scenario, seed) {
            (S7SmokeScenario::FailParity, 2) => 1.11,
            _ => 1.00 + seed as f64 * 0.01,
        };
        let dense_bpc = 1.12 + seed as f64 * 0.01;
        reports.push((
            score_report(seed, S7Topology::MoeTiny, corpus_sha, moe_bpc)?,
            score_report(seed, S7Topology::MoeTinyDenseMatched, corpus_sha, dense_bpc)?,
        ));
    }
    Ok(reports)
}

fn score_report(
    seed: u64,
    topology: S7Topology,
    corpus_sha: Hash256,
    bpc: f64,
) -> S7SmokeResult<S7ScoreReport> {
    let token_count = crate::s7::schema::charset_v1_normalized_token_count(FIXTURE_CORPUS)?;
    let checkpoint_sha = sha256(format!("s7-smoke-checkpoint:{topology:?}:{seed}"));
    Ok(s7_score_report_from_val_bytes(
        seed,
        topology,
        checkpoint_sha,
        corpus_sha,
        FIXTURE_CORPUS,
        bpc * token_count as f64,
    )?)
}

fn per_seed_comparisons(
    scores: &[(S7ScoreReport, S7ScoreReport)],
) -> S7SmokeResult<Vec<S7PerSeedComparison>> {
    scores
        .iter()
        .map(|(moe, dense)| {
            let delta = dense.bpc - moe.bpc;
            let parity = if moe.bpc < dense.bpc - 0.05 {
                S7ParityVerdict::Pass
            } else {
                S7ParityVerdict::Fail
            };
            Ok(S7PerSeedComparison::new(
                moe.seed, moe.bpc, dense.bpc, delta, parity,
            )?)
        })
        .collect()
}

fn dense_vs_moe_report(
    pin: MatchedBytesPin,
    per_seed: Vec<S7PerSeedComparison>,
    aggregate_parity: AggregateParityVerdict,
    pareto: ParetoVerdict,
    collapse_sweep: &RouterCollapseSweepReport,
) -> S7SmokeResult<S7DenseVsMoeComparisonReport> {
    Ok(S7DenseVsMoeComparisonReport::new(
        sha256(FIXTURE_TOPOLOGY),
        sha256(b"s7-smoke-dense-matched-topology"),
        pin.clone(),
        per_seed.clone(),
        median_bpc(&per_seed, |entry| entry.val_bpc_moe),
        median_bpc(&per_seed, |entry| entry.val_bpc_dense),
        pin.b_deployed_total_moe,
        pin.b_deployed_total_dense,
        checked_signed_bytes_diff(pin.b_deployed_total_dense, pin.b_deployed_total_moe)?,
        pin.b_deployed_total_dense
            .abs_diff(pin.b_deployed_total_moe)
            <= pin.tolerance_bytes,
        artifact_aggregate_parity(aggregate_parity),
        pareto,
        smoke_switch_stats_summary()?,
        sweep_summary_from_report(collapse_sweep)?,
    )?
    .with_computed_self_hash()?)
}

fn collapse_sweep_report(scenario: S7SmokeScenario) -> S7SmokeResult<RouterCollapseSweepReport> {
    let input = LambdaSwitchSweepInput::d11_from_val_eval_subset_bytes(
        D11_LAMBDA_SWITCH_SWEEP_SEED,
        sha256(b"s7-smoke-phase-d-checkpoint"),
        BASE_TRAIN_STEP,
        FIXTURE_CORPUS,
    )?;
    let report = run_lambda_switch_sweep(&input, &DeterministicFixtureSweepProducer)?;
    if scenario == S7SmokeScenario::FailCollapse {
        // Keep the real D11 sweep artifact intact; the production-lambda
        // collapse path is represented in the closure rows and diagnostic
        // event because real producer adoption belongs to bd-2v9r.
        return Ok(report);
    }
    Ok(report)
}

fn closure_input(
    outcome: S7Outcome,
    decision: S7Decision,
    scores: &[(S7ScoreReport, S7ScoreReport)],
    dense_vs_moe: &S7DenseVsMoeComparisonReport,
    collapse_sweep: &RouterCollapseSweepReport,
    scenario: S7SmokeScenario,
) -> S7ClosureValidationInput {
    let mut required_artifacts = S7_REQUIRED_CLOSURE_ARTIFACTS
        .iter()
        .map(|kind| S7ClosureArtifactStatus {
            kind: *kind,
            status: S7ArtifactHashStatus::present_valid(match kind {
                S7ClosureArtifactKind::DenseVsMoe => dense_vs_moe.comparison_self_hash,
                S7ClosureArtifactKind::RouterCollapseSweep => collapse_sweep.sweep_self_hash,
                _ => sha256(kind.field_name()),
            }),
        })
        .collect::<Vec<_>>();
    if outcome == S7Outcome::FailParity {
        required_artifacts.push(S7ClosureArtifactStatus {
            kind: S7ClosureArtifactKind::EmulatorOneTokenDense,
            status: S7ArtifactHashStatus::present_valid(sha256(
                S7ClosureArtifactKind::EmulatorOneTokenDense.field_name(),
            )),
        });
    }

    S7ClosureValidationInput {
        outcome,
        decision,
        bytes_within_tolerance: dense_vs_moe.bytes_within_tolerance,
        per_seed_bpc_parity_failed: dense_vs_moe
            .per_seed
            .iter()
            .any(|entry| !entry.parity_verdict.passed()),
        predictions_verified: true,
        gates: S7ClosureGateStatus {
            h6_router_collapse_guardrail_confirmed: scenario != S7SmokeScenario::FailCollapse,
            ..S7ClosureGateStatus::all_confirmed()
        },
        per_seed_artifacts: per_seed_artifacts(scores, scenario),
        required_artifacts,
    }
}

fn per_seed_artifacts(
    scores: &[(S7ScoreReport, S7ScoreReport)],
    scenario: S7SmokeScenario,
) -> Vec<S7PerSeedClosureArtifacts> {
    let mut rows = Vec::new();
    for (moe, dense) in scores {
        rows.push(S7PerSeedClosureArtifacts {
            seed: moe.seed,
            topology: S7Topology::MoeTiny,
            completion: if let Some(step) = collapsed_at_step_for_scenario(scenario)
                && moe.seed == 0
            {
                S7Completion::CollapsedAt { step }
            } else {
                S7Completion::Completed
            },
            checkpoint_self_hash: S7ArtifactHashStatus::present_valid(moe.checkpoint_sha),
            run_log_self_hash: S7ArtifactHashStatus::present_valid(sha256(format!(
                "s7-smoke-run-log:MoeTiny:{}",
                moe.seed
            ))),
            score_self_hash: S7ArtifactHashStatus::present_valid(moe.score_self_hash),
        });
        rows.push(S7PerSeedClosureArtifacts {
            seed: dense.seed,
            topology: S7Topology::MoeTinyDenseMatched,
            completion: S7Completion::Completed,
            checkpoint_self_hash: S7ArtifactHashStatus::present_valid(dense.checkpoint_sha),
            run_log_self_hash: S7ArtifactHashStatus::present_valid(sha256(format!(
                "s7-smoke-run-log:MoeTinyDenseMatched:{}",
                dense.seed
            ))),
            score_self_hash: S7ArtifactHashStatus::present_valid(dense.score_self_hash),
        });
    }
    rows
}

fn emit_smoke_trace(run: &S7SmokeRun) -> S7SmokeResult<()> {
    let common = EventCommon::new(run);
    emit_transition(&common, "BaselineMatched", "TrainAttempted");
    emit_verdict(
        &common,
        S7_SMOKE_BYTES_EVENT,
        if run.dense_vs_moe.bytes_within_tolerance {
            "Pass"
        } else {
            "Fail"
        },
        "matched deployed byte tolerance",
        &format!(
            "bytes_diff={} tolerance={}",
            run.dense_vs_moe.bytes_diff.abs(),
            run.dense_vs_moe.matched_bytes_pin.tolerance_bytes
        ),
    );
    emit_transition(&common, "TrainAttempted", "ParityAggregated");
    emit_verdict(
        &common,
        S7_SMOKE_PARITY_EVENT,
        &run.transcript.aggregate_parity,
        "per-seed production bpc margin",
        &format!("per_seed={:?}", run.dense_vs_moe.per_seed),
    );
    emit_transition(&common, "ParityAggregated", "ParetoChecked");
    emit_verdict(
        &common,
        S7_SMOKE_PARETO_EVENT,
        &run.transcript.pareto,
        "median bpc plus deployed bytes",
        &format!(
            "moe_bpc={} dense_bpc={} moe_bytes={} dense_bytes={}",
            run.dense_vs_moe.median_val_bpc_moe,
            run.dense_vs_moe.median_val_bpc_dense,
            run.dense_vs_moe.deployed_bytes_total_moe,
            run.dense_vs_moe.deployed_bytes_total_dense
        ),
    );
    emit_transition(&common, "ParetoChecked", "GuardrailChecked");
    emit_verdict(
        &common,
        S7_SMOKE_GUARDRAIL_EVENT,
        &run.transcript.guardrail,
        "D11 collapse sweep A/B/C/D",
        &format!("sweep_self_hash={}", run.collapse_sweep.sweep_self_hash),
    );
    emit_verdict(
        &common,
        S7_SMOKE_SWITCH_STATS_EVENT,
        "Pass",
        "digest-only tiny switch stats replay",
        "temporal_switch_digest replayed; aggregate owner bd-2v9r",
    );
    emit_transition(&common, "GuardrailChecked", "OutcomeDispatched");
    emit_verdict(
        &common,
        S7_SMOKE_OUTCOME_EVENT,
        &run.transcript.outcome,
        &run.transcript.decision,
        &format!(
            "aggregate_parity={} pareto={} guardrail={}",
            run.transcript.aggregate_parity, run.transcript.pareto, run.transcript.guardrail
        ),
    );
    emit_transition(&common, "OutcomeDispatched", "ClosureValidated");
    emit_verdict(
        &common,
        S7_SMOKE_CLOSURE_EVENT,
        if run.transcript.closure_valid {
            "Pass"
        } else {
            "Fail"
        },
        &run.transcript.closure_reason,
        "§15 fixture closure envelope",
    );

    // The tiny harness emits root diagnostics only for the failure variants it
    // synthesizes honestly. Pareto and switch-stat producer diagnostics remain
    // with the real S7 producer/report owner, bd-2v9r.
    if !run.dense_vs_moe.bytes_within_tolerance {
        emit_diagnostic(
            &common,
            S7_SMOKE_BYTES_DIAGNOSTIC_EVENT,
            "bytes tolerance exceeded; DenseOnly is blocked by §21 line 12",
            run,
        )?;
    } else if run.transcript.aggregate_parity == "Fail-parity" {
        emit_diagnostic(
            &common,
            S7_SMOKE_PARITY_DIAGNOSTIC_EVENT,
            "per-seed bpc margin failed under valid matched bytes",
            run,
        )?;
    } else if run.transcript.outcome == "Fail-router-collapse" {
        emit_diagnostic(
            &common,
            S7_SMOKE_COLLAPSE_DIAGNOSTIC_EVENT,
            "production lambda collapsed in tiny fixture path",
            run,
        )?;
    }
    Ok(())
}

fn emit_transition(common: &EventCommon, from_state: &'static str, to_state: &'static str) {
    tracing::info!(
        target: S7_LOG_TARGET,
        event_name = S7_SMOKE_TRANSITION_EVENT,
        topology = common.topology,
        seed = common.seed,
        train_step = common.train_step,
        self_hash_prefix = common.self_hash_prefix,
        schema_version = S7_SMOKE_SCHEMA_VERSION,
        from_state = from_state,
        to_state = to_state,
        fixture = S7_SMOKE_FIXTURE,
        "s7 smoke transition"
    );
}

fn emit_verdict(
    common: &EventCommon,
    event_name: &'static str,
    verdict: &str,
    reason: &str,
    inputs: &str,
) {
    tracing::info!(
        target: S7_LOG_TARGET,
        event_name = event_name,
        topology = common.topology,
        seed = common.seed,
        train_step = common.train_step,
        self_hash_prefix = common.self_hash_prefix,
        schema_version = S7_SMOKE_SCHEMA_VERSION,
        verdict = verdict,
        reason = reason,
        inputs = inputs,
        "s7 smoke verdict"
    );
}

fn emit_diagnostic(
    common: &EventCommon,
    event_name: &'static str,
    reason: &str,
    run: &S7SmokeRun,
) -> S7SmokeResult<()> {
    let diagnostics = RawLossDiagnostics::new(
        1.0,
        DistillRawDiagnostic::NotAvailable {
            reason: "tiny_fixture_no_frozen_teacher".to_owned(),
            phase: TrainPhase::PhaseA,
        },
        0.02,
        0.01,
        0.25,
    )?
    .with_computed_self_hash()?;
    let raw_loss_diagnostics = String::from_utf8(diagnostics.canonical_json_bytes()?)?;
    let router_grad_norms = GradNormSummary::new(0.75, 0.42, 0.10)?;
    let n_experts = fixture_n_experts()?;
    tracing::debug!(
        target: S7_LOG_TARGET,
        event_name = event_name,
        topology = common.topology,
        seed = common.seed,
        train_step = common.train_step,
        self_hash_prefix = common.self_hash_prefix,
        schema_version = S7_SMOKE_SCHEMA_VERSION,
        verdict = "Fail",
        reason,
        inputs = format!(
            "outcome={} decision={}",
            run.transcript.outcome, run.transcript.decision
        ),
        n_tokens = crate::s7::schema::charset_v1_normalized_token_count(FIXTURE_CORPUS)?,
        n_experts = n_experts,
        routing_distribution_digest = %sha256(format!("s7-smoke-routing-distribution:{n_experts}")),
        raw_l_switch = diagnostics.switch_loss_raw,
        router_grad_norm_l2 = router_grad_norms.global_l2,
        balance_loss_raw = diagnostics.balance_loss_raw,
        zrouter_loss_raw = diagnostics.zrouter_loss_raw,
        distill_loss_raw = "NotAvailable(tiny_fixture_no_frozen_teacher)",
        conformance_gap_diagnostic = "teacher_high_switch_vs_student_smooth:not_exercised_fixture_only",
        raw_loss_diagnostics,
        moved_real_producer_scope_to = S7_SMOKE_REAL_PRODUCER_OWNER,
        "s7 smoke diagnostic"
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct EventCommon {
    topology: &'static str,
    seed: u64,
    train_step: u64,
    self_hash_prefix: String,
}

impl EventCommon {
    fn new(run: &S7SmokeRun) -> Self {
        Self {
            topology: "MoeTiny",
            seed: 0,
            train_step: TRAIN_STEP,
            self_hash_prefix: run.transcript.transcript_self_hash_prefix.clone(),
        }
    }
}

fn human_report(transcript: &S7SmokeTranscript) -> String {
    format!(
        "\
S7 SMOKE - fixture={} pass_version={}
  H1 MoE-train .................. Fixture [synthetic score path topology=MoeTiny seed=0]
  H2 Dense-train ................ Fixture [synthetic score path topology=MoeTinyDenseMatched seed=0]
  H3 Parity gate ................ {} [{}]
  H4 Pareto ..................... {} [{}]
  H5 Switch-stats ............... Carried [fixture digest; aggregate producer owner bd-2v9r]
  H6 Guardrail .................. {} [{}{}]
  H7 Gradient provenance ........ Moved [diagnostic fields captured; full evidence bd-2v9r]
  H8 Burn ExpertBlockQat grad ... Moved [bd-2v9r]
  H9 Oracle (routed FFN) ........ Moved [bd-2v9r]
  H10 Emulator one-token ........ Separate [integration_s7 H10 schema test; not smoke-measured]
Outcome: {} -> Decision: {}
Scope: production Gutenberg training/report adoption remains with {}
Diagnostics: DEBUG roots are fixture-limited to bytes/parity/collapse; Pareto and switch-stats producer diagnostics remain with {}
",
        transcript.fixture,
        S7_SMOKE_SCHEMA_VERSION,
        confirmed_word(transcript.aggregate_parity == "Pass-clean"),
        transcript.aggregate_parity,
        confirmed_word(transcript.pareto == "MoE-dominates"),
        transcript.pareto,
        confirmed_word(transcript.guardrail == "Pass"),
        transcript.guardrail,
        collapsed_at_report_suffix(transcript.collapsed_at_step),
        transcript.outcome,
        transcript.decision,
        S7_SMOKE_REAL_PRODUCER_OWNER,
        S7_SMOKE_REAL_PRODUCER_OWNER
    )
}

fn pretty_json_bytes(value: &impl Serialize) -> S7SmokeResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn bytes_with_newline(mut bytes: Vec<u8>) -> S7SmokeResult<Vec<u8>> {
    bytes.push(b'\n');
    Ok(bytes)
}

fn artifact_aggregate_parity(verdict: AggregateParityVerdict) -> S7AggregateParityVerdict {
    match verdict {
        AggregateParityVerdict::PassClean => S7AggregateParityVerdict::PassClean,
        AggregateParityVerdict::FailParity => S7AggregateParityVerdict::FailParity,
        AggregateParityVerdict::FailBytes => S7AggregateParityVerdict::FailBytes,
    }
}

fn fixture_n_experts() -> S7SmokeResult<u64> {
    let topology = std::str::from_utf8(FIXTURE_TOPOLOGY)?;
    let value: toml::Value = toml::from_str(topology)?;
    let n_experts = value
        .get("moe_tiny")
        .and_then(|table| table.get("n_experts"))
        .and_then(toml::Value::as_integer)
        .ok_or("s7_tiny_topology.toml must define moe_tiny.n_experts")?;
    if n_experts <= 0 {
        return Err("s7_tiny_topology.toml moe_tiny.n_experts must be positive".into());
    }
    Ok(u64::try_from(n_experts)?)
}

const fn collapsed_at_step_for_scenario(scenario: S7SmokeScenario) -> Option<u64> {
    match scenario {
        S7SmokeScenario::FailCollapse => Some(FAIL_COLLAPSED_AT_STEP),
        S7SmokeScenario::PassClean | S7SmokeScenario::FailParity | S7SmokeScenario::FailBytes => {
            None
        }
    }
}

fn collapsed_at_report_suffix(collapsed_at_step: Option<u64>) -> String {
    collapsed_at_step
        .map(|step| format!("; CollapsedAt(step={step})"))
        .unwrap_or_default()
}

fn smoke_switch_stats_summary() -> S7SmokeResult<SwitchStatsSummary> {
    Ok(SwitchStatsSummary::new(vec![256, 128, 64, 0], 1.75, 0.5)?)
}

fn sweep_summary_from_report(report: &RouterCollapseSweepReport) -> S7SmokeResult<SweepSummary> {
    let mut bpc_at_lambda = BTreeMap::new();
    let mut entropy_at_lambda = BTreeMap::new();

    for record in &report.records {
        let lambda_switch = LambdaSwitch::new(record.lambda_switch.to_string())?;
        if let Some(bpc) = record.bpc_eval_subset {
            bpc_at_lambda.insert(lambda_switch.clone(), bpc);
        }
        entropy_at_lambda.insert(lambda_switch, record.expert_usage_entropy_bits_mean);
    }

    Ok(SweepSummary::new(
        bpc_at_lambda,
        entropy_at_lambda,
        artifact_guardrail_verdict(report.guardrail_verdict)?,
    )?)
}

fn artifact_guardrail_verdict(
    verdict: GuardrailVerdict,
) -> S7SmokeResult<ArtifactGuardrailVerdict> {
    Ok(match verdict {
        GuardrailVerdict::Pass => ArtifactGuardrailVerdict::Pass,
        GuardrailVerdict::FailA => ArtifactGuardrailVerdict::FailA,
        GuardrailVerdict::FailB => ArtifactGuardrailVerdict::FailB,
        GuardrailVerdict::FailC => ArtifactGuardrailVerdict::FailC,
        GuardrailVerdict::FailD => ArtifactGuardrailVerdict::FailD,
        GuardrailVerdict::InconclusiveDiverged {
            lambda_switch,
            step,
        } => ArtifactGuardrailVerdict::InconclusiveDiverged {
            lambda_switch: LambdaSwitch::new(lambda_switch.to_string())?,
            step,
        },
    })
}

fn median_bpc(per_seed: &[S7PerSeedComparison], f: impl Fn(&S7PerSeedComparison) -> f64) -> f64 {
    let mut values = per_seed.iter().map(f).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn checked_signed_bytes_diff(dense: u64, moe: u64) -> S7SmokeResult<i64> {
    let diff = i128::from(dense) - i128::from(moe);
    Ok(i64::try_from(diff)?)
}

fn hash_prefix(hash: Hash256) -> String {
    hash.to_hex().chars().take(12).collect()
}

fn aggregate_parity_name(verdict: AggregateParityVerdict) -> &'static str {
    match verdict {
        AggregateParityVerdict::PassClean => "Pass-clean",
        AggregateParityVerdict::FailParity => "Fail-parity",
        AggregateParityVerdict::FailBytes => "Fail-bytes",
    }
}

fn pareto_name(verdict: ParetoVerdict) -> &'static str {
    match verdict {
        ParetoVerdict::MoeDominates => "MoE-dominates",
        ParetoVerdict::DenseDominates => "dense-dominates",
        ParetoVerdict::MoeWinsUnderByteEquivalence => "MoE-wins-under-byte-equivalence",
        ParetoVerdict::DenseWinsUnderByteEquivalence => "Dense-wins-under-byte-equivalence",
        ParetoVerdict::Incomparable => "Incomparable",
        ParetoVerdict::Tied => "Tied",
    }
}

fn guardrail_name(verdict: GuardrailVerdict) -> String {
    match verdict {
        GuardrailVerdict::Pass => "Pass".to_owned(),
        GuardrailVerdict::FailA => "FailA".to_owned(),
        GuardrailVerdict::FailB => "FailB".to_owned(),
        GuardrailVerdict::FailC => "FailC".to_owned(),
        GuardrailVerdict::FailD => "FailD".to_owned(),
        GuardrailVerdict::InconclusiveDiverged {
            lambda_switch,
            step,
        } => {
            format!("InconclusiveDiverged(lambda_switch={lambda_switch}, step={step})")
        }
    }
}

fn outcome_name(outcome: S7Outcome) -> &'static str {
    match outcome {
        S7Outcome::PassClean => "Pass-clean",
        S7Outcome::FailMoeTrain => "Fail-moe-train",
        S7Outcome::FailRouterCollapse => "Fail-router-collapse",
        S7Outcome::FailDenseBaseline => "Fail-dense-baseline",
        S7Outcome::FailParity => "Fail-parity",
        S7Outcome::FailBytes => "Fail-bytes",
        S7Outcome::FailPareto => "Fail-pareto",
        S7Outcome::FailSwitchStats => "Fail-switch-stats",
        S7Outcome::FailRouterCollapseGuardrail => "Fail-router-collapse-guardrail",
        S7Outcome::FailGradProvenance => "Fail-grad-provenance",
        S7Outcome::FailBurnGrad => "Fail-burn-grad",
        S7Outcome::FailOracleRouted => "Fail-oracle-routed",
        S7Outcome::FailEmulatorRouted => "Fail-emulator-routed",
        S7Outcome::FailSuspicious => "Fail-suspicious",
    }
}

fn decision_name(decision: S7Decision) -> String {
    match decision {
        S7Decision::ProceedToS8 => "ProceedToS8".to_owned(),
        S7Decision::ProceedToS8DenseOnly => "ProceedToS8-DenseOnly".to_owned(),
        S7Decision::Investigate { reason } => format!("Investigate({reason})"),
        S7Decision::Halt { reason } => format!("Halt({reason})"),
    }
}

fn confirmed_word(confirmed: bool) -> &'static str {
    if confirmed { "Confirmed" } else { "Refuted" }
}

impl fmt::Display for S7SmokeScenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
