#![cfg(feature = "s3-schemas")]

#[path = "v0_success_s3_support/mod.rs"]
mod v0_success_s3_support;

#[test]
fn v0_success_canonical_s3() {
    let manifest = v0_success_s3_support::load_v0_success();
    let canonical = String::from_utf8(
        manifest
            .canonical_bytes()
            .expect("v0_success canonical bytes encode"),
    )
    .expect("canonical JSON is UTF-8");

    insta::assert_snapshot!(canonical);
}
