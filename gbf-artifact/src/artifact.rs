//! F-S3 model artifact core schema.

use std::error::Error;
use std::fmt;

use gbf_foundation::{
    ArtifactFeature, ArtifactSchemaVersion, CanonicalJson, DomainHash, Hash256,
    canonical_json_bytes_omitting_fields, self_hash_omitting_fields, sha256,
};
use serde::{Deserialize, Serialize};

use crate::aux::ArtifactAux;
use crate::bundle::DecodeCapabilitySet;
use crate::canonical_artifact_write::canonical_artifact_bytes;
use crate::canonical_tensor::{CanonicalTensor, CanonicalTensorId, PayloadRole};
use crate::lexical_spec::LexicalSpec_v1;
use crate::lowerings::TargetDataLoweringArtifact;
use crate::manifest::{ArtifactManifest, ManifestTimestamp};
use crate::quant::QuantSpec_S3;
use crate::sequence::SequenceSemanticsSpec;
use crate::tied_alias::TiedEmbeddingAlias;
use crate::{ComponentId, LineageId};

const ARTIFACT_CORE_SCHEMA_ID: &str = "artifact_core.s3.v1";
const ARTIFACT_CORE_SCHEMA_VERSION: &str = "1";
const MODEL_ARTIFACT_SCHEMA_VERSION: &str = "1";

/// Schema id for the S3 model artifact object.
pub const MODEL_ARTIFACT_SCHEMA: &str = "model_artifact.s3.v1";

/// S3 model spec carried by `ArtifactCore`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec_S3 {
    pub model_id: String,
    pub vocab_size: u32,
    pub hidden_width: u32,
    pub layer_count: u32,
}

impl ModelSpec_S3 {
    #[must_use]
    pub fn tiny(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            vocab_size: 80,
            hidden_width: 16,
            layer_count: 1,
        }
    }
}

/// Placeholder logical LUT schema. S3 artifacts keep this vector empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalLutSpec {
    pub id: String,
}

/// Canonical artifact object exported by S3 for the Phase-D hard ternary student.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCore {
    pub manifest: ArtifactManifest,
    pub lexical: LexicalSpec_v1,
    pub model: ModelSpec_S3,
    pub quant: QuantSpec_S3,
    pub sequence: SequenceSemanticsSpec,
    pub tensors: Vec<CanonicalTensor>,
    pub luts: Vec<LogicalLutSpec>,
    pub decode_caps: DecodeCapabilitySet,
    pub tied_embedding_alias: Option<TiedEmbeddingAlias>,
}

impl ArtifactCore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest: ArtifactManifest,
        lexical: LexicalSpec_v1,
        model: ModelSpec_S3,
        quant: QuantSpec_S3,
        sequence: SequenceSemanticsSpec,
        mut tensors: Vec<CanonicalTensor>,
        mut luts: Vec<LogicalLutSpec>,
        decode_caps: DecodeCapabilitySet,
        tied_embedding_alias: Option<TiedEmbeddingAlias>,
    ) -> Result<Self, ArtifactError> {
        tensors.sort_by(|left, right| left.id.cmp(&right.id));
        luts.sort();
        verify_quant_tensor_references(&quant, &tensors)?;
        verify_deployable_weight_coverage(&quant, &tensors)?;

        tracing::debug!(
            target: "gbf_artifact::artifact",
            event_name = "s3::artifact::constructed",
            tensor_count = tensors.len(),
            weight_quant_coverage_count = quant.weight_quant.len(),
            tied_alias_present = tied_embedding_alias.is_some(),
        );

        Ok(Self {
            manifest,
            lexical,
            model,
            quant,
            sequence,
            tensors,
            luts,
            decode_caps,
            tied_embedding_alias,
        })
    }

    /// Canonical JSON bytes for deterministic S3 artifact-core identity tests.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        CanonicalJson::to_vec(self).map_err(ArtifactError::CanonicalJson)
    }

    /// DomainHash over this core's canonical JSON payload.
    pub fn compute_core_hash(&self) -> Result<Hash256, ArtifactError> {
        let canonical = self.canonical_bytes()?;
        Self::domain()
            .hash_canonical_bytes(&canonical)
            .map_err(ArtifactError::CanonicalJson)
    }

    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ArtifactCore",
            ARTIFACT_CORE_SCHEMA_ID,
            ARTIFACT_CORE_SCHEMA_VERSION,
        )
    }

    pub(crate) fn canonicalized_for_encoding(&self) -> Self {
        let mut clone = self.clone();
        clone.tensors.sort_by(|left, right| left.id.cmp(&right.id));
        clone.luts.sort();
        clone
    }
}

