#![cfg(feature = "s3")]

mod bundle_s3_support;

use bundle_s3_support::export_product;
use serde_json::Value;

#[test]
fn canonical_bundle_json_excludes_nondeterministic_container_metadata() {
    let first = export_product(0);
    let second = export_product(0);

    assert_eq!(first.canonical_bundle_bytes, second.canonical_bundle_bytes);
    assert_eq!(
        first.canonical_bundle_payload_sha,
        second.canonical_bundle_payload_sha
    );

    let json: Value =
        serde_json::from_slice(&first.canonical_bundle_bytes).expect("canonical bundle parses");
    assert_no_forbidden_metadata(&json);
}

fn assert_no_forbidden_metadata(value: &Value) {
    const FORBIDDEN_KEYS: &[&str] = &[
        "build_duration",
        "created_at",
        "duration",
        "host",
        "host_path",
        "mtime",
        "timestamp",
        "wall_clock",
    ];

    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                assert!(
                    !FORBIDDEN_KEYS.contains(&key.as_str()),
                    "canonical bundle contains forbidden metadata key {key:?}"
                );
                assert_no_forbidden_metadata(nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                assert_no_forbidden_metadata(nested);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
