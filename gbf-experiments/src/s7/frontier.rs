//! S7 Pareto frontier artifact derivation.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gbf_artifact::{
    S7DenseVsMoeComparisonReport, S7ProjectedFit, S7SchemaError, S7ScoreReport, S7Topology,
};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::s7::run::topology_path_segment;

/// Public schema id for the S7 frontier report.
pub const S7_FRONTIER_SCHEMA: &str = "s7_frontier.v1";

const S7_FRONTIER_SELF_HASH_FIELD: &str = "frontier_self_hash";
const S7_FRONTIER_SCHEMA_VERSION: &str = "1";
const S7_FRONTIER_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];
const BPC_EPSILON: f64 = 1.0e-12;

/// Inputs for deriving `s7_frontier.v1` from materialized comparison evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7FrontierArtifactInputs {
    /// Packet/repository root containing comparison and score artifacts.
    pub root: PathBuf,
    /// Materialized `s7_dense_vs_moe.v1` comparison path.
    pub comparison: PathBuf,
    /// JSON object containing MoE conformance evidence.
    pub moe_conformance: PathBuf,
    /// JSON object containing dense-matched conformance evidence.
    pub dense_conformance: PathBuf,
    /// MoE deployed-byte fit per deployment block.
    pub moe_deployed_bytes_per_block: Vec<u64>,
    /// Dense-matched deployed-byte fit per deployment block.
    pub dense_deployed_bytes_per_block: Vec<u64>,
    /// Optional JSON value containing MoE schedule-cost evidence.
    pub moe_schedule_cost: Option<PathBuf>,
    /// Optional JSON value containing dense-matched schedule-cost evidence.
    pub dense_schedule_cost: Option<PathBuf>,
    /// Output path for `s7_frontier.v1`, relative to `root` unless absolute.
    pub output: PathBuf,
}

/// Materialized frontier output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7MaterializedFrontierArtifact {
    /// Canonical frontier packet path.
    pub frontier_path: PathBuf,
    /// Verified `frontier_self_hash`.
    pub frontier_self_hash: Hash256,
}

/// Validation-quality fields carried by each frontier point.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7FrontierQuality {
    /// Median validation bpc over the five S7 seeds.
    pub median_val_bpc: f64,
    /// Validation bpc per seed, ordered by seed 0 through 4.
    pub per_seed_val_bpc: Vec<f64>,
}

/// One topology point on the S7 frontier.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7FrontierPoint {
    /// S7 topology represented by this point.
    pub topology: S7Topology,
    /// Checkpoint hash for the median-bpc seed.
    pub checkpoint_sha: Hash256,
    /// Quality summary for the topology.
    pub quality: S7FrontierQuality,
    /// Conformance summary supplied by the producer that evaluated this topology.
    pub conformance: Value,
    /// Projected deployable byte fit for the topology.
    pub projected_fit: S7ProjectedFit,
    /// Optional schedule-cost evidence supplied by the producer.
    pub schedule_cost: Value,
}

/// S7 frontier report consumed by the final closure packet.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S7FrontierReport {
    /// Schema literal.
    pub schema: String,
    /// Frontier points for MoE and dense-matched topology.
    pub points: Vec<S7FrontierPoint>,
    /// Pareto verdict derived by `s7_dense_vs_moe.v1`.
    pub pareto_verdict: gbf_artifact::ParetoVerdict,
    /// Self-hash over canonical report bytes with this field omitted.
    pub frontier_self_hash: Hash256,
}

