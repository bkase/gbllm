#![cfg(feature = "s3")]

mod report_s3_support;

use gbf_experiments::s3::report::{
    ReportError, ReportValidationError, S3ReportValidator, emit_report, report_self_hash,
    validate_report, validate_report_validator,
};
use gbf_experiments::s3::schema::{HypothesisStatus, S3Decision, S3Hypothesis};

#[test]
fn r_validators_reject_each_public_violation_shape() {
    let mut report = report_s3_support::pass_clean_report();
    report.body = report.body.replace(
        report_s3_support::predictions_text(),
        "tampered prediction text",
    );
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::Predictions),
        "R-Predictions",
    );

    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.per_seed_artifacts.pop();
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::AllSeeds),
        "R-AllSeeds",
    );

    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.report_self_hash = report_s3_support::hash(99);
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::SelfHash),
        "R-Self-Hash",
    );

    let mut report = report_s3_support::pass_clean_report();
    report
        .front_matter
        .hypothesis_statuses
        .remove(&S3Hypothesis::H7);
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::AllHypotheses),
        "R-AllHypotheses",
    );

    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.oracle_owner_beads.artifact.clear();
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::OwnerBeads),
        "R-OwnerBeads",
    );

    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.decision = S3Decision::Halt {
        reason: "wrong".to_owned(),
    };
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::Decision),
        "R-Decision",
    );

    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.per_seed_artifacts[0].artifact_self_hash = None;
    assert_validation_variant(
        validate_report_validator(&report, S3ReportValidator::ClosureArtifacts),
        "R-ClosureArtifacts",
    );
}

#[test]
fn closure_report_rejects_not_evaluated_and_refuted_hypotheses() {
    let mut not_evaluated = report_s3_support::pass_clean_report();
    not_evaluated.front_matter.hypothesis_statuses.insert(
        S3Hypothesis::H4,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "prior-gate".to_owned(),
        },
    );
    assert!(matches!(
        validate_report_validator(&not_evaluated, S3ReportValidator::AllHypotheses),
        Err(ReportError::Validation(
            ReportValidationError::NotEvaluatedClosureHypothesis { .. }
        ))
    ));

    let mut refuted = report_s3_support::pass_clean_report();
    refuted
        .front_matter
        .hypothesis_statuses
        .insert(S3Hypothesis::H4, HypothesisStatus::Refuted);
    assert!(matches!(
        validate_report_validator(&refuted, S3ReportValidator::AllHypotheses),
        Err(ReportError::Validation(
            ReportValidationError::RefutedClosureHypothesis { .. }
        ))
    ));
}

#[test]
fn report_validation_rejects_wrong_schema_literal() {
    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.schema = "s3_report.v2".to_owned();

    for result in [validate_report(&report), emit_report(&report).map(|_| ())] {
        assert!(matches!(
            result,
            Err(ReportError::Validation(
                ReportValidationError::InvalidSchema {
                    expected: "s3_report.v1",
                    ..
                }
            ))
        ));
    }
}

#[test]
fn report_validation_rejects_missing_required_body_section() {
    let mut report = report_s3_support::pass_clean_report();
    report.body = report.body.replace(
        "## Surprises\nNo out-of-band surprise notes for this fixture.\n\n",
        "",
    );

    assert!(matches!(
        validate_report(&report),
        Err(ReportError::Validation(
            ReportValidationError::MissingBodySection {
                heading: "## Surprises"
            }
        ))
    ));
}

#[test]
fn report_rehash_helper_exposes_exact_self_hash_mismatch() {
    let mut report = report_s3_support::pass_clean_report();
    let expected = report_self_hash(&report.front_matter, &report.body).expect("hash");
    report.front_matter.report_self_hash = report_s3_support::hash(101);
    let error = validate_report_validator(&report, S3ReportValidator::SelfHash)
        .expect_err("self hash mismatch");
    match error {
        ReportError::Validation(ReportValidationError::SelfHashMismatch {
            expected: got,
            actual,
        }) => {
            assert_eq!(got, expected);
            assert_eq!(actual, report_s3_support::hash(101));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn assert_validation_variant(result: Result<(), ReportError>, label: &str) {
    let error = result.expect_err("validator must reject fixture mutation");
    assert!(
        error.to_string().contains(label),
        "expected {label} error, got {error}"
    );
}
