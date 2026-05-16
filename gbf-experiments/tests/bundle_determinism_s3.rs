#![cfg(feature = "s3")]

mod bundle_s3_support;

use bundle_s3_support::{export_product_from, frozen_toy_teacher};

#[test]
fn replay_pair_exports_are_byte_equal_for_all_s3_seeds() {
    for seed in 0..5 {
        let first = frozen_toy_teacher(seed);
        let second = frozen_toy_teacher(seed);

        let first_product = export_product_from(&first);
        let second_product = export_product_from(&second);

        assert_eq!(
            first_product.canonical_bundle_payload_sha, second_product.canonical_bundle_payload_sha,
            "canonical payload sha differed for seed {seed}"
        );
        assert_eq!(
            first_product.bundle_self_hash, second_product.bundle_self_hash,
            "bundle self hash differed for seed {seed}"
        );
        assert_eq!(
            first_product.canonical_bundle_bytes, second_product.canonical_bundle_bytes,
            "canonical bundle bytes differed for seed {seed}"
        );
    }
}
