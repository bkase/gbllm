//! S3 reference model bundle helpers.

use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use gbf_artifact::{
    ActivationKind, ClassifierView, DecodeCapabilitySet, LexicalSpec_v1, ReferenceEdge,
    ReferenceEvalGraph, ReferenceManifest, ReferenceModelBundle, ReferenceModelSpec, ReferenceNode,
    ReferenceNumericProfile, ReferenceOp, ReferenceOpsetId, ReferenceProgram, ReferenceTensor,
    ReferenceTensorRole, TextCharSeq, TiedEmbeddingAlias, VOCAB_SIZE, canonical_bundle_bytes,
    canonical_payload_sha, evaluate_reference_program,
};
use gbf_foundation::{Hash256, sha256};
use gbf_train::export_visitor::{
    EXPORT_VISITOR_ID, EXPORT_VISITOR_VERSION_HASH, ExportVisitor, ExportVisitorError,
    ReferenceBundleExportModel,
};
use gbf_train::teacher::{
    DenseTeacherModel, FrozenTeacher, TeacherStorageFingerprint, TeacherStorageIdentity,
    TeacherWeightFingerprint, freeze_teacher,
};
use gbf_workload::WorkloadManifest_v0;

use crate::s3::schema::{
    S3BundleMetadata, S3BundleProgramValidation, S3BundleSchemaError, S3BundleTiedEmbeddingAlias,
};

/// Tracing target used by B13 bundle export events.
pub const BUNDLE_EXPORT_LOG_TARGET: &str = "gbf_experiments::s3::bundle";

/// Phase-A elementwise logit agreement tolerance for bundle validation.
pub const PHASE_A_LOGIT_TOLERANCE: f32 = 4.0e-6;

/// Number of v0_success prompt cases used by B13 semantic validation.
pub const BUNDLE_AGREEMENT_PROMPT_COUNT: usize = 3;

static NEXT_FIXTURE_STORAGE_ID: AtomicU64 = AtomicU64::new(70_000);

/// Bundle export started event name.
pub const EVENT_NAME_BUNDLE_EXPORT_STARTED: &str = "s3::bundle_export::started";
/// Bundle tensor emitted event name.
pub const EVENT_NAME_BUNDLE_EXPORT_TENSOR_EMITTED: &str = "s3::bundle_export::tensor_emitted";
/// Bundle program emitted event name.
pub const EVENT_NAME_BUNDLE_EXPORT_PROGRAM_EMITTED: &str = "s3::bundle_export::program_emitted";
/// Bundle program validated event name.
pub const EVENT_NAME_BUNDLE_EXPORT_PROGRAM_VALIDATED: &str = "s3::bundle_export::program_validated";
/// Bundle export complete event name.
pub const EVENT_NAME_BUNDLE_EXPORT_COMPLETE: &str = "s3::bundle_export::complete";

/// Inputs consumed by `s3_export_reference_bundle`.
pub struct BundleExportInputs<'a, M>
where
    M: ReferenceBundleExportModel,
{
    /// Frozen teacher snapshot produced by F-S2.
    pub frozen_teacher: &'a FrozenTeacher<M>,
    /// Export visitor identity and lowering implementation.
    pub export_visitor: ExportVisitor,
    /// Agreement prompts; the first three are used for semantic validation.
    pub agreement_prompts: Vec<TextCharSeq>,
}

impl<'a, M> BundleExportInputs<'a, M>
where
    M: ReferenceBundleExportModel,
{
    /// Construct bundle export inputs.
    #[must_use]
    pub fn new(
        frozen_teacher: &'a FrozenTeacher<M>,
        export_visitor: ExportVisitor,
        agreement_prompts: Vec<TextCharSeq>,
    ) -> Self {
        Self {
            frozen_teacher,
            export_visitor,
            agreement_prompts,
        }
    }

    /// Construct bundle export inputs from the v0_success agreement subset.
    #[must_use]
    pub fn from_workload_manifest(
        frozen_teacher: &'a FrozenTeacher<M>,
        export_visitor: ExportVisitor,
        workload: &WorkloadManifest_v0,
    ) -> Self {
        Self {
            frozen_teacher,
            export_visitor,
            agreement_prompts: workload
                .agreement_subset()
                .iter()
                .take(BUNDLE_AGREEMENT_PROMPT_COUNT)
                .map(|case| case.prompt_chars.clone())
                .collect(),
        }
    }
}

