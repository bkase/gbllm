//! S3 exported model artifact helpers.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use gbf_artifact::{
    Accumulator, ArtifactError, CanonicalIntegerThenScale, CanonicalTensor, CanonicalTensorId,
    CanonicalTensorSchemaError, ClassifierView, DecodeMode, Dtype, ModelArtifact, ModelSpec_S3,
    PayloadRole, Q8_8Scale, QuantSpec_S3, SequenceSemanticsSpec, TiedEmbeddingAlias, WeightQuant,
    canonical_artifact_bytes, canonical_payload_sha,
};
use gbf_foundation::{CanonicalJson, DomainHash, Hash256, sha256};
use gbf_train::export_visitor::{ArtifactExportModel, ExportVisitor, ExportVisitorError};
use gbf_train::student::{
    FrozenStudent, HardTernaryStudentModel, StudentStorageFingerprint, StudentWeightFingerprint,
    freeze_student_as_artifact,
};
use serde::Serialize;

use crate::s3::schema::{
    S3ArtifactMetadata, S3ArtifactSchemaError, S3ArtifactTiedEmbeddingAlias,
    S3ArtifactWeightResolutionSummary,
};

/// Tracing target used by B14 artifact export events.
pub const ARTIFACT_EXPORT_LOG_TARGET: &str = "gbf_experiments::s3::artifact";

/// Artifact export started event name.
pub const EVENT_NAME_ARTIFACT_EXPORT_STARTED: &str = "s3::artifact_export::started";
/// Artifact tensor emitted event name.
pub const EVENT_NAME_ARTIFACT_EXPORT_TENSOR_EMITTED: &str = "s3::artifact_export::tensor_emitted";
/// Artifact QuantSpec validation event name.
pub const EVENT_NAME_ARTIFACT_EXPORT_QUANTSPEC_VALIDATED: &str =
    "s3::artifact_export::quantspec_validated";
/// Artifact tied-alias event name.
pub const EVENT_NAME_ARTIFACT_EXPORT_TIED_ALIAS_RECORDED: &str =
    "s3::artifact_export::tied_alias_recorded";
/// Artifact export complete event name.
pub const EVENT_NAME_ARTIFACT_EXPORT_COMPLETE: &str = "s3::artifact_export::complete";

/// Inputs consumed by `s3_export_model_artifact`.
pub struct ArtifactExportInputs<'a, M>
where
    M: ArtifactExportModel,
{
    /// Frozen hard-ternary student snapshot produced after step 10000.
    pub frozen_student: &'a FrozenStudent<M>,
    /// Shared B13/B14 export visitor identity and lowering implementation.
    pub export_visitor: ExportVisitor,
}

impl<'a, M> ArtifactExportInputs<'a, M>
where
    M: ArtifactExportModel,
{
    /// Construct artifact export inputs.
    #[must_use]
    pub fn new(frozen_student: &'a FrozenStudent<M>, export_visitor: ExportVisitor) -> Self {
        Self {
            frozen_student,
            export_visitor,
        }
    }
}

/// Product returned after exporting and validating a model artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactExportProduct {
    /// Exported model artifact.
    pub artifact: ModelArtifact,
    /// Stored artifact self-hash.
    pub artifact_self_hash: Hash256,
    /// SHA-256 of canonical artifact bytes.
    pub canonical_artifact_payload_sha: Hash256,
    /// Canonical artifact bytes emitted by `CanonicalArtifactWrite`.
    pub canonical_artifact_bytes: Vec<u8>,
    /// Artifact validation report.
    pub artifact_validation: ArtifactValidationReport,
    /// Deployable byte total used by the Q6 chrome-budget gate.
    pub artifact_deployable_bytes: u64,
    /// `s3_artifact.v1` metadata record.
    pub metadata: S3ArtifactMetadata,
}

/// Artifact export validation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactValidationReport {
    /// QuantSpec resolution summary.
    pub weight_resolution_summary: S3ArtifactWeightResolutionSummary,
    /// Per-tensor resolution log.
    pub weight_resolution_log: Vec<WeightResolutionLogEntry>,
    /// True when tied embedding/classifier alias metadata is preserved.
    pub tied_embedding_alias_preserved: bool,
    /// True when every deployable weight resolved through QuantSpec.
    pub quant_spec_coverage_passed: bool,
}

