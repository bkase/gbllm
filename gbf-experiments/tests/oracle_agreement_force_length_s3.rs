#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod oracle_agreement_s3_support;

use gbf_experiments::s3::oracle::S3OracleAgreementError;
use gbf_oracle::denotational::SemanticCheckpoint;
use gbf_oracle::phase_surface_agreement::PhaseId;
use oracle_agreement_s3_support::{
    fixture_policy, force_length_eos_product, run_default_agreement_with_policy,
};

#[test]
fn oracle_agreement_force_length_s3() {
    let product = force_length_eos_product();

    assert_eq!(product.records.len(), 2 * 16);
    assert!(product.overall_pass);
    assert!(product.records.iter().any(|record| {
        record.phase == PhaseId::PhaseA
            && record.checkpoint == SemanticCheckpoint::PostDecode
            && record.step == 5
            && record.train_vs_bundle_argmax_match == Some(true)
    }));
    assert!(product.records.iter().any(|record| {
        record.phase == PhaseId::PhaseD
            && record.checkpoint == SemanticCheckpoint::PostDecode
            && record.step == 15
            && record.train_vs_artifact_argmax_match == Some(true)
    }));
}

#[test]
fn oracle_agreement_runner_rejects_stop_on_eos_s3() {
    let mut policy = fixture_policy();
    policy.agreement_trace.stop_on_eos = true;

    let error = run_default_agreement_with_policy(&policy)
        .expect_err("runner must reject stop_on_eos=true for forced agreement traces");

    assert!(matches!(error, S3OracleAgreementError::StopOnEosEnabled));
}
