//! Canonical `s4_report.v1` markdown emitter and S4-R-* validators.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::FromUtf8Error;

use gbf_foundation::{CanonicalJson, CanonicalJsonError, Hash256, sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::s4::schema::{
    HypothesisStatus, S4_CANONICAL_SEEDS, S4Completion, S4Decision, S4Hypothesis, S4Outcome,
};

const REPORT_HASH_DOMAIN_SEPARATOR: &[u8] = b"s4_report.v1/frontmatter+body\0";
const S4_REPORT_SCHEMA: &str = "s4_report.v1";
const REQUIRED_REPORT_SECTIONS: [&str; 7] = [
    "## Pre-registered predictions",
    "## Observed",
    "## Hypothesis verdicts",
    "## Falsification analysis",
    "## Surprises",
    "## Decision",
    "## Reproducibility statement",
];

/// Default checked-in S4 report artifact path.
pub const S4_REPORT_OUTPUT_PATH: &str = "docs/experiments/S4-report.md";

/// Event emitted when report emission starts.
pub const EVENT_NAME_EMISSION_STARTED: &str = "s4::report::emission_started";
/// Event emitted after an S4-R-* validator passes.
pub const EVENT_NAME_R_VALIDATOR_PASSED: &str = "s4::report::r_validator_passed";
/// Event emitted when report emission completes.
pub const EVENT_NAME_EMISSION_COMPLETE: &str = "s4::report::emission_complete";

/// Closure-packet labels required in the final S4 closure comment.
pub const S4_CLOSURE_PACKET_REQUIRED_ENTRIES: &[&str] = &[
    "predictions_section_hash",
    "gutenberg_manifest_self_hash",
    "baseline_gutenberg_self_hash",
    "promotion_gate_self_hash",
    "contamination_self_hash",
    "oracle_agreement_self_hash",
    "report_self_hash",
    "seed_0_score_self_hash",
    "seed_1_score_self_hash",
    "seed_2_score_self_hash",
    "seed_3_score_self_hash",
    "seed_4_score_self_hash",
    "H1",
    "H2",
    "H3",
    "H4",
    "H5",
    "H6",
    "H7",
    "F1-broken-S4",
    "F2-broken-S4",
    "F3-broken-S4",
    "F4-broken-S4",
    "F5-broken-S4",
    "F6-broken-S4",
    "determinism_transcript",
];

/// Exact markdown bytes emitted by `s4_report.v1`.
pub type MarkdownBytes = Vec<u8>;

/// A 40-character lowercase hexadecimal Git commit id recorded by `s4_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct S4GitCommitId(String);

impl S4GitCommitId {
    /// Create a checked Git commit id.
    pub fn new(value: impl Into<String>) -> Result<Self, S4ReportValueError> {
        let value = value.into();
        if value.len() == 40
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(S4ReportValueError::InvalidGitCommitId(value))
        }
    }

    /// Borrow the commit id string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for S4GitCommitId {
    type Error = S4ReportValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<S4GitCommitId> for String {
    fn from(value: S4GitCommitId) -> Self {
        value.0
    }
}

/// RFC revision reference recorded in `s4_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum S4RfcRevisionRef {
    /// Git commit id for the RFC revision.
    GitCommitId(S4GitCommitId),
    /// SHA-256 digest when the RFC is materialized as a content blob.
    Hash256(Hash256),
}

/// Checked S4 report scalar value errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S4ReportValueError {
    /// Git commit id was not a 40-character lowercase hexadecimal string.
    InvalidGitCommitId(String),
}

impl fmt::Display for S4ReportValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGitCommitId(value) => {
                write!(f, "invalid S4 Git commit id {value:?}")
            }
        }
    }
}

impl std::error::Error for S4ReportValueError {}

/// Per-seed closure artifacts recorded by `s4_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4PerSeedArtifacts {
    /// S4 seed.
    pub seed: u64,
    /// Gutenberg continuation completion state.
    pub completion: S4Completion,
    /// Gutenberg checkpoint self-hash.
    pub checkpoint_self_hash: Option<Hash256>,
    /// S4 run-log self-hash.
    pub run_log_self_hash: Option<Hash256>,
    /// S4 Gutenberg score self-hash.
    pub score_self_hash: Option<Hash256>,
    /// S4 oracle-agreement self-hash.
    pub oracle_agreement_self_hash: Option<Hash256>,
}

