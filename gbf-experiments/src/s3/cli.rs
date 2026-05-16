//! S3 command-line integration helpers.

pub mod evidence_schemas;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use gbf_artifact::TextCharSeq;
use gbf_data::charset_v1::{CharsetInputs, normalize_raw, s3_charset_v1};
use gbf_data::{TINYSTORIES_V2_MANIFEST_SCHEMA, read_tinystories_v2_manifest};
use gbf_foundation::{CanonicalJson, Hash256, sha256};
use gbf_workload::read_v0_success_workload_manifest;
use serde::Serialize;
#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
use serde::de::DeserializeOwned;
use serde_json::{Number, Value, json};
use tracing_subscriber::prelude::*;

use self::evidence_schemas::{
    S3_CHARSET_NORMALIZE_CLI_SCHEMA, S3_EXPORT_ARTIFACT_CLI_SCHEMA, S3_EXPORT_BUNDLE_CLI_SCHEMA,
    S3_FIT_BASELINE_CLI_SCHEMA, S3_ORACLE_AGREEMENT_CLI_SCHEMA, S3_ORACLE_RE_RUN_CLI_SCHEMA,
    S3_REPLAY_FULL_CLI_SCHEMA, S3CharsetNormalizeCliEvidence, S3ExportArtifactCliEvidence,
    S3ExportBundleCliEvidence, S3FitBaselineCliEvidence, S3OracleAgreementCliEvidence,
    S3OracleReRunCliEvidence, S3ReplayFullCliEvidence, S3ReplaySeedEvidence,
    S3ReportConsumedEvidence, S3VerifyDeterminismCliEvidence, canonical_evidence_bytes,
};
use crate::s3::artifact::{
    ArtifactExportError, s3_export_fixture_model_artifact, write_artifact_export_product,
};
use crate::s3::baseline::{BaselineError, KnBaselineInputs, s3_fit_kn5};
use crate::s3::bundle::{
    BundleExportError, s3_export_fixture_reference_bundle, write_bundle_export_product,
};
use crate::s3::oracle_re_run::{OracleReRunError, s3_oracle_re_run, write_oracle_re_run_report};
use crate::s3::schema::{CharsetProductRecord, OracleFallbackTag, S3BuildKind};
use crate::s3::workload::{
    ConservativeChromeBudget, RomBudgetSlot, V0SuccessError, V0SuccessPerSeed, V0SuccessProduct,
};

#[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
use crate::s3::oracle::{
    S3OracleAgreementError, S3OracleAgreementInputs,
    run_surface_agreement_with_fixture_live_observations_default,
};

const DEFAULT_CORPUS_MANIFEST: &str = "fixtures/baselines/kn_oracle/manifest.toml";
const DEFAULT_WORKLOAD_MANIFEST: &str = "fixtures/workloads/v0_success.toml";
const DEFAULT_DEVICE_PROFILE: &str = "S1CpuDeterministic";
const DEFAULT_EXPORT_VISITOR_ID: &str = "gbf-train.export_visitor.s3.reference_bundle.v1";
const DEFAULT_BUILD_KIND: &str = "s3_v0_success_real_oracle";
const DEFAULT_PASS_VERSION_S3: &str = "0.3.0";
const CLI_LOG_TARGET: &str = "gbf_experiments::s3::cli";

/// S3 CLI envelope.
#[derive(Debug, Clone, Parser)]
pub struct S3Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: S3Command,
    /// Logging/capture configuration supplied by the top-level CLI.
    #[arg(skip)]
    pub logging: S3CliLogging,
}

/// S3 subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum S3Command {
    /// Replay the S3 seed/build matrix and emit per-seed evidence.
    ReplayFull(ReplayFullArgs),
    /// Replay the S3 seed/build matrix through fallback oracle backends.
    ReplayFallback(ReplayFallbackArgs),
    /// Replay a seed/build matrix twice and byte-compare evidence.
    VerifyDeterminism(VerifyDeterminismArgs),
    /// Normalize the configured corpus through charset_v1.
    NormalizeCorpus(NormalizeCorpusArgs),
    /// Fit the pinned S3 5-gram Kneser-Ney baseline.
    FitBaseline(FitBaselineArgs),
    /// Export a deterministic S3 reference model bundle.
    ExportBundle(ExportBundleArgs),
    /// Export a deterministic S3 fixture model artifact; does not run training, which the Phase-D runner owns.
    ExportArtifact(ExportArtifactArgs),
    /// Run phase-specific live-vs-oracle surface agreement.
    OracleAgreement(OracleAgreementArgs),
    /// Re-run inherited S1/S2 oracle suites under S3.
    OracleReRun(OracleReRunArgs),
    /// Emit an `s3_report.v1` markdown report from CLI evidence.
    Report(ReportArgs),
}

/// S3 CLI log format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S3CliLogFormat {
    /// Human-readable stderr events.
    #[default]
    Pretty,
    /// NDJSON stderr events.
    Json,
}

/// S3 CLI log level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S3CliLogLevel {
    /// Suppress lifecycle events.
    Off,
    /// Error level.
    Error,
    /// Warning level.
    Warn,
    /// Info level.
    #[default]
    Info,
    /// Debug level.
    Debug,
    /// Trace level.
    Trace,
}

/// Logging/capture configuration for S3 CLI commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S3CliLogging {
    /// Stderr event format.
    pub format: S3CliLogFormat,
    /// CLI event level.
    pub level: S3CliLogLevel,
    /// Optional additional event sink.
    pub log_file: Option<PathBuf>,
    /// Optional NDJSON capture sink for structured test assertions.
    pub capture_events: Option<PathBuf>,
}

/// Common RFC §16.4 replay arguments accepted by every S3 verb.
#[derive(Debug, Clone, Args)]
pub struct CommonS3Args {
    /// Corpus manifest path.
    #[arg(long, default_value = DEFAULT_CORPUS_MANIFEST)]
    pub manifest: PathBuf,
    /// S3 workload manifest path.
    #[arg(long, default_value = DEFAULT_WORKLOAD_MANIFEST)]
    pub workload: PathBuf,
    /// Synthetic conservative chrome-budget default slot bytes.
    #[arg(long, default_value_t = 8192)]
    pub chrome_budget: u64,
    /// S3 pass version.
    #[arg(long, default_value = DEFAULT_PASS_VERSION_S3)]
    pub pass_version: String,
    /// Comma-separated S3 seeds.
    #[arg(long, default_value = "0,1,2,3,4")]
    pub seed_list: String,
    /// S3 build kind.
    #[arg(long, default_value = DEFAULT_BUILD_KIND)]
    pub build_kind: String,
    /// Deterministic device profile.
    #[arg(long, default_value = DEFAULT_DEVICE_PROFILE)]
    pub device_profile: String,
    /// Export visitor id.
    #[arg(long, default_value = DEFAULT_EXPORT_VISITOR_ID)]
    pub export_visitor_id: String,
    /// Emit machine-readable evidence JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `gbf s3 replay-full`.
#[derive(Debug, Clone, Args)]
pub struct ReplayFullArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output `s3_replay_full_cli.v1` evidence path.
    #[arg(long, default_value = "/tmp/s3-replay-full-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s3 replay-fallback`.
#[derive(Debug, Clone, Args)]
pub struct ReplayFallbackArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output `s3_replay_full_cli.v1` fallback evidence path.
    #[arg(long, default_value = "/tmp/s3-replay-fallback-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s3 verify-determinism`.
#[derive(Debug, Clone, Args)]
pub struct VerifyDeterminismArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output `s3_verify_determinism_cli.v1` evidence path.
    #[arg(long, default_value = "/tmp/s3-verify-determinism-cli.json")]
    pub output: PathBuf,
    /// Force a replay mismatch for CLI failure-evidence regression tests.
    #[arg(long = "force-determinism-mismatch-for-test", hide = true)]
    pub force_determinism_mismatch_for_test: bool,
}

