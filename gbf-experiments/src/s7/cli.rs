//! S7 command-line integration surface.
//!
//! The replay command proves the split-feature fixture path; the production
//! bundle command owns the upstream source bundle consumed by F-S7 closure
//! packet assembly.

use std::ffi::OsStr;
use std::fmt;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use clap::{Args, Parser, Subcommand};
use gbf_artifact::S7Topology;
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use serde_json::{Value, json};

#[cfg(feature = "s7-burn-grad-smoke")]
use crate::s7::burn_grad_smoke::{
    S7BurnGradSmokeError, S7BurnGradSmokeInputs, produce_burn_grad_smoke_artifact,
};
use crate::s7::closure_packet::{
    S7ClosurePacketError, ValidateClosurePacketArgs, validate_closure_packet,
};
use crate::s7::comparison::{
    S7ComparisonArtifactInputs, S7ComparisonMaterializeError, materialize_dense_vs_moe_comparison,
};
use crate::s7::emulator_one_token::{
    ArtifactOracleOneTokenTrace, EmulatorOneTokenComparison, EmulatorOneTokenObservation,
    compare_with_artifact_oracle_trace,
};
use crate::s7::frontier::{
    S7FrontierArtifactInputs, S7FrontierMaterializeError, materialize_s7_frontier,
};
use crate::s7::production_runner::{
    S7ProductionBundleInputs, S7ProductionRunnerError, produce_s7_production_bundle,
};
use crate::s7::replay::{
    S7_DETERMINISM_FIXTURE_SCOPE, S7_FULL_CLI_REPLAY_OWNER, S7_FULL_CLOSURE_OWNER, S7ReplayError,
    fixture_replay_product,
};
use crate::s7::run::{
    S7CompletedRunArtifactInputs, S7RunMaterializeError, materialize_completed_run_artifacts,
    topology_path_segment,
};
use crate::s7::schema::{
    EmulatorOneTokenReportError, OracleRouteCoverage, OracleRoutedReport, OracleRoutedReportError,
};
use crate::s7::summaries::{
    S7SummaryArtifactInputs, S7SummaryMaterializeError, materialize_s7_summaries,
};
use crate::s7::support_artifacts::{
    S7SupportArtifactInputs, S7SupportArtifactKind, S7SupportArtifactMaterializeError,
    materialize_support_artifact,
};

const DEFAULT_GUTENBERG_MANIFEST: &str = "fixtures/corpora/gutenberg.toml";
const DEFAULT_CHARSET: &str = "fixtures/charsets/charset_v1.toml";
const DEFAULT_MATCHED_BYTES: &str = "experiments/S7/profile/matched_bytes.json";
const DEFAULT_SWITCH_STATS_SUMMARY: &str = "experiments/S7/summaries/switch-stats-summary.json";
const DEFAULT_SWEEP_SUMMARY: &str = "experiments/S7/summaries/router-collapse-sweep-summary.json";
const DEFAULT_COMPARISON_OUTPUT: &str = "experiments/S7/dense-vs-moe/comparison.json";
const DEFAULT_FRONTIER_OUTPUT: &str = "experiments/S7/frontier/frontier.json";
const DEFAULT_DEVICE_PROFILE: &str = "S7CpuDeterministic";
const DEFAULT_SEED_LIST: &str = "0,1,2,3,4";
const DEFAULT_REPORT_EMITTER: &str = "scripts/review/f-s7/emit-report.py";
const DEFAULT_REPORT_OUTPUT: &str = "docs/experiments/S7-report.md";
const DEFAULT_PRODUCTION_BUNDLE_DIR: &str = "experiments/S7/production-bundle";
const DEFAULT_PRODUCTION_BUNDLE_MANIFEST: &str =
    "experiments/S7/production-bundle/s7-production-bundle-manifest.json";
const DEFAULT_GUTENBERG_TRAIN: &str = "experiments/S4/corpus/gutenberg-train.bin";
const DEFAULT_GUTENBERG_VAL: &str = "experiments/S4/corpus/gutenberg-val.bin";
#[cfg(feature = "s7-burn-grad-smoke")]
const DEFAULT_BURN_GRAD_SMOKE_OUTPUT: &str = "experiments/S7/burn-grad-smoke/expert_block_qat.json";
const DEFAULT_ORACLE_ROUTED_OUTPUT: &str = "experiments/S7/oracle-routed/seed-0/oracle.json";
const DEFAULT_EMULATOR_ONE_TOKEN_BLOCKS: u32 = 4;
const S7_CLI_REPLAY_DOMAIN: DomainHash<'static> =
    DomainHash::new("gbf-experiments", "S7CliReplay", "s7_replay_cli.v1", "1");

/// S7 CLI envelope.
#[derive(Debug, Clone, Parser)]
pub struct S7Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: S7Command,
}

/// S7 subcommands registered by the replay gate.
// Clap derives expect direct args payloads here; command enums are short-lived at dispatch time.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Subcommand)]
pub enum S7Command {
    /// Replay the current deterministic S7 fixture for one split-feature topology.
    Replay(S7ReplayArgs),
    /// Validate externally produced completed-run artifacts and write packet paths.
    MaterializeRun(S7MaterializeRunArgs),
    /// Derive comparison summary inputs from landed production artifacts.
    DeriveSummaries(S7DeriveSummariesArgs),
    /// Derive the dense-vs-MoE comparison from materialized score artifacts.
    DeriveComparison(S7DeriveComparisonArgs),
    /// Derive the S7 Pareto frontier from materialized comparison evidence.
    DeriveFrontier(S7DeriveFrontierArgs),
    /// Validate and materialize a closure support artifact.
    MaterializeSupportArtifact(S7MaterializeSupportArtifactArgs),
    /// Build an H9 routed artifact-oracle report from measured logits.
    OracleRouted(S7OracleRoutedArgs),
    /// Build an H10 one-token emulator/oracle comparison artifact from measured outputs.
    EmulatorOneToken(S7EmulatorOneTokenArgs),
    /// Produce the H8 Burn ExpertBlockQat gradient smoke artifact.
    #[cfg(feature = "s7-burn-grad-smoke")]
    BurnGradSmoke(S7BurnGradSmokeArgs),
    /// Emit the final S7 report from already-produced production artifact JSON.
    EmitReport(S7EmitReportArgs),
    /// Validate the final S7 report against the Rust closure contract.
    ValidateClosure(S7ValidateClosureArgs),
    /// Produce the real S7 source bundle consumed by the closure packet assembler.
    ProduceProductionBundle(S7ProduceProductionBundleArgs),
}

