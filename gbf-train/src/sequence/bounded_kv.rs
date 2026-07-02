//! Burn-backed BoundedKv sequence block adapter.
//!
//! The training adapter keeps the same logical record layout as the scalar
//! byte-backed block: each record is `[valid_flag, tied_payload...]`. Burn state
//! is a flat `f32` tensor so gradients can flow through valid payload slots; the
//! validity flags remain discrete metadata and are validated before mutation.

use std::error::Error;
use std::fmt;

use gbf_model::qat::{QatHardnessControl, QuantHardness};
use gbf_model::sequence::{BoundedKvBlock, BoundedKvForwardOptions};

use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, BurnModule, burn_softmax,
    float_tensor_from_vec, float_tensor_into_vec, float_tensor_shape,
};
use crate::qat::{
    ActFakeQuantBurnQat, ActFakeQuantBurnQatError, NormApproxBurnQat, NormApproxBurnQatError,
    TernaryLinearBurnQat, TernaryLinearBurnQatError, ThresholdScheduleProgress,
};
use crate::scheduler::{PhaseControlledModel, PhaseControls};

const VALID_FLAG_VALUE: f32 = 1.0;
const EMPTY_FLAG_VALUE: f32 = 0.0;

#[derive(BurnModule, Debug)]
pub struct BoundedKvBurnQat<B: BurnBackend> {
    #[module(skip)]
    d_model: usize,
    #[module(skip)]
    max_context: usize,
    #[module(skip)]
    record_slots: usize,
    #[module(skip)]
    payload_slots: usize,
    input_norm: NormApproxBurnQat<B>,
    #[module(skip)]
    input_activation: ActFakeQuantBurnQat,
    query_projection: TernaryLinearBurnQat<B>,
    kv_projection: TernaryLinearBurnQat<B>,
    output_projection: TernaryLinearBurnQat<B>,
    #[module(skip)]
    output_activation: ActFakeQuantBurnQat,
}

