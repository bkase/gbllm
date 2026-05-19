#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod oracle_agreement_s3_support;

use oracle_agreement_s3_support::run_default_agreement;

#[test]
fn oracle_agreement_canonical_s3() {
    let first = run_default_agreement();
    let first_bytes = first
        .canonical_json_bytes()
        .expect("agreement product canonicalizes");

    for _ in 0..10 {
        let replay = run_default_agreement();
        assert_eq!(replay.agreement_self_hash, first.agreement_self_hash);
        assert_eq!(
            replay
                .canonical_json_bytes()
                .expect("agreement product canonicalizes"),
            first_bytes
        );
    }
}
