#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod oracle_agreement_s3_support;

use gbf_oracle::phase_surface_agreement::{
    LiveObservationSourceKind, OracleFallbackTag, PhaseId, S3_LIVE_OBSERVATION_REAL_OWNER_BEAD,
};
use oracle_agreement_s3_support::{
    EXPECTED_FULL_RECORD_COUNT, run_default_agreement, write_product_if_requested,
};

#[test]
fn oracle_agreement_s3() {
    let product = run_default_agreement();
    write_product_if_requested(&product);

    assert_eq!(product.records.len(), EXPECTED_FULL_RECORD_COUNT);
    assert!(product.phase_a_pass);
    assert!(product.phase_d_pass);
    assert!(product.overall_pass);
    assert_eq!(
        product.live_observation_source.kind,
        LiveObservationSourceKind::OracleDerivedFixture
    );
    assert_eq!(
        product.live_observation_source.real_owner_bead.as_deref(),
        Some(S3_LIVE_OBSERVATION_REAL_OWNER_BEAD)
    );
    assert_eq!(
        product.fallback_used,
        vec![OracleFallbackTag::S3LiveObservationFixture]
    );

    let phase_a = product
        .records
        .iter()
        .filter(|record| record.phase == PhaseId::PhaseA)
        .collect::<Vec<_>>();
    assert!(phase_a.iter().all(|record| {
        record
            .train_vs_bundle_max_abs_diff
            .is_some_and(|diff| diff <= 4.0e-6)
            && record.train_vs_bundle_argmax_match == Some(true)
            && record.train_vs_artifact_max_abs_diff.is_none()
    }));

    let phase_d = product
        .records
        .iter()
        .filter(|record| record.phase == PhaseId::PhaseD)
        .collect::<Vec<_>>();
    assert!(phase_d.iter().all(|record| {
        record.train_vs_artifact_max_abs_diff == Some(0.0)
            && record.train_vs_artifact_argmax_match == Some(true)
            && record.train_vs_bundle_max_abs_diff.is_none()
    }));

    assert!(
        product.records.iter().any(|record| {
            record
                .bundle_vs_artifact_max_abs_diff
                .is_some_and(|diff| diff > 0.0)
        }),
        "fixture should retain a report-only bundle-vs-artifact quantization gap"
    );
}
