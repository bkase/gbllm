#![cfg(feature = "s3")]

mod bundle_s3_support;

use bundle_s3_support::{expected_single_copy_payload_bytes, export_product, tensor_payload_bytes};

#[test]
fn tied_embedding_classifier_alias_is_encoded_without_duplicate_payload() {
    let product = export_product(0);
    let alias = product
        .bundle
        .tied_embedding_alias
        .as_ref()
        .expect("toy bundle exports tied embedding alias");

    assert!(alias.shared);
    assert_eq!(alias.embedding_canonical_id, alias.classifier_canonical_id);
    assert!(
        product
            .bundle
            .tensors
            .iter()
            .any(|tensor| tensor.id.as_str() == "tensor.embedding")
    );
    assert!(
        product
            .bundle
            .tensors
            .iter()
            .all(|tensor| tensor.id.as_str() != "tensor.classifier.weight")
    );
    assert_eq!(
        tensor_payload_bytes(&product.bundle),
        expected_single_copy_payload_bytes()
    );
}
