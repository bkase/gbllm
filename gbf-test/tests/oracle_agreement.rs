use std::collections::BTreeMap;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::quant::{
    ActivationEvalModeSpec, ActivationNonlinearitySpec, ActivationQuantEntry,
    ActivationQuantFormatSpec, ActivationRangeModeSpec, ActivationRangeSpec, QuantSpec,
    TernaryQuantEntry, WeightQuantEntry,
};
use gbf_artifact::sequence::SequenceSemanticsSpec;
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorPayload};
use gbf_model::embeddings::EmbeddingTied;
use gbf_model::qat::{
    ActivationForwardMode, DenseBranchProjection, ExpertForwardOptions, ExpertQatForwardMode,
    RouterForwardOptions,
};
use gbf_test::fixtures::{
    TinyDenseFfn, TinyModel, make_tiny_exported_artifact, make_tiny_model, tiny_prompt_corpus,
};
use serde_json::json;

const AGREEMENT_PROMPT_COUNT: usize = 4;
const TOP_K: usize = 5;
const TOP_K_TEMPERATURE: f32 = 0.75;

#[test]
fn oracle_agreement_training_hard_quant_forward_matches_artifact_evaluator() {
    let model = make_tiny_model();
    let exported = make_tiny_exported_artifact();
    let artifact = ArtifactEvaluator::new(&exported.core);

    for prompt in tiny_prompt_corpus()
        .into_iter()
        .take(AGREEMENT_PROMPT_COUNT)
    {
        let trace = training_hard_quant_trace(&model, &prompt);
        let artifact_trace = artifact.evaluate_prompt(&prompt);

        assert_prompt_trace_exact("hard-quant forward", &prompt, &artifact_trace, &trace);
    }
}

#[test]
fn oracle_agreement_records_structured_quantization_gap_metrics() {
    let model = make_tiny_model();
    let exported = make_tiny_exported_artifact();
    let artifact = ArtifactEvaluator::new(&exported.core);
    let mut gaps = Vec::new();

    for prompt in tiny_prompt_corpus()
        .into_iter()
        .take(AGREEMENT_PROMPT_COUNT)
    {
        let teacher = training_full_precision_trace(&model, &prompt);
        let quantized = artifact.evaluate_prompt(&prompt);
        gaps.push(quantization_gap_for_prompt(
            &prompt,
            &teacher.logits,
            &quantized.logits,
            model.config().vocab_size(),
        ));
    }

    let summary = QuantizationGapSummary::from_prompt_gaps(&gaps);
    assert!(summary.max_abs_diff.is_finite());
    assert!(summary.mean_abs_diff.is_finite());
    assert!(summary.kl_divergence.is_finite());
    assert_eq!(summary.prompt_count, AGREEMENT_PROMPT_COUNT);
    assert_eq!(
        summary.logit_count,
        gaps.iter().map(|gap| gap.logit_count).sum::<usize>()
    );

    assert_eq!(
        summary.to_json(),
        json!({
            "metric": "oracle_agreement.quantization_gap",
            "prompt_count": summary.prompt_count,
            "logit_count": summary.logit_count,
            "max_abs_diff": summary.max_abs_diff,
            "mean_abs_diff": summary.mean_abs_diff,
            "kl_divergence": summary.kl_divergence,
        })
    );
}

#[test]
fn oracle_agreement_fallback_evaluator_preserves_passthrough_activation_values() {
    let activation = ArtifactPath::new("activation").unwrap();
    let core = ArtifactCore::new(
        Vec::new(),
        QuantSpec::new_with_weight_quant(
            Vec::new(),
            Vec::new(),
            vec![ActivationQuantEntry {
                activation,
                range: ActivationRangeSpec {
                    lo: -1.0,
                    hi: 1.0,
                    mode: ActivationRangeModeSpec::Fixed,
                },
                quant_format: ActivationQuantFormatSpec::Int8,
                eval_mode: ActivationEvalModeSpec::Passthrough,
                nonlinearity: ActivationNonlinearitySpec::Identity,
            }],
            Vec::new(),
        ),
        SequenceSemanticsSpec::linear_state(8).unwrap(),
    )
    .unwrap();
    let evaluator = ArtifactEvaluator::new(&core);

    assert_eq!(
        evaluator.activation("activation", &[-2.0, 0.5, 2.0]),
        vec![-2.0, 0.5, 2.0],
        "identity passthrough activation must not clamp to the exported range"
    );
}