impl<B: BurnBackend> BoundedKvBurnQat<B> {
    pub fn from_core(
        core: BoundedKvBlock,
        device: &BurnDevice<B>,
    ) -> Result<Self, BoundedKvBurnQatError> {
        Ok(Self {
            d_model: core.config().d_model(),
            max_context: usize::from(core.config().max_context()),
            record_slots: core.config().record_slots(),
            payload_slots: core.config().tied_kv_payload_slots(),
            input_norm: NormApproxBurnQat::from_core(core.input_norm().clone(), device)?,
            input_activation: ActFakeQuantBurnQat::from_core(core.input_activation().clone())?,
            query_projection: TernaryLinearBurnQat::from_core(
                core.query_projection().clone(),
                device,
            )?,
            kv_projection: TernaryLinearBurnQat::from_core(core.kv_projection().clone(), device)?,
            output_projection: TernaryLinearBurnQat::from_core(
                core.output_projection().clone(),
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
    pub const fn max_context(&self) -> usize {
        self.max_context
    }

    #[must_use]
    pub const fn record_slots(&self) -> usize {
        self.record_slots
    }

    #[must_use]
    pub const fn payload_slots(&self) -> usize {
        self.payload_slots
    }

    #[must_use]
    pub const fn state_slots(&self) -> usize {
        self.max_context * self.record_slots
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
    pub fn query_projection(&self) -> &TernaryLinearBurnQat<B> {
        &self.query_projection
    }

    #[must_use]
    pub fn kv_projection(&self) -> &TernaryLinearBurnQat<B> {
        &self.kv_projection
    }

    #[must_use]
    pub fn output_projection(&self) -> &TernaryLinearBurnQat<B> {
        &self.output_projection
    }

    #[must_use]
    pub fn output_activation(&self) -> &ActFakeQuantBurnQat {
        &self.output_activation
    }

    #[must_use]
    pub fn zero_state(&self, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
        BurnFloatTensor::<B, 1>::zeros([self.state_slots()], device)
    }

    pub fn set_hardness(
        &mut self,
        expert_qat: QuantHardness,
        activation_qat: QuantHardness,
        norm_qat: QuantHardness,
    ) {
        self.input_norm.set_hardness(norm_qat);
        self.input_activation.set_hardness(activation_qat);
        self.query_projection.set_hardness(expert_qat);
        self.kv_projection.set_hardness(expert_qat);
        self.output_projection.set_hardness(expert_qat);
        self.output_activation.set_hardness(activation_qat);
    }

    pub fn set_threshold_schedule_progress(&mut self, progress: ThresholdScheduleProgress) {
        self.query_projection
            .set_threshold_schedule_progress(progress);
        self.kv_projection.set_threshold_schedule_progress(progress);
        self.output_projection
            .set_threshold_schedule_progress(progress);
    }

    pub fn forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
        options: BoundedKvForwardOptions,
    ) -> Result<BoundedKvBurnRun<B>, BoundedKvBurnQatError> {
        let input_shape = float_tensor_shape(&input);
        if input_shape[1] != self.d_model {
            return Err(BoundedKvBurnQatError::InputLastDimMismatch {
                expected: self.d_model,
                actual: input_shape[1],
                shape: input_shape.to_vec(),
            });
        }

        let state_shape = float_tensor_shape(&initial_state);
        if state_shape[0] != self.state_slots() {
            return Err(BoundedKvBurnQatError::StateLenMismatch {
                expected: self.state_slots(),
                actual: state_shape[0],
            });
        }

        validate_finite_input(&input)?;
        let valid_records = validate_state_layout(
            &initial_state,
            self.max_context,
            self.record_slots,
            self.payload_slots,
        )?;

        if input_shape[0] == 0 {
            let device = input.device();
            return Ok(BoundedKvBurnRun {
                activations: BurnFloatTensor::<B, 2>::zeros([0, self.d_model], &device),
                final_state: initial_state,
            });
        }

        let mut records = self.records_from_state(initial_state, valid_records);
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
            let query = self
                .query_projection
                .fake_quant_forward_validated_input(activated.clone())?;
            let kv_payload = self
                .kv_projection
                .fake_quant_forward_validated_input(activated)?;

            append_record(&mut records, kv_payload);
            let attended = attend_records(&records, query)?;
            let projected = self
                .output_projection
                .fake_quant_forward_validated_input(attended)?;
            rows.push(
                self.output_activation
                    .fake_quant_forward(projected, options.activation()),
            );
        }

        Ok(BoundedKvBurnRun {
            activations: BurnFloatTensor::<B, 1>::stack::<2>(rows, 0),
            final_state: stack_records(records, input.device())?,
        })
    }

    pub fn train_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<BoundedKvBurnRun<B>, BoundedKvBurnQatError> {
        self.forward(input, initial_state, BoundedKvForwardOptions::train())
    }

    pub fn eval_forward(
        &self,
        input: BurnFloatTensor<B, 2>,
        initial_state: BurnFloatTensor<B, 1>,
    ) -> Result<BoundedKvBurnRun<B>, BoundedKvBurnQatError> {
        self.forward(input, initial_state, BoundedKvForwardOptions::eval())
    }

    #[allow(clippy::single_range_in_vec_init)]
    fn records_from_state(
        &self,
        state: BurnFloatTensor<B, 1>,
        valid_records: Vec<bool>,
    ) -> Vec<BurnKvRecord<B>> {
        valid_records
            .into_iter()
            .enumerate()
            .map(|(record, valid)| {
                let start = record * self.record_slots + 1;
                BurnKvRecord {
                    valid,
                    payload: state
                        .clone()
                        .slice([start..start + self.payload_slots])
                        .reshape([self.payload_slots]),
                }
            })
            .collect()
    }
}

impl<B: BurnBackend> PhaseControlledModel for BoundedKvBurnQat<B> {
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
pub struct BoundedKvBurnRun<B: BurnBackend> {
    activations: BurnFloatTensor<B, 2>,
    final_state: BurnFloatTensor<B, 1>,
}

impl<B: BurnBackend> BoundedKvBurnRun<B> {
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
pub enum BoundedKvBurnQatError {
    Norm(NormApproxBurnQatError),
    Activation(ActFakeQuantBurnQatError),
    Projection(TernaryLinearBurnQatError),
    TensorRead(BurnAdapterError),
    NonFiniteInput {
        index: usize,
    },
    NonFiniteState {
        record: usize,
        slot: usize,
    },
    NonCanonicalState {
        record: usize,
        reason: &'static str,
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
    EmptyContext,
}

impl fmt::Display for BoundedKvBurnQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Norm(error) => write!(f, "bounded-kv Burn norm failed: {error}"),
            Self::Activation(error) => {
                write!(f, "bounded-kv Burn activation setup failed: {error}")
            }
            Self::Projection(error) => write!(f, "bounded-kv Burn projection failed: {error}"),
            Self::TensorRead(error) => {
                write!(f, "bounded-kv Burn tensor read failed: {error}")
            }
            Self::NonFiniteInput { index } => write!(
                f,
                "bounded-kv Burn input value at flat index {index} is not finite"
            ),
            Self::NonFiniteState { record, slot } => write!(
                f,
                "bounded-kv Burn state record {record} slot {slot} is not finite"
            ),
            Self::NonCanonicalState { record, reason } => write!(
                f,
                "bounded-kv Burn state record {record} is not canonical: {reason}"
            ),
            Self::InputLastDimMismatch {
                expected,
                actual,
                shape,
            } => write!(
                f,
                "bounded-kv Burn input last dimension mismatch: expected {expected}, got {actual} in shape {shape:?}"
            ),
            Self::StateLenMismatch { expected, actual } => write!(
                f,
                "bounded-kv Burn state length mismatch: expected {expected}, got {actual}"
            ),
            Self::EmptyContext => f.write_str("bounded-kv Burn attention context is empty"),
        }
    }
}

impl Error for BoundedKvBurnQatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Norm(error) => Some(error),
            Self::Activation(error) => Some(error),
            Self::Projection(error) => Some(error),
            Self::TensorRead(error) => Some(error),
            Self::NonFiniteInput { .. }
            | Self::NonFiniteState { .. }
            | Self::NonCanonicalState { .. }
            | Self::InputLastDimMismatch { .. }
            | Self::StateLenMismatch { .. }
            | Self::EmptyContext => None,
        }
    }
}