/// Product returned after exporting and validating a reference bundle.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleExportProduct {
    /// Exported reference bundle.
    pub bundle: ReferenceModelBundle,
    /// Program validation report.
    pub program_validation: ProgramValidationReport,
    /// Stored bundle self-hash.
    pub bundle_self_hash: Hash256,
    /// SHA-256 of canonical bundle bytes.
    pub canonical_bundle_payload_sha: Hash256,
    /// Canonical bundle bytes emitted by `CanonicalBundleWrite`.
    pub canonical_bundle_bytes: Vec<u8>,
    /// `s3_bundle.v1` metadata record.
    pub metadata: S3BundleMetadata,
}

impl BundleExportProduct {
    /// Total tensor payload byte count, counting each stored tensor once.
    #[must_use]
    pub fn total_tensor_payload_bytes(&self) -> u64 {
        self.bundle
            .tensors
            .iter()
            .map(|tensor| tensor.values.len() as u64 * 4)
            .sum()
    }
}

/// Structural and semantic validation for an exported bundle program.
///
/// The semantic check compares the exported reference program against the
/// `FrozenTeacher` interface provided by the caller. The in-repo CLI smoke uses
/// a deterministic toy teacher fixture, so that gate proves exporter wiring and
/// canonicalization rather than Burn-topology numerical parity.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramValidationReport {
    /// True when graph canonicalization and opset checks passed.
    pub structural_valid: bool,
    /// Maximum absolute logit difference against the live frozen teacher.
    pub semantic_max_logit_abs_diff: f32,
    /// True when every agreement prompt matched the live teacher argmax token.
    pub argmax_token_all_match: bool,
}

impl ProgramValidationReport {
    /// Return true when the report satisfies the B13 prompt subset contract.
    #[must_use]
    pub fn prompt_subset_pass(&self) -> bool {
        self.structural_valid
            && self.semantic_max_logit_abs_diff <= PHASE_A_LOGIT_TOLERANCE
            && self.argmax_token_all_match
    }

    fn as_schema_record(&self) -> S3BundleProgramValidation {
        S3BundleProgramValidation {
            structural_valid: self.structural_valid,
            semantic_max_logit_abs_diff: self.semantic_max_logit_abs_diff,
            argmax_token_all_match: self.argmax_token_all_match,
        }
    }
}

