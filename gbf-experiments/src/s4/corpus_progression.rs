//! S4 TinyStories-to-Gutenberg corpus progression artifact surface.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use gbf_train::phase::TrainPhaseKind;
use serde::{Deserialize, Serialize};

use crate::S4_LOG_TARGET;
use crate::s4::promote::{PromotionGateError, PromotionGateProduct};
use crate::s4::schema::{
    S4_CANONICAL_SEEDS, S4_OPTIMIZER_STEPS_GUTENBERG, S4SchemaError,
    validate_s4_canonical_seed_list,
};

/// Schema id for the S4 corpus progression report.
pub const S4_CORPUS_PROGRESSION_SCHEMA: &str = "s4_corpus_progression.v1";

/// Canonical path for the S4 corpus progression artifact.
pub const S4_CORPUS_PROGRESSION_PATH: &str = "experiments/S4/corpus_progression/schedule.json";

/// RFC-pinned schedule version for the S4 TinyStories-to-Gutenberg instance.
pub const S4_CORPUS_PROGRESSION_SCHEDULE_VERSION: &str = "s4.v1";

/// RFC-pinned gate label for the only S4 corpus progression edge.
pub const S4_CORPUS_PROGRESSION_GATE_TS_TO_GUTENBERG: &str = "G_TS->Gutenberg";

/// Structured event emitted after `s4_corpus_progression.v1` is written.
pub const S4_CORPUS_PROGRESSION_EMIT_EVENT: &str = "s4_corpus_progression_emit";

const PRODUCT_SCHEMA_VERSION: &str = "1";
const SCHEDULE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "CorpusProgressionScheduleSnapshot",
    S4_CORPUS_PROGRESSION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const REPORT_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4CorpusProgressionReport",
    S4_CORPUS_PROGRESSION_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);

/// Corpus labels used by the S4 progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S4CorpusProgressionCorpus {
    /// TinyStories source corpus.
    TinyStories,
    /// Project Gutenberg target corpus.
    Gutenberg,
}

/// Ordered corpus identity entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4CorpusProgressionCorpusRef {
    /// Corpus label.
    pub corpus: S4CorpusProgressionCorpus,
    /// Self-hash of the corpus manifest/artifact.
    pub corpus_self_hash: Hash256,
}

impl S4CorpusProgressionCorpusRef {
    /// Construct a corpus identity entry.
    #[must_use]
    pub const fn new(corpus: S4CorpusProgressionCorpus, corpus_self_hash: Hash256) -> Self {
        Self {
            corpus,
            corpus_self_hash,
        }
    }
}

/// Directed edge in the S4 corpus progression schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4CorpusProgressionEdge {
    /// Source corpus.
    pub from: S4CorpusProgressionCorpus,
    /// Target corpus.
    pub to: S4CorpusProgressionCorpus,
    /// Gate label that authorizes the transition.
    pub gate: String,
}

/// Phase-boundary interval in the declarative corpus progression schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4CorpusProgressionPhaseBoundary {
    /// Active corpus over this interval.
    pub active_corpus: S4CorpusProgressionCorpus,
    /// Phase controls active over this interval.
    pub train_phase: TrainPhaseKind,
    /// Start in the schedule-local progression-step namespace.
    pub start_progression_step: u64,
    /// Exclusive end in the schedule-local progression-step namespace.
    pub end_progression_step_exclusive: u64,
}

/// Declarative S4 CorpusProgressionSchedule snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusProgressionScheduleSnapshot {
    /// Schedule schema version, always `s4.v1`.
    pub schedule_version: String,
    /// Ordered corpus identities in progression order.
    pub ordered_corpora: Vec<S4CorpusProgressionCorpusRef>,
    /// Corpus progression edges.
    pub edges: Vec<S4CorpusProgressionEdge>,
    /// Active corpus at the start of the schedule.
    pub active_corpus_at_start: S4CorpusProgressionCorpus,
    /// Active corpus at the end of the schedule.
    pub active_corpus_at_finish: S4CorpusProgressionCorpus,
    /// Canonical S4 seed list audited by this schedule.
    pub seed_list: Vec<u64>,
    /// Non-overlapping phase/corpus intervals.
    pub phase_boundaries: Vec<S4CorpusProgressionPhaseBoundary>,
    /// Self-hash over the schedule with this field omitted.
    pub schedule_self_hash: Hash256,
}

