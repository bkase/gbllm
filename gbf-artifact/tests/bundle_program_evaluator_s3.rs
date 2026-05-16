#![cfg(feature = "s3-schemas")]

#[path = "bundle_s3_support/mod.rs"]
mod bundle_s3_support;

use std::panic::{AssertUnwindSafe, catch_unwind};

use gbf_artifact::{
    ReferenceModelBundle, ReferenceNode, ReferenceOp, ReferenceTensor, ReferenceTensorRole,
    TextCharSeq, evaluate_reference_program,
};

#[test]
fn reference_program_evaluator_is_deterministic_for_toy0_surface() {
    let bundle = bundle_s3_support::toy_bundle();
    let prompt = TextCharSeq::new(vec![0, 1, 2]).expect("prompt is valid charset_v1 text");
    let policy = bundle_s3_support::observation_policy();

    let first = evaluate_reference_program(&bundle, &prompt, &policy);
    for _ in 0..10 {
        assert_eq!(evaluate_reference_program(&bundle, &prompt, &policy), first);
    }

    assert_eq!(first.logits.len(), 80);
    assert_eq!(first.argmax_token, 79);
    assert_eq!(first.node_count, bundle.program.graph.nodes.len());
    assert!(first.logits.iter().all(|value| value.is_finite()));
}

#[test]
fn linear_state_block_panics_until_toy0_semantics_are_implemented() {
    let bundle = bundle_s3_support::toy_bundle_with_graph(
        vec![ReferenceNode::new(
            bundle_s3_support::id("op.state"),
            ReferenceOp::LinearStateBlock,
            vec![bundle_s3_support::id("tensor.embedding")],
            vec![bundle_s3_support::id("activation.state")],
        )],
        vec![],
    );
    let prompt = TextCharSeq::new(vec![0, 1, 2]).expect("prompt is valid charset_v1 text");
    let policy = bundle_s3_support::observation_policy();

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = evaluate_reference_program(&bundle, &prompt, &policy);
    }))
    .expect_err("LinearStateBlock evaluation must be loud in B4");

    assert!(
        panic_message(panic.as_ref())
            .contains("LinearStateBlock reference evaluation is not implemented")
    );
}

#[test]
fn evaluator_panics_before_argmax_when_final_logits_are_not_finite() {
    let base = bundle_s3_support::toy_bundle();
    let mut tensors = base.tensors.clone();
    for tensor in &mut tensors {
        match tensor.id.as_str() {
            "tensor.linear.weight" => {
                *tensor = ReferenceTensor::new(
                    bundle_s3_support::id("tensor.linear.weight"),
                    ReferenceTensorRole::Weight,
                    vec![16, 16],
                    vec![f32::MAX; 16 * 16],
                )
                .expect("finite but overflowing linear tensor is valid input");
            }
            "tensor.classifier.weight" => {
                *tensor = ReferenceTensor::new(
                    bundle_s3_support::id("tensor.classifier.weight"),
                    ReferenceTensorRole::Classifier,
                    vec![80, 16],
                    vec![f32::MAX; 80 * 16],
                )
                .expect("finite but overflowing classifier tensor is valid input");
            }
            _ => {}
        }
    }
    let bundle = ReferenceModelBundle::new(
        base.manifest,
        base.numeric,
        base.lexical,
        base.model,
        base.program,
        tensors,
        base.decode,
        base.tied_embedding_alias,
    )
    .expect("overflowing bundle still has finite tensor payloads");
    let prompt = TextCharSeq::new(vec![0, 1, 2]).expect("prompt is valid charset_v1 text");
    let policy = bundle_s3_support::observation_policy();

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = evaluate_reference_program(&bundle, &prompt, &policy);
    }))
    .expect_err("non-finite final logits must panic before argmax");

    assert!(panic_message(panic.as_ref()).contains("reference program produced non-finite logit"));
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    "<non-string panic>".to_owned()
}
