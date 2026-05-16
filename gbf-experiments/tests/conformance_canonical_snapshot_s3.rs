#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod conformance_s3_support;

use conformance_s3_support::{canonical_bytes, fixture_envelope};
use gbf_foundation::sha256;

#[test]
fn conformance_canonical_snapshot_s3() {
    let envelope = fixture_envelope();
    let bytes = canonical_bytes(&envelope).expect("canonical conformance bytes encode");
    let hash = sha256(&bytes);

    assert_eq!(bytes.first(), Some(&b'{'));
    assert_eq!(
        hash.to_string(),
        "sha256:77cf356354df3105764de40bb593178a4305e1cc7eed5597fdc59a73dd6b75cb"
    );
}