impl From<NormApproxBurnQatError> for BoundedKvBurnQatError {
    fn from(error: NormApproxBurnQatError) -> Self {
        Self::Norm(error)
    }
}

impl From<ActFakeQuantBurnQatError> for BoundedKvBurnQatError {
    fn from(error: ActFakeQuantBurnQatError) -> Self {
        Self::Activation(error)
    }
}

impl From<TernaryLinearBurnQatError> for BoundedKvBurnQatError {
    fn from(error: TernaryLinearBurnQatError) -> Self {
        Self::Projection(error)
    }
}

impl From<BurnAdapterError> for BoundedKvBurnQatError {
    fn from(error: BurnAdapterError) -> Self {
        Self::TensorRead(error)
    }
}

#[derive(Debug)]
struct BurnKvRecord<B: BurnBackend> {
    valid: bool,
    payload: BurnFloatTensor<B, 1>,
}

fn validate_finite_input<B: BurnBackend>(
    input: &BurnFloatTensor<B, 2>,
) -> Result<(), BoundedKvBurnQatError> {
    if let Some(index) = float_tensor_into_vec(input.clone().detach())?
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(BoundedKvBurnQatError::NonFiniteInput { index });
    }
    Ok(())
}

fn validate_state_layout<B: BurnBackend>(
    state: &BurnFloatTensor<B, 1>,
    max_context: usize,
    record_slots: usize,
    payload_slots: usize,
) -> Result<Vec<bool>, BoundedKvBurnQatError> {
    debug_assert_eq!(record_slots, payload_slots + 1);
    let values = float_tensor_into_vec(state.clone().detach())?;
    let mut valid_records = Vec::with_capacity(max_context);
    let mut seen_empty_record = false;

    for (record_index, record) in values.chunks_exact(record_slots).enumerate() {
        if let Some(slot) = record.iter().position(|value| !value.is_finite()) {
            return Err(BoundedKvBurnQatError::NonFiniteState {
                record: record_index,
                slot,
            });
        }
        let flag = record[0];
        if flag != EMPTY_FLAG_VALUE && flag != VALID_FLAG_VALUE {
            return Err(BoundedKvBurnQatError::NonCanonicalState {
                record: record_index,
                reason: "valid flag must be exactly 0.0 or 1.0",
            });
        }
        let valid = flag == VALID_FLAG_VALUE;
        if valid && seen_empty_record {
            return Err(BoundedKvBurnQatError::NonCanonicalState {
                record: record_index,
                reason: "valid records must be contiguous",
            });
        }
        if !valid && record[1..].iter().any(|value| *value != 0.0) {
            return Err(BoundedKvBurnQatError::NonCanonicalState {
                record: record_index,
                reason: "empty records must have zero payload",
            });
        }
        if !valid {
            seen_empty_record = true;
        }
        valid_records.push(valid);
    }

    debug_assert_eq!(valid_records.len(), max_context);
    Ok(valid_records)
}

