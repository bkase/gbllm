use gbf_foundation::{BlobCodec, BlobRef, Hash256};
use gbf_workload::manifest::{
    GoldenVectorId, GoldenVectorRef, RegistryId, WorkloadFuturePlaceholder, WorkloadId,
    WorkloadLocator, WorkloadManifest, WorkloadManifestRef, WorkloadSchemaVersion,
};

const HASH_04: &str = "sha256:0404040404040404040404040404040404040404040404040404040404040404";
const HASH_08: &str = "sha256:0808080808080808080808080808080808080808080808080808080808080808";
const BLOB_HASH_0B: &str =
    "sha256:0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b";

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn golden_vector_ref(id: &str, manifest_hash: Hash256) -> GoldenVectorRef {
    GoldenVectorRef {
        id: GoldenVectorId(id.to_owned()),
        manifest_hash,
    }
}

fn workload_manifest(golden_vectors: Vec<GoldenVectorRef>) -> WorkloadManifest {
    WorkloadManifest {
        id: WorkloadId::from("smoke-tinystory-001"),
        schema_version: WorkloadSchemaVersion { epoch: 1, minor: 0 },
        self_hash: hash(8),
        golden_vectors,
        future_fields: WorkloadFuturePlaceholder::default(),
    }
}

#[test]
fn workload_manifest_round_trip_canonical_fixture() {
    let manifest = workload_manifest(Vec::new());

    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");
    let decoded: WorkloadManifest = serde_json::from_str(&encoded).expect("manifest deserializes");

    assert_eq!(decoded, manifest);
}

#[test]
fn workload_manifest_round_trip_with_golden_vectors() {
    let manifest = workload_manifest(vec![golden_vector_ref("vec.smoke.001", hash(4))]);

    assert_eq!(
        serde_json::to_value(&manifest).expect("manifest json value"),
        serde_json::json!({
            "id": "smoke-tinystory-001",
            "schema_version": { "epoch": 1, "minor": 0 },
            "self_hash": HASH_08,
            "golden_vectors": [
                {
                    "id": "vec.smoke.001",
                    "manifest_hash": HASH_04
                }
            ],
            "future_fields": {}
        })
    );

    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");
    let decoded: WorkloadManifest = serde_json::from_str(&encoded).expect("manifest deserializes");

    assert_eq!(decoded, manifest);
}

#[test]
fn workload_manifest_ref_round_trip() {
    let manifest_ref = WorkloadManifestRef {
        id: WorkloadId::from("smoke-tinystory-001"),
        manifest_hash: hash(8),
        locator: WorkloadLocator::Path {
            path: "fixtures/workloads/smoke-tinystory-001.workload.json".to_owned(),
        },
    };

    assert_eq!(
        serde_json::to_value(&manifest_ref).expect("manifest ref json value"),
        serde_json::json!({
            "id": "smoke-tinystory-001",
            "manifest_hash": HASH_08,
            "locator": {
                "kind": "Path",
                "path": "fixtures/workloads/smoke-tinystory-001.workload.json"
            }
        })
    );

    let encoded = serde_json::to_string(&manifest_ref).expect("manifest ref serializes");
    let decoded: WorkloadManifestRef =
        serde_json::from_str(&encoded).expect("manifest ref deserializes");

    assert_eq!(decoded, manifest_ref);
}

#[test]
fn workload_locator_round_trip_path_variant() {
    let locator = WorkloadLocator::Path {
        path: "fixtures/workloads/smoke-tinystory-001.workload.json".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&locator).expect("path locator json value"),
        serde_json::json!({
            "kind": "Path",
            "path": "fixtures/workloads/smoke-tinystory-001.workload.json"
        })
    );

    let encoded = serde_json::to_string(&locator).expect("locator serializes");
    let decoded: WorkloadLocator = serde_json::from_str(&encoded).expect("locator deserializes");

    assert_eq!(decoded, locator);
}

#[test]
fn workload_locator_round_trip_inline_variant() {
    let locator = WorkloadLocator::Inline {
        blob: BlobRef {
            hash: hash(11),
            len: 128,
            codec: BlobCodec::Raw,
        },
    };

    assert_eq!(
        serde_json::to_value(&locator).expect("inline locator json value"),
        serde_json::json!({
            "kind": "Inline",
            "blob": {
                "hash": BLOB_HASH_0B,
                "len": 128,
                "codec": "raw"
            }
        })
    );

    let encoded = serde_json::to_string(&locator).expect("locator serializes");
    let decoded: WorkloadLocator = serde_json::from_str(&encoded).expect("locator deserializes");

    assert_eq!(decoded, locator);
}

#[test]
fn workload_locator_round_trip_registry_variant() {
    let locator = WorkloadLocator::RegistryEntry {
        registry: RegistryId("fixture-registry".to_owned()),
        key: "smoke-tinystory-001".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&locator).expect("registry locator json value"),
        serde_json::json!({
            "kind": "RegistryEntry",
            "registry": "fixture-registry",
            "key": "smoke-tinystory-001"
        })
    );

    let encoded = serde_json::to_string(&locator).expect("locator serializes");
    let decoded: WorkloadLocator = serde_json::from_str(&encoded).expect("locator deserializes");

    assert_eq!(decoded, locator);
}

