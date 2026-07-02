//! S7 determinism and fixture replay helpers.
//!
//! The helpers here prove byte identity for the S7 tiny fixture and existing
//! artifact contracts. They deliberately do not claim production CLI/full
//! checkpoint replay; that adoption remains owned by bd-1ryn and bd-2v9r.

use std::fmt;

use gbf_artifact::{
    DistillRawDiagnostic, GradNormSummary, RawLossDiagnostics, S7_N_EXPERTS, S7Completion,
    S7RunLog, S7ScoreReport, S7Topology, TemporalSwitchDigest, TrainPhase, TransitionEntry,
};
use gbf_foundation::{CanonicalJsonError, DomainHash, ExpertId, Hash256, LayerId, SemVer, sha256};
use serde::Serialize;

use crate::S7_LOG_TARGET;
use crate::s7::baseline_match::{
    S7MatchedBytesError, canonical_s7_matched_bytes_json_bytes, canonical_s7_matched_bytes_pin,
};
use crate::s7::collapse_sweep::{
    CollapseSweepError, DeterministicFixtureSweepProducer, LambdaSwitchSweepInput,
    RouterCollapseSweepReport, run_lambda_switch_sweep,
};
use crate::s7::schema::{S7ScoreFromBytesError, charset_v1_normalized_token_count};

/// Scope marker emitted by fixture determinism events.
pub const S7_DETERMINISM_FIXTURE_SCOPE: &str = "s7_fixture_contract_no_full_cli_replay";

/// Full S7 CLI replay producer owner named when this fixture scope stops short.
pub const S7_FULL_CLI_REPLAY_OWNER: &str = "bd-1ryn";

/// Full S7 closure/report adoption owner named when this fixture scope stops short.
pub const S7_FULL_CLOSURE_OWNER: &str = "bd-2v9r";

/// Rep-S7-1 fixture replay byte-identity event.
pub const S7_DETERMINISM_REPLAY_EVENT: &str = "s7.determinism.replay";

/// Rep-S7-2 RouterRng recomputability event.
pub const S7_DETERMINISM_ROUTER_RNG_EVENT: &str = "s7.determinism.router_rng";

/// Rep-S7-3 scaffold parity event.
pub const S7_DETERMINISM_TOPOLOGY_SCAFFOLD_EVENT: &str = "s7.determinism.topology_scaffold";

/// Rep-S7-4 router-collapse sweep replay event.
pub const S7_DETERMINISM_SWEEP_EVENT: &str = "s7.determinism.sweep";

/// Rep-S7-5 matched-bytes pin replay event.
pub const S7_DETERMINISM_MATCHED_BYTES_PIN_EVENT: &str = "s7.determinism.matched_bytes_pin";

/// Rep-S7-6 switch-stats digest replay event.
pub const S7_DETERMINISM_SWITCH_STATS_EVENT: &str = "s7.determinism.switch_stats";

/// Rep-S7-7 Pareto totality event.
pub const S7_DETERMINISM_PARETO_TOTALITY_EVENT: &str = "s7.determinism.pareto_totality";

/// D14 eval/export zero RouterRng draws event.
pub const S7_DETERMINISM_EVAL_EXPORT_ZERO_DRAWS_EVENT: &str =
    "s7.determinism.eval_export_zero_draws";

/// O8 hidden-input isolation event.
pub const S7_DETERMINISM_ENV_ISOLATION_EVENT: &str = "s7.determinism.env_isolation";

/// O9 run-order isolation event.
pub const S7_DETERMINISM_RUN_ORDER_ISOLATION_EVENT: &str = "s7.determinism.run_order_isolation";

/// Mismatch diagnostic event emitted after a failed axis comparison.
pub const S7_DETERMINISM_DIFF_EVENT: &str = "s7.determinism.diff";

/// Dashboard-style summary event for a determinism run.
pub const S7_DETERMINISM_SUMMARY_EVENT: &str = "s7.determinism.summary";

