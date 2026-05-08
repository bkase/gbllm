//! Target-data lowering fixtures.

use gbf_artifact::lowerings::*;
use gbf_foundation::{Hash256, TargetProfileId};

pub struct TargetDataLoweringArtifactBuilder {
    artifact: TargetDataLoweringArtifact,
}

impl TargetDataLoweringArtifactBuilder {
    #[must_use]
    pub fn canonical() -> Self {
        Self {
            artifact: TargetDataLoweringArtifact {
                profile: DataLoweringProfileId("DMG-MBC5-default".to_owned()),
                target: TargetProfileId::from("DMG-MBC5"),
                packer_version: PackerVersion::new(1, 0, 0),
                manifest_hash: hash(5),
                shards: vec![canonical_weight_shard()],
            },
        }
    }

    #[must_use]
    pub fn with_packer_version(mut self, version: PackerVersion) -> Self {
        self.artifact.packer_version = version;
        self
    }

    #[must_use]
    pub fn with_target(mut self, target: TargetProfileId) -> Self {
        self.artifact.target = target;
        self
    }

    #[must_use]
    pub fn with_shard(mut self, shard: LoweringShard) -> Self {
        self.artifact.shards.push(shard);
        self
    }

    #[must_use]
    pub fn build(self) -> TargetDataLoweringArtifact {
        self.artifact
    }
}

#[must_use]
pub fn canonical_lowering_fixture() -> TargetDataLoweringArtifact {
    TargetDataLoweringArtifactBuilder::canonical().build()
}

fn canonical_weight_shard() -> LoweringShard {
    LoweringShard {
        id: LoweringShardId("weight.layer0.expert0".to_owned()),
        kind: LoweringShardKind::WeightShard,
        payload_hash: hash(6),
        packed_bytes_hash: hash(7),
    }
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_foundation::SemVer;

    #[test]
    fn builder_canonical_matches_fixture_constant() {
        let expected = TargetDataLoweringArtifact {
            profile: DataLoweringProfileId("DMG-MBC5-default".to_owned()),
            target: TargetProfileId::from("DMG-MBC5"),
            packer_version: PackerVersion(SemVer::new(1, 0, 0)),
            manifest_hash: Hash256::from_bytes([0x05; 32]),
            shards: vec![LoweringShard {
                id: LoweringShardId("weight.layer0.expert0".to_owned()),
                kind: LoweringShardKind::WeightShard,
                payload_hash: Hash256::from_bytes([0x06; 32]),
                packed_bytes_hash: Hash256::from_bytes([0x07; 32]),
            }],
        };

        assert_eq!(canonical_lowering_fixture(), expected);
        assert_eq!(
            TargetDataLoweringArtifactBuilder::canonical().build(),
            expected
        );
    }

    #[test]
    fn builder_supports_with_shard_chaining() {
        let shard = LoweringShard {
            id: LoweringShardId("scale.layer0.expert0".to_owned()),
            kind: LoweringShardKind::ScaleShard,
            payload_hash: hash(0x20),
            packed_bytes_hash: hash(0x21),
        };

        let artifact = TargetDataLoweringArtifactBuilder::canonical()
            .with_shard(shard.clone())
            .build();

        assert_eq!(artifact.shards.len(), 2);
        assert_eq!(artifact.shards[1], shard);
    }

    #[test]
    fn builder_supports_with_packer_version_chaining() {
        let version = PackerVersion(SemVer::new(2, 3, 4));
        let artifact = TargetDataLoweringArtifactBuilder::canonical()
            .with_packer_version(version)
            .build();

        assert_eq!(artifact.packer_version, version);
    }
}
