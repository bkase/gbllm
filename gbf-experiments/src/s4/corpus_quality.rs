//! S4 corpus-quality artifact surface.

use std::error::Error;
use std::fmt;

use gbf_artifact::{GutenbergManifest, GutenbergSourceRecord, GutenbergSplit};
use gbf_data::UNMAPPABLE_EXAMPLE_DROP_THRESHOLD;
use gbf_foundation::{CanonicalJson, DomainHash, Hash256};
use serde::{Deserialize, Serialize};

pub use gbf_artifact::GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX;

const S4_CORPUS_QUALITY_SCHEMA: &str = "s4_corpus_quality.v1";
const S4_CORPUS_QUALITY_SCHEMA_VERSION: &str = "1";
const S4_CORPUS_QUALITY_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4CorpusQuality",
    S4_CORPUS_QUALITY_SCHEMA,
    S4_CORPUS_QUALITY_SCHEMA_VERSION,
);

/// Structured event emitted for each retained document checked by the D5 gate.
pub const S4_UNMAPPABLE_GATE_DOC_EVENT: &str = "s4_unmappable_gate_doc";

/// Structured event emitted with aggregate D5 pass/fail gate evidence.
pub const S4_UNMAPPABLE_GATE_OUTCOME_EVENT: &str = "s4_unmappable_gate_outcome";

/// Tracing target for S4 corpus-quality and unmappable gate events.
pub const S4_CORPUS_QUALITY_LOG_TARGET: &str = "gbf_experiments::s4::corpus_quality";

/// Verified D5 unmappable-gate accounting for a Gutenberg manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4UnmappableGateReport {
    /// Gate outcome. A report is only returned when the gate passes.
    pub status: String,
    /// Number of retained Gutenberg documents included in the aggregate.
    pub retained_book_count: u32,
    /// Sum of `<unk>` body token ids over retained documents.
    pub unmappable_count: u64,
    /// Sum of post-charset body token ids over retained documents.
    pub body_token_count: u64,
    /// Aggregate retained-corpus unmappable rate.
    pub unmappable_rate_corpus: f64,
    /// S4 D5 maximum aggregate retained-corpus unmappable rate.
    pub max_unmappable_rate_corpus: f64,
    /// Inherited F-G2/S3 per-document unmappable-density threshold.
    pub max_unmappable_density_per_doc: f64,
    /// Whether the aggregate retained-corpus gate passed.
    pub aggregate_passed: bool,
    /// Per-retained-document density evidence.
    pub retained_document_densities: Vec<S4RetainedUnmappableDensity>,
}

impl S4UnmappableGateReport {
    fn validate(&self) -> Result<(), S4CorpusQualityError> {
        if self.status != "passed" {
            return Err(S4CorpusQualityError::InvalidUnmappableGateStatus {
                observed: self.status.clone(),
            });
        }
        validate_finite_nonnegative(
            "unmappable_gate.unmappable_rate_corpus",
            self.unmappable_rate_corpus,
        )?;
        validate_finite_nonnegative(
            "unmappable_gate.max_unmappable_rate_corpus",
            self.max_unmappable_rate_corpus,
        )?;
        validate_finite_nonnegative(
            "unmappable_gate.max_unmappable_density_per_doc",
            self.max_unmappable_density_per_doc,
        )?;
        if !self.aggregate_passed || self.unmappable_rate_corpus > self.max_unmappable_rate_corpus {
            return Err(S4CorpusQualityError::InvalidUnmappableGateStatus {
                observed: self.status.clone(),
            });
        }
        let retained_doc_count = u32::try_from(self.retained_document_densities.len())
            .map_err(|_| S4CorpusQualityError::UnmappableGateDocCountOverflow)?;
        if retained_doc_count != self.retained_book_count {
            return Err(S4CorpusQualityError::UnmappableGateDocCountMismatch {
                retained_book_count: self.retained_book_count,
                retained_document_density_count: retained_doc_count,
            });
        }
        let mut unmappable_count = 0_u64;
        let mut body_token_count = 0_u64;
        for doc in &self.retained_document_densities {
            doc.validate(self.max_unmappable_density_per_doc)?;
            unmappable_count = unmappable_count.checked_add(doc.unmappable_count).ok_or(
                S4CorpusQualityError::UnmappableGateCountOverflow {
                    field: "unmappable_gate.unmappable_count",
                },
            )?;
            body_token_count = body_token_count.checked_add(doc.body_token_count).ok_or(
                S4CorpusQualityError::UnmappableGateCountOverflow {
                    field: "unmappable_gate.body_token_count",
                },
            )?;
        }
        if unmappable_count != self.unmappable_count || body_token_count != self.body_token_count {
            return Err(S4CorpusQualityError::UnmappableGateAggregateMismatch);
        }
        Ok(())
    }
}

