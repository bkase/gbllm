#![cfg(all(feature = "s3", feature = "s3-oracle-fallback"))]

mod oracle_agreement_s3_support;

use gbf_oracle::phase_surface_agreement::OracleFallbackTag;
use oracle_agreement_s3_support::{EXPECTED_FULL_RECORD_COUNT, run_default_agreement};

#[test]
fn oracle_agreement_fallback_s3() {
    let product = run_default_agreement();

    assert_eq!(product.records.len(), EXPECTED_FULL_RECORD_COUNT);
    assert!(product.overall_pass);
    assert_eq!(
        product.fallback_used,
        vec![
            OracleFallbackTag::S3ArtifactFallback,
            OracleFallbackTag::S3DenotationalFallback,
            OracleFallbackTag::S3LiveObservationFixture
        ]
    );
}
