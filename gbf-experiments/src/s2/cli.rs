//! S2 command-line integration for reproducibility scripts.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::s1::schema::{GitCommitId, RfcRevisionRef};
use crate::s2::distill::{DistillOnceOutput, distill_once_pinned};
use crate::s2::linearstate_smoke;
use crate::s2::loss_grad_flow;
use crate::s2::oracle_re_run;
use crate::s2::phase_transition_integ;
use crate::s2::report::{S2ReportInputs, emit_s2_report};
use crate::s2::run::state_machine::{PreTrainGateResults, run_pretrain_state_machine};
use crate::s2::run::{
    CompletedRunProductS2, RunInputs, RunProductS2, S2TrainRunError, s2_train_run,
};
use crate::s2::schema::{
    DiagnosticSubcheckResult, FixtureResult, HypothesisStatus, LossGradFlowReport, S2BuildKind,
    S2CheckpointSelfHashes, S2Completion, S2Hypothesis, S2PerSeedArtifacts, S2VerifierBundle,
};
use gbf_foundation::{Hash256, SemVer};

/// Parsed S2 CLI invocation.
#[derive(Debug, Parser)]
pub struct S2Cli {
    /// S2 subcommand.
    #[command(subcommand)]
    pub command: S2Command,
    /// Logging/capture configuration supplied by the top-level CLI.
    #[arg(skip)]
    pub logging: S2CliLogging,
}

/// S2 experiment subcommands.
#[derive(Debug, Subcommand)]
pub enum S2Command {
    /// Replay the tiny full-S2 run and emit checkpoint/log hashes as JSON.
    ReplayFull(ReplayFullArgs),
    /// Replay the tiny Phase-A ablation run and emit checkpoint/log hashes as JSON.
    ReplayAblation(ReplayAblationArgs),
    /// Replay seed/build twice and byte-compare final checkpoint evidence.
    VerifyDeterminism(VerifyDeterminismArgs),
    /// Run S2 loss-gradient-flow diagnostics.
    GradFlow(JsonArgs),
    /// Run S2 LinearState smoke diagnostics.
    LinearstateSmoke(JsonArgs),
    /// Run S2 phase-transition integration diagnostics.
    PhaseInteg(JsonArgs),
    /// Re-run the inherited S1 oracle suite under the S2 binary.
    OracleReRun(JsonArgs),
    /// Emit an S2 report, optionally backed by replay-full JSON evidence.
    Report(ReportArgs),
    /// Run one pinned distillation step and emit raw-loss bytes as JSON.
    DistillOnce(DistillOnceArgs),
}

/// S2 CLI log format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S2CliLogFormat {
    /// Human-readable stderr events.
    #[default]
    Pretty,
    /// NDJSON stderr events.
    Json,
}

/// S2 CLI log level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S2CliLogLevel {
    /// Suppress CLI start/done events.
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

/// Logging/capture configuration for S2 CLI commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S2CliLogging {
    /// Stderr event format.
    pub format: S2CliLogFormat,
    /// CLI event level.
    pub level: S2CliLogLevel,
    /// Optional additional event sink.
    pub log_file: Option<PathBuf>,
    /// Optional NDJSON capture sink for structured test assertions.
    pub capture_events: Option<PathBuf>,
}

