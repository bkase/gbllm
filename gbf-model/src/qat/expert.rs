//! Backend-independent expert block QAT core.

use std::error::Error;
use std::fmt;

use crate::qat::{
    ActFakeQuant, ActFakeQuantError, ActivationForwardMode, MatrixShape, TernaryLinearQat,
    TernaryLinearQatError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertQatForwardMode {
    FullPrecision,
    HardQuantized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertForwardOptions {
    expert_qat: ExpertQatForwardMode,
    activation: ActivationForwardMode,
}

impl ExpertForwardOptions {
    pub fn hard_quantized_train() -> Self {
        Self {
            expert_qat: ExpertQatForwardMode::HardQuantized,
            activation: ActivationForwardMode::Train,
        }
    }

    pub fn full_precision_train() -> Self {
        Self {
            expert_qat: ExpertQatForwardMode::FullPrecision,
            activation: ActivationForwardMode::Train,
        }
    }

    pub fn expert_qat(self) -> ExpertQatForwardMode {
        self.expert_qat
    }

    pub fn activation(self) -> ActivationForwardMode {
        self.activation
    }

    pub fn with_expert_qat(mut self, expert_qat: ExpertQatForwardMode) -> Self {
        self.expert_qat = expert_qat;
        self
    }

    pub fn with_activation(mut self, activation: ActivationForwardMode) -> Self {
        self.activation = activation;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpertBlockQat {
    experts: Vec<ExpertQat>,
    shared_dense: Option<SharedDenseBranch>,
}

impl ExpertBlockQat {
    pub fn new(
        experts: Vec<ExpertQat>,
        shared_dense: Option<SharedDenseBranch>,
    ) -> Result<Self, ExpertBlockQatError> {
        if experts.is_empty() {
            return Err(ExpertBlockQatError::EmptyExpertSet);
        }

        let d_model = experts[0].d_model();
        for (expert_id, expert) in experts.iter().enumerate() {
            if expert.d_model() != d_model {
                return Err(ExpertBlockQatError::ExpertModelDimMismatch {
                    expert_id,
                    expected: d_model,
                    actual: expert.d_model(),
                });
            }
        }

        if let Some(shared_dense) = &shared_dense {
            shared_dense.validate_d_model(d_model)?;
        }

        Ok(Self {
            experts,
            shared_dense,
        })
    }

    pub fn experts(&self) -> &[ExpertQat] {
        &self.experts
    }

    pub fn shared_dense(&self) -> Option<&SharedDenseBranch> {
        self.shared_dense.as_ref()
    }

    pub fn d_model(&self) -> usize {
        self.experts[0].d_model()
    }

    pub fn forward(
        &self,
        input: &[f32],
        expert_id: usize,
    ) -> Result<Vec<f32>, ExpertBlockQatError> {
        self.forward_with_options(
            input,
            expert_id,
            ExpertForwardOptions::hard_quantized_train(),
        )
    }

    pub fn forward_with_options(
        &self,
        input: &[f32],
        expert_id: usize,
        options: ExpertForwardOptions,
    ) -> Result<Vec<f32>, ExpertBlockQatError> {
        validate_input_len(input, self.d_model())?;
        validate_finite("expert input", input)?;

        let expert =
            self.experts
                .get(expert_id)
                .ok_or(ExpertBlockQatError::ExpertIdOutOfRange {
                    expert_id,
                    n_experts: self.experts.len(),
                })?;
        let expert_delta = expert.forward_delta(input, options)?;
        let shared_delta = self
            .shared_dense
            .as_ref()
            .map(|shared_dense| shared_dense.forward(input, options.activation()))
            .transpose()?;

        Ok(input
            .iter()
            .copied()
            .zip(expert_delta)
            .enumerate()
            .map(|(index, (residual, expert_value))| {
                let shared_value = shared_delta.as_ref().map_or(0.0, |values| values[index]);
                residual + expert_value + shared_value
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpertQat {
    up_projection: TernaryLinearQat,
    activation: ActFakeQuant,
    down_projection: TernaryLinearQat,
}

impl ExpertQat {
    pub fn new(
        up_projection: TernaryLinearQat,
        activation: ActFakeQuant,
        down_projection: TernaryLinearQat,
    ) -> Result<Self, ExpertBlockQatError> {
        let up_shape = up_projection.shape();
        let down_shape = down_projection.shape();

        if up_shape.output_rows() != down_shape.input_cols() {
            return Err(ExpertBlockQatError::HiddenDimMismatch {
                up_output_rows: up_shape.output_rows(),
                down_input_cols: down_shape.input_cols(),
            });
        }

        if down_shape.output_rows() != up_shape.input_cols() {
            return Err(ExpertBlockQatError::ResidualDimMismatch {
                input_dim: up_shape.input_cols(),
                output_dim: down_shape.output_rows(),
            });
        }

        Ok(Self {
            up_projection,
            activation,
            down_projection,
        })
    }

    pub fn up_projection(&self) -> &TernaryLinearQat {
        &self.up_projection
    }

    pub fn activation(&self) -> &ActFakeQuant {
        &self.activation
    }

    pub fn down_projection(&self) -> &TernaryLinearQat {
        &self.down_projection
    }

    pub fn d_model(&self) -> usize {
        self.up_projection.shape().input_cols()
    }

    pub fn d_ff(&self) -> usize {
        self.up_projection.shape().output_rows()
    }

    fn forward_delta(
        &self,
        input: &[f32],
        options: ExpertForwardOptions,
    ) -> Result<Vec<f32>, ExpertBlockQatError> {
        let hidden = match options.expert_qat() {
            ExpertQatForwardMode::FullPrecision => {
                full_precision_linear(&self.up_projection, input)
            }
            ExpertQatForwardMode::HardQuantized => self.up_projection.inference_forward(input)?,
        };
        let activated = self
            .activation
            .inference_forward(&hidden, options.activation())?;
        let output = match options.expert_qat() {
            ExpertQatForwardMode::FullPrecision => {
                full_precision_linear(&self.down_projection, &activated)
            }
            ExpertQatForwardMode::HardQuantized => {
                self.down_projection.inference_forward(&activated)?
            }
        };

        Ok(output)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SharedDenseBranch {
    up_projection: DenseBranchProjection,
    activation: ActFakeQuant,
    down_projection: DenseBranchProjection,
    alpha: f32,
}

impl SharedDenseBranch {
    pub fn new(
        up_projection: DenseBranchProjection,
        activation: ActFakeQuant,
        down_projection: DenseBranchProjection,
        alpha: f32,
    ) -> Result<Self, ExpertBlockQatError> {
        if up_projection.shape().output_rows() != down_projection.shape().input_cols() {
            return Err(ExpertBlockQatError::SharedHiddenDimMismatch {
                up_output_rows: up_projection.shape().output_rows(),
                down_input_cols: down_projection.shape().input_cols(),
            });
        }

        if down_projection.shape().output_rows() != up_projection.shape().input_cols() {
            return Err(ExpertBlockQatError::SharedResidualDimMismatch {
                input_dim: up_projection.shape().input_cols(),
                output_dim: down_projection.shape().output_rows(),
            });
        }

        up_projection.validate("shared up")?;
        down_projection.validate("shared down")?;

        if !alpha.is_finite() {
            return Err(ExpertBlockQatError::InvalidSharedAlpha(alpha));
        }

        Ok(Self {
            up_projection,
            activation,
            down_projection,
            alpha,
        })
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn up_shape(&self) -> MatrixShape {
        self.up_projection.shape()
    }

    pub fn down_shape(&self) -> MatrixShape {
        self.down_projection.shape()
    }

    pub fn up_projection(&self) -> &DenseBranchProjection {
        &self.up_projection
    }

    pub fn activation(&self) -> &ActFakeQuant {
        &self.activation
    }

    pub fn down_projection(&self) -> &DenseBranchProjection {
        &self.down_projection
    }

    pub fn validate_d_model(&self, expected: usize) -> Result<(), ExpertBlockQatError> {
        if self.up_projection.shape().input_cols() != expected {
            return Err(ExpertBlockQatError::SharedModelDimMismatch {
                expected,
                actual: self.up_projection.shape().input_cols(),
            });
        }

        Ok(())
    }

    fn forward(
        &self,
        input: &[f32],
        activation_mode: ActivationForwardMode,
    ) -> Result<Vec<f32>, ExpertBlockQatError> {
        let hidden = dense_linear(
            self.up_projection.shape(),
            self.up_projection.weights(),
            self.up_projection.bias(),
            input,
        )?;
        let activated = self
            .activation
            .inference_forward(&hidden, activation_mode)?;
        let output = dense_linear(
            self.down_projection.shape(),
            self.down_projection.weights(),
            self.down_projection.bias(),
            &activated,
        )?;

        Ok(output.into_iter().map(|value| value * self.alpha).collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DenseBranchProjection {
    shape: MatrixShape,
    weights: Vec<f32>,
    bias: Option<Vec<f32>>,
}

impl DenseBranchProjection {
    pub fn new(
        shape: MatrixShape,
        weights: Vec<f32>,
        bias: Option<Vec<f32>>,
    ) -> Result<Self, ExpertBlockQatError> {
        let projection = Self {
            shape,
            weights,
            bias,
        };
        projection.validate("dense projection")?;
        Ok(projection)
    }

    pub fn shape(&self) -> MatrixShape {
        self.shape
    }

    pub fn weights(&self) -> &[f32] {
        &self.weights
    }

    pub fn bias(&self) -> Option<&[f32]> {
        self.bias.as_deref()
    }

    fn validate(&self, name: &'static str) -> Result<(), ExpertBlockQatError> {
        validate_dense_state(name, self.shape, &self.weights, self.bias.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExpertBlockQatError {
    EmptyExpertSet,
    ExpertModelDimMismatch {
        expert_id: usize,
        expected: usize,
        actual: usize,
    },
    ExpertIdOutOfRange {
        expert_id: usize,
        n_experts: usize,
    },
    HiddenDimMismatch {
        up_output_rows: usize,
        down_input_cols: usize,
    },
    ResidualDimMismatch {
        input_dim: usize,
        output_dim: usize,
    },
    SharedHiddenDimMismatch {
        up_output_rows: usize,
        down_input_cols: usize,
    },
    SharedResidualDimMismatch {
        input_dim: usize,
        output_dim: usize,
    },
    SharedModelDimMismatch {
        expected: usize,
        actual: usize,
    },
    DenseWeightLenMismatch {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    DenseBiasLenMismatch {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteDenseWeight {
        name: &'static str,
        index: usize,
    },
    NonFiniteDenseBias {
        name: &'static str,
        index: usize,
    },
    InvalidSharedAlpha(f32),
    InputLenMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        name: &'static str,
        index: usize,
    },
    Ternary(TernaryLinearQatError),
    Activation(ActFakeQuantError),
}

impl fmt::Display for ExpertBlockQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyExpertSet => f.write_str("expert block must contain at least one expert"),
            Self::ExpertModelDimMismatch {
                expert_id,
                expected,
                actual,
            } => write!(
                f,
                "expert {expert_id} d_model mismatch: expected {expected}, got {actual}"
            ),
            Self::ExpertIdOutOfRange {
                expert_id,
                n_experts,
            } => write!(
                f,
                "expert id {expert_id} is out of range for {n_experts} experts"
            ),
            Self::HiddenDimMismatch {
                up_output_rows,
                down_input_cols,
            } => write!(
                f,
                "expert hidden dimension mismatch: up outputs {up_output_rows}, down inputs {down_input_cols}"
            ),
            Self::ResidualDimMismatch {
                input_dim,
                output_dim,
            } => write!(
                f,
                "expert residual dimension mismatch: input {input_dim}, output {output_dim}"
            ),
            Self::SharedHiddenDimMismatch {
                up_output_rows,
                down_input_cols,
            } => write!(
                f,
                "shared dense hidden dimension mismatch: up outputs {up_output_rows}, down inputs {down_input_cols}"
            ),
            Self::SharedResidualDimMismatch {
                input_dim,
                output_dim,
            } => write!(
                f,
                "shared dense residual dimension mismatch: input {input_dim}, output {output_dim}"
            ),
            Self::SharedModelDimMismatch { expected, actual } => write!(
                f,
                "shared dense d_model mismatch: expected {expected}, got {actual}"
            ),
            Self::DenseWeightLenMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} dense weight length mismatch: expected {expected}, got {actual}"
            ),
            Self::DenseBiasLenMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} dense bias length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteDenseWeight { name, index } => {
                write!(f, "{name} dense weight at index {index} is not finite")
            }
            Self::NonFiniteDenseBias { name, index } => {
                write!(f, "{name} dense bias at index {index} is not finite")
            }
            Self::InvalidSharedAlpha(alpha) => {
                write!(f, "shared dense alpha must be finite, got {alpha}")
            }
            Self::InputLenMismatch { expected, actual } => write!(
                f,
                "expert block input length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteInput { name, index } => {
                write!(f, "{name} value at index {index} is not finite")
            }
            Self::Ternary(error) => write!(f, "{error}"),
            Self::Activation(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ExpertBlockQatError {}

impl From<TernaryLinearQatError> for ExpertBlockQatError {
    fn from(error: TernaryLinearQatError) -> Self {
        Self::Ternary(error)
    }
}

impl From<ActFakeQuantError> for ExpertBlockQatError {
    fn from(error: ActFakeQuantError) -> Self {
        Self::Activation(error)
    }
}

fn full_precision_linear(layer: &TernaryLinearQat, input: &[f32]) -> Vec<f32> {
    let shape = layer.shape();
    layer
        .full_precision_weights()
        .chunks_exact(shape.input_cols())
        .enumerate()
        .map(|(row_index, row)| {
            let weighted_sum = row
                .iter()
                .zip(input)
                .map(|(&weight, &value)| weight * value)
                .sum::<f32>();
            weighted_sum + layer.bias().map_or(0.0, |bias| bias[row_index])
        })
        .collect()
}

fn dense_linear(
    shape: MatrixShape,
    weights: &[f32],
    bias: Option<&[f32]>,
    input: &[f32],
) -> Result<Vec<f32>, ExpertBlockQatError> {
    if input.len() != shape.input_cols() {
        return Err(ExpertBlockQatError::InputLenMismatch {
            expected: shape.input_cols(),
            actual: input.len(),
        });
    }

    validate_finite("dense input", input)?;

    Ok(weights
        .chunks_exact(shape.input_cols())
        .enumerate()
        .map(|(row_index, row)| {
            let weighted_sum = row
                .iter()
                .zip(input)
                .map(|(&weight, &value)| weight * value)
                .sum::<f32>();
            weighted_sum + bias.map_or(0.0, |bias| bias[row_index])
        })
        .collect())
}

fn validate_dense_state(
    name: &'static str,
    shape: MatrixShape,
    weights: &[f32],
    bias: Option<&[f32]>,
) -> Result<(), ExpertBlockQatError> {
    if weights.len() != shape.weight_len() {
        return Err(ExpertBlockQatError::DenseWeightLenMismatch {
            name,
            expected: shape.weight_len(),
            actual: weights.len(),
        });
    }

    if let Some(index) = weights.iter().position(|value| !value.is_finite()) {
        return Err(ExpertBlockQatError::NonFiniteDenseWeight { name, index });
    }

    let Some(bias) = bias else {
        return Ok(());
    };

    if bias.len() != shape.output_rows() {
        return Err(ExpertBlockQatError::DenseBiasLenMismatch {
            name,
            expected: shape.output_rows(),
            actual: bias.len(),
        });
    }

    if let Some(index) = bias.iter().position(|value| !value.is_finite()) {
        return Err(ExpertBlockQatError::NonFiniteDenseBias { name, index });
    }

    Ok(())
}

fn validate_input_len(input: &[f32], expected: usize) -> Result<(), ExpertBlockQatError> {
    if input.len() != expected {
        return Err(ExpertBlockQatError::InputLenMismatch {
            expected,
            actual: input.len(),
        });
    }

    Ok(())
}

fn validate_finite(name: &'static str, values: &[f32]) -> Result<(), ExpertBlockQatError> {
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(ExpertBlockQatError::NonFiniteInput { name, index });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::qat::{
        ActivationQuantFormat, ActivationRange, ActivationRangeMode, Q8_8Scale, TernaryThreshold,
    };

    use super::*;

    #[test]
    fn qat_expert_composes_up_activation_down_and_residual() {
        let block = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        let input = vec![1.0, 2.0];

        let output = block.forward(&input, 0).unwrap();

        assert_eq!(output, vec![2.0, 1.0]);
    }

    #[test]
    fn qat_expert_activation_boundary_is_between_up_and_down_projection() {
        let block = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        let input = vec![2.0, -2.0];

        let output = block.forward(&input, 0).unwrap();

        assert_eq!(output, vec![3.0, -1.0]);
    }

    #[test]
    fn qat_expert_supports_full_precision_phase_control() {
        let block = ExpertBlockQat::new(vec![phase_control_expert()], None).unwrap();
        let input = vec![1.0, 2.0];

        let hard = block
            .forward_with_options(
                &input,
                0,
                ExpertForwardOptions::hard_quantized_train()
                    .with_activation(ActivationForwardMode::Eval),
            )
            .unwrap();
        let full_precision = block
            .forward_with_options(
                &input,
                0,
                ExpertForwardOptions::full_precision_train()
                    .with_activation(ActivationForwardMode::Eval),
            )
            .unwrap();

        assert_eq!(hard, vec![2.0, 4.0]);
        assert_close(&full_precision, &[1.36, 2.72], 1.0e-6);
    }

    #[test]
    fn qat_expert_optional_shared_dense_branch_is_explicit_and_alpha_gated() {
        let shared = fixture_shared_dense(0.5);
        let block = ExpertBlockQat::new(vec![fixture_expert()], Some(shared)).unwrap();
        let input = vec![1.0, 2.0];

        let output = block.forward(&input, 0).unwrap();

        assert_eq!(output, vec![2.5, 2.0]);
        assert_eq!(block.shared_dense().unwrap().alpha(), 0.5);
    }

    #[test]
    fn qat_expert_shared_dense_branch_can_initialize_to_expert_only() {
        let shared = fixture_shared_dense(0.0);
        let block = ExpertBlockQat::new(vec![fixture_expert()], Some(shared)).unwrap();
        let input = vec![1.0, 2.0];

        let output = block.forward(&input, 0).unwrap();

        assert_eq!(output, vec![2.0, 1.0]);
    }

    #[test]
    fn qat_expert_shared_dense_branch_honors_activation_phase_mode() {
        let shared = SharedDenseBranch::new(
            DenseBranchProjection::new(MatrixShape::new(1, 2).unwrap(), vec![2.0, 2.0], None)
                .unwrap(),
            activation().with_eval_passthrough(true),
            DenseBranchProjection::new(MatrixShape::new(2, 1).unwrap(), vec![1.0, 1.0], None)
                .unwrap(),
            1.0,
        )
        .unwrap();
        let block = ExpertBlockQat::new(vec![fixture_expert()], Some(shared)).unwrap();
        let input = vec![1.0, 1.0];

        let train = block.forward(&input, 0).unwrap();
        let eval = block
            .forward_with_options(
                &input,
                0,
                ExpertForwardOptions::hard_quantized_train()
                    .with_activation(ActivationForwardMode::Eval),
            )
            .unwrap();

        assert_eq!(train, vec![3.0, 1.0]);
        assert_eq!(eval, vec![6.0, 4.0]);
    }

    #[test]
    fn qat_expert_rejects_invalid_contracts() {
        assert_eq!(
            ExpertBlockQat::new(vec![], None),
            Err(ExpertBlockQatError::EmptyExpertSet)
        );

        let err = ExpertQat::new(
            ternary_linear(2, 2, vec![1.0, 0.0, 0.0, 1.0], None),
            activation(),
            ternary_linear(2, 3, vec![1.0; 6], None),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ExpertBlockQatError::HiddenDimMismatch {
                up_output_rows: 2,
                down_input_cols: 3
            }
        );

        let block = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        assert_eq!(
            block.forward(&[1.0, 2.0], 2),
            Err(ExpertBlockQatError::ExpertIdOutOfRange {
                expert_id: 2,
                n_experts: 1
            })
        );
        assert_eq!(
            block.forward(&[1.0], 0),
            Err(ExpertBlockQatError::InputLenMismatch {
                expected: 2,
                actual: 1
            })
        );
    }

    fn fixture_expert() -> ExpertQat {
        ExpertQat::new(
            ternary_linear(
                3,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0, //
                    0.25, 0.25,
                ],
                Some(vec![0.0, 0.0, 0.0]),
            ),
            activation(),
            ternary_linear(
                2,
                3,
                vec![
                    1.0, 0.0, 0.0, //
                    0.0, -1.0, 0.0,
                ],
                Some(vec![0.0, 0.0]),
            ),
        )
        .unwrap()
    }

    fn fixture_shared_dense(alpha: f32) -> SharedDenseBranch {
        SharedDenseBranch::new(
            DenseBranchProjection::new(MatrixShape::new(1, 2).unwrap(), vec![1.0, 1.0], None)
                .unwrap(),
            activation(),
            DenseBranchProjection::new(MatrixShape::new(2, 1).unwrap(), vec![1.0, 2.0], None)
                .unwrap(),
            alpha,
        )
        .unwrap()
    }

    fn phase_control_expert() -> ExpertQat {
        ExpertQat::new(
            ternary_linear(
                2,
                2,
                vec![
                    0.6, 0.0, //
                    0.0, 0.6,
                ],
                None,
            ),
            passthrough_activation(),
            ternary_linear(
                2,
                2,
                vec![
                    0.6, 0.0, //
                    0.0, 0.6,
                ],
                None,
            ),
        )
        .unwrap()
    }

    fn activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
    }

    fn passthrough_activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-10.0, 10.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
        .with_eval_passthrough(true)
    }

    fn ternary_linear(
        output_rows: usize,
        input_cols: usize,
        weights: Vec<f32>,
        bias: Option<Vec<f32>>,
    ) -> TernaryLinearQat {
        TernaryLinearQat::new(
            MatrixShape::new(output_rows, input_cols).unwrap(),
            weights,
            bias,
            vec![TernaryThreshold::new(0.5).unwrap(); output_rows],
            vec![Q8_8Scale::from_f32(1.0).unwrap(); output_rows],
        )
        .unwrap()
    }

    fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual - expected).abs() <= tolerance,
                "{actual} != {expected} within {tolerance}"
            );
        }
    }
}
