//! S3 outcome dispatcher.

use std::fmt;

use serde_json::json;

use crate::S3_LOG_TARGET;
use crate::s3::schema::{HypothesisStatus, S3Decision, S3Hypothesis, S3Outcome, S3VerifierBundle};

/// Event emitted when S3 outcome dispatch starts.
pub const EVENT_NAME_DISPATCH_STARTED: &str = "s3::dispatcher::dispatch_started";
/// Event emitted for each evaluated dispatch ladder branch.
pub const EVENT_NAME_LADDER_DECISION: &str = "s3::dispatcher::ladder_decision";
/// Event emitted when S3 outcome dispatch completes.
pub const EVENT_NAME_DISPATCH_COMPLETE: &str = "s3::dispatcher::dispatch_complete";

/// Dispatch a verifier bundle to the unique S3 outcome and decision.
#[must_use]
pub fn dispatch(bundle: &S3VerifierBundle) -> (S3Outcome, S3Decision) {
    tracing::info!(
        target: report_log_target(),
        event_name = EVENT_NAME_DISPATCH_STARTED,
        verifier_bundle_summary = %bundle_summary(bundle),
        "s3 outcome dispatch started"
    );
    let outcome = dispatch_outcome(bundle);
    let decision = decision_for_outcome(outcome);
    tracing::info!(
        target: report_log_target(),
        event_name = EVENT_NAME_DISPATCH_COMPLETE,
        s3_outcome = %outcome,
        s3_decision = %decision,
        "s3 outcome dispatch complete"
    );
    (outcome, decision)
}

/// Dispatch a verifier bundle to the unique S3 outcome using RFC section 10.
#[must_use]
pub fn dispatch_outcome(bundle: &S3VerifierBundle) -> S3Outcome {
    first_matching_outcome(bundle)
}

/// Return the required decision for an S3 outcome.
#[must_use]
pub fn decision_for_outcome(outcome: S3Outcome) -> S3Decision {
    match outcome {
        S3Outcome::PassClean => S3Decision::ProceedToS4,
        S3Outcome::PassWithFallbackOracle => S3Decision::ProceedToS4WithDeferredClause,
        S3Outcome::FailCharset => halt("charset-broken"),
        S3Outcome::FailBaseline => halt("baseline-broken"),
        S3Outcome::FailQuality => investigate("quality-gap"),
        S3Outcome::FailSuspicious => halt("audit-split-and-bpc-char"),
        S3Outcome::FailOracleAgreement => halt("oracle-disagreement"),
        S3Outcome::FailBundle => halt("bundle-nondeterministic"),
        S3Outcome::FailQuantspec => halt("quantspec-resolution-broken"),
        S3Outcome::FailSubstrate => investigate("burn-or-autodiff-or-phase"),
        S3Outcome::FailPhase => investigate("F4-phase-contract"),
        S3Outcome::FailFalsification => halt("s3-falsification-suite"),
        S3Outcome::FailApiDrift => halt("public-api-drift"),
        S3Outcome::FailMetric => halt("oracle-re-run-regressed"),
        S3Outcome::FailPreregistration => halt("preregistration-proof"),
        S3Outcome::FailArtifact => halt("artifact-self-hash"),
        S3Outcome::FailIncomplete => investigate("missing-controls"),
    }
}

