//! H8 Burn-backed ExpertBlockQat gradient smoke producer.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256};
use gbf_model::qat::{
    ActFakeQuant, ActFakeQuantError, ActivationQuantFormat, ActivationRange, ActivationRangeMode,
    ActivationRangeModeKind, ClippedActivation, EmaDecay, ExpertBlockQat, ExpertBlockQatError,
    ExpertForwardOptions, ExpertMlpConfig, ExpertMlpVariant, ExpertQat, MatrixShape, Q8_8Scale,
    TernaryLinearQat, TernaryLinearQatError, TernaryThreshold,
};
use gbf_train::adapter::burn::{
    BurnAdapterError, BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec,
    float_tensor_into_vec,
};
use gbf_train::qat::{
    ActFakeQuantBurnQat, ActFakeQuantBurnQatError, ExpertBlockBurnQat, ExpertBlockBurnQatError,
};
use serde::Serialize;
use serde_json::{Value, json};

const FIXTURE_SEED: u64 = 0xFEED;
const FIXTURE_INPUT: [f32; 2] = [0.25, 0.5];
const UP_WEIGHTS: [f32; 6] = [
    1.0, 0.0, //
    0.0, 1.0, //
    0.25, 0.25,
];
const DOWN_WEIGHTS: [f32; 6] = [
    1.0, 0.0, 0.0, //
    0.0, -1.0, 0.0,
];
const BURN_ADAPTER_VERSION: &str = concat!(
    "gbf-train/burn-adapter;burn=0.21.0-pre.3;gbf-experiments=",
    env!("CARGO_PKG_VERSION")
);
const DEFAULT_OUTPUT: &str = "experiments/S7/burn-grad-smoke/expert_block_qat.json";
const S7_BURN_GRAD_SMOKE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7BurnGradSmokeReport",
    "s7_burn_grad_smoke.v1",
    "1",
);
const S7_BURN_GRAD_SMOKE_FIXTURE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S7BurnGradSmokeFixture",
    "s7_burn_grad_smoke_fixture.v1",
    "1",
);

/// Inputs for producing the H8 Burn gradient smoke artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7BurnGradSmokeInputs {
    /// Packet/repository root used when `output` is relative.
    pub root: PathBuf,
    /// Output path for `s7_burn_grad_smoke.v1`, relative to `root` unless absolute.
    pub output: PathBuf,
}

impl Default for S7BurnGradSmokeInputs {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            output: PathBuf::from(DEFAULT_OUTPUT),
        }
    }
}

/// Produced H8 Burn gradient smoke artifact metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7BurnGradSmokeArtifact {
    /// JSON artifact path written by the producer.
    pub output_path: PathBuf,
    /// Verified report self-hash.
    pub smoke_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BurnGradSmokeMeasurements {
    fixture_input_sha: Hash256,
    grad_up_weight_sum_abs: f64,
    grad_down_weight_sum_abs: f64,
    supported_clipped_activation_count: u64,
    learned_activation_range_unsupported: bool,
    projection_biases_unsupported: bool,
    glu_construction_rejected: bool,
}

/// Produce `s7_burn_grad_smoke.v1` from a deterministic differentiable Burn fixture.
pub fn produce_burn_grad_smoke_artifact(
    inputs: &S7BurnGradSmokeInputs,
) -> Result<S7BurnGradSmokeArtifact, S7BurnGradSmokeError> {
    let first = compute_measurements()?;
    let second = compute_measurements()?;
    if first != second {
        return Err(S7BurnGradSmokeError::ReplayMismatch);
    }

    let mut report = stable_canonical_value(&report_value(&first, true))?;
    let smoke_self_hash = smoke_self_hash(&report)?;
    report
        .as_object_mut()
        .expect("burn grad smoke report is a JSON object")
        .insert(
            "smoke_self_hash".to_owned(),
            Value::String(smoke_self_hash.to_string()),
        );
    let report = stable_canonical_value(&report)?;

    let output_path = resolve_under_root(&inputs.root, &inputs.output);
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| S7BurnGradSmokeError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(&output_path, CanonicalJson::value_to_vec(&report)?).map_err(|source| {
        S7BurnGradSmokeError::Io {
            path: output_path.display().to_string(),
            source,
        }
    })?;

    Ok(S7BurnGradSmokeArtifact {
        output_path,
        smoke_self_hash,
    })
}

