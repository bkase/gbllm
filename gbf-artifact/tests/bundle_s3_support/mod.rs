#![allow(dead_code)]

use gbf_artifact::{
    DecodeSpec, ReferenceEdge, ReferenceEvalGraph, ReferenceManifest, ReferenceModelBundle,
    ReferenceModelSpec, ReferenceNode, ReferenceNumericProfile, ReferenceOp, ReferenceProgram,
    ReferenceTensor, ReferenceTensorRole, TensorRef,
};
use gbf_foundation::Hash256;
use serde_json::{Value, json};

pub fn observation_policy() {}

pub fn toy_bundle() -> ReferenceModelBundle {
    toy_bundle_with_nodes(toy_nodes())
}

pub fn toy_bundle_with_nodes(nodes: Vec<ReferenceNode>) -> ReferenceModelBundle {
    toy_bundle_with_graph(nodes, toy_edges())
}

pub fn toy_bundle_with_graph(
    nodes: Vec<ReferenceNode>,
    edges: Vec<ReferenceEdge>,
) -> ReferenceModelBundle {
    let graph = ReferenceEvalGraph::new(nodes, edges).expect("toy graph is valid");
    let program = ReferenceProgram::new(graph, Hash256::from_bytes([0x44; 32]))
        .expect("toy reference program is valid");
    ReferenceModelBundle::new(
        ReferenceManifest::new(
            7,
            Hash256::from_bytes([0x11; 32]),
            Hash256::from_bytes([0x22; 32]),
            "bundle-s3-support",
            Hash256::from_bytes([0x33; 32]),
        ),
        ReferenceNumericProfile::pinned(),
        gbf_artifact::LexicalSpec_v1::pinned(),
        ReferenceModelSpec::toy0(),
        program,
        toy_tensors(),
        DecodeSpec::argmax(),
        None,
    )
    .expect("toy bundle is valid")
}

pub fn toy_nodes() -> Vec<ReferenceNode> {
    vec![
        ReferenceNode::new(
            id("op.embedding"),
            ReferenceOp::Embedding,
            vec![id("tensor.embedding")],
            vec![id("activation.embedding")],
        ),
        ReferenceNode::new(
            id("op.linear"),
            ReferenceOp::Linear,
            vec![
                id("activation.embedding"),
                id("tensor.linear.weight"),
                id("tensor.linear.bias"),
            ],
            vec![id("activation.linear")],
        ),
        ReferenceNode::new(
            id("op.activation"),
            ReferenceOp::Activation(gbf_artifact::ActivationKind::ReLU),
            vec![id("activation.linear")],
            vec![id("activation.hidden")],
        ),
        ReferenceNode::new(
            id("op.classifier"),
            ReferenceOp::Classifier,
            vec![
                id("activation.hidden"),
                id("tensor.classifier.weight"),
                id("tensor.classifier.bias"),
            ],
            vec![id("activation.logits")],
        ),
    ]
}

pub fn toy_edges() -> Vec<ReferenceEdge> {
    vec![
        ReferenceEdge::new(
            id("op.embedding"),
            id("op.linear"),
            id("activation.embedding"),
        ),
        ReferenceEdge::new(
            id("op.linear"),
            id("op.activation"),
            id("activation.linear"),
        ),
        ReferenceEdge::new(
            id("op.activation"),
            id("op.classifier"),
            id("activation.hidden"),
        ),
    ]
}

pub fn permute_nodes(mut nodes: Vec<ReferenceNode>, mut seed: usize) -> Vec<ReferenceNode> {
    for index in (1..nodes.len()).rev() {
        let swap_with = seed % (index + 1);
        nodes.swap(index, swap_with);
        seed /= index + 1;
    }
    nodes
}

pub fn redacted_canonical_summary(bundle: &ReferenceModelBundle) -> Value {
    json!({
        "schema": &bundle.schema,
        "numeric": &bundle.numeric,
        "opset": bundle.program.opset,
        "node_order": bundle.program.graph.nodes.iter().map(|node| node.op_id.as_str()).collect::<Vec<_>>(),
        "edge_order": bundle.program.graph.edges.iter().map(|edge| {
            format!("{}->{}:{}", edge.from, edge.to, edge.tensor)
        }).collect::<Vec<_>>(),
        "tensor_ids": bundle.tensors.iter().map(|tensor| tensor.id.as_str()).collect::<Vec<_>>(),
        "decode": &bundle.decode,
        "tied_embedding_alias": &bundle.tied_embedding_alias,
        "bundle_self_hash_round_trips": bundle.self_hash_round_trips(),
    })
}

fn toy_tensors() -> Vec<ReferenceTensor> {
    let d_model = 16_usize;
    let vocab = 80_usize;
    let embedding_values = (0..vocab)
        .flat_map(|row| (0..d_model).map(move |col| row as f32 * 0.01 + col as f32 * 0.001))
        .collect::<Vec<_>>();
    let linear_values = (0..d_model)
        .flat_map(|row| {
            (0..d_model).map(move |col| {
                if row == col {
                    0.5
                } else {
                    ((row + col) % 5) as f32 * 0.005
                }
            })
        })
        .collect::<Vec<_>>();
    let linear_bias = (0..d_model)
        .map(|index| index as f32 * 0.000_5)
        .collect::<Vec<_>>();
    let classifier_values = (0..vocab)
        .flat_map(|token| {
            (0..d_model).map(move |col| (token as f32 + 1.0) * (col as f32 + 1.0) * 0.000_1)
        })
        .collect::<Vec<_>>();
    let classifier_bias = (0..vocab)
        .map(|token| token as f32 * 0.000_01)
        .collect::<Vec<_>>();

    vec![
        ReferenceTensor::new(
            id("tensor.embedding"),
            ReferenceTensorRole::Embedding,
            vec![vocab as u32, d_model as u32],
            embedding_values,
        )
        .unwrap(),
        ReferenceTensor::new(
            id("tensor.linear.weight"),
            ReferenceTensorRole::Weight,
            vec![d_model as u32, d_model as u32],
            linear_values,
        )
        .unwrap(),
        ReferenceTensor::new(
            id("tensor.linear.bias"),
            ReferenceTensorRole::Bias,
            vec![d_model as u32],
            linear_bias,
        )
        .unwrap(),
        ReferenceTensor::new(
            id("tensor.classifier.weight"),
            ReferenceTensorRole::Classifier,
            vec![vocab as u32, d_model as u32],
            classifier_values,
        )
        .unwrap(),
        ReferenceTensor::new(
            id("tensor.classifier.bias"),
            ReferenceTensorRole::Bias,
            vec![vocab as u32],
            classifier_bias,
        )
        .unwrap(),
    ]
}

pub fn id(value: &str) -> TensorRef {
    TensorRef::new(value).expect("test artifact path is valid")
}