/// Per-retained-document unmappable-density evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4RetainedUnmappableDensity {
    /// Gutenberg book id.
    pub book_id: u32,
    /// Retained split for this document.
    pub split: GutenbergSplit,
    /// `<unk>` count in post-charset body tokens.
    pub unmappable_count: u64,
    /// Post-charset body-token count, excluding BOS/EOS stream markers.
    pub body_token_count: u64,
    /// Per-document unmappable density.
    pub unmappable_density: f64,
    /// Inherited F-G2/S3 per-document maximum density.
    pub max_unmappable_density_per_doc: f64,
    /// Whether this retained document passed the density gate.
    pub passed: bool,
}

impl S4RetainedUnmappableDensity {
    fn validate(&self, expected_max_density: f64) -> Result<(), S4CorpusQualityError> {
        validate_finite_nonnegative(
            "unmappable_gate.retained_document_densities[].unmappable_density",
            self.unmappable_density,
        )?;
        validate_finite_nonnegative(
            "unmappable_gate.retained_document_densities[].max_unmappable_density_per_doc",
            self.max_unmappable_density_per_doc,
        )?;
        if self.max_unmappable_density_per_doc != expected_max_density
            || !self.passed
            || self.unmappable_density > self.max_unmappable_density_per_doc
        {
            return Err(S4CorpusQualityError::InvalidUnmappableGateDocument {
                book_id: self.book_id,
            });
        }
        Ok(())
    }
}

/// Drop-count summary copied from `gutenberg_manifest.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4CorpusDropCounts {
    /// Total number of selected Gutenberg ids dropped before retention.
    pub total: u32,
    /// Books dropped because no supported plaintext resource was available.
    pub no_supported_plaintext_format: u32,
    /// Archive resources dropped because no safe plaintext member was found.
    pub no_plaintext_archive_member: u32,
    /// Sources dropped because bytes could not be decoded losslessly.
    pub source_decode_failed: u32,
    /// Zip archives dropped because plaintext member selection was ambiguous.
    pub ambiguous_plaintext_archive: u32,
    /// Sources dropped because UTF-8 validation failed after decode.
    pub invalid_utf8: u32,
    /// Sources dropped because D3 stripping left no body text.
    pub empty_after_strip: u32,
    /// Sources dropped because Gutenberg start/end markers were missing.
    pub marker_missing: u32,
    /// Sources dropped because charset-v1 unmappable density exceeded bounds.
    pub unmappable_density: u32,
    /// Sources dropped because deduplication found a retained duplicate.
    pub dedup_collision: u32,
}

impl S4CorpusDropCounts {
    /// Copy the currently supported drop counters from a Gutenberg manifest.
    #[must_use]
    pub fn from_manifest(manifest: &GutenbergManifest) -> Self {
        Self {
            total: manifest.drop_count_total,
            no_supported_plaintext_format: manifest.drop_count_no_supported_plaintext_format,
            no_plaintext_archive_member: manifest.drop_count_no_plaintext_archive_member,
            source_decode_failed: manifest.drop_count_source_decode_failed,
            ambiguous_plaintext_archive: manifest.drop_count_ambiguous_plaintext_archive,
            invalid_utf8: manifest.drop_count_invalid_utf8,
            empty_after_strip: manifest.drop_count_empty_after_strip,
            marker_missing: manifest.drop_count_marker_missing,
            unmappable_density: manifest.drop_count_unmappable_density,
            dedup_collision: manifest.drop_count_dedup_collision,
        }
    }
}

/// One corpus summary row in `s4_corpus_quality.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4PerCorpusQuality {
    /// Stable corpus identifier.
    pub corpus_id: String,
    /// Aggregate unmappable-character rate for retained documents.
    pub unmappable_rate: f64,
    /// Mean retained token count per document.
    pub tokens_per_doc_mean: f64,
    /// Median retained token count per document.
    pub tokens_per_doc_median: f64,
    /// Maximum retained token count across documents.
    pub tokens_per_doc_max: u64,
    /// Document id that produced `tokens_per_doc_max`, when any documents exist.
    pub longest_doc_id: Option<String>,
    /// Number of charset-v1 symbols observed in retained documents.
    pub charset_coverage_count: u64,
}

