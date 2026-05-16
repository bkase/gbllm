#![cfg(feature = "s3")]

mod artifact_s3_support;

use artifact_s3_support::{export_product_from, frozen_with_view, id};
use gbf_artifact::ClassifierView;

#[test]
fn artifact_tied_embedding_s3() {
    for view in [ClassifierView::SameTensor, ClassifierView::TransposedView] {
        let frozen = frozen_with_view(0, view);
        let product = export_product_from(&frozen);
        let alias = product
            .artifact
            .core
            .tied_embedding_alias
            .as_ref()
            .expect("tied alias present");

        assert_eq!(alias.embedding_canonical_id, id("tensor.embedding"));
        assert_eq!(alias.classifier_canonical_id, id("tensor.embedding"));
        assert!(alias.shared);
        assert_eq!(alias.classifier_view, view);
        assert!(product.artifact_validation.tied_embedding_alias_preserved);
        assert_eq!(
            product
                .metadata
                .tied_embedding_alias
                .as_ref()
                .expect("metadata alias")
                .classifier_view,
            view
        );
    }
}
