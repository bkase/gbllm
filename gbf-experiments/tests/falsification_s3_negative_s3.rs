#![cfg(feature = "s3")]

use gbf_experiments::s3::schema::{HypothesisStatus, S3Hypothesis, S3VerifierBundle};

#[test]
fn falsification_s3_normal_path_confirms_without_broken_substitutes() {
    let bundle = S3VerifierBundle::closure_candidate();

    assert!(bundle.falsification_s3_passed);
    for hypothesis in S3Hypothesis::ALL {
        assert_eq!(bundle.status(hypothesis), HypothesisStatus::Confirmed);
    }
}