/// Arguments for `gbf s7 replay`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 replay --topology MoeTiny --seed-list 0 --pass-version 0.7.0 --output experiments/S7/replay/moe.json"
)]
pub struct S7ReplayArgs {
    /// Gutenberg manifest path pinned by the final S7 report.
    #[arg(long, default_value = DEFAULT_GUTENBERG_MANIFEST)]
    pub gutenberg_manifest: PathBuf,
    /// Charset manifest path.
    #[arg(long, default_value = DEFAULT_CHARSET)]
    pub charset: PathBuf,
    /// Matched-bytes pin path.
    #[arg(long, default_value = DEFAULT_MATCHED_BYTES)]
    pub matched_bytes: PathBuf,
    /// S7 pass version pinned in the report.
    #[arg(long)]
    pub pass_version: String,
    /// Topology replayed by this split-feature invocation.
    #[arg(long, value_parser = parse_topology)]
    pub topology: S7Topology,
    /// Comma-separated seed list.
    #[arg(long, default_value = DEFAULT_SEED_LIST)]
    pub seed_list: String,
    /// Deterministic device profile.
    #[arg(long, default_value = DEFAULT_DEVICE_PROFILE)]
    pub device_profile: String,
    /// Output `s7_replay_cli.v1` fixture evidence path.
    #[arg(long, default_value = "/tmp/s7-replay-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s7 materialize-run`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 materialize-run --topology MoeTiny --seed 0 --run-log /tmp/run-log.json --score /tmp/score.json --grad-log /tmp/grad-log.jsonl --router-step-telemetry /tmp/router-step-telemetry.jsonl"
)]
pub struct S7MaterializeRunArgs {
    /// Packet/repository root where `experiments/S7/...` should be written.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Topology being materialized.
    #[arg(long, value_parser = parse_topology)]
    pub topology: S7Topology,
    /// Seed being materialized.
    #[arg(long)]
    pub seed: u64,
    /// Completed `s7_run_log.v1` input path.
    #[arg(long)]
    pub run_log: PathBuf,
    /// `s7_score.v1` input path.
    #[arg(long)]
    pub score: PathBuf,
    /// Per-step `s7_grad_log.v1` JSONL input path.
    #[arg(long)]
    pub grad_log: PathBuf,
    /// Router telemetry JSONL input path, empty for dense matched topology.
    #[arg(long)]
    pub router_step_telemetry: PathBuf,
}

/// Arguments for `gbf s7 derive-comparison`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 derive-comparison --root . --moe-topology-hash sha256:... --dense-matched-topology-hash sha256:..."
)]
pub struct S7DeriveComparisonArgs {
    /// Packet/repository root containing materialized S7 score artifacts.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Matched-bytes pin path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_MATCHED_BYTES)]
    pub matched_bytes: PathBuf,
    /// Production MoE topology hash recorded in `s7_dense_vs_moe.v1`.
    #[arg(long)]
    pub moe_topology_hash: Hash256,
    /// Production dense matched topology hash recorded in `s7_dense_vs_moe.v1`.
    #[arg(long)]
    pub dense_matched_topology_hash: Hash256,
    /// Switch-stat summary JSON path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_SWITCH_STATS_SUMMARY)]
    pub switch_stats_summary: PathBuf,
    /// Router-collapse sweep summary JSON path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_SWEEP_SUMMARY)]
    pub sweep_summary: PathBuf,
    /// Output `s7_dense_vs_moe.v1` path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_COMPARISON_OUTPUT)]
    pub output: PathBuf,
}

/// Arguments for `gbf s7 derive-summaries`.
#[derive(Debug, Clone, Args)]
#[command(after_help = "Examples:\n  gbf s7 derive-summaries --root .")]
pub struct S7DeriveSummariesArgs {
    /// Packet/repository root containing materialized S7 support artifacts and telemetry.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Switch-stat summary JSON path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_SWITCH_STATS_SUMMARY)]
    pub switch_stats_output: PathBuf,
    /// Router-collapse sweep summary JSON path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_SWEEP_SUMMARY)]
    pub sweep_output: PathBuf,
}

/// Arguments for `gbf s7 derive-frontier`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 derive-frontier --root . --moe-conformance /tmp/moe-conformance.json --dense-conformance /tmp/dense-conformance.json --moe-deployed-bytes-per-block 20944,20944,20944,20944 --dense-deployed-bytes-per-block 20948,20948,20948,20948"
)]
pub struct S7DeriveFrontierArgs {
    /// Packet/repository root containing materialized S7 comparison and scores.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Materialized `s7_dense_vs_moe.v1` path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_COMPARISON_OUTPUT)]
    pub comparison: PathBuf,
    /// JSON object containing MoE conformance evidence.
    #[arg(long)]
    pub moe_conformance: PathBuf,
    /// JSON object containing dense-matched conformance evidence.
    #[arg(long)]
    pub dense_conformance: PathBuf,
    /// Comma-separated MoE deployed bytes per deployment block.
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub moe_deployed_bytes_per_block: Vec<u64>,
    /// Comma-separated dense-matched deployed bytes per deployment block.
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub dense_deployed_bytes_per_block: Vec<u64>,
    /// Optional JSON value containing MoE schedule-cost evidence.
    #[arg(long)]
    pub moe_schedule_cost: Option<PathBuf>,
    /// Optional JSON value containing dense-matched schedule-cost evidence.
    #[arg(long)]
    pub dense_schedule_cost: Option<PathBuf>,
    /// Output `s7_frontier.v1` path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_FRONTIER_OUTPUT)]
    pub output: PathBuf,
}