/// Export a frozen teacher as an S3 `ReferenceModelBundle`.
pub fn s3_export_reference_bundle<M>(
    inputs: BundleExportInputs<'_, M>,
) -> Result<BundleExportProduct, BundleExportError>
where
    M: ReferenceBundleExportModel,
    M::ForwardError: fmt::Display,
{
    if inputs.agreement_prompts.len() < BUNDLE_AGREEMENT_PROMPT_COUNT {
        return Err(BundleExportError::InsufficientAgreementPrompts {
            expected: BUNDLE_AGREEMENT_PROMPT_COUNT,
            actual: inputs.agreement_prompts.len(),
        });
    }

    let lexical_self_hash = inputs
        .frozen_teacher
        .snapshot()
        .reference_bundle_lexical()
        .lexical_self_hash;
    tracing::info!(
        target: BUNDLE_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_BUNDLE_EXPORT_STARTED,
        seed = inputs.frozen_teacher.snapshot().reference_bundle_seed(),
        frozen_teacher_storage_fingerprint = %inputs.frozen_teacher.storage_fingerprint().to_hex(),
        export_visitor_hash = %inputs.export_visitor.version_hash(),
        lexical_self_hash = %lexical_self_hash,
    );

    let bundle = inputs
        .export_visitor
        .visit_for_bundle(inputs.frozen_teacher)
        .map_err(BundleExportError::Visitor)?;

    for tensor in &bundle.tensors {
        emit_tensor_emitted(tensor, &bundle);
    }
    tracing::info!(
        target: BUNDLE_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_BUNDLE_EXPORT_PROGRAM_EMITTED,
        node_count = bundle.program.graph.nodes.len() as u64,
        edge_count = bundle.program.graph.edges.len() as u64,
        opset = opset_name(bundle.program.opset),
        checkpoint_schema_hash = %bundle.program.checkpoint_schema_hash,
    );

    let program_validation =
        validate_bundle_program(&bundle, inputs.frozen_teacher, &inputs.agreement_prompts)?;
    tracing::info!(
        target: BUNDLE_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_BUNDLE_EXPORT_PROGRAM_VALIDATED,
        structural_valid = program_validation.structural_valid,
        semantic_max_logit_abs_diff = f64::from(program_validation.semantic_max_logit_abs_diff),
        argmax_token_all_match = program_validation.argmax_token_all_match,
    );
    if !program_validation.prompt_subset_pass() {
        return Err(BundleExportError::ValidationFailed(program_validation));
    }

    let computed_self_hash = bundle.compute_self_hash();
    if computed_self_hash != bundle.bundle_self_hash {
        return Err(BundleExportError::BundleSelfHashMismatch {
            stored: bundle.bundle_self_hash,
            computed: computed_self_hash,
        });
    }

    let canonical_bundle_bytes = canonical_bundle_bytes(&bundle);
    let canonical_bundle_payload_sha = canonical_payload_sha(&canonical_bundle_bytes);
    let bundle_self_hash = bundle.bundle_self_hash;
    let metadata = S3BundleMetadata::new(
        bundle.manifest.seed,
        bundle.manifest.frozen_teacher_sha,
        bundle.lexical.lexical_self_hash,
        bundle.manifest.sequence_semantics_hash,
        DecodeCapabilitySet::argmax_only()
            .modes
            .into_iter()
            .collect(),
        bundle.manifest.export_visitor_id.clone(),
        bundle.manifest.export_visitor_hash,
        bundle_self_hash,
        canonical_bundle_payload_sha,
        program_validation.as_schema_record(),
        bundle
            .tied_embedding_alias
            .as_ref()
            .map(S3BundleTiedEmbeddingAlias::from),
    )
    .map_err(BundleExportError::Schema)?;

    tracing::info!(
        target: BUNDLE_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_BUNDLE_EXPORT_COMPLETE,
        seed = bundle.manifest.seed,
        bundle_self_hash = %bundle_self_hash,
        canonical_bundle_payload_sha = %canonical_bundle_payload_sha,
        tied_alias_present = bundle.tied_embedding_alias.is_some(),
    );

    Ok(BundleExportProduct {
        bundle,
        program_validation,
        bundle_self_hash,
        canonical_bundle_payload_sha,
        canonical_bundle_bytes,
        metadata,
    })
}

/// Export a deterministic in-repo toy reference bundle for CLI smoke tests.
///
/// This is intentionally a fixture path: it validates public CLI plumbing,
/// canonical bundle bytes, metadata, self-hashes, logging, and the
/// `ReferenceBundleExportModel` contract. It is not evidence of Burn-backed
/// teacher parity; real run export must pass a real frozen teacher into
/// `s3_export_reference_bundle`.
pub fn s3_export_fixture_reference_bundle(
    seed: u64,
) -> Result<BundleExportProduct, BundleExportError> {
    let teacher = FixtureBundleTeacher::new(seed);
    let frozen = freeze_teacher(&teacher)
        .map_err(|error| BundleExportError::FreezeTeacher(error.to_string()))?;
    s3_export_reference_bundle(BundleExportInputs::new(
        &frozen,
        ExportVisitor::pinned(),
        fixture_agreement_prompts(),
    ))
}