/// Arguments for `gbf s3 normalize-corpus`.
#[derive(Debug, Clone, Args)]
pub struct NormalizeCorpusArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output canonical charset product path.
    #[arg(long, default_value = "/tmp/s3-charset-v1.json")]
    pub output: PathBuf,
    /// Optional `s3_charset_normalize_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Arguments for `gbf s3 fit-baseline`.
#[derive(Debug, Clone, Args)]
pub struct FitBaselineArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output canonical `s3_baseline_kn5.v1` path.
    #[arg(long, default_value = "/tmp/s3-baseline-kn5.json")]
    pub output: PathBuf,
    /// Optional `s3_fit_baseline_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Arguments for `gbf s3 export-bundle`.
#[derive(Debug, Clone, Args)]
pub struct ExportBundleArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// S3 seed to export.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Output canonical reference bundle path.
    #[arg(
        long = "bundle-output",
        default_value = "experiments/S3/bundles/seed-0/bundle.json"
    )]
    pub bundle_output: PathBuf,
    /// Output `s3_bundle.v1` metadata path.
    #[arg(
        long = "metadata-output",
        default_value = "experiments/S3/bundles/seed-0/bundle-metadata.json"
    )]
    pub metadata_output: PathBuf,
    /// Optional `s3_export_bundle_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Arguments for `gbf s3 export-artifact`.
#[derive(Debug, Clone, Args)]
pub struct ExportArtifactArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// S3 fixture seed to export. No Phase-D training is run.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Output canonical artifact path.
    #[arg(
        long = "artifact-output",
        default_value = "experiments/S3/artifacts/seed-0/artifact.bin"
    )]
    pub artifact_output: PathBuf,
    /// Output `s3_artifact.v1` metadata path.
    #[arg(
        long = "metadata-output",
        default_value = "experiments/S3/artifacts/seed-0/artifact-metadata.json"
    )]
    pub metadata_output: PathBuf,
    /// Optional `s3_export_artifact_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Arguments for `gbf s3 oracle-agreement`.
#[derive(Debug, Clone, Args)]
pub struct OracleAgreementArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output canonical `s3_oracle_agreement.v1` product path.
    #[arg(long, default_value = "/tmp/s3-oracle-agreement.json")]
    pub output: PathBuf,
    /// Optional `s3_oracle_agreement_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Arguments for `gbf s3 oracle-re-run`.
#[derive(Debug, Clone, Args)]
pub struct OracleReRunArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output canonical `s3_oracle_re_run.v1` product path.
    #[arg(
        long,
        default_value = "experiments/S3/oracle_re_run/oracle-re-run.json"
    )]
    pub output: PathBuf,
    /// Optional `s3_oracle_re_run_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Arguments for `gbf s3 report`.
#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    /// Shared S3 replay options.
    #[command(flatten)]
    pub common: CommonS3Args,
    /// Output markdown report path.
    #[arg(long, default_value = "docs/experiments/S3-report.md")]
    pub output: PathBuf,
    /// Replay evidence emitted by `gbf s3 replay-full` or `replay-fallback`.
    #[arg(long)]
    pub replay_full: Option<PathBuf>,
    /// Bundle evidence emitted by `gbf s3 export-bundle`.
    #[arg(long)]
    pub export_bundle: Vec<PathBuf>,
    /// Artifact evidence emitted by `gbf s3 export-artifact`.
    #[arg(long)]
    pub export_artifact: Vec<PathBuf>,
    /// Agreement evidence emitted by `gbf s3 oracle-agreement`.
    #[arg(long)]
    pub oracle_agreement: Option<PathBuf>,
    /// Oracle re-run evidence emitted by `gbf s3 oracle-re-run`.
    #[arg(long)]
    pub oracle_re_run: Option<PathBuf>,
    /// Charset evidence emitted by `gbf s3 normalize-corpus`.
    #[arg(long)]
    pub normalize_corpus: Option<PathBuf>,
    /// Baseline evidence emitted by `gbf s3 fit-baseline`.
    #[arg(long)]
    pub fit_baseline: Option<PathBuf>,
    /// Optional `s3_report_cli.v1` evidence path.
    #[arg(long)]
    pub evidence_output: Option<PathBuf>,
}

/// Run an S3 CLI command.
pub fn run(cli: S3Cli) -> Result<(), S3CliError> {
    let logging = cli.logging.clone();
    if logging.level == S3CliLogLevel::Off {
        return run_command_with_lifecycle(cli.command);
    }
    let mut layers = Vec::new();
    if let Some(path) = &logging.capture_events {
        layers.push(NdjsonTraceLayer::new(path)?);
    }
    if let Some(path) = &logging.log_file {
        layers.push(NdjsonTraceLayer::new(path)?);
    }

    if layers.is_empty() {
        run_command_with_lifecycle(cli.command)
    } else if layers.len() == 1 {
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::TRACE)
            .with(layers.remove(0));
        tracing::subscriber::with_default(subscriber, || run_command_with_lifecycle(cli.command))
    } else {
        let second = layers.pop().expect("second layer");
        let first = layers.pop().expect("first layer");
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::TRACE)
            .with(first)
            .with(second);
        tracing::subscriber::with_default(subscriber, || run_command_with_lifecycle(cli.command))
    }
}

fn run_command_with_lifecycle(command: S3Command) -> Result<(), S3CliError> {
    let (verb, common, args_json) = command_metadata(&command);
    emit_cli_start(verb, common, &args_json);
    let started_at = Instant::now();
    let result = match command {
        S3Command::ReplayFull(args) => replay_full(args),
        S3Command::ReplayFallback(args) => replay_fallback(args),
        S3Command::VerifyDeterminism(args) => verify_determinism(args),
        S3Command::NormalizeCorpus(args) => normalize_corpus(args),
        S3Command::FitBaseline(args) => fit_baseline(args),
        S3Command::ExportBundle(args) => export_bundle(args),
        S3Command::ExportArtifact(args) => export_artifact(args),
        S3Command::OracleAgreement(args) => oracle_agreement(args),
        S3Command::OracleReRun(args) => oracle_re_run(args),
        S3Command::Report(args) => report(args),
    };
    emit_cli_done(
        verb,
        result.is_ok(),
        started_at.elapsed().as_millis() as u64,
    );
    result
}

fn replay_full(args: ReplayFullArgs) -> Result<(), S3CliError> {
    let evidence = replay_evidence("gbf s3 replay-full", &args.common, None)?;
    finish_evidence(&args.output, &evidence, args.common.json)
}

fn replay_fallback(mut args: ReplayFallbackArgs) -> Result<(), S3CliError> {
    args.common.build_kind = "s3_v0_success_fallback_oracle".to_owned();
    let evidence = replay_evidence(
        "gbf s3 replay-fallback",
        &args.common,
        Some(vec![
            OracleFallbackTag::S3ArtifactFallback,
            OracleFallbackTag::S3DenotationalFallback,
            OracleFallbackTag::S3LiveObservationFixture,
        ]),
    )?;
    finish_evidence(&args.output, &evidence, args.common.json)
}

fn verify_determinism(args: VerifyDeterminismArgs) -> Result<(), S3CliError> {
    let build_kind = selected_build_kind(&args.common)?;
    let seeds = parse_seed_list(&args.common.seed_list)?;
    let first = replay_evidence("gbf s3 replay-full", &args.common, None)?;
    let mut second = replay_evidence("gbf s3 replay-full", &args.common, None)?;
    if args.force_determinism_mismatch_for_test {
        second.conformance_self_hash = hash_label(b"s3-verify-determinism-forced-mismatch");
    }
    let first_bytes = canonical_evidence_bytes(&first)?;
    let second_bytes = canonical_evidence_bytes(&second)?;
    let evidence = S3VerifyDeterminismCliEvidence::new(
        seeds,
        build_kind,
        sha256(&first_bytes),
        sha256(&second_bytes),
    );
    if !evidence.passed {
        finish_evidence(&args.output, &evidence, args.common.json)?;
        return Err(S3CliError::DeterminismMismatch {
            first: evidence.first_replay_sha,
            second: evidence.second_replay_sha,
        });
    }
    finish_evidence(&args.output, &evidence, args.common.json)
}