fn compute_measurements() -> Result<BurnGradSmokeMeasurements, S7BurnGradSmokeError> {
    let mut grad_up_weight_sum_abs = 0.0;
    let mut grad_down_weight_sum_abs = 0.0;
    let mut supported_clipped_activation_count = 0;

    for clipped_activation in [
        ClippedActivation::relu(),
        ClippedActivation::gelu_clip(),
        ClippedActivation::silu_clip(),
    ] {
        let grad = activation_gradient_sums(clipped_activation)?;
        grad_up_weight_sum_abs += grad.up_weight_sum_abs;
        grad_down_weight_sum_abs += grad.down_weight_sum_abs;
        supported_clipped_activation_count += 1;
    }

    Ok(BurnGradSmokeMeasurements {
        fixture_input_sha: fixture_input_sha()?,
        grad_up_weight_sum_abs,
        grad_down_weight_sum_abs,
        supported_clipped_activation_count,
        learned_activation_range_unsupported: learned_activation_range_unsupported()?,
        projection_biases_unsupported: projection_biases_unsupported()?,
        glu_construction_rejected: glu_construction_rejected()?,
    })
}

#[derive(Debug, Clone, Copy)]
struct GradientSums {
    up_weight_sum_abs: f64,
    down_weight_sum_abs: f64,
}

fn activation_gradient_sums(
    clipped_activation: ClippedActivation,
) -> Result<GradientSums, S7BurnGradSmokeError> {
    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let core = ExpertBlockQat::without_shared_dense(vec![fixture_expert(clipped_activation)?])?;
    let layer = ExpertBlockBurnQat::<B>::from_core(core, &device)?;
    let input = float_tensor_from_vec::<B, 1>(FIXTURE_INPUT.to_vec(), [2], &device)?.require_grad();
    let output = layer.forward(input, 0, ExpertForwardOptions::hard_quantized_train())?;
    let loss = (output.clone() * output).sum();
    let gradients = loss.backward();
    let expert = &layer.experts()[0];
    let activation_name = clipped_activation_name(clipped_activation);
    let up_gradient = expert
        .up_projection()
        .full_precision_weights()
        .grad(&gradients)
        .ok_or(S7BurnGradSmokeError::MissingGradient {
            projection: "up_projection",
            activation: activation_name,
        })?;
    let down_gradient = expert
        .down_projection()
        .full_precision_weights()
        .grad(&gradients)
        .ok_or(S7BurnGradSmokeError::MissingGradient {
            projection: "down_projection",
            activation: activation_name,
        })?;

    Ok(GradientSums {
        up_weight_sum_abs: finite_positive_sum_abs(
            "up_projection",
            activation_name,
            &float_tensor_into_vec(up_gradient)?,
        )?,
        down_weight_sum_abs: finite_positive_sum_abs(
            "down_projection",
            activation_name,
            &float_tensor_into_vec(down_gradient)?,
        )?,
    })
}

fn learned_activation_range_unsupported() -> Result<bool, S7BurnGradSmokeError> {
    let learned = ActFakeQuant::new(
        ActivationRangeMode::Learned(activation_range()?),
        ActivationQuantFormat::UInt4,
    )?;
    let ema = ActFakeQuant::new(
        ActivationRangeMode::Ema {
            range: activation_range()?,
            decay: EmaDecay::new(0.25)?,
        },
        ActivationQuantFormat::UInt8,
    )?;

    Ok(matches!(
        ActFakeQuantBurnQat::from_core(learned),
        Err(ActFakeQuantBurnQatError::UnsupportedRangeMode {
            mode: ActivationRangeModeKind::Learned,
        })
    ) && matches!(
        ActFakeQuantBurnQat::from_core(ema),
        Err(ActFakeQuantBurnQatError::UnsupportedRangeMode {
            mode: ActivationRangeModeKind::Ema,
        })
    ))
}

fn projection_biases_unsupported() -> Result<bool, S7BurnGradSmokeError> {
    let biased_up = ternary_linear(3, 2, &UP_WEIGHTS, Some(vec![0.125, 0.0, -0.125]))?;
    let down = ternary_linear(2, 3, &DOWN_WEIGHTS, None)?;

    Ok(matches!(
        ExpertQat::new(biased_up, activation()?, down),
        Err(ExpertBlockQatError::ExpertBiasUnsupported {
            projection: "up_projection",
        })
    ))
}

