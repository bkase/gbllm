#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::export_product;

#[test]
fn artifact_canonical_snapshot_s3() {
    let product = export_product(0);

    assert_eq!(
        product.canonical_artifact_payload_sha.to_string(),
        "sha256:d6f0d9194644d19c75d77017d61a0b5d1523e81865f35e5aec638e7083bc0eeb"
    );
}
