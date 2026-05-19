mod artifact_b5_support;

use gbf_artifact::{
    ArtifactCore, ArtifactError, DecodeCapabilitySet, Dtype, LexicalSpec_v1, ModelSpec_S3,
    PayloadRole, QuantSpec_S3, QuantSpecError, ReferenceEdge, ReferenceEvalGraph, ReferenceNode,
    ReferenceOp, SequenceSemanticsSpec,
};

use artifact_b5_support::{id, manifest, quant_for, tensor};

#[test]
fn artifact_core_quant_coverage_missing_linear_tensor_rejects() {
    let linear = tensor(
        "tensor.linear.weight",
        Dtype::Ternary2,
        vec![16, 16],
        PayloadRole::DeployableWeight,
    );

    let err = ArtifactCore::new(
        manifest(),
        LexicalSpec_v1::pinned(),
        ModelSpec_S3::tiny("toy0"),
        QuantSpec_S3::default(),
        SequenceSemanticsSpec::linear_state(4).expect("sequence spec"),
        vec![linear.clone()],
        vec![],
        DecodeCapabilitySet::argmax_only(),
        None,
    )
    .expect_err("missing deployable weight quant rejects");

    assert!(matches!(
        err,
        ArtifactError::QuantSpecCoverageMissing { tensor_id } if tensor_id == linear.id
    ));
}

#[test]
fn quant_spec_verify_coverage_missing_graph_weight_rejects() {
    let graph = ReferenceEvalGraph::new(
        vec![
            ReferenceNode::new(
                id("op.embedding"),
                ReferenceOp::Embedding,
                vec![id("tensor.embedding")],
                vec![id("hidden.embedding")],
            ),
            ReferenceNode::new(
                id("op.linear"),
                ReferenceOp::Linear,
                vec![id("hidden.embedding"), id("tensor.linear.weight")],
                vec![id("hidden.linear")],
            ),
        ],
        vec![ReferenceEdge::new(
            id("op.embedding"),
            id("op.linear"),
            id("hidden.embedding"),
        )],
    )
    .expect("valid graph");

    let quant = quant_for(&[id("tensor.embedding")]);
    let err = quant
        .verify_coverage(&graph)
        .expect_err("missing linear weight rejects");

    assert!(matches!(
        err,
        QuantSpecError::CoverageMissing { tensor_id, op_kind }
            if tensor_id == id("tensor.linear.weight") && op_kind == "linear"
    ));
}
