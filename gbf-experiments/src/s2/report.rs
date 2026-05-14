//! Deterministic S2 report hashing and R-S2 validators.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use gbf_foundation::{Hash256, SemVer, sha256};
use serde_json::{Value, json};

use crate::S2_LOG_TARGET;
use crate::s1::schema::{GitCommitId, RfcRevisionRef, S1CanonicalJson, S1SchemaError};
use crate::s2::schema::{
    HypothesisStatus, S2BuildKind, S2Completion, S2Decision, S2Hypothesis, S2Outcome,
    S2PerSeedArtifacts, S2ReportFrontMatter, S2VerifierBundle,
};

const REPORT_HASH_DOMAIN_SEPARATOR: &[u8] = b"s2_report.v1/frontmatter+body\0";
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
const REQUIRED_REPORT_BUILDS: [S2BuildKind; 3] = [
    S2BuildKind::s2_ternary_full,
    S2BuildKind::s2_fp_full,
    S2BuildKind::s2_ternary_nodistill,
];
const REQUIRED_HYPOTHESES: [S2Hypothesis; 6] = [
    S2Hypothesis::H1,
    S2Hypothesis::H2,
    S2Hypothesis::H3,
    S2Hypothesis::H4,
    S2Hypothesis::H5,
    S2Hypothesis::H6,
];

/// Default checked-in S2 report artifact path.
pub const S2_REPORT_OUTPUT_PATH: &str = "docs/experiments/S2-report.md";

/// A rendered `s2_report.v1` artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S2ReportFile {
    /// Canonical front matter with `report_self_hash` populated.
    pub front_matter: S2ReportFrontMatter,
    /// Exact markdown body bytes covered by `report_self_hash`.
    pub body: String,
}

impl S2ReportFile {
    /// Construct a self-hashed report from front matter and markdown body.
    pub fn new(
        mut front_matter: S2ReportFrontMatter,
        body: impl Into<String>,
    ) -> Result<Self, S2ReportError> {
        let body = body.into();
        front_matter.report_self_hash = Hash256::ZERO;
        front_matter.report_self_hash = report_self_hash(&front_matter, &body)?;
        let report = Self { front_matter, body };
        validate_report(&report)?;
        Ok(report)
    }

    /// Render canonical-JSON front matter plus the markdown body.
    ///
    /// The RFC names YAML front matter; this deterministic emitter renders the
    /// already-parsed front matter as canonical JSON between markdown
    /// delimiters. `report_self_hash_from_front_matter_value` validates the
    /// key-order-independent parsed-object contract used by YAML readers.
    pub fn to_markdown(&self) -> Result<String, S2ReportError> {
        let front_matter = String::from_utf8(S1CanonicalJson::to_vec(&self.front_matter)?)?;
        Ok(format!("---\n{front_matter}\n---\n{}", self.body))
    }
}

/// Inputs needed to emit a final `s2_report.v1` markdown artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S2ReportInputs {
    /// Destination path for the rendered report.
    pub output_path: PathBuf,
    /// S1 baseline self-hash carried forward for S2 closure.
    pub baseline_self_hash_carried_from_s1: Hash256,
    /// Oracle re-run report self-hash.
    pub oracle_re_run_self_hash: Hash256,
    /// Public QAT API snapshot hash.
    pub qat_public_api_snapshot_hash: Hash256,
    /// Public LinearState API snapshot hash.
    pub linearstate_public_api_snapshot_hash: Hash256,
    /// Per-seed S2 artifact references.
    pub per_seed_artifacts: Vec<S2PerSeedArtifacts>,
    /// S2 ablation report self-hash, when produced.
    pub ablation_self_hash: Option<Hash256>,
    /// Loss gradient-flow verifier self-hash.
    pub loss_grad_flow_self_hash: Hash256,
    /// LinearState smoke verifier self-hash.
    pub linearstate_smoke_self_hash: Hash256,
    /// Phase-transition integration verifier self-hash.
    pub phase_transition_integ_self_hash: Hash256,
    /// S2 falsification suite hash.
    pub falsification_s2_suite_hash: Hash256,
    /// RFC3339 UTC generation time.
    pub generated_at: String,
    /// RFC revision used for the report.
    pub rfc_revision: RfcRevisionRef,
    /// Commit introducing the predictions section.
    pub predictions_commit: GitCommitId,
    /// First commit that introduced any S2 result artifact.
    pub first_result_commit: GitCommitId,
    /// S2 pass implementation version.
    pub pass_version_s2: SemVer,
    /// Verifier statuses and early-gate inputs used for outcome dispatch.
    pub verifier_bundle: S2VerifierBundle,
    /// Pre-registered prediction markdown body.
    pub predictions_markdown: String,
    /// Human-readable observed-results markdown.
    pub observed_markdown: String,
    /// Falsification analysis markdown.
    pub falsification_analysis: String,
    /// Surprise notes markdown.
    pub surprises: String,
    /// Decision justification markdown.
    pub decision_justification: String,
    /// Replay/reproduction command.
    pub replay_command: String,
    /// Manifest/hash references used by the reproducibility statement.
    pub manifest_references: String,
}

/// Result of emitting an S2 report artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedS2Report {
    /// Destination path that was written.
    pub path: PathBuf,
    /// Validated report payload.
    pub report: S2ReportFile,
    /// Exact bytes written to disk.
    pub markdown: String,
}

