//! Target-data lowering schema.

pub use gbf_foundation::{DataLoweringProfileId, LoweringShardId, LoweringShardRef, PackerVersion};
use gbf_foundation::{Hash256, TargetProfileId};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

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
        // Work around serde's tagged-unit enum caveat: derived deserialization
        // accepts extra fields on `{ "kind": "..." }` unit-variant objects.
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

#[derive(Serialize)]
struct LoweringShardPackRepr<'a> {
    id: &'a LoweringShardId,
    kind: LoweringShardKind,
    payload_hash: Hash256,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoweringShardUnpackRepr {
    id: LoweringShardId,
    kind: LoweringShardKind,
    payload_hash: Hash256,
}

impl Pack for LoweringShard {
    type Error = serde_json::Error;

    fn pack(&self) -> Result<Vec<u8>, Self::Error> {
        serde_json::to_vec(&LoweringShardPackRepr {
            id: &self.id,
            kind: self.kind,
            payload_hash: self.payload_hash,
        })
    }
}

impl Unpack for LoweringShard {
    type Error = serde_json::Error;

    fn unpack(bytes: &[u8]) -> Result<Self, Self::Error> {
        let repr: LoweringShardUnpackRepr = serde_json::from_slice(bytes)?;
        Ok(Self {
            id: repr.id,
            kind: repr.kind,
            payload_hash: repr.payload_hash,
            packed_bytes_hash: sha256_hash(bytes),
        })
    }
}

#[derive(Serialize)]
struct LoweringManifestPackRepr<'a> {
    shard_refs: &'a [LoweringShardRef],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoweringManifestUnpackRepr {
    shard_refs: Vec<LoweringShardRef>,
}

impl Pack for LoweringManifest {
    type Error = serde_json::Error;

    fn pack(&self) -> Result<Vec<u8>, Self::Error> {
        serde_json::to_vec(&LoweringManifestPackRepr {
            shard_refs: &self.shard_refs,
        })
    }
}

impl Unpack for LoweringManifest {
    type Error = serde_json::Error;

    fn unpack(bytes: &[u8]) -> Result<Self, Self::Error> {
        let repr: LoweringManifestUnpackRepr = serde_json::from_slice(bytes)?;
        Ok(Self {
            shard_refs: repr.shard_refs,
            aggregate_hash: sha256_hash(bytes),
        })
    }
}

fn sha256_hash(bytes: &[u8]) -> Hash256 {
    Hash256::from_bytes(Sha256::digest(bytes).into())
}
