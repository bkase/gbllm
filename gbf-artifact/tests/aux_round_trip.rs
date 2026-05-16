use gbf_artifact::aux::{
    ArtifactAux, ConformanceEnvelopeId, ConformanceEnvelopeRef, GoldenVectorId, GoldenVectorRef,
    InteractionBundleId, InteractionBundleRef, LexicalSpecId, LexicalSpecRef,
    ReferenceObservationCacheId, ReferenceObservationCacheRef, SemanticCheckpointSchemaId,
    SemanticCheckpointSchemaRef, SidecarKind,
};
use gbf_foundation::Hash256;

const HASH_04: &str = "sha256:0404040404040404040404040404040404040404040404040404040404040404";

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn hash_json(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn golden_vector_ref(id: &str, byte: u8) -> GoldenVectorRef {
    GoldenVectorRef {
        id: GoldenVectorId(id.to_owned()),
        manifest_hash: hash(byte),
    }
}

fn canonical_aux_fixture() -> ArtifactAux {
    ArtifactAux {
        checkpoint_schema: None,
        conformance_envelope: None,
        golden_vectors: vec![golden_vector_ref("vec.smoke.001", 4)],
        interaction_bundle: None,
        lexical_spec: None,
        reference_observation_cache: None,
    }
}

fn all_optional_aux_fixture() -> ArtifactAux {
    ArtifactAux {
        checkpoint_schema: Some(SemanticCheckpointSchemaRef {
            id: SemanticCheckpointSchemaId("checkpoint.schema.smoke".to_owned()),
            hash: hash(0x10),
        }),
        conformance_envelope: Some(ConformanceEnvelopeRef {
            id: ConformanceEnvelopeId("conformance.envelope.smoke".to_owned()),
            hash: hash(0x11),
        }),
        golden_vectors: vec![golden_vector_ref("vec.smoke.001", 4)],
        interaction_bundle: Some(InteractionBundleRef {
            id: InteractionBundleId("interaction.bundle.smoke".to_owned()),
            hash: hash(0x13),
        }),
        lexical_spec: Some(LexicalSpecRef {
            id: LexicalSpecId("lexical.spec.smoke".to_owned()),
            hash: hash(0x14),
        }),
        reference_observation_cache: Some(ReferenceObservationCacheRef {
            id: ReferenceObservationCacheId("reference.cache.smoke".to_owned()),
            hash: hash(0x12),
        }),
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
fn aux_round_trip_canonical_fixture() {
    let aux = canonical_aux_fixture();

    assert_round_trip(&aux);
    assert_eq!(
        serde_json::to_value(&aux).expect("aux json value"),
        serde_json::json!({
            "checkpoint_schema": null,
            "conformance_envelope": null,
            "golden_vectors": [
                {
                    "id": "vec.smoke.001",
                    "manifest_hash": HASH_04
                }
            ],
            "interaction_bundle": null,
            "lexical_spec": null,
            "reference_observation_cache": null
        })
    );
}

#[test]
fn aux_round_trip_with_all_optional_sidecars_present() {
    let aux = all_optional_aux_fixture();

    assert_round_trip(&aux);
    assert_eq!(
        serde_json::to_value(&aux).expect("aux json value"),
        serde_json::json!({
            "checkpoint_schema": {
                "id": "checkpoint.schema.smoke",
                "hash": hash_json(0x10)
            },
            "conformance_envelope": {
                "id": "conformance.envelope.smoke",
                "hash": hash_json(0x11)
            },
            "golden_vectors": [
                {
                    "id": "vec.smoke.001",
                    "manifest_hash": HASH_04
                }
            ],
            "interaction_bundle": {
                "id": "interaction.bundle.smoke",
                "hash": hash_json(0x13)
            },
            "lexical_spec": {
                "id": "lexical.spec.smoke",
                "hash": hash_json(0x14)
            },
            "reference_observation_cache": {
                "id": "reference.cache.smoke",
                "hash": hash_json(0x12)
            }
        })
    );
}

#[test]
fn sidecar_kind_round_trip_all_variants() {
    let variants = [
        (
            SidecarKind::GoldenVector,
            serde_json::json!({ "kind": "GoldenVector" }),
        ),
        (
            SidecarKind::SemanticCheckpointSchema,
            serde_json::json!({ "kind": "SemanticCheckpointSchema" }),
        ),
        (
            SidecarKind::ConformanceEnvelope,
            serde_json::json!({ "kind": "ConformanceEnvelope" }),
        ),
        (
            SidecarKind::ReferenceObservationCache,
            serde_json::json!({ "kind": "ReferenceObservationCache" }),
        ),
        (
            SidecarKind::InteractionBundle,
            serde_json::json!({ "kind": "InteractionBundle" }),
        ),
        (
            SidecarKind::LexicalSpec,
            serde_json::json!({ "kind": "LexicalSpec" }),
        ),
    ];

    for (kind, expected_json) in variants {
        assert_round_trip(&kind);
        assert_eq!(
            serde_json::to_value(kind).expect("sidecar kind json value"),
            expected_json
        );
        assert_eq!(
            serde_json::from_value::<SidecarKind>(expected_json)
                .expect("sidecar kind deserializes"),
            kind
        );
    }
}

#[test]
fn aux_rejects_unknown_top_level_field() {
    let mut value = serde_json::to_value(canonical_aux_fixture()).expect("aux json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<ArtifactAux>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn aux_rejects_unknown_sidecar_kind() {
    let json = r#"{"kind":"FutureSidecar"}"#;

    let err = serde_json::from_str::<SidecarKind>(json).expect_err("unknown kind rejects");

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn aux_rejects_extra_sidecar_kind_field() {
    let json = r#"{"kind":"GoldenVector","unexpected":true}"#;

    serde_json::from_str::<SidecarKind>(json).expect_err("extra field rejects");
}

#[test]
fn aux_rejects_missing_required_field() {
    let mut value = serde_json::to_value(canonical_aux_fixture()).expect("aux json value");
    value
        .as_object_mut()
        .expect("aux is an object")
        .remove("golden_vectors");

    let err = serde_json::from_value::<ArtifactAux>(value).expect_err("missing field rejects");

    assert!(err.to_string().contains("missing field"));
}

#[test]
fn sidecar_ref_structs_round_trip() {
    assert_round_trip(&SemanticCheckpointSchemaRef {
        id: SemanticCheckpointSchemaId("checkpoint.schema.smoke".to_owned()),
        hash: hash(0x10),
    });
    assert_round_trip(&ConformanceEnvelopeRef {
        id: ConformanceEnvelopeId("conformance.envelope.smoke".to_owned()),
        hash: hash(0x11),
    });
    assert_round_trip(&ReferenceObservationCacheRef {
        id: ReferenceObservationCacheId("reference.cache.smoke".to_owned()),
        hash: hash(0x12),
    });
    assert_round_trip(&InteractionBundleRef {
        id: InteractionBundleId("interaction.bundle.smoke".to_owned()),
        hash: hash(0x13),
    });
    assert_round_trip(&LexicalSpecRef {
        id: LexicalSpecId("lexical.spec.smoke".to_owned()),
        hash: hash(0x14),
    });
}

#[test]
fn sidecar_ref_id_serializes_as_string() {
    let id = SemanticCheckpointSchemaId("checkpoint.schema.smoke".to_owned());

    assert_eq!(
        serde_json::to_string(&id).expect("id serializes"),
        "\"checkpoint.schema.smoke\""
    );
}

#[test]
fn sidecar_ref_rejects_unknown_field() {
    let mut value = serde_json::to_value(SemanticCheckpointSchemaRef {
        id: SemanticCheckpointSchemaId("checkpoint.schema.smoke".to_owned()),
        hash: hash(0x10),
    })
    .expect("ref json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<SemanticCheckpointSchemaRef>(value)
        .expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn aux_with_no_golden_vectors_serializes_as_empty_array() {
    let aux = ArtifactAux {
        golden_vectors: Vec::new(),
        ..canonical_aux_fixture()
    };

    assert_eq!(
        serde_json::to_value(&aux).expect("aux json value")["golden_vectors"],
        serde_json::json!([])
    );

    let decoded: ArtifactAux =
        serde_json::from_str(&serde_json::to_string(&aux).expect("aux serializes"))
            .expect("aux deserializes");
    assert!(decoded.golden_vectors.is_empty());
}

#[test]
fn aux_re_exports_golden_vector_ref_from_foundation() {
    let foundation_ref = gbf_foundation::GoldenVectorRef {
        id: gbf_foundation::GoldenVectorId("vec.smoke.001".to_owned()),
        manifest_hash: hash(4),
    };

    let aux_ref: gbf_artifact::aux::GoldenVectorRef = foundation_ref.clone();
    let root_ref: gbf_artifact::GoldenVectorRef = aux_ref.clone();
    let foundation_ref_again: gbf_foundation::GoldenVectorRef = root_ref;

    assert_eq!(foundation_ref_again, foundation_ref);
}
