//! Command-line integration for S1 workflows.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType, canonical_tensor_payload_hash,
};
use gbf_foundation::{Hash256, SemVer, sha256};
use gbf_policy::model_profile::ModelSizeProfile;
use safetensors::{Dtype, SafeTensors};
use serde::Serialize;
use serde_json::{Value, json};

use crate::s1::ablation::{AblationCheckpoint, AblationError, compare};
use crate::s1::baseline::{BaselineError, fit_baseline_report};
use crate::s1::build_metadata::{BuildMetadata, build_metadata};
use crate::s1::device_profile::{DeviceProfileEnforceError, S1CpuDeterministic, enforce};
use crate::s1::logging::{
    CliDiagnosticCompleteEvent, CliDiagnosticFailedEvent, CliDiagnosticStartEvent,
    LoggingEventError, S1LogEmitter,
};
use crate::s1::manifest::{
    CorpusManifestError, TinyStoriesManifest, load_train_bytes, load_val_bytes,
    read_tinystories_manifest,
};
use crate::s1::neg_test::{NegativeTestError, run_negative_test};
#[cfg(feature = "falsify")]
use crate::s1::oracle::{MetricOracleResults, emit_oracle_report};
use crate::s1::oracle::{OracleEmitError, run_metric_oracles};
use crate::s1::report::{
    Hypothesis, HypothesisFinding, HypothesisStatus, ObservedSeed, OutcomeDispatchError,
    OutcomeDispatchInput, ReportError, ReportInput, dispatch_outcome, predictions_section_hash,
};
use crate::s1::run::{
    RunInputs, RunProduct, RunTestOptions, S1RunError, TrainBudgetProfile, TrainConfig,
    s1_train_run, s1_train_run_with_options,
};
use crate::s1::schema::{
    AblationReport, BaselineReport, CheckpointMetadata, DomainHash, GitCommitId,
    NegativeTestReport, OracleReport, PerSeedArtifacts, ReportFrontMatter, RfcRevisionRef, RunLog,
    S1BuildKind, S1Completion, S1Decision, ScoreReport,
};
#[cfg(feature = "falsify")]
use crate::s1::score::RESET_CONTEXT_CHUNK_SIZE;
use crate::s1::score::{ResetContextScorer, ScoreError, score};

/// Current S1 replay pass version accepted by `gbf s1 replay`.
pub const CURRENT_PASS_VERSION: SemVer = SemVer::new(0, 1, 0);

/// `gbf s1` command surface.
#[derive(Debug, Parser)]
#[command(
    name = "s1",
    about = "S1 First Pulse experiment workflows",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct S1Cli {
    /// S1 workflow to execute.
    #[command(subcommand)]
    pub command: S1Command,
}

/// S1 subcommands.
#[derive(Debug, Subcommand)]
pub enum S1Command {
    /// Run read-only S1 environment and substrate diagnostics.
    Doctor(DoctorArgs),
    /// Inspect an S1 JSON artifact and recompute its self-hash.
    Inspect(InspectArgs),
    /// Compare two S1 SafeTensors checkpoints at canonical tensor level.
    DiffCheckpoints(DiffCheckpointsArgs),
    /// Print the resolved S1 training/build configuration.
    PrintConfig(PrintConfigArgs),
    /// Replay S1 runs from a manifest and seed list.
    Replay(ReplayArgs),
    /// Fit the 3-gram baseline from manifest train/validation bytes.
    FitBaseline(ManifestSeedArgs),
    /// Run the D7 measurement-oracle command surface.
    Oracle(OracleArgs),
    /// Replay a seed twice and assert deterministic byte output.
    VerifyDeterminism(VerifyDeterminismArgs),
    /// Score validation bytes with an explicit fixture scorer.
    Score(ScoreArgs),
    /// Run the negative-test helper with an explicit fixture scorer.
    NegativeTest(ScoreArgs),
    /// Run the ablation comparator fixture surface.
    Ablation(AblationArgs),
    /// Report composition command surface.
    Report(ReportArgs),
}

/// Doctor command arguments.
#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {
    /// TinyStories manifest path whose train/validation bytes should be verified.
    #[arg(long, default_value = "fixtures/corpora/tinystories.toml")]
    pub manifest: PathBuf,
    /// Output root whose available disk space should be checked.
    #[arg(long, default_value = "experiments/S1")]
    pub out_dir: PathBuf,
    /// Emit machine-readable JSON instead of a human table.
    #[arg(long)]
    pub json: bool,
}

/// Inspect command arguments.
#[derive(Debug, Clone, Args)]
pub struct InspectArgs {
    /// S1 JSON artifact path.
    pub artifact: PathBuf,
}

/// Checkpoint diff command arguments.
#[derive(Debug, Clone, Args)]
pub struct DiffCheckpointsArgs {
    /// First SafeTensors checkpoint path.
    pub a: PathBuf,
    /// Second SafeTensors checkpoint path.
    pub b: PathBuf,
}

/// Print-config command arguments.
#[derive(Debug, Clone, Args)]
pub struct PrintConfigArgs {
    /// Emit machine-readable JSON. The default output is already JSON.
    #[arg(long)]
    pub json: bool,
}

/// Common manifest and seed arguments.
#[derive(Debug, Clone, Args)]
pub struct ManifestSeedArgs {
    /// TinyStories manifest path.
    #[arg(long)]
    pub manifest: PathBuf,
    /// S1 seed.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
}

/// Normative replay arguments.
#[derive(Debug, Clone, Args)]
pub struct ReplayArgs {
    /// TinyStories manifest path.
    #[arg(long)]
    pub manifest: PathBuf,
    /// Expected S1 pass version. Must match the CLI's current pass version.
    #[arg(long)]
    pub pass_version: SemVer,
    /// Comma-separated seed list. S1 accepts only seeds 0..=4.
    #[arg(long)]
    pub seed_list: String,
    /// Deterministic device profile name. Must be S1CpuDeterministic.
    #[arg(long)]
    pub device_profile: String,
    /// Output root for checkpoints and run logs.
    #[arg(long, default_value = "experiments/S1")]
    pub out_dir: PathBuf,
    /// Training budget profile. Production is the normative default.
    #[arg(long, value_enum, default_value_t = CliBudgetProfile::Production)]
    pub budget_profile: CliBudgetProfile,
    /// Explicitly opt into the non-canonical integration fixture budget.
    #[arg(long)]
    pub allow_noncanonical_integration_fixture: bool,
    /// Test-only divergence injection. Hidden and compiled only with `falsify`.
    #[cfg(feature = "falsify")]
    #[arg(long, hide = true)]
    pub inject_non_finite_loss_at_step: Option<u64>,
    /// Test-only gradient divergence injection. Hidden and compiled only with `falsify`.
    #[cfg(feature = "falsify")]
    #[arg(long, hide = true)]
    pub inject_non_finite_grad_norm_at_step: Option<u64>,
    /// Test-only zero-gradient replay substitute. Hidden and compiled only with `falsify`.
    #[cfg(feature = "falsify")]
    #[arg(long, hide = true)]
    pub zero_gradients: bool,
}

/// Determinism-check arguments.
#[derive(Debug, Clone, Args)]
pub struct VerifyDeterminismArgs {
    /// TinyStories manifest path.
    #[arg(long)]
    pub manifest: PathBuf,
    /// Expected S1 pass version. Must match the CLI's current pass version.
    #[arg(long)]
    pub pass_version: SemVer,
    /// Seed to replay twice. S1 accepts only seeds 0..=4.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Deterministic device profile name. Must be S1CpuDeterministic.
    #[arg(long)]
    pub device_profile: String,
    /// Training budget profile. Production is the normative default.
    #[arg(long, value_enum, default_value_t = CliBudgetProfile::Production)]
    pub budget_profile: CliBudgetProfile,
    /// Explicitly opt into the non-canonical integration fixture budget.
    #[arg(long)]
    pub allow_noncanonical_integration_fixture: bool,
}

/// Score-like helper arguments.
#[derive(Debug, Clone, Args)]
pub struct ScoreArgs {
    /// TinyStories manifest path.
    #[arg(long)]
    pub manifest: PathBuf,
    /// S1 seed.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Checkpoint hash to record as metadata only in fixture scorer mode.
    #[arg(
        long,
        default_value = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    )]
    pub checkpoint_sha: Hash256,
    /// Production SafeTensors checkpoint to load when not using fixture scorer mode.
    #[arg(long)]
    pub checkpoint: Option<PathBuf>,
    /// Production checkpoint metadata sidecar to validate before loading checkpoint bytes.
    #[arg(long)]
    pub checkpoint_metadata: Option<PathBuf>,
    /// Use the deterministic uniform-logits fixture scorer.
    #[arg(long)]
    pub fixture_uniform_scorer: bool,
    /// Test-only forced bpc substitute. Hidden and compiled only with `falsify`.
    #[cfg(feature = "falsify")]
    #[arg(long, hide = true)]
    pub fixture_forced_bpc: Option<f64>,
}

/// Oracle command arguments.
#[derive(Debug, Clone, Args)]
pub struct OracleArgs {
    /// TinyStories manifest path for the production oracle artifact producer.
    #[arg(long)]
    pub manifest: Option<PathBuf>,
    /// S1 seed recorded on oracle telemetry.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Smoke the command surface without invoking the full oracle suite.
    #[arg(long)]
    pub smoke: bool,
    /// Test-only O-metric-4 failure substitute. Hidden and compiled only with `falsify`.
    #[cfg(feature = "falsify")]
    #[arg(long, hide = true)]
    pub fixture_fail_o_metric_4: bool,
}

/// Ablation command arguments.
#[derive(Debug, Clone, Args)]
pub struct AblationArgs {
    /// TinyStories manifest path for production metadata validation.
    #[arg(long, default_value = "fixtures/corpora/tinystories.toml")]
    pub manifest: PathBuf,
    /// Run the in-memory self-compare fixture surface.
    #[arg(long)]
    pub fixture_self_compare: bool,
    /// Seed-0 Phase A production SafeTensors checkpoint.
    #[arg(long)]
    pub phase_a_checkpoint: Option<PathBuf>,
    /// Seed-0 Phase A production checkpoint metadata sidecar.
    #[arg(long)]
    pub phase_a_metadata: Option<PathBuf>,
    /// Seed-0 ablation production SafeTensors checkpoint.
    #[arg(long)]
    pub ablation_checkpoint: Option<PathBuf>,
    /// Seed-0 ablation production checkpoint metadata sidecar.
    #[arg(long)]
    pub ablation_metadata: Option<PathBuf>,
    /// Test-only tensor mismatch substitute. Hidden and compiled only with `falsify`.
    #[cfg(feature = "falsify")]
    #[arg(long, hide = true)]
    pub fixture_mismatch: bool,
}

/// Report command arguments.
#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    /// TinyStories manifest path for production report artifact validation.
    #[arg(long, default_value = "fixtures/corpora/tinystories.toml")]
    pub manifest: PathBuf,
    /// Smoke the report command surface without composing production reports.
    #[arg(long)]
    pub smoke: bool,
    /// Compose an IntegrationFixture report from CLI-produced fixture artifacts.
    #[arg(long, value_enum)]
    pub fixture_scenario: Option<CliE2eScenario>,
    /// Directory containing CLI-produced fixture artifacts.
    #[arg(long)]
    pub artifact_dir: Option<PathBuf>,
    /// Compose a production report from full S1 artifacts.
    #[arg(long)]
    pub production: bool,
    /// RFC revision commit or sha256: content hash recorded in production reports.
    #[arg(long)]
    pub rfc_revision: Option<String>,
    /// Commit introducing the pre-registered predictions section.
    #[arg(long)]
    pub predictions_commit: Option<String>,
    /// First commit introducing S1 result artifacts.
    #[arg(long)]
    pub first_result_commit: Option<String>,
    /// File containing the exact pre-registered predictions markdown section.
    #[arg(long)]
    pub predictions_section_file: Option<PathBuf>,
    /// RFC3339 timestamp for production report front matter.
    #[arg(long, default_value = "1970-01-01T00:00:00Z")]
    pub generated_at: String,
}

/// Fixture E2E scenario accepted by `gbf s1 report --fixture-scenario`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
pub enum CliE2eScenario {
    /// All H1..H5 confirm.
    PassClean,
    /// H3 refuted while H1/H2/H4/H5 confirm.
    PassWithWarning,
    /// H1 refuted by a non-finite loss substitute.
    FailSubstrateNan,
    /// H1 refuted by a zero-gradient substitute.
    FailSubstrateZeroGrad,
    /// H2 refuted by a ToyTiny substitute.
    FailCapacityToytiny,
    /// Suspicious-low-bpc sentinel fired.
    FailSuspiciousLowBpc,
    /// H4 refuted by an ablation mismatch substitute.
    FailPhaseTernaryLeak,
    /// H5 refuted by an O-metric-4 shuffle substitute.
    FailMetricModuloShuffle,
}

impl fmt::Display for CliE2eScenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::PassClean => "pass_clean",
            Self::PassWithWarning => "pass_with_warning",
            Self::FailSubstrateNan => "fail_substrate_nan",
            Self::FailSubstrateZeroGrad => "fail_substrate_zero_grad",
            Self::FailCapacityToytiny => "fail_capacity_toytiny",
            Self::FailSuspiciousLowBpc => "fail_suspicious_low_bpc",
            Self::FailPhaseTernaryLeak => "fail_phase_ternary_leak",
            Self::FailMetricModuloShuffle => "fail_metric_modulo_shuffle",
        };
        f.write_str(value)
    }
}

/// CLI-selectable training budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
pub enum CliBudgetProfile {
    /// RFC production budget.
    Production,
    /// Explicit test-only fixture budget.
    IntegrationFixture,
}

impl From<CliBudgetProfile> for TrainBudgetProfile {
    fn from(value: CliBudgetProfile) -> Self {
        match value {
            CliBudgetProfile::Production => Self::Production,
            CliBudgetProfile::IntegrationFixture => Self::IntegrationFixture,
        }
    }
}

/// Run a parsed `gbf s1` command.
pub fn run(cli: S1Cli) -> Result<(), S1CliError> {
    match cli.command {
        S1Command::Doctor(args) => doctor(args),
        S1Command::Inspect(args) => inspect(args),
        S1Command::DiffCheckpoints(args) => diff_checkpoints(args),
        S1Command::PrintConfig(args) => print_config(args),
        S1Command::Replay(args) => replay(args),
        S1Command::FitBaseline(args) => fit_baseline(args),
        S1Command::Oracle(args) => oracle(args),
        S1Command::VerifyDeterminism(args) => verify_determinism(args),
        S1Command::Score(args) => score_command(args),
        S1Command::NegativeTest(args) => negative_test(args),
        S1Command::Ablation(args) => ablation(args),
        S1Command::Report(args) => report(args),
    }
}

fn with_diagnostic_logging<T>(
    command: &'static str,
    run_command: impl FnOnce() -> Result<T, S1CliError>,
) -> Result<T, S1CliError> {
    let emitter = S1LogEmitter::new();
    let span = emitter.cli_diagnostic_span(command)?;
    let _guard = span.enter();
    emitter.cli_diagnostic_start(&CliDiagnosticStartEvent { command })?;

    let result = run_command();
    match &result {
        Ok(_) => emitter.cli_diagnostic_complete(&CliDiagnosticCompleteEvent { command })?,
        Err(error) => emitter.cli_diagnostic_failed(&CliDiagnosticFailedEvent {
            command,
            diagnostic: error.to_string(),
        })?,
    }
    result
}

fn doctor(args: DoctorArgs) -> Result<(), S1CliError> {
    with_diagnostic_logging("doctor", || doctor_impl(args))
}

fn doctor_impl(args: DoctorArgs) -> Result<(), S1CliError> {
    let mut checks = Vec::new();

    checks.push(check_result(
        "device_profile_enforce",
        "F-S1.04 enforce(S1CpuDeterministic) over current env",
        enforce(&S1CpuDeterministic::canonical())
            .map(|enforcement| enforcement.device_profile_hash().to_string()),
    ));

    match read_tinystories_manifest(&args.manifest) {
        Ok(manifest) => {
            checks.push(check_result(
                "manifest_train_sha256",
                "train split bytes match manifest sha256",
                load_train_bytes(&manifest)
                    .map(|bytes| format!("{} bytes {}", bytes.len(), sha256(&bytes))),
            ));
            checks.push(check_result(
                "manifest_val_sha256",
                "validation split bytes match manifest sha256",
                load_val_bytes(&manifest)
                    .map(|bytes| format!("{} bytes {}", bytes.len(), sha256(&bytes))),
            ));
        }
        Err(error) => checks.push(DoctorCheck::fail(
            "manifest_read",
            "TinyStories manifest parses",
            error.to_string(),
        )),
    }

    checks.push(check_result(
        "dependency_lockfile_sha",
        "runtime Cargo.lock sha matches compile-time lock hash",
        dependency_lockfile_check(),
    ));
    checks.push(DoctorCheck::pass(
        "rust_toolchain_hash",
        "compiled rust/toolchain identity hash is recorded; no runtime expected-vs-observed check is available in current build metadata",
        cli_rust_toolchain_hash().to_string(),
    ));
    checks.push(check_result(
        "burn_version_pin",
        "workspace Burn dependency is pinned with =",
        burn_version_pin_check(),
    ));
    checks.push(check_result(
        "gpu_absence",
        "S1CpuDeterministic forbids GPU selection and no CUDA env is active",
        gpu_absence_check(),
    ));
    checks.push(check_result(
        "disk_space",
        "output root has more than 1 GiB available",
        disk_space_check(&args.out_dir),
    ));

    let summary = DoctorSummary {
        command: "doctor",
        ok: checks
            .iter()
            .all(|check| check.status == DoctorStatus::Pass),
        checks,
    };
    if args.json {
        print_json(&summary)?;
    } else {
        print_doctor_table(&summary);
    }

    if summary.ok {
        Ok(())
    } else {
        Err(S1CliError::DiagnosticFailure {
            command: "doctor",
            failed: summary
                .checks
                .iter()
                .filter(|check| check.status == DoctorStatus::Fail)
                .count(),
        })
    }
}

