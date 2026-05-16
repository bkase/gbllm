#![cfg(all(feature = "s3", feature = "s3-oracle-fallback"))]

mod denotational_s3_support;

use denotational_s3_support::evaluate;
use gbf_oracle::denotational::{
    DenotationalBackendKind, DenotationalDeterminismClass, Observation,
    S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD, S3DenotationalFallback, SemanticCheckpoint,
};

#[test]
fn oracle_fallback_contract_s3() {
    let product = evaluate(S3DenotationalFallback);

    assert_eq!(product.backend_kind, DenotationalBackendKind::Fallback);
    assert_eq!(
        product.determinism_class,
        DenotationalDeterminismClass::BitExact
    );
    assert_eq!(
        product.real_owner_bead,
        Some(S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD)
    );
    assert_eq!(S3DenotationalFallback::REAL_OWNER_BEAD, "bd-1rcc");

    let rows =
        gbf_oracle::denotational::ReferenceObservationsCanonical::rows(&product.observations);
    assert_eq!(rows[0].prompt_id, "prompt-00");
    assert_eq!(rows[0].checkpoint, SemanticCheckpoint::PostEmbedding);

    let decoded = product
        .observations
        .iter()
        .find(|((prompt_id, checkpoint, step), _)| {
            prompt_id.as_str() == "prompt-00"
                && *checkpoint == SemanticCheckpoint::PostDecode
                && *step == 0
        })
        .map(|(_, observation)| observation)
        .expect("decode observation exists");
    assert!(
        matches!(decoded, Observation::PostDecode { token: 7 }),
        "fallback must evaluate the graph end-to-end"
    );
}