const FIXTURE_VAL_BYTES: &[u8] = b"S7 fixture validation bytes\n";
const FIXTURE_TRAIN_BYTES: &[u8] = b"S7 fixture train bytes\n";
const FIXTURE_PASS_VERSION: SemVer = SemVer::new(0, 2, 0);
const FIXTURE_BURN_VERSION: &str = "fixture-burn-version-pinned-by-lockfile";
const FIXTURE_RUST_TOOLCHAIN: &str = "fixture-rust-toolchain";

/// One axis comparison emitted by the determinism harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterminismAxisReport {
    /// Full tracing event name, e.g. `s7.determinism.replay`.
    pub event_name: &'static str,
    /// Short axis label, e.g. `replay`.
    pub axis: &'static str,
    /// Hash observed in the original run.
    pub original_hash: Hash256,
    /// Hash observed in the replayed run.
    pub replayed_hash: Hash256,
    /// True iff both hashes are equal.
    pub equal: bool,
}

/// One row in an `s7.determinism.diff` side-by-side hash table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeterminismHashPair {
    /// Artifact hash field name.
    pub field: &'static str,
    /// Hash observed in the original artifact.
    pub original_hash: Hash256,
    /// Hash observed in the replayed artifact.
    pub replayed_hash: Hash256,
    /// True iff both field hashes are equal.
    pub equal: bool,
}

impl DeterminismHashPair {
    /// Construct one side-by-side hash row.
    #[must_use]
    pub fn new(field: &'static str, original_hash: Hash256, replayed_hash: Hash256) -> Self {
        Self {
            field,
            original_hash,
            replayed_hash,
            equal: original_hash == replayed_hash,
        }
    }
}

/// A fixture replay product containing the byte surfaces that currently exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7FixtureReplayProduct {
    /// Topology under replay.
    pub topology: S7Topology,
    /// Experiment seed.
    pub seed: u64,
    /// Metadata-free fixture checkpoint bytes.
    pub checkpoint_bytes: Vec<u8>,
    /// Canonical `s7_run_log.v1` bytes.
    pub run_log_bytes: Vec<u8>,
    /// Canonical `s7_score.v1` bytes.
    pub score_bytes: Vec<u8>,
    /// SHA-256 of `checkpoint_bytes`.
    pub checkpoint_sha: Hash256,
    /// `s7_run_log.v1` self-hash.
    pub run_log_self_hash: Hash256,
    /// `s7_score.v1` self-hash.
    pub score_self_hash: Hash256,
}

impl S7FixtureReplayProduct {
    /// Concatenate all replayed byte surfaces with length prefixes.
    #[must_use]
    pub fn combined_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_len_prefixed(&mut bytes, &self.checkpoint_bytes);
        push_len_prefixed(&mut bytes, &self.run_log_bytes);
        push_len_prefixed(&mut bytes, &self.score_bytes);
        bytes
    }

    /// Hash of all replayed byte surfaces.
    #[must_use]
    pub fn combined_hash(&self) -> Hash256 {
        sha256(self.combined_bytes())
    }
}