/// Arguments for `gbf s7 materialize-support-artifact`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 materialize-support-artifact --kind frontier --input /tmp/frontier.json\n  gbf s7 materialize-support-artifact --kind emulator-one-token --topology MoeTiny --input /tmp/emulator.json"
)]
pub struct S7MaterializeSupportArtifactArgs {
    /// Packet/repository root where `experiments/S7/...` should be written.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Support artifact kind.
    #[arg(long, value_parser = parse_support_artifact_kind)]
    pub kind: S7SupportArtifactKind,
    /// Externally produced support artifact JSON.
    #[arg(long)]
    pub input: PathBuf,
    /// Topology for `emulator-one-token` artifacts.
    #[arg(long, value_parser = parse_topology)]
    pub topology: Option<S7Topology>,
    /// Seed for per-seed support artifacts such as `switch-stats`.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Output path, relative to --root unless absolute. Defaults to the canonical packet path.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// Arguments for `gbf s7 oracle-routed`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 oracle-routed --topology MoeTiny --fixture-prompt-sha sha256:... --train-logits-sha sha256:... --bundle-logits-sha sha256:... --artifact-logits-sha sha256:... --frozen-teacher-checkpoint-sha sha256:... --pairwise-max-abs-diff-train-bundle 0 --pairwise-max-abs-diff-bundle-artifact 0 --pairwise-max-abs-diff-train-artifact 0 --s3-tolerance 0.125 --cross-layer-route-difference --consecutive-token-route-change --consecutive-token-route-same"
)]
pub struct S7OracleRoutedArgs {
    /// Packet/repository root where the H9 report should be written.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Experiment seed; the closure packet currently admits only seed 0.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// S7 topology under routed oracle comparison.
    #[arg(long, value_parser = parse_topology)]
    pub topology: S7Topology,
    /// Fixed routed fixture prompt hash.
    #[arg(long)]
    pub fixture_prompt_sha: Hash256,
    /// Training logits hash for the routed fixture.
    #[arg(long)]
    pub train_logits_sha: Hash256,
    /// Bundle logits hash for the routed fixture.
    #[arg(long)]
    pub bundle_logits_sha: Hash256,
    /// Deployable artifact logits hash for the routed fixture.
    #[arg(long)]
    pub artifact_logits_sha: Hash256,
    /// Frozen teacher checkpoint hash for this topology and seed.
    #[arg(long)]
    pub frozen_teacher_checkpoint_sha: Hash256,
    /// Pairwise max absolute difference between training and bundle logits.
    #[arg(long)]
    pub pairwise_max_abs_diff_train_bundle: f64,
    /// Pairwise max absolute difference between bundle and artifact logits.
    #[arg(long)]
    pub pairwise_max_abs_diff_bundle_artifact: f64,
    /// Pairwise max absolute difference between training and artifact logits.
    #[arg(long)]
    pub pairwise_max_abs_diff_train_artifact: f64,
    /// S3 pinned routed-oracle tolerance.
    #[arg(long)]
    pub s3_tolerance: f64,
    /// Prove at least one prompt route differs across layers.
    #[arg(long)]
    pub cross_layer_route_difference: bool,
    /// Prove at least one consecutive token route changes expert.
    #[arg(long)]
    pub consecutive_token_route_change: bool,
    /// Prove at least one consecutive token route stays on the same expert.
    #[arg(long)]
    pub consecutive_token_route_same: bool,
    /// Output `s7_oracle_routed.v1` path, relative to --root unless absolute.
    /// Defaults to the canonical packet path.
    #[arg(long, default_value = DEFAULT_ORACLE_ROUTED_OUTPUT)]
    pub output: PathBuf,
}

/// Arguments for `gbf s7 emulator-one-token`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 emulator-one-token --topology MoeTiny --encoded-rom-sha sha256:... --prompt-sha sha256:... --artifact-oracle-logits-sha sha256:... --emulator-logits-sha sha256:... --pairwise-max-abs-diff 0 --s5-tolerance 0.125 --observed-bank-switches-per-token 0.5 --oracle-recorded-bank-switches 0.5"
)]
pub struct S7EmulatorOneTokenArgs {
    /// Packet/repository root where the H10 report should be written.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Experiment seed; the closure packet currently admits only seed 0.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// S7 topology under emulator/oracle comparison.
    #[arg(long, value_parser = parse_topology)]
    pub topology: S7Topology,
    /// Encoded ROM hash loaded by the emulator.
    #[arg(long)]
    pub encoded_rom_sha: Hash256,
    /// Fixed H10 prompt hash.
    #[arg(long)]
    pub prompt_sha: Hash256,
    /// Artifact-oracle logits hash for the same prompt.
    #[arg(long)]
    pub artifact_oracle_logits_sha: Hash256,
    /// Emulator logits hash for the same prompt.
    #[arg(long)]
    pub emulator_logits_sha: Hash256,
    /// Pairwise max absolute logit difference.
    #[arg(long)]
    pub pairwise_max_abs_diff: f64,
    /// S5 pinned one-token tolerance.
    #[arg(long)]
    pub s5_tolerance: f64,
    /// Bank switches per token observed in the emulator.
    #[arg(long)]
    pub observed_bank_switches_per_token: f32,
    /// Bank switches per token recorded by the artifact-oracle route tracer.
    #[arg(long)]
    pub oracle_recorded_bank_switches: f32,
    /// Number of deployable blocks used for bank-switch upper-bound validation.
    #[arg(long, default_value_t = DEFAULT_EMULATOR_ONE_TOKEN_BLOCKS)]
    pub n_blocks: u32,
    /// Output `s7_emulator_one_token.v1` path, relative to --root unless absolute.
    /// Defaults to the canonical packet path for the selected topology.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// Arguments for `gbf s7 burn-grad-smoke`.
#[cfg(feature = "s7-burn-grad-smoke")]
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 burn-grad-smoke --root . --output experiments/S7/burn-grad-smoke/expert_block_qat.json"
)]
pub struct S7BurnGradSmokeArgs {
    /// Packet/repository root where the H8 smoke artifact should be written.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Output `s7_burn_grad_smoke.v1` path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_BURN_GRAD_SMOKE_OUTPUT)]
    pub output: PathBuf,
}

