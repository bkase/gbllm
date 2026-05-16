#![allow(dead_code)]

use gbf_artifact::{
    ActivationKind, ClassifierView, DecodeSpec, LexicalSpec_v1, ReferenceEdge, ReferenceEvalGraph,
    ReferenceManifest, ReferenceModelBundle, ReferenceModelSpec, ReferenceNode,
    ReferenceNumericProfile, ReferenceOp, ReferenceProgram, ReferenceTensor, ReferenceTensorRole,
    TiedEmbeddingAlias, VOCAB_SIZE,
};
use gbf_foundation::{Hash256, WorkloadId, sha256};
use gbf_oracle::denotational::{DenotationalOracle, DenotationalOracleInputs};
use gbf_workload::{
    AcceptanceMatrix_S3, ExecutionMatrix_S3, ObservationPolicy_S3, PromptCase, SessionProfile_S3,
    V0_SUCCESS_HELD_OUT_CHAPTER_SHA, V0_SUCCESS_PROMPT_COUNT, WorkloadClass, WorkloadManifest_v0,
};

pub fn fixture_bundle() -> ReferenceModelBundle {
    ReferenceModelBundle::new(
        ReferenceManifest::new(
            0,
            sha256(b"denotational-fixture-frozen-teacher"),
            sha256(b"denotational-fixture-sequence-semantics"),
            "gbf-oracle.denotational.fixture",
            sha256(b"gbf-oracle.denotational.fixture.visitor"),
        ),
        ReferenceNumericProfile::pinned(),
        LexicalSpec_v1::pinned(),
        ReferenceModelSpec::toy0(),
        fixture_program(),
        fixture_tensors(),
        DecodeSpec::argmax(),
        Some(TiedEmbeddingAlias::new(
            path("tensor.embedding"),
            path("tensor.embedding"),
            true,
            ClassifierView::SameTensor,
        )),
    )
    .expect("fixture bundle builds")
}

pub fn fixture_workload() -> WorkloadManifest_v0 {
    let held_out_sha = V0_SUCCESS_HELD_OUT_CHAPTER_SHA
        .parse()
        .expect("held-out hash parses");
    let prompts = (0..V0_SUCCESS_PROMPT_COUNT)
        .map(|index| {
            let chars = (0..64)
                .map(|offset| ((index + offset) % 75) as u8)
                .collect::<Vec<_>>();
            PromptCase::new(format!("prompt-{index:02}"), chars, held_out_sha)
                .expect("prompt case builds")
        })
        .collect::<Vec<_>>();
    let mut workload = WorkloadManifest_v0 {
        schema: "workload_manifest.v1".to_owned(),
        id: WorkloadId::from("v0_success"),
        class: WorkloadClass::Conformance,
        prompts,
        seeds: vec![0, 1, 2, 3, 4],
        session: SessionProfile_S3::pinned(),
        observation: ObservationPolicy_S3::pinned(),
        execution: ExecutionMatrix_S3::pinned(),
        acceptance: AcceptanceMatrix_S3::pinned(),
        workload_self_hash: Hash256::ZERO,
    };
    workload.workload_self_hash = workload.compute_self_hash().expect("workload self-hash");
    workload.validate().expect("workload fixture validates");
    workload
}

pub fn fixture_policy() -> ObservationPolicy_S3 {
    ObservationPolicy_S3::pinned()
}

pub fn evaluate<O: DenotationalOracle>(
    oracle: O,
) -> gbf_oracle::denotational::DenotationalOracleProduct {
    let bundle = fixture_bundle();
    let workload = fixture_workload();
    let policy = fixture_policy();
    oracle
        .evaluate(DenotationalOracleInputs::new(&bundle, &workload, &policy))
        .expect("denotational oracle evaluates")
}

fn fixture_program() -> ReferenceProgram {
    ReferenceProgram::new(
        ReferenceEvalGraph::new(
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
            ],
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
            ],
        )
        .expect("fixture graph builds"),
        sha256(b"denotational-fixture-program-schema"),
    )
    .expect("fixture program builds")
}

fn fixture_tensors() -> Vec<ReferenceTensor> {
    vec![
        ReferenceTensor::new(
            path("tensor.embedding"),
            ReferenceTensorRole::Embedding,
            vec![VOCAB_SIZE as u32, 16],
            vec![0.0; VOCAB_SIZE * 16],
        )
        .expect("embedding tensor builds"),
        ReferenceTensor::new(
            path("tensor.linear.weight"),
            ReferenceTensorRole::Weight,
            vec![16, 16],
            vec![0.0; 16 * 16],
        )
        .expect("linear weight tensor builds"),
        ReferenceTensor::new(
            path("tensor.linear.bias"),
            ReferenceTensorRole::Bias,
            vec![16],
            vec![0.0; 16],
        )
        .expect("linear bias tensor builds"),
        ReferenceTensor::new(
            path("tensor.classifier.bias"),
            ReferenceTensorRole::Bias,
            vec![VOCAB_SIZE as u32],
            classifier_bias(),
        )
        .expect("classifier bias tensor builds"),
    ]
}

fn classifier_bias() -> Vec<f32> {
    let mut values = vec![0.0; VOCAB_SIZE];
    values[7] = 1.0;
    values
}

fn path(value: &str) -> gbf_artifact::TensorRef {
    gbf_artifact::TensorRef::new(value).expect("fixture artifact path is valid")
}
