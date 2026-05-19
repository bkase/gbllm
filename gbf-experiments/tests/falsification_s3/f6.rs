use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F6TiedEmbeddingExportSplit;
    crate::run_logged(kind, || {
        let evidence = falsify::f6_tied_embedding_export_split();
        assert!(!evidence.classifier_alias_preserved);
        assert!(evidence.split_payload_bytes > evidence.tied_payload_bytes);
        FalsificationCaseResult::new(
            kind,
            format!(
                "H5 Refuted: split payload {} > tied payload {}",
                evidence.split_payload_bytes, evidence.tied_payload_bytes
            ),
            evidence.h5_refuted,
        )
    })
}

#[test]
fn f6_broken_s3_tied_embedding_split_refutes_h5() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H5]);
    crate::assert_case(result);
}
