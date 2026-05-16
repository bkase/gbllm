use gbf_artifact::{ClassifierView, TiedEmbeddingAlias};

#[test]
fn tied_alias_classifier_view_round_trips_for_same_tensor_and_transposed_view() {
    for classifier_view in [ClassifierView::SameTensor, ClassifierView::TransposedView] {
        let alias = TiedEmbeddingAlias::new(
            gbf_artifact::CanonicalTensorId::new("tensor.embedding").unwrap(),
            gbf_artifact::CanonicalTensorId::new("tensor.embedding").unwrap(),
            true,
            classifier_view,
        );

        let encoded = serde_json::to_string(&alias).expect("tied alias serializes");
        let decoded: TiedEmbeddingAlias =
            serde_json::from_str(&encoded).expect("tied alias decodes");

        assert_eq!(decoded, alias);
        assert_eq!(decoded.classifier_view, classifier_view);
        assert_eq!(
            serde_json::to_value(&alias).expect("tied alias serializes"),
            serde_json::json!({
                "embedding_canonical_id": "tensor.embedding",
                "classifier_canonical_id": "tensor.embedding",
                "shared": true,
                "classifier_view": classifier_view,
            })
        );
    }
}