/// Scaffold parity comparison for Rep-S7-3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScaffoldFingerprint {
    /// Optimizer configuration hash.
    pub optimizer_config_hash: Hash256,
    /// Phase schedule hash.
    pub phase_schedule_hash: Hash256,
    /// Pinned RNG implementation kind.
    pub rng_kind: String,
    /// S7 CPU deterministic device-profile hash.
    pub device_profile_hash: Hash256,
    /// Training corpus hash.
    pub corpus_train_sha: Hash256,
    /// Validation corpus hash.
    pub corpus_val_sha: Hash256,
    /// Charset v1 manifest hash.
    pub charset_v1_sha: Hash256,
    /// BPC chunk size.
    pub bpc_chunk_size: u64,
    /// Training sequence length.
    pub sequence_length: u64,
    /// Training batch size.
    pub batch_size: u64,
    /// Optimizer step count.
    pub optimizer_steps: u64,
    /// Evaluation cadence.
    pub eval_every_steps: u64,
    /// Evaluation subset size.
    pub eval_subset_size: u64,
    /// Burn version pinned by the replay environment.
    pub burn_pinned_version: String,
    /// Dependency lockfile hash.
    pub dependency_lockfile_sha: Hash256,
    /// Rust toolchain hash.
    pub rust_toolchain_hash: Hash256,
    /// Build configuration hash.
    pub build_config_hash: Hash256,
    /// Pass version.
    pub pass_version: SemVer,
    /// Topology hash, permitted to differ between MoE and dense.
    pub model_topology_hash: Hash256,
    /// Router config hash, permitted to differ between MoE and dense.
    pub router_config_hash: Option<Hash256>,
    /// Expert-block config hash, permitted to differ between MoE and dense.
    pub expert_block_config_hash: Option<Hash256>,
}

/// Result of comparing two scaffold fingerprints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldParityReport {
    /// Field names whose differences are allowed by §6.3.
    pub permitted_differences: Vec<&'static str>,
    /// Field names whose differences would violate §6.3.
    pub unpermitted_differences: Vec<&'static str>,
}

impl ScaffoldParityReport {
    /// True iff no unpermitted scaffold fields differed.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.unpermitted_differences.is_empty()
    }
}

/// Current switch-stats replay support exposed by this bead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7FixtureSwitchStatsReplay {
    /// Support marker for the current substrate.
    pub support_level: &'static str,
    /// Per-layer temporal switch digest canonical bytes.
    pub temporal_switch_digest_bytes: Vec<Vec<u8>>,
    /// Per-layer temporal switch digest self-hashes.
    pub temporal_switch_digest_hashes: Vec<Hash256>,
    /// Aggregate `s7_switch_stats.v1` self-hash, absent until the producer exists.
    pub aggregate_switch_stats_self_hash: Option<Hash256>,
    /// Follow-up owners for the full aggregate producer/adoption.
    pub moved_scope_owners: Vec<&'static str>,
}

impl S7FixtureSwitchStatsReplay {
    /// Hash of the digest-level switch-stats replay bytes.
    #[must_use]
    pub fn digest_level_hash(&self) -> Hash256 {
        let mut bytes = Vec::new();
        for digest_bytes in &self.temporal_switch_digest_bytes {
            push_len_prefixed(&mut bytes, digest_bytes);
        }
        sha256(bytes)
    }
}

/// Build the fixture replay product for one topology and seed.
pub fn fixture_replay_product(
    topology: S7Topology,
    seed: u64,
) -> Result<S7FixtureReplayProduct, S7ReplayError> {
    let checkpoint_bytes = fixture_checkpoint_bytes(&topology, seed)?;
    let checkpoint_sha = sha256(&checkpoint_bytes);
    let run_log = fixture_run_log(topology.clone(), seed)?;
    let score = fixture_score(topology.clone(), seed, checkpoint_sha)?;
    let run_log_bytes = run_log.canonical_json_bytes()?;
    let score_bytes = score.canonical_json_bytes()?;

    Ok(S7FixtureReplayProduct {
        topology,
        seed,
        checkpoint_bytes,
        run_log_bytes,
        score_bytes,
        checkpoint_sha,
        run_log_self_hash: run_log.run_log_self_hash,
        score_self_hash: score.score_self_hash,
    })
}