/// Per-tensor QuantSpec resolution log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WeightResolutionLogEntry {
    /// Canonical tensor id.
    pub tensor_id: CanonicalTensorId,
    /// Payload role.
    pub payload_role: PayloadRole,
    /// Canonical tensor byte count.
    pub byte_count: u64,
    /// Weight quantization kind, or `not_applicable`.
    pub weight_quant_kind: String,
    /// Whether this tensor resolved through `QuantSpec_S3::weight_quant`.
    pub resolved_via_quant_spec: bool,
    /// Whether this tensor resolved through naming fallback.
    pub resolved_via_naming: bool,
}

/// Export a frozen hard-ternary student as an S3 `ModelArtifact`.
pub fn s3_export_model_artifact<M>(
    inputs: ArtifactExportInputs<'_, M>,
) -> Result<ArtifactExportProduct, ArtifactExportError>
where
    M: ArtifactExportModel,
{
    if inputs.frozen_student.requires_grad() {
        return Err(ArtifactExportError::FrozenStudentRequiresGrad);
    }

    let seed = inputs.frozen_student.snapshot().artifact_seed();
    let quant_spec = inputs.frozen_student.snapshot().artifact_quant_spec();
    let quant_spec_hash = quant_spec_hash(&quant_spec)?;
    tracing::info!(
        target: ARTIFACT_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_ARTIFACT_EXPORT_STARTED,
        seed = seed,
        frozen_student_storage_fingerprint = %inputs.frozen_student.storage_fingerprint().to_hex(),
        export_visitor_hash = %inputs.export_visitor.version_hash(),
        quant_spec_hash = %quant_spec_hash,
    );

    let artifact = inputs
        .export_visitor
        .visit_for_artifact(inputs.frozen_student)
        .map_err(map_visitor_error)?;
    artifact.validate().map_err(map_artifact_error)?;

    for tensor in &artifact.core.tensors {
        emit_tensor_emitted(tensor, &artifact);
    }

    let artifact_validation = validate_artifact_export(&artifact)?;
    tracing::info!(
        target: ARTIFACT_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_ARTIFACT_EXPORT_QUANTSPEC_VALIDATED,
        total_tensors = artifact_validation.weight_resolution_summary.total_tensors,
        tensors_resolved_via_quant_spec = artifact_validation
            .weight_resolution_summary
            .tensors_resolved_via_quant_spec,
        tensors_resolved_via_naming = artifact_validation
            .weight_resolution_summary
            .tensors_resolved_via_naming,
    );

    if let Some(alias) = &artifact.core.tied_embedding_alias {
        tracing::info!(
            target: ARTIFACT_EXPORT_LOG_TARGET,
            event_name = EVENT_NAME_ARTIFACT_EXPORT_TIED_ALIAS_RECORDED,
            embedding_canonical_id = alias.embedding_canonical_id.as_str(),
            classifier_canonical_id = alias.classifier_canonical_id.as_str(),
            shared = alias.shared,
            classifier_view = classifier_view_name(alias.classifier_view),
        );
    }

    let artifact_self_hash = artifact.artifact_self_hash;
    let computed_self_hash = artifact.compute_self_hash().map_err(map_artifact_error)?;
    if artifact_self_hash != computed_self_hash {
        return Err(ArtifactExportError::ArtifactSelfHashMismatch {
            stored: artifact_self_hash,
            computed: computed_self_hash,
        });
    }

    let canonical_artifact_bytes = canonical_artifact_bytes(&artifact);
    let canonical_artifact_payload_sha = canonical_payload_sha(&canonical_artifact_bytes);
    let artifact_deployable_bytes = artifact_deployable_bytes(&artifact)?;
    let metadata = S3ArtifactMetadata::new(
        seed,
        sha256(inputs.frozen_student.weight_fingerprint().bytes()),
        artifact.core.lexical.lexical_self_hash,
        quant_spec_hash,
        artifact
            .core
            .decode_caps
            .modes
            .iter()
            .copied()
            .collect::<Vec<DecodeMode>>(),
        inputs.export_visitor.id().as_str().to_owned(),
        inputs.export_visitor.version_hash(),
        artifact_self_hash,
        canonical_artifact_payload_sha,
        artifact.canonical_aux_payload_sha,
        artifact_deployable_bytes,
        artifact_validation.weight_resolution_summary.clone(),
        artifact
            .core
            .tied_embedding_alias
            .as_ref()
            .map(S3ArtifactTiedEmbeddingAlias::from),
    )
    .map_err(ArtifactExportError::Schema)?;

    tracing::info!(
        target: ARTIFACT_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_ARTIFACT_EXPORT_COMPLETE,
        seed = seed,
        artifact_self_hash = %artifact_self_hash,
        canonical_artifact_payload_sha = %canonical_artifact_payload_sha,
        artifact_deployable_bytes = artifact_deployable_bytes,
        tied_alias_present = artifact.core.tied_embedding_alias.is_some(),
    );

    Ok(ArtifactExportProduct {
        artifact,
        artifact_self_hash,
        canonical_artifact_payload_sha,
        canonical_artifact_bytes,
        artifact_validation,
        artifact_deployable_bytes,
        metadata,
    })
}