fn inspect(args: InspectArgs) -> Result<(), S1CliError> {
    with_diagnostic_logging("inspect", || inspect_impl(args))
}

fn inspect_impl(args: InspectArgs) -> Result<(), S1CliError> {
    let bytes = fs::read(&args.artifact)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            S1CliError::InvalidArtifact("artifact is missing string field schema".to_owned())
        })?
        .to_owned();
    let inspection = inspect_json_artifact(&schema, value)?;
    print_inspection(&inspection)?;
    if inspection.self_hash_ok {
        Ok(())
    } else {
        Err(S1CliError::SelfHashMismatch {
            schema: inspection.schema,
            expected: inspection.computed_self_hash,
            observed: inspection.stored_self_hash,
        })
    }
}

fn diff_checkpoints(args: DiffCheckpointsArgs) -> Result<(), S1CliError> {
    with_diagnostic_logging("diff-checkpoints", || diff_checkpoints_impl(args))
}

fn diff_checkpoints_impl(args: DiffCheckpointsArgs) -> Result<(), S1CliError> {
    let a_tensors = read_canonical_tensors(&args.a)?;
    let b_tensors = read_canonical_tensors(&args.b)?;
    let diff = checkpoint_diff(a_tensors, b_tensors);
    print_json(&diff)?;
    if diff.equal {
        Ok(())
    } else {
        Err(S1CliError::DiagnosticFailure {
            command: "diff-checkpoints",
            failed: diff.tensors.iter().filter(|tensor| !tensor.equal).count(),
        })
    }
}

fn print_config(args: PrintConfigArgs) -> Result<(), S1CliError> {
    with_diagnostic_logging("print-config", || print_config_impl(args))
}

fn print_config_impl(_args: PrintConfigArgs) -> Result<(), S1CliError> {
    let train_config = TrainConfig::pinned();
    let model_config = ModelSizeProfile::toy0();
    let build_metadata = build_metadata();
    let device_profile = S1CpuDeterministic::canonical();
    let summary = PrintConfigSummary {
        command: "print-config",
        pass_version: CURRENT_PASS_VERSION.to_string(),
        train_config_hash: cli_train_config_hash(&train_config)?.to_string(),
        model_config_hash: cli_model_config_hash(&model_config)?.to_string(),
        build_config_hash: cli_build_config_hash()?.to_string(),
        dependency_lockfile_sha: compiled_dependency_lockfile_sha().to_string(),
        rust_toolchain_hash: cli_rust_toolchain_hash().to_string(),
        active_features: active_features(),
        build_metadata,
        train_config,
        model_config,
        device_profile,
    };
    print_json(&summary)
}

fn replay(args: ReplayArgs) -> Result<(), S1CliError> {
    validate_pass_version(args.pass_version)?;
    validate_device_profile(&args.device_profile)?;
    let seed_list = parse_seed_list(&args.seed_list).map_err(S1CliError::InvalidSeedList)?;
    validate_seed_list(&seed_list)?;

    let manifest = read_tinystories_manifest(&args.manifest)?;
    let train = load_train_bytes(&manifest)?;
    let val = load_val_bytes(&manifest)?;
    let budget_profile = validate_replay_budget(
        "replay",
        args.budget_profile,
        args.allow_noncanonical_integration_fixture,
    )?;
    let run_options = replay_run_options(&args);
    let mut products = Vec::new();

    for seed in seed_list {
        let product = s1_train_run_with_options(
            RunInputs {
                corpus_train: train.clone(),
                corpus_val: val.clone(),
                model_config: ModelSizeProfile::toy0(),
                train_config: budget_profile.train_config(),
                seed,
                budget_profile,
            },
            run_options,
        )?;
        write_run_product(&args.out_dir, &product)?;
        products.push(replay_summary(seed, &product));
    }

    print_json(&ReplaySummary {
        pass_version: CURRENT_PASS_VERSION.to_string(),
        budget_profile: args.budget_profile,
        products,
    })
}

fn replay_run_options(args: &ReplayArgs) -> RunTestOptions {
    #[cfg(feature = "falsify")]
    {
        RunTestOptions {
            inject_non_finite_loss_at_step: args.inject_non_finite_loss_at_step,
            inject_non_finite_grad_norm_at_step: args.inject_non_finite_grad_norm_at_step,
            zero_gradients: args.zero_gradients,
            ..RunTestOptions::default()
        }
    }
    #[cfg(not(feature = "falsify"))]
    {
        let _ = args;
        RunTestOptions::default()
    }
}

fn fit_baseline(args: ManifestSeedArgs) -> Result<(), S1CliError> {
    validate_seed(args.seed)?;
    let manifest = read_tinystories_manifest(&args.manifest)?;
    let train = load_train_bytes(&manifest)?;
    let val = load_val_bytes(&manifest)?;
    let product = fit_baseline_report(
        args.seed,
        manifest.train_sha256,
        manifest.val_sha256,
        &train,
        &val,
    )?;
    print_json(&product.report)
}

fn oracle(args: OracleArgs) -> Result<(), S1CliError> {
    validate_seed(args.seed)?;
    if args.smoke {
        return print_json(&OracleSmokeSummary {
            status: "smoke",
            owner: "bd-1ehz",
        });
    }
    #[cfg(feature = "falsify")]
    if args.fixture_fail_o_metric_4 {
        let report = emit_oracle_report(
            args.seed,
            MetricOracleResults {
                o_metric_0: true,
                o_metric_1: true,
                o_metric_2: true,
                o_metric_3: true,
                o_metric_4: false,
            },
        )?;
        return print_json(&report);
    }
    let Some(manifest_path) = args.manifest else {
        return Err(S1CliError::Deferred {
            command: "oracle",
            owner: "bd-1ehz",
            detail: "production oracle artifact emission requires --manifest; bd-1ehz owns running it against canonical TinyStories in the full S1 protocol",
        });
    };
    let manifest = read_tinystories_manifest(manifest_path)?;
    let val = load_val_bytes(&manifest)?;
    let expected = manifest
        .val_shuffle_deadeef_sha256
        .ok_or(S1CliError::MissingShufflePin)?;
    let report = run_metric_oracles(args.seed, &val, expected)?;
    print_json(&report)
}

fn verify_determinism(args: VerifyDeterminismArgs) -> Result<(), S1CliError> {
    validate_pass_version(args.pass_version)?;
    validate_device_profile(&args.device_profile)?;
    validate_seed(args.seed)?;

    let manifest = read_tinystories_manifest(&args.manifest)?;
    let train = load_train_bytes(&manifest)?;
    let val = load_val_bytes(&manifest)?;
    let budget_profile = validate_replay_budget(
        "verify-determinism",
        args.budget_profile,
        args.allow_noncanonical_integration_fixture,
    )?;
    let inputs = || RunInputs {
        corpus_train: train.clone(),
        corpus_val: val.clone(),
        model_config: ModelSizeProfile::toy0(),
        train_config: budget_profile.train_config(),
        seed: args.seed,
        budget_profile,
    };
    let first = s1_train_run(inputs())?;
    let second = s1_train_run(inputs())?;

    assert_deterministic_products(args.seed, &first, &second)?;

    print_json(&VerifyDeterminismSummary {
        seed: args.seed,
        deterministic: true,
        checkpoint_sha: product_checkpoint_sha(&first)?.to_string(),
        run_log_self_hash: product_run_log_self_hash(&first)?.to_string(),
        checkpoint_self_hash: product_checkpoint_metadata(&first)?
            .checkpoint_self_hash
            .to_string(),
    })
}

fn assert_deterministic_products(
    seed: u64,
    first: &RunProduct,
    second: &RunProduct,
) -> Result<(), S1CliError> {
    let first_bytes = product_checkpoint_bytes(first)?;
    let second_bytes = product_checkpoint_bytes(second)?;
    if first_bytes != second_bytes {
        return Err(S1CliError::DeterminismMismatch {
            seed,
            detail: DeterminismMismatchDetail::SafetensorsBytes(first_byte_mismatch(
                first_bytes,
                second_bytes,
            )),
        });
    }

    let first_run_log_self_hash = product_run_log_self_hash(first)?;
    let second_run_log_self_hash = product_run_log_self_hash(second)?;
    if first_run_log_self_hash != second_run_log_self_hash {
        return Err(S1CliError::DeterminismMismatch {
            seed,
            detail: DeterminismMismatchDetail::RunLogSelfHash {
                expected: first_run_log_self_hash,
                observed: second_run_log_self_hash,
            },
        });
    }

    let first_metadata = product_checkpoint_metadata(first)?;
    let second_metadata = product_checkpoint_metadata(second)?;
    if first_metadata != second_metadata {
        return Err(S1CliError::DeterminismMismatch {
            seed,
            detail: DeterminismMismatchDetail::CheckpointMetadata,
        });
    }

    Ok(())
}

fn score_command(args: ScoreArgs) -> Result<(), S1CliError> {
    validate_seed(args.seed)?;
    let manifest = read_tinystories_manifest(&args.manifest)?;
    let val = load_val_bytes(&manifest)?;
    #[cfg(feature = "falsify")]
    if let Some(bpc) = args.fixture_forced_bpc {
        validate_fixture_checkpoint_sha("score", args.checkpoint_sha)?;
        let report = forced_fixture_score_report(&args, manifest.val_sha256, &val, bpc)?;
        return print_json(&report);
    }
    if args.fixture_uniform_scorer {
        validate_fixture_checkpoint_sha("score", args.checkpoint_sha)?;
        let scorer = fixture_scorer("score", true)?;
        let report = score(
            &scorer,
            args.seed,
            args.checkpoint_sha,
            manifest.val_sha256,
            &val,
        )?;
        return print_json(&report);
    }

    let checkpoint = required_path(
        args.checkpoint.as_deref(),
        "gbf s1 score production mode requires --checkpoint",
    )?;
    let metadata = required_path(
        args.checkpoint_metadata.as_deref(),
        "gbf s1 score production mode requires --checkpoint-metadata",
    )?;
    let (checkpoint_sha, tensors) = read_checkpoint_tensors_and_sha(checkpoint)?;
    let metadata = load_validated_production_metadata(
        metadata,
        &manifest,
        args.seed,
        S1BuildKind::PhaseA,
        true,
        Some(checkpoint_sha),
    )?;
    let scorer = ProductionCheckpointScorer::from_tensors(&tensors)?;
    let report = score(
        &scorer,
        metadata.seed,
        checkpoint_sha,
        manifest.val_sha256,
        &val,
    )?;
    print_json(&report)
}

