//! S4 cross-corpus contamination surface.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use gbf_artifact::{BOS_ID, EOS_ID};
use gbf_foundation::{CanonicalJson, DomainHash, Hash256, sha256};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::s4::schema::HypothesisStatus;

/// Schema id for the S4 cross-corpus contamination report.
pub const S4_CONTAMINATION_REPORT_SCHEMA: &str = "s4_contamination_report.v1";

/// D6 n-gram width in charset_v1 token ids.
pub const S4_CONTAMINATION_NGRAM_N: usize = 13;

/// D6 fingerprint index kind.
pub const S4_CONTAMINATION_FINGERPRINT_KIND: &str = "sha256_high_u64";

/// D6 collision disambiguation rule.
pub const S4_CONTAMINATION_COLLISION_DISAMBIGUATION: &str = "exact_13_token_bytes_on_hit";

/// D6 diagnostic sample cap per split, measured in token ids.
pub const S4_CONTAMINATION_DIAGNOSTIC_SAMPLE_CAP_TOKEN_IDS_PER_SPLIT: usize = 1_048_576;

/// D6 hard-fail threshold for closure-gated directions.
pub const S4_CONTAMINATION_HARD_FAIL_THRESHOLD: f64 = 0.0010;

/// D6 warning threshold for closure-gated directions.
pub const S4_CONTAMINATION_WARN_THRESHOLD: f64 = 0.0005;

/// Structured event emitted when D6 contamination starts.
pub const S4_CONTAMINATION_STARTED_EVENT: &str = "s4_contamination_started";

/// Structured event emitted for each measured contamination direction.
pub const S4_CONTAMINATION_DIRECTION_EVENT: &str = "s4_contamination_direction";

/// Structured event emitted after the D6 contamination outcome is derived.
pub const S4_CONTAMINATION_OUTCOME_EVENT: &str = "s4_contamination_outcome";

/// Tracing target for S4 contamination events.
pub const S4_CONTAMINATION_LOG_TARGET: &str = "gbf_experiments::s4::contamination";

/// RFC D6 threshold provenance marker carried in logs and reports.
pub const S4_CONTAMINATION_THRESHOLD_PROVENANCE: &str = "D6 [ESTIMATE for review]";

/// RFC estimate marker for threshold provenance.
pub const S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG: &str = "[ESTIMATE]";

const S4_CONTAMINATION_REPORT_SCHEMA_VERSION: &str = "1";
const GATED_DENOMINATOR_POLICY: &str = "full validation split against full opposite train split";
const DIAGNOSTIC_DENOMINATOR_POLICY: &str = "train/train diagnostic directions use deterministic per-document stratified samples if cap applies; validation/validation diagnostic directions reuse full validation sets";
const DIAGNOSTIC_NOT_AVAILABLE: &str = "diagnostic_not_available";
const CONTAMINATION_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4ContaminationReport",
    S4_CONTAMINATION_REPORT_SCHEMA,
    S4_CONTAMINATION_REPORT_SCHEMA_VERSION,
);

type TokenWindow = [u8; S4_CONTAMINATION_NGRAM_N];

/// One normalized corpus split, retaining document boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossCorpusSplit {
    /// SHA-256 carried into the report for this split.
    pub split_sha: Hash256,
    /// Charset-v1 token-id documents; 13-grams never cross these boundaries.
    pub documents: Vec<Vec<u8>>,
}

impl CrossCorpusSplit {
    /// Build a split from pre-segmented documents and an externally pinned hash.
    #[must_use]
    pub fn new(split_sha: Hash256, documents: Vec<Vec<u8>>) -> Self {
        Self {
            split_sha,
            documents,
        }
    }

    /// Fixture helper that hashes the concatenated document bytes.
    #[must_use]
    pub fn from_fixture_documents(documents: Vec<Vec<u8>>) -> Self {
        let mut bytes = Vec::new();
        for document in &documents {
            bytes.extend_from_slice(document);
        }
        Self::new(sha256(bytes), documents)
    }
}

/// Inputs to `s4_cross_corpus_contamination`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossCorpusInputs {
    /// TinyStories manifest self-hash.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// TinyStories training split.
    pub ts_train: CrossCorpusSplit,
    /// TinyStories validation split.
    pub ts_val: CrossCorpusSplit,
    /// Gutenberg training split.
    pub gb_train: CrossCorpusSplit,
    /// Gutenberg validation split.
    pub gb_val: CrossCorpusSplit,
}

/// Contamination directions pinned by D6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContaminationDirection {
    /// Closure-gated direction: TS train contains Gutenberg validation.
    #[serde(rename = "TS_train_contains_GB_val")]
    TsTrainContainsGbVal,
    /// Closure-gated direction: Gutenberg train contains TinyStories validation.
    #[serde(rename = "GB_train_contains_TS_val")]
    GbTrainContainsTsVal,
    /// Diagnostic direction: TS train contains Gutenberg train.
    #[serde(rename = "TS_train_contains_GB_train")]
    TsTrainContainsGbTrain,
    /// Diagnostic direction: Gutenberg train contains TinyStories train.
    #[serde(rename = "GB_train_contains_TS_train")]
    GbTrainContainsTsTrain,
    /// Diagnostic direction: TinyStories validation overlaps Gutenberg validation.
    #[serde(rename = "TS_val_overlaps_GB_val")]
    TsValOverlapsGbVal,
    /// Diagnostic direction: Gutenberg validation overlaps TinyStories validation.
    #[serde(rename = "GB_val_overlaps_TS_val")]
    GbValOverlapsTsVal,
}

impl ContaminationDirection {
    /// Stable RFC label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TsTrainContainsGbVal => "TS_train_contains_GB_val",
            Self::GbTrainContainsTsVal => "GB_train_contains_TS_val",
            Self::TsTrainContainsGbTrain => "TS_train_contains_GB_train",
            Self::GbTrainContainsTsTrain => "GB_train_contains_TS_train",
            Self::TsValOverlapsGbVal => "TS_val_overlaps_GB_val",
            Self::GbValOverlapsTsVal => "GB_val_overlaps_TS_val",
        }
    }
}

