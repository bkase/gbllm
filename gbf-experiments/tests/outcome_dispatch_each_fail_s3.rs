#![cfg(feature = "s3")]

use gbf_experiments::s3::report::{decision_for_outcome, dispatch_outcome};
use gbf_experiments::s3::schema::{
    HypothesisStatus, S3Completion, S3Decision, S3Hypothesis, S3Outcome, S3VerifierBundle,
};

type MutateBundle = fn(&mut S3VerifierBundle);

#[test]
fn every_fail_outcome_is_reachable_via_dispatcher() {
    let cases: [(&str, MutateBundle, S3Outcome); 15] = [
        (
            "preregistration",
            |b| b.preregistration_passed = false,
            S3Outcome::FailPreregistration,
        ),
        (
            "artifact",
            |b| b.artifact_integrity_passed = false,
            S3Outcome::FailArtifact,
        ),
        (
            "falsification",
            |b| b.falsification_s3_passed = false,
            S3Outcome::FailFalsification,
        ),
        (
            "api_drift",
            |b| b.api_drift_check_passed = false,
            S3Outcome::FailApiDrift,
        ),
        (
            "metric",
            |b| b.oracle_re_run_passed = false,
            S3Outcome::FailMetric,
        ),
        (
            "substrate",
            |b| b.completions[0] = S3Completion::DivergedAt { step: 17 },
            S3Outcome::FailSubstrate,
        ),
        (
            "charset",
            |b| set_status(b, S3Hypothesis::H1, HypothesisStatus::Refuted),
            S3Outcome::FailCharset,
        ),
        (
            "baseline",
            |b| set_status(b, S3Hypothesis::H2, HypothesisStatus::Refuted),
            S3Outcome::FailBaseline,
        ),
        (
            "phase",
            |b| set_status(b, S3Hypothesis::H7, HypothesisStatus::Refuted),
            S3Outcome::FailPhase,
        ),
        (
            "bundle",
            |b| set_status(b, S3Hypothesis::H5, HypothesisStatus::Refuted),
            S3Outcome::FailBundle,
        ),
        (
            "quantspec",
            |b| set_status(b, S3Hypothesis::H6, HypothesisStatus::Refuted),
            S3Outcome::FailQuantspec,
        ),
        (
            "oracle_agreement",
            |b| set_status(b, S3Hypothesis::H4, HypothesisStatus::Refuted),
            S3Outcome::FailOracleAgreement,
        ),
        (
            "suspicious",
            |b| b.suspicious_low_bpc = true,
            S3Outcome::FailSuspicious,
        ),
        (
            "quality",
            |b| set_status(b, S3Hypothesis::H3, HypothesisStatus::Refuted),
            S3Outcome::FailQuality,
        ),
        (
            "incomplete",
            |b| b.methodological_controls_present = false,
            S3Outcome::FailIncomplete,
        ),
    ];

    let reached = cases
        .into_iter()
        .map(|(name, mutate, expected)| {
            let mut bundle = S3VerifierBundle::closure_candidate();
            mutate(&mut bundle);
            let actual = dispatch_outcome(&bundle);
            assert_eq!(actual, expected, "{name}");
            actual
        })
        .collect::<Vec<_>>();

    for outcome in S3Outcome::ALL {
        if matches!(
            outcome,
            S3Outcome::PassClean | S3Outcome::PassWithFallbackOracle
        ) {
            continue;
        }
        assert!(
            reached.contains(&outcome),
            "{outcome:?} lacks a direct dispatcher fixture"
        );
    }
}

#[test]
fn decision_table_matches_rfc_section_10() {
    assert_eq!(
        decision_for_outcome(S3Outcome::PassClean),
        S3Decision::ProceedToS4
    );
    assert_eq!(
        decision_for_outcome(S3Outcome::PassWithFallbackOracle),
        S3Decision::ProceedToS4WithDeferredClause
    );
    assert_eq!(
        decision_for_outcome(S3Outcome::FailCharset),
        S3Decision::Halt {
            reason: "charset-broken".to_owned()
        }
    );
    assert_eq!(
        decision_for_outcome(S3Outcome::FailQuality),
        S3Decision::Investigate {
            reason: "quality-gap".to_owned()
        }
    );
    assert_eq!(
        decision_for_outcome(S3Outcome::FailIncomplete),
        S3Decision::Investigate {
            reason: "missing-controls".to_owned()
        }
    );
}

#[test]
fn dispatcher_first_matching_ladder_rung_wins_over_later_failures() {
    let mut bundle = S3VerifierBundle::closure_candidate();
    bundle.artifact_integrity_passed = false;
    bundle.falsification_s3_passed = false;
    bundle.oracle_re_run_passed = false;
    bundle.suspicious_low_bpc = true;
    set_status(&mut bundle, S3Hypothesis::H1, HypothesisStatus::Refuted);
    set_status(&mut bundle, S3Hypothesis::H3, HypothesisStatus::Refuted);

    assert_eq!(dispatch_outcome(&bundle), S3Outcome::FailArtifact);
}

fn set_status(bundle: &mut S3VerifierBundle, hypothesis: S3Hypothesis, status: HypothesisStatus) {
    bundle.hypothesis_statuses.insert(hypothesis, status);
}