/// Canonical front matter for `s4_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4ReportFrontMatter {
    /// Pinned schema id.
    pub schema: String,
    /// Dispatched S4 outcome.
    pub s4_outcome: S4Outcome,
    /// Dispatched S4 decision.
    pub decision: S4Decision,
    /// TinyStories manifest self-hash.
    pub ts_manifest_self_hash: Option<Hash256>,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Option<Hash256>,
    /// Gutenberg baseline self-hash.
    pub baseline_gutenberg_self_hash: Option<Hash256>,
    /// Gutenberg corpus-quality self-hash.
    pub corpus_quality_self_hash: Option<Hash256>,
    /// Cross-corpus contamination self-hash.
    pub contamination_self_hash: Option<Hash256>,
    /// S3-to-S4 promotion-gate self-hash.
    pub promotion_gate_self_hash: Option<Hash256>,
    /// Corpus-progression replay self-hash.
    pub corpus_progression_self_hash: Option<Hash256>,
    /// Promoted TinyStories checkpoint self-hash.
    #[serde(rename = "c_TS_checkpoint_self_hash")]
    pub c_ts_checkpoint_self_hash: Option<Hash256>,
    /// Per-seed S4 artifacts.
    pub per_seed_artifacts: Vec<S4PerSeedArtifacts>,
    /// RFC3339 UTC generation time. Excluded from `report_self_hash`.
    pub generated_at: String,
    /// RFC revision used for the S4 report.
    pub rfc_revision: S4RfcRevisionRef,
    /// Hash of the body pre-registration section.
    pub predictions_section_hash: Hash256,
    /// Commit introducing the predictions.
    pub predictions_commit: S4GitCommitId,
    /// First commit introducing an S4 result artifact.
    pub first_result_commit: S4GitCommitId,
    /// Explicit verdict status for all seven hypotheses.
    pub hypothesis_statuses: std::collections::BTreeMap<S4Hypothesis, HypothesisStatus>,
    /// Report self-hash. This is the S4 closure pin.
    pub report_self_hash: Hash256,
}

/// A rendered `s4_report.v1` artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4Report {
    /// Canonical front matter with `report_self_hash` populated.
    pub front_matter: S4ReportFrontMatter,
    /// Exact markdown body bytes covered by `report_self_hash`.
    pub body: String,
}

impl S4Report {
    /// Construct a self-hashed report from front matter and markdown body.
    pub fn new(
        mut front_matter: S4ReportFrontMatter,
        body: impl Into<String>,
    ) -> Result<Self, S4ReportError> {
        let body = body.into();
        front_matter.report_self_hash = Hash256::ZERO;
        front_matter.report_self_hash = report_self_hash(&front_matter, &body)?;
        let report = Self { front_matter, body };
        validate_report(&report)?;
        Ok(report)
    }

    /// Render canonical-JSON front matter plus markdown body.
    pub fn to_markdown(&self) -> Result<MarkdownBytes, S4ReportError> {
        let front_matter = String::from_utf8(CanonicalJson::to_vec(&self.front_matter)?)?;
        Ok(format!("---\n{front_matter}\n---\n{}", self.body).into_bytes())
    }
}

/// Result of emitting an S4 report artifact to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedS4Report {
    /// Destination path that was written.
    pub path: PathBuf,
    /// Validated report payload.
    pub report: S4Report,
    /// Exact bytes written to disk.
    pub markdown: MarkdownBytes,
}

/// Individual S4-R-* report validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S4ReportValidator {
    /// S4-R-Predictions.
    Predictions,
    /// S4-R-AllSeeds.
    AllSeeds,
    /// S4-R-Self-Hash.
    SelfHash,
    /// S4-R-AllHypotheses.
    AllHypotheses,
    /// S4-R-Decision.
    Decision,
    /// S4-R-ClosureArtifacts.
    ClosureArtifacts,
}

