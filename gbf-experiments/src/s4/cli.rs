//! S4 command-line integration surface.
//!
//! F-S4.23 wires command dispatch only. The command bodies below emit explicit
//! skeleton envelopes and leave corpus, training, scoring, oracle, and report
//! behavior to their owning downstream beads. Falsification-only CLI verbs are
//! intentionally not registered here; F-S4.20 (`bd-fii7`) owns those concrete
//! commands and their `s4-falsify` gating.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use gbf_foundation::{DomainHash, Hash256, Hash256ParseError, self_hash_omitting_fields};
use serde::de::DeserializeOwned;
use serde_json::{Number, Value, json};
use tracing_subscriber::prelude::*;

use crate::s4::harvest::{
    DEFAULT_CACHE_DIR, DEFAULT_CATALOG_SNAPSHOT_URL, DEFAULT_FIXTURE_OUTPUT, DEFAULT_TARGET_SLICE,
    DEFAULT_USER_AGENT, GutenbergHarvestOptions,
};
use crate::s4::manifest::{
    DEFAULT_GUTENBERG_FIXTURE_PATH, DEFAULT_GUTENBERG_MANIFEST_PATH, DEFAULT_GUTENBERG_TRAIN_PATH,
    DEFAULT_GUTENBERG_VAL_PATH, DEFAULT_S4_CORPUS_QUALITY_PATH, DEFAULT_TINYSTORIES_MANIFEST_PATH,
    GutenbergBuildOptions, build_gutenberg_corpus,
};
use crate::s4::promote::{
    PromotionGateArtifactRef, PromotionGateBoundArtifact, PromotionGateInputs,
    S3CheckpointPromotionArtifact, S3OracleAgreementPromotionArtifact,
    S3RepetitionCollapsePromotionArtifact, S3V0SuccessPromotionArtifact, S4_PROMOTION_GATE_PATH,
    S4BaselineGutenbergPromotionArtifact, S4ContaminationPromotionArtifact, promotion_gate,
};
use crate::s4::schema::S4BuildKind;

const DEFAULT_PASS_VERSION_S4: &str = "0.4.0";
const DEFAULT_BUILD_KIND: &str = "phase_d_continuation";
const DEFAULT_DEVICE_PROFILE: &str = "S1CpuDeterministic";
const DEFAULT_GUTENBERG_MANIFEST: &str = DEFAULT_GUTENBERG_MANIFEST_PATH;
const CLI_LOG_TARGET: &str = "gbf_experiments::s4::cli";
const FALSIFICATION_VERBS_OWNER_BEAD: &str = "bd-fii7";
const S4_CLI_DISPATCH_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4CliDispatch",
    "s4_cli_dispatch.v1",
    "1",
);
const S4_GUTENBERG_HARVEST_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4GutenbergHarvestCli",
    "s4_gutenberg_harvest.v1",
    "1",
);

/// S4 CLI envelope.
#[derive(Debug, Clone, Parser)]
pub struct S4Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: S4Command,
    /// Logging/capture configuration supplied by the top-level CLI.
    #[arg(skip)]
    pub logging: S4CliLogging,
}

/// S4 subcommands registered by F-S4.23.
#[derive(Debug, Clone, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum S4Command {
    /// Dispatch the full S4 replay skeleton.
    ReplayFull(S4ReplayFullArgs),
    /// Dispatch the fallback replay skeleton.
    ReplayFallback(S4ReplayFallbackArgs),
    /// Dispatch the network-permitted Gutenberg fixture harvest skeleton.
    HarvestGutenbergFixture(S4HarvestGutenbergFixtureArgs),
    /// Dispatch the network-disabled Gutenberg corpus build skeleton.
    BuildCorpus(S4BuildCorpusArgs),
    /// Dispatch the RFC §17.4 Gutenberg baseline skeleton.
    FitBaselineGutenberg(S4FitBaselineGutenbergArgs),
    /// Dispatch the RFC §17.4 cross-corpus contamination skeleton.
    Contamination(S4ContaminationArgs),
    /// Dispatch the RFC §17.4 promotion gate skeleton.
    Promote(S4PromoteArgs),
    /// Dispatch the RFC §17.4 corpus-oracle skeleton.
    Oracle(S4OracleArgs),
    /// Dispatch the Gutenberg scoring skeleton.
    ScoreGutenberg(S4ScoreGutenbergArgs),
    /// Dispatch the S4 determinism verification skeleton.
    VerifyDeterminism(S4VerifyDeterminismArgs),
    /// Dispatch the Gutenberg normalization skeleton.
    NormalizeCorpus(S4NormalizeCorpusArgs),
    /// Dispatch the S4 report emission skeleton.
    EmitReport(S4EmitReportArgs),
}

/// S4 CLI log format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S4CliLogFormat {
    /// Human-readable stderr events.
    #[default]
    Pretty,
    /// NDJSON stderr events.
    Json,
}