/// Export a deterministic in-repo fixture student for CLI smoke tests.
pub fn s3_export_fixture_model_artifact(
    seed: u64,
) -> Result<ArtifactExportProduct, ArtifactExportError> {
    let student = FixtureArtifactStudent::new(seed);
    let frozen = freeze_student_as_artifact(&student)
        .map_err(|error| ArtifactExportError::FreezeStudent(error.to_string()))?;
    s3_export_model_artifact(ArtifactExportInputs::new(&frozen, ExportVisitor::pinned()))
}

/// Compute the deployable byte total for an artifact.
pub fn artifact_deployable_bytes(artifact: &ModelArtifact) -> Result<u64, ArtifactExportError> {
    let tensor_bytes = artifact
        .core
        .tensors
        .iter()
        .filter(|tensor| {
            matches!(
                tensor.payload_role,
                PayloadRole::DeployableWeight | PayloadRole::DeployableQuantParam
            )
        })
        .try_fold(0_u64, |sum, tensor| {
            let len = tensor
                .byte_length()
                .map_err(ArtifactExportError::TensorByteLength)?;
            sum.checked_add(len)
                .ok_or(ArtifactExportError::DeployableByteOverflow)
        })?;
    let metadata_bytes = deployable_resolution_metadata_bytes(artifact)? as u64;
    tensor_bytes
        .checked_add(metadata_bytes)
        .ok_or(ArtifactExportError::DeployableByteOverflow)
}

