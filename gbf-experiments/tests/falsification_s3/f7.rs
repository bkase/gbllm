use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F7V0SuccessRepetitionCollapse;
    crate::run_logged(kind, || {
        let evidence = falsify::f7_v0_success_repetition_collapse();
        assert!(!evidence.q4_holds);
        assert!(evidence.max_consecutive_same_token > 8);
        FalsificationCaseResult::new(
            kind,
            format!(
                "H3 Refuted: max_consecutive_same_token={} and Q4_holds={}",
                evidence.max_consecutive_same_token, evidence.q4_holds
            ),
            evidence.h3_refuted,
        )
    })
}

#[test]
fn f7_broken_s3_repetition_collapse_refutes_h3() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H3]);
    crate::assert_case(result);
}
