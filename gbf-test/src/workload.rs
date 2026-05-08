use gbf_foundation::Hash256;
use gbf_workload::manifest::{
    GoldenVectorId, GoldenVectorRef, RegistryId, WorkloadFuturePlaceholder, WorkloadId,
    WorkloadLocator, WorkloadManifest, WorkloadManifestRef, WorkloadSchemaVersion,
};

pub struct WorkloadManifestBuilder {
    id: WorkloadId,
    schema_version: WorkloadSchemaVersion,
    self_hash: Hash256,
    golden_vectors: Vec<GoldenVectorRef>,
    future_fields: WorkloadFuturePlaceholder,
}

impl WorkloadManifestBuilder {
    pub fn canonical() -> Self {
        Self {
            id: WorkloadId::from("smoke-tinystory-001"),
            schema_version: WorkloadSchemaVersion { epoch: 1, minor: 0 },
            self_hash: fixture_hash(8),
            golden_vectors: Vec::new(),
            future_fields: WorkloadFuturePlaceholder::default(),
        }
    }

    pub fn with_id(mut self, id: WorkloadId) -> Self {
        self.id = id;
        self
    }

    pub fn with_golden_vector(mut self, vector: GoldenVectorRef) -> Self {
        self.golden_vectors.push(vector);
        self
    }

    pub fn build(self) -> WorkloadManifest {
        WorkloadManifest {
            id: self.id,
            schema_version: self.schema_version,
            self_hash: self.self_hash,
            golden_vectors: self.golden_vectors,
            future_fields: self.future_fields,
        }
    }
}

pub fn canonical_workload_manifest_fixture() -> WorkloadManifest {
    WorkloadManifestBuilder::canonical().build()
}

pub fn workload_manifest_ref_fixture() -> WorkloadManifestRef {
    WorkloadManifestRef {
        id: WorkloadId::from("smoke-tinystory-001"),
        manifest_hash: fixture_hash(8),
        locator: WorkloadLocator::RegistryEntry {
            registry: RegistryId("fixture-registry".to_owned()),
            key: "smoke-tinystory-001".to_owned(),
        },
    }
}

pub fn golden_vector_ref_fixture() -> GoldenVectorRef {
    GoldenVectorRef {
        id: GoldenVectorId("vec.smoke.001".to_owned()),
        manifest_hash: fixture_hash(4),
    }
}

fn fixture_hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_04: &str = "sha256:0404040404040404040404040404040404040404040404040404040404040404";
    const HASH_08: &str = "sha256:0808080808080808080808080808080808080808080808080808080808080808";

    #[test]
    fn builder_canonical_matches_fixture_constant() {
        assert_eq!(
            WorkloadManifestBuilder::canonical().build(),
            canonical_workload_manifest_fixture()
        );
    }

    #[test]
    fn builder_supports_with_golden_vector_chaining() {
        let vector = golden_vector_ref_fixture();
        let manifest = WorkloadManifestBuilder::canonical()
            .with_golden_vector(vector.clone())
            .with_golden_vector(GoldenVectorRef {
                id: GoldenVectorId("vec.smoke.002".to_owned()),
                manifest_hash: Hash256::from_bytes([5; 32]),
            })
            .build();

        assert_eq!(manifest.golden_vectors[0], vector);
        assert_eq!(manifest.golden_vectors.len(), 2);
    }

    #[test]
    fn builder_supports_with_non_default_workload_id() {
        let id = WorkloadId::from("smoke-tinystory-alt");
        let manifest = WorkloadManifestBuilder::canonical()
            .with_id(id.clone())
            .build();

        assert_eq!(manifest.id, id);
        assert_eq!(
            serde_json::to_value(&manifest).expect("manifest json value")["id"],
            serde_json::json!("smoke-tinystory-alt")
        );
    }

    #[test]
    fn canonical_workload_manifest_fixture_shape_and_bytes_are_pinned() {
        let manifest = canonical_workload_manifest_fixture();
        let expected_value = serde_json::json!({
            "id": "smoke-tinystory-001",
            "schema_version": { "epoch": 1, "minor": 0 },
            "self_hash": HASH_08,
            "golden_vectors": [],
            "future_fields": {}
        });
        let expected_bytes = format!(
            r#"{{"id":"smoke-tinystory-001","schema_version":{{"epoch":1,"minor":0}},"self_hash":"{HASH_08}","golden_vectors":[],"future_fields":{{}}}}"#
        );

        let encoded = serde_json::to_string(&manifest).expect("manifest serializes");

        assert_eq!(
            serde_json::to_value(&manifest).expect("manifest json value"),
            expected_value
        );
        assert_eq!(encoded, expected_bytes);
        assert_eq!(
            serde_json::from_str::<WorkloadManifest>(&encoded).expect("manifest deserializes"),
            manifest
        );
    }

    #[test]
    fn workload_manifest_ref_fixture_round_trips() {
        let manifest_ref = workload_manifest_ref_fixture();

        let encoded = serde_json::to_string(&manifest_ref).expect("manifest ref serializes");
        let decoded: WorkloadManifestRef =
            serde_json::from_str(&encoded).expect("manifest ref deserializes");

        assert_eq!(decoded, manifest_ref);
    }

    #[test]
    fn workload_manifest_ref_fixture_shape_and_bytes_are_pinned() {
        let manifest_ref = workload_manifest_ref_fixture();
        let expected_value = serde_json::json!({
            "id": "smoke-tinystory-001",
            "manifest_hash": HASH_08,
            "locator": {
                "kind": "RegistryEntry",
                "registry": "fixture-registry",
                "key": "smoke-tinystory-001"
            }
        });
        let expected_bytes = format!(
            r#"{{"id":"smoke-tinystory-001","manifest_hash":"{HASH_08}","locator":{{"kind":"RegistryEntry","registry":"fixture-registry","key":"smoke-tinystory-001"}}}}"#
        );

        let encoded = serde_json::to_string(&manifest_ref).expect("manifest ref serializes");

        assert_eq!(
            serde_json::to_value(&manifest_ref).expect("manifest ref json value"),
            expected_value
        );
        assert_eq!(encoded, expected_bytes);
        assert_eq!(
            serde_json::from_str::<WorkloadManifestRef>(&encoded)
                .expect("manifest ref deserializes"),
            manifest_ref
        );
    }

    #[test]
    fn golden_vector_ref_fixture_round_trips() {
        let vector = golden_vector_ref_fixture();

        let encoded = serde_json::to_string(&vector).expect("golden vector ref serializes");
        let decoded: GoldenVectorRef =
            serde_json::from_str(&encoded).expect("golden vector ref deserializes");

        assert_eq!(decoded, vector);
    }

    #[test]
    fn golden_vector_ref_fixture_shape_and_bytes_are_pinned() {
        let vector = golden_vector_ref_fixture();
        let expected_value = serde_json::json!({
            "id": "vec.smoke.001",
            "manifest_hash": HASH_04
        });
        let expected_bytes = format!(r#"{{"id":"vec.smoke.001","manifest_hash":"{HASH_04}"}}"#);

        let encoded = serde_json::to_string(&vector).expect("golden vector ref serializes");

        assert_eq!(
            serde_json::to_value(&vector).expect("golden vector ref json value"),
            expected_value
        );
        assert_eq!(encoded, expected_bytes);
        assert_eq!(
            serde_json::from_str::<GoldenVectorRef>(&encoded)
                .expect("golden vector ref deserializes"),
            vector
        );
    }
}
