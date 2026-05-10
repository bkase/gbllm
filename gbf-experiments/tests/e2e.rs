//! F-S1.36 tiny-fixture end-to-end composition harness.
//!
//! This target is intentionally fixture-only. It verifies that the S1 outcome
//! dispatcher, structured logging, artifact schemas, oracle/ablation/report
//! emitters, and checked golden files compose for every S1Outcome variant. It
//! does not replace the full TinyStories closure run for bd-12pl.

mod common;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType,
};
use gbf_experiments::s1::ablation::{AblationCheckpoint, compare};
use gbf_experiments::s1::logging::{
    DivergenceObserved, E2ePhaseEvent, E2eScenarioCompleteEvent, E2eScenarioStartEvent,
    RunDivergenceEvent, S1LogEmitter, event,
};
use gbf_experiments::s1::manifest::{load_train_bytes, load_val_bytes, read_tinystories_manifest};
use gbf_experiments::s1::neg_test::negative_test_report_from_bpcs;
use gbf_experiments::s1::oracle::{MetricOracleResults, emit_oracle_report};
use gbf_experiments::s1::report::{
    Hypothesis, HypothesisFinding, HypothesisStatus, ObservedSeed, OutcomeDispatchInput,
    ReportFile, ReportInput, dispatch_outcome, predictions_section_hash,
};
use gbf_experiments::s1::run::TrainConfig;
use gbf_experiments::s1::schema::{
    AblationReport, BaselineReport, CheckpointMetadata, CountsSummary, GitCommitId,
    GradNormSummary, NegativeTestReport, OracleReport, PerSeedArtifacts, ReportFrontMatter,
    RfcRevisionRef, RunLog, S1BuildKind, S1CanonicalJson, S1Completion, S1Decision, S1Outcome,
    ScoreReport, SmoothingScheme,
};
use gbf_foundation::{Hash256, SemVer, sha256};
use pretty_assertions::assert_eq;
use serde::Serialize;
use serde_json::{Value, json};

const MANIFEST: &str = "gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml";
const GOLDEN_ROOT: &str = "gbf-experiments/tests/e2e/golden";
const PREDICTIONS: &str = "Fixture predictions are pre-registered for the S1 E2E harness.";

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[test]
fn all_s1_outcome_scenarios_match_goldens_and_structured_logs() -> TestResult {
    let fixture = FixtureInputs::load()?;
    let scenario_filter = std::env::var("S1_E2E_SCENARIO").ok();
    let mut matched = 0_u64;
    for scenario in scenarios() {
        if scenario_filter
            .as_deref()
            .is_some_and(|filter| filter != scenario.name)
        {
            continue;
        }
        matched += 1;
        let rendered = render_scenario(scenario.clone(), &fixture)?;
        assert_scenario_golden(scenario.name, &rendered)?;
    }
    if let Some(filter) = scenario_filter {
        assert!(matched > 0, "unknown S1_E2E_SCENARIO={filter}");
    }
    Ok(())
}

#[test]
fn golden_comparison_reports_byte_drift() {
    let error = golden_diff_message("scenario/thing.json", b"{\"a\":1}\n", b"{\"a\":2}\n")
        .expect_err("different golden bytes should fail");
    assert!(
        error.contains("golden drift for scenario/thing.json"),
        "{error}"
    );
    assert!(
        error.contains("expected 8 bytes, actual 8 bytes"),
        "{error}"
    );
    assert!(error.contains("first difference at byte 5"), "{error}");
    assert!(error.contains("line 1, column 6"), "{error}");
    assert!(
        error.contains("expected line: \"{\\\"a\\\":1}\""),
        "{error}"
    );
    assert!(error.contains("actual line: \"{\\\"a\\\":2}\""), "{error}");
}