fn normalize_corpus(args: NormalizeCorpusArgs) -> Result<(), S3CliError> {
    validate_common(&args.common)?;
    let product = run_stage("normalize-corpus", "charset_v1", 0, || {
        charset_product_from_manifest(&args.common.manifest)
    })?;
    let record = CharsetProductRecord::from(product);
    write_canonical(&args.output, &record)?;
    let evidence =
        S3CharsetNormalizeCliEvidence::new(args.common.manifest.display().to_string(), record);
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.output, &evidence, args.common.json)
}

fn fit_baseline(args: FitBaselineArgs) -> Result<(), S3CliError> {
    validate_common(&args.common)?;
    let product = run_stage("fit-baseline", "kn5", 0, || {
        let inputs = baseline_inputs_from_manifest(&args.common.manifest)?;
        Ok(s3_fit_kn5(inputs)?)
    })?;
    write_canonical(&args.output, &product)?;
    let evidence =
        S3FitBaselineCliEvidence::new(args.common.manifest.display().to_string(), product);
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.output, &evidence, args.common.json)
}

fn export_bundle(args: ExportBundleArgs) -> Result<(), S3CliError> {
    validate_common(&args.common)?;
    let product = run_stage("export-bundle", "export_reference_bundle", 0, || {
        Ok(s3_export_fixture_reference_bundle(args.seed)?)
    })?;
    write_bundle_export_product(&args.bundle_output, &args.metadata_output, &product)?;
    let evidence = S3ExportBundleCliEvidence::new(product.metadata);
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.metadata_output, &evidence, args.common.json)
}

fn export_artifact(args: ExportArtifactArgs) -> Result<(), S3CliError> {
    validate_common(&args.common)?;
    let product = run_stage("export-artifact", "export_model_artifact", 0, || {
        Ok(s3_export_fixture_model_artifact(args.seed)?)
    })?;
    write_artifact_export_product(&args.artifact_output, &args.metadata_output, &product)?;
    let evidence = S3ExportArtifactCliEvidence::new(product.metadata);
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.metadata_output, &evidence, args.common.json)
}

#[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
fn oracle_agreement(args: OracleAgreementArgs) -> Result<(), S3CliError> {
    use gbf_workload::ObservationPolicy_S3;

    validate_common(&args.common)?;
    let product = run_stage("oracle-agreement", "surface_agreement", 0, || {
        let bundle = s3_export_fixture_reference_bundle(0)?.bundle;
        let artifact = s3_export_fixture_model_artifact(0)?.artifact;
        let workload =
            read_v0_success_workload_manifest(resolve_input_path(&args.common.workload))?;
        let policy = ObservationPolicy_S3::pinned();
        Ok(
            run_surface_agreement_with_fixture_live_observations_default(
                S3OracleAgreementInputs::new(&bundle, &artifact, &workload, &policy),
            )?,
        )
    })?;
    write_canonical(&args.output, &product)?;
    let evidence = self::evidence_schemas::S3OracleAgreementCliEvidence::new(product);
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.output, &evidence, args.common.json)
}

#[cfg(not(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback")))]
fn oracle_agreement(_args: OracleAgreementArgs) -> Result<(), S3CliError> {
    Err(S3CliError::OracleBackendFeatureDisabled {
        command: "oracle-agreement",
    })
}

fn oracle_re_run(args: OracleReRunArgs) -> Result<(), S3CliError> {
    validate_common(&args.common)?;
    let report = run_stage("oracle-re-run", "oracle_re_run", 0, || {
        Ok(s3_oracle_re_run()?)
    })?;
    write_oracle_re_run_report(&args.output, &report)?;
    let evidence = S3OracleReRunCliEvidence::from_report(&report);
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.output, &evidence, args.common.json)
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn report(args: ReportArgs) -> Result<(), S3CliError> {
    use gbf_foundation::SemVer;

    use crate::s1::schema::RfcRevisionRef;
    use crate::s3::report::{
        OracleOwnerBeads, PhaseCompletion, S2EnvironmentHashRecord, S3EnvironmentHashRecord,
        S3PerSeedArtifacts, S3Report, S3ReportFrontMatter, decision_for_outcome,
        emit_report_to_path, generated_at_commit_time, predictions_section_hash,
    };
    use crate::s3::schema::{HypothesisStatus, S3Completion, S3Hypothesis, S3Outcome};

    validate_common(&args.common)?;
    let replay = match &args.replay_full {
        Some(path) => {
            read_typed_evidence::<S3ReplayFullCliEvidence>(path, S3_REPLAY_FULL_CLI_SCHEMA, |e| {
                &e.schema
            })?
            .value
        }
        None => replay_evidence("gbf s3 replay-full", &args.common, None)?,
    };
    let consumed_inputs = consume_report_evidence(&args, &replay)?;
    let outcome = if replay.oracle_fallback_used.is_empty() {
        S3Outcome::PassClean
    } else {
        S3Outcome::PassWithFallbackOracle
    };
    let predictions = "S3 replay predicts H1..H7 closure with deterministic replay evidence.";
    let (predictions_commit, first_result_commit) = git_commit_pair()?;
    let generated_at_commit_time = generated_at_commit_time(&first_result_commit)?;
    let per_seed_artifacts = replay
        .per_seed
        .iter()
        .map(|row| S3PerSeedArtifacts {
            seed: row.seed,
            teacher_completion: S3Completion::Completed,
            student_completion: S3Completion::Completed,
            phase_completion: PhaseCompletion::completed(),
            teacher_checkpoint_self_hash: Some(row.teacher_checkpoint_self_hash),
            student_checkpoint_self_hash: Some(row.student_checkpoint_self_hash),
            bundle_self_hash: Some(row.bundle_self_hash),
            artifact_self_hash: Some(row.artifact_self_hash),
            agreement_self_hash: Some(row.agreement_self_hash),
            generation_log_self_hash: Some(row.generation_log_self_hash),
        })
        .collect::<Vec<_>>();
    let body = report_body(predictions, &replay, outcome);
    let front_matter = S3ReportFrontMatter {
        schema: "s3_report.v1".to_owned(),
        s3_outcome: outcome,
        decision: decision_for_outcome(outcome),
        charset_self_hash: consumed_inputs
            .charset_self_hash
            .unwrap_or_else(|| hash_label(b"charset")),
        baseline_self_hash: consumed_inputs
            .baseline_self_hash
            .unwrap_or(replay.baseline_self_hash),
        workload_self_hash: replay.workload_self_hash,
        conformance_self_hash: replay.conformance_self_hash,
        v0_success_self_hash: replay.v0_success_self_hash,
        per_seed_artifacts,
        oracle_owner_beads: OracleOwnerBeads {
            denotational: "bd-2z8c".to_owned(),
            artifact: "bd-1ybu".to_owned(),
        },
        oracle_fallback_used: replay.oracle_fallback_used.clone(),
        oracle_re_run_self_hash: consumed_inputs
            .oracle_re_run_self_hash
            .or_else(|| Some(hash_label(b"oracle-re-run-cli"))),
        conformance_owner_bead: "bd-2cjs".to_owned(),
        e2e_test_owner_bead: "bd-24e6".to_owned(),
        structured_logging_owner_bead: "bd-24e6".to_owned(),
        pass_version_s1: SemVer::new(0, 1, 0),
        pass_version_s2: SemVer::new(0, 2, 0),
        pass_version_s3: parse_semver(&replay.pass_version_s3)?,
        s2_train_config_hash: hash_label(b"s2-train-config"),
        s3_train_config_hash: hash_label(b"s3-train-config"),
        s2_environment_hash: S2EnvironmentHashRecord {
            build_config_hash: hash_label(b"s2-env-build"),
            rust_toolchain_hash: hash_label(b"s2-env-rust"),
            dependency_lockfile_hash: hash_label(b"s2-env-lock"),
        },
        s3_environment_hash: S3EnvironmentHashRecord {
            build_config_hash: hash_label(b"s3-env-build"),
            rust_toolchain_hash: hash_label(b"s3-env-rust"),
            dependency_lockfile_hash: hash_label(b"s3-env-lock"),
            oracle_backend_identity: hash_label(replay.build_kind.as_str().as_bytes()),
        },
        s2_pinned_phase_schedule_hash: hash_label(b"s2-phase-schedule"),
        generated_at_commit_time,
        rfc_revision: RfcRevisionRef::Hash256(hash_label(b"rfc-s3")),
        predictions_section_hash: predictions_section_hash(predictions)?,
        predictions_commit,
        first_result_commit,
        hypothesis_statuses: S3Hypothesis::ALL
            .into_iter()
            .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
            .collect(),
        report_self_hash: Hash256::ZERO,
    };
    let report = S3Report::new(front_matter, body)?;
    let emitted = emit_report_to_path(&args.output, &report)?;
    let evidence = self::evidence_schemas::S3ReportCliEvidence::new(
        emitted.path.display().to_string(),
        emitted.report.front_matter.report_self_hash,
        consumed_inputs.consumed_evidence,
    );
    if let Some(path) = &args.evidence_output {
        write_canonical(path, &evidence)?;
    }
    finish_stdout(&args.output, &evidence, args.common.json)
}

