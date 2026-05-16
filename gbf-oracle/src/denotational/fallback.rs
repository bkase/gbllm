//! S3 fallback denotational oracle backend.

use super::{
    DenotationalBackendKind, DenotationalOracle, DenotationalOracleInputs,
    DenotationalOracleProduct, OracleError, S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD,
    evaluate_with_backend_kind,
};

/// Fallback denotational oracle pending the richer F-C1 real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct S3DenotationalFallback;

impl S3DenotationalFallback {
    /// Real owner bead recorded by fallback reports.
    pub const REAL_OWNER_BEAD: &'static str = S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD;
}

impl DenotationalOracle for S3DenotationalFallback {
    fn evaluate(
        &self,
        inputs: DenotationalOracleInputs<'_>,
    ) -> Result<DenotationalOracleProduct, OracleError> {
        evaluate_with_backend_kind(
            inputs,
            DenotationalBackendKind::Fallback,
            Some(Self::REAL_OWNER_BEAD),
        )
    }
}
