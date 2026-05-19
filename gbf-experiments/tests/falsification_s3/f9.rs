use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F9PhaseSchedulerWrongRamp;
    crate::run_logged(kind, || {
        let evidence = falsify::f9_phase_scheduler_wrong_ramp();
        assert_eq!(evidence.phase_log_event_kind, "student_freeze");
        assert_ne!(
            evidence.observed_expert_qat_ramp,
            evidence.expected_expert_qat_ramp
        );
        assert!(evidence.phase_c_distill_loss_histogram_empty);
        FalsificationCaseResult::new(
            kind,
            format!(
                "H7 Refuted: observed ramp {:?}, distill histogram empty={}",
                evidence.observed_expert_qat_ramp, evidence.phase_c_distill_loss_histogram_empty
            ),
            evidence.h7_refuted,
        )
    })
}

#[test]
fn f9_broken_s3_wrong_phase_ramp_refutes_h7() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H7]);
    crate::assert_case(result);
}