#[derive(Clone, Debug)]
struct ScenarioSpec {
    name: &'static str,
    outcome: S1Outcome,
    h1: HypothesisStatus,
    h2: HypothesisStatus,
    h3: HypothesisStatus,
    h4: HypothesisStatus,
    h5: HypothesisStatus,
    any_seed_diverged: bool,
    suspicious_low_bpc: bool,
    bpc: f64,
    neg_delta: f64,
    phase_a_eq_ablation: bool,
    oracle_results: MetricOracleResults,
    divergence: Option<(u64, u64, DivergenceObserved)>,
}

#[derive(Debug)]
struct FixtureInputs {
    train_sha: Hash256,
    val_sha: Hash256,
    shuffled_val_sha: Hash256,
    train_len: u64,
    val_len: u64,
}

#[derive(Debug)]
struct RenderedScenario {
    files: BTreeMap<PathBuf, Vec<u8>>,
}

#[derive(Debug)]
struct ScenarioArtifacts {
    baseline: BaselineReport,
    checkpoints: Vec<Option<CheckpointMetadata>>,
    run_logs: Vec<Option<RunLog>>,
    scores: Vec<Option<ScoreReport>>,
    negative: NegativeTestReport,
    ablation: AblationReport,
    oracle: OracleReport,
    report: ReportFile,
}

fn scenarios() -> Vec<ScenarioSpec> {
    let confirmed = HypothesisStatus::Confirmed;
    vec![
        ScenarioSpec {
            name: "pass_clean",
            outcome: S1Outcome::PassClean,
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: confirmed.clone(),
            h5: confirmed.clone(),
            any_seed_diverged: false,
            suspicious_low_bpc: false,
            bpc: 1.70,
            neg_delta: 2.50,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: None,
        },
        ScenarioSpec {
            name: "pass_with_warning",
            outcome: S1Outcome::PassWithWarning,
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: HypothesisStatus::Refuted,
            h4: confirmed.clone(),
            h5: confirmed.clone(),
            any_seed_diverged: false,
            suspicious_low_bpc: false,
            bpc: 1.72,
            neg_delta: 0.25,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: None,
        },
        ScenarioSpec {
            name: "fail_substrate_nan",
            outcome: S1Outcome::FailSubstrate,
            h1: HypothesisStatus::Refuted,
            h2: not_reached("H1 diverged before scoring"),
            h3: not_reached("H1 diverged before negative testing"),
            h4: not_reached("H1 diverged before ablation"),
            h5: confirmed.clone(),
            any_seed_diverged: true,
            suspicious_low_bpc: false,
            bpc: 1.80,
            neg_delta: 2.25,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: Some((2, 17, DivergenceObserved::NonFiniteLoss)),
        },
        ScenarioSpec {
            name: "fail_substrate_zero_grad",
            outcome: S1Outcome::FailSubstrate,
            h1: HypothesisStatus::Refuted,
            h2: not_reached("H1 zero-gradient hook stopped scoring"),
            h3: not_reached("H1 zero-gradient hook stopped negative testing"),
            h4: not_reached("H1 zero-gradient hook stopped ablation"),
            h5: confirmed.clone(),
            any_seed_diverged: true,
            suspicious_low_bpc: false,
            bpc: 1.81,
            neg_delta: 2.20,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: Some((1, 23, DivergenceObserved::ZeroGrad)),
        },
        ScenarioSpec {
            name: "fail_capacity_toytiny",
            outcome: S1Outcome::FailCapacity,
            h1: confirmed.clone(),
            h2: HypothesisStatus::Refuted,
            h3: confirmed.clone(),
            h4: confirmed.clone(),
            h5: confirmed.clone(),
            any_seed_diverged: false,
            suspicious_low_bpc: false,
            bpc: 2.40,
            neg_delta: 2.30,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: None,
        },
        ScenarioSpec {
            name: "fail_suspicious_low_bpc",
            outcome: S1Outcome::FailSuspicious,
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: confirmed.clone(),
            h5: confirmed.clone(),
            any_seed_diverged: false,
            suspicious_low_bpc: true,
            bpc: 0.40,
            neg_delta: 2.10,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: None,
        },
        ScenarioSpec {
            name: "fail_phase_ternary_leak",
            outcome: S1Outcome::FailPhase,
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: HypothesisStatus::Refuted,
            h5: confirmed.clone(),
            any_seed_diverged: false,
            suspicious_low_bpc: false,
            bpc: 1.75,
            neg_delta: 2.15,
            phase_a_eq_ablation: false,
            oracle_results: oracle_results(true, true, true, true, true),
            divergence: None,
        },
        ScenarioSpec {
            name: "fail_metric_modulo_shuffle",
            outcome: S1Outcome::FailMetric,
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: confirmed,
            h5: HypothesisStatus::Refuted,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
            bpc: 1.76,
            neg_delta: 2.05,
            phase_a_eq_ablation: true,
            oracle_results: oracle_results(true, true, true, true, false),
            divergence: None,
        },
    ]
}

