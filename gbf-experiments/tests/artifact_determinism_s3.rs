#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::{export_product_from, frozen_student};

#[test]
fn artifact_determinism_s3() {
    for seed in 0..5 {
        let first = frozen_student(seed);
        let second = frozen_student(seed);
        let first_product = export_product_from(&first);
        let second_product = export_product_from(&second);

        assert_eq!(
            first_product.canonical_artifact_payload_sha,
            second_product.canonical_artifact_payload_sha,
            "canonical artifact payload sha differed for seed {seed}"
        );
        assert_eq!(
            first_product.artifact_self_hash, second_product.artifact_self_hash,
            "artifact self hash differed for seed {seed}"
        );
        assert_eq!(
            first_product.canonical_artifact_bytes, second_product.canonical_artifact_bytes,
            "canonical artifact bytes differed for seed {seed}"
        );
    }
}
