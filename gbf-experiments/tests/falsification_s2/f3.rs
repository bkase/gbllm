use gbf_experiments::s2::falsify::{
    BrokenKind, FalsificationCaseResult, distill_temperature_inverted_config,
    is_distill_temperature_rejection,
};

pub fn run() -> FalsificationCaseResult {
    crate::run_logged(BrokenKind::F3DistillTempInverted, || {
        let error = distill_temperature_inverted_config()
            .validate()
            .expect_err("F3 config must be rejected");
        let caught = is_distill_temperature_rejection(&error);
        FalsificationCaseResult::new(
            BrokenKind::F3DistillTempInverted,
            format!("config-validator rejected: {error}"),
            caught,
        )
    })
}

#[test]
fn falsify_f3_distill_temperature_inverted() {
    crate::assert_case(run());
}
