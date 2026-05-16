#![allow(dead_code)]

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use gbf_artifact::{
    ActivationKind, ClassifierView, DecodeSpec, LexicalSpec_v1, ReferenceEdge, ReferenceEvalGraph,
    ReferenceManifest, ReferenceModelBundle, ReferenceModelSpec, ReferenceNode,
    ReferenceNumericProfile, ReferenceOp, ReferenceProgram, ReferenceTensor, ReferenceTensorRole,
    TextCharSeq, TiedEmbeddingAlias, VOCAB_SIZE, evaluate_reference_program,
};
use gbf_experiments::s3::bundle::{
    BundleExportInputs, BundleExportProduct, s3_export_reference_bundle,
};
use gbf_foundation::sha256;
use gbf_train::export_visitor::{
    EXPORT_VISITOR_ID, EXPORT_VISITOR_VERSION_HASH, ExportVisitor, ExportVisitorError,
    ReferenceBundleExportModel,
};
use gbf_train::teacher::{
    DenseTeacherModel, FrozenTeacher, TeacherStorageFingerprint, TeacherStorageIdentity,
    TeacherWeightFingerprint, freeze_teacher,
};

static NEXT_STORAGE_ID: AtomicU64 = AtomicU64::new(40_000);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToyForwardError;

impl fmt::Display for ToyForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("toy forward failed")
    }
}

impl std::error::Error for ToyForwardError {}

#[derive(Debug, PartialEq)]
pub struct ToyBundleTeacher {
    seed: u64,
    requires_grad: bool,
    storage_identity: u64,
    node_order: Vec<usize>,
}

impl ToyBundleTeacher {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            requires_grad: true,
            storage_identity: next_storage_id(),
            node_order: vec![0, 1, 2, 3],
        }
    }

    pub fn with_node_order(seed: u64, node_order: Vec<usize>) -> Self {
        Self {
            seed,
            requires_grad: true,
            storage_identity: next_storage_id(),
            node_order,
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
            DecodeSpec::argmax(),
            self.reference_bundle_tied_embedding_alias(),
        )
        .map_err(ExportVisitorError::from)
    }

    fn fingerprint_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::from("s3-toy-bundle-teacher:v1:");
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        for tensor in tensor_specs(self.seed) {
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

impl Clone for ToyBundleTeacher {
    fn clone(&self) -> Self {
        Self {
            seed: self.seed,
            requires_grad: self.requires_grad,
            storage_identity: next_storage_id(),
            node_order: self.node_order.clone(),
        }
    }
}

impl DenseTeacherModel for ToyBundleTeacher {
    type Input = TextCharSeq;
    type Output = Vec<f32>;
    type ForwardError = ToyForwardError;

    fn detach_for_teacher(&mut self) {
        self.requires_grad = false;
        self.storage_identity = next_storage_id();
    }

    fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
        let bundle = self.build_bundle().map_err(|_| ToyForwardError)?;
        Ok(evaluate_reference_program(&bundle, &input, &()).logits)
    }

    fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
        TeacherWeightFingerprint::new(self.fingerprint_bytes()).unwrap()
    }

    fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
        let mut bytes = Vec::from("s3-toy-bundle-teacher-storage:f32:");
        bytes.extend_from_slice(&self.fingerprint_bytes());
        TeacherStorageFingerprint::new(bytes).unwrap()
    }

    fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
        TeacherStorageIdentity::new(self.storage_identity.to_le_bytes().to_vec()).unwrap()
    }

    fn teacher_requires_grad(&self) -> bool {
        self.requires_grad
    }
}

impl ReferenceBundleExportModel for ToyBundleTeacher {
    fn reference_bundle_seed(&self) -> u64 {
        self.seed
    }

    fn reference_bundle_program(&self) -> Result<ReferenceProgram, ExportVisitorError> {
        ReferenceProgram::new(
            ReferenceEvalGraph::new(reordered_nodes(&self.node_order), toy_edges())?,
            self.reference_bundle_checkpoint_schema_hash(),
        )
        .map_err(ExportVisitorError::from)
    }

    fn reference_bundle_tensors(&self) -> Result<Vec<ReferenceTensor>, ExportVisitorError> {
        Ok(tensor_specs(self.seed))
    }

    fn reference_bundle_tied_embedding_alias(&self) -> Option<TiedEmbeddingAlias> {
        Some(TiedEmbeddingAlias::new(
            path("tensor.embedding"),
            path("tensor.embedding"),
            true,
            ClassifierView::SameTensor,
        ))
    }
}

pub fn frozen_toy_teacher(seed: u64) -> FrozenTeacher<ToyBundleTeacher> {
    freeze_teacher(&ToyBundleTeacher::new(seed)).expect("toy teacher freezes")
}

pub fn frozen_toy_teacher_with_order(
    seed: u64,
    node_order: Vec<usize>,
) -> FrozenTeacher<ToyBundleTeacher> {
    freeze_teacher(&ToyBundleTeacher::with_node_order(seed, node_order))
        .expect("toy teacher freezes")
}

