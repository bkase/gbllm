//! Backend-independent ternary linear QAT core.

use std::error::Error;
use std::fmt;

use gbf_artifact::quant::QuantSpec;
use gbf_artifact::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};

const Q8_8_SCALE: f32 = 256.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixShape {
    output_rows: usize,
    input_cols: usize,
    weight_len: usize,
}

impl MatrixShape {
    pub fn new(output_rows: usize, input_cols: usize) -> Result<Self, TernaryLinearQatError> {
        if output_rows == 0 || input_cols == 0 {
            return Err(TernaryLinearQatError::EmptyShape {
                output_rows,
                input_cols,
            });
        }

        let weight_len = output_rows.checked_mul(input_cols).ok_or(
            TernaryLinearQatError::ShapeElementOverflow {
                output_rows,
                input_cols,
            },
        )?;

        Ok(Self {
            output_rows,
            input_cols,
            weight_len,
        })
    }

    pub fn output_rows(self) -> usize {
        self.output_rows
    }

    pub fn input_cols(self) -> usize {
        self.input_cols
    }

    pub fn weight_len(self) -> usize {
        self.weight_len
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q8_8Scale {
    raw: u16,
}

impl Q8_8Scale {
    pub const ZERO: Self = Self { raw: 0 };
    pub const MAX: Self = Self { raw: u16::MAX };
    pub const QUANTIZATION_SCALE: f32 = Q8_8_SCALE;

    pub fn from_raw(raw: u16) -> Self {
        Self { raw }
    }

    /// Strict conversion for explicit artifact-scale values.
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

    /// Training/export fake-quant conversion: clamp to representable Q8_8.
    pub fn from_f32_clamped(value: f32) -> Result<Self, TernaryLinearQatError> {
        if !value.is_finite() {
            return Err(TernaryLinearQatError::InvalidScale(value));
        }

        Self::from_f32(value.clamp(Self::ZERO.to_f32(), Self::MAX.to_f32()))
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

    pub fn from_f32_clamped_q8_8(value: f32) -> Result<Self, TernaryLinearQatError> {
        let quantized = Q8_8Scale::from_f32_clamped(value)?;
        Self::new(quantized.to_f32())
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

/// Pre-export ternary projection data.
///
/// This is not the final artifact tensor container; `ExportVisitor` owns
/// conversion into artifact canonical tensors once that boundary exists.
#[derive(Debug, Clone, PartialEq)]
pub struct TernaryLinearExport {
    plan: TernaryWeightPlan,
    shape: MatrixShape,
    ternary_weights: Vec<TernaryValue>,
    scales: Vec<Q8_8Scale>,
    bias: Option<Vec<f32>>,
}

impl TernaryLinearExport {
    pub fn plan(&self) -> TernaryWeightPlan {
        self.plan
    }

    pub fn shape(&self) -> MatrixShape {
        self.shape
    }

    pub fn ternary_values(&self) -> &[TernaryValue] {
        &self.ternary_weights
    }

    pub fn scales(&self) -> &[Q8_8Scale] {
        &self.scales
    }

    pub fn bias_values(&self) -> Option<&[f32]> {
        self.bias.as_deref()
    }

    pub fn projected_weights(&self) -> Vec<f32> {
        self.ternary_values()
            .chunks_exact(self.shape.input_cols())
            .zip(self.scales())
            .flat_map(|(row, &scale)| row.iter().map(move |&value| value.scaled(scale)))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryLinearQat {
    plan: TernaryWeightPlan,
    shape: MatrixShape,
    full_precision_weights: Vec<f32>,
    bias: Option<Vec<f32>>,
    thresholds: Vec<TernaryThreshold>,
    scales: Vec<Q8_8Scale>,
}

impl TernaryLinearQat {
    pub fn canonical_weight_plan() -> TernaryWeightPlan {
        QuantSpec::default_expert_ternary_plan()
    }

    pub fn new(
        shape: MatrixShape,
        full_precision_weights: Vec<f32>,
        bias: Option<Vec<f32>>,
        thresholds: Vec<TernaryThreshold>,
        scales: Vec<Q8_8Scale>,
    ) -> Result<Self, TernaryLinearQatError> {
        Self::new_with_plan(
            Self::canonical_weight_plan(),
            shape,
            full_precision_weights,
            bias,
            thresholds,
            scales,
        )
    }

    pub fn new_with_plan(
        plan: TernaryWeightPlan,
        shape: MatrixShape,
        full_precision_weights: Vec<f32>,
        bias: Option<Vec<f32>>,
        thresholds: Vec<TernaryThreshold>,
        scales: Vec<Q8_8Scale>,
    ) -> Result<Self, TernaryLinearQatError> {
        validate_weight_plan(plan)?;
        validate_shape_state(
            shape,
            &full_precision_weights,
            bias.as_deref(),
            &thresholds,
            &scales,
            ScaleValidation::RequirePerRow,
        )?;

        Ok(Self {
            plan,
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
        Self::with_plan_and_derived_per_row_scales(
            Self::canonical_weight_plan(),
            shape,
            full_precision_weights,
            bias,
            thresholds,
        )
    }

    pub fn with_plan_and_derived_per_row_scales(
        plan: TernaryWeightPlan,
        shape: MatrixShape,
        full_precision_weights: Vec<f32>,
        bias: Option<Vec<f32>>,
        thresholds: Vec<TernaryThreshold>,
    ) -> Result<Self, TernaryLinearQatError> {
        validate_weight_plan(plan)?;
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

        Self::new_with_plan(
            plan,
            shape,
            full_precision_weights,
            bias,
            thresholds,
            scales,
        )
    }

    pub fn plan(&self) -> TernaryWeightPlan {
        self.plan
    }

    pub fn shape(&self) -> MatrixShape {
        self.shape
    }

    pub fn full_precision_weights(&self) -> &[f32] {
        &self.full_precision_weights
    }

    pub fn bias(&self) -> Option<&[f32]> {
        self.bias.as_deref()
    }

    pub fn thresholds(&self) -> &[TernaryThreshold] {
        &self.thresholds
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
        build_export(
            self.plan,
            self.shape,
            &self.full_precision_weights,
            &self.thresholds,
            &self.scales,
            self.bias.as_deref(),
        )
    }

    pub fn export_canonical_from_trained_state(
        &self,
        full_precision_weights: &[f32],
        thresholds: &[f32],
        scale_factors: &[f32],
        bias: Option<&[f32]>,
    ) -> Result<TernaryLinearExport, TernaryLinearQatError> {
        if self.bias.is_some() != bias.is_some() {
            return Err(TernaryLinearQatError::BiasPresenceMismatch {
                expected: self.bias.is_some(),
                actual: bias.is_some(),
            });
        }

        let scales = scale_factors
            .iter()
            .copied()
            .map(Q8_8Scale::from_f32_clamped)
            .collect::<Result<Vec<_>, _>>()?;
        let thresholds = thresholds
            .iter()
            .copied()
            .map(TernaryThreshold::from_f32_clamped_q8_8)
            .collect::<Result<Vec<_>, _>>()?;

        validate_shape_state(
            self.shape,
            full_precision_weights,
            bias,
            &thresholds,
            &scales,
            ScaleValidation::RequirePerRow,
        )?;

        Ok(build_export(
            self.plan,
            self.shape,
            full_precision_weights,
            &thresholds,
            &scales,
            bias,
        ))
    }

    pub fn projected_weights(&self) -> Vec<f32> {
        self.export_canonical().projected_weights()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TernaryLinearQatError {
    UnsupportedWeightPlan {
        plan: TernaryWeightPlan,
    },
    EmptyShape {
        output_rows: usize,
        input_cols: usize,
    },
    ShapeElementOverflow {
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
    BiasPresenceMismatch {
        expected: bool,
        actual: bool,
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
            Self::UnsupportedWeightPlan { plan } => {
                write!(f, "unsupported ternary weight plan: {plan:?}")
            }
            Self::EmptyShape {
                output_rows,
                input_cols,
            } => write!(
                f,
                "matrix shape must be non-empty, got {output_rows}x{input_cols}"
            ),
            Self::ShapeElementOverflow {
                output_rows,
                input_cols,
            } => write!(
                f,
                "matrix shape {output_rows}x{input_cols} overflows addressable weight length"
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
            Self::BiasPresenceMismatch { expected, actual } => write!(
                f,
                "bias presence mismatch: expected {expected}, got {actual}"
            ),
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

fn validate_weight_plan(plan: TernaryWeightPlan) -> Result<(), TernaryLinearQatError> {
    let supported_threshold = matches!(
        plan.threshold,
        ThresholdPlan::FixedQ8_8 | ThresholdPlan::AnnealedGlobalThenPerOutputRow
    );
    if plan.encoding == WeightEncoding::Ternary2
        && plan.scale_granularity == ScaleGranularity::PerOutputRow
        && plan.scale_format == ScaleFormat::Q8_8
        && supported_threshold
    {
        Ok(())
    } else {
        Err(TernaryLinearQatError::UnsupportedWeightPlan { plan })
    }
}

pub fn project_ternary_values(
    shape: MatrixShape,
    full_precision_weights: &[f32],
    thresholds: &[TernaryThreshold],
) -> Result<Vec<TernaryValue>, TernaryLinearQatError> {
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

    if thresholds.len() != shape.output_rows() {
        return Err(TernaryLinearQatError::ThresholdLenMismatch {
            expected: shape.output_rows(),
            actual: thresholds.len(),
        });
    }

    Ok(full_precision_weights
        .chunks_exact(shape.input_cols())
        .enumerate()
        .flat_map(|(row_index, row)| {
            let threshold = thresholds[row_index];
            row.iter()
                .copied()
                .map(move |weight| TernaryValue::from_weight(weight, threshold))
        })
        .collect())
}

fn build_export(
    plan: TernaryWeightPlan,
    shape: MatrixShape,
    full_precision_weights: &[f32],
    thresholds: &[TernaryThreshold],
    scales: &[Q8_8Scale],
    bias: Option<&[f32]>,
) -> TernaryLinearExport {
    let ternary_values = project_ternary_values(shape, full_precision_weights, thresholds)
        .expect("validated ternary export state");

    TernaryLinearExport {
        plan,
        shape,
        ternary_weights: ternary_values,
        scales: scales.to_vec(),
        bias: bias.map(ToOwned::to_owned),
    }
}

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
    use gbf_foundation::ByteCost;

    use super::*;

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
        let inference_output = layer.inference_forward(&input).unwrap();
        let export = layer.export_canonical();

        assert_eq!(inference_output, vec![0.75]);
        assert_eq!(export.plan(), TernaryLinearQat::canonical_weight_plan());
        assert_eq!(export.shape(), MatrixShape::new(1, 3).unwrap());
        assert_eq!(export.plan().compute_byte_cost(1, 3), ByteCost::new(3));
        assert_eq!(
            export.ternary_values(),
            &[
                TernaryValue::Negative,
                TernaryValue::Zero,
                TernaryValue::Positive,
            ]
        );
        assert_eq!(export.scales(), &[Q8_8Scale::from_f32(0.25).unwrap()]);
        assert_eq!(export.bias_values(), None);
        assert_eq!(export.projected_weights(), vec![-0.25, 0.0, 0.25]);
        assert_eq!(layer.projected_weights(), vec![-0.25, 0.0, 0.25]);
    }

    #[test]
    fn qat_ternary_rejects_invalid_numeric_contracts() {
        assert!(MatrixShape::new(0, 3).is_err());
        assert!(matches!(
            MatrixShape::new(usize::MAX, 2),
            Err(TernaryLinearQatError::ShapeElementOverflow { .. })
        ));
        assert!(TernaryThreshold::new(-0.1).is_err());
        assert!(TernaryThreshold::new(f32::NAN).is_err());
        assert!(Q8_8Scale::from_f32(-0.1).is_err());
        assert!(Q8_8Scale::from_f32(f32::INFINITY).is_err());
    }

    #[test]
    fn qat_ternary_rejects_unsupported_artifact_weight_plans() {
        let plan = TernaryWeightPlan::new(
            WeightEncoding::Binary1,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q4_4,
            ThresholdPlan::FixedQ8_8,
        );

        let err = TernaryLinearQat::new_with_plan(
            plan,
            MatrixShape::new(1, 2).unwrap(),
            vec![1.0, -1.0],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(1.0).unwrap()],
        )
        .unwrap_err();

        assert_eq!(err, TernaryLinearQatError::UnsupportedWeightPlan { plan });

        let annealed = TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::AnnealedGlobalThenPerOutputRow,
        );
        let layer = TernaryLinearQat::new_with_plan(
            annealed,
            MatrixShape::new(1, 2).unwrap(),
            vec![1.0, -1.0],
            None,
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(1.0).unwrap()],
        )
        .unwrap();

        assert_eq!(layer.export_canonical().plan(), annealed);
    }

    #[test]
    fn qat_ternary_default_plan_matches_artifact_expert_default() {
        assert_eq!(
            TernaryLinearQat::canonical_weight_plan(),
            QuantSpec::default_expert_ternary_plan()
        );
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
    fn qat_ternary_export_from_trained_state_uses_supplied_weights_and_scales() {
        let layer = TernaryLinearQat::new(
            MatrixShape::new(1, 2).unwrap(),
            vec![0.1, 0.1],
            Some(vec![0.0]),
            vec![TernaryThreshold::new(0.5).unwrap()],
            vec![Q8_8Scale::from_f32(1.0).unwrap()],
        )
        .unwrap();

        let export = layer
            .export_canonical_from_trained_state(&[0.75, -0.75], &[0.5], &[0.5], Some(&[0.25]))
            .unwrap();

        assert_eq!(
            export.ternary_values(),
            &[TernaryValue::Positive, TernaryValue::Negative]
        );
        assert_eq!(export.scales(), &[Q8_8Scale::from_f32(0.5).unwrap()]);
        assert_eq!(export.bias_values(), Some([0.25].as_slice()));
        assert_eq!(export.projected_weights(), vec![0.5, -0.5]);
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
            export.ternary_values(),
            &[
                TernaryValue::Positive,
                TernaryValue::Negative,
                TernaryValue::Zero,
                TernaryValue::Zero,
                TernaryValue::Zero,
                TernaryValue::Zero,
            ]
        );
        assert_eq!(export.scales()[0], Q8_8Scale::from_f32(2.0).unwrap());
        assert_eq!(export.scales()[1], Q8_8Scale::ZERO);
        assert_eq!(export.bias_values().unwrap(), &[0.5, -0.5]);
        assert_eq!(
            layer.inference_forward(&[1.0, 1.0, 1.0]).unwrap(),
            vec![0.5, -0.5]
        );
    }
}
