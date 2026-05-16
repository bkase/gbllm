#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

mod common;
mod v0_success_s3_support;

use common::assertions::{assert_canonical_json_byte_eq, assert_no_nondeterministic_field};
use gbf_experiments::s3::schema::{S3_V0_SUCCESS_SCHEMA, s3_v0_success_schema};
use serde_json::Value;
use v0_success_s3_support::passing_product;

#[test]
fn v0_success_canonical_s3() {
    let product = passing_product();
    let canonical = product.canonical_bytes().expect("product canonicalizes");
    let value: Value = serde_json::from_slice(&canonical).expect("canonical JSON parses");

    assert_eq!(
        value.get("schema").and_then(Value::as_str),
        Some(S3_V0_SUCCESS_SCHEMA)
    );
    assert_eq!(
        value.get("overall_pass").and_then(Value::as_bool),
        Some(true)
    );
    assert_no_nondeterministic_field(&value);

    let decoded: gbf_experiments::s3::workload::V0SuccessProduct =
        serde_json::from_slice(&canonical).expect("canonical product decodes");
    assert_eq!(decoded, product);
    assert_eq!(
        decoded
            .compute_self_hash()
            .expect("decoded self-hash computes"),
        product.v0_success_self_hash
    );

    for _ in 0..10 {
        let replay = passing_product();
        assert_eq!(replay.v0_success_self_hash, product.v0_success_self_hash);
        assert_canonical_json_byte_eq(
            &replay.canonical_bytes().expect("replay canonicalizes"),
            &canonical,
        );
    }

    let schema = s3_v0_success_schema();
    assert_eq!(
        schema.get("schema").and_then(Value::as_str),
        Some(S3_V0_SUCCESS_SCHEMA)
    );
    assert!(
        schema["fields"]
            .as_array()
            .expect("fields array")
            .contains(&Value::String("v0_success_self_hash".to_owned()))
    );
    assert!(
        schema["per_seed_fields"]
            .as_array()
            .expect("per-seed fields array")
            .contains(&Value::String("per_prompt_generation".to_owned()))
    );
}
