//! S4 Project Gutenberg corpus manifest schema.

use std::error::Error;
use std::fmt;

use gbf_foundation::{CanonicalJson, DomainHash, Hash256};
use serde::{Deserialize, Deserializer, Serialize};

const CRATE_NAME: &str = "gbf-artifact";
const GUTENBERG_MANIFEST_SCHEMA_ID: &str = "gutenberg_manifest.v1";
const GUTENBERG_MANIFEST_SCHEMA_VERSION: &str = "1";
const GUTENBERG_SOURCE_NAME: &str = "Project Gutenberg";
const PUBLIC_DOMAIN_IN_USA: &str = "public_domain_in_usa";
const DEDUP_KIND: &str = "exact_post_strip_charset_body_sha";
const DEDUP_NOTES: &str = "Two retained books with identical post_charset_body_sha256 (i.e. identical body token-id streams excluding <bos>/<eos>) are treated as duplicates; only the lowest book_id is retained. Raw source_blob_sha256 is reported but is not the dedup key, because Gutenberg boilerplate divergence (release notes, edition metadata) can mask body-identical duplicates.";
const RAW_BYTE_POLICY: &str = "post-strip, post-charset_v1 token-id stream, one octet per token id; <bos>/<eos> inserted at book boundaries (id 80 / 81); <unk> id 82.";
const SPLIT_TRAIN_FRACTION: f64 = 0.90;
const SPLIT_VAL_FRACTION: f64 = 0.10;
const MIN_SPLIT_BYTE_LENGTH: u64 = 128;

/// S4 D5 hard cap for aggregate retained Gutenberg unmappable rate.
pub const GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX: f64 = 0.005;

/// Per-book source and normalization provenance in `gutenberg_manifest.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GutenbergSourceRecord {
    pub book_id: u32,
    pub title: String,
    pub author: String,
    pub source_landing_url: String,
    pub mirror_fetch_url: Option<String>,
    pub mirror_snapshot_id: Option<String>,
    pub selected_format: Option<String>,
    pub source_blob_sha256: Option<Hash256>,
    pub pre_strip_utf8_sha256: Option<Hash256>,
    #[serde(deserialize_with = "deserialize_gutenberg_license")]
    pub license: String,
    pub fetch_namespace_kind: Option<GutenbergFetchNamespaceKind>,
    pub fetch_namespace_id: Option<String>,
    pub compression_kind: Option<GutenbergCompressionKind>,
    pub archive_member_path: Option<String>,
    pub pre_strip_byte_length: Option<u64>,
    pub drop_reason: Option<GutenbergDropReason>,
    pub duplicate_of_book_id: Option<u32>,
    pub post_strip_byte_length: Option<u64>,
    pub post_strip_sha256: Option<Hash256>,
    pub post_charset_body_sha256: Option<Hash256>,
    pub post_charset_token_length: Option<u64>,
    pub unmappable_count: Option<u64>,
    pub unmappable_density: Option<f64>,
    pub split: Option<GutenbergSplit>,
}

impl GutenbergSourceRecord {
    /// Return the literal RFC license value.
    #[must_use]
    pub fn public_domain_in_usa_license() -> String {
        PUBLIC_DOMAIN_IN_USA.to_owned()
    }

