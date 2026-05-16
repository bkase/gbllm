//! F-S3 reference model bundle schema and deterministic self-hash support.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_foundation::{DomainHash, Hash256, self_hash_omitting_fields, sha256};
use gbf_policy::model_profile::ModelSizeProfile;
use serde::{Deserialize, Serialize};

use crate::canonical_bundle_write::canonical_bundle_bytes;
use crate::lexical::VOCAB_SIZE;
use crate::lexical_spec::LexicalSpec_v1;
use crate::opset_v1::ReferenceOpsetId;
use crate::reference_eval_graph::{ReferenceEvalGraph, ReferenceGraphError, TensorRef};
use crate::tied_alias::TiedEmbeddingAlias;

/// Bundle schema id pinned by the S3 reference bundle contract.
pub const REFERENCE_MODEL_BUNDLE_SCHEMA: &str = "reference_model_bundle.v1";

/// Manifest schema id for the bundle-local manifest block.
pub const REFERENCE_MANIFEST_SCHEMA: &str = "reference_manifest.v1";

const REFERENCE_MODEL_BUNDLE_SCHEMA_VERSION: &str = "1";

/// Bundle-local manifest metadata. Export/report metadata remains owned by B13.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceManifest {
    pub schema: String,
    pub seed: u64,
    pub frozen_teacher_sha: Hash256,
    pub sequence_semantics_hash: Hash256,
    pub export_visitor_id: String,
    pub export_visitor_hash: Hash256,
}

impl ReferenceManifest {
    #[must_use]
    pub fn new(
        seed: u64,
        frozen_teacher_sha: Hash256,
        sequence_semantics_hash: Hash256,
        export_visitor_id: impl Into<String>,
        export_visitor_hash: Hash256,
    ) -> Self {
        Self {
            schema: REFERENCE_MANIFEST_SCHEMA.to_owned(),
            seed,
            frozen_teacher_sha,
            sequence_semantics_hash,
            export_visitor_id: export_visitor_id.into(),
            export_visitor_hash,
        }
    }
}

/// Model shape metadata needed by the B4 reference program surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceModelSpec {
    pub profile: ModelSizeProfile,
    pub vocab_size: u32,
    pub d_model: u32,
    pub d_ff: u32,
    pub n_blocks: u32,
}

impl ReferenceModelSpec {
    #[must_use]
    pub fn toy0() -> Self {
        Self {
            profile: ModelSizeProfile::toy0(),
            vocab_size: VOCAB_SIZE as u32,
            d_model: u32::from(ModelSizeProfile::TOY0_D_MODEL),
            d_ff: u32::from(ModelSizeProfile::TOY0_D_FF),
            n_blocks: u32::from(ModelSizeProfile::TOY0_N_BLOCKS),
        }
    }
}

/// Reference program pinned to `opset_v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceProgram {
    pub opset: ReferenceOpsetId,
    pub graph: ReferenceEvalGraph,
    pub checkpoint_schema_hash: Hash256,
}

impl ReferenceProgram {
    pub fn new(
        graph: ReferenceEvalGraph,
        checkpoint_schema_hash: Hash256,
    ) -> Result<Self, ReferenceGraphError> {
        Ok(Self {
            opset: ReferenceOpsetId::OpsetV1,
            graph: graph.canonicalized()?,
            checkpoint_schema_hash,
        })
    }

    pub fn canonicalized(&self) -> Result<Self, ReferenceGraphError> {
        Ok(Self {
            opset: self.opset,
            graph: self.graph.canonicalized()?,
            checkpoint_schema_hash: self.checkpoint_schema_hash,
        })
    }
}

/// Numeric profile pinned by F-S3 bundle evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceNumericProfile {
    pub scalar_format: ReferenceScalarFormat,
    pub reduction_order: Option<ReductionOrderCanonical>,
    pub reduction_order_policy: ReductionOrderPolicy,
    pub rng: ReferenceRngProfile,
    pub determinism: ReferenceDeterminism,
}

