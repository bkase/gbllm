//! Artifact quantization references.

use serde::{Deserialize, Serialize};

use crate::ids::ArtifactPath;
use crate::norm_plan::NormPlan;
use crate::tensor::CanonicalTensorId;
use crate::weight_plan::TernaryWeightPlan;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct QuantSpec {
    pub ternary_weight_plans: Vec<TernaryQuantEntry>,
    pub activation_quant: Vec<ActivationQuantEntry>,
    pub norm_plans: Vec<NormQuantEntry>,
}

impl QuantSpec {
    pub fn new(
        ternary_weight_plans: Vec<TernaryQuantEntry>,
        activation_quant: Vec<ActivationQuantEntry>,
        norm_plans: Vec<NormQuantEntry>,
    ) -> Self {
        Self {
            ternary_weight_plans,
            activation_quant,
            norm_plans,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TernaryQuantEntry {
    pub projection: ArtifactPath,
    pub weight: CanonicalTensorId,
    pub scale: CanonicalTensorId,
    pub bias: Option<CanonicalTensorId>,
    pub plan: TernaryWeightPlan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivationQuantEntry {
    pub activation: ArtifactPath,
    pub quant_format: ActivationQuantFormatSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ActivationQuantFormatSpec {
    Int8,
    UInt8,
    UInt4,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormQuantEntry {
    pub norm: ArtifactPath,
    pub plan: NormPlan,
    pub lut: Option<CanonicalTensorId>,
}