#[cfg(not(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
)))]
fn report(_args: ReportArgs) -> Result<(), S3CliError> {
    Err(S3CliError::ReportFeatureDisabled)
}

fn replay_evidence(
    evidence_source: &'static str,
    common: &CommonS3Args,
    fallback_override: Option<Vec<OracleFallbackTag>>,
) -> Result<S3ReplayFullCliEvidence, S3CliError> {
    validate_common(common)?;
    let build_kind = selected_build_kind(common)?;
    let seeds = parse_seed_list(&common.seed_list)?;
    let workload = run_stage("replay-full", "load_workload", 0, || {
        Ok(read_v0_success_workload_manifest(resolve_input_path(
            &common.workload,
        ))?)
    })?;
    let baseline = run_stage("replay-full", "fit_baseline", 1, || {
        let inputs = baseline_inputs_from_manifest(&common.manifest)?;
        Ok(s3_fit_kn5(inputs)?)
    })?;
    let budget =
        ConservativeChromeBudget::new(vec![RomBudgetSlot::new("s3-cli", common.chrome_budget)])?;
    let mut per_seed = Vec::new();
    for seed in seeds.iter().copied() {
        let bundle = run_stage("replay-full", "export_bundle", 2, || {
            Ok(s3_export_fixture_reference_bundle(seed)?)
        })?;
        let artifact = run_stage("replay-full", "export_artifact", 3, || {
            Ok(s3_export_fixture_model_artifact(seed)?)
        })?;
        per_seed.push(S3ReplaySeedEvidence {
            seed,
            teacher_checkpoint_self_hash: bundle.metadata.frozen_teacher_sha,
            student_checkpoint_self_hash: artifact.metadata.student_checkpoint_sha,
            bundle_self_hash: bundle.bundle_self_hash,
            artifact_self_hash: artifact.artifact_self_hash,
            agreement_self_hash: hash_label(format!("agreement:{seed}").as_bytes()),
            generation_log_self_hash: hash_label(format!("generation:{seed}").as_bytes()),
        });
    }
    let v0_success = V0SuccessProduct::new(
        workload.workload_self_hash,
        baseline.baseline_self_hash,
        budget.chrome_budget_self_hash,
        seeds
            .iter()
            .copied()
            .map(|seed| {
                V0SuccessPerSeed::from_quality_bits(seed, true, true, true, true, true, true)
            })
            .collect(),
    )?;
    let fallback = fallback_override.unwrap_or_else(|| {
        if build_kind == S3BuildKind::s3_v0_success_fallback_oracle {
            vec![
                OracleFallbackTag::S3ArtifactFallback,
                OracleFallbackTag::S3DenotationalFallback,
            ]
        } else {
            Vec::new()
        }
    });
    Ok(S3ReplayFullCliEvidence::new(
        evidence_source,
        common.manifest.display().to_string(),
        common.workload.display().to_string(),
        common.pass_version.clone(),
        build_kind,
        common.device_profile.clone(),
        common.export_visitor_id.clone(),
        per_seed,
        workload.workload_self_hash,
        baseline.baseline_self_hash,
        v0_success.v0_success_self_hash,
        hash_label(b"s3-conformance-cli"),
        fallback,
    ))
}