/// Arguments for `gbf s7 emit-report`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 emit-report --predictions-section-hash sha256:... --predictions-commit <commit> --first-result-commit <commit> --output docs/experiments/S7-report.md"
)]
pub struct S7EmitReportArgs {
    /// Packet/repository root containing experiments/S7 artifacts.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// Fail-closed report emitter script.
    #[arg(long, default_value = DEFAULT_REPORT_EMITTER)]
    pub emitter_script: PathBuf,
    /// Python executable used to run the emitter.
    #[arg(long, default_value = "python3")]
    pub python: PathBuf,
    /// Output s7_report.v1 markdown path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_REPORT_OUTPUT)]
    pub output: PathBuf,
    /// S7 outcome selected by the closure packet.
    #[arg(long, default_value = "PassClean", value_parser = ["PassClean", "FailParity"])]
    pub s7_outcome: String,
    /// Decision tag. Defaults from --s7-outcome.
    #[arg(long, value_parser = ["ProceedToS8", "ProceedToS8DenseOnly"])]
    pub decision: Option<String>,
    /// RFC revision commit/hash pinned into report front matter.
    #[arg(long)]
    pub rfc_revision: Option<String>,
    /// Hash of the pre-registered predictions section.
    #[arg(long)]
    pub predictions_section_hash: String,
    /// Commit containing pre-registered predictions.
    #[arg(long)]
    pub predictions_commit: String,
    /// First commit introducing S7 result evidence.
    #[arg(long)]
    pub first_result_commit: String,
    /// RFC3339 UTC timestamp recorded as hash-excluded generated_at.
    #[arg(long)]
    pub generated_at: Option<String>,
}

/// Arguments for `gbf s7 validate-closure`.
#[derive(Debug, Clone, Args)]
#[command(after_help = "Examples:\n  gbf s7 validate-closure --root . --predictions-verified")]
pub struct S7ValidateClosureArgs {
    /// Packet/repository root containing docs/experiments/S7-report.md and artifacts.
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
    /// s7_report.v1 markdown path, relative to --root unless absolute.
    #[arg(long, default_value = DEFAULT_REPORT_OUTPUT)]
    pub report: PathBuf,
    /// Set only after scripts/s7_preregistration_check.sh has passed for this packet.
    #[arg(long)]
    pub predictions_verified: bool,
}

/// Arguments for `gbf s7 produce-production-bundle`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s7 produce-production-bundle --burn-grad-smoke /tmp/h8.json --oracle-routed /tmp/h9.json --emulator-one-token-moe /tmp/h10-moe.json --moe-conformance /tmp/moe-conformance.json --dense-conformance /tmp/dense-conformance.json --predictions-section-hash sha256:... --predictions-commit <commit> --first-result-commit <commit> --rfc-revision <commit> --generated-at 2026-07-01T00:00:00Z"
)]
pub struct S7ProduceProductionBundleArgs {
    /// Directory where source bundle artifacts are written.
    #[arg(long, default_value = DEFAULT_PRODUCTION_BUNDLE_DIR)]
    pub output_dir: PathBuf,
    /// Manifest path to write.
    #[arg(long, default_value = DEFAULT_PRODUCTION_BUNDLE_MANIFEST)]
    pub manifest_output: PathBuf,
    /// Pinned Project Gutenberg fixture/manifest path.
    #[arg(long, default_value = DEFAULT_GUTENBERG_MANIFEST)]
    pub gutenberg_manifest: PathBuf,
    /// Gutenberg training byte stream produced by S4.
    #[arg(long, default_value = DEFAULT_GUTENBERG_TRAIN)]
    pub train_corpus: PathBuf,
    /// Gutenberg validation byte stream produced by S4.
    #[arg(long, default_value = DEFAULT_GUTENBERG_VAL)]
    pub val_corpus: PathBuf,
    /// Real H8 Burn gradient smoke artifact.
    #[arg(long)]
    pub burn_grad_smoke: PathBuf,
    /// Real H9 routed oracle artifact.
    #[arg(long)]
    pub oracle_routed: PathBuf,
    /// Real H10 MoE emulator one-token artifact.
    #[arg(long)]
    pub emulator_one_token_moe: PathBuf,
    /// Optional real H10 dense emulator one-token artifact.
    #[arg(long)]
    pub emulator_one_token_dense: Option<PathBuf>,
    /// MoE conformance evidence used by frontier derivation.
    #[arg(long)]
    pub moe_conformance: PathBuf,
    /// Dense conformance evidence used by frontier derivation.
    #[arg(long)]
    pub dense_conformance: PathBuf,
    /// Optional MoE schedule-cost evidence used by frontier derivation.
    #[arg(long)]
    pub moe_schedule_cost: Option<PathBuf>,
    /// Optional dense schedule-cost evidence used by frontier derivation.
    #[arg(long)]
    pub dense_schedule_cost: Option<PathBuf>,
    /// S7 report outcome.
    #[arg(long, default_value = "PassClean", value_parser = ["PassClean", "FailParity"])]
    pub s7_outcome: String,
    /// Report decision matching the outcome.
    #[arg(long, default_value = "ProceedToS8", value_parser = ["ProceedToS8", "ProceedToS8DenseOnly"])]
    pub decision: String,
    /// RFC revision commit/hash pinned into the final report.
    #[arg(long)]
    pub rfc_revision: String,
    /// Hash of the pre-registered predictions section.
    #[arg(long)]
    pub predictions_section_hash: String,
    /// Commit containing pre-registered predictions.
    #[arg(long)]
    pub predictions_commit: String,
    /// First commit introducing S7 result evidence.
    #[arg(long)]
    pub first_result_commit: String,
    /// RFC3339 UTC timestamp recorded as generated_at in the report.
    #[arg(long)]
    pub generated_at: String,
}

