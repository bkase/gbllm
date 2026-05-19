#![cfg(feature = "s3-schemas")]

#[path = "bundle_s3_support/mod.rs"]
mod bundle_s3_support;

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1000,
        .. ProptestConfig::default()
    })]

    #[test]
    fn topological_canonicalization_is_stable_for_node_permutations(seed in 0usize..1000) {
        let canonical = bundle_s3_support::toy_bundle().canonical_bytes();
        let permuted = bundle_s3_support::toy_bundle_with_nodes(
            bundle_s3_support::permute_nodes(bundle_s3_support::toy_nodes(), seed)
        );

        prop_assert_eq!(permuted.canonical_bytes(), canonical);
    }
}
