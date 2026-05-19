#![cfg(feature = "s3")]

use gbf_experiments::s3::report::dispatch;
use gbf_experiments::s3::schema::{OracleFallbackTag, S3Decision, S3Outcome, S3VerifierBundle};

#[test]
fn fallback_oracle_dispatches_to_deferred_clause() {
    let mut bundle = S3VerifierBundle::closure_candidate();
    bundle
        .oracle_fallback_used
        .push(OracleFallbackTag::S3DenotationalFallback);

    let (outcome, decision) = dispatch(&bundle);

    assert_eq!(outcome, S3Outcome::PassWithFallbackOracle);
    assert_eq!(decision, S3Decision::ProceedToS4WithDeferredClause);
}
