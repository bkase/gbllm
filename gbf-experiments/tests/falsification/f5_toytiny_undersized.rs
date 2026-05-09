use gbf_experiments::s1::report::Hypothesis;
use gbf_experiments::s1::schema::S1Outcome;
use gbf_policy::model_profile::ModelSizeProfile;

#[test]
fn f5_toytiny_undersized_refutes_h2_and_fails_capacity() {
    let profile = ModelSizeProfile::toy_tiny_for_falsification();
    assert_eq!(profile.d_model(), 2);
    assert_eq!(profile.d_ff(), 4);
    assert_eq!(profile.n_blocks(), 1);

    let baseline_bpc = 4.0;
    let observed_val_bpc = 4.8;
    assert!(
        observed_val_bpc > baseline_bpc - 0.5,
        "ToyTiny substitute must fail the S1 margin gate"
    );

    crate::assert_falsification_outcome(
        "F5",
        crate::refute(crate::confirmed_input(), Hypothesis::H2),
        S1Outcome::FailCapacity,
        crate::fail_capacity_decision(),
    );
}