/// Individual R-S2 report validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S2ReportValidator {
    /// R-S2-Decision.
    Decision,
    /// R-S2-AllSeeds.
    AllSeeds,
    /// R-S2-ClosureArtifacts.
    ClosureArtifacts,
    /// R-S2-Self-Hash.
    SelfHash,
    /// R-S2-Predictions.
    Predictions,
    /// R-S2-AllHypotheses.
    AllHypotheses,
}

impl S2ReportValidator {
    fn all() -> &'static [Self; 6] {
        &[
            Self::Decision,
            Self::AllSeeds,
            Self::ClosureArtifacts,
            Self::SelfHash,
            Self::Predictions,
            Self::AllHypotheses,
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "R-S2-Decision",
            Self::AllSeeds => "R-S2-AllSeeds",
            Self::ClosureArtifacts => "R-S2-ClosureArtifacts",
            Self::SelfHash => "R-S2-Self-Hash",
            Self::Predictions => "R-S2-Predictions",
            Self::AllHypotheses => "R-S2-AllHypotheses",
        }
    }
}

/// Errors from S2 report emission and validation.
#[derive(Debug)]
pub enum S2ReportError {
    /// Canonical schema serialization or hashing failed.
    Schema(S1SchemaError),
    /// UTF-8 rendering failed unexpectedly.
    Utf8(FromUtf8Error),
    /// Filesystem write failed.
    Io(std::io::Error),
    /// One of the R-S2 validators failed.
    Validation(S2ReportValidationError),
}

impl fmt::Display for S2ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "{error}"),
            Self::Utf8(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Validation(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S2ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::Utf8(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

impl From<S1SchemaError> for S2ReportError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<FromUtf8Error> for S2ReportError {
    fn from(error: FromUtf8Error) -> Self {
        Self::Utf8(error)
    }
}

impl From<std::io::Error> for S2ReportError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<S2ReportValidationError> for S2ReportError {
    fn from(error: S2ReportValidationError) -> Self {
        Self::Validation(error)
    }
}

/// A typed R-S2 validator failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S2ReportValidationError {
    /// A required report body section was absent.
    MissingBodySection {
        /// Missing markdown heading.
        heading: &'static str,
    },
    /// The front-matter decision does not match the selected outcome.
    DecisionMismatch {
        /// Selected outcome.
        outcome: S2Outcome,
        /// Expected decision.
        expected: S2Decision,
        /// Actual decision.
        actual: S2Decision,
    },
    /// A seed/build row appeared more than once.
    DuplicateSeedBuild {
        /// Runtime build kind.
        build_kind: S2BuildKind,
        /// Seed id.
        seed: u64,
    },
    /// A required seed/build row was absent.
    MissingSeedBuild {
        /// Runtime build kind.
        build_kind: S2BuildKind,
        /// Seed id.
        seed: u64,
    },
    /// A per-seed row used a build kind outside the report matrix.
    UnexpectedBuildKind {
        /// Runtime build kind.
        build_kind: S2BuildKind,
    },
    /// A closure-candidate artifact hash was absent.
    MissingClosureArtifact {
        /// Seed id, when the field is per-seed.
        seed: Option<u64>,
        /// Build kind, when the field is per-seed.
        build_kind: Option<S2BuildKind>,
        /// Missing field.
        field: &'static str,
    },
    /// A closure-candidate boolean gate was false.
    ClosureGateFailed {
        /// Failed gate field.
        field: &'static str,
    },
    /// Recomputed report self-hash did not match front matter.
    SelfHashMismatch {
        /// Expected recomputed hash.
        expected: Hash256,
        /// Actual front-matter hash.
        actual: Hash256,
    },
    /// The predictions section hash does not match the body section.
    PredictionsSectionHashMismatch {
        /// Expected hash from the report body.
        expected: Hash256,
        /// Actual front-matter hash.
        actual: Hash256,
    },
    /// The fixture-level commit ids are equal, so no later ancestry check could be strict.
    PredictionsCommitEqualsFirstResult {
        /// Predictions commit.
        predictions_commit: String,
        /// First result commit.
        first_result_commit: String,
    },
    /// A required hypothesis was absent.
    MissingHypothesis {
        /// Hypothesis id.
        hypothesis: S2Hypothesis,
    },
    /// A closure-candidate report tried to proceed with a skipped hypothesis.
    NotEvaluatedClosureHypothesis {
        /// Hypothesis id.
        hypothesis: S2Hypothesis,
        /// Prior-gate reason.
        reason: String,
    },
    /// A closure-candidate report tried to proceed with a refuted hypothesis.
    RefutedClosureHypothesis {
        /// Hypothesis id.
        hypothesis: S2Hypothesis,
    },
}

impl fmt::Display for S2ReportValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBodySection { heading } => {
                write!(f, "s2_report.v1 body is missing required section {heading}")
            }
            Self::DecisionMismatch {
                outcome,
                expected,
                actual,
            } => write!(
                f,
                "R-S2-Decision failed for {outcome}: expected {expected}, got {actual}"
            ),
            Self::DuplicateSeedBuild { build_kind, seed } => write!(
                f,
                "R-S2-AllSeeds failed: duplicate {build_kind} seed {seed}"
            ),
            Self::MissingSeedBuild { build_kind, seed } => {
                write!(f, "R-S2-AllSeeds failed: missing {build_kind} seed {seed}")
            }
            Self::UnexpectedBuildKind { build_kind } => write!(
                f,
                "R-S2-AllSeeds failed: unexpected report build kind {build_kind}"
            ),
            Self::MissingClosureArtifact {
                seed,
                build_kind,
                field,
            } => write!(
                f,
                "R-S2-ClosureArtifacts failed: seed={seed:?} build_kind={build_kind:?} missing {field}"
            ),
            Self::ClosureGateFailed { field } => {
                write!(f, "R-S2-ClosureArtifacts failed: {field} is false")
            }
            Self::SelfHashMismatch { expected, actual } => {
                write!(
                    f,
                    "R-S2-Self-Hash failed: expected {expected}, got {actual}"
                )
            }
            Self::PredictionsSectionHashMismatch { expected, actual } => write!(
                f,
                "R-S2-Predictions failed: expected predictions_section_hash {expected}, got {actual}"
            ),
            Self::PredictionsCommitEqualsFirstResult {
                predictions_commit,
                first_result_commit,
            } => write!(
                f,
                "R-S2-Predictions failed: predictions_commit {predictions_commit} equals first_result_commit {first_result_commit}; strict ancestry is checked by scripts/s2_preregistration_check.sh"
            ),
            Self::MissingHypothesis { hypothesis } => {
                write!(f, "R-S2-AllHypotheses failed: missing {hypothesis}")
            }
            Self::NotEvaluatedClosureHypothesis { hypothesis, reason } => write!(
                f,
                "R-S2-AllHypotheses failed: closure-candidate {hypothesis} is NotEvaluatedDueToPriorGate({reason})"
            ),
            Self::RefutedClosureHypothesis { hypothesis } => write!(
                f,
                "R-S2-AllHypotheses failed: closure-candidate {hypothesis} is Refuted"
            ),
        }
    }
}