/// Build the fixture scaffold fingerprint for one S7 topology.
pub fn fixture_scaffold_fingerprint(
    topology: S7Topology,
) -> Result<ScaffoldFingerprint, S7ReplayError> {
    Ok(ScaffoldFingerprint {
        optimizer_config_hash: fixture_hash("optimizer_config", "shared")?,
        phase_schedule_hash: fixture_hash("phase_schedule", "shared")?,
        rng_kind: "Pcg64Mcg".to_owned(),
        device_profile_hash: fixture_hash("device_profile", "S7CpuDeterministic")?,
        corpus_train_sha: sha256(FIXTURE_TRAIN_BYTES),
        corpus_val_sha: sha256(FIXTURE_VAL_BYTES),
        charset_v1_sha: fixture_hash("charset_v1", "fixture")?,
        bpc_chunk_size: 256,
        sequence_length: 256,
        batch_size: 32,
        optimizer_steps: 20_000,
        eval_every_steps: 1_000,
        eval_subset_size: 4_096,
        burn_pinned_version: FIXTURE_BURN_VERSION.to_owned(),
        dependency_lockfile_sha: fixture_hash("dependency_lockfile", "Cargo.lock")?,
        rust_toolchain_hash: fixture_hash("rust_toolchain", FIXTURE_RUST_TOOLCHAIN)?,
        build_config_hash: fixture_hash("build_config", "s7")?,
        pass_version: FIXTURE_PASS_VERSION,
        model_topology_hash: fixture_hash("model_topology", topology_name(&topology))?,
        router_config_hash: match topology {
            S7Topology::MoeTiny => Some(fixture_hash("router_config", "moe")?),
            S7Topology::MoeTinyDenseMatched => None,
        },
        expert_block_config_hash: match topology {
            S7Topology::MoeTiny => Some(fixture_hash("expert_block_config", "moe")?),
            S7Topology::MoeTinyDenseMatched => None,
        },
    })
}

/// Compare two scaffold fingerprints using the RFC §6.3 exception list.
#[must_use]
pub fn compare_scaffold_fingerprints(
    moe: &ScaffoldFingerprint,
    dense: &ScaffoldFingerprint,
) -> ScaffoldParityReport {
    let mut permitted_differences = Vec::new();
    let mut unpermitted_differences = Vec::new();

    macro_rules! compare_field {
        ($field:ident, allowed) => {
            if moe.$field != dense.$field {
                permitted_differences.push(stringify!($field));
            }
        };
        ($field:ident) => {
            if moe.$field != dense.$field {
                unpermitted_differences.push(stringify!($field));
            }
        };
    }

    compare_field!(optimizer_config_hash);
    compare_field!(phase_schedule_hash);
    compare_field!(rng_kind);
    compare_field!(device_profile_hash);
    compare_field!(corpus_train_sha);
    compare_field!(corpus_val_sha);
    compare_field!(charset_v1_sha);
    compare_field!(bpc_chunk_size);
    compare_field!(sequence_length);
    compare_field!(batch_size);
    compare_field!(optimizer_steps);
    compare_field!(eval_every_steps);
    compare_field!(eval_subset_size);
    compare_field!(burn_pinned_version);
    compare_field!(dependency_lockfile_sha);
    compare_field!(rust_toolchain_hash);
    compare_field!(build_config_hash);
    compare_field!(pass_version);
    compare_field!(model_topology_hash, allowed);
    compare_field!(router_config_hash, allowed);
    compare_field!(expert_block_config_hash, allowed);

    ScaffoldParityReport {
        permitted_differences,
        unpermitted_differences,
    }
}

/// Run the deterministic fixture lambda-switch sweep for Rep-S7-4.
pub fn fixture_sweep_report(seed: u64) -> Result<RouterCollapseSweepReport, S7ReplayError> {
    let replay = fixture_replay_product(S7Topology::MoeTiny, seed)?;
    let input = LambdaSwitchSweepInput::d11_from_val_eval_subset_bytes(
        seed,
        replay.checkpoint_sha,
        16_000,
        FIXTURE_VAL_BYTES,
    )?;
    Ok(run_lambda_switch_sweep(
        &input,
        &DeterministicFixtureSweepProducer,
    )?)
}

