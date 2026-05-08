//! Manifest fixtures for artifact validation tests.

use std::collections::BTreeSet;

use gbf_artifact::{
    ArtifactFeature, ArtifactManifest, ArtifactSchemaVersion, ComponentId, ComponentKind,
    LineageId, ManifestComponent, ManifestTimestamp,
};
use gbf_foundation::Hash256;

pub struct ArtifactManifestBuilder {
    manifest: ArtifactManifest,
}

impl ArtifactManifestBuilder {
    /// Returns a minimal deterministic fixture with the manifest self-hash
    /// sentinel left in place for T-B2.5 to replace.
    #[must_use]
    pub fn canonical() -> Self {
        Self {
            manifest: ArtifactManifest {
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
            },
        }
    }

    #[must_use]
    pub fn with_required_features(
        mut self,
        features: impl IntoIterator<Item = ArtifactFeature>,
    ) -> Self {
        self.manifest.required_features = features.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_lineage(mut self, lineage: LineageId) -> Self {
        self.manifest.lineage = lineage;
        self
    }

    #[must_use]
    pub fn with_component(mut self, component: ManifestComponent) -> Self {
        self.manifest.components.push(component);
        self
    }

    #[must_use]
    pub fn with_self_hash(mut self, hash: Hash256) -> Self {
        self.manifest.manifest_self_hash = hash;
        self
    }

    #[must_use]
    pub fn build(self) -> ArtifactManifest {
        self.manifest
    }
}

#[must_use]
pub fn canonical_manifest_fixture() -> ArtifactManifest {
    ArtifactManifestBuilder::canonical().build()
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[cfg(test)]
fn hash_json(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

#[cfg(test)]
fn expected_canonical_fixture_json() -> String {
    format!(
        "{{\"components\":[{{\"digest\":\"{}\",\"id\":\"tensor.embed.weight\",\"kind\":{{\"kind\":\"CanonicalTensor\"}}}}],\"created_at\":0,\"lineage\":\"{}\",\"manifest_self_hash\":\"{}\",\"required_features\":[{{\"kind\":\"DenseI8\"}}],\"schema_version\":{{\"epoch\":1,\"minor\":0}},\"semantic_core_hash\":\"{}\"}}",
        hash_json(1),
        hash_json(2),
        hash_json(0),
        hash_json(3)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_canonical_matches_fixture_constant() {
        assert_eq!(
            ArtifactManifestBuilder::canonical().build(),
            canonical_manifest_fixture()
        );
    }

    #[test]
    fn canonical_fixture_byte_stable_round_trip() {
        let manifest = canonical_manifest_fixture();
        let expected = expected_canonical_fixture_json();
        let encoded = serde_json::to_string(&manifest).expect("manifest serializes");
        let decoded: ArtifactManifest =
            serde_json::from_str(&encoded).expect("manifest deserializes");

        assert_eq!(encoded, expected);
        assert_eq!(decoded, manifest);
        assert_eq!(
            serde_json::to_string(&decoded).expect("decoded manifest serializes"),
            expected
        );
    }

    #[test]
    fn builder_supports_with_required_features_chaining() {
        let manifest = ArtifactManifestBuilder::canonical()
            .with_required_features([
                ArtifactFeature::MoeRouting,
                ArtifactFeature::DenseI8,
                ArtifactFeature::BoundedKvSequence,
            ])
            .build();

        assert_eq!(
            manifest.required_features,
            BTreeSet::from([
                ArtifactFeature::DenseI8,
                ArtifactFeature::MoeRouting,
                ArtifactFeature::BoundedKvSequence,
            ])
        );
    }

    #[test]
    fn builder_supports_with_component_chaining() {
        let component = ManifestComponent {
            digest: hash(4),
            id: ComponentId("quant.spec".to_owned()),
            kind: ComponentKind::QuantSpec,
        };

        let manifest = ArtifactManifestBuilder::canonical()
            .with_component(component.clone())
            .build();

        assert_eq!(manifest.components.len(), 2);
        assert_eq!(manifest.components[1], component);
    }

    #[test]
    fn builder_supports_with_lineage_and_self_hash_chaining() {
        let lineage = LineageId(hash(0xab));
        let self_hash = hash(0xcd);

        let manifest = ArtifactManifestBuilder::canonical()
            .with_lineage(lineage.clone())
            .with_self_hash(self_hash)
            .build();

        assert_eq!(manifest.lineage, lineage);
        assert_eq!(manifest.manifest_self_hash, self_hash);
    }
}