impl std::error::Error for S2ReportValidationError {}

/// Validate a rendered S2 report against every R-S2 validator.
pub fn validate_report(report: &S2ReportFile) -> Result<(), S2ReportError> {
    validate_required_body_sections(&report.body)?;
    for validator in S2ReportValidator::all() {
        validate_report_validator(report, *validator)?;
    }
    Ok(())
}

/// Run one R-S2 validator against a rendered S2 report.
pub fn validate_report_validator(
    report: &S2ReportFile,
    validator: S2ReportValidator,
) -> Result<(), S2ReportError> {
    let result = match validator {
        S2ReportValidator::Decision => validate_decision(&report.front_matter),
        S2ReportValidator::AllSeeds => validate_all_seeds(&report.front_matter),
        S2ReportValidator::ClosureArtifacts => validate_closure_artifacts(&report.front_matter),
        S2ReportValidator::SelfHash => validate_self_hash(report),
        S2ReportValidator::Predictions => validate_predictions(&report.front_matter, &report.body),
        S2ReportValidator::AllHypotheses => validate_all_hypotheses(&report.front_matter),
    };
    let passed = result.is_ok();
    let diagnostic = result.as_ref().err().map_or_else(
        || "null".to_owned(),
        |error| {
            json!({
                "validator": validator.as_str(),
                "error": error.to_string(),
            })
            .to_string()
        },
    );
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "r_s2_validator_run",
        event = "r_s2_validator_run",
        id = validator.as_str(),
        passed,
        remediation = if passed { None } else { Some(diagnostic.as_str()) },
        diagnostic = diagnostic.as_str(),
        "s2 report validator run"
    );
    if let Err(error) = &result {
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "r_s2_validator_failed",
            event = "r_s2_validator_failed",
            id = validator.as_str(),
            diagnostic = %json!({
                "validator": validator.as_str(),
                "error": error.to_string(),
            }),
            "s2 report validator failed"
        );
    }
    result.map_err(Into::into)
}

/// Compute the `s2_report.v1` self-hash from parsed front matter plus body bytes.
pub fn report_self_hash(
    front_matter: &S2ReportFrontMatter,
    body: &str,
) -> Result<Hash256, S1SchemaError> {
    hash_report_preimage(&front_matter.canonical_json_bytes()?, body)
}

/// Compute the report hash from a parsed front-matter object.
///
/// This mirrors the RFC rule that YAML key order is non-normative: callers pass
/// the parsed object, then this helper omits `generated_at` and
/// `report_self_hash` before canonical JSON serialization.
pub fn report_self_hash_from_front_matter_value(
    front_matter: &Value,
    body: &str,
) -> Result<Hash256, S1SchemaError> {
    let mut stripped = front_matter.clone();
    let object = stripped
        .as_object_mut()
        .ok_or(S1SchemaError::ExpectedObjectForSelfHash)?;
    object.remove("generated_at");
    object.remove("report_self_hash");
    hash_report_preimage(&S1CanonicalJson::value_to_vec(&stripped)?, body)
}

/// Compute `predictions_section_hash` for the exact pre-registered section.
pub fn predictions_section_hash(markdown: &str) -> Result<Hash256, S1SchemaError> {
    Ok(sha256(S1CanonicalJson::to_vec(&markdown.trim())?))
}