impl fmt::Display for ContaminationDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Threshold class that produced a contamination finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContaminationFindingKind {
    /// Non-gating warning threshold.
    Warning,
    /// Gating hard-fail threshold.
    HardFailure,
}

/// One threshold finding for a gated contamination direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContaminationFinding {
    /// Finding class.
    pub kind: ContaminationFindingKind,
    /// Direction that crossed the threshold.
    pub direction: ContaminationDirection,
    /// Computed overlap fraction.
    pub overlap_fraction: f64,
    /// Threshold that was crossed.
    pub threshold: f64,
    /// Unique exact 13-token windows in the intersection.
    pub overlap_count: u64,
    /// Unique exact 13-token windows in the denominator set.
    pub denominator_count: u64,
}

/// D6 threshold provenance carried by `s4_contamination_report.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContaminationThresholdProvenance {
    /// Hard-fail threshold from D6.
    pub hard_fail_threshold: f64,
    /// Warning threshold from D6.
    pub warn_threshold: f64,
    /// RFC section and review status for these thresholds.
    pub provenance: String,
    /// Literal estimate marker required by the review contract.
    pub estimate_tag: String,
}

impl ContaminationThresholdProvenance {
    /// Return the D6 threshold provenance.
    #[must_use]
    pub fn d6_estimate() -> Self {
        Self {
            hard_fail_threshold: S4_CONTAMINATION_HARD_FAIL_THRESHOLD,
            warn_threshold: S4_CONTAMINATION_WARN_THRESHOLD,
            provenance: S4_CONTAMINATION_THRESHOLD_PROVENANCE.to_owned(),
            estimate_tag: S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG.to_owned(),
        }
    }

    fn validate(&self) -> Result<(), S4ContaminationError> {
        if self.hard_fail_threshold != S4_CONTAMINATION_HARD_FAIL_THRESHOLD {
            return Err(S4ContaminationError::InvalidThresholdProvenance {
                field: "threshold_provenance.hard_fail_threshold",
            });
        }
        if self.warn_threshold != S4_CONTAMINATION_WARN_THRESHOLD {
            return Err(S4ContaminationError::InvalidThresholdProvenance {
                field: "threshold_provenance.warn_threshold",
            });
        }
        if self.provenance != S4_CONTAMINATION_THRESHOLD_PROVENANCE {
            return Err(S4ContaminationError::InvalidThresholdProvenance {
                field: "threshold_provenance.provenance",
            });
        }
        if self.estimate_tag != S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG {
            return Err(S4ContaminationError::InvalidThresholdProvenance {
                field: "threshold_provenance.estimate_tag",
            });
        }
        Ok(())
    }
}

/// D6 denominator policy carried in the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContaminationDenominatorPolicy {
    /// Closure-gated denominator policy.
    pub gated_directions: String,
    /// Diagnostic-only denominator policy.
    pub diagnostic_directions: String,
}

impl ContaminationDenominatorPolicy {
    /// Return the pinned D6 denominator policy text.
    #[must_use]
    pub fn d6() -> Self {
        Self {
            gated_directions: GATED_DENOMINATOR_POLICY.to_owned(),
            diagnostic_directions: DIAGNOSTIC_DENOMINATOR_POLICY.to_owned(),
        }
    }
}

/// Diagnostic-only overlap value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiagnosticOverlap {
    /// A diagnostic direction had a non-empty denominator.
    Fraction(f64),
    /// A diagnostic sample produced no 13-grams; H2 gating is unaffected.
    DiagnosticNotAvailable,
}

impl DiagnosticOverlap {
    fn from_measurement(measurement: OverlapMeasurement) -> Self {
        measurement
            .fraction
            .map_or(Self::DiagnosticNotAvailable, DiagnosticOverlap::Fraction)
    }

    fn validate(self, field: &'static str) -> Result<(), S4ContaminationError> {
        match self {
            Self::Fraction(value) => validate_fraction(field, value),
            Self::DiagnosticNotAvailable => Ok(()),
        }
    }

    fn status(self) -> &'static str {
        match self {
            Self::Fraction(_) => "available",
            Self::DiagnosticNotAvailable => DIAGNOSTIC_NOT_AVAILABLE,
        }
    }
}

impl Serialize for DiagnosticOverlap {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Fraction(value) => serializer.serialize_f64(*value),
            Self::DiagnosticNotAvailable => serializer.serialize_str(DIAGNOSTIC_NOT_AVAILABLE),
        }
    }
}

impl<'de> Deserialize<'de> for DiagnosticOverlap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DiagnosticOverlapVisitor)
    }
}

struct DiagnosticOverlapVisitor;

impl<'de> Visitor<'de> for DiagnosticOverlapVisitor {
    type Value = DiagnosticOverlap;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a finite overlap fraction or diagnostic_not_available")
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(DiagnosticOverlap::Fraction(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value < 0 {
            return Err(E::custom("diagnostic overlap fraction cannot be negative"));
        }
        Ok(DiagnosticOverlap::Fraction(value as f64))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(DiagnosticOverlap::Fraction(value as f64))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value == DIAGNOSTIC_NOT_AVAILABLE {
            Ok(DiagnosticOverlap::DiagnosticNotAvailable)
        } else {
            Err(E::unknown_variant(value, &[DIAGNOSTIC_NOT_AVAILABLE]))
        }
    }
}

/// Overall D6 contamination outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum ContaminationOutcome {
    /// No gated direction reached the warning threshold.
    Clean,
    /// At least one gated direction reached the warning threshold but none hard-failed.
    Warn {
        /// Warning direction labels.
        findings: Vec<ContaminationDirection>,
    },
    /// At least one gated direction exceeded the hard-fail threshold.
    HardFail {
        /// Hard-fail direction labels.
        failures: Vec<ContaminationDirection>,
        /// Warning direction labels.
        warnings: Vec<ContaminationDirection>,
    },
}

