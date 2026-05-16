use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F4ArtifactOracleDroppedQuantResolve;
    crate::run_logged(kind, || {
        let evidence = falsify::f4_artifact_oracle_dropped_quant_resolve();
        assert!(evidence.max_abs_logit_diff > 0.0);
        FalsificationCaseResult::new(
            kind,
            format!(
                "H4+H6 Refuted: name resolver max_abs_diff={:.6}",
                evidence.max_abs_logit_diff
            ),
            evidence.h4_refuted && evidence.h6_refuted,
        )
    })
}

#[test]
fn f4_broken_s3_name_resolution_refutes_h4_and_h6() {
    let result = run();
    assert_eq!(
        result.target_hypotheses,
        vec![S3Hypothesis::H4, S3Hypothesis::H6]
    );
    crate::assert_case(result);
}