/// S4 CLI log level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum S4CliLogLevel {
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

/// Logging/capture configuration for S4 CLI commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S4CliLogging {
    /// Stderr event format.
    pub format: S4CliLogFormat,
    /// CLI event level.
    pub level: S4CliLogLevel,
    /// Optional additional event sink.
    pub log_file: Option<PathBuf>,
    /// Optional NDJSON capture sink for structured test assertions.
    pub capture_events: Option<PathBuf>,
}

/// Common S4 skeleton arguments accepted by every verb.
#[derive(Debug, Clone, Args)]
pub struct CommonS4Args {
    /// Gutenberg manifest path.
    #[arg(long, default_value = DEFAULT_GUTENBERG_MANIFEST)]
    pub gutenberg_manifest: PathBuf,
    /// S4 pass version.
    #[arg(long, default_value = DEFAULT_PASS_VERSION_S4)]
    pub pass_version: String,
    /// Comma-separated S4 seeds.
    #[arg(long, default_value = "0,1,2,3,4")]
    pub seed_list: String,
    /// S4 build kind.
    #[arg(long, default_value = DEFAULT_BUILD_KIND)]
    pub build_kind: String,
    /// Deterministic device profile.
    #[arg(long, default_value = DEFAULT_DEVICE_PROFILE)]
    pub device_profile: String,
    /// Compatibility flag; stdout remains a single artifact self-hash.
    #[arg(long, hide = true)]
    pub json: bool,
}

/// Arguments for `gbf s4 replay-full`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 replay-full --seed-list 0,1,2,3,4 --output experiments/S4/replay-full-cli.json"
)]
pub struct S4ReplayFullArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-replay-full-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 replay-fallback`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 replay-fallback --seed-list 0 --output experiments/S4/replay-fallback-cli.json"
)]
pub struct S4ReplayFallbackArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-replay-fallback-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 harvest-gutenberg-fixture`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 harvest-gutenberg-fixture --network-permitted --fixture-output fixtures/corpora/gutenberg.toml --output experiments/S4/harvest-gutenberg-fixture-cli.json"
)]
pub struct S4HarvestGutenbergFixtureArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-harvest-gutenberg-fixture-cli.json")]
    pub output: PathBuf,
    /// Explicit acknowledgment that this is the only network-permitted S4 op.
    #[arg(long)]
    pub network_permitted: bool,
    /// RDF catalog snapshot URL. Supports `https://`, `http://`, and `file://`.
    #[arg(long, default_value = DEFAULT_CATALOG_SNAPSHOT_URL)]
    pub catalog_url: String,
    /// Optional local RDF catalog tar.bz2 path, used instead of fetching `--catalog-url`.
    #[arg(long)]
    pub catalog_path: Option<PathBuf>,
    /// Harvest cache root for the catalog snapshot and source blobs.
    #[arg(long, default_value = DEFAULT_CACHE_DIR)]
    pub cache_dir: PathBuf,
    /// Fixture TOML path written for later network-disabled S4 corpus operations.
    #[arg(long, default_value = DEFAULT_FIXTURE_OUTPUT)]
    pub fixture_output: PathBuf,
    /// Number of selected Gutenberg book IDs. Production S4 uses 1500.
    #[arg(long, default_value_t = DEFAULT_TARGET_SLICE)]
    pub target_slice: usize,
    /// RFC3339 UTC observation timestamp for the catalog snapshot.
    #[arg(long)]
    pub catalog_observed_at_utc: Option<String>,
    /// Optional RFC3339 UTC Last-Modified timestamp for the catalog snapshot.
    #[arg(long)]
    pub catalog_last_modified_utc: Option<String>,
    /// Fetch namespace kind recorded into each source row.
    #[arg(
        long,
        default_value = "official_robot_harvest",
        value_parser = ["local_private_mirror", "official_robot_harvest", "content_addressed_cache"]
    )]
    pub fetch_namespace_kind: String,
    /// Fetch namespace identifier recorded into each source row.
    #[arg(long, default_value = "https://www.gutenberg.org/")]
    pub fetch_namespace_id: String,
    /// User-Agent used for HTTP(S) fetches.
    #[arg(long, default_value = DEFAULT_USER_AGENT)]
    pub user_agent: String,
    /// Per-request timeout for catalog and source fetches.
    #[arg(long, default_value_t = 60)]
    pub fetch_timeout_seconds: u64,
}