/// Emit a validated, self-hashed `s2_report.v1` markdown artifact.
pub fn emit_s2_report(inputs: &S2ReportInputs) -> Result<EmittedS2Report, S2ReportError> {
    let outcome = dispatch_outcome(&inputs.verifier_bundle);
    let decision = decision_for_outcome(outcome);
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "report_emit_start",
        event = "report_emit_start",
        outcome = %outcome,
        decision = %decision,
        per_seed_artifact_count = inputs.per_seed_artifacts.len() as u32,
        "s2 report emit start"
    );

    let mut per_seed_artifacts = inputs.per_seed_artifacts.clone();
    per_seed_artifacts.sort_by_key(|row| (row.build_kind, row.seed));
    let body = render_s2_report_body(inputs, outcome, &decision);
    let front_matter = S2ReportFrontMatter {
        schema: "s2_report.v1".to_owned(),
        s2_outcome: outcome,
        decision,
        baseline_self_hash_carried_from_s1: inputs.baseline_self_hash_carried_from_s1,
        oracle_re_run_passed: inputs.verifier_bundle.oracle_re_run_passed,
        oracle_re_run_self_hash: inputs.oracle_re_run_self_hash,
        api_drift_check_passed: inputs.verifier_bundle.api_drift_check_passed,
        qat_public_api_snapshot_hash: inputs.qat_public_api_snapshot_hash,
        linearstate_public_api_snapshot_hash: inputs.linearstate_public_api_snapshot_hash,
        per_seed_artifacts,
        ablation_self_hash: inputs.ablation_self_hash,
        loss_grad_flow_self_hash: inputs.loss_grad_flow_self_hash,
        linearstate_smoke_self_hash: inputs.linearstate_smoke_self_hash,
        phase_transition_integ_self_hash: inputs.phase_transition_integ_self_hash,
        phase_transition_integ_passed: inputs.verifier_bundle.phase_transition_integ_passed,
        falsification_s2_passed: inputs.verifier_bundle.falsification_s2_passed,
        falsification_s2_suite_hash: inputs.falsification_s2_suite_hash,
        generated_at: inputs.generated_at.clone(),
        rfc_revision: inputs.rfc_revision.clone(),
        predictions_section_hash: predictions_section_hash(&section_or_none(
            &inputs.predictions_markdown,
        ))?,
        predictions_commit: inputs.predictions_commit.clone(),
        first_result_commit: inputs.first_result_commit.clone(),
        hypothesis_statuses: inputs.verifier_bundle.hypothesis_statuses.clone(),
        pass_version_s2: inputs.pass_version_s2,
        report_self_hash: Hash256::ZERO,
    };
    let report = S2ReportFile::new(front_matter, body)?;
    let markdown = report.to_markdown()?;
    if let Some(parent) = inputs.output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&inputs.output_path, &markdown)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "report_written",
        event = "report_written",
        path = %inputs.output_path.display(),
        report_self_hash = %report.front_matter.report_self_hash,
        file_size_bytes = markdown.len() as u64,
        "s2 report written"
    );
    Ok(EmittedS2Report {
        path: inputs.output_path.clone(),
        report,
        markdown,
    })
}

/// Write a validated `s2_report.v1` markdown artifact.
pub fn write_report(path: impl AsRef<Path>, report: &S2ReportFile) -> Result<(), S2ReportError> {
    validate_report(report)?;
    let path = path.as_ref();
    let markdown = report.to_markdown()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &markdown)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "report_written",
        event = "report_written",
        path = %path.display(),
        report_self_hash = %report.front_matter.report_self_hash,
        file_size_bytes = markdown.len() as u64,
        "s2 report written"
    );
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "s2_report_finalized",
        event = "s2_report_finalized",
        outcome = %report.front_matter.s2_outcome,
        decision = %report.front_matter.decision,
        report_self_hash = %report.front_matter.report_self_hash,
        "s2 report finalized"
    );
    Ok(())
}

