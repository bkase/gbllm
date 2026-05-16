use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F8OracleSoftmaxOverConcatLogits;
    crate::run_logged(kind, || {
        let evidence = falsify::f8_oracle_softmax_over_concat_logits();
        assert_eq!(evidence.rejection_kind, "PromptWideSoftmaxAggregation");
        assert_eq!(evidence.prompt_id, "prompt-00");
        FalsificationCaseResult::new(
            kind,
            format!(
                "H4 Refuted: CanonicalConformanceWrite rejected {}",
                evidence.rejection_kind
            ),
            evidence.h4_refuted,
        )
    })
}

#[test]
fn f8_broken_s3_prompt_wide_softmax_refutes_h4() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H4]);
    crate::assert_case(result);
}
