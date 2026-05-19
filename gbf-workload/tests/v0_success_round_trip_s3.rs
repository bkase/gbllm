#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

#[test]
fn v0_success_round_trip_s3() {
    let manifest = v0_success_s3_support::load_v0_success();
    let encoded = manifest
        .to_toml_string()
        .expect("v0_success manifest serializes to TOML");

    assert_eq!(encoded, v0_success_s3_support::fixture_text());
}