impl CorpusProgressionScheduleSnapshot {
    /// Construct the pinned S4 TinyStories-to-Gutenberg schedule.
    #[must_use]
    pub fn pinned(
        tinystories_manifest_self_hash: Hash256,
        gutenberg_manifest_self_hash: Hash256,
    ) -> Self {
        Self {
            schedule_version: S4_CORPUS_PROGRESSION_SCHEDULE_VERSION.to_owned(),
            ordered_corpora: vec![
                S4CorpusProgressionCorpusRef::new(
                    S4CorpusProgressionCorpus::TinyStories,
                    tinystories_manifest_self_hash,
                ),
                S4CorpusProgressionCorpusRef::new(
                    S4CorpusProgressionCorpus::Gutenberg,
                    gutenberg_manifest_self_hash,
                ),
            ],
            edges: vec![S4CorpusProgressionEdge {
                from: S4CorpusProgressionCorpus::TinyStories,
                to: S4CorpusProgressionCorpus::Gutenberg,
                gate: S4_CORPUS_PROGRESSION_GATE_TS_TO_GUTENBERG.to_owned(),
            }],
            active_corpus_at_start: S4CorpusProgressionCorpus::TinyStories,
            active_corpus_at_finish: S4CorpusProgressionCorpus::Gutenberg,
            seed_list: S4_CANONICAL_SEEDS.to_vec(),
            phase_boundaries: vec![
                S4CorpusProgressionPhaseBoundary {
                    active_corpus: S4CorpusProgressionCorpus::TinyStories,
                    train_phase: TrainPhaseKind::FullNumericQat,
                    start_progression_step: 0,
                    end_progression_step_exclusive: 1,
                },
                S4CorpusProgressionPhaseBoundary {
                    active_corpus: S4CorpusProgressionCorpus::Gutenberg,
                    train_phase: TrainPhaseKind::FullNumericQat,
                    start_progression_step: 1,
                    end_progression_step_exclusive: S4_OPTIMIZER_STEPS_GUTENBERG + 1,
                },
            ],
            schedule_self_hash: Hash256::ZERO,
        }
    }

