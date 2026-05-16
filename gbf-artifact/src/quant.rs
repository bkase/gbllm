//! Artifact quantization references.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::ArtifactPath;
use crate::norm_plan::NormPlan;
use crate::opset_v1::ReferenceOp;
use crate::reference_eval_graph::ReferenceEvalGraph;
use crate::tensor::CanonicalTensorId;
use crate::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};

/// F-S3 QuantSpec shape: total quantization resolution by canonical tensor id.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct QuantSpec_S3 {
    pub weight_quant: BTreeMap<CanonicalTensorId, WeightQuant>,
}

impl QuantSpec_S3 {
    #[must_use]
    pub fn new(weight_quant: BTreeMap<CanonicalTensorId, WeightQuant>) -> Self {
        Self { weight_quant }
    }

    #[must_use]
    pub fn weight_quant(&self, tensor_id: &CanonicalTensorId) -> Option<&WeightQuant> {
        self.weight_quant.get(tensor_id)
    }

    /// Verify that all graph-consumed Linear/Embedding/Classifier weights
    /// resolve through `QuantSpec::weight_quant`.
    pub fn verify_coverage(&self, graph: &ReferenceEvalGraph) -> Result<(), QuantSpecError> {
        for node in &graph.nodes {
            let Some((tensor_id, op_kind)) = graph_weight_input(node) else {
                continue;
            };
            if !self.weight_quant.contains_key(tensor_id) {
                tracing::error!(
                    target: "gbf_artifact::quant",
                    event_name = "s3::quant::coverage_missing",
                    tensor_id = %tensor_id,
                    op_kind = op_kind,
                );
                return Err(QuantSpecError::CoverageMissing {
                    tensor_id: tensor_id.clone(),
                    op_kind,
                });
            }
        }
        Ok(())
    }
}

/// S3 weight-quantization resolution result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WeightQuant {
    Fp32,
    Ternary2 {
        row_scale: Q8_8Scale,
        threshold: Q8_8Scale,
        accumulator: Accumulator,
        reduction_order: CanonicalIntegerThenScale,
    },
}

/// Raw Q8.8 scale/threshold value.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Q8_8Scale(pub u16);

impl Q8_8Scale {
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

/// Accumulator dtype pinned by F-S3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Accumulator {
    I32,
}

/// Canonical integer accumulation followed by scale application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CanonicalIntegerThenScale {
    HardenedReductionPolicyV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantSpecError {
    CoverageMissing {
        tensor_id: CanonicalTensorId,
        op_kind: &'static str,
    },
}

impl fmt::Display for QuantSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoverageMissing { tensor_id, op_kind } => {
                write!(
                    f,
                    "QuantSpec_S3 missing weight_quant entry for {op_kind} tensor {tensor_id}"
                )
            }
        }
    }
}

impl Error for QuantSpecError {}

fn graph_weight_input(
    node: &crate::reference_eval_graph::ReferenceNode,
) -> Option<(&CanonicalTensorId, &'static str)> {
    match node.op {
        ReferenceOp::Embedding => node.inputs.first().map(|id| (id, "embedding")),
        ReferenceOp::Linear => node.inputs.get(1).map(|id| (id, "linear")),
        ReferenceOp::Classifier => node.inputs.get(1).map(|id| (id, "classifier")),
        ReferenceOp::LinearStateBlock
        | ReferenceOp::Activation(_)
        | ReferenceOp::MatMul
        | ReferenceOp::Add
        | ReferenceOp::Mul
        | ReferenceOp::LayerNorm
        | ReferenceOp::Softmax => None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct QuantSpec {
    pub weight_quant: Vec<WeightQuantEntry>,
    pub ternary_weight_plans: Vec<TernaryQuantEntry>,
    pub activation_quant: Vec<ActivationQuantEntry>,
    pub norm_plans: Vec<NormQuantEntry>,
}

impl<'de> Deserialize<'de> for QuantSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct QuantSpecSerde {
            weight_quant: Option<Vec<WeightQuantEntry>>,
            #[serde(default)]
            ternary_weight_plans: Vec<TernaryQuantEntry>,
            #[serde(default)]
            activation_quant: Vec<ActivationQuantEntry>,
            #[serde(default)]
            norm_plans: Vec<NormQuantEntry>,
        }

        let raw = QuantSpecSerde::deserialize(deserializer)?;
        let weight_quant = raw.weight_quant.unwrap_or_else(|| {
            raw.ternary_weight_plans
                .iter()
                .map(WeightQuantEntry::from_ternary)
                .collect()
        });

        Ok(Self::new_with_weight_quant(
            weight_quant,
            raw.ternary_weight_plans,
            raw.activation_quant,
            raw.norm_plans,
        ))
    }
}

