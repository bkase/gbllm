//! S2 LinearState gradient-smoke wrapper.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_foundation::Hash256;
use gbf_model::qat::{
    ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode, AffineParams,
    LutSpec, MatrixShape, NormApproxPlan, NormApproxQat, NormClip, Q8_8Scale, QuantHardness,
    TernaryLinearQat, TernaryThreshold,
};
use gbf_model::sequence::{LinearStateBlock, LinearStateBlockConfig};
use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::sequence::LinearStateBurnQat;

use crate::S2_LOG_TARGET;
use crate::s1::rng::{Pcg64Mcg, S1Rng, seed128};
use crate::s1::schema::S1SchemaError;
use crate::s2::schema::LinearStateSmokeReport;

/// Canonical H6 fixture identifier.
pub const FIXTURE_ID: &str = "FIXTURE_V1";
/// Canonical H6 sequence length.
pub const FIXTURE_SEQ_LEN: usize = 8;
/// Canonical H6 hidden dimension.
pub const FIXTURE_HIDDEN_DIM: usize = 4;
/// Canonical H6 batch size. The LinearState executable state is one shared
/// buffer, so this fixture intentionally rejects broader batch semantics.
pub const FIXTURE_BATCH: usize = 1;
/// Fixed recurrence decay used by LinearState v1.
///
/// This mirrors the Burn adapter's fixed `STATE_DECAY = 0.5` contract. The H6
/// snapshots pin the resulting forward/gradient bytes so an adapter-side drift
/// changes this fixture visibly.
pub const FIXTURE_DECAY: f32 = 0.5;
/// RFC domain for the FIXTURE_V1 input stream.
pub const LINEARSTATE_INPUT_RNG_DOMAIN: &str = "linearstate_smoke/linearstate_input_v1";
/// RFC domain for the FIXTURE_V1 parameter stream.
pub const LINEARSTATE_PARAMS_RNG_DOMAIN: &str = "linearstate_smoke/linearstate_params_v1";
/// RFC seed used by FIXTURE_V1 LinearStateSmokeRng streams.
pub const LINEARSTATE_SMOKE_RNG_SEED: u64 = 0;
/// Declared-active input projection full-precision weight report key.
pub const INPUT_PROJECTION_WEIGHT: &str = "input_weight";
/// Declared-active state readout/output projection full-precision weight report key.
///
/// The historical report key is `recurrence_weight`, but this is not a
/// trainable decay kernel. It is the state-to-output readout projection whose
/// gradient proves the recurrent state participates in the weighted-mean loss.
pub const STATE_READOUT_OUTPUT_PROJECTION_WEIGHT: &str = "recurrence_weight";
/// Explicit H6 declared-active parameter set.
///
/// H6 checks the two projection full-precision ternary weights because those
/// are the trainable tensors owned by the LinearState smoke wrapper. The fixed
/// decay, norm plan, activation quantization, thresholds, and scales are
/// fixture constants for this bead rather than declared-active trainable
/// parameters.
pub const DECLARED_ACTIVE_PARAMETERS: [&str; 2] = [
    INPUT_PROJECTION_WEIGHT,
    STATE_READOUT_OUTPUT_PROJECTION_WEIGHT,
];

/// Result of running the H6 LinearState smoke wrapper.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearStateSmokeRun {
    /// Canonical report emitted by the wrapper.
    pub report: LinearStateSmokeReport,
    /// Deterministic gradient/output byte capture for the first run.
    pub run_1_bytes: Vec<u8>,
    /// Deterministic gradient/output byte capture for the second run.
    pub run_2_bytes: Vec<u8>,
}

/// Fixture controls used by negative tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearStateSmokeMode {
    /// Canonical non-degenerate fixture.
    Passing,
    /// Structural LS-2 negative fixture with the state readout excluded from
    /// the differentiable loss before backward.
    StructuralDeadRecurrence,
    /// Attempt all-zero initialization and require construction rejection.
    AllZeroInit,
}

impl LinearStateSmokeMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Passing => "passing",
            Self::StructuralDeadRecurrence => "structural-dead-recurrence",
            Self::AllZeroInit => "all-zero-init",
        }
    }
}

/// Return the explicit H6 declared-active parameter set.
#[must_use]
pub const fn declared_active_parameters() -> &'static [&'static str] {
    &DECLARED_ACTIVE_PARAMETERS
}

