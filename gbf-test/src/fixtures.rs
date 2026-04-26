//! Deterministic tiny fixtures for fast integration tests.

use std::fmt::Write as _;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::tensor::CanonicalTensor;
use gbf_model::config::{
    DenseFfnConfig, ModelTopologyConfig, MoeBlockConfig, MoeFfnConfig, SharedSequenceConfig,
    SharedSequenceKind,
};
use gbf_model::embeddings::{EmbeddingConfig, EmbeddingTied};
use gbf_model::qat::{
    ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode,
    DenseBranchProjection, ExpertBlockQat, ExpertQat, ExportVisitor, ExportedQatArtifact,
    MatrixShape, RouterShape, TernaryLinearQat, TernaryThreshold, Top1RouterQat,
};

pub const TINY_D_MODEL: usize = 8;
pub const TINY_D_FF: usize = 16;
pub const TINY_N_EXPERTS: usize = 2;
pub const TINY_N_LAYERS: usize = 2;
pub const TINY_VOCAB_SIZE: usize = 32;
pub const TINY_MOE_BLOCK_INDEX: usize = 1;
pub const TINY_BOUNDED_KV_STATE_WIDTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinyModelConfig {
    d_model: usize,
    d_ff: usize,
    n_experts: usize,
    n_layers: usize,
    vocab_size: usize,
    moe_block_index: usize,
    sequence_kind: SharedSequenceKind,
    topology: ModelTopologyConfig,
    embedding: EmbeddingConfig,
}

impl TinyModelConfig {
    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn d_ff(&self) -> usize {
        self.d_ff
    }

    pub fn n_experts(&self) -> usize {
        self.n_experts
    }

