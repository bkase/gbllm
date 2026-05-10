//! Deterministic report generation for S1.

use std::collections::BTreeSet;
use std::fmt;
use std::string::FromUtf8Error;

use gbf_foundation::{Hash256, sha256};
use serde_json::Value;

use crate::s1::logging::{
    LoggingEventError, OutcomeDispatchCompleteEvent, OutcomeDispatchStartEvent,
    OutcomeRefutedInputEvent, ReportEmitCompleteEvent, ReportEmitStartEvent, ReportValidatorEvent,
    ReportValidatorsRunEvent, S1LogEmitter,
};
use crate::s1::schema::{
    PerSeedArtifacts, ReportFrontMatter, RfcRevisionRef, S1CanonicalJson, S1Completion, S1Decision,
    S1Outcome, S1SchemaError,
};

const REPORT_HASH_DOMAIN_PREFIX: &[u8] = b"gbf:gbf-experiments:ReportFile:s1_report.v1:1\0";
const REPORT_OUTPUT_PATH: &str = "docs/experiments/S1-report.md";
const REQUIRED_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];

/// Canonical reason tag for the human-approved Toy1 H2 narrow-miss waiver.
pub const S1_H2_WAIVER_REASON: &str = "toy1-narrow-h2-miss";

/// Binary verdict used by S1 hypothesis dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Hypothesis confirmed by its evidence.
    Confirmed,
    /// Hypothesis refuted by its evidence.
    Refuted,
}

/// Evaluation status for one S1 hypothesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HypothesisStatus {
    /// Hypothesis confirmed by its evidence.
    Confirmed,
    /// Hypothesis refuted by its evidence.
    Refuted,
    /// Hypothesis was not reachable because an earlier gate failed.
    NotEvaluatedDueToPriorGate(String),
}

impl HypothesisStatus {
    fn verdict(&self, hypothesis: Hypothesis) -> Result<Verdict, OutcomeDispatchError> {
        match self {
            Self::Confirmed => Ok(Verdict::Confirmed),
            Self::Refuted => Ok(Verdict::Refuted),
            Self::NotEvaluatedDueToPriorGate(reason) => {
                Err(OutcomeDispatchError::NotEvaluatedHypothesis {
                    hypothesis,
                    reason: reason.clone(),
                })
            }
        }
    }
}

impl From<Verdict> for HypothesisStatus {
    fn from(value: Verdict) -> Self {
        match value {
            Verdict::Confirmed => Self::Confirmed,
            Verdict::Refuted => Self::Refuted,
        }
    }
}

/// One of the five F-S1 hypotheses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hypothesis {
    /// H1 plumbing/substrate.
    H1,
    /// H2 Toy0 capacity.
    H2,
    /// H3 sequence-state utility.
    H3,
    /// H4 Phase A cleanliness.
    H4,
    /// H5 measurement correctness.
    H5,
}

/// Inputs to the §8 S1 outcome dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeDispatchInput {
    /// H1 status.
    pub h1: HypothesisStatus,
    /// H2 status.
    pub h2: HypothesisStatus,
    /// H3 status.
    pub h3: HypothesisStatus,
    /// H4 status.
    pub h4: HypothesisStatus,
    /// H5 status.
    pub h5: HypothesisStatus,
    /// Whether any seed completion diverged.
    pub any_seed_diverged: bool,
    /// Whether median validation bpc is below the suspicious threshold.
    pub suspicious_low_bpc: bool,
}

/// Successful §8 outcome dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeDispatch {
    /// Selected S1 outcome.
    pub outcome: S1Outcome,
    /// Decision derived from the selected outcome.
    pub decision: S1Decision,
}

/// A rendered `s1_report.v1` artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportFile {
    /// Canonical front matter with `report_self_hash` populated.
    pub front_matter: ReportFrontMatter,
    /// Markdown body bytes covered by `report_self_hash`.
    pub body: String,
}

impl ReportFile {
    /// Render canonical-JSON front matter plus the markdown body.
    pub fn to_markdown(&self) -> Result<String, ReportError> {
        let front_matter = String::from_utf8(S1CanonicalJson::to_vec(&self.front_matter)?)?;
        Ok(format!("---\n{front_matter}\n---\n{}", self.body))
    }
}