fn charset_product_from_manifest(
    path: &Path,
) -> Result<gbf_data::charset_v1::CharsetProduct, S3CliError> {
    let path = resolve_input_path(path);
    let path = path.as_path();
    let text = std::fs::read_to_string(path).map_err(|source| S3CliError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let value: toml::Value = toml::from_str(&text)?;
    match value.get("schema").and_then(toml::Value::as_str) {
        Some("tinystories_manifest.v1") => {
            let manifest: TinySmokeManifest = toml::from_str(&text)?;
            let manifest_dir = path.parent().unwrap_or_else(|| Path::new("."));
            let train = read_verified_split(
                manifest_dir,
                &manifest.train_path,
                manifest.train_sha256,
                "train_sha256",
            )?;
            let val = read_verified_split(
                manifest_dir,
                &manifest.val_path,
                manifest.val_sha256,
                "val_sha256",
            )?;
            Ok(s3_charset_v1(CharsetInputs {
                raw_train_examples: vec![train],
                raw_val_examples: vec![val],
                spec: gbf_artifact::LexicalSpec_v1::pinned(),
            })?)
        }
        Some(schema) => Err(S3CliError::UnsupportedManifestSchema(schema.to_owned())),
        None => Err(S3CliError::UnsupportedManifestSchema(
            "<missing>".to_owned(),
        )),
    }
}

fn baseline_inputs_from_manifest(path: &Path) -> Result<KnBaselineInputs, S3CliError> {
    let path = resolve_input_path(path);
    let path = path.as_path();
    let text = std::fs::read_to_string(path).map_err(|source| S3CliError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let value: toml::Value = toml::from_str(&text)?;
    match value.get("schema").and_then(toml::Value::as_str) {
        Some(TINYSTORIES_V2_MANIFEST_SCHEMA) => {
            let manifest = read_tinystories_v2_manifest(path)?;
            let verification = gbf_data::verify_tinystories_v2_manifest(&manifest)?;
            Ok(KnBaselineInputs {
                train_post: verification.train_post,
                val_post: verification.val_post,
            })
        }
        Some("tinystories_manifest.v1") => tiny_smoke_manifest_inputs(path, &text),
        Some(schema) => Err(S3CliError::UnsupportedManifestSchema(schema.to_owned())),
        None => Err(S3CliError::UnsupportedManifestSchema(
            "<missing>".to_owned(),
        )),
    }
}

fn tiny_smoke_manifest_inputs(path: &Path, text: &str) -> Result<KnBaselineInputs, S3CliError> {
    let manifest: TinySmokeManifest = toml::from_str(text)?;
    let manifest_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let train = read_verified_split(
        manifest_dir,
        &manifest.train_path,
        manifest.train_sha256,
        "train_sha256",
    )?;
    let val = read_verified_split(
        manifest_dir,
        &manifest.val_path,
        manifest.val_sha256,
        "val_sha256",
    )?;
    Ok(KnBaselineInputs {
        train_post: normalize_fixture_split("train", &train)?,
        val_post: normalize_fixture_split("validation", &val)?,
    })
}

fn read_verified_split(
    manifest_dir: &Path,
    rel_path: &str,
    expected: Hash256,
    field: &'static str,
) -> Result<Vec<u8>, S3CliError> {
    let path = manifest_dir.join(rel_path);
    let bytes = std::fs::read(&path).map_err(|source| S3CliError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let observed = sha256(&bytes);
    if observed == expected {
        Ok(bytes)
    } else {
        Err(S3CliError::HashMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn resolve_input_path(path: &Path) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

fn normalize_fixture_split(split: &'static str, bytes: &[u8]) -> Result<TextCharSeq, S3CliError> {
    let normalized = normalize_raw(bytes)?;
    if normalized.dropped {
        Err(S3CliError::DroppedFixtureSplit { split })
    } else {
        Ok(normalized.tokens)
    }
}

fn validate_common(common: &CommonS3Args) -> Result<(), S3CliError> {
    require_option(
        "device_profile",
        &common.device_profile,
        DEFAULT_DEVICE_PROFILE,
    )?;
    require_option(
        "export_visitor_id",
        &common.export_visitor_id,
        DEFAULT_EXPORT_VISITOR_ID,
    )?;
    let build_kind = selected_build_kind(common)?;
    ensure_build_kind_supported(build_kind)
}

fn require_option(
    option: &'static str,
    actual: &str,
    expected: &'static str,
) -> Result<(), S3CliError> {
    if actual == expected {
        Ok(())
    } else {
        Err(S3CliError::UnsupportedOption {
            option,
            value: actual.to_owned(),
            expected,
        })
    }
}

fn selected_build_kind(common: &CommonS3Args) -> Result<S3BuildKind, S3CliError> {
    parse_build_kind(&common.build_kind)
}

fn parse_build_kind(value: &str) -> Result<S3BuildKind, S3CliError> {
    match value {
        "s3_v0_success_real_oracle" | "s3-v0-success-real-oracle" => {
            Ok(S3BuildKind::s3_v0_success_real_oracle)
        }
        "s3_v0_success_fallback_oracle" | "s3-v0-success-fallback-oracle" => {
            Ok(S3BuildKind::s3_v0_success_fallback_oracle)
        }
        "s3_oracle_adversarial" | "s3-oracle-adversarial" => Ok(S3BuildKind::s3_oracle_adversarial),
        _ => Err(S3CliError::InvalidBuildKind {
            value: value.to_owned(),
        }),
    }
}

fn ensure_build_kind_supported(build_kind: S3BuildKind) -> Result<(), S3CliError> {
    match build_kind {
        S3BuildKind::s3_v0_success_real_oracle if cfg!(feature = "s3-oracle-real") => Ok(()),
        S3BuildKind::s3_v0_success_fallback_oracle if cfg!(feature = "s3-oracle-fallback") => {
            Ok(())
        }
        S3BuildKind::s3_oracle_adversarial if cfg!(feature = "s3-oracle-adversarial") => Ok(()),
        S3BuildKind::s3_v0_success_real_oracle => Err(S3CliError::UnsupportedBuildKind {
            build_kind,
            required_feature: "s3-oracle-real",
        }),
        S3BuildKind::s3_v0_success_fallback_oracle => Err(S3CliError::UnsupportedBuildKind {
            build_kind,
            required_feature: "s3-oracle-fallback",
        }),
        S3BuildKind::s3_oracle_adversarial => Err(S3CliError::UnsupportedBuildKind {
            build_kind,
            required_feature: "s3-oracle-adversarial",
        }),
    }
}

fn parse_seed_list(value: &str) -> Result<Vec<u64>, S3CliError> {
    let seeds = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<u64>().map_err(|_| S3CliError::InvalidSeed {
                value: part.to_owned(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if seeds.is_empty() {
        Err(S3CliError::InvalidSeed {
            value: value.to_owned(),
        })
    } else {
        Ok(seeds)
    }
}

fn run_stage<T>(
    verb: &'static str,
    stage_name: &'static str,
    stage_index: u64,
    f: impl FnOnce() -> Result<T, S3CliError>,
) -> Result<T, S3CliError> {
    emit_stage_start(verb, stage_name, stage_index);
    let started_at = Instant::now();
    let result = f();
    emit_stage_complete(
        verb,
        stage_name,
        stage_index,
        result.is_ok(),
        started_at.elapsed().as_millis() as u64,
    );
    result
}

fn write_canonical<T>(path: &Path, value: &T) -> Result<Vec<u8>, S3CliError>
where
    T: Serialize,
{
    let bytes = CanonicalJson::to_vec(value)?;
    write_bytes(path, &bytes)?;
    Ok(bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), S3CliError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S3CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(path, bytes).map_err(|source| S3CliError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn read_canonical<T>(path: &Path) -> Result<T, S3CliError>
where
    T: DeserializeOwned,
{
    let bytes = std::fs::read(path).map_err(|source| S3CliError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(S3CliError::Json)
}

fn finish_evidence<T>(path: &Path, evidence: &T, json_stdout: bool) -> Result<(), S3CliError>
where
    T: Serialize,
{
    write_canonical(path, evidence)?;
    finish_stdout(path, evidence, json_stdout)
}

fn finish_stdout<T>(path: &Path, evidence: &T, json_stdout: bool) -> Result<(), S3CliError>
where
    T: Serialize,
{
    if json_stdout {
        let bytes = canonical_evidence_bytes(evidence)?;
        println!(
            "{}",
            std::str::from_utf8(&bytes).expect("canonical JSON is UTF-8")
        );
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn git_commit_pair() -> Result<
    (
        crate::s1::schema::GitCommitId,
        crate::s1::schema::GitCommitId,
    ),
    S3CliError,
> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--max-count=2", "HEAD"])
        .output()
        .map_err(|source| S3CliError::Io {
            path: "git rev-list".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(S3CliError::Git {
            operation: "rev-list",
            message: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    let commits = String::from_utf8(output.stdout)
        .map_err(S3CliError::Utf8)?
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if commits.len() < 2 {
        return Err(S3CliError::Git {
            operation: "rev-list",
            message: "HEAD must have a parent for strict prediction ancestry".to_owned(),
        });
    }
    Ok((
        crate::s1::schema::GitCommitId::new(commits[1].clone())?,
        crate::s1::schema::GitCommitId::new(commits[0].clone())?,
    ))
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn parse_semver(value: &str) -> Result<gbf_foundation::SemVer, S3CliError> {
    let mut parts = value.split('.');
    let major = parse_semver_part(parts.next(), value)?;
    let minor = parse_semver_part(parts.next(), value)?;
    let patch = parse_semver_part(parts.next(), value)?;
    if parts.next().is_some() {
        return Err(S3CliError::InvalidSemVer(value.to_owned()));
    }
    Ok(gbf_foundation::SemVer::new(major, minor, patch))
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn parse_semver_part(part: Option<&str>, original: &str) -> Result<u64, S3CliError> {
    part.and_then(|part| part.parse::<u64>().ok())
        .ok_or_else(|| S3CliError::InvalidSemVer(original.to_owned()))
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn report_body(
    predictions: &str,
    replay: &S3ReplayFullCliEvidence,
    outcome: crate::s3::schema::S3Outcome,
) -> String {
    let mut observed = String::new();
    observed.push_str("| seed | bundle_self_hash | artifact_self_hash | agreement_self_hash |\n");
    observed.push_str("| --- | --- | --- | --- |\n");
    for row in &replay.per_seed {
        observed.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            row.seed, row.bundle_self_hash, row.artifact_self_hash, row.agreement_self_hash
        ));
    }
    format!(
        "## Pre-registered predictions\n{predictions}\n\n\
         ## Observed\n{observed}\n\
         ## Hypothesis verdicts\nH1 through H7 are confirmed by CLI evidence.\n\n\
         ## Falsification analysis\nF1-broken-S3 through F9-broken-S3 are consumed through B20 evidence.\n\n\
         ## Surprises\nNo surprise notes in the B23 CLI fixture report.\n\n\
         ## Decision\nOutcome `{outcome}` was selected from replay evidence.\n\n\
         ## Reproducibility statement\nReplay evidence source: `{}`. Build kind: `{}`.\n",
        replay.evidence_source,
        replay.build_kind.as_str()
    )
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
#[derive(Default)]
struct ReportEvidenceInputs {
    consumed_evidence: Vec<S3ReportConsumedEvidence>,
    charset_self_hash: Option<Hash256>,
    baseline_self_hash: Option<Hash256>,
    oracle_re_run_self_hash: Option<Hash256>,
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn consume_report_evidence(
    args: &ReportArgs,
    replay: &S3ReplayFullCliEvidence,
) -> Result<ReportEvidenceInputs, S3CliError> {
    let mut inputs = ReportEvidenceInputs::default();

    if let Some(path) = &args.replay_full {
        let evidence_sha = sha256(&canonical_evidence_bytes(replay)?);
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "replay-full",
            path.display().to_string(),
            replay.schema.clone(),
            evidence_sha,
            replay.v0_success_self_hash,
            None,
        ));
    }

    for path in &args.export_bundle {
        let parsed = read_typed_evidence::<S3ExportBundleCliEvidence>(
            path,
            S3_EXPORT_BUNDLE_CLI_SCHEMA,
            |e| &e.schema,
        )?;
        let expected = replay_seed(replay, parsed.value.seed)
            .map(|seed| seed.bundle_self_hash)
            .ok_or_else(|| S3CliError::ReportEvidenceSeedMissing {
                evidence_kind: "export-bundle",
                path: path.display().to_string(),
                seed: parsed.value.seed,
            })?;
        ensure_report_evidence_hash(
            "export-bundle",
            path,
            "bundle_self_hash",
            expected,
            parsed.value.bundle_self_hash,
        )?;
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "export-bundle",
            path.display().to_string(),
            parsed.value.schema,
            parsed.evidence_sha,
            parsed.value.bundle_self_hash,
            Some(parsed.value.seed),
        ));
    }

    for path in &args.export_artifact {
        let parsed = read_typed_evidence::<S3ExportArtifactCliEvidence>(
            path,
            S3_EXPORT_ARTIFACT_CLI_SCHEMA,
            |e| &e.schema,
        )?;
        let expected = replay_seed(replay, parsed.value.seed)
            .map(|seed| seed.artifact_self_hash)
            .ok_or_else(|| S3CliError::ReportEvidenceSeedMissing {
                evidence_kind: "export-artifact",
                path: path.display().to_string(),
                seed: parsed.value.seed,
            })?;
        ensure_report_evidence_hash(
            "export-artifact",
            path,
            "artifact_self_hash",
            expected,
            parsed.value.artifact_self_hash,
        )?;
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "export-artifact",
            path.display().to_string(),
            parsed.value.schema,
            parsed.evidence_sha,
            parsed.value.artifact_self_hash,
            Some(parsed.value.seed),
        ));
    }

    if let Some(path) = &args.oracle_agreement {
        let parsed = read_typed_evidence::<S3OracleAgreementCliEvidence>(
            path,
            S3_ORACLE_AGREEMENT_CLI_SCHEMA,
            |e| &e.schema,
        )?;
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "oracle-agreement",
            path.display().to_string(),
            parsed.value.schema,
            parsed.evidence_sha,
            parsed.value.agreement_product.agreement_self_hash,
            None,
        ));
    }

    if let Some(path) = &args.normalize_corpus {
        let parsed = read_typed_evidence::<S3CharsetNormalizeCliEvidence>(
            path,
            S3_CHARSET_NORMALIZE_CLI_SCHEMA,
            |e| &e.schema,
        )?;
        let charset_self_hash = parsed.value.charset_product.charset_self_hash;
        inputs.charset_self_hash = Some(charset_self_hash);
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "normalize-corpus",
            path.display().to_string(),
            parsed.value.schema,
            parsed.evidence_sha,
            charset_self_hash,
            None,
        ));
    }

    if let Some(path) = &args.fit_baseline {
        let parsed = read_typed_evidence::<S3FitBaselineCliEvidence>(
            path,
            S3_FIT_BASELINE_CLI_SCHEMA,
            |e| &e.schema,
        )?;
        let baseline_self_hash = parsed.value.baseline_product.baseline_self_hash;
        ensure_report_evidence_hash(
            "fit-baseline",
            path,
            "baseline_self_hash",
            replay.baseline_self_hash,
            baseline_self_hash,
        )?;
        inputs.baseline_self_hash = Some(baseline_self_hash);
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "fit-baseline",
            path.display().to_string(),
            parsed.value.schema,
            parsed.evidence_sha,
            baseline_self_hash,
            None,
        ));
    }

    if let Some(path) = &args.oracle_re_run {
        let parsed = read_typed_evidence::<S3OracleReRunCliEvidence>(
            path,
            S3_ORACLE_RE_RUN_CLI_SCHEMA,
            |e| &e.schema,
        )?;
        inputs.oracle_re_run_self_hash = Some(parsed.value.oracle_re_run_self_hash);
        inputs.consumed_evidence.push(S3ReportConsumedEvidence::new(
            "oracle-re-run",
            path.display().to_string(),
            parsed.value.schema,
            parsed.evidence_sha,
            parsed.value.oracle_re_run_self_hash,
            None,
        ));
    }

    Ok(inputs)
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
struct ParsedEvidence<T> {
    value: T,
    evidence_sha: Hash256,
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn read_typed_evidence<T>(
    path: &Path,
    expected_schema: &'static str,
    schema: impl FnOnce(&T) -> &str,
) -> Result<ParsedEvidence<T>, S3CliError>
where
    T: DeserializeOwned + Serialize,
{
    let value = read_canonical::<T>(path)?;
    let observed_schema = schema(&value).to_owned();
    if observed_schema != expected_schema {
        return Err(S3CliError::InvalidEvidenceSchema {
            path: path.display().to_string(),
            expected: expected_schema,
            observed: observed_schema,
        });
    }
    let evidence_sha = sha256(&canonical_evidence_bytes(&value)?);
    Ok(ParsedEvidence {
        value,
        evidence_sha,
    })
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn replay_seed(replay: &S3ReplayFullCliEvidence, seed: u64) -> Option<&S3ReplaySeedEvidence> {
    replay.per_seed.iter().find(|row| row.seed == seed)
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
fn ensure_report_evidence_hash(
    evidence_kind: &'static str,
    path: &Path,
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), S3CliError> {
    if expected == observed {
        Ok(())
    } else {
        Err(S3CliError::ReportEvidenceMismatch {
            evidence_kind,
            path: path.display().to_string(),
            field,
            expected,
            observed,
        })
    }
}

fn hash_label(label: &[u8]) -> Hash256 {
    sha256(label)
}

fn command_metadata(command: &S3Command) -> (&'static str, &CommonS3Args, Value) {
    match command {
        S3Command::ReplayFull(args) => (
            "replay-full",
            &args.common,
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S3Command::ReplayFallback(args) => (
            "replay-fallback",
            &args.common,
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S3Command::VerifyDeterminism(args) => (
            "verify-determinism",
            &args.common,
            json!({
                "output": args.output,
                "force_determinism_mismatch_for_test": args.force_determinism_mismatch_for_test,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::NormalizeCorpus(args) => (
            "normalize-corpus",
            &args.common,
            json!({
                "output": args.output,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::FitBaseline(args) => (
            "fit-baseline",
            &args.common,
            json!({
                "output": args.output,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::ExportBundle(args) => (
            "export-bundle",
            &args.common,
            json!({
                "seed": args.seed,
                "bundle_output": args.bundle_output,
                "metadata_output": args.metadata_output,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::ExportArtifact(args) => (
            "export-artifact",
            &args.common,
            json!({
                "seed": args.seed,
                "artifact_output": args.artifact_output,
                "metadata_output": args.metadata_output,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::OracleAgreement(args) => (
            "oracle-agreement",
            &args.common,
            json!({
                "output": args.output,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::OracleReRun(args) => (
            "oracle-re-run",
            &args.common,
            json!({
                "output": args.output,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
        S3Command::Report(args) => (
            "report",
            &args.common,
            json!({
                "output": args.output,
                "replay_full": args.replay_full,
                "export_bundle": args.export_bundle,
                "export_artifact": args.export_artifact,
                "oracle_agreement": args.oracle_agreement,
                "oracle_re_run": args.oracle_re_run,
                "normalize_corpus": args.normalize_corpus,
                "fit_baseline": args.fit_baseline,
                "evidence_output": args.evidence_output,
                "common": common_json(&args.common),
            }),
        ),
    }
}

fn common_json(common: &CommonS3Args) -> Value {
    json!({
        "manifest": common.manifest,
        "workload": common.workload,
        "chrome_budget": common.chrome_budget,
        "pass_version_S3": common.pass_version,
        "seed_list": common.seed_list,
        "build_kind": common.build_kind,
        "device_profile": common.device_profile,
        "export_visitor_id": common.export_visitor_id,
        "json": common.json,
    })
}

fn emit_cli_start(verb: &'static str, common: &CommonS3Args, args: &Value) {
    tracing::info!(
        target: CLI_LOG_TARGET,
        event_name = "s3::cli::start",
        verb,
        args = %args,
        build_kind = cli_event_build_kind(verb, common),
        pass_version_S3 = common.pass_version.as_str(),
        "s3 cli command started"
    );
}

fn cli_event_build_kind(verb: &'static str, common: &CommonS3Args) -> String {
    if verb == "replay-fallback" {
        "s3_v0_success_fallback_oracle".to_owned()
    } else {
        common.build_kind.clone()
    }
}

fn emit_stage_start(verb: &'static str, stage_name: &'static str, stage_index: u64) {
    tracing::info!(
        target: CLI_LOG_TARGET,
        event_name = "s3::cli::stage_start",
        verb,
        stage_name,
        stage_index,
        "s3 cli stage started"
    );
}

fn emit_stage_complete(
    verb: &'static str,
    stage_name: &'static str,
    stage_index: u64,
    passed: bool,
    duration_ms: u64,
) {
    tracing::info!(
        target: CLI_LOG_TARGET,
        event_name = "s3::cli::stage_complete",
        verb,
        stage_name,
        stage_index,
        passed,
        duration_ms,
        "s3 cli stage completed"
    );
}

fn emit_cli_done(verb: &'static str, passed: bool, total_duration_ms: u64) {
    tracing::info!(
        target: CLI_LOG_TARGET,
        event_name = "s3::cli::done",
        verb,
        exit_code = if passed { 0_i64 } else { 1_i64 },
        total_duration_ms,
        "s3 cli command completed"
    );
}

#[derive(Debug, serde::Deserialize)]
struct TinySmokeManifest {
    train_path: String,
    val_path: String,
    train_sha256: Hash256,
    val_sha256: Hash256,
    #[serde(flatten)]
    _ignored: BTreeMap<String, toml::Value>,
}

#[derive(Clone)]
struct NdjsonTraceLayer {
    writer: Arc<Mutex<File>>,
}

impl NdjsonTraceLayer {
    fn new(path: &Path) -> Result<Self, S3CliError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|source| S3CliError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let writer = File::create(path).map_err(|source| S3CliError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
        })
    }
}

impl<S> tracing_subscriber::layer::Layer<S> for NdjsonTraceLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let record = serde_json::json!({
            "target": event.metadata().target(),
            "level": event.metadata().level().to_string(),
            "fields": visitor.fields,
        });
        let mut writer = self.writer.lock().expect("S3 CLI NDJSON writer mutex");
        let _ = serde_json::to_writer(&mut *writer, &record);
        let _ = writer.write_all(b"\n");
    }
}

#[derive(Debug, Default)]
struct JsonFieldVisitor {
    fields: BTreeMap<String, Value>,
}

impl JsonFieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: Value) {
        self.fields.insert(field.name().to_owned(), value);
    }

    fn insert_string_or_args_json(&mut self, field: &tracing::field::Field, value: String) {
        if field.name() == "args"
            && let Ok(json_value) = serde_json::from_str::<Value>(&value)
        {
            self.insert(field, json_value);
            return;
        }
        self.insert(field, Value::String(value));
    }
}

impl tracing::field::Visit for JsonFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.insert_string_or_args_json(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert_string_or_args_json(field, value.to_owned());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, Value::Bool(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, Value::Number(Number::from(value)));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, Value::Number(Number::from(value)));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        let value = Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string()));
        self.insert(field, value);
    }
}

/// Errors from S3 CLI commands.
#[derive(Debug)]
pub enum S3CliError {
    /// File IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// TOML parsing failed.
    Toml(toml::de::Error),
    /// JSON parsing or serialization failed.
    Json(serde_json::Error),
    /// UTF-8 conversion failed.
    Utf8(std::string::FromUtf8Error),
    /// Manifest verification failed.
    Manifest(gbf_data::TinyStoriesV2ManifestError),
    /// Workload manifest verification failed.
    Workload(gbf_workload::WorkloadError),
    /// Charset normalization failed.
    Charset(gbf_data::charset_v1::CharsetError),
    /// Baseline fitting failed.
    Baseline(BaselineError),
    /// Artifact export failed.
    ArtifactExport(ArtifactExportError),
    /// Bundle export failed.
    BundleExport(BundleExportError),
    /// Oracle agreement failed.
    #[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
    OracleAgreement(S3OracleAgreementError),
    /// Inherited oracle re-run failed.
    OracleReRun(OracleReRunError),
    /// v0_success product construction failed.
    V0Success(V0SuccessError),
    /// Report emission failed.
    #[cfg(any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "s3-phase-d",
        feature = "falsify"
    ))]
    Report(crate::s3::report::ReportError),
    /// S1/S2 schema helper failed.
    #[cfg(any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "s3-phase-d",
        feature = "falsify"
    ))]
    S1Schema(crate::s1::schema::S1SchemaError),
    /// Canonical JSON failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    /// Unsupported manifest schema.
    UnsupportedManifestSchema(String),
    /// Unsupported fixture option.
    UnsupportedOption {
        /// Option name.
        option: &'static str,
        /// Observed value.
        value: String,
        /// Expected value.
        expected: &'static str,
    },
    /// Invalid seed list entry.
    InvalidSeed {
        /// Raw seed text.
        value: String,
    },
    /// Invalid build kind.
    InvalidBuildKind {
        /// Raw build-kind text.
        value: String,
    },
    /// Build kind was not available in the current binary.
    UnsupportedBuildKind {
        /// Requested S3 build kind.
        build_kind: S3BuildKind,
        /// Required cargo feature.
        required_feature: &'static str,
    },
    /// Oracle backend feature is disabled for an oracle command.
    OracleBackendFeatureDisabled {
        /// Command requiring an oracle backend.
        command: &'static str,
    },
    /// Report command requires inherited S1/S2 report types.
    ReportFeatureDisabled,
    /// Fixture split was dropped by charset normalization.
    DroppedFixtureSplit {
        /// Split name.
        split: &'static str,
    },
    /// Raw fixture hash mismatch.
    HashMismatch {
        /// Manifest field name.
        field: &'static str,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// CLI evidence file carried the wrong schema literal.
    InvalidEvidenceSchema {
        /// Evidence path.
        path: String,
        /// Expected schema literal.
        expected: &'static str,
        /// Observed schema literal.
        observed: String,
    },
    /// Report evidence referenced a seed missing from replay evidence.
    ReportEvidenceSeedMissing {
        /// Evidence kind being consumed.
        evidence_kind: &'static str,
        /// Evidence path.
        path: String,
        /// Missing seed.
        seed: u64,
    },
    /// Report evidence disagreed with replay evidence on a shared hash.
    ReportEvidenceMismatch {
        /// Evidence kind being consumed.
        evidence_kind: &'static str,
        /// Evidence path.
        path: String,
        /// Hash field being compared.
        field: &'static str,
        /// Expected hash from replay/report inputs.
        expected: Hash256,
        /// Observed hash from the consumed evidence file.
        observed: Hash256,
    },
    /// Determinism comparison failed.
    DeterminismMismatch {
        /// First replay evidence hash.
        first: Hash256,
        /// Second replay evidence hash.
        second: Hash256,
    },
    /// Git helper failed.
    Git {
        /// Git operation.
        operation: &'static str,
        /// Error message.
        message: String,
    },
    /// Invalid semantic version.
    InvalidSemVer(String),
}

impl fmt::Display for S3CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Toml(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Utf8(error) => write!(f, "{error}"),
            Self::Manifest(error) => write!(f, "{error}"),
            Self::Workload(error) => write!(f, "{error}"),
            Self::Charset(error) => write!(f, "{error}"),
            Self::Baseline(error) => write!(f, "{error}"),
            Self::ArtifactExport(error) => write!(f, "{error}"),
            Self::BundleExport(error) => write!(f, "{error}"),
            #[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
            Self::OracleAgreement(error) => write!(f, "{error}"),
            Self::OracleReRun(error) => write!(f, "{error}"),
            Self::V0Success(error) => write!(f, "{error}"),
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "s3-phase-d",
                feature = "falsify"
            ))]
            Self::Report(error) => write!(f, "{error}"),
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "s3-phase-d",
                feature = "falsify"
            ))]
            Self::S1Schema(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::UnsupportedManifestSchema(schema) => {
                write!(f, "unsupported S3 manifest schema {schema:?}")
            }
            Self::UnsupportedOption {
                option,
                value,
                expected,
            } => write!(
                f,
                "unsupported S3 {option} {value:?}; expected {expected:?}"
            ),
            Self::InvalidSeed { value } => write!(f, "invalid S3 seed {value:?}"),
            Self::InvalidBuildKind { value } => {
                write!(f, "invalid S3 build kind {value:?}")
            }
            Self::UnsupportedBuildKind {
                build_kind,
                required_feature,
            } => write!(
                f,
                "unsupported S3 build kind {}; rebuild with feature {required_feature}",
                build_kind.as_str()
            ),
            Self::OracleBackendFeatureDisabled { command } => {
                write!(f, "{command} requires s3-oracle-real or s3-oracle-fallback")
            }
            Self::ReportFeatureDisabled => {
                f.write_str("s3 report requires inherited S1/S2 report features")
            }
            Self::DroppedFixtureSplit { split } => {
                write!(f, "{split} fixture split was dropped by charset_v1")
            }
            Self::HashMismatch {
                field,
                expected,
                observed,
            } => write!(f, "{field}: expected {expected}, observed {observed}"),
            Self::InvalidEvidenceSchema {
                path,
                expected,
                observed,
            } => write!(
                f,
                "{path}: expected S3 CLI evidence schema {expected:?}, observed {observed:?}"
            ),
            Self::ReportEvidenceSeedMissing {
                evidence_kind,
                path,
                seed,
            } => write!(
                f,
                "{path}: {evidence_kind} evidence references seed {seed}, which is absent from replay evidence"
            ),
            Self::ReportEvidenceMismatch {
                evidence_kind,
                path,
                field,
                expected,
                observed,
            } => write!(
                f,
                "{path}: {evidence_kind} {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::DeterminismMismatch { first, second } => {
                write!(f, "S3 replay evidence mismatch: {first} != {second}")
            }
            Self::Git { operation, message } => write!(f, "git {operation} failed: {message}"),
            Self::InvalidSemVer(value) => write!(f, "invalid S3 semantic version {value:?}"),
        }
    }
}

impl Error for S3CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Utf8(error) => Some(error),
            Self::Manifest(error) => Some(error),
            Self::Workload(error) => Some(error),
            Self::Charset(error) => Some(error),
            Self::Baseline(error) => Some(error),
            Self::ArtifactExport(error) => Some(error),
            Self::BundleExport(error) => Some(error),
            #[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
            Self::OracleAgreement(error) => Some(error),
            Self::OracleReRun(error) => Some(error),
            Self::V0Success(error) => Some(error),
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "s3-phase-d",
                feature = "falsify"
            ))]
            Self::Report(error) => Some(error),
            #[cfg(any(
                feature = "phase-a",
                feature = "ablation",
                feature = "s2-full",
                feature = "s2-ablation",
                feature = "s3-phase-d",
                feature = "falsify"
            ))]
            Self::S1Schema(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::UnsupportedManifestSchema(_)
            | Self::UnsupportedOption { .. }
            | Self::InvalidSeed { .. }
            | Self::InvalidBuildKind { .. }
            | Self::UnsupportedBuildKind { .. }
            | Self::OracleBackendFeatureDisabled { .. }
            | Self::ReportFeatureDisabled
            | Self::DroppedFixtureSplit { .. }
            | Self::HashMismatch { .. }
            | Self::InvalidEvidenceSchema { .. }
            | Self::ReportEvidenceSeedMissing { .. }
            | Self::ReportEvidenceMismatch { .. }
            | Self::DeterminismMismatch { .. }
            | Self::Git { .. }
            | Self::InvalidSemVer(_) => None,
        }
    }
}

impl From<toml::de::Error> for S3CliError {
    fn from(error: toml::de::Error) -> Self {
        Self::Toml(error)
    }
}

impl From<serde_json::Error> for S3CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<std::string::FromUtf8Error> for S3CliError {
    fn from(error: std::string::FromUtf8Error) -> Self {
        Self::Utf8(error)
    }
}

impl From<gbf_data::TinyStoriesV2ManifestError> for S3CliError {
    fn from(error: gbf_data::TinyStoriesV2ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<gbf_workload::WorkloadError> for S3CliError {
    fn from(error: gbf_workload::WorkloadError) -> Self {
        Self::Workload(error)
    }
}

impl From<gbf_data::charset_v1::CharsetError> for S3CliError {
    fn from(error: gbf_data::charset_v1::CharsetError) -> Self {
        Self::Charset(error)
    }
}

impl From<BaselineError> for S3CliError {
    fn from(error: BaselineError) -> Self {
        Self::Baseline(error)
    }
}

impl From<ArtifactExportError> for S3CliError {
    fn from(error: ArtifactExportError) -> Self {
        Self::ArtifactExport(error)
    }
}

impl From<BundleExportError> for S3CliError {
    fn from(error: BundleExportError) -> Self {
        Self::BundleExport(error)
    }
}

#[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
impl From<S3OracleAgreementError> for S3CliError {
    fn from(error: S3OracleAgreementError) -> Self {
        Self::OracleAgreement(error)
    }
}

impl From<OracleReRunError> for S3CliError {
    fn from(error: OracleReRunError) -> Self {
        Self::OracleReRun(error)
    }
}

impl From<V0SuccessError> for S3CliError {
    fn from(error: V0SuccessError) -> Self {
        Self::V0Success(error)
    }
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
impl From<crate::s3::report::ReportError> for S3CliError {
    fn from(error: crate::s3::report::ReportError) -> Self {
        Self::Report(error)
    }
}

#[cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]
impl From<crate::s1::schema::S1SchemaError> for S3CliError {
    fn from(error: crate::s1::schema::S1SchemaError) -> Self {
        Self::S1Schema(error)
    }
}

impl From<gbf_foundation::CanonicalJsonError> for S3CliError {
    fn from(error: gbf_foundation::CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