#[test]
fn workload_id_round_trip() {
    let id = WorkloadId::from("smoke-tinystory-001");

    let encoded = serde_json::to_string(&id).expect("workload id serializes");
    let decoded: WorkloadId = serde_json::from_str(&encoded).expect("workload id deserializes");

    assert_eq!(decoded, id);
    assert_eq!(encoded, "\"smoke-tinystory-001\"");
}

#[test]
fn workload_id_is_foundation_type_reexport() {
    let manifest_id = WorkloadId::from("smoke-tinystory-001");
    let foundation_id: gbf_foundation::WorkloadId = manifest_id;

    assert_eq!(foundation_id.as_str(), "smoke-tinystory-001");
}

#[test]
fn golden_vector_id_is_foundation_type_reexport() {
    let manifest_id = GoldenVectorId("vec.smoke.001".to_owned());
    let foundation_id: gbf_foundation::GoldenVectorId = manifest_id;

    assert_eq!(foundation_id.0, "vec.smoke.001");
}

#[test]
fn workload_schema_version_round_trip() {
    let version = WorkloadSchemaVersion { epoch: 1, minor: 7 };

    assert_eq!(
        serde_json::to_value(version).expect("schema version json value"),
        serde_json::json!({
            "epoch": 1,
            "minor": 7
        })
    );

    let encoded = serde_json::to_string(&version).expect("schema version serializes");
    let decoded: WorkloadSchemaVersion =
        serde_json::from_str(&encoded).expect("schema version deserializes");

    assert_eq!(decoded, version);
}

#[test]
fn golden_vector_id_round_trip() {
    let id = GoldenVectorId("vec.smoke.001".to_owned());

    let encoded = serde_json::to_string(&id).expect("golden vector id serializes");
    let decoded: GoldenVectorId =
        serde_json::from_str(&encoded).expect("golden vector id deserializes");

    assert_eq!(decoded, id);
    assert_eq!(encoded, "\"vec.smoke.001\"");
}

#[test]
fn golden_vector_ref_round_trip() {
    let vector = golden_vector_ref("vec.smoke.001", hash(4));

    assert_eq!(
        serde_json::to_value(&vector).expect("golden vector ref json value"),
        serde_json::json!({
            "id": "vec.smoke.001",
            "manifest_hash": HASH_04
        })
    );

    let encoded = serde_json::to_string(&vector).expect("golden vector ref serializes");
    let decoded: GoldenVectorRef =
        serde_json::from_str(&encoded).expect("golden vector ref deserializes");

    assert_eq!(decoded, vector);
}

#[test]
fn workload_manifest_rejects_unknown_top_level_field() {
    let mut value = serde_json::to_value(workload_manifest(Vec::new())).expect("json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<WorkloadManifest>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn workload_manifest_ref_rejects_unknown_field() {
    let mut value = serde_json::to_value(WorkloadManifestRef {
        id: WorkloadId::from("smoke-tinystory-001"),
        manifest_hash: hash(8),
        locator: WorkloadLocator::Path {
            path: "fixtures/workloads/smoke-tinystory-001.workload.json".to_owned(),
        },
    })
    .expect("json value");
    value["unexpected"] = serde_json::json!(true);

    let err =
        serde_json::from_value::<WorkloadManifestRef>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn workload_locator_rejects_unknown_kind() {
    let value = serde_json::json!({
        "kind": "OciImage",
        "image": "registry.example/workload:latest"
    });

    let err = serde_json::from_value::<WorkloadLocator>(value).expect_err("unknown kind rejects");

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn golden_vector_ref_rejects_unknown_field() {
    let mut value =
        serde_json::to_value(golden_vector_ref("vec.smoke.001", hash(4))).expect("json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<GoldenVectorRef>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn workload_future_placeholder_round_trips_empty() {
    let placeholder = WorkloadFuturePlaceholder::default();

    let encoded = serde_json::to_string(&placeholder).expect("placeholder serializes");
    let decoded: WorkloadFuturePlaceholder =
        serde_json::from_str(&encoded).expect("placeholder deserializes");

    assert_eq!(decoded, placeholder);
    assert_eq!(encoded, "{}");
}

#[test]
fn golden_vectors_preserve_declaration_order() {
    let manifest = workload_manifest(vec![
        golden_vector_ref("vec.smoke.002", hash(2)),
        golden_vector_ref("vec.smoke.001", hash(1)),
    ]);

    let encoded = serde_json::to_string(&manifest).expect("manifest serializes");
    let decoded: WorkloadManifest = serde_json::from_str(&encoded).expect("manifest deserializes");

    assert_eq!(decoded.golden_vectors, manifest.golden_vectors);
}

#[test]
fn workload_manifest_defaults_future_fields_when_missing() {
    let mut value = serde_json::to_value(workload_manifest(Vec::new())).expect("json value");
    value
        .as_object_mut()
        .expect("manifest object")
        .remove("future_fields");

    let decoded: WorkloadManifest =
        serde_json::from_value(value).expect("missing future fields defaults");

    assert_eq!(decoded.future_fields, WorkloadFuturePlaceholder::default());
}
