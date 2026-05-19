#![cfg(feature = "s3")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{
    BaselineError, KnBaselineInputs, KnConditionalModel, KnEffectiveCounts, fit_discounts,
    fit_discounts_for_order, s3_fit_kn5,
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn valid_count_of_counts_fit_bounded_discounts((n1, n2, n3, n4) in valid_count_of_counts()) {
        let coc = BTreeMap::from([(1, n1), (2, n2), (3, n3), (4, n4)]);
        let discounts = fit_discounts_for_order(5, &coc).expect("valid D-rule counts fit");

        prop_assert!(discounts.y_k > 0.0 && discounts.y_k < 1.0);
        prop_assert!((0.0..=1.0).contains(&discounts.d_1));
        prop_assert!((0.0..=2.0).contains(&discounts.d_2));
        prop_assert!((0.0..=3.0).contains(&discounts.d_3p));
    }

    #[test]
    fn invalid_count_of_counts_n3_zero_reports_missing_3(n1 in 1_u64..100, n2 in 1_u64..100) {
        let coc = BTreeMap::from([(1, n1), (2, n2), (3, 0)]);
        let error = fit_discounts_for_order(4, &coc).expect_err("n3=0 is invalid");

        match error {
            BaselineError::DiscountPreconditionsViolated { order, missing } => {
                prop_assert_eq!(order, 4);
                prop_assert_eq!(missing, vec![3]);
            }
            other => prop_assert!(false, "unexpected error: {other:?}"),
        }
    }

    #[test]
    fn kn5_probability_mass_is_normalized_for_arbitrary_context(context in proptest::collection::vec(prop_oneof![
        Just(26_u8), Just(27_u8), Just(28_u8), Just(29_u8), Just(30_u8), Just(31_u8), Just(75_u8)
    ], 4)) {
        let (model, _) = oracle_model();
        let mass = model.probability_mass(5, &context).expect("probability mass computes");

        prop_assert!(
            (mass - 1.0).abs() <= 1.0e-9,
            "context={context:?} mass={mass}"
        );
    }
}

#[test]
fn invalid_count_of_counts_reports_discount_out_of_bounds() {
    let coc = BTreeMap::from([(1, 10), (2, 1), (3, 100), (4, 1)]);
    let error = fit_discounts(&coc).expect_err("d_2 falls below its lower bound");

    match error {
        BaselineError::DiscountOutOfBounds {
            order,
            field,
            value,
            lower,
            upper,
        } => {
            assert_eq!(order, 0);
            assert_eq!(field, "d_2");
            assert!(value < lower, "d_2={value} should be below {lower}");
            assert_eq!(lower, 0.0);
            assert_eq!(upper, 2.0);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn valid_count_of_counts() -> impl Strategy<Value = (u64, u64, u64, u64)> {
    (1_u64..100).prop_flat_map(|n1| {
        (Just(n1), n1..=3 * n1).prop_flat_map(|(n1, n2)| {
            (Just(n1), Just(n2), 1_u64..=n2)
                .prop_flat_map(|(n1, n2, n3)| (Just(n1), Just(n2), Just(n3), 1_u64..=n3))
        })
    })
}

fn oracle_model() -> (
    KnConditionalModel,
    gbf_experiments::s3::baseline::KnBaselineProduct,
) {
    let root = workspace_root().join("fixtures/baselines/kn_oracle");
    let train = normalize_file(&root.join("train.bytes"));
    let val = normalize_file(&root.join("eval.bytes"));
    let product = s3_fit_kn5(KnBaselineInputs {
        train_post: train.clone(),
        val_post: val,
    })
    .expect("oracle baseline fits");
    let counts = KnEffectiveCounts::fit(&train).expect("oracle counts fit");
    (
        KnConditionalModel::new(counts, product.discounts.clone()),
        product,
    )
}

fn normalize_file(path: &Path) -> gbf_artifact::TextCharSeq {
    let bytes = std::fs::read(path).expect("fixture bytes read");
    let normalized = normalize_raw(&bytes).expect("fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
