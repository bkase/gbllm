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
        let mut spec = Self {
            ternary_weight_plans,
            activation_quant,
            norm_plans,
        };
        spec.sort_canonical();
        spec
    }

    pub fn ternary_weight_plans(&self) -> &[TernaryQuantEntry] {
        &self.ternary_weight_plans
    }

    pub fn activation_quant(&self) -> &[ActivationQuantEntry] {
        &self.activation_quant
    }

    pub fn norm_plans(&self) -> &[NormQuantEntry] {
        &self.norm_plans
    }

    pub(crate) fn canonicalized(mut self) -> Self {
        self.sort_canonical();
        self
    }

    fn sort_canonical(&mut self) {
        self.ternary_weight_plans
            .sort_by(|left, right| left.projection.cmp(&right.projection));
        self.activation_quant
            .sort_by(|left, right| left.activation.cmp(&right.activation));
        self.norm_plans
            .sort_by(|left, right| left.norm.cmp(&right.norm));
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
    pub range: ActivationRangeSpec,
    pub quant_format: ActivationQuantFormatSpec,
    pub eval_mode: ActivationEvalModeSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ActivationQuantFormatSpec {
    Int8,
    UInt8,
    UInt4,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ActivationRangeSpec {
    pub lo: f32,
    pub hi: f32,
    pub mode: ActivationRangeModeSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ActivationRangeModeSpec {
    Fixed,
    Learned,
    Ema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ActivationEvalModeSpec {
    Quantized,
    Passthrough,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormQuantEntry {
    pub norm: ArtifactPath,
    pub plan: NormPlan,
    pub lut: Option<CanonicalTensorId>,
}