/// Inputs consumed by the `s1_report.v1` emitter.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportInput {
    /// Front matter before `report_self_hash` is populated.
    pub front_matter: ReportFrontMatter,
    /// Exact pre-registered predictions section body.
    pub predictions_markdown: String,
    /// Per-seed observations rendered in `## Observed`.
    pub observed_per_seed: Vec<ObservedSeed>,
    /// Hypothesis verdicts and the observations that drove them.
    pub hypotheses: Vec<HypothesisFinding>,
    /// Markdown text for `## Falsification analysis`.
    pub falsification_analysis: String,
    /// Markdown text for `## Surprises`.
    pub surprises: String,
    /// Short decision justification.
    pub decision_justification: String,
    /// Replay command for `## Reproducibility statement`.
    pub replay_command: String,
    /// Manifest/hash summary for `## Reproducibility statement`.
    pub manifest_hashes: String,
    /// S1 pass implementation version.
    pub pass_version: String,
}

/// One row in the observed per-seed report table.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedSeed {
    /// Seed id.
    pub seed: u64,
    /// Completion state observed for this seed.
    pub completion: S1Completion,
    /// Validation bpc, when scoring was reached.
    pub val_bpc: Option<f64>,
    /// Seed-0 shuffled-minus-original negative-test delta, if produced.
    pub neg_test_delta: Option<f64>,
    /// Seed-0 ablation equality result, if produced.
    pub ablation_eq: Option<bool>,
}

/// One hypothesis verdict plus its load-bearing observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypothesisFinding {
    /// Hypothesis id.
    pub hypothesis: Hypothesis,
    /// Evaluation status.
    pub status: HypothesisStatus,
    /// Concrete observation that drove this status.
    pub observation: String,
}

/// Individual report validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportValidator {
    /// R-Decision.
    Decision,
    /// R-AllSeeds.
    AllSeeds,
    /// R-ClosureArtifacts.
    ClosureArtifacts,
    /// R-Self-Hash.
    SelfHash,
    /// R-Predictions.
    Predictions,
    /// R-AllHypotheses.
    AllHypotheses,
}

impl ReportValidator {
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
            Self::Decision => "R-Decision",
            Self::AllSeeds => "R-AllSeeds",
            Self::ClosureArtifacts => "R-ClosureArtifacts",
            Self::SelfHash => "R-Self-Hash",
            Self::Predictions => "R-Predictions",
            Self::AllHypotheses => "R-AllHypotheses",
        }
    }
}