#[test]
fn oracle_agreement_quantization_gap_kl_is_per_token() {
    let teacher = [0.0, 0.0, 10.0, 0.0];
    let artifact = [0.0, 0.0, 0.0, 10.0];

    let gap = quantization_gap_for_prompt(&[1, 2], &teacher, &artifact, 2);
    let expected = (kl_divergence(&softmax(&teacher[0..2]), &softmax(&artifact[0..2]))
        + kl_divergence(&softmax(&teacher[2..4]), &softmax(&artifact[2..4])))
        / 2.0;
    let global = kl_divergence(&softmax(&teacher), &softmax(&artifact));

    assert_eq!(gap.kl_divergence, expected);
    assert_ne!(
        gap.kl_divergence, global,
        "prompt-level KL must average token vocab distributions, not mix all prompt logits"
    );
}

#[derive(Debug, Clone, PartialEq)]
struct PromptTrace {
    post_embedding: Vec<f32>,
    post_router_logits: Vec<f32>,
    post_router_probs: Vec<f32>,
    post_router_experts: Vec<usize>,
    post_expert_downcast: Vec<f32>,
    logits: Vec<f32>,
    argmax_decode: Vec<usize>,
    top_k_temperature_distributions: Vec<f32>,
}

struct ArtifactEvaluator<'a> {
    core: &'a ArtifactCore,
    tensors: BTreeMap<&'a str, &'a CanonicalTensor>,
}

impl<'a> ArtifactEvaluator<'a> {
    fn new(core: &'a ArtifactCore) -> Self {
        let tensors = core
            .tensors()
            .iter()
            .map(|tensor| (tensor.id.as_str(), tensor))
            .collect();
        Self { core, tensors }
    }

    fn evaluate_prompt(&self, prompt: &[u8]) -> PromptTrace {
        let mut trace = PromptTrace::empty();

        for &token in prompt {
            let post_embedding = self.embedding("token_embedding", usize::from(token));
            trace.post_embedding.extend_from_slice(&post_embedding);

            let post_dense = self.dense_ffn("block.0.dense_ffn", &post_embedding);
            let router = self.router("block.1.router", &post_dense);
            trace.post_router_logits.extend_from_slice(&router.logits);
            trace.post_router_probs.extend_from_slice(&router.probs);
            trace.post_router_experts.push(router.expert);

            let post_expert = self.expert_block("block.1.expert_block", &post_dense, router.expert);
            trace.post_expert_downcast.extend_from_slice(&post_expert);

            let logits = self.classifier("classifier", &post_expert);
            trace.logits.extend_from_slice(&logits);
            trace.argmax_decode.push(argmax(&logits));
            trace
                .top_k_temperature_distributions
                .extend(top_k_temperature_distribution(
                    &logits,
                    TOP_K,
                    TOP_K_TEMPERATURE,
                ));
        }

        trace
    }

    fn embedding(&self, path: &str, token_id: usize) -> Vec<f32> {
        let (values, rows, cols) = self.f32_matrix_for_weight(path);
        assert!(
            token_id < rows,
            "token id {token_id} exceeds artifact embedding rows {rows}"
        );
        values[token_id * cols..(token_id + 1) * cols].to_vec()
    }

    fn classifier(&self, path: &str, input: &[f32]) -> Vec<f32> {
        self.dense_linear(path, input)
    }

    fn dense_ffn(&self, prefix: &str, input: &[f32]) -> Vec<f32> {
        let hidden = self.dense_linear(&format!("{prefix}.up"), input);
        let activated = self.activation(&format!("{prefix}.activation"), &hidden);
        let delta = self.dense_linear(&format!("{prefix}.down"), &activated);
        residual_add(input, &delta)
    }

    fn router(&self, prefix: &str, input: &[f32]) -> RouterEval {
        let hidden = self.dense_linear(&format!("{prefix}.input_projection"), input);
        let logits = self.dense_linear(&format!("{prefix}.expert_projection"), &hidden);
        let probs = softmax(&logits);
        let expert = argmax(&logits);

        RouterEval {
            logits,
            probs,
            expert,
        }
    }

