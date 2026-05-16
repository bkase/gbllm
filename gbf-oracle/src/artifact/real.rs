//! Real S3 artifact oracle backend.

use super::{
    ArtifactBackendKind, ArtifactOracle, ArtifactOracleInputs, ArtifactOracleProduct, OracleError,
    evaluate_with_backend_kind,
};

/// Real artifact oracle for S3 model artifacts.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealArtifactOracle;

impl ArtifactOracle for RealArtifactOracle {
    fn evaluate(
        &self,
        inputs: ArtifactOracleInputs<'_>,
    ) -> Result<ArtifactOracleProduct, OracleError> {
        evaluate_with_backend_kind(inputs, ArtifactBackendKind::Real, None)
    }
}
