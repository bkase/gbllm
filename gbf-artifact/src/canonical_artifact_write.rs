//! Canonical byte encoder for `ModelArtifact`.

use gbf_foundation::CanonicalJson;

use crate::artifact::ModelArtifact;

/// Encode a model artifact using the F-S3 canonical artifact writer.
///
/// The encoder normalizes artifact order-sensitive surfaces before writing:
/// core tensors are sorted by ascending canonical tensor id and target-data
/// lowerings are sorted by their stable target/profile identifiers.
#[must_use]
pub fn canonical_artifact_bytes(artifact: &ModelArtifact) -> Vec<u8> {
    CanonicalJson::to_vec(&artifact.canonicalized_for_encoding())
        .expect("model artifact canonical encoding should not fail")
}