/// Run an S7 CLI command.
pub fn run(cli: S7Cli) -> Result<(), S7CliError> {
    match cli.command {
        S7Command::Replay(args) => replay(args),
        S7Command::MaterializeRun(args) => materialize_run(args),
        S7Command::DeriveSummaries(args) => derive_summaries(args),
        S7Command::DeriveComparison(args) => derive_comparison(args),
        S7Command::DeriveFrontier(args) => derive_frontier(args),
        S7Command::MaterializeSupportArtifact(args) => materialize_support(args),
        S7Command::OracleRouted(args) => oracle_routed(args),
        S7Command::EmulatorOneToken(args) => emulator_one_token(args),
        #[cfg(feature = "s7-burn-grad-smoke")]
        S7Command::BurnGradSmoke(args) => burn_grad_smoke(args),
        S7Command::EmitReport(args) => emit_report(args),
        S7Command::ValidateClosure(args) => validate_closure(args),
        S7Command::ProduceProductionBundle(args) => produce_production_bundle(args),
    }
}

fn replay(args: S7ReplayArgs) -> Result<(), S7CliError> {
    validate_topology_feature(&args.topology)?;
    let seeds = parse_seed_list(&args.seed_list)?;
    let mut runs = Vec::new();
    for seed in seeds {
        let product = fixture_replay_product(args.topology.clone(), seed)?;
        runs.push(json!({
            "seed": seed,
            "topology": topology_name(&args.topology),
            "checkpoint_sha": product.checkpoint_sha,
            "run_log_self_hash": product.run_log_self_hash,
            "score_self_hash": product.score_self_hash,
            "combined_replay_hash": product.combined_hash(),
        }));
    }

    let mut evidence = json!({
        "schema": "s7_replay_cli.v1",
        "artifact_self_hash": Hash256::ZERO,
        "status": "fixture_replayed",
        "support_scope": S7_DETERMINISM_FIXTURE_SCOPE,
        "moved_full_cli_replay_to": S7_FULL_CLI_REPLAY_OWNER,
        "moved_full_closure_to": S7_FULL_CLOSURE_OWNER,
        "command": "replay",
        "feature_gate": active_feature_gate(),
        "gutenberg_manifest": args.gutenberg_manifest,
        "charset": args.charset,
        "matched_bytes": args.matched_bytes,
        "pass_version_S7": args.pass_version,
        "topology": topology_name(&args.topology),
        "seed_list": args.seed_list,
        "device_profile": args.device_profile,
        "stdout": "artifact_self_hash",
        "runs": runs,
    });
    let artifact_self_hash = gbf_foundation::self_hash_omitting_fields(
        S7_CLI_REPLAY_DOMAIN,
        &evidence,
        "artifact_self_hash",
        &[],
    )?;
    evidence
        .as_object_mut()
        .expect("S7 replay evidence is a JSON object")
        .insert(
            "artifact_self_hash".to_owned(),
            Value::String(artifact_self_hash.to_string()),
        );

    if let Some(parent) = args
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S7CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let bytes = CanonicalJson::value_to_vec(&evidence)?;
    std::fs::write(&args.output, bytes).map_err(|source| S7CliError::Io {
        path: args.output.display().to_string(),
        source,
    })?;
    println!("{artifact_self_hash}");
    Ok(())
}

fn materialize_run(args: S7MaterializeRunArgs) -> Result<(), S7CliError> {
    let materialized = materialize_completed_run_artifacts(&S7CompletedRunArtifactInputs {
        root: args.root,
        topology: args.topology,
        seed: args.seed,
        run_log: args.run_log,
        score: args.score,
        grad_log: args.grad_log,
        router_step_telemetry: args.router_step_telemetry,
    })?;
    println!("{}", materialized.run_log_self_hash);
    Ok(())
}

fn derive_summaries(args: S7DeriveSummariesArgs) -> Result<(), S7CliError> {
    let materialized = materialize_s7_summaries(&S7SummaryArtifactInputs {
        root: args.root,
        switch_stats_output: args.switch_stats_output,
        sweep_output: args.sweep_output,
    })?;
    println!("{}", materialized.summary_bundle_hash);
    Ok(())
}

fn derive_comparison(args: S7DeriveComparisonArgs) -> Result<(), S7CliError> {
    let materialized = materialize_dense_vs_moe_comparison(&S7ComparisonArtifactInputs {
        root: args.root,
        matched_bytes: args.matched_bytes,
        moe_topology_hash: args.moe_topology_hash,
        dense_matched_topology_hash: args.dense_matched_topology_hash,
        switch_stats_summary: args.switch_stats_summary,
        sweep_summary: args.sweep_summary,
        output: args.output,
    })?;
    println!("{}", materialized.comparison_self_hash);
    Ok(())
}

