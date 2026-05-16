#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

use gbf_artifact::AggregationKind;
use gbf_oracle::denotational::SemanticCheckpoint;
use gbf_oracle::phase_surface_agreement::{AgreementError, AgreementRecordBuilder};

#[test]
fn oracle_agreement_aggregation_s3() {
    let error = AgreementRecordBuilder::for_phase_a_with_seed(
        0,
        "prompt-a",
        SemanticCheckpoint::PostLogits,
        0,
        AggregationKind::PromptWideSoftmaxForbidden,
    )
    .with_train_vs_bundle(0.0, true, true)
    .build()
    .expect_err("prompt-wide aggregation must be rejected");

    assert!(matches!(error, AgreementError::InvalidAggregation { .. }));
}