impl ReferenceNumericProfile {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            scalar_format: ReferenceScalarFormat::F32,
            reduction_order: Some(ReductionOrderCanonical::Canonical),
            reduction_order_policy: ReductionOrderPolicy::Enforced,
            rng: ReferenceRngProfile::NoRng,
            determinism: ReferenceDeterminism::BitExact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceScalarFormat {
    F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReductionOrderCanonical {
    Canonical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReductionOrderPolicy {
    Enforced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRngProfile {
    NoRng,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceDeterminism {
    BitExact,
}

/// Bundle tensor payload used by the B4 pure interpreter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceTensor {
    pub id: TensorRef,
    pub role: ReferenceTensorRole,
    pub shape: Vec<u32>,
    pub values: Vec<f32>,
    pub tensor_hash: Hash256,
}

impl ReferenceTensor {
    pub fn new(
        id: TensorRef,
        role: ReferenceTensorRole,
        shape: Vec<u32>,
        values: Vec<f32>,
    ) -> Result<Self, ReferenceTensorError> {
        validate_tensor_shape_and_values(&id, &shape, &values)?;
        let tensor_hash = reference_tensor_hash(&id, role, &shape, &values);
        Ok(Self {
            id,
            role,
            shape,
            values,
            tensor_hash,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceTensorRole {
    Embedding,
    Weight,
    Bias,
    Classifier,
    IntermediateFixture,
}

impl ReferenceTensorRole {
    const fn stable_name(self) -> &'static str {
        match self {
            Self::Embedding => "embedding",
            Self::Weight => "weight",
            Self::Bias => "bias",
            Self::Classifier => "classifier",
            Self::IntermediateFixture => "intermediate_fixture",
        }
    }
}

/// Decode mode pinned by the S3 reference bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodeMode {
    Argmax,
}

/// Concrete decode spec for program evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeSpec {
    pub mode: DecodeMode,
}

impl DecodeSpec {
    #[must_use]
    pub const fn argmax() -> Self {
        Self {
            mode: DecodeMode::Argmax,
        }
    }
}

/// Capability-set wrapper used by S3 export inputs and metadata checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecodeCapabilitySet {
    pub modes: BTreeSet<DecodeMode>,
}

impl DecodeCapabilitySet {
    #[must_use]
    pub fn argmax_only() -> Self {
        Self {
            modes: BTreeSet::from([DecodeMode::Argmax]),
        }
    }
}

/// S3 reference model bundle. Tied alias semantics are represented by B5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceModelBundle {
    pub schema: String,
    pub manifest: ReferenceManifest,
    pub numeric: ReferenceNumericProfile,
    pub lexical: LexicalSpec_v1,
    pub model: ReferenceModelSpec,
    pub program: ReferenceProgram,
    pub tensors: Vec<ReferenceTensor>,
    pub decode: DecodeSpec,
    pub tied_embedding_alias: Option<TiedEmbeddingAlias>,
    pub bundle_self_hash: Hash256,
}

impl ReferenceModelBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest: ReferenceManifest,
        numeric: ReferenceNumericProfile,
        lexical: LexicalSpec_v1,
        model: ReferenceModelSpec,
        program: ReferenceProgram,
        tensors: Vec<ReferenceTensor>,
        decode: DecodeSpec,
        tied_embedding_alias: Option<TiedEmbeddingAlias>,
    ) -> Result<Self, ReferenceBundleError> {
        let program = program.canonicalized()?;
        let tensors = canonicalize_tensors(tensors)?;
        let mut bundle = Self {
            schema: REFERENCE_MODEL_BUNDLE_SCHEMA.to_owned(),
            manifest,
            numeric,
            lexical,
            model,
            program,
            tensors,
            decode,
            tied_embedding_alias,
            bundle_self_hash: Hash256::ZERO,
        };
        bundle.bundle_self_hash = bundle.compute_self_hash();
        Ok(bundle)
    }

    /// Canonical JSON bytes including `bundle_self_hash`.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_bundle_bytes(self)
    }

    /// Self hash over canonical bundle JSON with `bundle_self_hash` omitted.
    #[must_use]
    pub fn compute_self_hash(&self) -> Hash256 {
        self_hash_omitting_fields(
            Self::domain(),
            &self.canonicalized_for_encoding(),
            "bundle_self_hash",
            &[],
        )
        .expect("reference bundle self-hash canonicalizes")
    }

    /// Recompute the self-hash and compare it to the stored field.
    #[must_use]
    pub fn self_hash_round_trips(&self) -> bool {
        self.compute_self_hash() == self.bundle_self_hash
    }

