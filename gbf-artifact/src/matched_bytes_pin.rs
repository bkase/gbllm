//! S7 matched-bytes preregistration pin.

use std::fmt;

use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, self_hash_omitting_fields,
};
use gbf_policy::{BiasPolicy, MatchedBytesPolicy, MatchedBytesSolution};
use serde::{Deserialize, Serialize};

/// Public schema id for the S7 matched-bytes pin.
pub const MATCHED_BYTES_PIN_SCHEMA: &str = "s7_matched_bytes_pin.v1";
const MATCHED_BYTES_PIN_SCHEMA_VERSION: &str = "1";
const MATCHED_BYTES_SELF_HASH_FIELD: &str = "matched_bytes_self_hash";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchedBytesPin {
    pub formula_version: gbf_foundation::SemVer,
    pub d_ff_dense_resolved: u16,
    pub bias_policy: BiasPolicy,
    pub b_experts_total: u64,
    pub b_router_overhead_total: u64,
    pub b_dense_ffn_total: u64,
    pub b_deployed_total_moe: u64,
    pub b_deployed_total_dense: u64,
    pub tolerance_bytes: u64,
    pub matched_bytes_self_hash: Hash256,
}

impl MatchedBytesPin {
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "MatchedBytesPin",
            MATCHED_BYTES_PIN_SCHEMA,
            MATCHED_BYTES_PIN_SCHEMA_VERSION,
        )
    }

    #[must_use]
    pub fn from_solution(solution: MatchedBytesSolution, policy: MatchedBytesPolicy) -> Self {
        Self {
            formula_version: policy.formula_version,
            d_ff_dense_resolved: solution.d_ff_dense,
            bias_policy: policy.bias_policy,
            b_experts_total: solution.b_experts_total.as_u64(),
            b_router_overhead_total: solution.b_router_overhead_total.as_u64(),
            b_dense_ffn_total: solution.b_dense_ffn_total.as_u64(),
            b_deployed_total_moe: solution.b_deployed_total_moe.as_u64(),
            b_deployed_total_dense: solution.b_deployed_total_dense.as_u64(),
            tolerance_bytes: solution.tolerance_bytes.as_u64(),
            matched_bytes_self_hash: Hash256::ZERO,
        }
    }

    pub fn computed_self_hash(&self) -> Result<Hash256, MatchedBytesPinError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            MATCHED_BYTES_SELF_HASH_FIELD,
            &[],
        )?)
    }

    pub fn with_computed_self_hash(mut self) -> Result<Self, MatchedBytesPinError> {
        self.matched_bytes_self_hash = self.computed_self_hash()?;
        Ok(self)
    }

    pub fn verify_self_hash(&self) -> Result<(), MatchedBytesPinError> {
        let expected = self.computed_self_hash()?;
        if self.matched_bytes_self_hash != expected {
            return Err(MatchedBytesPinError::SelfHashMismatch {
                expected,
                observed: self.matched_bytes_self_hash,
            });
        }
        Ok(())
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, MatchedBytesPinError> {
        Ok(CanonicalJson::to_vec(self)?)
    }
}

#[derive(Debug)]
pub enum MatchedBytesPinError {
    CanonicalJson(CanonicalJsonError),
    SelfHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
}

impl fmt::Display for MatchedBytesPinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "matched_bytes_self_hash mismatch: expected {expected}, observed {observed}"
            ),
        }
    }
}

impl std::error::Error for MatchedBytesPinError {}

impl From<CanonicalJsonError> for MatchedBytesPinError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

#[cfg(test)]
mod tests {
    use gbf_foundation::{ByteCost, SemVer};
    use gbf_policy::{
        BiasPolicy, MATCHED_BYTES_FORMULA_VERSION, MatchedBytesConfig, MatchedBytesPolicy,
        solve_d_ff_dense,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn matched_bytes_pin_round_trips_with_self_hash() {
        let policy = MatchedBytesPolicy::s7_canonical();
        let solution =
            solve_d_ff_dense(MatchedBytesConfig::s7_moe_tiny(), policy).expect("solution");
        let pin = MatchedBytesPin::from_solution(solution, policy)
            .with_computed_self_hash()
            .expect("self hash");

        pin.verify_self_hash().expect("self hash verifies");
        assert_ne!(pin.matched_bytes_self_hash, Hash256::ZERO);

        let encoded = pin.canonical_json_bytes().expect("canonical json");
        let decoded: MatchedBytesPin = serde_json::from_slice(&encoded).expect("deserializes");

        assert_eq!(decoded, pin);
        decoded
            .verify_self_hash()
            .expect("decoded self hash verifies");
    }

    #[test]
    fn matched_bytes_pin_public_json_shape_is_pinned() {
        let pin = MatchedBytesPin {
            formula_version: MATCHED_BYTES_FORMULA_VERSION,
            d_ff_dense_resolved: 572,
            bias_policy: BiasPolicy::Q8_8PerOutput,
            b_experts_total: 79_424,
            b_router_overhead_total: 4_352,
            b_dense_ffn_total: 83_792,
            b_deployed_total_moe: 83_776,
            b_deployed_total_dense: 83_792,
            tolerance_bytes: 65_536,
            matched_bytes_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
        .expect("hash");

        let value = serde_json::to_value(&pin).expect("json value");
        assert_eq!(
            value,
            json!({
                "formula_version": {"major": 0, "minor": 2, "patch": 0},
                "d_ff_dense_resolved": 572,
                "bias_policy": "q8_8_per_output",
                "b_experts_total": 79_424,
                "b_router_overhead_total": 4_352,
                "b_dense_ffn_total": 83_792,
                "b_deployed_total_moe": 83_776,
                "b_deployed_total_dense": 83_792,
                "tolerance_bytes": 65_536,
                "matched_bytes_self_hash": pin.matched_bytes_self_hash,
            })
        );
    }

    #[test]
    fn matched_bytes_pin_detects_self_hash_drift() {
        let policy = MatchedBytesPolicy::s7_canonical();
        let solution =
            solve_d_ff_dense(MatchedBytesConfig::s7_moe_tiny(), policy).expect("solution");
        let mut pin = MatchedBytesPin::from_solution(solution, policy)
            .with_computed_self_hash()
            .expect("self hash");

        pin.b_dense_ffn_total += ByteCost::new(1).as_u64();

        assert!(matches!(
            pin.verify_self_hash(),
            Err(MatchedBytesPinError::SelfHashMismatch { .. })
        ));
    }

    #[test]
    fn matched_bytes_pin_formula_version_is_semver_struct() {
        let value = serde_json::to_value(SemVer::new(0, 2, 0)).expect("semver json");
        assert_eq!(value, json!({"major": 0, "minor": 2, "patch": 0}));
    }
}