fn glu_construction_rejected() -> Result<bool, S7BurnGradSmokeError> {
    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let core =
        ExpertBlockQat::without_shared_dense(vec![fixture_expert(ClippedActivation::relu())?])?;
    let (glu, _event) = ExpertMlpConfig::glu_explicit(2, 3)?;

    Ok(matches!(
        ExpertBlockBurnQat::<B>::from_core_with_config(core, glu, &device),
        Err(ExpertBlockBurnQatError::Model(
            ExpertBlockQatError::UnsupportedExpertMlpVariant {
                variant: ExpertMlpVariant::GatedLinearUnit,
            }
        ))
    ))
}

fn fixture_input_sha() -> Result<Hash256, S7BurnGradSmokeError> {
    Ok(S7_BURN_GRAD_SMOKE_FIXTURE_DOMAIN.hash(&json!({
        "schema": "s7_burn_grad_smoke_fixture.v1",
        "fixture_seed": FIXTURE_SEED,
        "loss": "sum(expert_output ** 2)",
        "fixture_input": FIXTURE_INPUT,
        "up_projection": {
            "shape": [3, 2],
            "weights": UP_WEIGHTS,
            "bias": null,
        },
        "down_projection": {
            "shape": [2, 3],
            "weights": DOWN_WEIGHTS,
            "bias": null,
        },
        "activation_range": {
            "lo": -1.0,
            "hi": 1.0,
        },
        "activation_quant_format": "Int8",
        "supported_clipped_activations": ["relu", "gelu_clip", "silu_clip"],
    }))?)
}

fn report_value(measurements: &BurnGradSmokeMeasurements, replay_byte_identical: bool) -> Value {
    json!({
        "schema": "s7_burn_grad_smoke.v1",
        "fixture_seed": FIXTURE_SEED,
        "burn_adapter_version": BURN_ADAPTER_VERSION,
        "fixture_input_sha": measurements.fixture_input_sha,
        "grad_up_weight_sum_abs": measurements.grad_up_weight_sum_abs,
        "grad_down_weight_sum_abs": measurements.grad_down_weight_sum_abs,
        "supported_clipped_activation_count": measurements.supported_clipped_activation_count,
        "learned_activation_range_unsupported": measurements.learned_activation_range_unsupported,
        "projection_biases_unsupported": measurements.projection_biases_unsupported,
        "glu_construction_rejected": measurements.glu_construction_rejected,
        "replay_byte_identical": replay_byte_identical,
        "smoke_self_hash": Hash256::ZERO,
    })
}

fn smoke_self_hash(report: &Value) -> Result<Hash256, S7BurnGradSmokeError> {
    let mut payload = report.clone();
    payload
        .as_object_mut()
        .expect("burn grad smoke report is a JSON object")
        .remove("smoke_self_hash");
    let canonical = CanonicalJson::value_to_vec(&payload)?;
    Ok(S7_BURN_GRAD_SMOKE_DOMAIN.hash_canonical_bytes(&canonical)?)
}

fn stable_canonical_value(value: &Value) -> Result<Value, S7BurnGradSmokeError> {
    let canonical = CanonicalJson::value_to_vec(value)?;
    serde_json::from_slice(&canonical)
        .map_err(CanonicalJsonError::Json)
        .map_err(Into::into)
}

fn fixture_expert(
    clipped_activation: ClippedActivation,
) -> Result<ExpertQat, S7BurnGradSmokeError> {
    Ok(ExpertQat::new_with_clipped_activation(
        ternary_linear(3, 2, &UP_WEIGHTS, None)?,
        clipped_activation,
        activation()?,
        ternary_linear(2, 3, &DOWN_WEIGHTS, None)?,
    )?)
}

fn activation() -> Result<ActFakeQuant, S7BurnGradSmokeError> {
    Ok(ActFakeQuant::new(
        ActivationRangeMode::Fixed(activation_range()?),
        ActivationQuantFormat::Int8,
    )?)
}

fn activation_range() -> Result<ActivationRange, S7BurnGradSmokeError> {
    Ok(ActivationRange::new(-1.0, 1.0)?)
}

fn ternary_linear(
    output_rows: usize,
    input_cols: usize,
    weights: &[f32],
    bias: Option<Vec<f32>>,
) -> Result<TernaryLinearQat, S7BurnGradSmokeError> {
    Ok(TernaryLinearQat::new(
        MatrixShape::new(output_rows, input_cols)?,
        weights.to_vec(),
        bias,
        vec![TernaryThreshold::new(0.5)?; output_rows],
        vec![Q8_8Scale::from_f32(1.0)?; output_rows],
    )?)
}