#[cfg(feature = "falsify")]
fn forced_fixture_score_report(
    args: &ScoreArgs,
    corpus_val_sha: Hash256,
    val: &[u8],
    bpc: f64,
) -> Result<ScoreReport, S1CliError> {
    fixture_scorer("score", args.fixture_uniform_scorer)?;
    if val.is_empty() {
        return Err(ScoreError::EmptyValidation.into());
    }
    if !bpc.is_finite() || bpc < 0.0 {
        return Err(S1CliError::InvalidArtifact(
            "fixture forced bpc must be finite and non-negative".to_owned(),
        ));
    }

    Ok(ScoreReport {
        schema: "s1_score.v1".to_owned(),
        seed: args.seed,
        checkpoint_sha: args.checkpoint_sha,
        corpus_val_sha,
        chunk_size: RESET_CONTEXT_CHUNK_SIZE as u64,
        token_count: val.len() as u64,
        log2_sum: bpc * val.len() as f64,
        bpc,
        score_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?)
}

fn negative_test(args: ScoreArgs) -> Result<(), S1CliError> {
    validate_seed(args.seed)?;
    let manifest = read_tinystories_manifest(&args.manifest)?;
    let val = load_val_bytes(&manifest)?;
    let expected = manifest
        .val_shuffle_deadeef_sha256
        .ok_or(S1CliError::MissingShufflePin)?;
    if args.fixture_uniform_scorer {
        validate_fixture_checkpoint_sha("negative-test", args.checkpoint_sha)?;
        let scorer = fixture_scorer("negative-test", true)?;
        let report = run_negative_test(
            &scorer,
            args.seed,
            args.checkpoint_sha,
            manifest.val_sha256,
            expected,
            &val,
        )?;
        return print_json(&report);
    }
    if args.seed != 0 {
        return Err(S1CliError::InvalidArtifact(
            "gbf s1 negative-test production mode is defined only for seed 0".to_owned(),
        ));
    }
    let checkpoint = required_path(
        args.checkpoint.as_deref(),
        "gbf s1 negative-test production mode requires --checkpoint",
    )?;
    let metadata = required_path(
        args.checkpoint_metadata.as_deref(),
        "gbf s1 negative-test production mode requires --checkpoint-metadata",
    )?;
    let (checkpoint_sha, tensors) = read_checkpoint_tensors_and_sha(checkpoint)?;
    let metadata = load_validated_production_metadata(
        metadata,
        &manifest,
        args.seed,
        S1BuildKind::PhaseA,
        true,
        Some(checkpoint_sha),
    )?;
    let scorer = ProductionCheckpointScorer::from_tensors(&tensors)?;
    let report = run_negative_test(
        &scorer,
        metadata.seed,
        checkpoint_sha,
        manifest.val_sha256,
        expected,
        &val,
    )?;
    print_json(&report)
}

fn ablation(args: AblationArgs) -> Result<(), S1CliError> {
    if args.fixture_self_compare {
        let phase_a = checkpoint_metadata(S1BuildKind::PhaseA)?;
        let ablation = checkpoint_metadata(S1BuildKind::Ablation)?;
        let tensors = vec![fixture_tensor()?];
        let ablation_tensors;
        #[cfg(feature = "falsify")]
        {
            ablation_tensors = if args.fixture_mismatch {
                vec![fixture_mismatch_tensor()?]
            } else {
                tensors.clone()
            };
        }
        #[cfg(not(feature = "falsify"))]
        {
            let _ = args;
            ablation_tensors = tensors.clone();
        }
        let report = compare(
            AblationCheckpoint {
                metadata: &phase_a,
                checkpoint_sha: Hash256::ZERO,
                tensors: &tensors,
            },
            AblationCheckpoint {
                metadata: &ablation,
                checkpoint_sha: Hash256::ZERO,
                tensors: &ablation_tensors,
            },
        )?;
        return print_json(&report);
    }

    let phase_a_checkpoint = required_path(
        args.phase_a_checkpoint.as_deref(),
        "gbf s1 ablation production mode requires --phase-a-checkpoint",
    )?;
    let phase_a_metadata = required_path(
        args.phase_a_metadata.as_deref(),
        "gbf s1 ablation production mode requires --phase-a-metadata",
    )?;
    let ablation_checkpoint = required_path(
        args.ablation_checkpoint.as_deref(),
        "gbf s1 ablation production mode requires --ablation-checkpoint",
    )?;
    let ablation_metadata = required_path(
        args.ablation_metadata.as_deref(),
        "gbf s1 ablation production mode requires --ablation-metadata",
    )?;
    let manifest = read_tinystories_manifest(&args.manifest)?;
    let (phase_a_checkpoint_sha, tensors) = read_checkpoint_tensors_and_sha(phase_a_checkpoint)?;
    let (ablation_checkpoint_sha, ablation_tensors) =
        read_checkpoint_tensors_and_sha(ablation_checkpoint)?;
    let phase_a = load_validated_production_metadata(
        phase_a_metadata,
        &manifest,
        0,
        S1BuildKind::PhaseA,
        true,
        Some(phase_a_checkpoint_sha),
    )?;
    let ablation = load_validated_production_metadata(
        ablation_metadata,
        &manifest,
        0,
        S1BuildKind::Ablation,
        true,
        Some(ablation_checkpoint_sha),
    )?;
    let report = compare(
        AblationCheckpoint {
            metadata: &phase_a,
            checkpoint_sha: phase_a_checkpoint_sha,
            tensors: &tensors,
        },
        AblationCheckpoint {
            metadata: &ablation,
            checkpoint_sha: ablation_checkpoint_sha,
            tensors: &ablation_tensors,
        },
    )?;
    print_json(&report)
}

fn report(args: ReportArgs) -> Result<(), S1CliError> {
    if args.smoke {
        return print_json(&ReportSmokeSummary {
            status: "smoke",
            owner: "bd-16mx",
        });
    }
    if let Some(scenario) = args.fixture_scenario {
        let Some(artifact_dir) = args.artifact_dir else {
            return Err(S1CliError::InvalidArtifact(
                "--fixture-scenario requires --artifact-dir".to_owned(),
            ));
        };
        return compose_fixture_report(scenario, &artifact_dir);
    }
    if args.production {
        return compose_production_report(&args);
    }
    Err(S1CliError::InvalidArtifact(
        "gbf s1 report requires --production with production report inputs or --fixture-scenario for IntegrationFixture reports".to_owned(),
    ))
}

fn compose_fixture_report(scenario: CliE2eScenario, artifact_dir: &Path) -> Result<(), S1CliError> {
    let baseline: BaselineReport = read_json_artifact(artifact_dir.join("s1_baseline.v1.json"))?;
    let oracle: OracleReport = read_json_artifact(artifact_dir.join("s1_oracle.v1.json"))?;
    let negative: NegativeTestReport =
        read_json_artifact(artifact_dir.join("seed-0/s1_negative_test.v1.json"))?;
    let ablation: AblationReport =
        read_json_artifact(artifact_dir.join("seed-0/s1_ablation.v1.json"))?;

    let mut metadata = Vec::new();
    let mut run_logs = Vec::new();
    let mut scores = Vec::new();
    for seed in 0..5 {
        metadata.push(read_optional_json_artifact::<CheckpointMetadata>(
            artifact_dir.join(format!("checkpoints/seed-{seed}/metadata.json")),
        )?);
        run_logs.push(read_optional_json_artifact::<RunLog>(
            artifact_dir.join(format!("runs/seed-{seed}/run_log.json")),
        )?);
        scores.push(read_optional_json_artifact::<ScoreReport>(
            artifact_dir.join(format!("seed-{seed}/s1_score.v1.json")),
        )?);
    }

    let dispatch_input = fixture_dispatch_input_from_artifacts(
        scenario, &baseline, &oracle, &negative, &ablation, &metadata, &run_logs, &scores,
    )?;
    let dispatch = dispatch_outcome(&dispatch_input)?;
    let report_input = fixture_report_input(FixtureReportArtifacts {
        scenario,
        baseline: &baseline,
        oracle: &oracle,
        negative: &negative,
        ablation: &ablation,
        metadata: &metadata,
        run_logs: &run_logs,
        scores: &scores,
        dispatch_input,
        decision: dispatch.decision.clone(),
    })?;
    let report = crate::s1::report::emit_report(&report_input)?;
    let report_path = artifact_dir.join("S1-report.md");
    fs::write(&report_path, report.to_markdown()?)?;
    print_json(&ReportFixtureSummary {
        scenario: scenario.to_string(),
        outcome: dispatch.outcome.to_string(),
        decision: dispatch.decision.to_string(),
        report_self_hash: report.front_matter.report_self_hash.to_string(),
        output_path: report_path.display().to_string(),
    })
}

fn compose_production_report(args: &ReportArgs) -> Result<(), S1CliError> {
    let artifact_dir = required_path(
        args.artifact_dir.as_deref(),
        "--production requires --artifact-dir",
    )?;
    let rfc_revision = parse_rfc_revision(required_string(
        args.rfc_revision.as_deref(),
        "--production requires --rfc-revision",
    )?)?;
    let predictions_commit = parse_git_commit(required_string(
        args.predictions_commit.as_deref(),
        "--production requires --predictions-commit",
    )?)?;
    let first_result_commit = parse_git_commit(required_string(
        args.first_result_commit.as_deref(),
        "--production requires --first-result-commit",
    )?)?;
    let predictions_file = required_path(
        args.predictions_section_file.as_deref(),
        "--production requires --predictions-section-file",
    )?;
    let predictions_markdown = fs::read_to_string(predictions_file)?.trim().to_owned();
    let manifest = read_tinystories_manifest(&args.manifest)?;

    let baseline: BaselineReport = read_json_artifact_candidate(
        production_candidates(
            artifact_dir,
            &["s1_baseline.v1.json", "baseline/s1_baseline.v1.json"],
        ),
        "s1_baseline.v1.json",
    )?;
    let oracle: OracleReport = read_json_artifact_candidate(
        production_candidates(
            artifact_dir,
            &["s1_oracle.v1.json", "oracle/s1_oracle.v1.json"],
        ),
        "s1_oracle.v1.json",
    )?;
    let negative: NegativeTestReport = read_json_artifact_candidate(
        production_candidates(
            artifact_dir,
            &[
                "seed-0/s1_negative_test.v1.json",
                "negative-test/seed-0/s1_negative_test.v1.json",
            ],
        ),
        "s1_negative_test.v1.json",
    )?;
    let ablation: AblationReport = read_json_artifact_candidate(
        production_candidates(
            artifact_dir,
            &[
                "seed-0/s1_ablation.v1.json",
                "ablation/seed-0/s1_ablation.v1.json",
            ],
        ),
        "s1_ablation.v1.json",
    )?;

    if baseline.corpus_train_sha != manifest.train_sha256
        || baseline.corpus_val_sha != manifest.val_sha256
        || negative.corpus_val_sha != manifest.val_sha256
    {
        return Err(S1CliError::InvalidArtifact(
            "production report artifact corpus hashes do not match canonical TinyStories manifest"
                .to_owned(),
        ));
    }

    let mut metadata = Vec::new();
    let mut run_logs = Vec::new();
    let mut scores = Vec::new();
    let mut checkpoint_shas = Vec::new();
    for seed in 0..5 {
        let checkpoint_path = read_json_artifact_candidate_path(
            production_candidates(
                artifact_dir,
                &[&format!("checkpoints/seed-{seed}/final.safetensors")],
            ),
            "phase-a checkpoint",
        )?;
        let checkpoint_sha = sha256(fs::read(&checkpoint_path)?);
        let metadata_path = read_json_artifact_candidate_path(
            production_candidates(
                artifact_dir,
                &[&format!("checkpoints/seed-{seed}/metadata.json")],
            ),
            "checkpoint metadata",
        )?;
        let metadata_row = load_validated_production_metadata(
            &metadata_path,
            &manifest,
            seed,
            S1BuildKind::PhaseA,
            true,
            Some(checkpoint_sha),
        )?;
        let run_log: RunLog = read_json_artifact_candidate(
            production_candidates(artifact_dir, &[&format!("runs/seed-{seed}/run_log.json")]),
            "s1_run_log.v1.json",
        )?;
        if run_log.seed != seed {
            return Err(S1CliError::InvalidArtifact(format!(
                "production report run log seed mismatch: expected {seed}, observed {}",
                run_log.seed
            )));
        }
        let score: ScoreReport = read_json_artifact_candidate(
            production_candidates(
                artifact_dir,
                &[
                    &format!("seed-{seed}/s1_score.v1.json"),
                    &format!("scores/seed-{seed}/s1_score.v1.json"),
                ],
            ),
            "s1_score.v1.json",
        )?;
        if score.seed != seed || score.corpus_val_sha != manifest.val_sha256 {
            return Err(S1CliError::InvalidArtifact(format!(
                "production report score artifact mismatch for seed {seed}"
            )));
        }
        if score.checkpoint_sha != checkpoint_sha {
            return Err(S1CliError::InvalidArtifact(format!(
                "production report score checkpoint_sha mismatch for seed {seed}: expected {checkpoint_sha}, observed {}",
                score.checkpoint_sha
            )));
        }
        metadata.push(Some(metadata_row));
        run_logs.push(Some(run_log));
        scores.push(Some(score));
        checkpoint_shas.push(checkpoint_sha);
    }
    if negative.checkpoint_sha != checkpoint_shas[0] {
        return Err(S1CliError::InvalidArtifact(format!(
            "production report negative-test checkpoint_sha mismatch: expected {}, observed {}",
            checkpoint_shas[0], negative.checkpoint_sha
        )));
    }
    if ablation.phase_a_checkpoint_sha != checkpoint_shas[0] {
        return Err(S1CliError::InvalidArtifact(format!(
            "production report ablation phase_a_checkpoint_sha mismatch: expected {}, observed {}",
            checkpoint_shas[0], ablation.phase_a_checkpoint_sha
        )));
    }
    let ablation_checkpoint_path = read_json_artifact_candidate_path(
        production_candidates(
            artifact_dir,
            &[
                "ablation/checkpoints/seed-0/final.safetensors",
                "checkpoints/seed-0-ablation/final.safetensors",
            ],
        ),
        "ablation checkpoint",
    )?;
    let ablation_checkpoint_sha = sha256(fs::read(ablation_checkpoint_path)?);
    if ablation.ablation_checkpoint_sha != ablation_checkpoint_sha {
        return Err(S1CliError::InvalidArtifact(format!(
            "production report ablation checkpoint_sha mismatch: expected {ablation_checkpoint_sha}, observed {}",
            ablation.ablation_checkpoint_sha
        )));
    }

    let dispatch_input = production_dispatch_input_from_artifacts(
        &baseline, &oracle, &negative, &ablation, &metadata, &run_logs, &scores,
    )?;
    let dispatch = dispatch_outcome(&dispatch_input)?;
    let report_input = production_report_input(ProductionReportArtifacts {
        baseline: &baseline,
        oracle: &oracle,
        negative: &negative,
        ablation: &ablation,
        metadata: &metadata,
        run_logs: &run_logs,
        scores: &scores,
        dispatch_input,
        decision: dispatch.decision.clone(),
        generated_at: args.generated_at.clone(),
        rfc_revision,
        predictions_markdown,
        predictions_commit,
        first_result_commit,
    })?;
    let report = crate::s1::report::emit_report(&report_input)?;
    let report_path = artifact_dir.join("S1-report.md");
    fs::write(&report_path, report.to_markdown()?)?;
    print_json(&ReportProductionSummary {
        mode: "production",
        outcome: dispatch.outcome.to_string(),
        decision: dispatch.decision.to_string(),
        report_self_hash: report.front_matter.report_self_hash.to_string(),
        output_path: report_path.display().to_string(),
    })
}

fn validate_pass_version(observed: SemVer) -> Result<(), S1CliError> {
    if observed == CURRENT_PASS_VERSION {
        Ok(())
    } else {
        Err(S1CliError::PassVersionMismatch {
            expected: CURRENT_PASS_VERSION,
            observed,
        })
    }
}

fn validate_device_profile(observed: &str) -> Result<(), S1CliError> {
    if observed == "S1CpuDeterministic" {
        enforce(&S1CpuDeterministic::canonical())?;
        Ok(())
    } else {
        Err(S1CliError::InvalidDeviceProfile {
            observed: observed.to_owned(),
        })
    }
}

fn validate_replay_budget(
    command: &'static str,
    budget_profile: CliBudgetProfile,
    allow_noncanonical_integration_fixture: bool,
) -> Result<TrainBudgetProfile, S1CliError> {
    match budget_profile {
        CliBudgetProfile::Production => Ok(TrainBudgetProfile::Production),
        CliBudgetProfile::IntegrationFixture if allow_noncanonical_integration_fixture => {
            Ok(TrainBudgetProfile::IntegrationFixture)
        }
        CliBudgetProfile::IntegrationFixture => {
            Err(S1CliError::NonCanonicalFixtureBudget { command })
        }
    }
}

fn validate_seed_list(seeds: &[u64]) -> Result<(), S1CliError> {
    if seeds.is_empty() {
        return Err(S1CliError::EmptySeedList);
    }
    for &seed in seeds {
        validate_seed(seed)?;
    }
    Ok(())
}

fn validate_seed(seed: u64) -> Result<(), S1CliError> {
    if seed <= 4 {
        Ok(())
    } else {
        Err(S1CliError::InvalidSeed { observed: seed })
    }
}

fn required_path<'a>(path: Option<&'a Path>, detail: &str) -> Result<&'a Path, S1CliError> {
    path.ok_or_else(|| S1CliError::InvalidArtifact(detail.to_owned()))
}

fn load_validated_production_metadata(
    path: &Path,
    manifest: &TinyStoriesManifest,
    seed: u64,
    build_kind: S1BuildKind,
    require_completed: bool,
    checkpoint_sha: Option<Hash256>,
) -> Result<CheckpointMetadata, S1CliError> {
    let metadata: CheckpointMetadata = read_json_artifact(path.to_path_buf())?;
    validate_production_metadata(
        &metadata,
        manifest,
        seed,
        build_kind,
        require_completed,
        checkpoint_sha,
    )?;
    Ok(metadata)
}

fn validate_production_metadata(
    metadata: &CheckpointMetadata,
    manifest: &TinyStoriesManifest,
    seed: u64,
    build_kind: S1BuildKind,
    require_completed: bool,
    checkpoint_sha: Option<Hash256>,
) -> Result<(), S1CliError> {
    let computed = metadata.computed_self_hash()?;
    if computed != metadata.checkpoint_self_hash {
        return Err(S1CliError::SelfHashMismatch {
            schema: metadata.schema.clone(),
            expected: computed,
            observed: metadata.checkpoint_self_hash,
        });
    }
    if metadata.seed != seed {
        return Err(S1CliError::InvalidArtifact(format!(
            "production checkpoint metadata seed mismatch: expected {seed}, observed {}",
            metadata.seed
        )));
    }
    if metadata.build_kind != build_kind {
        return Err(S1CliError::InvalidArtifact(format!(
            "production checkpoint metadata build_kind mismatch: expected {build_kind:?}, observed {:?}",
            metadata.build_kind
        )));
    }
    if metadata.corpus_train_sha != manifest.train_sha256
        || metadata.corpus_val_sha != manifest.val_sha256
    {
        return Err(S1CliError::InvalidArtifact(
            "production checkpoint metadata corpus hashes do not match manifest".to_owned(),
        ));
    }
    if metadata.pass_version != CURRENT_PASS_VERSION {
        return Err(S1CliError::PassVersionMismatch {
            expected: CURRENT_PASS_VERSION,
            observed: metadata.pass_version,
        });
    }
    if metadata.budget_profile != "production" {
        return Err(S1CliError::InvalidArtifact(format!(
            "production CLI artifact consumers require budget_profile=production, observed {}",
            metadata.budget_profile
        )));
    }
    let pinned_steps = TrainConfig::pinned().optimizer_steps;
    if metadata.final_step != pinned_steps {
        return Err(S1CliError::InvalidArtifact(format!(
            "production checkpoint metadata final_step mismatch: expected {pinned_steps}, observed {}",
            metadata.final_step
        )));
    }
    if require_completed && metadata.completion != S1Completion::Completed {
        return Err(S1CliError::InvalidArtifact(format!(
            "production checkpoint metadata must be completed, observed {:?}",
            metadata.completion
        )));
    }
    if let Some(checkpoint_sha) = checkpoint_sha
        && metadata.checkpoint_safetensors_sha256 != checkpoint_sha
    {
        return Err(S1CliError::InvalidArtifact(format!(
            "production checkpoint metadata checkpoint_safetensors_sha256 mismatch: expected {checkpoint_sha}, observed {}",
            metadata.checkpoint_safetensors_sha256
        )));
    }
    Ok(())
}

fn parse_seed_list(value: &str) -> Result<Vec<u64>, String> {
    if value.trim().is_empty() {
        return Err("seed-list must not be empty".to_owned());
    }
    value
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid seed {part:?}"))
        })
        .collect()
}

fn write_run_product(out_dir: &Path, product: &RunProduct) -> Result<(), S1CliError> {
    match product {
        RunProduct::Completed(product) => {
            let checkpoint_dir = out_dir
                .join("checkpoints")
                .join(format!("seed-{}", product.seed));
            let run_dir = out_dir.join("runs").join(format!("seed-{}", product.seed));
            fs::create_dir_all(&checkpoint_dir)?;
            fs::create_dir_all(&run_dir)?;
            fs::write(
                checkpoint_dir.join("final.safetensors"),
                &product.final_checkpoint,
            )?;
            fs::write(
                checkpoint_dir.join("metadata.json"),
                checkpoint_metadata_json(&product.metadata)?,
            )?;
            fs::write(
                run_dir.join("run_log.json"),
                run_log_json(&product.run_log)?,
            )?;
            Ok(())
        }
        RunProduct::Diverged(product) => {
            let run_dir = out_dir.join("runs").join(format!("seed-{}", product.seed));
            fs::create_dir_all(&run_dir)?;
            fs::write(
                run_dir.join("run_log.json"),
                run_log_json(&product.run_log)?,
            )?;
            Err(S1CliError::RunDiverged { seed: product.seed })
        }
    }
}

fn checkpoint_metadata_json(metadata: &CheckpointMetadata) -> Result<Vec<u8>, S1CliError> {
    let mut normalized: CheckpointMetadata =
        serde_json::from_value(serde_json::to_value(metadata)?)?;
    normalized.checkpoint_self_hash = Hash256::ZERO;
    let normalized = normalized.with_computed_self_hash()?;
    Ok(crate::s1::schema::S1CanonicalJson::to_vec(&normalized)?)
}

fn run_log_json(run_log: &RunLog) -> Result<Vec<u8>, S1CliError> {
    let mut normalized: RunLog = serde_json::from_value(serde_json::to_value(run_log)?)?;
    for _ in 0..4 {
        normalized.run_log_self_hash = Hash256::ZERO;
        normalized = normalized.with_computed_self_hash()?;
        let bytes = crate::s1::schema::S1CanonicalJson::to_vec(&normalized)?;
        let reparsed: RunLog = serde_json::from_slice(&bytes)?;
        if reparsed.run_log_self_hash == reparsed.computed_self_hash()? {
            return Ok(bytes);
        }
        normalized = reparsed;
    }
    Ok(crate::s1::schema::S1CanonicalJson::to_vec(&normalized)?)
}

fn replay_summary(seed: u64, product: &RunProduct) -> ReplayProductSummary {
    match product {
        RunProduct::Completed(product) => ReplayProductSummary {
            seed,
            completion: "completed",
            checkpoint_sha: Some(product.final_checkpoint_sha.to_string()),
            run_log_self_hash: product.run_log.run_log_self_hash.to_string(),
        },
        RunProduct::Diverged(product) => ReplayProductSummary {
            seed,
            completion: "diverged",
            checkpoint_sha: None,
            run_log_self_hash: product.run_log.run_log_self_hash.to_string(),
        },
    }
}

fn product_checkpoint_bytes(product: &RunProduct) -> Result<&[u8], S1CliError> {
    match product {
        RunProduct::Completed(product) => Ok(&product.final_checkpoint),
        RunProduct::Diverged(product) => Err(S1CliError::RunDiverged { seed: product.seed }),
    }
}

fn product_checkpoint_sha(product: &RunProduct) -> Result<Hash256, S1CliError> {
    match product {
        RunProduct::Completed(product) => Ok(product.final_checkpoint_sha),
        RunProduct::Diverged(product) => Err(S1CliError::RunDiverged { seed: product.seed }),
    }
}

fn product_run_log_self_hash(product: &RunProduct) -> Result<Hash256, S1CliError> {
    match product {
        RunProduct::Completed(product) => Ok(product.run_log.run_log_self_hash),
        RunProduct::Diverged(product) => Err(S1CliError::RunDiverged { seed: product.seed }),
    }
}

fn product_checkpoint_metadata(product: &RunProduct) -> Result<&CheckpointMetadata, S1CliError> {
    match product {
        RunProduct::Completed(product) => Ok(&product.metadata),
        RunProduct::Diverged(product) => Err(S1CliError::RunDiverged { seed: product.seed }),
    }
}

fn fixture_scorer(command: &'static str, enabled: bool) -> Result<UniformScorer, S1CliError> {
    if enabled {
        Ok(UniformScorer)
    } else {
        Err(S1CliError::FixtureScorerRequired { command })
    }
}

fn validate_fixture_checkpoint_sha(
    command: &'static str,
    checkpoint_sha: Hash256,
) -> Result<(), S1CliError> {
    if checkpoint_sha == Hash256::ZERO {
        Ok(())
    } else {
        Err(S1CliError::InvalidArtifact(format!(
            "gbf s1 {command} fixture scorer mode cannot stamp a non-zero --checkpoint-sha; fixture scorer checkpoint hashes are metadata-only and must not masquerade as production closure artifacts"
        )))
    }
}

