//! Canonical byte encoder for `ReferenceModelBundle`.

use gbf_foundation::CanonicalJson;

use crate::bundle::ReferenceModelBundle;

/// Encode a reference bundle using the F-S3 canonical bundle writer.
///
/// The encoder normalizes bundle order-sensitive surfaces before writing:
/// tensors are sorted by ascending canonical tensor id and the reference program
/// graph is re-canonicalized into deterministic topological order.
#[must_use]
pub fn canonical_bundle_bytes(bundle: &ReferenceModelBundle) -> Vec<u8> {
    CanonicalJson::to_vec(&bundle.canonicalized_for_encoding())
        .expect("reference bundle canonical encoding should not fail")
}