    /// Validate source-record invariants needed before canonical write.
    pub fn validate(&self) -> Result<(), GutenbergManifestError> {
        if self.license != PUBLIC_DOMAIN_IN_USA {
            return Err(GutenbergManifestError::InvalidLiteral {
                field: "sources[].license",
                expected: PUBLIC_DOMAIN_IN_USA,
                observed: self.license.clone(),
            });
        }

        if let Some(unmappable_density) = self.unmappable_density
            && (!unmappable_density.is_finite() || !(0.0..=1.0).contains(&unmappable_density))
        {
            return Err(GutenbergManifestError::InvalidUnmappableDensity {
                book_id: self.book_id,
                value: unmappable_density,
            });
        }

        match (&self.drop_reason, &self.split) {
            (None, Some(_)) => {}
            (None, None) => {
                return Err(GutenbergManifestError::RetainedSourceMissingSplit {
                    book_id: self.book_id,
                });
            }
            (Some(_), None) => {}
            (Some(_), Some(split)) => {
                return Err(GutenbergManifestError::DroppedSourceHasSplit {
                    book_id: self.book_id,
                    split: *split,
                });
            }
        }

        match (
            self.drop_reason == Some(GutenbergDropReason::DedupCollision),
            self.duplicate_of_book_id,
        ) {
            (true, Some(_)) | (false, None) => {}
            (true, None) => {
                return Err(GutenbergManifestError::DedupDropMissingDuplicate {
                    book_id: self.book_id,
                });
            }
            (false, Some(duplicate_of_book_id)) => {
                return Err(GutenbergManifestError::UnexpectedDuplicateOfBookId {
                    book_id: self.book_id,
                    duplicate_of_book_id,
                });
            }
        }

        if self.drop_reason.is_none() {
            validate_retained_source_required_fields(self)?;
        } else {
            validate_dropped_source_g_ok_12(self)?;
        }

        Ok(())
    }
}

/// Namespace provenance for source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GutenbergFetchNamespaceKind {
    LocalPrivateMirror,
    OfficialRobotHarvest,
    ContentAddressedCache,
}

/// Compression wrapper for the selected source blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GutenbergCompressionKind {
    None,
    Gzip,
    Zip,
}

/// Manifest-level reason a source did not produce retained book bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GutenbergDropReason {
    NoSupportedPlaintextFormat,
    NoPlaintextArchiveMember,
    GutenbergMarkerMissing,
    SourceDecodeFailed,
    InvalidUtf8,
    AmbiguousPlaintextArchive,
    EmptyAfterStrip,
    UnmappableDensityHigh,
    DedupCollision,
}

/// Book split in the S4 train/validation partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GutenbergSplit {
    Train,
    Val,
}

/// Deduplication policy carried verbatim in `gutenberg_manifest.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GutenbergDedupPolicy {
    #[serde(deserialize_with = "deserialize_dedup_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_dedup_notes")]
    pub notes: String,
}

impl GutenbergDedupPolicy {
    /// Return the pinned S4 exact-body dedup policy.
    #[must_use]
    pub fn exact_post_strip_charset_body_sha() -> Self {
        Self {
            kind: DEDUP_KIND.to_owned(),
            notes: DEDUP_NOTES.to_owned(),
        }
    }

    fn validate(&self) -> Result<(), GutenbergManifestError> {
        validate_literal("dedup_policy.kind", DEDUP_KIND, &self.kind)?;
        validate_literal("dedup_policy.notes", DEDUP_NOTES, &self.notes)?;
        Ok(())
    }
}

/// S4 Project Gutenberg corpus manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GutenbergManifest {
    #[serde(deserialize_with = "deserialize_gutenberg_manifest_schema")]
    pub schema: String,
    #[serde(deserialize_with = "deserialize_gutenberg_source_name")]
    pub source_name: String,
    pub catalog_snapshot_url: String,
    pub catalog_snapshot_sha256: Hash256,
    pub catalog_snapshot_observed_at_utc: String,
    pub catalog_snapshot_last_modified_utc: Option<String>,
    pub selection_filter_canonical_json: String,
    pub selection_filter_sha256: Hash256,
    pub book_ids: Vec<u32>,
    pub sources: Vec<GutenbergSourceRecord>,
    pub header_regex_pattern: String,
    pub footer_regex_pattern: String,
    pub normalization_spec_self_hash: Hash256,
    pub dedup_policy: GutenbergDedupPolicy,
    pub split_seed_u128: String,
    pub split_train_fraction: f64,
    pub split_val_fraction: f64,
    pub train_path: String,
    pub val_path: String,
    pub train_sha256: Hash256,
    pub val_sha256: Hash256,
    pub train_byte_length: u64,
    pub val_byte_length: u64,
    pub train_book_count: u32,
    pub val_book_count: u32,
    pub drop_count_total: u32,
    pub drop_count_no_supported_plaintext_format: u32,
    pub drop_count_no_plaintext_archive_member: u32,
    pub drop_count_source_decode_failed: u32,
    pub drop_count_ambiguous_plaintext_archive: u32,
    pub drop_count_invalid_utf8: u32,
    pub drop_count_empty_after_strip: u32,
    pub drop_count_marker_missing: u32,
    pub drop_count_unmappable_density: u32,
    pub drop_count_dedup_collision: u32,
    pub unmappable_rate_corpus: f64,
    #[serde(deserialize_with = "deserialize_raw_byte_policy")]
    pub raw_byte_policy: String,
    pub retained_book_count_min: u32,
    pub manifest_self_hash: Hash256,
}

