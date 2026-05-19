//! Shared public schema carriers for F-B6/F-B7 report bodies.

use gbf_foundation::{ExpertId, Hash256, LayerId};
use gbf_policy::InferOpTag;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValueId(pub u32);

impl ValueId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectId(pub u32);

impl EffectId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StateSlotId(pub u32);

impl StateSlotId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TensorId(pub u32);

impl TensorId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NormPlanId(pub u32);

impl NormPlanId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DecodePlanId(pub u32);

impl DecodePlanId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenInputId(pub u8);

impl TokenInputId {
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticAnchor {
    pub anchor_id: Hash256,
}

impl SemanticAnchor {
    #[must_use]
    pub const fn new(anchor_id: Hash256) -> Self {
        Self { anchor_id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum ExpertWeightSlot {
    FfnGate,
    FfnUp,
    FfnDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NormSite {
    LayerSequence { layer: LayerId },
    LayerFfn { layer: LayerId },
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResidualSite {
    PostFfn,
    PostSequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorDomain {
    RawIntegerProducts,
    PostScaleQ8_8,
    PostScaleQ16_16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalProvenanceTuple {
    pub op_tag: InferOpTag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert: Option<ExpertId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert_weight_slot: Option<ExpertWeightSlot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub norm_site: Option<NormSite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_slot: Option<StateSlotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residual_site: Option<ResidualSite>,
    pub occurrence_index: u32,
}

impl CanonicalProvenanceTuple {
    #[must_use]
    pub const fn new(op_tag: InferOpTag, occurrence_index: u32) -> Self {
        Self {
            op_tag,
            layer: None,
            expert: None,
            expert_weight_slot: None,
            norm_site: None,
            state_slot: None,
            residual_site: None,
            occurrence_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum QuantGraphEntityRef {
    Embedding,
    NormPlan {
        plan: NormPlanId,
    },
    NormSite {
        site: NormSite,
    },
    RouterLayer {
        layer: LayerId,
    },
    RouterTensor {
        layer: LayerId,
        tensor: TensorId,
    },
    RouterSelection {
        layer: LayerId,
    },
    ExpertSection {
        layer: LayerId,
        expert: ExpertId,
    },
    ExpertTensor {
        layer: LayerId,
        expert: ExpertId,
        slot: ExpertWeightSlot,
        tensor: TensorId,
    },
    FfnActivationSite {
        layer: LayerId,
        expert: ExpertId,
    },
    ResidualSiteRef {
        #[serde(skip_serializing_if = "Option::is_none")]
        layer: Option<LayerId>,
        site: ResidualSite,
    },
    DecodePlan {
        plan: DecodePlanId,
    },
    ClassifyHead,
    SequenceSlot {
        slot: StateSlotId,
    },
    SequenceStep {
        layer: LayerId,
    },
    TokenInput {
        token_input: TokenInputId,
    },
}
