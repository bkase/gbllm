#![cfg(feature = "s3-schemas")]

#[path = "bundle_s3_support/mod.rs"]
mod bundle_s3_support;

use gbf_artifact::ReferenceModelBundle;
use insta::assert_snapshot;

#[test]
fn bundle_canonical_round_trips_with_redacted_shape_snapshot() {
    let bundle = bundle_s3_support::toy_bundle();
    let canonical_bytes = bundle.canonical_bytes();
    let decoded: ReferenceModelBundle =
        serde_json::from_slice(&canonical_bytes).expect("canonical bundle json decodes");

    assert_eq!(decoded.canonical_bytes(), canonical_bytes);
    assert!(decoded.self_hash_round_trips());
    assert_snapshot!(
        "bundle_canonical_redacted_shape_s3",
        serde_json::to_string_pretty(&bundle_s3_support::redacted_canonical_summary(&decoded))
            .expect("redacted summary serializes")
    );
    assert_snapshot!(
        "bundle_canonical_full_bytes_hex_s3",
        hex_bytes(&canonical_bytes)
    );
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}