impl S4ReportValidator {
    /// All S4 report validators implemented by this surface.
    #[must_use]
    pub const fn all() -> &'static [Self; 6] {
        &[
            Self::Predictions,
            Self::AllSeeds,
            Self::SelfHash,
            Self::AllHypotheses,
            Self::Decision,
            Self::ClosureArtifacts,
        ]
    }

    /// Stable validator name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Predictions => "S4-R-Predictions",
            Self::AllSeeds => "S4-R-AllSeeds",
            Self::SelfHash => "S4-R-Self-Hash",
            Self::AllHypotheses => "S4-R-AllHypotheses",
            Self::Decision => "S4-R-Decision",
            Self::ClosureArtifacts => "S4-R-ClosureArtifacts",
        }
    }
}

/// Errors from S4 report emission and validation.
#[derive(Debug)]
pub enum S4ReportError {
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// Front-matter conversion failed before canonicalization.
    FrontMatter(String),
    /// UTF-8 rendering failed unexpectedly.
    Utf8(FromUtf8Error),
    /// Filesystem write failed.
    Io(std::io::Error),
    /// One of the S4-R-* validators failed.
    Validation(S4ReportValidationError),
}

impl fmt::Display for S4ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::FrontMatter(error) => write!(f, "{error}"),
            Self::Utf8(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Validation(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S4ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CanonicalJson(error) => Some(error),
            Self::FrontMatter(_) => None,
            Self::Utf8(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

impl From<CanonicalJsonError> for S4ReportError {
    fn from(value: CanonicalJsonError) -> Self {
        Self::CanonicalJson(value)
    }
}

impl From<FromUtf8Error> for S4ReportError {
    fn from(value: FromUtf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<std::io::Error> for S4ReportError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<S4ReportValidationError> for S4ReportError {
    fn from(value: S4ReportValidationError) -> Self {
        Self::Validation(value)
    }
}

/// S4-R-* validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S4ReportValidationError {
    /// Front matter schema did not match the required S4 report schema id.
    InvalidSchema {
        /// Required schema id.
        expected: &'static str,
        /// Observed schema id.
        actual: String,
    },
    /// Required body heading was missing.
    MissingBodySection {
        /// Required heading.
        heading: &'static str,
    },
    /// Body contained a CR or CRLF line ending.
    InvalidLineEnding,
    /// Front-matter decision did not match the outcome.
    DecisionMismatch {
        /// S4 outcome.
        outcome: S4Outcome,
        /// Expected decision.
        expected: S4Decision,
        /// Actual decision.
        actual: S4Decision,
    },
    /// Missing per-seed report row.
    MissingSeed {
        /// Required seed.
        seed: u64,
    },
    /// Duplicate per-seed report row.
    DuplicateSeed {
        /// Duplicated seed.
        seed: u64,
    },
    /// Unexpected per-seed report row.
    UnexpectedSeed {
        /// Unexpected seed.
        seed: u64,
    },
    /// Observed markdown table omitted a seed.
    ObservedMissingSeed {
        /// Missing seed.
        seed: u64,
    },
    /// Closure decision missed a required artifact.
    MissingClosureArtifact {
        /// Seed whose row failed, when applicable.
        seed: Option<u64>,
        /// Missing field.
        field: &'static str,
    },
    /// Closure packet omitted a required named hash, verdict, or transcript.
    MissingClosurePacketEntry {
        /// Missing closure packet field.
        field: &'static str,
    },
    /// Closure packet carried an empty or placeholder value.
    EmptyClosurePacketEntry {
        /// Empty closure packet field.
        field: &'static str,
    },
    /// Report self-hash mismatch.
    SelfHashMismatch {
        /// Expected hash.
        expected: Hash256,
        /// Actual hash.
        actual: Hash256,
    },
    /// Predictions section hash mismatch.
    PredictionsSectionHashMismatch {
        /// Expected hash.
        expected: Hash256,
        /// Actual hash.
        actual: Hash256,
    },
    /// Predictions commit was equal to the first result commit.
    PredictionsCommitEqualsFirstResult {
        /// Predictions commit.
        predictions_commit: String,
        /// First result commit.
        first_result_commit: String,
    },
    /// Git history did not prove strict ancestry.
    PredictionsCommitNotStrictAncestor {
        /// Predictions commit.
        predictions_commit: String,
        /// First result commit.
        first_result_commit: String,
    },
    /// Hypothesis verdict missing.
    MissingHypothesis {
        /// Missing hypothesis.
        hypothesis: S4Hypothesis,
    },
    /// Closure decision carried a not-evaluated hypothesis.
    NotEvaluatedClosureHypothesis {
        /// Hypothesis.
        hypothesis: S4Hypothesis,
        /// Prior-gate reason.
        reason: String,
    },
    /// Closure decision carried a refuted hypothesis.
    RefutedClosureHypothesis {
        /// Hypothesis.
        hypothesis: S4Hypothesis,
    },
    /// Git command failed to run.
    GitCommandFailed {
        /// Operation name.
        operation: &'static str,
        /// Error message.
        message: String,
    },
}

impl fmt::Display for S4ReportValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { expected, actual } => write!(
                f,
                "s4_report.v1 schema validation failed: expected {expected}, got {actual}"
            ),
            Self::MissingBodySection { heading } => {
                write!(f, "missing required s4_report.v1 section {heading:?}")
            }
            Self::InvalidLineEnding => write!(
                f,
                "S4-R-Self-Hash failed: committed s4_report.v1 must use LF line endings"
            ),
            Self::DecisionMismatch {
                outcome,
                expected,
                actual,
            } => write!(
                f,
                "S4-R-Decision failed: {outcome:?} requires {expected:?}, got {actual:?}"
            ),
            Self::MissingSeed { seed } => {
                write!(f, "S4-R-AllSeeds failed: missing seed {seed}")
            }
            Self::DuplicateSeed { seed } => {
                write!(f, "S4-R-AllSeeds failed: duplicate seed {seed}")
            }
            Self::UnexpectedSeed { seed } => {
                write!(f, "S4-R-AllSeeds failed: unexpected seed {seed}")
            }
            Self::ObservedMissingSeed { seed } => {
                write!(
                    f,
                    "S4-R-AllSeeds failed: observed table missing seed {seed}"
                )
            }
            Self::MissingClosureArtifact { seed, field } => match seed {
                Some(seed) => write!(
                    f,
                    "S4-R-ClosureArtifacts failed: seed {seed} missing {field}"
                ),
                None => write!(f, "S4-R-ClosureArtifacts failed: missing {field}"),
            },
            Self::MissingClosurePacketEntry { field } => {
                write!(f, "S4 closure packet missing required entry {field}")
            }
            Self::EmptyClosurePacketEntry { field } => {
                write!(f, "S4 closure packet entry {field} is empty or placeholder")
            }
            Self::SelfHashMismatch { expected, actual } => write!(
                f,
                "S4-R-Self-Hash failed: expected report_self_hash {expected}, got {actual}"
            ),
            Self::PredictionsSectionHashMismatch { expected, actual } => write!(
                f,
                "S4-R-Predictions failed: expected predictions_section_hash {expected}, got {actual}"
            ),
            Self::PredictionsCommitEqualsFirstResult {
                predictions_commit,
                first_result_commit,
            } => write!(
                f,
                "S4-R-Predictions failed: predictions_commit {predictions_commit} equals first_result_commit {first_result_commit}"
            ),
            Self::PredictionsCommitNotStrictAncestor {
                predictions_commit,
                first_result_commit,
            } => write!(
                f,
                "S4-R-Predictions failed: {predictions_commit} is not a strict ancestor of {first_result_commit}"
            ),
            Self::MissingHypothesis { hypothesis } => {
                write!(f, "S4-R-AllHypotheses failed: missing {hypothesis:?}")
            }
            Self::NotEvaluatedClosureHypothesis { hypothesis, reason } => write!(
                f,
                "S4-R-AllHypotheses failed: {hypothesis:?} remained NotEvaluatedDueToPriorGate({reason})"
            ),
            Self::RefutedClosureHypothesis { hypothesis } => write!(
                f,
                "S4-R-AllHypotheses failed: closure report refuted {hypothesis:?}"
            ),
            Self::GitCommandFailed { operation, message } => {
                write!(f, "S4-R-Predictions git {operation} failed: {message}")
            }
        }
    }
}

impl std::error::Error for S4ReportValidationError {}

/// Return the RFC-pinned decision for an S4 outcome.
#[must_use]
pub fn decision_for_outcome(outcome: S4Outcome) -> S4Decision {
    match outcome {
        S4Outcome::PassClean => S4Decision::ProceedToS5,
        S4Outcome::PassWithContaminationWarning => S4Decision::ProceedToS5WithContaminationWarning,
        S4Outcome::FailCorpusIntegrity => S4Decision::Halt {
            reason: "corpus-integrity-broken".to_owned(),
        },
        S4Outcome::FailContamination => S4Decision::Halt {
            reason: "contamination-dirty".to_owned(),
        },
        S4Outcome::FailPromotionGate => S4Decision::Halt {
            reason: "promotion-gate-unsound".to_owned(),
        },
        S4Outcome::FailPromotionGateReadiness => S4Decision::Halt {
            reason: "promotion-gate-rejected-canonical".to_owned(),
        },
        S4Outcome::FailQualityOnGutenberg => S4Decision::Investigate {
            reason: "propose-step-budget-or-Toy1".to_owned(),
        },
        S4Outcome::FailOracleDisagreement => S4Decision::Halt {
            reason: "oracle-disagrees-on-gutenberg".to_owned(),
        },
        S4Outcome::FailSubstrate => S4Decision::Investigate {
            reason: "burn-or-corpus-loader".to_owned(),
        },
        S4Outcome::FailSuspicious => S4Decision::Halt {
            reason: "audit-split-and-bpc".to_owned(),
        },
    }
}

/// Emit a validated report as markdown bytes.
pub fn emit_report(report: &S4Report) -> Result<MarkdownBytes, S4ReportError> {
    emit_report_inner(report, "memory")
}

/// Emit a validated report to a path and return the written artifact.
pub fn emit_report_to_path(
    path: impl Into<PathBuf>,
    report: &S4Report,
) -> Result<EmittedS4Report, S4ReportError> {
    let path = path.into();
    let output_path = path.display().to_string();
    let markdown = emit_report_inner(report, &output_path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &markdown)?;
    Ok(EmittedS4Report {
        path,
        report: report.clone(),
        markdown,
    })
}

fn emit_report_inner(report: &S4Report, output_path: &str) -> Result<MarkdownBytes, S4ReportError> {
    tracing::info!(
        target: crate::S4_LOG_TARGET,
        event_name = EVENT_NAME_EMISSION_STARTED,
        s4_outcome = ?report.front_matter.s4_outcome,
        decision = ?report.front_matter.decision,
        "s4 report emission started"
    );
    validate_report_with_logging(report)?;
    let markdown = report.to_markdown()?;
    tracing::info!(
        target: crate::S4_LOG_TARGET,
        event_name = EVENT_NAME_EMISSION_COMPLETE,
        report_self_hash = %report.front_matter.report_self_hash,
        output_path,
        "s4 report emission complete"
    );
    Ok(markdown)
}

/// Validate every S4-R-* invariant and body-section requirement.
pub fn validate_report(report: &S4Report) -> Result<(), S4ReportError> {
    validate_report_schema(report)?;
    validate_required_body_sections(&report.body)?;
    for validator in S4ReportValidator::all() {
        validate_report_validator(report, *validator)?;
    }
    Ok(())
}

/// Validate that a bead closure packet names the final S4 hash/verdict matrix.
pub fn validate_s4_closure_packet(packet: &str) -> Result<(), S4ReportError> {
    for &field in S4_CLOSURE_PACKET_REQUIRED_ENTRIES {
        let value = closure_packet_entry_value(packet, field)
            .ok_or(S4ReportValidationError::MissingClosurePacketEntry { field })?;
        if closure_packet_value_is_placeholder(value) {
            return Err(S4ReportValidationError::EmptyClosurePacketEntry { field }.into());
        }
    }
    Ok(())
}

fn validate_report_with_logging(report: &S4Report) -> Result<(), S4ReportError> {
    validate_report_schema(report)?;
    validate_required_body_sections(&report.body)?;
    for validator in S4ReportValidator::all() {
        validate_report_validator(report, *validator)?;
        tracing::info!(
            target: crate::S4_LOG_TARGET,
            event_name = EVENT_NAME_R_VALIDATOR_PASSED,
            validator_name = validator.as_str(),
            "s4 report validator passed"
        );
    }
    Ok(())
}

fn validate_report_schema(report: &S4Report) -> Result<(), S4ReportError> {
    if report.front_matter.schema == S4_REPORT_SCHEMA {
        Ok(())
    } else {
        Err(S4ReportValidationError::InvalidSchema {
            expected: S4_REPORT_SCHEMA,
            actual: report.front_matter.schema.clone(),
        }
        .into())
    }
}

/// Validate one named S4-R-* invariant.
pub fn validate_report_validator(
    report: &S4Report,
    validator: S4ReportValidator,
) -> Result<(), S4ReportError> {
    match validator {
        S4ReportValidator::Predictions => validate_r_predictions(report),
        S4ReportValidator::AllSeeds => validate_r_all_seeds(report),
        S4ReportValidator::SelfHash => validate_r_self_hash(report),
        S4ReportValidator::AllHypotheses => validate_r_all_hypotheses(report),
        S4ReportValidator::Decision => validate_r_decision(report),
        S4ReportValidator::ClosureArtifacts => validate_r_closure_artifacts(report),
    }
}

/// Validate S4-R-Predictions.
pub fn validate_r_predictions(report: &S4Report) -> Result<(), S4ReportError> {
    let section = section_body(&report.body, "## Pre-registered predictions").ok_or(
        S4ReportValidationError::MissingBodySection {
            heading: "## Pre-registered predictions",
        },
    )?;
    let expected = predictions_section_hash(section)?;
    let actual = report.front_matter.predictions_section_hash;
    if expected != actual {
        return Err(
            S4ReportValidationError::PredictionsSectionHashMismatch { expected, actual }.into(),
        );
    }
    let predictions_commit = report.front_matter.predictions_commit.as_str();
    let first_result_commit = report.front_matter.first_result_commit.as_str();
    if predictions_commit == first_result_commit {
        return Err(
            S4ReportValidationError::PredictionsCommitEqualsFirstResult {
                predictions_commit: predictions_commit.to_owned(),
                first_result_commit: first_result_commit.to_owned(),
            }
            .into(),
        );
    }
    if !git_is_ancestor(predictions_commit, first_result_commit)? {
        return Err(
            S4ReportValidationError::PredictionsCommitNotStrictAncestor {
                predictions_commit: predictions_commit.to_owned(),
                first_result_commit: first_result_commit.to_owned(),
            }
            .into(),
        );
    }
    Ok(())
}

/// Validate S4-R-AllSeeds.
pub fn validate_r_all_seeds(report: &S4Report) -> Result<(), S4ReportError> {
    let mut observed = BTreeSet::new();
    for row in &report.front_matter.per_seed_artifacts {
        if !S4_CANONICAL_SEEDS.contains(&row.seed) {
            return Err(S4ReportValidationError::UnexpectedSeed { seed: row.seed }.into());
        }
        if !observed.insert(row.seed) {
            return Err(S4ReportValidationError::DuplicateSeed { seed: row.seed }.into());
        }
    }
    for seed in S4_CANONICAL_SEEDS {
        if !observed.contains(&seed) {
            return Err(S4ReportValidationError::MissingSeed { seed }.into());
        }
    }
    let observed_section = section_body(&report.body, "## Observed").ok_or(
        S4ReportValidationError::MissingBodySection {
            heading: "## Observed",
        },
    )?;
    for seed in S4_CANONICAL_SEEDS {
        if !observed_section.contains(&format!("| {seed} |")) {
            return Err(S4ReportValidationError::ObservedMissingSeed { seed }.into());
        }
    }
    Ok(())
}

/// Validate S4-R-Self-Hash.
pub fn validate_r_self_hash(report: &S4Report) -> Result<(), S4ReportError> {
    if report.body.as_bytes().contains(&b'\r') {
        return Err(S4ReportValidationError::InvalidLineEnding.into());
    }
    let actual = report.front_matter.report_self_hash;
    let mut front_matter = report.front_matter.clone();
    front_matter.report_self_hash = Hash256::ZERO;
    let expected = report_self_hash(&front_matter, &report.body)?;
    if expected == actual {
        Ok(())
    } else {
        Err(S4ReportValidationError::SelfHashMismatch { expected, actual }.into())
    }
}

/// Validate S4-R-AllHypotheses.
pub fn validate_r_all_hypotheses(report: &S4Report) -> Result<(), S4ReportError> {
    for hypothesis in S4Hypothesis::ALL {
        let status = report
            .front_matter
            .hypothesis_statuses
            .get(&hypothesis)
            .ok_or(S4ReportValidationError::MissingHypothesis { hypothesis })?;
        if is_closure_decision(&report.front_matter.decision) {
            match status {
                HypothesisStatus::Confirmed => {}
                HypothesisStatus::Refuted => {
                    return Err(
                        S4ReportValidationError::RefutedClosureHypothesis { hypothesis }.into(),
                    );
                }
                HypothesisStatus::NotEvaluatedDueToPriorGate { reason } => {
                    return Err(S4ReportValidationError::NotEvaluatedClosureHypothesis {
                        hypothesis,
                        reason: reason.clone(),
                    }
                    .into());
                }
            }
        }
    }
    Ok(())
}

/// Validate S4-R-Decision.
pub fn validate_r_decision(report: &S4Report) -> Result<(), S4ReportError> {
    let expected = decision_for_outcome(report.front_matter.s4_outcome);
    if report.front_matter.decision == expected {
        Ok(())
    } else {
        Err(S4ReportValidationError::DecisionMismatch {
            outcome: report.front_matter.s4_outcome,
            expected,
            actual: report.front_matter.decision.clone(),
        }
        .into())
    }
}

/// Validate S4-R-ClosureArtifacts.
pub fn validate_r_closure_artifacts(report: &S4Report) -> Result<(), S4ReportError> {
    if !is_closure_decision(&report.front_matter.decision) {
        return Ok(());
    }
    require_top_level_artifact(
        "ts_manifest_self_hash",
        report.front_matter.ts_manifest_self_hash,
    )?;
    require_top_level_artifact(
        "gutenberg_manifest_self_hash",
        report.front_matter.gutenberg_manifest_self_hash,
    )?;
    require_top_level_artifact(
        "baseline_gutenberg_self_hash",
        report.front_matter.baseline_gutenberg_self_hash,
    )?;
    require_top_level_artifact(
        "corpus_quality_self_hash",
        report.front_matter.corpus_quality_self_hash,
    )?;
    require_top_level_artifact(
        "contamination_self_hash",
        report.front_matter.contamination_self_hash,
    )?;
    require_top_level_artifact(
        "promotion_gate_self_hash",
        report.front_matter.promotion_gate_self_hash,
    )?;
    require_top_level_artifact(
        "corpus_progression_self_hash",
        report.front_matter.corpus_progression_self_hash,
    )?;
    require_top_level_artifact(
        "c_TS_checkpoint_self_hash",
        report.front_matter.c_ts_checkpoint_self_hash,
    )?;

    for row in &report.front_matter.per_seed_artifacts {
        require_completed(row.seed, &row.completion)?;
        require_row_artifact(row.seed, "checkpoint_self_hash", row.checkpoint_self_hash)?;
        require_row_artifact(row.seed, "run_log_self_hash", row.run_log_self_hash)?;
        require_row_artifact(row.seed, "score_self_hash", row.score_self_hash)?;
        if row.seed == 0 {
            require_row_artifact(
                row.seed,
                "oracle_agreement_self_hash",
                row.oracle_agreement_self_hash,
            )?;
        }
    }
    Ok(())
}

/// Hash a pre-registration section body.
pub fn predictions_section_hash(section: &str) -> Result<Hash256, S4ReportError> {
    Ok(sha256(CanonicalJson::value_to_vec(&Value::String(
        section.trim().to_owned(),
    ))?))
}

/// Compute the `s4_report.v1` self-hash from front matter plus body bytes.
pub fn report_self_hash(
    front_matter: &S4ReportFrontMatter,
    body: &str,
) -> Result<Hash256, S4ReportError> {
    let mut value = serde_json::to_value(front_matter)
        .map_err(|error| S4ReportError::FrontMatter(error.to_string()))?;
    let object = value.as_object_mut().ok_or_else(|| {
        S4ReportError::FrontMatter("s4_report front matter must be an object".to_owned())
    })?;
    object.remove("generated_at");
    object.remove("report_self_hash");
    let canonical = CanonicalJson::value_to_vec(&value)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&canonical);
    bytes.push(0);
    bytes.extend_from_slice(REPORT_HASH_DOMAIN_SEPARATOR);
    bytes.extend_from_slice(body.as_bytes());
    Ok(sha256(bytes))
}