fn append_record<B: BurnBackend>(records: &mut [BurnKvRecord<B>], payload: BurnFloatTensor<B, 1>) {
    if let Some(record) = records.iter_mut().find(|record| !record.valid) {
        *record = BurnKvRecord {
            valid: true,
            payload,
        };
        return;
    }

    records.rotate_left(1);
    if let Some(record) = records.last_mut() {
        *record = BurnKvRecord {
            valid: true,
            payload,
        };
    }
}

#[allow(clippy::single_range_in_vec_init)]
fn attend_records<B: BurnBackend>(
    records: &[BurnKvRecord<B>],
    query: BurnFloatTensor<B, 1>,
) -> Result<BurnFloatTensor<B, 1>, BoundedKvBurnQatError> {
    let valid_records = records
        .iter()
        .filter(|record| record.valid)
        .collect::<Vec<_>>();
    if valid_records.is_empty() {
        return Err(BoundedKvBurnQatError::EmptyContext);
    }

    let scale = (query.shape().as_slice()[0] as f32).sqrt().max(1.0);
    let logits = valid_records
        .iter()
        .map(|record| (query.clone() * record.payload.clone()).sum() / scale)
        .collect::<Vec<_>>();
    let weights = burn_softmax(
        BurnFloatTensor::<B, 1>::stack::<2>(logits, 0).reshape([valid_records.len()]),
        0,
    );
    let mut attended = query.zeros_like();
    for (index, record) in valid_records.iter().enumerate() {
        let weight = weights
            .clone()
            .slice([index..index + 1])
            .expand(record.payload.shape());
        attended = attended + record.payload.clone() * weight;
    }

    Ok(attended)
}

