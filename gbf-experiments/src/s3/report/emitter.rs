//! Canonical `s3_report.v1` markdown emitter and R-* validators.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::FromUtf8Error;

use gbf_foundation::{Hash256, SemVer, sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::dispatcher::{decision_for_outcome, report_log_target};
use crate::s1::schema::{GitCommitId, RfcRevisionRef, S1CanonicalJson, S1SchemaError};
use crate::s2::environment::S2EnvironmentHash;
use crate::s3::environment::S3EnvironmentHash;
use crate::s3::schema::{
    HypothesisStatus, OracleFallbackTag, S3Completion, S3Decision, S3Hypothesis, S3Outcome,
};

const REPORT_HASH_DOMAIN_SEPARATOR: &[u8] = b"s3_report.v1/frontmatter+body\0";
const S3_REPORT_SCHEMA: &str = "s3_report.v1";
const REQUIRED_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];
const REQUIRED_REPORT_SECTIONS: [&str; 7] = [
    "## Pre-registered predictions",
    "## Observed",
    "## Hypothesis verdicts",
    "## Falsification analysis",
    "## Surprises",
    "## Decision",
    "## Reproducibility statement",
];

/// Default checked-in S3 report artifact path.
pub const S3_REPORT_OUTPUT_PATH: &str = "docs/experiments/S3-report.md";

/// Event emitted when report emission starts.
pub const EVENT_NAME_EMISSION_STARTED: &str = "s3::report::emission_started";
/// Event emitted after an R-* validator passes.
pub const EVENT_NAME_R_VALIDATOR_PASSED: &str = "s3::report::r_validator_passed";
/// Event emitted when report emission completes.
pub const EVENT_NAME_EMISSION_COMPLETE: &str = "s3::report::emission_complete";

/// Exact markdown bytes emitted by `s3_report.v1`.
pub type MarkdownBytes = Vec<u8>;

/// Per-seed Phase A..D completion state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseCompletion {
    /// Phase A completion.
    #[serde(rename = "A")]
    pub a: S3Completion,
    /// Phase B completion.
    #[serde(rename = "B")]
    pub b: S3Completion,
    /// Phase C completion.
    #[serde(rename = "C")]
    pub c: S3Completion,
    /// Phase D completion.
    #[serde(rename = "D")]
    pub d: S3Completion,
}

impl PhaseCompletion {
    /// A fully completed four-phase row.
    #[must_use]
    pub fn completed() -> Self {
        Self {
            a: S3Completion::Completed,
            b: S3Completion::Completed,
            c: S3Completion::Completed,
            d: S3Completion::Completed,
        }
    }

    fn all_completed(&self) -> bool {
        [&self.a, &self.b, &self.c, &self.d]
            .iter()
            .all(|completion| matches!(completion, S3Completion::Completed))
    }
}

/// Per-seed closure artifacts recorded by `s3_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3PerSeedArtifacts {
    /// S3 seed.
    pub seed: u64,
    /// Teacher-side completion.
    pub teacher_completion: S3Completion,
    /// Student-side completion.
    pub student_completion: S3Completion,
    /// Per-phase completion.
    pub phase_completion: PhaseCompletion,
    /// Teacher checkpoint self-hash.
    pub teacher_checkpoint_self_hash: Option<Hash256>,
    /// Student checkpoint self-hash.
    pub student_checkpoint_self_hash: Option<Hash256>,
    /// Reference bundle self-hash.
    pub bundle_self_hash: Option<Hash256>,
    /// Student artifact self-hash.
    pub artifact_self_hash: Option<Hash256>,
    /// Agreement product self-hash.
    pub agreement_self_hash: Option<Hash256>,
    /// Generation log self-hash.
    pub generation_log_self_hash: Option<Hash256>,
}

/// Oracle owner beads recorded by `s3_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleOwnerBeads {
    /// Denotational oracle real-owner bead.
    pub denotational: String,
    /// Artifact oracle real-owner bead.
    pub artifact: String,
}

/// Serialized S2 environment hash record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2EnvironmentHashRecord {
    /// Hash of the active S2 build configuration.
    pub build_config_hash: Hash256,
    /// Hash of the Rust toolchain identity.
    pub rust_toolchain_hash: Hash256,
    /// Hash of the dependency lockfile bytes.
    pub dependency_lockfile_hash: Hash256,
}