fn validate_required_body_sections(body: &str) -> Result<(), S4ReportError> {
    for heading in REQUIRED_REPORT_SECTIONS {
        if !body.contains(heading) {
            return Err(S4ReportValidationError::MissingBodySection { heading }.into());
        }
    }
    Ok(())
}

fn closure_packet_entry_value<'a>(packet: &'a str, field: &str) -> Option<&'a str> {
    packet.lines().find_map(|line| {
        let trimmed = line.trim();
        let trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed).trim();
        let (key, value) = trimmed.split_once(':')?;
        (key.trim() == field).then_some(value.trim())
    })
}

fn closure_packet_value_is_placeholder(value: &str) -> bool {
    let normalized = value.trim().trim_matches('`').trim();
    if normalized.is_empty() {
        return true;
    }
    matches!(
        normalized.to_ascii_lowercase().as_str(),
        "-" | "todo" | "tbd" | "missing" | "none" | "null" | "n/a"
    )
}

fn section_body<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    let start = body
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((current, line))
        })
        .find_map(|(offset, line)| (line.trim_end() == heading).then_some(offset))?;
    let content_start = start + heading.len();
    let remainder = &body[content_start..];
    let next = remainder.find("\n## ").unwrap_or(remainder.len());
    Some(remainder[..next].trim())
}