impl GutenbergManifest {
    /// Return the literal schema id.
    #[must_use]
    pub fn schema_id() -> String {
        GUTENBERG_MANIFEST_SCHEMA_ID.to_owned()
    }

    /// Return the literal source name.
    #[must_use]
    pub fn source_name_literal() -> String {
        GUTENBERG_SOURCE_NAME.to_owned()
    }

    /// Return the literal raw byte policy.
    #[must_use]
    pub fn raw_byte_policy_literal() -> String {
        RAW_BYTE_POLICY.to_owned()
    }

    /// Validate structure, compute, and fill `manifest_self_hash`.
    pub fn with_computed_self_hash(mut self) -> Result<Self, GutenbergManifestError> {
        self.validate_structure()?;
        self.manifest_self_hash = self.compute_self_hash()?;
        Ok(self)
    }

    /// Canonical JSON bytes including `manifest_self_hash`, after validation.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GutenbergManifestError> {
        self.validate_canonical_write()?;
        self.canonical_bytes_unchecked()
    }

    /// Canonical JSON bytes including `manifest_self_hash`, without validation.
    pub fn canonical_bytes_unchecked(&self) -> Result<Vec<u8>, GutenbergManifestError> {
        CanonicalJson::to_vec(self).map_err(GutenbergManifestError::CanonicalJson)
    }

    /// Self-hash over canonical JSON with `manifest_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, GutenbergManifestError> {
        let mut value = serde_json::to_value(self).map_err(GutenbergManifestError::Json)?;
        value
            .as_object_mut()
            .ok_or(GutenbergManifestError::ExpectedObjectForSelfHash)?
            .remove("manifest_self_hash");
        let canonical =
            CanonicalJson::value_to_vec(&value).map_err(GutenbergManifestError::CanonicalJson)?;
        Self::domain()
            .hash_canonical_bytes(&canonical)
            .map_err(GutenbergManifestError::CanonicalJson)
    }

    /// Validate all invariants enforced by the canonical writer.
    pub fn validate_canonical_write(&self) -> Result<(), GutenbergManifestError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.manifest_self_hash {
            return Err(GutenbergManifestError::SelfHashMismatch {
                expected: recomputed,
                observed: self.manifest_self_hash,
            });
        }
        Ok(())
    }

    /// DomainHash context for `gutenberg_manifest.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            CRATE_NAME,
            "GutenbergManifest",
            GUTENBERG_MANIFEST_SCHEMA_ID,
            GUTENBERG_MANIFEST_SCHEMA_VERSION,
        )
    }

    fn validate_structure(&self) -> Result<(), GutenbergManifestError> {
        validate_literal("schema", GUTENBERG_MANIFEST_SCHEMA_ID, &self.schema)?;
        validate_literal("source_name", GUTENBERG_SOURCE_NAME, &self.source_name)?;
        self.dedup_policy.validate()?;
        validate_literal("raw_byte_policy", RAW_BYTE_POLICY, &self.raw_byte_policy)?;

        if self.split_train_fraction != SPLIT_TRAIN_FRACTION {
            return Err(GutenbergManifestError::InvalidSplitFraction {
                field: "split_train_fraction",
                expected: SPLIT_TRAIN_FRACTION,
                observed: self.split_train_fraction,
            });
        }
        if self.split_val_fraction != SPLIT_VAL_FRACTION {
            return Err(GutenbergManifestError::InvalidSplitFraction {
                field: "split_val_fraction",
                expected: SPLIT_VAL_FRACTION,
                observed: self.split_val_fraction,
            });
        }
        if self.retained_book_count_min == 0 {
            return Err(GutenbergManifestError::InvalidRetainedBookCountMin {
                observed: self.retained_book_count_min,
            });
        }
        validate_split_byte_length("train_byte_length", self.train_byte_length)?;
        validate_split_byte_length("val_byte_length", self.val_byte_length)?;
        let retained_count = self
            .train_book_count
            .checked_add(self.val_book_count)
            .ok_or(GutenbergManifestError::RetainedBookCountOverflow {
                train_book_count: self.train_book_count,
                val_book_count: self.val_book_count,
            })?;
        if retained_count < self.retained_book_count_min {
            return Err(GutenbergManifestError::RetainedBookCountBelowFloor {
                retained_book_count: retained_count,
                retained_book_count_min: self.retained_book_count_min,
            });
        }
        if !self.unmappable_rate_corpus.is_finite()
            || !(0.0..=GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX).contains(&self.unmappable_rate_corpus)
        {
            return Err(GutenbergManifestError::InvalidUnmappableRateCorpus {
                value: self.unmappable_rate_corpus,
            });
        }
        if !is_lower_hex_32(&self.split_seed_u128) {
            return Err(GutenbergManifestError::InvalidSplitSeed {
                value: self.split_seed_u128.clone(),
            });
        }

        validate_book_ids(&self.book_ids)?;
        validate_sources(self)?;
        validate_drop_counts(self)?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum GutenbergManifestError {
    InvalidLiteral {
        field: &'static str,
        expected: &'static str,
        observed: String,
    },
    InvalidSplitFraction {
        field: &'static str,
        expected: f64,
        observed: f64,
    },
    InvalidRetainedBookCountMin {
        observed: u32,
    },
    InvalidSplitByteLength {
        field: &'static str,
        min: u64,
        observed: u64,
    },
    RetainedBookCountOverflow {
        train_book_count: u32,
        val_book_count: u32,
    },
    RetainedBookCountBelowFloor {
        retained_book_count: u32,
        retained_book_count_min: u32,
    },
    InvalidUnmappableRateCorpus {
        value: f64,
    },
    InvalidSplitSeed {
        value: String,
    },
    BookIdsNotSorted,
    BookIdsContainDuplicate {
        book_id: u32,
    },
    SourceCountMismatch {
        book_ids: usize,
        sources: usize,
    },
    SourceBookIdMismatch {
        index: usize,
        expected: u32,
        observed: u32,
    },
    RetainedSourceMissingSplit {
        book_id: u32,
    },
    DroppedSourceHasSplit {
        book_id: u32,
        split: GutenbergSplit,
    },
    DedupDropMissingDuplicate {
        book_id: u32,
    },
    UnexpectedDuplicateOfBookId {
        book_id: u32,
        duplicate_of_book_id: u32,
    },
    DropReasonFieldMustBeNull {
        book_id: u32,
        reason: GutenbergDropReason,
        field: &'static str,
    },
    DropReasonFieldMustBePresent {
        book_id: u32,
        reason: GutenbergDropReason,
        field: &'static str,
    },
    DropReasonCompressionKindMismatch {
        book_id: u32,
        reason: GutenbergDropReason,
        expected: GutenbergCompressionKind,
        observed: Option<GutenbergCompressionKind>,
    },
    RetainedSourceMissingField {
        book_id: u32,
        field: &'static str,
    },
    InvalidUnmappableDensity {
        book_id: u32,
        value: f64,
    },
    CountOverflow {
        field: &'static str,
        actual: usize,
    },
    DropCountMismatch {
        field: &'static str,
        expected: u32,
        observed: u32,
    },
    SelfHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    ExpectedObjectForSelfHash,
    Json(serde_json::Error),
    CanonicalJson(gbf_foundation::CanonicalJsonError),
}

impl fmt::Display for GutenbergManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLiteral {
                field,
                expected,
                observed,
            } => write!(
                f,
                "expected {field} {expected:?} in gutenberg_manifest.v1, got {observed:?}"
            ),
            Self::InvalidSplitFraction {
                field,
                expected,
                observed,
            } => write!(
                f,
                "expected {field} {expected} in gutenberg_manifest.v1, got {observed}"
            ),
            Self::InvalidRetainedBookCountMin { observed } => write!(
                f,
                "retained_book_count_min must be positive in gutenberg_manifest.v1, got {observed}"
            ),
            Self::InvalidSplitByteLength {
                field,
                min,
                observed,
            } => write!(
                f,
                "{field} must be at least S4 sequence length {min} in gutenberg_manifest.v1, got {observed}"
            ),
            Self::RetainedBookCountOverflow {
                train_book_count,
                val_book_count,
            } => write!(
                f,
                "train_book_count {train_book_count} + val_book_count {val_book_count} overflowed in gutenberg_manifest.v1"
            ),
            Self::RetainedBookCountBelowFloor {
                retained_book_count,
                retained_book_count_min,
            } => write!(
                f,
                "retained Gutenberg book count {retained_book_count} is below retained_book_count_min {retained_book_count_min}"
            ),
            Self::InvalidUnmappableRateCorpus { value } => write!(
                f,
                "unmappable_rate_corpus must be finite and in [0, {GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX}], got {value}"
            ),
            Self::InvalidSplitSeed { value } => write!(
                f,
                "split_seed_u128 must be 32 lowercase hex characters, got {value:?}"
            ),
            Self::BookIdsNotSorted => f.write_str("book_ids must be sorted ascending"),
            Self::BookIdsContainDuplicate { book_id } => {
                write!(f, "book_ids contains duplicate book id {book_id}")
            }
            Self::SourceCountMismatch { book_ids, sources } => write!(
                f,
                "book_ids has {book_ids} entries but sources has {sources} entries"
            ),
            Self::SourceBookIdMismatch {
                index,
                expected,
                observed,
            } => write!(
                f,
                "sources[{index}].book_id must match book_ids[{index}] {expected}, got {observed}"
            ),
            Self::RetainedSourceMissingSplit { book_id } => {
                write!(f, "retained source {book_id} must have split")
            }
            Self::DroppedSourceHasSplit { book_id, split } => {
                write!(f, "dropped source {book_id} must not have split {split:?}")
            }
            Self::DedupDropMissingDuplicate { book_id } => write!(
                f,
                "source {book_id} has dedup_collision drop_reason but no duplicate_of_book_id"
            ),
            Self::UnexpectedDuplicateOfBookId {
                book_id,
                duplicate_of_book_id,
            } => write!(
                f,
                "source {book_id} records duplicate_of_book_id {duplicate_of_book_id} without dedup_collision"
            ),
            Self::DropReasonFieldMustBeNull {
                book_id,
                reason,
                field,
            } => write!(
                f,
                "source {book_id} with drop_reason {reason:?} must have {field}=null"
            ),
            Self::DropReasonFieldMustBePresent {
                book_id,
                reason,
                field,
            } => write!(
                f,
                "source {book_id} with drop_reason {reason:?} must have non-null {field}"
            ),
            Self::DropReasonCompressionKindMismatch {
                book_id,
                reason,
                expected,
                observed,
            } => write!(
                f,
                "source {book_id} with drop_reason {reason:?} must have compression_kind {expected:?}, got {observed:?}"
            ),
            Self::RetainedSourceMissingField { book_id, field } => write!(
                f,
                "retained source {book_id} is missing required computed field {field}"
            ),
            Self::InvalidUnmappableDensity { book_id, value } => write!(
                f,
                "source {book_id} unmappable_density must be finite and in [0, 1], got {value}"
            ),
            Self::CountOverflow { field, actual } => write!(
                f,
                "{field} count {actual} exceeds u32 range in gutenberg_manifest.v1"
            ),
            Self::DropCountMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "{field} must be {expected} from sources in gutenberg_manifest.v1, got {observed}"
            ),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "gutenberg manifest self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("gutenberg manifest self-hash requires a top-level object")
            }
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for GutenbergManifestError {}