impl FixtureInputs {
    fn load() -> TestResult<Self> {
        let manifest = read_tinystories_manifest(repo_root().join(MANIFEST))?;
        let train = load_train_bytes(&manifest)?;
        let val = load_val_bytes(&manifest)?;
        Ok(Self {
            train_sha: manifest.train_sha256,
            val_sha: manifest.val_sha256,
            shuffled_val_sha: manifest
                .val_shuffle_deadeef_sha256
                .expect("tiny fixture manifest pins shuffled validation hash"),
            train_len: train.len() as u64,
            val_len: val.len() as u64,
        })
    }
}

fn render_scenario(
    scenario: ScenarioSpec,
    fixture: &FixtureInputs,
) -> TestResult<RenderedScenario> {
    let capture = TraceCapture::default();
    let artifacts = with_trace_capture(&capture, || produce_scenario(scenario.clone(), fixture))?;
    let event_log = selected_event_log(&captured_events(&capture));

    let mut files = BTreeMap::new();
    insert_json(&mut files, "s1_baseline.v1.json", &artifacts.baseline)?;
    insert_json(&mut files, "s1_oracle.v1.json", &artifacts.oracle)?;
    insert_json(&mut files, "structured_events.v1.json", &event_log)?;
    insert_bytes(
        &mut files,
        "S1-report.md",
        artifacts.report.to_markdown()?.into_bytes(),
    );
    for seed in 0..5 {
        if let Some(metadata) = &artifacts.checkpoints[seed] {
            insert_json(
                &mut files,
                format!("seed-{seed}/s1_checkpoint.v1.metadata.json"),
                metadata,
            )?;
        }
        if let Some(run_log) = &artifacts.run_logs[seed] {
            insert_json(
                &mut files,
                format!("seed-{seed}/s1_run_log.v1.json"),
                run_log,
            )?;
        }
        if let Some(score) = &artifacts.scores[seed] {
            insert_json(&mut files, format!("seed-{seed}/s1_score.v1.json"), score)?;
        }
    }
    insert_json(
        &mut files,
        "seed-0/s1_negative_test.v1.json",
        &artifacts.negative,
    )?;
    insert_json(
        &mut files,
        "seed-0/s1_ablation.v1.json",
        &artifacts.ablation,
    )?;

    Ok(RenderedScenario { files })
}