impl S7FrontierReport {
    /// Domain used for canonical self-hashing.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "S7FrontierReport",
            S7_FRONTIER_SCHEMA,
            S7_FRONTIER_SCHEMA_VERSION,
        )
    }

    /// Construct a frontier report.
    pub fn new(
        points: Vec<S7FrontierPoint>,
        pareto_verdict: gbf_artifact::ParetoVerdict,
    ) -> Result<Self, S7FrontierMaterializeError> {
        let report = Self {
            schema: S7_FRONTIER_SCHEMA.to_owned(),
            points,
            pareto_verdict,
            frontier_self_hash: Hash256::ZERO,
        };
        report.validate()?;
        report.with_computed_self_hash()
    }

    /// Validate frontier invariants that do not require re-running producers.
    pub fn validate(&self) -> Result<(), S7FrontierMaterializeError> {
        if self.schema != S7_FRONTIER_SCHEMA {
            return Err(S7FrontierMaterializeError::UnexpectedSchema {
                observed: self.schema.clone(),
            });
        }
        if self.points.len() != 2 {
            return Err(S7FrontierMaterializeError::FrontierPointCount {
                observed: self.points.len(),
            });
        }
        let mut saw_moe = false;
        let mut saw_dense = false;
        for point in &self.points {
            match point.topology {
                S7Topology::MoeTiny => saw_moe = true,
                S7Topology::MoeTinyDenseMatched => saw_dense = true,
            }
            reject_zero_hash("frontier point checkpoint_sha", point.checkpoint_sha)?;
            validate_quality(&point.quality)?;
            validate_conformance(&point.conformance, &point.topology)?;
            point.projected_fit.validate()?;
            validate_schedule_cost(&point.schedule_cost)?;
        }
        if !saw_moe || !saw_dense {
            return Err(S7FrontierMaterializeError::FrontierTopologyCoverage);
        }
        Ok(())
    }

    /// Compute the canonical self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S7FrontierMaterializeError> {
        Ok(gbf_foundation::self_hash_omitting_fields(
            Self::domain(),
            self,
            S7_FRONTIER_SELF_HASH_FIELD,
            &[],
        )?)
    }

    /// Return a copy with `frontier_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S7FrontierMaterializeError> {
        self.frontier_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    /// Canonical JSON bytes for this report.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S7FrontierMaterializeError> {
        Ok(CanonicalJson::to_vec(self)?)
    }
}

/// Derive and materialize the S7 frontier artifact.
///
/// This helper consumes the already materialized comparison and score products.
/// It derives medians and median-seed checkpoints, while requiring producer
/// conformance evidence and per-block deployable byte fits as explicit inputs.
pub fn materialize_s7_frontier(
    inputs: &S7FrontierArtifactInputs,
) -> Result<S7MaterializedFrontierArtifact, S7FrontierMaterializeError> {
    let comparison_path = resolve_under_root(&inputs.root, &inputs.comparison);
    let comparison: S7DenseVsMoeComparisonReport = read_json_file(&comparison_path)?;
    comparison.verify_self_hash()?;

    let moe_scores = read_scores(&inputs.root, S7Topology::MoeTiny)?;
    let dense_scores = read_scores(&inputs.root, S7Topology::MoeTinyDenseMatched)?;
    let moe_point = frontier_point(
        &inputs.root,
        &comparison,
        S7Topology::MoeTiny,
        &moe_scores,
        inputs.moe_deployed_bytes_per_block.clone(),
        &inputs.moe_conformance,
        &inputs.moe_schedule_cost,
    )?;
    let dense_point = frontier_point(
        &inputs.root,
        &comparison,
        S7Topology::MoeTinyDenseMatched,
        &dense_scores,
        inputs.dense_deployed_bytes_per_block.clone(),
        &inputs.dense_conformance,
        &inputs.dense_schedule_cost,
    )?;
    let frontier = S7FrontierReport::new(vec![moe_point, dense_point], comparison.pareto_verdict)?;

    let frontier_path = resolve_under_root(&inputs.root, &inputs.output);
    write_canonical_json(&frontier_path, &frontier.canonical_json_bytes()?)?;

    Ok(S7MaterializedFrontierArtifact {
        frontier_path,
        frontier_self_hash: frontier.frontier_self_hash,
    })
}