impl ContaminationOutcome {
    fn kind_str(&self) -> &'static str {
        match self {
            Self::Clean => "Clean",
            Self::Warn { .. } => "Warn",
            Self::HardFail { .. } => "HardFail",
        }
    }
}

/// `s4_contamination_report.v1` artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossCorpusReport {
    /// Schema id, always `s4_contamination_report.v1`.
    pub schema: String,
    /// TinyStories manifest self-hash.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// TinyStories train split SHA-256.
    pub ts_train_sha: Hash256,
    /// TinyStories validation split SHA-256.
    pub ts_val_sha: Hash256,
    /// Gutenberg train split SHA-256.
    pub gb_train_sha: Hash256,
    /// Gutenberg validation split SHA-256.
    pub gb_val_sha: Hash256,
    /// D6 n-gram width.
    pub n: u64,
    /// D6 fingerprint index kind.
    pub fingerprint_kind: String,
    /// D6 collision disambiguation rule.
    pub collision_disambiguation: String,
    /// D6 hard/warn threshold values and `[ESTIMATE]` provenance.
    pub threshold_provenance: ContaminationThresholdProvenance,
    /// Unique full TinyStories validation 13-grams.
    pub fingerprint_count_ts_val_ngrams: u64,
    /// Unique full Gutenberg validation 13-grams.
    pub fingerprint_count_gb_val_ngrams: u64,
    /// Unique diagnostic-sampled TinyStories train 13-grams.
    pub fingerprint_count_ts_train_ngrams: u64,
    /// Unique diagnostic-sampled Gutenberg train 13-grams.
    pub fingerprint_count_gb_train_ngrams: u64,
    /// Gated exact overlap: TS_train contains GB_val.
    pub overlap_ts_train_to_gb_val: f64,
    /// Gated exact overlap: GB_train contains TS_val.
    pub overlap_gb_train_to_ts_val: f64,
    /// Diagnostic overlap: TS_train contains GB_train.
    pub overlap_ts_train_contains_gb_train: DiagnosticOverlap,
    /// Diagnostic overlap: GB_train contains TS_train.
    pub overlap_gb_train_contains_ts_train: DiagnosticOverlap,
    /// Diagnostic overlap: TS_val contains GB_val. Uses the full validation sets.
    pub overlap_ts_val_to_gb_val: DiagnosticOverlap,
    /// Diagnostic overlap: GB_val contains TS_val. Uses the full validation sets.
    pub overlap_gb_val_to_ts_val: DiagnosticOverlap,
    /// Denominator policy summary.
    pub denominator_policy: ContaminationDenominatorPolicy,
    /// Gated warning findings.
    pub warnings: Vec<ContaminationFinding>,
    /// Gated hard-fail findings.
    pub hard_failures: Vec<ContaminationFinding>,
    /// Overall contamination outcome.
    pub outcome: ContaminationOutcome,
    /// Self-hash over canonical JSON with this field omitted.
    pub contamination_self_hash: Hash256,
}

