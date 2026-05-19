//! Pure Toy0-surface reference program evaluator for S3 bundle tests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bundle::{ReferenceModelBundle, ReferenceTensor};
use crate::lexical::{CharId, TextCharSeq};
use crate::opset_v1::{ActivationKind, ReferenceOp};
use crate::reference_eval_graph::TensorRef;

/// Deterministic observations produced by evaluating a reference bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceObservations {
    pub logits: Vec<f32>,
    pub argmax_token: CharId,
    pub node_count: usize,
}

/// Evaluate a bundle's reference program without consulting train/oracle crates.
#[must_use]
pub fn evaluate_reference_program(
    bundle: &ReferenceModelBundle,
    prompt: &TextCharSeq,
    observation_policy: &impl ?Sized,
) -> ReferenceObservations {
    let _ = observation_policy;
    let tensors = bundle
        .tensors
        .iter()
        .map(|tensor| (tensor.id.clone(), EvalTensor::from_reference(tensor)))
        .collect::<BTreeMap<_, _>>();
    let mut runtime = BTreeMap::<TensorRef, EvalTensor>::new();
    let mut last_output = None::<TensorRef>;

    for (evaluation_step, node) in bundle.program.graph.nodes.iter().enumerate() {
        trace!(
            target: "gbf_artifact::bundle_program_evaluator",
            event_name = "s3::bundle_eval::node_executed",
            op_id = %node.op_id,
            op_kind = reference_op_evaluator_branch(&node.op),
            input_count = node.inputs.len(),
            output_count = node.outputs.len(),
            evaluation_step,
        );

        let output = match &node.op {
            ReferenceOp::Embedding => {
                let embedding = resolve_input(&node.inputs[0], &runtime, &tensors);
                embedding_lookup(&embedding, prompt)
            }
            ReferenceOp::Linear | ReferenceOp::Classifier => {
                let input = resolve_input(&node.inputs[0], &runtime, &tensors);
                let weight = resolve_input(&node.inputs[1], &runtime, &tensors);
                let bias = node
                    .inputs
                    .get(2)
                    .map(|id| resolve_input(id, &runtime, &tensors));
                linear(&input, &weight, bias.as_ref())
            }
            ReferenceOp::LinearStateBlock => {
                panic!("LinearStateBlock reference evaluation is not implemented for B4 Toy0")
            }
            ReferenceOp::Activation(kind) => {
                let input = resolve_input(&node.inputs[0], &runtime, &tensors);
                activation(kind, &input)
            }
            ReferenceOp::MatMul => {
                let left = resolve_input(&node.inputs[0], &runtime, &tensors);
                let right = resolve_input(&node.inputs[1], &runtime, &tensors);
                mat_mul(&left, &right)
            }
            ReferenceOp::Add => {
                let left = resolve_input(&node.inputs[0], &runtime, &tensors);
                let right = resolve_input(&node.inputs[1], &runtime, &tensors);
                binary_elementwise(&left, &right, |a, b| a + b)
            }
            ReferenceOp::Mul => {
                let left = resolve_input(&node.inputs[0], &runtime, &tensors);
                let right = resolve_input(&node.inputs[1], &runtime, &tensors);
                binary_elementwise(&left, &right, |a, b| a * b)
            }
            ReferenceOp::LayerNorm => {
                let input = resolve_input(&node.inputs[0], &runtime, &tensors);
                layer_norm(&input)
            }
            ReferenceOp::Softmax => {
                let input = resolve_input(&node.inputs[0], &runtime, &tensors);
                softmax(&input)
            }
        };

        for output_id in &node.outputs {
            runtime.insert(output_id.clone(), output.clone());
            last_output = Some(output_id.clone());
        }
    }

    let logits_ref = last_output.expect("reference program must produce at least one output");
    let logits = runtime
        .get(&logits_ref)
        .expect("last output exists in runtime tensor map")
        .values
        .clone();
    assert_finite_logits(&logits);
    let argmax_token = argmax_lowest_index(&logits);
    ReferenceObservations {
        logits,
        argmax_token,
        node_count: bundle.program.graph.nodes.len(),
    }
}

/// Exhaustive branch marker used by coverage tests for `opset_v1`.
#[must_use]
pub const fn reference_op_evaluator_branch(op: &ReferenceOp) -> &'static str {
    match op {
        ReferenceOp::Linear => "linear",
        ReferenceOp::Embedding => "embedding",
        ReferenceOp::Classifier => "classifier",
        ReferenceOp::LinearStateBlock => "linear_state_block",
        ReferenceOp::Activation(ActivationKind::ReLU) => "activation_relu",
        ReferenceOp::Activation(ActivationKind::GeLU) => "activation_gelu",
        ReferenceOp::Activation(ActivationKind::SiLU) => "activation_silu",
        ReferenceOp::Activation(ActivationKind::Tanh) => "activation_tanh",
        ReferenceOp::MatMul => "mat_mul",
        ReferenceOp::Add => "add",
        ReferenceOp::Mul => "mul",
        ReferenceOp::LayerNorm => "layer_norm",
        ReferenceOp::Softmax => "softmax",
    }
}

#[derive(Debug, Clone)]
struct EvalTensor {
    shape: Vec<usize>,
    values: Vec<f32>,
}

impl EvalTensor {
    fn from_reference(tensor: &ReferenceTensor) -> Self {
        Self {
            shape: tensor.shape.iter().map(|dim| *dim as usize).collect(),
            values: tensor.values.clone(),
        }
    }
}

fn resolve_input(
    id: &TensorRef,
    runtime: &BTreeMap<TensorRef, EvalTensor>,
    tensors: &BTreeMap<TensorRef, EvalTensor>,
) -> EvalTensor {
    runtime
        .get(id)
        .or_else(|| tensors.get(id))
        .unwrap_or_else(|| panic!("reference program input {id} is missing"))
        .clone()
}

