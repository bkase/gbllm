use gbf_experiments::s3::falsify::{self, BrokenKind, FalsificationCaseResult};
use gbf_experiments::s3::schema::S3Hypothesis;

pub fn run() -> FalsificationCaseResult {
    let kind = BrokenKind::F5BundleExportNondeterministicMapIter;
    crate::run_logged(kind, || {
        let evidence = falsify::f5_bundle_export_nondeterministic_map_iter();
        assert_ne!(
            evidence.first_bundle_self_hash,
            evidence.second_bundle_self_hash
        );
        FalsificationCaseResult::new(
            kind,
            format!(
                "H5 Refuted: replay hashes {} and {} differ",
                evidence.first_bundle_self_hash, evidence.second_bundle_self_hash
            ),
            evidence.h5_refuted,
        )
    })
}

#[test]
fn f5_broken_s3_nondeterministic_map_iter_refutes_h5() {
    let result = run();
    assert_eq!(result.target_hypotheses, vec![S3Hypothesis::H5]);
    crate::assert_case(result);
}
