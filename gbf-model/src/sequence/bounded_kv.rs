//! Bounded causal-KV sequence block over backend-independent QAT kernels.
//!
//! The v1 executable layout uses fixed-width serialized records. Each record's
//! first `f32` slot is a canonical validity flag and the remaining slots are a
//! tied key/value payload. The public `kv_bytes_per_token` value is therefore
//! total record bytes, not payload-only bytes.

use std::error::Error;
use std::fmt;

use crate::qat::{
    ActFakeQuant, ActFakeQuantError, ActivationForwardMode, MatrixShape, NormApproxError,
    NormApproxPlan, NormApproxQat, QatHardnessControl, QuantHardness, TernaryLinearQat,
    TernaryLinearQatError,
};
use crate::sequence::{
    SequenceActivation, SequenceActivationError, SequenceBlock, SequenceExportFacts,
    SequenceSemanticsSpec, SequenceState, SequenceStateSize,
};

const STATE_SLOT_BYTES: usize = core::mem::size_of::<f32>();
const VALID_FLAG_THRESHOLD: f32 = 0.5;
const VALID_FLAG_VALUE: f32 = 1.0;
const EMPTY_FLAG_VALUE: f32 = 0.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedKvBlockConfig {
    d_model: usize,
    max_context: u16,
    kv_bytes_per_token: u16,
    record_slots: usize,
    tied_kv_payload_slots: usize,
}

impl BoundedKvBlockConfig {
    pub fn new(
        d_model: usize,
        max_context: u16,
        kv_bytes_per_token: u16,
    ) -> Result<Self, BoundedKvBlockError> {
        if d_model == 0 {
            return Err(BoundedKvBlockError::ZeroDim { field: "d_model" });
        }
        if max_context == 0 {
            return Err(BoundedKvBlockError::ZeroDim {
                field: "max_context",
            });
        }
        if kv_bytes_per_token == 0 {
            return Err(BoundedKvBlockError::ZeroDim {
                field: "kv_bytes_per_token",
            });
        }
        if !usize::from(kv_bytes_per_token).is_multiple_of(STATE_SLOT_BYTES) {
            return Err(BoundedKvBlockError::UnalignedKvBytes {
                kv_bytes_per_token,
                slot_bytes: STATE_SLOT_BYTES,
            });
        }

        let record_slots = usize::from(kv_bytes_per_token) / STATE_SLOT_BYTES;
        if record_slots < 2 {
            return Err(BoundedKvBlockError::InsufficientKvPayload {
                kv_bytes_per_token,
                min_bytes: (2 * STATE_SLOT_BYTES) as u16,
            });
        }

        Ok(Self {
            d_model,
            max_context,
            kv_bytes_per_token,
            record_slots,
            tied_kv_payload_slots: record_slots - 1,
        })
    }

    pub fn d_model(&self) -> usize {
        self.d_model
    }

    pub fn max_context(&self) -> u16 {
        self.max_context
    }

    pub fn kv_bytes_per_token(&self) -> u16 {
        self.kv_bytes_per_token
    }

    pub fn record_slots(&self) -> usize {
        self.record_slots
    }

    pub fn tied_kv_payload_slots(&self) -> usize {
        self.tied_kv_payload_slots
    }

