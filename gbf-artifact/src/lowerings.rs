//! Target-data lowering schema.

use gbf_foundation::{Hash256, SemVer, TargetProfileId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Per-target-profile packed lowering of a frozen artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetDataLoweringArtifact {
    pub profile: DataLoweringProfileId,
    pub target: TargetProfileId,
    pub packer_version: PackerVersion,
    pub manifest_hash: Hash256,
    pub shards: Vec<LoweringShard>,
}

/// One packed shard of the lowering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoweringShard {
    pub id: LoweringShardId,
    pub kind: LoweringShardKind,
    pub payload_hash: Hash256,
    pub packed_bytes_hash: Hash256,
}

/// Lightweight reference to a lowering shard for diagnostic carriers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoweringShardRef {
    pub id: LoweringShardId,
    pub manifest_hash: Hash256,
}

/// Aggregate manifest of all shards in a `TargetDataLoweringArtifact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoweringManifest {
    pub shard_refs: Vec<LoweringShardRef>,
    pub aggregate_hash: Hash256,
}

/// Closed kind enum for packed lowering shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum LoweringShardKind {
    WeightShard,
    ScaleShard,
    LutShard,
    RoutingTableShard,
    SequenceStateShard,
    EmbeddingShard,
}

impl<'de> Deserialize<'de> for LoweringShardKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[allow(clippy::enum_variant_names)]
        enum LoweringShardKindTag {
            WeightShard,
            ScaleShard,
            LutShard,
            RoutingTableShard,
            SequenceStateShard,
            EmbeddingShard,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TaggedLoweringShardKind {
            kind: LoweringShardKindTag,
        }

        Ok(
            match TaggedLoweringShardKind::deserialize(deserializer)?.kind {
                LoweringShardKindTag::WeightShard => Self::WeightShard,
                LoweringShardKindTag::ScaleShard => Self::ScaleShard,
                LoweringShardKindTag::LutShard => Self::LutShard,
                LoweringShardKindTag::RoutingTableShard => Self::RoutingTableShard,
                LoweringShardKindTag::SequenceStateShard => Self::SequenceStateShard,
                LoweringShardKindTag::EmbeddingShard => Self::EmbeddingShard,
            },
        )
    }
}

/// Newtype around `SemVer` for packer compatibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackerVersion(pub SemVer);

impl PackerVersion {
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self(SemVer::new(major, minor, patch))
    }
}

impl Serialize for PackerVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for PackerVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map(Self).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataLoweringProfileId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoweringShardId(pub String);

/// Round-trip packer trait. Implementations are owned by T-B2.7.
pub trait Pack {
    type Error: std::error::Error + Send + Sync + 'static;

    fn pack(&self) -> Result<Vec<u8>, Self::Error>;
}

/// Round-trip unpacker trait. Implementations are owned by T-B2.7.
pub trait Unpack: Sized {
    type Error: std::error::Error + Send + Sync + 'static;

    fn unpack(bytes: &[u8]) -> Result<Self, Self::Error>;
}
