//! Export-side measured facts.

use serde::{Deserialize, Serialize};

use crate::ids::ArtifactPath;
use crate::quant::{ActivationEvalModeSpec, ActivationQuantFormatSpec, ActivationRangeSpec};
use crate::sequence::SequenceExportFacts;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportFacts {
    pub activation_ranges: Vec<RangeDigest>,
    pub sequence: SequenceExportFacts,
}

impl ExportFacts {
    pub fn new(activation_ranges: Vec<RangeDigest>, sequence: SequenceExportFacts) -> Self {
        Self {
            activation_ranges,
            sequence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeDigest {
    pub activation: ArtifactPath,
    pub range: ActivationRangeSpec,
    pub quant_format: ActivationQuantFormatSpec,
    pub eval_mode: ActivationEvalModeSpec,
}
