//! Burn-specific adapter surface.
//!
//! Keep direct Burn imports contained here. Downstream training modules should
//! depend on these aliases/helpers instead of binding to Burn paths directly.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use burn::module::{
    AutodiffModule as BurnAutodiffModule, Module as BurnModule, Param as BurnParam,
};
pub use burn::nn;
pub use burn::optim::{AdamConfig as BurnAdamConfig, AdamWConfig as BurnAdamWConfig};
pub use burn::record::{
    DefaultFileRecorder as BurnDefaultFileRecorder, FileRecorder as BurnFileRecorder,
    FullPrecisionSettings as BurnFullPrecisionSettings, RecorderError as BurnRecorderError,
};
pub use burn::tensor::{
    Bool as BurnBool, Float as BurnFloat, Int as BurnInt, Shape as BurnShape, Tensor as BurnTensor,
    TensorData as BurnTensorData,
    backend::{
        AutodiffBackend as BurnAutodiffBackend, Backend as BurnBackend,
        ExecutionError as BurnExecutionError,
    },
    module::linear as burn_linear,
};

/// Burn autodiff backend decorator pinned behind this adapter.
pub type BurnAutodiff<B> = burn::backend::Autodiff<B>;

/// CPU backend used for adapter tests and tiny training fixtures.
pub type BurnNdArrayBackend = burn::backend::NdArray;

/// Autodiff-enabled CPU backend used for QAT tests and tiny fixtures.
pub type BurnNdArrayAutodiffBackend = BurnAutodiff<BurnNdArrayBackend>;

/// Default full-precision model checkpoint recorder.
pub type BurnFullPrecisionFileRecorder = BurnDefaultFileRecorder<BurnFullPrecisionSettings>;

/// Convenience alias for Burn float tensors.
pub type BurnFloatTensor<B, const D: usize> = BurnTensor<B, D, BurnFloat>;

/// Convenience alias for Burn int tensors.
pub type BurnIntTensor<B, const D: usize> = BurnTensor<B, D, BurnInt>;

/// Convenience alias for Burn bool tensors.
pub type BurnBoolTensor<B, const D: usize> = BurnTensor<B, D, BurnBool>;

/// Convenience alias for backend devices.
pub type BurnDevice<B> = <B as BurnBackend>::Device;

/// Stable optimizer selection enum for gbf training configs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BurnOptimizerKind {
    Adam,
    AdamW,
}

/// Scalar metric event emitted by the training adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScalarMetric {
    pub name: String,
    pub value: f64,
    pub step: u64,
}

impl ScalarMetric {
    #[must_use]
    pub fn new(name: impl Into<String>, value: f64, step: u64) -> Self {
        Self {
            name: name.into(),
            value,
            step,
        }
    }
}

/// Minimal metric sink interface used by training loops.
pub trait MetricSink {
    fn log_scalar(&self, metric: &ScalarMetric);
}

/// Metric sink backed by `tracing` events.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingMetricSink;

impl MetricSink for TracingMetricSink {
    fn log_scalar(&self, metric: &ScalarMetric) {
        tracing::info!(
            event_name = crate::logging::EVENT_NAME_SCALAR_METRIC,
            metric_name = %metric.name,
            metric_value = metric.value,
            step = metric.step,
        );
    }
}

#[derive(Debug)]
pub enum BurnAdapterError {
    ShapeElementCountMismatch { expected: usize, actual: usize },
    ShapeElementCountOverflow,
    Execution(BurnExecutionError),
}

impl fmt::Display for BurnAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShapeElementCountMismatch { expected, actual } => {
                write!(f, "tensor shape expects {expected} elements, got {actual}")
            }
            Self::ShapeElementCountOverflow => {
                f.write_str("tensor shape element count overflowed usize")
            }
            Self::Execution(error) => write!(f, "{error}"),
        }
    }
}

impl Error for BurnAdapterError {}

impl From<BurnExecutionError> for BurnAdapterError {
    fn from(error: BurnExecutionError) -> Self {
        Self::Execution(error)
    }
}

