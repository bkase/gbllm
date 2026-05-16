//! S3 reference-program opset surface.

use serde::{Deserialize, Serialize};

/// Reference program opset id pinned for F-S3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceOpsetId {
    OpsetV1,
}

/// Activation variants in `opset_v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ActivationKind {
    #[serde(rename = "relu")]
    ReLU,
    #[serde(rename = "gelu")]
    GeLU,
    #[serde(rename = "silu")]
    SiLU,
    #[serde(rename = "tanh")]
    Tanh,
}

impl ActivationKind {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::ReLU => "relu",
            Self::GeLU => "gelu",
            Self::SiLU => "silu",
            Self::Tanh => "tanh",
        }
    }
}

/// Pure reference evaluator operations allowed by `opset_v1`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "activation", rename_all = "snake_case")]
pub enum ReferenceOp {
    Linear,
    Embedding,
    Classifier,
    LinearStateBlock,
    Activation(ActivationKind),
    MatMul,
    Add,
    Mul,
    LayerNorm,
    Softmax,
}

impl ReferenceOp {
    #[must_use]
    pub const fn stable_kind(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Embedding => "embedding",
            Self::Classifier => "classifier",
            Self::LinearStateBlock => "linear_state_block",
            Self::Activation(kind) => (*kind).stable_name(),
            Self::MatMul => "mat_mul",
            Self::Add => "add",
            Self::Mul => "mul",
            Self::LayerNorm => "layer_norm",
            Self::Softmax => "softmax",
        }
    }
}