    fn expert_block(&self, prefix: &str, input: &[f32], expert_id: usize) -> Vec<f32> {
        let expert_prefix = format!("{prefix}.expert.{expert_id}");
        let hidden = self.ternary_linear(&format!("{expert_prefix}.up"), input);
        let activated = self.activation(&format!("{expert_prefix}.activation"), &hidden);
        let delta = self.ternary_linear(&format!("{expert_prefix}.down"), &activated);
        residual_add(input, &delta)
    }

    fn ternary_linear(&self, projection: &str, input: &[f32]) -> Vec<f32> {
        let entry = self.ternary_entry(projection);
        let weight = self.i8_tensor(entry.weight.as_str());
        let scales = self.u16_tensor(entry.scale.as_str());
        let rows = weight.layout.shape.dims()[0] as usize;
        let cols = weight.layout.shape.dims()[1] as usize;
        assert_eq!(
            input.len(),
            cols,
            "ternary projection {projection} input length mismatch"
        );
        assert_eq!(
            scales.len(),
            rows,
            "ternary projection {projection} scale length mismatch"
        );
        let bias = entry.bias.as_ref().map(|id| self.f32_vector(id.as_str()));

        weight
            .payload
            .as_i8_slice()
            .expect("ternary weight tensor must carry i8 payload")
            .chunks_exact(cols)
            .zip(scales)
            .enumerate()
            .map(|(row_index, (row, &scale_q8_8))| {
                let scale = f32::from(scale_q8_8) / 256.0;
                let weighted_sum = row
                    .iter()
                    .zip(input)
                    .map(|(&weight, &value)| f32::from(weight) * scale * value)
                    .sum::<f32>();
                weighted_sum + bias.as_ref().map_or(0.0, |values| values[row_index])
            })
            .collect()
    }

    fn activation(&self, path: &str, input: &[f32]) -> Vec<f32> {
        let entry = self.activation_entry(path);
        input
            .iter()
            .copied()
            .map(|value| {
                let value = apply_nonlinearity(value, entry.nonlinearity, entry.range);
                match entry.eval_mode {
                    ActivationEvalModeSpec::Quantized => {
                        fake_quantize_value(value, entry.range, entry.quant_format)
                    }
                    ActivationEvalModeSpec::Passthrough => value,
                }
            })
            .collect()
    }

    fn dense_linear(&self, weight_path: &str, input: &[f32]) -> Vec<f32> {
        let (weights, rows, cols) = self.f32_matrix_for_weight(weight_path);
        assert_eq!(
            input.len(),
            cols,
            "dense projection {weight_path} input length mismatch"
        );
        let bias_id = Some(format!("{weight_path}.bias"));
        let bias = bias_id
            .as_deref()
            .and_then(|id| self.tensors.get(id).map(|_| self.f32_vector(id)));

        weights
            .chunks_exact(cols)
            .enumerate()
            .take(rows)
            .map(|(row_index, row)| {
                let weighted_sum = row
                    .iter()
                    .zip(input)
                    .map(|(&weight, &value)| weight * value)
                    .sum::<f32>();
                weighted_sum + bias.as_ref().map_or(0.0, |values| values[row_index])
            })
            .collect()
    }

    fn f32_matrix_for_weight(&self, weight_path: &str) -> (&[f32], usize, usize) {
        let entry = self.full_precision_weight_entry(weight_path);
        let tensor = self.tensor(entry.tensor.as_str());
        self.f32_matrix_from_tensor(tensor, weight_path)
    }

    fn f32_matrix_from_tensor(
        &self,
        tensor: &'a CanonicalTensor,
        weight_path: &str,
    ) -> (&[f32], usize, usize) {
        let dims = tensor.layout.shape.dims();
        assert_eq!(
            dims.len(),
            2,
            "weight {weight_path} tensor must be a matrix"
        );
        let values = tensor
            .payload
            .as_f32_slice()
            .unwrap_or_else(|| panic!("weight {weight_path} tensor must carry f32 payload"));
        (values, dims[0] as usize, dims[1] as usize)
    }

