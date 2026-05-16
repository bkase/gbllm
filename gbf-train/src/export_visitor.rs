//! Frozen-teacher visitor for F-S3 reference bundle export.

use std::error::Error;
use std::fmt;

use gbf_artifact::{
    ArtifactAux, ArtifactCore, ArtifactError, ArtifactFeature, ArtifactManifest, CanonicalTensor,
    DecodeCapabilitySet, DecodeSpec, LexicalSpec_v1, LineageId, ManifestTimestamp, ModelArtifact,
    ModelSpec_S3, QuantSpec_S3, ReferenceBundleError, ReferenceGraphError, ReferenceLink,
    ReferenceManifest, ReferenceModelBundle, ReferenceModelSpec, ReferenceNumericProfile,
    ReferenceProgram, ReferenceTensor, SequenceSemanticsSpec, TargetDataLoweringArtifact,
    TextCharSeq, TiedEmbeddingAlias,
};
use gbf_foundation::{ArtifactSchemaVersion, Hash256, sha256};
use serde::{Deserialize, Serialize};

use crate::student::{FrozenStudent, HardTernaryStudentModel};
use crate::teacher::{DenseTeacherModel, FrozenTeacher};

/// Pinned visitor id written into `reference_manifest.v1` and `s3_bundle.v1`.
pub const EXPORT_VISITOR_ID: &str = "gbf-train.export_visitor.s3.reference_bundle.v1";

/// Stable source-identity preimage for the B13 export visitor.
pub const EXPORT_VISITOR_VERSION_PREIMAGE: &[u8] =
    b"gbf-train::export_visitor::s3_reference_bundle::v1";

/// Pinned visitor hash. Changes require an RFC/schema bump.
pub const EXPORT_VISITOR_VERSION_HASH: Hash256 = Hash256::from_bytes([
    0xad, 0x94, 0xa0, 0xf4, 0xef, 0x8a, 0x3d, 0x17, 0x59, 0x55, 0xb8, 0x66, 0xce, 0x91, 0xee, 0xfe,
    0x2c, 0x59, 0xe3, 0x21, 0x52, 0x86, 0x23, 0x71, 0x93, 0x5f, 0xb4, 0x91, 0xef, 0x52, 0xc7, 0xd4,
]);

/// Export visitor identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportVisitorId(String);

impl ExportVisitorId {
    /// Create a non-empty visitor id.
    pub fn new(value: impl Into<String>) -> Result<Self, ExportVisitorError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ExportVisitorError::EmptyVisitorId);
        }
        Ok(Self(value))
    }

    /// Borrow the stable id string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExportVisitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Visitor that lowers a frozen dense teacher into a reference model bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportVisitor {
    id: ExportVisitorId,
    version_hash: Hash256,
}

impl ExportVisitor {
    /// Create an export visitor with an explicit stable id and hash.
    pub fn new(id: ExportVisitorId, version_hash: Hash256) -> Self {
        Self { id, version_hash }
    }

    /// Return the pinned B13 visitor.
    pub fn pinned() -> Self {
        Self {
            id: ExportVisitorId::new(EXPORT_VISITOR_ID)
                .expect("pinned export visitor id is non-empty"),
            version_hash: EXPORT_VISITOR_VERSION_HASH,
        }
    }

    /// Visitor id written into exported bundle manifests.
    #[must_use]
    pub fn id(&self) -> &ExportVisitorId {
        &self.id
    }

    /// Visitor version hash written into exported bundle manifests.
    #[must_use]
    pub const fn version_hash(&self) -> Hash256 {
        self.version_hash
    }

    /// Visit a frozen teacher snapshot and produce a self-hashed bundle.
    pub fn visit_for_bundle<M>(
        &self,
        frozen: &FrozenTeacher<M>,
    ) -> Result<ReferenceModelBundle, ExportVisitorError>
    where
        M: ReferenceBundleExportModel,
    {
        let snapshot = frozen.snapshot();
        let manifest = ReferenceManifest::new(
            snapshot.reference_bundle_seed(),
            snapshot.reference_bundle_frozen_teacher_sha(frozen),
            snapshot.reference_bundle_sequence_semantics_hash(),
            self.id.as_str().to_owned(),
            self.version_hash,
        );

        ReferenceModelBundle::new(
            manifest,
            ReferenceNumericProfile::pinned(),
            snapshot.reference_bundle_lexical(),
            snapshot.reference_bundle_model(),
            snapshot.reference_bundle_program()?,
            snapshot.reference_bundle_tensors()?,
            DecodeSpec::argmax(),
            snapshot.reference_bundle_tied_embedding_alias(),
        )
        .map_err(ExportVisitorError::Bundle)
    }

