use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F2FiveGramSmoothingUniform;
    crate::run_logged(kind, || {
        let evidence = falsify::f2_five_gram_smoothing_uniform();
        assert!(evidence.bpc_delta > 1.0e-12);
        FalsificationCaseResult::new(
            kind,
            format!(
                "H2 Refuted: |uniform_bpc - kn5_fixture_bpc| = {:.12}",
                evidence.bpc_delta
            ),
            evidence.h2_refuted,
        )
    })
}

#[test]
fn f2_broken_s3_uniform_smoothing_refutes_h2() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H2]);
    crate::assert_case(result);
}