/// Canonical matched-bytes pin bytes from the current S7 policy substrate.
pub fn fixture_matched_bytes_pin_bytes() -> Result<Vec<u8>, S7ReplayError> {
    let pin = canonical_s7_matched_bytes_pin()?;
    pin.verify_self_hash().map_err(S7MatchedBytesError::from)?;
    Ok(canonical_s7_matched_bytes_json_bytes()?)
}

/// Build deterministic digest-level switch-stats replay evidence.
pub fn fixture_switch_stats_replay(seed: u64) -> Result<S7FixtureSwitchStatsReplay, S7ReplayError> {
    let mut temporal_switch_digest_bytes = Vec::new();
    let mut temporal_switch_digest_hashes = Vec::new();
    for layer in 0..4 {
        let digest = fixture_temporal_switch_digest(seed, layer)?
            .with_computed_self_hash()
            .map_err(S7ReplayError::ArtifactSchema)?;
        temporal_switch_digest_hashes.push(digest.digest_self_hash);
        temporal_switch_digest_bytes.push(digest.canonical_json_bytes()?);
    }

    Ok(S7FixtureSwitchStatsReplay {
        support_level: "digest_only_no_aggregate_s7_switch_stats_v1",
        temporal_switch_digest_bytes,
        temporal_switch_digest_hashes,
        aggregate_switch_stats_self_hash: None,
        moved_scope_owners: vec![S7_FULL_CLI_REPLAY_OWNER, S7_FULL_CLOSURE_OWNER],
    })
}

/// Hash the fixture replay product without reading environment variables.
pub fn fixture_replay_hash(topology: S7Topology, seed: u64) -> Result<Hash256, S7ReplayError> {
    Ok(fixture_replay_product(topology, seed)?.combined_hash())
}

/// Compare byte slices, emit the axis event, and return the report.
#[must_use]
pub fn compare_bytes_and_emit(
    event_name: &'static str,
    original_bytes: &[u8],
    replayed_bytes: &[u8],
) -> DeterminismAxisReport {
    let original_hash = sha256(original_bytes);
    let replayed_hash = sha256(replayed_bytes);
    emit_axis_hashes_with_hash_pairs(event_name, original_hash, replayed_hash, &[])
}

/// Compare fixture replay products and emit granular replay-product hash pairs.
#[must_use]
pub fn compare_replay_products_and_emit(
    event_name: &'static str,
    original: &S7FixtureReplayProduct,
    replayed: &S7FixtureReplayProduct,
) -> DeterminismAxisReport {
    let hash_pairs = replay_product_hash_pairs(original, replayed);
    emit_axis_hashes_with_hash_pairs(
        event_name,
        original.combined_hash(),
        replayed.combined_hash(),
        &hash_pairs,
    )
}

/// Side-by-side replay-product hash rows used by mismatch diagnostics.
#[must_use]
pub fn replay_product_hash_pairs(
    original: &S7FixtureReplayProduct,
    replayed: &S7FixtureReplayProduct,
) -> Vec<DeterminismHashPair> {
    vec![
        DeterminismHashPair::new(
            "checkpoint_sha",
            original.checkpoint_sha,
            replayed.checkpoint_sha,
        ),
        DeterminismHashPair::new(
            "run_log_self_hash",
            original.run_log_self_hash,
            replayed.run_log_self_hash,
        ),
        DeterminismHashPair::new(
            "score_self_hash",
            original.score_self_hash,
            replayed.score_self_hash,
        ),
    ]
}

/// Emit one determinism axis event from two precomputed hashes.
#[must_use]
pub fn emit_axis_hashes(
    event_name: &'static str,
    original_hash: Hash256,
    replayed_hash: Hash256,
) -> DeterminismAxisReport {
    emit_axis_hashes_with_hash_pairs(event_name, original_hash, replayed_hash, &[])
}

