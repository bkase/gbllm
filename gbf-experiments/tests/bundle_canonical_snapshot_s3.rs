#![cfg(feature = "s3")]

mod bundle_s3_support;

use bundle_s3_support::export_product;

#[test]
fn canonical_bundle_payload_sha_matches_fixed_toy0_snapshot() {
    let product = export_product(0);

    assert_eq!(
        product.canonical_bundle_payload_sha.to_string(),
        "sha256:7750838e6d9e6e05327460102c988300ab731542328d7be270b5ab308b39ff8d"
    );
}
