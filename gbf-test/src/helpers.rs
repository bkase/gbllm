//! Convenience re-exports for shared integration-test assertions.

pub use crate::fixtures::{
    assert_artifact_core_valid, assert_artifact_valid, assert_bytes_equal, assert_f32_slice_close,
    assert_tensor_close, assert_tensor_values_close,
};

use sha2::{Digest, Sha256};

#[track_caller]
pub(crate) fn assert_fixture_hash(bytes: &[u8], expected_hex: &str, label: &str) {
    let actual = hex_sha256(bytes);
    assert_eq!(actual, expected_hex, "{label} fixture hash");
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write to string cannot fail");
    }
    out
}
