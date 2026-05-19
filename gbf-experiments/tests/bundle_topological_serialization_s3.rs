#![cfg(feature = "s3")]

mod bundle_s3_support;

use bundle_s3_support::{export_product_from, frozen_toy_teacher_with_order};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn graph_node_permutations_produce_identical_canonical_payload_sha(
        order in prop::sample::select(node_permutations())
    ) {
        let canonical = export_product_from(&frozen_toy_teacher_with_order(0, vec![0, 1, 2, 3]));
        let permuted = export_product_from(&frozen_toy_teacher_with_order(0, order));

        prop_assert_eq!(
            permuted.canonical_bundle_payload_sha,
            canonical.canonical_bundle_payload_sha
        );
        prop_assert_eq!(permuted.canonical_bundle_bytes, canonical.canonical_bundle_bytes);
    }
}

fn node_permutations() -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = vec![a, b, c, d];
                    if all_unique(&order) {
                        out.push(order);
                    }
                }
            }
        }
    }
    out
}

fn all_unique(values: &[usize]) -> bool {
    values
        .iter()
        .enumerate()
        .all(|(index, value)| !values[..index].contains(value))
}