impl S4PerCorpusQuality {
    /// Build the Project Gutenberg row from retained per-document token ids.
    pub fn gutenberg_from_docs(
        doc_lengths: &[u64],
        longest_doc_id: Option<String>,
        charset_coverage_count: u64,
        unmappable_rate: f64,
    ) -> Result<Self, S4CorpusQualityError> {
        let tokens_per_doc_mean = mean(doc_lengths);
        let tokens_per_doc_median = median(doc_lengths);
        let tokens_per_doc_max = doc_lengths.iter().copied().max().unwrap_or(0);
        let row = Self {
            corpus_id: "Gutenberg".to_owned(),
            unmappable_rate,
            tokens_per_doc_mean,
            tokens_per_doc_median,
            tokens_per_doc_max,
            longest_doc_id,
            charset_coverage_count,
        };
        row.validate()?;
        Ok(row)
    }

    fn validate(&self) -> Result<(), S4CorpusQualityError> {
        validate_finite_nonnegative("per_corpus[].unmappable_rate", self.unmappable_rate)?;
        validate_finite_nonnegative("per_corpus[].tokens_per_doc_mean", self.tokens_per_doc_mean)?;
        validate_finite_nonnegative(
            "per_corpus[].tokens_per_doc_median",
            self.tokens_per_doc_median,
        )?;
        Ok(())
    }
}

/// Deferred artifact pointer for later S4 beads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4DeferredArtifactPointer {
    /// Schema id of the downstream artifact.
    pub artifact_schema: String,
    /// Planned canonical artifact path.
    pub artifact_path: String,
    /// Bead that owns producing the downstream artifact.
    pub owner_bead: String,
    /// Pointer status, usually `deferred` until the owner bead closes.
    pub status: String,
    /// Artifact self-hash once produced.
    pub artifact_self_hash: Option<Hash256>,
    /// Downstream outcome label once produced.
    pub outcome: Option<String>,
}

impl S4DeferredArtifactPointer {
    /// Pointer to the future Gutenberg KN-5 baseline artifact.
    #[must_use]
    pub fn kn_baseline_gutenberg() -> Self {
        Self {
            artifact_schema: "s4_baseline_gutenberg.v1".to_owned(),
            artifact_path: "experiments/S4/baseline/baseline_gutenberg.json".to_owned(),
            owner_bead: "bd-2nca".to_owned(),
            status: "deferred".to_owned(),
            artifact_self_hash: None,
            outcome: None,
        }
    }

    /// Pointer to the future cross-corpus contamination artifact.
    #[must_use]
    pub fn contamination_outcome() -> Self {
        Self {
            artifact_schema: "s4_contamination_report.v1".to_owned(),
            artifact_path: "experiments/S4/contamination/cross_corpus.json".to_owned(),
            owner_bead: "bd-2p3n".to_owned(),
            status: "deferred".to_owned(),
            artifact_self_hash: None,
            outcome: None,
        }
    }
}

/// `s4_corpus_quality.v1` emitted after the Gutenberg manifest is built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4CorpusQuality {
    /// Schema id, always `s4_corpus_quality.v1`.
    pub schema: String,
    /// Self-hash of the canonical `gutenberg_manifest.v1` input.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Self-hash of the TinyStories manifest used for progression comparison.
    pub tinystories_manifest_self_hash: Hash256,
    /// Drop-count summary copied from the Gutenberg manifest.
    pub drop_counts: S4CorpusDropCounts,
    /// Retained Gutenberg document count after all recoverable corpus drops.
    pub retained_book_count: u32,
    /// Auditable D5 unmappable gate evidence for retained Gutenberg documents.
    pub unmappable_gate: S4UnmappableGateReport,
    /// Per-corpus quality summary rows.
    pub per_corpus: Vec<S4PerCorpusQuality>,
    /// Pointer to the downstream KN-5 Gutenberg baseline artifact.
    pub kn_baseline_pointer: S4DeferredArtifactPointer,
    /// Pointer to the downstream cross-corpus contamination artifact.
    pub contamination_outcome_pointer: S4DeferredArtifactPointer,
    /// Self-hash over canonical JSON with this field omitted.
    pub corpus_quality_self_hash: Hash256,
}

