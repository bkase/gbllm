//! Backend-independent ternary linear QAT core.

use std::error::Error;
use std::fmt;

const Q8_8_SCALE: f32 = 256.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixShape {
    output_rows: usize,
    input_cols: usize,
}

impl MatrixShape {
    pub fn new(output_rows: usize, input_cols: usize) -> Result<Self, TernaryLinearQatError> {
        if output_rows == 0 || input_cols == 0 {
            return Err(TernaryLinearQatError::EmptyShape {
                output_rows,
                input_cols,
            });
        }

        Ok(Self {
            output_rows,
            input_cols,
        })
    }

    pub fn output_rows(self) -> usize {
        self.output_rows
    }

    pub fn input_cols(self) -> usize {
        self.input_cols
    }

    fn weight_len(self) -> usize {
        self.output_rows * self.input_cols
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q8_8Scale {
    raw: u16,
}

impl Q8_8Scale {
    pub const ZERO: Self = Self { raw: 0 };

    pub fn from_raw(raw: u16) -> Self {
        Self { raw }
    }

    pub fn from_f32(value: f32) -> Result<Self, TernaryLinearQatError> {
        if !value.is_finite() || value < 0.0 {
            return Err(TernaryLinearQatError::InvalidScale(value));
        }

        let raw = (value * Q8_8_SCALE).round();
        if raw > u16::MAX as f32 {
            return Err(TernaryLinearQatError::InvalidScale(value));
        }

        Ok(Self { raw: raw as u16 })
    }

    pub fn raw(self) -> u16 {
        self.raw
    }

    pub fn to_f32(self) -> f32 {
        f32::from(self.raw) / Q8_8_SCALE
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TernaryThreshold {
    value: f32,
}

impl TernaryThreshold {
    pub fn new(value: f32) -> Result<Self, TernaryLinearQatError> {
        if !value.is_finite() || value < 0.0 {
            return Err(TernaryLinearQatError::InvalidThreshold(value));
        }

        Ok(Self { value })
    }

    pub fn value(self) -> f32 {
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i8)]
pub enum TernaryValue {
    Negative = -1,
    Zero = 0,
    Positive = 1,
}

impl TernaryValue {
    pub fn from_weight(weight: f32, threshold: TernaryThreshold) -> Self {
        if weight > threshold.value() {
            Self::Positive
        } else if weight < -threshold.value() {
            Self::Negative
        } else {
            Self::Zero
        }
    }

    pub fn as_i8(self) -> i8 {
        self as i8
    }

    pub fn scaled(self, scale: Q8_8Scale) -> f32 {
        f32::from(self.as_i8()) * scale.to_f32()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalTensor<T> {
    shape: Vec<usize>,
    values: Vec<T>,
}

impl<T> CanonicalTensor<T> {
    fn from_parts_unchecked(shape: Vec<usize>, values: Vec<T>) -> Self {
        Self { shape, values }
    }

    pub fn shape(&self) -> &[usize] {
        &self.shape
    }

    pub fn values(&self) -> &[T] {
        &self.values
    }

    pub fn into_values(self) -> Vec<T> {
        self.values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryLinearExport {
    ternary_weights: CanonicalTensor<TernaryValue>,
    scales: CanonicalTensor<Q8_8Scale>,
    bias: Option<CanonicalTensor<f32>>,
}

impl TernaryLinearExport {
    pub fn ternary_weights(&self) -> &CanonicalTensor<TernaryValue> {
        &self.ternary_weights
    }

    pub fn scales(&self) -> &CanonicalTensor<Q8_8Scale> {
        &self.scales
    }

    pub fn bias(&self) -> Option<&CanonicalTensor<f32>> {
        self.bias.as_ref()
    }

    pub fn into_parts(
        self,
    ) -> (
        CanonicalTensor<TernaryValue>,
        CanonicalTensor<Q8_8Scale>,
        Option<CanonicalTensor<f32>>,
    ) {
        (self.ternary_weights, self.scales, self.bias)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TernarySteLinear<'a> {
    shape: MatrixShape,
    thresholds: &'a [TernaryThreshold],
    bias: Option<&'a [f32]>,
}

impl<'a> TernarySteLinear<'a> {
    pub fn shape(self) -> MatrixShape {
        self.shape
    }

    pub fn thresholds(self) -> &'a [TernaryThreshold] {
        self.thresholds
    }

    pub fn bias(self) -> Option<&'a [f32]> {
        self.bias
    }
}

pub trait TernarySteBackend {
    type Tensor;

    /// Implement hard ternary forward values while preserving STE gradients
    /// through backend-owned full-precision weights and scale factors.
    fn ternary_linear_ste(
        &self,
        spec: TernarySteLinear<'_>,
        full_precision_weights: &Self::Tensor,
        scale_factors: &Self::Tensor,
        input: &Self::Tensor,
    ) -> Self::Tensor;
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryLinearQat {
    shape: MatrixShape,
    full_precision_weights: Vec<f32>,
    bias: Option<Vec<f32>>,
    thresholds: Vec<TernaryThreshold>,
    scales: Vec<Q8_8Scale>,
}

impl TernaryLinearQat {
    pub fn new(
        shape: MatrixShape,
        full_precision_weights: Vec<f32>,
        bias: Option<Vec<f32>>,
        thresholds: Vec<TernaryThreshold>,
        scales: Vec<Q8_8Scale>,
    ) -> Result<Self, TernaryLinearQatError> {
        validate_shape_state(
            shape,
            &full_precision_weights,
            bias.as_deref(),
            &thresholds,
            &scales,
            ScaleValidation::RequirePerRow,
        )?;

        Ok(Self {
            shape,
            full_precision_weights,
            bias,
            thresholds,
            scales,
        })
    }

    pub fn with_derived_per_row_scales(
        shape: MatrixShape,
        full_precision_weights: Vec<f32>,
        bias: Option<Vec<f32>>,
        thresholds: Vec<TernaryThreshold>,
    ) -> Result<Self, TernaryLinearQatError> {
        validate_shape_state(
            shape,
            &full_precision_weights,
            bias.as_deref(),
            &thresholds,
            &[],
            ScaleValidation::AllowEmptyForDerivation,
        )?;

        let scales = full_precision_weights
            .chunks_exact(shape.input_cols())
            .zip(thresholds.iter().copied())
            .map(|(row, threshold)| derive_row_scale(row, threshold))
            .collect::<Result<Vec<_>, _>>()?;

        Self::new(shape, full_precision_weights, bias, thresholds, scales)
    }

    pub fn fake_quant_forward<B: TernarySteBackend>(
        &self,
        backend: &B,
        full_precision_weights: &B::Tensor,
        scale_factors: &B::Tensor,
        input: &B::Tensor,
    ) -> B::Tensor {
        let spec = TernarySteLinear {
            shape: self.shape,
            thresholds: &self.thresholds,
            bias: self.bias.as_deref(),
        };

        backend.ternary_linear_ste(spec, full_precision_weights, scale_factors, input)
    }

    pub fn full_precision_weights(&self) -> &[f32] {
        &self.full_precision_weights
    }

    pub fn scales(&self) -> &[Q8_8Scale] {
        &self.scales
    }

    pub fn inference_forward(&self, x: &[f32]) -> Result<Vec<f32>, TernaryLinearQatError> {
        if x.len() != self.shape.input_cols() {
            return Err(TernaryLinearQatError::InputLenMismatch {
                expected: self.shape.input_cols(),
                actual: x.len(),
            });
        }

        if let Some(index) = x.iter().position(|value| !value.is_finite()) {
            return Err(TernaryLinearQatError::NonFiniteInput { index });
        }

        let output = self
            .full_precision_weights
            .chunks_exact(self.shape.input_cols())
            .enumerate()
            .map(|(row_index, row)| {
                let threshold = self.thresholds[row_index];
                let scale = self.scales[row_index];
                let weighted_sum = row
                    .iter()
                    .zip(x.iter())
                    .map(|(&weight, &input)| {
                        TernaryValue::from_weight(weight, threshold).scaled(scale) * input
                    })
                    .sum::<f32>();
                weighted_sum + self.bias.as_ref().map_or(0.0, |bias| bias[row_index])
            })
            .collect();

        Ok(output)
    }

    pub fn export_canonical(&self) -> TernaryLinearExport {
        let ternary_values = self
            .full_precision_weights
            .chunks_exact(self.shape.input_cols())
            .enumerate()
            .flat_map(|(row_index, row)| {
                let threshold = self.thresholds[row_index];
                row.iter()
                    .copied()
                    .map(move |weight| TernaryValue::from_weight(weight, threshold))
            })
            .collect();

        TernaryLinearExport {
            ternary_weights: CanonicalTensor::from_parts_unchecked(
                vec![self.shape.output_rows(), self.shape.input_cols()],
                ternary_values,
            ),
            scales: CanonicalTensor::from_parts_unchecked(
                vec![self.shape.output_rows()],
                self.scales.clone(),
            ),
            bias: self.bias.as_ref().map(|bias| {
                CanonicalTensor::from_parts_unchecked(vec![self.shape.output_rows()], bias.clone())
            }),
        }
    }

    pub fn projected_weights(&self) -> CanonicalTensor<f32> {
        let export = self.export_canonical();
        let projected = export
            .ternary_weights()
            .values()
            .chunks_exact(self.shape.input_cols())
            .zip(export.scales().values())
            .flat_map(|(row, &scale)| row.iter().map(move |&value| value.scaled(scale)))
            .collect();

        CanonicalTensor::from_parts_unchecked(
            vec![self.shape.output_rows(), self.shape.input_cols()],
            projected,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TernaryLinearQatError {
    EmptyShape {
        output_rows: usize,
        input_cols: usize,
    },
    WeightLenMismatch {
        expected: usize,
        actual: usize,
    },
    BiasLenMismatch {
        expected: usize,
        actual: usize,
    },
    ThresholdLenMismatch {
        expected: usize,
        actual: usize,
    },
    ScaleLenMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidThreshold(f32),
    InvalidScale(f32),
    NonFiniteWeight {
        index: usize,
    },
    NonFiniteBias {
        index: usize,
    },
    NonFiniteInput {
        index: usize,
    },
    InputLenMismatch {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for TernaryLinearQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyShape {
                output_rows,
                input_cols,
            } => write!(
                f,
                "matrix shape must be non-empty, got {output_rows}x{input_cols}"
            ),
            Self::WeightLenMismatch { expected, actual } => {
                write!(
                    f,
                    "weight length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::BiasLenMismatch { expected, actual } => {
                write!(f, "bias length mismatch: expected {expected}, got {actual}")
            }
            Self::ThresholdLenMismatch { expected, actual } => write!(
                f,
                "threshold length mismatch: expected {expected}, got {actual}"
            ),
            Self::ScaleLenMismatch { expected, actual } => {
                write!(
                    f,
                    "scale length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::InvalidThreshold(value) => write!(f, "invalid ternary threshold {value}"),
            Self::InvalidScale(value) => write!(f, "invalid Q8_8 scale {value}"),
            Self::NonFiniteWeight { index } => write!(f, "weight at index {index} is not finite"),
            Self::NonFiniteBias { index } => write!(f, "bias at index {index} is not finite"),
            Self::NonFiniteInput { index } => write!(f, "input at index {index} is not finite"),
            Self::InputLenMismatch { expected, actual } => {
                write!(
                    f,
                    "input length mismatch: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl Error for TernaryLinearQatError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScaleValidation {
    RequirePerRow,
    AllowEmptyForDerivation,
}

fn validate_shape_state(
    shape: MatrixShape,
    full_precision_weights: &[f32],
    bias: Option<&[f32]>,
    thresholds: &[TernaryThreshold],
    scales: &[Q8_8Scale],
    scale_validation: ScaleValidation,
) -> Result<(), TernaryLinearQatError> {
    if full_precision_weights.len() != shape.weight_len() {
        return Err(TernaryLinearQatError::WeightLenMismatch {
            expected: shape.weight_len(),
            actual: full_precision_weights.len(),
        });
    }

    if let Some(index) = full_precision_weights
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(TernaryLinearQatError::NonFiniteWeight { index });
    }

    if let Some(bias) = bias {
        if bias.len() != shape.output_rows() {
            return Err(TernaryLinearQatError::BiasLenMismatch {
                expected: shape.output_rows(),
                actual: bias.len(),
            });
        }

        if let Some(index) = bias.iter().position(|value| !value.is_finite()) {
            return Err(TernaryLinearQatError::NonFiniteBias { index });
        }
    }

    if thresholds.len() != shape.output_rows() {
        return Err(TernaryLinearQatError::ThresholdLenMismatch {
            expected: shape.output_rows(),
            actual: thresholds.len(),
        });
    }

    match scale_validation {
        ScaleValidation::RequirePerRow if scales.len() != shape.output_rows() => {
            return Err(TernaryLinearQatError::ScaleLenMismatch {
                expected: shape.output_rows(),
                actual: scales.len(),
            });
        }
        ScaleValidation::AllowEmptyForDerivation
            if !scales.is_empty() && scales.len() != shape.output_rows() =>
        {
            return Err(TernaryLinearQatError::ScaleLenMismatch {
                expected: shape.output_rows(),
                actual: scales.len(),
            });
        }
        _ => {}
    }

    Ok(())
}

fn derive_row_scale(
    row: &[f32],
    threshold: TernaryThreshold,
) -> Result<Q8_8Scale, TernaryLinearQatError> {
    let mut active_count = 0usize;
    let active_abs_sum = row
        .iter()
        .copied()
        .filter(|weight| TernaryValue::from_weight(*weight, threshold) != TernaryValue::Zero)
        .inspect(|_| active_count += 1)
        .map(f32::abs)
        .sum::<f32>();

    if active_count == 0 {
        return Ok(Q8_8Scale::ZERO);
    }

    Q8_8Scale::from_f32(active_abs_sum / active_count as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScalarSteBackend;

    impl TernarySteBackend for ScalarSteBackend {
        type Tensor = Vec<f32>;

        fn ternary_linear_ste(
            &self,
            spec: TernarySteLinear<'_>,
            full_precision_weights: &Self::Tensor,
            scale_factors: &Self::Tensor,
            input: &Self::Tensor,
        ) -> Self::Tensor {
            assert_eq!(
                full_precision_weights.len(),
                spec.shape().output_rows() * spec.shape().input_cols()
            );
            assert_eq!(spec.thresholds().len(), spec.shape().output_rows());
            assert_eq!(scale_factors.len(), spec.shape().output_rows());
            assert_eq!(input.len(), spec.shape().input_cols());

            full_precision_weights
                .chunks_exact(spec.shape().input_cols())
                .enumerate()
                .map(|(row_index, row)| {
                    let threshold = spec.thresholds()[row_index];
                    let scale = scale_factors[row_index];
                    let weighted_sum = row
                        .iter()
                        .zip(input.iter())
                        .map(|(&weight, &input)| {
                            f32::from(TernaryValue::from_weight(weight, threshold).as_i8())
                                * scale
                                * input
                        })
                        .sum::<f32>();
                    weighted_sum + spec.bias().map_or(0.0, |bias| bias[row_index])
                })
                .collect()
        }
    }

    #[test]
    fn qat_ternary_forward_and_export_share_projection() {
        let layer = TernaryLinearQat::new(
            MatrixShape::new(1, 3).unwrap(),
            vec![-2.0, -0.1, 0.6],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(0.25).unwrap()],
        )
        .unwrap();

        let input = vec![1.0, 2.0, 4.0];
        let train_weights = layer.full_precision_weights().to_vec();
        let train_scales = layer
            .scales()
            .iter()
            .map(|scale| scale.to_f32())
            .collect::<Vec<_>>();
        let output =
            layer.fake_quant_forward(&ScalarSteBackend, &train_weights, &train_scales, &input);
        let inference_output = layer.inference_forward(&input).unwrap();
        let export = layer.export_canonical();

        assert_eq!(output, vec![0.75]);
        assert_eq!(output, inference_output);
        assert_eq!(
            export.ternary_weights().values(),
            &[
                TernaryValue::Negative,
                TernaryValue::Zero,
                TernaryValue::Positive,
            ]
        );
        assert_eq!(
            export.scales().values(),
            &[Q8_8Scale::from_f32(0.25).unwrap()]
        );
        assert_eq!(export.bias(), None);
        assert_eq!(layer.projected_weights().values(), &[-0.25, 0.0, 0.25]);
    }

    #[test]
    fn qat_ternary_rejects_invalid_numeric_contracts() {
        assert!(MatrixShape::new(0, 3).is_err());
        assert!(TernaryThreshold::new(-0.1).is_err());
        assert!(TernaryThreshold::new(f32::NAN).is_err());
        assert!(Q8_8Scale::from_f32(-0.1).is_err());
        assert!(Q8_8Scale::from_f32(f32::INFINITY).is_err());
    }

    #[test]
    fn qat_ternary_rejects_empty_explicit_scales() {
        let err = TernaryLinearQat::new(
            MatrixShape::new(1, 2).unwrap(),
            vec![1.0, -1.0],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![],
        )
        .unwrap_err();

        assert_eq!(
            err,
            TernaryLinearQatError::ScaleLenMismatch {
                expected: 1,
                actual: 0
            }
        );
    }

    #[test]
    fn qat_ternary_derives_q8_8_scales_from_active_weights() {
        let layer = TernaryLinearQat::with_derived_per_row_scales(
            MatrixShape::new(2, 3).unwrap(),
            vec![1.0, -3.0, 0.25, 0.1, -0.2, 0.3],
            Some(vec![0.5, -0.5]),
            vec![
                TernaryThreshold::new(0.5).unwrap(),
                TernaryThreshold::new(0.5).unwrap(),
            ],
        )
        .unwrap();

        let export = layer.export_canonical();

        assert_eq!(
            export.ternary_weights().values(),
            &[
                TernaryValue::Positive,
                TernaryValue::Negative,
                TernaryValue::Zero,
                TernaryValue::Zero,
                TernaryValue::Zero,
                TernaryValue::Zero,
            ]
        );
        assert_eq!(
            export.scales().values()[0],
            Q8_8Scale::from_f32(2.0).unwrap()
        );
        assert_eq!(export.scales().values()[1], Q8_8Scale::ZERO);
        assert_eq!(export.bias().unwrap().values(), &[0.5, -0.5]);
        assert_eq!(
            layer.inference_forward(&[1.0, 1.0, 1.0]).unwrap(),
            vec![0.5, -0.5]
        );
    }
}