/// Errors from report emission and validation.
#[derive(Debug)]
pub enum ReportError {
    /// Canonical schema serialization or hashing failed.
    Schema(S1SchemaError),
    /// Structured logging failed validation.
    Logging(LoggingEventError),
    /// UTF-8 rendering failed unexpectedly.
    Utf8(FromUtf8Error),
    /// One of the R-* validators failed.
    Validation(ReportValidationError),
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
            Self::Utf8(error) => write!(f, "{error}"),
            Self::Validation(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::Utf8(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

impl From<S1SchemaError> for ReportError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<LoggingEventError> for ReportError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

impl From<FromUtf8Error> for ReportError {
    fn from(error: FromUtf8Error) -> Self {
        Self::Utf8(error)
    }
}

impl From<ReportValidationError> for ReportError {
    fn from(error: ReportValidationError) -> Self {
        Self::Validation(error)
    }
}

/// A typed R-* validator failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportValidationError {
    /// The front-matter decision does not match the selected outcome.
    DecisionMismatch {
        /// Selected outcome.
        outcome: S1Outcome,
        /// Expected decision.
        expected: S1Decision,
        /// Actual decision.
        actual: S1Decision,
    },
    /// A seed id appeared more than once.
    DuplicateSeed {
        /// Surface where the duplicate was found.
        surface: &'static str,
        /// Seed id.
        seed: u64,
    },
    /// A required seed id was absent.
    MissingSeed {
        /// Surface where the seed was absent.
        surface: &'static str,
        /// Seed id.
        seed: u64,
    },
    /// A seed outside the required set appeared.
    UnexpectedSeed {
        /// Surface where the seed was found.
        surface: &'static str,
        /// Seed id.
        seed: u64,
    },
    /// A closure-candidate artifact hash was absent.
    MissingClosureArtifact {
        /// Seed id.
        seed: u64,
        /// Missing field.
        field: &'static str,
    },
    /// Recomputed report self-hash did not match front matter.
    SelfHashMismatch {
        /// Expected recomputed hash.
        expected: Hash256,
        /// Actual front-matter hash.
        actual: Hash256,
    },
    /// The predictions section hash does not match the section body.
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
    /// A hypothesis appeared more than once.
    DuplicateHypothesis {
        /// Hypothesis id.
        hypothesis: Hypothesis,
    },
    /// A required hypothesis was absent.
    MissingHypothesis {
        /// Hypothesis id.
        hypothesis: Hypothesis,
    },
    /// A closure-candidate report tried to proceed with a skipped hypothesis.
    NotEvaluatedClosureHypothesis {
        /// Hypothesis id.
        hypothesis: Hypothesis,
        /// Prior-gate reason.
        reason: String,
    },
    /// A hypothesis finding did not carry a concrete observation.
    EmptyHypothesisObservation {
        /// Hypothesis id.
        hypothesis: Hypothesis,
    },
}

impl fmt::Display for ReportValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecisionMismatch {
                outcome,
                expected,
                actual,
            } => write!(
                f,
                "R-Decision failed for {outcome}: expected {expected}, got {actual}"
            ),
            Self::DuplicateSeed { surface, seed } => {
                write!(f, "R-AllSeeds failed: duplicate seed {seed} in {surface}")
            }
            Self::MissingSeed { surface, seed } => {
                write!(f, "R-AllSeeds failed: missing seed {seed} in {surface}")
            }
            Self::UnexpectedSeed { surface, seed } => {
                write!(f, "R-AllSeeds failed: unexpected seed {seed} in {surface}")
            }
            Self::MissingClosureArtifact { seed, field } => {
                write!(f, "R-ClosureArtifacts failed: seed {seed} missing {field}")
            }
            Self::SelfHashMismatch { expected, actual } => {
                write!(f, "R-Self-Hash failed: expected {expected}, got {actual}")
            }
            Self::PredictionsSectionHashMismatch { expected, actual } => write!(
                f,
                "R-Predictions failed: expected predictions_section_hash {expected}, got {actual}"
            ),
            Self::PredictionsCommitEqualsFirstResult {
                predictions_commit,
                first_result_commit,
            } => write!(
                f,
                "R-Predictions failed: predictions_commit {predictions_commit} equals first_result_commit {first_result_commit}; strict ancestry is verified by scripts/s1_preregistration_check.sh"
            ),
            Self::DuplicateHypothesis { hypothesis } => write!(
                f,
                "R-AllHypotheses failed: duplicate hypothesis {hypothesis}"
            ),
            Self::MissingHypothesis { hypothesis } => {
                write!(f, "R-AllHypotheses failed: missing hypothesis {hypothesis}")
            }
            Self::NotEvaluatedClosureHypothesis { hypothesis, reason } => write!(
                f,
                "R-AllHypotheses failed: closure-candidate {hypothesis} is NotEvaluatedDueToPriorGate({reason})"
            ),
            Self::EmptyHypothesisObservation { hypothesis } => write!(
                f,
                "R-AllHypotheses failed: {hypothesis} observation must not be empty"
            ),
        }
    }
}

impl std::error::Error for ReportValidationError {}

/// Errors from S1 outcome dispatch.
#[derive(Debug, Clone, PartialEq)]
pub enum OutcomeDispatchError {
    /// A required hypothesis status was not evaluated.
    NotEvaluatedHypothesis {
        /// Hypothesis with missing binary evidence.
        hypothesis: Hypothesis,
        /// Prior-gate reason recorded by the report producer.
        reason: String,
    },
    /// Structured outcome logging failed validation.
    Logging(LoggingEventError),
}