/// Emit one determinism axis event plus optional granular hash pairs for diffs.
#[must_use]
pub fn emit_axis_hashes_with_hash_pairs(
    event_name: &'static str,
    original_hash: Hash256,
    replayed_hash: Hash256,
    hash_pairs: &[DeterminismHashPair],
) -> DeterminismAxisReport {
    let equal = original_hash == replayed_hash;
    let axis = axis_label(event_name);
    tracing::info!(
        target: S7_LOG_TARGET,
        event_name,
        axis,
        original_hash = %original_hash,
        replayed_hash = %replayed_hash,
        equal,
        support_scope = S7_DETERMINISM_FIXTURE_SCOPE,
        moved_full_cli_replay_to = S7_FULL_CLI_REPLAY_OWNER,
        moved_full_closure_to = S7_FULL_CLOSURE_OWNER,
        "s7 determinism axis"
    );
    if !equal {
        let hash_table = serde_json::to_string(hash_pairs)
            .expect("determinism hash-pair table should serialize");
        let mismatched_hash_fields = hash_pairs
            .iter()
            .filter(|pair| !pair.equal)
            .map(|pair| pair.field)
            .collect::<Vec<_>>();
        tracing::error!(
            target: S7_LOG_TARGET,
            event_name = S7_DETERMINISM_DIFF_EVENT,
            axis,
            source_event_name = event_name,
            original_hash = %original_hash,
            replayed_hash = %replayed_hash,
            hash_table_schema = "s7_determinism_hash_table.v1",
            hash_table = hash_table.as_str(),
            mismatched_hash_fields = ?mismatched_hash_fields,
            "s7 determinism hash mismatch"
        );
    }

    DeterminismAxisReport {
        event_name,
        axis,
        original_hash,
        replayed_hash,
        equal,
    }
}

/// Emit a dashboard-style determinism summary event.
pub fn emit_determinism_summary(reports: &[DeterminismAxisReport]) {
    let axes_failed = reports.iter().filter(|report| !report.equal).count() as u64;
    let axes_passed = reports.len() as u64 - axes_failed;
    let failing_axes = reports
        .iter()
        .filter(|report| !report.equal)
        .map(|report| report.axis)
        .collect::<Vec<_>>();
    tracing::info!(
        target: S7_LOG_TARGET,
        event_name = S7_DETERMINISM_SUMMARY_EVENT,
        axes_passed,
        axes_failed,
        failing_axes = ?failing_axes,
        support_scope = S7_DETERMINISM_FIXTURE_SCOPE,
        "s7 determinism summary"
    );
}

/// Short axis label for a full determinism event name.
#[must_use]
pub fn axis_label(event_name: &'static str) -> &'static str {
    event_name
        .strip_prefix("s7.determinism.")
        .unwrap_or(event_name)
}

/// Errors raised by S7 replay fixture helpers.
#[derive(Debug)]
pub enum S7ReplayError {
    /// S7 artifact schema validation failed.
    ArtifactSchema(gbf_artifact::S7SchemaError),
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// Collapse sweep validation failed.
    CollapseSweep(CollapseSweepError),
    /// Matched-bytes pin emission failed.
    MatchedBytes(S7MatchedBytesError),
    /// Score construction from bytes failed.
    ScoreFromBytes(S7ScoreFromBytesError),
    /// Fixture SafeTensors-like header construction failed.
    CheckpointHeader(serde_json::Error),
    /// Fixture checkpoint payload offsets overflowed.
    CheckpointOffsetOverflow,
}

impl fmt::Display for S7ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArtifactSchema(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::CollapseSweep(error) => write!(f, "{error}"),
            Self::MatchedBytes(error) => write!(f, "{error}"),
            Self::ScoreFromBytes(error) => write!(f, "{error}"),
            Self::CheckpointHeader(error) => {
                write!(f, "failed to encode S7 fixture checkpoint header: {error}")
            }
            Self::CheckpointOffsetOverflow => {
                f.write_str("S7 fixture checkpoint payload offset overflowed")
            }
        }
    }
}

