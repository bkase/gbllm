//! S7 derived dense-vs-MoE packet artifact materialization.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gbf_artifact::{
    MatchedBytesPin, MatchedBytesPinError, ParetoVerdict, S7AggregateParityVerdict,
    S7DenseVsMoeComparisonReport, S7ParityVerdict, S7PerSeedComparison, S7SchemaError,
    S7ScoreReport, S7Topology, SweepSummary, SwitchStatsSummary,
};
use gbf_foundation::{CanonicalJsonError, Hash256};
use serde::de::DeserializeOwned;

use crate::s7::run::topology_path_segment;

const S7_COMPARISON_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];
const S7_PARITY_BPC_MARGIN: f64 = 0.05;
const SCORE_BPC_EPSILON: f64 = 1.0e-12;

/// Inputs for deriving the `s7_dense_vs_moe.v1` artifact from materialized scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7ComparisonArtifactInputs {
    /// Packet/repository root containing `experiments/S7/scores/...`.
    pub root: PathBuf,
    /// Verified `s7_matched_bytes_pin.v1` path.
    pub matched_bytes: PathBuf,
    /// Topology hash for the MoE production topology.
    pub moe_topology_hash: Hash256,
    /// Topology hash for the dense matched production topology.
    pub dense_matched_topology_hash: Hash256,
    /// JSON file containing a `SwitchStatsSummary`.
    pub switch_stats_summary: PathBuf,
    /// JSON file containing a `SweepSummary`.
    pub sweep_summary: PathBuf,
    /// Output path for `s7_dense_vs_moe.v1`, relative to `root` unless absolute.
    pub output: PathBuf,
}

/// Materialized comparison output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7MaterializedComparisonArtifact {
    /// Canonical comparison packet path.
    pub comparison_path: PathBuf,
    /// Verified `comparison_self_hash`.
    pub comparison_self_hash: Hash256,
}

/// Derive and materialize the dense-vs-MoE comparison artifact from production score products.
///
/// This helper intentionally consumes already materialized run products. It
/// computes only deterministic aggregate fields from `s7_score.v1` and refuses
/// to invent switch-stat or sweep observations; those summaries must arrive
/// from their owning production producers.
pub fn materialize_dense_vs_moe_comparison(
    inputs: &S7ComparisonArtifactInputs,
) -> Result<S7MaterializedComparisonArtifact, S7ComparisonMaterializeError> {
    reject_zero_hash("moe_topology_hash", inputs.moe_topology_hash)?;
    reject_zero_hash(
        "dense_matched_topology_hash",
        inputs.dense_matched_topology_hash,
    )?;
    if inputs.moe_topology_hash == inputs.dense_matched_topology_hash {
        return Err(S7ComparisonMaterializeError::TopologyHashesNotDistinct);
    }

    let matched_bytes_path = resolve_under_root(&inputs.root, &inputs.matched_bytes);
    let matched_bytes: MatchedBytesPin = read_json_file(&matched_bytes_path)?;
    matched_bytes.verify_self_hash()?;
    reject_zero_hash(
        "matched_bytes_self_hash",
        matched_bytes.matched_bytes_self_hash,
    )?;

    let switch_stats_summary_path = resolve_under_root(&inputs.root, &inputs.switch_stats_summary);
    let switch_stats_summary: SwitchStatsSummary = read_json_file(&switch_stats_summary_path)?;
    switch_stats_summary.validate()?;
    let sweep_summary_path = resolve_under_root(&inputs.root, &inputs.sweep_summary);
    let sweep_summary: SweepSummary = read_json_file(&sweep_summary_path)?;
    sweep_summary.validate()?;

    let mut per_seed = Vec::with_capacity(S7_COMPARISON_SEEDS.len());
    let mut corpus_val_sha: Option<Hash256> = None;
    for seed in S7_COMPARISON_SEEDS {
        let moe = read_score(&inputs.root, S7Topology::MoeTiny, seed)?;
        let dense = read_score(&inputs.root, S7Topology::MoeTinyDenseMatched, seed)?;
        compare_corpus_sha(&mut corpus_val_sha, &moe)?;
        compare_corpus_sha(&mut corpus_val_sha, &dense)?;

        let parity_verdict = parity_verdict(moe.bpc, dense.bpc);
        per_seed.push(S7PerSeedComparison::new(
            seed,
            moe.bpc,
            dense.bpc,
            dense.bpc - moe.bpc,
            parity_verdict,
        )?);
    }

    let median_val_bpc_moe = median_bpc(&per_seed, |entry| entry.val_bpc_moe)?;
    let median_val_bpc_dense = median_bpc(&per_seed, |entry| entry.val_bpc_dense)?;
    let deployed_bytes_total_moe = matched_bytes.b_deployed_total_moe;
    let deployed_bytes_total_dense = matched_bytes.b_deployed_total_dense;
    let tolerance_bytes = matched_bytes.tolerance_bytes;
    let bytes_diff =
        checked_signed_bytes_diff(deployed_bytes_total_dense, deployed_bytes_total_moe)?;
    let bytes_abs_diff = deployed_bytes_total_dense.abs_diff(deployed_bytes_total_moe);
    let bytes_within_tolerance = bytes_abs_diff <= tolerance_bytes;
    let aggregate_parity_verdict = aggregate_parity_verdict(&per_seed, bytes_within_tolerance);
    let pareto_verdict = pareto_verdict(
        median_val_bpc_moe,
        median_val_bpc_dense,
        deployed_bytes_total_moe,
        deployed_bytes_total_dense,
        tolerance_bytes,
    );

    let comparison = S7DenseVsMoeComparisonReport::new(
        inputs.moe_topology_hash,
        inputs.dense_matched_topology_hash,
        matched_bytes,
        per_seed,
        median_val_bpc_moe,
        median_val_bpc_dense,
        deployed_bytes_total_moe,
        deployed_bytes_total_dense,
        bytes_diff,
        bytes_within_tolerance,
        aggregate_parity_verdict,
        pareto_verdict,
        switch_stats_summary,
        sweep_summary,
    )?
    .with_computed_self_hash()?;

    let comparison_path = resolve_under_root(&inputs.root, &inputs.output);
    write_canonical_json(&comparison_path, &comparison.canonical_json_bytes()?)?;

    Ok(S7MaterializedComparisonArtifact {
        comparison_path,
        comparison_self_hash: comparison.comparison_self_hash,
    })
}