impl S4CorpusQuality {
    /// Build and self-hash a corpus-quality artifact.
    pub fn new(
        gutenberg_manifest: &GutenbergManifest,
        tinystories_manifest_self_hash: Hash256,
        per_corpus: Vec<S4PerCorpusQuality>,
    ) -> Result<Self, S4CorpusQualityError> {
        let unmappable_gate = verify_gutenberg_unmappable_gate(gutenberg_manifest)?;
        let retained_book_count = gutenberg_manifest
            .train_book_count
            .checked_add(gutenberg_manifest.val_book_count)
            .ok_or(S4CorpusQualityError::RetainedBookCountOverflow {
                train_book_count: gutenberg_manifest.train_book_count,
                val_book_count: gutenberg_manifest.val_book_count,
            })?;
        let quality = Self {
            schema: S4_CORPUS_QUALITY_SCHEMA.to_owned(),
            gutenberg_manifest_self_hash: gutenberg_manifest.manifest_self_hash,
            tinystories_manifest_self_hash,
            drop_counts: S4CorpusDropCounts::from_manifest(gutenberg_manifest),
            retained_book_count,
            unmappable_gate,
            per_corpus,
            kn_baseline_pointer: S4DeferredArtifactPointer::kn_baseline_gutenberg(),
            contamination_outcome_pointer: S4DeferredArtifactPointer::contamination_outcome(),
            corpus_quality_self_hash: Hash256::ZERO,
        };
        quality.validate_structure()?;
        let canonical =
            CanonicalJson::to_vec(&quality).map_err(S4CorpusQualityError::CanonicalJson)?;
        let mut normalized: Self =
            serde_json::from_slice(&canonical).map_err(S4CorpusQualityError::Json)?;
        normalized.corpus_quality_self_hash = Hash256::ZERO;
        normalized.validate_structure()?;
        normalized.corpus_quality_self_hash = normalized.compute_self_hash()?;
        Ok(normalized)
    }

