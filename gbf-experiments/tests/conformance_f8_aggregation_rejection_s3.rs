#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use conformance_s3_support::{
    canonical_bytes, fixture_agreement_product, fixture_envelope_with_product,
};
use gbf_artifact::{AggregationKind, ConformanceError as ArtifactConformanceError};

#[test]
fn conformance_f8_aggregation_rejection_s3() {
    let mut agreement = fixture_agreement_product();
    let target = agreement
        .records
        .iter()
        .position(|record| {
            record
                .train_vs_bundle_max_abs_diff
                .or(record.train_vs_artifact_max_abs_diff)
                .is_some()
        })
        .expect("fixture has max-diff record");
    let prompt_id = agreement.records[target].prompt_id.clone();
    agreement.records[target].aggregation_kind = AggregationKind::PromptWideSoftmaxForbidden;
    let envelope =
        fixture_envelope_with_product(agreement).expect("forbidden marker can be carried");

    let error = canonical_bytes(&envelope)
        .expect_err("canonical conformance write rejects forbidden aggregation");
    match error {
        ArtifactConformanceError::PromptWideSoftmaxAggregation {
            seed,
            prompt_id: observed_prompt_id,
            metric_id,
        } => {
            assert_eq!(seed, 0);
            assert_eq!(observed_prompt_id, prompt_id);
            assert!(metric_id.as_str().contains("max_abs_logit_diff"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let valid = conformance_s3_support::fixture_envelope();
    canonical_bytes(&valid).expect("per-token/per-vocab-row aggregation canonicalizes");
}
