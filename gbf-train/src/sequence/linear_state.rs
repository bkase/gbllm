//! Burn-backed LinearState sequence block adapter.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{QatHardnessControl, QuantHardness};
use gbf_model::sequence::{LinearStateBlock, LinearStateForwardOptions};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, float_tensor_into_vec,
    float_tensor_shape,
};
use crate::qat::{
    ActFakeQuantBurnQat, ActFakeQuantBurnQatError, NormApproxBurnQat, NormApproxBurnQatError,
    TernaryLinearBurnQat, TernaryLinearBurnQatError, ThresholdScheduleProgress,
};
use crate::scheduler::{PhaseControlledModel, PhaseControls};

// Mirrors gbf_model::sequence::LinearStateBlock's fixed recurrence law until a
// future decay-policy owner introduces configurable or learned state decay.
const STATE_DECAY: f32 = 0.5;

#[derive(BurnModule, Debug)]
pub struct LinearStateBurnQat<B: BurnBackend> {
    #[module(skip)]
    d_model: usize,
    #[module(skip)]
    state_slots: usize,
    input_norm: NormApproxBurnQat<B>,
    #[module(skip)]
    input_activation: ActFakeQuantBurnQat,
    input_to_state: TernaryLinearBurnQat<B>,
    state_to_output: TernaryLinearBurnQat<B>,
    #[module(skip)]
    output_activation: ActFakeQuantBurnQat,
}

impl<B: BurnBackend> LinearStateBurnQat<B> {
    pub fn from_core(
        core: LinearStateBlock,
        device: &BurnDevice<B>,
    ) -> Result<Self, LinearStateBurnQatError> {
        Ok(Self {
            d_model: core.config().d_model(),
            state_slots: core.config().state_slots(),
            input_norm: NormApproxBurnQat::from_core(core.input_norm().clone(), device)?,
            input_activation: ActFakeQuantBurnQat::from_core(core.input_activation().clone())?,
            input_to_state: TernaryLinearBurnQat::from_core(core.input_to_state().clone(), device)?,
            state_to_output: TernaryLinearBurnQat::from_core(
                core.state_to_output().clone(),
                device,
            )?,
            output_activation: ActFakeQuantBurnQat::from_core(core.output_activation().clone())?,
        })
    }

    #[must_use]
    pub const fn d_model(&self) -> usize {
        self.d_model
    }

    #[must_use]
    pub const fn state_slots(&self) -> usize {
        self.state_slots
    }

    #[must_use]
    pub fn input_norm(&self) -> &NormApproxBurnQat<B> {
        &self.input_norm
    }

    #[must_use]
    pub fn input_activation(&self) -> &ActFakeQuantBurnQat {
        &self.input_activation
    }

    #[must_use]
    pub fn input_to_state(&self) -> &TernaryLinearBurnQat<B> {
        &self.input_to_state
    }

    #[must_use]
    pub fn state_to_output(&self) -> &TernaryLinearBurnQat<B> {
        &self.state_to_output
    }

    #[must_use]
    pub fn output_activation(&self) -> &ActFakeQuantBurnQat {
        &self.output_activation
    }

    #[must_use]
    pub fn zero_state(&self, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
        BurnFloatTensor::<B, 1>::zeros([self.state_slots], device)
    }

    pub fn set_hardness(
        &mut self,
        expert_qat: QuantHardness,
        activation_qat: QuantHardness,
        norm_qat: QuantHardness,
    ) {
        self.input_norm.set_hardness(norm_qat);
        self.input_activation.set_hardness(activation_qat);
        self.input_to_state.set_hardness(expert_qat);
        self.state_to_output.set_hardness(expert_qat);
        self.output_activation.set_hardness(activation_qat);
    }

    pub fn set_threshold_schedule_progress(&mut self, progress: ThresholdScheduleProgress) {
        self.input_to_state
            .set_threshold_schedule_progress(progress);
        self.state_to_output
            .set_threshold_schedule_progress(progress);
    }

