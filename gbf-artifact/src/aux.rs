//! Artifact auxiliary sidecar schema.

use gbf_foundation::Hash256;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SidecarKind {
    GoldenVector,
    SemanticCheckpointSchema,
    ConformanceEnvelope,
    ReferenceObservationCache,
    InteractionBundle,
    LexicalSpec,
}

impl<'de> Deserialize<'de> for SidecarKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum SidecarKindTag {
            GoldenVector,
            SemanticCheckpointSchema,
            ConformanceEnvelope,
            ReferenceObservationCache,
            InteractionBundle,
            LexicalSpec,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TaggedSidecarKind {
            kind: SidecarKindTag,
        }

        Ok(match TaggedSidecarKind::deserialize(deserializer)?.kind {
            SidecarKindTag::GoldenVector => Self::GoldenVector,
            SidecarKindTag::SemanticCheckpointSchema => Self::SemanticCheckpointSchema,
            SidecarKindTag::ConformanceEnvelope => Self::ConformanceEnvelope,
            SidecarKindTag::ReferenceObservationCache => Self::ReferenceObservationCache,
            SidecarKindTag::InteractionBundle => Self::InteractionBundle,
            SidecarKindTag::LexicalSpec => Self::LexicalSpec,
        })
    }
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