pub fn export_product(seed: u64) -> BundleExportProduct {
    let frozen = frozen_toy_teacher(seed);
    export_product_from(&frozen)
}

pub fn export_product_from(frozen: &FrozenTeacher<ToyBundleTeacher>) -> BundleExportProduct {
    s3_export_reference_bundle(BundleExportInputs::new(
        frozen,
        ExportVisitor::pinned(),
        agreement_prompts(),
    ))
    .expect("toy bundle export succeeds")
}

pub fn agreement_prompts() -> Vec<TextCharSeq> {
    vec![
        TextCharSeq::new(vec![0, 1, 2]).unwrap(),
        TextCharSeq::new(vec![10, 11, 12, 13]).unwrap(),
        TextCharSeq::new(vec![30, 31, 32, 33, 34]).unwrap(),
    ]
}

pub fn tensor_payload_bytes(bundle: &ReferenceModelBundle) -> u64 {
    bundle
        .tensors
        .iter()
        .map(|tensor| tensor.values.len() as u64 * 4)
        .sum()
}

pub fn expected_single_copy_payload_bytes() -> u64 {
    (VOCAB_SIZE as u64 * 16 + 16 * 16 + 16 + VOCAB_SIZE as u64) * 4
}

fn tensor_specs(seed: u64) -> Vec<ReferenceTensor> {
    vec![
        ReferenceTensor::new(
            path("tensor.embedding"),
            ReferenceTensorRole::Embedding,
            vec![VOCAB_SIZE as u32, 16],
            embedding_values(seed),
        )
        .unwrap(),
        ReferenceTensor::new(
            path("tensor.linear.weight"),
            ReferenceTensorRole::Weight,
            vec![16, 16],
            linear_weight_values(seed),
        )
        .unwrap(),
        ReferenceTensor::new(
            path("tensor.linear.bias"),
            ReferenceTensorRole::Bias,
            vec![16],
            linear_bias_values(seed),
        )
        .unwrap(),
        ReferenceTensor::new(
            path("tensor.classifier.bias"),
            ReferenceTensorRole::Bias,
            vec![VOCAB_SIZE as u32],
            classifier_bias_values(seed),
        )
        .unwrap(),
    ]
}

fn embedding_values(seed: u64) -> Vec<f32> {
    let seed_offset = seed as f32 * 0.000_01;
    (0..VOCAB_SIZE)
        .flat_map(|row| {
            (0..16).map(move |col| {
                ((row as f32 - 40.0) * 0.001) + (col as f32 * 0.000_3) + seed_offset
            })
        })
        .collect()
}

fn linear_weight_values(seed: u64) -> Vec<f32> {
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

fn linear_bias_values(seed: u64) -> Vec<f32> {
    (0..16)
        .map(|index| index as f32 * 0.000_1 + seed as f32 * 0.000_01)
        .collect()
}

fn classifier_bias_values(seed: u64) -> Vec<f32> {
    (0..VOCAB_SIZE)
        .map(|index| index as f32 * 0.000_05 + seed as f32 * 0.000_01)
        .collect()
}

fn reordered_nodes(order: &[usize]) -> Vec<ReferenceNode> {
    let nodes = toy_nodes();
    order.iter().map(|index| nodes[*index].clone()).collect()
}

fn toy_nodes() -> Vec<ReferenceNode> {
    vec![
        ReferenceNode::new(
            path("op.embedding"),
            ReferenceOp::Embedding,
            vec![path("tensor.embedding")],
            vec![path("runtime.embedding")],
        ),
        ReferenceNode::new(
            path("op.linear"),
            ReferenceOp::Linear,
            vec![
                path("runtime.embedding"),
                path("tensor.linear.weight"),
                path("tensor.linear.bias"),
            ],
            vec![path("runtime.hidden")],
        ),
        ReferenceNode::new(
            path("op.activation"),
            ReferenceOp::Activation(ActivationKind::ReLU),
            vec![path("runtime.hidden")],
            vec![path("runtime.hidden_relu")],
        ),
        ReferenceNode::new(
            path("op.classifier"),
            ReferenceOp::Classifier,
            vec![
                path("runtime.hidden_relu"),
                path("tensor.embedding"),
                path("tensor.classifier.bias"),
            ],
            vec![path("runtime.logits")],
        ),
    ]
}

fn toy_edges() -> Vec<ReferenceEdge> {
    vec![
        ReferenceEdge::new(
            path("op.embedding"),
            path("op.linear"),
            path("runtime.embedding"),
        ),
        ReferenceEdge::new(
            path("op.linear"),
            path("op.activation"),
            path("runtime.hidden"),
        ),
        ReferenceEdge::new(
            path("op.activation"),
            path("op.classifier"),
            path("runtime.hidden_relu"),
        ),
    ]
}

fn path(value: &str) -> gbf_artifact::TensorRef {
    gbf_artifact::TensorRef::new(value).unwrap()
}

fn next_storage_id() -> u64 {
    NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed)
}