fn produce_scenario(
    scenario: ScenarioSpec,
    fixture: &FixtureInputs,
) -> TestResult<ScenarioArtifacts> {
    let emitter = S1LogEmitter::new();
    let span = emitter.e2e_scenario_span(scenario.name);
    let _guard = span.enter();

    emitter.e2e_scenario_start(&E2eScenarioStartEvent {
        scenario: scenario.name.to_owned(),
        budget_profile: "integration_fixture".to_owned(),
        n_seeds: 5,
    })?;

    phase(&emitter, scenario.name, "baseline")?;
    let baseline = baseline_report(fixture)?;

    phase(&emitter, scenario.name, "train")?;
    if let Some((seed, step, observed)) = scenario.divergence {
        emitter.run_divergence(RunDivergenceEvent {
            seed,
            step,
            observed,
            last_finite_loss: 0.75,
        })?;
    }
    let completions = completions_for(&scenario);
    let checkpoints = (0..5)
        .map(|seed| checkpoint_for_seed(seed as u64, fixture, completions[seed].clone()))
        .collect::<Result<Vec<_>, _>>()?;
    let run_logs = (0..5)
        .map(|seed| run_log_for_seed(seed as u64, completions[seed].clone()))
        .collect::<Result<Vec<_>, _>>()?;

    phase(&emitter, scenario.name, "score")?;
    let scores = (0..5)
        .map(|seed| {
            score_report(
                seed as u64,
                fixture,
                scenario.bpc + seed as f64 * 0.01,
                checkpoints[seed]
                    .as_ref()
                    .map(|metadata| metadata.checkpoint_self_hash),
                completions[seed].clone(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    phase(&emitter, scenario.name, "neg-test")?;
    let checkpoint_sha = checkpoints[0]
        .as_ref()
        .map(|metadata| metadata.checkpoint_self_hash)
        .unwrap_or(Hash256::ZERO);
    let negative = negative_test_report_from_bpcs(
        0,
        checkpoint_sha,
        fixture.val_sha,
        fixture.shuffled_val_sha,
        scenario.bpc,
        scenario.bpc + scenario.neg_delta,
    )?;

    phase(&emitter, scenario.name, "ablation")?;
    let ablation = ablation_report(fixture, scenario.phase_a_eq_ablation)?;

    phase(&emitter, scenario.name, "oracle")?;
    let oracle = emit_oracle_report(0, scenario.oracle_results)?;

    phase(&emitter, scenario.name, "report")?;
    let dispatch = dispatch_outcome(&OutcomeDispatchInput {
        h1: scenario.h1.clone(),
        h2: scenario.h2.clone(),
        h3: scenario.h3.clone(),
        h4: scenario.h4.clone(),
        h5: scenario.h5.clone(),
        any_seed_diverged: scenario.any_seed_diverged,
        suspicious_low_bpc: scenario.suspicious_low_bpc,
    })?;
    assert_eq!(dispatch.outcome, scenario.outcome);
    let report_input = report_input(
        &scenario,
        fixture,
        &baseline,
        &checkpoints,
        &run_logs,
        &scores,
        &negative,
        &ablation,
        dispatch.decision.clone(),
    )?;
    let report = gbf_experiments::s1::report::emit_report(&report_input)?;

    emitter.e2e_scenario_complete(&E2eScenarioCompleteEvent {
        scenario: scenario.name.to_owned(),
        outcome: dispatch.outcome.to_string(),
        decision: dispatch.decision.to_string(),
        pass: true,
        duration_seconds: 0.0,
    })?;

    Ok(ScenarioArtifacts {
        baseline,
        checkpoints,
        run_logs,
        scores,
        negative,
        ablation,
        oracle,
        report,
    })
}

fn phase(emitter: &S1LogEmitter, scenario: &str, phase: &str) -> TestResult {
    emitter.e2e_phase(&E2ePhaseEvent {
        scenario: scenario.to_owned(),
        phase: phase.to_owned(),
    })?;
    Ok(())
}

fn completions_for(scenario: &ScenarioSpec) -> Vec<S1Completion> {
    let mut completions = vec![S1Completion::Completed; 5];
    if let Some((seed, step, _)) = scenario.divergence {
        completions[seed as usize] = S1Completion::DivergedAt { step };
        for completion in completions.iter_mut().skip(seed as usize + 1) {
            *completion = S1Completion::NotReached;
        }
    }
    completions
}

fn baseline_report(fixture: &FixtureInputs) -> TestResult<BaselineReport> {
    Ok(BaselineReport {
        schema: "s1_baseline.v1".to_owned(),
        corpus_train_sha: fixture.train_sha,
        corpus_val_sha: fixture.val_sha,
        smoothing: SmoothingScheme {
            alpha: 0.01,
            lambdas: [0.6, 0.3, 0.1],
        },
        bpc_3gram: 2.30,
        bpc_2gram: 2.60,
        bpc_unigram: 3.10,
        counts_summary: CountsSummary {
            train_bytes: fixture.train_len,
            distinct_unigrams: 64,
            distinct_bigrams: 128,
            distinct_trigrams: 256,
        },
        counts_blob_sha256: hash(4),
        baseline_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?)
}

fn checkpoint_for_seed(
    seed: u64,
    fixture: &FixtureInputs,
    completion: S1Completion,
) -> TestResult<Option<CheckpointMetadata>> {
    if completion == S1Completion::NotReached {
        return Ok(None);
    }
    Ok(Some(checkpoint_metadata(
        seed,
        fixture,
        S1BuildKind::PhaseA,
        completion,
    )?))
}

fn checkpoint_metadata(
    seed: u64,
    fixture: &FixtureInputs,
    build_kind: S1BuildKind,
    completion: S1Completion,
) -> TestResult<CheckpointMetadata> {
    Ok(CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed,
        corpus_train_sha: fixture.train_sha,
        corpus_val_sha: fixture.val_sha,
        model_config_hash: hash(10),
        train_config_hash: sha256(S1CanonicalJson::to_vec(&TrainConfig::integration_fixture())?),
        build_kind,
        build_config_hash: hash(11),
        dependency_lockfile_sha: hash(12),
        rust_toolchain_hash: hash(13),
        device_profile_hash: hash(14),
        rng_stream_def_hash: hash(15),
        pass_version: SemVer::new(0, 1, 0),
        budget_profile: "integration_fixture".to_owned(),
        final_step: if completion == S1Completion::Completed {
            100
        } else {
            17
        },
        final_train_loss: 0.90 + seed as f32 * 0.01,
        completion,
        checkpoint_safetensors_sha256: hash(16),
        checkpoint_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?)
}

fn run_log_for_seed(seed: u64, completion: S1Completion) -> TestResult<Option<RunLog>> {
    if completion == S1Completion::NotReached {
        return Ok(None);
    }
    let final_step = match completion {
        S1Completion::Completed => 100,
        S1Completion::DivergedAt { step } => step,
        S1Completion::NotReached => unreachable!("handled above"),
    };
    Ok(Some(
        RunLog {
            schema: "s1_run_log.v1".to_owned(),
            seed,
            train_config_hash: hash(16),
            losses: (1..=final_step)
                .map(|step| (step, 1.25_f32 / step as f32 + seed as f32 * 0.001))
                .collect(),
            eval_points: vec![(0, 2.5 + seed as f64 * 0.01), (final_step, 1.9)],
            final_grad_norms: GradNormSummary {
                global_l2: if completion == S1Completion::Completed {
                    0.30
                } else {
                    0.0
                },
                max_l2: 0.20,
                mean_l2: 0.10,
            },
            run_log_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()?,
    ))
}

fn score_report(
    seed: u64,
    fixture: &FixtureInputs,
    bpc: f64,
    checkpoint_sha: Option<Hash256>,
    completion: S1Completion,
) -> TestResult<Option<ScoreReport>> {
    if completion != S1Completion::Completed {
        return Ok(None);
    }
    let log2_sum = bpc * fixture.val_len as f64;
    Ok(Some(
        ScoreReport {
            schema: "s1_score.v1".to_owned(),
            seed,
            checkpoint_sha: checkpoint_sha.unwrap_or(Hash256::ZERO),
            corpus_val_sha: fixture.val_sha,
            chunk_size: 128,
            token_count: fixture.val_len,
            log2_sum,
            bpc,
            score_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()?,
    ))
}

fn ablation_report(fixture: &FixtureInputs, equal: bool) -> TestResult<AblationReport> {
    let phase_a = checkpoint_metadata(0, fixture, S1BuildKind::PhaseA, S1Completion::Completed)?;
    let ablation = checkpoint_metadata(0, fixture, S1BuildKind::Ablation, S1Completion::Completed)?;
    let phase_a_tensors = vec![fixture_tensor(1.0)?];
    let ablation_tensors = vec![fixture_tensor(if equal { 1.0 } else { -1.0 })?];
    Ok(compare(
        AblationCheckpoint {
            metadata: &phase_a,
            checkpoint_sha: phase_a.checkpoint_self_hash,
            tensors: &phase_a_tensors,
        },
        AblationCheckpoint {
            metadata: &ablation,
            checkpoint_sha: ablation.checkpoint_self_hash,
            tensors: &ablation_tensors,
        },
    )?)
}

fn fixture_tensor(value: f32) -> TestResult<CanonicalTensor> {
    Ok(CanonicalTensor::new(
        ArtifactPath::new("toy0.e2e.weight")?,
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[1])?,
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(vec![value]),
    )?)
}

#[allow(clippy::too_many_arguments)]
fn report_input(
    scenario: &ScenarioSpec,
    fixture: &FixtureInputs,
    baseline: &BaselineReport,
    checkpoints: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    scores: &[Option<ScoreReport>],
    negative: &NegativeTestReport,
    ablation: &AblationReport,
    decision: S1Decision,
) -> TestResult<ReportInput> {
    let per_seed_artifacts = (0..5)
        .map(|seed| PerSeedArtifacts {
            seed: seed as u64,
            completion: completions_for(scenario)[seed].clone(),
            checkpoint_self_hash: checkpoints[seed]
                .as_ref()
                .map(|metadata| metadata.checkpoint_self_hash),
            run_log_self_hash: run_logs[seed]
                .as_ref()
                .map(|run_log| run_log.run_log_self_hash),
            score_self_hash: scores[seed].as_ref().map(|score| score.score_self_hash),
            negative_self_hash: (seed == 0).then_some(negative.negative_self_hash),
            ablation_self_hash: (seed == 0).then_some(ablation.ablation_self_hash),
        })
        .collect::<Vec<_>>();

    Ok(ReportInput {
        front_matter: ReportFrontMatter {
            schema: "s1_report.v1".to_owned(),
            s1_outcome: scenario.outcome,
            decision,
            baseline_self_hash: baseline.baseline_self_hash,
            per_seed_artifacts,
            generated_at: "2026-05-09T12:00:00Z".to_owned(),
            rfc_revision: RfcRevisionRef::GitCommitId(commit('a')?),
            predictions_section_hash: predictions_section_hash(PREDICTIONS)?,
            predictions_commit: commit('b')?,
            first_result_commit: commit('c')?,
            report_self_hash: Hash256::ZERO,
        },
        predictions_markdown: PREDICTIONS.to_owned(),
        observed_per_seed: (0..5)
            .map(|seed| ObservedSeed {
                seed: seed as u64,
                completion: completions_for(scenario)[seed].clone(),
                val_bpc: scores[seed].as_ref().map(|score| score.bpc),
                neg_test_delta: (seed == 0).then_some(negative.delta),
                ablation_eq: (seed == 0).then_some(ablation.phase_a_eq_ablation),
            })
            .collect(),
        hypotheses: hypothesis_findings(scenario),
        falsification_analysis: format!(
            "{} fixture exercised the existing S1 outcome/report composition path.",
            scenario.name
        ),
        surprises: if scenario.suspicious_low_bpc {
            "Suspicious-low-bpc sentinel fired in the synthetic fixture.".to_owned()
        } else {
            "None.".to_owned()
        },
        decision_justification: "Decision follows RFC section 8 dispatch.".to_owned(),
        replay_command: format!(
            "scripts/s1_e2e.sh --scenario {} --fixture tiny",
            scenario.name
        ),
        manifest_hashes: format!(
            "train_sha={} val_sha={}",
            fixture.train_sha, fixture.val_sha
        ),
        pass_version: "0.1.0".to_owned(),
    })
}

fn hypothesis_findings(scenario: &ScenarioSpec) -> Vec<HypothesisFinding> {
    [
        (
            Hypothesis::H1,
            scenario.h1.clone(),
            observation_for(scenario, Hypothesis::H1),
        ),
        (
            Hypothesis::H2,
            scenario.h2.clone(),
            observation_for(scenario, Hypothesis::H2),
        ),
        (
            Hypothesis::H3,
            scenario.h3.clone(),
            observation_for(scenario, Hypothesis::H3),
        ),
        (
            Hypothesis::H4,
            scenario.h4.clone(),
            observation_for(scenario, Hypothesis::H4),
        ),
        (
            Hypothesis::H5,
            scenario.h5.clone(),
            observation_for(scenario, Hypothesis::H5),
        ),
    ]
    .into_iter()
    .map(|(hypothesis, status, observation)| HypothesisFinding {
        hypothesis,
        status,
        observation,
    })
    .collect()
}

fn observation_for(scenario: &ScenarioSpec, hypothesis: Hypothesis) -> String {
    match (scenario.name, hypothesis) {
        ("fail_substrate_nan", Hypothesis::H1) => {
            "seed 2 diverged at step 17 with observed=non_finite_loss".to_owned()
        }
        ("fail_substrate_zero_grad", Hypothesis::H1) => {
            "seed 1 diverged at step 23 with observed=zero_grad".to_owned()
        }
        ("fail_capacity_toytiny", Hypothesis::H2) => {
            "ToyTiny fixture bpc 2.400000 did not beat baseline 2.300000 by 0.05".to_owned()
        }
        ("pass_with_warning", Hypothesis::H3) => {
            "shuffle delta 0.250000 did not exceed the 2.0 bpc sensitivity threshold".to_owned()
        }
        ("fail_phase_ternary_leak", Hypothesis::H4) => {
            "ablation tensor payload differed for toy0.e2e.weight at byte 0".to_owned()
        }
        ("fail_metric_modulo_shuffle", Hypothesis::H5) => {
            "O-metric-4 modulo-shuffle fixture returned false".to_owned()
        }
        (_, Hypothesis::H1) => "all fixture seeds reached the expected substrate state".to_owned(),
        (_, Hypothesis::H2) => format!(
            "fixture val_bpc {:.6} was evaluated against baseline 2.300000",
            scenario.bpc
        ),
        (_, Hypothesis::H3) => format!(
            "fixture shuffle delta {:.6} was evaluated against the sensitivity threshold",
            scenario.neg_delta
        ),
        (_, Hypothesis::H4) => format!("phase_a_eq_ablation={}", scenario.phase_a_eq_ablation),
        (_, Hypothesis::H5) => {
            if scenario.oracle_results.metric_oracle_passed() {
                "all metric oracle fixture checks passed".to_owned()
            } else {
                format!(
                    "metric oracle fixture failed ids={:?}",
                    scenario.oracle_results.failed_oracle_ids()
                )
            }
        }
    }
}

fn selected_event_log(events: &[common::tracing_capture::TracingEvent]) -> Vec<Value> {
    let selected_names = [
        event::E2E_SCENARIO_START,
        event::E2E_PHASE,
        event::RUN_DIVERGENCE,
        event::ABLATION_COMPLETE,
        event::ORACLE_AGGREGATE_COMPLETE,
        event::OUTCOME_DISPATCH_COMPLETE,
        event::OUTCOME_REFUTED_INPUT,
        event::REPORT_EMIT_COMPLETE,
        event::E2E_SCENARIO_COMPLETE,
    ];
    events
        .iter()
        .filter(|event| selected_names.contains(&event.name.as_str()))
        .map(|event| {
            json!({
                "name": event.name,
                "level": event.level,
                "fields": event.fields,
            })
        })
        .collect()
}

fn assert_scenario_golden(name: &str, rendered: &RenderedScenario) -> TestResult {
    let golden_dir = repo_root().join(GOLDEN_ROOT).join(name);
    if update_goldens() && golden_dir.exists() {
        fs::remove_dir_all(&golden_dir)?;
    }
    for (relative, actual) in &rendered.files {
        let path = golden_dir.join(relative);
        if update_goldens() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, actual)?;
            continue;
        }
        let expected = fs::read(&path).map_err(|error| {
            format!(
                "missing golden {} for scenario {name}: {error}",
                path.display()
            )
        })?;
        if let Err(message) =
            golden_diff_message(&relative.display().to_string(), &expected, actual)
        {
            return Err(format!("{name}: {message}").into());
        }
    }
    Ok(())
}

fn golden_diff_message(relative: &str, expected: &[u8], actual: &[u8]) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let mismatch = first_mismatch_offset(expected, actual);
    let (line, column) = line_column(expected, actual, mismatch);
    let expected_line = line_preview(expected, mismatch);
    let actual_line = line_preview(actual, mismatch);
    Err(format!(
        "golden drift for {relative}: expected {} bytes, actual {} bytes; first difference at byte {mismatch} (line {line}, column {column}); expected line: {expected_line:?}; actual line: {actual_line:?}",
        expected.len(),
        actual.len()
    ))
}

fn first_mismatch_offset(expected: &[u8], actual: &[u8]) -> usize {
    let shared = expected.len().min(actual.len());
    expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual)
        .unwrap_or(shared)
}

fn line_column(expected: &[u8], actual: &[u8], offset: usize) -> (usize, usize) {
    let context = if offset < expected.len() {
        expected
    } else {
        actual
    };
    let mut line = 1;
    let mut column = 1;
    for &byte in context.iter().take(offset) {
        if byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn line_preview(bytes: &[u8], offset: usize) -> String {
    if offset >= bytes.len() {
        return "<EOF>".to_owned();
    }
    let line_start = bytes[..offset]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |index| index + 1);
    let line_end = bytes[offset..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(bytes.len(), |index| offset + index);
    let preview = String::from_utf8_lossy(&bytes[line_start..line_end]);
    const MAX_PREVIEW_CHARS: usize = 160;
    let mut chars = preview.chars();
    let mut truncated = chars.by_ref().take(MAX_PREVIEW_CHARS).collect::<String>();
    if chars.next().is_some() {
        truncated.push_str("...");
    }
    truncated
}

fn insert_json(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    relative: impl Into<PathBuf>,
    value: &impl Serialize,
) -> TestResult {
    insert_bytes(files, relative, S1CanonicalJson::to_vec(value)?);
    Ok(())
}

fn insert_bytes(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    relative: impl Into<PathBuf>,
    bytes: Vec<u8>,
) {
    files.insert(relative.into(), bytes);
}

fn oracle_results(
    o_metric_0: bool,
    o_metric_1: bool,
    o_metric_2: bool,
    o_metric_3: bool,
    o_metric_4: bool,
) -> MetricOracleResults {
    MetricOracleResults {
        o_metric_0,
        o_metric_1,
        o_metric_2,
        o_metric_3,
        o_metric_4,
    }
}

fn not_reached(reason: &str) -> HypothesisStatus {
    HypothesisStatus::NotEvaluatedDueToPriorGate(reason.to_owned())
}

fn commit(fill: char) -> Result<GitCommitId, gbf_experiments::s1::schema::S1SchemaError> {
    GitCommitId::new(fill.to_string().repeat(40))
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}

fn update_goldens() -> bool {
    std::env::var_os("GBF_UPDATE_GOLDENS").is_some()
        || std::env::args().any(|arg| arg == "--update-goldens")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .to_path_buf()
}