fn first_matching_outcome(bundle: &S3VerifierBundle) -> S3Outcome {
    if branch(
        "preregistration_passed",
        bundle.preregistration_passed,
        "Fail-preregistration",
    ) {
        return S3Outcome::FailPreregistration;
    }
    if branch(
        "artifact_integrity_passed",
        bundle.artifact_integrity_passed,
        "Fail-artifact",
    ) {
        return S3Outcome::FailArtifact;
    }
    if branch(
        "falsification_s3_passed",
        bundle.falsification_s3_passed,
        "Fail-falsification",
    ) {
        return S3Outcome::FailFalsification;
    }
    if branch(
        "api_drift_check_passed",
        bundle.api_drift_check_passed,
        "Fail-api-drift",
    ) {
        return S3Outcome::FailApiDrift;
    }
    if branch(
        "oracle_re_run_passed",
        bundle.oracle_re_run_passed,
        "Fail-metric",
    ) {
        return S3Outcome::FailMetric;
    }
    if branch(
        "completion_diverged",
        !bundle.any_seed_diverged(),
        "Fail-substrate",
    ) {
        return S3Outcome::FailSubstrate;
    }
    if branch(
        "charset_idempotence_passed",
        bundle.charset_idempotence_passed,
        "Fail-charset",
    ) || branch(
        "H1",
        bundle.status(S3Hypothesis::H1) != HypothesisStatus::Refuted,
        "Fail-charset",
    ) {
        return S3Outcome::FailCharset;
    }
    if branch("kn_oracle_passed", bundle.kn_oracle_passed, "Fail-baseline")
        || branch(
            "H2",
            bundle.status(S3Hypothesis::H2) != HypothesisStatus::Refuted,
            "Fail-baseline",
        )
    {
        return S3Outcome::FailBaseline;
    }
    if branch(
        "H7",
        bundle.status(S3Hypothesis::H7) != HypothesisStatus::Refuted,
        "Fail-phase",
    ) {
        return S3Outcome::FailPhase;
    }
    if branch(
        "bundle_determinism_passed",
        bundle.bundle_determinism_passed,
        "Fail-bundle",
    ) || branch(
        "artifact_determinism_passed",
        bundle.artifact_determinism_passed,
        "Fail-bundle",
    ) || branch(
        "H5",
        bundle.status(S3Hypothesis::H5) != HypothesisStatus::Refuted,
        "Fail-bundle",
    ) {
        return S3Outcome::FailBundle;
    }
    if branch(
        "quantspec_resolution_passed",
        bundle.quantspec_resolution_passed,
        "Fail-quantspec",
    ) || branch(
        "H6",
        bundle.status(S3Hypothesis::H6) != HypothesisStatus::Refuted,
        "Fail-quantspec",
    ) {
        return S3Outcome::FailQuantspec;
    }
    if branch(
        "oracle_agreement_passed",
        bundle.oracle_agreement_passed,
        "Fail-oracle-agreement",
    ) || branch(
        "H4",
        bundle.status(S3Hypothesis::H4) != HypothesisStatus::Refuted,
        "Fail-oracle-agreement",
    ) {
        return S3Outcome::FailOracleAgreement;
    }
    if branch(
        "suspicious_low_bpc",
        !bundle.suspicious_low_bpc,
        "Fail-suspicious",
    ) {
        return S3Outcome::FailSuspicious;
    }
    if branch(
        "H3",
        bundle.status(S3Hypothesis::H3) != HypothesisStatus::Refuted,
        "Fail-quality",
    ) {
        return S3Outcome::FailQuality;
    }
    if branch(
        "methodological_controls_present",
        bundle.methodological_controls_present,
        "Fail-incomplete",
    ) {
        return S3Outcome::FailIncomplete;
    }
    if branch(
        "completion_reached",
        !bundle.any_not_reached(),
        "Fail-incomplete",
    ) {
        return S3Outcome::FailIncomplete;
    }
    if let Some((hypothesis, status)) = bundle.first_not_evaluated() {
        tracing::warn!(
            target: report_log_target(),
            event_name = "s3::dispatcher::unbinary_hypothesis_at_closure",
            hypothesis_id = %hypothesis,
            status = %status_label(&status),
            "s3 outcome saw unbinary hypothesis at closure"
        );
        let _ = branch("all_hypotheses_evaluated", false, "Fail-incomplete");
        return S3Outcome::FailIncomplete;
    }
    if branch(
        "oracle_fallback_absent",
        bundle.oracle_fallback_used.is_empty(),
        "Pass-with-fallback-oracle",
    ) {
        return S3Outcome::PassWithFallbackOracle;
    }
    let _ = branch("pass_clean", true, "Pass-clean");
    S3Outcome::PassClean
}

fn branch(gate_name: &'static str, gate_passed: bool, decision_branch: &'static str) -> bool {
    tracing::info!(
        target: report_log_target(),
        event_name = EVENT_NAME_LADDER_DECISION,
        gate_name,
        gate_passed,
        decision_branch,
        "s3 dispatch ladder branch evaluated"
    );
    !gate_passed
}

