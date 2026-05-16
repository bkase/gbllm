#![cfg(feature = "s3-schemas")]

#[path = "bundle_s3_support/mod.rs"]
mod bundle_s3_support;

use gbf_artifact::ReferenceModelBundle;

#[test]
fn bundle_self_hash_is_deterministic_and_round_trips() {
    let bundle = bundle_s3_support::toy_bundle();
    let expected = bundle.bundle_self_hash;

    for _ in 0..10 {
        assert_eq!(bundle.compute_self_hash(), expected);
    }

    let decoded: ReferenceModelBundle =
        serde_json::from_slice(&bundle.canonical_bytes()).expect("canonical bundle json decodes");
    assert_eq!(decoded.compute_self_hash(), expected);
    assert!(decoded.self_hash_round_trips());
}