fn validate_retained_source_required_fields(
    source: &GutenbergSourceRecord,
) -> Result<(), GutenbergManifestError> {
    for (field, present) in [
        ("selected_format", source.selected_format.is_some()),
        ("source_blob_sha256", source.source_blob_sha256.is_some()),
        (
            "pre_strip_utf8_sha256",
            source.pre_strip_utf8_sha256.is_some(),
        ),
        ("compression_kind", source.compression_kind.is_some()),
        (
            "pre_strip_byte_length",
            source.pre_strip_byte_length.is_some(),
        ),
        (
            "post_strip_byte_length",
            source.post_strip_byte_length.is_some(),
        ),
        ("post_strip_sha256", source.post_strip_sha256.is_some()),
        (
            "post_charset_body_sha256",
            source.post_charset_body_sha256.is_some(),
        ),
        (
            "post_charset_token_length",
            source.post_charset_token_length.is_some(),
        ),
        ("unmappable_count", source.unmappable_count.is_some()),
        ("unmappable_density", source.unmappable_density.is_some()),
    ] {
        if !present {
            return Err(GutenbergManifestError::RetainedSourceMissingField {
                book_id: source.book_id,
                field,
            });
        }
    }
    Ok(())
}

fn validate_dropped_source_g_ok_12(
    source: &GutenbergSourceRecord,
) -> Result<(), GutenbergManifestError> {
    let reason = source
        .drop_reason
        .expect("validate_dropped_source_g_ok_12 is called only for dropped sources");
    match reason {
        GutenbergDropReason::NoSupportedPlaintextFormat => {
            require_null(
                source.mirror_fetch_url.as_ref(),
                source.book_id,
                reason,
                "mirror_fetch_url",
            )?;
            require_null(
                source.mirror_snapshot_id.as_ref(),
                source.book_id,
                reason,
                "mirror_snapshot_id",
            )?;
            require_null(
                source.selected_format.as_ref(),
                source.book_id,
                reason,
                "selected_format",
            )?;
            require_null(
                source.source_blob_sha256.as_ref(),
                source.book_id,
                reason,
                "source_blob_sha256",
            )?;
            require_null(
                source.pre_strip_utf8_sha256.as_ref(),
                source.book_id,
                reason,
                "pre_strip_utf8_sha256",
            )?;
            require_null(
                source.pre_strip_byte_length.as_ref(),
                source.book_id,
                reason,
                "pre_strip_byte_length",
            )?;
        }
        GutenbergDropReason::NoPlaintextArchiveMember
        | GutenbergDropReason::AmbiguousPlaintextArchive => {
            require_present(
                source.source_blob_sha256.as_ref(),
                source.book_id,
                reason,
                "source_blob_sha256",
            )?;
            require_compression_kind(source, reason, GutenbergCompressionKind::Zip)?;
            require_null(
                source.archive_member_path.as_ref(),
                source.book_id,
                reason,
                "archive_member_path",
            )?;
            require_null(
                source.pre_strip_utf8_sha256.as_ref(),
                source.book_id,
                reason,
                "pre_strip_utf8_sha256",
            )?;
            require_null(
                source.pre_strip_byte_length.as_ref(),
                source.book_id,
                reason,
                "pre_strip_byte_length",
            )?;
        }
        GutenbergDropReason::SourceDecodeFailed => {
            require_present(
                source.source_blob_sha256.as_ref(),
                source.book_id,
                reason,
                "source_blob_sha256",
            )?;
            require_present(
                source.selected_format.as_ref(),
                source.book_id,
                reason,
                "selected_format",
            )?;
            require_null(
                source.pre_strip_utf8_sha256.as_ref(),
                source.book_id,
                reason,
                "pre_strip_utf8_sha256",
            )?;
            require_null(
                source.pre_strip_byte_length.as_ref(),
                source.book_id,
                reason,
                "pre_strip_byte_length",
            )?;
        }
        GutenbergDropReason::GutenbergMarkerMissing
        | GutenbergDropReason::InvalidUtf8
        | GutenbergDropReason::EmptyAfterStrip
        | GutenbergDropReason::UnmappableDensityHigh
        | GutenbergDropReason::DedupCollision => {
            require_present(
                source.source_blob_sha256.as_ref(),
                source.book_id,
                reason,
                "source_blob_sha256",
            )?;
            require_present(
                source.selected_format.as_ref(),
                source.book_id,
                reason,
                "selected_format",
            )?;
            require_present(
                source.pre_strip_utf8_sha256.as_ref(),
                source.book_id,
                reason,
                "pre_strip_utf8_sha256",
            )?;
            require_present(
                source.pre_strip_byte_length.as_ref(),
                source.book_id,
                reason,
                "pre_strip_byte_length",
            )?;
        }
    }
    Ok(())
}

