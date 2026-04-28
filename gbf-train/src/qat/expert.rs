//! Burn-backed expert block QAT adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{
    ActivationForwardMode, ClippedActivation, ClippedActivationKind, DenseBranchProjection,
    ExpertBlockQat, ExpertBlockQatError, ExpertForwardOptions, ExpertQat, ExpertQatForwardMode,
    MatrixShape, QatHardnessControl, QuantHardness, SharedDenseBranch,
};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, BurnParam,
    burn_gelu_approximate, burn_linear, burn_relu, burn_silu, float_tensor_from_vec,
    float_tensor_into_vec, float_tensor_shape, ste_clamp,
};
use crate::qat::{
    ActFakeQuantBurnQat, ActFakeQuantBurnQatError, TernaryLinearBurnQat, TernaryLinearBurnQatError,
    ThresholdScheduleProgress,
};
use crate::scheduler::{PhaseControlledModel, PhaseControls};

#[derive(BurnModule, Debug)]
pub struct ExpertBlockBurnQat<B: BurnBackend> {
    experts: Vec<ExpertBurnQat<B>>,
    shared_dense: Option<SharedDenseBurnBranch<B>>,
}

impl<B: BurnBackend> ExpertBlockBurnQat<B> {
    pub fn from_core(
        core: ExpertBlockQat,
        device: &BurnDevice<B>,
    ) -> Result<Self, ExpertBlockBurnQatError> {
        let experts = core
            .experts()
            .iter()
            .cloned()
            .map(|expert| ExpertBurnQat::from_core(expert, device))
            .collect::<Result<Vec<_>, _>>()?;
        let shared_dense = core
            .shared_dense()
            .cloned()
            .map(|shared_dense| SharedDenseBurnBranch::from_core(shared_dense, device))
            .transpose()?;

        Ok(Self {
            experts,
            shared_dense,
        })
    }

    #[must_use]
    pub fn experts(&self) -> &[ExpertBurnQat<B>] {
        &self.experts
    }

    #[must_use]
    pub fn shared_dense(&self) -> Option<&SharedDenseBurnBranch<B>> {
        self.shared_dense.as_ref()
    }

    #[must_use]
    pub fn d_model(&self) -> usize {
        self.experts[0].d_model()
    }

    pub fn set_hardness(&mut self, expert_qat: QuantHardness, activation_qat: QuantHardness) {
        for expert in &mut self.experts {
            expert.set_hardness(expert_qat, activation_qat);
        }
        if let Some(shared_dense) = &mut self.shared_dense {
            shared_dense.set_hardness(activation_qat);
        }
    }

    pub fn set_threshold_schedule_progress(&mut self, progress: ThresholdScheduleProgress) {
        for expert in &mut self.experts {
            expert.set_threshold_schedule_progress(progress);
        }
    }

    pub fn forward(
        &self,
        input: BurnFloatTensor<B, 1>,
        expert_id: usize,
        options: ExpertForwardOptions,
    ) -> Result<BurnFloatTensor<B, 1>, ExpertBlockBurnQatError> {
        validate_input_shape(self.d_model(), &input)?;
        validate_finite_input("expert input", &input)?;
        let expert =
            self.experts
                .get(expert_id)
                .ok_or(ExpertBlockBurnQatError::ExpertIdOutOfRange {
                    expert_id,
                    n_experts: self.experts.len(),
                })?;
        let expert_delta = expert.forward_delta(input.clone(), options)?;
        let shared_delta = self
            .shared_dense
            .as_ref()
            .map(|shared_dense| shared_dense.forward(input.clone(), options.activation()))
            .transpose()?;
        let shared_delta = shared_delta.unwrap_or_else(|| input.zeros_like());

        Ok(input + expert_delta + shared_delta)
    }