#[allow(clippy::single_range_in_vec_init)]
fn stack_records<B: BurnBackend>(
    records: Vec<BurnKvRecord<B>>,
    device: BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, BoundedKvBurnQatError> {
    let total_slots = records
        .iter()
        .map(|record| 1 + record.payload.shape().as_slice()[0])
        .sum::<usize>();
    let mut slots = Vec::new();
    for record in records {
        let flag = if record.valid {
            VALID_FLAG_VALUE
        } else {
            EMPTY_FLAG_VALUE
        };
        slots.push(float_tensor_from_vec(vec![flag], [1], &device)?);
        for slot in 0..record.payload.shape().as_slice()[0] {
            slots.push(record.payload.clone().slice([slot..slot + 1]));
        }
    }

    Ok(BurnFloatTensor::<B, 1>::stack::<2>(slots, 0).reshape([total_slots]))
}

#[cfg(test)]
mod gradient {
    use gbf_model::qat::{
        ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams,
        MatrixShape, NormApproxPlan, NormApproxQat, NormClip, Q8_8Scale, TernaryLinearQat,
        TernaryThreshold,
    };
    use gbf_model::sequence::{BoundedKvBlockConfig, SequenceActivation, SequenceState};

    use super::*;
    use crate::adapter::burn::{
        BurnDevice, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, float_tensor_from_vec,
        float_tensor_into_vec,
    };

    #[test]
    fn bounded_kv_gradient_flows_through_bounded_attention() {
        type B = BurnNdArrayAutodiffBackend;

        let device = BurnDevice::<B>::default();
        let layer = BoundedKvBurnQat::<B>::from_core(gradient_block(), &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(vec![1.0, 0.0], [1, 2], &device)
            .unwrap()
            .require_grad();
        let initial_state = float_tensor_from_vec::<B, 1>(
            vec![
                1.0, 0.5, -0.25, //
                0.0, 0.0, 0.0,
            ],
            [6],
            &device,
        )
        .unwrap()
        .require_grad();

        let run = layer
            .train_forward(input.clone(), initial_state.clone())
            .unwrap();
        let loss = run.activations().sum();
        let gradients = loss.backward();
        let input_grad = float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap();
        let state_grad = float_tensor_into_vec(initial_state.grad(&gradients).unwrap()).unwrap();
        let query_grad = float_tensor_into_vec(
            layer
                .query_projection()
                .full_precision_weights()
                .grad(&gradients)
                .unwrap(),
        )
        .unwrap();
        let kv_grad = float_tensor_into_vec(
            layer
                .kv_projection()
                .full_precision_weights()
                .grad(&gradients)
                .unwrap(),
        )
        .unwrap();
        let output_grad = float_tensor_into_vec(
            layer
                .output_projection()
                .full_precision_weights()
                .grad(&gradients)
                .unwrap(),
        )
        .unwrap();

        assert_any_nonzero("input", &input_grad);
        assert_any_nonzero("initial_state_payload", &state_grad[1..3]);
        assert_any_nonzero("query_projection", &query_grad);
        assert_any_nonzero("kv_projection", &kv_grad);
        assert_any_nonzero("output_projection", &output_grad);
    }

    #[test]
    fn bounded_kv_burn_forward_matches_scalar_attention_oracle() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let block = gradient_block();
        let layer = BoundedKvBurnQat::<B>::from_core(block.clone(), &device).unwrap();
        let input_values = vec![
            1.0, 0.0, //
            0.0, 1.0,
        ];
        let burn_input =
            float_tensor_from_vec::<B, 2>(input_values.clone(), [2, 2], &device).unwrap();
        let burn_state = float_tensor_from_vec::<B, 1>(
            vec![
                1.0, 0.5, -0.25, //
                0.0, 0.0, 0.0,
            ],
            [6],
            &device,
        )
        .unwrap();
        let mut scalar_state = SequenceState::zeroed(block.spec());
        write_scalar_record(&mut scalar_state, 0, &[0.5, -0.25]);
        let scalar_input = SequenceActivation::new(1, 2, 2, input_values).unwrap();

        let burn = layer.eval_forward(burn_input, burn_state).unwrap();
        let scalar = block
            .forward_with_options(
                scalar_input,
                &mut scalar_state,
                BoundedKvForwardOptions::eval(),
            )
            .unwrap();

        assert_close_slice(
            &float_tensor_into_vec(burn.activations()).unwrap(),
            scalar.values(),
            1.0e-6,
        );
        assert_close_slice(
            &float_tensor_into_vec(burn.final_state()).unwrap(),
            &read_scalar_state(&scalar_state),
            1.0e-6,
        );
    }

    #[test]
    fn bounded_kv_burn_rejects_non_canonical_state_layout() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = BoundedKvBurnQat::<B>::from_core(gradient_block(), &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(vec![1.0, 0.0], [1, 2], &device).unwrap();
        let gap_state = float_tensor_from_vec::<B, 1>(
            vec![
                1.0, 0.5, -0.25, //
                0.0, 0.0, 0.0, //
                1.0, 0.25, 0.5,
            ],
            [9],
            &device,
        )
        .unwrap();
        let three_record_layer =
            BoundedKvBurnQat::<B>::from_core(three_record_block(), &device).unwrap();

        assert!(matches!(
            three_record_layer.eval_forward(input.clone(), gap_state),
            Err(BoundedKvBurnQatError::NonCanonicalState {
                record: 2,
                reason: "valid records must be contiguous",
            })
        ));

        let bad_flag_state = float_tensor_from_vec::<B, 1>(
            vec![
                0.75, 0.0, 0.0, //
                0.0, 0.0, 0.0,
            ],
            [6],
            &device,
        )
        .unwrap();
        assert!(matches!(
            layer.eval_forward(input.clone(), bad_flag_state),
            Err(BoundedKvBurnQatError::NonCanonicalState {
                record: 0,
                reason: "valid flag must be exactly 0.0 or 1.0",
            })
        ));

        let dirty_empty_state = float_tensor_from_vec::<B, 1>(
            vec![
                0.0, 0.0, 0.25, //
                0.0, 0.0, 0.0,
            ],
            [6],
            &device,
        )
        .unwrap();
        assert!(matches!(
            layer.eval_forward(input, dirty_empty_state),
            Err(BoundedKvBurnQatError::NonCanonicalState {
                record: 0,
                reason: "empty records must have zero payload",
            })
        ));
    }

    #[test]
    fn bounded_kv_burn_rejects_shape_and_finite_boundary_errors() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = BoundedKvBurnQat::<B>::from_core(gradient_block(), &device).unwrap();
        let wrong_input =
            float_tensor_from_vec::<B, 2>(vec![1.0, 0.0, 0.0], [1, 3], &device).unwrap();

        assert!(matches!(
            layer.eval_forward(wrong_input, layer.zero_state(&device)),
            Err(BoundedKvBurnQatError::InputLastDimMismatch {
                expected: 2,
                actual: 3,
                ..
            })
        ));

        let short_state = float_tensor_from_vec::<B, 1>(vec![0.0; 5], [5], &device).unwrap();
        let input = float_tensor_from_vec::<B, 2>(vec![1.0, 0.0], [1, 2], &device).unwrap();
        assert!(matches!(
            layer.eval_forward(input.clone(), short_state),
            Err(BoundedKvBurnQatError::StateLenMismatch {
                expected: 6,
                actual: 5,
            })
        ));

        let nan_input =
            float_tensor_from_vec::<B, 2>(vec![1.0, f32::NAN], [1, 2], &device).unwrap();
        assert!(matches!(
            layer.eval_forward(nan_input, layer.zero_state(&device)),
            Err(BoundedKvBurnQatError::NonFiniteInput { index: 1 })
        ));

        let nonfinite_state = float_tensor_from_vec::<B, 1>(
            vec![1.0, f32::INFINITY, 0.0, 0.0, 0.0, 0.0],
            [6],
            &device,
        )
        .unwrap();
        assert!(matches!(
            layer.eval_forward(input, nonfinite_state),
            Err(BoundedKvBurnQatError::NonFiniteState { record: 0, slot: 1 })
        ));
    }

    #[test]
    fn bounded_kv_burn_eval_forward_honors_activation_passthrough() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let layer = BoundedKvBurnQat::<B>::from_core(eval_passthrough_block(), &device).unwrap();
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

    pub(super) fn gradient_block() -> BoundedKvBlock {
        let mut block = BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 2, 12).unwrap(),
            identity_norm(),
            activation(false),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            activation(false),
        )
        .unwrap();
        block.set_hardness(QuantHardness::Off, QuantHardness::Off, QuantHardness::Off);
        block
    }

    fn three_record_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 3, 12).unwrap(),
            identity_norm(),
            activation(false),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            activation(false),
        )
        .unwrap()
    }

    fn eval_passthrough_block() -> BoundedKvBlock {
        let mut block = BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 2, 12).unwrap(),
            identity_norm(),
            activation(true),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            activation(true),
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

    pub(super) fn activation(eval_passthrough: bool) -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-8.0, 8.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
        .with_eval_passthrough(eval_passthrough)
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

    fn write_scalar_record(state: &mut SequenceState, record: usize, payload: &[f32]) {
        let record_slots = payload.len() + 1;
        let start = record * record_slots * 4;
        state.bytes_mut()[start..start + 4].copy_from_slice(&VALID_FLAG_VALUE.to_le_bytes());
        for (slot, value) in payload.iter().enumerate() {
            let offset = start + (slot + 1) * 4;
            state.bytes_mut()[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    fn read_scalar_state(state: &SequenceState) -> Vec<f32> {
        state
            .bytes()
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }

    fn assert_any_nonzero(label: &str, values: &[f32]) {
        assert!(
            values
                .iter()
                .any(|value| value.is_finite() && value.abs() > 1.0e-6),
            "{label} gradient should contain at least one nonzero finite value: {values:?}"
        );
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
}

#[cfg(test)]
mod phase {
    use gbf_model::qat::{
        ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode,
    };
    use gbf_model::sequence::{BoundedKvBlock, BoundedKvBlockConfig};

    use super::*;
    use crate::adapter::burn::{BurnDevice, BurnNdArrayBackend};
    use crate::logging::TrainingLogEmitter;
    use crate::scheduler::TrainingPhaseScheduler;

    #[test]
    fn bounded_kv_hardness_controls_reach_burn_boundary() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let mut layer =
            BoundedKvBurnQat::<B>::from_core(super::gradient::gradient_block(), &device).unwrap();
        let mut scheduler = TrainingPhaseScheduler::new(
            crate::phase::TrainingPhaseSchedule::default_five_phase(10).unwrap(),
        );
        let emitter = TrainingLogEmitter::new();

        scheduler.apply_step(24, &mut layer, &emitter).unwrap();

        assert_eq!(layer.input_norm().hardness(), QuantHardness::Soft);
        assert_eq!(layer.input_activation().hardness(), QuantHardness::Soft);
        assert_eq!(layer.output_activation().hardness(), QuantHardness::Soft);
        assert_eq!(layer.query_projection().hardness(), QuantHardness::Hard);
        assert_eq!(layer.kv_projection().hardness(), QuantHardness::Hard);
        assert_eq!(layer.output_projection().hardness(), QuantHardness::Hard);
        let query_progress = layer
            .query_projection()
            .threshold_schedule_progress()
            .value();
        assert!(query_progress > 0.44, "{query_progress}");
        assert!(query_progress < 0.45, "{query_progress}");
        assert_eq!(
            query_progress,
            layer.kv_projection().threshold_schedule_progress().value()
        );
        assert_eq!(
            query_progress,
            layer
                .output_projection()
                .threshold_schedule_progress()
                .value()
        );

        scheduler.apply_step(30, &mut layer, &emitter).unwrap();
        assert_eq!(layer.input_norm().hardness(), QuantHardness::Hard);
        assert_eq!(layer.input_activation().hardness(), QuantHardness::Hard);
        assert_eq!(layer.output_activation().hardness(), QuantHardness::Hard);
        assert_eq!(layer.query_projection().hardness(), QuantHardness::Hard);
        assert_eq!(layer.kv_projection().hardness(), QuantHardness::Hard);
        assert_eq!(layer.output_projection().hardness(), QuantHardness::Hard);
    }

    #[test]
    fn bounded_kv_hardness_rejects_dynamic_activation_range_until_burn_state_owner() {
        type B = BurnNdArrayBackend;

        let device = BurnDevice::<B>::default();
        let dynamic_activation = ActFakeQuant::new(
            ActivationRangeMode::Learned(ActivationRange::new(-8.0, 8.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap();
        let block = BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 2, 12).unwrap(),
            super::gradient::identity_norm(),
            dynamic_activation,
            super::gradient::ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            super::gradient::ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            super::gradient::ternary(
                2,
                2,
                vec![
                    1.0, 0.0, //
                    0.0, 1.0,
                ],
            ),
            super::gradient::activation(false),
        )
        .unwrap();

        assert!(matches!(
            BoundedKvBurnQat::<B>::from_core(block, &device),
            Err(BoundedKvBurnQatError::Activation(
                ActFakeQuantBurnQatError::UnsupportedRangeMode { .. }
            ))
        ));
    }
}