fn derive_frontier(args: S7DeriveFrontierArgs) -> Result<(), S7CliError> {
    let materialized = materialize_s7_frontier(&S7FrontierArtifactInputs {
        root: args.root,
        comparison: args.comparison,
        moe_conformance: args.moe_conformance,
        dense_conformance: args.dense_conformance,
        moe_deployed_bytes_per_block: args.moe_deployed_bytes_per_block,
        dense_deployed_bytes_per_block: args.dense_deployed_bytes_per_block,
        moe_schedule_cost: args.moe_schedule_cost,
        dense_schedule_cost: args.dense_schedule_cost,
        output: args.output,
    })?;
    println!("{}", materialized.frontier_self_hash);
    Ok(())
}

fn materialize_support(args: S7MaterializeSupportArtifactArgs) -> Result<(), S7CliError> {
    let materialized = materialize_support_artifact(&S7SupportArtifactInputs {
        root: args.root,
        kind: args.kind,
        input: args.input,
        topology: args.topology,
        seed: args.seed,
        output: args.output,
    })?;
    println!("{}", materialized.self_hash);
    Ok(())
}

fn oracle_routed(args: S7OracleRoutedArgs) -> Result<(), S7CliError> {
    let report = OracleRoutedReport::new(
        args.seed,
        args.topology,
        args.fixture_prompt_sha,
        args.train_logits_sha,
        args.bundle_logits_sha,
        args.artifact_logits_sha,
        args.frozen_teacher_checkpoint_sha,
        args.pairwise_max_abs_diff_train_bundle,
        args.pairwise_max_abs_diff_bundle_artifact,
        args.pairwise_max_abs_diff_train_artifact,
        args.s3_tolerance,
        OracleRouteCoverage::new(
            args.cross_layer_route_difference,
            args.consecutive_token_route_change,
            args.consecutive_token_route_same,
        ),
    )?;
    let output_path = if args.output.is_absolute() {
        args.output
    } else {
        args.root.join(args.output)
    };
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S7CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(&output_path, report.canonical_json_bytes()?).map_err(|source| {
        S7CliError::Io {
            path: output_path.display().to_string(),
            source,
        }
    })?;
    println!("{}", report.oracle_self_hash);
    Ok(())
}

fn emulator_one_token(args: S7EmulatorOneTokenArgs) -> Result<(), S7CliError> {
    if args.seed != 0 {
        return Err(S7CliError::InvalidEmulatorOneTokenSeed { seed: args.seed });
    }
    let output = args.output.clone().unwrap_or_else(|| {
        PathBuf::from("experiments/S7/emulator-one-token/seed-0")
            .join(topology_path_segment(&args.topology))
            .join("result.json")
    });
    let report = compare_with_artifact_oracle_trace(EmulatorOneTokenComparison {
        seed: args.seed,
        topology: args.topology,
        encoded_rom_sha: args.encoded_rom_sha,
        prompt_sha: args.prompt_sha,
        artifact_oracle_trace: ArtifactOracleOneTokenTrace {
            logits_sha: args.artifact_oracle_logits_sha,
            bank_switches_per_token: args.oracle_recorded_bank_switches,
        },
        emulator_observation: EmulatorOneTokenObservation {
            logits_sha: args.emulator_logits_sha,
            bank_switches_per_token: args.observed_bank_switches_per_token,
        },
        pairwise_max_abs_diff: args.pairwise_max_abs_diff,
        s5_tolerance: args.s5_tolerance,
        n_blocks: args.n_blocks,
    })?;
    let output_path = if output.is_absolute() {
        output
    } else {
        args.root.join(output)
    };
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S7CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(&output_path, report.canonical_json_bytes()?).map_err(|source| {
        S7CliError::Io {
            path: output_path.display().to_string(),
            source,
        }
    })?;
    println!("{}", report.emulator_self_hash);
    Ok(())
}

#[cfg(feature = "s7-burn-grad-smoke")]
fn burn_grad_smoke(args: S7BurnGradSmokeArgs) -> Result<(), S7CliError> {
    let artifact = produce_burn_grad_smoke_artifact(&S7BurnGradSmokeInputs {
        root: args.root,
        output: args.output,
    })?;
    println!("{}", artifact.smoke_self_hash);
    Ok(())
}