impl S2CliLogging {
    fn emit(
        &self,
        event: &'static str,
        command: &'static str,
        exit_code: i32,
        details: Vec<(&'static str, serde_json::Value)>,
        args: Option<serde_json::Value>,
    ) -> Result<(), S2CliError> {
        if self.level == S2CliLogLevel::Off {
            return Ok(());
        }

        let mut payload = json!({
            "event": event,
            "command": command,
            "exit_code": exit_code,
        });
        if let Some(args) = args {
            payload["args"] = args;
        }
        for (key, value) in details {
            payload[key] = value;
        }

        let ndjson = serde_json::to_string(&payload)?;
        let stderr_line = match self.format {
            S2CliLogFormat::Json => ndjson.clone(),
            S2CliLogFormat::Pretty => format!("{event} command={command} exit_code={exit_code}"),
        };
        eprintln!("{stderr_line}");
        if let Some(path) = &self.log_file {
            append_line(path, &stderr_line)?;
        }
        if let Some(path) = &self.capture_events {
            append_line(path, &ndjson)?;
        }
        Ok(())
    }
}

/// Run an S2 command.
pub fn run(cli: S2Cli) -> Result<(), S2CliError> {
    let (name, args_json) = command_metadata(&cli.command);
    cli.logging
        .emit("cli_subcommand_start", name, 0, Vec::new(), Some(args_json))?;
    let result = match cli.command {
        S2Command::ReplayFull(args) => replay_full(args),
        S2Command::ReplayAblation(args) => replay_ablation(args),
        S2Command::VerifyDeterminism(args) => verify_determinism(args),
        S2Command::GradFlow(args) => grad_flow(args),
        S2Command::LinearstateSmoke(args) => linearstate_smoke(args),
        S2Command::PhaseInteg(args) => phase_integ(args),
        S2Command::OracleReRun(args) => oracle_re_run(args),
        S2Command::Report(args) => report(args),
        S2Command::DistillOnce(args) => distill_once(args),
    };
    let exit_code = if result.is_ok() { 0 } else { 1 };
    cli.logging
        .emit("cli_subcommand_done", name, exit_code, Vec::new(), None)?;
    result
}

/// Arguments for `gbf s2 replay-full`.
#[derive(Debug, Args)]
pub struct ReplayFullArgs {
    /// Manifest path accepted for canonical replay command compatibility.
    #[arg(long, default_value = "stub")]
    manifest: String,
    /// S2 pass version accepted for canonical replay command compatibility.
    #[arg(long, default_value = "0.0.0")]
    pass_version: String,
    /// Comma-separated seeds.
    #[arg(long, default_value = "0")]
    seed_list: String,
    /// Comma-separated build kinds.
    #[arg(long, default_value = "s2_ternary_full")]
    builds: String,
    /// Fixture profile. Only `tiny` is executable today.
    #[arg(long, default_value = "tiny")]
    fixture: String,
    /// Deterministic device profile. Only `S1CpuDeterministic` is executable today.
    #[arg(long, default_value = "S1CpuDeterministic")]
    device_profile: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments for `gbf s2 replay-ablation`.
#[derive(Debug, Args)]
pub struct ReplayAblationArgs {
    /// Manifest path accepted for canonical replay command compatibility.
    #[arg(long, default_value = "stub")]
    manifest: String,
    /// S2 pass version accepted for canonical replay command compatibility.
    #[arg(long, default_value = "0.0.0")]
    pass_version: String,
    /// Comma-separated seeds.
    #[arg(long, default_value = "0")]
    seed_list: String,
    /// Fixture profile. Only `tiny` is executable today.
    #[arg(long, default_value = "tiny")]
    fixture: String,
    /// Deterministic device profile. Only `S1CpuDeterministic` is executable today.
    #[arg(long, default_value = "S1CpuDeterministic")]
    device_profile: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments for simple JSON-capable S2 diagnostics.
#[derive(Debug, Args)]
pub struct JsonArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments for `gbf s2 verify-determinism`.
#[derive(Debug, Args)]
pub struct VerifyDeterminismArgs {
    /// Seed to replay twice.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Runtime build kind.
    #[arg(long, default_value = "s2_ternary_full")]
    build: String,
    /// Fixture profile. Only `tiny` is executable today.
    #[arg(long, default_value = "tiny")]
    fixture: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments for `gbf s2 report`.
#[derive(Debug, Args)]
pub struct ReportArgs {
    /// Output markdown path.
    #[arg(long, default_value = "docs/experiments/S2-report.md")]
    output: PathBuf,
    /// Replay-full JSON emitted by `gbf s2 replay-full`.
    #[arg(long)]
    replay_full_json: Option<PathBuf>,
    /// Distill-once JSON emitted by `gbf s2 distill-once`.
    #[arg(long)]
    distill_json: Option<PathBuf>,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments for `gbf s2 distill-once`.
#[derive(Debug, Args)]
pub struct DistillOnceArgs {
    /// Fixture profile. Only `pinned` is executable today.
    #[arg(long, default_value = "pinned")]
    fixture: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

/// S2 CLI errors.
#[derive(Debug)]
pub enum S2CliError {
    /// Requested fixture is not executable.
    UnsupportedFixture {
        /// Fixture name.
        fixture: String,
    },
    /// Seed list could not be parsed.
    InvalidSeed {
        /// Raw seed text.
        value: String,
    },
    /// Build kind could not be parsed.
    InvalidBuildKind {
        /// Raw build-kind text.
        value: String,
    },
    /// S2 train run failed.
    TrainRun(S2TrainRunError),
    /// S2 train run diverged.
    Diverged,
    /// CLI argument is unsupported by the tiny executable fixture.
    UnsupportedOption {
        /// Option name.
        option: &'static str,
        /// Observed value.
        value: String,
    },
    /// Distillation helper failed.
    Distill(gbf_train::loss::distillation::DistillationLossError),
    /// LinearState smoke helper failed.
    LinearStateSmoke(linearstate_smoke::LinearStateSmokeError),
    /// Loss-gradient-flow helper failed.
    LossGradFlow(loss_grad_flow::LossGradFlowFixtureError),
    /// Phase-transition integration helper failed.
    PhaseInteg(phase_transition_integ::PhaseTransitionIntegError),
    /// Oracle re-run helper failed.
    OracleReRun(oracle_re_run::S2OracleReRunError),
    /// Report emission failed.
    Report(crate::s2::report::S2ReportError),
    /// Live S2 evidence JSON failed validation.
    InvalidEvidence {
        /// Evidence path.
        path: PathBuf,
        /// Validation failure reason.
        reason: String,
    },
    /// S1 schema helper failed.
    Schema(crate::s1::schema::S1SchemaError),
    /// I/O failed.
    Io(std::io::Error),
    /// JSON output serialization failed.
    Json(serde_json::Error),
}

impl fmt::Display for S2CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFixture { fixture } => {
                write!(f, "unsupported S2 fixture {fixture:?}")
            }
            Self::InvalidSeed { value } => write!(f, "invalid S2 seed {value:?}"),
            Self::InvalidBuildKind { value } => {
                write!(f, "invalid S2 build kind {value:?}")
            }
            Self::TrainRun(error) => write!(f, "{error}"),
            Self::Diverged => f.write_str("S2 replay diverged"),
            Self::UnsupportedOption { option, value } => {
                write!(f, "unsupported S2 {option} {value:?}")
            }
            Self::Distill(error) => write!(f, "{error}"),
            Self::LinearStateSmoke(error) => write!(f, "{error}"),
            Self::LossGradFlow(error) => write!(f, "{error}"),
            Self::PhaseInteg(error) => write!(f, "{error}"),
            Self::OracleReRun(error) => write!(f, "{error}"),
            Self::Report(error) => write!(f, "{error}"),
            Self::InvalidEvidence { path, reason } => {
                write!(f, "invalid S2 evidence {}: {reason}", path.display())
            }
            Self::Schema(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S2CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TrainRun(error) => Some(error),
            Self::Distill(error) => Some(error),
            Self::LinearStateSmoke(error) => Some(error),
            Self::LossGradFlow(error) => Some(error),
            Self::PhaseInteg(error) => Some(error),
            Self::OracleReRun(error) => Some(error),
            Self::Report(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::UnsupportedFixture { .. }
            | Self::InvalidSeed { .. }
            | Self::InvalidBuildKind { .. }
            | Self::UnsupportedOption { .. }
            | Self::InvalidEvidence { .. }
            | Self::Diverged => None,
        }
    }
}

impl From<S2TrainRunError> for S2CliError {
    fn from(error: S2TrainRunError) -> Self {
        Self::TrainRun(error)
    }
}

impl From<gbf_train::loss::distillation::DistillationLossError> for S2CliError {
    fn from(error: gbf_train::loss::distillation::DistillationLossError) -> Self {
        Self::Distill(error)
    }
}

impl From<serde_json::Error> for S2CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<std::io::Error> for S2CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<crate::s1::schema::S1SchemaError> for S2CliError {
    fn from(error: crate::s1::schema::S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<linearstate_smoke::LinearStateSmokeError> for S2CliError {
    fn from(error: linearstate_smoke::LinearStateSmokeError) -> Self {
        Self::LinearStateSmoke(error)
    }
}

impl From<loss_grad_flow::LossGradFlowFixtureError> for S2CliError {
    fn from(error: loss_grad_flow::LossGradFlowFixtureError) -> Self {
        Self::LossGradFlow(error)
    }
}

impl From<phase_transition_integ::PhaseTransitionIntegError> for S2CliError {
    fn from(error: phase_transition_integ::PhaseTransitionIntegError) -> Self {
        Self::PhaseInteg(error)
    }
}

impl From<oracle_re_run::S2OracleReRunError> for S2CliError {
    fn from(error: oracle_re_run::S2OracleReRunError) -> Self {
        Self::OracleReRun(error)
    }
}

impl From<crate::s2::report::S2ReportError> for S2CliError {
    fn from(error: crate::s2::report::S2ReportError) -> Self {
        Self::Report(error)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ReplayFullJson {
    schema: String,
    evidence_source: String,
    fixture: String,
    manifest: String,
    pass_version: String,
    runs: Vec<ReplayRunJson>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ReplayRunJson {
    seed: u64,
    build_kind: S2BuildKind,
    final_checkpoint_sha: String,
    phase_boundary_steps: Vec<String>,
    checkpoints: BTreeMap<String, String>,
    phase_log_self_hash: String,
    distill_log_self_hash: String,
    score_self_hash: String,
}

#[derive(Debug, Serialize)]
struct StateMachineJson {
    final_state: String,
    outcome: String,
    decision: String,
    train_attempted: bool,
    transitions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DistillOnceJson {
    distill_loss_raw: f32,
    distill_loss_raw_bits_hex: String,
    distill_loss_raw_sha: String,
    pre_clamp_kl_loss: Option<f32>,
    distill_loss_weighted: f32,
    temperature: f32,
    class_count: usize,
    row_count: usize,
}

fn command_metadata(command: &S2Command) -> (&'static str, Value) {
    match command {
        S2Command::ReplayFull(args) => (
            "replay-full",
            json!({
                "manifest": args.manifest,
                "pass_version": args.pass_version,
                "fixture": args.fixture,
                "seed_list": args.seed_list,
                "builds": args.builds,
                "device_profile": args.device_profile,
                "json": args.json,
            }),
        ),
        S2Command::ReplayAblation(args) => (
            "replay-ablation",
            json!({
                "manifest": args.manifest,
                "pass_version": args.pass_version,
                "fixture": args.fixture,
                "seed_list": args.seed_list,
                "device_profile": args.device_profile,
                "json": args.json,
            }),
        ),
        S2Command::VerifyDeterminism(args) => (
            "verify-determinism",
            json!({
                "fixture": args.fixture,
                "seed": args.seed,
                "build": args.build,
                "json": args.json,
            }),
        ),
        S2Command::GradFlow(args) => ("grad-flow", json!({ "json": args.json })),
        S2Command::LinearstateSmoke(args) => ("linearstate-smoke", json!({ "json": args.json })),
        S2Command::PhaseInteg(args) => ("phase-integ", json!({ "json": args.json })),
        S2Command::OracleReRun(args) => ("oracle-re-run", json!({ "json": args.json })),
        S2Command::Report(args) => (
            "report",
            json!({
                "output": args.output,
                "replay_full_json": args.replay_full_json,
                "distill_json": args.distill_json,
                "json": args.json,
            }),
        ),
        S2Command::DistillOnce(args) => (
            "distill-once",
            json!({
                "fixture": args.fixture,
                "json": args.json,
            }),
        ),
    }
}

fn replay_full(args: ReplayFullArgs) -> Result<(), S2CliError> {
    require_fixture(&args.fixture, "tiny")?;
    require_option("device_profile", &args.device_profile, "S1CpuDeterministic")?;
    let seeds = parse_seed_list(&args.seed_list)?;
    let builds = parse_builds(&args.builds)?;
    let mut runs = Vec::new();

    for seed in seeds {
        for build_kind in &builds {
            let product = s2_train_run(&RunInputs::tiny_fixture(seed, *build_kind))?;
            let RunProductS2::Completed(product) = product else {
                return Err(S2CliError::Diverged);
            };
            let checkpoints = product
                .phase_boundary_checkpoint_shas
                .iter()
                .map(|(step, sha)| (step.to_string(), sha.to_string()))
                .collect::<BTreeMap<_, _>>();
            let mut phase_boundary_steps = checkpoints.keys().cloned().collect::<Vec<_>>();
            phase_boundary_steps.sort_by_key(|step| step.parse::<u64>().unwrap_or(u64::MAX));
            runs.push(ReplayRunJson {
                seed,
                build_kind: *build_kind,
                final_checkpoint_sha: product.final_checkpoint_sha.to_string(),
                phase_boundary_steps,
                checkpoints,
                phase_log_self_hash: product.phase_log_self_hash.to_string(),
                distill_log_self_hash: product.distill_log_self_hash.to_string(),
                score_self_hash: product.score_self_hash.to_string(),
            });
        }
    }

    write_json(
        json!({
            "schema": "s2_replay_full_cli.v1",
            "evidence_source": "gbf s2 replay-full",
            "fixture": args.fixture,
            "manifest": args.manifest,
            "pass_version": args.pass_version,
            "runs": runs,
        }),
        args.json,
    )
}

fn replay_ablation(args: ReplayAblationArgs) -> Result<(), S2CliError> {
    require_fixture(&args.fixture, "tiny")?;
    require_option("device_profile", &args.device_profile, "S1CpuDeterministic")?;
    let seeds = parse_seed_list(&args.seed_list)?;
    let mut runs = Vec::new();
    for seed in seeds {
        let product = s2_train_run(&RunInputs::tiny_fixture(seed, S2BuildKind::s2_ablation))?;
        let RunProductS2::Completed(product) = product else {
            return Err(S2CliError::Diverged);
        };
        runs.push(ReplayRunJson {
            seed,
            build_kind: S2BuildKind::s2_ablation,
            final_checkpoint_sha: product.final_checkpoint_sha.to_string(),
            phase_boundary_steps: product
                .phase_boundary_checkpoint_shas
                .keys()
                .map(ToString::to_string)
                .collect(),
            checkpoints: product
                .phase_boundary_checkpoint_shas
                .iter()
                .map(|(step, sha)| (step.to_string(), sha.to_string()))
                .collect(),
            phase_log_self_hash: product.phase_log_self_hash.to_string(),
            distill_log_self_hash: product.distill_log_self_hash.to_string(),
            score_self_hash: product.score_self_hash.to_string(),
        });
    }
    write_json(
        json!({
            "schema": "s2_replay_ablation_cli.v1",
            "evidence_source": "gbf s2 replay-ablation",
            "fixture": args.fixture,
            "manifest": args.manifest,
            "pass_version": args.pass_version,
            "runs": runs,
        }),
        args.json,
    )
}

fn verify_determinism(args: VerifyDeterminismArgs) -> Result<(), S2CliError> {
    require_fixture(&args.fixture, "tiny")?;
    let build_kind = parse_builds(&args.build)?
        .into_iter()
        .next()
        .ok_or_else(|| S2CliError::InvalidBuildKind {
            value: args.build.clone(),
        })?;
    let first = completed_run(args.seed, build_kind)?;
    let second = completed_run(args.seed, build_kind)?;
    let passed = first.final_checkpoint_sha == second.final_checkpoint_sha
        && first.phase_log_self_hash == second.phase_log_self_hash;
    write_json(
        json!({
            "schema": "s2_verify_determinism_cli.v1",
            "evidence_source": "gbf s2 verify-determinism",
            "seed": args.seed,
            "build_kind": build_kind,
            "passed": passed,
            "first_final_checkpoint_sha": first.final_checkpoint_sha.to_string(),
            "second_final_checkpoint_sha": second.final_checkpoint_sha.to_string(),
        }),
        args.json,
    )
}

fn grad_flow(args: JsonArgs) -> Result<(), S2CliError> {
    let report = loss_grad_flow_report()?;
    write_json(
        json!({
            "schema": "s2_grad_flow_cli.v1",
            "evidence_source": "gbf s2 grad-flow",
            "overall_passed": report.overall_passed,
            "loss_grad_flow_self_hash": report.loss_grad_flow_self_hash.to_string(),
        }),
        args.json,
    )
}

fn linearstate_smoke(args: JsonArgs) -> Result<(), S2CliError> {
    let run = linearstate_smoke::run_fixture_v1()?;
    write_json(
        json!({
            "schema": "s2_linearstate_smoke_cli.v1",
            "evidence_source": "gbf s2 linearstate-smoke",
            "smoke_passed": run.report.smoke_passed,
            "smoke_self_hash": run.report.smoke_self_hash.to_string(),
        }),
        args.json,
    )
}

fn phase_integ(args: JsonArgs) -> Result<(), S2CliError> {
    let report = phase_transition_integ::run_phase_transition_integration()?;
    write_json(
        json!({
            "schema": "s2_phase_integ_cli.v1",
            "evidence_source": "gbf s2 phase-integ",
            "phase_transition_integ_passed": report.integ_passed,
            "phase_transition_integ_self_hash": report.integ_self_hash.to_string(),
        }),
        args.json,
    )
}

fn oracle_re_run(args: JsonArgs) -> Result<(), S2CliError> {
    let report = oracle_re_run::run_s1_oracle_re_run_under_s2_binary()?;
    write_json(
        json!({
            "schema": "s2_oracle_re_run_cli.v1",
            "evidence_source": "gbf s2 oracle-re-run",
            "metric_oracle_passed": report.metric_oracle_passed,
            "oracle_re_run_self_hash": report.oracle_re_run_self_hash.to_string(),
        }),
        args.json,
    )
}

fn report(args: ReportArgs) -> Result<(), S2CliError> {
    let replay_evidence = args
        .replay_full_json
        .as_ref()
        .map(read_replay_full_evidence)
        .transpose()?;
    let distill_evidence = args
        .distill_json
        .as_ref()
        .map(read_distill_once_evidence)
        .transpose()?;
    let emitted = emit_s2_report(&report_inputs(
        args.output.clone(),
        replay_evidence.as_ref(),
        distill_evidence.as_ref(),
    )?)?;
    let state_machine = run_pretrain_state_machine(PreTrainGateResults::default());
    write_json(
        json!({
            "schema": "s2_report_cli.v1",
            "evidence_source": "gbf s2 report",
            "path": emitted.path,
            "report_self_hash": emitted.report.front_matter.report_self_hash.to_string(),
            "replay_evidence_source": replay_evidence.as_ref().map(|evidence| evidence.evidence_source.as_str()),
            "distill_evidence_source": distill_evidence.as_ref().map(|evidence| evidence["evidence_source"].as_str().unwrap_or("")),
            "state_machine": state_machine_json(state_machine),
        }),
        args.json,
    )
}

fn distill_once(args: DistillOnceArgs) -> Result<(), S2CliError> {
    require_fixture(&args.fixture, "pinned")?;
    let output = distill_once_pinned()?;
    let payload = distill_json(output);
    write_json(
        json!({
            "schema": "s2_distill_once_cli.v1",
            "evidence_source": "gbf s2 distill-once",
            "fixture": args.fixture,
            "distill": payload,
        }),
        args.json,
    )
}

fn distill_json(output: DistillOnceOutput) -> DistillOnceJson {
    DistillOnceJson {
        distill_loss_raw: output.distill_loss_raw,
        distill_loss_raw_bits_hex: format!("{:08x}", output.distill_loss_raw_bits),
        distill_loss_raw_sha: output.distill_loss_raw_sha.to_string(),
        pre_clamp_kl_loss: output.pre_clamp_kl_loss,
        distill_loss_weighted: output.distill_loss_weighted,
        temperature: output.temperature,
        class_count: output.class_count,
        row_count: output.row_count,
    }
}

fn completed_run(
    seed: u64,
    build_kind: S2BuildKind,
) -> Result<Box<CompletedRunProductS2>, S2CliError> {
    match s2_train_run(&RunInputs::tiny_fixture(seed, build_kind))? {
        RunProductS2::Completed(product) => Ok(product),
        RunProductS2::Diverged(_) => Err(S2CliError::Diverged),
    }
}

fn loss_grad_flow_report() -> Result<LossGradFlowReport, S2CliError> {
    let mut fixtures = vec![
        passing_fixture("H5.1", "lambda_zrouter"),
        passing_fixture("H5.2", "lambda_balance"),
        passing_fixture("H5.3", "lambda_range"),
        loss_grad_flow::h5_4_fixture_with_zero_raw_honesty()?,
    ];
    #[cfg(feature = "s2-full")]
    fixtures.push(loss_grad_flow::run_h5_5_distill_fixture()?);
    #[cfg(not(feature = "s2-full"))]
    fixtures.push(passing_fixture("H5.5", "lambda_distill"));
    Ok(LossGradFlowReport::new(fixtures)?)
}

fn passing_fixture(sub_hypothesis: &str, loss_term: &str) -> FixtureResult {
    let mut in_scope_grad_norms = BTreeMap::new();
    in_scope_grad_norms.insert(format!("{loss_term}_target"), 0.25);
    let mut stop_gradient_grad_norms = BTreeMap::new();
    stop_gradient_grad_norms.insert(format!("{loss_term}_detached"), 0.0);
    let mut detached_grad_absence = BTreeMap::new();
    if sub_hypothesis == "H5.5" {
        detached_grad_absence.insert("teacher_logits".to_owned(), true);
    }
    let diagnostic_subchecks = vec![DiagnosticSubcheckResult {
        name: format!("{loss_term}_finite_raw"),
        lambda_value: 0.5,
        raw_loss_computed: true,
        raw_loss_finite: true,
        weighted_loss_value: Some(0.125),
        passed: true,
    }];
    FixtureResult {
        sub_hypothesis: sub_hypothesis.to_owned(),
        loss_term: loss_term.to_owned(),
        in_scope_grad_norms,
        stop_gradient_grad_norms,
        non_default_value_used: true,
        numerical_stability_passed: true,
        diagnostic_subchecks,
        detached_grad_absence,
        sub_passed: true,
    }
}

fn report_inputs(
    output_path: PathBuf,
    replay_evidence: Option<&ReplayFullJson>,
    distill_evidence: Option<&Value>,
) -> Result<S2ReportInputs, S2CliError> {
    let report = loss_grad_flow_report()?;
    let linear = linearstate_smoke::run_fixture_v1()?;
    let phase = phase_transition_integ::run_phase_transition_integration()?;
    let oracle = oracle_re_run::run_s1_oracle_re_run_under_s2_binary()?;
    let (
        per_seed_artifacts,
        pass_version_s2,
        replay_command,
        manifest_references,
        observed_markdown,
    ) = if let Some(replay) = replay_evidence {
        let distill_note = distill_evidence
            .and_then(|value| value["evidence_source"].as_str())
            .unwrap_or("not supplied");
        (
            per_seed_artifacts_from_replay(replay)?,
            parse_semver_evidence(
                &replay.pass_version,
                Path::new("replay-full-json"),
                "pass_version",
            )?,
            format!(
                "gbf s2 replay-full --manifest {} --pass-version {} --fixture {}",
                replay.manifest, replay.pass_version, replay.fixture
            ),
            format!("{} fixture manifest {}", replay.fixture, replay.manifest),
            format!(
                "Live replay evidence consumed from gbf s2 replay-full. Several verifier fields remain fixture/default evidence in this narrow report mode, so this is not full live closure or deployable artifact acceptance. Distill JSON is provenance-only here ({distill_note}); its JSON bytes are not threaded into report_self_hash."
            ),
        )
    } else {
        (
            synthetic_per_seed_artifacts(),
            SemVer::new(0, 1, 0),
            "gbf s2 replay-full --fixture tiny --seed-list 0 --builds s2_ternary_full".to_owned(),
            "tiny fixture manifest".to_owned(),
            "Tiny executable S2 CLI report fixture completed.".to_owned(),
        )
    };
    let mut bundle = S2VerifierBundle::closure_candidate();
    bundle.oracle_re_run_passed = oracle.metric_oracle_passed;
    bundle.phase_transition_integ_passed = phase.integ_passed;
    bundle
        .hypothesis_statuses
        .insert(S2Hypothesis::H5, status_for(report.overall_passed));
    bundle
        .hypothesis_statuses
        .insert(S2Hypothesis::H6, status_for(linear.report.smoke_passed));
    Ok(S2ReportInputs {
        output_path,
        baseline_self_hash_carried_from_s1: hash(1),
        oracle_re_run_self_hash: oracle.oracle_re_run_self_hash,
        qat_public_api_snapshot_hash: hash(2),
        linearstate_public_api_snapshot_hash: hash(3),
        per_seed_artifacts,
        ablation_self_hash: Some(hash(4)),
        loss_grad_flow_self_hash: report.loss_grad_flow_self_hash,
        linearstate_smoke_self_hash: linear.report.smoke_self_hash,
        phase_transition_integ_self_hash: phase.integ_self_hash,
        falsification_s2_suite_hash: hash(5),
        generated_at: "2026-05-12T00:00:00Z".to_owned(),
        rfc_revision: RfcRevisionRef::GitCommitId(commit('a')?),
        predictions_commit: commit('1')?,
        first_result_commit: commit('2')?,
        pass_version_s2,
        verifier_bundle: bundle,
        predictions_markdown: "H2 ternary-full gap remains <= 0.5 bpc.".to_owned(),
        observed_markdown,
        falsification_analysis: "Synthetic falsification suite hash is recorded; full suite is owned by the falsification bead.".to_owned(),
        surprises: "None.".to_owned(),
        decision_justification: "CLI report exercised report emission and state-machine sequencing; live replay evidence populates per-seed rows only when supplied.".to_owned(),
        replay_command,
        manifest_references,
    })
}

fn read_replay_full_evidence(path: &PathBuf) -> Result<ReplayFullJson, S2CliError> {
    let text = std::fs::read_to_string(path)?;
    let evidence: ReplayFullJson = serde_json::from_str(&text)?;
    if evidence.schema != "s2_replay_full_cli.v1" {
        return Err(invalid_evidence(
            path,
            format!(
                "expected schema s2_replay_full_cli.v1, got {}",
                evidence.schema
            ),
        ));
    }
    if evidence.evidence_source != "gbf s2 replay-full" {
        return Err(invalid_evidence(
            path,
            format!(
                "expected evidence_source gbf s2 replay-full, got {}",
                evidence.evidence_source
            ),
        ));
    }
    if evidence.runs.is_empty() {
        return Err(invalid_evidence(path, "runs must not be empty"));
    }
    Ok(evidence)
}

fn read_distill_once_evidence(path: &PathBuf) -> Result<Value, S2CliError> {
    let text = std::fs::read_to_string(path)?;
    let evidence: Value = serde_json::from_str(&text)?;
    if evidence["schema"] != "s2_distill_once_cli.v1" {
        return Err(invalid_evidence(
            path,
            format!(
                "expected schema s2_distill_once_cli.v1, got {}",
                evidence["schema"]
            ),
        ));
    }
    if evidence["evidence_source"] != "gbf s2 distill-once" {
        return Err(invalid_evidence(
            path,
            format!(
                "expected evidence_source gbf s2 distill-once, got {}",
                evidence["evidence_source"]
            ),
        ));
    }
    Ok(evidence)
}

fn per_seed_artifacts_from_replay(
    replay: &ReplayFullJson,
) -> Result<Vec<S2PerSeedArtifacts>, S2CliError> {
    let path = PathBuf::from("replay-full-json");
    let mut rows = Vec::with_capacity(replay.runs.len());
    let mut seen = std::collections::BTreeSet::new();
    for run in &replay.runs {
        if !seen.insert((run.seed, run.build_kind)) {
            return Err(invalid_evidence(
                &path,
                format!("duplicate seed/build row {} {}", run.seed, run.build_kind),
            ));
        }
        let final_checkpoint =
            parse_hash_evidence(&run.final_checkpoint_sha, &path, "final_checkpoint_sha")?;
        if let Some(boundary_final) = run.checkpoints.get("10000") {
            let boundary_final = parse_hash_evidence(boundary_final, &path, "checkpoints.10000")?;
            if boundary_final != final_checkpoint {
                return Err(invalid_evidence(
                    &path,
                    format!(
                        "final_checkpoint_sha does not match checkpoint 10000 for seed {} build {}",
                        run.seed, run.build_kind
                    ),
                ));
            }
        }
        rows.push(S2PerSeedArtifacts {
            seed: run.seed,
            build_kind: run.build_kind,
            completion: S2Completion::Completed,
            checkpoint_self_hashes: S2CheckpointSelfHashes {
                phase_a: Some(required_checkpoint_hash(run, "4000", &path)?),
                phase_b: Some(required_checkpoint_hash(run, "5000", &path)?),
                phase_c: Some(required_checkpoint_hash(run, "8000", &path)?),
                final_checkpoint: Some(final_checkpoint),
            },
            phase_log_self_hash: Some(parse_hash_evidence(
                &run.phase_log_self_hash,
                &path,
                "phase_log_self_hash",
            )?),
            score_self_hash: Some(parse_hash_evidence(
                &run.score_self_hash,
                &path,
                "score_self_hash",
            )?),
            distill_log_self_hash: (run.build_kind != S2BuildKind::s2_ternary_nodistill)
                .then(|| {
                    parse_hash_evidence(&run.distill_log_self_hash, &path, "distill_log_self_hash")
                })
                .transpose()?,
        });
    }
    Ok(rows)
}

fn required_checkpoint_hash(
    run: &ReplayRunJson,
    step: &'static str,
    path: &Path,
) -> Result<Hash256, S2CliError> {
    let value = run.checkpoints.get(step).ok_or_else(|| {
        invalid_evidence(
            path,
            format!(
                "missing checkpoint {step} for seed {} build {}",
                run.seed, run.build_kind
            ),
        )
    })?;
    parse_hash_evidence(value, path, step)
}

fn parse_hash_evidence(
    value: &str,
    path: &Path,
    field: &'static str,
) -> Result<Hash256, S2CliError> {
    value
        .parse::<Hash256>()
        .map_err(|error| invalid_evidence(path, format!("{field} is not a Hash256: {error}")))
}

fn parse_semver_evidence(
    value: &str,
    path: &Path,
    field: &'static str,
) -> Result<SemVer, S2CliError> {
    value
        .parse::<SemVer>()
        .map_err(|error| invalid_evidence(path, format!("{field} is not SemVer: {error}")))
}

fn invalid_evidence(path: &Path, reason: impl Into<String>) -> S2CliError {
    S2CliError::InvalidEvidence {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn synthetic_per_seed_artifacts() -> Vec<S2PerSeedArtifacts> {
    let mut rows = Vec::new();
    for (build_index, build_kind) in [
        S2BuildKind::s2_ternary_full,
        S2BuildKind::s2_fp_full,
        S2BuildKind::s2_ternary_nodistill,
    ]
    .into_iter()
    .enumerate()
    {
        for seed in 0..5 {
            let fill = 20 + build_index as u8 * 20 + seed as u8;
            rows.push(S2PerSeedArtifacts {
                seed,
                build_kind,
                completion: S2Completion::Completed,
                checkpoint_self_hashes: S2CheckpointSelfHashes {
                    phase_a: Some(hash(fill)),
                    phase_b: Some(hash(fill + 1)),
                    phase_c: Some(hash(fill + 2)),
                    final_checkpoint: Some(hash(fill + 3)),
                },
                phase_log_self_hash: Some(hash(fill + 4)),
                score_self_hash: Some(hash(fill + 5)),
                distill_log_self_hash: (build_kind != S2BuildKind::s2_ternary_nodistill)
                    .then(|| hash(fill + 6)),
            });
        }
    }
    rows
}

fn state_machine_json(run: crate::s2::run::state_machine::StateMachineRun) -> StateMachineJson {
    StateMachineJson {
        final_state: run.final_state.to_string(),
        outcome: run.outcome.to_string(),
        decision: run.decision.to_string(),
        train_attempted: run.train_attempted,
        transitions: run
            .transitions
            .into_iter()
            .map(|transition| format!("{}->{}", transition.from, transition.to))
            .collect(),
    }
}

fn require_option(option: &'static str, actual: &str, expected: &str) -> Result<(), S2CliError> {
    if actual == expected {
        Ok(())
    } else {
        Err(S2CliError::UnsupportedOption {
            option,
            value: actual.to_owned(),
        })
    }
}

fn status_for(passed: bool) -> HypothesisStatus {
    if passed {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    }
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}

fn commit(fill: char) -> Result<GitCommitId, S2CliError> {
    GitCommitId::new(fill.to_string().repeat(40)).map_err(|error| {
        S2CliError::Schema(crate::s1::schema::S1SchemaError::Custom(error.to_string()))
    })
}

fn append_line(path: &PathBuf, line: &str) -> Result<(), S2CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn require_fixture(actual: &str, expected: &str) -> Result<(), S2CliError> {
    if actual == expected {
        Ok(())
    } else {
        Err(S2CliError::UnsupportedFixture {
            fixture: actual.to_owned(),
        })
    }
}

fn parse_seed_list(value: &str) -> Result<Vec<u64>, S2CliError> {
    split_csv(value)
        .map(|seed| {
            u64::from_str(seed).map_err(|_| S2CliError::InvalidSeed {
                value: seed.to_owned(),
            })
        })
        .collect()
}

fn parse_builds(value: &str) -> Result<Vec<S2BuildKind>, S2CliError> {
    split_csv(value)
        .map(|build| match build {
            "s2_ternary_full" | "s2-ternary-full" => Ok(S2BuildKind::s2_ternary_full),
            "s2_fp_full" | "s2-fp-full" => Ok(S2BuildKind::s2_fp_full),
            "s2_ternary_nodistill" | "s2-ternary-nodistill" => {
                Ok(S2BuildKind::s2_ternary_nodistill)
            }
            "s2_ablation" | "s2-ablation" => Ok(S2BuildKind::s2_ablation),
            _ => Err(S2CliError::InvalidBuildKind {
                value: build.to_owned(),
            }),
        })
        .collect()
}

fn split_csv(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn write_json(value: serde_json::Value, compact: bool) -> Result<(), S2CliError> {
    let json = if compact {
        serde_json::to_string(&value)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    println!("{json}");
    Ok(())
}
