//! S3 fallback artifact oracle backend.

use super::{
    ArtifactBackendKind, ArtifactOracle, ArtifactOracleInputs, ArtifactOracleProduct, OracleError,
    S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD, evaluate_with_backend_kind,
};

/// Fallback artifact oracle pending the richer F-C2 real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct S3ArtifactFallback;

impl S3ArtifactFallback {
    /// Real owner bead recorded by fallback reports.
    pub const REAL_OWNER_BEAD: &'static str = S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD;
}

impl ArtifactOracle for S3ArtifactFallback {
    fn evaluate(
        &self,
        inputs: ArtifactOracleInputs<'_>,
    ) -> Result<ArtifactOracleProduct, OracleError> {
        evaluate_with_backend_kind(
            inputs,
            ArtifactBackendKind::Fallback,
            Some(Self::REAL_OWNER_BEAD),
        )
    }
}