fn frontier_point(
    root: &Path,
    comparison: &S7DenseVsMoeComparisonReport,
    topology: S7Topology,
    scores: &[S7ScoreReport],
    deployed_bytes_per_block: Vec<u64>,
    conformance_path: &Path,
    schedule_cost_path: &Option<PathBuf>,
) -> Result<S7FrontierPoint, S7FrontierMaterializeError> {
    let (per_seed_val_bpc, median_val_bpc, deployed_bytes_total) = match topology {
        S7Topology::MoeTiny => (
            comparison
                .per_seed
                .iter()
                .map(|entry| entry.val_bpc_moe)
                .collect::<Vec<_>>(),
            comparison.median_val_bpc_moe,
            comparison.deployed_bytes_total_moe,
        ),
        S7Topology::MoeTinyDenseMatched => (
            comparison
                .per_seed
                .iter()
                .map(|entry| entry.val_bpc_dense)
                .collect::<Vec<_>>(),
            comparison.median_val_bpc_dense,
            comparison.deployed_bytes_total_dense,
        ),
    };
    for (index, score) in scores.iter().enumerate() {
        let expected = per_seed_val_bpc[index];
        if !f64_close(score.bpc, expected) {
            return Err(S7FrontierMaterializeError::ComparisonScoreMismatch {
                topology: topology_path_segment(&topology),
                seed: score.seed,
                expected,
                observed: score.bpc,
            });
        }
    }
    let median_score = median_score(scores)?;
    if !f64_close(median_score.bpc, median_val_bpc) {
        return Err(S7FrontierMaterializeError::MedianScoreMismatch {
            topology: topology_path_segment(&topology),
            expected: median_val_bpc,
            observed: median_score.bpc,
        });
    }
    let conformance = read_json_value(&resolve_under_root(root, conformance_path))?;
    validate_conformance(&conformance, &topology)?;
    let schedule_cost = match schedule_cost_path {
        Some(path) => read_json_value(&resolve_under_root(root, path))?,
        None => Value::Null,
    };
    validate_schedule_cost(&schedule_cost)?;
    Ok(S7FrontierPoint {
        topology,
        checkpoint_sha: median_score.checkpoint_sha,
        quality: S7FrontierQuality {
            median_val_bpc,
            per_seed_val_bpc,
        },
        conformance,
        projected_fit: S7ProjectedFit::new(deployed_bytes_total, deployed_bytes_per_block)?,
        schedule_cost,
    })
}

fn read_scores(
    root: &Path,
    topology: S7Topology,
) -> Result<Vec<S7ScoreReport>, S7FrontierMaterializeError> {
    let mut scores = Vec::with_capacity(S7_FRONTIER_SEEDS.len());
    for seed in S7_FRONTIER_SEEDS {
        let path = root
            .join("experiments/S7/scores")
            .join(topology_path_segment(&topology))
            .join(format!("seed-{seed}"))
            .join("score.json");
        let score: S7ScoreReport = read_json_file(&path)?;
        if score.seed != seed {
            return Err(S7FrontierMaterializeError::ScoreIdentityMismatch {
                topology: topology_path_segment(&topology),
                field: "seed",
                expected: seed.to_string(),
                observed: score.seed.to_string(),
            });
        }
        if score.topology != topology {
            return Err(S7FrontierMaterializeError::ScoreIdentityMismatch {
                topology: topology_path_segment(&topology),
                field: "topology",
                expected: topology_path_segment(&topology).to_owned(),
                observed: topology_path_segment(&score.topology).to_owned(),
            });
        }
        reject_zero_hash("score.checkpoint_sha", score.checkpoint_sha)?;
        reject_zero_hash("score.score_self_hash", score.score_self_hash)?;
        let expected = score.computed_self_hash()?;
        if score.score_self_hash != expected {
            return Err(S7FrontierMaterializeError::ScoreSelfHashMismatch {
                topology: topology_path_segment(&topology),
                seed,
                expected,
                observed: score.score_self_hash,
            });
        }
        scores.push(score);
    }
    Ok(scores)
}

fn median_score(scores: &[S7ScoreReport]) -> Result<&S7ScoreReport, S7FrontierMaterializeError> {
    if scores.len() != S7_FRONTIER_SEEDS.len() {
        return Err(S7FrontierMaterializeError::ScoreCount {
            observed: scores.len(),
        });
    }
    let mut scores = scores.iter().collect::<Vec<_>>();
    scores.sort_by(|left, right| {
        left.bpc
            .total_cmp(&right.bpc)
            .then_with(|| left.seed.cmp(&right.seed))
    });
    Ok(scores[scores.len() / 2])
}

