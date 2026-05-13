use gbf_experiments::s2::falsify::{
    BrokenKind, FalsificationCaseResult, linearstate_grad_dead_verifier_evidence,
};

pub fn run() -> FalsificationCaseResult {
    crate::run_logged(BrokenKind::F6LinearStateGradDead, || {
        let evidence =
            linearstate_grad_dead_verifier_evidence().expect("F6 verifier evidence runs");
        evidence
            .run
            .report
            .validate()
            .expect("F6 H6 report validates");
        let caught = evidence.h6_refuted
            && evidence.recurrence_grad_norm == Some(0.0)
            && evidence.run.run_1_bytes == evidence.run.run_2_bytes;
        FalsificationCaseResult::new(
            BrokenKind::F6LinearStateGradDead,
            format!(
                "H6 structural smoke fallback Refuted h6_refuted={} recurrence_grad={:?} deterministic_bytes={}",
                evidence.h6_refuted,
                evidence.recurrence_grad_norm,
                evidence.run.run_1_bytes == evidence.run.run_2_bytes
            ),
            caught,
        )
    })
}

#[test]
fn falsify_f6_structural_dead_recurrence_fallback() {
    crate::assert_case(run());
}