impl fmt::Display for OutcomeDispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEvaluatedHypothesis { hypothesis, reason } => {
                write!(
                    f,
                    "{hypothesis} cannot be NotEvaluatedDueToPriorGate for §8 outcome dispatch: {reason}"
                )
            }
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OutcomeDispatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Logging(error) => Some(error),
            Self::NotEvaluatedHypothesis { .. } => None,
        }
    }
}

impl From<LoggingEventError> for OutcomeDispatchError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

/// Emit a self-hashed `s1_report.v1` file from explicit report inputs.
pub fn emit_report(input: &ReportInput) -> Result<ReportFile, ReportError> {
    let emitter = S1LogEmitter::new();
    let mut front_matter = input.front_matter.clone();
    front_matter.per_seed_artifacts.sort_by_key(|row| row.seed);
    front_matter.report_self_hash = Hash256::ZERO;

    emitter.report_emit_start(&ReportEmitStartEvent {
        rfc_revision: rfc_revision_to_string(&front_matter.rfc_revision),
        pass_version: input.pass_version.clone(),
    })?;

    let body = render_report_body(input);
    front_matter.report_self_hash = report_self_hash(&front_matter, &body)?;
    let report = ReportFile { front_matter, body };

    validate_report(&report, input)?;

    for finding in input
        .hypotheses
        .iter()
        .filter(|finding| finding.status == HypothesisStatus::Refuted)
    {
        emitter.outcome_refuted_input(&OutcomeRefutedInputEvent {
            hypothesis: finding.hypothesis.to_string(),
            observation: finding.observation.clone(),
        })?;
    }

    emitter.report_validators_run(&ReportValidatorsRunEvent {
        decision: report.front_matter.decision.to_string(),
        verdict: "PASS".to_owned(),
        validators: ReportValidator::all()
            .iter()
            .map(|validator| validator.as_str())
            .collect::<Vec<_>>()
            .join(","),
    })?;
    emitter.report_emit_complete(&ReportEmitCompleteEvent {
        report_self_hash: report.front_matter.report_self_hash.to_string(),
        output_path: REPORT_OUTPUT_PATH.to_owned(),
        outcome: report.front_matter.s1_outcome.to_string(),
        decision: report.front_matter.decision.to_string(),
    })?;

    Ok(report)
}

/// Validate a rendered report against the explicit inputs that produced it.
pub fn validate_report(report: &ReportFile, input: &ReportInput) -> Result<(), ReportError> {
    let emitter = S1LogEmitter::new();
    run_validator(&emitter, ReportValidator::Decision, || {
        validate_decision(&report.front_matter)
    })?;
    run_validator(&emitter, ReportValidator::AllSeeds, || {
        validate_all_seeds(report, input)
    })?;
    run_validator(&emitter, ReportValidator::ClosureArtifacts, || {
        validate_closure_artifacts(&report.front_matter)
    })?;
    run_validator(&emitter, ReportValidator::SelfHash, || {
        validate_self_hash(report)
    })?;
    run_validator(&emitter, ReportValidator::Predictions, || {
        validate_predictions(&report.front_matter, &input.predictions_markdown)
    })?;
    run_validator(&emitter, ReportValidator::AllHypotheses, || {
        validate_all_hypotheses(&report.front_matter, &input.hypotheses)
    })?;
    Ok(())
}

/// Compute the `s1_report.v1` self-hash from canonical front matter plus body bytes.
pub fn report_self_hash(
    front_matter: &ReportFrontMatter,
    body: &str,
) -> Result<Hash256, S1SchemaError> {
    hash_report_preimage(&front_matter.canonical_json_bytes()?, body)
}

/// Compute a pre-registration report self-hash from placeholder front matter.
///
/// This exists for the pre-result `S1-report.md` commit, where fields such as
/// `s1_outcome`, `decision`, `predictions_commit`, and `first_result_commit`
/// are intentionally placeholders until F-S1.29 assembles the final report.
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