impl std::error::Error for S7ReplayError {}

impl From<gbf_artifact::S7SchemaError> for S7ReplayError {
    fn from(error: gbf_artifact::S7SchemaError) -> Self {
        Self::ArtifactSchema(error)
    }
}

impl From<CanonicalJsonError> for S7ReplayError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<CollapseSweepError> for S7ReplayError {
    fn from(error: CollapseSweepError) -> Self {
        Self::CollapseSweep(error)
    }
}

impl From<S7MatchedBytesError> for S7ReplayError {
    fn from(error: S7MatchedBytesError) -> Self {
        Self::MatchedBytes(error)
    }
}

impl From<S7ScoreFromBytesError> for S7ReplayError {
    fn from(error: S7ScoreFromBytesError) -> Self {
        Self::ScoreFromBytes(error)
    }
}

impl From<serde_json::Error> for S7ReplayError {
    fn from(error: serde_json::Error) -> Self {
        Self::CheckpointHeader(error)
    }
}

fn fixture_run_log(topology: S7Topology, seed: u64) -> Result<S7RunLog, S7ReplayError> {
    let losses = (1..=2)
        .map(|step| {
            Ok((
                step,
                RawLossDiagnostics::new(
                    1.0 + topology_offset(&topology) as f32 + seed as f32 * 0.001,
                    DistillRawDiagnostic::NotAvailable {
                        reason: "fixture-replay-no-teacher-logits".to_owned(),
                        phase: TrainPhase::PhaseA,
                    },
                    0.125,
                    0.0625,
                    0.25,
                )?
                .with_computed_self_hash()?,
            ))
        })
        .collect::<Result<Vec<_>, S7ReplayError>>()?;
    let grad_norms = (1..=2)
        .map(|step| {
            Ok((
                step,
                GradNormSummary::new(0.5 + step as f32 * 0.01, 0.25, 0.125)?,
            ))
        })
        .collect::<Result<Vec<_>, S7ReplayError>>()?;
    let eval_points = vec![(0, fixture_bpc(&topology, seed))];

    Ok(S7RunLog::new(
        seed,
        topology.clone(),
        fixture_hash("train_config", "shared")?,
        fixture_hash("model_topology", topology_name(&topology))?,
        match topology {
            S7Topology::MoeTiny => Some(fixture_hash("router_config", "moe")?),
            S7Topology::MoeTinyDenseMatched => None,
        },
        match topology {
            S7Topology::MoeTiny => Some(fixture_hash("expert_block_config", "moe")?),
            S7Topology::MoeTinyDenseMatched => None,
        },
        fixture_hash("loss_config", "shared")?,
        fixture_hash("phase_schedule", "shared")?,
        Some(fixture_hash(
            "frozen_teacher_checkpoint",
            topology_name(&topology),
        )?),
        losses,
        grad_norms,
        eval_points,
        GradNormSummary::new(0.5, 0.25, 0.125)?,
        S7Completion::DivergedAt { step: 2 },
    )?
    .with_computed_self_hash()?)
}

fn fixture_score(
    topology: S7Topology,
    seed: u64,
    checkpoint_sha: Hash256,
) -> Result<S7ScoreReport, S7ReplayError> {
    let token_count = charset_v1_normalized_token_count(FIXTURE_VAL_BYTES)?;
    let bpc = fixture_bpc(&topology, seed);
    let log2_sum = bpc * token_count as f64;
    Ok(S7ScoreReport::new(
        seed,
        topology,
        checkpoint_sha,
        sha256(FIXTURE_VAL_BYTES),
        token_count,
        log2_sum,
    )?
    .with_computed_self_hash()?)
}

