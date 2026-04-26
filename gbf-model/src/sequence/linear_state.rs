//! Linear-state sequence block over the backend-independent QAT kernels.

use std::error::Error;
use std::fmt;

use crate::qat::{
    ActFakeQuant, ActFakeQuantError, ActivationForwardMode, MatrixShape, NormApproxError,
    NormApproxPlan, NormApproxQat, TernaryLinearQat, TernaryLinearQatError,
};
use crate::sequence::{
    SequenceActivation, SequenceActivationError, SequenceBlock, SequenceExportFacts,
    SequenceSemanticsSpec, SequenceState, SequenceStateSize,
};

const STATE_SLOT_BYTES: usize = core::mem::size_of::<f32>();
const STATE_DECAY: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearStateBlockConfig {
    d_model: usize,
    state_bytes_per_layer: u16,
    state_slots: usize,
}

impl LinearStateBlockConfig {
    pub fn new(d_model: usize, state_bytes_per_layer: u16) -> Result<Self, LinearStateBlockError> {
        if d_model == 0 {
            return Err(LinearStateBlockError::ZeroDim { field: "d_model" });
        }
        if state_bytes_per_layer == 0 {
            return Err(LinearStateBlockError::ZeroDim {
                field: "state_bytes_per_layer",
            });
        }
        if !usize::from(state_bytes_per_layer).is_multiple_of(STATE_SLOT_BYTES) {
            return Err(LinearStateBlockError::UnalignedStateBytes {
                state_bytes_per_layer,
                slot_bytes: STATE_SLOT_BYTES,
            });
        }

        Ok(Self {
            d_model,
            state_bytes_per_layer,
            state_slots: usize::from(state_bytes_per_layer) / STATE_SLOT_BYTES,
        })
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn state_bytes_per_layer(&self) -> u16 {
        self.state_bytes_per_layer
    }

    pub fn state_slots(&self) -> usize {
        self.state_slots
    }

    fn spec(&self) -> SequenceSemanticsSpec {
        SequenceSemanticsSpec::linear_state(self.state_bytes_per_layer)
            .expect("validated linear-state byte size must construct sequence semantics")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinearStateForwardOptions {
    activation: ActivationForwardMode,
}

impl LinearStateForwardOptions {
    pub fn train() -> Self {
        Self {
            activation: ActivationForwardMode::Train,
        }
    }

    pub fn eval() -> Self {
        Self {
            activation: ActivationForwardMode::Eval,
        }
    }

    pub fn activation(self) -> ActivationForwardMode {
        self.activation
    }

    pub fn with_activation(mut self, activation: ActivationForwardMode) -> Self {
        self.activation = activation;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearStateBlock {
    config: LinearStateBlockConfig,
    input_norm: NormApproxQat,
    input_activation: ActFakeQuant,
    input_to_state: TernaryLinearQat,
    state_to_output: TernaryLinearQat,
    output_activation: ActFakeQuant,
}

impl LinearStateBlock {
    pub fn new(
        config: LinearStateBlockConfig,
        input_norm: NormApproxQat,
        input_activation: ActFakeQuant,
        input_to_state: TernaryLinearQat,
        state_to_output: TernaryLinearQat,
        output_activation: ActFakeQuant,
    ) -> Result<Self, LinearStateBlockError> {
        validate_norm_compatibility(input_norm.plan(), config.d_model())?;
        validate_projection_shape(
            "input_to_state",
            input_to_state.shape(),
            config.state_slots(),
            config.d_model(),
        )?;
        validate_projection_shape(
            "state_to_output",
            state_to_output.shape(),
            config.d_model(),
            config.state_slots(),
        )?;

        Ok(Self {
            config,
            input_norm,
            input_activation,
            input_to_state,
            state_to_output,
            output_activation,
        })
    }

    pub fn config(&self) -> &LinearStateBlockConfig {
        &self.config
    }

    pub fn input_norm(&self) -> &NormApproxQat {
        &self.input_norm
    }

    pub fn input_activation(&self) -> &ActFakeQuant {
        &self.input_activation
    }

    pub fn input_to_state(&self) -> &TernaryLinearQat {
        &self.input_to_state
    }

    pub fn state_to_output(&self) -> &TernaryLinearQat {
        &self.state_to_output
    }

    pub fn output_activation(&self) -> &ActFakeQuant {
        &self.output_activation
    }

    pub fn spec(&self) -> SequenceSemanticsSpec {
        self.config.spec()
    }

    pub fn forward_with_options(
        &self,
        input: SequenceActivation,
        state: &mut SequenceState,
        options: LinearStateForwardOptions,
    ) -> Result<SequenceActivation, LinearStateBlockError> {
        self.validate_input_and_state(&input, state)?;

        let mut state_values = read_state_values(state.bytes(), self.config.state_slots())?;
        let mut output_values = Vec::with_capacity(input.values().len());

        for token in input.values().chunks_exact(self.config.d_model()) {
            let normed = self
                .input_norm
                .forward(token)
                .map_err(LinearStateBlockError::InputNorm)?;
            let activated = self
                .input_activation
                .inference_forward(&normed, options.activation())
                .map_err(LinearStateBlockError::InputActivation)?;
            let delta = self
                .input_to_state
                .inference_forward(&activated)
                .map_err(LinearStateBlockError::InputToState)?;
            apply_state_update(&mut state_values, &delta)?;

            let projected = self
                .state_to_output
                .inference_forward(&state_values)
                .map_err(LinearStateBlockError::StateToOutput)?;
            let quantized = self
                .output_activation
                .inference_forward(&projected, options.activation())
                .map_err(LinearStateBlockError::OutputActivation)?;
            output_values.extend(quantized);
        }

        write_state_values(&state_values, state.bytes_mut());
        SequenceActivation::new(
            input.batch(),
            input.tokens(),
            self.config.d_model(),
            output_values,
        )
        .map_err(LinearStateBlockError::OutputActivationShape)
    }

    fn validate_input_and_state(
        &self,
        input: &SequenceActivation,
        state: &SequenceState,
    ) -> Result<(), LinearStateBlockError> {
        if input.d_model() != self.config.d_model() {
            return Err(LinearStateBlockError::InputDModelMismatch {
                expected: self.config.d_model(),
                actual: input.d_model(),
            });
        }
        if input.batch() != 1 {
            return Err(LinearStateBlockError::UnsupportedBatchSize {
                expected: 1,
                actual: input.batch(),
            });
        }

        let expected_spec = self.spec();
        if state.spec() != expected_spec {
            return Err(LinearStateBlockError::StateSpecMismatch {
                expected: expected_spec,
                actual: state.spec(),
            });
        }

        let expected_len = usize::from(self.config.state_bytes_per_layer());
        if state.bytes().len() != expected_len {
            return Err(LinearStateBlockError::StateLenMismatch {
                expected: expected_len,
                actual: state.bytes().len(),
            });
        }

        Ok(())
    }
}

impl SequenceBlock for LinearStateBlock {
    type Error = LinearStateBlockError;

    fn forward(
        &self,
        input: SequenceActivation,
        state: &mut SequenceState,
    ) -> Result<SequenceActivation, Self::Error> {
        self.forward_with_options(input, state, LinearStateForwardOptions::train())
    }

    fn state_init(&self) -> SequenceState {
        SequenceState::zeroed(self.spec())
    }

    fn state_size(&self) -> SequenceStateSize {
        self.spec().state_size()
    }

    fn export_facts(&self) -> SequenceExportFacts {
        SequenceExportFacts::for_spec(self.spec())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinearStateBlockError {
    ZeroDim {
        field: &'static str,
    },
    UnalignedStateBytes {
        state_bytes_per_layer: u16,
        slot_bytes: usize,
    },
    ProjectionShapeMismatch {
        projection: &'static str,
        expected_output_rows: usize,
        expected_input_cols: usize,
        actual_output_rows: usize,
        actual_input_cols: usize,
    },
    NormTileWidthMismatch {
        d_model: usize,
        tile_width: usize,
    },
    InputDModelMismatch {
        expected: usize,
        actual: usize,
    },
    UnsupportedBatchSize {
        expected: usize,
        actual: usize,
    },
    StateSpecMismatch {
        expected: SequenceSemanticsSpec,
        actual: SequenceSemanticsSpec,
    },
    StateLenMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteState {
        slot: usize,
    },
    ComputedNonFiniteState {
        slot: usize,
    },
    InputNorm(NormApproxError),
    InputActivation(ActFakeQuantError),
    InputToState(TernaryLinearQatError),
    StateToOutput(TernaryLinearQatError),
    OutputActivation(ActFakeQuantError),
    OutputActivationShape(SequenceActivationError),
}

impl fmt::Display for LinearStateBlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDim { field } => write!(f, "{field} must be nonzero"),
            Self::UnalignedStateBytes {
                state_bytes_per_layer,
                slot_bytes,
            } => write!(
                f,
                "linear-state byte budget {state_bytes_per_layer} must be divisible by {slot_bytes}"
            ),
            Self::ProjectionShapeMismatch {
                projection,
                expected_output_rows,
                expected_input_cols,
                actual_output_rows,
                actual_input_cols,
            } => write!(
                f,
                "{projection} shape mismatch: expected {expected_output_rows}x{expected_input_cols}, got {actual_output_rows}x{actual_input_cols}"
            ),
            Self::NormTileWidthMismatch {
                d_model,
                tile_width,
            } => write!(
                f,
                "linear-state norm tile width {tile_width} must divide d_model {d_model}"
            ),
            Self::InputDModelMismatch { expected, actual } => {
                write!(
                    f,
                    "input d_model mismatch: expected {expected}, got {actual}"
                )
            }
            Self::UnsupportedBatchSize { expected, actual } => {
                write!(
                    f,
                    "linear-state scalar block supports batch {expected}, got {actual}"
                )
            }
            Self::StateSpecMismatch { expected, actual } => {
                write!(
                    f,
                    "sequence state spec mismatch: expected {expected:?}, got {actual:?}"
                )
            }
            Self::StateLenMismatch { expected, actual } => {
                write!(
                    f,
                    "sequence state byte length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NonFiniteState { slot } => {
                write!(f, "linear-state slot {slot} is not finite")
            }
            Self::ComputedNonFiniteState { slot } => {
                write!(f, "linear-state update for slot {slot} is not finite")
            }
            Self::InputNorm(err) => write!(f, "linear-state input norm failed: {err}"),
            Self::InputActivation(err) => {
                write!(
                    f,
                    "linear-state input activation quantization failed: {err}"
                )
            }
            Self::InputToState(err) => {
                write!(f, "linear-state input-to-state projection failed: {err}")
            }
            Self::StateToOutput(err) => {
                write!(f, "linear-state state-to-output projection failed: {err}")
            }
            Self::OutputActivation(err) => {
                write!(
                    f,
                    "linear-state output activation quantization failed: {err}"
                )
            }
            Self::OutputActivationShape(err) => {
                write!(f, "linear-state output activation shape failed: {err:?}")
            }
        }
    }
}

impl Error for LinearStateBlockError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InputNorm(err) => Some(err),
            Self::InputActivation(err) => Some(err),
            Self::InputToState(err) => Some(err),
            Self::StateToOutput(err) => Some(err),
            Self::OutputActivation(err) => Some(err),
            _ => None,
        }
    }
}

fn validate_norm_compatibility(
    plan: NormApproxPlan,
    d_model: usize,
) -> Result<(), LinearStateBlockError> {
    if let NormApproxPlan::TileRmsThenAffineClip { tile, .. } = plan {
        let tile_width = tile.tile_width();
        if !d_model.is_multiple_of(tile_width) {
            return Err(LinearStateBlockError::NormTileWidthMismatch {
                d_model,
                tile_width,
            });
        }
    }

    Ok(())
}

fn validate_projection_shape(
    projection: &'static str,
    actual: MatrixShape,
    expected_output_rows: usize,
    expected_input_cols: usize,
) -> Result<(), LinearStateBlockError> {
    if actual.output_rows() != expected_output_rows || actual.input_cols() != expected_input_cols {
        return Err(LinearStateBlockError::ProjectionShapeMismatch {
            projection,
            expected_output_rows,
            expected_input_cols,
            actual_output_rows: actual.output_rows(),
            actual_input_cols: actual.input_cols(),
        });
    }

    Ok(())
}

fn read_state_values(
    bytes: &[u8],
    expected_slots: usize,
) -> Result<Vec<f32>, LinearStateBlockError> {
    let mut values = Vec::with_capacity(expected_slots);
    for (slot, chunk) in bytes.chunks_exact(STATE_SLOT_BYTES).enumerate() {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !value.is_finite() {
            return Err(LinearStateBlockError::NonFiniteState { slot });
        }
        values.push(value);
    }

    debug_assert_eq!(values.len(), expected_slots);
    Ok(values)
}

fn write_state_values(values: &[f32], bytes: &mut [u8]) {
    for (value, chunk) in values.iter().zip(bytes.chunks_exact_mut(STATE_SLOT_BYTES)) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
}

fn apply_state_update(
    state_values: &mut [f32],
    delta: &[f32],
) -> Result<(), LinearStateBlockError> {
    debug_assert_eq!(state_values.len(), delta.len());

    for (slot, (state_value, update)) in state_values.iter_mut().zip(delta.iter()).enumerate() {
        let next = *state_value * STATE_DECAY + *update;
        if !next.is_finite() {
            return Err(LinearStateBlockError::ComputedNonFiniteState { slot });
        }
        *state_value = next;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::qat::{
        ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams, NormApproxPlan,
        NormClip, Q8_8Scale, TernaryThreshold, TileRmsSpec,
    };

    #[test]
    fn linear_state_config_requires_nonzero_float_aligned_state() {
        assert_eq!(
            LinearStateBlockConfig::new(0, 64).unwrap_err(),
            LinearStateBlockError::ZeroDim { field: "d_model" }
        );
        assert_eq!(
            LinearStateBlockConfig::new(2, 0).unwrap_err(),
            LinearStateBlockError::ZeroDim {
                field: "state_bytes_per_layer"
            }
        );
        assert_eq!(
            LinearStateBlockConfig::new(2, 6).unwrap_err(),
            LinearStateBlockError::UnalignedStateBytes {
                state_bytes_per_layer: 6,
                slot_bytes: STATE_SLOT_BYTES,
            }
        );
    }

    #[test]
    fn linear_state_block_reports_fixed_layer_state_size() {
        let block = fixture_block();

        assert_eq!(
            block.state_size(),
            SequenceStateSize {
                bytes_per_layer: 64,
                bytes_per_token: 0,
                fixed_overhead: 0,
            }
        );
        assert_eq!(block.state_init().bytes().len(), 64);
        assert_eq!(
            block.export_facts().spec(),
            SequenceSemanticsSpec::linear_state(64).unwrap()
        );
        assert!(block.export_facts().canonical_tensor_handles().is_empty());
    }

    #[test]
    fn linear_state_forward_updates_fixed_state_and_preserves_activation_shape() {
        let block = fixture_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(1, 2, 2, vec![2.0, 0.0, 0.0, 2.0]).unwrap();

        let first_output = block.forward(input.clone(), &mut state).unwrap();
        let first_state = state.bytes().to_vec();
        let second_output = block.forward(input, &mut state).unwrap();

        assert_eq!(first_output.batch(), 1);
        assert_eq!(first_output.tokens(), 2);
        assert_eq!(first_output.d_model(), 2);
        assert_eq!(first_output.values().len(), 4);
        assert!(first_state.iter().any(|&byte| byte != 0));
        assert_ne!(state.bytes(), first_state.as_slice());
        assert_ne!(second_output.values(), first_output.values());
    }

    #[test]
    fn linear_state_forward_matches_literal_two_token_recurrence_oracle() {
        let block = oracle_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(1, 2, 2, vec![1.0, 0.0, 0.0, 1.0]).unwrap();

        let output = block
            .forward_with_options(input, &mut state, LinearStateForwardOptions::eval())
            .unwrap();

        assert_eq!(output.values(), &[1.0, 0.0, 0.5, 1.0]);
        assert_eq!(test_state_values(&state, 2), vec![0.5, 1.0]);
    }

    #[test]
    fn linear_state_forward_honors_activation_eval_passthrough() {
        let block = fixture_block_with_eval_passthrough();
        let input = SequenceActivation::new(1, 1, 2, vec![0.25, 0.0]).unwrap();
        let mut train_state = block.state_init();
        let mut eval_state = block.state_init();

        let train = block
            .forward_with_options(
                input.clone(),
                &mut train_state,
                LinearStateForwardOptions::train(),
            )
            .unwrap();
        let eval = block
            .forward_with_options(input, &mut eval_state, LinearStateForwardOptions::eval())
            .unwrap();

        assert_ne!(train.values(), eval.values());
    }

    #[test]
    fn linear_state_forward_rejects_wrong_input_or_state_contract() {
        let block = fixture_block();
        let mut state = block.state_init();
        let wrong_input = SequenceActivation::new(1, 1, 3, vec![1.0, 2.0, 3.0]).unwrap();

        assert!(matches!(
            block.forward(wrong_input, &mut state).unwrap_err(),
            LinearStateBlockError::InputDModelMismatch {
                expected: 2,
                actual: 3
            }
        ));

        let mut wrong_state =
            SequenceState::zeroed(SequenceSemanticsSpec::linear_state(32).unwrap());
        let input = SequenceActivation::new(1, 1, 2, vec![1.0, 2.0]).unwrap();
        assert!(matches!(
            block.forward(input, &mut wrong_state).unwrap_err(),
            LinearStateBlockError::StateSpecMismatch { .. }
        ));
    }

    #[test]
    fn linear_state_forward_rejects_multi_batch_until_state_is_batch_shaped() {
        let block = fixture_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(2, 1, 2, vec![1.0, 0.0, 0.0, 1.0]).unwrap();

        assert_eq!(
            block.forward(input, &mut state).unwrap_err(),
            LinearStateBlockError::UnsupportedBatchSize {
                expected: 1,
                actual: 2,
            }
        );
        assert!(state.bytes().iter().all(|&byte| byte == 0));
    }

    #[test]
    fn linear_state_constructor_rejects_projection_shape_mismatch() {
        let config = LinearStateBlockConfig::new(2, 64).unwrap();
        let err = LinearStateBlock::new(
            config,
            test_norm(),
            test_activation(false),
            ternary(15, 2, vec![0.0; 30]),
            ternary(2, 16, vec![0.0; 32]),
            test_activation(false),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            LinearStateBlockError::ProjectionShapeMismatch {
                projection: "input_to_state",
                expected_output_rows: 16,
                expected_input_cols: 2,
                actual_output_rows: 15,
                actual_input_cols: 2,
            }
        ));
    }

    #[test]
    fn linear_state_constructor_rejects_incompatible_norm_tile_width() {
        let err = LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 64).unwrap(),
            NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
                tile: TileRmsSpec::new(3, 1.0).unwrap(),
                affine: AffineParams::new(1.0, 0.0).unwrap(),
                clip: NormClip::new(-8.0, 8.0).unwrap(),
            }),
            test_activation(false),
            input_to_state_projection(),
            state_to_output_projection(),
            test_activation(false),
        )
        .unwrap_err();

        assert_eq!(
            err,
            LinearStateBlockError::NormTileWidthMismatch {
                d_model: 2,
                tile_width: 3,
            }
        );
    }

    #[test]
    fn linear_state_forward_rejects_non_finite_existing_state() {
        let block = fixture_block();
        let mut state = block.state_init();
        state.bytes_mut()[0..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
        let input = SequenceActivation::new(1, 1, 2, vec![1.0, 2.0]).unwrap();

        assert_eq!(
            block.forward(input, &mut state).unwrap_err(),
            LinearStateBlockError::NonFiniteState { slot: 0 }
        );
    }

    #[test]
    fn linear_state_failed_forward_does_not_advance_state_bytes() {
        let block = overflowing_output_block();
        let mut state = block.state_init();
        state.bytes_mut()[0..4].copy_from_slice(&f32::MAX.to_le_bytes());
        let before = state.bytes().to_vec();
        let input = SequenceActivation::new(1, 1, 2, vec![0.0, 0.0]).unwrap();

        assert!(matches!(
            block
                .forward_with_options(input, &mut state, LinearStateForwardOptions::eval())
                .unwrap_err(),
            LinearStateBlockError::OutputActivation(ActFakeQuantError::NonFiniteInput { index: 0 })
        ));
        assert_eq!(state.bytes(), before);
    }

    mod sequence_block {
        use super::*;

        mod linear_state {
            use super::*;

            #[test]
            fn linear_state_implements_sequence_block_trait() {
                fn state_size_from_trait(block: &impl SequenceBlock) -> SequenceStateSize {
                    block.state_size()
                }

                let block = fixture_block();

                assert_eq!(
                    state_size_from_trait(&block),
                    SequenceStateSize {
                        bytes_per_layer: 64,
                        bytes_per_token: 0,
                        fixed_overhead: 0,
                    }
                );
            }
        }
    }

    fn fixture_block() -> LinearStateBlock {
        LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 64).unwrap(),
            test_norm(),
            test_activation(false),
            input_to_state_projection(),
            state_to_output_projection(),
            test_activation(false),
        )
        .unwrap()
    }

    fn oracle_block() -> LinearStateBlock {
        LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 8).unwrap(),
            identity_lut_norm(),
            test_activation(true),
            ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            ternary(2, 2, vec![1.0, 0.0, 0.0, 1.0]),
            test_activation(true),
        )
        .unwrap()
    }

    fn fixture_block_with_eval_passthrough() -> LinearStateBlock {
        LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 64).unwrap(),
            test_norm(),
            test_activation(true),
            input_to_state_projection(),
            state_to_output_projection(),
            test_activation(true),
        )
        .unwrap()
    }

    fn overflowing_output_block() -> LinearStateBlock {
        LinearStateBlock::new(
            LinearStateBlockConfig::new(2, 8).unwrap(),
            identity_lut_norm(),
            test_activation(true),
            ternary(2, 2, vec![0.0; 4]),
            ternary_with_scales(2, 2, vec![1.0, 0.0, 0.0, 1.0], vec![Q8_8Scale::MAX; 2]),
            test_activation(true),
        )
        .unwrap()
    }

    fn test_norm() -> NormApproxQat {
        NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
            tile: TileRmsSpec::new(2, 1.0).unwrap(),
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-8.0, 8.0).unwrap(),
        })
    }

    fn identity_lut_norm() -> NormApproxQat {
        NormApproxQat::new(NormApproxPlan::AffineClipLut {
            affine: AffineParams::new(1.0, 0.0).unwrap(),
            clip: NormClip::new(-8.0, 8.0).unwrap(),
            lut: crate::qat::LutSpec::new(-1.0, 1.0, 3).unwrap(),
        })
    }

    fn test_activation(eval_passthrough: bool) -> ActFakeQuant {
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(-8.0, 8.0).unwrap()),
            ActivationQuantFormat::Int8,
        )
        .unwrap()
        .with_eval_passthrough(eval_passthrough)
    }

    fn input_to_state_projection() -> TernaryLinearQat {
        let mut weights = vec![0.0; 16 * 2];
        weights[0] = 1.0;
        weights[3] = 1.0;
        ternary(16, 2, weights)
    }

    fn state_to_output_projection() -> TernaryLinearQat {
        let mut weights = vec![0.0; 2 * 16];
        weights[0] = 1.0;
        weights[17] = 1.0;
        ternary(2, 16, weights)
    }

    fn ternary(output_rows: usize, input_cols: usize, weights: Vec<f32>) -> TernaryLinearQat {
        ternary_with_scales(
            output_rows,
            input_cols,
            weights,
            vec![Q8_8Scale::from_f32(1.0).unwrap(); output_rows],
        )
    }

    fn ternary_with_scales(
        output_rows: usize,
        input_cols: usize,
        weights: Vec<f32>,
        scales: Vec<Q8_8Scale>,
    ) -> TernaryLinearQat {
        TernaryLinearQat::new(
            MatrixShape::new(output_rows, input_cols).unwrap(),
            weights,
            None,
            vec![TernaryThreshold::new(0.0).unwrap(); output_rows],
            scales,
        )
        .unwrap()
    }

    fn test_state_values(state: &SequenceState, slots: usize) -> Vec<f32> {
        state
            .bytes()
            .chunks_exact(STATE_SLOT_BYTES)
            .take(slots)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }
}