/// Create a Burn float tensor from a flat `f32` buffer and explicit shape.
pub fn float_tensor_from_vec<B, const D: usize>(
    values: Vec<f32>,
    shape: [usize; D],
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, D>, BurnAdapterError>
where
    B: BurnBackend,
{
    validate_shape_len(values.len(), &shape)?;
    let data = BurnTensorData::new(values, shape);

    Ok(BurnTensor::from_data(data, device))
}

/// Read a Burn float tensor into a host `Vec<f32>`.
pub fn float_tensor_into_vec<B, const D: usize>(
    tensor: BurnFloatTensor<B, D>,
) -> Result<Vec<f32>, BurnAdapterError>
where
    B: BurnBackend,
{
    let data = tensor.try_into_data()?;

    Ok(data.iter::<f32>().collect())
}

/// Return the tensor shape as a fixed-size array.
pub fn float_tensor_shape<B, const D: usize>(tensor: &BurnFloatTensor<B, D>) -> [usize; D]
where
    B: BurnBackend,
{
    let mut shape = [0; D];
    shape.copy_from_slice(tensor.shape().as_slice());
    shape
}

/// Straight-through estimator helper: use `projected` in the forward pass and
/// preserve identity gradients with respect to `input`.
pub fn ste_replace_forward<B, const D: usize>(
    input: BurnFloatTensor<B, D>,
    projected: BurnFloatTensor<B, D>,
) -> BurnFloatTensor<B, D>
where
    B: BurnBackend,
{
    input.clone() + (projected - input).detach()
}

/// Clamp in the forward pass with clipped gradients outside the clamp range.
pub fn ste_clamp<B, const D: usize>(
    input: BurnFloatTensor<B, D>,
    lo: f32,
    hi: f32,
) -> BurnFloatTensor<B, D>
where
    B: BurnBackend,
{
    input.clamp(lo, hi)
}

/// Round in the forward pass while preserving identity gradients.
pub fn ste_round<B, const D: usize>(input: BurnFloatTensor<B, D>) -> BurnFloatTensor<B, D>
where
    B: BurnBackend,
{
    let projected = input.clone().round();
    ste_replace_forward(input, projected)
}

/// Construct Burn's Adam optimizer config through the adapter boundary.
#[must_use]
pub fn adam_config() -> BurnAdamConfig {
    BurnAdamConfig::new()
}

/// Construct Burn's AdamW optimizer config through the adapter boundary.
#[must_use]
pub fn adamw_config() -> BurnAdamWConfig {
    BurnAdamWConfig::new()
}

/// Construct the default full-precision Burn file recorder.
#[must_use]
pub fn full_precision_file_recorder() -> BurnFullPrecisionFileRecorder {
    BurnFullPrecisionFileRecorder::default()
}

/// Save a Burn module through an explicitly supplied file recorder.
pub fn save_module<B, M, FR, PB>(
    module: M,
    file_path: PB,
    recorder: &FR,
) -> Result<(), BurnRecorderError>
where
    B: BurnBackend,
    M: BurnModule<B>,
    FR: BurnFileRecorder<B>,
    PB: Into<PathBuf>,
{
    module.save_file(file_path, recorder)
}

/// Load a Burn module through an explicitly supplied file recorder.
pub fn load_module<B, M, FR, PB>(
    module: M,
    file_path: PB,
    recorder: &FR,
    device: &BurnDevice<B>,
) -> Result<M, BurnRecorderError>
where
    B: BurnBackend,
    M: BurnModule<B>,
    FR: BurnFileRecorder<B>,
    PB: Into<PathBuf>,
{
    module.load_file(file_path, recorder, device)
}

fn validate_shape_len<const D: usize>(
    actual: usize,
    shape: &[usize; D],
) -> Result<(), BurnAdapterError> {
    let expected = shape
        .iter()
        .try_fold(1usize, |acc, dim| acc.checked_mul(*dim))
        .ok_or(BurnAdapterError::ShapeElementCountOverflow)?;

    if expected != actual {
        return Err(BurnAdapterError::ShapeElementCountMismatch { expected, actual });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burn_adapter_round_trips_float_tensor_data() {
        let device = BurnDevice::<BurnNdArrayBackend>::default();
        let tensor = float_tensor_from_vec::<BurnNdArrayBackend, 2>(
            vec![1.0, 2.0, 3.0, 4.0],
            [2, 2],
            &device,
        )
        .unwrap();

        assert_eq!(float_tensor_shape(&tensor), [2, 2]);
        assert_eq!(
            float_tensor_into_vec(tensor).unwrap(),
            vec![1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn burn_adapter_rejects_shape_value_mismatch() {
        let device = BurnDevice::<BurnNdArrayBackend>::default();

        assert!(matches!(
            float_tensor_from_vec::<BurnNdArrayBackend, 2>(vec![1.0, 2.0, 3.0], [2, 2], &device),
            Err(BurnAdapterError::ShapeElementCountMismatch {
                expected: 4,
                actual: 3,
            })
        ));
    }

    #[test]
    fn ste_round_preserves_identity_gradient() {
        let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
        let input =
            float_tensor_from_vec::<BurnNdArrayAutodiffBackend, 1>(vec![0.2, 1.8], [2], &device)
                .unwrap()
                .require_grad();

        let output = ste_round(input.clone()).sum();
        let gradients = output.backward();
        let grad = input.grad(&gradients).expect("gradient should exist");

        assert_eq!(float_tensor_into_vec(grad).unwrap(), vec![1.0, 1.0]);
    }

    #[test]
    fn ste_clamp_zeroes_gradients_outside_bounds() {
        let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
        let input = float_tensor_from_vec::<BurnNdArrayAutodiffBackend, 1>(
            vec![-2.0, -0.25, 0.25, 2.0],
            [4],
            &device,
        )
        .unwrap()
        .require_grad();

        let output = ste_clamp(input.clone(), -1.0, 1.0).sum();
        let gradients = output.backward();
        let grad = input.grad(&gradients).expect("gradient should exist");

        assert_eq!(
            float_tensor_into_vec(grad).unwrap(),
            vec![0.0, 1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn optimizer_and_metric_adapters_are_constructible() {
        let _adam = adam_config();
        let _adamw = adamw_config();
        let _recorder = full_precision_file_recorder();

        TracingMetricSink.log_scalar(&ScalarMetric::new("loss", 0.25, 7));
    }
}
