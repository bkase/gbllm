//! Artifact manifest schema.

use std::collections::BTreeSet;

use gbf_foundation::{FieldPath, Hash256};
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

/// Manifest invariants Stage 0 class 3 dispatches on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ManifestInvariant {
    FeatureSetEpochInconsistent {
        epoch: ArtifactSchemaVersion,
        feature: ArtifactFeature,
    },
    RequiredComponentMissing {
        component: ComponentId,
    },
    ComponentDigestMismatch {
        component: ComponentId,
        expected: Hash256,
        observed: Hash256,
    },
    LineageContradiction {
        derived: LineageId,
        recorded: LineageId,
    },
    ManifestSelfHashMismatch {
        recomputed: Hash256,
        recorded: Hash256,
    },
    ForbiddenBuildIdentityField {
        field: FieldPath,
    },
}

/// Schema version of the artifact manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSchemaVersion {
    pub epoch: u32,
    pub minor: u32,
}

/// Closed feature set the artifact requires from the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArtifactFeature {
    DenseI8,
    Ternary2Quant,
    Binary1Quant,
    SparseTernaryBitplanes,
    MoeRouting,
    LinearStateSequence,
    BoundedKvSequence,
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

/// Lineage id linking the artifact back to its training/export run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LineageId(pub Hash256);

/// Wall-clock-free deterministic timestamp in milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManifestTimestamp(pub u64);

/// Component identity. Stable across re-runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentId(pub String);