/// Compute `predictions_section_hash` for the exact pre-registered section body.
///
/// The contract is:
///
/// ```text
/// sha256(S1CanonicalJson::to_vec(markdown.trim()))
/// ```
///
/// This intentionally hashes the markdown section as a canonical JSON string,
/// not raw UTF-8 bytes. `scripts/s1_preregistration_check.sh` mirrors the same
/// convention so report validation and the history check reject the same drift.
pub fn predictions_section_hash(markdown: &str) -> Result<Hash256, S1SchemaError> {
    Ok(sha256(S1CanonicalJson::to_vec(&markdown.trim())?))
}

fn hash_report_preimage(
    front_matter_canonical_json: &[u8],
    body: &str,
) -> Result<Hash256, S1SchemaError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(REPORT_HASH_DOMAIN_PREFIX);
    bytes.extend_from_slice(front_matter_canonical_json);
    bytes.push(0);
    bytes.extend_from_slice(body.as_bytes());
    Ok(sha256(bytes))
}

fn run_validator(
    emitter: &S1LogEmitter,
    validator: ReportValidator,
    validate: impl FnOnce() -> Result<(), ReportValidationError>,
) -> Result<(), ReportError> {
    match validate() {
        Ok(()) => {
            emitter.report_validator(&ReportValidatorEvent {
                validator: validator.as_str().to_owned(),
                status: "PASS".to_owned(),
                diagnostic: "ok".to_owned(),
            })?;
            Ok(())
        }
        Err(error) => {
            let diagnostic = error.to_string();
            emitter.report_validator(&ReportValidatorEvent {
                validator: validator.as_str().to_owned(),
                status: "FAIL".to_owned(),
                diagnostic,
            })?;
            Err(error.into())
        }
    }
}

fn render_report_body(input: &ReportInput) -> String {
    let mut observed = input.observed_per_seed.clone();
    observed.sort_by_key(|row| row.seed);
    let mut hypotheses = input.hypotheses.clone();
    hypotheses.sort_by_key(|finding| finding.hypothesis);

    let predictions = input.predictions_markdown.trim();
    let falsification = input.falsification_analysis.trim();
    let surprises = input.surprises.trim();
    let decision_justification = input.decision_justification.trim();
    let replay_command = input.replay_command.trim();
    let manifest_hashes = input.manifest_hashes.trim();

    let mut body = String::new();
    body.push_str("# S1 Report\n\n");
    body.push_str("## Pre-registered predictions\n\n");
    body.push_str(if predictions.is_empty() {
        "None."
    } else {
        predictions
    });
    body.push_str("\n\n## Observed\n\n");
    body.push_str("| seed | completion | val_bpc | neg_test_delta | ablation_eq |\n");
    body.push_str("| --- | --- | --- | --- | --- |\n");
    for row in observed {
        body.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.seed,
            completion_to_string(&row.completion),
            optional_f64(row.val_bpc),
            optional_f64(row.neg_test_delta),
            optional_bool(row.ablation_eq),
        ));
    }
    body.push_str("\n## Hypothesis verdicts\n\n");
    body.push_str("| hypothesis | status | observation |\n");
    body.push_str("| --- | --- | --- |\n");
    for finding in hypotheses {
        body.push_str(&format!(
            "| {} | {} | {} |\n",
            finding.hypothesis,
            finding.status,
            escape_table_cell(&finding.observation)
        ));
    }
    body.push_str("\n## Falsification analysis\n\n");
    body.push_str(if falsification.is_empty() {
        "None."
    } else {
        falsification
    });
    body.push_str("\n\n## Surprises\n\n");
    body.push_str(if surprises.is_empty() {
        "None."
    } else {
        surprises
    });
    body.push_str("\n\n## Decision\n\n");
    body.push_str(&format!(
        "`{}`. {}\n\n",
        input.front_matter.decision, decision_justification
    ));
    body.push_str("## Reproducibility statement\n\n");
    body.push_str(&format!(
        "- command: `{replay_command}`\n- pass_version: `{}`\n- manifests: {manifest_hashes}\n",
        input.pass_version
    ));
    body
}