/// Compute canonical metadata byte length needed for QuantSpec resolution.
pub fn deployable_resolution_metadata_bytes(
    artifact: &ModelArtifact,
) -> Result<usize, ArtifactExportError> {
    let mut weight_quant = BTreeMap::new();
    for tensor in artifact
        .core
        .tensors
        .iter()
        .filter(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
    {
        let quant = artifact
            .core
            .quant
            .weight_quant(&tensor.id)
            .ok_or_else(|| ArtifactExportError::QuantSpecCoverageMissing {
                tensor_id: tensor.id.clone(),
            })?;
        weight_quant.insert(tensor.id.clone(), *quant);
    }
    CanonicalJson::to_vec(&DeployableResolutionMetadata { weight_quant })
        .map(|bytes| bytes.len())
        .map_err(ArtifactExportError::CanonicalJson)
}

/// Validate QuantSpec coverage and tied-alias preservation.
pub fn validate_artifact_export(
    artifact: &ModelArtifact,
) -> Result<ArtifactValidationReport, ArtifactExportError> {
    let mut weight_resolution_log = Vec::new();
    let mut total_tensors = 0_u32;
    let mut tensors_resolved_via_quant_spec = 0_u32;

    for tensor in &artifact.core.tensors {
        let byte_count = tensor
            .byte_length()
            .map_err(ArtifactExportError::TensorByteLength)?;
        let mut resolved_via_quant_spec = false;
        let weight_quant_kind = if tensor.payload_role == PayloadRole::DeployableWeight {
            total_tensors = total_tensors
                .checked_add(1)
                .ok_or(ArtifactExportError::DeployableByteOverflow)?;
            let quant = artifact
                .core
                .quant
                .weight_quant(&tensor.id)
                .ok_or_else(|| ArtifactExportError::QuantSpecCoverageMissing {
                    tensor_id: tensor.id.clone(),
                })?;
            resolved_via_quant_spec = true;
            tensors_resolved_via_quant_spec = tensors_resolved_via_quant_spec
                .checked_add(1)
                .ok_or(ArtifactExportError::DeployableByteOverflow)?;
            weight_quant_kind(quant).to_owned()
        } else {
            "not_applicable".to_owned()
        };
        weight_resolution_log.push(WeightResolutionLogEntry {
            tensor_id: tensor.id.clone(),
            payload_role: tensor.payload_role,
            byte_count,
            weight_quant_kind,
            resolved_via_quant_spec,
            resolved_via_naming: false,
        });
    }

    let tied_embedding_alias_preserved =
        artifact
            .core
            .tied_embedding_alias
            .as_ref()
            .is_some_and(|alias| {
                alias.shared
                    && alias.embedding_canonical_id == alias.classifier_canonical_id
                    && artifact
                        .core
                        .tensors
                        .iter()
                        .any(|tensor| tensor.id == alias.embedding_canonical_id)
            });

    Ok(ArtifactValidationReport {
        weight_resolution_summary: S3ArtifactWeightResolutionSummary {
            total_tensors,
            tensors_resolved_via_quant_spec,
            tensors_resolved_via_naming: 0,
        },
        weight_resolution_log,
        tied_embedding_alias_preserved,
        quant_spec_coverage_passed: total_tensors == tensors_resolved_via_quant_spec,
    })
}

/// Write canonical artifact bytes and metadata JSON to disk.
pub fn write_artifact_export_product(
    artifact_path: &std::path::Path,
    metadata_path: &std::path::Path,
    product: &ArtifactExportProduct,
) -> Result<(), ArtifactExportError> {
    if let Some(parent) = artifact_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| ArtifactExportError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    if let Some(parent) = metadata_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| ArtifactExportError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(artifact_path, &product.canonical_artifact_bytes).map_err(|source| {
        ArtifactExportError::Io {
            path: artifact_path.display().to_string(),
            source,
        }
    })?;
    let metadata_bytes =
        CanonicalJson::to_vec(&product.metadata).map_err(ArtifactExportError::CanonicalJson)?;
    std::fs::write(metadata_path, metadata_bytes).map_err(|source| ArtifactExportError::Io {
        path: metadata_path.display().to_string(),
        source,
    })?;
    Ok(())
}

/// Errors produced by S3 artifact export.
#[derive(Debug)]
pub enum ArtifactExportError {
    /// Frozen student was not detached from gradients.
    FrozenStudentRequiresGrad,
    /// Fixture student freeze failed.
    FreezeStudent(String),
    /// The export visitor failed.
    Visitor(ExportVisitorError),
    /// Artifact construction failed.
    Artifact(ArtifactError),
    /// A deployable weight did not resolve through QuantSpec.
    QuantSpecCoverageMissing {
        /// Missing canonical tensor id.
        tensor_id: CanonicalTensorId,
    },
    /// Canonical tensor byte length failed.
    TensorByteLength(CanonicalTensorSchemaError),
    /// Deployable byte total overflowed u64.
    DeployableByteOverflow,
    /// Artifact self-hash did not recompute.
    ArtifactSelfHashMismatch {
        /// Stored hash in the artifact.
        stored: Hash256,
        /// Recomputed hash.
        computed: Hash256,
    },
    /// Canonical JSON or DomainHash construction failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    /// `s3_artifact.v1` metadata construction failed.
    Schema(S3ArtifactSchemaError),
    /// File IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
}

impl fmt::Display for ArtifactExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrozenStudentRequiresGrad => {
                f.write_str("frozen student still requires gradients")
            }
            Self::FreezeStudent(message) => write!(f, "student freeze failed: {message}"),
            Self::Visitor(error) => write!(f, "{error}"),
            Self::Artifact(error) => write!(f, "{error}"),
            Self::QuantSpecCoverageMissing { tensor_id } => {
                write!(f, "QuantSpec_S3 missing weight_quant entry for {tensor_id}")
            }
            Self::TensorByteLength(error) => write!(f, "{error}"),
            Self::DeployableByteOverflow => {
                f.write_str("artifact deployable byte total overflowed")
            }
            Self::ArtifactSelfHashMismatch { stored, computed } => write!(
                f,
                "artifact self-hash mismatch: stored {stored}, computed {computed}"
            ),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Io { path, source } => write!(f, "{path}: {source}"),
        }
    }
}