/// Parameter selections backed by FIXTURE_V1's RFC parameter RNG stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearStateSmokeParameter {
    /// Input-to-state projection full-precision weights.
    InputProjection,
    /// State-readout/output projection full-precision weights.
    StateReadoutOutputProjection,
}

/// Deterministic FIXTURE_V1 input values from the RFC LinearStateSmokeRng input domain.
#[must_use]
pub fn fixture_input_values() -> Vec<f32> {
    deterministic_input_values()
}

/// Deterministic FIXTURE_V1 projection weights from the RFC LinearStateSmokeRng parameter domain.
#[must_use]
pub fn fixture_projection_weights(parameter: LinearStateSmokeParameter) -> Vec<f32> {
    let (input_to_state, state_to_output) = deterministic_projection_weight_pair();
    match parameter {
        LinearStateSmokeParameter::InputProjection => input_to_state,
        LinearStateSmokeParameter::StateReadoutOutputProjection => state_to_output,
    }
}

/// Run the canonical S2 H6 LinearState smoke fixture twice and compare bytes.
pub fn run_fixture_v1() -> Result<LinearStateSmokeRun, LinearStateSmokeError> {
    #[cfg(feature = "falsify")]
    if crate::s2::falsify::is_active(crate::s2::falsify::BrokenKind::F6LinearStateGradDead) {
        return run_fixture_v1_with_mode(LinearStateSmokeMode::StructuralDeadRecurrence);
    }

    run_fixture_v1_with_mode(LinearStateSmokeMode::Passing)
}