fn validate_decision(front_matter: &ReportFrontMatter) -> Result<(), ReportValidationError> {
    let expected = decision_for_outcome(front_matter.s1_outcome);
    if front_matter.decision == expected
        || is_registered_capacity_successor_decision(front_matter)
        || is_registered_capacity_waiver_decision(front_matter)
    {
        Ok(())
    } else {
        Err(ReportValidationError::DecisionMismatch {
            outcome: front_matter.s1_outcome,
            expected,
            actual: front_matter.decision.clone(),
        })
    }
}

fn is_registered_capacity_waiver_decision(front_matter: &ReportFrontMatter) -> bool {
    matches!(
        (&front_matter.s1_outcome, &front_matter.decision),
        (
            S1Outcome::FailCapacity,
            S1Decision::ProceedToS2WithH2Waiver { reason }
        ) if reason == S1_H2_WAIVER_REASON
    )
}

fn is_registered_capacity_successor_decision(front_matter: &ReportFrontMatter) -> bool {
    matches!(
        (&front_matter.s1_outcome, &front_matter.decision),
        (
            S1Outcome::FailCapacity,
            S1Decision::Investigate { reason }
        ) if reason == "propose-Toy2"
    )
}

fn validate_all_seeds(
    report: &ReportFile,
    input: &ReportInput,
) -> Result<(), ReportValidationError> {
    validate_seed_set(
        "per_seed_artifacts",
        report
            .front_matter
            .per_seed_artifacts
            .iter()
            .map(|row| row.seed),
    )?;
    validate_seed_set(
        "observed_per_seed",
        input.observed_per_seed.iter().map(|row| row.seed),
    )
}

fn validate_seed_set(
    surface: &'static str,
    seeds: impl IntoIterator<Item = u64>,
) -> Result<(), ReportValidationError> {
    let mut observed = BTreeSet::new();
    for seed in seeds {
        if !REQUIRED_SEEDS.contains(&seed) {
            return Err(ReportValidationError::UnexpectedSeed { surface, seed });
        }
        if !observed.insert(seed) {
            return Err(ReportValidationError::DuplicateSeed { surface, seed });
        }
    }
    for seed in REQUIRED_SEEDS {
        if !observed.contains(&seed) {
            return Err(ReportValidationError::MissingSeed { surface, seed });
        }
    }
    Ok(())
}

fn validate_closure_artifacts(
    front_matter: &ReportFrontMatter,
) -> Result<(), ReportValidationError> {
    if !is_closure_decision(&front_matter.decision) {
        return Ok(());
    }
    for row in &front_matter.per_seed_artifacts {
        require_artifact(row, "checkpoint_self_hash", row.checkpoint_self_hash)?;
        require_artifact(row, "run_log_self_hash", row.run_log_self_hash)?;
        require_artifact(row, "score_self_hash", row.score_self_hash)?;
        if row.seed == 0 {
            require_artifact(row, "negative_self_hash", row.negative_self_hash)?;
            require_artifact(row, "ablation_self_hash", row.ablation_self_hash)?;
        }
    }
    Ok(())
}

fn require_artifact(
    row: &PerSeedArtifacts,
    field: &'static str,
    hash: Option<Hash256>,
) -> Result<(), ReportValidationError> {
    if hash.is_some() {
        Ok(())
    } else {
        Err(ReportValidationError::MissingClosureArtifact {
            seed: row.seed,
            field,
        })
    }
}

fn validate_self_hash(report: &ReportFile) -> Result<(), ReportValidationError> {
    let mut front_matter = report.front_matter.clone();
    let actual = front_matter.report_self_hash;
    front_matter.report_self_hash = Hash256::ZERO;
    let expected = report_self_hash(&front_matter, &report.body).map_err(|_| {
        ReportValidationError::SelfHashMismatch {
            expected: Hash256::ZERO,
            actual,
        }
    })?;
    if expected == actual {
        Ok(())
    } else {
        Err(ReportValidationError::SelfHashMismatch { expected, actual })
    }
}

