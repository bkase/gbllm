#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

use gbf_oracle::denotational::SemanticCheckpoint;
use gbf_oracle::phase_surface_agreement::{AgreementRecordBuilder, PhaseId};

#[test]
fn oracle_agreement_builder_phase_safety_s3() {
    let phase_a =
        AgreementRecordBuilder::for_phase_a("prompt-a", SemanticCheckpoint::PostLogits, 0)
            .with_train_vs_bundle(0.0, true, true)
            .with_bundle_vs_artifact(0.25, false)
            .build()
            .expect("phase A record builds");
    assert_eq!(phase_a.phase, PhaseId::PhaseA);
    assert!(phase_a.train_vs_bundle_max_abs_diff.is_some());
    assert!(phase_a.train_vs_artifact_max_abs_diff.is_none());

    let phase_d =
        AgreementRecordBuilder::for_phase_d("prompt-d", SemanticCheckpoint::PostLogits, 0)
            .with_train_vs_artifact(0.0, true, true)
            .with_bundle_vs_artifact(0.25, false)
            .build()
            .expect("phase D record builds");
    assert_eq!(phase_d.phase, PhaseId::PhaseD);
    assert!(phase_d.train_vs_artifact_max_abs_diff.is_some());
    assert!(phase_d.train_vs_bundle_max_abs_diff.is_none());
}