    /// Canonical JSON bytes including `corpus_quality_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4CorpusQualityError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4CorpusQualityError::CanonicalJson)
    }

    /// Self-hash over canonical JSON with `corpus_quality_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4CorpusQualityError> {
        let mut value = serde_json::to_value(self).map_err(S4CorpusQualityError::Json)?;
        value
            .as_object_mut()
            .ok_or(S4CorpusQualityError::ExpectedObjectForSelfHash)?
            .remove("corpus_quality_self_hash");
        let canonical =
            CanonicalJson::value_to_vec(&value).map_err(S4CorpusQualityError::CanonicalJson)?;
        S4_CORPUS_QUALITY_DOMAIN
            .hash_canonical_bytes(&canonical)
            .map_err(S4CorpusQualityError::CanonicalJson)
    }

    /// Validate the artifact including self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4CorpusQualityError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.corpus_quality_self_hash {
            return Err(S4CorpusQualityError::SelfHashMismatch {
                expected: recomputed,
                observed: self.corpus_quality_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), S4CorpusQualityError> {
        if self.schema != S4_CORPUS_QUALITY_SCHEMA {
            return Err(S4CorpusQualityError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        if self.per_corpus.is_empty() {
            return Err(S4CorpusQualityError::EmptyPerCorpus);
        }
        self.unmappable_gate.validate()?;
        if self.unmappable_gate.retained_book_count != self.retained_book_count {
            return Err(S4CorpusQualityError::UnmappableGateDocCountMismatch {
                retained_book_count: self.retained_book_count,
                retained_document_density_count: self.unmappable_gate.retained_book_count,
            });
        }
        for row in &self.per_corpus {
            row.validate()?;
        }
        Ok(())
    }
}

/// Errors from the corpus-quality artifact boundary.
#[derive(Debug)]
pub enum S4CorpusQualityError {
    /// The artifact schema field did not match `s4_corpus_quality.v1`.
    InvalidSchema {
        /// Observed schema value.
        observed: String,
    },
    /// No per-corpus quality rows were supplied.
    EmptyPerCorpus,
    /// A floating-point metric was NaN or infinite.
    NonFiniteMetric {
        /// Metric field name.
        field: &'static str,
        /// Observed invalid value.
        value: f64,
    },
    /// A floating-point metric was negative.
    NegativeMetric {
        /// Metric field name.
        field: &'static str,
        /// Observed invalid value.
        value: f64,
    },
    /// The stored self-hash did not match the recomputed hash.
    SelfHashMismatch {
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// Retained train + validation book count overflowed.
    RetainedBookCountOverflow {
        /// Retained train book count.
        train_book_count: u32,
        /// Retained validation book count.
        val_book_count: u32,
    },
    /// The canonical unmappable gate status was not a passing gate.
    InvalidUnmappableGateStatus {
        /// Observed status.
        observed: String,
    },
    /// Per-document unmappable gate evidence overflowed a public count field.
    UnmappableGateCountOverflow {
        /// Field whose count overflowed.
        field: &'static str,
    },
    /// The retained document-density list length did not match retained count.
    UnmappableGateDocCountMismatch {
        /// Retained count field.
        retained_book_count: u32,
        /// Count of retained document density rows.
        retained_document_density_count: u32,
    },
    /// A retained document row failed the canonical gate evidence invariants.
    InvalidUnmappableGateDocument {
        /// Gutenberg book id.
        book_id: u32,
    },
    /// Aggregate unmappable gate counts did not match per-document rows.
    UnmappableGateAggregateMismatch,
    /// Retained document-density count overflowed u32.
    UnmappableGateDocCountOverflow,
    /// The Gutenberg manifest failed the S4 D5 unmappable gate.
    UnmappableGate(S4UnmappableGateError),
    /// Self-hash computation expected a top-level JSON object.
    ExpectedObjectForSelfHash,
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON serialization failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
}

impl fmt::Display for S4CorpusQualityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { observed } => {
                write!(f, "expected s4_corpus_quality.v1 schema, got {observed:?}")
            }
            Self::EmptyPerCorpus => {
                f.write_str("s4_corpus_quality.v1 requires at least one per_corpus row")
            }
            Self::NonFiniteMetric { field, value } => {
                write!(
                    f,
                    "{field} must be finite in s4_corpus_quality.v1, got {value}"
                )
            }
            Self::NegativeMetric { field, value } => {
                write!(
                    f,
                    "{field} must be non-negative in s4_corpus_quality.v1, got {value}"
                )
            }
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "s4_corpus_quality.v1 self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::RetainedBookCountOverflow {
                train_book_count,
                val_book_count,
            } => write!(
                f,
                "s4_corpus_quality.v1 retained_book_count overflow: train={train_book_count} val={val_book_count}"
            ),
            Self::InvalidUnmappableGateStatus { observed } => write!(
                f,
                "s4_corpus_quality.v1 unmappable_gate must carry passed D5 gate evidence, got {observed:?}"
            ),
            Self::UnmappableGateCountOverflow { field } => {
                write!(f, "s4_corpus_quality.v1 {field} overflowed")
            }
            Self::UnmappableGateDocCountMismatch {
                retained_book_count,
                retained_document_density_count,
            } => write!(
                f,
                "s4_corpus_quality.v1 retained_book_count={retained_book_count} does not match unmappable_gate retained_document_densities length {retained_document_density_count}"
            ),
            Self::InvalidUnmappableGateDocument { book_id } => write!(
                f,
                "s4_corpus_quality.v1 unmappable gate row for book {book_id} failed D5 invariants"
            ),
            Self::UnmappableGateAggregateMismatch => f.write_str(
                "s4_corpus_quality.v1 unmappable_gate aggregate counts do not match per-document evidence",
            ),
            Self::UnmappableGateDocCountOverflow => {
                f.write_str("s4_corpus_quality.v1 unmappable_gate document count overflowed u32")
            }
            Self::UnmappableGate(error) => write!(f, "{error}"),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("s4_corpus_quality.v1 self-hash requires a top-level object")
            }
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4CorpusQualityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::UnmappableGate(error) => Some(error),
            _ => None,
        }
    }
}

impl From<S4UnmappableGateError> for S4CorpusQualityError {
    fn from(error: S4UnmappableGateError) -> Self {
        Self::UnmappableGate(error)
    }
}

/// Verify the S4 D5 Gutenberg unmappable gate against the RFC hard cap.
pub fn verify_gutenberg_unmappable_gate(
    manifest: &GutenbergManifest,
) -> Result<S4UnmappableGateReport, S4UnmappableGateError> {
    verify_gutenberg_unmappable_gate_with_max(manifest, GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX)
}