    pub fn forward_batch(
        &self,
        input: BurnFloatTensor<B, 2>,
        expert_ids: &[usize],
        options: ExpertForwardOptions,
    ) -> Result<BurnFloatTensor<B, 2>, ExpertBlockBurnQatError> {
        let shape = float_tensor_shape(&input);
        if shape[1] != self.d_model() {
            return Err(ExpertBlockBurnQatError::InputLastDimMismatch {
                expected: self.d_model(),
                actual: shape[1],
                shape: shape.to_vec(),
            });
        }
        if expert_ids.len() != shape[0] {
            return Err(ExpertBlockBurnQatError::ExpertIdLenMismatch {
                expected: shape[0],
                actual: expert_ids.len(),
            });
        }
        validate_finite_input("expert batch input", &input)?;
        if shape[0] == 0 {
            let device = input.device();
            return Ok(BurnFloatTensor::<B, 2>::zeros([0, self.d_model()], &device));
        }

        let rows = expert_ids
            .iter()
            .enumerate()
            .map(|(row_index, &expert_id)| {
                let row = input
                    .clone()
                    .slice([row_index..row_index + 1, 0..self.d_model()])
                    .reshape([self.d_model()]);
                self.forward(row, expert_id, options)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(BurnFloatTensor::<B, 1>::stack::<2>(rows, 0))
    }

    pub fn to_core_from_trained_state(&self) -> Result<ExpertBlockQat, ExpertBlockBurnQatError> {
        let experts = self
            .experts
            .iter()
            .map(ExpertBurnQat::to_core_from_trained_state)
            .collect::<Result<Vec<_>, _>>()?;
        let shared_dense = self
            .shared_dense
            .as_ref()
            .map(SharedDenseBurnBranch::to_core_from_trained_state)
            .transpose()?;

        ExpertBlockQat::new(experts, shared_dense).map_err(ExpertBlockBurnQatError::Model)
    }
}

impl<B: BurnBackend> PhaseControlledModel for ExpertBlockBurnQat<B> {
    fn apply_phase_controls(&mut self, controls: PhaseControls) {
        self.set_hardness(controls.expert_qat(), controls.activation_qat());
        self.set_threshold_schedule_progress(
            ThresholdScheduleProgress::new(controls.threshold_schedule_progress())
                .expect("phase progress is finite and within [0, 1]"),
        );
    }
}

#[derive(BurnModule, Debug)]
pub struct ExpertBurnQat<B: BurnBackend> {
    up_projection: TernaryLinearBurnQat<B>,
    #[module(skip)]
    clipped_activation: ClippedActivation,
    #[module(skip)]
    activation: ActFakeQuantBurnQat,
    down_projection: TernaryLinearBurnQat<B>,
}

impl<B: BurnBackend> ExpertBurnQat<B> {
    pub fn from_core(
        core: ExpertQat,
        device: &BurnDevice<B>,
    ) -> Result<Self, ExpertBlockBurnQatError> {
        Ok(Self {
            up_projection: TernaryLinearBurnQat::from_core(core.up_projection().clone(), device)?,
            clipped_activation: core.clipped_activation(),
            activation: ActFakeQuantBurnQat::from_core(core.activation().clone())?,
            down_projection: TernaryLinearBurnQat::from_core(
                core.down_projection().clone(),
                device,
            )?,
        })
    }

    #[must_use]
    pub fn up_projection(&self) -> &TernaryLinearBurnQat<B> {
        &self.up_projection
    }

    #[must_use]
    pub fn down_projection(&self) -> &TernaryLinearBurnQat<B> {
        &self.down_projection
    }

    #[must_use]
    pub fn d_model(&self) -> usize {
        self.up_projection.shape().input_cols()
    }

    pub fn set_hardness(&mut self, expert_qat: QuantHardness, activation_qat: QuantHardness) {
        self.up_projection.set_hardness(expert_qat);
        self.activation.set_hardness(activation_qat);
        self.down_projection.set_hardness(expert_qat);
    }

    pub fn set_threshold_schedule_progress(&mut self, progress: ThresholdScheduleProgress) {
        self.up_projection.set_threshold_schedule_progress(progress);
        self.down_projection
            .set_threshold_schedule_progress(progress);
    }

    fn forward_delta(
        &self,
        input: BurnFloatTensor<B, 1>,
        options: ExpertForwardOptions,
    ) -> Result<BurnFloatTensor<B, 1>, ExpertBlockBurnQatError> {
        let hidden = match options.expert_qat() {
            ExpertQatForwardMode::FullPrecision => {
                full_precision_forward(&self.up_projection, input)?
            }
            ExpertQatForwardMode::HardQuantized => self.up_projection.fake_quant_forward(input)?,
        };
        let clipped = clipped_activation_forward(
            hidden,
            self.clipped_activation,
            self.activation.export_range(),
        );
        let activated = self
            .activation
            .fake_quant_forward(clipped, options.activation());

        match options.expert_qat() {
            ExpertQatForwardMode::FullPrecision => {
                full_precision_forward(&self.down_projection, activated)
            }
            ExpertQatForwardMode::HardQuantized => self
                .down_projection
                .fake_quant_forward(activated)
                .map_err(Into::into),
        }
    }

    fn to_core_from_trained_state(&self) -> Result<ExpertQat, ExpertBlockBurnQatError> {
        ExpertQat::new_with_clipped_activation(
            self.up_projection.to_core_from_trained_state()?,
            self.clipped_activation,
            self.activation.core().clone(),
            self.down_projection.to_core_from_trained_state()?,
        )
        .map_err(ExpertBlockBurnQatError::Model)
    }
}

#[derive(BurnModule, Debug)]
pub struct SharedDenseBurnBranch<B: BurnBackend> {
    up_projection: DenseBranchBurnProjection<B>,
    #[module(skip)]
    activation: ActFakeQuantBurnQat,
    down_projection: DenseBranchBurnProjection<B>,
    alpha: BurnParam<BurnFloatTensor<B, 1>>,
}

impl<B: BurnBackend> SharedDenseBurnBranch<B> {
    pub fn from_core(
        core: SharedDenseBranch,
        device: &BurnDevice<B>,
    ) -> Result<Self, ExpertBlockBurnQatError> {
        Ok(Self {
            up_projection: DenseBranchBurnProjection::from_core(
                core.up_projection().clone(),
                device,
            )?,
            activation: ActFakeQuantBurnQat::from_core(core.activation().clone())?,
            down_projection: DenseBranchBurnProjection::from_core(
                core.down_projection().clone(),
                device,
            )?,
            alpha: BurnParam::from_tensor(float_tensor_from_vec(vec![core.alpha()], [1], device)?),
        })
    }

    #[must_use]
    pub fn alpha(&self) -> BurnFloatTensor<B, 1> {
        self.alpha.val()
    }

    #[must_use]
    pub fn up_projection(&self) -> &DenseBranchBurnProjection<B> {
        &self.up_projection
    }

    #[must_use]
    pub fn down_projection(&self) -> &DenseBranchBurnProjection<B> {
        &self.down_projection
    }

    pub fn set_hardness(&mut self, activation_qat: QuantHardness) {
        self.activation.set_hardness(activation_qat);
    }

    fn forward(
        &self,
        input: BurnFloatTensor<B, 1>,
        activation_mode: ActivationForwardMode,
    ) -> Result<BurnFloatTensor<B, 1>, ExpertBlockBurnQatError> {
        let hidden = self.up_projection.forward(input)?;
        let activated = self.activation.fake_quant_forward(hidden, activation_mode);
        let output = self.down_projection.forward(activated)?;
        let alpha = self.alpha().expand(output.shape());

        Ok(output * alpha)
    }

    fn to_core_from_trained_state(&self) -> Result<SharedDenseBranch, ExpertBlockBurnQatError> {
        SharedDenseBranch::new(
            self.up_projection.to_core_from_trained_state()?,
            self.activation.core().clone(),
            self.down_projection.to_core_from_trained_state()?,
            scalar_from_tensor("shared_dense.alpha", self.alpha().detach())?,
        )
        .map_err(ExpertBlockBurnQatError::Model)
    }
}

#[derive(BurnModule, Debug)]
pub struct DenseBranchBurnProjection<B: BurnBackend> {
    #[module(skip)]
    shape: MatrixShape,
    weights: BurnParam<BurnFloatTensor<B, 2>>,
    bias: Option<BurnParam<BurnFloatTensor<B, 1>>>,
}

impl<B: BurnBackend> DenseBranchBurnProjection<B> {
    pub fn from_core(
        core: DenseBranchProjection,
        device: &BurnDevice<B>,
    ) -> Result<Self, ExpertBlockBurnQatError> {
        let shape = core.shape();
        Ok(Self {
            shape,
            weights: BurnParam::from_tensor(float_tensor_from_vec(
                core.weights().to_vec(),
                [shape.output_rows(), shape.input_cols()],
                device,
            )?),
            bias: core
                .bias()
                .map(|bias| float_tensor_from_vec(bias.to_vec(), [shape.output_rows()], device))
                .transpose()?
                .map(BurnParam::from_tensor),
        })
    }

    #[must_use]
    pub fn weights(&self) -> BurnFloatTensor<B, 2> {
        self.weights.val()
    }

    #[must_use]
    pub fn bias(&self) -> Option<BurnFloatTensor<B, 1>> {
        self.bias.as_ref().map(BurnParam::val)
    }

    fn forward(
        &self,
        input: BurnFloatTensor<B, 1>,
    ) -> Result<BurnFloatTensor<B, 1>, ExpertBlockBurnQatError> {
        validate_input_shape(self.shape.input_cols(), &input)?;

        Ok(burn_linear(input, self.weights().transpose(), self.bias()))
    }

    fn to_core_from_trained_state(&self) -> Result<DenseBranchProjection, ExpertBlockBurnQatError> {
        let weights = float_tensor_into_vec(self.weights().detach())?;
        let bias = self
            .bias()
            .map(|bias| float_tensor_into_vec(bias.detach()))
            .transpose()?;

        DenseBranchProjection::new(self.shape, weights, bias)
            .map_err(ExpertBlockBurnQatError::Model)
    }
}

#[derive(Debug)]
pub enum ExpertBlockBurnQatError {
    Adapter(BurnAdapterError),
    Model(ExpertBlockQatError),
    Ternary(TernaryLinearBurnQatError),
    Activation(ActFakeQuantBurnQatError),
    ExpertIdOutOfRange {
        expert_id: usize,
        n_experts: usize,
    },
    ExpertIdLenMismatch {
        expected: usize,
        actual: usize,
    },
    InputLastDimMismatch {
        expected: usize,
        actual: usize,
        shape: Vec<usize>,
    },
    ScalarParamLen {
        name: &'static str,
        actual: usize,
    },
}

impl fmt::Display for ExpertBlockBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(error) => write!(f, "{error}"),
            Self::Model(error) => write!(f, "{error}"),
            Self::Ternary(error) => write!(f, "{error}"),
            Self::Activation(error) => write!(f, "{error}"),
            Self::ExpertIdOutOfRange {
                expert_id,
                n_experts,
            } => write!(
                f,
                "expert id {expert_id} is out of range for {n_experts} experts"
            ),
            Self::ExpertIdLenMismatch { expected, actual } => write!(
                f,
                "expert id batch length mismatch: expected {expected}, got {actual}"
            ),
            Self::InputLastDimMismatch {
                expected,
                actual,
                shape,
            } => write!(
                f,
                "expert input last dimension mismatch: expected {expected}, got {actual} in shape {shape:?}"
            ),
            Self::ScalarParamLen { name, actual } => {
                write!(f, "{name} expected one scalar value, got {actual}")
            }
        }
    }
}