    pub fn forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
        options: LinearStateForwardOptions,
    ) -> Result<LinearStateBurnRun<B>, LinearStateBurnQatError> {
        let input_shape = float_tensor_shape(&input);
        if input_shape[1] != self.d_model {
            return Err(LinearStateBurnQatError::InputLastDimMismatch {
                expected: self.d_model,
                actual: input_shape[1],
                shape: input_shape.to_vec(),
            });
        }

        let state_shape = float_tensor_shape(&initial_state);
        if state_shape[0] != self.state_slots {
            return Err(LinearStateBurnQatError::StateLenMismatch {
                expected: self.state_slots,
                actual: state_shape[0],
            });
        }

        validate_finite_input(&input)?;
        validate_finite_initial_state(&initial_state)?;

        if input_shape[0] == 0 {
            let device = input.device();
            return Ok(LinearStateBurnRun {
                activations: BurnFloatTensor::<B, 2>::zeros([0, self.d_model], &device),
                final_state: initial_state,
            });
        }

        let mut state = initial_state;
        let mut rows = Vec::with_capacity(input_shape[0]);
        for token_index in 0..input_shape[0] {
            let token = input
                .clone()
                .slice([token_index..token_index + 1, 0..self.d_model])
                .reshape([self.d_model]);
            let normed = self.input_norm.forward(token)?;
            let activated = self
                .input_activation
                .fake_quant_forward(normed, options.activation());
            let delta = self
                .input_to_state
                .fake_quant_forward_validated_input(activated)?;
            state = state * STATE_DECAY + delta;

            let projected = self
                .state_to_output
                .fake_quant_forward_validated_input(state.clone())?;
            rows.push(
                self.output_activation
                    .fake_quant_forward(projected, options.activation()),
            );
        }

        Ok(LinearStateBurnRun {
            activations: BurnFloatTensor::<B, 1>::stack::<2>(rows, 0),
            final_state: state,
        })
    }

    pub fn train_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<LinearStateBurnRun<B>, LinearStateBurnQatError> {
        self.forward(input, initial_state, LinearStateForwardOptions::train())
    }

    pub fn eval_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<LinearStateBurnRun<B>, LinearStateBurnQatError> {
        self.forward(input, initial_state, LinearStateForwardOptions::eval())
    }
}

impl<B: BurnBackend> PhaseControlledModel for LinearStateBurnQat<B> {
    fn apply_phase_controls(&mut self, controls: PhaseControls) {
        self.set_hardness(
            controls.expert_qat(),
            controls.activation_qat(),
            controls.norm_qat(),
        );
        self.set_threshold_schedule_progress(
            ThresholdScheduleProgress::new(controls.threshold_schedule_progress().value())
                .unwrap_or_else(|_| ThresholdScheduleProgress::start()),
        );
    }
}

#[derive(Debug)]
pub struct LinearStateBurnRun<B: BurnBackend> {
    activations: BurnFloatTensor<B, 2>,
    final_state: BurnFloatTensor<B, 1>,
}

impl<B: BurnBackend> LinearStateBurnRun<B> {
    #[must_use]
    pub fn activations(&self) -> BurnFloatTensor<B, 2> {
        self.activations.clone()
    }

    #[must_use]
    pub fn final_state(&self) -> BurnFloatTensor<B, 1> {
        self.final_state.clone()
    }

    #[must_use]
    pub fn into_parts(self) -> (BurnFloatTensor<B, 2>, BurnFloatTensor<B, 1>) {
        (self.activations, self.final_state)
    }
}

#[derive(Debug)]
pub enum LinearStateBurnQatError {
    Norm(NormApproxBurnQatError),
    Activation(ActFakeQuantBurnQatError),
    Projection(TernaryLinearBurnQatError),
    TensorRead(BurnAdapterError),
    NonFiniteInput {
        index: usize,
    },
    NonFiniteInitialState {
        slot: usize,
    },
    InputLastDimMismatch {
        expected: usize,
        actual: usize,
        shape: Vec<usize>,
    },
    StateLenMismatch {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for LinearStateBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Norm(error) => write!(f, "linear-state Burn norm failed: {error}"),
            Self::Activation(error) => {
                write!(f, "linear-state Burn activation setup failed: {error}")
            }
            Self::Projection(error) => write!(f, "linear-state Burn projection failed: {error}"),
            Self::TensorRead(error) => {
                write!(f, "linear-state Burn tensor read failed: {error}")
            }
            Self::NonFiniteInput { index } => write!(
                f,
                "linear-state Burn input value at flat index {index} is not finite"
            ),
            Self::NonFiniteInitialState { slot } => write!(
                f,
                "linear-state Burn initial recurrent state slot {slot} is not finite"
            ),
            Self::InputLastDimMismatch {
                expected,
                actual,
                shape,
            } => write!(
                f,
                "linear-state Burn input last dimension mismatch: expected {expected}, got {actual} in shape {shape:?}"
            ),
            Self::StateLenMismatch { expected, actual } => write!(
                f,
                "linear-state Burn state length mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl Error for LinearStateBurnQatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Norm(error) => Some(error),
            Self::Activation(error) => Some(error),
            Self::Projection(error) => Some(error),
            Self::TensorRead(error) => Some(error),
            Self::NonFiniteInput { .. }
            | Self::NonFiniteInitialState { .. }
            | Self::InputLastDimMismatch { .. }
            | Self::StateLenMismatch { .. } => None,
        }
    }
}

