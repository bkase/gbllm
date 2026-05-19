use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F1CharsetV1LossyNormalization;
    crate::run_logged(kind, || {
        let evidence = falsify::f1_charset_v1_lossy_normalization();
        assert!(evidence.canonical_preserved_case);
        assert_ne!(
            evidence.canonical_train_post_sha256,
            evidence.lossy_train_post_sha256
        );
        FalsificationCaseResult::new(
            kind,
            format!(
                "H1 Refuted: canonical {} != lowercased {}",
                evidence.canonical_train_post_sha256, evidence.lossy_train_post_sha256
            ),
            evidence.h1_refuted,
        )
    })
}

#[test]
fn f1_broken_s3_charset_lossy_normalization_refutes_h1() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H1]);
    crate::assert_case(result);
}
