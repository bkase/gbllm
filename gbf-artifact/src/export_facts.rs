//! Export-side measured facts.

use serde::{Deserialize, Serialize};

use crate::ids::ArtifactPath;
use crate::quant::ActivationQuantFormatSpec;

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
    pub lo: f32,
    pub hi: f32,
    pub mode: RangeDigestMode,
    pub quant_format: ActivationQuantFormatSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RangeDigestMode {
    Fixed,
    Learned,
    Ema,
}