fn read_score(
    root: &Path,
    topology: S7Topology,
    seed: u64,
) -> Result<S7ScoreReport, S7ComparisonMaterializeError> {
    let path = root
        .join("experiments/S7/scores")
        .join(topology_path_segment(&topology))
        .join(format!("seed-{seed}"))
        .join("score.json");
    let score: S7ScoreReport = read_json_file(&path)?;
    if score.seed != seed {
        return Err(S7ComparisonMaterializeError::IdentityMismatch {
            artifact: "score",
            field: "seed",
            expected: seed.to_string(),
            observed: score.seed.to_string(),
        });
    }
    if score.topology != topology {
        return Err(S7ComparisonMaterializeError::IdentityMismatch {
            artifact: "score",
            field: "topology",
            expected: topology_path_segment(&topology).to_owned(),
            observed: topology_path_segment(&score.topology).to_owned(),
        });
    }
    reject_zero_hash("score.checkpoint_sha", score.checkpoint_sha)?;
    reject_zero_hash("score.corpus_val_sha", score.corpus_val_sha)?;
    reject_zero_hash("score.score_self_hash", score.score_self_hash)?;
    let expected = score.computed_self_hash()?;
    if score.score_self_hash != expected {
        return Err(S7ComparisonMaterializeError::SelfHashMismatch {
            artifact: "score",
            field: "score_self_hash",
            seed,
            expected,
            observed: score.score_self_hash,
        });
    }
    Ok(score)
}

fn compare_corpus_sha(
    expected: &mut Option<Hash256>,
    score: &S7ScoreReport,
) -> Result<(), S7ComparisonMaterializeError> {
    if let Some(expected) = expected {
        if score.corpus_val_sha != *expected {
            return Err(S7ComparisonMaterializeError::CorpusValMismatch {
                seed: score.seed,
                topology: topology_path_segment(&score.topology),
                expected: *expected,
                observed: score.corpus_val_sha,
            });
        }
    } else {
        *expected = Some(score.corpus_val_sha);
    }
    Ok(())
}