impl From<S2EnvironmentHash> for S2EnvironmentHashRecord {
    fn from(value: S2EnvironmentHash) -> Self {
        Self {
            build_config_hash: value.build_config_hash,
            rust_toolchain_hash: value.rust_toolchain_hash,
            dependency_lockfile_hash: value.dependency_lockfile_hash,
        }
    }
}

/// Serialized S3 environment hash record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3EnvironmentHashRecord {
    /// Hash of the active S3 build configuration.
    pub build_config_hash: Hash256,
    /// Hash of the Rust toolchain identity.
    pub rust_toolchain_hash: Hash256,
    /// Hash of the dependency lockfile bytes.
    pub dependency_lockfile_hash: Hash256,
    /// Hash of the selected oracle backend identity.
    pub oracle_backend_identity: Hash256,
}

impl From<S3EnvironmentHash> for S3EnvironmentHashRecord {
    fn from(value: S3EnvironmentHash) -> Self {
        Self {
            build_config_hash: value.build_config_hash,
            rust_toolchain_hash: value.rust_toolchain_hash,
            dependency_lockfile_hash: value.dependency_lockfile_hash,
            oracle_backend_identity: value.oracle_backend_identity,
        }
    }
}

/// Canonical front matter for `s3_report.v1`.
#[allow(non_snake_case)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ReportFrontMatter {
    /// Pinned schema id.
    pub schema: String,
    /// Dispatched S3 outcome.
    pub s3_outcome: S3Outcome,
    /// Dispatched S3 decision.
    pub decision: S3Decision,
    /// Charset product self-hash.
    pub charset_self_hash: Hash256,
    /// Baseline product self-hash.
    pub baseline_self_hash: Hash256,
    /// Workload manifest self-hash.
    pub workload_self_hash: Hash256,
    /// Conformance product self-hash.
    pub conformance_self_hash: Hash256,
    /// v0_success product self-hash.
    pub v0_success_self_hash: Hash256,
    /// Per-seed artifacts.
    pub per_seed_artifacts: Vec<S3PerSeedArtifacts>,
    /// Oracle owner beads.
    pub oracle_owner_beads: OracleOwnerBeads,
    /// Named fallback oracle backends used during the run.
    pub oracle_fallback_used: Vec<OracleFallbackTag>,
    /// Oracle re-run self-hash.
    pub oracle_re_run_self_hash: Option<Hash256>,
    /// B19 conformance owner bead.
    pub conformance_owner_bead: String,
    /// Cross-crate E2E owner bead.
    pub e2e_test_owner_bead: String,
    /// Structured logging owner bead.
    pub structured_logging_owner_bead: String,
    /// Inherited S1 pass version.
    #[serde(rename = "pass_version_S1")]
    pub pass_version_s1: SemVer,
    /// Inherited S2 pass version.
    #[serde(rename = "pass_version_S2")]
    pub pass_version_s2: SemVer,
    /// S3 pass version.
    #[serde(rename = "pass_version_S3")]
    pub pass_version_s3: SemVer,
    /// Inherited S2 train config hash.
    pub s2_train_config_hash: Hash256,
    /// S3 train config hash.
    pub s3_train_config_hash: Hash256,
    /// Inherited S2 environment hash.
    pub s2_environment_hash: S2EnvironmentHashRecord,
    /// S3 environment hash.
    pub s3_environment_hash: S3EnvironmentHashRecord,
    /// Inherited S2 pinned phase schedule hash.
    pub s2_pinned_phase_schedule_hash: Hash256,
    /// Commit time of the first result commit; excluded from report self-hash.
    pub generated_at_commit_time: String,
    /// RFC revision.
    pub rfc_revision: RfcRevisionRef,
    /// Hash of the body pre-registration section.
    pub predictions_section_hash: Hash256,
    /// Commit introducing the predictions.
    pub predictions_commit: GitCommitId,
    /// First commit introducing an S3 result artifact.
    pub first_result_commit: GitCommitId,
    /// Explicit verdict status for all seven hypotheses.
    pub hypothesis_statuses: std::collections::BTreeMap<S3Hypothesis, HypothesisStatus>,
    /// Report self-hash.
    pub report_self_hash: Hash256,
}