impl Error for ArtifactExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Visitor(error) => Some(error),
            Self::Artifact(error) => Some(error),
            Self::TensorByteLength(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::FrozenStudentRequiresGrad
            | Self::FreezeStudent(_)
            | Self::QuantSpecCoverageMissing { .. }
            | Self::DeployableByteOverflow
            | Self::ArtifactSelfHashMismatch { .. } => None,
        }
    }
}

impl From<S3ArtifactSchemaError> for ArtifactExportError {
    fn from(error: S3ArtifactSchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<gbf_foundation::CanonicalJsonError> for ArtifactExportError {
    fn from(error: gbf_foundation::CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct DeployableResolutionMetadata {
    weight_quant: BTreeMap<CanonicalTensorId, WeightQuant>,
}

fn map_visitor_error(error: ExportVisitorError) -> ArtifactExportError {
    match error {
        ExportVisitorError::Artifact(ArtifactError::QuantSpecCoverageMissing { tensor_id }) => {
            ArtifactExportError::QuantSpecCoverageMissing { tensor_id }
        }
        other => ArtifactExportError::Visitor(other),
    }
}

fn map_artifact_error(error: ArtifactError) -> ArtifactExportError {
    match error {
        ArtifactError::QuantSpecCoverageMissing { tensor_id } => {
            ArtifactExportError::QuantSpecCoverageMissing { tensor_id }
        }
        other => ArtifactExportError::Artifact(other),
    }
}

fn emit_tensor_emitted(tensor: &CanonicalTensor, artifact: &ModelArtifact) {
    let quant = artifact.core.quant.weight_quant(&tensor.id);
    let alias_target = alias_target_for(tensor, artifact);
    let byte_count = tensor.byte_length().unwrap_or(0);
    tracing::trace!(
        target: ARTIFACT_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_ARTIFACT_EXPORT_TENSOR_EMITTED,
        tensor_id = tensor.id.as_str(),
        payload_role = payload_role_name(tensor.payload_role),
        byte_count = byte_count,
        weight_quant_kind = quant.map(weight_quant_kind).unwrap_or("not_applicable"),
        alias_target = alias_target.unwrap_or(""),
        alias_target_present = alias_target.is_some(),
    );
}

fn alias_target_for<'a>(tensor: &CanonicalTensor, artifact: &'a ModelArtifact) -> Option<&'a str> {
    let alias = artifact.core.tied_embedding_alias.as_ref()?;
    (alias.shared && tensor.id == alias.embedding_canonical_id)
        .then_some(alias.classifier_canonical_id.as_str())
}

fn quant_spec_hash(quant: &QuantSpec_S3) -> Result<Hash256, ArtifactExportError> {
    DomainHash::new("gbf-experiments", "QuantSpec_S3", "s3_quant_spec.v1", "1")
        .hash(quant)
        .map_err(ArtifactExportError::CanonicalJson)
}

const fn payload_role_name(role: PayloadRole) -> &'static str {
    match role {
        PayloadRole::DeployableWeight => "deployable_weight",
        PayloadRole::DeployableQuantParam => "deployable_quant_param",
        PayloadRole::ReferenceFp32 => "reference_fp32",
    }
}

const fn weight_quant_kind(quant: &WeightQuant) -> &'static str {
    match quant {
        WeightQuant::Fp32 => "fp32",
        WeightQuant::Ternary2 { .. } => "ternary2",
    }
}

const fn classifier_view_name(view: ClassifierView) -> &'static str {
    match view {
        ClassifierView::SameTensor => "same_tensor",
        ClassifierView::TransposedView => "transposed_view",
    }
}

static NEXT_FIXTURE_STORAGE_ID: AtomicUsize = AtomicUsize::new(80_000);

#[derive(Clone, Debug, PartialEq, Eq)]
struct FixtureArtifactStudent {
    seed: u64,
    requires_grad: bool,
    storage_identity: usize,
}