fn checkpoint_metadata(build_kind: S1BuildKind) -> Result<CheckpointMetadata, S1CliError> {
    Ok(CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed: 0,
        corpus_train_sha: Hash256::ZERO,
        corpus_val_sha: Hash256::ZERO,
        model_config_hash: Hash256::ZERO,
        train_config_hash: Hash256::ZERO,
        build_kind,
        build_config_hash: Hash256::ZERO,
        dependency_lockfile_sha: Hash256::ZERO,
        rust_toolchain_hash: Hash256::ZERO,
        device_profile_hash: Hash256::ZERO,
        rng_stream_def_hash: crate::s1::rng::rng_stream_def_hash(),
        pass_version: CURRENT_PASS_VERSION,
        budget_profile: "integration_fixture".to_owned(),
        final_step: 0,
        final_train_loss: 0.0,
        completion: S1Completion::Completed,
        checkpoint_safetensors_sha256: Hash256::ZERO,
        checkpoint_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?)
}

fn fixture_tensor() -> Result<CanonicalTensor, S1CliError> {
    Ok(CanonicalTensor::new(
        ArtifactPath::new("toy0.fixture.weight")?,
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[1])?,
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(vec![1.0]),
    )?)
}

#[cfg(feature = "falsify")]
fn fixture_mismatch_tensor() -> Result<CanonicalTensor, S1CliError> {
    Ok(CanonicalTensor::new(
        ArtifactPath::new("toy0.fixture.weight")?,
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(&[1])?,
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(vec![2.0]),
    )?)
}

fn print_json<T: Serialize>(value: &T) -> Result<(), S1CliError> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct UniformScorer;

impl ResetContextScorer for UniformScorer {
    type State = ();

    fn fresh_state(&self) -> Self::State {}

    fn logits(&self, _state: &Self::State) -> Vec<f64> {
        vec![0.0; 256]
    }

    fn consume(&self, _state: &mut Self::State, _byte: u8) {}
}

const PRODUCTION_STATE_SLOTS: usize = 4;
const PRODUCTION_STATE_DECAY: f64 = 0.5;
const PRODUCTION_VOCAB_SIZE: usize = 256;
const PREREGISTERED_BPC_3GRAM_BASELINE_MIN: f64 = 1.7;
const PREREGISTERED_BPC_3GRAM_BASELINE_MAX: f64 = 2.0;
const PREREGISTERED_MEDIAN_VAL_BPC_MIN: f64 = 1.4;
const PREREGISTERED_MEDIAN_VAL_BPC_MAX: f64 = 1.8;

#[derive(Debug, Clone)]
struct ProductionCheckpointScorer {
    d_model: usize,
    d_ff: usize,
    embedding: Vec<f64>,
    input_to_state: Vec<f64>,
    state_to_output: Vec<f64>,
    dense_up: Vec<f64>,
    dense_down: Vec<f64>,
}

impl ProductionCheckpointScorer {
    fn from_tensors(tensors: &[CanonicalTensor]) -> Result<Self, S1CliError> {
        let d_model = usize::from(ModelSizeProfile::toy0().d_model());
        let d_ff = usize::from(ModelSizeProfile::toy0().d_ff());
        Ok(Self {
            d_model,
            d_ff,
            embedding: production_f32_tensor(
                tensors,
                "toy0.production.embedding_tied.weight",
                &[PRODUCTION_VOCAB_SIZE, d_model],
            )?,
            input_to_state: production_f32_tensor(
                tensors,
                "toy0.production.linear_state.input_to_state.weight",
                &[d_model, PRODUCTION_STATE_SLOTS],
            )?,
            state_to_output: production_f32_tensor(
                tensors,
                "toy0.production.linear_state.state_to_output.weight",
                &[PRODUCTION_STATE_SLOTS, d_model],
            )?,
            dense_up: production_f32_tensor(
                tensors,
                "toy0.production.dense_ffn.up.weight",
                &[d_model, d_ff],
            )?,
            dense_down: production_f32_tensor(
                tensors,
                "toy0.production.dense_ffn.down.weight",
                &[d_ff, d_model],
            )?,
        })
    }

    fn dense_row_major(values: &[f64], row: usize, col: usize, cols: usize) -> f64 {
        values[row * cols + col]
    }
}

impl ResetContextScorer for ProductionCheckpointScorer {
    type State = Vec<f64>;

    fn fresh_state(&self) -> Self::State {
        vec![0.0; PRODUCTION_STATE_SLOTS]
    }

    fn logits(&self, state: &Self::State) -> Vec<f64> {
        let mut hidden = vec![0.0; self.d_model];
        for (slot, state_value) in state.iter().enumerate().take(PRODUCTION_STATE_SLOTS) {
            for (dim, value) in hidden.iter_mut().enumerate() {
                *value += *state_value
                    * Self::dense_row_major(&self.state_to_output, slot, dim, self.d_model);
            }
        }

        let mut up = vec![0.0; self.d_ff];
        for (ff, value) in up.iter_mut().enumerate() {
            for (dim, hidden_value) in hidden.iter().enumerate() {
                *value += *hidden_value * Self::dense_row_major(&self.dense_up, dim, ff, self.d_ff);
            }
            *value = (*value).max(0.0);
        }

        let mut ffn = vec![0.0; self.d_model];
        for (dim, value) in ffn.iter_mut().enumerate() {
            for (ff, up_value) in up.iter().enumerate() {
                *value +=
                    *up_value * Self::dense_row_major(&self.dense_down, ff, dim, self.d_model);
            }
        }

        for (dim, value) in hidden.iter_mut().enumerate() {
            *value += ffn[dim];
        }

        let mut logits = vec![0.0; PRODUCTION_VOCAB_SIZE];
        for (token, logit) in logits.iter_mut().enumerate() {
            for (dim, hidden_value) in hidden.iter().enumerate() {
                *logit += *hidden_value
                    * Self::dense_row_major(&self.embedding, token, dim, self.d_model);
            }
        }
        logits
    }

    fn consume(&self, state: &mut Self::State, byte: u8) {
        let token = usize::from(byte);
        let mut delta = [0.0_f64; PRODUCTION_STATE_SLOTS];
        for dim in 0..self.d_model {
            let embedding_value = Self::dense_row_major(&self.embedding, token, dim, self.d_model);
            for (slot, value) in delta.iter_mut().enumerate() {
                *value += embedding_value
                    * Self::dense_row_major(
                        &self.input_to_state,
                        dim,
                        slot,
                        PRODUCTION_STATE_SLOTS,
                    );
            }
        }
        for slot in 0..PRODUCTION_STATE_SLOTS {
            state[slot] = state[slot] * PRODUCTION_STATE_DECAY + delta[slot];
        }
    }
}