fn render_s2_report_body(
    inputs: &S2ReportInputs,
    outcome: S2Outcome,
    decision: &S2Decision,
) -> String {
    let predictions = section_or_none(&inputs.predictions_markdown);
    let observed = section_or_none(&inputs.observed_markdown);
    let falsification = section_or_none(&inputs.falsification_analysis);
    let surprises = section_or_none(&inputs.surprises);
    let decision_justification = section_or_none(&inputs.decision_justification);
    let replay_command = section_or_none(&inputs.replay_command);
    let manifest_references = section_or_none(&inputs.manifest_references);

    let mut rows = inputs.per_seed_artifacts.clone();
    rows.sort_by_key(|row| (row.build_kind, row.seed));

    let mut hypotheses = inputs
        .verifier_bundle
        .hypothesis_statuses
        .iter()
        .map(|(hypothesis, status)| (*hypothesis, status_label(status)))
        .collect::<Vec<_>>();
    hypotheses.sort_by_key(|(hypothesis, _)| *hypothesis);

    let mut body = String::new();
    body.push_str("# S2 Report\n\n");
    body.push_str("## Pre-registered predictions\n\n");
    body.push_str(&predictions);
    body.push_str("\n\n## Observed\n\n");
    body.push_str(&observed);
    body.push_str(
        "\n\n| build_kind | seed | completion | checkpoints | phase_log | score | distill_log |\n",
    );
    body.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for row in rows {
        body.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            row.build_kind,
            row.seed,
            completion_to_string(&row.completion),
            checkpoint_summary(&row.checkpoint_self_hashes),
            optional_hash(row.phase_log_self_hash),
            optional_hash(row.score_self_hash),
            optional_hash(row.distill_log_self_hash),
        ));
    }
    body.push_str("\n## Hypothesis verdicts\n\n");
    body.push_str("| hypothesis | status |\n");
    body.push_str("| --- | --- |\n");
    for (hypothesis, status) in hypotheses {
        body.push_str(&format!(
            "| {} | {} |\n",
            hypothesis,
            escape_table_cell(&status)
        ));
    }
    body.push_str("\n## Falsification analysis\n\n");
    body.push_str(&falsification);
    body.push_str("\n\n## Surprises\n\n");
    body.push_str(&surprises);
    body.push_str("\n\n## Decision\n\n");
    body.push_str(&format!(
        "`{decision}` from `{outcome}`. {decision_justification}\n\n"
    ));
    body.push_str("## Reproducibility statement\n\n");
    body.push_str(&format!(
        "- command: `{}`\n- pass_version_S2: `{}`\n- manifests: {}\n",
        escape_inline_code(&replay_command),
        inputs.pass_version_s2,
        manifest_references,
    ));
    body
}

fn section_or_none(markdown: &str) -> String {
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        "None.".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn completion_to_string(completion: &S2Completion) -> String {
    match completion {
        S2Completion::Completed => "Completed".to_owned(),
        S2Completion::DivergedAt { step } => format!("DivergedAt({step})"),
        S2Completion::NotReached => "NotReached".to_owned(),
    }
}

fn checkpoint_summary(checkpoints: &crate::s2::schema::S2CheckpointSelfHashes) -> String {
    format!(
        "A={} B={} C={} final={}",
        optional_hash(checkpoints.phase_a),
        optional_hash(checkpoints.phase_b),
        optional_hash(checkpoints.phase_c),
        optional_hash(checkpoints.final_checkpoint),
    )
}

fn optional_hash(hash: Option<Hash256>) -> String {
    hash.map_or_else(|| "null".to_owned(), |hash| hash.to_string())
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

fn escape_inline_code(value: &str) -> String {
    value.replace('`', "\\`")
}

/// Return the required decision for an S2 outcome.
#[must_use]
pub fn decision_for_outcome(outcome: S2Outcome) -> S2Decision {
    match outcome {
        S2Outcome::PassClean => S2Decision::ProceedToS3,
        S2Outcome::PassWithDistillWarn => S2Decision::ProceedToS3WithDistillReview,
        S2Outcome::FailGap => investigate("propose-tighten-D2-ramp-or-D3-temp"),
        S2Outcome::FailSubstrate => investigate("burn-or-distill-substrate"),
        S2Outcome::FailPhase => investigate("F4-phase-contract"),
        S2Outcome::FailLossGradFlow => investigate("loss-module-of-failing-sub-hyp"),
        S2Outcome::FailLinearstate => investigate("linearstate-autodiff-or-burn-adapter"),
        S2Outcome::FailPhaseIntegration => investigate("F4-phase-transition-integration"),
        S2Outcome::FailFalsification => investigate("S2-verifier-insensitive"),
        S2Outcome::FailApiDrift => investigate("public-api-drift-requires-amendment"),
        S2Outcome::FailMetric => halt("measurement-broken"),
        S2Outcome::FailSuspicious => halt("audit-split-and-bpc"),
        S2Outcome::FailPreregistration => halt("preregistration-invalid"),
        S2Outcome::FailArtifact => halt("artifact-missing-or-self-hash-invalid"),
        S2Outcome::FailIncomplete => halt("required-methodological-control-missing"),
    }
}

/// Dispatch a verifier bundle to the unique S2 outcome using the round-4 early-gate order.
#[must_use]
pub fn dispatch_outcome(bundle: &S2VerifierBundle) -> S2Outcome {
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "outcome_dispatch_start",
        bundle_summary = %bundle_summary(bundle),
        "s2 outcome dispatch start"
    );

    let outcome = first_matching_outcome(bundle);
    let decision = decision_for_outcome(outcome);
    let remediation_payload = decision_payload(&decision);
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "outcome_dispatch_decided",
        outcome = %outcome,
        decision = %decision,
        remediation_payload,
        "s2 outcome dispatch decided"
    );
    outcome
}

