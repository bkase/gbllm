#![cfg(feature = "s3")]

mod report_s3_support;

use gbf_experiments::s3::report::emit_report;
use gbf_experiments::s3::schema::{OracleFallbackTag, S3Decision, S3Outcome};

#[test]
fn pass_with_fallback_report_emits_deferred_clause_front_matter() {
    let report = report_s3_support::pass_with_fallback_report();
    let bytes = emit_report(&report).expect("fallback report emits");
    let markdown = String::from_utf8(bytes).expect("markdown is UTF-8");

    assert_eq!(
        report.front_matter.s3_outcome,
        S3Outcome::PassWithFallbackOracle
    );
    assert_eq!(
        report.front_matter.decision,
        S3Decision::ProceedToS4WithDeferredClause
    );
    assert_eq!(
        report.front_matter.oracle_fallback_used,
        vec![OracleFallbackTag::S3DenotationalFallback]
    );
    assert!(markdown.contains("Pass-with-fallback-oracle"));
    assert!(markdown.contains("S3DenotationalFallback"));
}