fn production_f32_tensor(
    tensors: &[CanonicalTensor],
    name: &str,
    expected_shape: &[usize],
) -> Result<Vec<f64>, S1CliError> {
    let tensor = tensors
        .iter()
        .find(|tensor| tensor.id.as_str() == name)
        .ok_or_else(|| {
            S1CliError::InvalidArtifact(format!("production checkpoint is missing tensor {name}"))
        })?;
    let expected_shape = expected_shape
        .iter()
        .copied()
        .map(|dim| {
            u32::try_from(dim).map_err(|_| {
                S1CliError::InvalidArtifact(format!(
                    "production checkpoint expected shape for {name} is too large"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if tensor.layout.shape.dims() != expected_shape.as_slice() {
        return Err(S1CliError::InvalidArtifact(format!(
            "production checkpoint tensor {name} shape mismatch: expected {:?}, observed {:?}",
            expected_shape,
            tensor.layout.shape.dims()
        )));
    }
    let values = tensor.payload.as_f32_slice().ok_or_else(|| {
        S1CliError::InvalidArtifact(format!(
            "production checkpoint tensor {name} must have Float32 payload"
        ))
    })?;
    Ok(values.iter().map(|value| f64::from(*value)).collect())
}

#[derive(Debug, Serialize)]
struct ReplaySummary {
    pass_version: String,
    budget_profile: CliBudgetProfile,
    products: Vec<ReplayProductSummary>,
}

#[derive(Debug, Serialize)]
struct ReplayProductSummary {
    seed: u64,
    completion: &'static str,
    checkpoint_sha: Option<String>,
    run_log_self_hash: String,
}

#[derive(Debug, Serialize)]
struct VerifyDeterminismSummary {
    seed: u64,
    deterministic: bool,
    checkpoint_sha: String,
    run_log_self_hash: String,
    checkpoint_self_hash: String,
}

/// First byte-level mismatch between two replayed byte streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteMismatch {
    /// Byte offset of the first mismatch.
    pub offset: usize,
    /// Expected byte from the first replay, or EOF when the first replay ended.
    pub expected: Option<u8>,
    /// Observed byte from the second replay, or EOF when the second replay ended.
    pub observed: Option<u8>,
}

/// First structure-level mismatch from `gbf s1 verify-determinism`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismMismatchDetail {
    /// Final SafeTensors bytes differed.
    SafetensorsBytes(ByteMismatch),
    /// Run-log self-hashes differed.
    RunLogSelfHash {
        /// Expected self-hash from the first replay.
        expected: Hash256,
        /// Observed self-hash from the second replay.
        observed: Hash256,
    },
    /// Checkpoint metadata differed.
    CheckpointMetadata,
}

impl fmt::Display for DeterminismMismatchDetail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SafetensorsBytes(mismatch) => {
                write!(
                    f,
                    "structure=safetensors_bytes byte_offset={}",
                    mismatch.offset
                )?;
                write_byte_detail(f, "expected", mismatch.expected)?;
                write_byte_detail(f, "observed", mismatch.observed)
            }
            Self::RunLogSelfHash { expected, observed } => write!(
                f,
                "structure=run_log_self_hash expected={expected} observed={observed}"
            ),
            Self::CheckpointMetadata => f.write_str("structure=checkpoint_metadata"),
        }
    }
}

fn write_byte_detail(
    f: &mut fmt::Formatter<'_>,
    label: &'static str,
    value: Option<u8>,
) -> fmt::Result {
    match value {
        Some(byte) => write!(f, " {label}=0x{byte:02X}"),
        None => write!(f, " {label}=EOF"),
    }
}

fn first_byte_mismatch(expected: &[u8], observed: &[u8]) -> ByteMismatch {
    let limit = expected.len().min(observed.len());
    for offset in 0..limit {
        if expected[offset] != observed[offset] {
            return ByteMismatch {
                offset,
                expected: Some(expected[offset]),
                observed: Some(observed[offset]),
            };
        }
    }
    ByteMismatch {
        offset: limit,
        expected: expected.get(limit).copied(),
        observed: observed.get(limit).copied(),
    }
}

fn fixture_dispatch_input(scenario: CliE2eScenario) -> OutcomeDispatchInput {
    let confirmed = HypothesisStatus::Confirmed;
    match scenario {
        CliE2eScenario::PassClean => OutcomeDispatchInput {
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: confirmed.clone(),
            h5: confirmed,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        },
        CliE2eScenario::PassWithWarning => OutcomeDispatchInput {
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: HypothesisStatus::Refuted,
            h4: confirmed.clone(),
            h5: confirmed,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        },
        CliE2eScenario::FailSubstrateNan => OutcomeDispatchInput {
            h1: HypothesisStatus::Refuted,
            h2: not_evaluated("H1 fixture substitute stopped downstream capacity claim"),
            h3: not_evaluated("H1 fixture substitute stopped downstream context claim"),
            h4: not_evaluated("H1 fixture substitute stopped downstream phase claim"),
            h5: confirmed,
            any_seed_diverged: true,
            suspicious_low_bpc: false,
        },
        CliE2eScenario::FailSubstrateZeroGrad => OutcomeDispatchInput {
            h1: HypothesisStatus::Refuted,
            h2: not_evaluated("H1 fixture substitute stopped downstream capacity claim"),
            h3: not_evaluated("H1 fixture substitute stopped downstream context claim"),
            h4: not_evaluated("H1 fixture substitute stopped downstream phase claim"),
            h5: confirmed,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        },
        CliE2eScenario::FailCapacityToytiny => OutcomeDispatchInput {
            h1: confirmed.clone(),
            h2: HypothesisStatus::Refuted,
            h3: confirmed.clone(),
            h4: confirmed.clone(),
            h5: confirmed,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        },
        CliE2eScenario::FailSuspiciousLowBpc => OutcomeDispatchInput {
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: confirmed.clone(),
            h5: confirmed,
            any_seed_diverged: false,
            suspicious_low_bpc: true,
        },
        CliE2eScenario::FailPhaseTernaryLeak => OutcomeDispatchInput {
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: HypothesisStatus::Refuted,
            h5: confirmed,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        },
        CliE2eScenario::FailMetricModuloShuffle => OutcomeDispatchInput {
            h1: confirmed.clone(),
            h2: confirmed.clone(),
            h3: confirmed.clone(),
            h4: confirmed,
            h5: HypothesisStatus::Refuted,
            any_seed_diverged: false,
            suspicious_low_bpc: false,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn fixture_dispatch_input_from_artifacts(
    scenario: CliE2eScenario,
    baseline: &BaselineReport,
    oracle: &OracleReport,
    negative: &NegativeTestReport,
    ablation: &AblationReport,
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    scores: &[Option<ScoreReport>],
) -> Result<OutcomeDispatchInput, S1CliError> {
    let scenario_defaults = fixture_dispatch_input(scenario);
    let any_seed_diverged = any_seed_diverged(metadata, run_logs);
    let zero_gradient_substitute = any_zero_final_grad_norm(run_logs);
    let h1 = if any_seed_diverged || zero_gradient_substitute {
        HypothesisStatus::Refuted
    } else {
        scenario_defaults.h1
    };
    let h3 = if negative.sensitive {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };
    let h2 = match best_score_bpc(scores) {
        Some(best_bpc) if score_beats_baseline(best_bpc, baseline) => HypothesisStatus::Confirmed,
        Some(_) => HypothesisStatus::Refuted,
        None => scenario_defaults.h2,
    };
    let h4 = if ablation.phase_a_eq_ablation {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };
    let h5 = oracle.h5_status()?;
    let suspicious_low_bpc = scores
        .iter()
        .flatten()
        .any(|score| score.bpc.is_finite() && score.bpc < 0.5);

    Ok(OutcomeDispatchInput {
        h1,
        h2,
        h3,
        h4,
        h5,
        any_seed_diverged,
        suspicious_low_bpc,
    })
}

fn any_seed_diverged(metadata: &[Option<CheckpointMetadata>], run_logs: &[Option<RunLog>]) -> bool {
    run_logs.iter().enumerate().any(|(seed, run_log)| {
        run_log.is_some() && metadata.get(seed).and_then(Option::as_ref).is_none()
    })
}

fn any_zero_final_grad_norm(run_logs: &[Option<RunLog>]) -> bool {
    run_logs.iter().flatten().any(|run_log| {
        !run_log.losses.is_empty()
            && run_log.final_grad_norms.global_l2 == 0.0
            && run_log.final_grad_norms.max_l2 == 0.0
            && run_log.final_grad_norms.mean_l2 == 0.0
    })
}

fn best_score_bpc(scores: &[Option<ScoreReport>]) -> Option<f64> {
    scores
        .iter()
        .flatten()
        .map(|score| score.bpc)
        .min_by(f64::total_cmp)
}

fn production_score_bpcs(scores: &[Option<ScoreReport>]) -> Option<Vec<(u64, f64)>> {
    let mut bpcs = Vec::new();
    for seed in 0..5 {
        let score = scores.get(seed)?.as_ref()?;
        bpcs.push((seed as u64, score.bpc));
    }
    Some(bpcs)
}

fn bpc_summary(bpcs: &[(u64, f64)]) -> Option<(f64, f64, f64)> {
    if bpcs.is_empty() {
        return None;
    }
    let mut values = bpcs.iter().map(|(_, bpc)| *bpc).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    Some((
        values[0],
        values[values.len() / 2],
        values[values.len() - 1],
    ))
}

fn score_beats_baseline(best_bpc: f64, baseline: &BaselineReport) -> bool {
    best_bpc <= baseline.bpc_3gram - 0.05
}

struct FixtureReportArtifacts<'a> {
    scenario: CliE2eScenario,
    baseline: &'a BaselineReport,
    oracle: &'a OracleReport,
    negative: &'a NegativeTestReport,
    ablation: &'a AblationReport,
    metadata: &'a [Option<CheckpointMetadata>],
    run_logs: &'a [Option<RunLog>],
    scores: &'a [Option<ScoreReport>],
    dispatch_input: OutcomeDispatchInput,
    decision: S1Decision,
}

fn fixture_report_input(artifacts: FixtureReportArtifacts<'_>) -> Result<ReportInput, S1CliError> {
    let FixtureReportArtifacts {
        scenario,
        baseline,
        oracle,
        negative,
        ablation,
        metadata,
        run_logs,
        scores,
        dispatch_input,
        decision,
    } = artifacts;
    let outcome = dispatch_outcome(&dispatch_input)?.outcome;
    let per_seed_artifacts = (0..5)
        .map(|seed| PerSeedArtifacts {
            seed: seed as u64,
            completion: completion_for_report(seed, metadata, run_logs),
            checkpoint_self_hash: metadata
                .get(seed)
                .and_then(Option::as_ref)
                .map(|artifact| artifact.checkpoint_self_hash),
            run_log_self_hash: run_logs
                .get(seed)
                .and_then(Option::as_ref)
                .map(|artifact| artifact.run_log_self_hash),
            score_self_hash: scores
                .get(seed)
                .and_then(Option::as_ref)
                .map(|artifact| artifact.score_self_hash),
            negative_self_hash: (seed == 0).then_some(negative.negative_self_hash),
            ablation_self_hash: (seed == 0).then_some(ablation.ablation_self_hash),
        })
        .collect::<Vec<_>>();

    Ok(ReportInput {
        front_matter: ReportFrontMatter {
            schema: "s1_report.v1".to_owned(),
            s1_outcome: outcome,
            decision,
            baseline_self_hash: baseline.baseline_self_hash,
            per_seed_artifacts,
            generated_at: "2026-05-09T12:00:00Z".to_owned(),
            rfc_revision: RfcRevisionRef::GitCommitId(fixture_commit('a')?),
            predictions_section_hash: predictions_section_hash(fixture_predictions())?,
            predictions_commit: fixture_commit('b')?,
            first_result_commit: fixture_commit('c')?,
            report_self_hash: Hash256::ZERO,
        },
        predictions_markdown: fixture_predictions().to_owned(),
        observed_per_seed: (0..5)
            .map(|seed| ObservedSeed {
                seed: seed as u64,
                completion: completion_for_report(seed, metadata, run_logs),
                val_bpc: scores
                    .get(seed)
                    .and_then(Option::as_ref)
                    .map(|score| score.bpc),
                neg_test_delta: (seed == 0).then_some(negative.delta),
                ablation_eq: (seed == 0).then_some(ablation.phase_a_eq_ablation),
            })
            .collect(),
        hypotheses: fixture_hypotheses(
            scenario,
            baseline,
            oracle,
            negative,
            ablation,
            metadata,
            run_logs,
            scores,
            dispatch_input,
        ),
        falsification_analysis: format!(
            "{scenario} CLI-backed IntegrationFixture producer composed existing S1 artifacts."
        ),
        surprises: if scenario == CliE2eScenario::FailSuspiciousLowBpc {
            "Suspicious-low-bpc fixture substitute fired.".to_owned()
        } else {
            "None.".to_owned()
        },
        decision_justification: "Decision follows RFC section 8 dispatch.".to_owned(),
        replay_command: format!("scripts/s1_e2e_cli.sh --scenario {scenario} --fixture tiny"),
        manifest_hashes: format!(
            "train_sha={} val_sha={}",
            baseline.corpus_train_sha, baseline.corpus_val_sha
        ),
        pass_version: CURRENT_PASS_VERSION.to_string(),
    })
}

struct ProductionReportArtifacts<'a> {
    baseline: &'a BaselineReport,
    oracle: &'a OracleReport,
    negative: &'a NegativeTestReport,
    ablation: &'a AblationReport,
    metadata: &'a [Option<CheckpointMetadata>],
    run_logs: &'a [Option<RunLog>],
    scores: &'a [Option<ScoreReport>],
    dispatch_input: OutcomeDispatchInput,
    decision: S1Decision,
    generated_at: String,
    rfc_revision: RfcRevisionRef,
    predictions_markdown: String,
    predictions_commit: GitCommitId,
    first_result_commit: GitCommitId,
}

fn production_report_input(
    artifacts: ProductionReportArtifacts<'_>,
) -> Result<ReportInput, S1CliError> {
    let ProductionReportArtifacts {
        baseline,
        oracle,
        negative,
        ablation,
        metadata,
        run_logs,
        scores,
        dispatch_input,
        decision,
        generated_at,
        rfc_revision,
        predictions_markdown,
        predictions_commit,
        first_result_commit,
    } = artifacts;
    let outcome = dispatch_outcome(&dispatch_input)?.outcome;
    let suspicious_low_bpc = dispatch_input.suspicious_low_bpc;
    let per_seed_artifacts = (0..5)
        .map(|seed| PerSeedArtifacts {
            seed: seed as u64,
            completion: completion_for_report(seed, metadata, run_logs),
            checkpoint_self_hash: metadata
                .get(seed)
                .and_then(Option::as_ref)
                .map(|artifact| artifact.checkpoint_self_hash),
            run_log_self_hash: run_logs
                .get(seed)
                .and_then(Option::as_ref)
                .map(|artifact| artifact.run_log_self_hash),
            score_self_hash: scores
                .get(seed)
                .and_then(Option::as_ref)
                .map(|artifact| artifact.score_self_hash),
            negative_self_hash: (seed == 0).then_some(negative.negative_self_hash),
            ablation_self_hash: (seed == 0).then_some(ablation.ablation_self_hash),
        })
        .collect::<Vec<_>>();

    Ok(ReportInput {
        front_matter: ReportFrontMatter {
            schema: "s1_report.v1".to_owned(),
            s1_outcome: outcome,
            decision,
            baseline_self_hash: baseline.baseline_self_hash,
            per_seed_artifacts,
            generated_at,
            rfc_revision,
            predictions_section_hash: predictions_section_hash(&predictions_markdown)?,
            predictions_commit,
            first_result_commit,
            report_self_hash: Hash256::ZERO,
        },
        predictions_markdown,
        observed_per_seed: (0..5)
            .map(|seed| ObservedSeed {
                seed: seed as u64,
                completion: completion_for_report(seed, metadata, run_logs),
                val_bpc: scores
                    .get(seed)
                    .and_then(Option::as_ref)
                    .map(|score| score.bpc),
                neg_test_delta: (seed == 0).then_some(negative.delta),
                ablation_eq: (seed == 0).then_some(ablation.phase_a_eq_ablation),
            })
            .collect(),
        hypotheses: production_hypotheses(
            baseline,
            oracle,
            negative,
            ablation,
            metadata,
            run_logs,
            scores,
            dispatch_input.clone(),
        ),
        falsification_analysis: production_falsification_analysis(
            &dispatch_input,
            baseline,
            scores,
        ),
        surprises: production_surprises(baseline, scores, suspicious_low_bpc),
        decision_justification: "Decision follows RFC section 8 dispatch.".to_owned(),
        replay_command:
            "gbf s1 replay --manifest fixtures/corpora/tinystories.toml --seed-list 0,1,2,3,4"
                .to_owned(),
        manifest_hashes: format!(
            "train_sha={} val_sha={}",
            baseline.corpus_train_sha, baseline.corpus_val_sha
        ),
        pass_version: CURRENT_PASS_VERSION.to_string(),
    })
}

fn completion_for_report(
    seed: usize,
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
) -> S1Completion {
    metadata
        .get(seed)
        .and_then(Option::as_ref)
        .map(|artifact| artifact.completion.clone())
        .or_else(|| {
            run_logs
                .get(seed)
                .and_then(Option::as_ref)
                .map(|_| S1Completion::DivergedAt { step: 0 })
        })
        .unwrap_or(S1Completion::NotReached)
}

#[allow(clippy::too_many_arguments)]
fn fixture_hypotheses(
    scenario: CliE2eScenario,
    baseline: &BaselineReport,
    oracle: &OracleReport,
    negative: &NegativeTestReport,
    ablation: &AblationReport,
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    scores: &[Option<ScoreReport>],
    input: OutcomeDispatchInput,
) -> Vec<HypothesisFinding> {
    [
        (
            Hypothesis::H1,
            input.h1,
            h1_observation(scenario, metadata, run_logs),
        ),
        (Hypothesis::H2, input.h2, h2_observation(baseline, scores)),
        (
            Hypothesis::H3,
            input.h3,
            format!(
                "CLI negative-test delta {:.6} sensitive={}",
                negative.delta, negative.sensitive
            ),
        ),
        (
            Hypothesis::H4,
            input.h4,
            format!(
                "CLI ablation phase_a_eq_ablation={}",
                ablation.phase_a_eq_ablation
            ),
        ),
        (
            Hypothesis::H5,
            input.h5,
            format!(
                "CLI oracle metric_oracle_passed={} failed_ids={:?}",
                oracle.metric_oracle_passed, oracle.failed_oracle_ids
            ),
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

fn h1_observation(
    scenario: CliE2eScenario,
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
) -> String {
    if any_seed_diverged(metadata, run_logs) {
        return match scenario {
            CliE2eScenario::FailSubstrateNan => {
                "run_log artifact exists without checkpoint metadata after non_finite_loss"
                    .to_owned()
            }
            _ => "run_log artifact exists without checkpoint metadata, indicating divergence"
                .to_owned(),
        };
    }
    if any_zero_final_grad_norm(run_logs) {
        return "run_log final_grad_norms observed zero_grad".to_owned();
    }
    "CLI replay produced the expected seed artifacts".to_owned()
}

fn h2_observation(baseline: &BaselineReport, scores: &[Option<ScoreReport>]) -> String {
    match best_score_bpc(scores) {
        Some(best_bpc) => format!(
            "CLI score artifacts best val_bpc {:.6} evaluated against baseline bpc_3gram {:.6}",
            best_bpc, baseline.bpc_3gram
        ),
        None => format!(
            "CLI score artifacts unavailable; baseline bpc_3gram {:.6}",
            baseline.bpc_3gram
        ),
    }
}

fn production_dispatch_input_from_artifacts(
    baseline: &BaselineReport,
    oracle: &OracleReport,
    negative: &NegativeTestReport,
    ablation: &AblationReport,
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    scores: &[Option<ScoreReport>],
) -> Result<OutcomeDispatchInput, S1CliError> {
    let any_seed_diverged = any_seed_diverged(metadata, run_logs);
    let h1 = production_h1_status(metadata, run_logs, any_seed_diverged);

    let (h2, h3, h4) = if matches!(h1, HypothesisStatus::Confirmed) {
        let h2 = production_h2_status(baseline, scores);
        let h3 = if production_h3_passes(baseline, negative, scores) {
            HypothesisStatus::Confirmed
        } else {
            HypothesisStatus::Refuted
        };
        let h4 = if ablation.phase_a_eq_ablation {
            HypothesisStatus::Confirmed
        } else {
            HypothesisStatus::Refuted
        };
        (h2, h3, h4)
    } else {
        (
            not_evaluated("H1 production replay gate stopped downstream capacity claim"),
            not_evaluated("H1 production replay gate stopped downstream context claim"),
            not_evaluated("H1 production replay gate stopped downstream ablation claim"),
        )
    };
    let h5 = oracle.h5_status()?;
    let suspicious_low_bpc = production_score_bpcs(scores)
        .and_then(|bpcs| bpc_summary(&bpcs).map(|(_, median, _)| median < 0.5))
        .unwrap_or(false);

    Ok(OutcomeDispatchInput {
        h1,
        h2,
        h3,
        h4,
        h5,
        any_seed_diverged,
        suspicious_low_bpc,
    })
}

fn production_h1_status(
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    any_seed_diverged: bool,
) -> HypothesisStatus {
    if any_seed_diverged {
        return HypothesisStatus::Refuted;
    }
    for seed in 0..5 {
        let completed = metadata
            .get(seed)
            .and_then(Option::as_ref)
            .map(|metadata| metadata.completion == S1Completion::Completed)
            .unwrap_or(false);
        let Some(run_log) = run_logs.get(seed).and_then(Option::as_ref) else {
            return HypothesisStatus::Refuted;
        };
        if !completed || !production_run_log_h1_passes(run_log) {
            return HypothesisStatus::Refuted;
        }
    }
    HypothesisStatus::Confirmed
}

fn production_run_log_h1_passes(run_log: &RunLog) -> bool {
    let Some(mean_1_10) = mean_loss_for_steps(run_log, 1, 10) else {
        return false;
    };
    let Some(mean_91_100) = mean_loss_for_steps(run_log, 91, 100) else {
        return false;
    };
    if mean_91_100.partial_cmp(&(mean_1_10 - 0.5)) != Some(std::cmp::Ordering::Less) {
        return false;
    }
    if !run_log
        .losses
        .iter()
        .all(|(_, loss)| loss.is_finite() && *loss >= 0.0)
    {
        return false;
    }
    let grad_norms = [
        run_log.final_grad_norms.global_l2,
        run_log.final_grad_norms.max_l2,
        run_log.final_grad_norms.mean_l2,
    ];
    grad_norms
        .iter()
        .all(|grad_norm| grad_norm.is_finite() && *grad_norm >= 0.0)
        && grad_norms.iter().any(|grad_norm| *grad_norm > 0.0)
}

fn mean_loss_for_steps(run_log: &RunLog, start: u64, end: u64) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0usize;
    for (step, loss) in &run_log.losses {
        if (start..=end).contains(step) {
            total += f64::from(*loss);
            count += 1;
        }
    }
    let expected = (end - start + 1) as usize;
    (count == expected).then_some(total / count as f64)
}

fn production_h2_status(
    baseline: &BaselineReport,
    scores: &[Option<ScoreReport>],
) -> HypothesisStatus {
    if production_h2_passes(baseline, scores) {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    }
}

fn production_h2_passes(baseline: &BaselineReport, scores: &[Option<ScoreReport>]) -> bool {
    let threshold = baseline.bpc_3gram - 0.05;
    production_score_bpcs(scores)
        .map(|bpcs| {
            bpcs.iter()
                .all(|(_, bpc)| bpc.is_finite() && *bpc < threshold)
        })
        .unwrap_or(false)
}

fn production_h3_passes(
    baseline: &BaselineReport,
    negative: &NegativeTestReport,
    scores: &[Option<ScoreReport>],
) -> bool {
    let threshold = baseline.bpc_unigram - 0.5;
    let scores_pass = production_score_bpcs(scores)
        .map(|bpcs| {
            bpcs.iter()
                .all(|(_, bpc)| bpc.is_finite() && *bpc < threshold)
        })
        .unwrap_or(false);
    scores_pass && negative.sensitive && negative.delta.is_finite() && negative.delta > 2.0
}

#[allow(clippy::too_many_arguments)]
fn production_hypotheses(
    baseline: &BaselineReport,
    oracle: &OracleReport,
    negative: &NegativeTestReport,
    ablation: &AblationReport,
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    scores: &[Option<ScoreReport>],
    input: OutcomeDispatchInput,
) -> Vec<HypothesisFinding> {
    [
        (
            Hypothesis::H1,
            input.h1,
            production_h1_observation(metadata, run_logs, input.any_seed_diverged),
        ),
        (
            Hypothesis::H2,
            input.h2,
            production_h2_observation(baseline, scores),
        ),
        (
            Hypothesis::H3,
            input.h3,
            production_h3_observation(baseline, negative, scores),
        ),
        (
            Hypothesis::H4,
            input.h4,
            format!(
                "Production ablation phase_a_eq_ablation={}",
                ablation.phase_a_eq_ablation
            ),
        ),
        (
            Hypothesis::H5,
            input.h5,
            format!(
                "Production oracle metric_oracle_passed={} failed_ids={:?}",
                oracle.metric_oracle_passed, oracle.failed_oracle_ids
            ),
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

fn production_falsification_analysis(
    input: &OutcomeDispatchInput,
    baseline: &BaselineReport,
    scores: &[Option<ScoreReport>],
) -> String {
    let mut lines = vec![
        "Production S1 CLI report collected canonical TinyStories artifacts from disk.".to_owned(),
    ];
    if input.h2 == HypothesisStatus::Refuted {
        lines.push(format!(
            "H2 falsification: {}",
            production_h2_observation(baseline, scores)
        ));
    }
    lines.join("\n")
}

fn production_surprises(
    baseline: &BaselineReport,
    scores: &[Option<ScoreReport>],
    suspicious_low_bpc: bool,
) -> String {
    let mut surprises = Vec::new();
    if outside_inclusive(
        baseline.bpc_3gram,
        PREREGISTERED_BPC_3GRAM_BASELINE_MIN,
        PREREGISTERED_BPC_3GRAM_BASELINE_MAX,
    ) {
        surprises.push(format!(
            "bpc_3gram_baseline {:.6} was outside preregistered sanity range [{:.1}, {:.1}]; this range miss is reported as a Surprise, not a verdict change.",
            baseline.bpc_3gram,
            PREREGISTERED_BPC_3GRAM_BASELINE_MIN,
            PREREGISTERED_BPC_3GRAM_BASELINE_MAX
        ));
    }
    if let Some(bpcs) = production_score_bpcs(scores) {
        let (_, median, _) = bpc_summary(&bpcs).expect("non-empty bpcs");
        if outside_inclusive(
            median,
            PREREGISTERED_MEDIAN_VAL_BPC_MIN,
            PREREGISTERED_MEDIAN_VAL_BPC_MAX,
        ) && !suspicious_low_bpc
        {
            surprises.push(format!(
                "median(val_bpc) {:.6} was outside preregistered sanity range [{:.1}, {:.1}]; this range miss is reported as a Surprise, not a verdict change.",
                median,
                PREREGISTERED_MEDIAN_VAL_BPC_MIN,
                PREREGISTERED_MEDIAN_VAL_BPC_MAX
            ));
        }
    }
    if suspicious_low_bpc {
        surprises.push("Suspicious-low-bpc sentinel fired.".to_owned());
    }
    if surprises.is_empty() {
        "None.".to_owned()
    } else {
        surprises
            .into_iter()
            .map(|surprise| format!("- {surprise}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn outside_inclusive(value: f64, min: f64, max: f64) -> bool {
    !value.is_finite() || value < min || value > max
}

fn production_h1_observation(
    metadata: &[Option<CheckpointMetadata>],
    run_logs: &[Option<RunLog>],
    any_seed_diverged: bool,
) -> String {
    if any_seed_diverged {
        return "Production run log exists without completed checkpoint metadata, indicating divergence".to_owned();
    }
    for seed in 0..5 {
        let completed = metadata
            .get(seed)
            .and_then(Option::as_ref)
            .map(|metadata| metadata.completion == S1Completion::Completed)
            .unwrap_or(false);
        let Some(run_log) = run_logs.get(seed).and_then(Option::as_ref) else {
            return format!("Production seed {seed} is missing run_log evidence");
        };
        let mean_1_10 = mean_loss_for_steps(run_log, 1, 10);
        let mean_91_100 = mean_loss_for_steps(run_log, 91, 100);
        let grad_norms = [
            run_log.final_grad_norms.global_l2,
            run_log.final_grad_norms.max_l2,
            run_log.final_grad_norms.mean_l2,
        ];
        if !completed {
            return format!("Production seed {seed} did not complete metadata gate");
        }
        if let (Some(first), Some(later)) = (mean_1_10, mean_91_100) {
            let grad_summary = format!(
                "global={:.6} max={:.6} mean={:.6}",
                grad_norms[0], grad_norms[1], grad_norms[2]
            );
            if !production_run_log_h1_passes(run_log) {
                return format!(
                    "Production seed {seed} failed H1: mean1_10={first:.6}, mean91_100={later:.6}, final_grad_norms {grad_summary}"
                );
            }
        } else {
            return format!("Production seed {seed} is missing H1 loss-window steps");
        }
    }
    let min_drop = (0..5)
        .filter_map(|seed| run_logs.get(seed).and_then(Option::as_ref))
        .filter_map(|run_log| {
            Some(mean_loss_for_steps(run_log, 1, 10)? - mean_loss_for_steps(run_log, 91, 100)?)
        })
        .fold(f64::INFINITY, f64::min);
    format!(
        "Production seeds 0..4 completed; minimum H1 loss-window drop was {min_drop:.6} and final grad norms were finite, nonnegative, and nonzero"
    )
}

fn production_h2_observation(baseline: &BaselineReport, scores: &[Option<ScoreReport>]) -> String {
    let threshold = baseline.bpc_3gram - 0.05;
    let Some(bpcs) = production_score_bpcs(scores) else {
        return format!(
            "Production score artifacts incomplete; H2 threshold is val_bpc < {threshold:.6}"
        );
    };
    let (best, median, worst) = bpc_summary(&bpcs).expect("non-empty bpcs");
    if let Some((seed, bpc)) = bpcs
        .iter()
        .find(|(_, bpc)| !bpc.is_finite() || *bpc >= threshold)
    {
        return format!(
            "Production seed {seed} val_bpc {bpc:.6} failed H2 threshold {threshold:.6}; best={best:.6} median={median:.6} worst={worst:.6}"
        );
    }
    format!(
        "Production all-seed val_bpc beat H2 threshold {threshold:.6}; best={best:.6} median={median:.6} worst={worst:.6}"
    )
}

fn production_h3_observation(
    baseline: &BaselineReport,
    negative: &NegativeTestReport,
    scores: &[Option<ScoreReport>],
) -> String {
    let threshold = baseline.bpc_unigram - 0.5;
    let Some(bpcs) = production_score_bpcs(scores) else {
        return format!(
            "Production score artifacts incomplete; H3 score threshold is val_bpc < {threshold:.6}; negative delta {:.6} sensitive={}",
            negative.delta, negative.sensitive
        );
    };
    let (best, median, worst) = bpc_summary(&bpcs).expect("non-empty bpcs");
    if let Some((seed, bpc)) = bpcs
        .iter()
        .find(|(_, bpc)| !bpc.is_finite() || *bpc >= threshold)
    {
        return format!(
            "Production seed {seed} val_bpc {bpc:.6} failed H3 unigram threshold {threshold:.6}; best={best:.6} median={median:.6} worst={worst:.6}; negative delta {:.6} sensitive={}",
            negative.delta, negative.sensitive
        );
    }
    format!(
        "Production all-seed val_bpc beat H3 unigram threshold {threshold:.6}; best={best:.6} median={median:.6} worst={worst:.6}; negative delta {:.6} sensitive={}",
        negative.delta, negative.sensitive
    )
}

fn read_json_artifact<T: serde::de::DeserializeOwned>(path: PathBuf) -> Result<T, S1CliError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn read_json_artifact_candidate<T: serde::de::DeserializeOwned>(
    candidates: Vec<PathBuf>,
    label: &str,
) -> Result<T, S1CliError> {
    let path = read_json_artifact_candidate_path(candidates, label)?;
    read_json_artifact(path)
}

fn read_json_artifact_candidate_path(
    candidates: Vec<PathBuf>,
    label: &str,
) -> Result<PathBuf, S1CliError> {
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .ok_or_else(|| {
            S1CliError::InvalidArtifact(format!(
                "production report missing {label}; checked {}",
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

fn production_candidates(root: &Path, relative: &[&str]) -> Vec<PathBuf> {
    relative.iter().map(|path| root.join(path)).collect()
}

fn read_optional_json_artifact<T: serde::de::DeserializeOwned>(
    path: PathBuf,
) -> Result<Option<T>, S1CliError> {
    if path.exists() {
        Ok(Some(read_json_artifact(path)?))
    } else {
        Ok(None)
    }
}

fn fixture_predictions() -> &'static str {
    "Fixture predictions are pre-registered for the S1 CLI-backed E2E producer."
}

fn fixture_commit(fill: char) -> Result<GitCommitId, crate::s1::schema::S1SchemaError> {
    GitCommitId::new(fill.to_string().repeat(40))
}

fn not_evaluated(reason: &str) -> HypothesisStatus {
    HypothesisStatus::NotEvaluatedDueToPriorGate(reason.to_owned())
}

fn required_string<'a>(value: Option<&'a str>, detail: &str) -> Result<&'a str, S1CliError> {
    value.ok_or_else(|| S1CliError::InvalidArtifact(detail.to_owned()))
}

fn parse_git_commit(value: &str) -> Result<GitCommitId, S1CliError> {
    GitCommitId::new(value.to_owned()).map_err(S1CliError::Schema)
}

fn parse_rfc_revision(value: &str) -> Result<RfcRevisionRef, S1CliError> {
    if value.starts_with("sha256:") {
        return Ok(RfcRevisionRef::Hash256(Hash256::from_str(value).map_err(
            |error| S1CliError::InvalidArtifact(format!("invalid --rfc-revision hash: {error}")),
        )?));
    }
    Ok(RfcRevisionRef::GitCommitId(parse_git_commit(value)?))
}

#[derive(Debug, Serialize)]
struct OracleSmokeSummary {
    status: &'static str,
    owner: &'static str,
}

#[derive(Debug, Serialize)]
struct ReportSmokeSummary {
    status: &'static str,
    owner: &'static str,
}

#[derive(Debug, Serialize)]
struct ReportFixtureSummary {
    scenario: String,
    outcome: String,
    decision: String,
    report_self_hash: String,
    output_path: String,
}

#[derive(Debug, Serialize)]
struct ReportProductionSummary {
    mode: &'static str,
    outcome: String,
    decision: String,
    report_self_hash: String,
    output_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DoctorStatus {
    Pass,
    Fail,
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: &'static str,
    description: &'static str,
    status: DoctorStatus,
    detail: String,
}

impl DoctorCheck {
    fn pass(name: &'static str, description: &'static str, detail: String) -> Self {
        Self {
            name,
            description,
            status: DoctorStatus::Pass,
            detail,
        }
    }

    fn fail(name: &'static str, description: &'static str, detail: String) -> Self {
        Self {
            name,
            description,
            status: DoctorStatus::Fail,
            detail,
        }
    }
}

#[derive(Debug, Serialize)]
struct DoctorSummary {
    command: &'static str,
    ok: bool,
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
struct PrintConfigSummary {
    command: &'static str,
    pass_version: String,
    train_config_hash: String,
    model_config_hash: String,
    build_config_hash: String,
    dependency_lockfile_sha: String,
    rust_toolchain_hash: String,
    active_features: Vec<&'static str>,
    build_metadata: BuildMetadata,
    train_config: TrainConfig,
    model_config: ModelSizeProfile,
    device_profile: S1CpuDeterministic,
}

#[derive(Debug)]
struct ArtifactInspection {
    schema: String,
    stored_self_hash: Hash256,
    computed_self_hash: Hash256,
    self_hash_ok: bool,
    extra: Option<Value>,
    artifact: Value,
}

#[derive(Debug, Serialize)]
struct CheckpointDiffSummary {
    equal: bool,
    a_tensor_payload_hash: String,
    b_tensor_payload_hash: String,
    tensors: Vec<CheckpointTensorDiff>,
}

#[derive(Debug, Serialize)]
struct CheckpointTensorDiff {
    tensor_name: String,
    a_shape: Option<Vec<u32>>,
    b_shape: Option<Vec<u32>>,
    a_dtype: Option<TensorElementType>,
    b_dtype: Option<TensorElementType>,
    equal: bool,
    first_byte_mismatch: Option<CheckpointByteMismatch>,
}

#[derive(Debug, Serialize)]
struct CheckpointByteMismatch {
    offset: usize,
    a: Option<u8>,
    b: Option<u8>,
}

#[derive(Debug, Serialize)]
struct RunLogInspectionExtra {
    eval_points: Vec<RunLogEvalPoint>,
    final_grad_norms: crate::s1::schema::GradNormSummary,
}

#[derive(Debug, Serialize)]
struct RunLogEvalPoint {
    step: u64,
    bpc: f64,
}

fn check_result<T, E>(
    name: &'static str,
    description: &'static str,
    result: Result<T, E>,
) -> DoctorCheck
where
    T: fmt::Display,
    E: fmt::Display,
{
    match result {
        Ok(detail) => DoctorCheck::pass(name, description, detail.to_string()),
        Err(error) => DoctorCheck::fail(name, description, error.to_string()),
    }
}

fn print_doctor_table(summary: &DoctorSummary) {
    let color = std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none();
    println!("S1 doctor: {}", doctor_status_label(summary.ok, color));
    println!("{:<32} {:<4} detail", "check", "stat");
    for check in &summary.checks {
        let status = doctor_status_label(check.status == DoctorStatus::Pass, color);
        println!("{:<32} {:<4} {}", check.name, status, check.detail);
    }
}

fn doctor_status_label(pass: bool, color: bool) -> &'static str {
    match (pass, color) {
        (true, true) => "\x1b[32mPASS\x1b[0m",
        (false, true) => "\x1b[31mFAIL\x1b[0m",
        (true, false) => "PASS",
        (false, false) => "FAIL",
    }
}

fn compiled_dependency_lockfile_sha() -> Hash256 {
    sha256(include_bytes!("../../../Cargo.lock"))
}

fn dependency_lockfile_check() -> Result<String, S1CliError> {
    let runtime = sha256(fs::read(workspace_root().join("Cargo.lock"))?);
    let compiled = compiled_dependency_lockfile_sha();
    if runtime == compiled {
        Ok(runtime.to_string())
    } else {
        Err(S1CliError::InvalidArtifact(format!(
            "Cargo.lock sha mismatch: runtime {runtime}, compiled {compiled}"
        )))
    }
}

fn burn_version_pin_check() -> Result<String, S1CliError> {
    let cargo_toml = fs::read_to_string(workspace_root().join("Cargo.toml"))?;
    let Some(line) = cargo_toml
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("burn ") || line.starts_with("burn="))
    else {
        return Err(S1CliError::InvalidArtifact(
            "workspace Cargo.toml does not declare burn".to_owned(),
        ));
    };
    if line.contains("version = \"=") || line.contains("version=\"=") {
        Ok(line.to_owned())
    } else {
        Err(S1CliError::InvalidArtifact(format!(
            "Burn dependency must use exact = pin, observed {line}"
        )))
    }
}

fn gpu_absence_check() -> Result<String, S1CliError> {
    let profile = S1CpuDeterministic::canonical();
    if profile.gpu_allowed {
        return Err(S1CliError::InvalidArtifact(
            "S1CpuDeterministic unexpectedly allows GPU".to_owned(),
        ));
    }
    let active = [
        "CUDA_VISIBLE_DEVICES",
        "HIP_VISIBLE_DEVICES",
        "ROCR_VISIBLE_DEVICES",
        "NVIDIA_VISIBLE_DEVICES",
    ]
    .into_iter()
    .filter_map(|var| {
        env::var(var)
            .ok()
            .filter(|value| {
                let normalized = value.trim().to_ascii_lowercase();
                !(normalized.is_empty() || normalized == "-1" || normalized == "none")
            })
            .map(|value| format!("{var}={value}"))
    })
    .collect::<Vec<_>>();
    if active.is_empty() {
        Ok("gpu_allowed=false and no active CUDA/HIP visibility env".to_owned())
    } else {
        Err(S1CliError::InvalidArtifact(format!(
            "GPU visibility env is active: {}",
            active.join(", ")
        )))
    }
}

fn disk_space_check(out_dir: &Path) -> Result<String, S1CliError> {
    let probe_path = existing_path_for_df(out_dir);
    let output = Command::new("/bin/df")
        .args(["-Pk"])
        .arg(&probe_path)
        .output()?;
    if !output.status.success() {
        return Err(S1CliError::InvalidArtifact(format!(
            "df failed for {}",
            probe_path.display()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().last() else {
        return Err(S1CliError::InvalidArtifact(
            "df produced no output".to_owned(),
        ));
    };
    let fields = line.split_whitespace().collect::<Vec<_>>();
    let Some(available_kib) = fields.get(3).and_then(|field| field.parse::<u64>().ok()) else {
        return Err(S1CliError::InvalidArtifact(format!(
            "could not parse df available blocks from {line:?}"
        )));
    };
    let available_bytes = available_kib.saturating_mul(1024);
    if available_bytes > 1_073_741_824 {
        Ok(format!(
            "{available_bytes} bytes available at {}",
            probe_path.display()
        ))
    } else {
        Err(S1CliError::InvalidArtifact(format!(
            "only {available_bytes} bytes available at {}",
            probe_path.display()
        )))
    }
}

fn existing_path_for_df(path: &Path) -> PathBuf {
    let mut probe = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root().join(path)
    };
    while !probe.exists() {
        if !probe.pop() {
            return workspace_root();
        }
    }
    probe
}

fn inspect_json_artifact(schema: &str, value: Value) -> Result<ArtifactInspection, S1CliError> {
    match schema {
        "s1_checkpoint.v1" => inspect_self_hashed_artifact::<CheckpointMetadata>(
            value,
            schema,
            |artifact| artifact.checkpoint_self_hash,
            CheckpointMetadata::computed_self_hash,
            None,
        ),
        "s1_run_log.v1" => {
            let run_log: RunLog = serde_json::from_value(value.clone())?;
            let extra = json!(RunLogInspectionExtra {
                eval_points: run_log
                    .eval_points
                    .iter()
                    .map(|(step, bpc)| RunLogEvalPoint {
                        step: *step,
                        bpc: *bpc,
                    })
                    .collect(),
                final_grad_norms: run_log.final_grad_norms.clone(),
            });
            inspect_self_hashed_artifact::<RunLog>(
                value,
                schema,
                |artifact| artifact.run_log_self_hash,
                RunLog::computed_self_hash,
                Some(extra),
            )
        }
        "s1_score.v1" => inspect_self_hashed_artifact::<ScoreReport>(
            value,
            schema,
            |artifact| artifact.score_self_hash,
            ScoreReport::computed_self_hash,
            None,
        ),
        "s1_negative_test.v1" => inspect_self_hashed_artifact::<NegativeTestReport>(
            value,
            schema,
            |artifact| artifact.negative_self_hash,
            NegativeTestReport::computed_self_hash,
            None,
        ),
        "s1_ablation.v1" => inspect_self_hashed_artifact::<AblationReport>(
            value,
            schema,
            |artifact| artifact.ablation_self_hash,
            AblationReport::computed_self_hash,
            None,
        ),
        "s1_baseline.v1" => inspect_self_hashed_artifact::<BaselineReport>(
            value,
            schema,
            |artifact| artifact.baseline_self_hash,
            BaselineReport::computed_self_hash,
            None,
        ),
        "s1_oracle.v1" => inspect_self_hashed_artifact::<OracleReport>(
            value,
            schema,
            |artifact| artifact.oracle_self_hash,
            OracleReport::computed_self_hash,
            None,
        ),
        "s1_report.v1" => inspect_self_hashed_artifact::<ReportFrontMatter>(
            value,
            schema,
            |artifact| artifact.report_self_hash,
            ReportFrontMatter::computed_self_hash,
            Some(json!({
                "note": "JSON front-matter inspection uses the front-matter-only schema hash; full markdown reports must be verified through the report emitter self-hash contract."
            })),
        ),
        _ => Err(S1CliError::InvalidArtifact(format!(
            "unsupported S1 artifact schema {schema:?}"
        ))),
    }
}

fn inspect_self_hashed_artifact<T>(
    value: Value,
    schema: &str,
    stored_hash: fn(&T) -> Hash256,
    computed_hash: fn(&T) -> Result<Hash256, crate::s1::schema::S1SchemaError>,
    extra: Option<Value>,
) -> Result<ArtifactInspection, S1CliError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let artifact: T = serde_json::from_value(value)?;
    let stored_self_hash = stored_hash(&artifact);
    let computed_self_hash = computed_hash(&artifact)?;
    let artifact = serde_json::to_value(&artifact)?;
    Ok(ArtifactInspection {
        schema: schema.to_owned(),
        stored_self_hash,
        computed_self_hash,
        self_hash_ok: stored_self_hash == computed_self_hash,
        extra,
        artifact,
    })
}

fn print_inspection(inspection: &ArtifactInspection) -> Result<(), S1CliError> {
    println!("schema: {}", inspection.schema);
    println!("stored_self_hash: {}", inspection.stored_self_hash);
    println!("computed_self_hash: {}", inspection.computed_self_hash);
    println!("self_hash_ok: {}", inspection.self_hash_ok);
    if let Some(extra) = &inspection.extra {
        println!("{}", serde_json::to_string_pretty(extra)?);
    }
    println!("{}", serde_json::to_string_pretty(&inspection.artifact)?);
    Ok(())
}

fn read_canonical_tensors(path: &Path) -> Result<Vec<CanonicalTensor>, S1CliError> {
    let bytes = fs::read(path)?;
    read_canonical_tensors_from_bytes(&bytes)
}

fn read_checkpoint_tensors_and_sha(
    path: &Path,
) -> Result<(Hash256, Vec<CanonicalTensor>), S1CliError> {
    let bytes = fs::read(path)?;
    let checkpoint_sha = sha256(&bytes);
    Ok((checkpoint_sha, read_canonical_tensors_from_bytes(&bytes)?))
}

fn read_canonical_tensors_from_bytes(bytes: &[u8]) -> Result<Vec<CanonicalTensor>, S1CliError> {
    let safetensors = SafeTensors::deserialize(bytes)?;
    let mut tensors = Vec::with_capacity(safetensors.len());
    for (name, view) in safetensors.iter() {
        tensors.push(CanonicalTensor::new(
            ArtifactPath::new(name)?,
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(view.shape())?,
                checkpoint_element_type(view.dtype())?,
            ),
            checkpoint_payload(view.dtype(), view.data())?,
        )?);
    }
    Ok(tensors)
}

fn checkpoint_element_type(dtype: Dtype) -> Result<TensorElementType, S1CliError> {
    match dtype {
        Dtype::F32 => Ok(TensorElementType::Float32),
        Dtype::I8 => Ok(TensorElementType::TernaryI2),
        Dtype::U16 => Ok(TensorElementType::Q8_8),
        _ => Err(S1CliError::UnsupportedCheckpointDtype(format!("{dtype:?}"))),
    }
}

fn checkpoint_payload(dtype: Dtype, data: &[u8]) -> Result<CanonicalTensorPayload, S1CliError> {
    match dtype {
        Dtype::F32 => Ok(CanonicalTensorPayload::F32(
            data.chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunk size is 4")))
                .collect(),
        )),
        Dtype::I8 => Ok(CanonicalTensorPayload::I8(
            data.iter().copied().map(|byte| byte as i8).collect(),
        )),
        Dtype::U16 => {
            let chunks = data.chunks_exact(2);
            if !chunks.remainder().is_empty() {
                return Err(S1CliError::InvalidTensorPayload(
                    "U16 tensor payload length is not divisible by 2".to_owned(),
                ));
            }
            Ok(CanonicalTensorPayload::U16(
                chunks
                    .map(|chunk| u16::from_le_bytes(chunk.try_into().expect("chunk size is 2")))
                    .collect(),
            ))
        }
        _ => Err(S1CliError::UnsupportedCheckpointDtype(format!("{dtype:?}"))),
    }
}

fn checkpoint_diff(
    a_tensors: Vec<CanonicalTensor>,
    b_tensors: Vec<CanonicalTensor>,
) -> CheckpointDiffSummary {
    let names = a_tensors
        .iter()
        .chain(&b_tensors)
        .map(|tensor| tensor.id.to_string())
        .collect::<BTreeSet<_>>();
    let tensor_diffs = names
        .into_iter()
        .map(|name| {
            let a = a_tensors.iter().find(|tensor| tensor.id.as_str() == name);
            let b = b_tensors.iter().find(|tensor| tensor.id.as_str() == name);
            checkpoint_tensor_diff(name, a, b)
        })
        .collect::<Vec<_>>();
    let equal = tensor_diffs.iter().all(|diff| diff.equal);
    CheckpointDiffSummary {
        equal,
        a_tensor_payload_hash: canonical_tensor_payload_hash(&a_tensors).to_string(),
        b_tensor_payload_hash: canonical_tensor_payload_hash(&b_tensors).to_string(),
        tensors: tensor_diffs,
    }
}

fn checkpoint_tensor_diff(
    tensor_name: String,
    a: Option<&CanonicalTensor>,
    b: Option<&CanonicalTensor>,
) -> CheckpointTensorDiff {
    let a_bytes = a.map(tensor_payload_bytes);
    let b_bytes = b.map(tensor_payload_bytes);
    let first_byte_mismatch = match (&a_bytes, &b_bytes) {
        (Some(a_bytes), Some(b_bytes)) if a_bytes != b_bytes => {
            let mismatch = first_byte_mismatch(a_bytes, b_bytes);
            Some(CheckpointByteMismatch {
                offset: mismatch.offset,
                a: mismatch.expected,
                b: mismatch.observed,
            })
        }
        (None, Some(_)) | (Some(_), None) => Some(CheckpointByteMismatch {
            offset: 0,
            a: a_bytes.as_ref().and_then(|bytes| bytes.first().copied()),
            b: b_bytes.as_ref().and_then(|bytes| bytes.first().copied()),
        }),
        _ => None,
    };
    let equal = matches!((a, b), (Some(a), Some(b)) if a.layout == b.layout && a_bytes == b_bytes);
    CheckpointTensorDiff {
        tensor_name,
        a_shape: a.map(|tensor| tensor.layout.shape.dims().to_vec()),
        b_shape: b.map(|tensor| tensor.layout.shape.dims().to_vec()),
        a_dtype: a.map(|tensor| tensor.layout.element_type),
        b_dtype: b.map(|tensor| tensor.layout.element_type),
        equal,
        first_byte_mismatch,
    }
}

fn tensor_payload_bytes(tensor: &CanonicalTensor) -> Vec<u8> {
    match &tensor.payload {
        CanonicalTensorPayload::F32(values) => values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        CanonicalTensorPayload::I8(values) => values.iter().map(|value| *value as u8).collect(),
        CanonicalTensorPayload::U16(values) => values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
    }
}

fn cli_model_config_hash(model_config: &ModelSizeProfile) -> Result<Hash256, S1CliError> {
    Ok(DomainHash::new(
        "gbf-policy",
        "ModelSizeProfile",
        "model_size_profile.v1",
        "1",
    )
    .hash(model_config)?)
}

fn cli_train_config_hash(train_config: &TrainConfig) -> Result<Hash256, S1CliError> {
    Ok(
        DomainHash::new("gbf-experiments", "TrainConfig", "s1_train_config.v1", "1")
            .hash(train_config)?,
    )
}

fn cli_build_config_hash() -> Result<Hash256, S1CliError> {
    Ok(DomainHash::new(
        "gbf-experiments",
        "BuildMetadata",
        "s1_build_metadata.v1",
        "1",
    )
    .hash(&build_metadata())?)
}

fn cli_rust_toolchain_hash() -> Hash256 {
    let metadata = build_metadata();
    sha256(format!(
        "rustc:{version};gbf_experiments:{exp};gbf_train:{train}",
        version = env!("CARGO_PKG_RUST_VERSION"),
        exp = metadata.gbf_experiments_sha,
        train = metadata.gbf_train_sha
    ))
}

fn active_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    if cfg!(feature = "phase-a") {
        features.push("phase-a");
    }
    if cfg!(feature = "ablation") {
        features.push("ablation");
    }
    if cfg!(feature = "falsify") {
        features.push("falsify");
    }
    features
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}

/// Errors returned by the S1 CLI surface.
#[derive(Debug)]
pub enum S1CliError {
    /// Replay pass-version mismatch.
    PassVersionMismatch {
        /// Required version.
        expected: SemVer,
        /// Observed version.
        observed: SemVer,
    },
    /// Unsupported device profile name.
    InvalidDeviceProfile {
        /// Observed value.
        observed: String,
    },
    /// Seed list was empty.
    EmptySeedList,
    /// Seed-list syntax was invalid.
    InvalidSeedList(String),
    /// Seed was outside S1's 0..=4 range.
    InvalidSeed {
        /// Observed seed.
        observed: u64,
    },
    /// A run diverged before producing a completed checkpoint.
    RunDiverged {
        /// Seed that diverged.
        seed: u64,
    },
    /// Two determinism replay attempts differed.
    DeterminismMismatch {
        /// Seed under test.
        seed: u64,
        /// First structure-level mismatch.
        detail: DeterminismMismatchDetail,
    },
    /// A non-canonical integration-fixture budget was requested without opt-in.
    NonCanonicalFixtureBudget {
        /// CLI command.
        command: &'static str,
    },
    /// Fixture scorer flag was not supplied for scorer-smoke helpers.
    FixtureScorerRequired {
        /// CLI command.
        command: &'static str,
    },
    /// Manifest did not pin the validation shuffle hash.
    MissingShufflePin,
    /// Command surface intentionally delegated to a named owner bead.
    Deferred {
        /// CLI command.
        command: &'static str,
        /// Owner bead.
        owner: &'static str,
        /// Details.
        detail: &'static str,
    },
    /// A read-only diagnostic command found one or more failures.
    DiagnosticFailure {
        /// CLI command.
        command: &'static str,
        /// Number of failed checks.
        failed: usize,
    },
    /// S1 artifact schema, hash, or shape was not inspectable.
    InvalidArtifact(String),
    /// Stored and recomputed artifact self-hashes differed.
    SelfHashMismatch {
        /// Artifact schema.
        schema: String,
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// SafeTensors dtype cannot be mapped into the S1 canonical tensor contract.
    UnsupportedCheckpointDtype(String),
    /// SafeTensors payload bytes cannot be mapped into the S1 canonical tensor contract.
    InvalidTensorPayload(String),
    /// File I/O failure.
    Io(std::io::Error),
    /// Manifest failure.
    Corpus(CorpusManifestError),
    /// Device-profile enforcement failure.
    DeviceProfile(DeviceProfileEnforceError),
    /// S1 run failure.
    Run(S1RunError),
    /// Oracle producer failure.
    Oracle(OracleEmitError),
    /// Baseline failure.
    Baseline(BaselineError),
    /// Score failure.
    Score(ScoreError),
    /// Negative-test failure.
    NegativeTest(NegativeTestError),
    /// Ablation failure.
    Ablation(AblationError),
    /// Structured logging failure.
    Logging(LoggingEventError),
    /// Outcome dispatch failure.
    OutcomeDispatch(OutcomeDispatchError),
    /// Report emission failure.
    Report(ReportError),
    /// Schema failure.
    Schema(crate::s1::schema::S1SchemaError),
    /// Tensor construction failure.
    Tensor(gbf_artifact::tensor::CanonicalTensorError),
    /// Artifact path failure.
    ArtifactPath(gbf_artifact::ids::ArtifactPathError),
    /// SafeTensors parsing failure.
    Safetensors(safetensors::SafeTensorError),
    /// JSON serialization failure.
    Json(serde_json::Error),
}

impl fmt::Display for S1CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PassVersionMismatch { expected, observed } => write!(
                f,
                "Rep-5 pass_version mismatch: expected {expected}, observed {observed}"
            ),
            Self::InvalidDeviceProfile { observed } => write!(
                f,
                "S1 replay requires --device-profile S1CpuDeterministic, observed {observed}"
            ),
            Self::EmptySeedList => f.write_str("S1 replay requires a non-empty --seed-list"),
            Self::InvalidSeedList(error) => write!(f, "invalid --seed-list: {error}"),
            Self::InvalidSeed { observed } => {
                write!(
                    f,
                    "S1-Pre-4 failed: seed must be in 0..=4, observed {observed}"
                )
            }
            Self::RunDiverged { seed } => write!(f, "S1 run diverged for seed {seed}"),
            Self::DeterminismMismatch { seed, detail } => {
                write!(f, "S1 determinism check failed for seed {seed}: {detail}")
            }
            Self::NonCanonicalFixtureBudget { command } => write!(
                f,
                "gbf s1 {command} is canonical Production by default; --budget-profile integration-fixture requires --allow-noncanonical-integration-fixture and must not be used for closure artifacts under experiments/S1"
            ),
            Self::FixtureScorerRequired { command } => write!(
                f,
                "gbf s1 {command} currently requires --fixture-uniform-scorer until production checkpoint scoring lands in bd-1ehz; in fixture mode --checkpoint-sha is metadata-only and no checkpoint bytes are loaded"
            ),
            Self::MissingShufflePin => {
                f.write_str("manifest is missing val_shuffle_deadeef_sha256")
            }
            Self::Deferred {
                command,
                owner,
                detail,
            } => write!(f, "gbf s1 {command} is deferred to {owner}: {detail}"),
            Self::DiagnosticFailure { command, failed } => {
                write!(f, "gbf s1 {command} failed {failed} diagnostic check(s)")
            }
            Self::InvalidArtifact(detail) => write!(f, "{detail}"),
            Self::SelfHashMismatch {
                schema,
                expected,
                observed,
            } => write!(
                f,
                "{schema} self-hash mismatch: stored {observed}, recomputed {expected}"
            ),
            Self::UnsupportedCheckpointDtype(dtype) => {
                write!(f, "unsupported checkpoint tensor dtype {dtype}")
            }
            Self::InvalidTensorPayload(detail) => {
                write!(f, "invalid checkpoint tensor payload: {detail}")
            }
            Self::Io(error) => write!(f, "{error}"),
            Self::Corpus(error) => write!(f, "{error}"),
            Self::DeviceProfile(error) => write!(f, "{error}"),
            Self::Run(error) => write!(f, "{error}"),
            Self::Oracle(error) => write!(f, "{error}"),
            Self::Baseline(error) => write!(f, "{error}"),
            Self::Score(error) => write!(f, "{error}"),
            Self::NegativeTest(error) => write!(f, "{error}"),
            Self::Ablation(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
            Self::OutcomeDispatch(error) => write!(f, "{error}"),
            Self::Report(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Tensor(error) => write!(f, "{error}"),
            Self::ArtifactPath(error) => write!(f, "{error}"),
            Self::Safetensors(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S1CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Corpus(error) => Some(error),
            Self::DeviceProfile(error) => Some(error),
            Self::Run(error) => Some(error),
            Self::Oracle(error) => Some(error),
            Self::Baseline(error) => Some(error),
            Self::Score(error) => Some(error),
            Self::NegativeTest(error) => Some(error),
            Self::Ablation(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::OutcomeDispatch(error) => Some(error),
            Self::Report(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Tensor(error) => Some(error),
            Self::ArtifactPath(error) => Some(error),
            Self::Safetensors(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::PassVersionMismatch { .. }
            | Self::InvalidDeviceProfile { .. }
            | Self::EmptySeedList
            | Self::InvalidSeedList(_)
            | Self::InvalidSeed { .. }
            | Self::RunDiverged { .. }
            | Self::DeterminismMismatch { .. }
            | Self::NonCanonicalFixtureBudget { .. }
            | Self::FixtureScorerRequired { .. }
            | Self::MissingShufflePin
            | Self::Deferred { .. }
            | Self::DiagnosticFailure { .. }
            | Self::InvalidArtifact(_)
            | Self::SelfHashMismatch { .. }
            | Self::UnsupportedCheckpointDtype(_)
            | Self::InvalidTensorPayload(_) => None,
        }
    }
}

impl From<std::io::Error> for S1CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CorpusManifestError> for S1CliError {
    fn from(error: CorpusManifestError) -> Self {
        Self::Corpus(error)
    }
}

impl From<DeviceProfileEnforceError> for S1CliError {
    fn from(error: DeviceProfileEnforceError) -> Self {
        Self::DeviceProfile(error)
    }
}

impl From<S1RunError> for S1CliError {
    fn from(error: S1RunError) -> Self {
        Self::Run(error)
    }
}

impl From<OracleEmitError> for S1CliError {
    fn from(error: OracleEmitError) -> Self {
        Self::Oracle(error)
    }
}

impl From<BaselineError> for S1CliError {
    fn from(error: BaselineError) -> Self {
        Self::Baseline(error)
    }
}

impl From<ScoreError> for S1CliError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<NegativeTestError> for S1CliError {
    fn from(error: NegativeTestError) -> Self {
        Self::NegativeTest(error)
    }
}

impl From<AblationError> for S1CliError {
    fn from(error: AblationError) -> Self {
        Self::Ablation(error)
    }
}

impl From<LoggingEventError> for S1CliError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

impl From<OutcomeDispatchError> for S1CliError {
    fn from(error: OutcomeDispatchError) -> Self {
        Self::OutcomeDispatch(error)
    }
}

impl From<ReportError> for S1CliError {
    fn from(error: ReportError) -> Self {
        Self::Report(error)
    }
}

impl From<crate::s1::schema::S1SchemaError> for S1CliError {
    fn from(error: crate::s1::schema::S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<gbf_artifact::tensor::CanonicalTensorError> for S1CliError {
    fn from(error: gbf_artifact::tensor::CanonicalTensorError) -> Self {
        Self::Tensor(error)
    }
}

impl From<gbf_artifact::ids::ArtifactPathError> for S1CliError {
    fn from(error: gbf_artifact::ids::ArtifactPathError) -> Self {
        Self::ArtifactPath(error)
    }
}

impl From<safetensors::SafeTensorError> for S1CliError {
    fn from(error: safetensors::SafeTensorError) -> Self {
        Self::Safetensors(error)
    }
}

impl From<serde_json::Error> for S1CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s1::run::{
        TrainConfig, production_loss_nats_per_byte_for_tensors, s1_train_run_with_environment,
    };
    use crate::s1::schema::{CountsSummary, GradNormSummary, SmoothingScheme};
    use crate::s1::score::reset_context_bpc;
    use proptest::prelude::*;

    #[test]
    fn determinism_mismatch_reports_safetensors_first_byte() {
        let first = completed_fixture_run();
        let mut second = first.clone();
        let RunProduct::Completed(product) = &mut second else {
            panic!("fixture run should complete");
        };
        product.final_checkpoint[3] ^= 0xFF;

        let error = assert_deterministic_products(0, &first, &second)
            .expect_err("checkpoint byte mismatch");
        assert!(
            error
                .to_string()
                .contains("structure=safetensors_bytes byte_offset=3"),
            "{error}"
        );
        assert!(error.to_string().contains("expected=0x"), "{error}");
        assert!(error.to_string().contains("observed=0x"), "{error}");
    }

    #[test]
    fn determinism_mismatch_reports_run_log_self_hash() {
        let first = completed_fixture_run();
        let mut second = first.clone();
        let RunProduct::Completed(product) = &mut second else {
            panic!("fixture run should complete");
        };
        product.run_log.run_log_self_hash = Hash256::ZERO;

        let error =
            assert_deterministic_products(0, &first, &second).expect_err("run-log hash mismatch");
        assert!(
            error.to_string().contains("structure=run_log_self_hash"),
            "{error}"
        );
        assert!(error.to_string().contains("expected=sha256:"), "{error}");
        assert!(error.to_string().contains("observed=sha256:"), "{error}");
    }

    #[test]
    fn determinism_mismatch_reports_checkpoint_metadata() {
        let first = completed_fixture_run();
        let mut second = first.clone();
        let RunProduct::Completed(product) = &mut second else {
            panic!("fixture run should complete");
        };
        product.metadata.final_step += 1;

        let error = assert_deterministic_products(0, &first, &second)
            .expect_err("checkpoint metadata mismatch");
        assert!(
            error.to_string().contains("structure=checkpoint_metadata"),
            "{error}"
        );
    }

    #[test]
    fn fixture_dispatch_derives_h3_from_negative_sensitive_artifact() {
        let input = fixture_dispatch_input_from_artifacts(
            CliE2eScenario::PassClean,
            &baseline_report(2.3),
            &oracle_report(true),
            &negative_report(false),
            &ablation_report(true),
            &[],
            &[],
            &[Some(score_report(1.8))],
        )
        .expect("dispatch input");

        assert_eq!(input.h3, HypothesisStatus::Refuted);
    }

    #[test]
    fn fixture_dispatch_derives_h2_from_score_artifacts_for_pass_scenario() {
        let input = fixture_dispatch_input_from_artifacts(
            CliE2eScenario::PassClean,
            &baseline_report(2.3),
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &[],
            &[],
            &[Some(score_report(2.27))],
        )
        .expect("dispatch input");

        assert_eq!(input.h2, HypothesisStatus::Refuted);

        let observation = h2_observation(&baseline_report(2.3), &[Some(score_report(2.27))]);
        assert!(observation.contains("best val_bpc 2.270000"));
        assert!(observation.contains("baseline bpc_3gram 2.300000"));
    }

    #[test]
    fn fixture_dispatch_derives_h1_zero_gradient_from_run_log_artifact() {
        let input = fixture_dispatch_input_from_artifacts(
            CliE2eScenario::PassClean,
            &baseline_report(2.3),
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &[Some(
                checkpoint_metadata(S1BuildKind::PhaseA).expect("metadata"),
            )],
            &[Some(run_log_with_grad_norms(0.0, 0.0, 0.0))],
            &[Some(score_report(1.8))],
        )
        .expect("dispatch input");

        assert_eq!(input.h1, HypothesisStatus::Refuted);

        let observation = h1_observation(
            CliE2eScenario::PassClean,
            &[Some(
                checkpoint_metadata(S1BuildKind::PhaseA).expect("metadata"),
            )],
            &[Some(run_log_with_grad_norms(0.0, 0.0, 0.0))],
        );
        assert!(observation.contains("zero_grad"));
    }

    #[test]
    fn production_dispatch_refutes_h2_when_best_seed_passes_but_another_seed_fails() {
        let mut baseline = baseline_report(2.3);
        baseline.bpc_unigram = 4.0;
        let scores = score_set([2.20, 2.24, 2.249, 2.10, 2.30]);
        let input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &h1_passing_run_logs(),
            &scores,
        )
        .expect("production dispatch input");

        assert_eq!(input.h1, HypothesisStatus::Confirmed);
        assert_eq!(input.h2, HypothesisStatus::Refuted);
        assert_eq!(input.h3, HypothesisStatus::Confirmed);
        assert_eq!(input.suspicious_low_bpc, false);

        let observation = production_h2_observation(&baseline, &scores);
        assert!(observation.contains("seed 4"));
        assert!(observation.contains("threshold 2.250000"));
        assert!(observation.contains("best=2.100000"));
        assert!(observation.contains("median=2.240000"));
        assert!(observation.contains("worst=2.300000"));
    }

    #[test]
    fn production_report_names_h2_falsification_and_sanity_range_surprises() {
        let mut baseline = baseline_report(2.620544);
        baseline.bpc_unigram = 4.450948;
        let scores = score_set([3.111780, 3.081588, 3.115104, 3.148044, 3.120000]);
        let dispatch_input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &h1_passing_run_logs(),
            &scores,
        )
        .expect("production dispatch input");

        assert_eq!(dispatch_input.h2, HypothesisStatus::Refuted);
        assert_eq!(dispatch_input.h3, HypothesisStatus::Confirmed);

        let report = production_report_input(ProductionReportArtifacts {
            baseline: &baseline,
            oracle: &oracle_report(true),
            negative: &negative_report(true),
            ablation: &ablation_report(true),
            metadata: &completed_metadata_set(),
            run_logs: &h1_passing_run_logs(),
            scores: &scores,
            dispatch_input,
            decision: S1Decision::Investigate {
                reason: "propose-Toy1".to_owned(),
            },
            generated_at: "2026-05-09T23:10:50Z".to_owned(),
            rfc_revision: RfcRevisionRef::GitCommitId(fixture_commit('a').expect("commit")),
            predictions_markdown: fixture_predictions().to_owned(),
            predictions_commit: fixture_commit('b').expect("commit"),
            first_result_commit: fixture_commit('c').expect("commit"),
        })
        .expect("production report input");

        assert!(report.falsification_analysis.contains("H2 falsification:"));
        assert!(
            report
                .falsification_analysis
                .contains("failed H2 threshold 2.570544"),
            "{}",
            report.falsification_analysis
        );
        assert!(
            report.surprises.contains(
                "bpc_3gram_baseline 2.620544 was outside preregistered sanity range [1.7, 2.0]"
            ),
            "{}",
            report.surprises
        );
        assert!(
            report.surprises.contains(
                "median(val_bpc) 3.115104 was outside preregistered sanity range [1.4, 1.8]"
            ),
            "{}",
            report.surprises
        );
        assert_eq!(
            report.front_matter.s1_outcome,
            crate::s1::schema::S1Outcome::FailCapacity
        );
    }

    #[test]
    fn production_dispatch_uses_median_for_suspicious_low_bpc() {
        let baseline = baseline_report(2.3);
        let input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &h1_passing_run_logs(),
            &score_set([0.1, 2.0, 2.1, 2.2, 2.3]),
        )
        .expect("production dispatch input");

        assert_eq!(
            input.suspicious_low_bpc, false,
            "one low outlier is not a median-low sentinel"
        );

        let input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &h1_passing_run_logs(),
            &score_set([0.1, 0.2, 0.3, 2.2, 2.3]),
        )
        .expect("production dispatch input");

        assert_eq!(input.suspicious_low_bpc, true);
    }

    #[test]
    fn production_dispatch_refutes_h3_when_sensitive_true_but_score_fails_unigram_threshold() {
        let mut baseline = baseline_report(2.7);
        baseline.bpc_unigram = 3.0;
        let scores = score_set([2.20, 2.25, 2.30, 2.40, 2.60]);
        let input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &h1_passing_run_logs(),
            &scores,
        )
        .expect("production dispatch input");

        assert_eq!(input.h2, HypothesisStatus::Confirmed);
        assert_eq!(input.h3, HypothesisStatus::Refuted);

        let observation = production_h3_observation(&baseline, &negative_report(true), &scores);
        assert!(observation.contains("seed 4"));
        assert!(observation.contains("unigram threshold 2.500000"));
        assert!(observation.contains("negative delta 2.200000 sensitive=true"));
    }

    #[test]
    fn production_dispatch_refutes_h1_on_loss_window_or_grad_checks() {
        let baseline = baseline_report(2.3);
        let mut flat_loss_logs = h1_passing_run_logs();
        flat_loss_logs[3] = Some(run_log_for_h1(3, 5.0, 4.8, 1.0, 1.0, 1.0));
        let input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &flat_loss_logs,
            &score_set([2.0, 2.0, 2.0, 2.0, 2.0]),
        )
        .expect("production dispatch input");

        assert_eq!(input.h1, HypothesisStatus::Refuted);
        assert!(matches!(
            input.h2,
            HypothesisStatus::NotEvaluatedDueToPriorGate(_)
        ));
        assert!(
            production_h1_observation(&completed_metadata_set(), &flat_loss_logs, false)
                .contains("seed 3 failed H1")
        );

        let mut zero_grad_logs = h1_passing_run_logs();
        zero_grad_logs[1] = Some(run_log_for_h1(1, 5.0, 4.0, 0.0, 0.0, 0.0));
        let input = production_dispatch_input_from_artifacts(
            &baseline,
            &oracle_report(true),
            &negative_report(true),
            &ablation_report(true),
            &completed_metadata_set(),
            &zero_grad_logs,
            &score_set([2.0, 2.0, 2.0, 2.0, 2.0]),
        )
        .expect("production dispatch input");

        assert_eq!(input.h1, HypothesisStatus::Refuted);
        assert!(
            production_h1_observation(&completed_metadata_set(), &zero_grad_logs, false)
                .contains("final_grad_norms global=0.000000")
        );
    }

    #[test]
    fn production_checkpoint_scorer_matches_burn_training_forward_for_fixed_tensors() {
        let tensors = fixed_production_tensors();
        let sequence = [3_u8, 7, 3, 11, 5, 19];

        let scorer = ProductionCheckpointScorer::from_tensors(&tensors).expect("production scorer");
        let score_bpc = reset_context_bpc(&scorer, &sequence)
            .expect("score path")
            .bpc;
        let burn_nats =
            production_loss_nats_per_byte_for_tensors(&tensors, &sequence).expect("burn forward");
        let burn_bpc = f64::from(burn_nats) / std::f64::consts::LN_2;

        assert!(
            (score_bpc - burn_bpc).abs() < 1.0e-5,
            "score path bpc={score_bpc:.10} burn path bpc={burn_bpc:.10}"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 32,
            ..ProptestConfig::default()
        })]

        #[test]
        fn checkpoint_diff_keeps_identical_random_u16_tensor_sets_equal(values in proptest::collection::vec(any::<u16>(), 1..=64)) {
            let left = vec![u16_cli_tensor("toy0.random.weight", &values)];
            let right = vec![u16_cli_tensor("toy0.random.weight", &values)];

            let diff = checkpoint_diff(left, right);

            prop_assert!(diff.equal);
            prop_assert_eq!(diff.tensors.len(), 1);
            prop_assert!(diff.tensors[0].first_byte_mismatch.is_none());
            prop_assert_eq!(diff.a_tensor_payload_hash, diff.b_tensor_payload_hash);
        }

        #[test]
        fn checkpoint_diff_detects_random_payload_mutations(
            values in proptest::collection::vec(any::<u16>(), 1..=64),
            index in 0usize..64,
        ) {
            let mutate_at = index % values.len();
            let mut mutated = values.clone();
            mutated[mutate_at] ^= 0x00FF;
            prop_assume!(mutated[mutate_at] != values[mutate_at]);

            let diff = checkpoint_diff(
                vec![u16_cli_tensor("toy0.random.weight", &values)],
                vec![u16_cli_tensor("toy0.random.weight", &mutated)],
            );

            prop_assert!(!diff.equal);
            prop_assert_ne!(diff.a_tensor_payload_hash, diff.b_tensor_payload_hash);
            let mismatch = diff.tensors[0]
                .first_byte_mismatch
                .as_ref()
                .expect("payload mutation reports first byte mismatch");
            prop_assert_eq!(mismatch.offset, mutate_at * 2);
        }
    }

    fn completed_fixture_run() -> RunProduct {
        let product = s1_train_run_with_environment(
            RunInputs {
                corpus_train: patterned_corpus(512),
                corpus_val: patterned_corpus(384),
                model_config: ModelSizeProfile::toy0(),
                train_config: TrainConfig::integration_fixture(),
                seed: 0,
                budget_profile: TrainBudgetProfile::IntegrationFixture,
            },
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("BURN_DETERMINISTIC", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "1"),
            ],
        )
        .expect("fixture run");
        assert!(matches!(product, RunProduct::Completed(_)));
        product
    }

    fn patterned_corpus(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    fn u16_cli_tensor(name: &str, values: &[u16]) -> CanonicalTensor {
        CanonicalTensor::new(
            ArtifactPath::new(name).expect("valid artifact path"),
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[values.len()]).expect("shape"),
                TensorElementType::Q8_8,
            ),
            CanonicalTensorPayload::U16(values.to_vec()),
        )
        .expect("tensor")
    }

    fn fixed_production_tensors() -> Vec<CanonicalTensor> {
        let d_model = usize::from(ModelSizeProfile::toy0().d_model());
        let d_ff = usize::from(ModelSizeProfile::toy0().d_ff());
        vec![
            f32_cli_tensor(
                "toy0.production.embedding_tied.weight",
                &[PRODUCTION_VOCAB_SIZE, d_model],
                production_values(PRODUCTION_VOCAB_SIZE * d_model, 1),
            ),
            f32_cli_tensor(
                "toy0.production.linear_state.input_to_state.weight",
                &[d_model, PRODUCTION_STATE_SLOTS],
                production_values(d_model * PRODUCTION_STATE_SLOTS, 2),
            ),
            f32_cli_tensor(
                "toy0.production.linear_state.state_to_output.weight",
                &[PRODUCTION_STATE_SLOTS, d_model],
                production_values(PRODUCTION_STATE_SLOTS * d_model, 3),
            ),
            f32_cli_tensor(
                "toy0.production.dense_ffn.up.weight",
                &[d_model, d_ff],
                production_values(d_model * d_ff, 4),
            ),
            f32_cli_tensor(
                "toy0.production.dense_ffn.down.weight",
                &[d_ff, d_model],
                production_values(d_ff * d_model, 5),
            ),
        ]
    }

    fn production_values(len: usize, salt: usize) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let value = ((index * 37 + salt * 11) % 29) as f32 - 14.0;
                value / 75.0
            })
            .collect()
    }

    fn f32_cli_tensor(name: &str, shape: &[usize], values: Vec<f32>) -> CanonicalTensor {
        CanonicalTensor::new(
            ArtifactPath::new(name).expect("valid artifact path"),
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(shape).expect("shape"),
                TensorElementType::Float32,
            ),
            CanonicalTensorPayload::F32(values),
        )
        .expect("tensor")
    }

    fn baseline_report(bpc_3gram: f64) -> BaselineReport {
        BaselineReport {
            schema: "s1_baseline.v1".to_owned(),
            corpus_train_sha: Hash256::ZERO,
            corpus_val_sha: Hash256::ZERO,
            smoothing: SmoothingScheme {
                alpha: 0.01,
                lambdas: [0.6, 0.3, 0.1],
            },
            bpc_3gram,
            bpc_2gram: bpc_3gram + 0.1,
            bpc_unigram: bpc_3gram + 0.2,
            counts_summary: CountsSummary {
                train_bytes: 16,
                distinct_unigrams: 4,
                distinct_bigrams: 4,
                distinct_trigrams: 4,
            },
            counts_blob_sha256: Hash256::ZERO,
            baseline_self_hash: Hash256::ZERO,
        }
    }

    fn completed_metadata_set() -> Vec<Option<CheckpointMetadata>> {
        (0..5)
            .map(|seed| {
                let mut metadata =
                    checkpoint_metadata(S1BuildKind::PhaseA).expect("metadata fixture");
                metadata.seed = seed;
                Some(metadata)
            })
            .collect()
    }

    fn h1_passing_run_logs() -> Vec<Option<RunLog>> {
        (0..5)
            .map(|seed| Some(run_log_for_h1(seed, 5.0, 4.0, 1.0, 1.0, 1.0)))
            .collect()
    }

    fn run_log_for_h1(
        seed: u64,
        first_window_loss: f32,
        later_window_loss: f32,
        global_l2: f32,
        max_l2: f32,
        mean_l2: f32,
    ) -> RunLog {
        let mut losses = (1..=100).map(|step| (step, 4.5_f32)).collect::<Vec<_>>();
        for (_, loss) in losses.iter_mut().filter(|(step, _)| *step <= 10) {
            *loss = first_window_loss;
        }
        for (_, loss) in losses
            .iter_mut()
            .filter(|(step, _)| (91..=100).contains(step))
        {
            *loss = later_window_loss;
        }
        RunLog {
            schema: "s1_run_log.v1".to_owned(),
            seed,
            train_config_hash: Hash256::ZERO,
            losses,
            eval_points: vec![(0, 8.0)],
            final_grad_norms: GradNormSummary {
                global_l2,
                max_l2,
                mean_l2,
            },
            run_log_self_hash: Hash256::ZERO,
        }
    }

    fn score_set(bpcs: [f64; 5]) -> Vec<Option<ScoreReport>> {
        bpcs.into_iter()
            .enumerate()
            .map(|(seed, bpc)| {
                let mut score = score_report(bpc);
                score.seed = seed as u64;
                Some(score)
            })
            .collect()
    }

    fn score_report(bpc: f64) -> ScoreReport {
        ScoreReport {
            schema: "s1_score.v1".to_owned(),
            seed: 0,
            checkpoint_sha: Hash256::ZERO,
            corpus_val_sha: Hash256::ZERO,
            chunk_size: 128,
            token_count: 128,
            log2_sum: bpc * 128.0,
            bpc,
            score_self_hash: Hash256::ZERO,
        }
    }

    fn negative_report(sensitive: bool) -> NegativeTestReport {
        NegativeTestReport {
            schema: "s1_negative_test.v1".to_owned(),
            seed: 0,
            checkpoint_sha: Hash256::ZERO,
            corpus_val_sha: Hash256::ZERO,
            shuffle_seed: 0xDEAD_BEEF,
            bpc_original: 1.8,
            bpc_shuffled: if sensitive { 4.0 } else { 1.9 },
            shuffled_val_sha256: Hash256::ZERO,
            delta: if sensitive { 2.2 } else { 0.1 },
            sensitive,
            negative_self_hash: Hash256::ZERO,
        }
    }

    fn ablation_report(phase_a_eq_ablation: bool) -> AblationReport {
        AblationReport {
            schema: "s1_ablation.v1".to_owned(),
            seed: 0,
            phase_a_checkpoint_sha: Hash256::ZERO,
            ablation_checkpoint_sha: Hash256::ZERO,
            phase_a_tensor_payload_sha: Hash256::ZERO,
            ablation_tensor_payload_sha: Hash256::ZERO,
            phase_a_eq_ablation,
            first_mismatch: None,
            ablation_self_hash: Hash256::ZERO,
        }
    }

    fn oracle_report(metric_oracle_passed: bool) -> OracleReport {
        OracleReport::from_oracle_bools(true, true, true, true, metric_oracle_passed)
            .expect("oracle report")
    }

    fn run_log_with_grad_norms(global_l2: f32, max_l2: f32, mean_l2: f32) -> RunLog {
        RunLog {
            schema: "s1_run_log.v1".to_owned(),
            seed: 0,
            train_config_hash: Hash256::ZERO,
            losses: vec![(1, 1.0)],
            eval_points: vec![(0, 8.0)],
            final_grad_norms: GradNormSummary {
                global_l2,
                max_l2,
                mean_l2,
            },
            run_log_self_hash: Hash256::ZERO,
        }
    }
}