/// Write canonical bundle bytes and metadata JSON to disk.
pub fn write_bundle_export_product(
    bundle_path: &std::path::Path,
    metadata_path: &std::path::Path,
    product: &BundleExportProduct,
) -> Result<(), BundleExportError> {
    if let Some(parent) = bundle_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| BundleExportError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    if let Some(parent) = metadata_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| BundleExportError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(bundle_path, &product.canonical_bundle_bytes).map_err(|source| {
        BundleExportError::Io {
            path: bundle_path.display().to_string(),
            source,
        }
    })?;
    std::fs::write(
        metadata_path,
        product
            .metadata
            .canonical_json_bytes()
            .map_err(BundleExportError::Schema)?,
    )
    .map_err(|source| BundleExportError::Io {
        path: metadata_path.display().to_string(),
        source,
    })?;
    Ok(())
}

fn validate_bundle_program<M>(
    bundle: &ReferenceModelBundle,
    frozen_teacher: &FrozenTeacher<M>,
    agreement_prompts: &[TextCharSeq],
) -> Result<ProgramValidationReport, BundleExportError>
where
    M: ReferenceBundleExportModel,
    M::ForwardError: fmt::Display,
{
    let structural_valid = validate_program_structure(bundle);
    let mut semantic_max_logit_abs_diff = 0.0_f32;
    let mut argmax_token_all_match = true;

    for prompt in agreement_prompts.iter().take(BUNDLE_AGREEMENT_PROMPT_COUNT) {
        let reference = evaluate_reference_program(bundle, prompt, &());
        let live = frozen_teacher
            .forward_no_grad(prompt.clone())
            .map_err(|error| BundleExportError::TeacherForward(error.to_string()))?;
        if live.len() != reference.logits.len() {
            return Err(BundleExportError::LogitLengthMismatch {
                prompt_len: prompt.len(),
                expected: reference.logits.len(),
                actual: live.len(),
            });
        }

        for (left, right) in live.iter().zip(&reference.logits) {
            let diff = (*left - *right).abs();
            if !diff.is_finite() {
                return Err(BundleExportError::NonFiniteLogitDiff);
            }
            semantic_max_logit_abs_diff = semantic_max_logit_abs_diff.max(diff);
        }
        argmax_token_all_match &= argmax_lowest_index(&live) == reference.argmax_token;
    }

    Ok(ProgramValidationReport {
        structural_valid,
        semantic_max_logit_abs_diff,
        argmax_token_all_match,
    })
}

fn validate_program_structure(bundle: &ReferenceModelBundle) -> bool {
    if bundle.program.graph.nodes.is_empty() {
        return false;
    }
    if bundle
        .program
        .graph
        .nodes
        .iter()
        .any(|node| node.outputs.is_empty() || !opset_v1_covers(&node.op))
    {
        return false;
    }
    bundle.program.canonicalized().is_ok()
}

fn emit_tensor_emitted(tensor: &ReferenceTensor, bundle: &ReferenceModelBundle) {
    let alias_target = alias_target_for(tensor, bundle);
    tracing::trace!(
        target: BUNDLE_EXPORT_LOG_TARGET,
        event_name = EVENT_NAME_BUNDLE_EXPORT_TENSOR_EMITTED,
        tensor_id = tensor.id.as_str(),
        byte_count = tensor.values.len() as u64 * 4,
        role = tensor_role_name(tensor.role),
        alias_target = alias_target.unwrap_or(""),
        alias_target_present = alias_target.is_some(),
    );
}

fn alias_target_for<'a>(
    tensor: &ReferenceTensor,
    bundle: &'a ReferenceModelBundle,
) -> Option<&'a str> {
    let alias = bundle.tied_embedding_alias.as_ref()?;
    (alias.shared && tensor.id == alias.embedding_canonical_id)
        .then_some(alias.classifier_canonical_id.as_str())
}

fn argmax_lowest_index(values: &[f32]) -> u8 {
    values
        .iter()
        .enumerate()
        .max_by(|(left_index, left_value), (right_index, right_value)| {
            left_value
                .total_cmp(right_value)
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| u8::try_from(index).expect("S3 vocab index fits in u8"))
        .expect("logits must not be empty")
}

fn opset_name(opset: ReferenceOpsetId) -> &'static str {
    match opset {
        ReferenceOpsetId::OpsetV1 => "opset_v1",
    }
}

