#![cfg(feature = "s3")]

mod common;
mod common_s3;

use common_s3::proptest_strategies_s3::arb_s3_verifier_bundle;
use gbf_experiments::s3::report::{decision_for_outcome, dispatch, dispatch_outcome};
use gbf_experiments::s3::schema::{
    HypothesisStatus, S3Completion, S3Decision, S3Hypothesis, S3Outcome, S3VerifierBundle,
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn dispatcher_is_total_and_deterministic(bundle in arb_s3_verifier_bundle()) {
        let first = dispatch_outcome(&bundle);
        let second = dispatch_outcome(&bundle);
        prop_assert_eq!(first, second);
        prop_assert_eq!(first, expected_outcome_for_bundle(&bundle));
        prop_assert!(S3Outcome::ALL.contains(&first));
        let (_, decision) = dispatch(&bundle);
        let decision_is_covered = matches!(
            &decision,
            S3Decision::ProceedToS4
                | S3Decision::ProceedToS4WithDeferredClause
                | S3Decision::Investigate { .. }
                | S3Decision::Halt { .. }
        );
        prop_assert_eq!(decision, decision_for_outcome(first));
        prop_assert!(decision_is_covered);
    }

    #[test]
    fn proceed_decisions_require_completed_binary_closure(bundle in arb_s3_verifier_bundle()) {
        let outcome = dispatch_outcome(&bundle);
        let decision = decision_for_outcome(outcome);
        if matches!(decision, S3Decision::ProceedToS4 | S3Decision::ProceedToS4WithDeferredClause) {
            prop_assert!(bundle.completions.iter().all(|completion| matches!(completion, S3Completion::Completed)));
            prop_assert!(bundle.methodological_controls_present);
            prop_assert!(bundle.preregistration_passed);
            prop_assert!(bundle.artifact_integrity_passed);
            prop_assert!(bundle.falsification_s3_passed);
            prop_assert!(bundle.api_drift_check_passed);
            prop_assert!(bundle.oracle_re_run_passed);
            prop_assert!(bundle.bundle_determinism_passed);
            prop_assert!(bundle.artifact_determinism_passed);
            prop_assert!(bundle.charset_idempotence_passed);
            prop_assert!(bundle.kn_oracle_passed);
            prop_assert!(bundle.oracle_agreement_passed);
            prop_assert!(bundle.quantspec_resolution_passed);
            prop_assert!(!bundle.suspicious_low_bpc);
            for hypothesis in S3Hypothesis::ALL {
                prop_assert_eq!(bundle.status(hypothesis), HypothesisStatus::Confirmed);
            }
        }
    }
}

fn expected_outcome_for_bundle(bundle: &S3VerifierBundle) -> S3Outcome {
    if !bundle.preregistration_passed {
        S3Outcome::FailPreregistration
    } else if !bundle.artifact_integrity_passed {
        S3Outcome::FailArtifact
    } else if !bundle.falsification_s3_passed {
        S3Outcome::FailFalsification
    } else if !bundle.api_drift_check_passed {
        S3Outcome::FailApiDrift
    } else if !bundle.oracle_re_run_passed {
        S3Outcome::FailMetric
    } else if bundle.any_seed_diverged() {
        S3Outcome::FailSubstrate
    } else if !bundle.charset_idempotence_passed
        || bundle.status(S3Hypothesis::H1) == HypothesisStatus::Refuted
    {
        S3Outcome::FailCharset
    } else if !bundle.kn_oracle_passed
        || bundle.status(S3Hypothesis::H2) == HypothesisStatus::Refuted
    {
        S3Outcome::FailBaseline
    } else if bundle.status(S3Hypothesis::H7) == HypothesisStatus::Refuted {
        S3Outcome::FailPhase
    } else if !bundle.bundle_determinism_passed
        || !bundle.artifact_determinism_passed
        || bundle.status(S3Hypothesis::H5) == HypothesisStatus::Refuted
    {
        S3Outcome::FailBundle
    } else if !bundle.quantspec_resolution_passed
        || bundle.status(S3Hypothesis::H6) == HypothesisStatus::Refuted
    {
        S3Outcome::FailQuantspec
    } else if !bundle.oracle_agreement_passed
        || bundle.status(S3Hypothesis::H4) == HypothesisStatus::Refuted
    {
        S3Outcome::FailOracleAgreement
    } else if bundle.suspicious_low_bpc {
        S3Outcome::FailSuspicious
    } else if bundle.status(S3Hypothesis::H3) == HypothesisStatus::Refuted {
        S3Outcome::FailQuality
    } else if !bundle.methodological_controls_present
        || bundle.any_not_reached()
        || bundle.first_not_evaluated().is_some()
    {
        S3Outcome::FailIncomplete
    } else if !bundle.oracle_fallback_used.is_empty() {
        S3Outcome::PassWithFallbackOracle
    } else {
        S3Outcome::PassClean
    }
}
