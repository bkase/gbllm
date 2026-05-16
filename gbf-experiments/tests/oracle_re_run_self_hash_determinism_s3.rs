#![cfg(feature = "s3")]

use std::collections::BTreeSet;

use gbf_experiments::s3::oracle_re_run::s3_oracle_re_run;

#[test]
fn oracle_re_run_self_hash_determinism_s3() {
    let mut hashes = BTreeSet::new();
    let mut canonical_payloads = BTreeSet::new();

    for _ in 0..10 {
        let report = s3_oracle_re_run().expect("S3 oracle re-run succeeds");
        report.validate_closure().expect("closure validates");
        hashes.insert(report.oracle_re_run_self_hash);
        canonical_payloads.insert(
            report
                .canonical_json_bytes()
                .expect("self-hash preimage canonicalizes"),
        );
    }

    assert_eq!(hashes.len(), 1);
    assert_eq!(canonical_payloads.len(), 1);
}