fn investigate(reason: &str) -> S3Decision {
    S3Decision::Investigate {
        reason: reason.to_owned(),
    }
}

fn halt(reason: &str) -> S3Decision {
    S3Decision::Halt {
        reason: reason.to_owned(),
    }
}

fn bundle_summary(bundle: &S3VerifierBundle) -> serde_json::Value {
    json!({
        "preregistration_passed": bundle.preregistration_passed,
        "artifact_integrity_passed": bundle.artifact_integrity_passed,
        "oracle_re_run_passed": bundle.oracle_re_run_passed,
        "api_drift_check_passed": bundle.api_drift_check_passed,
        "falsification_s3_passed": bundle.falsification_s3_passed,
        "bundle_determinism_passed": bundle.bundle_determinism_passed,
        "artifact_determinism_passed": bundle.artifact_determinism_passed,
        "charset_idempotence_passed": bundle.charset_idempotence_passed,
        "kn_oracle_passed": bundle.kn_oracle_passed,
        "oracle_agreement_passed": bundle.oracle_agreement_passed,
        "quantspec_resolution_passed": bundle.quantspec_resolution_passed,
        "methodological_controls_present": bundle.methodological_controls_present,
        "suspicious_low_bpc": bundle.suspicious_low_bpc,
        "any_seed_diverged": bundle.any_seed_diverged(),
        "any_not_reached": bundle.any_not_reached(),
        "oracle_fallback_used": bundle.oracle_fallback_used,
        "h1": status_label(&bundle.status(S3Hypothesis::H1)),
        "h2": status_label(&bundle.status(S3Hypothesis::H2)),
        "h3": status_label(&bundle.status(S3Hypothesis::H3)),
        "h4": status_label(&bundle.status(S3Hypothesis::H4)),
        "h5": status_label(&bundle.status(S3Hypothesis::H5)),
        "h6": status_label(&bundle.status(S3Hypothesis::H6)),
        "h7": status_label(&bundle.status(S3Hypothesis::H7)),
    })
}

pub(crate) fn status_label(status: &HypothesisStatus) -> String {
    match status {
        HypothesisStatus::Confirmed => "Confirmed".to_owned(),
        HypothesisStatus::Refuted => "Refuted".to_owned(),
        HypothesisStatus::NotEvaluatedDueToPriorGate { reason } => {
            format!("NotEvaluatedDueToPriorGate({reason})")
        }
    }
}

pub(crate) const fn report_log_target() -> &'static str {
    "gbf_experiments::s3::report"
}

impl fmt::Display for S3Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::PassClean => "Pass-clean",
            Self::PassWithFallbackOracle => "Pass-with-fallback-oracle",
            Self::FailCharset => "Fail-charset",
            Self::FailBaseline => "Fail-baseline",
            Self::FailQuality => "Fail-quality",
            Self::FailSuspicious => "Fail-suspicious",
            Self::FailOracleAgreement => "Fail-oracle-agreement",
            Self::FailBundle => "Fail-bundle",
            Self::FailQuantspec => "Fail-quantspec",
            Self::FailSubstrate => "Fail-substrate",
            Self::FailPhase => "Fail-phase",
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

impl fmt::Display for S3Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProceedToS4 => f.write_str("ProceedToS4"),
            Self::ProceedToS4WithDeferredClause => f.write_str("ProceedToS4-with-deferred-clause"),
            Self::Investigate { reason } => write!(f, "Investigate({reason})"),
            Self::Halt { reason } => write!(f, "Halt({reason})"),
        }
    }
}

impl fmt::Display for S3Hypothesis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::H1 => "H1",
            Self::H2 => "H2",
            Self::H3 => "H3",
            Self::H4 => "H4",
            Self::H5 => "H5",
            Self::H6 => "H6",
            Self::H7 => "H7",
        };
        f.write_str(value)
    }
}

#[allow(dead_code)]
fn _assert_target_constant_is_kept_alive() {
    let _ = S3_LOG_TARGET;
}