impl Error for ExpertBlockBurnQatError {}

impl From<BurnAdapterError> for ExpertBlockBurnQatError {
    fn from(error: BurnAdapterError) -> Self {
        Self::Adapter(error)
    }
}

impl From<ExpertBlockQatError> for ExpertBlockBurnQatError {
    fn from(error: ExpertBlockQatError) -> Self {
        Self::Model(error)
    }
}

impl From<TernaryLinearBurnQatError> for ExpertBlockBurnQatError {
    fn from(error: TernaryLinearBurnQatError) -> Self {
        Self::Ternary(error)
    }
}

impl From<ActFakeQuantBurnQatError> for ExpertBlockBurnQatError {
    fn from(error: ActFakeQuantBurnQatError) -> Self {
        Self::Activation(error)
    }
}

fn validate_input_shape<B: BurnBackend>(
    expected: usize,
    input: &BurnFloatTensor<B, 1>,
) -> Result<(), ExpertBlockBurnQatError> {
    let shape = float_tensor_shape(input);
    if shape[0] != expected {
        return Err(ExpertBlockBurnQatError::InputLastDimMismatch {
            expected,
            actual: shape[0],
            shape: shape.to_vec(),
        });
    }

    Ok(())
}

fn validate_finite_input<B: BurnBackend, const D: usize>(
    name: &'static str,
    input: &BurnFloatTensor<B, D>,
) -> Result<(), ExpertBlockBurnQatError> {
    let values = float_tensor_into_vec(input.clone().detach())?;
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(ExpertBlockQatError::NonFiniteInput { name, index }.into());
    }

    Ok(())
}