const fn opset_v1_covers(op: &ReferenceOp) -> bool {
    matches!(
        op,
        ReferenceOp::Linear
            | ReferenceOp::Embedding
            | ReferenceOp::Classifier
            | ReferenceOp::LinearStateBlock
            | ReferenceOp::Activation(_)
            | ReferenceOp::MatMul
            | ReferenceOp::Add
            | ReferenceOp::Mul
            | ReferenceOp::LayerNorm
            | ReferenceOp::Softmax
    )
}

const fn tensor_role_name(role: ReferenceTensorRole) -> &'static str {
    match role {
        ReferenceTensorRole::Embedding => "embedding",
        ReferenceTensorRole::Weight => "weight",
        ReferenceTensorRole::Bias => "bias",
        ReferenceTensorRole::Classifier => "classifier",
        ReferenceTensorRole::IntermediateFixture => "intermediate_fixture",
    }
}

/// Errors produced by S3 bundle export.
#[derive(Debug)]
pub enum BundleExportError {
    /// Fixture teacher freeze failed before bundle export.
    FreezeTeacher(String),
    /// Fewer than three agreement prompts were provided.
    InsufficientAgreementPrompts {
        /// Required prompt count.
        expected: usize,
        /// Observed prompt count.
        actual: usize,
    },
    /// The export visitor failed.
    Visitor(ExportVisitorError),
    /// Live teacher forward failed during semantic validation.
    TeacherForward(String),
    /// Live and reference logits had different lengths.
    LogitLengthMismatch {
        /// Prompt length that triggered the mismatch.
        prompt_len: usize,
        /// Expected reference logit length.
        expected: usize,
        /// Observed live logit length.
        actual: usize,
    },
    /// A logit difference was not finite.
    NonFiniteLogitDiff,
    /// Bundle self-hash did not recompute.
    BundleSelfHashMismatch {
        /// Stored hash in the bundle.
        stored: Hash256,
        /// Recomputed hash.
        computed: Hash256,
    },
    /// `s3_bundle.v1` metadata construction failed.
    Schema(S3BundleSchemaError),
    /// File IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// Program validation report failed the B13 prompt subset rule.
    ValidationFailed(ProgramValidationReport),
}

impl fmt::Display for BundleExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FreezeTeacher(message) => write!(f, "fixture teacher freeze failed: {message}"),
            Self::InsufficientAgreementPrompts { expected, actual } => {
                write!(
                    f,
                    "expected at least {expected} agreement prompts, got {actual}"
                )
            }
            Self::Visitor(error) => write!(f, "{error}"),
            Self::TeacherForward(message) => {
                write!(f, "live frozen teacher forward failed: {message}")
            }
            Self::LogitLengthMismatch {
                prompt_len,
                expected,
                actual,
            } => write!(
                f,
                "logit length mismatch for prompt length {prompt_len}: expected {expected}, got {actual}"
            ),
            Self::NonFiniteLogitDiff => f.write_str("non-finite logit difference"),
            Self::BundleSelfHashMismatch { stored, computed } => write!(
                f,
                "bundle self-hash mismatch: stored {stored}, computed {computed}"
            ),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::ValidationFailed(report) => write!(
                f,
                "program validation failed: structural_valid={}, max_diff={}, argmax_token_all_match={}",
                report.structural_valid,
                report.semantic_max_logit_abs_diff,
                report.argmax_token_all_match
            ),
        }
    }
}