/// Run the fixture with explicit controls for negative tests.
pub fn run_fixture_v1_with_mode(
    mode: LinearStateSmokeMode,
) -> Result<LinearStateSmokeRun, LinearStateSmokeError> {
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "linearstate_smoke_start",
        event = "linearstate_smoke_start",
        fixture_id = FIXTURE_ID,
        seq_len = FIXTURE_SEQ_LEN,
        hidden_dim = FIXTURE_HIDDEN_DIM,
        batch = FIXTURE_BATCH,
        decay = "Fixed(0.5)",
        mode = mode.as_str(),
        "s2 linearstate smoke start"
    );

    let run_1 = single_run(mode)?;
    let run_2 = single_run(mode)?;
    let determinism_byte_equal = run_1.bytes == run_2.bytes;
    let run_1_bytes = run_1.bytes.clone();
    let run_2_bytes = run_2.bytes.clone();
    let mut report = report_from_single_run(run_1, determinism_byte_equal)?;

    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "linearstate_smoke_byte_compare",
        event = "linearstate_smoke_byte_compare",
        run_1_bytes = hex_prefix(&run_1_bytes),
        run_2_bytes = hex_prefix(&run_2_bytes),
        equal = determinism_byte_equal,
        "s2 linearstate smoke byte compare"
    );

    report = report.with_computed_self_hash()?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "linearstate_smoke_finalized",
        event = "linearstate_smoke_finalized",
        smoke_passed = report.smoke_passed,
        forward_finite = report.forward_finite,
        determinism_byte_equal = report.determinism_byte_equal,
        smoke_self_hash = %report.smoke_self_hash,
        "s2 linearstate smoke finalized"
    );

    Ok(LinearStateSmokeRun {
        report,
        run_1_bytes,
        run_2_bytes,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct SingleRun {
    forward_finite: bool,
    param_grad_norms: BTreeMap<String, f32>,
    input_grad_norm: f32,
    bytes: Vec<u8>,
}

fn single_run(mode: LinearStateSmokeMode) -> Result<SingleRun, LinearStateSmokeError> {
    let device = BurnDevice::<BurnNdArrayAutodiffBackend>::default();
    let layer =
        LinearStateBurnQat::<BurnNdArrayAutodiffBackend>::from_core(fixture_block(mode)?, &device)?;
    let input_values = deterministic_input_values();
    let input = float_tensor_from_vec::<BurnNdArrayAutodiffBackend, 2>(
        input_values,
        [FIXTURE_SEQ_LEN, FIXTURE_HIDDEN_DIM],
        &device,
    )?
    .require_grad();
    let initial_state = layer.zero_state(&device).require_grad();
    let coeff = float_tensor_from_vec::<BurnNdArrayAutodiffBackend, 2>(
        weighted_mean_coefficients(),
        [FIXTURE_SEQ_LEN, FIXTURE_HIDDEN_DIM],
        &device,
    )?;

    let run = layer.train_forward(input.clone(), initial_state)?;
    let activations = run.activations();
    let output_values = float_tensor_into_vec(activations.clone().detach())?;
    let forward_finite = output_values.iter().all(|value| value.is_finite());
    let loss = fixture_loss(mode, &layer, input.clone(), activations, coeff)?;
    let gradients = loss.backward();
    let input_grad = float_tensor_into_vec(
        input
            .grad(&gradients)
            .ok_or(LinearStateSmokeError::MissingGradient { parameter: "input" })?,
    )?;
    let input_grad_norm = l2_norm(&input_grad)?;

    let input_weight_grad = float_tensor_into_vec(
        layer
            .input_to_state()
            .full_precision_weights()
            .grad(&gradients)
            .ok_or(LinearStateSmokeError::MissingGradient {
                parameter: INPUT_PROJECTION_WEIGHT,
            })?,
    )?;
    let recurrence_weight_grad = float_tensor_into_vec(
        layer
            .state_to_output()
            .full_precision_weights()
            .grad(&gradients)
            .ok_or(LinearStateSmokeError::MissingGradient {
                parameter: STATE_READOUT_OUTPUT_PROJECTION_WEIGHT,
            })?,
    )?;

    let mut param_grad_norms = BTreeMap::new();
    param_grad_norms.insert(
        INPUT_PROJECTION_WEIGHT.to_owned(),
        l2_norm(&input_weight_grad)?,
    );
    param_grad_norms.insert(
        STATE_READOUT_OUTPUT_PROJECTION_WEIGHT.to_owned(),
        l2_norm(&recurrence_weight_grad)?,
    );
    for (parameter, grad_norm) in &param_grad_norms {
        tracing::debug!(
            target: S2_LOG_TARGET,
            event_name = "linearstate_smoke_grad",
            event = "linearstate_smoke_grad",
            parameter = parameter.as_str(),
            parameter_role = parameter_role(parameter),
            grad_norm = *grad_norm,
            declared_active = declared_active_parameters().contains(&parameter.as_str()),
            "s2 linearstate smoke gradient"
        );
    }

    let bytes = capture_run_bytes(
        &output_values,
        &input_grad,
        &input_weight_grad,
        &recurrence_weight_grad,
    );
    Ok(SingleRun {
        forward_finite,
        param_grad_norms,
        input_grad_norm,
        bytes,
    })
}

fn report_from_single_run(
    run: SingleRun,
    determinism_byte_equal: bool,
) -> Result<LinearStateSmokeReport, LinearStateSmokeError> {
    let smoke_passed = run.forward_finite
        && determinism_byte_equal
        && run.input_grad_norm > 0.0
        && run
            .param_grad_norms
            .values()
            .all(|value| value.is_finite() && *value > 0.0);
    if !smoke_passed {
        let failing_parameter = run
            .param_grad_norms
            .iter()
            .find_map(|(name, value)| (!value.is_finite() || *value <= 0.0).then_some(name));
        tracing::error!(
            target: S2_LOG_TARGET,
            event_name = "linearstate_smoke_failed",
            event = "linearstate_smoke_failed",
            reason = failure_reason(&run, determinism_byte_equal),
            failing_parameter = failing_parameter.map(String::as_str).unwrap_or("null"),
            "s2 linearstate smoke failed"
        );
    }
    Ok(LinearStateSmokeReport {
        schema: "s2_linearstate_grad_smoke.v1".to_owned(),
        fixture_id: FIXTURE_ID.to_owned(),
        seq_len: FIXTURE_SEQ_LEN as u64,
        hidden_dim: FIXTURE_HIDDEN_DIM as u64,
        batch: FIXTURE_BATCH as u64,
        forward_finite: run.forward_finite,
        param_grad_norms: run.param_grad_norms,
        inactive_parameters: Default::default(),
        input_grad_norm: run.input_grad_norm,
        determinism_byte_equal,
        smoke_passed,
        smoke_self_hash: Hash256::ZERO,
    })
}

fn failure_reason(run: &SingleRun, determinism_byte_equal: bool) -> &'static str {
    if !run.forward_finite {
        "LS-1: forward output is non-finite"
    } else if let Some((name, _)) = run
        .param_grad_norms
        .iter()
        .find(|(_, value)| !value.is_finite() || **value <= 0.0)
    {
        if name == STATE_READOUT_OUTPUT_PROJECTION_WEIGHT {
            "LS-2: param 'recurrence_weight' grad_norm = 0"
        } else {
            "LS-2: active parameter grad_norm = 0"
        }
    } else if run.input_grad_norm <= 0.0 {
        "LS-3: input_grad_norm = 0"
    } else if !determinism_byte_equal {
        "LS-4: rerun bytes differ"
    } else {
        "LS-5: smoke_passed mismatch"
    }
}

/// Build the backend-independent LinearState block for FIXTURE_V1.
pub fn fixture_block(
    mode: LinearStateSmokeMode,
) -> Result<LinearStateBlock, LinearStateSmokeError> {
    let (input_to_state, state_to_output) = match mode {
        LinearStateSmokeMode::Passing | LinearStateSmokeMode::StructuralDeadRecurrence => {
            deterministic_projection_weight_pair()
        }
        LinearStateSmokeMode::AllZeroInit => (vec![0.0; 16], vec![0.0; 16]),
    };
    validate_non_degenerate_init(INPUT_PROJECTION_WEIGHT, &input_to_state, FIXTURE_HIDDEN_DIM)?;
    validate_non_degenerate_init(
        STATE_READOUT_OUTPUT_PROJECTION_WEIGHT,
        &state_to_output,
        FIXTURE_HIDDEN_DIM,
    )?;

    let mut block = LinearStateBlock::new(
        LinearStateBlockConfig::new(FIXTURE_HIDDEN_DIM, (FIXTURE_HIDDEN_DIM * 4) as u16)?,
        identity_norm(),
        activation(),
        ternary(FIXTURE_HIDDEN_DIM, FIXTURE_HIDDEN_DIM, input_to_state)?,
        ternary(FIXTURE_HIDDEN_DIM, FIXTURE_HIDDEN_DIM, state_to_output)?,
        activation(),
    )?;
    block.set_hardness(QuantHardness::Off, QuantHardness::Off, QuantHardness::Off);
    Ok(block)
}

fn validate_non_degenerate_init(
    parameter: &'static str,
    weights: &[f32],
    row_width: usize,
) -> Result<(), LinearStateSmokeError> {
    if weights.iter().all(|value| *value == 0.0) {
        return Err(LinearStateSmokeError::DegenerateInit {
            parameter,
            reason: "all-zero weights",
        });
    }
    if weights.chunks_exact(row_width).all_equal() {
        return Err(LinearStateSmokeError::DegenerateInit {
            parameter,
            reason: "repeated rows",
        });
    }
    if weights.iter().any(|value| !value.is_finite()) {
        return Err(LinearStateSmokeError::DegenerateInit {
            parameter,
            reason: "non-finite weights",
        });
    }
    Ok(())
}

trait ChunksAllEqual {
    fn all_equal(self) -> bool;
}

impl<'a, I> ChunksAllEqual for I
where
    I: Iterator<Item = &'a [f32]>,
{
    fn all_equal(mut self) -> bool {
        let Some(first) = self.next() else {
            return true;
        };
        self.all(|row| row == first)
    }
}

fn deterministic_input_values() -> Vec<f32> {
    let mut rng = Pcg64Mcg::new(seed128(
        LINEARSTATE_INPUT_RNG_DOMAIN,
        LINEARSTATE_SMOKE_RNG_SEED,
    ));
    let mut values = Vec::with_capacity(FIXTURE_SEQ_LEN * FIXTURE_HIDDEN_DIM);
    for t in 0..FIXTURE_SEQ_LEN {
        for h in 0..FIXTURE_HIDDEN_DIM {
            let sign = if (t + h) % 2 == 0 { 1.0 } else { -1.0 };
            let trend = t as f32 * 0.015625 + h as f32 * 0.03125;
            values.push(sign * (0.25 + trend + draw_range(&mut rng, 0.0, 0.125)));
        }
    }
    values
}

fn deterministic_projection_weight_pair() -> (Vec<f32>, Vec<f32>) {
    let mut rng = Pcg64Mcg::new(seed128(
        LINEARSTATE_PARAMS_RNG_DOMAIN,
        LINEARSTATE_SMOKE_RNG_SEED,
    ));
    let input_to_state =
        deterministic_projection_weights(&mut rng, LinearStateSmokeParameter::InputProjection);
    let state_to_output = deterministic_projection_weights(
        &mut rng,
        LinearStateSmokeParameter::StateReadoutOutputProjection,
    );
    (input_to_state, state_to_output)
}

fn deterministic_projection_weights(
    rng: &mut Pcg64Mcg,
    parameter: LinearStateSmokeParameter,
) -> Vec<f32> {
    let projection_bias = match parameter {
        LinearStateSmokeParameter::InputProjection => 0.0,
        LinearStateSmokeParameter::StateReadoutOutputProjection => 0.045,
    };
    let mut weights = Vec::with_capacity(FIXTURE_HIDDEN_DIM * FIXTURE_HIDDEN_DIM);
    for row in 0..FIXTURE_HIDDEN_DIM {
        for col in 0..FIXTURE_HIDDEN_DIM {
            let diagonal = if row == col { 0.35 } else { -0.14 };
            weights.push(
                diagonal + projection_bias + row as f32 * 0.055 - col as f32 * 0.025
                    + draw_range(rng, -0.045, 0.045),
            );
        }
    }
    weights
}

fn draw_range(rng: &mut Pcg64Mcg, lo: f32, hi: f32) -> f32 {
    debug_assert!(lo < hi);
    lo + (hi - lo) * draw_unit_f32(rng)
}

fn draw_unit_f32(rng: &mut Pcg64Mcg) -> f32 {
    const MANTISSA_BITS: u32 = 24;
    let mantissa = rng.next_u64() >> (u64::BITS - MANTISSA_BITS);
    mantissa as f32 / (1_u32 << MANTISSA_BITS) as f32
}

fn parameter_role(parameter: &str) -> &'static str {
    match parameter {
        INPUT_PROJECTION_WEIGHT => "input_projection_full_precision_weight",
        STATE_READOUT_OUTPUT_PROJECTION_WEIGHT => {
            "state_readout_output_projection_full_precision_weight"
        }
        _ => "unknown",
    }
}