fn full_precision_forward<B: BurnBackend>(
    layer: &TernaryLinearBurnQat<B>,
    input: BurnFloatTensor<B, 1>,
) -> Result<BurnFloatTensor<B, 1>, ExpertBlockBurnQatError> {
    validate_input_shape(layer.shape().input_cols(), &input)?;

    Ok(burn_linear(
        input,
        layer.full_precision_weights().transpose(),
        layer.bias(),
    ))
}

fn clipped_activation_forward<B: BurnBackend>(
    input: BurnFloatTensor<B, 1>,
    activation: ClippedActivation,
    range: gbf_model::qat::ActivationRange,
) -> BurnFloatTensor<B, 1> {
    let activated = match activation.kind() {
        ClippedActivationKind::Relu => burn_relu(input),
        ClippedActivationKind::GeluClip => burn_gelu_approximate(input),
        ClippedActivationKind::SiluClip => burn_silu(input),
    };

    ste_clamp(activated, range.lo(), range.hi())
}

fn scalar_from_tensor<B: BurnBackend>(
    name: &'static str,
    tensor: BurnFloatTensor<B, 1>,
) -> Result<f32, ExpertBlockBurnQatError> {
    let values = float_tensor_into_vec(tensor)?;
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(ExpertBlockBurnQatError::ScalarParamLen {
            name,
            actual: values.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use gbf_model::qat::{
        ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
        ActivationRangeMode, Q8_8Scale, TernaryLinearQat, TernaryThreshold,
    };

    use super::*;
    use crate::adapter::burn::{
        BurnModuleMapper, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, BurnParam,
        float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape,
    };
    use crate::logging::{TestEventCollector, TrainingLogEmitter};
    use crate::phase::TrainingPhaseSchedule;
    use crate::scheduler::TrainingPhaseScheduler;

    #[test]
    fn burn_expert_forward_matches_scalar_expert_block() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = fixture_block(0.5);
        let layer = ExpertBlockBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let input = vec![1.0, 2.0];
        let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [2], &device).unwrap();

        let output = layer
            .forward(tensor, 0, ExpertForwardOptions::hard_quantized_train())
            .unwrap();

        assert_eq!(
            float_tensor_into_vec(output).unwrap(),
            core.forward(&input, 0).unwrap()
        );
    }

    #[test]
    fn burn_expert_forwards_full_precision_and_eval_activation_options() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core = fixture_eval_passthrough_block(0.25);
        let layer = ExpertBlockBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let input = vec![0.23, 0.41];

        for options in [
            ExpertForwardOptions::full_precision_train(),
            ExpertForwardOptions::hard_quantized_train()
                .with_activation(ActivationForwardMode::Eval),
        ] {
            let tensor = float_tensor_from_vec::<B, 1>(input.clone(), [2], &device).unwrap();
            let output = layer.forward(tensor, 0, options).unwrap();
            let scalar_output = core.forward_with_options(&input, 0, options).unwrap();

            assert_close(
                &float_tensor_into_vec(output).unwrap(),
                &scalar_output,
                1.0e-6,
            );
        }

        let train = core
            .forward_with_options(&input, 0, ExpertForwardOptions::hard_quantized_train())
            .unwrap();
        let eval = core
            .forward_with_options(
                &input,
                0,
                ExpertForwardOptions::hard_quantized_train()
                    .with_activation(ActivationForwardMode::Eval),
            )
            .unwrap();
        assert_ne!(train, eval);
    }

    #[test]
    fn burn_expert_hardness_can_change_after_adapter_construction() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let mut core = fixture_block(0.5);
        let mut layer = ExpertBlockBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let input = vec![0.25, 0.5];

        let hard_options =
            ExpertForwardOptions::for_hardness(QuantHardness::Hard, QuantHardness::Hard);
        let hard_tensor = float_tensor_from_vec::<B, 1>(input.clone(), [2], &device).unwrap();
        let hard = layer.forward(hard_tensor, 0, hard_options).unwrap();
        let hard_values = float_tensor_into_vec(hard).unwrap();
        assert_close(
            &hard_values,
            &core.forward_with_options(&input, 0, hard_options).unwrap(),
            1.0e-6,
        );

        core.set_hardness(QuantHardness::Soft, QuantHardness::Soft);
        layer.set_hardness(QuantHardness::Soft, QuantHardness::Soft);
        let soft_options =
            ExpertForwardOptions::for_hardness(QuantHardness::Soft, QuantHardness::Soft);
        let soft_tensor = float_tensor_from_vec::<B, 1>(input.clone(), [2], &device).unwrap();
        let soft = layer.forward(soft_tensor, 0, soft_options).unwrap();
        let soft_values = float_tensor_into_vec(soft).unwrap();
        assert_close(
            &soft_values,
            &core.forward_with_options(&input, 0, soft_options).unwrap(),
            1.0e-6,
        );

        core.set_hardness(QuantHardness::Off, QuantHardness::Off);
        layer.set_hardness(QuantHardness::Off, QuantHardness::Off);
        let off_options =
            ExpertForwardOptions::for_hardness(QuantHardness::Off, QuantHardness::Off);
        let off_tensor = float_tensor_from_vec::<B, 1>(input.clone(), [2], &device).unwrap();
        let off = layer.forward(off_tensor, 0, off_options).unwrap();
        let off_values = float_tensor_into_vec(off).unwrap();
        assert_close(
            &off_values,
            &core.forward_with_options(&input, 0, off_options).unwrap(),
            1.0e-6,
        );
        assert_ne!(hard_values, soft_values);
        assert_ne!(soft_values, off_values);
    }

    #[test]
    fn burn_expert_phase_controls_apply_threshold_schedule_progress() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let mut scheduler =
            TrainingPhaseScheduler::new(TrainingPhaseSchedule::default_five_phase(10).unwrap());
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector);
        let mut layer = ExpertBlockBurnQat::<B>::from_core(fixture_block(0.0), &device).unwrap();

        scheduler.apply_step(20, &mut layer, &emitter).unwrap();
        let expert = &layer.experts()[0];
        assert_eq!(
            expert.up_projection().threshold_schedule_progress(),
            ThresholdScheduleProgress::start()
        );
        assert_eq!(
            expert.down_projection().threshold_schedule_progress(),
            ThresholdScheduleProgress::start()
        );

        scheduler.apply_step(24, &mut layer, &emitter).unwrap();
        let expert = &layer.experts()[0];
        let up_progress = expert.up_projection().threshold_schedule_progress().value();
        let down_progress = expert
            .down_projection()
            .threshold_schedule_progress()
            .value();

        assert!(up_progress > 0.44, "{up_progress}");
        assert!(up_progress < 0.45, "{up_progress}");
        assert_eq!(up_progress, down_progress);

        scheduler.apply_step(29, &mut layer, &emitter).unwrap();
        let expert = &layer.experts()[0];
        assert_eq!(
            expert.up_projection().threshold_schedule_progress(),
            ThresholdScheduleProgress::complete()
        );

        scheduler.apply_step(30, &mut layer, &emitter).unwrap();
        let expert = &layer.experts()[0];
        assert_eq!(
            expert.up_projection().threshold_schedule_progress(),
            ThresholdScheduleProgress::complete()
        );
    }

    #[test]
    fn burn_expert_batch_forward_routes_each_row() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let core =
            ExpertBlockQat::new(vec![fixture_expert(), fixture_gelu_expert()], None).unwrap();
        let layer = ExpertBlockBurnQat::<B>::from_core(core.clone(), &device).unwrap();
        let input = vec![1.0, 2.0, 2.0, -2.0];
        let tensor = float_tensor_from_vec::<B, 2>(input.clone(), [2, 2], &device).unwrap();

        let output = layer
            .forward_batch(
                tensor,
                &[0, 1],
                ExpertForwardOptions::hard_quantized_train(),
            )
            .unwrap();

        assert_eq!(float_tensor_shape(&output), [2, 2]);
        assert_eq!(
            float_tensor_into_vec(output).unwrap(),
            core.forward_batch(&input, &[0, 1]).unwrap().into_values()
        );

        let empty = float_tensor_from_vec::<B, 2>(Vec::new(), [0, 2], &device).unwrap();
        let output = layer
            .forward_batch(empty, &[], ExpertForwardOptions::hard_quantized_train())
            .unwrap();

        assert_eq!(float_tensor_shape(&output), [0, 2]);
        assert!(float_tensor_into_vec(output).unwrap().is_empty());
    }

    #[test]
    fn burn_expert_gradients_reach_selected_expert_and_shared_branch() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let layer = ExpertBlockBurnQat::<B>::from_core(fixture_block(0.5), &device).unwrap();
        let input = float_tensor_from_vec::<B, 1>(vec![0.25, 0.5], [2], &device)
            .unwrap()
            .require_grad();

        let output = layer
            .forward(
                input.clone(),
                0,
                ExpertForwardOptions::hard_quantized_train(),
            )
            .unwrap();
        let gradients = output.sum().backward();
        let expert = &layer.experts()[0];
        let shared = layer.shared_dense().expect("fixture has a shared branch");

        assert!(
            float_tensor_into_vec(
                expert
                    .up_projection()
                    .full_precision_weights()
                    .grad(&gradients)
                    .unwrap()
            )
            .unwrap()
            .iter()
            .any(|value| value.abs() > 0.0)
        );
        assert!(
            float_tensor_into_vec(
                expert
                    .down_projection()
                    .full_precision_weights()
                    .grad(&gradients)
                    .unwrap()
            )
            .unwrap()
            .iter()
            .any(|value| value.abs() > 0.0)
        );
        assert!(
            float_tensor_into_vec(shared.up_projection().weights().grad(&gradients).unwrap())
                .unwrap()
                .iter()
                .any(|value| value.abs() > 0.0)
        );
        assert!(
            float_tensor_into_vec(shared.alpha().grad(&gradients).unwrap()).unwrap()[0].abs() > 0.0
        );
    }

    #[test]
    fn burn_expert_zero_alpha_shared_branch_keeps_expert_output_and_trains_alpha() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let core = fixture_block(0.0);
        let expert_only = ExpertBlockQat::new(vec![fixture_expert()], None).unwrap();
        let layer = ExpertBlockBurnQat::<B>::from_core(core, &device).unwrap();
        let input_values = vec![0.25, 0.5];
        let input = float_tensor_from_vec::<B, 1>(input_values.clone(), [2], &device)
            .unwrap()
            .require_grad();

        let output = layer
            .forward(input, 0, ExpertForwardOptions::hard_quantized_train())
            .unwrap();
        assert_eq!(
            float_tensor_into_vec(output.clone()).unwrap(),
            expert_only.forward(&input_values, 0).unwrap()
        );

        let gradients = output.sum().backward();
        let expert = &layer.experts()[0];
        let shared = layer.shared_dense().expect("fixture has a shared branch");

        assert!(
            float_tensor_into_vec(
                expert
                    .up_projection()
                    .full_precision_weights()
                    .grad(&gradients)
                    .unwrap()
            )
            .unwrap()
            .iter()
            .any(|value| value.abs() > 0.0)
        );
        assert!(
            float_tensor_into_vec(
                expert
                    .down_projection()
                    .full_precision_weights()
                    .grad(&gradients)
                    .unwrap()
            )
            .unwrap()
            .iter()
            .any(|value| value.abs() > 0.0)
        );
        assert!(
            float_tensor_into_vec(shared.alpha().grad(&gradients).unwrap()).unwrap()[0].abs() > 0.0
        );
    }

    #[test]
    fn burn_expert_export_handoff_uses_burn_owned_tensors() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let original = fixture_block(0.25);
        let layer = ExpertBlockBurnQat::<B>::from_core(original.clone(), &device).unwrap();
        let mut mapper = AddToFloatParams(0.125);
        let layer = layer.map(&mut mapper);
        let exported = layer.to_core_from_trained_state().unwrap();
        let original_expert = &original.experts()[0];
        let exported_expert = &exported.experts()[0];

        assert_close(
            exported_expert.up_projection().full_precision_weights(),
            &add_delta(
                original_expert.up_projection().full_precision_weights(),
                0.125,
            ),
            0.0,
        );
        assert_close(
            exported_expert.down_projection().full_precision_weights(),
            &add_delta(
                original_expert.down_projection().full_precision_weights(),
                0.125,
            ),
            0.0,
        );
        assert_eq!(exported.shared_dense().unwrap().alpha(), 0.375);
    }

    #[test]
    fn burn_expert_rejects_shape_and_dynamic_activation_contracts() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = ExpertBlockBurnQat::<B>::from_core(fixture_block(0.0), &device).unwrap();
        let bad_input = float_tensor_from_vec::<B, 1>(vec![1.0], [1], &device).unwrap();

        assert!(matches!(
            layer.forward(bad_input, 0, ExpertForwardOptions::hard_quantized_train()),
            Err(ExpertBlockBurnQatError::InputLastDimMismatch {
                expected: 2,
                actual: 1,
                ..
            })
        ));

        let bad_input =
            float_tensor_from_vec::<B, 1>(vec![1.0, f32::INFINITY], [2], &device).unwrap();
        assert!(matches!(
            layer.forward(bad_input, 0, ExpertForwardOptions::hard_quantized_train()),
            Err(ExpertBlockBurnQatError::Model(
                ExpertBlockQatError::NonFiniteInput {
                    name: "expert input",
                    index: 1,
                }
            ))
        ));

        let dynamic = ExpertBlockQat::new(vec![dynamic_activation_expert()], None).unwrap();
        assert!(matches!(
            ExpertBlockBurnQat::<B>::from_core(dynamic, &device),
            Err(ExpertBlockBurnQatError::Activation(
                ActFakeQuantBurnQatError::UnsupportedRangeMode { .. }
            ))
        ));
    }

    fn fixture_block(shared_alpha: f32) -> ExpertBlockQat {
        ExpertBlockQat::new(
            vec![fixture_expert()],
            Some(fixture_shared_dense(shared_alpha)),
        )
        .unwrap()
    }

    fn fixture_eval_passthrough_block(shared_alpha: f32) -> ExpertBlockQat {
        ExpertBlockQat::new(
            vec![fixture_eval_passthrough_expert()],
            Some(fixture_eval_passthrough_shared_dense(shared_alpha)),
        )
        .unwrap()
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

    fn fixture_eval_passthrough_expert() -> ExpertQat {
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
            eval_passthrough_activation(),
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

    fn dynamic_activation_expert() -> ExpertQat {
        ExpertQat::new(
            ternary_linear(1, 2, vec![1.0, 1.0], None),
            ActFakeQuant::new(
                ActivationRangeMode::Learned(ActivationRange::new(-1.0, 1.0).unwrap()),
                ActivationQuantFormat::Int8,
            )
            .unwrap(),
            ternary_linear(2, 1, vec![1.0, 1.0], None),
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

    fn fixture_eval_passthrough_shared_dense(alpha: f32) -> SharedDenseBranch {
        SharedDenseBranch::new(
            DenseBranchProjection::new(MatrixShape::new(1, 2).unwrap(), vec![1.0, 1.0], None)
                .unwrap(),
            eval_passthrough_activation(),
            DenseBranchProjection::new(MatrixShape::new(2, 1).unwrap(), vec![1.0, 2.0], None)
                .unwrap(),
            alpha,
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

    fn eval_passthrough_activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt4,
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

    fn add_delta(values: &[f32], delta: f32) -> Vec<f32> {
        values.iter().map(|value| value + delta).collect()
    }

    struct AddToFloatParams(f32);

    impl<B: BurnBackend> BurnModuleMapper<B> for AddToFloatParams {
        fn map_float<const D: usize>(
            &mut self,
            param: BurnParam<BurnFloatTensor<B, D>>,
        ) -> BurnParam<BurnFloatTensor<B, D>> {
            param.map(|tensor| tensor + self.0)
        }
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
