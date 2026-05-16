#![cfg(feature = "s3")]

use gbf_experiments::s3::oracle_re_run::{
    OracleReRunReport, S3_ORACLE_RE_RUN_SCHEMA, s3_oracle_re_run,
};
use gbf_foundation::CanonicalJson;
use serde_json::Value;

#[test]
fn oracle_re_run_canonical_s3() {
    let report = s3_oracle_re_run().expect("S3 oracle re-run succeeds");
    let bytes = CanonicalJson::to_vec(&report).expect("full report canonicalizes");
    let decoded: OracleReRunReport = serde_json::from_slice(&bytes).expect("round trip decodes");
    let decoded_bytes = CanonicalJson::to_vec(&decoded).expect("decoded report canonicalizes");

    assert_eq!(decoded, report);
    assert_eq!(decoded_bytes, bytes);
    decoded
        .validate_closure()
        .expect("decoded closure validates");

    let value: Value = serde_json::from_slice(&bytes).expect("canonical JSON parses");
    assert_eq!(
        value.get("schema").and_then(Value::as_str),
        Some(S3_ORACLE_RE_RUN_SCHEMA)
    );
    assert!(value.get("per_metric").and_then(Value::as_object).is_some());
}