fn fixture_loss(
    mode: LinearStateSmokeMode,
    layer: &LinearStateBurnQat<BurnNdArrayAutodiffBackend>,
    input: gbf_train::adapter::burn::BurnFloatTensor<BurnNdArrayAutodiffBackend, 2>,
    activations: gbf_train::adapter::burn::BurnFloatTensor<BurnNdArrayAutodiffBackend, 2>,
    coeff: gbf_train::adapter::burn::BurnFloatTensor<BurnNdArrayAutodiffBackend, 2>,
) -> Result<
    gbf_train::adapter::burn::BurnFloatTensor<BurnNdArrayAutodiffBackend, 1>,
    LinearStateSmokeError,
> {
    let loss_input = match mode {
        LinearStateSmokeMode::Passing | LinearStateSmokeMode::AllZeroInit => activations,
        LinearStateSmokeMode::StructuralDeadRecurrence => {
            let input_projection = layer
                .input_to_state()
                .fake_quant_forward(input)
                .map_err(gbf_train::sequence::LinearStateBurnQatError::from)?;
            // This is the smallest H6-local structural hook: the real adapter
            // forward above still proves finite readout behavior, but the
            // negative fixture's differentiable loss routes through the input
            // projection and gives the state readout only a zero-gated
            // contribution before backward. No gradient buffer is edited after
            // autodiff runs.
            input_projection + activations * 0.0
        }
    };
    let coeff_sum = weighted_mean_coefficients().iter().sum::<f32>();
    Ok((loss_input * coeff).sum() / coeff_sum)
}