    /// Visit a frozen hard-ternary student and produce a self-hashed artifact.
    pub fn visit_for_artifact<M>(
        &self,
        frozen: &FrozenStudent<M>,
    ) -> Result<ModelArtifact, ExportVisitorError>
    where
        M: ArtifactExportModel,
    {
        let snapshot = frozen.snapshot();
        let semantic_core_hash = snapshot.artifact_semantic_core_hash(frozen);
        let manifest = snapshot.artifact_manifest(frozen, semantic_core_hash);
        let core = ArtifactCore::new(
            manifest,
            snapshot.artifact_lexical(),
            snapshot.artifact_model(),
            snapshot.artifact_quant_spec(),
            snapshot.artifact_sequence_semantics(),
            snapshot.artifact_tensors()?,
            Vec::new(),
            snapshot.artifact_decode_caps(),
            snapshot.artifact_tied_embedding_alias(),
        )
        .map_err(ExportVisitorError::Artifact)?;
        ModelArtifact::new(
            core,
            snapshot.artifact_lowerings(),
            snapshot.artifact_aux(),
            snapshot.artifact_reference_link(),
        )
        .map_err(ExportVisitorError::Artifact)
    }
}

impl Default for ExportVisitor {
    fn default() -> Self {
        Self::pinned()
    }
}

/// Minimal model-owned bundle export surface used by B13.
///
/// `DenseTeacherModel` intentionally owns only freeze/forward invariants. Models
/// that can be exported as S3 reference bundles opt into this trait to expose
/// the canonical tensors, program, and tied-alias metadata without adding a
/// dependency from the freezer to any concrete Burn topology.
pub trait ReferenceBundleExportModel:
    DenseTeacherModel<Input = TextCharSeq, Output = Vec<f32>>
{
    /// Deterministic S3 seed recorded in the reference manifest.
    fn reference_bundle_seed(&self) -> u64;

    /// Stable source hash for the frozen teacher.
    fn reference_bundle_frozen_teacher_sha(&self, frozen: &FrozenTeacher<Self>) -> Hash256
    where
        Self: Sized,
    {
        sha256(frozen.weight_fingerprint().bytes())
    }

    /// Sequence-semantics hash paired with this export.
    fn reference_bundle_sequence_semantics_hash(&self) -> Hash256 {
        sha256(b"gbf:s3:sequence-semantics:charset-v1:argmax:last-token:v1")
    }

    /// Checkpoint/program schema hash for the reference program.
    fn reference_bundle_checkpoint_schema_hash(&self) -> Hash256 {
        sha256(b"gbf:s3:reference-program-checkpoint-schema:v1")
    }

    /// Lexical spec pinned for the exported bundle.
    fn reference_bundle_lexical(&self) -> LexicalSpec_v1 {
        LexicalSpec_v1::pinned()
    }

    /// Model shape metadata for the exported bundle.
    fn reference_bundle_model(&self) -> ReferenceModelSpec {
        ReferenceModelSpec::toy0()
    }

    /// Reference program for the frozen teacher.
    fn reference_bundle_program(&self) -> Result<ReferenceProgram, ExportVisitorError>;

    /// Canonical reference tensors for the frozen teacher.
    fn reference_bundle_tensors(&self) -> Result<Vec<ReferenceTensor>, ExportVisitorError>;

    /// Tied embedding/classifier alias metadata, if present.
    fn reference_bundle_tied_embedding_alias(&self) -> Option<TiedEmbeddingAlias> {
        None
    }
}

/// Minimal model-owned artifact export surface used by B14.
pub trait ArtifactExportModel: HardTernaryStudentModel {
    /// Deterministic S3 seed recorded in artifact metadata.
    fn artifact_seed(&self) -> u64;