fn emit_report(args: S7EmitReportArgs) -> Result<(), S7CliError> {
    if args.output.as_os_str() == OsStr::new("-") {
        return Err(S7CliError::UnsupportedStdoutReport);
    }
    let mut command = ProcessCommand::new(&args.python);
    command
        .arg(&args.emitter_script)
        .arg("--root")
        .arg(&args.root)
        .arg("--output")
        .arg(&args.output)
        .arg("--s7-outcome")
        .arg(&args.s7_outcome)
        .arg("--predictions-section-hash")
        .arg(&args.predictions_section_hash)
        .arg("--predictions-commit")
        .arg(&args.predictions_commit)
        .arg("--first-result-commit")
        .arg(&args.first_result_commit);
    if let Some(decision) = &args.decision {
        command.arg("--decision").arg(decision);
    }
    if let Some(rfc_revision) = &args.rfc_revision {
        command.arg("--rfc-revision").arg(rfc_revision);
    }
    if let Some(generated_at) = &args.generated_at {
        command.arg("--generated-at").arg(generated_at);
    }

    let output = command.output().map_err(|source| S7CliError::Io {
        path: args.python.display().to_string(),
        source,
    })?;
    if !output.status.success() {
        return Err(S7CliError::ReportEmitterFailed {
            status: output
                .status
                .code()
                .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let report_path = if args.output.is_absolute() {
        args.output.clone()
    } else {
        args.root.join(&args.output)
    };
    let report_text = std::fs::read_to_string(&report_path).map_err(|source| S7CliError::Io {
        path: report_path.display().to_string(),
        source,
    })?;
    let report_self_hash = report_self_hash_from_markdown(&report_text).ok_or_else(|| {
        S7CliError::InvalidReportSelfHash {
            path: report_path.display().to_string(),
        }
    })?;
    println!("{report_self_hash}");
    Ok(())
}

fn validate_closure(args: S7ValidateClosureArgs) -> Result<(), S7CliError> {
    let report_self_hash = validate_closure_packet(ValidateClosurePacketArgs {
        root: args.root,
        report: args.report,
        predictions_verified: args.predictions_verified,
    })?;
    println!("{report_self_hash}");
    Ok(())
}

fn produce_production_bundle(args: S7ProduceProductionBundleArgs) -> Result<(), S7CliError> {
    let output = produce_s7_production_bundle(&S7ProductionBundleInputs {
        output_dir: args.output_dir,
        manifest_output: args.manifest_output,
        gutenberg_manifest: args.gutenberg_manifest,
        train_corpus: args.train_corpus,
        val_corpus: args.val_corpus,
        burn_grad_smoke: args.burn_grad_smoke,
        oracle_routed: args.oracle_routed,
        emulator_one_token_moe: args.emulator_one_token_moe,
        emulator_one_token_dense: args.emulator_one_token_dense,
        moe_conformance: args.moe_conformance,
        dense_conformance: args.dense_conformance,
        moe_schedule_cost: args.moe_schedule_cost,
        dense_schedule_cost: args.dense_schedule_cost,
        s7_outcome: args.s7_outcome,
        decision: args.decision,
        rfc_revision: args.rfc_revision,
        predictions_section_hash: args.predictions_section_hash,
        predictions_commit: args.predictions_commit,
        first_result_commit: args.first_result_commit,
        generated_at: args.generated_at,
    })?;
    println!("{}", output.manifest_self_hash);
    Ok(())
}

fn parse_topology(value: &str) -> Result<S7Topology, String> {
    match value {
        "MoeTiny" => Ok(S7Topology::MoeTiny),
        "MoeTinyDenseMatched" => Ok(S7Topology::MoeTinyDenseMatched),
        _ => Err("expected MoeTiny or MoeTinyDenseMatched".to_owned()),
    }
}

fn parse_support_artifact_kind(value: &str) -> Result<S7SupportArtifactKind, String> {
    value.parse()
}

fn parse_seed_list(value: &str) -> Result<Vec<u64>, S7CliError> {
    if value.trim().is_empty() {
        return Err(S7CliError::InvalidSeedList {
            value: value.to_owned(),
        });
    }
    value
        .split(',')
        .map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(S7CliError::InvalidSeedList {
                    value: value.to_owned(),
                });
            }
            trimmed
                .parse::<u64>()
                .map_err(|_| S7CliError::InvalidSeedList {
                    value: value.to_owned(),
                })
        })
        .collect()
}

fn validate_topology_feature(topology: &S7Topology) -> Result<(), S7CliError> {
    if cfg!(feature = "s7-moe") && topology != &S7Topology::MoeTiny {
        return Err(S7CliError::FeatureTopologyMismatch {
            feature: "s7-moe",
            topology: topology_name(topology),
        });
    }
    if cfg!(feature = "s7-dense-matched") && topology != &S7Topology::MoeTinyDenseMatched {
        return Err(S7CliError::FeatureTopologyMismatch {
            feature: "s7-dense-matched",
            topology: topology_name(topology),
        });
    }
    Ok(())
}

const fn topology_name(topology: &S7Topology) -> &'static str {
    match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    }
}

const fn active_feature_gate() -> &'static str {
    if cfg!(feature = "s7-moe") {
        "s7-moe"
    } else if cfg!(feature = "s7-dense-matched") {
        "s7-dense-matched"
    } else {
        "s7"
    }
}

fn report_self_hash_from_markdown(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let raw = line.strip_prefix("report_self_hash:")?.trim();
        let unquoted = raw
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(raw);
        is_sha256_hash(unquoted).then_some(unquoted)
    })
}

fn is_sha256_hash(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
}

/// Errors emitted by the S7 CLI.
#[derive(Debug)]
pub enum S7CliError {
    /// Replay helper failed.
    Replay(S7ReplayError),
    /// Completed-run artifact materialization failed.
    MaterializeRun(S7RunMaterializeError),
    /// Comparison-summary derivation failed.
    DeriveSummaries(S7SummaryMaterializeError),
    /// Dense-vs-MoE comparison derivation failed.
    DeriveComparison(S7ComparisonMaterializeError),
    /// Frontier derivation failed.
    DeriveFrontier(S7FrontierMaterializeError),
    /// Support artifact materialization failed.
    MaterializeSupportArtifact(S7SupportArtifactMaterializeError),
    /// Burn gradient smoke producer failed.
    #[cfg(feature = "s7-burn-grad-smoke")]
    BurnGradSmoke(S7BurnGradSmokeError),
    /// H10 one-token emulator report construction failed.
    EmulatorOneToken(EmulatorOneTokenReportError),
    /// H9 routed oracle report construction failed.
    OracleRouted(OracleRoutedReportError),
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// Filesystem operation failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source I/O error.
        source: std::io::Error,
    },
    /// Seed list could not be parsed.
    InvalidSeedList {
        /// Original seed-list string.
        value: String,
    },
    /// The closure packet currently expects H10 seed 0.
    InvalidEmulatorOneTokenSeed {
        /// Observed seed.
        seed: u64,
    },
    /// The selected topology does not match the active split feature gate.
    FeatureTopologyMismatch {
        /// Active feature gate.
        feature: &'static str,
        /// Requested topology.
        topology: &'static str,
    },
    /// Report emitter process returned non-zero status.
    ReportEmitterFailed {
        /// Exit status code or signal marker.
        status: String,
        /// Captured stdout.
        stdout: String,
        /// Captured stderr.
        stderr: String,
    },
    /// The emitted report did not contain a valid report_self_hash.
    InvalidReportSelfHash {
        /// Report path.
        path: String,
    },
    /// The CLI wrapper requires a file output so it can print only the report hash.
    UnsupportedStdoutReport,
    /// Closure packet failed the Rust closure contract adapter.
    ClosurePacket(S7ClosurePacketError),
    /// Production bundle runner failed.
    ProductionRunner(S7ProductionRunnerError),
}