fn first_matching_outcome(bundle: &S2VerifierBundle) -> S2Outcome {
    if branch(
        "preregistration",
        !bundle.preregistration_passed,
        "pre-registration proof failed",
    ) {
        return S2Outcome::FailPreregistration;
    }
    if branch(
        "artifact_integrity",
        !bundle.artifact_integrity_passed,
        "required artifact missing or self-hash invalid",
    ) {
        return S2Outcome::FailArtifact;
    }
    if branch(
        "h6_linearstate",
        bundle.status(S2Hypothesis::H6) == HypothesisStatus::Refuted,
        "H6 LinearState smoke refuted",
    ) {
        return S2Outcome::FailLinearstate;
    }
    if branch(
        "h5_loss_grad_flow",
        bundle.status(S2Hypothesis::H5) == HypothesisStatus::Refuted,
        "H5 loss gradient-flow refuted",
    ) {
        return S2Outcome::FailLossGradFlow;
    }
    if branch(
        "d8_phase_transition_integration",
        !bundle.phase_transition_integ_passed,
        "D8 phase-transition integration failed",
    ) {
        return S2Outcome::FailPhaseIntegration;
    }
    if branch(
        "falsification",
        !bundle.falsification_s2_passed,
        "falsification suite failed",
    ) {
        return S2Outcome::FailFalsification;
    }
    if branch(
        "oracle_re_run",
        !bundle.oracle_re_run_passed,
        "metric oracle re-run failed",
    ) {
        return S2Outcome::FailMetric;
    }
    if branch(
        "api_drift",
        !bundle.api_drift_check_passed,
        "public API drift check failed",
    ) {
        return S2Outcome::FailApiDrift;
    }
    if branch(
        "substrate",
        bundle.any_seed_diverged() || bundle.status(S2Hypothesis::H1) == HypothesisStatus::Refuted,
        "seed diverged or H1 substrate verifier refuted",
    ) {
        return S2Outcome::FailSubstrate;
    }
    if branch(
        "h4_phase",
        bundle.status(S2Hypothesis::H4) == HypothesisStatus::Refuted,
        "H4 phase/ablation verifier refuted",
    ) {
        return S2Outcome::FailPhase;
    }
    if branch(
        "suspicious_low_bpc",
        bundle.suspicious_low_bpc,
        "median bpc crossed suspicious-low sentinel",
    ) {
        return S2Outcome::FailSuspicious;
    }
    if branch(
        "h2_gap",
        bundle.status(S2Hypothesis::H2) == HypothesisStatus::Refuted,
        "H2 ternary-vs-fp gap verifier refuted",
    ) {
        return S2Outcome::FailGap;
    }
    if branch(
        "h3_incomplete",
        !bundle.methodological_controls_present
            || matches!(
                bundle.status(S2Hypothesis::H3),
                HypothesisStatus::NotEvaluatedDueToPriorGate { .. }
            ),
        "H3 methodological control missing or not evaluated",
    ) {
        warn_not_evaluated_at_closure(bundle);
        return S2Outcome::FailIncomplete;
    }
    if branch(
        "not_reached",
        bundle.any_not_reached(),
        "one or more seed/build rows were not reached",
    ) {
        warn_not_evaluated_at_closure(bundle);
        return S2Outcome::FailIncomplete;
    }
    if let Some((hypothesis, status)) = bundle.first_not_evaluated() {
        warn_unbinary_hypothesis(hypothesis, &status);
        if branch(
            "unbinary_hypothesis",
            true,
            "hypothesis remained not evaluated at closure boundary",
        ) {
            return S2Outcome::FailIncomplete;
        }
    }
    if branch(
        "h3_distill_warn",
        bundle.status(S2Hypothesis::H3) == HypothesisStatus::Refuted,
        "H3 distillation contribution refuted",
    ) {
        return S2Outcome::PassWithDistillWarn;
    }
    let _ = branch(
        "pass_clean",
        true,
        "all closure gates and hypotheses passed",
    );
    S2Outcome::PassClean
}

fn branch(branch_id: &'static str, matched: bool, reasoning: &'static str) -> bool {
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "outcome_branch_evaluated",
        branch_id,
        matched,
        reasoning,
        "s2 outcome branch evaluated"
    );
    matched
}

fn warn_not_evaluated_at_closure(bundle: &S2VerifierBundle) {
    if let Some((hypothesis, status)) = bundle.first_not_evaluated() {
        warn_unbinary_hypothesis(hypothesis, &status);
    }
}

fn warn_unbinary_hypothesis(hypothesis: S2Hypothesis, status: &HypothesisStatus) {
    tracing::warn!(
        target: S2_LOG_TARGET,
        event_name = "outcome_unbinary_hypothesis_at_closure",
        hypothesis_id = %hypothesis,
        status = %status_label(status),
        "s2 outcome saw unbinary hypothesis at closure"
    );
}

fn status_label(status: &HypothesisStatus) -> String {
    match status {
        HypothesisStatus::Confirmed => "Confirmed".to_owned(),
        HypothesisStatus::Refuted => "Refuted".to_owned(),
        HypothesisStatus::NotEvaluatedDueToPriorGate { reason } => {
            format!("NotEvaluatedDueToPriorGate({reason})")
        }
    }
}

fn bundle_summary(bundle: &S2VerifierBundle) -> serde_json::Value {
    json!({
        "preregistration_passed": bundle.preregistration_passed,
        "artifact_integrity_passed": bundle.artifact_integrity_passed,
        "oracle_re_run_passed": bundle.oracle_re_run_passed,
        "api_drift_check_passed": bundle.api_drift_check_passed,
        "falsification_s2_passed": bundle.falsification_s2_passed,
        "phase_transition_integ_passed": bundle.phase_transition_integ_passed,
        "methodological_controls_present": bundle.methodological_controls_present,
        "suspicious_low_bpc": bundle.suspicious_low_bpc,
        "any_seed_diverged": bundle.any_seed_diverged(),
        "any_not_reached": bundle.any_not_reached(),
        "h1": status_label(&bundle.status(S2Hypothesis::H1)),
        "h2": status_label(&bundle.status(S2Hypothesis::H2)),
        "h3": status_label(&bundle.status(S2Hypothesis::H3)),
        "h4": status_label(&bundle.status(S2Hypothesis::H4)),
        "h5": status_label(&bundle.status(S2Hypothesis::H5)),
        "h6": status_label(&bundle.status(S2Hypothesis::H6)),
    })
}

