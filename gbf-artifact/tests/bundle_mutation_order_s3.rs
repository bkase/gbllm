#![cfg(feature = "s3-schemas")]

#[path = "bundle_s3_support/mod.rs"]
mod bundle_s3_support;

use std::collections::HashMap;

use gbf_artifact::ReferenceModelBundle;

#[test]
fn bundle_canonicalization_is_stable_when_tensor_input_order_changes() {
    let bundle = bundle_s3_support::toy_bundle();
    for trial in 0..10 {
        let mut tensors = bundle.tensors.clone();
        let len = tensors.len();
        tensors.rotate_left(trial % len);
        if trial % 2 == 1 {
            tensors.reverse();
        }

        let mut transient = HashMap::new();
        for tensor in tensors {
            transient.insert(tensor.id.as_str().to_owned(), tensor);
        }

        let reordered = ReferenceModelBundle::new(
            bundle.manifest.clone(),
            bundle.numeric.clone(),
            bundle.lexical.clone(),
            bundle.model.clone(),
            bundle.program.clone(),
            transient.into_values().collect(),
            bundle.decode.clone(),
            bundle.tied_embedding_alias.clone(),
        )
        .expect("reordered tensor bundle is valid");

        assert_eq!(reordered.canonical_bytes(), bundle.canonical_bytes());
        assert_eq!(reordered.bundle_self_hash, bundle.bundle_self_hash);
    }
}
