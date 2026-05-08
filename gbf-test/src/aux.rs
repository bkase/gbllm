//! Artifact auxiliary sidecar fixtures.

use gbf_artifact::aux::*;
use gbf_foundation::Hash256;

pub struct ArtifactAuxBuilder {
    aux: ArtifactAux,
}

impl ArtifactAuxBuilder {
    #[must_use]
    pub fn canonical() -> Self {
        Self {
            aux: ArtifactAux {
                checkpoint_schema: None,
                conformance_envelope: None,
                golden_vectors: vec![golden_vector_ref()],
                interaction_bundle: None,
                lexical_spec: None,
                reference_observation_cache: None,
            },
        }
    }

    #[must_use]
    pub fn with_golden_vector(mut self, vector: GoldenVectorRef) -> Self {
        self.aux.golden_vectors.push(vector);
        self
    }

    #[must_use]
    pub fn with_checkpoint_schema(mut self, r: SemanticCheckpointSchemaRef) -> Self {
        self.aux.checkpoint_schema = Some(r);
        self
    }

    #[must_use]
    pub fn with_conformance_envelope(mut self, r: ConformanceEnvelopeRef) -> Self {
        self.aux.conformance_envelope = Some(r);
        self
    }

    #[must_use]
    pub fn with_reference_observation_cache(mut self, r: ReferenceObservationCacheRef) -> Self {
        self.aux.reference_observation_cache = Some(r);
        self
    }

    #[must_use]
    pub fn with_interaction_bundle(mut self, r: InteractionBundleRef) -> Self {
        self.aux.interaction_bundle = Some(r);
        self
    }

    #[must_use]
    pub fn with_lexical_spec(mut self, r: LexicalSpecRef) -> Self {
        self.aux.lexical_spec = Some(r);
        self
    }

    #[must_use]
    pub fn build(self) -> ArtifactAux {
        self.aux
    }
}

#[must_use]
pub fn canonical_aux_fixture() -> ArtifactAux {
    ArtifactAuxBuilder::canonical().build()
}

fn golden_vector_ref() -> GoldenVectorRef {
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
        let expected = ArtifactAux {
            checkpoint_schema: None,
            conformance_envelope: None,
            golden_vectors: vec![GoldenVectorRef {
                id: GoldenVectorId("vec.smoke.001".to_owned()),
                manifest_hash: Hash256::from_bytes([0x04; 32]),
            }],
            interaction_bundle: None,
            lexical_spec: None,
            reference_observation_cache: None,
        };

        assert_eq!(canonical_aux_fixture(), expected);
        assert_eq!(ArtifactAuxBuilder::canonical().build(), expected);
    }

    #[test]
    fn builder_appends_golden_vector_to_canonical_fixture() {
        let extra = GoldenVectorRef {
            id: GoldenVectorId("vec.smoke.extra".to_owned()),
            manifest_hash: fixture_hash(0x20),
        };
        let aux = ArtifactAuxBuilder::canonical()
            .with_golden_vector(extra.clone())
            .build();

        assert_eq!(
            aux.golden_vectors,
            vec![
                GoldenVectorRef {
                    id: GoldenVectorId("vec.smoke.001".to_owned()),
                    manifest_hash: fixture_hash(0x04),
                },
                extra,
            ]
        );
    }

    #[test]
    fn builder_supports_with_checkpoint_schema_chaining() {
        let checkpoint = SemanticCheckpointSchemaRef {
            id: SemanticCheckpointSchemaId("checkpoint.schema.smoke".to_owned()),
            hash: fixture_hash(0x10),
        };
        let aux = ArtifactAuxBuilder::canonical()
            .with_checkpoint_schema(checkpoint.clone())
            .build();

        assert_eq!(aux.checkpoint_schema, Some(checkpoint));
    }

    #[test]
    fn builder_supports_with_conformance_envelope_chaining() {
        let conformance = ConformanceEnvelopeRef {
            id: ConformanceEnvelopeId("conformance.envelope.smoke".to_owned()),
            hash: fixture_hash(0x11),
        };
        let aux = ArtifactAuxBuilder::canonical()
            .with_conformance_envelope(conformance.clone())
            .build();

        assert_eq!(aux.conformance_envelope, Some(conformance));
    }

    #[test]
    fn builder_supports_with_reference_observation_cache_chaining() {
        let reference_cache = ReferenceObservationCacheRef {
            id: ReferenceObservationCacheId("reference.cache.smoke".to_owned()),
            hash: fixture_hash(0x12),
        };
        let aux = ArtifactAuxBuilder::canonical()
            .with_reference_observation_cache(reference_cache.clone())
            .build();

        assert_eq!(aux.reference_observation_cache, Some(reference_cache));
    }

    #[test]
    fn builder_supports_with_interaction_bundle_chaining() {
        let interaction = InteractionBundleRef {
            id: InteractionBundleId("interaction.bundle.smoke".to_owned()),
            hash: fixture_hash(0x13),
        };
        let aux = ArtifactAuxBuilder::canonical()
            .with_interaction_bundle(interaction.clone())
            .build();

        assert_eq!(aux.interaction_bundle, Some(interaction));
    }

    #[test]
    fn builder_supports_with_lexical_spec_chaining() {
        let lexical = LexicalSpecRef {
            id: LexicalSpecId("lexical.spec.smoke".to_owned()),
            hash: fixture_hash(0x14),
        };
        let aux = ArtifactAuxBuilder::canonical()
            .with_lexical_spec(lexical.clone())
            .build();

        assert_eq!(aux.lexical_spec, Some(lexical));
    }
}