fn validate_quality(quality: &S7FrontierQuality) -> Result<(), S7FrontierMaterializeError> {
    validate_finite_nonnegative("quality.median_val_bpc", quality.median_val_bpc)?;
    if quality.per_seed_val_bpc.len() != S7_FRONTIER_SEEDS.len() {
        return Err(S7FrontierMaterializeError::QualitySeedCount {
            observed: quality.per_seed_val_bpc.len(),
        });
    }
    for value in &quality.per_seed_val_bpc {
        validate_finite_nonnegative("quality.per_seed_val_bpc", *value)?;
    }
    Ok(())
}

fn validate_conformance(
    conformance: &Value,
    topology: &S7Topology,
) -> Result<(), S7FrontierMaterializeError> {
    if conformance
        .as_object()
        .is_none_or(|object| object.is_empty())
    {
        return Err(S7FrontierMaterializeError::InvalidConformance {
            topology: topology_path_segment(topology),
        });
    }
    Ok(())
}

fn validate_schedule_cost(value: &Value) -> Result<(), S7FrontierMaterializeError> {
    if matches!(value, Value::Null | Value::Object(_)) {
        Ok(())
    } else {
        Err(S7FrontierMaterializeError::InvalidScheduleCost)
    }
}

fn validate_finite_nonnegative(
    field: &'static str,
    value: f64,
) -> Result<(), S7FrontierMaterializeError> {
    if !value.is_finite() {
        return Err(S7FrontierMaterializeError::NonFiniteBpc { field, value });
    }
    if value < 0.0 {
        return Err(S7FrontierMaterializeError::NegativeBpc { field, value });
    }
    Ok(())
}

fn reject_zero_hash(field: &'static str, hash: Hash256) -> Result<(), S7FrontierMaterializeError> {
    if hash == Hash256::ZERO {
        Err(S7FrontierMaterializeError::ZeroHash { field })
    } else {
        Ok(())
    }
}