    /// DomainHash context for `reference_model_bundle.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-artifact",
            "ReferenceModelBundle",
            REFERENCE_MODEL_BUNDLE_SCHEMA,
            REFERENCE_MODEL_BUNDLE_SCHEMA_VERSION,
        )
    }

    pub(crate) fn canonicalized_for_encoding(&self) -> Self {
        let mut clone = self.clone();
        clone.program = clone
            .program
            .canonicalized()
            .expect("reference program canonicalizes for bundle encoding");
        clone.tensors.sort_by(|left, right| left.id.cmp(&right.id));
        clone
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceTensorError {
    EmptyShape {
        id: TensorRef,
    },
    ZeroDimension {
        id: TensorRef,
    },
    ShapeOverflow {
        id: TensorRef,
    },
    ValueCountMismatch {
        id: TensorRef,
        expected: usize,
        actual: usize,
    },
    NonFiniteValue {
        id: TensorRef,
        index: usize,
    },
}

impl fmt::Display for ReferenceTensorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyShape { id } => write!(f, "reference tensor {id} has empty shape"),
            Self::ZeroDimension { id } => {
                write!(f, "reference tensor {id} has a zero dimension")
            }
            Self::ShapeOverflow { id } => {
                write!(f, "reference tensor {id} shape element count overflowed")
            }
            Self::ValueCountMismatch {
                id,
                expected,
                actual,
            } => write!(
                f,
                "reference tensor {id} has {actual} values, expected {expected}"
            ),
            Self::NonFiniteValue { id, index } => {
                write!(
                    f,
                    "reference tensor {id} contains non-finite value at {index}"
                )
            }
        }
    }
}

impl Error for ReferenceTensorError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceBundleError {
    Graph(ReferenceGraphError),
    Tensor(ReferenceTensorError),
    DuplicateTensor { id: TensorRef },
}

impl fmt::Display for ReferenceBundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => write!(f, "{error}"),
            Self::Tensor(error) => write!(f, "{error}"),
            Self::DuplicateTensor { id } => {
                write!(f, "reference bundle contains duplicate tensor {id}")
            }
        }
    }
}

impl Error for ReferenceBundleError {}

impl From<ReferenceGraphError> for ReferenceBundleError {
    fn from(error: ReferenceGraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<ReferenceTensorError> for ReferenceBundleError {
    fn from(error: ReferenceTensorError) -> Self {
        Self::Tensor(error)
    }
}

fn canonicalize_tensors(
    mut tensors: Vec<ReferenceTensor>,
) -> Result<Vec<ReferenceTensor>, ReferenceBundleError> {
    tensors.sort_by(|left, right| left.id.cmp(&right.id));
    let mut seen = BTreeSet::new();
    for tensor in &tensors {
        if !seen.insert(tensor.id.clone()) {
            return Err(ReferenceBundleError::DuplicateTensor {
                id: tensor.id.clone(),
            });
        }
        validate_tensor_shape_and_values(&tensor.id, &tensor.shape, &tensor.values)?;
    }
    Ok(tensors)
}

fn validate_tensor_shape_and_values(
    id: &TensorRef,
    shape: &[u32],
    values: &[f32],
) -> Result<(), ReferenceTensorError> {
    if shape.is_empty() {
        return Err(ReferenceTensorError::EmptyShape { id: id.clone() });
    }
    if shape.contains(&0) {
        return Err(ReferenceTensorError::ZeroDimension { id: id.clone() });
    }
    let mut expected = 1_usize;
    for dim in shape {
        expected = expected
            .checked_mul(*dim as usize)
            .ok_or_else(|| ReferenceTensorError::ShapeOverflow { id: id.clone() })?;
    }
    if expected != values.len() {
        return Err(ReferenceTensorError::ValueCountMismatch {
            id: id.clone(),
            expected,
            actual: values.len(),
        });
    }
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(ReferenceTensorError::NonFiniteValue {
                id: id.clone(),
                index,
            });
        }
    }
    Ok(())
}

fn reference_tensor_hash(
    id: &TensorRef,
    role: ReferenceTensorRole,
    shape: &[u32],
    values: &[f32],
) -> Hash256 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"gbf:gbf-artifact:ReferenceTensor:v1\0");
    push_len_prefixed(&mut bytes, id.as_str().as_bytes());
    push_len_prefixed(&mut bytes, role.stable_name().as_bytes());
    for dim in shape {
        bytes.extend_from_slice(&dim.to_le_bytes());
    }
    bytes.push(0xff);
    for value in values {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    sha256(bytes)
}

fn push_len_prefixed(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}