impl Error for BundleExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Visitor(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::FreezeTeacher(_) => None,
            Self::InsufficientAgreementPrompts { .. }
            | Self::TeacherForward(_)
            | Self::LogitLengthMismatch { .. }
            | Self::NonFiniteLogitDiff
            | Self::BundleSelfHashMismatch { .. }
            | Self::ValidationFailed(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FixtureBundleTeacher {
    seed: u64,
    requires_grad: bool,
    storage_identity: u64,
}

impl FixtureBundleTeacher {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            requires_grad: true,
            storage_identity: next_fixture_storage_id(),
        }
    }

    fn build_bundle(&self) -> Result<ReferenceModelBundle, ExportVisitorError> {
        ReferenceModelBundle::new(
            ReferenceManifest::new(
                self.seed,
                sha256(self.fingerprint_bytes()),
                self.reference_bundle_sequence_semantics_hash(),
                EXPORT_VISITOR_ID,
                EXPORT_VISITOR_VERSION_HASH,
            ),
            ReferenceNumericProfile::pinned(),
            LexicalSpec_v1::pinned(),
            ReferenceModelSpec::toy0(),
            self.reference_bundle_program()?,
            self.reference_bundle_tensors()?,
            gbf_artifact::DecodeSpec::argmax(),
            self.reference_bundle_tied_embedding_alias(),
        )
        .map_err(ExportVisitorError::from)
    }

    fn fingerprint_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::from("s3-fixture-bundle-teacher:v1:");
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        for tensor in fixture_tensor_specs(self.seed) {
            bytes.extend_from_slice(tensor.id.as_str().as_bytes());
            bytes.push(0);
            for value in tensor.values {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            bytes.push(0xff);
        }
        bytes
    }
}

impl DenseTeacherModel for FixtureBundleTeacher {
    type ForwardError = FixtureBundleForwardError;
    type Input = TextCharSeq;
    type Output = Vec<f32>;

    fn detach_for_teacher(&mut self) {
        self.requires_grad = false;
        self.storage_identity = next_fixture_storage_id();
    }

    fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
        let bundle = self.build_bundle().map_err(|_| FixtureBundleForwardError)?;
        Ok(evaluate_reference_program(&bundle, &input, &()).logits)
    }

    fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
        TeacherWeightFingerprint::new(self.fingerprint_bytes()).expect("fixture fingerprint valid")
    }

    fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
        let mut bytes = Vec::from("s3-fixture-bundle-teacher-storage:f32:");
        bytes.extend_from_slice(&self.fingerprint_bytes());
        TeacherStorageFingerprint::new(bytes).expect("fixture storage fingerprint valid")
    }

    fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
        TeacherStorageIdentity::new(self.storage_identity.to_le_bytes().to_vec())
            .expect("fixture storage identity valid")
    }

    fn teacher_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

impl ReferenceBundleExportModel for FixtureBundleTeacher {
    fn reference_bundle_seed(&self) -> u64 {
        self.seed
    }

    fn reference_bundle_program(&self) -> Result<ReferenceProgram, ExportVisitorError> {
        ReferenceProgram::new(
            ReferenceEvalGraph::new(fixture_nodes(), fixture_edges())?,
            self.reference_bundle_checkpoint_schema_hash(),
        )
        .map_err(ExportVisitorError::from)
    }

    fn reference_bundle_tensors(&self) -> Result<Vec<ReferenceTensor>, ExportVisitorError> {
        Ok(fixture_tensor_specs(self.seed))
    }

