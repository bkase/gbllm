#![cfg(feature = "s3")]

use gbf_artifact::TextCharSeq;
use gbf_experiments::s3::baseline::{BaselineError, KnBaselineInputs, s3_fit_kn5};

#[test]
fn s3_fit_kn5_returns_fail_baseline_when_order4_n3_is_zero() {
    let train = TextCharSeq::new(vec![
        3, 1, 3, 1, 4, 4, 1, 5, 4, 1, 4, 1, 4, 1, 0, 4, 4, 1, 4, 4,
    ])
    .expect("failure fixture uses valid text ids");
    let val = TextCharSeq::new(vec![1]).expect("validation fixture uses valid text ids");

    let error = s3_fit_kn5(KnBaselineInputs {
        train_post: train,
        val_post: val,
    })
    .expect_err("order 4 missing n3 aborts fitting");

    assert!(matches!(
        error,
        BaselineError::DiscountPreconditionsViolated { order: 4, missing }
            if missing == vec![3]
    ));
}
