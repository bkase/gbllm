//! Shared schema carrier atoms used across artifact, workload, and policy crates.

use serde::{Deserialize, Deserializer, Serialize};

use crate::{FieldPath, Hash256};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSchemaVersion {
    pub epoch: u32,
    pub minor: u32,
}

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
        D: Deserializer<'de>,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LineageId(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataLoweringProfileId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoweringShardRef {
    pub id: LoweringShardId,
    pub manifest_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoweringShardId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoldenVectorId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub kind: String,
    pub reference: String,
    pub hash: Option<Hash256>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    #[test]
    fn sidecar_kind_rejects_extra_fields() {
        let value = serde_json::json!({
            "kind": "GoldenVector",
            "unexpected": true
        });

        serde_json::from_value::<SidecarKind>(value).expect_err("extra field rejects");
    }

    #[test]
    fn shared_carriers_pin_representative_json_shapes() {
        assert_eq!(
            serde_json::to_value(ManifestInvariant::ComponentDigestMismatch {
                component: ComponentId("core".to_owned()),
                expected: hash(1),
                observed: hash(2),
            })
            .expect("manifest invariant serializes"),
            serde_json::json!({
                "kind": "ComponentDigestMismatch",
                "component": "core",
                "expected": "sha256:0101010101010101010101010101010101010101010101010101010101010101",
                "observed": "sha256:0202020202020202020202020202020202020202020202020202020202020202"
            })
        );

        assert_eq!(
            serde_json::to_value(LoweringShardRef {
                id: LoweringShardId("weights.0".to_owned()),
                manifest_hash: hash(3),
            })
            .expect("lowering shard ref serializes"),
            serde_json::json!({
                "id": "weights.0",
                "manifest_hash": "sha256:0303030303030303030303030303030303030303030303030303030303030303"
            })
        );
    }
}