/// Verify the S4 D5 Gutenberg unmappable gate with an optional stricter cap.
pub(crate) fn verify_gutenberg_unmappable_gate_with_max(
    manifest: &GutenbergManifest,
    max_unmappable_rate_corpus: f64,
) -> Result<S4UnmappableGateReport, S4UnmappableGateError> {
    if !manifest.unmappable_rate_corpus.is_finite() || manifest.unmappable_rate_corpus < 0.0 {
        return Err(S4UnmappableGateError::InvalidCorpusUnmappableRate {
            observed: manifest.unmappable_rate_corpus,
        });
    }
    if !max_unmappable_rate_corpus.is_finite() || max_unmappable_rate_corpus < 0.0 {
        return Err(S4UnmappableGateError::InvalidCorpusUnmappableRateCap {
            observed: max_unmappable_rate_corpus,
        });
    }

    let mut retained_book_count = 0_u32;
    let mut unmappable_count = 0_u128;
    let mut body_token_count = 0_u128;
    let mut retained_document_densities = Vec::new();

    for source in &manifest.sources {
        if source.drop_reason.is_some() {
            continue;
        }
        let per_doc = retained_unmappable_accounting(source)?;
        emit_unmappable_gate_doc(&per_doc);
        retained_book_count = retained_book_count
            .checked_add(1)
            .ok_or(S4UnmappableGateError::RetainedBookCountOverflow)?;
        unmappable_count += u128::from(per_doc.unmappable_count);
        body_token_count += u128::from(per_doc.body_token_count);
        retained_document_densities.push(per_doc);
    }

    let computed_rate = unmappable_rate(unmappable_count, body_token_count);
    if !rate_matches(computed_rate, manifest.unmappable_rate_corpus) {
        emit_unmappable_gate_outcome(
            "failed",
            retained_book_count,
            unmappable_count,
            body_token_count,
            computed_rate,
            max_unmappable_rate_corpus,
        );
        return Err(S4UnmappableGateError::CorpusUnmappableRateMismatch {
            expected: computed_rate,
            observed: manifest.unmappable_rate_corpus,
            unmappable_count,
            body_token_count,
        });
    }
    if computed_rate > max_unmappable_rate_corpus {
        emit_unmappable_gate_outcome(
            "failed",
            retained_book_count,
            unmappable_count,
            body_token_count,
            computed_rate,
            max_unmappable_rate_corpus,
        );
        return Err(S4UnmappableGateError::CorpusUnmappableRateHigh {
            rate: computed_rate,
            max_rate: max_unmappable_rate_corpus,
            unmappable_count,
            body_token_count,
        });
    }

    emit_unmappable_gate_outcome(
        "passed",
        retained_book_count,
        unmappable_count,
        body_token_count,
        computed_rate,
        max_unmappable_rate_corpus,
    );

    Ok(S4UnmappableGateReport {
        status: "passed".to_owned(),
        retained_book_count,
        unmappable_count: u64::try_from(unmappable_count).map_err(|_| {
            S4UnmappableGateError::CountOverflow {
                field: "unmappable_count",
            }
        })?,
        body_token_count: u64::try_from(body_token_count).map_err(|_| {
            S4UnmappableGateError::CountOverflow {
                field: "body_token_count",
            }
        })?,
        unmappable_rate_corpus: computed_rate,
        max_unmappable_rate_corpus,
        max_unmappable_density_per_doc: UNMAPPABLE_EXAMPLE_DROP_THRESHOLD,
        aggregate_passed: true,
        retained_document_densities,
    })
}

fn retained_unmappable_accounting(
    source: &GutenbergSourceRecord,
) -> Result<S4RetainedUnmappableDensity, S4UnmappableGateError> {
    let unmappable_count =
        source
            .unmappable_count
            .ok_or(S4UnmappableGateError::MissingPerDocUnmappableField {
                book_id: source.book_id,
                field: "unmappable_count",
            })?;
    let body_token_count = source.post_charset_token_length.ok_or(
        S4UnmappableGateError::MissingPerDocUnmappableField {
            book_id: source.book_id,
            field: "post_charset_token_length",
        },
    )?;
    let observed_density =
        source
            .unmappable_density
            .ok_or(S4UnmappableGateError::MissingPerDocUnmappableField {
                book_id: source.book_id,
                field: "unmappable_density",
            })?;
    let split = source
        .split
        .ok_or(S4UnmappableGateError::MissingPerDocUnmappableField {
            book_id: source.book_id,
            field: "split",
        })?;

    if body_token_count == 0 {
        return Err(S4UnmappableGateError::RetainedBookHasEmptyBody {
            book_id: source.book_id,
        });
    }
    if unmappable_count > body_token_count {
        return Err(
            S4UnmappableGateError::PerDocUnmappableCountExceedsBodyTokens {
                book_id: source.book_id,
                unmappable_count,
                body_token_count,
            },
        );
    }
    let expected_density =
        unmappable_rate(u128::from(unmappable_count), u128::from(body_token_count));
    if !rate_matches(expected_density, observed_density) {
        return Err(S4UnmappableGateError::PerDocUnmappableDensityMismatch {
            book_id: source.book_id,
            expected: expected_density,
            observed: observed_density,
            unmappable_count,
            body_token_count,
        });
    }
    if expected_density > UNMAPPABLE_EXAMPLE_DROP_THRESHOLD {
        return Err(S4UnmappableGateError::RetainedBookUnmappableDensityHigh {
            book_id: source.book_id,
            density: expected_density,
            max_density: UNMAPPABLE_EXAMPLE_DROP_THRESHOLD,
        });
    }

    Ok(S4RetainedUnmappableDensity {
        book_id: source.book_id,
        split,
        unmappable_count,
        body_token_count,
        unmappable_density: expected_density,
        max_unmappable_density_per_doc: UNMAPPABLE_EXAMPLE_DROP_THRESHOLD,
        passed: true,
    })
}