impl From<NormApproxBurnQatError> for LinearStateBurnQatError {
    fn from(error: NormApproxBurnQatError) -> Self {
        Self::Norm(error)
    }
}

impl From<ActFakeQuantBurnQatError> for LinearStateBurnQatError {
    fn from(error: ActFakeQuantBurnQatError) -> Self {
        Self::Activation(error)
    }
}

impl From<TernaryLinearBurnQatError> for LinearStateBurnQatError {
    fn from(error: TernaryLinearBurnQatError) -> Self {
        Self::Projection(error)
    }
}

impl From<BurnAdapterError> for LinearStateBurnQatError {
    fn from(error: BurnAdapterError) -> Self {
        Self::TensorRead(error)
    }
}

fn validate_finite_input<B: BurnBackend>(
    input: &BurnFloatTensor<B, 2>,
) -> Result<(), LinearStateBurnQatError> {
    if let Some(index) = float_tensor_into_vec(input.clone().detach())?
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(LinearStateBurnQatError::NonFiniteInput { index });
    }
    Ok(())
}

fn validate_finite_initial_state<B: BurnBackend>(
    state: &BurnFloatTensor<B, 1>,
) -> Result<(), LinearStateBurnQatError> {
    if let Some(slot) = float_tensor_into_vec(state.clone().detach())?
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(LinearStateBurnQatError::NonFiniteInitialState { slot });
    }
    Ok(())
}

#[cfg(test)]
mod gradient {
    use gbf_model::qat::{
        ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams,
        MatrixShape, NormApproxPlan, NormApproxQat, NormClip, Q8_8Scale, TernaryLinearQat,
        TernaryThreshold,
    };
    use gbf_model::sequence::{LinearStateBlockConfig, SequenceActivation, SequenceState};

    use super::*;
    use crate::adapter::burn::{
        BurnDevice, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, float_tensor_from_vec,
        float_tensor_into_vec,
    };

    #[test]
    fn linear_state_gradient_flows_through_recurrent_burn_state() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let layer = LinearStateBurnQat::<B>::from_core(gradient_block(), &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(
            vec![
                1.0, 0.0, //
                0.0, 1.0,
            ],
            [2, 2],
            &device,
        )
        .unwrap()
        .require_grad();
        let initial_state = layer.zero_state(&device).require_grad();

        let run = layer
            .train_forward(input.clone(), initial_state.clone())
            .unwrap();
        let second_token_loss = run.activations().slice([1..2, 0..2]).sum();
        let gradients = second_token_loss.backward();
        let input_grad = float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap();
        let state_grad = float_tensor_into_vec(initial_state.grad(&gradients).unwrap()).unwrap();
        let input_weight_grad = float_tensor_into_vec(
            layer
                .input_to_state()
                .full_precision_weights()
                .grad(&gradients)
                .unwrap(),
        )
        .unwrap();
        let output_weight_grad = float_tensor_into_vec(
            layer
                .state_to_output()
                .full_precision_weights()
                .grad(&gradients)
                .unwrap(),
        )
        .unwrap();

        assert_close_slice(&input_grad, &[0.5, 0.5, 1.0, 1.0], 1.0e-6);
        assert_close_slice(&state_grad, &[0.25, 0.25], 1.0e-6);
        assert_close_slice(&input_weight_grad, &[0.5, 1.0, 0.5, 1.0], 1.0e-6);
        assert_close_slice(&output_weight_grad, &[0.5, 1.0, 0.5, 1.0], 1.0e-6);
    }