fn validate_predictions(
    front_matter: &ReportFrontMatter,
    predictions_markdown: &str,
) -> Result<(), ReportValidationError> {
    let expected = predictions_section_hash(predictions_markdown).map_err(|_| {
        ReportValidationError::PredictionsSectionHashMismatch {
            expected: Hash256::ZERO,
            actual: front_matter.predictions_section_hash,
        }
    })?;
    if expected != front_matter.predictions_section_hash {
        return Err(ReportValidationError::PredictionsSectionHashMismatch {
            expected,
            actual: front_matter.predictions_section_hash,
        });
    }
    if front_matter.predictions_commit.as_str() == front_matter.first_result_commit.as_str() {
        return Err(ReportValidationError::PredictionsCommitEqualsFirstResult {
            predictions_commit: front_matter.predictions_commit.as_str().to_owned(),
            first_result_commit: front_matter.first_result_commit.as_str().to_owned(),
        });
    }
    Ok(())
}

fn validate_all_hypotheses(
    front_matter: &ReportFrontMatter,
    hypotheses: &[HypothesisFinding],
) -> Result<(), ReportValidationError> {
    let mut observed = BTreeSet::new();
    for finding in hypotheses {
        if !observed.insert(finding.hypothesis) {
            return Err(ReportValidationError::DuplicateHypothesis {
                hypothesis: finding.hypothesis,
            });
        }
        if finding.observation.trim().is_empty() {
            return Err(ReportValidationError::EmptyHypothesisObservation {
                hypothesis: finding.hypothesis,
            });
        }
        if is_closure_decision(&front_matter.decision)
            && let HypothesisStatus::NotEvaluatedDueToPriorGate(reason) = &finding.status
        {
            return Err(ReportValidationError::NotEvaluatedClosureHypothesis {
                hypothesis: finding.hypothesis,
                reason: reason.clone(),
            });
        }
    }
    for hypothesis in [
        Hypothesis::H1,
        Hypothesis::H2,
        Hypothesis::H3,
        Hypothesis::H4,
        Hypothesis::H5,
    ] {
        if !observed.contains(&hypothesis) {
            return Err(ReportValidationError::MissingHypothesis { hypothesis });
        }
    }
    Ok(())
}

fn is_closure_decision(decision: &S1Decision) -> bool {
    matches!(
        decision,
        S1Decision::ProceedToS2
            | S1Decision::ProceedToS2WithT125Prereq
            | S1Decision::ProceedToS2WithH2Waiver { .. }
    )
}

fn rfc_revision_to_string(revision: &RfcRevisionRef) -> String {
    match revision {
        RfcRevisionRef::GitCommitId(commit) => commit.as_str().to_owned(),
        RfcRevisionRef::Hash256(hash) => hash.to_string(),
    }
}

fn completion_to_string(completion: &S1Completion) -> String {
    match completion {
        S1Completion::Completed => "Completed".to_owned(),
        S1Completion::DivergedAt { step } => format!("DivergedAt({step})"),
        S1Completion::NotReached => "NotReached".to_owned(),
    }
}

fn optional_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| "NA".to_owned())
}

fn optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "NA",
    }
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

/// Dispatch S1 hypothesis statuses to the unique §8 outcome and decision.
///
/// The dispatcher evaluates hypotheses lazily in RFC §8 precedence order:
/// substrate divergence and H1 failure short-circuit before downstream
/// hypotheses are required. Proceed outcomes still require every hypothesis
/// needed to justify proceeding to have a binary status, so
/// `NotEvaluatedDueToPriorGate` cannot silently become `ProceedToS2`.
pub fn dispatch_outcome(
    input: &OutcomeDispatchInput,
) -> Result<OutcomeDispatch, OutcomeDispatchError> {
    let emitter = S1LogEmitter::new();
    emitter.outcome_dispatch_start(&OutcomeDispatchStartEvent {
        h1: input.h1.to_string(),
        h2: input.h2.to_string(),
        h3: input.h3.to_string(),
        h4: input.h4.to_string(),
        h5: input.h5.to_string(),
        any_seed_diverged: input.any_seed_diverged,
        suspicious_low_bpc: input.suspicious_low_bpc,
    })?;

    let outcome = dispatch_outcome_kind(input)?;
    let decision = decision_for_outcome(outcome);

    emitter.outcome_dispatch_complete(&OutcomeDispatchCompleteEvent {
        outcome: outcome.to_string(),
        decision: decision.to_string(),
    })?;

    Ok(OutcomeDispatch { outcome, decision })
}

