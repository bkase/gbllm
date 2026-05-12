use gbf_experiments::s2::falsify::{
    BrokenKind, FalsificationCaseResult, zero_short_circuit_verifier_evidence,
};

pub fn run() -> FalsificationCaseResult {
    crate::run_logged(BrokenKind::F5ZeroLossShortCircuit, || {
        let evidence = zero_short_circuit_verifier_evidence().expect("F5 verifier evidence runs");
        evidence.report.validate().expect("F5 H5 report validates");
        let subcheck = evidence
            .h5_4_fixture
            .diagnostic_subchecks
            .iter()
            .find(|subcheck| subcheck.name == "lambda_zero_raw_honesty_at_zero_weight")
            .expect("H5.4b subcheck present");
        let caught = evidence.h5_4_refuted
            && evidence.h5_refuted
            && !subcheck.passed
            && !subcheck.raw_loss_computed
            && subcheck.weighted_loss_value.is_none();
        FalsificationCaseResult::new(
            BrokenKind::F5ZeroLossShortCircuit,
            format!(
                "H5.4b diagnostic-runner fallback Refuted h5_4_refuted={} h5_refuted={} passed={} raw_loss_computed={} weighted_loss_value={:?}",
                evidence.h5_4_refuted,
                evidence.h5_refuted,
                subcheck.passed,
                subcheck.raw_loss_computed,
                subcheck.weighted_loss_value
            ),
            caught,
        )
    })
}

#[test]
fn falsify_f5_zero_loss_diagnostic_runner_fallback() {
    crate::assert_case(run());
}