    /// Return a copy with `schedule_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S4CorpusProgressionError> {
        self.schedule_self_hash = Hash256::ZERO;
        self.validate_structure_without_self_hash()?;
        self.schedule_self_hash = self.compute_self_hash()?;
        Ok(self)
    }

    /// Compute the schedule self-hash with `schedule_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4CorpusProgressionError> {
        self.validate_structure_without_self_hash()?;
        compute_self_hash(self, "schedule_self_hash", SCHEDULE_DOMAIN)
    }

    /// Validate structure and schedule self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4CorpusProgressionError> {
        self.validate_structure_without_self_hash()?;
        let expected = self.compute_self_hash()?;
        if expected != self.schedule_self_hash {
            return Err(S4CorpusProgressionError::SelfHashMismatch {
                field: "schedule_self_hash",
                expected,
                observed: self.schedule_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure_without_self_hash(&self) -> Result<(), S4CorpusProgressionError> {
        if self.schedule_version != S4_CORPUS_PROGRESSION_SCHEDULE_VERSION {
            return Err(S4CorpusProgressionError::InvalidScheduleVersion {
                observed: self.schedule_version.clone(),
            });
        }
        validate_ordered_corpora(&self.ordered_corpora)?;
        validate_edges(&self.edges)?;
        if self.active_corpus_at_start != S4CorpusProgressionCorpus::TinyStories {
            return Err(S4CorpusProgressionError::InvalidActiveCorpus {
                field: "active_corpus_at_start",
                expected: S4CorpusProgressionCorpus::TinyStories,
                observed: self.active_corpus_at_start,
            });
        }
        if self.active_corpus_at_finish != S4CorpusProgressionCorpus::Gutenberg {
            return Err(S4CorpusProgressionError::InvalidActiveCorpus {
                field: "active_corpus_at_finish",
                expected: S4CorpusProgressionCorpus::Gutenberg,
                observed: self.active_corpus_at_finish,
            });
        }
        validate_s4_canonical_seed_list(&self.seed_list)
            .map_err(S4CorpusProgressionError::Schema)?;
        validate_phase_boundaries(&self.phase_boundaries)?;
        Ok(())
    }
}

/// `s4_corpus_progression.v1` artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4CorpusProgressionReport {
    /// Schema id, always `s4_corpus_progression.v1`.
    pub schema: String,
    /// TinyStories manifest self-hash.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Back-reference to the sibling promotion-gate product.
    ///
    /// This field is omitted from this report's own self-hash to avoid a
    /// two-artifact hash cycle. The promotion gate includes this report's
    /// self-hash in its own hash; this field completes the back-reference.
    pub promotion_gate_self_hash: Option<Hash256>,
    /// Declarative corpus progression schedule snapshot.
    pub schedule: CorpusProgressionScheduleSnapshot,
    /// Self-hash over canonical JSON with this field omitted.
    pub corpus_progression_self_hash: Hash256,
}

impl S4CorpusProgressionReport {
    /// Construct the pinned S4 corpus progression report.
    pub fn new(
        tinystories_manifest_self_hash: Hash256,
        gutenberg_manifest_self_hash: Hash256,
        promotion_gate_self_hash: Option<Hash256>,
    ) -> Result<Self, S4CorpusProgressionError> {
        Self {
            schema: S4_CORPUS_PROGRESSION_SCHEMA.to_owned(),
            tinystories_manifest_self_hash,
            gutenberg_manifest_self_hash,
            promotion_gate_self_hash,
            schedule: CorpusProgressionScheduleSnapshot::pinned(
                tinystories_manifest_self_hash,
                gutenberg_manifest_self_hash,
            ),
            corpus_progression_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Return a copy with the promotion-gate back-reference set.
    pub fn with_bound_promotion_gate(
        mut self,
        promotion_gate_self_hash: Hash256,
    ) -> Result<Self, S4CorpusProgressionError> {
        self.promotion_gate_self_hash = Some(promotion_gate_self_hash);
        self.validate_canonical_write()?;
        Ok(self)
    }

    /// Return a copy with schedule and report self-hashes recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S4CorpusProgressionError> {
        self.corpus_progression_self_hash = Hash256::ZERO;
        self.schedule = self.schedule.with_computed_self_hash()?;
        self.validate_structure_without_self_hash()?;
        self.corpus_progression_self_hash = self.compute_self_hash()?;
        Ok(self)
    }

    /// Canonical JSON bytes including self-hashes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4CorpusProgressionError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4CorpusProgressionError::CanonicalJson)
    }

    /// Compute the report self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4CorpusProgressionError> {
        self.validate_structure_without_self_hash()?;
        compute_self_hash(self, "corpus_progression_self_hash", REPORT_DOMAIN)
    }

    /// Validate structure and self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4CorpusProgressionError> {
        self.validate_structure_without_self_hash()?;
        let expected = self.compute_self_hash()?;
        if expected != self.corpus_progression_self_hash {
            return Err(S4CorpusProgressionError::SelfHashMismatch {
                field: "corpus_progression_self_hash",
                expected,
                observed: self.corpus_progression_self_hash,
            });
        }
        Ok(())
    }

    /// Validate the mutual reference with `s4_promotion_gate.v1`.
    pub fn validate_promotion_gate_binding(
        &self,
        promotion_gate: &PromotionGateProduct,
    ) -> Result<(), S4CorpusProgressionError> {
        self.validate_canonical_write()?;
        promotion_gate
            .validate_canonical_write()
            .map_err(S4CorpusProgressionError::PromotionGate)?;
        match self.promotion_gate_self_hash {
            Some(hash) if hash == promotion_gate.promotion_gate_self_hash => {}
            Some(hash) => {
                return Err(S4CorpusProgressionError::PromotionGateBindingMismatch {
                    field: "promotion_gate_self_hash",
                    expected: promotion_gate.promotion_gate_self_hash,
                    observed: hash,
                });
            }
            None => {
                return Err(S4CorpusProgressionError::MissingPromotionGateBinding);
            }
        }
        match promotion_gate.corpus_progression_self_hash {
            Some(hash) if hash == self.corpus_progression_self_hash => Ok(()),
            Some(hash) => Err(S4CorpusProgressionError::PromotionGateBindingMismatch {
                field: "corpus_progression_self_hash",
                expected: self.corpus_progression_self_hash,
                observed: hash,
            }),
            None => Err(S4CorpusProgressionError::MissingPromotionGateBinding),
        }
    }

    fn validate_structure_without_self_hash(&self) -> Result<(), S4CorpusProgressionError> {
        if self.schema != S4_CORPUS_PROGRESSION_SCHEMA {
            return Err(S4CorpusProgressionError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        self.schedule.validate_canonical_write()?;
        let [tinystories, gutenberg] = self.schedule.ordered_corpora.as_slice() else {
            return Err(S4CorpusProgressionError::InvalidCorpusOrder);
        };
        if tinystories.corpus_self_hash != self.tinystories_manifest_self_hash {
            return Err(S4CorpusProgressionError::CorpusHashMismatch {
                corpus: S4CorpusProgressionCorpus::TinyStories,
                expected: self.tinystories_manifest_self_hash,
                observed: tinystories.corpus_self_hash,
            });
        }
        if gutenberg.corpus_self_hash != self.gutenberg_manifest_self_hash {
            return Err(S4CorpusProgressionError::CorpusHashMismatch {
                corpus: S4CorpusProgressionCorpus::Gutenberg,
                expected: self.gutenberg_manifest_self_hash,
                observed: gutenberg.corpus_self_hash,
            });
        }
        if self.promotion_gate_self_hash == Some(Hash256::ZERO) {
            return Err(S4CorpusProgressionError::InvalidHash {
                field: "promotion_gate_self_hash",
            });
        }
        Ok(())
    }
}

/// Write a corpus progression report as canonical JSON.
pub fn write_s4_corpus_progression_report(
    path: &Path,
    report: &S4CorpusProgressionReport,
) -> Result<(), S4CorpusProgressionError> {
    let bytes = report.canonical_bytes()?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(S4CorpusProgressionError::Io)?;
    }
    fs::write(path, bytes).map_err(S4CorpusProgressionError::Io)?;
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = S4_CORPUS_PROGRESSION_EMIT_EVENT,
        schema = S4_CORPUS_PROGRESSION_SCHEMA,
        schedule_self_hash = %report.schedule.schedule_self_hash,
        corpus_progression_self_hash = %report.corpus_progression_self_hash,
        corpora = ?report.schedule.ordered_corpora.iter().map(|entry| entry.corpus).collect::<Vec<_>>(),
        path = %path.display(),
        "s4 corpus progression emitted"
    );
    Ok(())
}