    pub fn n_layers(&self) -> usize {
        self.n_layers
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    pub fn moe_block_index(&self) -> usize {
        self.moe_block_index
    }

    pub fn sequence_kind(&self) -> SharedSequenceKind {
        self.sequence_kind
    }

    pub fn topology(&self) -> &ModelTopologyConfig {
        &self.topology
    }

    pub fn embedding(&self) -> EmbeddingConfig {
        self.embedding
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TinyModel {
    config: TinyModelConfig,
    embedding: EmbeddingTied,
    dense_ffn: TinyDenseFfn,
    router: Top1RouterQat,
    expert_block: ExpertBlockQat,
}

impl TinyModel {
    pub fn config(&self) -> &TinyModelConfig {
        &self.config
    }

    pub fn embedding(&self) -> &EmbeddingTied {
        &self.embedding
    }

    pub fn dense_ffn(&self) -> &TinyDenseFfn {
        &self.dense_ffn
    }

    pub fn router(&self) -> &Top1RouterQat {
        &self.router
    }

    pub fn expert_block(&self) -> &ExpertBlockQat {
        &self.expert_block
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TinyDenseFfn {
    up_projection: DenseBranchProjection,
    activation: ActFakeQuant,
    down_projection: DenseBranchProjection,
}

impl TinyDenseFfn {
    pub fn up_projection(&self) -> &DenseBranchProjection {
        &self.up_projection
    }

    pub fn activation(&self) -> &ActFakeQuant {
        &self.activation
    }

    pub fn down_projection(&self) -> &DenseBranchProjection {
        &self.down_projection
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinyCompileRequestPlaceholder {
    artifact_name: String,
    profile: TinyCompileProfilePlaceholder,
    fixture_artifact_core_hash: String,
    workload_name: String,
}

impl TinyCompileRequestPlaceholder {
    pub fn artifact_name(&self) -> &str {
        &self.artifact_name
    }

    pub fn profile(&self) -> TinyCompileProfilePlaceholder {
        self.profile
    }

    pub fn fixture_artifact_core_hash(&self) -> &str {
        &self.fixture_artifact_core_hash
    }

    pub fn workload_name(&self) -> &str {
        &self.workload_name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TinyCompileProfilePlaceholder {
    Bringup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinyWorkloadManifestPlaceholder {
    name: String,
    prompts: Vec<Vec<u8>>,
}

impl TinyWorkloadManifestPlaceholder {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn prompts(&self) -> &[Vec<u8>] {
        &self.prompts
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestTensor {
    shape: Vec<usize>,
    values: Vec<f32>,
}

impl TestTensor {
    pub fn from_values(shape: Vec<usize>, values: Vec<f32>) -> Self {
        validate_shape_and_values(&shape, values.len());
        Self { shape, values }
    }

    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

pub fn tiny_moe_config() -> TinyModelConfig {
    let dense_block = MoeBlockConfig::dense_ffn(
        SharedSequenceConfig::bounded_kv(TINY_D_MODEL, TINY_BOUNDED_KV_STATE_WIDTH).unwrap(),
        DenseFfnConfig::new(TINY_D_MODEL, TINY_D_FF).unwrap(),
    )
    .unwrap();
    let moe_block = MoeBlockConfig::moe_ffn(
        SharedSequenceConfig::bounded_kv(TINY_D_MODEL, TINY_BOUNDED_KV_STATE_WIDTH).unwrap(),
        MoeFfnConfig::new(TINY_D_MODEL, TINY_D_FF, TINY_N_EXPERTS).unwrap(),
    )
    .unwrap();
    let topology = ModelTopologyConfig::new(vec![dense_block, moe_block]).unwrap();
    let embedding = EmbeddingConfig::tied(TINY_VOCAB_SIZE, TINY_D_MODEL).unwrap();

    TinyModelConfig {
        d_model: TINY_D_MODEL,
        d_ff: TINY_D_FF,
        n_experts: TINY_N_EXPERTS,
        n_layers: TINY_N_LAYERS,
        vocab_size: TINY_VOCAB_SIZE,
        moe_block_index: TINY_MOE_BLOCK_INDEX,
        sequence_kind: SharedSequenceKind::BoundedKv,
        topology,
        embedding,
    }
}

pub fn make_tiny_model() -> TinyModel {
    let config = tiny_moe_config();
    let weights =
        deterministic_tensor_by_name(&[config.vocab_size(), config.d_model()], "tiny.embedding");
    let embedding =
        EmbeddingTied::from_config(config.embedding(), weights.values().to_vec()).unwrap();
    let dense_ffn = tiny_dense_ffn(&config);
    let router = tiny_router(&config);
    let expert_block = tiny_expert_block(&config);

    TinyModel {
        config,
        embedding,
        dense_ffn,
        router,
        expert_block,
    }
}

pub fn make_tiny_artifact() -> ArtifactCore {
    make_tiny_exported_artifact().core
}

pub fn make_tiny_exported_artifact() -> ExportedQatArtifact {
    let model = make_tiny_model();
    let config = model.config();
    let embedding = model.embedding();
    let mut visitor = ExportVisitor::new();

    visitor
        .visit_embedding(
            "token_embedding",
            config.vocab_size(),
            config.d_model(),
            embedding.embedding_weights(),
        )
        .unwrap();
    visitor
        .visit_classifier(
            "classifier",
            config.vocab_size(),
            config.d_model(),
            embedding.classifier_weights(),
        )
        .unwrap();
    let dense_ffn = model.dense_ffn();
    visitor
        .visit_dense_projection("block.0.dense_ffn.up", dense_ffn.up_projection())
        .unwrap();
    visitor
        .visit_activation("block.0.dense_ffn.activation", dense_ffn.activation())
        .unwrap();
    visitor
        .visit_dense_projection("block.0.dense_ffn.down", dense_ffn.down_projection())
        .unwrap();
    visitor
        .visit_router("block.1.router", model.router())
        .unwrap();
    visitor
        .visit_expert_block("block.1.expert_block", model.expert_block())
        .unwrap();

    visitor.finish().unwrap()
}

pub fn make_tiny_compile_request() -> TinyCompileRequestPlaceholder {
    TinyCompileRequestPlaceholder {
        artifact_name: "tiny-moe".to_owned(),
        profile: TinyCompileProfilePlaceholder::Bringup,
        fixture_artifact_core_hash: make_tiny_artifact().semantic_hash().to_string(),
        workload_name: "tiny-prompts".to_owned(),
    }
}

pub fn make_tiny_workload() -> TinyWorkloadManifestPlaceholder {
    TinyWorkloadManifestPlaceholder {
        name: "tiny-prompts".to_owned(),
        prompts: tiny_prompt_corpus(),
    }
}

pub fn deterministic_tensor(shape: &[usize], seed: u64) -> TestTensor {
    let len = shape
        .iter()
        .try_fold(1usize, |acc, &dim| acc.checked_mul(dim))
        .expect("deterministic tensor shape length overflowed usize");
    validate_shape_and_values(shape, len);
    let mut state = seed;
    let values = (0..len)
        .map(|_| {
            state = splitmix64(state);
            unit_f32(state) * 2.0 - 1.0
        })
        .collect();

    TestTensor {
        shape: shape.to_vec(),
        values,
    }
}

pub fn deterministic_tensor_by_name(shape: &[usize], seed_name: &str) -> TestTensor {
    deterministic_tensor(shape, stable_seed(seed_name))
}

pub fn tiny_prompt_corpus() -> Vec<Vec<u8>> {
    vec![
        vec![1, 2, 3, 4],
        vec![4, 3, 2, 1, 0],
        vec![7, 8, 9, 10, 11, 12],
        vec![12, 11, 10, 9, 8, 7, 6],
        vec![0, 5, 10, 15, 20, 25, 30, 31],
        vec![3, 1, 4, 1, 5, 9, 2, 6, 5],
        vec![16, 17, 18, 19, 20, 21, 22, 23, 24, 25],
        vec![31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20],
    ]
}

pub fn assert_tensor_close(actual: &TestTensor, expected: &TestTensor, atol: f32, rtol: f32) {
    assert_tensor_values_close(
        actual.shape(),
        actual.values(),
        expected.shape(),
        expected.values(),
        atol,
        rtol,
    );
}

pub fn assert_tensor_values_close(
    actual_shape: &[usize],
    actual_values: &[f32],
    expected_shape: &[usize],
    expected_values: &[f32],
    atol: f32,
    rtol: f32,
) {
    assert_eq!(
        actual_shape, expected_shape,
        "tensor shape mismatch: actual {actual_shape:?}, expected {expected_shape:?}"
    );
    assert_eq!(
        actual_values.len(),
        expected_values.len(),
        "tensor value length mismatch for shape {actual_shape:?}: actual {}, expected {}",
        actual_values.len(),
        expected_values.len()
    );
    assert!(
        atol.is_finite() && atol >= 0.0,
        "atol must be finite and non-negative, got {atol}"
    );
    assert!(
        rtol.is_finite() && rtol >= 0.0,
        "rtol must be finite and non-negative, got {rtol}"
    );

    for (index, (&actual_value, &expected_value)) in
        actual_values.iter().zip(expected_values.iter()).enumerate()
    {
        let tolerance = atol + rtol * expected_value.abs();
        let diff = (actual_value - expected_value).abs();
        assert!(
            diff <= tolerance,
            "tensor mismatch at flat index {index}: actual {actual_value}, expected {expected_value}, diff {diff}, tolerance {tolerance}"
        );
    }
}

pub fn assert_f32_slice_close(actual: &[f32], expected: &[f32], atol: f32, rtol: f32) {
    assert_tensor_values_close(
        &[actual.len()],
        actual,
        &[expected.len()],
        expected,
        atol,
        rtol,
    );
}

pub fn assert_bytes_equal(actual: &[u8], expected: &[u8]) {
    if actual == expected {
        return;
    }

    let index = actual
        .iter()
        .zip(expected.iter())
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| actual.len().min(expected.len()));
    let actual_byte = actual.get(index).copied();
    let expected_byte = expected.get(index).copied();

    panic!(
        "byte mismatch at offset {index}: actual {}, expected {}; actual_len={}, expected_len={}; window actual=[{}], expected=[{}]",
        format_optional_byte(actual_byte),
        format_optional_byte(expected_byte),
        actual.len(),
        expected.len(),
        hex_window(actual, index),
        hex_window(expected, index)
    );
}

pub fn assert_artifact_core_valid(artifact: &ArtifactCore) {
    for tensor in artifact.tensors() {
        let reconstructed = CanonicalTensor::new(
            tensor.id.clone(),
            tensor.kind,
            tensor.layout.clone(),
            tensor.payload.clone(),
        )
        .expect("artifact core tensor must be internally self-consistent");
        assert_eq!(
            tensor.content_hash, reconstructed.content_hash,
            "artifact core tensor {} has stale content_hash",
            tensor.id
        );
    }
    ArtifactCore::new(artifact.tensors().to_vec(), artifact.quant().clone())
        .expect("tiny artifact must satisfy ArtifactCore invariants");
}

pub fn assert_artifact_valid(artifact: &ArtifactCore) {
    assert_artifact_core_valid(artifact);
}

fn tiny_dense_ffn(config: &TinyModelConfig) -> TinyDenseFfn {
    TinyDenseFfn {
        up_projection: tiny_dense_projection(config.d_ff(), config.d_model(), "tiny.dense_ffn.up"),
        activation: tiny_activation(),
        down_projection: tiny_dense_projection(
            config.d_model(),
            config.d_ff(),
            "tiny.dense_ffn.down",
        ),
    }
}

fn tiny_dense_projection(
    output_rows: usize,
    input_cols: usize,
    seed_name: &str,
) -> DenseBranchProjection {
    let weights = deterministic_tensor_by_name(&[output_rows, input_cols], seed_name);
    DenseBranchProjection::new(
        MatrixShape::new(output_rows, input_cols).unwrap(),
        weights.values().to_vec(),
        Some(vec![0.0; output_rows]),
    )
    .unwrap()
}

fn tiny_router(config: &TinyModelConfig) -> Top1RouterQat {
    let shape = RouterShape::with_default_rank(config.d_model(), config.n_experts()).unwrap();
    let input_projection =
        deterministic_tensor_by_name(&[shape.rank(), shape.d_model()], "tiny.router.input");
    let expert_projection =
        deterministic_tensor_by_name(&[shape.n_experts(), shape.rank()], "tiny.router.expert");

    Top1RouterQat::new(
        shape,
        input_projection.values().to_vec(),
        Some(vec![0.0; shape.rank()]),
        expert_projection.values().to_vec(),
        Some(vec![0.0; shape.n_experts()]),
    )
    .unwrap()
}

fn tiny_expert_block(config: &TinyModelConfig) -> ExpertBlockQat {
    let experts = (0..config.n_experts())
        .map(|expert_index| tiny_expert(config, expert_index))
        .collect();
    ExpertBlockQat::new(experts, None).unwrap()
}

fn tiny_expert(config: &TinyModelConfig, expert_index: usize) -> ExpertQat {
    let up = tiny_ternary_linear(
        config.d_ff(),
        config.d_model(),
        &format!("tiny.expert.{expert_index}.up"),
    );
    let down = tiny_ternary_linear(
        config.d_model(),
        config.d_ff(),
        &format!("tiny.expert.{expert_index}.down"),
    );
    ExpertQat::new(up, tiny_activation(), down).unwrap()
}

fn tiny_ternary_linear(output_rows: usize, input_cols: usize, seed_name: &str) -> TernaryLinearQat {
    let shape = MatrixShape::new(output_rows, input_cols).unwrap();
    let weights = deterministic_tensor_by_name(&[output_rows, input_cols], seed_name);
    let thresholds = vec![TernaryThreshold::from_f32_clamped_q8_8(0.25).unwrap(); output_rows];
    TernaryLinearQat::with_derived_per_row_scales(
        shape,
        weights.values().to_vec(),
        None,
        thresholds,
    )
    .unwrap()
}

fn tiny_activation() -> ActFakeQuant {
    ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-4.0, 4.0).unwrap()),
        ActivationQuantFormat::Int8,
    )
    .unwrap()
}

fn validate_shape_and_values(shape: &[usize], value_len: usize) {
    assert!(
        !shape.is_empty(),
        "test tensor shape must have at least one dimension"
    );
    assert!(
        shape.iter().all(|&dim| dim > 0),
        "test tensor shape must not contain zero dimensions: {shape:?}"
    );
    let expected_len = shape
        .iter()
        .try_fold(1usize, |acc, &dim| acc.checked_mul(dim))
        .expect("test tensor shape length overflowed usize");
    assert_eq!(
        value_len, expected_len,
        "test tensor value length mismatch for shape {shape:?}: actual {value_len}, expected {expected_len}"
    );
}

fn stable_seed(name: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn unit_f32(value: u64) -> f32 {
    let mantissa = (value >> 40) as u32;
    mantissa as f32 / 16_777_215.0
}

fn format_optional_byte(value: Option<u8>) -> String {
    value.map_or_else(|| "<missing>".to_owned(), |byte| format!("0x{byte:02x}"))
}

fn hex_window(bytes: &[u8], index: usize) -> String {
    if bytes.is_empty() {
        return "<empty>".to_owned();
    }

    let start = index.saturating_sub(4);
    let end = bytes.len().min(index.saturating_add(5));
    let mut output = String::new();
    for (offset, byte) in bytes[start..end].iter().enumerate() {
        if offset > 0 {
            output.push(' ');
        }
        write!(&mut output, "{byte:02x}").expect("writing to string cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use gbf_model::config::FfnPathConfig;

    use super::*;

    #[test]
    fn fixtures_tiny_moe_config_is_micro_and_instantiable() {
        let config = tiny_moe_config();

        assert_eq!(config.d_model(), 8);
        assert_eq!(config.d_ff(), 16);
        assert_eq!(config.n_experts(), 2);
        assert_eq!(config.n_layers(), 2);
        assert_eq!(config.vocab_size(), 32);
        assert_eq!(config.moe_block_index(), 1);
        assert_eq!(config.sequence_kind(), SharedSequenceKind::BoundedKv);
        assert_eq!(config.topology().blocks().len(), 2);
        assert!(matches!(
            config.topology().blocks()[0].ffn_path(),
            FfnPathConfig::Dense(_)
        ));
        assert!(matches!(
            config.topology().blocks()[1].ffn_path(),
            FfnPathConfig::Moe(_)
        ));

        let model = make_tiny_model();
        assert_eq!(model.embedding().vocab_size(), 32);
        assert_eq!(model.embedding().d_model(), 8);
        assert_eq!(model.embedding().parameter_count(), 32 * 8);
        assert_eq!(model.dense_ffn().up_projection().shape().output_rows(), 16);
        assert_eq!(model.dense_ffn().down_projection().shape().output_rows(), 8);
        assert_eq!(model.router().shape().n_experts(), 2);
        assert_eq!(model.expert_block().experts().len(), 2);
    }

    #[test]
    fn fixtures_deterministic_tensor_repeats_for_same_seed_and_name() {
        let seeded = deterministic_tensor(&[2, 3], 1234);
        assert_eq!(seeded, deterministic_tensor(&[2, 3], 1234));
        assert_ne!(seeded, deterministic_tensor(&[2, 3], 4321));

        let named = deterministic_tensor_by_name(&[2, 3], "fixtures.tensor");
        assert_eq!(
            named,
            deterministic_tensor_by_name(&[2, 3], "fixtures.tensor")
        );
        assert_ne!(
            named,
            deterministic_tensor_by_name(&[2, 3], "fixtures.other")
        );
        assert!(named.values().iter().all(|value| value.is_finite()));
    }

    #[test]
    fn fixtures_prompt_corpus_is_tiny_and_vocab_bounded() {
        let prompts = tiny_prompt_corpus();

        assert_eq!(prompts.len(), 8);
        assert!(
            prompts
                .iter()
                .all(|prompt| (4..=16).contains(&prompt.len()))
        );
        assert!(
            prompts
                .iter()
                .flatten()
                .all(|&token| usize::from(token) < TINY_VOCAB_SIZE)
        );
    }

    #[test]
    fn fixtures_make_tiny_artifact_is_valid_and_deterministic() {
        let first = make_tiny_artifact();
        let second = make_tiny_artifact();

        assert_artifact_valid(&first);
        assert_eq!(first.semantic_hash(), second.semantic_hash());
        assert!(
            first
                .tensors()
                .iter()
                .any(|tensor| tensor.id.to_string() == "block.0.dense_ffn.up.weight")
        );
        assert!(
            first
                .tensors()
                .iter()
                .any(|tensor| tensor.id.to_string() == "block.1.router.input_projection.weight")
        );
        assert!(
            first
                .tensors()
                .iter()
                .any(|tensor| tensor.id.to_string() == "block.1.expert_block.expert.0.up.weight")
        );
        assert!(
            first
                .quant()
                .activation_quant()
                .iter()
                .any(|entry| entry.activation.to_string() == "block.0.dense_ffn.activation")
        );
    }

    #[test]
    fn fixtures_make_compile_request_and_workload_are_consistent() {
        let request = make_tiny_compile_request();
        let workload = make_tiny_workload();

        assert_eq!(request.artifact_name(), "tiny-moe");
        assert_eq!(request.profile(), TinyCompileProfilePlaceholder::Bringup);
        assert_eq!(request.workload_name(), workload.name());
        assert_eq!(
            request.fixture_artifact_core_hash(),
            make_tiny_artifact().semantic_hash().to_string()
        );
        assert_eq!(workload.prompts(), tiny_prompt_corpus());
    }

    #[test]
    fn fixtures_assert_helpers_accept_matching_inputs() {
        let tensor = deterministic_tensor(&[2, 2], 7);
        assert_tensor_close(&tensor, &tensor, 0.0, 0.0);
        assert_f32_slice_close(&[1.0, 2.0], &[1.0, 2.0], 0.0, 0.0);
        assert_tensor_values_close(&[2], &[1.0, 2.0], &[2], &[1.0, 2.0], 0.0, 0.0);
        assert_bytes_equal(&[0xde, 0xad, 0xbe, 0xef], &[0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn fixtures_assert_helpers_report_first_bad_value_and_byte() {
        let tensor_panic = std::panic::catch_unwind(|| {
            assert_f32_slice_close(&[1.0, 3.0], &[1.0, 2.0], 0.0, 0.0);
        })
        .unwrap_err();
        assert!(panic_message(tensor_panic).contains("flat index 1"));

        let byte_panic = std::panic::catch_unwind(|| {
            assert_bytes_equal(&[0xde, 0x00, 0xbe], &[0xde, 0xad, 0xbe]);
        })
        .unwrap_err();
        let message = panic_message(byte_panic);
        assert!(message.contains("offset 1"));
        assert!(message.contains("0x00"));
        assert!(message.contains("0xad"));
    }

    #[test]
    fn fixtures_artifact_core_validation_rejects_stale_tensor_content_hash() {
        let artifact = make_tiny_artifact();
        let mut tensors = artifact.tensors().to_vec();
        tensors[0].content_hash = gbf_foundation::Hash256::ZERO;
        let corrupted = ArtifactCore::new(tensors, artifact.quant().clone()).unwrap();

        let panic =
            std::panic::catch_unwind(|| assert_artifact_core_valid(&corrupted)).unwrap_err();
        assert!(panic_message(panic).contains("stale content_hash"));
    }

    fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
        if let Some(message) = panic.downcast_ref::<String>() {
            message.clone()
        } else if let Some(message) = panic.downcast_ref::<&'static str>() {
            (*message).to_owned()
        } else {
            "<non-string panic>".to_owned()
        }
    }
}