fn parity_verdict(val_bpc_moe: f64, val_bpc_dense: f64) -> S7ParityVerdict {
    if val_bpc_moe < val_bpc_dense - S7_PARITY_BPC_MARGIN {
        S7ParityVerdict::Pass
    } else {
        S7ParityVerdict::Fail
    }
}

fn aggregate_parity_verdict(
    per_seed: &[S7PerSeedComparison],
    bytes_within_tolerance: bool,
) -> S7AggregateParityVerdict {
    if !bytes_within_tolerance {
        S7AggregateParityVerdict::FailBytes
    } else if per_seed.iter().all(|entry| entry.parity_verdict.passed()) {
        S7AggregateParityVerdict::PassClean
    } else {
        S7AggregateParityVerdict::FailParity
    }
}

fn pareto_verdict(
    median_val_bpc_moe: f64,
    median_val_bpc_dense: f64,
    deployed_bytes_total_moe: u64,
    deployed_bytes_total_dense: u64,
    tolerance_bytes: u64,
) -> ParetoVerdict {
    let bpc_equal = f64_close(median_val_bpc_moe, median_val_bpc_dense);
    let bpc_moe_less = median_val_bpc_moe < median_val_bpc_dense && !bpc_equal;
    let bpc_dense_less = median_val_bpc_dense < median_val_bpc_moe && !bpc_equal;
    let bpc_le_moe = bpc_moe_less || bpc_equal;
    let bpc_le_dense = bpc_dense_less || bpc_equal;
    let by_le_moe = deployed_bytes_total_moe <= deployed_bytes_total_dense;
    let by_le_dense = deployed_bytes_total_dense <= deployed_bytes_total_moe;

    if bpc_le_moe
        && by_le_moe
        && (bpc_moe_less || deployed_bytes_total_moe < deployed_bytes_total_dense)
    {
        return ParetoVerdict::MoeDominates;
    }

    if bpc_le_dense
        && by_le_dense
        && (bpc_dense_less || deployed_bytes_total_dense < deployed_bytes_total_moe)
    {
        return ParetoVerdict::DenseDominates;
    }

    if bpc_equal && deployed_bytes_total_moe == deployed_bytes_total_dense {
        return ParetoVerdict::Tied;
    }

    let bytes_equivalent =
        deployed_bytes_total_moe.abs_diff(deployed_bytes_total_dense) <= tolerance_bytes;
    if bytes_equivalent && bpc_moe_less {
        return ParetoVerdict::MoeWinsUnderByteEquivalence;
    }
    if bytes_equivalent && bpc_dense_less {
        return ParetoVerdict::DenseWinsUnderByteEquivalence;
    }
    ParetoVerdict::Incomparable
}

fn median_bpc(
    per_seed: &[S7PerSeedComparison],
    select: impl Fn(&S7PerSeedComparison) -> f64,
) -> Result<f64, S7ComparisonMaterializeError> {
    if per_seed.is_empty() {
        return Err(S7ComparisonMaterializeError::NoScores);
    }
    let mut values = per_seed.iter().map(select).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    Ok(values[values.len() / 2])
}

fn checked_signed_bytes_diff(dense: u64, moe: u64) -> Result<i64, S7ComparisonMaterializeError> {
    let diff = i128::from(dense) - i128::from(moe);
    i64::try_from(diff).map_err(|_| S7ComparisonMaterializeError::BytesDiffOverflow { dense, moe })
}