/// A rendered `s3_report.v1` artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Report {
    /// Canonical front matter with `report_self_hash` populated.
    pub front_matter: S3ReportFrontMatter,
    /// Exact markdown body bytes covered by `report_self_hash`.
    pub body: String,
}

impl S3Report {
    /// Construct a self-hashed report from front matter and markdown body.
    pub fn new(
        mut front_matter: S3ReportFrontMatter,
        body: impl Into<String>,
    ) -> Result<Self, ReportError> {
        let body = body.into();
        front_matter.report_self_hash = Hash256::ZERO;
        front_matter.report_self_hash = report_self_hash(&front_matter, &body)?;
        let report = Self { front_matter, body };
        validate_report(&report)?;
        Ok(report)
    }

    /// Render canonical-JSON front matter plus markdown body.
    pub fn to_markdown(&self) -> Result<MarkdownBytes, ReportError> {
        let front_matter = String::from_utf8(S1CanonicalJson::to_vec(&self.front_matter)?)?;
        Ok(format!("---\n{front_matter}\n---\n{}", self.body).into_bytes())
    }
}

/// Result of emitting an S3 report artifact to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedS3Report {
    /// Destination path that was written.
    pub path: PathBuf,
    /// Validated report payload.
    pub report: S3Report,
    /// Exact bytes written to disk.
    pub markdown: MarkdownBytes,
}

/// Individual R-* report validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3ReportValidator {
    /// R-Predictions.
    Predictions,
    /// R-AllSeeds.
    AllSeeds,
    /// R-Self-Hash.
    SelfHash,
    /// R-AllHypotheses.
    AllHypotheses,
    /// R-OwnerBeads.
    OwnerBeads,
    /// R-Decision.
    Decision,
    /// R-ClosureArtifacts.
    ClosureArtifacts,
}

impl S3ReportValidator {
    fn all() -> &'static [Self; 7] {
        &[
            Self::Predictions,
            Self::AllSeeds,
            Self::SelfHash,
            Self::AllHypotheses,
            Self::OwnerBeads,
            Self::Decision,
            Self::ClosureArtifacts,
        ]
    }

    /// Stable validator name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Predictions => "R-Predictions",
            Self::AllSeeds => "R-AllSeeds",
            Self::SelfHash => "R-Self-Hash",
            Self::AllHypotheses => "R-AllHypotheses",
            Self::OwnerBeads => "R-OwnerBeads",
            Self::Decision => "R-Decision",
            Self::ClosureArtifacts => "R-ClosureArtifacts",
        }
    }
}