fn validate_ordered_corpora(
    ordered_corpora: &[S4CorpusProgressionCorpusRef],
) -> Result<(), S4CorpusProgressionError> {
    let [tinystories, gutenberg] = ordered_corpora else {
        return Err(S4CorpusProgressionError::InvalidCorpusOrder);
    };
    if tinystories.corpus != S4CorpusProgressionCorpus::TinyStories
        || gutenberg.corpus != S4CorpusProgressionCorpus::Gutenberg
    {
        return Err(S4CorpusProgressionError::InvalidCorpusOrder);
    }
    if tinystories.corpus_self_hash == Hash256::ZERO {
        return Err(S4CorpusProgressionError::InvalidHash {
            field: "ordered_corpora[0].corpus_self_hash",
        });
    }
    if gutenberg.corpus_self_hash == Hash256::ZERO {
        return Err(S4CorpusProgressionError::InvalidHash {
            field: "ordered_corpora[1].corpus_self_hash",
        });
    }
    Ok(())
}

fn validate_edges(edges: &[S4CorpusProgressionEdge]) -> Result<(), S4CorpusProgressionError> {
    let [edge] = edges else {
        return Err(S4CorpusProgressionError::InvalidEdge {
            observed_count: edges.len(),
        });
    };
    if edge.from != S4CorpusProgressionCorpus::TinyStories
        || edge.to != S4CorpusProgressionCorpus::Gutenberg
        || edge.gate != S4_CORPUS_PROGRESSION_GATE_TS_TO_GUTENBERG
    {
        return Err(S4CorpusProgressionError::InvalidEdge {
            observed_count: edges.len(),
        });
    }
    Ok(())
}

