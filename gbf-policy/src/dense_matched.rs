//! Dense matched-bytes policy surface for F-S7.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::matched_bytes::{
    BiasPolicy, BiasPolicyParseError, MATCHED_BYTES_FORMULA_VERSION, MatchedBytesPolicy,
    S7_CANONICAL_BIAS_POLICY, S7_ONE_BANK_BYTES, S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM,
    S7_TERNARY_METADATA_BYTES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DenseMatchedBytesPolicy(MatchedBytesPolicy);

impl DenseMatchedBytesPolicy {
    pub fn new(bias_policy: &str) -> Result<Self, DenseMatchedBytesPolicyError> {
        Ok(Self::from_bias_policy(bias_policy.parse()?))
    }

    #[must_use]
    pub const fn from_bias_policy(bias_policy: BiasPolicy) -> Self {
        Self::from_parts(
            MATCHED_BYTES_FORMULA_VERSION,
            S7_TERNARY_METADATA_BYTES,
            bias_policy,
            S7_ONE_BANK_BYTES,
            S7_ROUTER_HIGH_PRECISION_BYTES_PER_PARAM,
        )
    }

    #[must_use]
    pub const fn s7_canonical() -> Self {
        Self::from_bias_policy(S7_CANONICAL_BIAS_POLICY)
    }

    #[must_use]
    pub const fn from_parts(
        formula_version: gbf_foundation::SemVer,
        ternary_metadata_bytes: gbf_foundation::ByteCost,
        bias_policy: BiasPolicy,
        one_bank_bytes: gbf_foundation::ByteCost,
        router_parameter_bytes: u8,
    ) -> Self {
        Self(MatchedBytesPolicy {
            formula_version,
            ternary_metadata_bytes,
            bias_policy,
            one_bank_bytes,
            router_parameter_bytes,
        })
    }

    #[must_use]
    pub const fn matched_bytes_policy(&self) -> MatchedBytesPolicy {
        self.0
    }
}

impl Default for DenseMatchedBytesPolicy {
    fn default() -> Self {
        Self::s7_canonical()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenseMatchedBytesPolicyError {
    BiasPolicy(BiasPolicyParseError),
}

impl fmt::Display for DenseMatchedBytesPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BiasPolicy(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DenseMatchedBytesPolicyError {}

impl From<BiasPolicyParseError> for DenseMatchedBytesPolicyError {
    fn from(error: BiasPolicyParseError) -> Self {
        Self::BiasPolicy(error)
    }
}