fn require_null<T>(
    value: Option<&T>,
    book_id: u32,
    reason: GutenbergDropReason,
    field: &'static str,
) -> Result<(), GutenbergManifestError> {
    if value.is_none() {
        Ok(())
    } else {
        Err(GutenbergManifestError::DropReasonFieldMustBeNull {
            book_id,
            reason,
            field,
        })
    }
}

fn require_present<T>(
    value: Option<&T>,
    book_id: u32,
    reason: GutenbergDropReason,
    field: &'static str,
) -> Result<(), GutenbergManifestError> {
    if value.is_some() {
        Ok(())
    } else {
        Err(GutenbergManifestError::DropReasonFieldMustBePresent {
            book_id,
            reason,
            field,
        })
    }
}

fn require_compression_kind(
    source: &GutenbergSourceRecord,
    reason: GutenbergDropReason,
    expected: GutenbergCompressionKind,
) -> Result<(), GutenbergManifestError> {
    if source.compression_kind == Some(expected) {
        Ok(())
    } else {
        Err(GutenbergManifestError::DropReasonCompressionKindMismatch {
            book_id: source.book_id,
            reason,
            expected,
            observed: source.compression_kind,
        })
    }
}

fn validate_split_byte_length(
    field: &'static str,
    observed: u64,
) -> Result<(), GutenbergManifestError> {
    if observed >= MIN_SPLIT_BYTE_LENGTH {
        Ok(())
    } else {
        Err(GutenbergManifestError::InvalidSplitByteLength {
            field,
            min: MIN_SPLIT_BYTE_LENGTH,
            observed,
        })
    }
}

