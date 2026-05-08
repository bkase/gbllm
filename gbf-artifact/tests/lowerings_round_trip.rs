use gbf_artifact::lowerings::{
    DataLoweringProfileId, LoweringManifest, LoweringShard, LoweringShardId, LoweringShardKind,
    LoweringShardRef, Pack, PackerVersion, TargetDataLoweringArtifact, Unpack,
};
use gbf_foundation::{Hash256, SemVer, TargetProfileId};

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn hash_json(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn canonical_shard() -> LoweringShard {
    LoweringShard {
        id: LoweringShardId("weight.layer0.expert0".to_owned()),
        kind: LoweringShardKind::WeightShard,
        payload_hash: hash(6),
        packed_bytes_hash: hash(7),
    }
}

fn shard(
    id: &str,
    kind: LoweringShardKind,
    payload_hash: u8,
    packed_bytes_hash: u8,
) -> LoweringShard {
    LoweringShard {
        id: LoweringShardId(id.to_owned()),
        kind,
        payload_hash: hash(payload_hash),
        packed_bytes_hash: hash(packed_bytes_hash),
    }
}

fn canonical_artifact() -> TargetDataLoweringArtifact {
    TargetDataLoweringArtifact {
        profile: DataLoweringProfileId("DMG-MBC5-default".to_owned()),
        target: TargetProfileId::from("DMG-MBC5"),
        packer_version: PackerVersion::new(1, 0, 0),
        manifest_hash: hash(5),
        shards: vec![canonical_shard()],
    }
}

fn assert_round_trip<T>(value: &T)
where
    T: std::fmt::Debug + PartialEq + serde::Serialize + serde::de::DeserializeOwned,
{
    let encoded = serde_json::to_string(value).expect("value serializes");
    let decoded: T = serde_json::from_str(&encoded).expect("value deserializes");
    let reencoded = serde_json::to_string(&decoded).expect("decoded value serializes");

    assert_eq!(&decoded, value);
    assert_eq!(reencoded, encoded);
}

#[test]
fn lowering_artifact_round_trip_canonical_fixture() {
    let artifact = canonical_artifact();

    assert_round_trip(&artifact);
    assert_eq!(
        serde_json::to_value(&artifact).expect("artifact json value"),
        serde_json::json!({
            "manifest_hash": hash_json(5),
            "packer_version": "1.0.0",
            "profile": "DMG-MBC5-default",
            "shards": [
                {
                    "id": "weight.layer0.expert0",
                    "kind": { "kind": "WeightShard" },
                    "packed_bytes_hash": hash_json(7),
                    "payload_hash": hash_json(6)
                }
            ],
            "target": "DMG-MBC5"
        })
    );
}

#[test]
fn lowering_artifact_round_trip_multiple_shards() {
    let artifact = TargetDataLoweringArtifact {
        shards: vec![
            canonical_shard(),
            shard("scale.layer0.expert0", LoweringShardKind::ScaleShard, 8, 9),
            shard("lut.layer0", LoweringShardKind::LutShard, 10, 11),
        ],
        ..canonical_artifact()
    };

    assert_round_trip(&artifact);
    assert_eq!(
        serde_json::to_value(&artifact).expect("artifact json value")["shards"],
        serde_json::json!([
            {
                "id": "weight.layer0.expert0",
                "kind": { "kind": "WeightShard" },
                "packed_bytes_hash": hash_json(7),
                "payload_hash": hash_json(6)
            },
            {
                "id": "scale.layer0.expert0",
                "kind": { "kind": "ScaleShard" },
                "packed_bytes_hash": hash_json(9),
                "payload_hash": hash_json(8)
            },
            {
                "id": "lut.layer0",
                "kind": { "kind": "LutShard" },
                "packed_bytes_hash": hash_json(11),
                "payload_hash": hash_json(10)
            }
        ])
    );
}

#[test]
fn lowering_shard_round_trip() {
    let shard = shard(
        "routing.layer0",
        LoweringShardKind::RoutingTableShard,
        0x12,
        0x13,
    );

    assert_round_trip(&shard);
    assert_eq!(
        serde_json::to_value(&shard).expect("shard json value"),
        serde_json::json!({
            "id": "routing.layer0",
            "kind": { "kind": "RoutingTableShard" },
            "packed_bytes_hash": hash_json(0x13),
            "payload_hash": hash_json(0x12)
        })
    );
}

#[test]
fn lowering_shard_ref_round_trip() {
    let shard_ref = LoweringShardRef {
        id: LoweringShardId("weight.layer0.expert0".to_owned()),
        manifest_hash: hash(0x14),
    };

    assert_round_trip(&shard_ref);
    assert_eq!(
        serde_json::to_value(&shard_ref).expect("shard ref json value"),
        serde_json::json!({
            "id": "weight.layer0.expert0",
            "manifest_hash": hash_json(0x14)
        })
    );
}

#[test]
fn lowering_manifest_round_trip() {
    let manifest = LoweringManifest {
        shard_refs: vec![
            LoweringShardRef {
                id: LoweringShardId("weight.layer0.expert0".to_owned()),
                manifest_hash: hash(0x14),
            },
            LoweringShardRef {
                id: LoweringShardId("scale.layer0.expert0".to_owned()),
                manifest_hash: hash(0x15),
            },
        ],
        aggregate_hash: hash(0x16),
    };

    assert_round_trip(&manifest);
    assert_eq!(
        serde_json::to_value(&manifest).expect("manifest json value"),
        serde_json::json!({
            "aggregate_hash": hash_json(0x16),
            "shard_refs": [
                {
                    "id": "weight.layer0.expert0",
                    "manifest_hash": hash_json(0x14)
                },
                {
                    "id": "scale.layer0.expert0",
                    "manifest_hash": hash_json(0x15)
                }
            ]
        })
    );
}

#[test]
fn lowering_manifest_shard_refs_preserve_declaration_order() {
    let manifest = LoweringManifest {
        shard_refs: vec![
            LoweringShardRef {
                id: LoweringShardId("zeta".to_owned()),
                manifest_hash: hash(0x17),
            },
            LoweringShardRef {
                id: LoweringShardId("alpha".to_owned()),
                manifest_hash: hash(0x18),
            },
            LoweringShardRef {
                id: LoweringShardId("middle".to_owned()),
                manifest_hash: hash(0x19),
            },
        ],
        aggregate_hash: hash(0x1a),
    };

    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");
    let decoded: LoweringManifest = serde_json::from_str(&encoded).expect("manifest deserializes");
    let encoded_value: serde_json::Value =
        serde_json::from_str(&encoded).expect("encoded manifest is json");

    assert_eq!(
        decoded
            .shard_refs
            .iter()
            .map(|shard_ref| shard_ref.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["zeta", "alpha", "middle"]
    );
    assert_eq!(
        encoded_value["shard_refs"]
            .as_array()
            .expect("shard_refs is an array")
            .iter()
            .map(|shard_ref| shard_ref["id"].as_str().expect("id is string"))
            .collect::<Vec<_>>(),
        vec!["zeta", "alpha", "middle"]
    );
}

#[test]
fn lowering_shard_kind_round_trip_all_variants() {
    let variants = [
        (
            LoweringShardKind::WeightShard,
            serde_json::json!({ "kind": "WeightShard" }),
        ),
        (
            LoweringShardKind::ScaleShard,
            serde_json::json!({ "kind": "ScaleShard" }),
        ),
        (
            LoweringShardKind::LutShard,
            serde_json::json!({ "kind": "LutShard" }),
        ),
        (
            LoweringShardKind::RoutingTableShard,
            serde_json::json!({ "kind": "RoutingTableShard" }),
        ),
        (
            LoweringShardKind::SequenceStateShard,
            serde_json::json!({ "kind": "SequenceStateShard" }),
        ),
        (
            LoweringShardKind::EmbeddingShard,
            serde_json::json!({ "kind": "EmbeddingShard" }),
        ),
    ];

    for (kind, expected_json) in variants {
        assert_round_trip(&kind);
        assert_eq!(
            serde_json::to_value(kind).expect("lowering shard kind json value"),
            expected_json
        );
        assert_eq!(
            serde_json::from_value::<LoweringShardKind>(expected_json)
                .expect("lowering shard kind deserializes"),
            kind
        );
    }
}

#[test]
fn packer_version_round_trip_serializes_as_semver_string() {
    let version = PackerVersion(SemVer::new(2, 3, 4));

    assert_round_trip(&version);
    assert_eq!(
        serde_json::to_value(version).expect("packer version json value"),
        serde_json::json!("2.3.4")
    );
}

#[test]
fn packer_version_rejects_malformed_strings() {
    for json in [r#""1.two.3""#, r#""1.2""#] {
        let err =
            serde_json::from_str::<PackerVersion>(json).expect_err("malformed version rejects");

        assert!(err.to_string().contains("semantic version"));
    }
}

#[test]
fn data_lowering_profile_id_round_trip() {
    let id = DataLoweringProfileId("DMG-MBC5-default".to_owned());

    assert_round_trip(&id);
    assert_eq!(
        serde_json::to_value(&id).expect("profile id json value"),
        serde_json::json!("DMG-MBC5-default")
    );
}

#[test]
fn lowering_shard_id_round_trip() {
    let id = LoweringShardId("weight.layer0.expert0".to_owned());

    assert_round_trip(&id);
    assert_eq!(
        serde_json::to_value(&id).expect("shard id json value"),
        serde_json::json!("weight.layer0.expert0")
    );
}

#[test]
fn lowering_artifact_rejects_unknown_field() {
    let mut value = serde_json::to_value(canonical_artifact()).expect("artifact json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<TargetDataLoweringArtifact>(value)
        .expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn lowering_shard_rejects_unknown_field() {
    let mut value = serde_json::to_value(canonical_shard()).expect("shard json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<LoweringShard>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn lowering_shard_ref_rejects_unknown_field() {
    let shard_ref = LoweringShardRef {
        id: LoweringShardId("weight.layer0.expert0".to_owned()),
        manifest_hash: hash(0x14),
    };
    let mut value = serde_json::to_value(shard_ref).expect("shard ref json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<LoweringShardRef>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn lowering_manifest_rejects_unknown_field() {
    let manifest = LoweringManifest {
        shard_refs: vec![LoweringShardRef {
            id: LoweringShardId("weight.layer0.expert0".to_owned()),
            manifest_hash: hash(0x14),
        }],
        aggregate_hash: hash(0x16),
    };
    let mut value = serde_json::to_value(manifest).expect("manifest json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<LoweringManifest>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn lowering_shard_kind_rejects_unknown_kind() {
    let json = r#"{"kind":"FutureShard"}"#;

    let err = serde_json::from_str::<LoweringShardKind>(json).expect_err("unknown kind rejects");

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn lowering_shard_kind_rejects_extra_field() {
    let json = r#"{"kind":"WeightShard","unexpected":true}"#;

    serde_json::from_str::<LoweringShardKind>(json).expect_err("extra field rejects");
}

#[test]
fn shards_preserve_declaration_order() {
    let artifact = TargetDataLoweringArtifact {
        shards: vec![
            shard("zeta", LoweringShardKind::WeightShard, 0x20, 0x21),
            shard("alpha", LoweringShardKind::ScaleShard, 0x22, 0x23),
            shard("middle", LoweringShardKind::EmbeddingShard, 0x24, 0x25),
        ],
        ..canonical_artifact()
    };

    let encoded = serde_json::to_string(&artifact).expect("artifact serializes");
    let decoded: TargetDataLoweringArtifact =
        serde_json::from_str(&encoded).expect("artifact deserializes");
    let encoded_value: serde_json::Value =
        serde_json::from_str(&encoded).expect("encoded artifact is json");

    assert_eq!(
        decoded
            .shards
            .iter()
            .map(|shard| shard.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["zeta", "alpha", "middle"]
    );
    assert_eq!(
        encoded_value["shards"]
            .as_array()
            .expect("shards is an array")
            .iter()
            .map(|shard| shard["id"].as_str().expect("id is string"))
            .collect::<Vec<_>>(),
        vec!["zeta", "alpha", "middle"]
    );
}

#[test]
fn pack_unpack_traits_compile_with_required_bounds() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DummyShardPayload(Vec<u8>);

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DummyError;

    impl std::fmt::Display for DummyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("dummy pack error")
        }
    }

    impl std::error::Error for DummyError {}

    impl Pack for DummyShardPayload {
        type Error = DummyError;

        fn pack(&self) -> Result<Vec<u8>, Self::Error> {
            Ok(self.0.clone())
        }
    }

    impl Unpack for DummyShardPayload {
        type Error = DummyError;

        fn unpack(bytes: &[u8]) -> Result<Self, Self::Error> {
            Ok(Self(bytes.to_vec()))
        }
    }

    fn assert_round_trip_bounds<T>()
    where
        T: Pack<Error = DummyError> + Unpack<Error = DummyError> + PartialEq + std::fmt::Debug,
    {
    }

    assert_round_trip_bounds::<DummyShardPayload>();

    let payload = DummyShardPayload(vec![1, 2, 3]);
    let packed = payload.pack().expect("dummy payload packs");
    let unpacked = DummyShardPayload::unpack(&packed).expect("dummy payload unpacks");

    assert_eq!(unpacked, payload);
}