fn emit_unmappable_gate_doc(doc: &S4RetainedUnmappableDensity) {
    tracing::info!(
        target: S4_CORPUS_QUALITY_LOG_TARGET,
        event_name = S4_UNMAPPABLE_GATE_DOC_EVENT,
        book_id = doc.book_id as u64,
        split = split_label(doc.split),
        unmappable_count = doc.unmappable_count,
        body_token_count = doc.body_token_count,
        unmappable_density = doc.unmappable_density,
        max_unmappable_density_per_doc = doc.max_unmappable_density_per_doc,
        passed = doc.passed,
        "S4 retained document unmappable gate checked"
    );
}

fn emit_unmappable_gate_outcome(
    status: &'static str,
    retained_book_count: u32,
    unmappable_count: u128,
    body_token_count: u128,
    unmappable_rate_corpus: f64,
    max_unmappable_rate_corpus: f64,
) {
    tracing::info!(
        target: S4_CORPUS_QUALITY_LOG_TARGET,
        event_name = S4_UNMAPPABLE_GATE_OUTCOME_EVENT,
        status,
        retained_book_count = retained_book_count as u64,
        unmappable_count = %unmappable_count,
        body_token_count = %body_token_count,
        unmappable_rate_corpus,
        max_unmappable_rate_corpus,
        aggregate_passed = status == "passed",
        "S4 aggregate unmappable gate outcome"
    );
}

const fn split_label(split: GutenbergSplit) -> &'static str {
    match split {
        GutenbergSplit::Train => "train",
        GutenbergSplit::Val => "val",
    }
}

fn unmappable_rate(unmappable_count: u128, body_token_count: u128) -> f64 {
    if body_token_count == 0 {
        0.0
    } else {
        unmappable_count as f64 / body_token_count as f64
    }
}

fn rate_matches(expected: f64, observed: f64) -> bool {
    expected == observed
        || (expected - observed).abs()
            <= f64::EPSILON * 8.0 * expected.abs().max(observed.abs()).max(1.0)
}

/// Errors produced by the S4 D5 Gutenberg unmappable-gate verifier.
#[derive(Debug, Clone, PartialEq)]
pub enum S4UnmappableGateError {
    /// The manifest-level aggregate rate was NaN, infinite, or negative.
    InvalidCorpusUnmappableRate {
        /// Observed invalid rate.
        observed: f64,
    },
    /// The verifier was given an invalid aggregate hard cap.
    InvalidCorpusUnmappableRateCap {
        /// Observed invalid cap.
        observed: f64,
    },
    /// Retained book count overflowed the public report type.
    RetainedBookCountOverflow,
    /// A retained source lacked a required per-document accounting field.
    MissingPerDocUnmappableField {
        /// Affected Gutenberg book id.
        book_id: u32,
        /// Missing field name.
        field: &'static str,
    },
    /// A retained source had no body tokens after charset normalization.
    RetainedBookHasEmptyBody {
        /// Affected Gutenberg book id.
        book_id: u32,
    },
    /// A source reported more unknown tokens than body tokens.
    PerDocUnmappableCountExceedsBodyTokens {
        /// Affected Gutenberg book id.
        book_id: u32,
        /// Reported unknown-token count.
        unmappable_count: u64,
        /// Reported body-token count.
        body_token_count: u64,
    },
    /// A source's reported density did not match count/body_token_count.
    PerDocUnmappableDensityMismatch {
        /// Affected Gutenberg book id.
        book_id: u32,
        /// Recomputed density.
        expected: f64,
        /// Reported density.
        observed: f64,
        /// Reported unknown-token count.
        unmappable_count: u64,
        /// Reported body-token count.
        body_token_count: u64,
    },
    /// A source exceeded the inherited F-G2 per-document threshold but was retained.
    RetainedBookUnmappableDensityHigh {
        /// Affected Gutenberg book id.
        book_id: u32,
        /// Recomputed density.
        density: f64,
        /// Inherited per-document maximum density.
        max_density: f64,
    },
    /// The manifest aggregate did not match retained per-document accounting.
    CorpusUnmappableRateMismatch {
        /// Recomputed aggregate rate.
        expected: f64,
        /// Manifest aggregate rate.
        observed: f64,
        /// Aggregate unknown-token count.
        unmappable_count: u128,
        /// Aggregate body-token count.
        body_token_count: u128,
    },
    /// The aggregate retained-corpus rate exceeded the S4 D5 hard cap.
    CorpusUnmappableRateHigh {
        /// Recomputed aggregate rate.
        rate: f64,
        /// S4 D5 hard cap.
        max_rate: f64,
        /// Aggregate unknown-token count.
        unmappable_count: u128,
        /// Aggregate body-token count.
        body_token_count: u128,
    },
    /// Aggregate count did not fit the public report type.
    CountOverflow {
        /// Field whose count overflowed.
        field: &'static str,
    },
}

