//! Artifact manifest schema.

use std::collections::BTreeSet;

use gbf_foundation::Hash256;
pub use gbf_foundation::{
    ArtifactFeature, ArtifactSchemaVersion, ComponentId, LineageId, ManifestInvariant,
};
use serde::{Deserialize, Serialize};

/// Manifest of record for a frozen artifact.
///
/// Stage 0 class 2 recomputes the canonical semantic-core hash and compares
/// against `semantic_core_hash`; class 3 enumerates manifest invariants. The
/// `manifest_self_hash` field is computed by zeroing it to the sentinel before
/// canonical hashing. That helper is owned by T-B2.5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub components: Vec<ManifestComponent>,
    pub created_at: ManifestTimestamp,
    pub lineage: LineageId,
    pub manifest_self_hash: Hash256,
    pub required_features: BTreeSet<ArtifactFeature>,
    pub schema_version: ArtifactSchemaVersion,
    pub semantic_core_hash: Hash256,
}

/// One component declared by the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestComponent {
    pub digest: Hash256,
    pub id: ComponentId,
    pub kind: ComponentKind,
}

/// Closed kind of a manifest component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ComponentKind {
    CanonicalTensor,
    QuantSpec,
    NormPlan,
    LutSpec,
    SequenceSemantics,
    DecodeSpec,
    LexicalSpec,
    InteractionBundle,
    SemanticCheckpointSchema,
    ConformanceEnvelope,
    ReferenceObservationCache,
    HintBundle,
}

/// Wall-clock-free deterministic timestamp in milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManifestTimestamp(pub u64);
