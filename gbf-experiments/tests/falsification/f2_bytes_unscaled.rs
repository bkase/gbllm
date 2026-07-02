use gbf_experiments::s7::baseline_match::canonical_s7_matched_bytes_pin;
use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f2_bytes_unscaled, run_s7_falsification_evidence,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};

#[test]
fn f2_bytes_unscaled_refutes_h3() {
    let pin = canonical_s7_matched_bytes_pin().expect("canonical S7 matched-bytes pin");
    assert_ne!(
        pin.d_ff_dense_resolved, 128,
        "the dense matched-bytes width must not silently reuse MoE d_ff"
    );
    assert!(
        pin.b_deployed_total_moe
            .abs_diff(pin.b_deployed_total_dense)
            <= pin.tolerance_bytes,
        "canonical pin should satisfy D6 matched-byte tolerance"
    );

    let evidence =
        f2_bytes_unscaled::broken_substitute_with_expected_dense(pin.d_ff_dense_resolved);
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::BytesUnscaled {
            moe_d_ff: 128,
            dense_d_ff_observed: 128,
            dense_d_ff_expected
        } if dense_d_ff_expected == pin.d_ff_dense_resolved
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::BytesUnscaled,
        S7Outcome::FailParity,
        S7Decision::ProceedToS8DenseOnly,
        || run_s7_falsification_evidence(evidence),
    );
}
