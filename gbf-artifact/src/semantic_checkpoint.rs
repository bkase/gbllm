//! F-S3 semantic checkpoint schema.
use serde::{Deserialize, Serialize};

/// Observation checkpoint variants used by S3 agreement and conformance.
#[allow(clippy::enum_variant_names)] // RFC names are PostEmbedding/PostLogits/PostDecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCheckpointSchema {
    PostEmbedding,
    PostLogits,
    PostDecode,
}

/// Short alias used by conformance maps.
pub type SemanticCheckpoint = SemanticCheckpointSchema;

/// Checkpoint role under the S3 observation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointRole {
    ObservationOnly,
    AgreementGated,
}