fn weighted_mean_coefficients() -> Vec<f32> {
    (0..FIXTURE_SEQ_LEN)
        .flat_map(|t| (0..FIXTURE_HIDDEN_DIM).map(move |h| 1.0 + t as f32 + 17.0 * h as f32))
        .collect()
}

fn identity_norm() -> NormApproxQat {
    NormApproxQat::new(NormApproxPlan::AffineClipLut {
        affine: AffineParams::new(1.0, 0.0).expect("identity affine validates"),
        clip: NormClip::new(-8.0, 8.0).expect("wide clip validates"),
        lut: LutSpec::new(-1.0, 1.0, 3).expect("small LUT validates"),
    })
}

fn activation() -> ActFakeQuant {
    ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-8.0, 8.0).expect("range validates")),
        ActivationQuantFormat::Int8,
    )
    .expect("fixed activation fake quant validates")
}

fn ternary(
    output_rows: usize,
    input_cols: usize,
    weights: Vec<f32>,
) -> Result<TernaryLinearQat, LinearStateSmokeError> {
    Ok(TernaryLinearQat::new(
        MatrixShape::new(output_rows, input_cols)?,
        weights,
        None,
        vec![TernaryThreshold::new(0.5)?; output_rows],
        vec![Q8_8Scale::from_f32(1.0)?; output_rows],
    )?)
}