fn git_is_ancestor(
    predictions_commit: &str,
    first_result_commit: &str,
) -> Result<bool, S4ReportError> {
    let status = Command::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            predictions_commit,
            first_result_commit,
        ])
        .status()
        .map_err(|error| S4ReportValidationError::GitCommandFailed {
            operation: "merge-base",
            message: error.to_string(),
        })?;
    Ok(status.success())
}

fn require_top_level_artifact(
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), S4ReportError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(S4ReportValidationError::MissingClosureArtifact { seed: None, field }.into())
    }
}

fn require_row_artifact(
    seed: u64,
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), S4ReportError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(S4ReportValidationError::MissingClosureArtifact {
            seed: Some(seed),
            field,
        }
        .into())
    }
}

fn require_completed(seed: u64, completion: &S4Completion) -> Result<(), S4ReportError> {
    if matches!(completion, S4Completion::Completed) {
        Ok(())
    } else {
        Err(S4ReportValidationError::MissingClosureArtifact {
            seed: Some(seed),
            field: "completion",
        }
        .into())
    }
}

fn is_closure_decision(decision: &S4Decision) -> bool {
    matches!(
        decision,
        S4Decision::ProceedToS5 | S4Decision::ProceedToS5WithContaminationWarning
    )
}

#[allow(dead_code)]
fn _assert_default_output_path_is_pathlike() -> &'static Path {
    Path::new(S4_REPORT_OUTPUT_PATH)
}