fn validate_phase_boundaries(
    phase_boundaries: &[S4CorpusProgressionPhaseBoundary],
) -> Result<(), S4CorpusProgressionError> {
    if phase_boundaries.len() != 2 {
        return Err(S4CorpusProgressionError::InvalidPhaseBoundaryCount {
            expected: 2,
            observed: phase_boundaries.len(),
        });
    }
    let mut expected_start = 0_u64;
    for (index, boundary) in phase_boundaries.iter().enumerate() {
        if boundary.start_progression_step != expected_start {
            return Err(S4CorpusProgressionError::InvalidPhaseBoundaryRange {
                index,
                expected_start,
                observed_start: boundary.start_progression_step,
                observed_end_exclusive: boundary.end_progression_step_exclusive,
            });
        }
        if boundary.start_progression_step >= boundary.end_progression_step_exclusive {
            return Err(S4CorpusProgressionError::InvalidPhaseBoundaryRange {
                index,
                expected_start,
                observed_start: boundary.start_progression_step,
                observed_end_exclusive: boundary.end_progression_step_exclusive,
            });
        }
        expected_start = boundary.end_progression_step_exclusive;
    }

    let [source, continuation] = phase_boundaries else {
        unreachable!("length checked above")
    };
    if source.active_corpus != S4CorpusProgressionCorpus::TinyStories
        || source.train_phase != TrainPhaseKind::FullNumericQat
    {
        return Err(S4CorpusProgressionError::InvalidPhaseBoundaryCorpus {
            index: 0,
            expected: S4CorpusProgressionCorpus::TinyStories,
            observed: source.active_corpus,
        });
    }
    if source.end_progression_step_exclusive != 1 {
        return Err(S4CorpusProgressionError::InvalidPhaseBoundaryRange {
            index: 0,
            expected_start: 0,
            observed_start: source.start_progression_step,
            observed_end_exclusive: source.end_progression_step_exclusive,
        });
    }
    if continuation.active_corpus != S4CorpusProgressionCorpus::Gutenberg
        || continuation.train_phase != TrainPhaseKind::FullNumericQat
    {
        return Err(S4CorpusProgressionError::InvalidPhaseBoundaryCorpus {
            index: 1,
            expected: S4CorpusProgressionCorpus::Gutenberg,
            observed: continuation.active_corpus,
        });
    }
    if continuation.end_progression_step_exclusive != S4_OPTIMIZER_STEPS_GUTENBERG + 1 {
        return Err(S4CorpusProgressionError::InvalidPhaseBoundaryRange {
            index: 1,
            expected_start: 1,
            observed_start: continuation.start_progression_step,
            observed_end_exclusive: continuation.end_progression_step_exclusive,
        });
    }
    Ok(())
}

fn compute_self_hash<T: Serialize>(
    payload: &T,
    self_hash_field: &'static str,
    domain: DomainHash<'static>,
) -> Result<Hash256, S4CorpusProgressionError> {
    let mut value = serde_json::to_value(payload).map_err(S4CorpusProgressionError::Json)?;
    value
        .as_object_mut()
        .ok_or(S4CorpusProgressionError::ExpectedObjectForSelfHash)?
        .remove(self_hash_field);
    if self_hash_field == "corpus_progression_self_hash" {
        value
            .as_object_mut()
            .expect("object checked above")
            .remove("promotion_gate_self_hash");
    }
    let canonical =
        CanonicalJson::value_to_vec(&value).map_err(S4CorpusProgressionError::CanonicalJson)?;
    domain
        .hash_canonical_bytes(&canonical)
        .map_err(S4CorpusProgressionError::CanonicalJson)
}