impl CrossCorpusReport {
    /// Canonical JSON bytes including `contamination_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4ContaminationError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4ContaminationError::CanonicalJson)
    }

    /// Compute the report self-hash with `contamination_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4ContaminationError> {
        let mut value = serde_json::to_value(self).map_err(S4ContaminationError::Json)?;
        value
            .as_object_mut()
            .ok_or(S4ContaminationError::ExpectedObjectForSelfHash)?
            .remove("contamination_self_hash");
        let canonical =
            CanonicalJson::value_to_vec(&value).map_err(S4ContaminationError::CanonicalJson)?;
        CONTAMINATION_DOMAIN
            .hash_canonical_bytes(&canonical)
            .map_err(S4ContaminationError::CanonicalJson)
    }

    /// Validate structure and self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4ContaminationError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.contamination_self_hash {
            return Err(S4ContaminationError::SelfHashMismatch {
                expected: recomputed,
                observed: self.contamination_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), S4ContaminationError> {
        if self.schema != S4_CONTAMINATION_REPORT_SCHEMA {
            return Err(S4ContaminationError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        if self.n != S4_CONTAMINATION_NGRAM_N as u64 {
            return Err(S4ContaminationError::InvalidN { observed: self.n });
        }
        validate_literal(
            "fingerprint_kind",
            S4_CONTAMINATION_FINGERPRINT_KIND,
            &self.fingerprint_kind,
        )?;
        validate_literal(
            "collision_disambiguation",
            S4_CONTAMINATION_COLLISION_DISAMBIGUATION,
            &self.collision_disambiguation,
        )?;
        self.threshold_provenance.validate()?;
        validate_fraction(
            "overlap_ts_train_to_gb_val",
            self.overlap_ts_train_to_gb_val,
        )?;
        validate_fraction(
            "overlap_gb_train_to_ts_val",
            self.overlap_gb_train_to_ts_val,
        )?;
        self.overlap_ts_train_contains_gb_train
            .validate("overlap_ts_train_contains_gb_train")?;
        self.overlap_gb_train_contains_ts_train
            .validate("overlap_gb_train_contains_ts_train")?;
        self.overlap_ts_val_to_gb_val
            .validate("overlap_ts_val_to_gb_val")?;
        self.overlap_gb_val_to_ts_val
            .validate("overlap_gb_val_to_ts_val")?;
        for finding in &self.warnings {
            validate_finding(finding, ContaminationFindingKind::Warning)?;
        }
        for finding in &self.hard_failures {
            validate_finding(finding, ContaminationFindingKind::HardFailure)?;
        }
        validate_outcome(&self.outcome, &self.warnings, &self.hard_failures)
    }
}

/// H2 verification result derived from a contamination report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H2ContaminationVerdict {
    /// H2 hypothesis status.
    pub status: HypothesisStatus,
    /// Whether S4 report decision must carry a contamination warning.
    pub contamination_warning: bool,
}

/// Run the D6 cross-corpus contamination operation.
pub fn s4_cross_corpus_contamination(
    inputs: CrossCorpusInputs,
) -> Result<CrossCorpusReport, S4ContaminationError> {
    emit_contamination_started(&inputs);

    let full_ts_train = FingerprintSet::from_documents(&inputs.ts_train.documents, sha256_high_u64);
    let full_ts_val = FingerprintSet::from_documents(&inputs.ts_val.documents, sha256_high_u64);
    let full_gb_train = FingerprintSet::from_documents(&inputs.gb_train.documents, sha256_high_u64);
    let full_gb_val = FingerprintSet::from_documents(&inputs.gb_val.documents, sha256_high_u64);

    require_nonempty_gated_split(SplitName::TsTrain, &full_ts_train)?;
    require_nonempty_gated_split(SplitName::TsVal, &full_ts_val)?;
    require_nonempty_gated_split(SplitName::GbTrain, &full_gb_train)?;
    require_nonempty_gated_split(SplitName::GbVal, &full_gb_val)?;

    let sampled_ts_train = diagnostic_fingerprint_set(&inputs.ts_train.documents);
    let sampled_gb_train = diagnostic_fingerprint_set(&inputs.gb_train.documents);

    let ts_train_to_gb_val = full_ts_train.overlap_against_denominator(&full_gb_val)?;
    let gb_train_to_ts_val = full_gb_train.overlap_against_denominator(&full_ts_val)?;
    let ts_train_contains_gb_train =
        sampled_ts_train.overlap_against_denominator(&sampled_gb_train)?;
    let gb_train_contains_ts_train =
        sampled_gb_train.overlap_against_denominator(&sampled_ts_train)?;
    let ts_val_to_gb_val = full_ts_val.overlap_against_denominator(&full_gb_val)?;
    let gb_val_to_ts_val = full_gb_val.overlap_against_denominator(&full_ts_val)?;

    emit_contamination_direction(
        ContaminationDirection::TsTrainContainsGbVal,
        DirectionKind::Gated,
        GATED_DENOMINATOR_POLICY,
        ts_train_to_gb_val,
    );
    emit_contamination_direction(
        ContaminationDirection::GbTrainContainsTsVal,
        DirectionKind::Gated,
        GATED_DENOMINATOR_POLICY,
        gb_train_to_ts_val,
    );
    emit_contamination_direction(
        ContaminationDirection::TsTrainContainsGbTrain,
        DirectionKind::Diagnostic,
        DIAGNOSTIC_DENOMINATOR_POLICY,
        ts_train_contains_gb_train,
    );
    emit_contamination_direction(
        ContaminationDirection::GbTrainContainsTsTrain,
        DirectionKind::Diagnostic,
        DIAGNOSTIC_DENOMINATOR_POLICY,
        gb_train_contains_ts_train,
    );
    emit_contamination_direction(
        ContaminationDirection::TsValOverlapsGbVal,
        DirectionKind::Diagnostic,
        "full validation split against full validation split",
        ts_val_to_gb_val,
    );
    emit_contamination_direction(
        ContaminationDirection::GbValOverlapsTsVal,
        DirectionKind::Diagnostic,
        "full validation split against full validation split",
        gb_val_to_ts_val,
    );

    let (warnings, hard_failures) = gated_findings(ts_train_to_gb_val, gb_train_to_ts_val)?;
    let outcome = contamination_outcome(&warnings, &hard_failures);

    let mut report = CrossCorpusReport {
        schema: S4_CONTAMINATION_REPORT_SCHEMA.to_owned(),
        tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: inputs.gutenberg_manifest_self_hash,
        ts_train_sha: inputs.ts_train.split_sha,
        ts_val_sha: inputs.ts_val.split_sha,
        gb_train_sha: inputs.gb_train.split_sha,
        gb_val_sha: inputs.gb_val.split_sha,
        n: S4_CONTAMINATION_NGRAM_N as u64,
        fingerprint_kind: S4_CONTAMINATION_FINGERPRINT_KIND.to_owned(),
        collision_disambiguation: S4_CONTAMINATION_COLLISION_DISAMBIGUATION.to_owned(),
        threshold_provenance: ContaminationThresholdProvenance::d6_estimate(),
        fingerprint_count_ts_val_ngrams: full_ts_val.len_u64()?,
        fingerprint_count_gb_val_ngrams: full_gb_val.len_u64()?,
        fingerprint_count_ts_train_ngrams: sampled_ts_train.len_u64()?,
        fingerprint_count_gb_train_ngrams: sampled_gb_train.len_u64()?,
        overlap_ts_train_to_gb_val: ts_train_to_gb_val
            .gated_fraction(ContaminationDirection::TsTrainContainsGbVal)?,
        overlap_gb_train_to_ts_val: gb_train_to_ts_val
            .gated_fraction(ContaminationDirection::GbTrainContainsTsVal)?,
        overlap_ts_train_contains_gb_train: DiagnosticOverlap::from_measurement(
            ts_train_contains_gb_train,
        ),
        overlap_gb_train_contains_ts_train: DiagnosticOverlap::from_measurement(
            gb_train_contains_ts_train,
        ),
        overlap_ts_val_to_gb_val: DiagnosticOverlap::from_measurement(ts_val_to_gb_val),
        overlap_gb_val_to_ts_val: DiagnosticOverlap::from_measurement(gb_val_to_ts_val),
        denominator_policy: ContaminationDenominatorPolicy::d6(),
        warnings,
        hard_failures,
        outcome,
        contamination_self_hash: Hash256::ZERO,
    };
    report.validate_structure()?;
    report.contamination_self_hash = report.compute_self_hash()?;
    emit_contamination_outcome(&report);
    Ok(report)
}

/// Verify H2 from a canonical contamination report.
pub fn verify_h2_contamination_report(report: &CrossCorpusReport) -> H2ContaminationVerdict {
    if report.validate_canonical_write().is_err() {
        return H2ContaminationVerdict {
            status: HypothesisStatus::Refuted,
            contamination_warning: false,
        };
    }

    match &report.outcome {
        ContaminationOutcome::Clean => H2ContaminationVerdict {
            status: HypothesisStatus::Confirmed,
            contamination_warning: false,
        },
        ContaminationOutcome::Warn { .. } => H2ContaminationVerdict {
            status: HypothesisStatus::Confirmed,
            contamination_warning: true,
        },
        ContaminationOutcome::HardFail { .. } => H2ContaminationVerdict {
            status: HypothesisStatus::Refuted,
            contamination_warning: false,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectionKind {
    Gated,
    Diagnostic,
}

impl DirectionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Gated => "gated",
            Self::Diagnostic => "diagnostic",
        }
    }

    const fn thresholds_apply(self) -> bool {
        matches!(self, Self::Gated)
    }
}

fn emit_contamination_started(inputs: &CrossCorpusInputs) {
    tracing::info!(
        target: S4_CONTAMINATION_LOG_TARGET,
        event_name = S4_CONTAMINATION_STARTED_EVENT,
        n = S4_CONTAMINATION_NGRAM_N as u64,
        fingerprint_kind = S4_CONTAMINATION_FINGERPRINT_KIND,
        collision_disambiguation = S4_CONTAMINATION_COLLISION_DISAMBIGUATION,
        diagnostic_sample_cap_token_ids_per_split =
            S4_CONTAMINATION_DIAGNOSTIC_SAMPLE_CAP_TOKEN_IDS_PER_SPLIT as u64,
        hard_fail_threshold = S4_CONTAMINATION_HARD_FAIL_THRESHOLD,
        warn_threshold = S4_CONTAMINATION_WARN_THRESHOLD,
        threshold_provenance = S4_CONTAMINATION_THRESHOLD_PROVENANCE,
        threshold_estimate_tag = S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG,
        ts_train_documents = inputs.ts_train.documents.len() as u64,
        ts_val_documents = inputs.ts_val.documents.len() as u64,
        gb_train_documents = inputs.gb_train.documents.len() as u64,
        gb_val_documents = inputs.gb_val.documents.len() as u64,
        "s4 contamination check started"
    );
}

fn emit_contamination_direction(
    direction: ContaminationDirection,
    kind: DirectionKind,
    denominator_policy: &'static str,
    measurement: OverlapMeasurement,
) {
    let overlap_status = DiagnosticOverlap::from_measurement(measurement).status();
    if let Some(fraction) = measurement.fraction {
        tracing::info!(
            target: S4_CONTAMINATION_LOG_TARGET,
            event_name = S4_CONTAMINATION_DIRECTION_EVENT,
            direction = direction.as_str(),
            direction_kind = kind.as_str(),
            denominator_policy = denominator_policy,
            overlap_status = overlap_status,
            overlap_count = measurement.overlap_count,
            denominator_count = measurement.denominator_count,
            overlap_fraction = fraction,
            thresholds_apply = kind.thresholds_apply(),
            exceeds_warn = kind.thresholds_apply()
                && fraction >= S4_CONTAMINATION_WARN_THRESHOLD,
            exceeds_hard_fail = kind.thresholds_apply()
                && fraction > S4_CONTAMINATION_HARD_FAIL_THRESHOLD,
            threshold_provenance = S4_CONTAMINATION_THRESHOLD_PROVENANCE,
            threshold_estimate_tag = S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG,
            "s4 contamination direction measured"
        );
    } else {
        tracing::info!(
            target: S4_CONTAMINATION_LOG_TARGET,
            event_name = S4_CONTAMINATION_DIRECTION_EVENT,
            direction = direction.as_str(),
            direction_kind = kind.as_str(),
            denominator_policy = denominator_policy,
            overlap_status = overlap_status,
            overlap_count = measurement.overlap_count,
            denominator_count = measurement.denominator_count,
            thresholds_apply = kind.thresholds_apply(),
            exceeds_warn = false,
            exceeds_hard_fail = false,
            threshold_provenance = S4_CONTAMINATION_THRESHOLD_PROVENANCE,
            threshold_estimate_tag = S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG,
            "s4 contamination direction unavailable"
        );
    }
}

fn emit_contamination_outcome(report: &CrossCorpusReport) {
    tracing::info!(
        target: S4_CONTAMINATION_LOG_TARGET,
        event_name = S4_CONTAMINATION_OUTCOME_EVENT,
        outcome = report.outcome.kind_str(),
        warning_count = report.warnings.len() as u64,
        hard_failure_count = report.hard_failures.len() as u64,
        contamination_self_hash = %report.contamination_self_hash,
        threshold_provenance = S4_CONTAMINATION_THRESHOLD_PROVENANCE,
        threshold_estimate_tag = S4_CONTAMINATION_THRESHOLD_ESTIMATE_TAG,
        "s4 contamination outcome emitted"
    );
}

/// Compute the D6 `sha256_high_u64` fingerprint index for one 13-token window.
#[must_use]
pub fn sha256_high_u64(window: &[u8; S4_CONTAMINATION_NGRAM_N]) -> u64 {
    let digest = sha256(window).to_bytes();
    u64::from_be_bytes(
        digest[0..8]
            .try_into()
            .expect("sha256 digest always has eight high bytes"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FingerprintSet {
    buckets: BTreeMap<u64, BTreeSet<TokenWindow>>,
    len: usize,
}

impl FingerprintSet {
    fn from_documents(documents: &[Vec<u8>], indexer: impl Fn(&TokenWindow) -> u64) -> Self {
        let mut buckets: BTreeMap<u64, BTreeSet<TokenWindow>> = BTreeMap::new();
        let mut len = 0_usize;

        for document in documents {
            let body_tokens = document_body_tokens(document);
            if body_tokens.len() < S4_CONTAMINATION_NGRAM_N {
                continue;
            }
            for window in body_tokens.windows(S4_CONTAMINATION_NGRAM_N) {
                let window: TokenWindow = window
                    .try_into()
                    .expect("slice::windows returns exact-width windows");
                let inserted = buckets.entry(indexer(&window)).or_default().insert(window);
                if inserted {
                    len += 1;
                }
            }
        }

        Self { buckets, len }
    }

    fn len_u64(&self) -> Result<u64, S4ContaminationError> {
        u64::try_from(self.len).map_err(|_| S4ContaminationError::CountOverflow)
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn overlap_against_denominator(
        &self,
        denominator: &Self,
    ) -> Result<OverlapMeasurement, S4ContaminationError> {
        let denominator_count = denominator.len_u64()?;
        if denominator_count == 0 {
            return Ok(OverlapMeasurement {
                overlap_count: 0,
                denominator_count: 0,
                fraction: None,
            });
        }

        let mut overlap_count = 0_u64;
        for (fingerprint, denominator_windows) in &denominator.buckets {
            let Some(containing_windows) = self.buckets.get(fingerprint) else {
                continue;
            };
            for window in denominator_windows {
                if containing_windows.contains(window) {
                    overlap_count = overlap_count
                        .checked_add(1)
                        .ok_or(S4ContaminationError::CountOverflow)?;
                }
            }
        }
        Ok(OverlapMeasurement {
            overlap_count,
            denominator_count,
            fraction: Some(overlap_count as f64 / denominator_count as f64),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OverlapMeasurement {
    overlap_count: u64,
    denominator_count: u64,
    fraction: Option<f64>,
}

impl OverlapMeasurement {
    fn gated_fraction(
        self,
        direction: ContaminationDirection,
    ) -> Result<f64, S4ContaminationError> {
        self.fraction
            .ok_or(S4ContaminationError::GatedDenominatorUnavailable { direction })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplitName {
    TsTrain,
    TsVal,
    GbTrain,
    GbVal,
}

impl SplitName {
    const fn as_str(self) -> &'static str {
        match self {
            Self::TsTrain => "ts_train",
            Self::TsVal => "ts_val",
            Self::GbTrain => "gb_train",
            Self::GbVal => "gb_val",
        }
    }
}

fn diagnostic_fingerprint_set(documents: &[Vec<u8>]) -> FingerprintSet {
    let sampled = diagnostic_sample_documents(
        documents,
        S4_CONTAMINATION_DIAGNOSTIC_SAMPLE_CAP_TOKEN_IDS_PER_SPLIT,
    );
    FingerprintSet::from_documents(&sampled, sha256_high_u64)
}

fn diagnostic_sample_documents(documents: &[Vec<u8>], cap_token_ids: usize) -> Vec<Vec<u8>> {
    if documents.is_empty() || cap_token_ids == 0 {
        return Vec::new();
    }

    let body_documents = documents
        .iter()
        .map(|document| document_body_tokens(document))
        .filter(|document| !document.is_empty())
        .collect::<Vec<_>>();
    if body_documents.is_empty() {
        return Vec::new();
    }

    let per_doc_cap = ceil_div(cap_token_ids, body_documents.len());
    let mut sampled = Vec::new();
    for document in &body_documents {
        if document.len() <= per_doc_cap {
            sampled.push(document.clone());
        } else {
            sampled.extend(sample_document_fragments(document, per_doc_cap));
        }
    }
    sampled
}

fn sample_document_fragments(document: &[u8], per_doc_cap: usize) -> Vec<Vec<u8>> {
    if per_doc_cap == 0 {
        return Vec::new();
    }
    if document.len() <= per_doc_cap {
        return vec![document.to_vec()];
    }

    let head_len = per_doc_cap / 3;
    let middle_len = per_doc_cap / 3;
    let tail_len = per_doc_cap - head_len - middle_len;
    let tail_start = document.len() - tail_len;

    let mut fragments = Vec::new();
    if head_len > 0 {
        fragments.push(document[..head_len].to_vec());
    }
    if middle_len > 0 {
        let middle_start =
            centered_fragment_start(document.len(), middle_len, head_len, tail_start);
        fragments.push(document[middle_start..middle_start + middle_len].to_vec());
    }
    if tail_len > 0 {
        fragments.push(document[tail_start..].to_vec());
    }
    fragments
}

fn document_body_tokens(document: &[u8]) -> Vec<u8> {
    document
        .iter()
        .copied()
        .filter(|id| *id != BOS_ID && *id != EOS_ID)
        .collect()
}

fn centered_fragment_start(
    document_len: usize,
    fragment_len: usize,
    min_start: usize,
    max_end: usize,
) -> usize {
    let ideal = document_len / 2 - fragment_len / 2;
    let max_start = max_end - fragment_len;
    ideal.clamp(min_start, max_start)
}

const fn ceil_div(numerator: usize, denominator: usize) -> usize {
    numerator / denominator
        + if numerator / denominator * denominator == numerator {
            0
        } else {
            1
        }
}

fn gated_findings(
    ts_train_to_gb_val: OverlapMeasurement,
    gb_train_to_ts_val: OverlapMeasurement,
) -> Result<(Vec<ContaminationFinding>, Vec<ContaminationFinding>), S4ContaminationError> {
    let mut warnings = Vec::new();
    let mut hard_failures = Vec::new();

    for (direction, measurement) in [
        (
            ContaminationDirection::TsTrainContainsGbVal,
            ts_train_to_gb_val,
        ),
        (
            ContaminationDirection::GbTrainContainsTsVal,
            gb_train_to_ts_val,
        ),
    ] {
        let fraction = measurement.gated_fraction(direction)?;
        if fraction >= S4_CONTAMINATION_WARN_THRESHOLD {
            warnings.push(ContaminationFinding {
                kind: ContaminationFindingKind::Warning,
                direction,
                overlap_fraction: fraction,
                threshold: S4_CONTAMINATION_WARN_THRESHOLD,
                overlap_count: measurement.overlap_count,
                denominator_count: measurement.denominator_count,
            });
        }
        if fraction > S4_CONTAMINATION_HARD_FAIL_THRESHOLD {
            hard_failures.push(ContaminationFinding {
                kind: ContaminationFindingKind::HardFailure,
                direction,
                overlap_fraction: fraction,
                threshold: S4_CONTAMINATION_HARD_FAIL_THRESHOLD,
                overlap_count: measurement.overlap_count,
                denominator_count: measurement.denominator_count,
            });
        }
    }

    Ok((warnings, hard_failures))
}

fn contamination_outcome(
    warnings: &[ContaminationFinding],
    hard_failures: &[ContaminationFinding],
) -> ContaminationOutcome {
    if !hard_failures.is_empty() {
        ContaminationOutcome::HardFail {
            failures: hard_failures
                .iter()
                .map(|finding| finding.direction)
                .collect(),
            warnings: warnings.iter().map(|finding| finding.direction).collect(),
        }
    } else if !warnings.is_empty() {
        ContaminationOutcome::Warn {
            findings: warnings.iter().map(|finding| finding.direction).collect(),
        }
    } else {
        ContaminationOutcome::Clean
    }
}

fn require_nonempty_gated_split(
    split: SplitName,
    set: &FingerprintSet,
) -> Result<(), S4ContaminationError> {
    if set.is_empty() {
        Err(S4ContaminationError::GatedSplitTooShort {
            split: split.as_str(),
            n: S4_CONTAMINATION_NGRAM_N,
        })
    } else {
        Ok(())
    }
}

fn validate_literal(
    field: &'static str,
    expected: &'static str,
    observed: &str,
) -> Result<(), S4ContaminationError> {
    if observed == expected {
        Ok(())
    } else {
        Err(S4ContaminationError::InvalidLiteral {
            field,
            expected,
            observed: observed.to_owned(),
        })
    }
}

fn validate_fraction(field: &'static str, value: f64) -> Result<(), S4ContaminationError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(S4ContaminationError::InvalidOverlapFraction { field, value })
    }
}

fn validate_finding(
    finding: &ContaminationFinding,
    expected_kind: ContaminationFindingKind,
) -> Result<(), S4ContaminationError> {
    if finding.kind != expected_kind {
        return Err(S4ContaminationError::FindingMismatch {
            direction: finding.direction,
        });
    }
    if !matches!(
        finding.direction,
        ContaminationDirection::TsTrainContainsGbVal | ContaminationDirection::GbTrainContainsTsVal
    ) {
        return Err(S4ContaminationError::DiagnosticFinding {
            direction: finding.direction,
        });
    }
    validate_fraction(
        "ContaminationFinding.overlap_fraction",
        finding.overlap_fraction,
    )?;
    if finding.denominator_count == 0 || finding.overlap_count > finding.denominator_count {
        return Err(S4ContaminationError::FindingCountMismatch {
            direction: finding.direction,
            overlap_count: finding.overlap_count,
            denominator_count: finding.denominator_count,
        });
    }
    match expected_kind {
        ContaminationFindingKind::Warning => {
            if finding.threshold != S4_CONTAMINATION_WARN_THRESHOLD
                || finding.overlap_fraction < S4_CONTAMINATION_WARN_THRESHOLD
            {
                return Err(S4ContaminationError::FindingMismatch {
                    direction: finding.direction,
                });
            }
        }
        ContaminationFindingKind::HardFailure => {
            if finding.threshold != S4_CONTAMINATION_HARD_FAIL_THRESHOLD
                || finding.overlap_fraction <= S4_CONTAMINATION_HARD_FAIL_THRESHOLD
            {
                return Err(S4ContaminationError::FindingMismatch {
                    direction: finding.direction,
                });
            }
        }
    }
    Ok(())
}

fn validate_outcome(
    outcome: &ContaminationOutcome,
    warnings: &[ContaminationFinding],
    hard_failures: &[ContaminationFinding],
) -> Result<(), S4ContaminationError> {
    let warning_directions = warnings
        .iter()
        .map(|finding| finding.direction)
        .collect::<Vec<_>>();
    let hard_failure_directions = hard_failures
        .iter()
        .map(|finding| finding.direction)
        .collect::<Vec<_>>();
    if hard_failure_directions
        .iter()
        .any(|direction| !warning_directions.contains(direction))
    {
        return Err(S4ContaminationError::OutcomeMismatch);
    }
    match outcome {
        ContaminationOutcome::Clean if warnings.is_empty() && hard_failures.is_empty() => Ok(()),
        ContaminationOutcome::Warn { findings }
            if hard_failures.is_empty()
                && !warnings.is_empty()
                && *findings == warning_directions =>
        {
            Ok(())
        }
        ContaminationOutcome::HardFail { failures, warnings }
            if !hard_failures.is_empty()
                && *failures == hard_failure_directions
                && *warnings == warning_directions =>
        {
            Ok(())
        }
        _ => Err(S4ContaminationError::OutcomeMismatch),
    }
}

/// Errors from S4 contamination report construction and validation.
#[derive(Debug)]
pub enum S4ContaminationError {
    /// Report schema did not match `s4_contamination_report.v1`.
    InvalidSchema {
        /// Observed schema id.
        observed: String,
    },
    /// Report literal field did not match D6.
    InvalidLiteral {
        /// Field name.
        field: &'static str,
        /// Expected literal.
        expected: &'static str,
        /// Observed literal.
        observed: String,
    },
    /// The report did not carry the pinned D6 threshold provenance.
    InvalidThresholdProvenance {
        /// Field name.
        field: &'static str,
    },
    /// The report used an n-gram width other than D6 n=13.
    InvalidN {
        /// Observed n value.
        observed: u64,
    },
    /// A gated direction had no denominator, violating D6 preconditions.
    GatedDenominatorUnavailable {
        /// Gated direction.
        direction: ContaminationDirection,
    },
    /// A closure-gated split did not contain any valid 13-gram.
    GatedSplitTooShort {
        /// Split field name.
        split: &'static str,
        /// Required n-gram width.
        n: usize,
    },
    /// A reported overlap fraction was outside [0, 1] or non-finite.
    InvalidOverlapFraction {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// A diagnostic-only direction appeared in a gated finding list.
    DiagnosticFinding {
        /// Invalid direction.
        direction: ContaminationDirection,
    },
    /// A finding kind, threshold, or crossed-threshold invariant did not match.
    FindingMismatch {
        /// Invalid direction.
        direction: ContaminationDirection,
    },
    /// A finding carried impossible overlap counts.
    FindingCountMismatch {
        /// Invalid direction.
        direction: ContaminationDirection,
        /// Overlap count.
        overlap_count: u64,
        /// Denominator count.
        denominator_count: u64,
    },
    /// The outcome field did not match warnings and hard failures.
    OutcomeMismatch,
    /// Count conversion overflowed u64.
    CountOverflow,
    /// Stored report self-hash differed from recomputation.
    SelfHashMismatch {
        /// Expected recomputed self-hash.
        expected: Hash256,
        /// Observed stored self-hash.
        observed: Hash256,
    },
    /// Self-hash computation expected a top-level object.
    ExpectedObjectForSelfHash,
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Canonical JSON serialization failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
}

impl fmt::Display for S4ContaminationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { observed } => {
                write!(
                    f,
                    "expected s4_contamination_report.v1 schema, got {observed:?}"
                )
            }
            Self::InvalidLiteral {
                field,
                expected,
                observed,
            } => write!(
                f,
                "expected {field} to be {expected:?} in s4_contamination_report.v1, got {observed:?}"
            ),
            Self::InvalidThresholdProvenance { field } => write!(
                f,
                "expected {field} to carry D6 [ESTIMATE] contamination threshold provenance"
            ),
            Self::InvalidN { observed } => {
                write!(f, "D6 contamination n must be 13, got {observed}")
            }
            Self::GatedDenominatorUnavailable { direction } => write!(
                f,
                "gated contamination direction {} had no denominator",
                direction.as_str()
            ),
            Self::GatedSplitTooShort { split, n } => {
                write!(
                    f,
                    "gated contamination split {split} has no {n}-token windows"
                )
            }
            Self::InvalidOverlapFraction { field, value } => {
                write!(f, "{field} must be finite and in [0, 1], got {value}")
            }
            Self::DiagnosticFinding { direction } => write!(
                f,
                "diagnostic contamination direction {} cannot be a gated finding",
                direction.as_str()
            ),
            Self::FindingMismatch { direction } => write!(
                f,
                "contamination finding for {} does not match its kind or threshold",
                direction.as_str()
            ),
            Self::FindingCountMismatch {
                direction,
                overlap_count,
                denominator_count,
            } => write!(
                f,
                "contamination finding for {} has invalid counts {overlap_count}/{denominator_count}",
                direction.as_str()
            ),
            Self::OutcomeMismatch => {
                f.write_str("contamination outcome does not match warnings/hard_failures")
            }
            Self::CountOverflow => f.write_str("contamination n-gram count overflowed u64"),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "s4_contamination_report.v1 self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("contamination self-hash requires a top-level object")
            }
            Self::Json(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4ContaminationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_high_u64_uses_first_eight_digest_bytes_big_endian() {
        let window = [42_u8; S4_CONTAMINATION_NGRAM_N];
        let digest = sha256(window).to_bytes();
        let expected = u64::from_be_bytes(digest[0..8].try_into().unwrap());

        assert_eq!(sha256_high_u64(&window), expected);
        assert_ne!(
            sha256_high_u64(&window),
            u64::from_le_bytes(digest[0..8].try_into().unwrap())
        );
        assert_ne!(
            sha256_high_u64(&window),
            u64::from_be_bytes(digest[24..32].try_into().unwrap())
        );
    }

    #[test]
    fn forced_index_collision_requires_exact_window_equality() {
        let left = vec![vec![1_u8; S4_CONTAMINATION_NGRAM_N]];
        let right = vec![vec![2_u8; S4_CONTAMINATION_NGRAM_N]];
        let left = FingerprintSet::from_documents(&left, |_| 7);
        let right = FingerprintSet::from_documents(&right, |_| 7);

        let overlap = left.overlap_against_denominator(&right).unwrap();

        assert_eq!(overlap.overlap_count, 0);
        assert_eq!(overlap.denominator_count, 1);
        assert_eq!(overlap.fraction, Some(0.0));
    }

    #[test]
    fn diagnostic_sampling_is_per_doc_head_middle_tail() {
        let mut doc = vec![42_u8; 100];
        doc[44..57].copy_from_slice(&[250_u8; S4_CONTAMINATION_NGRAM_N]);

        let sampled = diagnostic_sample_documents(&[doc], 39);

        assert_eq!(sampled.len(), 3);
        assert_eq!(sampled[0].len(), 13);
        assert_eq!(sampled[1], vec![250_u8; S4_CONTAMINATION_NGRAM_N]);
        assert_eq!(sampled[2].len(), 13);
    }

    #[test]
    fn empty_diagnostic_denominator_is_not_available_marker() {
        let measurement = OverlapMeasurement {
            overlap_count: 0,
            denominator_count: 0,
            fraction: None,
        };

        assert_eq!(
            DiagnosticOverlap::from_measurement(measurement),
            DiagnosticOverlap::DiagnosticNotAvailable
        );
        assert_eq!(
            serde_json::to_value(DiagnosticOverlap::from_measurement(measurement)).unwrap(),
            serde_json::json!("diagnostic_not_available")
        );
    }

    #[test]
    fn document_body_tokens_remove_bos_eos_but_keep_unk_like_bytes() {
        let body = document_body_tokens(&[BOS_ID, 1, 79, EOS_ID, 2]);

        assert_eq!(body, vec![1, 79, 2]);
    }
}