/// Optional link from an artifact to a sibling reference object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceLink {
    /// Human-readable reference relation tag.
    pub relation: String,
    /// Referenced artifact or bundle self hash.
    pub self_hash: Hash256,
}

impl ReferenceLink {
    /// Construct a non-empty reference link.
    pub fn new(relation: impl Into<String>, self_hash: Hash256) -> Result<Self, ArtifactError> {
        let relation = relation.into();
        if relation.trim().is_empty() {
            return Err(ArtifactError::EmptyReferenceRelation);
        }
        Ok(Self {
            relation,
            self_hash,
        })
    }
}

/// Canonical artifact exported by S3 for the Phase-D hard ternary student.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelArtifact {
    /// Pinned schema literal.
    pub schema: String,
    /// Immutable artifact core.
    pub core: ArtifactCore,
    /// Target-data lowerings; empty at S3.
    pub lowerings: Vec<TargetDataLoweringArtifact>,
    /// Mutable auxiliary sidecar references.
    pub aux: ArtifactAux,
    /// Optional sibling reference link.
    pub reference: Option<ReferenceLink>,
    /// Self-hash over the artifact with mutable aux sidecars omitted.
    pub artifact_self_hash: Hash256,
    /// SHA-256 of the canonical auxiliary sidecar reference payload.
    pub canonical_aux_payload_sha: Hash256,
}

