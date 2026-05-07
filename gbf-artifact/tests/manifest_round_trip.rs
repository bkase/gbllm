use std::collections::BTreeSet;

use gbf_artifact::{
    ArtifactFeature, ArtifactManifest, ArtifactSchemaVersion, ComponentId, ComponentKind,
    LineageId, ManifestComponent, ManifestInvariant, ManifestTimestamp,
};
use gbf_foundation::{FieldPath, Hash256};

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn hash_json(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn canonical_manifest_fixture() -> ArtifactManifest {
    ArtifactManifest {
        components: vec![ManifestComponent {
            digest: hash(1),
            id: ComponentId("tensor.embed.weight".to_owned()),
            kind: ComponentKind::CanonicalTensor,
        }],
        created_at: ManifestTimestamp(0),
        lineage: LineageId(hash(2)),
        manifest_self_hash: Hash256::ZERO,
        required_features: BTreeSet::from([ArtifactFeature::DenseI8]),
        schema_version: ArtifactSchemaVersion { epoch: 1, minor: 0 },
        semantic_core_hash: hash(3),
    }
}

fn expected_canonical_fixture_json() -> String {
    format!(
        "{{\"components\":[{{\"digest\":\"{}\",\"id\":\"tensor.embed.weight\",\"kind\":{{\"kind\":\"CanonicalTensor\"}}}}],\"created_at\":0,\"lineage\":\"{}\",\"manifest_self_hash\":\"{}\",\"required_features\":[{{\"kind\":\"DenseI8\"}}],\"schema_version\":{{\"epoch\":1,\"minor\":0}},\"semantic_core_hash\":\"{}\"}}",
        hash_json(1),
        hash_json(2),
        hash_json(0),
        hash_json(3)
    )
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
fn manifest_round_trip_canonical_fixture() {
    let manifest = canonical_manifest_fixture();
    assert_round_trip(&manifest);
    assert_eq!(
        serde_json::to_value(&manifest).expect("manifest serializes"),
        serde_json::json!({
            "components": [
                {
                    "digest": hash_json(1),
                    "id": "tensor.embed.weight",
                    "kind": {"kind": "CanonicalTensor"}
                }
            ],
            "created_at": 0,
            "lineage": hash_json(2),
            "manifest_self_hash": hash_json(0),
            "required_features": [
                {"kind": "DenseI8"}
            ],
            "schema_version": {"epoch": 1, "minor": 0},
            "semantic_core_hash": hash_json(3)
        })
    );
}

#[test]
fn manifest_round_trip_minimal() {
    let manifest = ArtifactManifest {
        components: Vec::new(),
        created_at: ManifestTimestamp(1),
        lineage: LineageId(hash(4)),
        manifest_self_hash: hash(5),
        required_features: BTreeSet::new(),
        schema_version: ArtifactSchemaVersion { epoch: 1, minor: 1 },
        semantic_core_hash: hash(6),
    };

    assert_round_trip(&manifest);
}

#[test]
fn manifest_invariant_round_trip_all_variants() {
    let component = ComponentId("tensor.embed.weight".to_owned());
    let variants = [
        ManifestInvariant::FeatureSetEpochInconsistent {
            epoch: ArtifactSchemaVersion { epoch: 1, minor: 0 },
            feature: ArtifactFeature::Ternary2Quant,
        },
        ManifestInvariant::RequiredComponentMissing {
            component: component.clone(),
        },
        ManifestInvariant::ComponentDigestMismatch {
            component,
            expected: hash(1),
            observed: hash(2),
        },
        ManifestInvariant::LineageContradiction {
            derived: LineageId(hash(3)),
            recorded: LineageId(hash(4)),
        },
        ManifestInvariant::ManifestSelfHashMismatch {
            recomputed: hash(5),
            recorded: hash(6),
        },
        ManifestInvariant::ForbiddenBuildIdentityField {
            field: FieldPath::from("/build_identity"),
        },
    ];

    for invariant in variants {
        assert_round_trip(&invariant);
    }
}

#[test]
fn artifact_feature_round_trip_all_variants() {
    let variants = [
        ArtifactFeature::DenseI8,
        ArtifactFeature::Ternary2Quant,
        ArtifactFeature::Binary1Quant,
        ArtifactFeature::SparseTernaryBitplanes,
        ArtifactFeature::MoeRouting,
        ArtifactFeature::LinearStateSequence,
        ArtifactFeature::BoundedKvSequence,
    ];

    for feature in variants {
        assert_round_trip(&feature);
    }
}

#[test]
fn component_kind_round_trip_all_variants() {
    let variants = [
        ComponentKind::CanonicalTensor,
        ComponentKind::QuantSpec,
        ComponentKind::NormPlan,
        ComponentKind::LutSpec,
        ComponentKind::SequenceSemantics,
        ComponentKind::DecodeSpec,
        ComponentKind::LexicalSpec,
        ComponentKind::InteractionBundle,
        ComponentKind::SemanticCheckpointSchema,
        ComponentKind::ConformanceEnvelope,
        ComponentKind::ReferenceObservationCache,
        ComponentKind::HintBundle,
    ];

    for kind in variants {
        assert_round_trip(&kind);
    }
}

#[test]
fn artifact_schema_version_round_trip() {
    let version = ArtifactSchemaVersion {
        epoch: 7,
        minor: 11,
    };

    assert_round_trip(&version);
    assert_eq!(
        serde_json::to_string(&version).expect("version serializes"),
        "{\"epoch\":7,\"minor\":11}"
    );
}

#[test]
fn lineage_id_serializes_as_hex_hash() {
    let lineage = LineageId(hash(0xab));

    assert_eq!(
        serde_json::to_string(&lineage).expect("lineage serializes"),
        format!("\"{}\"", hash_json(0xab))
    );
}

#[test]
fn component_id_serializes_as_string() {
    let id = ComponentId("tensor.embed.weight".to_owned());

    assert_eq!(
        serde_json::to_string(&id).expect("component id serializes"),
        "\"tensor.embed.weight\""
    );
}

#[test]
fn manifest_timestamp_serializes_as_u64() {
    let timestamp = ManifestTimestamp(42);

    assert_eq!(
        serde_json::to_string(&timestamp).expect("timestamp serializes"),
        "42"
    );
}

#[test]
fn manifest_rejects_unknown_top_level_field() {
    let mut value =
        serde_json::to_value(canonical_manifest_fixture()).expect("manifest serializes");
    value["unexpected"] = serde_json::json!(true);

    assert!(serde_json::from_value::<ArtifactManifest>(value).is_err());
}

#[test]
fn manifest_rejects_forbidden_build_identity_field() {
    let mut value =
        serde_json::to_value(canonical_manifest_fixture()).expect("manifest serializes");
    value["build_identity"] = serde_json::json!({"backend": "stage12"});

    assert!(serde_json::from_value::<ArtifactManifest>(value).is_err());
}

#[test]
fn manifest_rejects_unknown_component_field() {
    let json = format!(
        "{{\"digest\":\"{}\",\"id\":\"tensor.embed.weight\",\"kind\":{{\"kind\":\"CanonicalTensor\"}},\"extra\":0}}",
        hash_json(1)
    );

    assert!(serde_json::from_str::<ManifestComponent>(&json).is_err());
}

#[test]
fn manifest_rejects_unknown_invariant_kind() {
    let json = r#"{"kind":"FutureInvariant","field":"/build_identity"}"#;

    assert!(serde_json::from_str::<ManifestInvariant>(json).is_err());
}

#[test]
fn manifest_rejects_missing_required_field() {
    let mut value =
        serde_json::to_value(canonical_manifest_fixture()).expect("manifest serializes");
    value
        .as_object_mut()
        .expect("manifest is an object")
        .remove("manifest_self_hash");

    assert!(serde_json::from_value::<ArtifactManifest>(value).is_err());
}

#[test]
fn manifest_rejects_unknown_artifact_feature() {
    let json = r#"{"kind":"FutureFeature"}"#;

    assert!(serde_json::from_str::<ArtifactFeature>(json).is_err());
}

#[test]
fn required_features_serialize_in_sort_order() {
    let manifest = ArtifactManifest {
        required_features: BTreeSet::from([
            ArtifactFeature::BoundedKvSequence,
            ArtifactFeature::DenseI8,
            ArtifactFeature::Binary1Quant,
        ]),
        ..canonical_manifest_fixture()
    };
    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");

    let dense = encoded.find("\"DenseI8\"").expect("DenseI8 is present");
    let binary = encoded
        .find("\"Binary1Quant\"")
        .expect("Binary1Quant is present");
    let bounded = encoded
        .find("\"BoundedKvSequence\"")
        .expect("BoundedKvSequence is present");
    assert!(dense < binary);
    assert!(binary < bounded);
}

#[test]
fn components_preserve_declaration_order() {
    let manifest = ArtifactManifest {
        components: vec![
            ManifestComponent {
                digest: hash(1),
                id: ComponentId("first".to_owned()),
                kind: ComponentKind::CanonicalTensor,
            },
            ManifestComponent {
                digest: hash(2),
                id: ComponentId("second".to_owned()),
                kind: ComponentKind::QuantSpec,
            },
        ],
        ..canonical_manifest_fixture()
    };
    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");

    let first = encoded
        .find("\"first\"")
        .expect("first component is present");
    let second = encoded
        .find("\"second\"")
        .expect("second component is present");
    assert!(first < second);

    let decoded: ArtifactManifest = serde_json::from_str(&encoded).expect("manifest deserializes");
    assert_eq!(decoded.components[0].id, ComponentId("first".to_owned()));
    assert_eq!(decoded.components[1].id, ComponentId("second".to_owned()));
}

#[test]
fn canonical_fixture_byte_stable_round_trip() {
    let manifest = canonical_manifest_fixture();
    let expected = expected_canonical_fixture_json();
    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");
    let decoded: ArtifactManifest = serde_json::from_str(&encoded).expect("manifest deserializes");

    assert_eq!(encoded, expected);
    assert_eq!(decoded, manifest);
    assert_eq!(
        serde_json::to_string(&decoded).expect("decoded manifest serializes"),
        expected
    );
}