    fn reference_bundle_tied_embedding_alias(&self) -> Option<TiedEmbeddingAlias> {
        Some(TiedEmbeddingAlias::new(
            tensor_ref("tensor.embedding"),
            tensor_ref("tensor.embedding"),
            true,
            ClassifierView::SameTensor,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FixtureBundleForwardError;

impl fmt::Display for FixtureBundleForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("fixture bundle forward failed")
    }
}

impl Error for FixtureBundleForwardError {}

fn fixture_agreement_prompts() -> Vec<TextCharSeq> {
    vec![
        TextCharSeq::new(vec![0, 1, 2]).expect("fixture prompt uses valid char ids"),
        TextCharSeq::new(vec![10, 11, 12, 13]).expect("fixture prompt uses valid char ids"),
        TextCharSeq::new(vec![30, 31, 32, 33, 34]).expect("fixture prompt uses valid char ids"),
    ]
}

fn fixture_tensor_specs(seed: u64) -> Vec<ReferenceTensor> {
    vec![
        ReferenceTensor::new(
            tensor_ref("tensor.embedding"),
            ReferenceTensorRole::Embedding,
            vec![VOCAB_SIZE as u32, 16],
            fixture_embedding_values(seed),
        )
        .expect("fixture embedding tensor valid"),
        ReferenceTensor::new(
            tensor_ref("tensor.linear.weight"),
            ReferenceTensorRole::Weight,
            vec![16, 16],
            fixture_linear_weight_values(seed),
        )
        .expect("fixture linear weight tensor valid"),
        ReferenceTensor::new(
            tensor_ref("tensor.linear.bias"),
            ReferenceTensorRole::Bias,
            vec![16],
            fixture_linear_bias_values(seed),
        )
        .expect("fixture linear bias tensor valid"),
        ReferenceTensor::new(
            tensor_ref("tensor.classifier.bias"),
            ReferenceTensorRole::Bias,
            vec![VOCAB_SIZE as u32],
            fixture_classifier_bias_values(seed),
        )
        .expect("fixture classifier bias tensor valid"),
    ]
}

fn fixture_embedding_values(seed: u64) -> Vec<f32> {
    let seed_offset = seed as f32 * 0.000_01;
    (0..VOCAB_SIZE)
        .flat_map(|row| {
            (0..16).map(move |col| {
                ((row as f32 - 40.0) * 0.001) + (col as f32 * 0.000_3) + seed_offset
            })
        })
        .collect()
}

fn fixture_linear_weight_values(seed: u64) -> Vec<f32> {
    let seed_offset = seed as f32 * 0.000_02;
    (0..16)
        .flat_map(|row| {
            (0..16).map(move |col| {
                if row == col {
                    0.75 + seed_offset
                } else {
                    ((row + col) as f32 % 5.0) * 0.000_2
                }
            })
        })
        .collect()
}

fn fixture_linear_bias_values(seed: u64) -> Vec<f32> {
    (0..16)
        .map(|index| index as f32 * 0.000_1 + seed as f32 * 0.000_01)
        .collect()
}

fn fixture_classifier_bias_values(seed: u64) -> Vec<f32> {
    (0..VOCAB_SIZE)
        .map(|index| index as f32 * 0.000_05 + seed as f32 * 0.000_01)
        .collect()
}

fn fixture_nodes() -> Vec<ReferenceNode> {
    vec![
        ReferenceNode::new(
            tensor_ref("op.embedding"),
            ReferenceOp::Embedding,
            vec![tensor_ref("tensor.embedding")],
            vec![tensor_ref("runtime.embedding")],
        ),
        ReferenceNode::new(
            tensor_ref("op.linear"),
            ReferenceOp::Linear,
            vec![
                tensor_ref("runtime.embedding"),
                tensor_ref("tensor.linear.weight"),
                tensor_ref("tensor.linear.bias"),
            ],
            vec![tensor_ref("runtime.hidden")],
        ),
        ReferenceNode::new(
            tensor_ref("op.activation"),
            ReferenceOp::Activation(ActivationKind::ReLU),
            vec![tensor_ref("runtime.hidden")],
            vec![tensor_ref("runtime.hidden_relu")],
        ),
        ReferenceNode::new(
            tensor_ref("op.classifier"),
            ReferenceOp::Classifier,
            vec![
                tensor_ref("runtime.hidden_relu"),
                tensor_ref("tensor.embedding"),
                tensor_ref("tensor.classifier.bias"),
            ],
            vec![tensor_ref("runtime.logits")],
        ),
    ]
}

fn fixture_edges() -> Vec<ReferenceEdge> {
    vec![
        ReferenceEdge::new(
            tensor_ref("op.embedding"),
            tensor_ref("op.linear"),
            tensor_ref("runtime.embedding"),
        ),
        ReferenceEdge::new(
            tensor_ref("op.linear"),
            tensor_ref("op.activation"),
            tensor_ref("runtime.hidden"),
        ),
        ReferenceEdge::new(
            tensor_ref("op.activation"),
            tensor_ref("op.classifier"),
            tensor_ref("runtime.hidden_relu"),
        ),
    ]
}

fn tensor_ref(value: &str) -> gbf_artifact::TensorRef {
    gbf_artifact::TensorRef::new(value).expect("fixture tensor ref valid")
}

fn next_fixture_storage_id() -> u64 {
    NEXT_FIXTURE_STORAGE_ID.fetch_add(1, Ordering::Relaxed)
}
