#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use conformance_s3_support::{canonical_bytes, fixture_envelope};

#[test]
fn conformance_self_hash_determinism_s3() {
    let baseline = fixture_envelope();
    let baseline_bytes = canonical_bytes(&baseline).expect("canonical bytes encode");

    for _ in 0..10 {
        let replay = fixture_envelope();
        assert_eq!(replay.conformance_self_hash, baseline.conformance_self_hash);
        assert_eq!(
            canonical_bytes(&replay).expect("canonical bytes encode"),
            baseline_bytes
        );
    }
}