    fn f32_vector(&self, id: &str) -> &[f32] {
        let tensor = self.tensor(id);
        let dims = tensor.layout.shape.dims();
        assert_eq!(dims.len(), 1, "tensor {id} must be a vector");
        tensor
            .payload
            .as_f32_slice()
            .unwrap_or_else(|| panic!("tensor {id} must carry f32 payload"))
    }

    fn i8_tensor(&self, id: &str) -> &CanonicalTensor {
        let tensor = self.tensor(id);
        assert!(
            matches!(tensor.payload, CanonicalTensorPayload::I8(_)),
            "tensor {id} must carry i8 payload"
        );
        tensor
    }

    fn u16_tensor(&self, id: &str) -> &[u16] {
        self.tensor(id)
            .payload
            .as_u16_slice()
            .unwrap_or_else(|| panic!("tensor {id} must carry u16 payload"))
    }

    fn tensor(&self, id: &str) -> &'a CanonicalTensor {
        self.tensors
            .get(id)
            .copied()
            .unwrap_or_else(|| panic!("missing artifact tensor {id}"))
    }

    fn ternary_entry(&self, projection: &str) -> &TernaryQuantEntry {
        let projection = ArtifactPath::new(projection)
            .unwrap_or_else(|error| panic!("invalid projection path {projection}: {error}"));
        self.core
            .quant()
            .ternary_weight_plans()
            .iter()
            .find(|entry| entry.projection == projection)
            .unwrap_or_else(|| panic!("missing ternary quant entry for {projection}"))
    }

    fn activation_entry(&self, path: &str) -> &ActivationQuantEntry {
        let path = ArtifactPath::new(path)
            .unwrap_or_else(|error| panic!("invalid activation path {path}: {error}"));
        self.core
            .quant()
            .activation_quant()
            .iter()
            .find(|entry| entry.activation == path)
            .unwrap_or_else(|| panic!("missing activation quant entry for {path}"))
    }

    fn full_precision_weight_entry(&self, path: &str) -> &WeightQuantEntry {
        let path = ArtifactPath::new(path)
            .unwrap_or_else(|error| panic!("invalid weight path {path}: {error}"));
        let entry = self
            .core
            .quant()
            .weight_quant()
            .iter()
            .find(|entry| entry.weight == path)
            .unwrap_or_else(|| panic!("missing weight quant entry for {path}"));
        assert!(
            entry.ternary_plan.is_none(),
            "weight {path} must be full precision in this evaluator path"
        );
        entry
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RouterEval {
    logits: Vec<f32>,
    probs: Vec<f32>,
    expert: usize,
}

impl PromptTrace {
    fn empty() -> Self {
        Self {
            post_embedding: Vec::new(),
            post_router_logits: Vec::new(),
            post_router_probs: Vec::new(),
            post_router_experts: Vec::new(),
            post_expert_downcast: Vec::new(),
            logits: Vec::new(),
            argmax_decode: Vec::new(),
            top_k_temperature_distributions: Vec::new(),
        }
    }
}

fn training_hard_quant_trace(model: &TinyModel, prompt: &[u8]) -> PromptTrace {
    training_trace(
        model,
        prompt,
        ExpertForwardOptions::hard_quantized_train(),
        ActivationForwardMode::Train,
    )
}

fn training_full_precision_trace(model: &TinyModel, prompt: &[u8]) -> PromptTrace {
    training_trace(
        model,
        prompt,
        ExpertForwardOptions::full_precision_train()
            .with_expert_qat(ExpertQatForwardMode::FullPrecision)
            .with_activation(ActivationForwardMode::Passthrough),
        ActivationForwardMode::Passthrough,
    )
}

fn training_trace(
    model: &TinyModel,
    prompt: &[u8],
    expert_options: ExpertForwardOptions,
    dense_activation_mode: ActivationForwardMode,
) -> PromptTrace {
    let mut trace = PromptTrace::empty();
    let mut router = model.router().clone();
    router.reset_sequence();

    for &token in prompt {
        let post_embedding = model
            .embedding()
            .embed_one(usize::from(token))
            .unwrap_or_else(|error| panic!("tiny prompt token {token} should embed: {error}"))
            .to_vec();
        trace.post_embedding.extend_from_slice(&post_embedding);

        let post_dense =
            training_dense_ffn(model.dense_ffn(), &post_embedding, dense_activation_mode);
        let router_output = router
            .forward_with_options(
                &post_dense,
                &RouterForwardOptions::hard_top1(model.config().n_experts()),
            )
            .unwrap_or_else(|error| panic!("tiny router forward should succeed: {error}"));
        trace
            .post_router_logits
            .extend_from_slice(router_output.logits());
        trace
            .post_router_probs
            .extend_from_slice(router_output.soft_probs());
        trace.post_router_experts.push(router_output.expert_index());

        let post_expert = model
            .expert_block()
            .forward_with_options(&post_dense, router_output.expert_index(), expert_options)
            .unwrap_or_else(|error| panic!("tiny expert block forward should succeed: {error}"));
        trace.post_expert_downcast.extend_from_slice(&post_expert);

        let logits = classify(model.embedding(), &post_expert);
        trace.logits.extend_from_slice(&logits);
        trace.argmax_decode.push(argmax(&logits));
        trace
            .top_k_temperature_distributions
            .extend(top_k_temperature_distribution(
                &logits,
                TOP_K,
                TOP_K_TEMPERATURE,
            ));
    }

    trace
}

fn training_dense_ffn(
    dense_ffn: &TinyDenseFfn,
    input: &[f32],
    activation_mode: ActivationForwardMode,
) -> Vec<f32> {
    let hidden = dense_projection(dense_ffn.up_projection(), input);
    let activated = dense_ffn
        .activation()
        .inference_forward(&hidden, activation_mode)
        .expect("tiny dense activation should accept finite hidden values");
    let delta = dense_projection(dense_ffn.down_projection(), &activated);
    residual_add(input, &delta)
}

fn dense_projection(projection: &DenseBranchProjection, input: &[f32]) -> Vec<f32> {
    let shape = projection.shape();
    assert_eq!(input.len(), shape.input_cols());
    projection
        .weights()
        .chunks_exact(shape.input_cols())
        .enumerate()
        .map(|(row_index, row)| {
            let weighted_sum = row
                .iter()
                .zip(input)
                .map(|(&weight, &value)| weight * value)
                .sum::<f32>();
            weighted_sum + projection.bias().map_or(0.0, |bias| bias[row_index])
        })
        .collect()
}

fn classify(embedding: &EmbeddingTied, hidden: &[f32]) -> Vec<f32> {
    embedding
        .classify(hidden)
        .expect("tiny classifier should accept finite hidden values")
        .values()
        .to_vec()
}

fn residual_add(input: &[f32], delta: &[f32]) -> Vec<f32> {
    assert_eq!(
        input.len(),
        delta.len(),
        "residual add requires equal vector lengths"
    );
    input
        .iter()
        .zip(delta)
        .map(|(&residual, &delta)| residual + delta)
        .collect()
}

fn apply_nonlinearity(
    value: f32,
    nonlinearity: ActivationNonlinearitySpec,
    range: ActivationRangeSpec,
) -> f32 {
    match nonlinearity {
        ActivationNonlinearitySpec::Identity => value,
        ActivationNonlinearitySpec::Relu => range.clamp(value.max(0.0)),
        ActivationNonlinearitySpec::GeluClip => range.clamp(gelu(value)),
        ActivationNonlinearitySpec::SiluClip => range.clamp(silu(value)),
    }
}

fn fake_quantize_value(
    value: f32,
    range: ActivationRangeSpec,
    quant_format: ActivationQuantFormatSpec,
) -> f32 {
    let clamped = range.clamp(value);
    if matches!(quant_format, ActivationQuantFormatSpec::Int8) {
        let qmax = f64::from(quant_steps(quant_format));
        let max_abs = f64::from(range.lo.abs().max(range.hi.abs()));
        let quantized = (f64::from(clamped) * qmax / max_abs)
            .round()
            .clamp(-qmax, qmax);
        range.clamp((quantized * max_abs / qmax) as f32)
    } else {
        let qmax = f64::from(quant_steps(quant_format));
        let lo = f64::from(range.lo);
        let width = f64::from(range.hi) - lo;
        let quantized = ((f64::from(clamped) - lo) * qmax / width)
            .round()
            .clamp(0.0, qmax);
        range.clamp((quantized * width / qmax + lo) as f32)
    }
}

fn quant_steps(quant_format: ActivationQuantFormatSpec) -> u16 {
    match quant_format {
        ActivationQuantFormatSpec::Int8 => 127,
        ActivationQuantFormatSpec::UInt8 => 255,
        ActivationQuantFormatSpec::UInt4 => 15,
    }
}

trait ClampRange {
    fn clamp(self, value: f32) -> f32;
}

impl ClampRange for ActivationRangeSpec {
    fn clamp(self, value: f32) -> f32 {
        value.clamp(self.lo, self.hi)
    }
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_values = logits
        .iter()
        .map(|&logit| (logit - max).exp())
        .collect::<Vec<_>>();
    let sum = exp_values.iter().sum::<f32>();
    exp_values.into_iter().map(|value| value / sum).collect()
}

fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            left.partial_cmp(right)
                .expect("oracle agreement logits must be finite")
                .then(std::cmp::Ordering::Greater)
        })
        .map(|(index, _)| index)
        .expect("argmax requires at least one value")
}