fn embedding_lookup(embedding: &EvalTensor, prompt: &TextCharSeq) -> EvalTensor {
    assert_eq!(embedding.shape.len(), 2, "embedding tensor must be rank-2");
    let vocab = embedding.shape[0];
    let width = embedding.shape[1];
    let token = prompt.as_slice().last().copied().unwrap_or(0) as usize;
    assert!(
        token < vocab,
        "prompt token {token} exceeds embedding vocab"
    );
    let start = token * width;
    EvalTensor {
        shape: vec![width],
        values: embedding.values[start..start + width].to_vec(),
    }
}

fn linear(input: &EvalTensor, weight: &EvalTensor, bias: Option<&EvalTensor>) -> EvalTensor {
    assert_eq!(input.shape.len(), 1, "linear input must be rank-1");
    assert_eq!(weight.shape.len(), 2, "linear weight must be rank-2");
    let out = weight.shape[0];
    let inner = weight.shape[1];
    assert_eq!(input.shape[0], inner, "linear inner dimension mismatch");
    let mut values = vec![0.0_f32; out];
    for (row, output) in values.iter_mut().enumerate() {
        let mut acc = 0.0_f32;
        for col in 0..inner {
            acc += weight.values[row * inner + col] * input.values[col];
        }
        if let Some(bias) = bias {
            assert_eq!(bias.shape, vec![out], "linear bias dimension mismatch");
            acc += bias.values[row];
        }
        *output = acc;
    }
    EvalTensor {
        shape: vec![out],
        values,
    }
}

fn activation(kind: &ActivationKind, input: &EvalTensor) -> EvalTensor {
    EvalTensor {
        shape: input.shape.clone(),
        values: input
            .values
            .iter()
            .copied()
            .map(|value| match kind {
                ActivationKind::ReLU => value.max(0.0),
                ActivationKind::GeLU => {
                    0.5 * value * (1.0 + (0.797_884_6 * (value + 0.044_715 * value.powi(3))).tanh())
                }
                ActivationKind::SiLU => value / (1.0 + (-value).exp()),
                ActivationKind::Tanh => value.tanh(),
            })
            .collect(),
    }
}

fn mat_mul(left: &EvalTensor, right: &EvalTensor) -> EvalTensor {
    assert_eq!(left.shape.len(), 2, "mat_mul left tensor must be rank-2");
    assert_eq!(right.shape.len(), 2, "mat_mul right tensor must be rank-2");
    let rows = left.shape[0];
    let inner = left.shape[1];
    let cols = right.shape[1];
    assert_eq!(right.shape[0], inner, "mat_mul inner dimension mismatch");
    let mut values = vec![0.0_f32; rows * cols];
    for row in 0..rows {
        for col in 0..cols {
            let mut acc = 0.0_f32;
            for k in 0..inner {
                acc += left.values[row * inner + k] * right.values[k * cols + col];
            }
            values[row * cols + col] = acc;
        }
    }
    EvalTensor {
        shape: vec![rows, cols],
        values,
    }
}

fn binary_elementwise(
    left: &EvalTensor,
    right: &EvalTensor,
    f: impl Fn(f32, f32) -> f32,
) -> EvalTensor {
    if right.values.len() == 1 {
        return EvalTensor {
            shape: left.shape.clone(),
            values: left
                .values
                .iter()
                .map(|left| f(*left, right.values[0]))
                .collect(),
        };
    }
    if left.values.len() == 1 {
        return EvalTensor {
            shape: right.shape.clone(),
            values: right
                .values
                .iter()
                .map(|right| f(left.values[0], *right))
                .collect(),
        };
    }
    assert_eq!(left.shape, right.shape, "elementwise shape mismatch");
    EvalTensor {
        shape: left.shape.clone(),
        values: left
            .values
            .iter()
            .zip(&right.values)
            .map(|(left, right)| f(*left, *right))
            .collect(),
    }
}

fn layer_norm(input: &EvalTensor) -> EvalTensor {
    assert_eq!(input.shape.len(), 1, "layer_norm input must be rank-1");
    let mean = input.values.iter().sum::<f32>() / input.values.len() as f32;
    let variance = input
        .values
        .iter()
        .map(|value| {
            let centered = value - mean;
            centered * centered
        })
        .sum::<f32>()
        / input.values.len() as f32;
    let inv_std = (variance + 1.0e-5).sqrt().recip();
    EvalTensor {
        shape: input.shape.clone(),
        values: input
            .values
            .iter()
            .map(|value| (value - mean) * inv_std)
            .collect(),
    }
}

fn softmax(input: &EvalTensor) -> EvalTensor {
    assert_eq!(input.shape.len(), 1, "softmax input must be rank-1");
    let max = input
        .values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let exp_values = input
        .values
        .iter()
        .map(|value| (value - max).exp())
        .collect::<Vec<_>>();
    let sum = exp_values.iter().sum::<f32>();
    EvalTensor {
        shape: input.shape.clone(),
        values: exp_values.into_iter().map(|value| value / sum).collect(),
    }
}

fn argmax_lowest_index(values: &[f32]) -> CharId {
    values
        .iter()
        .enumerate()
        .max_by(|(left_index, left), (right_index, right)| {
            left.partial_cmp(right)
                .expect("reference logits must be finite")
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index as CharId)
        .expect("reference logits must not be empty")
}

fn assert_finite_logits(values: &[f32]) {
    for (index, value) in values.iter().enumerate() {
        assert!(
            value.is_finite(),
            "reference program produced non-finite logit at index {index}: {value}"
        );
    }
}
