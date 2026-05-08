//! Workload manifest schema consumed by pipeline-entry validation.

use gbf_foundation::{BlobRef, Hash256};
pub use gbf_foundation::{GoldenVectorId, WorkloadId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadManifestRef {
    pub id: WorkloadId,
    pub manifest_hash: Hash256,
    pub locator: WorkloadLocator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WorkloadLocator {
    Path { path: String },
    Inline { blob: BlobRef },
    RegistryEntry { registry: RegistryId, key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadManifest {
    pub id: WorkloadId,
    pub schema_version: WorkloadSchemaVersion,
    pub self_hash: Hash256,
    pub golden_vectors: Vec<GoldenVectorRef>,
    #[serde(default)]
    pub future_fields: WorkloadFuturePlaceholder,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadFuturePlaceholder {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadSchemaVersion {
    pub epoch: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenVectorRef {
    pub id: GoldenVectorId,
    pub manifest_hash: Hash256,
}

/// Identifier scoped to workload registry namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistryId(pub String);
