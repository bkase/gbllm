#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gbf_artifact::{
    ArtifactAux, ArtifactCore, CanonicalTensor, CanonicalTensorId, DecodeCapabilitySet, DecodeSpec,
    Dtype, LexicalSpec_v1, ModelArtifact, ModelSpec_S3, PayloadRole, QuantSpec_S3, ReferenceEdge,
    ReferenceEvalGraph, ReferenceManifest, ReferenceModelBundle, ReferenceModelSpec, ReferenceNode,
    ReferenceNumericProfile, ReferenceOp, ReferenceProgram, ReferenceTensor, ReferenceTensorRole,
    SequenceSemanticsSpec, TextCharSeq, VOCAB_SIZE, WeightQuant, canonical_payload_sha,
};
use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{KnBaselineInputs, KnBaselineProduct, s3_fit_kn5};
use gbf_experiments::s3::score::{Evaluator, EvaluatorOutput, ScorerKind};
use gbf_foundation::{Hash256, sha256};

pub const TARGET_A: u8 = 26;

#[derive(Debug, Clone, Default)]
pub struct UniformEvaluator;

impl Evaluator for UniformEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, _prefix: &[u8], target_ix: usize) -> EvaluatorOutput {
        EvaluatorOutput::from_logits(vec![0.0; VOCAB_SIZE], target_ix).unwrap()
    }

    fn reset_state(&mut self) {}
}

#[derive(Debug, Clone, Default)]
pub struct ContextSensitiveEvaluator;

impl Evaluator for ContextSensitiveEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, prefix: &[u8], target_ix: usize) -> EvaluatorOutput {
        let mut logits = vec![0.0; VOCAB_SIZE];
        if !prefix.is_empty() {
            logits[usize::from(TARGET_A)] = 10.0;
        }
        EvaluatorOutput::from_logits(logits, target_ix).unwrap()
    }

    fn reset_state(&mut self) {}
}

#[derive(Debug, Clone, Default)]
pub struct ShortLogitsEvaluator;

impl Evaluator for ShortLogitsEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, _prefix: &[u8], _target_ix: usize) -> EvaluatorOutput {
        EvaluatorOutput {
            logits: vec![0.0; VOCAB_SIZE - 1],
            target_logprob: -(VOCAB_SIZE as f64).ln(),
        }
    }

    fn reset_state(&mut self) {}
}

#[derive(Debug, Clone, Default)]
pub struct PromptWideSoftmaxShapeEvaluator;

impl Evaluator for PromptWideSoftmaxShapeEvaluator {
    fn scorer_kind(&self) -> ScorerKind {
        ScorerKind::ReferenceScorer
    }

    fn forward(&self, prefix: &[u8], _target_ix: usize) -> EvaluatorOutput {
        let len = (prefix.len() + 1) * VOCAB_SIZE;
        EvaluatorOutput {
            logits: vec![0.0; len],
            target_logprob: -(len as f64).ln(),
        }
    }

    fn reset_state(&mut self) {}
}

pub fn repeated_a(len: usize) -> TextCharSeq {
    TextCharSeq::new(vec![TARGET_A; len]).unwrap()
}

pub fn oracle_train() -> TextCharSeq {
    normalize_file(&workspace_root().join("fixtures/baselines/kn_oracle/train.bytes"))
}

pub fn oracle_eval() -> TextCharSeq {
    normalize_file(&workspace_root().join("fixtures/baselines/kn_oracle/eval.bytes"))
}

pub fn kn_product_for_val(val: TextCharSeq) -> (TextCharSeq, KnBaselineProduct) {
    let train = oracle_train();
    let product = s3_fit_kn5(KnBaselineInputs {
        train_post: train.clone(),
        val_post: val,
    })
    .expect("KN oracle train fits supplied val");
    (train, product)
}

pub fn predictable_a_bundle() -> ReferenceModelBundle {
    let embedding = ReferenceTensor::new(
        path("tensor.embedding"),
        ReferenceTensorRole::Embedding,
        vec![VOCAB_SIZE as u32, 16],
        predictable_embedding(),
    )
    .unwrap();
    let classifier = ReferenceTensor::new(
        path("tensor.classifier.weight"),
        ReferenceTensorRole::Classifier,
        vec![VOCAB_SIZE as u32, 16],
        predictable_classifier_weight(),
    )
    .unwrap();
    let graph = ReferenceEvalGraph::new(
        vec![
            ReferenceNode::new(
                path("op.embedding"),
                ReferenceOp::Embedding,
                vec![path("tensor.embedding")],
                vec![path("runtime.embedding")],
            ),
            ReferenceNode::new(
                path("op.classifier"),
                ReferenceOp::Classifier,
                vec![path("runtime.embedding"), path("tensor.classifier.weight")],
                vec![path("runtime.logits")],
            ),
        ],
        vec![ReferenceEdge::new(
            path("op.embedding"),
            path("op.classifier"),
            path("runtime.embedding"),
        )],
    )
    .unwrap();

    ReferenceModelBundle::new(
        ReferenceManifest::new(
            11,
            sha256("predictable-a-teacher"),
            sha256("predictable-a-sequence"),
            "score-test-export-visitor",
            sha256("score-test-export-visitor"),
        ),
        ReferenceNumericProfile::pinned(),
        LexicalSpec_v1::pinned(),
        ReferenceModelSpec::toy0(),
        ReferenceProgram::new(graph, sha256("predictable-a-checkpoint-schema")).unwrap(),
        vec![embedding, classifier],
        DecodeSpec::argmax(),
        None,
    )
    .unwrap()
}

pub fn artifact_fixture() -> ModelArtifact {
    let tensor_id = CanonicalTensorId::new("tensor.embedding").unwrap();
    let tensor = CanonicalTensor::new(
        tensor_id.clone(),
        Dtype::Fp32,
        vec![VOCAB_SIZE as u32, 16],
        canonical_payload_sha("artifact-scorer-payload"),
        PayloadRole::DeployableWeight,
    )
    .unwrap();
    let core = ArtifactCore::new(
        ModelArtifact::fixture_manifest(7, Hash256::from_bytes([7; 32])),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny("score-artifact-fixture"),
        QuantSpec_S3::new(BTreeMap::from([(tensor_id, WeightQuant::Fp32)])),
        SequenceSemanticsSpec::linear_state(16).unwrap(),
        vec![tensor],
        vec![],
        DecodeCapabilitySet::argmax_only(),
        None,
    )
    .unwrap();
    ModelArtifact::new(core, vec![], ArtifactAux::sparse(), None).unwrap()
}

fn predictable_embedding() -> Vec<f32> {
    let mut values = vec![0.0; VOCAB_SIZE * 16];
    values[0] = 1.0;
    values[usize::from(TARGET_A) * 16] = 1.0;
    values
}

fn predictable_classifier_weight() -> Vec<f32> {
    let mut values = vec![-10.0; VOCAB_SIZE * 16];
    values[usize::from(TARGET_A) * 16] = 10.0;
    values
}

fn normalize_file(path: &Path) -> TextCharSeq {
    let bytes = std::fs::read(path).expect("fixture bytes read");
    let normalized = normalize_raw(&bytes).expect("fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn path(value: &str) -> gbf_artifact::TensorRef {
    gbf_artifact::TensorRef::new(value).unwrap()
}