fn finite_positive_sum_abs(
    projection: &'static str,
    activation: &'static str,
    values: &[f32],
) -> Result<f64, S7BurnGradSmokeError> {
    let mut sum = 0.0;
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(S7BurnGradSmokeError::NonFiniteGradient {
                projection,
                activation,
                index,
            });
        }
        sum += f64::from(value.abs());
    }
    if sum <= 0.0 {
        return Err(S7BurnGradSmokeError::NonPositiveGradient {
            projection,
            activation,
            sum,
        });
    }
    Ok(sum)
}

fn clipped_activation_name(activation: ClippedActivation) -> &'static str {
    if activation == ClippedActivation::relu() {
        "relu"
    } else if activation == ClippedActivation::gelu_clip() {
        "gelu_clip"
    } else {
        "silu_clip"
    }
}

fn resolve_under_root(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Errors emitted while producing the H8 Burn gradient smoke artifact.
#[derive(Debug)]
pub enum S7BurnGradSmokeError {
    /// Filesystem operation failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source I/O error.
        source: io::Error,
    },
    /// Canonical JSON or domain hashing failed.
    CanonicalJson(CanonicalJsonError),
    /// Burn tensor adapter failed.
    BurnAdapter(BurnAdapterError),
    /// Scalar/model fixture construction failed.
    Model(ExpertBlockQatError),
    /// Burn ExpertBlockQat adapter failed.
    BurnExpert(ExpertBlockBurnQatError),
    /// Expected a parameter gradient but Burn did not produce one.
    MissingGradient {
        /// Projection name.
        projection: &'static str,
        /// Clipped activation name.
        activation: &'static str,
    },
    /// Gradient tensor contained a non-finite value.
    NonFiniteGradient {
        /// Projection name.
        projection: &'static str,
        /// Clipped activation name.
        activation: &'static str,
        /// Tensor element index.
        index: usize,
    },
    /// Gradient tensor was finite but all-zero.
    NonPositiveGradient {
        /// Projection name.
        projection: &'static str,
        /// Clipped activation name.
        activation: &'static str,
        /// Observed sum of absolute values.
        sum: f64,
    },
    /// Replaying the deterministic fixture produced different measurements.
    ReplayMismatch,
}

impl fmt::Display for S7BurnGradSmokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::BurnAdapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::BurnExpert(error) => write!(f, "{error}"),
            Self::MissingGradient {
                projection,
                activation,
            } => write!(f, "{projection} missing gradient for {activation}"),
            Self::NonFiniteGradient {
                projection,
                activation,
                index,
            } => write!(
                f,
                "{projection} gradient for {activation} contains non-finite value at {index}"
            ),
            Self::NonPositiveGradient {
                projection,
                activation,
                sum,
            } => write!(
                f,
                "{projection} gradient for {activation} must have positive sum_abs, observed {sum}"
            ),
            Self::ReplayMismatch => {
                f.write_str("S7 burn-grad-smoke fixture replay was not byte-identical")
            }
        }
    }
}

impl Error for S7BurnGradSmokeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CanonicalJson(error) => Some(error),
            Self::BurnAdapter(error) => Some(error),
            Self::Model(error) => Some(error),
            Self::BurnExpert(error) => Some(error),
            Self::MissingGradient { .. }
            | Self::NonFiniteGradient { .. }
            | Self::NonPositiveGradient { .. }
            | Self::ReplayMismatch => None,
        }
    }
}

impl From<CanonicalJsonError> for S7BurnGradSmokeError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}

impl From<BurnAdapterError> for S7BurnGradSmokeError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

impl From<ExpertBlockQatError> for S7BurnGradSmokeError {
    fn from(error: ExpertBlockQatError) -> Self {
        Self::Model(error)
    }
}

impl From<TernaryLinearQatError> for S7BurnGradSmokeError {
    fn from(error: TernaryLinearQatError) -> Self {
        Self::Model(error.into())
    }
}

impl From<ActFakeQuantError> for S7BurnGradSmokeError {
    fn from(error: ActFakeQuantError) -> Self {
        Self::Model(error.into())
    }
}

impl From<ExpertBlockBurnQatError> for S7BurnGradSmokeError {
    fn from(error: ExpertBlockBurnQatError) -> Self {
        Self::BurnExpert(error)
    }
}
