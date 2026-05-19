#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

use gbf_oracle::denotational::SemanticCheckpoint;
use gbf_oracle::phase_surface_agreement::{AgreementRecordBuilder, PhaseId};
use proptest::prelude::*;

proptest! {
    #[test]
    fn phase_a_builder_preserves_optional_discipline(
        diff in 0.0f32..10.0,
        gap in 0.0f32..10.0,
        argmax in any::<bool>(),
        gap_argmax in any::<bool>(),
        passed in any::<bool>(),
    ) {
        let record = AgreementRecordBuilder::for_phase_a(
            "prompt-a",
            SemanticCheckpoint::PostLogits,
            3,
        )
        .with_train_vs_bundle(diff, argmax, passed)
        .with_bundle_vs_artifact(gap, gap_argmax)
        .build()
        .expect("phase A record builds");

        prop_assert_eq!(record.phase, PhaseId::PhaseA);
        prop_assert!(record.train_vs_bundle_max_abs_diff.is_some());
        prop_assert!(record.train_vs_bundle_argmax_match.is_some());
        prop_assert!(record.train_vs_bundle_pass.is_some());
        prop_assert!(record.train_vs_artifact_max_abs_diff.is_none());
        prop_assert!(record.train_vs_artifact_argmax_match.is_none());
        prop_assert!(record.train_vs_artifact_pass.is_none());
    }

    #[test]
    fn phase_d_builder_preserves_optional_discipline(
        diff in 0.0f32..10.0,
        gap in 0.0f32..10.0,
        argmax in any::<bool>(),
        gap_argmax in any::<bool>(),
        passed in any::<bool>(),
    ) {
        let record = AgreementRecordBuilder::for_phase_d(
            "prompt-d",
            SemanticCheckpoint::PostLogits,
            3,
        )
        .with_train_vs_artifact(diff, argmax, passed)
        .with_bundle_vs_artifact(gap, gap_argmax)
        .build()
        .expect("phase D record builds");

        prop_assert_eq!(record.phase, PhaseId::PhaseD);
        prop_assert!(record.train_vs_artifact_max_abs_diff.is_some());
        prop_assert!(record.train_vs_artifact_argmax_match.is_some());
        prop_assert!(record.train_vs_artifact_pass.is_some());
        prop_assert!(record.train_vs_bundle_max_abs_diff.is_none());
        prop_assert!(record.train_vs_bundle_argmax_match.is_none());
        prop_assert!(record.train_vs_bundle_pass.is_none());
    }
}