impl fmt::Display for S7CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Replay(error) => write!(f, "{error}"),
            Self::MaterializeRun(error) => write!(f, "{error}"),
            Self::DeriveSummaries(error) => write!(f, "{error}"),
            Self::DeriveComparison(error) => write!(f, "{error}"),
            Self::DeriveFrontier(error) => write!(f, "{error}"),
            Self::MaterializeSupportArtifact(error) => write!(f, "{error}"),
            #[cfg(feature = "s7-burn-grad-smoke")]
            Self::BurnGradSmoke(error) => write!(f, "{error}"),
            Self::EmulatorOneToken(error) => write!(f, "{error}"),
            Self::OracleRouted(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::InvalidSeedList { value } => {
                write!(
                    f,
                    "invalid S7 seed list {value:?}; expected comma-separated u64 values"
                )
            }
            Self::InvalidEmulatorOneTokenSeed { seed } => {
                write!(f, "gbf s7 emulator-one-token requires --seed 0, got {seed}")
            }
            Self::FeatureTopologyMismatch { feature, topology } => write!(
                f,
                "S7 feature/topology mismatch: --features {feature} cannot replay topology {topology}"
            ),
            Self::ReportEmitterFailed {
                status,
                stdout,
                stderr,
            } => write!(
                f,
                "S7 report emitter failed with status {status}; stdout={stdout:?}; stderr={stderr:?}"
            ),
            Self::InvalidReportSelfHash { path } => {
                write!(f, "{path} missing valid report_self_hash")
            }
            Self::UnsupportedStdoutReport => {
                f.write_str("gbf s7 emit-report requires a file --output, not '-'")
            }
            Self::ClosurePacket(error) => write!(f, "S7 closure packet validation failed: {error}"),
            Self::ProductionRunner(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S7CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Replay(error) => Some(error),
            Self::MaterializeRun(error) => Some(error),
            Self::DeriveSummaries(error) => Some(error),
            Self::DeriveComparison(error) => Some(error),
            Self::DeriveFrontier(error) => Some(error),
            Self::MaterializeSupportArtifact(error) => Some(error),
            #[cfg(feature = "s7-burn-grad-smoke")]
            Self::BurnGradSmoke(error) => Some(error),
            Self::EmulatorOneToken(error) => Some(error),
            Self::OracleRouted(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::InvalidSeedList { .. }
            | Self::InvalidEmulatorOneTokenSeed { .. }
            | Self::FeatureTopologyMismatch { .. }
            | Self::ReportEmitterFailed { .. }
            | Self::InvalidReportSelfHash { .. }
            | Self::UnsupportedStdoutReport => None,
            Self::ClosurePacket(error) => Some(error),
            Self::ProductionRunner(error) => Some(error),
        }
    }
}

impl From<S7ReplayError> for S7CliError {
    fn from(error: S7ReplayError) -> Self {
        Self::Replay(error)
    }
}

impl From<S7RunMaterializeError> for S7CliError {
    fn from(error: S7RunMaterializeError) -> Self {
        Self::MaterializeRun(error)
    }
}

impl From<S7SummaryMaterializeError> for S7CliError {
    fn from(error: S7SummaryMaterializeError) -> Self {
        Self::DeriveSummaries(error)
    }
}

impl From<S7ComparisonMaterializeError> for S7CliError {
    fn from(error: S7ComparisonMaterializeError) -> Self {
        Self::DeriveComparison(error)
    }
}

impl From<S7FrontierMaterializeError> for S7CliError {
    fn from(error: S7FrontierMaterializeError) -> Self {
        Self::DeriveFrontier(error)
    }
}

impl From<S7SupportArtifactMaterializeError> for S7CliError {
    fn from(error: S7SupportArtifactMaterializeError) -> Self {
        Self::MaterializeSupportArtifact(error)
    }
}

#[cfg(feature = "s7-burn-grad-smoke")]
impl From<S7BurnGradSmokeError> for S7CliError {
    fn from(error: S7BurnGradSmokeError) -> Self {
        Self::BurnGradSmoke(error)
    }
}

impl From<CanonicalJsonError> for S7CliError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<EmulatorOneTokenReportError> for S7CliError {
    fn from(error: EmulatorOneTokenReportError) -> Self {
        Self::EmulatorOneToken(error)
    }
}

impl From<OracleRoutedReportError> for S7CliError {
    fn from(error: OracleRoutedReportError) -> Self {
        Self::OracleRouted(error)
    }
}

impl From<S7ClosurePacketError> for S7CliError {
    fn from(error: S7ClosurePacketError) -> Self {
        Self::ClosurePacket(error)
    }
}

impl From<S7ProductionRunnerError> for S7CliError {
    fn from(error: S7ProductionRunnerError) -> Self {
        Self::ProductionRunner(error)
    }
}