impl ModelArtifact {
    /// Construct a model artifact and compute its canonical hashes.
    pub fn new(
        core: ArtifactCore,
        mut lowerings: Vec<TargetDataLoweringArtifact>,
        aux: ArtifactAux,
        reference: Option<ReferenceLink>,
    ) -> Result<Self, ArtifactError> {
        lowerings.sort_by(|left, right| {
            left.profile
                .cmp(&right.profile)
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.manifest_hash.cmp(&right.manifest_hash))
        });
        let canonical_aux_payload_sha = Self::compute_aux_payload_sha(&aux)?;
        let mut artifact = Self {
            schema: MODEL_ARTIFACT_SCHEMA.to_owned(),
            core,
            lowerings,
            aux,
            reference,
            artifact_self_hash: Hash256::ZERO,
            canonical_aux_payload_sha,
        };
        artifact.artifact_self_hash = artifact.compute_self_hash()?;
        Ok(artifact)
    }

    /// Build an empty deterministic S3 manifest for tests and fixture exports.
    #[must_use]
    pub fn fixture_manifest(seed: u64, semantic_core_hash: Hash256) -> ArtifactManifest {
        ArtifactManifest {
            components: vec![],
            created_at: ManifestTimestamp(0),
            lineage: LineageId(sha256(seed.to_le_bytes())),
            manifest_self_hash: Hash256::ZERO,
            required_features: std::collections::BTreeSet::from([
                ArtifactFeature::Ternary2Quant,
                ArtifactFeature::LinearStateSequence,
            ]),
            schema_version: ArtifactSchemaVersion { epoch: 3, minor: 0 },
            semantic_core_hash,
        }
    }

    /// Canonical JSON bytes including `artifact_self_hash`.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_artifact_bytes(self)
    }

    /// Canonical JSON bytes used for this artifact's self-hash.
    pub fn canonical_self_hash_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        canonical_json_bytes_omitting_fields(
            &self.canonicalized_for_encoding(),
            &["artifact_self_hash", "aux", "canonical_aux_payload_sha"],
        )
        .map_err(ArtifactError::CanonicalJson)
    }

    /// Compute the artifact self-hash with mutable aux sidecars omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, ArtifactError> {
        self.validate_without_self_hash()?;
        self_hash_omitting_fields(
            Self::domain(),
            &self.canonicalized_for_encoding(),
            "artifact_self_hash",
            &["aux", "canonical_aux_payload_sha"],
        )
        .map_err(ArtifactError::CanonicalJson)
    }

    /// Recompute and validate stored self-hash and aux payload hash.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        self.validate_without_self_hash()?;
        let expected_aux = Self::compute_aux_payload_sha(&self.aux)?;
        if self.canonical_aux_payload_sha != expected_aux {
            return Err(ArtifactError::AuxPayloadHashMismatch {
                expected: expected_aux,
                observed: self.canonical_aux_payload_sha,
            });
        }
        let expected_self_hash = self.compute_self_hash()?;
        if self.artifact_self_hash != expected_self_hash {
            return Err(ArtifactError::ArtifactSelfHashMismatch {
                expected: expected_self_hash,
                observed: self.artifact_self_hash,
            });
        }
        Ok(())
    }

    /// DomainHash context for `model_artifact.s3.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ModelArtifact",
            MODEL_ARTIFACT_SCHEMA,
            MODEL_ARTIFACT_SCHEMA_VERSION,
        )
    }

    pub(crate) fn canonicalized_for_encoding(&self) -> Self {
        let mut clone = self.clone();
        clone.core = clone.core.canonicalized_for_encoding();
        clone.lowerings.sort_by(|left, right| {
            left.profile
                .cmp(&right.profile)
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.manifest_hash.cmp(&right.manifest_hash))
        });
        clone
    }

    fn compute_aux_payload_sha(aux: &ArtifactAux) -> Result<Hash256, ArtifactError> {
        let bytes = CanonicalJson::to_vec(aux).map_err(ArtifactError::CanonicalJson)?;
        Ok(sha256(bytes))
    }

    fn validate_without_self_hash(&self) -> Result<(), ArtifactError> {
        if self.schema != MODEL_ARTIFACT_SCHEMA {
            return Err(ArtifactError::InvalidModelArtifactSchema {
                observed: self.schema.clone(),
            });
        }
        let mut components = std::collections::BTreeSet::new();
        for component in &self.core.manifest.components {
            if !components.insert(component.id.clone()) {
                return Err(ArtifactError::DuplicateManifestComponent {
                    id: component.id.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ArtifactError {
    InvalidModelArtifactSchema {
        observed: String,
    },
    EmptyReferenceRelation,
    DuplicateManifestComponent {
        id: ComponentId,
    },
    QuantSpecCoverageMissing {
        tensor_id: CanonicalTensorId,
    },
    QuantSpecReferencesMissingTensor {
        tensor_id: CanonicalTensorId,
    },
    DuplicateTensor {
        tensor_id: CanonicalTensorId,
    },
    ArtifactSelfHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    AuxPayloadHashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    CanonicalJson(gbf_foundation::CanonicalJsonError),
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModelArtifactSchema { observed } => {
                write!(
                    f,
                    "expected model artifact schema {MODEL_ARTIFACT_SCHEMA:?}, got {observed:?}"
                )
            }
            Self::EmptyReferenceRelation => f.write_str("reference relation must not be empty"),
            Self::DuplicateManifestComponent { id } => {
                write!(f, "artifact manifest contains duplicate component {}", id.0)
            }
            Self::QuantSpecCoverageMissing { tensor_id } => {
                write!(
                    f,
                    "ArtifactCore missing QuantSpec_S3 coverage for {tensor_id}"
                )
            }
            Self::QuantSpecReferencesMissingTensor { tensor_id } => {
                write!(
                    f,
                    "QuantSpec_S3 references tensor {tensor_id} not present in ArtifactCore"
                )
            }
            Self::DuplicateTensor { tensor_id } => {
                write!(f, "ArtifactCore contains duplicate tensor {tensor_id}")
            }
            Self::ArtifactSelfHashMismatch { expected, observed } => write!(
                f,
                "model artifact self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::AuxPayloadHashMismatch { expected, observed } => write!(
                f,
                "model artifact aux payload hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ArtifactError {}

fn verify_quant_tensor_references(
    quant: &QuantSpec_S3,
    tensors: &[CanonicalTensor],
) -> Result<(), ArtifactError> {
    let mut ids = std::collections::BTreeSet::new();
    for tensor in tensors {
        if !ids.insert(tensor.id.clone()) {
            return Err(ArtifactError::DuplicateTensor {
                tensor_id: tensor.id.clone(),
            });
        }
    }
    for tensor_id in quant.weight_quant.keys() {
        if !ids.contains(tensor_id) {
            return Err(ArtifactError::QuantSpecReferencesMissingTensor {
                tensor_id: tensor_id.clone(),
            });
        }
    }
    Ok(())
}

fn verify_deployable_weight_coverage(
    quant: &QuantSpec_S3,
    tensors: &[CanonicalTensor],
) -> Result<(), ArtifactError> {
    for tensor in tensors
        .iter()
        .filter(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
    {
        if !quant.weight_quant.contains_key(&tensor.id) {
            return Err(ArtifactError::QuantSpecCoverageMissing {
                tensor_id: tensor.id.clone(),
            });
        }
    }
    Ok(())
}