/// Arguments for `gbf s4 build-corpus`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 build-corpus --fixture fixtures/corpora/gutenberg.toml --gutenberg-manifest experiments/S4/corpus/gutenberg-manifest.json --train-output experiments/S4/corpus/gutenberg-train.bin --val-output experiments/S4/corpus/gutenberg-val.bin --corpus-quality-output experiments/S4/corpus_quality/corpus_quality.json"
)]
pub struct S4BuildCorpusArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_build_corpus_cli.v1` summary evidence path.
    #[arg(long, default_value = "/tmp/s4-build-corpus-cli.json")]
    pub output: PathBuf,
    /// Network-disabled Gutenberg fixture pin path.
    #[arg(long, default_value = DEFAULT_GUTENBERG_FIXTURE_PATH)]
    pub fixture: PathBuf,
    /// Output Gutenberg train token-id stream path.
    #[arg(long, default_value = DEFAULT_GUTENBERG_TRAIN_PATH)]
    pub train_output: PathBuf,
    /// Output Gutenberg validation token-id stream path.
    #[arg(long, default_value = DEFAULT_GUTENBERG_VAL_PATH)]
    pub val_output: PathBuf,
    /// Output `s4_corpus_quality.v1` path.
    #[arg(long, default_value = DEFAULT_S4_CORPUS_QUALITY_PATH)]
    pub corpus_quality_output: PathBuf,
    /// TinyStories manifest path used for corpus-quality manifest pointer hashing.
    #[arg(long, default_value = DEFAULT_TINYSTORIES_MANIFEST_PATH)]
    pub tinystories_manifest: PathBuf,
}

/// Arguments for `gbf s4 fit-baseline-gutenberg`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 fit-baseline-gutenberg --gutenberg-manifest experiments/S4/gutenberg_manifest.json --output experiments/S4/fit-baseline-gutenberg-cli.json"
)]
pub struct S4FitBaselineGutenbergArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-fit-baseline-gutenberg-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 contamination`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 contamination --gutenberg-manifest experiments/S4/gutenberg_manifest.json --output experiments/S4/contamination-cli.json"
)]
pub struct S4ContaminationArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-contamination-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 promote`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 promote --tinystories-manifest-self-hash sha256:... --c-ts experiments/S3/checkpoints/seed-0/checkpoint.json --c-ts-v0success experiments/S3/v0_success/seed-0.json --c-ts-oracle-agreement experiments/S3/oracle_agreement/seed-0.json --gutenberg-manifest experiments/S4/gutenberg_manifest.json --contamination-report experiments/S4/contamination/cross_corpus.json --baseline-gutenberg experiments/S4/baseline/baseline_gutenberg.json --repetition-collapse-check experiments/S3/repetition/seed-0.json --output experiments/S4/promotion_gate/promotion_gate.json"
)]
pub struct S4PromoteArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// TinyStories manifest self-hash expected across S3 promotion inputs.
    #[arg(long)]
    pub tinystories_manifest_self_hash: String,
    /// S3 ternary checkpoint summary artifact path.
    #[arg(long = "c-ts")]
    pub c_ts: PathBuf,
    /// S3 v0_success artifact path. Omit to emit a P-1 rejection product.
    #[arg(long = "c-ts-v0success")]
    pub c_ts_v0success: Option<PathBuf>,
    /// S3 TinyStories oracle-agreement artifact path. Omit to emit a P-2 rejection product.
    #[arg(long = "c-ts-oracle-agreement")]
    pub c_ts_oracle_agreement: Option<PathBuf>,
    /// S4 contamination report artifact path.
    #[arg(long)]
    pub contamination_report: PathBuf,
    /// S4 Gutenberg KN-5 baseline artifact path. Omit to emit a P-7 rejection product.
    #[arg(long)]
    pub baseline_gutenberg: Option<PathBuf>,
    /// S3 repetition-collapse check artifact path.
    #[arg(long)]
    pub repetition_collapse_check: PathBuf,
    /// Output `s4_promotion_gate.v1` path.
    #[arg(long, default_value = S4_PROMOTION_GATE_PATH)]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 oracle`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 oracle --gutenberg-manifest experiments/S4/gutenberg_manifest.json --output experiments/S4/oracle-cli.json"
)]
pub struct S4OracleArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-oracle-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 score-gutenberg`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 score-gutenberg --seed-list 0 --output experiments/S4/score-gutenberg-cli.json"
)]
pub struct S4ScoreGutenbergArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-score-gutenberg-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 verify-determinism`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 verify-determinism --seed-list 0 --output experiments/S4/verify-determinism-cli.json"
)]
pub struct S4VerifyDeterminismArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-verify-determinism-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 normalize-corpus`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 normalize-corpus --gutenberg-manifest experiments/S4/gutenberg_manifest.json --output experiments/S4/normalize-corpus-cli.json"
)]
pub struct S4NormalizeCorpusArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-normalize-corpus-cli.json")]
    pub output: PathBuf,
}

/// Arguments for `gbf s4 emit-report`.
#[derive(Debug, Clone, Args)]
#[command(
    after_help = "Examples:\n  gbf s4 emit-report --seed-list 0,1,2,3,4 --output experiments/S4/emit-report-cli.json"
)]
pub struct S4EmitReportArgs {
    /// Shared S4 options.
    #[command(flatten)]
    pub common: CommonS4Args,
    /// Output `s4_cli_dispatch.v1` skeleton evidence path.
    #[arg(long, default_value = "/tmp/s4-emit-report-cli.json")]
    pub output: PathBuf,
}

/// Run an S4 CLI command.
pub fn run(cli: S4Cli) -> Result<(), S4CliError> {
    let logging = cli.logging.clone();
    if logging.level == S4CliLogLevel::Off {
        return run_command_with_lifecycle(cli.command, &logging);
    }
    let mut layers = Vec::new();
    if let Some(path) = &logging.capture_events {
        layers.push(NdjsonTraceLayer::new(path)?);
    }
    if let Some(path) = &logging.log_file {
        layers.push(NdjsonTraceLayer::new(path)?);
    }

    if layers.is_empty() {
        run_command_with_lifecycle(cli.command, &logging)
    } else if layers.len() == 1 {
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::TRACE)
            .with(layers.remove(0));
        tracing::subscriber::with_default(subscriber, || {
            run_command_with_lifecycle(cli.command, &logging)
        })
    } else {
        let second = layers.pop().expect("second layer");
        let first = layers.pop().expect("first layer");
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::TRACE)
            .with(first)
            .with(second);
        tracing::subscriber::with_default(subscriber, || {
            run_command_with_lifecycle(cli.command, &logging)
        })
    }
}

fn run_command_with_lifecycle(
    command: S4Command,
    logging: &S4CliLogging,
) -> Result<(), S4CliError> {
    crate::s4::ensure_module_loaded();
    let metadata = command_metadata(&command)?;
    emit_cli_start(&metadata, logging);
    let started_at = Instant::now();
    let result = match &command {
        S4Command::HarvestGutenbergFixture(args) => write_harvest_dispatch(args, &metadata),
        S4Command::BuildCorpus(args) => write_build_corpus_dispatch(args, &metadata),
        S4Command::Promote(args) => write_promote_dispatch(args, &metadata),
        _ => write_skeleton_dispatch(&metadata),
    };
    emit_cli_done(
        metadata.verb,
        result.is_ok(),
        started_at.elapsed().as_millis() as u64,
        result.as_ref().ok().copied(),
        logging,
    );
    result.map(|_| ())
}

fn write_harvest_dispatch(
    args: &S4HarvestGutenbergFixtureArgs,
    metadata: &CommandMetadata<'_>,
) -> Result<Hash256, S4CliError> {
    if let Some(parent) = metadata
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S4CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let options = GutenbergHarvestOptions {
        network_permitted: args.network_permitted,
        catalog_url: args.catalog_url.clone(),
        catalog_path: args.catalog_path.clone(),
        cache_dir: args.cache_dir.clone(),
        fixture_output: args.fixture_output.clone(),
        target_slice: args.target_slice,
        catalog_observed_at_utc: args
            .catalog_observed_at_utc
            .clone()
            .map_or_else(crate::s4::harvest::current_rfc3339_utc, Ok)?,
        catalog_last_modified_utc: args.catalog_last_modified_utc.clone(),
        fetch_namespace_kind: args.fetch_namespace_kind.clone(),
        fetch_namespace_id: args.fetch_namespace_id.clone(),
        user_agent: args.user_agent.clone(),
        fetch_timeout_seconds: args.fetch_timeout_seconds,
    };
    let summary = crate::s4::harvest::harvest_gutenberg_fixture(&options)?;
    let mut evidence = json!({
        "schema": "s4_gutenberg_harvest.v1",
        "artifact_self_hash": Hash256::ZERO,
        "status": "fixture_harvested",
        "command": metadata.verb,
        "owner_bead": metadata.owner_bead,
        "build_kind": metadata.build_kind.as_str(),
        "pass_version_S4": metadata.common.pass_version,
        "seed_list": metadata.common.seed_list,
        "device_profile": metadata.common.device_profile,
        "artifact_path": metadata.output,
        "network_policy": metadata.network_policy,
        "behavior_deferred": false,
        "harvest": summary,
    });
    let artifact_self_hash = self_hash_omitting_fields(
        S4_GUTENBERG_HARVEST_DOMAIN,
        &evidence,
        "artifact_self_hash",
        &[],
    )?;
    evidence
        .as_object_mut()
        .expect("S4 harvest evidence is a JSON object")
        .insert(
            "artifact_self_hash".to_owned(),
            Value::String(artifact_self_hash.to_string()),
        );
    let bytes = gbf_foundation::CanonicalJson::value_to_vec(&evidence)?;
    std::fs::write(metadata.output, &bytes).map_err(|source| S4CliError::Io {
        path: metadata.output.display().to_string(),
        source,
    })?;
    println!("{artifact_self_hash}");
    Ok(artifact_self_hash)
}

fn write_build_corpus_dispatch(
    args: &S4BuildCorpusArgs,
    metadata: &CommandMetadata<'_>,
) -> Result<Hash256, S4CliError> {
    if let Some(parent) = metadata
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S4CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let options = GutenbergBuildOptions {
        fixture_path: args.fixture.clone(),
        manifest_path: args.common.gutenberg_manifest.clone(),
        train_path: args.train_output.clone(),
        val_path: args.val_output.clone(),
        corpus_quality_path: Some(args.corpus_quality_output.clone()),
        tinystories_manifest_path: Some(args.tinystories_manifest.clone()),
    };
    let summary = build_gutenberg_corpus(&options)?;
    let evidence = json!({
        "schema": "s4_build_corpus_cli.v1",
        "status": "built",
        "command": metadata.verb,
        "owner_bead": metadata.owner_bead,
        "build_kind": metadata.build_kind.as_str(),
        "pass_version_S4": metadata.common.pass_version,
        "seed_list": metadata.common.seed_list,
        "device_profile": metadata.common.device_profile,
        "fixture_path": args.fixture,
        "gutenberg_manifest": summary.manifest_path,
        "train_path": summary.train_path,
        "val_path": summary.val_path,
        "corpus_quality_path": summary.corpus_quality_path,
        "manifest_self_hash": summary.manifest_self_hash,
        "corpus_quality_self_hash": summary.corpus_quality_self_hash,
        "train_sha256": summary.train_sha256,
        "val_sha256": summary.val_sha256,
        "train_byte_length": summary.train_byte_length,
        "val_byte_length": summary.val_byte_length,
        "train_book_count": summary.train_book_count,
        "val_book_count": summary.val_book_count,
        "drop_count_total": summary.drop_count_total,
        "network_policy": metadata.network_policy,
        "behavior_deferred": false,
        "unmappable_gate": {
            "status": "emitted",
            "artifact_schema": "s4_corpus_quality.v1",
            "artifact_path": summary.corpus_quality_path,
            "corpus_quality_self_hash": summary.corpus_quality_self_hash,
            "owner_bead": "bd-bzx3"
        },
        "deferred_owner_beads": {
            "contamination_report": "bd-2p3n",
            "kn_baseline": "bd-2nca"
        }
    });
    let bytes = gbf_foundation::CanonicalJson::value_to_vec(&evidence)?;
    std::fs::write(metadata.output, &bytes).map_err(|source| S4CliError::Io {
        path: metadata.output.display().to_string(),
        source,
    })?;
    println!("{}", summary.manifest_self_hash);
    Ok(summary.manifest_self_hash)
}

fn write_promote_dispatch(
    args: &S4PromoteArgs,
    metadata: &CommandMetadata<'_>,
) -> Result<Hash256, S4CliError> {
    if let Some(parent) = metadata
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S4CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let product = promotion_gate(promotion_inputs_from_args(args)?)?;
    let bytes = product.canonical_bytes()?;
    std::fs::write(metadata.output, &bytes).map_err(|source| S4CliError::Io {
        path: metadata.output.display().to_string(),
        source,
    })?;
    println!("{}", product.promotion_gate_self_hash);
    Ok(product.promotion_gate_self_hash)
}

fn promotion_inputs_from_args(args: &S4PromoteArgs) -> Result<PromotionGateInputs, S4CliError> {
    let tinystories_manifest_self_hash = parse_hash(
        "tinystories_manifest_self_hash",
        &args.tinystories_manifest_self_hash,
    )?;

    let c_ts_artifact: S3CheckpointPromotionArtifact = read_promotion_json(&args.c_ts)?;
    let c_ts_self_hash = c_ts_artifact.checkpoint_self_hash;
    let gb_manifest_artifact: gbf_artifact::GutenbergManifest =
        read_promotion_json(&args.common.gutenberg_manifest)?;
    let gb_manifest_self_hash = gb_manifest_artifact.manifest_self_hash;
    let contamination_artifact: S4ContaminationPromotionArtifact =
        read_promotion_json(&args.contamination_report)?;
    let contamination_self_hash = contamination_artifact.contamination_self_hash;
    let repetition_artifact: S3RepetitionCollapsePromotionArtifact =
        read_promotion_json(&args.repetition_collapse_check)?;
    let repetition_self_hash = repetition_artifact.repetition_self_hash;

    let c_ts_v0success = if let Some(path) = &args.c_ts_v0success {
        let artifact: S3V0SuccessPromotionArtifact = read_promotion_json(path)?;
        let self_hash = artifact.v0_success_self_hash;
        Some(bound_promotion_artifact(path, self_hash, artifact))
    } else {
        None
    };
    let c_ts_oracle_agreement = if let Some(path) = &args.c_ts_oracle_agreement {
        let artifact: S3OracleAgreementPromotionArtifact = read_promotion_json(path)?;
        let self_hash = artifact.oracle_agreement_self_hash;
        Some(bound_promotion_artifact(path, self_hash, artifact))
    } else {
        None
    };
    let baseline_gutenberg = if let Some(path) = &args.baseline_gutenberg {
        let artifact: S4BaselineGutenbergPromotionArtifact = read_promotion_json(path)?;
        let self_hash = artifact.baseline_self_hash;
        Some(bound_promotion_artifact(path, self_hash, artifact))
    } else {
        None
    };

    Ok(PromotionGateInputs {
        tinystories_manifest_self_hash,
        c_ts: bound_promotion_artifact(&args.c_ts, c_ts_self_hash, c_ts_artifact),
        c_ts_v0success,
        c_ts_oracle_agreement,
        gb_manifest: bound_promotion_artifact(
            &args.common.gutenberg_manifest,
            gb_manifest_self_hash,
            gb_manifest_artifact,
        ),
        contamination_report: bound_promotion_artifact(
            &args.contamination_report,
            contamination_self_hash,
            contamination_artifact,
        ),
        baseline_gutenberg,
        repetition_collapse_check: bound_promotion_artifact(
            &args.repetition_collapse_check,
            repetition_self_hash,
            repetition_artifact,
        ),
    })
}

fn parse_hash(field: &'static str, value: &str) -> Result<Hash256, S4CliError> {
    value
        .parse::<Hash256>()
        .map_err(|source| S4CliError::InvalidHash {
            field,
            value: value.to_owned(),
            source,
        })
}

fn read_promotion_json<T>(path: &Path) -> Result<T, S4CliError>
where
    T: DeserializeOwned,
{
    let bytes = std::fs::read(path).map_err(|source| S4CliError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(S4CliError::Json)
}

fn bound_promotion_artifact<T>(
    path: &Path,
    self_hash: Hash256,
    artifact: T,
) -> PromotionGateBoundArtifact<T> {
    PromotionGateBoundArtifact::new(
        PromotionGateArtifactRef::new(path.to_string_lossy(), self_hash),
        artifact,
    )
}

fn write_skeleton_dispatch(metadata: &CommandMetadata<'_>) -> Result<Hash256, S4CliError> {
    if let Some(parent) = metadata
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| S4CliError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let mut evidence = json!({
        "schema": "s4_cli_dispatch.v1",
        "artifact_self_hash": Hash256::ZERO,
        "status": "skeleton_dispatched",
        "command": metadata.verb,
        "owner_bead": metadata.owner_bead,
        "build_kind": metadata.build_kind.as_str(),
        "pass_version_S4": metadata.common.pass_version,
        "seed_list": metadata.common.seed_list,
        "device_profile": metadata.common.device_profile,
        "gutenberg_manifest": metadata.common.gutenberg_manifest,
        "artifact_path": metadata.output,
        "network_policy": metadata.network_policy,
        "s4_full_enabled": cfg!(feature = "s4-full"),
        "s4_falsify_enabled": cfg!(feature = "s4-falsify"),
        "falsification_cli_verbs_registered": false,
        "falsification_verbs_owner_bead": FALSIFICATION_VERBS_OWNER_BEAD,
        "behavior_deferred": true,
    });
    let artifact_self_hash =
        self_hash_omitting_fields(S4_CLI_DISPATCH_DOMAIN, &evidence, "artifact_self_hash", &[])?;
    evidence
        .as_object_mut()
        .expect("S4 CLI dispatch evidence is a JSON object")
        .insert(
            "artifact_self_hash".to_owned(),
            Value::String(artifact_self_hash.to_string()),
        );
    let bytes = gbf_foundation::CanonicalJson::value_to_vec(&evidence)?;
    std::fs::write(metadata.output, &bytes).map_err(|source| S4CliError::Io {
        path: metadata.output.display().to_string(),
        source,
    })?;
    println!("{artifact_self_hash}");
    Ok(artifact_self_hash)
}

#[derive(Debug)]
struct CommandMetadata<'a> {
    verb: &'static str,
    owner_bead: &'static str,
    common: &'a CommonS4Args,
    output: &'a Path,
    args_json: Value,
    build_kind: S4BuildKind,
    network_policy: &'static str,
}

fn command_metadata(command: &S4Command) -> Result<CommandMetadata<'_>, S4CliError> {
    match command {
        S4Command::ReplayFull(args) => metadata(
            "replay-full",
            "bd-10iq",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::ReplayFallback(args) => metadata(
            "replay-fallback",
            "bd-25pg",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::HarvestGutenbergFixture(args) => metadata(
            "harvest-gutenberg-fixture",
            "bd-1zd1",
            &args.common,
            &args.output,
            "permitted",
            json!({
                "output": args.output,
                "network_permitted": args.network_permitted,
                "catalog_url": args.catalog_url,
                "catalog_path": args.catalog_path,
                "cache_dir": args.cache_dir,
                "fixture_output": args.fixture_output,
                "target_slice": args.target_slice,
                "catalog_observed_at_utc": args.catalog_observed_at_utc,
                "catalog_last_modified_utc": args.catalog_last_modified_utc,
                "fetch_namespace_kind": args.fetch_namespace_kind,
                "fetch_namespace_id": args.fetch_namespace_id,
                "fetch_timeout_seconds": args.fetch_timeout_seconds,
                "common": common_json(&args.common),
            }),
        ),
        S4Command::BuildCorpus(args) => metadata(
            "build-corpus",
            "bd-29lv",
            &args.common,
            &args.output,
            "disabled",
            json!({
                "output": args.output,
                "fixture": args.fixture,
                "train_output": args.train_output,
                "val_output": args.val_output,
                "corpus_quality_output": args.corpus_quality_output,
                "tinystories_manifest": args.tinystories_manifest,
                "common": common_json(&args.common),
            }),
        ),
        S4Command::FitBaselineGutenberg(args) => metadata(
            "fit-baseline-gutenberg",
            "bd-2nca",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::Contamination(args) => metadata(
            "contamination",
            "bd-2p3n",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::Promote(args) => metadata(
            "promote",
            "bd-13sa",
            &args.common,
            &args.output,
            "disabled",
            json!({
                "output": args.output,
                "tinystories_manifest_self_hash": args.tinystories_manifest_self_hash,
                "c_TS": args.c_ts,
                "c_TS_v0success": args.c_ts_v0success,
                "c_TS_oracle_agreement": args.c_ts_oracle_agreement,
                "contamination_report": args.contamination_report,
                "baseline_gutenberg": args.baseline_gutenberg,
                "repetition_collapse_check": args.repetition_collapse_check,
                "common": common_json(&args.common),
            }),
        ),
        S4Command::Oracle(args) => metadata(
            "oracle",
            "bd-3pcy",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::ScoreGutenberg(args) => metadata(
            "score-gutenberg",
            "bd-2eun",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::VerifyDeterminism(args) => metadata(
            "verify-determinism",
            "bd-u6tn",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::NormalizeCorpus(args) => metadata(
            "normalize-corpus",
            "bd-1zd1",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
        S4Command::EmitReport(args) => metadata(
            "emit-report",
            "bd-3f5b",
            &args.common,
            &args.output,
            "disabled",
            json!({"output": args.output, "common": common_json(&args.common)}),
        ),
    }
}

fn metadata<'a>(
    verb: &'static str,
    owner_bead: &'static str,
    common: &'a CommonS4Args,
    output: &'a Path,
    network_policy: &'static str,
    args_json: Value,
) -> Result<CommandMetadata<'a>, S4CliError> {
    Ok(CommandMetadata {
        verb,
        owner_bead,
        common,
        output,
        args_json,
        build_kind: parse_build_kind(&common.build_kind)?,
        network_policy,
    })
}

fn parse_build_kind(value: &str) -> Result<S4BuildKind, S4CliError> {
    match value {
        "phase_d_continuation" => Ok(S4BuildKind::phase_d_continuation),
        "ablation_compile_check" => Ok(S4BuildKind::ablation_compile_check),
        "s4_falsification" => Ok(S4BuildKind::s4_falsification),
        _ => Err(S4CliError::InvalidBuildKind {
            value: value.to_owned(),
        }),
    }
}

fn common_json(common: &CommonS4Args) -> Value {
    json!({
        "gutenberg_manifest": common.gutenberg_manifest,
        "pass_version_S4": common.pass_version,
        "seed_list": common.seed_list,
        "build_kind": common.build_kind,
        "device_profile": common.device_profile,
        "stdout": "artifact_self_hash",
        "json_requested": common.json,
    })
}

fn emit_cli_start(metadata: &CommandMetadata<'_>, logging: &S4CliLogging) {
    tracing::info!(
        target: CLI_LOG_TARGET,
        event_name = "s4_cli_verb_started",
        verb = metadata.verb,
        args = %metadata.args_json,
        build_kind = metadata.build_kind.as_str(),
        pass_version_S4 = metadata.common.pass_version.as_str(),
        "s4 cli command started"
    );
    write_stderr_event(
        logging,
        json!({
            "event_name": "s4_cli_verb_started",
            "verb": metadata.verb,
            "build_kind": metadata.build_kind.as_str(),
            "pass_version_S4": metadata.common.pass_version,
            "network_policy": metadata.network_policy,
        }),
    );
}

fn emit_cli_done(
    verb: &'static str,
    passed: bool,
    total_duration_ms: u64,
    artifact_self_hash: Option<Hash256>,
    logging: &S4CliLogging,
) {
    let artifact_self_hash_text = artifact_self_hash.map(|hash| hash.to_string());
    tracing::info!(
        target: CLI_LOG_TARGET,
        event_name = "s4_cli_verb_finalized",
        verb,
        outcome = if passed { "success" } else { "failure" },
        exit_code = if passed { 0_i64 } else { 1_i64 },
        artifact_self_hash = artifact_self_hash_text.as_deref().unwrap_or(""),
        total_duration_ms,
        "s4 cli command completed"
    );
    write_stderr_event(
        logging,
        json!({
            "event_name": "s4_cli_verb_finalized",
            "verb": verb,
            "outcome": if passed { "success" } else { "failure" },
            "exit_code": if passed { 0 } else { 1 },
            "artifact_self_hash": artifact_self_hash_text,
            "total_duration_ms": total_duration_ms,
        }),
    );
}

fn write_stderr_event(logging: &S4CliLogging, event: Value) {
    if logging.level == S4CliLogLevel::Off {
        return;
    }

    match logging.format {
        S4CliLogFormat::Json => {
            eprintln!(
                "{}",
                serde_json::to_string(&event).expect("S4 CLI event JSON serializes")
            );
        }
        S4CliLogFormat::Pretty => {
            let event_name = event
                .get("event_name")
                .and_then(Value::as_str)
                .unwrap_or("s4_cli_event");
            let verb = event
                .get("verb")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let outcome = event.get("outcome").and_then(Value::as_str).unwrap_or("");
            let artifact_self_hash = event
                .get("artifact_self_hash")
                .and_then(Value::as_str)
                .unwrap_or("");
            let build_kind = event
                .get("build_kind")
                .and_then(Value::as_str)
                .unwrap_or("");
            let suffix = if !artifact_self_hash.is_empty() {
                format!(" artifact_self_hash={artifact_self_hash}")
            } else if !build_kind.is_empty() {
                format!(" build_kind={build_kind}")
            } else if !outcome.is_empty() {
                format!(" outcome={outcome}")
            } else {
                String::new()
            };
            eprintln!("{event_name} verb={verb}{suffix}");
        }
    }
}

#[derive(Clone)]
struct NdjsonTraceLayer {
    writer: Arc<Mutex<File>>,
}

impl NdjsonTraceLayer {
    fn new(path: &Path) -> Result<Self, S4CliError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|source| S4CliError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let writer = File::create(path).map_err(|source| S4CliError::Io {
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
        let record = json!({
            "target": event.metadata().target(),
            "level": event.metadata().level().to_string(),
            "fields": visitor.fields,
        });
        let mut writer = self.writer.lock().expect("S4 CLI NDJSON writer mutex");
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

/// Errors from S4 CLI commands.
#[derive(Debug)]
pub enum S4CliError {
    /// File IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON serialization failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    /// Gutenberg build-corpus failed.
    BuildCorpus(crate::s4::manifest::GutenbergBuildError),
    /// Invalid build kind.
    InvalidBuildKind {
        /// Raw build-kind text.
        value: String,
    },
    /// Gutenberg harvest failed.
    Harvest(crate::s4::harvest::GutenbergHarvestError),
    /// Promotion-gate evaluation failed.
    PromotionGate(crate::s4::promote::PromotionGateError),
    /// A CLI hash argument was malformed.
    InvalidHash {
        /// Argument field name.
        field: &'static str,
        /// Raw hash text.
        value: String,
        /// Parse failure.
        source: Hash256ParseError,
    },
}

impl fmt::Display for S4CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::BuildCorpus(error) => write!(f, "{error}"),
            Self::InvalidBuildKind { value } => {
                write!(f, "invalid S4 build kind {value:?}")
            }
            Self::Harvest(error) => write!(f, "{error}"),
            Self::PromotionGate(error) => write!(f, "{error}"),
            Self::InvalidHash {
                field,
                value,
                source,
            } => write!(f, "invalid {field} hash {value:?}: {source}"),
        }
    }
}

impl std::error::Error for S4CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::BuildCorpus(error) => Some(error),
            Self::InvalidBuildKind { .. } => None,
            Self::Harvest(error) => Some(error),
            Self::PromotionGate(error) => Some(error),
            Self::InvalidHash { source, .. } => Some(source),
        }
    }
}

impl From<serde_json::Error> for S4CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<gbf_foundation::CanonicalJsonError> for S4CliError {
    fn from(error: gbf_foundation::CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<crate::s4::harvest::GutenbergHarvestError> for S4CliError {
    fn from(error: crate::s4::harvest::GutenbergHarvestError) -> Self {
        Self::Harvest(error)
    }
}

impl From<crate::s4::manifest::GutenbergBuildError> for S4CliError {
    fn from(error: crate::s4::manifest::GutenbergBuildError) -> Self {
        Self::BuildCorpus(error)
    }
}

impl From<crate::s4::promote::PromotionGateError> for S4CliError {
    fn from(error: crate::s4::promote::PromotionGateError) -> Self {
        Self::PromotionGate(error)
    }
}