    fn spec(&self) -> SequenceSemanticsSpec {
        SequenceSemanticsSpec::bounded_kv(self.max_context, self.kv_bytes_per_token)
            .expect("validated bounded-kv dimensions must construct sequence semantics")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedKvForwardOptions {
    activation: ActivationForwardMode,
}

impl BoundedKvForwardOptions {
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
pub struct BoundedKvBlock {
    config: BoundedKvBlockConfig,
    input_norm: NormApproxQat,
    input_activation: ActFakeQuant,
    query_projection: TernaryLinearQat,
    kv_projection: TernaryLinearQat,
    output_projection: TernaryLinearQat,
    output_activation: ActFakeQuant,
}

impl BoundedKvBlock {
    pub fn new(
        config: BoundedKvBlockConfig,
        input_norm: NormApproxQat,
        input_activation: ActFakeQuant,
        query_projection: TernaryLinearQat,
        kv_projection: TernaryLinearQat,
        output_projection: TernaryLinearQat,
        output_activation: ActFakeQuant,
    ) -> Result<Self, BoundedKvBlockError> {
        validate_norm_compatibility(input_norm.plan(), config.d_model())?;
        validate_projection_shape(
            "query_projection",
            query_projection.shape(),
            config.tied_kv_payload_slots(),
            config.d_model(),
        )?;
        validate_projection_shape(
            "kv_projection",
            kv_projection.shape(),
            config.tied_kv_payload_slots(),
            config.d_model(),
        )?;
        validate_projection_shape(
            "output_projection",
            output_projection.shape(),
            config.d_model(),
            config.tied_kv_payload_slots(),
        )?;

        Ok(Self {
            config,
            input_norm,
            input_activation,
            query_projection,
            kv_projection,
            output_projection,
            output_activation,
        })
    }

    pub fn config(&self) -> &BoundedKvBlockConfig {
        &self.config
    }

    pub fn input_norm(&self) -> &NormApproxQat {
        &self.input_norm
    }

    pub fn input_activation(&self) -> &ActFakeQuant {
        &self.input_activation
    }

    pub fn query_projection(&self) -> &TernaryLinearQat {
        &self.query_projection
    }

    pub fn kv_projection(&self) -> &TernaryLinearQat {
        &self.kv_projection
    }

    pub fn output_projection(&self) -> &TernaryLinearQat {
        &self.output_projection
    }

    pub fn output_activation(&self) -> &ActFakeQuant {
        &self.output_activation
    }

    pub fn spec(&self) -> SequenceSemanticsSpec {
        self.config.spec()
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

    pub fn forward_with_options(
        &self,
        input: SequenceActivation,
        state: &mut SequenceState,
        options: BoundedKvForwardOptions,
    ) -> Result<SequenceActivation, BoundedKvBlockError> {
        self.validate_input_and_state(&input, state)?;

        let mut records = read_records(
            state.bytes(),
            self.config.max_context as usize,
            self.config.kv_bytes_per_token as usize,
            self.config.tied_kv_payload_slots(),
        )?;
        let mut output_values = Vec::with_capacity(input.values().len());

        for token in input.values().chunks_exact(self.config.d_model()) {
            let normed = self
                .input_norm
                .forward(token)
                .map_err(BoundedKvBlockError::InputNorm)?;
            let activated = self
                .input_activation
                .inference_forward(&normed, options.activation())
                .map_err(BoundedKvBlockError::InputActivation)?;
            let query = self
                .query_projection
                .inference_forward(&activated)
                .map_err(BoundedKvBlockError::QueryProjection)?;
            validate_finite_slice("query", &query)?;
            let kv_payload = self
                .kv_projection
                .inference_forward(&activated)
                .map_err(BoundedKvBlockError::KvProjection)?;
            validate_finite_slice("kv_payload", &kv_payload)?;

            append_record(&mut records, kv_payload);
            let attended = attend_records(&records, &query)?;
            let projected = self
                .output_projection
                .inference_forward(&attended)
                .map_err(BoundedKvBlockError::OutputProjection)?;
            validate_finite_slice("output_projection", &projected)?;
            let quantized = self
                .output_activation
                .inference_forward(&projected, options.activation())
                .map_err(BoundedKvBlockError::OutputActivation)?;
            output_values.extend(quantized);
        }

        write_records(
            &records,
            state.bytes_mut(),
            self.config.kv_bytes_per_token as usize,
        );
        SequenceActivation::new(
            input.batch(),
            input.tokens(),
            self.config.d_model(),
            output_values,
        )
        .map_err(BoundedKvBlockError::OutputActivationShape)
    }

    fn validate_input_and_state(
        &self,
        input: &SequenceActivation,
        state: &SequenceState,
    ) -> Result<(), BoundedKvBlockError> {
        if input.d_model() != self.config.d_model() {
            return Err(BoundedKvBlockError::InputDModelMismatch {
                expected: self.config.d_model(),
                actual: input.d_model(),
            });
        }
        if input.batch() != 1 {
            return Err(BoundedKvBlockError::UnsupportedBatchSize {
                expected: 1,
                actual: input.batch(),
            });
        }

        let expected_spec = self.spec();
        if state.spec() != expected_spec {
            return Err(BoundedKvBlockError::StateSpecMismatch {
                expected: expected_spec,
                actual: state.spec(),
            });
        }

        let expected_len =
            usize::from(self.config.max_context()) * usize::from(self.config.kv_bytes_per_token());
        if state.bytes().len() != expected_len {
            return Err(BoundedKvBlockError::StateLenMismatch {
                expected: expected_len,
                actual: state.bytes().len(),
            });
        }

        Ok(())
    }
}

impl SequenceBlock for BoundedKvBlock {
    type Error = BoundedKvBlockError;

    fn forward(
        &self,
        input: SequenceActivation,
        state: &mut SequenceState,
    ) -> Result<SequenceActivation, Self::Error> {
        self.forward_with_options(input, state, BoundedKvForwardOptions::train())
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
pub enum BoundedKvBlockError {
    ZeroDim {
        field: &'static str,
    },
    UnalignedKvBytes {
        kv_bytes_per_token: u16,
        slot_bytes: usize,
    },
    InsufficientKvPayload {
        kv_bytes_per_token: u16,
        min_bytes: u16,
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
        record: usize,
        slot: usize,
    },
    NonCanonicalState {
        record: usize,
        reason: &'static str,
    },
    NonFiniteComputed {
        source: &'static str,
        index: usize,
    },
    EmptyContext,
    InputNorm(NormApproxError),
    InputActivation(ActFakeQuantError),
    QueryProjection(TernaryLinearQatError),
    KvProjection(TernaryLinearQatError),
    OutputProjection(TernaryLinearQatError),
    OutputActivation(ActFakeQuantError),
    OutputActivationShape(SequenceActivationError),
}

impl fmt::Display for BoundedKvBlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDim { field } => write!(f, "{field} must be nonzero"),
            Self::UnalignedKvBytes {
                kv_bytes_per_token,
                slot_bytes,
            } => write!(
                f,
                "bounded-kv record byte budget {kv_bytes_per_token} must be divisible by {slot_bytes}"
            ),
            Self::InsufficientKvPayload {
                kv_bytes_per_token,
                min_bytes,
            } => write!(
                f,
                "bounded-kv record byte budget {kv_bytes_per_token} must be at least {min_bytes}"
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
                "bounded-kv norm tile width {tile_width} must divide d_model {d_model}"
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
                    "bounded-kv scalar block supports batch {expected}, got {actual}"
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
            Self::NonFiniteState { record, slot } => {
                write!(
                    f,
                    "bounded-kv state record {record} slot {slot} is not finite"
                )
            }
            Self::NonCanonicalState { record, reason } => {
                write!(
                    f,
                    "bounded-kv state record {record} is not canonical: {reason}"
                )
            }
            Self::NonFiniteComputed { source, index } => {
                write!(
                    f,
                    "bounded-kv computed {source} at index {index} is not finite"
                )
            }
            Self::EmptyContext => f.write_str("bounded-kv attention context is empty"),
            Self::InputNorm(err) => write!(f, "bounded-kv input norm failed: {err}"),
            Self::InputActivation(err) => {
                write!(f, "bounded-kv input activation quantization failed: {err}")
            }
            Self::QueryProjection(err) => {
                write!(f, "bounded-kv query projection failed: {err}")
            }
            Self::KvProjection(err) => write!(f, "bounded-kv KV projection failed: {err}"),
            Self::OutputProjection(err) => {
                write!(f, "bounded-kv output projection failed: {err}")
            }
            Self::OutputActivation(err) => {
                write!(f, "bounded-kv output activation quantization failed: {err}")
            }
            Self::OutputActivationShape(err) => {
                write!(f, "bounded-kv output activation shape failed: {err:?}")
            }
        }
    }
}

impl Error for BoundedKvBlockError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InputNorm(err) => Some(err),
            Self::InputActivation(err) => Some(err),
            Self::QueryProjection(err) => Some(err),
            Self::KvProjection(err) => Some(err),
            Self::OutputProjection(err) => Some(err),
            Self::OutputActivation(err) => Some(err),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct KvRecord {
    valid: bool,
    payload: Vec<f32>,
}

fn validate_norm_compatibility(
    plan: NormApproxPlan,
    d_model: usize,
) -> Result<(), BoundedKvBlockError> {
    if let NormApproxPlan::TileRmsThenAffineClip { tile, .. } = plan {
        let tile_width = tile.tile_width();
        if !d_model.is_multiple_of(tile_width) {
            return Err(BoundedKvBlockError::NormTileWidthMismatch {
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
) -> Result<(), BoundedKvBlockError> {
    if actual.output_rows() != expected_output_rows || actual.input_cols() != expected_input_cols {
        return Err(BoundedKvBlockError::ProjectionShapeMismatch {
            projection,
            expected_output_rows,
            expected_input_cols,
            actual_output_rows: actual.output_rows(),
            actual_input_cols: actual.input_cols(),
        });
    }

    Ok(())
}

fn read_records(
    bytes: &[u8],
    expected_records: usize,
    record_bytes: usize,
    payload_slots: usize,
) -> Result<Vec<KvRecord>, BoundedKvBlockError> {
    let mut records = Vec::with_capacity(expected_records);
    let mut seen_empty_record = false;
    for (record_index, record_bytes) in bytes.chunks_exact(record_bytes).enumerate() {
        let mut slots = record_bytes
            .chunks_exact(STATE_SLOT_BYTES)
            .enumerate()
            .map(|(slot, chunk)| {
                let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if !value.is_finite() {
                    return Err(BoundedKvBlockError::NonFiniteState {
                        record: record_index,
                        slot,
                    });
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let flag = slots[0];
        if flag != EMPTY_FLAG_VALUE && flag != VALID_FLAG_VALUE {
            return Err(BoundedKvBlockError::NonCanonicalState {
                record: record_index,
                reason: "valid flag must be exactly 0.0 or 1.0",
            });
        }
        let valid = flag > VALID_FLAG_THRESHOLD;
        let payload = slots.split_off(1);
        if valid && seen_empty_record {
            return Err(BoundedKvBlockError::NonCanonicalState {
                record: record_index,
                reason: "valid records must be contiguous",
            });
        }
        if !valid && payload.iter().any(|value| *value != 0.0) {
            return Err(BoundedKvBlockError::NonCanonicalState {
                record: record_index,
                reason: "empty records must have zero payload",
            });
        }
        if !valid {
            seen_empty_record = true;
        }
        debug_assert_eq!(payload.len(), payload_slots);
        records.push(KvRecord { valid, payload });
    }

    debug_assert_eq!(records.len(), expected_records);
    Ok(records)
}

fn write_records(records: &[KvRecord], bytes: &mut [u8], record_bytes: usize) {
    for (record, record_bytes) in records.iter().zip(bytes.chunks_exact_mut(record_bytes)) {
        let valid = if record.valid {
            VALID_FLAG_VALUE
        } else {
            EMPTY_FLAG_VALUE
        };
        record_bytes[0..STATE_SLOT_BYTES].copy_from_slice(&valid.to_le_bytes());
        for (value, chunk) in record
            .payload
            .iter()
            .zip(record_bytes[STATE_SLOT_BYTES..].chunks_exact_mut(STATE_SLOT_BYTES))
        {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
    }
}

fn append_record(records: &mut [KvRecord], payload: Vec<f32>) {
    if let Some(record) = records.iter_mut().find(|record| !record.valid) {
        *record = KvRecord {
            valid: true,
            payload,
        };
        return;
    }

    records.rotate_left(1);
    if let Some(record) = records.last_mut() {
        *record = KvRecord {
            valid: true,
            payload,
        };
    }
}

fn attend_records(records: &[KvRecord], query: &[f32]) -> Result<Vec<f32>, BoundedKvBlockError> {
    let valid_records = records
        .iter()
        .filter(|record| record.valid)
        .collect::<Vec<_>>();
    if valid_records.is_empty() {
        return Err(BoundedKvBlockError::EmptyContext);
    }

    let scale = (query.len() as f32).sqrt().max(1.0);
    let logits = valid_records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let dot = query
                .iter()
                .zip(record.payload.iter())
                .map(|(&left, &right)| left * right)
                .sum::<f32>()
                / scale;
            if !dot.is_finite() {
                return Err(BoundedKvBlockError::NonFiniteComputed {
                    source: "attention_logit",
                    index,
                });
            }
            Ok(dot)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let max_logit = logits
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, |left, right| left.max(right));
    let weights = logits
        .iter()
        .enumerate()
        .map(|(index, &logit)| {
            let weight = (logit - max_logit).exp();
            if !weight.is_finite() {
                return Err(BoundedKvBlockError::NonFiniteComputed {
                    source: "attention_weight",
                    index,
                });
            }
            Ok(weight)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let denom = weights.iter().sum::<f32>();
    if !denom.is_finite() || denom == 0.0 {
        return Err(BoundedKvBlockError::NonFiniteComputed {
            source: "attention_denominator",
            index: 0,
        });
    }

    let mut attended = vec![0.0; query.len()];
    for (weight, record) in weights.iter().zip(valid_records.iter()) {
        let normalized = *weight / denom;
        for (slot, value) in attended.iter_mut().zip(record.payload.iter()) {
            *slot += normalized * *value;
        }
    }
    validate_finite_slice("attention_context", &attended)?;

    Ok(attended)
}

fn validate_finite_slice(source: &'static str, values: &[f32]) -> Result<(), BoundedKvBlockError> {
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(BoundedKvBlockError::NonFiniteComputed { source, index });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::qat::{
        ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams, LutSpec,
        NormClip, Q8_8Scale, TernaryThreshold, TileRmsSpec,
    };

    #[test]
    fn bounded_kv_config_requires_nonzero_float_aligned_records() {
        assert_eq!(
            BoundedKvBlockConfig::new(0, 32, 8).unwrap_err(),
            BoundedKvBlockError::ZeroDim { field: "d_model" }
        );
        assert_eq!(
            BoundedKvBlockConfig::new(2, 0, 8).unwrap_err(),
            BoundedKvBlockError::ZeroDim {
                field: "max_context"
            }
        );
        assert_eq!(
            BoundedKvBlockConfig::new(2, 32, 0).unwrap_err(),
            BoundedKvBlockError::ZeroDim {
                field: "kv_bytes_per_token"
            }
        );
        assert_eq!(
            BoundedKvBlockConfig::new(2, 32, 6).unwrap_err(),
            BoundedKvBlockError::UnalignedKvBytes {
                kv_bytes_per_token: 6,
                slot_bytes: STATE_SLOT_BYTES,
            }
        );
        assert_eq!(
            BoundedKvBlockConfig::new(2, 32, 4).unwrap_err(),
            BoundedKvBlockError::InsufficientKvPayload {
                kv_bytes_per_token: 4,
                min_bytes: 8,
            }
        );
    }

    #[test]
    fn bounded_kv_block_reports_bounded_token_growing_state_size() {
        let block = fixture_block();

        assert_eq!(
            block.state_size(),
            SequenceStateSize {
                bytes_per_layer: 256,
                bytes_per_token: 8,
                fixed_overhead: 0,
            }
        );
        assert_eq!(block.state_init().bytes().len(), 256);
        assert_eq!(
            block.export_facts().spec(),
            SequenceSemanticsSpec::bounded_kv(32, 8).unwrap()
        );
        assert!(block.state_size().bytes_per_token > 0);
        assert!(block.export_facts().canonical_tensor_handles().is_empty());
    }

    #[test]
    fn bounded_kv_forward_matches_literal_sliding_window_oracle() {
        let block = oracle_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(1, 3, 1, vec![1.0, 1.0, -1.0]).unwrap();

        let output = block
            .forward_with_options(input, &mut state, BoundedKvForwardOptions::eval())
            .unwrap();
        let records = test_records(&state, 2, 1);

        assert_eq!(output.values(), &[1.0, 1.0, 0.0]);
        assert_eq!(records, vec![(true, vec![1.0]), (true, vec![-1.0])]);
    }

    #[test]
    fn bounded_kv_forward_matches_nonzero_query_attention_oracle() {
        let block = two_slot_attention_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(
            1,
            2,
            2,
            vec![
                1.0, 0.0, //
                0.0, 1.0,
            ],
        )
        .unwrap();

        let output = block
            .forward_with_options(input, &mut state, BoundedKvForwardOptions::eval())
            .unwrap();
        let score = 1.0 / 2.0_f32.sqrt();
        let older_weight = (-score).exp() / ((-score).exp() + 1.0);
        let current_weight = 1.0 / ((-score).exp() + 1.0);

        assert_close(output.values()[0], 1.0);
        assert_close(output.values()[1], 0.0);
        assert_close(output.values()[2], older_weight);
        assert_close(output.values()[3], current_weight);
    }

    #[test]
    fn bounded_kv_forward_truncates_oldest_record_across_calls() {
        let block = oracle_block();
        let mut state = block.state_init();
        let first = SequenceActivation::new(1, 2, 1, vec![1.0, 1.0]).unwrap();
        let second = SequenceActivation::new(1, 1, 1, vec![-1.0]).unwrap();

        block
            .forward_with_options(first, &mut state, BoundedKvForwardOptions::eval())
            .unwrap();
        assert_eq!(
            test_records(&state, 2, 1),
            vec![(true, vec![1.0]), (true, vec![1.0])]
        );

        block
            .forward_with_options(second, &mut state, BoundedKvForwardOptions::eval())
            .unwrap();

        assert_eq!(
            test_records(&state, 2, 1),
            vec![(true, vec![1.0]), (true, vec![-1.0])]
        );
    }

    #[test]
    fn bounded_kv_forward_updates_state_and_preserves_activation_shape() {
        let block = fixture_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(1, 2, 2, vec![2.0, 0.0, 0.0, 2.0]).unwrap();

        let first_output = block.forward(input.clone(), &mut state).unwrap();
        let first_state = state.bytes().to_vec();
        let first_valid_records = valid_record_count(&state, 32);
        let _second_output = block.forward(input, &mut state).unwrap();

        assert_eq!(first_output.batch(), 1);
        assert_eq!(first_output.tokens(), 2);
        assert_eq!(first_output.d_model(), 2);
        assert_eq!(first_output.values().len(), 4);
        assert_eq!(first_valid_records, 2);
        assert_eq!(valid_record_count(&state, 32), 4);
        assert_ne!(state.bytes(), first_state.as_slice());
    }

    #[test]
    fn bounded_kv_forward_honors_activation_eval_passthrough() {
        let block = fixture_block_with_eval_passthrough();
        let input = SequenceActivation::new(1, 1, 2, vec![0.25, 0.0]).unwrap();
        let mut train_state = block.state_init();
        let mut eval_state = block.state_init();

        let train = block
            .forward_with_options(
                input.clone(),
                &mut train_state,
                BoundedKvForwardOptions::train(),
            )
            .unwrap();
        let eval = block
            .forward_with_options(input, &mut eval_state, BoundedKvForwardOptions::eval())
            .unwrap();

        assert_ne!(train.values(), eval.values());
    }

    #[test]
    fn bounded_kv_forward_rejects_wrong_input_or_state_contract() {
        let block = fixture_block();
        let mut state = block.state_init();
        let wrong_input = SequenceActivation::new(1, 1, 3, vec![1.0, 2.0, 3.0]).unwrap();

        assert!(matches!(
            block.forward(wrong_input, &mut state).unwrap_err(),
            BoundedKvBlockError::InputDModelMismatch {
                expected: 2,
                actual: 3
            }
        ));

        let mut wrong_state =
            SequenceState::zeroed(SequenceSemanticsSpec::bounded_kv(16, 8).unwrap());
        let input = SequenceActivation::new(1, 1, 2, vec![1.0, 2.0]).unwrap();
        assert!(matches!(
            block.forward(input, &mut wrong_state).unwrap_err(),
            BoundedKvBlockError::StateSpecMismatch { .. }
        ));
    }

    #[test]
    fn bounded_kv_forward_rejects_multi_batch_until_state_is_batch_shaped() {
        let block = fixture_block();
        let mut state = block.state_init();
        let input = SequenceActivation::new(2, 1, 2, vec![1.0, 0.0, 0.0, 1.0]).unwrap();

        assert_eq!(
            block.forward(input, &mut state).unwrap_err(),
            BoundedKvBlockError::UnsupportedBatchSize {
                expected: 1,
                actual: 2,
            }
        );
        assert!(state.bytes().iter().all(|&byte| byte == 0));
    }

    #[test]
    fn bounded_kv_constructor_rejects_projection_shape_mismatch() {
        let err = BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 32, 8).unwrap(),
            test_norm(),
            test_activation(false),
            ternary(2, 2, vec![0.0; 4]),
            kv_projection(),
            output_projection(),
            test_activation(false),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            BoundedKvBlockError::ProjectionShapeMismatch {
                projection: "query_projection",
                expected_output_rows: 1,
                expected_input_cols: 2,
                actual_output_rows: 2,
                actual_input_cols: 2,
            }
        ));
    }

    #[test]
    fn bounded_kv_constructor_rejects_incompatible_norm_tile_width() {
        let err = BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 32, 8).unwrap(),
            NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip {
                tile: TileRmsSpec::new(3, 1.0).unwrap(),
                affine: AffineParams::new(1.0, 0.0).unwrap(),
                clip: NormClip::new(-8.0, 8.0).unwrap(),
            }),
            test_activation(false),
            query_projection(),
            kv_projection(),
            output_projection(),
            test_activation(false),
        )
        .unwrap_err();

        assert_eq!(
            err,
            BoundedKvBlockError::NormTileWidthMismatch {
                d_model: 2,
                tile_width: 3,
            }
        );
    }

    #[test]
    fn bounded_kv_forward_rejects_non_finite_existing_state() {
        let block = fixture_block();
        let mut state = block.state_init();
        state.bytes_mut()[0..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
        let input = SequenceActivation::new(1, 1, 2, vec![1.0, 2.0]).unwrap();

        assert_eq!(
            block.forward(input, &mut state).unwrap_err(),
            BoundedKvBlockError::NonFiniteState { record: 0, slot: 0 }
        );
    }

    #[test]
    fn bounded_kv_forward_rejects_non_canonical_state_layout() {
        let block = three_record_oracle_block();
        let mut state = block.state_init();
        state.bytes_mut()[0..4].copy_from_slice(&VALID_FLAG_VALUE.to_le_bytes());
        state.bytes_mut()[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        state.bytes_mut()[16..20].copy_from_slice(&VALID_FLAG_VALUE.to_le_bytes());
        state.bytes_mut()[20..24].copy_from_slice(&(-1.0f32).to_le_bytes());
        let input = SequenceActivation::new(1, 1, 1, vec![0.0]).unwrap();

        assert_eq!(
            block.forward(input, &mut state).unwrap_err(),
            BoundedKvBlockError::NonCanonicalState {
                record: 2,
                reason: "valid records must be contiguous",
            }
        );

        state.bytes_mut()[8..12].copy_from_slice(&0.75f32.to_le_bytes());
        state.bytes_mut()[16..20].copy_from_slice(&EMPTY_FLAG_VALUE.to_le_bytes());
        state.bytes_mut()[20..24].copy_from_slice(&0.0f32.to_le_bytes());
        let input = SequenceActivation::new(1, 1, 1, vec![0.0]).unwrap();
        assert_eq!(
            block.forward(input, &mut state).unwrap_err(),
            BoundedKvBlockError::NonCanonicalState {
                record: 1,
                reason: "valid flag must be exactly 0.0 or 1.0",
            }
        );
    }

    #[test]
    fn bounded_kv_failed_forward_does_not_advance_state_bytes() {
        let block = overflowing_output_block();
        let mut state = block.state_init();
        state.bytes_mut()[0..4].copy_from_slice(&VALID_FLAG_VALUE.to_le_bytes());
        state.bytes_mut()[4..8].copy_from_slice(&f32::MAX.to_le_bytes());
        let before = state.bytes().to_vec();
        let input = SequenceActivation::new(1, 1, 1, vec![0.0]).unwrap();

        assert!(matches!(
            block
                .forward_with_options(input, &mut state, BoundedKvForwardOptions::eval())
                .unwrap_err(),
            BoundedKvBlockError::NonFiniteComputed {
                source: "output_projection",
                index: 0,
            }
        ));
        assert_eq!(state.bytes(), before);
    }

    mod sequence_block {
        use super::*;

        mod bounded_kv {
            use super::*;

            #[test]
            fn bounded_kv_implements_sequence_block_trait() {
                fn state_size_from_trait(block: &impl SequenceBlock) -> SequenceStateSize {
                    block.state_size()
                }

                let block = fixture_block();

                assert_eq!(
                    state_size_from_trait(&block),
                    SequenceStateSize {
                        bytes_per_layer: 256,
                        bytes_per_token: 8,
                        fixed_overhead: 0,
                    }
                );
            }
        }
    }

    fn fixture_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 32, 8).unwrap(),
            test_norm(),
            test_activation(false),
            query_projection(),
            kv_projection(),
            output_projection(),
            test_activation(false),
        )
        .unwrap()
    }

    fn oracle_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(1, 2, 8).unwrap(),
            identity_lut_norm(),
            test_activation(true),
            ternary(1, 1, vec![0.0]),
            ternary(1, 1, vec![1.0]),
            ternary(1, 1, vec![1.0]),
            test_activation(true),
        )
        .unwrap()
    }

    fn three_record_oracle_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(1, 3, 8).unwrap(),
            identity_lut_norm(),
            test_activation(true),
            ternary(1, 1, vec![0.0]),
            ternary(1, 1, vec![1.0]),
            ternary(1, 1, vec![1.0]),
            test_activation(true),
        )
        .unwrap()
    }

    fn two_slot_attention_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 2, 12).unwrap(),
            identity_lut_norm(),
            test_activation(true),
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
            test_activation(true),
        )
        .unwrap()
    }

    fn fixture_block_with_eval_passthrough() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(2, 32, 8).unwrap(),
            test_norm(),
            test_activation(true),
            query_projection(),
            kv_projection(),
            output_projection(),
            test_activation(true),
        )
        .unwrap()
    }

    fn overflowing_output_block() -> BoundedKvBlock {
        BoundedKvBlock::new(
            BoundedKvBlockConfig::new(1, 2, 8).unwrap(),
            identity_lut_norm(),
            test_activation(true),
            ternary(1, 1, vec![0.0]),
            ternary(1, 1, vec![0.0]),
            ternary_with_scales(1, 1, vec![1.0], vec![Q8_8Scale::MAX]),
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
            lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
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

    fn query_projection() -> TernaryLinearQat {
        ternary(1, 2, vec![0.0, 0.0])
    }

    fn kv_projection() -> TernaryLinearQat {
        ternary(1, 2, vec![1.0, 1.0])
    }

    fn output_projection() -> TernaryLinearQat {
        ternary(2, 1, vec![1.0, -1.0])
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

    fn valid_record_count(state: &SequenceState, max_context: usize) -> usize {
        state
            .bytes()
            .chunks_exact(8)
            .take(max_context)
            .filter(|record| {
                let flag = f32::from_le_bytes([record[0], record[1], record[2], record[3]]);
                flag > VALID_FLAG_THRESHOLD
            })
            .count()
    }

    fn test_records(
        state: &SequenceState,
        max_context: usize,
        payload_slots: usize,
    ) -> Vec<(bool, Vec<f32>)> {
        state
            .bytes()
            .chunks_exact(8)
            .take(max_context)
            .map(|record| {
                let flag = f32::from_le_bytes([record[0], record[1], record[2], record[3]]);
                let payload = record[4..]
                    .chunks_exact(STATE_SLOT_BYTES)
                    .take(payload_slots)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect::<Vec<_>>();
                (flag > VALID_FLAG_THRESHOLD, payload)
            })
            .collect()
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1.0e-6,
            "expected {expected}, got {actual}"
        );
    }
}