fn top_k_temperature_distribution(logits: &[f32], k: usize, temperature: f32) -> Vec<f32> {
    assert!(temperature.is_finite() && temperature > 0.0);
    let mut ranked = logits.iter().copied().enumerate().collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        right
            .partial_cmp(left)
            .expect("top-k logits must be finite")
            .then_with(|| left_index.cmp(right_index))
    });

    let mut keep = vec![false; logits.len()];
    for (index, _) in ranked.into_iter().take(k.min(logits.len())) {
        keep[index] = true;
    }

    let max = logits
        .iter()
        .zip(&keep)
        .filter_map(|(&logit, &keep)| keep.then_some(logit / temperature))
        .fold(f32::NEG_INFINITY, f32::max);
    let mut values = logits
        .iter()
        .zip(keep)
        .map(|(&logit, keep)| {
            if keep {
                (logit / temperature - max).exp()
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    let sum = values.iter().sum::<f32>();
    for value in &mut values {
        *value /= sum;
    }
    values
}

fn gelu(value: f32) -> f32 {
    const SQRT_2_OVER_PI: f32 = 0.797_884_6;
    0.5 * value * (1.0 + (SQRT_2_OVER_PI * (value + 0.044_715 * value.powi(3))).tanh())
}

fn silu(value: f32) -> f32 {
    value / (1.0 + (-value).exp())
}

fn assert_prompt_trace_exact(
    label: &str,
    prompt: &[u8],
    actual: &PromptTrace,
    expected: &PromptTrace,
) {
    assert_f32_checkpoint_exact(
        label,
        prompt,
        "PostEmbedding",
        &actual.post_embedding,
        &expected.post_embedding,
    );
    assert_f32_checkpoint_exact(
        label,
        prompt,
        "PostRouter.logits",
        &actual.post_router_logits,
        &expected.post_router_logits,
    );
    assert_f32_checkpoint_exact(
        label,
        prompt,
        "PostRouter.probs",
        &actual.post_router_probs,
        &expected.post_router_probs,
    );
    assert_eq!(
        actual.post_router_experts, expected.post_router_experts,
        "{label} prompt {prompt:?} PostRouter expert ids mismatch"
    );
    assert_f32_checkpoint_exact(
        label,
        prompt,
        "PostExpertDowncast",
        &actual.post_expert_downcast,
        &expected.post_expert_downcast,
    );
    assert_f32_checkpoint_exact(
        label,
        prompt,
        "PostLogits",
        &actual.logits,
        &expected.logits,
    );
    assert_eq!(
        actual.argmax_decode, expected.argmax_decode,
        "{label} prompt {prompt:?} PostDecode argmax mismatch"
    );
    assert_f32_checkpoint_exact(
        label,
        prompt,
        "TopKTemperature.distribution",
        &actual.top_k_temperature_distributions,
        &expected.top_k_temperature_distributions,
    );
}

fn assert_f32_checkpoint_exact(
    label: &str,
    prompt: &[u8],
    checkpoint: &str,
    actual: &[f32],
    expected: &[f32],
) {
    assert_eq!(
        actual, expected,
        "{label} prompt {prompt:?} {checkpoint} mismatch"
    );
}

#[derive(Debug, Clone, PartialEq)]
struct QuantizationGap {
    logit_count: usize,
    max_abs_diff: f32,
    mean_abs_diff: f32,
    kl_divergence: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct QuantizationGapSummary {
    prompt_count: usize,
    logit_count: usize,
    max_abs_diff: f32,
    mean_abs_diff: f32,
    kl_divergence: f32,
}

impl QuantizationGapSummary {
    fn from_prompt_gaps(gaps: &[QuantizationGap]) -> Self {
        let prompt_count = gaps.len();
        let logit_count = gaps.iter().map(|gap| gap.logit_count).sum::<usize>();
        let max_abs_diff = gaps
            .iter()
            .map(|gap| gap.max_abs_diff)
            .fold(0.0_f32, f32::max);
        let mean_abs_diff = gaps
            .iter()
            .map(|gap| gap.mean_abs_diff * gap.logit_count as f32)
            .sum::<f32>()
            / logit_count as f32;
        let kl_divergence =
            gaps.iter().map(|gap| gap.kl_divergence).sum::<f32>() / prompt_count as f32;

        Self {
            prompt_count,
            logit_count,
            max_abs_diff,
            mean_abs_diff,
            kl_divergence,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "metric": "oracle_agreement.quantization_gap",
            "prompt_count": self.prompt_count,
            "logit_count": self.logit_count,
            "max_abs_diff": self.max_abs_diff,
            "mean_abs_diff": self.mean_abs_diff,
            "kl_divergence": self.kl_divergence,
        })
    }
}

fn quantization_gap_for_prompt(
    prompt: &[u8],
    teacher_logits: &[f32],
    artifact_logits: &[f32],
    vocab_size: usize,
) -> QuantizationGap {
    assert_all_finite("teacher logits", teacher_logits);
    assert_all_finite("artifact logits", artifact_logits);
    assert!(vocab_size > 0, "vocab size must be nonzero");
    assert_eq!(
        teacher_logits.len(),
        artifact_logits.len(),
        "prompt {prompt:?} teacher/artifact logit count mismatch"
    );
    assert_eq!(
        teacher_logits.len() % vocab_size,
        0,
        "prompt {prompt:?} teacher logits must be token-major vocab rows"
    );
    assert_eq!(
        teacher_logits.len() / vocab_size,
        prompt.len(),
        "prompt {prompt:?} logit rows must match prompt length"
    );

    let mut max_abs_diff = 0.0_f32;
    let mut abs_diff_sum = 0.0_f32;
    for (&teacher, &artifact) in teacher_logits.iter().zip(artifact_logits) {
        let diff = (teacher - artifact).abs();
        max_abs_diff = max_abs_diff.max(diff);
        abs_diff_sum += diff;
    }

    QuantizationGap {
        logit_count: teacher_logits.len(),
        max_abs_diff,
        mean_abs_diff: abs_diff_sum / teacher_logits.len() as f32,
        kl_divergence: token_mean_kl_divergence(teacher_logits, artifact_logits, vocab_size),
    }
}

fn token_mean_kl_divergence(
    reference_logits: &[f32],
    observed_logits: &[f32],
    vocab_size: usize,
) -> f32 {
    let token_count = reference_logits.len() / vocab_size;
    reference_logits
        .chunks_exact(vocab_size)
        .zip(observed_logits.chunks_exact(vocab_size))
        .map(|(reference, observed)| kl_divergence(&softmax(reference), &softmax(observed)))
        .sum::<f32>()
        / token_count as f32
}

fn kl_divergence(reference: &[f32], observed: &[f32]) -> f32 {
    assert_eq!(reference.len(), observed.len());
    reference
        .iter()
        .zip(observed)
        .filter(|(reference, _)| **reference > 0.0)
        .map(|(&reference, &observed)| {
            reference * (reference / observed.max(f32::MIN_POSITIVE)).ln()
        })
        .sum()
}

fn assert_all_finite(name: &str, values: &[f32]) {
    if let Some((index, value)) = values
        .iter()
        .copied()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        panic!("{name} must be finite at index {index}, got {value}");
    }
}
