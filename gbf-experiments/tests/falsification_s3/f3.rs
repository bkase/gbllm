use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F3ModelEmitsInvalidCharset;
    crate::run_logged(kind, || {
        let evidence = falsify::f3_model_emits_invalid_charset();
        assert!(evidence.rejected_by_text_char_seq);
        assert!(!evidence.q3_holds);
        FalsificationCaseResult::new(
            kind,
            format!(
                "H3 Refuted: rejected ids {:?} and Q3_holds={}",
                evidence.rejected_ids, evidence.q3_holds
            ),
            evidence.h3_refuted,
        )
    })
}

#[test]
fn f3_broken_s3_invalid_charset_generation_refutes_h3() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H3]);
    crate::assert_case(result);
}