impl QuantSpec {
    pub fn new(
        ternary_weight_plans: Vec<TernaryQuantEntry>,
        activation_quant: Vec<ActivationQuantEntry>,
        norm_plans: Vec<NormQuantEntry>,
    ) -> Self {
        let weight_quant = ternary_weight_plans
            .iter()
            .map(WeightQuantEntry::from_ternary)
            .collect();
        Self::new_with_weight_quant(
            weight_quant,
            ternary_weight_plans,
            activation_quant,
            norm_plans,
        )
    }

    pub fn new_with_weight_quant(
        weight_quant: Vec<WeightQuantEntry>,
        ternary_weight_plans: Vec<TernaryQuantEntry>,
        activation_quant: Vec<ActivationQuantEntry>,
        norm_plans: Vec<NormQuantEntry>,
    ) -> Self {
        let mut spec = Self {
            weight_quant,
            ternary_weight_plans,
            activation_quant,
            norm_plans,
        };
        spec.sort_canonical();
        spec
    }

    pub fn default_expert_ternary_plan() -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::AnnealedGlobalThenPerOutputRow,
        )
    }

    pub fn weight_quant(&self) -> &[WeightQuantEntry] {
        &self.weight_quant
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
        self.weight_quant
            .sort_by(|left, right| left.weight.cmp(&right.weight));
        self.ternary_weight_plans
            .sort_by(|left, right| left.projection.cmp(&right.projection));
        self.activation_quant
            .sort_by(|left, right| left.activation.cmp(&right.activation));
        self.norm_plans
            .sort_by(|left, right| left.norm.cmp(&right.norm));
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightQuantEntry {
    pub weight: ArtifactPath,
    pub tensor: CanonicalTensorId,
    pub ternary_plan: Option<TernaryWeightPlan>,
}

impl WeightQuantEntry {
    pub fn full_precision(weight: ArtifactPath, tensor: CanonicalTensorId) -> Self {
        Self {
            weight,
            tensor,
            ternary_plan: None,
        }
    }

    pub fn ternary(
        weight: ArtifactPath,
        tensor: CanonicalTensorId,
        plan: TernaryWeightPlan,
    ) -> Self {
        Self {
            weight,
            tensor,
            ternary_plan: Some(plan),
        }
    }

    fn from_ternary(entry: &TernaryQuantEntry) -> Self {
        Self::ternary(entry.projection.clone(), entry.weight.clone(), entry.plan)
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
    #[serde(default)]
    pub nonlinearity: ActivationNonlinearitySpec,
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

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum ActivationNonlinearitySpec {
    #[default]
    Identity,
    Relu,
    GeluClip,
    SiluClip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormQuantEntry {
    pub norm: ArtifactPath,
    pub plan: NormPlan,
    pub lut: Option<CanonicalTensorId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_spec_default_expert_plan_matches_training_contract() {
        assert_eq!(
            QuantSpec::default_expert_ternary_plan(),
            TernaryWeightPlan::new(
                WeightEncoding::Ternary2,
                ScaleGranularity::PerOutputRow,
                ScaleFormat::Q8_8,
                ThresholdPlan::AnnealedGlobalThenPerOutputRow,
            )
        );
    }

    #[test]
    fn quant_spec_represents_mixed_precision_weight_groups() {
        let ternary = TernaryQuantEntry {
            projection: ArtifactPath::new("expert.0.up").unwrap(),
            weight: CanonicalTensorId::new("expert.0.up.weight").unwrap(),
            scale: CanonicalTensorId::new("expert.0.up.scale").unwrap(),
            bias: None,
            plan: QuantSpec::default_expert_ternary_plan(),
        };
        let spec = QuantSpec::new_with_weight_quant(
            vec![
                WeightQuantEntry::full_precision(
                    ArtifactPath::new("token_embedding").unwrap(),
                    CanonicalTensorId::new("token_embedding").unwrap(),
                ),
                WeightQuantEntry::from_ternary(&ternary),
            ],
            vec![ternary],
            vec![],
            vec![],
        );

        assert_eq!(spec.weight_quant().len(), 2);
        assert_eq!(
            spec.weight_quant()[0].weight,
            ArtifactPath::new("expert.0.up").unwrap()
        );
        assert_eq!(
            spec.weight_quant()[0].ternary_plan,
            Some(QuantSpec::default_expert_ternary_plan())
        );
        assert_eq!(
            spec.weight_quant()[1],
            WeightQuantEntry::full_precision(
                ArtifactPath::new("token_embedding").unwrap(),
                CanonicalTensorId::new("token_embedding").unwrap(),
            )
        );
    }

    #[test]
    fn quant_spec_new_derives_weight_quant_from_ternary_entries() {
        let ternary = TernaryQuantEntry {
            projection: ArtifactPath::new("expert.0.down").unwrap(),
            weight: CanonicalTensorId::new("expert.0.down.weight").unwrap(),
            scale: CanonicalTensorId::new("expert.0.down.scale").unwrap(),
            bias: None,
            plan: QuantSpec::default_expert_ternary_plan(),
        };
        let spec = QuantSpec::new(vec![ternary.clone()], vec![], vec![]);

        assert_eq!(
            spec.weight_quant(),
            &[WeightQuantEntry::from_ternary(&ternary)]
        );
    }

    #[test]
    fn quant_spec_deserializes_missing_weight_quant_from_ternary_entries() {
        let encoded = r#"{
            "ternary_weight_plans": [{
                "projection": "expert.0.up",
                "weight": "expert.0.up.weight",
                "scale": "expert.0.up.scale",
                "bias": null,
                "plan": {
                    "encoding": "Ternary2",
                    "scale_granularity": "PerOutputRow",
                    "scale_format": "Q8_8",
                    "threshold": "AnnealedGlobalThenPerOutputRow"
                }
            }],
            "activation_quant": [],
            "norm_plans": []
        }"#;

        let spec: QuantSpec = serde_json::from_str(encoded).unwrap();

        assert_eq!(
            spec.weight_quant(),
            &[WeightQuantEntry::ternary(
                ArtifactPath::new("expert.0.up").unwrap(),
                CanonicalTensorId::new("expert.0.up.weight").unwrap(),
                QuantSpec::default_expert_ternary_plan(),
            )]
        );
    }

    #[test]
    fn quant_spec_round_trips_explicit_weight_quant_entries() {
        let spec = QuantSpec::new_with_weight_quant(
            vec![WeightQuantEntry::full_precision(
                ArtifactPath::new("classifier").unwrap(),
                CanonicalTensorId::new("classifier").unwrap(),
            )],
            vec![],
            vec![],
            vec![],
        );

        let encoded = serde_json::to_string(&spec).unwrap();
        let decoded: QuantSpec = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, spec);
    }

    #[test]
    fn activation_quant_deserializes_missing_nonlinearity_as_identity() {
        let encoded = r#"{
            "weight_quant": [],
            "ternary_weight_plans": [],
            "activation_quant": [{
                "activation": "expert.activation",
                "range": { "lo": -1.0, "hi": 1.0, "mode": "Fixed" },
                "quant_format": "Int8",
                "eval_mode": "Quantized"
            }],
            "norm_plans": []
        }"#;

        let decoded: QuantSpec = serde_json::from_str(encoded).unwrap();

        assert_eq!(
            decoded.activation_quant()[0].nonlinearity,
            ActivationNonlinearitySpec::Identity
        );
    }

    #[test]
    fn activation_quant_round_trips_explicit_nonlinearity() {
        let spec = QuantSpec::new_with_weight_quant(
            vec![],
            vec![],
            vec![ActivationQuantEntry {
                activation: ArtifactPath::new("expert.activation").unwrap(),
                range: ActivationRangeSpec {
                    lo: -1.0,
                    hi: 1.0,
                    mode: ActivationRangeModeSpec::Fixed,
                },
                quant_format: ActivationQuantFormatSpec::Int8,
                eval_mode: ActivationEvalModeSpec::Quantized,
                nonlinearity: ActivationNonlinearitySpec::GeluClip,
            }],
            vec![],
        );

        let encoded = serde_json::to_string(&spec).unwrap();
        let decoded: QuantSpec = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, spec);
    }
}