fn f64_close(left: f64, right: f64) -> bool {
    (left - right).abs() <= BPC_EPSILON
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, S7FrontierMaterializeError> {
    let bytes = fs::read(path).map_err(|source| S7FrontierMaterializeError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| S7FrontierMaterializeError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn read_json_value(path: &Path) -> Result<Value, S7FrontierMaterializeError> {
    read_json_file(path)
}

fn write_canonical_json(path: &Path, bytes: &[u8]) -> Result<(), S7FrontierMaterializeError> {
    let mut bytes = bytes.to_vec();
    bytes.push(b'\n');
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), S7FrontierMaterializeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| S7FrontierMaterializeError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| S7FrontierMaterializeError::Io {
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

/// Errors raised while materializing the S7 frontier artifact.
#[derive(Debug)]
pub enum S7FrontierMaterializeError {
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
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// The schema literal did not match `s7_frontier.v1`.
    UnexpectedSchema {
        /// Observed schema literal.
        observed: String,
    },
    /// A required production lineage hash used the all-zero test sentinel.
    ZeroHash {
        /// Field label.
        field: &'static str,
    },
    /// Frontier report did not contain exactly two points.
    FrontierPointCount {
        /// Observed point count.
        observed: usize,
    },
    /// Frontier points did not cover both S7 topologies.
    FrontierTopologyCoverage,
    /// Frontier quality did not contain exactly five per-seed BPCs.
    QualitySeedCount {
        /// Observed value count.
        observed: usize,
    },
    /// Score reports did not contain exactly five seeds.
    ScoreCount {
        /// Observed score count.
        observed: usize,
    },
    /// A bpc value was not finite.
    NonFiniteBpc {
        /// Field label.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// A bpc value was negative.
    NegativeBpc {
        /// Field label.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// Score identity did not match the requested topology/seed.
    ScoreIdentityMismatch {
        /// Requested topology.
        topology: &'static str,
        /// Field label.
        field: &'static str,
        /// Expected value.
        expected: String,
        /// Observed value.
        observed: String,
    },
    /// Score self-hash did not match the payload.
    ScoreSelfHashMismatch {
        /// Score topology.
        topology: &'static str,
        /// Score seed.
        seed: u64,
        /// Expected self-hash.
        expected: Hash256,
        /// Observed self-hash.
        observed: Hash256,
    },
    /// Score bpc disagreed with the already materialized comparison.
    ComparisonScoreMismatch {
        /// Score topology.
        topology: &'static str,
        /// Score seed.
        seed: u64,
        /// Expected bpc from comparison.
        expected: f64,
        /// Observed bpc from score.
        observed: f64,
    },
    /// The median-score checkpoint did not match the comparison median bpc.
    MedianScoreMismatch {
        /// Score topology.
        topology: &'static str,
        /// Expected median bpc from comparison.
        expected: f64,
        /// Observed score bpc.
        observed: f64,
    },
    /// Conformance input was not a non-empty JSON object.
    InvalidConformance {
        /// Frontier topology.
        topology: &'static str,
    },
    /// Schedule-cost input was neither null nor an object.
    InvalidScheduleCost,
}

impl fmt::Display for S7FrontierMaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Json { path, source } => write!(f, "{path}: invalid JSON: {source}"),
            Self::ArtifactSchema(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::UnexpectedSchema { observed } => write!(
                f,
                "unexpected frontier schema: expected {S7_FRONTIER_SCHEMA}, observed {observed}"
            ),
            Self::ZeroHash { field } => {
                write!(
                    f,
                    "S7 frontier {field} must not use the all-zero test sentinel"
                )
            }
            Self::FrontierPointCount { observed } => {
                write!(f, "S7 frontier requires two points, observed {observed}")
            }
            Self::FrontierTopologyCoverage => {
                f.write_str("S7 frontier points must cover MoeTiny and MoeTinyDenseMatched")
            }
            Self::QualitySeedCount { observed } => {
                write!(
                    f,
                    "S7 frontier quality requires five per-seed BPCs, observed {observed}"
                )
            }
            Self::ScoreCount { observed } => {
                write!(
                    f,
                    "S7 frontier requires five score reports, observed {observed}"
                )
            }
            Self::NonFiniteBpc { field, value } => {
                write!(f, "{field} must be finite, observed {value}")
            }
            Self::NegativeBpc { field, value } => {
                write!(f, "{field} must be non-negative, observed {value}")
            }
            Self::ScoreIdentityMismatch {
                topology,
                field,
                expected,
                observed,
            } => write!(
                f,
                "S7 frontier score {topology} {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::ScoreSelfHashMismatch {
                topology,
                seed,
                expected,
                observed,
            } => write!(
                f,
                "S7 frontier score {topology} seed {seed} score_self_hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ComparisonScoreMismatch {
                topology,
                seed,
                expected,
                observed,
            } => write!(
                f,
                "S7 frontier score {topology} seed {seed} bpc mismatch with comparison: expected {expected}, observed {observed}"
            ),
            Self::MedianScoreMismatch {
                topology,
                expected,
                observed,
            } => write!(
                f,
                "S7 frontier median score {topology} mismatch: expected {expected}, observed {observed}"
            ),
            Self::InvalidConformance { topology } => write!(
                f,
                "S7 frontier conformance for {topology} must be a non-empty object"
            ),
            Self::InvalidScheduleCost => {
                f.write_str("S7 frontier schedule_cost must be null or an object")
            }
        }
    }
}

impl std::error::Error for S7FrontierMaterializeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::ArtifactSchema(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::UnexpectedSchema { .. }
            | Self::ZeroHash { .. }
            | Self::FrontierPointCount { .. }
            | Self::FrontierTopologyCoverage
            | Self::QualitySeedCount { .. }
            | Self::ScoreCount { .. }
            | Self::NonFiniteBpc { .. }
            | Self::NegativeBpc { .. }
            | Self::ScoreIdentityMismatch { .. }
            | Self::ScoreSelfHashMismatch { .. }
            | Self::ComparisonScoreMismatch { .. }
            | Self::MedianScoreMismatch { .. }
            | Self::InvalidConformance { .. }
            | Self::InvalidScheduleCost => None,
        }
    }
}

impl From<S7SchemaError> for S7FrontierMaterializeError {
    fn from(error: S7SchemaError) -> Self {
        Self::ArtifactSchema(error)
    }
}

impl From<CanonicalJsonError> for S7FrontierMaterializeError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