impl fmt::Display for S4UnmappableGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCorpusUnmappableRate { observed } => write!(
                f,
                "unmappable_rate_corpus must be finite and non-negative, got {observed}"
            ),
            Self::InvalidCorpusUnmappableRateCap { observed } => write!(
                f,
                "S4 D5 unmappable_rate_corpus cap must be finite and non-negative, got {observed}"
            ),
            Self::RetainedBookCountOverflow => {
                f.write_str("retained Gutenberg book count overflowed u32")
            }
            Self::MissingPerDocUnmappableField { book_id, field } => write!(
                f,
                "retained Gutenberg book {book_id} is missing {field} for S4 D5 unmappable accounting"
            ),
            Self::RetainedBookHasEmptyBody { book_id } => write!(
                f,
                "retained Gutenberg book {book_id} has zero post-charset body token ids"
            ),
            Self::PerDocUnmappableCountExceedsBodyTokens {
                book_id,
                unmappable_count,
                body_token_count,
            } => write!(
                f,
                "Gutenberg book {book_id} has unmappable_count={unmappable_count} greater than body_token_count={body_token_count}"
            ),
            Self::PerDocUnmappableDensityMismatch {
                book_id,
                expected,
                observed,
                unmappable_count,
                body_token_count,
            } => write!(
                f,
                "Gutenberg book {book_id} unmappable_density mismatch: expected {expected} from unmappable_count={unmappable_count} / body_token_count={body_token_count}, observed {observed}"
            ),
            Self::RetainedBookUnmappableDensityHigh {
                book_id,
                density,
                max_density,
            } => write!(
                f,
                "retained Gutenberg book {book_id} unmappable_density {density} exceeds inherited per-document cap {max_density}"
            ),
            Self::CorpusUnmappableRateMismatch {
                expected,
                observed,
                unmappable_count,
                body_token_count,
            } => write!(
                f,
                "unmappable_rate_corpus mismatch: expected {expected} from unmappable_count={unmappable_count} / body_token_count={body_token_count}, observed {observed}"
            ),
            Self::CorpusUnmappableRateHigh {
                rate,
                max_rate,
                unmappable_count,
                body_token_count,
            } => write!(
                f,
                "unmappable_rate_corpus {rate} exceeds S4 D5 hard cap {max_rate} (unmappable_count={unmappable_count}, body_token_count={body_token_count})"
            ),
            Self::CountOverflow { field } => {
                write!(f, "S4 D5 unmappable gate {field} overflowed u64")
            }
        }
    }
}

impl Error for S4UnmappableGateError {}

fn mean(values: &[u64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let sum: u128 = values.iter().map(|value| u128::from(*value)).sum();
    sum as f64 / values.len() as f64
}

fn median(values: &[u64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid] as f64
    } else {
        (sorted[mid - 1] as f64 + sorted[mid] as f64) / 2.0
    }
}

fn validate_finite_nonnegative(
    field: &'static str,
    value: f64,
) -> Result<(), S4CorpusQualityError> {
    if !value.is_finite() {
        return Err(S4CorpusQualityError::NonFiniteMetric { field, value });
    }
    if value < 0.0 {
        return Err(S4CorpusQualityError::NegativeMetric { field, value });
    }
    Ok(())
}
