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
            id: WorkloadId("smoke-tinystory-001".to_owned()),
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
        id: WorkloadId("smoke-tinystory-001".to_owned()),
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
    fn workload_manifest_ref_fixture_round_trips() {
        let manifest_ref = workload_manifest_ref_fixture();

        let encoded = serde_json::to_string(&manifest_ref).expect("manifest ref serializes");
        let decoded: WorkloadManifestRef =
            serde_json::from_str(&encoded).expect("manifest ref deserializes");

        assert_eq!(decoded, manifest_ref);
    }

    #[test]
    fn golden_vector_ref_fixture_round_trips() {
        let vector = golden_vector_ref_fixture();

        let encoded = serde_json::to_string(&vector).expect("golden vector ref serializes");
        let decoded: GoldenVectorRef =
            serde_json::from_str(&encoded).expect("golden vector ref deserializes");

        assert_eq!(decoded, vector);
    }
}
