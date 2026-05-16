#![cfg(feature = "s3")]

mod bundle_s3_support;

use gbf_artifact::TextCharSeq;
use gbf_experiments::s3::bundle::PHASE_A_LOGIT_TOLERANCE;
use gbf_experiments::s3::score::Evaluator;
use gbf_oracle::scorers::ReferenceScorer;
use gbf_train::teacher::DenseTeacherModel;

#[test]
fn reference_scorer_matches_live_toy0_teacher_logits() {
    let teacher = bundle_s3_support::ToyBundleTeacher::new(17);
    let bundle = bundle_s3_support::export_product_from(
        &gbf_train::teacher::freeze_teacher(&teacher).expect("teacher freezes"),
    )
    .bundle;
    let scorer = ReferenceScorer::new(&bundle);
    let prefix = TextCharSeq::new(vec![10, 11, 12]).unwrap();

    let observed = scorer.forward(prefix.as_slice(), 13).logits;
    let expected = teacher
        .forward_no_grad(prefix)
        .expect("toy teacher forward succeeds");

    assert_eq!(observed.len(), expected.len());
    for (observed, expected) in observed.iter().zip(expected) {
        assert!(
            (*observed - expected).abs() <= PHASE_A_LOGIT_TOLERANCE,
            "observed={observed} expected={expected}"
        );
    }
}
