use gbf_experiments::s2::falsify::{BrokenKind, FalsificationCaseResult};
use gbf_experiments::s2::schema::{HypothesisStatus, S2BuildKind};
use gbf_experiments::s2::verifiers::verify_h1;

pub fn run() -> FalsificationCaseResult {
    crate::run_logged(BrokenKind::F2PhaseDUnfreezesTeacher, || {
        let fixture = gbf_experiments::s2::falsify::h1_fixture_for_active_broken_kind()
            .expect("F2 H1 fixture builds");
        let verdict = verify_h1(
            &fixture.phase_log,
            &fixture.entries,
            S2BuildKind::s2_ternary_full,
        );
        let caught = verdict.status == HypothesisStatus::Refuted
            && verdict
                .hits
                .iter()
                .any(|hit| hit.check_name == "teacher_freeze" && hit.step == Some(8_001));
        FalsificationCaseResult::new(
            BrokenKind::F2PhaseDUnfreezesTeacher,
            format!("H1 {:?} hits={:?}", verdict.status, verdict.hits),
            caught,
        )
    })
}

#[test]
fn falsify_f2_phase_d_unfreezes_teacher() {
    crate::assert_case(run());
}