fn l2_norm(values: &[f32]) -> Result<f32, LinearStateSmokeError> {
    let sum = values.iter().try_fold(0.0_f32, |acc, value| {
        if value.is_finite() {
            Ok(acc + value * value)
        } else {
            Err(LinearStateSmokeError::NonFiniteGradient)
        }
    })?;
    Ok(sum.sqrt())
}

fn capture_float_chunks(chunks: &[&[f32]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in chunks {
        bytes.extend_from_slice(&(chunk.len() as u64).to_be_bytes());
        for value in *chunk {
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
    }
    bytes
}

fn capture_run_bytes(
    output_values: &[f32],
    input_grad: &[f32],
    input_weight_grad: &[f32],
    recurrence_weight_grad: &[f32],
) -> Vec<u8> {
    capture_float_chunks(&[
        output_values,
        input_grad,
        input_weight_grad,
        recurrence_weight_grad,
    ])
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

/// Errors from the S2 H6 LinearState smoke wrapper.
#[derive(Debug)]
pub enum LinearStateSmokeError {
    /// Core LinearState construction failed.
    Core(gbf_model::sequence::LinearStateBlockError),
    /// QAT projection construction failed.
    Ternary(gbf_model::qat::TernaryLinearQatError),
    /// Burn adapter setup or forward failed.
    Burn(gbf_train::sequence::LinearStateBurnQatError),
    /// Burn tensor helper failed.
    BurnAdapter(gbf_train::adapter::burn::BurnAdapterError),
    /// Report hashing or validation failed.
    Schema(S1SchemaError),
    /// A declared gradient was absent from Burn autodiff.
    MissingGradient {
        /// Parameter or input whose gradient was absent.
        parameter: &'static str,
    },
    /// A gradient buffer contained NaN or Inf.
    NonFiniteGradient,
    /// Fixture initialization violated the non-degenerate policy.
    DegenerateInit {
        /// Parameter rejected by the non-degenerate initializer policy.
        parameter: &'static str,
        /// Human-readable rejection reason.
        reason: &'static str,
    },
}

impl fmt::Display for LinearStateSmokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::Ternary(error) => write!(f, "{error}"),
            Self::Burn(error) => write!(f, "{error}"),
            Self::BurnAdapter(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::MissingGradient { parameter } => {
                write!(f, "missing LinearState gradient for {parameter}")
            }
            Self::NonFiniteGradient => f.write_str("LinearState gradient contained NaN or Inf"),
            Self::DegenerateInit { parameter, reason } => {
                write!(f, "LinearState {parameter} init rejected: {reason}")
            }
        }
    }
}

impl Error for LinearStateSmokeError {}

impl From<gbf_model::sequence::LinearStateBlockError> for LinearStateSmokeError {
    fn from(error: gbf_model::sequence::LinearStateBlockError) -> Self {
        Self::Core(error)
    }
}

impl From<gbf_model::qat::TernaryLinearQatError> for LinearStateSmokeError {
    fn from(error: gbf_model::qat::TernaryLinearQatError) -> Self {
        Self::Ternary(error)
    }
}

impl From<gbf_train::sequence::LinearStateBurnQatError> for LinearStateSmokeError {
    fn from(error: gbf_train::sequence::LinearStateBurnQatError) -> Self {
        Self::Burn(error)
    }
}

impl From<gbf_train::adapter::burn::BurnAdapterError> for LinearStateSmokeError {
    fn from(error: gbf_train::adapter::burn::BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

impl From<S1SchemaError> for LinearStateSmokeError {
    fn from(error: S1SchemaError) -> Self {
        Self::Schema(error)
    }
}
