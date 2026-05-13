use gbf_experiments::s2::falsify::{BrokenKind, FalsificationCaseResult};
use gbf_experiments::s2::schema::{HypothesisStatus, S2BuildKind};
use gbf_experiments::s2::verifiers::verify_h1;

pub fn run() -> FalsificationCaseResult {
    crate::run_logged(BrokenKind::F1PhaseBSkipsTernary, || {
        let fixture = gbf_experiments::s2::falsify::h1_fixture_for_active_broken_kind()
            .expect("F1 H1 fixture builds");
        let verdict = verify_h1(
            &fixture.phase_log,
            &fixture.entries,
            S2BuildKind::s2_ternary_full,
        );
        let caught = verdict.status == HypothesisStatus::Refuted
            && verdict
                .hits
                .iter()
                .any(|hit| hit.check_name == "d2_hardness_sequence" && hit.step == Some(6_001));
        FalsificationCaseResult::new(
            BrokenKind::F1PhaseBSkipsTernary,
            format!("H1 {:?} hits={:?}", verdict.status, verdict.hits),
            caught,
        )
    })
}

#[test]
fn falsify_f1_phase_b_skips_ternary() {
    crate::assert_case(run());
}
