//! Export-side measured facts.

use serde::{Deserialize, Serialize};

use crate::ids::ArtifactPath;
use crate::quant::{ActivationEvalModeSpec, ActivationQuantFormatSpec, ActivationRangeSpec};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ExportFacts {
    pub activation_ranges: Vec<RangeDigest>,
}

impl ExportFacts {
    pub fn new(activation_ranges: Vec<RangeDigest>) -> Self {
        Self { activation_ranges }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeDigest {
    pub activation: ArtifactPath,
    pub range: ActivationRangeSpec,
    pub quant_format: ActivationQuantFormatSpec,
    pub eval_mode: ActivationEvalModeSpec,
}