    #[test]
    fn linear_state_rejects_non_finite_burn_input_before_recurrence() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = LinearStateBurnQat::<B>::from_core(gradient_block(), &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(vec![1.0, f32::NAN], [1, 2], &device).unwrap();
        let initial_state = layer.zero_state(&device);

        let error = layer.eval_forward(input, initial_state).unwrap_err();

        assert!(matches!(
            error,
            LinearStateBurnQatError::NonFiniteInput { index: 1 }
        ));
    }

    #[test]
    fn linear_state_rejects_non_finite_initial_state_before_recurrence() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = LinearStateBurnQat::<B>::from_core(gradient_block(), &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(vec![0.0, 0.0], [1, 2], &device).unwrap();
        let initial_state =
            float_tensor_from_vec::<B, 1>(vec![0.0, f32::INFINITY], [2], &device).unwrap();

        let error = layer.eval_forward(input, initial_state).unwrap_err();

        assert!(matches!(
            error,
            LinearStateBurnQatError::NonFiniteInitialState { slot: 1 }
        ));
    }

    #[test]
    fn linear_state_burn_forward_matches_scalar_fixed_range_oracle() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let block = gradient_block();
        let layer = LinearStateBurnQat::<B>::from_core(block.clone(), &device).unwrap();
        let input_values = vec![
            1.0, 0.0, //
            0.0, 1.0,
        ];
        let initial_state_values = vec![0.25, -0.5];
        let burn_input =
            float_tensor_from_vec::<B, 2>(input_values.clone(), [2, 2], &device).unwrap();
        let burn_state =
            float_tensor_from_vec::<B, 1>(initial_state_values.clone(), [2], &device).unwrap();
        let mut scalar_state = SequenceState::zeroed(block.spec());
        write_state_values(&mut scalar_state, &initial_state_values);
        let scalar_input = SequenceActivation::new(1, 2, 2, input_values).unwrap();

        let burn = layer.eval_forward(burn_input, burn_state).unwrap();
        let scalar = block
            .forward_with_options(
                scalar_input,
                &mut scalar_state,
                LinearStateForwardOptions::eval(),
            )
            .unwrap();

        assert_close_slice(
            &float_tensor_into_vec(burn.activations()).unwrap(),
            scalar.values(),
            1.0e-6,
        );
        assert_close_slice(
            &float_tensor_into_vec(burn.final_state()).unwrap(),
            &read_state_values(&scalar_state, 2),
            1.0e-6,
        );
    }

    #[test]
    fn linear_state_eval_forward_honors_fixed_range_eval_passthrough() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = LinearStateBurnQat::<B>::from_core(eval_passthrough_block(), &device).unwrap();
        let input = vec![0.26, 0.0];
        let train = layer
            .train_forward(
                float_tensor_from_vec::<B, 2>(input.clone(), [1, 2], &device).unwrap(),
                layer.zero_state(&device),
            )
            .unwrap();
        let eval = layer
            .eval_forward(
                float_tensor_from_vec::<B, 2>(input, [1, 2], &device).unwrap(),
                layer.zero_state(&device),
            )
            .unwrap();

        assert_ne!(
            float_tensor_into_vec(train.activations()).unwrap(),
            float_tensor_into_vec(eval.activations()).unwrap()
        );
    }

    pub(super) fn gradient_block() -> LinearStateBlock {
        let mut block = LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 8).unwrap(),
            identity_norm(),
            activation(),
            ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            activation(),
        )
        .unwrap();
        block.set_hardness(QuantHardness::Off, QuantHardness::Off, QuantHardness::Off);
        block
    }

    pub(super) fn activation() -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-8.0, 8.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
    }

    fn eval_passthrough_block() -> LinearStateBlock {
        let activation = ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt4,
        )
        .unwrap()
        .with_eval_passthrough(true);
        let mut block = LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 8).unwrap(),
            identity_norm(),
            activation.clone(),
            ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            activation,
        )
        .unwrap();
        block.set_hardness(QuantHardness::Off, QuantHardness::Hard, QuantHardness::Off);
        block
    }

    pub(super) fn identity_norm() -> NormApproxQat {
        NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-8.0, 8.0).unwrap(),
            lut: gbf_model::qat::LutSpec::new(-1.0, 1.0, 3).unwrap(),
        })
    }

    pub(super) fn ternary(
        output_rows: usize,
        input_cols: usize,
        weights: Vec<f32>,
    ) -> TernaryLinearQat {
        TernaryLinearQat::new(
            MatrixShape::new(output_rows, input_cols).unwrap(),
            weights,
            None,
            vec![TernaryThreshold::new(0.5).unwrap(); output_rows],
            vec![Q8_8Scale::from_f32(1.0).unwrap(); output_rows],
        )
        .unwrap()
    }

    fn assert_close_slice(actual: &[f32], expected: &[f32], epsilon: f32) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (*actual - *expected).abs() <= epsilon,
                "index {index}: actual {actual}, expected {expected}, epsilon {epsilon}"
            );
        }
    }

    fn write_state_values(state: &mut SequenceState, values: &[f32]) {
        for (chunk, value) in state.bytes_mut().chunks_exact_mut(4).zip(values) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
    }

    fn read_state_values(state: &SequenceState, slots: usize) -> Vec<f32> {
        state
            .bytes()
            .chunks_exact(4)
            .take(slots)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }
}

