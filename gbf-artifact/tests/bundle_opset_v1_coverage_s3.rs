#![cfg(feature = "s3-schemas")]

use gbf_artifact::{ActivationKind, ReferenceOp, reference_op_evaluator_branch};

#[test]
fn opset_v1_variants_have_evaluator_branches() {
    let ops = [
        ReferenceOp::Linear,
        ReferenceOp::Embedding,
        ReferenceOp::Classifier,
        ReferenceOp::LinearStateBlock,
        ReferenceOp::Activation(ActivationKind::ReLU),
        ReferenceOp::Activation(ActivationKind::GeLU),
        ReferenceOp::Activation(ActivationKind::SiLU),
        ReferenceOp::Activation(ActivationKind::Tanh),
        ReferenceOp::MatMul,
        ReferenceOp::Add,
        ReferenceOp::Mul,
        ReferenceOp::LayerNorm,
        ReferenceOp::Softmax,
    ];

    for op in ops {
        assert!(!reference_op_evaluator_branch(&op).is_empty());
    }
}
