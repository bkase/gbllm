use gbf_experiments::s7::collapse_sweep::{
    CollapseSweepError, D11_PRODUCTION_LAMBDA_SWITCH, GuardrailVerdict, LambdaSwitchSweepRecord,
    RouterCollapseSweepReport, f8_constant_lambda_sweep_verdict,
};
use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f8_sweep_constant_lambda,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_foundation::Hash256;

#[test]
fn f8_sweep_constant_lambda_refutes_h6() {
    let production =
        LambdaSwitchSweepRecord::successful(D11_PRODUCTION_LAMBDA_SWITCH, 16_000, 1.02, 1.86, 1.02)
            .expect("production lambda record");

    assert_eq!(
        f8_constant_lambda_sweep_verdict(&[production]).expect("F8 verdict"),
        GuardrailVerdict::FailC
    );
    let err = RouterCollapseSweepReport::from_grid_records(
        0,
        Hash256::from_bytes([0x33; 32]),
        vec![D11_PRODUCTION_LAMBDA_SWITCH],
        vec![production],
    )
    .expect_err("constant-lambda grid is not a valid D11 report");
    assert!(matches!(
        err,
        CollapseSweepError::UnexpectedGridCount { .. }
    ));

    let evidence = f8_sweep_constant_lambda::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::SweepConstantLambda {
            grid_len: 1,
            production_only: true,
            fail_c: true,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::SweepConstantLambda,
        S7Outcome::FailRouterCollapseGuardrail,
        S7Decision::Investigate {
            reason: "sweep-grid-or-thresholds",
        },
        f8_sweep_constant_lambda::run,
    );
}
