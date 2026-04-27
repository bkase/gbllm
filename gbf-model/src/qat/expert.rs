//! Backend-independent expert block QAT core.

use std::error::Error;
use std::fmt;

use gbf_foundation::ByteCost;

use crate::budget::{compute_expert_bytes, compute_glu_expert_bytes_for_diagnostic};
use crate::qat::{
    ActFakeQuant, ActFakeQuantError, ActivationForwardMode, ActivationRange, MatrixShape,
    QatHardnessControl, QuantHardness, TernaryLinearQat, TernaryLinearQatError,
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

    /// Maps phase hardness fields to expert execution switches.
    ///
    /// Callers that want `Soft`/`Hard` projection behavior should also apply
    /// the same phase hardness to the expert block state.
    pub fn for_hardness(expert_qat: QuantHardness, activation_qat: QuantHardness) -> Self {
        let expert_qat = match expert_qat {
            QuantHardness::Off => ExpertQatForwardMode::FullPrecision,
            QuantHardness::Soft | QuantHardness::Hard => ExpertQatForwardMode::HardQuantized,
        };
        let activation = match activation_qat {
            QuantHardness::Off => ActivationForwardMode::Passthrough,
            QuantHardness::Soft | QuantHardness::Hard => ActivationForwardMode::Train,
        };

        Self {
            expert_qat,
            activation,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertMlpVariant {
    TwoMatrix,
    GatedLinearUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertMlpConfig {
    d_model: usize,
    d_ff: usize,
    variant: ExpertMlpVariant,
}

impl ExpertMlpConfig {
    pub fn default_two_matrix(d_model: usize, d_ff: usize) -> Result<Self, ExpertBlockQatError> {
        Self::new(d_model, d_ff, ExpertMlpVariant::TwoMatrix)
    }

    pub fn glu_explicit(
        d_model: usize,
        d_ff: usize,
    ) -> Result<(Self, ExpertMlpConfigEvent), ExpertBlockQatError> {
        let config = Self::new(d_model, d_ff, ExpertMlpVariant::GatedLinearUnit)?;
        Ok((config, ExpertMlpConfigEvent::glu_bank_fit_warning(config)))
    }

    fn new(
        d_model: usize,
        d_ff: usize,
        variant: ExpertMlpVariant,
    ) -> Result<Self, ExpertBlockQatError> {
        validate_nonzero_dimension("d_model", d_model)?;
        validate_nonzero_dimension("d_ff", d_ff)?;
        validate_budget_dimension("d_model", d_model)?;
        validate_budget_dimension("d_ff", d_ff)?;

        let matrix_count = matrix_count_for_variant(variant);
        d_model
            .checked_mul(d_ff)
            .and_then(|matrix_params| matrix_params.checked_mul(matrix_count))
            .ok_or(ExpertBlockQatError::ExpertParameterCountOverflow {
                d_model,
                d_ff,
                matrix_count,
            })?;

        Ok(Self {
            d_model,
            d_ff,
            variant,
        })
    }

    pub fn d_model(self) -> usize {
        self.d_model
    }

    pub fn d_ff(self) -> usize {
        self.d_ff
    }

    pub fn variant(self) -> ExpertMlpVariant {
        self.variant
    }

    pub fn ternary_linear_count(self) -> usize {
        matrix_count_for_variant(self.variant)
    }

    pub fn parameter_count(self) -> usize {
        self.d_model * self.d_ff * self.ternary_linear_count()
    }

    pub fn two_matrix_byte_cost(self) -> ByteCost {
        let plan = TernaryLinearQat::canonical_weight_plan();
        compute_expert_bytes(&plan, self.d_model as u32, self.d_ff as u32)
    }

    pub fn glu_byte_cost(self) -> ByteCost {
        let plan = TernaryLinearQat::canonical_weight_plan();
        compute_glu_expert_bytes_for_diagnostic(&plan, self.d_model as u32, self.d_ff as u32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertMlpConfigEventLevel {
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpertMlpConfigEventCode {
    GluBankFitWarning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertMlpConfigEvent {
    level: ExpertMlpConfigEventLevel,
    code: ExpertMlpConfigEventCode,
    variant: ExpertMlpVariant,
    d_model: usize,
    d_ff: usize,
    ternary_linear_count: usize,
    two_matrix_byte_cost: ByteCost,
    glu_byte_cost: ByteCost,
    message: &'static str,
}

impl ExpertMlpConfigEvent {
    fn glu_bank_fit_warning(config: ExpertMlpConfig) -> Self {
        Self {
            level: ExpertMlpConfigEventLevel::Warning,
            code: ExpertMlpConfigEventCode::GluBankFitWarning,
            variant: config.variant(),
            d_model: config.d_model(),
            d_ff: config.d_ff(),
            ternary_linear_count: config.ternary_linear_count(),
            two_matrix_byte_cost: config.two_matrix_byte_cost(),
            glu_byte_cost: config.glu_byte_cost(),
            message: "GLU experts add a third projection; check the three-matrix byte cost against the ExpertBank budget before enabling execution",
        }
    }

    pub fn level(self) -> ExpertMlpConfigEventLevel {
        self.level
    }

    pub fn code(self) -> ExpertMlpConfigEventCode {
        self.code
    }

    pub fn variant(self) -> ExpertMlpVariant {
        self.variant
    }

    pub fn d_model(self) -> usize {
        self.d_model
    }

    pub fn d_ff(self) -> usize {
        self.d_ff
    }

    pub fn ternary_linear_count(self) -> usize {
        self.ternary_linear_count
    }

    pub fn two_matrix_byte_cost(self) -> ByteCost {
        self.two_matrix_byte_cost
    }

    pub fn glu_byte_cost(self) -> ByteCost {
        self.glu_byte_cost
    }

    pub fn extra_projection_byte_cost(self) -> ByteCost {
        self.glu_byte_cost - self.two_matrix_byte_cost
    }

    pub fn message(self) -> &'static str {
        self.message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClippedActivationKind {
    Relu,
    GeluClip,
    SiluClip,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippedActivation {
    kind: ClippedActivationKind,
}

impl ClippedActivation {
    pub fn new(kind: ClippedActivationKind) -> Self {
        Self { kind }
    }

    pub fn relu() -> Self {
        Self::new(ClippedActivationKind::Relu)
    }

    pub fn gelu_clip() -> Self {
        Self::new(ClippedActivationKind::GeluClip)
    }

    pub fn silu_clip() -> Self {
        Self::new(ClippedActivationKind::SiluClip)
    }

    pub fn kind(self) -> ClippedActivationKind {
        self.kind
    }

    pub fn forward(
        &self,
        input: &[f32],
        range: ActivationRange,
    ) -> Result<Vec<f32>, ExpertBlockQatError> {
        validate_finite("clipped activation input", input)?;
        Ok(input
            .iter()
            .map(|&value| self.apply(value, range))
            .collect())
    }

    fn apply(self, value: f32, range: ActivationRange) -> f32 {
        let activated = match self.kind {
            ClippedActivationKind::Relu => value.max(0.0),
            ClippedActivationKind::GeluClip => gelu(value),
            ClippedActivationKind::SiluClip => silu(value),
        };
        range.clamp(activated)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpertBlockQat {
    experts: Vec<ExpertQat>,
    shared_dense: Option<SharedDenseBranch>,
}

impl ExpertBlockQat {
    pub fn without_shared_dense(experts: Vec<ExpertQat>) -> Result<Self, ExpertBlockQatError> {
        Self::new(experts, None)
    }

    pub fn with_shared_dense(
        experts: Vec<ExpertQat>,
        shared_dense: SharedDenseBranch,
    ) -> Result<Self, ExpertBlockQatError> {
        Self::new(experts, Some(shared_dense))
    }

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

    /// Applies phase hardness to every QAT leaf owned by the expert block.
    pub fn set_hardness(&mut self, expert_qat: QuantHardness, activation_qat: QuantHardness) {
        for expert in &mut self.experts {
            expert.set_hardness(expert_qat, activation_qat);
        }
        if let Some(shared_dense) = &mut self.shared_dense {
            shared_dense.set_hardness(activation_qat);
        }
    }

    /// Returns `residual + selected_expert_delta + alpha * shared_dense_delta`.
    ///
    /// Router weighting is outside this scalar block; this API receives the
    /// already-selected expert id.
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

    pub fn forward_batch(
        &self,
        input: &[f32],
        expert_ids: &[usize],
    ) -> Result<ExpertBatchOutput, ExpertBlockQatError> {
        self.forward_batch_with_options(
            input,
            expert_ids,
            ExpertForwardOptions::hard_quantized_train(),
        )
    }

    pub fn forward_batch_with_options(
        &self,
        input: &[f32],
        expert_ids: &[usize],
        options: ExpertForwardOptions,
    ) -> Result<ExpertBatchOutput, ExpertBlockQatError> {
        let d_model = self.d_model();
        if !input.len().is_multiple_of(d_model) {
            return Err(ExpertBlockQatError::BatchInputLenMismatch {
                d_model,
                actual: input.len(),
            });
        }
        validate_finite("expert batch input", input)?;

        let batch_size = input.len() / d_model;
        if expert_ids.len() != batch_size {
            return Err(ExpertBlockQatError::ExpertIdLenMismatch {
                expected: batch_size,
                actual: expert_ids.len(),
            });
        }

        let mut values = Vec::with_capacity(input.len());
        for (row, &expert_id) in input.chunks_exact(d_model).zip(expert_ids) {
            values.extend(self.forward_with_options(row, expert_id, options)?);
        }

        Ok(ExpertBatchOutput {
            batch_size,
            d_model,
            values,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpertBatchOutput {
    batch_size: usize,
    d_model: usize,
    values: Vec<f32>,
}

impl ExpertBatchOutput {
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn shape(&self) -> [usize; 2] {
        [self.batch_size, self.d_model]
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn into_values(self) -> Vec<f32> {
        self.values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpertQat {
    up_projection: TernaryLinearQat,
    clipped_activation: ClippedActivation,
    activation: ActFakeQuant,
    down_projection: TernaryLinearQat,
}

impl ExpertQat {
    pub fn new(
        up_projection: TernaryLinearQat,
        activation: ActFakeQuant,
        down_projection: TernaryLinearQat,
    ) -> Result<Self, ExpertBlockQatError> {
        let clipped_activation = ClippedActivation::relu();
        Self::new_with_clipped_activation(
            up_projection,
            clipped_activation,
            activation,
            down_projection,
        )
    }

    pub fn new_with_clipped_activation(
        up_projection: TernaryLinearQat,
        clipped_activation: ClippedActivation,
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
        validate_expert_projection_bias("up_projection", &up_projection)?;
        validate_expert_projection_bias("down_projection", &down_projection)?;

        Ok(Self {
            up_projection,
            clipped_activation,
            activation,
            down_projection,
        })
    }

    pub fn new_for_config(
        config: ExpertMlpConfig,
        up_projection: TernaryLinearQat,
        activation: ActFakeQuant,
        down_projection: TernaryLinearQat,
    ) -> Result<Self, ExpertBlockQatError> {
        let expert = Self::new(up_projection, activation, down_projection)?;
        expert.validate_config(config)?;
        Ok(expert)
    }

    pub fn up_projection(&self) -> &TernaryLinearQat {
        &self.up_projection
    }

    pub fn activation(&self) -> &ActFakeQuant {
        &self.activation
    }

    pub fn clipped_activation(&self) -> ClippedActivation {
        self.clipped_activation
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

    pub fn ternary_linear_count(&self) -> usize {
        2
    }

    pub fn set_hardness(&mut self, expert_qat: QuantHardness, activation_qat: QuantHardness) {
        self.up_projection.set_hardness(expert_qat);
        self.activation.set_hardness(activation_qat);
        self.down_projection.set_hardness(expert_qat);
    }

    pub fn validate_config(&self, config: ExpertMlpConfig) -> Result<(), ExpertBlockQatError> {
        if config.variant() != ExpertMlpVariant::TwoMatrix {
            return Err(ExpertBlockQatError::UnsupportedExpertMlpVariant {
                variant: config.variant(),
            });
        }
        validate_config_dimension("d_model", config.d_model(), self.d_model())?;
        validate_config_dimension("d_ff", config.d_ff(), self.d_ff())
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
        let clipped = self
            .clipped_activation
            .forward(&hidden, self.activation.export_range())?;
        let activated = self
            .activation
            .inference_forward(&clipped, options.activation())?;
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
    pub fn for_config(
        config: SharedDenseBranchConfig,
        up_projection: DenseBranchProjection,
        activation: ActFakeQuant,
        down_projection: DenseBranchProjection,
    ) -> Result<Self, ExpertBlockQatError> {
        let branch = Self::new(
            up_projection,
            activation,
            down_projection,
            config.initial_alpha(),
        )?;
        branch.validate_config(config)?;
        Ok(branch)
    }

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

    pub fn d_model(&self) -> usize {
        self.up_projection.shape().input_cols()
    }

    pub fn d_ff_shared(&self) -> usize {
        self.up_projection.shape().output_rows()
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

    pub fn validate_config(
        &self,
        config: SharedDenseBranchConfig,
    ) -> Result<(), ExpertBlockQatError> {
        validate_shared_config_dimension("d_model", config.d_model(), self.d_model())?;
        validate_shared_config_dimension("d_ff_shared", config.d_ff_shared(), self.d_ff_shared())
    }

    pub fn set_hardness(&mut self, activation_qat: QuantHardness) {
        self.activation.set_hardness(activation_qat);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedDenseBranchConfig {
    d_model: usize,
    d_ff_shared: usize,
}

impl SharedDenseBranchConfig {
    pub const INITIAL_ALPHA: f32 = 0.0;

    pub fn new(d_model: usize, d_ff_shared: usize) -> Result<Self, ExpertBlockQatError> {
        validate_nonzero_dimension("d_model", d_model)?;
        validate_nonzero_dimension("d_ff_shared", d_ff_shared)?;
        validate_budget_dimension("d_model", d_model)?;
        validate_budget_dimension("d_ff_shared", d_ff_shared)?;

        Ok(Self {
            d_model,
            d_ff_shared,
        })
    }

    pub fn d_model(self) -> usize {
        self.d_model
    }

    pub fn d_ff_shared(self) -> usize {
        self.d_ff_shared
    }

    pub fn initial_alpha(self) -> f32 {
        Self::INITIAL_ALPHA
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
    ExpertBiasUnsupported {
        projection: &'static str,
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
    SharedConfigDimMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    EmptyExpertDimension {
        field: &'static str,
    },
    ExpertParameterCountOverflow {
        d_model: usize,
        d_ff: usize,
        matrix_count: usize,
    },
    ExpertDimensionExceedsBudgetFormula {
        field: &'static str,
        value: usize,
    },
    UnsupportedExpertMlpVariant {
        variant: ExpertMlpVariant,
    },
    ExpertConfigDimMismatch {
        field: &'static str,
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
    BatchInputLenMismatch {
        d_model: usize,
        actual: usize,
    },
    ExpertIdLenMismatch {
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
            Self::ExpertBiasUnsupported { projection } => write!(
                f,
                "expert {projection} must be biasless; expert byte budgets account only for two ternary weight/scale projections"
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
            Self::SharedConfigDimMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "shared dense config {field} mismatch: expected {expected}, got {actual}"
            ),
            Self::EmptyExpertDimension { field } => write!(f, "{field} must be nonzero"),
            Self::ExpertParameterCountOverflow {
                d_model,
                d_ff,
                matrix_count,
            } => write!(
                f,
                "expert parameter count overflows for d_model={d_model}, d_ff={d_ff}, matrices={matrix_count}"
            ),
            Self::ExpertDimensionExceedsBudgetFormula { field, value } => write!(
                f,
                "{field}={value} exceeds the u32 range used by expert byte-cost formulas"
            ),
            Self::UnsupportedExpertMlpVariant { variant } => write!(
                f,
                "expert MLP variant {variant:?} is not supported by the executable QAT expert path"
            ),
            Self::ExpertConfigDimMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "expert config {field} mismatch: expected {expected}, got {actual}"
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
            Self::BatchInputLenMismatch { d_model, actual } => write!(
                f,
                "expert batch input length {actual} is not a multiple of d_model {d_model}"
            ),
            Self::ExpertIdLenMismatch { expected, actual } => write!(
                f,
                "expert id batch length mismatch: expected {expected}, got {actual}"
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

fn validate_expert_projection_bias(
    projection: &'static str,
    layer: &TernaryLinearQat,
) -> Result<(), ExpertBlockQatError> {
    if layer.bias().is_some() {
        return Err(ExpertBlockQatError::ExpertBiasUnsupported { projection });
    }

    Ok(())
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

fn validate_nonzero_dimension(
    field: &'static str,
    value: usize,
) -> Result<(), ExpertBlockQatError> {
    if value == 0 {
        return Err(ExpertBlockQatError::EmptyExpertDimension { field });
    }

    Ok(())
}

fn validate_budget_dimension(field: &'static str, value: usize) -> Result<(), ExpertBlockQatError> {
    if value > u32::MAX as usize {
        return Err(ExpertBlockQatError::ExpertDimensionExceedsBudgetFormula { field, value });
    }

    Ok(())
}

fn validate_config_dimension(
    field: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), ExpertBlockQatError> {
    if expected != actual {
        return Err(ExpertBlockQatError::ExpertConfigDimMismatch {
            field,
            expected,
            actual,
        });
    }

    Ok(())
}

fn validate_shared_config_dimension(
    field: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), ExpertBlockQatError> {
    if expected != actual {
        return Err(ExpertBlockQatError::SharedConfigDimMismatch {
            field,
            expected,
            actual,
        });
    }

    Ok(())
}

fn matrix_count_for_variant(variant: ExpertMlpVariant) -> usize {
    match variant {
        ExpertMlpVariant::TwoMatrix => 2,
        ExpertMlpVariant::GatedLinearUnit => 3,
    }
}

fn gelu(value: f32) -> f32 {
    const SQRT_2_OVER_PI: f32 = 0.797_884_6;
    0.5 * value * (1.0 + (SQRT_2_OVER_PI * (value + 0.044_715 * value.powi(3))).tanh())
}

fn silu(value: f32) -> f32 {
    value / (1.0 + (-value).exp())
}

#[cfg(test)]
mod tests {
    use crate::qat::{
        ActivationQuantFormat, ActivationRange, ActivationRangeMode, Q8_8Scale, TernaryThreshold,
    };

    use super::*;

    #[test]
    fn qat_expert_default_layout_has_exactly_two_ternary_layers_and_relu_clip() {
        let config = ExpertMlpConfig::default_two_matrix(2, 3).unwrap();
        let expert = fixture_expert();

        assert_eq!(config.variant(), ExpertMlpVariant::TwoMatrix);
        assert_eq!(config.ternary_linear_count(), 2);
        assert_eq!(config.parameter_count(), 2 * 3 * 2);
        expert.validate_config(config).unwrap();
        assert_eq!(expert.ternary_linear_count(), 2);
        assert_eq!(expert.up_projection().shape().input_cols(), 2);
        assert_eq!(expert.up_projection().shape().output_rows(), 3);
        assert_eq!(expert.down_projection().shape().input_cols(), 3);
        assert_eq!(expert.down_projection().shape().output_rows(), 2);
        assert_eq!(
            expert.clipped_activation().kind(),
            ClippedActivationKind::Relu
        );
        assert_eq!(
            expert.validate_config(ExpertMlpConfig::default_two_matrix(4, 3).unwrap()),
            Err(ExpertBlockQatError::ExpertConfigDimMismatch {
                field: "d_model",
                expected: 4,
                actual: 2,
            })
        );
    }

    #[test]
    fn qat_expert_glu_variant_requires_explicit_opt_in_and_structured_warning() {
        let default = ExpertMlpConfig::default_two_matrix(128, 224).unwrap();
        let (glu, event) = ExpertMlpConfig::glu_explicit(128, 224).unwrap();

        assert_eq!(default.variant(), ExpertMlpVariant::TwoMatrix);
        assert_eq!(default.ternary_linear_count(), 2);
        assert_eq!(glu.variant(), ExpertMlpVariant::GatedLinearUnit);
        assert_eq!(glu.ternary_linear_count(), 3);
        assert_eq!(glu.parameter_count(), 128 * 224 * 3);
        assert_eq!(event.level(), ExpertMlpConfigEventLevel::Warning);
        assert_eq!(event.code(), ExpertMlpConfigEventCode::GluBankFitWarning);
        assert_eq!(event.variant(), ExpertMlpVariant::GatedLinearUnit);
        assert_eq!(event.d_model(), 128);
        assert_eq!(event.d_ff(), 224);
        assert_eq!(event.ternary_linear_count(), 3);
        assert_eq!(event.two_matrix_byte_cost(), ByteCost::new(15090));
        assert_eq!(event.glu_byte_cost(), ByteCost::new(22706));
        assert_eq!(event.extra_projection_byte_cost(), ByteCost::new(7616));
        assert!(event.message().contains("third projection"));
        assert_eq!(
            fixture_expert().validate_config(glu),
            Err(ExpertBlockQatError::UnsupportedExpertMlpVariant {
                variant: ExpertMlpVariant::GatedLinearUnit,
            })
        );
        assert_eq!(
            ExpertMlpConfig::default_two_matrix(0, 224),
            Err(ExpertBlockQatError::EmptyExpertDimension { field: "d_model" })
        );
    }

    #[test]
    fn qat_expert_clipped_activation_variants_clamp_to_declared_range() {
        let range = ActivationRange::new(-0.25, 1.0).unwrap();

        let relu = ClippedActivation::relu();
        assert_eq!(
            relu.forward(&[-2.0, 0.5, 2.0], range).unwrap(),
            vec![0.0, 0.5, 1.0]
        );

        let gelu = ClippedActivation::gelu_clip();
        assert_close(
            &gelu.forward(&[-2.0, 2.0], range).unwrap(),
            &[-0.045_402, 1.0],
            1.0e-5,
        );

        let silu = ClippedActivation::silu_clip();
        assert_close(
            &silu.forward(&[-2.0, 2.0], range).unwrap(),
            &[-0.238_406, 1.0],
            1.0e-5,
        );
    }

    #[test]
    fn qat_expert_non_default_clipped_activation_changes_scalar_forward() {
        let relu = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        let gelu = ExpertBlockQat::new(vec![fixture_gelu_expert()], None).unwrap();
        let input = vec![2.0, -2.0];

        let relu_output = relu.forward(&input, 0).unwrap();
        let gelu_output = gelu.forward(&input, 0).unwrap();

        assert_eq!(relu_output, vec![3.0, -2.0]);
        assert_ne!(relu_output, gelu_output);
        assert!(gelu_output.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn qat_expert_composes_up_activation_down_and_residual() {
        let block = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        let input = vec![1.0, 2.0];

        let output = block.forward(&input, 0).unwrap();

        assert_eq!(output, vec![2.0, 1.0]);
    }

    #[test]
    fn qat_expert_clipped_activation_boundary_is_between_up_and_down_projection() {
        let block = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        let input = vec![2.0, -2.0];

        let output = block.forward(&input, 0).unwrap();

        assert_eq!(output, vec![3.0, -2.0]);
    }

    #[test]
    fn qat_expert_batch_forward_shape_is_batch_by_model_dim() {
        let block =
            ExpertBlockQat::new(vec![fixture_expert(), fixture_gelu_expert()], None).unwrap();

        let output = block
            .forward_batch(&[1.0, 2.0, 2.0, -2.0], &[0, 1])
            .unwrap();

        assert_eq!(output.shape(), [2, 2]);
        assert_eq!(output.batch_size(), 2);
        assert_eq!(output.d_model(), 2);
        assert_eq!(&output.values()[0..2], &[2.0, 1.0]);
        assert_ne!(&output.values()[2..4], &[3.0, -2.0]);

        assert_eq!(
            block.forward_batch(&[1.0, 2.0, 3.0], &[0]),
            Err(ExpertBlockQatError::BatchInputLenMismatch {
                d_model: 2,
                actual: 3,
            })
        );
        assert_eq!(
            block.forward_batch(&[1.0, 2.0, 3.0, 4.0], &[0]),
            Err(ExpertBlockQatError::ExpertIdLenMismatch {
                expected: 2,
                actual: 1,
            })
        );
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
    fn qat_expert_shared_dense_branch_is_absent_by_default_constructor() {
        let block = ExpertBlockQat::without_shared_dense(vec![fixture_expert()]).unwrap();

        assert!(block.shared_dense().is_none());
    }

    #[test]
    fn qat_expert_shared_dense_config_defaults_to_zero_alpha() {
        let config = SharedDenseBranchConfig::new(2, 1).unwrap();

        assert_eq!(config.d_model(), 2);
        assert_eq!(config.d_ff_shared(), 1);
        assert_eq!(config.initial_alpha(), 0.0);
        assert_eq!(SharedDenseBranchConfig::INITIAL_ALPHA, 0.0);
        assert_eq!(
            SharedDenseBranchConfig::new(0, 1),
            Err(ExpertBlockQatError::EmptyExpertDimension { field: "d_model" })
        );
        assert_eq!(
            SharedDenseBranchConfig::new(2, 0),
            Err(ExpertBlockQatError::EmptyExpertDimension {
                field: "d_ff_shared"
            })
        );
        assert_eq!(
            SharedDenseBranchConfig::new(2, u32::MAX as usize + 1),
            Err(ExpertBlockQatError::ExpertDimensionExceedsBudgetFormula {
                field: "d_ff_shared",
                value: u32::MAX as usize + 1,
            })
        );
    }

    #[test]
    fn qat_expert_shared_dense_branch_builds_from_config() {
        let config = SharedDenseBranchConfig::new(2, 1).unwrap();
        let branch = SharedDenseBranch::for_config(
            config,
            DenseBranchProjection::new(MatrixShape::new(1, 2).unwrap(), vec![1.0, 1.0], None)
                .unwrap(),
            activation(),
            DenseBranchProjection::new(MatrixShape::new(2, 1).unwrap(), vec![1.0, 2.0], None)
                .unwrap(),
        )
        .unwrap();

        assert_eq!(branch.d_model(), 2);
        assert_eq!(branch.d_ff_shared(), 1);
        assert_eq!(branch.alpha(), 0.0);
        assert_eq!(branch.validate_config(config), Ok(()));

        let mismatched_config = SharedDenseBranchConfig::new(2, 2).unwrap();
        assert_eq!(
            branch.validate_config(mismatched_config),
            Err(ExpertBlockQatError::SharedConfigDimMismatch {
                field: "d_ff_shared",
                expected: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn qat_expert_optional_shared_dense_branch_is_explicit_and_alpha_gated() {
        let shared = fixture_shared_dense(0.5);
        let block = ExpertBlockQat::with_shared_dense(vec![fixture_expert()], shared).unwrap();
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

        assert_eq!(
            ExpertQat::new(
                ternary_linear(2, 2, vec![1.0, 0.0, 0.0, 1.0], Some(vec![0.0, 0.0])),
                activation(),
                ternary_linear(2, 2, vec![1.0, 0.0, 0.0, 1.0], None),
            ),
            Err(ExpertBlockQatError::ExpertBiasUnsupported {
                projection: "up_projection",
            })
        );
        assert_eq!(
            ExpertQat::new(
                ternary_linear(2, 2, vec![1.0, 0.0, 0.0, 1.0], None),
                activation(),
                ternary_linear(2, 2, vec![1.0, 0.0, 0.0, 1.0], Some(vec![0.0, 0.0])),
            ),
            Err(ExpertBlockQatError::ExpertBiasUnsupported {
                projection: "down_projection",
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
                None,
            ),
            activation(),
            ternary_linear(
                2,
                3,
                vec![
                    1.0, 0.0, 0.0, //
                    0.0, -1.0, 0.0,
                ],
                None,
            ),
        )
        .unwrap()
    }

    fn fixture_gelu_expert() -> ExpertQat {
        ExpertQat::new_with_clipped_activation(
            ternary_linear(
                3,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0, //
                    0.25, 0.25,
                ],
                None,
            ),
            ClippedActivation::gelu_clip(),
            activation(),
            ternary_linear(
                2,
                3,
                vec![
                    1.0, 0.0, 0.0, //
                    0.0, -1.0, 0.0,
                ],
                None,
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