/// Errors from corpus progression validation and emission.
#[derive(Debug)]
pub enum S4CorpusProgressionError {
    /// Product schema did not match `s4_corpus_progression.v1`.
    InvalidSchema {
        /// Observed schema id.
        observed: String,
    },
    /// Schedule version did not match `s4.v1`.
    InvalidScheduleVersion {
        /// Observed schedule version.
        observed: String,
    },
    /// Ordered corpus list is not exactly TinyStories then Gutenberg.
    InvalidCorpusOrder,
    /// Corpus self-hash was the zero sentinel.
    InvalidHash {
        /// Invalid field name.
        field: &'static str,
    },
    /// Schedule edge list is not the single TinyStories-to-Gutenberg edge.
    InvalidEdge {
        /// Observed edge count.
        observed_count: usize,
    },
    /// Active start/finish corpus drifted away from the S4 pin.
    InvalidActiveCorpus {
        /// Field name.
        field: &'static str,
        /// Expected corpus.
        expected: S4CorpusProgressionCorpus,
        /// Observed corpus.
        observed: S4CorpusProgressionCorpus,
    },
    /// Phase boundary count drifted away from the S4 pin.
    InvalidPhaseBoundaryCount {
        /// Expected boundary count.
        expected: usize,
        /// Observed boundary count.
        observed: usize,
    },
    /// Phase boundaries overlap, leave a gap, or have invalid ranges.
    InvalidPhaseBoundaryRange {
        /// Boundary index.
        index: usize,
        /// Expected start for this interval.
        expected_start: u64,
        /// Observed start.
        observed_start: u64,
        /// Observed exclusive end.
        observed_end_exclusive: u64,
    },
    /// Phase boundary active corpus drifted.
    InvalidPhaseBoundaryCorpus {
        /// Boundary index.
        index: usize,
        /// Expected corpus.
        expected: S4CorpusProgressionCorpus,
        /// Observed corpus.
        observed: S4CorpusProgressionCorpus,
    },
    /// Report top-level manifest hash and schedule corpus entry disagreed.
    CorpusHashMismatch {
        /// Mismatched corpus.
        corpus: S4CorpusProgressionCorpus,
        /// Expected top-level hash.
        expected: Hash256,
        /// Observed schedule hash.
        observed: Hash256,
    },
    /// Stored self-hash differed from recomputation.
    SelfHashMismatch {
        /// Self-hash field name.
        field: &'static str,
        /// Expected recomputed self-hash.
        expected: Hash256,
        /// Observed stored self-hash.
        observed: Hash256,
    },
    /// Promotion-gate back-reference is absent on one side.
    MissingPromotionGateBinding,
    /// Promotion gate and corpus progression back-references disagree.
    PromotionGateBindingMismatch {
        /// Mismatched field name.
        field: &'static str,
        /// Expected value.
        expected: Hash256,
        /// Observed value.
        observed: Hash256,
    },
    /// Self-hash computation expected a top-level object.
    ExpectedObjectForSelfHash,
    /// S4 seed-list schema validation failed.
    Schema(S4SchemaError),
    /// Promotion-gate validation failed.
    PromotionGate(PromotionGateError),
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON serialization failed.
    CanonicalJson(CanonicalJsonError),
    /// Filesystem write failed.
    Io(std::io::Error),
}

impl fmt::Display for S4CorpusProgressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { observed } => write!(
                f,
                "expected s4_corpus_progression.v1 schema, observed {observed:?}"
            ),
            Self::InvalidScheduleVersion { observed } => {
                write!(f, "expected schedule_version s4.v1, observed {observed:?}")
            }
            Self::InvalidCorpusOrder => {
                f.write_str("S4 corpus progression requires TinyStories then Gutenberg")
            }
            Self::InvalidHash { field } => {
                write!(f, "S4 corpus progression field {field} must not be zero")
            }
            Self::InvalidEdge { observed_count } => write!(
                f,
                "S4 corpus progression requires one TinyStories-to-Gutenberg edge, observed {observed_count}"
            ),
            Self::InvalidActiveCorpus {
                field,
                expected,
                observed,
            } => write!(
                f,
                "S4 corpus progression {field} expected {expected:?}, observed {observed:?}"
            ),
            Self::InvalidPhaseBoundaryCount { expected, observed } => write!(
                f,
                "S4 corpus progression expected {expected} phase boundaries, observed {observed}"
            ),
            Self::InvalidPhaseBoundaryRange {
                index,
                expected_start,
                observed_start,
                observed_end_exclusive,
            } => write!(
                f,
                "S4 corpus progression boundary {index} expected start {expected_start}, observed {observed_start}..{observed_end_exclusive}"
            ),
            Self::InvalidPhaseBoundaryCorpus {
                index,
                expected,
                observed,
            } => write!(
                f,
                "S4 corpus progression boundary {index} expected {expected:?}, observed {observed:?}"
            ),
            Self::CorpusHashMismatch {
                corpus,
                expected,
                observed,
            } => write!(
                f,
                "S4 corpus progression {corpus:?} hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::SelfHashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "{field} mismatch: expected recomputed {expected}, observed {observed}"
            ),
            Self::MissingPromotionGateBinding => {
                f.write_str("S4 corpus progression and promotion gate are not mutually bound")
            }
            Self::PromotionGateBindingMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "S4 corpus progression promotion binding {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("S4 corpus progression self-hash requires a top-level object")
            }
            Self::Schema(error) => write!(f, "{error}"),
            Self::PromotionGate(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4CorpusProgressionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::PromotionGate(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
