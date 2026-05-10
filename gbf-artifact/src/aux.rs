//! Artifact auxiliary sidecar schema.

use gbf_foundation::Hash256;
pub use gbf_foundation::SidecarKind;
pub use gbf_workload::manifest::{GoldenVectorId, GoldenVectorRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactAux {
    pub checkpoint_schema: Option<SemanticCheckpointSchemaRef>,
    pub conformance_envelope: Option<ConformanceEnvelopeRef>,
    pub golden_vectors: Vec<GoldenVectorRef>,
    pub interaction_bundle: Option<InteractionBundleRef>,
    pub lexical_spec: Option<LexicalSpecRef>,
    pub reference_observation_cache: Option<ReferenceObservationCacheRef>,
}

macro_rules! sidecar_ref {
    ($name:ident, $id:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub id: $id,
            pub hash: Hash256,
        }
    };
}

sidecar_ref!(SemanticCheckpointSchemaRef, SemanticCheckpointSchemaId);
sidecar_ref!(ConformanceEnvelopeRef, ConformanceEnvelopeId);
sidecar_ref!(ReferenceObservationCacheRef, ReferenceObservationCacheId);
sidecar_ref!(InteractionBundleRef, InteractionBundleId);
sidecar_ref!(LexicalSpecRef, LexicalSpecId);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SemanticCheckpointSchemaId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConformanceEnvelopeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReferenceObservationCacheId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InteractionBundleId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LexicalSpecId(pub String);
