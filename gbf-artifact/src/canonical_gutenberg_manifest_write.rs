//! Canonical write boundary for `gutenberg_manifest.v1`.

use crate::{GutenbergManifest, GutenbergManifestError};

/// Encoder for the `gutenberg_manifest.v1` artifact write boundary.
#[derive(Debug, Clone, Copy)]
pub struct CanonicalGutenbergManifestWrite;

impl CanonicalGutenbergManifestWrite {
    /// Encode a Gutenberg manifest after enforcing S4 write invariants.
    pub fn to_vec(manifest: &GutenbergManifest) -> Result<Vec<u8>, GutenbergManifestError> {
        manifest.validate_canonical_write()?;
        manifest.canonical_bytes_unchecked()
    }
}

/// Encode a Gutenberg manifest at the canonical write boundary.
pub fn canonical_gutenberg_manifest_bytes(
    manifest: &GutenbergManifest,
) -> Result<Vec<u8>, GutenbergManifestError> {
    CanonicalGutenbergManifestWrite::to_vec(manifest)
}