fn dispatch_outcome_kind(input: &OutcomeDispatchInput) -> Result<S1Outcome, OutcomeDispatchError> {
    if input.any_seed_diverged {
        return Ok(S1Outcome::FailSubstrate);
    }

    if input.h1.verdict(Hypothesis::H1)? == Verdict::Refuted {
        return Ok(S1Outcome::FailSubstrate);
    }
    if input.h5.verdict(Hypothesis::H5)? == Verdict::Refuted {
        return Ok(S1Outcome::FailMetric);
    }
    if input.h4.verdict(Hypothesis::H4)? == Verdict::Refuted {
        return Ok(S1Outcome::FailPhase);
    }
    if input.suspicious_low_bpc {
        return Ok(S1Outcome::FailSuspicious);
    }
    if input.h2.verdict(Hypothesis::H2)? == Verdict::Refuted {
        return Ok(S1Outcome::FailCapacity);
    }
    if input.h3.verdict(Hypothesis::H3)? == Verdict::Refuted {
        return Ok(S1Outcome::PassWithWarning);
    }

    Ok(S1Outcome::PassClean)
}

/// Map an S1 outcome to its §8 decision.
#[must_use]
pub fn decision_for_outcome(outcome: S1Outcome) -> S1Decision {
    match outcome {
        S1Outcome::PassClean => S1Decision::ProceedToS2,
        S1Outcome::PassWithWarning => S1Decision::ProceedToS2WithT125Prereq,
        S1Outcome::FailCapacity => S1Decision::Investigate {
            reason: "propose-Toy1".to_owned(),
        },
        S1Outcome::FailSubstrate => S1Decision::Investigate {
            reason: "burn-or-autodiff".to_owned(),
        },
        S1Outcome::FailPhase => S1Decision::Investigate {
            reason: "F4-phase-contract".to_owned(),
        },
        S1Outcome::FailMetric => S1Decision::Halt {
            reason: "measurement-broken".to_owned(),
        },
        S1Outcome::FailSuspicious => S1Decision::Halt {
            reason: "audit-split-and-bpc".to_owned(),
        },
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirmed => f.write_str("Confirmed"),
            Self::Refuted => f.write_str("Refuted"),
        }
    }
}

impl fmt::Display for HypothesisStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirmed => f.write_str("Confirmed"),
            Self::Refuted => f.write_str("Refuted"),
            Self::NotEvaluatedDueToPriorGate(reason) => {
                write!(f, "NotEvaluatedDueToPriorGate({reason})")
            }
        }
    }
}

impl fmt::Display for Hypothesis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::H1 => f.write_str("H1"),
            Self::H2 => f.write_str("H2"),
            Self::H3 => f.write_str("H3"),
            Self::H4 => f.write_str("H4"),
            Self::H5 => f.write_str("H5"),
        }
    }
}

impl fmt::Display for S1Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PassClean => f.write_str("Pass-clean"),
            Self::PassWithWarning => f.write_str("Pass-with-warning"),
            Self::FailSubstrate => f.write_str("Fail-substrate"),
            Self::FailCapacity => f.write_str("Fail-capacity"),
            Self::FailSuspicious => f.write_str("Fail-suspicious"),
            Self::FailPhase => f.write_str("Fail-phase"),
            Self::FailMetric => f.write_str("Fail-metric"),
        }
    }
}

impl fmt::Display for S1Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProceedToS2 => f.write_str("ProceedToS2"),
            Self::ProceedToS2WithT125Prereq => f.write_str("ProceedToS2-with-T12.5-prereq"),
            Self::ProceedToS2WithH2Waiver { reason } => {
                write!(f, "ProceedToS2-with-H2-waiver({reason})")
            }
            Self::Investigate { reason } => write!(f, "Investigate({reason})"),
            Self::Halt { reason } => write!(f, "Halt({reason})"),
        }
    }
}
