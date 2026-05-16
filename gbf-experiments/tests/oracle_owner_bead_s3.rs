#![cfg(all(feature = "s3", feature = "s3-oracle-fallback"))]

mod denotational_s3_support;

use denotational_s3_support::evaluate;
use gbf_oracle::denotational::{S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD, S3DenotationalFallback};

#[test]
fn oracle_owner_bead_s3() {
    let product = evaluate(S3DenotationalFallback);

    assert_eq!(
        product.real_owner_bead,
        Some(S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD)
    );
    assert_eq!(product.real_owner_bead, Some("bd-1rcc"));
}
