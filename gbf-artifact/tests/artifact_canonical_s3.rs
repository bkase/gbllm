mod artifact_b5_support;

use artifact_b5_support::artifact_core;

#[test]
fn artifact_core_canonical_bytes_are_pinned_for_toy0() {
    let core = artifact_core();
    let canonical = String::from_utf8(core.canonical_bytes().expect("canonical artifact core"))
        .expect("canonical bytes are utf8");

    insta::assert_snapshot!(canonical);
}