/// Errors from S3 report emission and validation.
#[derive(Debug)]
pub enum ReportError {
    /// Canonical schema serialization or hashing failed.
    Schema(S1SchemaError),
    /// UTF-8 rendering failed unexpectedly.
    Utf8(FromUtf8Error),
    /// Filesystem write failed.
    Io(std::io::Error),
    /// One of the R-* validators failed.
    Validation(ReportValidationError),
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "{error}"),
            Self::Utf8(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Validation(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::Utf8(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

impl From<S1SchemaError> for ReportError {
    fn from(value: S1SchemaError) -> Self {
        Self::Schema(value)
    }
}

impl From<FromUtf8Error> for ReportError {
    fn from(value: FromUtf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<std::io::Error> for ReportError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ReportValidationError> for ReportError {
    fn from(value: ReportValidationError) -> Self {
        Self::Validation(value)
    }
}

/// R-* validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportValidationError {
    /// Front matter schema did not match the required S3 report schema id.
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
    /// Front-matter decision did not match the outcome.
    DecisionMismatch {
        /// S3 outcome.
        outcome: S3Outcome,
        /// Expected decision.
        expected: S3Decision,
        /// Actual decision.
        actual: S3Decision,
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
        hypothesis: S3Hypothesis,
    },
    /// Closure decision carried a not-evaluated hypothesis.
    NotEvaluatedClosureHypothesis {
        /// Hypothesis.
        hypothesis: S3Hypothesis,
        /// Prior-gate reason.
        reason: String,
    },
    /// Closure decision carried a refuted hypothesis.
    RefutedClosureHypothesis {
        /// Hypothesis.
        hypothesis: S3Hypothesis,
    },
    /// Owner bead was empty.
    MissingOwnerBead {
        /// Field name.
        field: &'static str,
    },
    /// Closure gate was false.
    ClosureGateFailed {
        /// Field name.
        field: &'static str,
    },
    /// Git command failed to run.
    GitCommandFailed {
        /// Operation name.
        operation: &'static str,
        /// Error message.
        message: String,
    },
}

impl fmt::Display for ReportValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema { expected, actual } => {
                write!(
                    f,
                    "s3_report.v1 schema validation failed: expected {expected}, got {actual}"
                )
            }
            Self::MissingBodySection { heading } => {
                write!(f, "missing required s3_report.v1 section {heading:?}")
            }
            Self::DecisionMismatch {
                outcome,
                expected,
                actual,
            } => write!(
                f,
                "R-Decision failed: {outcome} requires {expected}, got {actual}"
            ),
            Self::MissingSeed { seed } => write!(f, "R-AllSeeds failed: missing seed {seed}"),
            Self::DuplicateSeed { seed } => write!(f, "R-AllSeeds failed: duplicate seed {seed}"),
            Self::UnexpectedSeed { seed } => {
                write!(f, "R-AllSeeds failed: unexpected seed {seed}")
            }
            Self::ObservedMissingSeed { seed } => {
                write!(f, "R-AllSeeds failed: observed table missing seed {seed}")
            }
            Self::MissingClosureArtifact { seed, field } => match seed {
                Some(seed) => write!(f, "R-ClosureArtifacts failed: seed {seed} missing {field}"),
                None => write!(f, "R-ClosureArtifacts failed: missing {field}"),
            },
            Self::SelfHashMismatch { expected, actual } => write!(
                f,
                "R-Self-Hash failed: expected report_self_hash {expected}, got {actual}"
            ),
            Self::PredictionsSectionHashMismatch { expected, actual } => write!(
                f,
                "R-Predictions failed: expected predictions_section_hash {expected}, got {actual}"
            ),
            Self::PredictionsCommitEqualsFirstResult {
                predictions_commit,
                first_result_commit,
            } => write!(
                f,
                "R-Predictions failed: predictions_commit {predictions_commit} equals first_result_commit {first_result_commit}"
            ),
            Self::PredictionsCommitNotStrictAncestor {
                predictions_commit,
                first_result_commit,
            } => write!(
                f,
                "R-Predictions failed: {predictions_commit} is not a strict ancestor of {first_result_commit}"
            ),
            Self::MissingHypothesis { hypothesis } => {
                write!(f, "R-AllHypotheses failed: missing {hypothesis}")
            }
            Self::NotEvaluatedClosureHypothesis { hypothesis, reason } => write!(
                f,
                "R-AllHypotheses failed: {hypothesis} remained NotEvaluatedDueToPriorGate({reason})"
            ),
            Self::RefutedClosureHypothesis { hypothesis } => {
                write!(
                    f,
                    "R-AllHypotheses failed: closure report refuted {hypothesis}"
                )
            }
            Self::MissingOwnerBead { field } => {
                write!(f, "R-OwnerBeads failed: {field} must not be empty")
            }
            Self::ClosureGateFailed { field } => {
                write!(
                    f,
                    "R-ClosureArtifacts failed: closure gate {field} was false"
                )
            }
            Self::GitCommandFailed { operation, message } => {
                write!(f, "R-Predictions git {operation} failed: {message}")
            }
        }
    }
}

impl std::error::Error for ReportValidationError {}

/// Emit a validated report as markdown bytes.
pub fn emit_report(report: &S3Report) -> Result<MarkdownBytes, ReportError> {
    emit_report_inner(report, "memory")
}

/// Emit a validated report to a path and return the written artifact.
pub fn emit_report_to_path(
    path: impl Into<PathBuf>,
    report: &S3Report,
) -> Result<EmittedS3Report, ReportError> {
    let path = path.into();
    let output_path = path.display().to_string();
    let markdown = emit_report_inner(report, &output_path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &markdown)?;
    Ok(EmittedS3Report {
        path,
        report: report.clone(),
        markdown,
    })
}

fn emit_report_inner(report: &S3Report, output_path: &str) -> Result<MarkdownBytes, ReportError> {
    tracing::info!(
        target: report_log_target(),
        event_name = EVENT_NAME_EMISSION_STARTED,
        s3_outcome = %report.front_matter.s3_outcome,
        decision = %report.front_matter.decision,
        "s3 report emission started"
    );
    validate_report_with_logging(report)?;
    let markdown = report.to_markdown()?;
    tracing::info!(
        target: report_log_target(),
        event_name = EVENT_NAME_EMISSION_COMPLETE,
        report_self_hash = %report.front_matter.report_self_hash,
        output_path,
        "s3 report emission complete"
    );
    Ok(markdown)
}

/// Validate every R-* invariant and body-section requirement.
pub fn validate_report(report: &S3Report) -> Result<(), ReportError> {
    validate_report_schema(report)?;
    validate_required_body_sections(&report.body)?;
    for validator in S3ReportValidator::all() {
        validate_report_validator(report, *validator)?;
    }
    Ok(())
}

fn validate_report_with_logging(report: &S3Report) -> Result<(), ReportError> {
    validate_report_schema(report)?;
    validate_required_body_sections(&report.body)?;
    for validator in S3ReportValidator::all() {
        validate_report_validator(report, *validator)?;
        tracing::info!(
            target: report_log_target(),
            event_name = EVENT_NAME_R_VALIDATOR_PASSED,
            validator_name = validator.as_str(),
            "s3 report validator passed"
        );
    }
    Ok(())
}

fn validate_report_schema(report: &S3Report) -> Result<(), ReportError> {
    if report.front_matter.schema == S3_REPORT_SCHEMA {
        Ok(())
    } else {
        Err(ReportValidationError::InvalidSchema {
            expected: S3_REPORT_SCHEMA,
            actual: report.front_matter.schema.clone(),
        }
        .into())
    }
}

/// Validate one named R-* invariant.
pub fn validate_report_validator(
    report: &S3Report,
    validator: S3ReportValidator,
) -> Result<(), ReportError> {
    match validator {
        S3ReportValidator::Predictions => validate_r_predictions(report),
        S3ReportValidator::AllSeeds => validate_r_all_seeds(report),
        S3ReportValidator::SelfHash => validate_r_self_hash(report),
        S3ReportValidator::AllHypotheses => validate_r_all_hypotheses(report),
        S3ReportValidator::OwnerBeads => validate_r_owner_beads(report),
        S3ReportValidator::Decision => validate_r_decision(report),
        S3ReportValidator::ClosureArtifacts => validate_r_closure_artifacts(report),
    }
}

/// Validate R-Predictions.
pub fn validate_r_predictions(report: &S3Report) -> Result<(), ReportError> {
    let section = section_body(&report.body, "## Pre-registered predictions").ok_or(
        ReportValidationError::MissingBodySection {
            heading: "## Pre-registered predictions",
        },
    )?;
    let expected = predictions_section_hash(section)?;
    let actual = report.front_matter.predictions_section_hash;
    if expected != actual {
        return Err(
            ReportValidationError::PredictionsSectionHashMismatch { expected, actual }.into(),
        );
    }
    let predictions_commit = report.front_matter.predictions_commit.as_str();
    let first_result_commit = report.front_matter.first_result_commit.as_str();
    if predictions_commit == first_result_commit {
        return Err(ReportValidationError::PredictionsCommitEqualsFirstResult {
            predictions_commit: predictions_commit.to_owned(),
            first_result_commit: first_result_commit.to_owned(),
        }
        .into());
    }
    if !git_is_ancestor(predictions_commit, first_result_commit)? {
        return Err(ReportValidationError::PredictionsCommitNotStrictAncestor {
            predictions_commit: predictions_commit.to_owned(),
            first_result_commit: first_result_commit.to_owned(),
        }
        .into());
    }
    Ok(())
}

/// Validate R-AllSeeds.
pub fn validate_r_all_seeds(report: &S3Report) -> Result<(), ReportError> {
    let mut observed = BTreeSet::new();
    for row in &report.front_matter.per_seed_artifacts {
        if !REQUIRED_SEEDS.contains(&row.seed) {
            return Err(ReportValidationError::UnexpectedSeed { seed: row.seed }.into());
        }
        if !observed.insert(row.seed) {
            return Err(ReportValidationError::DuplicateSeed { seed: row.seed }.into());
        }
    }
    for seed in REQUIRED_SEEDS {
        if !observed.contains(&seed) {
            return Err(ReportValidationError::MissingSeed { seed }.into());
        }
    }
    let observed_section = section_body(&report.body, "## Observed").ok_or(
        ReportValidationError::MissingBodySection {
            heading: "## Observed",
        },
    )?;
    for seed in REQUIRED_SEEDS {
        if !observed_section.contains(&format!("| {seed} |")) {
            return Err(ReportValidationError::ObservedMissingSeed { seed }.into());
        }
    }
    Ok(())
}

/// Validate R-Self-Hash.
pub fn validate_r_self_hash(report: &S3Report) -> Result<(), ReportError> {
    let actual = report.front_matter.report_self_hash;
    let mut front_matter = report.front_matter.clone();
    front_matter.report_self_hash = Hash256::ZERO;
    let expected = report_self_hash(&front_matter, &report.body)?;
    if expected == actual {
        Ok(())
    } else {
        Err(ReportValidationError::SelfHashMismatch { expected, actual }.into())
    }
}

/// Validate R-AllHypotheses.
pub fn validate_r_all_hypotheses(report: &S3Report) -> Result<(), ReportError> {
    for hypothesis in S3Hypothesis::ALL {
        let status = report
            .front_matter
            .hypothesis_statuses
            .get(&hypothesis)
            .ok_or(ReportValidationError::MissingHypothesis { hypothesis })?;
        if is_closure_decision(&report.front_matter.decision) {
            match status {
                HypothesisStatus::Confirmed => {}
                HypothesisStatus::Refuted => {
                    return Err(
                        ReportValidationError::RefutedClosureHypothesis { hypothesis }.into(),
                    );
                }
                HypothesisStatus::NotEvaluatedDueToPriorGate { reason } => {
                    return Err(ReportValidationError::NotEvaluatedClosureHypothesis {
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

/// Validate R-OwnerBeads.
pub fn validate_r_owner_beads(report: &S3Report) -> Result<(), ReportError> {
    require_nonempty_owner(
        "oracle_owner_beads.denotational",
        &report.front_matter.oracle_owner_beads.denotational,
    )?;
    require_nonempty_owner(
        "oracle_owner_beads.artifact",
        &report.front_matter.oracle_owner_beads.artifact,
    )?;
    require_nonempty_owner(
        "conformance_owner_bead",
        &report.front_matter.conformance_owner_bead,
    )?;
    require_nonempty_owner(
        "e2e_test_owner_bead",
        &report.front_matter.e2e_test_owner_bead,
    )?;
    require_nonempty_owner(
        "structured_logging_owner_bead",
        &report.front_matter.structured_logging_owner_bead,
    )
}

/// Validate R-Decision.
pub fn validate_r_decision(report: &S3Report) -> Result<(), ReportError> {
    let expected = decision_for_outcome(report.front_matter.s3_outcome);
    if report.front_matter.decision == expected {
        Ok(())
    } else {
        Err(ReportValidationError::DecisionMismatch {
            outcome: report.front_matter.s3_outcome,
            expected,
            actual: report.front_matter.decision.clone(),
        }
        .into())
    }
}

/// Validate R-ClosureArtifacts.
pub fn validate_r_closure_artifacts(report: &S3Report) -> Result<(), ReportError> {
    if !is_closure_decision(&report.front_matter.decision) {
        return Ok(());
    }
    require_top_level_artifact(
        "oracle_re_run_self_hash",
        report.front_matter.oracle_re_run_self_hash,
    )?;
    for row in &report.front_matter.per_seed_artifacts {
        require_completed(row.seed, "teacher_completion", &row.teacher_completion)?;
        require_completed(row.seed, "student_completion", &row.student_completion)?;
        if !row.phase_completion.all_completed() {
            return Err(ReportValidationError::MissingClosureArtifact {
                seed: Some(row.seed),
                field: "phase_completion",
            }
            .into());
        }
        require_row_artifact(
            row.seed,
            "teacher_checkpoint_self_hash",
            row.teacher_checkpoint_self_hash,
        )?;
        require_row_artifact(
            row.seed,
            "student_checkpoint_self_hash",
            row.student_checkpoint_self_hash,
        )?;
        require_row_artifact(row.seed, "bundle_self_hash", row.bundle_self_hash)?;
        require_row_artifact(row.seed, "artifact_self_hash", row.artifact_self_hash)?;
        require_row_artifact(row.seed, "agreement_self_hash", row.agreement_self_hash)?;
    }
    Ok(())
}

/// Hash a pre-registration section body.
pub fn predictions_section_hash(section: &str) -> Result<Hash256, ReportError> {
    Ok(sha256(S1CanonicalJson::value_to_vec(&Value::String(
        section.trim().to_owned(),
    ))?))
}

/// Read the RFC3339 commit timestamp used for `generated_at_commit_time`.
pub fn generated_at_commit_time(first_result_commit: &GitCommitId) -> Result<String, ReportError> {
    let output = Command::new("git")
        .env("TZ", "UTC")
        .args([
            "show",
            "-s",
            "--date=format-local:%Y-%m-%dT%H:%M:%SZ",
            "--format=%cd",
            first_result_commit.as_str(),
        ])
        .output()
        .map_err(|error| ReportValidationError::GitCommandFailed {
            operation: "show",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(ReportValidationError::GitCommandFailed {
            operation: "show",
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// Compute the report self-hash with `generated_at_commit_time` and
/// `report_self_hash` omitted from front matter.
pub fn report_self_hash(
    front_matter: &S3ReportFrontMatter,
    body: &str,
) -> Result<Hash256, ReportError> {
    let mut value = serde_json::to_value(front_matter)
        .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
    let object = value.as_object_mut().ok_or_else(|| {
        S1SchemaError::Custom("s3_report front matter must be an object".to_owned())
    })?;
    object.remove("generated_at_commit_time");
    object.remove("report_self_hash");
    let canonical = S1CanonicalJson::value_to_vec(&value)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&canonical);
    bytes.push(0);
    bytes.extend_from_slice(REPORT_HASH_DOMAIN_SEPARATOR);
    bytes.extend_from_slice(body.as_bytes());
    Ok(sha256(bytes))
}

fn validate_required_body_sections(body: &str) -> Result<(), ReportError> {
    for heading in REQUIRED_REPORT_SECTIONS {
        if !body.contains(heading) {
            return Err(ReportValidationError::MissingBodySection { heading }.into());
        }
    }
    Ok(())
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
) -> Result<bool, ReportError> {
    let status = Command::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            predictions_commit,
            first_result_commit,
        ])
        .status()
        .map_err(|error| ReportValidationError::GitCommandFailed {
            operation: "merge-base",
            message: error.to_string(),
        })?;
    Ok(status.success())
}

fn require_nonempty_owner(field: &'static str, value: &str) -> Result<(), ReportError> {
    if value.trim().is_empty() {
        Err(ReportValidationError::MissingOwnerBead { field }.into())
    } else {
        Ok(())
    }
}

fn require_top_level_artifact(
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), ReportError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(ReportValidationError::MissingClosureArtifact { seed: None, field }.into())
    }
}

fn require_row_artifact(
    seed: u64,
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), ReportError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(ReportValidationError::MissingClosureArtifact {
            seed: Some(seed),
            field,
        }
        .into())
    }
}

fn require_completed(
    seed: u64,
    field: &'static str,
    completion: &S3Completion,
) -> Result<(), ReportError> {
    if matches!(completion, S3Completion::Completed) {
        Ok(())
    } else {
        Err(ReportValidationError::MissingClosureArtifact {
            seed: Some(seed),
            field,
        }
        .into())
    }
}

fn is_closure_decision(decision: &S3Decision) -> bool {
    matches!(
        decision,
        S3Decision::ProceedToS4 | S3Decision::ProceedToS4WithDeferredClause
    )
}

#[allow(dead_code)]
fn _assert_default_output_path_is_pathlike() -> &'static Path {
    Path::new(S3_REPORT_OUTPUT_PATH)
}