#[cfg(test)]
mod phase {
    use gbf_model::qat::{
        ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode,
    };
    use gbf_model::sequence::{LinearStateBlock, LinearStateBlockConfig};

    use gbf_model::qat::QatHardnessControl;

    use super::*;
    use crate::adapter::burn::{BurnDevice, BurnNdArrayBackend};
    use crate::logging::TrainingLogEmitter;
    use crate::scheduler::TrainingPhaseScheduler;

    #[test]
    fn linear_state_hardness_controls_reach_burn_boundary() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let mut layer =
            LinearStateBurnQat::<B>::from_core(super::gradient::gradient_block(), &device).unwrap();
        let mut scheduler = TrainingPhaseScheduler::new(
            crate::phase::TrainingPhaseSchedule::default_five_phase(10).unwrap(),
        );
        let emitter = TrainingLogEmitter::new();

        scheduler.apply_step(24, &mut layer, &emitter).unwrap();

        assert_eq!(layer.input_norm().hardness(), QuantHardness::Soft);
        assert_eq!(layer.input_activation().hardness(), QuantHardness::Soft);
        assert_eq!(layer.output_activation().hardness(), QuantHardness::Soft);
        assert_eq!(layer.input_to_state().hardness(), QuantHardness::Hard);
        assert_eq!(layer.state_to_output().hardness(), QuantHardness::Hard);
        let input_progress = layer.input_to_state().threshold_schedule_progress().value();
        assert!(input_progress > 0.44, "{input_progress}");
        assert!(input_progress < 0.45, "{input_progress}");
        assert_eq!(
            input_progress,
            layer
                .state_to_output()
                .threshold_schedule_progress()
                .value()
        );

        scheduler.apply_step(29, &mut layer, &emitter).unwrap();
        assert_eq!(
            layer.input_to_state().threshold_schedule_progress(),
            ThresholdScheduleProgress::complete()
        );

        scheduler.apply_step(30, &mut layer, &emitter).unwrap();
        assert_eq!(layer.input_norm().hardness(), QuantHardness::Hard);
        assert_eq!(layer.input_activation().hardness(), QuantHardness::Hard);
        assert_eq!(layer.output_activation().hardness(), QuantHardness::Hard);
        assert_eq!(layer.input_to_state().hardness(), QuantHardness::Hard);
        assert_eq!(layer.state_to_output().hardness(), QuantHardness::Hard);
    }

    #[test]
    fn linear_state_hardness_rejects_dynamic_activation_range_until_burn_state_owner() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let dynamic_activation = ActFakeQuant::new(
            ActivationRangeMode::Learned(ActivationRange::new(-8.0, 8.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();
        let block = LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 8).unwrap(),
            super::gradient::identity_norm(),
            dynamic_activation,
            super::gradient::ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            super::gradient::ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            super::gradient::activation(),
        )
        .unwrap();

        assert!(matches!(
            LinearStateBurnQat::<B>::from_core(block, &device),
            Err(LinearStateBurnQatError::Activation(
                ActFakeQuantBurnQatError::UnsupportedRangeMode { .. }
            ))
        ));
    }
}