fn f64_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= SCORE_BPC_EPSILON
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, S7ComparisonMaterializeError> {
    let bytes = fs::read(path).map_err(|source| S7ComparisonMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| S7ComparisonMaterializeError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn write_canonical_json(path: &Path, bytes: &[u8]) -> Result<(), S7ComparisonMaterializeError> {
    let mut bytes = bytes.to_vec();
    bytes.push(b'\n');
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), S7ComparisonMaterializeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| S7ComparisonMaterializeError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| S7ComparisonMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn resolve_under_root(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

fn reject_zero_hash(
    field: &'static str,
    hash: Hash256,
) -> Result<(), S7ComparisonMaterializeError> {
    if hash == Hash256::ZERO {
        Err(S7ComparisonMaterializeError::ZeroHash { field })
    } else {
        Ok(())
    }
}

/// Errors raised while materializing the S7 dense-vs-MoE comparison artifact.
#[derive(Debug)]
pub enum S7ComparisonMaterializeError {
    /// Filesystem I/O failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source error.
        source: io::Error,
    },
    /// JSON decoding failed.
    Json {
        /// Path being decoded.
        path: String,
        /// Source error.
        source: serde_json::Error,
    },
    /// Artifact schema validation failed.
    ArtifactSchema(S7SchemaError),
    /// Matched-bytes pin validation failed.
    MatchedBytes(MatchedBytesPinError),
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
    /// A required production lineage hash used the all-zero test sentinel.
    ZeroHash {
        /// Field label.
        field: &'static str,
    },
    /// MoE and dense topology hashes were identical.
    TopologyHashesNotDistinct,
    /// Artifact identity did not match the requested seed/topology.
    IdentityMismatch {
        /// Artifact label.
        artifact: &'static str,
        /// Field label.
        field: &'static str,
        /// Expected value.
        expected: String,
        /// Observed value.
        observed: String,
    },
    /// A self-hash did not match the artifact payload.
    SelfHashMismatch {
        /// Artifact label.
        artifact: &'static str,
        /// Self-hash field.
        field: &'static str,
        /// Seed associated with the artifact.
        seed: u64,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// Score reports used different validation corpora.
    CorpusValMismatch {
        /// Score seed.
        seed: u64,
        /// Score topology.
        topology: &'static str,
        /// Expected corpus hash.
        expected: Hash256,
        /// Observed corpus hash.
        observed: Hash256,
    },
    /// No score rows were available for median computation.
    NoScores,
    /// Dense-minus-MoE byte arithmetic did not fit the schema.
    BytesDiffOverflow {
        /// Dense deployed bytes.
        dense: u64,
        /// MoE deployed bytes.
        moe: u64,
    },
}

impl fmt::Display for S7ComparisonMaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json { path, source } => write!(f, "{path}: invalid JSON: {source}"),
            Self::ArtifactSchema(error) => write!(f, "{error}"),
            Self::MatchedBytes(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::ZeroHash { field } => {
                write!(
                    f,
                    "S7 comparison {field} must not use the all-zero test sentinel"
                )
            }
            Self::TopologyHashesNotDistinct => {
                f.write_str("S7 comparison topology hashes must be distinct")
            }
            Self::IdentityMismatch {
                artifact,
                field,
                expected,
                observed,
            } => write!(
                f,
                "S7 {artifact} {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::SelfHashMismatch {
                artifact,
                field,
                seed,
                expected,
                observed,
            } => write!(
                f,
                "S7 {artifact} seed {seed} {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::CorpusValMismatch {
                seed,
                topology,
                expected,
                observed,
            } => write!(
                f,
                "S7 score seed {seed} topology {topology} corpus_val_sha mismatch: expected {expected}, observed {observed}"
            ),
            Self::NoScores => f.write_str("S7 comparison requires score rows"),
            Self::BytesDiffOverflow { dense, moe } => write!(
                f,
                "S7 comparison dense-minus-MoE byte diff overflowed: dense={dense}, moe={moe}"
            ),
        }
    }
}

impl std::error::Error for S7ComparisonMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::ArtifactSchema(error) => Some(error),
            Self::MatchedBytes(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::ZeroHash { .. }
            | Self::TopologyHashesNotDistinct
            | Self::IdentityMismatch { .. }
            | Self::SelfHashMismatch { .. }
            | Self::CorpusValMismatch { .. }
            | Self::NoScores
            | Self::BytesDiffOverflow { .. } => None,
        }
    }
}

impl From<S7SchemaError> for S7ComparisonMaterializeError {
    fn from(error: S7SchemaError) -> Self {
        Self::ArtifactSchema(error)
    }
}

impl From<MatchedBytesPinError> for S7ComparisonMaterializeError {
    fn from(error: MatchedBytesPinError) -> Self {
        Self::MatchedBytes(error)
    }
}

impl From<CanonicalJsonError> for S7ComparisonMaterializeError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