fn validate_book_ids(book_ids: &[u32]) -> Result<(), GutenbergManifestError> {
    for pair in book_ids.windows(2) {
        let previous = pair[0];
        let next = pair[1];
        if previous == next {
            return Err(GutenbergManifestError::BookIdsContainDuplicate { book_id: next });
        }
        if previous > next {
            return Err(GutenbergManifestError::BookIdsNotSorted);
        }
    }
    Ok(())
}

fn validate_sources(manifest: &GutenbergManifest) -> Result<(), GutenbergManifestError> {
    if manifest.book_ids.len() != manifest.sources.len() {
        return Err(GutenbergManifestError::SourceCountMismatch {
            book_ids: manifest.book_ids.len(),
            sources: manifest.sources.len(),
        });
    }

    for (index, (expected, source)) in manifest
        .book_ids
        .iter()
        .copied()
        .zip(&manifest.sources)
        .enumerate()
    {
        if source.book_id != expected {
            return Err(GutenbergManifestError::SourceBookIdMismatch {
                index,
                expected,
                observed: source.book_id,
            });
        }
        source.validate()?;
    }
    Ok(())
}

fn validate_drop_counts(manifest: &GutenbergManifest) -> Result<(), GutenbergManifestError> {
    let total_drops = checked_source_count(
        "drop_count_total",
        manifest
            .sources
            .iter()
            .filter(|source| source.drop_reason.is_some())
            .count(),
    )?;
    validate_count("drop_count_total", total_drops, manifest.drop_count_total)?;

    let train = checked_source_count(
        "train_book_count",
        manifest
            .sources
            .iter()
            .filter(|source| source.split == Some(GutenbergSplit::Train))
            .count(),
    )?;
    validate_count("train_book_count", train, manifest.train_book_count)?;

    let val = checked_source_count(
        "val_book_count",
        manifest
            .sources
            .iter()
            .filter(|source| source.split == Some(GutenbergSplit::Val))
            .count(),
    )?;
    validate_count("val_book_count", val, manifest.val_book_count)?;

    validate_count(
        "drop_count_no_supported_plaintext_format",
        count_drop(
            manifest,
            GutenbergDropReason::NoSupportedPlaintextFormat,
            "drop_count_no_supported_plaintext_format",
        )?,
        manifest.drop_count_no_supported_plaintext_format,
    )?;
    validate_count(
        "drop_count_no_plaintext_archive_member",
        count_drop(
            manifest,
            GutenbergDropReason::NoPlaintextArchiveMember,
            "drop_count_no_plaintext_archive_member",
        )?,
        manifest.drop_count_no_plaintext_archive_member,
    )?;
    validate_count(
        "drop_count_source_decode_failed",
        count_drop(
            manifest,
            GutenbergDropReason::SourceDecodeFailed,
            "drop_count_source_decode_failed",
        )?,
        manifest.drop_count_source_decode_failed,
    )?;
    validate_count(
        "drop_count_ambiguous_plaintext_archive",
        count_drop(
            manifest,
            GutenbergDropReason::AmbiguousPlaintextArchive,
            "drop_count_ambiguous_plaintext_archive",
        )?,
        manifest.drop_count_ambiguous_plaintext_archive,
    )?;
    validate_count(
        "drop_count_invalid_utf8",
        count_drop(
            manifest,
            GutenbergDropReason::InvalidUtf8,
            "drop_count_invalid_utf8",
        )?,
        manifest.drop_count_invalid_utf8,
    )?;
    validate_count(
        "drop_count_empty_after_strip",
        count_drop(
            manifest,
            GutenbergDropReason::EmptyAfterStrip,
            "drop_count_empty_after_strip",
        )?,
        manifest.drop_count_empty_after_strip,
    )?;
    validate_count(
        "drop_count_marker_missing",
        count_drop(
            manifest,
            GutenbergDropReason::GutenbergMarkerMissing,
            "drop_count_marker_missing",
        )?,
        manifest.drop_count_marker_missing,
    )?;
    validate_count(
        "drop_count_unmappable_density",
        count_drop(
            manifest,
            GutenbergDropReason::UnmappableDensityHigh,
            "drop_count_unmappable_density",
        )?,
        manifest.drop_count_unmappable_density,
    )?;
    validate_count(
        "drop_count_dedup_collision",
        count_drop(
            manifest,
            GutenbergDropReason::DedupCollision,
            "drop_count_dedup_collision",
        )?,
        manifest.drop_count_dedup_collision,
    )?;

    Ok(())
}