impl FixtureArtifactStudent {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            requires_grad: true,
            storage_identity: next_fixture_storage_id(),
        }
    }

    fn fingerprint_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::from("s3-fixture-artifact-student:v1:");
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        for tensor in fixture_tensors(self.seed) {
            bytes.extend_from_slice(tensor.id.as_str().as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(tensor.payload_sha.as_bytes());
            bytes.push(0xff);
        }
        bytes
    }
}

impl HardTernaryStudentModel for FixtureArtifactStudent {
    fn detach_for_student(&mut self) {
        self.requires_grad = false;
        self.storage_identity = next_fixture_storage_id();
    }

    fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
        StudentWeightFingerprint::new(self.fingerprint_bytes()).expect("fixture fingerprint")
    }

    fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
        let mut bytes = Vec::from("s3-fixture-artifact-storage:v1:");
        bytes.extend_from_slice(&self.fingerprint_bytes());
        StudentStorageFingerprint::new(bytes).expect("fixture storage fingerprint")
    }

    fn student_storage_identity(&self) -> usize {
        self.storage_identity
    }

    fn student_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

impl ArtifactExportModel for FixtureArtifactStudent {
    fn artifact_seed(&self) -> u64 {
        self.seed
    }

    fn artifact_model(&self) -> ModelSpec_S3 {
        ModelSpec_S3::tiny(format!("fixture-artifact-student-{}", self.seed))
    }

    fn artifact_quant_spec(&self) -> QuantSpec_S3 {
        fixture_quant_spec(false)
    }

    fn artifact_sequence_semantics(&self) -> SequenceSemanticsSpec {
        SequenceSemanticsSpec::linear_state(4).expect("fixture sequence semantics")
    }

    fn artifact_tensors(&self) -> Result<Vec<CanonicalTensor>, ExportVisitorError> {
        Ok(fixture_tensors(self.seed))
    }

    fn artifact_tied_embedding_alias(&self) -> Option<TiedEmbeddingAlias> {
        Some(TiedEmbeddingAlias::new(
            tensor_id("tensor.embedding"),
            tensor_id("tensor.embedding"),
            true,
            ClassifierView::SameTensor,
        ))
    }
}

fn next_fixture_storage_id() -> usize {
    NEXT_FIXTURE_STORAGE_ID.fetch_add(1, Ordering::Relaxed)
}

fn fixture_tensors(seed: u64) -> Vec<CanonicalTensor> {
    vec![
        tensor(
            "tensor.embedding",
            Dtype::Ternary2,
            vec![80, 16],
            PayloadRole::DeployableWeight,
            seed,
        ),
        tensor(
            "tensor.linear.weight",
            Dtype::Ternary2,
            vec![16, 16],
            PayloadRole::DeployableWeight,
            seed,
        ),
        tensor(
            "tensor.linear.scale",
            Dtype::Q8_8,
            vec![16],
            PayloadRole::DeployableQuantParam,
            seed,
        ),
    ]
}

fn fixture_quant_spec(missing_linear: bool) -> QuantSpec_S3 {
    let mut weight_quant = BTreeMap::from([(
        tensor_id("tensor.embedding"),
        WeightQuant::Ternary2 {
            row_scale: Q8_8Scale(256),
            threshold: Q8_8Scale(32),
            accumulator: Accumulator::I32,
            reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
        },
    )]);
    if !missing_linear {
        weight_quant.insert(
            tensor_id("tensor.linear.weight"),
            WeightQuant::Ternary2 {
                row_scale: Q8_8Scale(192),
                threshold: Q8_8Scale(24),
                accumulator: Accumulator::I32,
                reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
            },
        );
    }
    QuantSpec_S3::new(weight_quant)
}

fn tensor(
    name: &str,
    dtype: Dtype,
    shape: Vec<u32>,
    payload_role: PayloadRole,
    seed: u64,
) -> CanonicalTensor {
    let mut preimage = Vec::from("s3-fixture-artifact-tensor:");
    preimage.extend_from_slice(name.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&seed.to_le_bytes());
    CanonicalTensor::new(
        tensor_id(name),
        dtype,
        shape,
        sha256(preimage),
        payload_role,
    )
    .expect("fixture tensor is valid")
}

fn tensor_id(value: &str) -> CanonicalTensorId {
    CanonicalTensorId::new(value).expect("fixture tensor id is valid")
}