    /// Stable semantic-core preimage hash for the frozen student.
    fn artifact_semantic_core_hash(&self, frozen: &FrozenStudent<Self>) -> Hash256
    where
        Self: Sized,
    {
        sha256(frozen.weight_fingerprint().bytes())
    }

    /// Artifact manifest paired with this export.
    fn artifact_manifest(
        &self,
        frozen: &FrozenStudent<Self>,
        semantic_core_hash: Hash256,
    ) -> ArtifactManifest
    where
        Self: Sized,
    {
        ArtifactManifest {
            components: vec![],
            created_at: ManifestTimestamp(0),
            lineage: LineageId(sha256(frozen.storage_fingerprint().bytes())),
            manifest_self_hash: Hash256::ZERO,
            required_features: std::collections::BTreeSet::from([
                ArtifactFeature::Ternary2Quant,
                ArtifactFeature::LinearStateSequence,
            ]),
            schema_version: ArtifactSchemaVersion { epoch: 3, minor: 0 },
            semantic_core_hash,
        }
    }

    /// Lexical spec pinned for the exported artifact.
    fn artifact_lexical(&self) -> LexicalSpec_v1 {
        LexicalSpec_v1::pinned()
    }

    /// Model shape metadata for the exported artifact.
    fn artifact_model(&self) -> ModelSpec_S3 {
        ModelSpec_S3::tiny(format!("toy0-student-seed-{}", self.artifact_seed()))
    }

    /// QuantSpec used to resolve all deployable weights.
    fn artifact_quant_spec(&self) -> QuantSpec_S3;

    /// Sequence semantics accepted by this artifact.
    fn artifact_sequence_semantics(&self) -> SequenceSemanticsSpec {
        SequenceSemanticsSpec::linear_state(4)
            .expect("pinned S3 linear-state sequence semantics are valid")
    }

    /// Canonical tensors emitted for the deployable artifact.
    fn artifact_tensors(&self) -> Result<Vec<CanonicalTensor>, ExportVisitorError>;

    /// Decode capabilities accepted by the artifact.
    fn artifact_decode_caps(&self) -> DecodeCapabilitySet {
        DecodeCapabilitySet::argmax_only()
    }

    /// Target-data lowerings; empty for S3 artifact export.
    fn artifact_lowerings(&self) -> Vec<TargetDataLoweringArtifact> {
        Vec::new()
    }

    /// Sparse mutable auxiliary sidecars.
    fn artifact_aux(&self) -> ArtifactAux {
        ArtifactAux::sparse()
    }

    /// Optional sibling reference link.
    fn artifact_reference_link(&self) -> Option<ReferenceLink> {
        None
    }

    /// Tied embedding/classifier alias metadata, if present.
    fn artifact_tied_embedding_alias(&self) -> Option<TiedEmbeddingAlias> {
        None
    }
}

/// Errors produced by B13 bundle export visiting.
#[derive(Debug)]
pub enum ExportVisitorError {
    /// Visitor id was empty.
    EmptyVisitorId,
    /// Bundle construction failed.
    Bundle(ReferenceBundleError),
    /// Artifact construction failed.
    Artifact(ArtifactError),
    /// Model-specific export failure.
    Model(String),
}

impl ExportVisitorError {
    /// Build a model-specific export error.
    #[must_use]
    pub fn model(message: impl Into<String>) -> Self {
        Self::Model(message.into())
    }
}

impl fmt::Display for ExportVisitorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyVisitorId => f.write_str("export visitor id must not be empty"),
            Self::Bundle(error) => write!(f, "{error}"),
            Self::Artifact(error) => write!(f, "{error}"),
            Self::Model(message) => f.write_str(message),
        }
    }
}

impl Error for ExportVisitorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Bundle(error) => Some(error),
            Self::Artifact(error) => Some(error),
            Self::EmptyVisitorId | Self::Model(_) => None,
        }
    }
}

impl From<ReferenceBundleError> for ExportVisitorError {
    fn from(error: ReferenceBundleError) -> Self {
        Self::Bundle(error)
    }
}

impl From<ReferenceGraphError> for ExportVisitorError {
    fn from(error: ReferenceGraphError) -> Self {
        Self::Model(error.to_string())
    }
}

impl From<ArtifactError> for ExportVisitorError {
    fn from(error: ArtifactError) -> Self {
        Self::Artifact(error)
    }
}