fn decision_payload(decision: &S2Decision) -> Option<&str> {
    match decision {
        S2Decision::ProceedToS3 | S2Decision::ProceedToS3WithDistillReview => None,
        S2Decision::Investigate { reason } | S2Decision::Halt { reason } => Some(reason),
    }
}

fn investigate(reason: &str) -> S2Decision {
    S2Decision::Investigate {
        reason: reason.to_owned(),
    }
}

fn halt(reason: &str) -> S2Decision {
    S2Decision::Halt {
        reason: reason.to_owned(),
    }
}

fn validate_required_body_sections(body: &str) -> Result<(), S2ReportValidationError> {
    for heading in REQUIRED_REPORT_SECTIONS {
        if !body.contains(heading) {
            return Err(S2ReportValidationError::MissingBodySection { heading });
        }
    }
    Ok(())
}

fn validate_decision(front_matter: &S2ReportFrontMatter) -> Result<(), S2ReportValidationError> {
    let expected = decision_for_outcome(front_matter.s2_outcome);
    if front_matter.decision == expected {
        Ok(())
    } else {
        Err(S2ReportValidationError::DecisionMismatch {
            outcome: front_matter.s2_outcome,
            expected,
            actual: front_matter.decision.clone(),
        })
    }
}

fn validate_all_seeds(front_matter: &S2ReportFrontMatter) -> Result<(), S2ReportValidationError> {
    let mut observed = BTreeSet::new();
    for row in &front_matter.per_seed_artifacts {
        if !REQUIRED_REPORT_BUILDS.contains(&row.build_kind) {
            return Err(S2ReportValidationError::UnexpectedBuildKind {
                build_kind: row.build_kind,
            });
        }
        if !observed.insert((row.build_kind, row.seed)) {
            return Err(S2ReportValidationError::DuplicateSeedBuild {
                build_kind: row.build_kind,
                seed: row.seed,
            });
        }
    }
    for build_kind in REQUIRED_REPORT_BUILDS {
        for seed in REQUIRED_SEEDS {
            if !observed.contains(&(build_kind, seed)) {
                return Err(S2ReportValidationError::MissingSeedBuild { build_kind, seed });
            }
        }
    }
    Ok(())
}

fn validate_closure_artifacts(
    front_matter: &S2ReportFrontMatter,
) -> Result<(), S2ReportValidationError> {
    if !is_closure_decision(&front_matter.decision) {
        return Ok(());
    }
    require_closure_gate("oracle_re_run_passed", front_matter.oracle_re_run_passed)?;
    require_closure_gate(
        "api_drift_check_passed",
        front_matter.api_drift_check_passed,
    )?;
    require_closure_gate(
        "phase_transition_integ_passed",
        front_matter.phase_transition_integ_passed,
    )?;
    require_closure_gate(
        "falsification_s2_passed",
        front_matter.falsification_s2_passed,
    )?;
    require_top_level_artifact("ablation_self_hash", front_matter.ablation_self_hash)?;
    for row in &front_matter.per_seed_artifacts {
        if row.completion != S2Completion::Completed {
            return Err(S2ReportValidationError::MissingClosureArtifact {
                seed: Some(row.seed),
                build_kind: Some(row.build_kind),
                field: "completion=Completed",
            });
        }
        if !row.checkpoint_self_hashes.all_present() {
            return Err(S2ReportValidationError::MissingClosureArtifact {
                seed: Some(row.seed),
                build_kind: Some(row.build_kind),
                field: "checkpoint_self_hashes",
            });
        }
        require_row_artifact(row, "phase_log_self_hash", row.phase_log_self_hash)?;
        require_row_artifact(row, "score_self_hash", row.score_self_hash)?;
        if row.build_kind != S2BuildKind::s2_ternary_nodistill {
            require_row_artifact(row, "distill_log_self_hash", row.distill_log_self_hash)?;
        }
    }
    Ok(())
}

fn validate_self_hash(report: &S2ReportFile) -> Result<(), S2ReportValidationError> {
    let mut front_matter = report.front_matter.clone();
    let actual = front_matter.report_self_hash;
    front_matter.report_self_hash = Hash256::ZERO;
    let expected = report_self_hash(&front_matter, &report.body).map_err(|_| {
        S2ReportValidationError::SelfHashMismatch {
            expected: Hash256::ZERO,
            actual,
        }
    })?;
    if expected == actual {
        Ok(())
    } else {
        Err(S2ReportValidationError::SelfHashMismatch { expected, actual })
    }
}

