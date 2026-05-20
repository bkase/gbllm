#![cfg(any(
    feature = "s5-falsify-1",
    feature = "s5-falsify-2",
    feature = "s5-falsify-3",
    feature = "s5-falsify-4",
    feature = "s5-falsify-5",
    feature = "s5-falsify-6",
    feature = "s5-falsify-7",
    feature = "s5-falsify-8",
    feature = "s5-falsify-9",
    feature = "s5-falsify-10",
    feature = "s5-falsify-11",
    feature = "s5-falsify-12",
    feature = "s5-falsify-13",
    feature = "s5-falsify-14",
    feature = "s5-falsify-15"
))]

use gbf_experiments::s5::falsify::{
    S5_FALSIFICATION_CASE_COUNT, S5FalsificationCase, active_s5_falsification_case,
    run_active_s5_falsification_case, run_s5_falsification_case,
};

#[test]
fn active_s5_falsify_feature_refutes_its_target() {
    let active = active_s5_falsification_case().expect("one s5-falsify-N feature is active");
    let result = run_active_s5_falsification_case().expect("active case runs");

    assert_eq!(result.feature, active.feature_name());
    assert!(
        result.matches_expected,
        "{} did not refute its target: {result:#?}",
        result.feature
    );
}

#[test]
fn s5_falsification_case_table_has_fifteen_refuting_entries() {
    assert_eq!(S5FalsificationCase::ALL.len(), S5_FALSIFICATION_CASE_COUNT);
    for case in S5FalsificationCase::ALL {
        let result = run_s5_falsification_case(case);
        assert!(
            result.matches_expected,
            "{} did not refute as expected: {result:#?}",
            case.feature_name()
        );
    }
}
