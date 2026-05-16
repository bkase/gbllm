#![cfg(feature = "s3")]

mod common;
mod common_s3;
mod report_s3_support;

use gbf_artifact::LexicalSpec_v1;
use gbf_data::charset_v1::{CharsetInputs, s3_charset_v1};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s3::schema::{
    CharsetProductRecord, OracleFallbackTag, S3BuildKind, S3Decision, S3Outcome, S3VerifierBundle,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[test]
fn s3_build_kind_serializes_as_kebab_case_strings() {
    let bytes = S1CanonicalJson::to_vec(&S3BuildKind::s3_v0_success_real_oracle)
        .expect("canonical S3 JSON");

    assert_eq!(bytes, br#""s3-v0-success-real-oracle""#);
}

#[test]
fn s3_outcome_serializes_with_rfc_tags() {
    let bytes =
        S1CanonicalJson::to_vec(&S3Outcome::PassWithFallbackOracle).expect("canonical S3 JSON");

    assert_eq!(bytes, br#""Pass-with-fallback-oracle""#);
}

#[test]
fn s3_decision_uses_tagged_shape() {
    let decision = S3Decision::ProceedToS4WithDeferredClause;
    let bytes = S1CanonicalJson::to_vec(&decision).expect("canonical S3 JSON");

    assert_eq!(bytes, br#"{"kind":"ProceedToS4-with-deferred-clause"}"#);
}

#[test]
fn oracle_fallback_tag_preserves_named_fallback_variant() {
    let bytes = S1CanonicalJson::to_vec(&OracleFallbackTag::S3DenotationalFallback)
        .expect("canonical S3 JSON");

    assert_eq!(bytes, br#""S3DenotationalFallback""#);
}

#[test]
fn s3_verifier_bundle_round_trips_through_s1_canonical_json() {
    let mut bundle = S3VerifierBundle::closure_candidate();
    bundle
        .oracle_fallback_used
        .push(OracleFallbackTag::S3ArtifactFallback);

    assert_canonical_round_trip(&bundle);
}

#[test]
fn charset_product_record_round_trips_through_s1_canonical_json() {
    let product = s3_charset_v1(CharsetInputs {
        raw_train_examples: vec![b"Train story one.".to_vec()],
        raw_val_examples: vec![b"Val story one.".to_vec()],
        spec: LexicalSpec_v1::pinned(),
    })
    .expect("charset product builds");
    let record = CharsetProductRecord::from(product);
    let bytes = S1CanonicalJson::to_vec(&record).expect("canonical S3 JSON");
    let json = std::str::from_utf8(&bytes).expect("canonical JSON is UTF-8");

    assert!(json.contains(r#""schema":"s3_charset_v1.v1""#));
    assert!(json.contains(r#""charset_self_hash":"sha256:"#));
    assert_canonical_round_trip(&record);
}

#[test]
fn s3_report_front_matter_round_trips_through_s1_canonical_json() {
    let report = report_s3_support::pass_clean_report();

    assert_canonical_round_trip(&report.front_matter);
}

fn assert_canonical_round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = S1CanonicalJson::to_vec(value).expect("canonical S3 JSON");
    let decoded: T = serde_json::from_slice(&bytes).expect("round trip");
    let decoded_bytes = S1CanonicalJson::to_vec(&decoded).expect("canonical S3 JSON");

    assert_eq!(&decoded, value);
    assert_eq!(decoded_bytes, bytes);
}
