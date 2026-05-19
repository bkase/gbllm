#![cfg(feature = "s3")]

mod report_s3_support;

use gbf_experiments::s3::report::{ReportError, ReportValidationError, validate_r_predictions};

#[test]
fn r_predictions_validates_hash_and_git_ancestry() {
    let report = report_s3_support::pass_clean_report();

    validate_r_predictions(&report).expect("fixture predictions are pre-registered");
}

#[test]
fn r_predictions_rejects_equal_prediction_and_result_commits() {
    let mut report = report_s3_support::pass_clean_report();
    report.front_matter.predictions_commit = report.front_matter.first_result_commit.clone();
    report.front_matter.report_self_hash =
        gbf_experiments::s3::report::report_self_hash(&report.front_matter, &report.body)
            .expect("rehash");

    let error = validate_r_predictions(&report).expect_err("equal commits are rejected");
    assert!(matches!(
        error,
        ReportError::Validation(ReportValidationError::PredictionsCommitEqualsFirstResult { .. })
    ));
}
