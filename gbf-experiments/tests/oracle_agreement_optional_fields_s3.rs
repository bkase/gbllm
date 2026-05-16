#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod oracle_agreement_s3_support;

use gbf_oracle::phase_surface_agreement::PhaseId;
use oracle_agreement_s3_support::run_default_agreement;

#[test]
fn oracle_agreement_optional_fields_s3() {
    let product = run_default_agreement();

    for record in &product.records {
        assert!(record.bundle_vs_artifact_max_abs_diff.is_some());
        assert!(record.bundle_vs_artifact_argmax_match.is_some());
        match record.phase {
            PhaseId::PhaseA => {
                assert!(record.train_vs_bundle_max_abs_diff.is_some());
                assert!(record.train_vs_bundle_argmax_match.is_some());
                assert!(record.train_vs_bundle_pass.is_some());
                assert!(record.train_vs_artifact_max_abs_diff.is_none());
                assert!(record.train_vs_artifact_argmax_match.is_none());
                assert!(record.train_vs_artifact_pass.is_none());
            }
            PhaseId::PhaseD => {
                assert!(record.train_vs_artifact_max_abs_diff.is_some());
                assert!(record.train_vs_artifact_argmax_match.is_some());
                assert!(record.train_vs_artifact_pass.is_some());
                assert!(record.train_vs_bundle_max_abs_diff.is_none());
                assert!(record.train_vs_bundle_argmax_match.is_none());
                assert!(record.train_vs_bundle_pass.is_none());
            }
        }
    }
}
