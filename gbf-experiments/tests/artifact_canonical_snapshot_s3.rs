#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::export_product;

#[test]
fn artifact_canonical_snapshot_s3() {
    let product = export_product(0);

    assert_eq!(
        product.canonical_artifact_payload_sha.to_string(),
        "sha256:739b6fa6f5f9668f3599669b436951ce2de5512d99abb76b58c563fcd8cfc59f"
    );
}