fn fixture_temporal_switch_digest(
    seed: u64,
    layer: u16,
) -> Result<TemporalSwitchDigest, S7ReplayError> {
    let from = ExpertId::new((seed as u16 + layer) % S7_N_EXPERTS);
    let to = ExpertId::new((from.get() + 1) % S7_N_EXPERTS);
    Ok(TemporalSwitchDigest::new(
        LayerId::new(layer),
        S7_N_EXPERTS,
        128 + layer,
        vec![
            TransitionEntry::new(from, to, 80)?,
            TransitionEntry::new(to, from, 64)?,
        ],
    )?)
}

fn fixture_checkpoint_bytes(topology: &S7Topology, seed: u64) -> Result<Vec<u8>, S7ReplayError> {
    let mut tensors = vec![
        FixtureTensor {
            name: "model.embedding.weight",
            bytes: fixture_tensor_payload(topology, seed, "embedding"),
        },
        FixtureTensor {
            name: match topology {
                S7Topology::MoeTiny => "model.layers.0.router.fixture",
                S7Topology::MoeTinyDenseMatched => "model.layers.0.dense_ffn.fixture",
            },
            bytes: fixture_tensor_payload(topology, seed, "topology"),
        },
    ];
    tensors.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));

    let mut header = String::from("{");
    let mut offset = 0usize;
    for (index, tensor) in tensors.iter().enumerate() {
        if index > 0 {
            header.push(',');
        }
        let next_offset = offset
            .checked_add(tensor.bytes.len())
            .ok_or(S7ReplayError::CheckpointOffsetOverflow)?;
        header.push_str(&serde_json::to_string(tensor.name)?);
        header.push_str(r#":{"dtype":"U8","shape":["#);
        header.push_str(&tensor.bytes.len().to_string());
        header.push_str(r#"],"data_offsets":["#);
        header.push_str(&offset.to_string());
        header.push(',');
        header.push_str(&next_offset.to_string());
        header.push_str("]}");
        offset = next_offset;
    }
    header.push('}');

    let aligned_header_len = header.len().next_multiple_of(size_of::<u64>());
    let mut header_bytes = header.into_bytes();
    header_bytes.resize(aligned_header_len, b' ');

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(header_bytes.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&header_bytes);
    for tensor in tensors {
        bytes.extend_from_slice(&tensor.bytes);
    }
    Ok(bytes)
}

fn fixture_tensor_payload(topology: &S7Topology, seed: u64, tensor_kind: &'static str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(
        fixture_hash_preimage("fixture_tensor", topology_name(topology), seed, tensor_kind)
            .as_bytes(),
    );
    sha256(bytes).as_bytes()[..16].to_vec()
}

fn fixture_hash(kind: &'static str, value: &'static str) -> Result<Hash256, S7ReplayError> {
    Ok(
        DomainHash::new("gbf-experiments", "S7FixtureReplay", kind, "1")
            .hash(&FixtureHashMaterial { kind, value })?,
    )
}

fn fixture_hash_preimage(
    scope: &'static str,
    topology: &'static str,
    seed: u64,
    value: &'static str,
) -> String {
    format!("gbf:s7:{scope}:{topology}:{seed}:{value}")
}

fn fixture_bpc(topology: &S7Topology, seed: u64) -> f64 {
    1.0 + topology_offset(topology) + seed as f64 * 0.001
}

const fn topology_offset(topology: &S7Topology) -> f64 {
    match topology {
        S7Topology::MoeTiny => 0.0,
        S7Topology::MoeTinyDenseMatched => 0.05,
    }
}

const fn topology_name(topology: &S7Topology) -> &'static str {
    match topology {
        S7Topology::MoeTiny => "MoeTiny",
        S7Topology::MoeTinyDenseMatched => "MoeTinyDenseMatched",
    }
}

fn push_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

#[derive(Debug)]
struct FixtureTensor {
    name: &'static str,
    bytes: Vec<u8>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureHashMaterial {
    kind: &'static str,
    value: &'static str,
}