fn validate_predictions(
    front_matter: &S2ReportFrontMatter,
    body: &str,
) -> Result<(), S2ReportValidationError> {
    let section = section_body(body, "## Pre-registered predictions").ok_or(
        S2ReportValidationError::MissingBodySection {
            heading: "## Pre-registered predictions",
        },
    )?;
    let expected = predictions_section_hash(section).map_err(|_| {
        S2ReportValidationError::PredictionsSectionHashMismatch {
            expected: Hash256::ZERO,
            actual: front_matter.predictions_section_hash,
        }
    })?;
    if expected != front_matter.predictions_section_hash {
        return Err(S2ReportValidationError::PredictionsSectionHashMismatch {
            expected,
            actual: front_matter.predictions_section_hash,
        });
    }
    if front_matter.predictions_commit.as_str() == front_matter.first_result_commit.as_str() {
        return Err(
            S2ReportValidationError::PredictionsCommitEqualsFirstResult {
                predictions_commit: front_matter.predictions_commit.as_str().to_owned(),
                first_result_commit: front_matter.first_result_commit.as_str().to_owned(),
            },
        );
    }
    Ok(())
}

fn validate_all_hypotheses(
    front_matter: &S2ReportFrontMatter,
) -> Result<(), S2ReportValidationError> {
    for hypothesis in REQUIRED_HYPOTHESES {
        let status = front_matter
            .hypothesis_statuses
            .get(&hypothesis)
            .ok_or(S2ReportValidationError::MissingHypothesis { hypothesis })?;
        if is_closure_decision(&front_matter.decision)
            && let HypothesisStatus::NotEvaluatedDueToPriorGate { reason } = status
        {
            return Err(S2ReportValidationError::NotEvaluatedClosureHypothesis {
                hypothesis,
                reason: reason.clone(),
            });
        }
        if is_refuted_closure_failure(&front_matter.decision, hypothesis, status) {
            return Err(S2ReportValidationError::RefutedClosureHypothesis { hypothesis });
        }
    }
    Ok(())
}

fn is_closure_decision(decision: &S2Decision) -> bool {
    matches!(
        decision,
        S2Decision::ProceedToS3 | S2Decision::ProceedToS3WithDistillReview
    )
}

fn is_refuted_closure_failure(
    decision: &S2Decision,
    hypothesis: S2Hypothesis,
    status: &HypothesisStatus,
) -> bool {
    if !is_closure_decision(decision) || !matches!(status, HypothesisStatus::Refuted) {
        return false;
    }
    !(hypothesis == S2Hypothesis::H3
        && matches!(decision, S2Decision::ProceedToS3WithDistillReview))
}

fn require_closure_gate(field: &'static str, passed: bool) -> Result<(), S2ReportValidationError> {
    if passed {
        Ok(())
    } else {
        Err(S2ReportValidationError::ClosureGateFailed { field })
    }
}

fn require_top_level_artifact(
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), S2ReportValidationError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(S2ReportValidationError::MissingClosureArtifact {
            seed: None,
            build_kind: None,
            field,
        })
    }
}

fn require_row_artifact(
    row: &S2PerSeedArtifacts,
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), S2ReportValidationError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(S2ReportValidationError::MissingClosureArtifact {
            seed: Some(row.seed),
            build_kind: Some(row.build_kind),
            field,
        })
    }
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

fn hash_report_preimage(
    front_matter_canonical_json: &[u8],
    body: &str,
) -> Result<Hash256, S1SchemaError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(front_matter_canonical_json);
    bytes.push(0);
    bytes.extend_from_slice(REPORT_HASH_DOMAIN_SEPARATOR);
    bytes.extend_from_slice(body.as_bytes());
    Ok(sha256(bytes))
}

impl fmt::Display for S2Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::PassClean => "Pass-clean",
            Self::PassWithDistillWarn => "Pass-with-distill-warn",
            Self::FailSubstrate => "Fail-substrate",
            Self::FailGap => "Fail-gap",
            Self::FailSuspicious => "Fail-suspicious",
            Self::FailPhase => "Fail-phase",
            Self::FailLossGradFlow => "Fail-loss-grad-flow",
            Self::FailLinearstate => "Fail-linearstate",
            Self::FailPhaseIntegration => "Fail-phase-integration",
            Self::FailFalsification => "Fail-falsification",
            Self::FailApiDrift => "Fail-api-drift",
            Self::FailMetric => "Fail-metric",
            Self::FailPreregistration => "Fail-preregistration",
            Self::FailArtifact => "Fail-artifact",
            Self::FailIncomplete => "Fail-incomplete",
        };
        f.write_str(value)
    }
}

impl fmt::Display for S2Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProceedToS3 => f.write_str("ProceedToS3"),
            Self::ProceedToS3WithDistillReview => f.write_str("ProceedToS3-with-distill-review"),
            Self::Investigate { reason } => write!(f, "Investigate({reason})"),
            Self::Halt { reason } => write!(f, "Halt({reason})"),
        }
    }
}

impl fmt::Display for S2Hypothesis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::H1 => "H1",
            Self::H2 => "H2",
            Self::H3 => "H3",
            Self::H4 => "H4",
            Self::H5 => "H5",
            Self::H6 => "H6",
        };
        f.write_str(value)
    }
}

impl fmt::Display for S2BuildKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::s2_ternary_full => "s2-ternary-full",
            Self::s2_fp_full => "s2-fp-full",
            Self::s2_ternary_nodistill => "s2-ternary-nodistill",
            Self::s2_ablation => "s2-ablation",
        };
        f.write_str(value)
    }
}
