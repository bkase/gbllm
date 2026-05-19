//! Real S3 denotational oracle backend.

use super::{
    DenotationalBackendKind, DenotationalOracle, DenotationalOracleInputs,
    DenotationalOracleProduct, OracleError, evaluate_with_backend_kind,
};

/// Real denotational oracle for S3 reference bundles.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealDenotationalOracle;

impl DenotationalOracle for RealDenotationalOracle {
    fn evaluate(
        &self,
        inputs: DenotationalOracleInputs<'_>,
    ) -> Result<DenotationalOracleProduct, OracleError> {
        evaluate_with_backend_kind(inputs, DenotationalBackendKind::Real, None)
    }
}