fn count_drop(
    manifest: &GutenbergManifest,
    reason: GutenbergDropReason,
    field: &'static str,
) -> Result<u32, GutenbergManifestError> {
    checked_source_count(
        field,
        manifest
            .sources
            .iter()
            .filter(|source| source.drop_reason == Some(reason))
            .count(),
    )
}

fn checked_source_count(field: &'static str, actual: usize) -> Result<u32, GutenbergManifestError> {
    u32::try_from(actual).map_err(|_| GutenbergManifestError::CountOverflow { field, actual })
}

fn validate_count(
    field: &'static str,
    expected: u32,
    observed: u32,
) -> Result<(), GutenbergManifestError> {
    if expected == observed {
        Ok(())
    } else {
        Err(GutenbergManifestError::DropCountMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn validate_literal(
    field: &'static str,
    expected: &'static str,
    observed: &str,
) -> Result<(), GutenbergManifestError> {
    if observed == expected {
        Ok(())
    } else {
        Err(GutenbergManifestError::InvalidLiteral {
            field,
            expected,
            observed: observed.to_owned(),
        })
    }
}

fn is_lower_hex_32(value: &str) -> bool {
    value.len() == 32
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn deserialize_gutenberg_manifest_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, GUTENBERG_MANIFEST_SCHEMA_ID, "schema")
}

fn deserialize_gutenberg_source_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, GUTENBERG_SOURCE_NAME, "source_name")
}

fn deserialize_gutenberg_license<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, PUBLIC_DOMAIN_IN_USA, "sources[].license")
}

fn deserialize_dedup_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, DEDUP_KIND, "dedup_policy.kind")
}

fn deserialize_dedup_notes<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, DEDUP_NOTES, "dedup_policy.notes")
}

fn deserialize_raw_byte_policy<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_literal(deserializer, RAW_BYTE_POLICY, "raw_byte_policy")
}

fn deserialize_literal<'de, D>(
    deserializer: D,
    expected: &'static str,
    field: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == expected {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "expected {field} {expected:?}, got {value:?}"
        )))
    }
}
