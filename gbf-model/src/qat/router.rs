//! Backend-independent top-1 router QAT core.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterShape {
    d_model: usize,
    n_experts: usize,
    rank: usize,
}

impl RouterShape {
    pub fn new(d_model: usize, n_experts: usize, rank: usize) -> Result<Self, Top1RouterQatError> {
        if d_model == 0 {
            return Err(Top1RouterQatError::EmptyModelDim);
        }

        if n_experts == 0 {
            return Err(Top1RouterQatError::EmptyExpertSet);
        }

        if rank == 0 {
            return Err(Top1RouterQatError::EmptyRouterRank);
        }

        Ok(Self {
            d_model,
            n_experts,
            rank,
        })
    }

    pub fn with_default_rank(d_model: usize, n_experts: usize) -> Result<Self, Top1RouterQatError> {
        Self::new(d_model, n_experts, default_router_rank(n_experts))
    }

    pub fn d_model(self) -> usize {
        self.d_model
    }

    pub fn n_experts(self) -> usize {
        self.n_experts
    }

    pub fn rank(self) -> usize {
        self.rank
    }

    fn input_projection_len(self) -> Result<usize, Top1RouterQatError> {
        self.rank
            .checked_mul(self.d_model)
            .ok_or(Top1RouterQatError::ShapeElementOverflow {
                rows: self.rank,
                cols: self.d_model,
            })
    }

    fn expert_projection_len(self) -> Result<usize, Top1RouterQatError> {
        self.n_experts
            .checked_mul(self.rank)
            .ok_or(Top1RouterQatError::ShapeElementOverflow {
                rows: self.n_experts,
                cols: self.rank,
            })
    }
}

/// Default low-rank router bottleneck.
///
/// The architectural target is `max(1, min(ceil(n_experts / 4), 8))`, keeping
/// tiny expert sets non-degenerate while preserving the larger-model cap.
pub fn default_router_rank(n_experts: usize) -> usize {
    n_experts.div_ceil(4).clamp(1, 8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterTrainMode {
    /// Produce a soft expert distribution for router-side training losses.
    SoftTop1,
    /// Produce one-hot dispatch weights for top-1 expert execution.
    HardTop1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterStochasticPhase {
    DenseTeacherWarmup,
    RouterWarmup,
    ExpertTernaryQat,
    FullNumericQat,
    HardenAndSelect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouterExecutionMode {
    Training,
    Eval,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouterStochasticContext {
    execution_mode: RouterExecutionMode,
    seed: u64,
    step: u64,
    layer_id: u64,
}

impl RouterStochasticContext {
    pub const fn training(seed: u64, step: u64, layer_id: u64) -> Self {
        Self {
            execution_mode: RouterExecutionMode::Training,
            seed,
            step,
            layer_id,
        }
    }

    pub const fn eval() -> Self {
        Self {
            execution_mode: RouterExecutionMode::Eval,
            seed: 0,
            step: 0,
            layer_id: 0,
        }
    }

    pub const fn export() -> Self {
        Self {
            execution_mode: RouterExecutionMode::Export,
            seed: 0,
            step: 0,
            layer_id: 0,
        }
    }

    pub const fn execution_mode(self) -> RouterExecutionMode {
        self.execution_mode
    }

    pub const fn seed(self) -> u64 {
        self.seed
    }

    pub const fn step(self) -> u64 {
        self.step
    }

    pub const fn layer_id(self) -> u64 {
        self.layer_id
    }

    pub const fn is_training(self) -> bool {
        matches!(self.execution_mode, RouterExecutionMode::Training)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterStochasticConfig {
    expert_dropout_rate: f32,
    logit_jitter_stddev: f32,
}

impl RouterStochasticConfig {
    pub const DISABLED: Self = Self {
        expert_dropout_rate: 0.0,
        logit_jitter_stddev: 0.0,
    };

    pub fn new(
        expert_dropout_rate: f32,
        logit_jitter_stddev: f32,
    ) -> Result<Self, Top1RouterQatError> {
        validate_expert_dropout_rate(expert_dropout_rate)?;
        validate_logit_jitter_stddev(logit_jitter_stddev)?;

        Ok(Self {
            expert_dropout_rate,
            logit_jitter_stddev,
        })
    }

    pub const fn for_phase(phase: RouterStochasticPhase) -> Self {
        match phase {
            RouterStochasticPhase::DenseTeacherWarmup => Self {
                expert_dropout_rate: 0.0,
                logit_jitter_stddev: 0.0,
            },
            RouterStochasticPhase::RouterWarmup => Self {
                expert_dropout_rate: 0.1,
                logit_jitter_stddev: 0.5,
            },
            RouterStochasticPhase::ExpertTernaryQat => Self {
                expert_dropout_rate: 0.1,
                logit_jitter_stddev: 0.3,
            },
            RouterStochasticPhase::FullNumericQat => Self {
                expert_dropout_rate: 0.05,
                logit_jitter_stddev: 0.1,
            },
            RouterStochasticPhase::HardenAndSelect => Self {
                expert_dropout_rate: 0.0,
                logit_jitter_stddev: 0.0,
            },
        }
    }

    pub const fn expert_dropout_rate(self) -> f32 {
        self.expert_dropout_rate
    }

    pub const fn logit_jitter_stddev(self) -> f32 {
        self.logit_jitter_stddev
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterAuxLossWeights {
    token_balance_proxy: f32,
    z_loss: f32,
    temporal_smoothness: f32,
}

impl RouterAuxLossWeights {
    pub fn new(
        token_balance_proxy: f32,
        z_loss: f32,
        temporal_smoothness: f32,
    ) -> Result<Self, Top1RouterQatError> {
        validate_nonnegative_finite("token balance proxy loss weight", token_balance_proxy)?;
        validate_nonnegative_finite("z-loss weight", z_loss)?;
        validate_nonnegative_finite("temporal smoothness loss weight", temporal_smoothness)?;

        Ok(Self {
            token_balance_proxy,
            z_loss,
            temporal_smoothness,
        })
    }

    pub fn token_balance_proxy(self) -> f32 {
        self.token_balance_proxy
    }

    pub fn z_loss(self) -> f32 {
        self.z_loss
    }

    pub fn temporal_smoothness(self) -> f32 {
        self.temporal_smoothness
    }
}

impl Default for RouterAuxLossWeights {
    fn default() -> Self {
        Self {
            token_balance_proxy: 1.0,
            z_loss: 1.0,
            temporal_smoothness: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterAuxLosses {
    token_balance_proxy_loss: f32,
    z_loss: f32,
    temporal_smoothness_loss: f32,
}

impl RouterAuxLosses {
    /// Single-token proxy for load balancing.
    ///
    /// The standard batch/token load-balance objective is implemented by the
    /// training loss owner, where batch expert fractions are available.
    pub fn token_balance_proxy_loss(self) -> f32 {
        self.token_balance_proxy_loss
    }

    pub fn z_loss(self) -> f32 {
        self.z_loss
    }

    pub fn temporal_smoothness_loss(self) -> f32 {
        self.temporal_smoothness_loss
    }

    pub fn weighted_sum(self, weights: RouterAuxLossWeights) -> f32 {
        self.token_balance_proxy_loss * weights.token_balance_proxy()
            + self.z_loss * weights.z_loss()
            + self.temporal_smoothness_loss * weights.temporal_smoothness()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterForwardOptions {
    mode: RouterTrainMode,
    dropped_experts: Vec<bool>,
    logit_jitter: Vec<f32>,
}

impl RouterForwardOptions {
    pub fn hard_top1(n_experts: usize) -> Self {
        Self {
            mode: RouterTrainMode::HardTop1,
            dropped_experts: vec![false; n_experts],
            logit_jitter: vec![0.0; n_experts],
        }
    }

    pub fn soft_top1(n_experts: usize) -> Self {
        Self {
            mode: RouterTrainMode::SoftTop1,
            dropped_experts: vec![false; n_experts],
            logit_jitter: vec![0.0; n_experts],
        }
    }

    pub fn from_stochastic_config(
        n_experts: usize,
        mode: RouterTrainMode,
        config: RouterStochasticConfig,
        context: RouterStochasticContext,
    ) -> Result<Self, Top1RouterQatError> {
        if n_experts == 0 {
            return Err(Top1RouterQatError::EmptyExpertSet);
        }

        let mut options = Self::hard_top1(n_experts).with_mode(mode);
        if !context.is_training() {
            return Ok(options);
        }

        if config.expert_dropout_rate() > 0.0 {
            options.dropped_experts =
                sample_expert_dropout_mask(n_experts, config.expert_dropout_rate(), context);
        }
        if config.logit_jitter_stddev() > 0.0 {
            options.logit_jitter =
                sample_logit_jitter(n_experts, config.logit_jitter_stddev(), context);
        }

        Ok(options)
    }

    pub fn for_stochastic_phase(
        n_experts: usize,
        mode: RouterTrainMode,
        phase: RouterStochasticPhase,
        context: RouterStochasticContext,
    ) -> Result<Self, Top1RouterQatError> {
        Self::from_stochastic_config(
            n_experts,
            mode,
            RouterStochasticConfig::for_phase(phase),
            context,
        )
    }

    pub fn mode(&self) -> RouterTrainMode {
        self.mode
    }

    pub fn dropped_experts(&self) -> &[bool] {
        &self.dropped_experts
    }

    pub fn logit_jitter(&self) -> &[f32] {
        &self.logit_jitter
    }

    pub fn with_mode(mut self, mode: RouterTrainMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_dropped_experts(mut self, dropped_experts: Vec<bool>) -> Self {
        self.dropped_experts = dropped_experts;
        self
    }

    pub fn with_logit_jitter(mut self, logit_jitter: Vec<f32>) -> Self {
        self.logit_jitter = logit_jitter;
        self
    }

    pub fn apply_expert_dropout(
        &self,
        expert_outputs: &[f32],
        d_model: usize,
    ) -> Result<Vec<f32>, Top1RouterQatError> {
        validate_expert_output_shape(self.dropped_experts().len(), d_model, expert_outputs.len())?;

        Ok(expert_outputs
            .chunks_exact(d_model)
            .zip(self.dropped_experts())
            .flat_map(|(expert_output, &dropped)| {
                expert_output
                    .iter()
                    .map(move |&value| if dropped { 0.0 } else { value })
            })
            .collect())
    }
}

/// Output of one top-1 router forward pass.
///
/// `expert_index` and `dispatch_indicator` are derived from the effective
/// logits. If multiple effective logits are equal, top-1 routing is
/// deterministic and selects the lowest expert index.
#[derive(Debug, Clone, PartialEq)]
pub struct RouterForwardOutput {
    expert_index: usize,
    dispatch_indicator: Vec<f32>,
    routing_weights: Vec<f32>,
    routing_probs: Vec<f32>,
    aux_losses: RouterAuxLosses,
    raw_router_logits: Vec<f32>,
    effective_logits: Vec<f32>,
}

impl RouterForwardOutput {
    /// Selected top-1 expert index.
    ///
    /// Equal effective logits are tie-broken deterministically by choosing the
    /// lowest expert index.
    pub fn expert_index(&self) -> usize {
        self.expert_index
    }

    /// One-hot dispatch indicator for the selected top-1 expert.
    ///
    /// Equal effective logits are tie-broken deterministically by choosing the
    /// lowest expert index before this indicator is built.
    pub fn dispatch_indicator(&self) -> &[f32] {
        &self.dispatch_indicator
    }

    pub fn routing_weights(&self) -> &[f32] {
        &self.routing_weights
    }

    pub fn routing_probs(&self) -> &[f32] {
        &self.routing_probs
    }

    pub fn raw_router_logits(&self) -> &[f32] {
        &self.raw_router_logits
    }

    pub fn aux_losses(&self) -> RouterAuxLosses {
        self.aux_losses
    }

    pub fn effective_logits(&self) -> &[f32] {
        &self.effective_logits
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Top1RouterQat {
    shape: RouterShape,
    input_projection: Vec<f32>,
    input_bias: Option<Vec<f32>>,
    expert_projection: Vec<f32>,
    expert_bias: Option<Vec<f32>>,
    aux_loss_weights: RouterAuxLossWeights,
    previous_distribution: Option<Vec<f32>>,
}

impl Top1RouterQat {
    pub fn new(
        shape: RouterShape,
        input_projection: Vec<f32>,
        input_bias: Option<Vec<f32>>,
        expert_projection: Vec<f32>,
        expert_bias: Option<Vec<f32>>,
    ) -> Result<Self, Top1RouterQatError> {
        Self::new_with_aux_loss_weights(
            shape,
            input_projection,
            input_bias,
            expert_projection,
            expert_bias,
            RouterAuxLossWeights::default(),
        )
    }

    pub fn new_with_aux_loss_weights(
        shape: RouterShape,
        input_projection: Vec<f32>,
        input_bias: Option<Vec<f32>>,
        expert_projection: Vec<f32>,
        expert_bias: Option<Vec<f32>>,
        aux_loss_weights: RouterAuxLossWeights,
    ) -> Result<Self, Top1RouterQatError> {
        validate_matrix(
            "input_projection",
            &input_projection,
            shape.input_projection_len()?,
        )?;
        validate_bias("input_bias", input_bias.as_deref(), shape.rank())?;
        validate_matrix(
            "expert_projection",
            &expert_projection,
            shape.expert_projection_len()?,
        )?;
        validate_bias("expert_bias", expert_bias.as_deref(), shape.n_experts())?;

        Ok(Self {
            shape,
            input_projection,
            input_bias,
            expert_projection,
            expert_bias,
            aux_loss_weights,
            previous_distribution: None,
        })
    }

    pub fn shape(&self) -> RouterShape {
        self.shape
    }

    pub fn input_projection(&self) -> &[f32] {
        &self.input_projection
    }

    pub fn input_bias(&self) -> Option<&[f32]> {
        self.input_bias.as_deref()
    }

    pub fn expert_projection(&self) -> &[f32] {
        &self.expert_projection
    }

    pub fn expert_bias(&self) -> Option<&[f32]> {
        self.expert_bias.as_deref()
    }

    pub fn aux_loss_weights(&self) -> RouterAuxLossWeights {
        self.aux_loss_weights
    }

    pub fn previous_distribution(&self) -> Option<&[f32]> {
        self.previous_distribution.as_deref()
    }

    pub fn reset_sequence(&mut self) {
        self.previous_distribution = None;
    }

    pub fn forward(&mut self, input: &[f32]) -> Result<RouterForwardOutput, Top1RouterQatError> {
        self.forward_with_options(
            input,
            &RouterForwardOptions::hard_top1(self.shape.n_experts()),
        )
    }

    pub fn forward_with_options(
        &mut self,
        input: &[f32],
        options: &RouterForwardOptions,
    ) -> Result<RouterForwardOutput, Top1RouterQatError> {
        let output =
            self.forward_stateless(input, self.previous_distribution.as_deref(), options)?;
        self.previous_distribution = Some(output.routing_probs.clone());
        Ok(output)
    }

    pub fn forward_stateless(
        &self,
        input: &[f32],
        previous_distribution: Option<&[f32]>,
        options: &RouterForwardOptions,
    ) -> Result<RouterForwardOutput, Top1RouterQatError> {
        self.validate_forward_input(input)?;
        self.validate_previous_distribution(previous_distribution)?;
        validate_router_options(self.shape, options)?;

        let hidden = matvec(
            self.shape.rank(),
            self.shape.d_model(),
            &self.input_projection,
            input,
            self.input_bias.as_deref(),
        );
        validate_computed_values("router hidden activation", &hidden)?;
        let raw_router_logits = matvec(
            self.shape.n_experts(),
            self.shape.rank(),
            &self.expert_projection,
            &hidden,
            self.expert_bias.as_deref(),
        );
        validate_computed_values("raw router logits", &raw_router_logits)?;
        let effective_logits = apply_router_training_noise(&raw_router_logits, options);
        validate_computed_values("effective router logits", &effective_logits)?;
        let masked_effective_logits = mask_dropped_experts(&effective_logits, options);
        let routing_probs = softmax(&masked_effective_logits)?;
        let expert_index = top1_index(&masked_effective_logits);
        let dispatch_indicator = one_hot(self.shape.n_experts(), expert_index);
        let routing_weights = match options.mode() {
            RouterTrainMode::SoftTop1 => routing_probs.clone(),
            RouterTrainMode::HardTop1 => dispatch_indicator.clone(),
        };
        let aux_losses = self.compute_aux_losses(
            &raw_router_logits,
            &routing_probs,
            expert_index,
            previous_distribution,
        )?;

        Ok(RouterForwardOutput {
            expert_index,
            dispatch_indicator,
            routing_weights,
            routing_probs,
            aux_losses,
            raw_router_logits,
            effective_logits,
        })
    }

    pub fn apply_expert_dropout(
        &self,
        expert_outputs: &[f32],
        options: &RouterForwardOptions,
    ) -> Result<Vec<f32>, Top1RouterQatError> {
        validate_router_options(self.shape, options)?;
        options.apply_expert_dropout(expert_outputs, self.shape.d_model())
    }

    fn validate_forward_input(&self, input: &[f32]) -> Result<(), Top1RouterQatError> {
        if input.len() != self.shape.d_model() {
            return Err(Top1RouterQatError::InputLenMismatch {
                expected: self.shape.d_model(),
                actual: input.len(),
            });
        }

        if let Some(index) = input.iter().position(|value| !value.is_finite()) {
            return Err(Top1RouterQatError::NonFiniteInput { index });
        }

        Ok(())
    }

    fn validate_previous_distribution(
        &self,
        previous_distribution: Option<&[f32]>,
    ) -> Result<(), Top1RouterQatError> {
        let Some(previous_distribution) = previous_distribution else {
            return Ok(());
        };

        if previous_distribution.len() != self.shape.n_experts() {
            return Err(Top1RouterQatError::PreviousDistributionLenMismatch {
                expected: self.shape.n_experts(),
                actual: previous_distribution.len(),
            });
        }

        if let Some(index) = previous_distribution
            .iter()
            .position(|value| !value.is_finite())
        {
            return Err(Top1RouterQatError::NonFinitePreviousDistribution { index });
        }

        Ok(())
    }

    fn compute_aux_losses(
        &self,
        raw_router_logits: &[f32],
        routing_probs: &[f32],
        expert_index: usize,
        previous_distribution: Option<&[f32]>,
    ) -> Result<RouterAuxLosses, Top1RouterQatError> {
        let z = logsumexp(raw_router_logits)?;
        let z_loss = z * z;
        let token_balance_proxy_loss = routing_probs[expert_index] * self.shape.n_experts() as f32;
        let temporal_smoothness_loss = previous_distribution.map_or(0.0, |previous| {
            let dot = routing_probs
                .iter()
                .zip(previous)
                .map(|(&current, &previous)| current * previous)
                .sum::<f32>();
            (1.0 - dot).clamp(0.0, 1.0)
        });

        let losses = RouterAuxLosses {
            token_balance_proxy_loss,
            z_loss,
            temporal_smoothness_loss,
        };
        validate_aux_loss(
            "token balance proxy loss",
            losses.token_balance_proxy_loss(),
        )?;
        validate_aux_loss("router z-loss", losses.z_loss())?;
        validate_aux_loss(
            "temporal smoothness loss",
            losses.temporal_smoothness_loss(),
        )?;

        Ok(losses)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Top1RouterQatError {
    EmptyModelDim,
    EmptyExpertSet,
    EmptyRouterRank,
    ShapeElementOverflow {
        rows: usize,
        cols: usize,
    },
    MatrixLenMismatch {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    BiasLenMismatch {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteMatrix {
        name: &'static str,
        index: usize,
    },
    NonFiniteBias {
        name: &'static str,
        index: usize,
    },
    NonFiniteLossWeight {
        name: &'static str,
        value: f32,
    },
    InvalidExpertDropoutRate {
        value: f32,
    },
    InvalidLogitJitterStddev {
        value: f32,
    },
    InputLenMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        index: usize,
    },
    PreviousDistributionLenMismatch {
        expected: usize,
        actual: usize,
    },
    NonFinitePreviousDistribution {
        index: usize,
    },
    ExpertOutputLenMismatch {
        expected: usize,
        actual: usize,
    },
    DroppedExpertLenMismatch {
        expected: usize,
        actual: usize,
    },
    LogitJitterLenMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteLogitJitter {
        index: usize,
    },
    NonFiniteRouterComputation {
        name: &'static str,
        index: usize,
    },
    InvalidSoftmaxNormalization {
        sum: f32,
    },
    NonFiniteRoutingProbability {
        index: usize,
    },
    NonFiniteAuxLoss {
        name: &'static str,
        value: f32,
    },
    AllExpertsDropped,
}

impl fmt::Display for Top1RouterQatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyModelDim => f.write_str("router d_model must be non-empty"),
            Self::EmptyExpertSet => f.write_str("router expert set must be non-empty"),
            Self::EmptyRouterRank => f.write_str("router rank must be non-empty"),
            Self::ShapeElementOverflow { rows, cols } => {
                write!(f, "router matrix shape {rows}x{cols} overflows length")
            }
            Self::MatrixLenMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} length mismatch: expected {expected}, got {actual}"
            ),
            Self::BiasLenMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "{name} length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteMatrix { name, index } => {
                write!(f, "{name} matrix value at index {index} is not finite")
            }
            Self::NonFiniteBias { name, index } => {
                write!(f, "{name} bias value at index {index} is not finite")
            }
            Self::NonFiniteLossWeight { name, value } => {
                write!(f, "{name} must be finite and nonnegative, got {value}")
            }
            Self::InvalidExpertDropoutRate { value } => write!(
                f,
                "expert dropout rate must be finite and in [0, 1), got {value}"
            ),
            Self::InvalidLogitJitterStddev { value } => write!(
                f,
                "logit jitter stddev must be finite and nonnegative, got {value}"
            ),
            Self::InputLenMismatch { expected, actual } => {
                write!(
                    f,
                    "router input length mismatch: expected {expected}, got {actual}"
                )
            }
            Self::NonFiniteInput { index } => {
                write!(f, "router input value at index {index} is not finite")
            }
            Self::PreviousDistributionLenMismatch { expected, actual } => write!(
                f,
                "previous routing distribution length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFinitePreviousDistribution { index } => write!(
                f,
                "previous routing distribution value at index {index} is not finite"
            ),
            Self::ExpertOutputLenMismatch { expected, actual } => write!(
                f,
                "expert output length mismatch: expected {expected}, got {actual}"
            ),
            Self::DroppedExpertLenMismatch { expected, actual } => write!(
                f,
                "dropped expert mask length mismatch: expected {expected}, got {actual}"
            ),
            Self::LogitJitterLenMismatch { expected, actual } => write!(
                f,
                "logit jitter length mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteLogitJitter { index } => {
                write!(f, "logit jitter value at index {index} is not finite")
            }
            Self::NonFiniteRouterComputation { name, index } => {
                write!(f, "{name} value at index {index} is not finite")
            }
            Self::InvalidSoftmaxNormalization { sum } => {
                write!(f, "router softmax normalization sum is invalid: {sum}")
            }
            Self::NonFiniteRoutingProbability { index } => {
                write!(f, "routing probability at index {index} is not finite")
            }
            Self::NonFiniteAuxLoss { name, value } => {
                write!(f, "{name} is not finite: {value}")
            }
            Self::AllExpertsDropped => f.write_str("router dropout cannot drop all experts"),
        }
    }
}

impl Error for Top1RouterQatError {}

fn validate_matrix(
    name: &'static str,
    values: &[f32],
    expected: usize,
) -> Result<(), Top1RouterQatError> {
    if values.len() != expected {
        return Err(Top1RouterQatError::MatrixLenMismatch {
            name,
            expected,
            actual: values.len(),
        });
    }

    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(Top1RouterQatError::NonFiniteMatrix { name, index });
    }

    Ok(())
}

fn validate_bias(
    name: &'static str,
    values: Option<&[f32]>,
    expected: usize,
) -> Result<(), Top1RouterQatError> {
    let Some(values) = values else {
        return Ok(());
    };

    if values.len() != expected {
        return Err(Top1RouterQatError::BiasLenMismatch {
            name,
            expected,
            actual: values.len(),
        });
    }

    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(Top1RouterQatError::NonFiniteBias { name, index });
    }

    Ok(())
}

fn validate_nonnegative_finite(name: &'static str, value: f32) -> Result<(), Top1RouterQatError> {
    if !value.is_finite() || value < 0.0 {
        return Err(Top1RouterQatError::NonFiniteLossWeight { name, value });
    }

    Ok(())
}

fn validate_expert_dropout_rate(value: f32) -> Result<(), Top1RouterQatError> {
    if !value.is_finite() || !(0.0..1.0).contains(&value) {
        return Err(Top1RouterQatError::InvalidExpertDropoutRate { value });
    }

    Ok(())
}

fn validate_logit_jitter_stddev(value: f32) -> Result<(), Top1RouterQatError> {
    if !value.is_finite() || value < 0.0 {
        return Err(Top1RouterQatError::InvalidLogitJitterStddev { value });
    }

    Ok(())
}

fn validate_computed_values(name: &'static str, values: &[f32]) -> Result<(), Top1RouterQatError> {
    if let Some(index) = values.iter().position(|value| !value.is_finite()) {
        return Err(Top1RouterQatError::NonFiniteRouterComputation { name, index });
    }

    Ok(())
}

fn validate_aux_loss(name: &'static str, value: f32) -> Result<(), Top1RouterQatError> {
    if !value.is_finite() {
        return Err(Top1RouterQatError::NonFiniteAuxLoss { name, value });
    }

    Ok(())
}

fn validate_router_options(
    shape: RouterShape,
    options: &RouterForwardOptions,
) -> Result<(), Top1RouterQatError> {
    if options.dropped_experts().len() != shape.n_experts() {
        return Err(Top1RouterQatError::DroppedExpertLenMismatch {
            expected: shape.n_experts(),
            actual: options.dropped_experts().len(),
        });
    }

    if options.dropped_experts().iter().all(|&dropped| dropped) {
        return Err(Top1RouterQatError::AllExpertsDropped);
    }

    if options.logit_jitter().len() != shape.n_experts() {
        return Err(Top1RouterQatError::LogitJitterLenMismatch {
            expected: shape.n_experts(),
            actual: options.logit_jitter().len(),
        });
    }

    if let Some(index) = options
        .logit_jitter()
        .iter()
        .position(|value| !value.is_finite())
    {
        return Err(Top1RouterQatError::NonFiniteLogitJitter { index });
    }

    Ok(())
}

fn validate_expert_output_shape(
    n_experts: usize,
    d_model: usize,
    actual: usize,
) -> Result<(), Top1RouterQatError> {
    let expected =
        n_experts
            .checked_mul(d_model)
            .ok_or(Top1RouterQatError::ShapeElementOverflow {
                rows: n_experts,
                cols: d_model,
            })?;
    if actual != expected {
        return Err(Top1RouterQatError::ExpertOutputLenMismatch { expected, actual });
    }

    Ok(())
}

fn matvec(
    rows: usize,
    cols: usize,
    weights: &[f32],
    input: &[f32],
    bias: Option<&[f32]>,
) -> Vec<f32> {
    weights
        .chunks_exact(cols)
        .take(rows)
        .enumerate()
        .map(|(row_index, row)| {
            let weighted_sum = row
                .iter()
                .zip(input)
                .map(|(&weight, &value)| weight * value)
                .sum::<f32>();
            weighted_sum + bias.map_or(0.0, |bias| bias[row_index])
        })
        .collect()
}

fn apply_router_training_noise(logits: &[f32], options: &RouterForwardOptions) -> Vec<f32> {
    logits
        .iter()
        .zip(options.logit_jitter())
        .map(|(&logit, &jitter)| logit + jitter)
        .collect()
}

fn mask_dropped_experts(logits: &[f32], options: &RouterForwardOptions) -> Vec<f32> {
    logits
        .iter()
        .zip(options.dropped_experts())
        .map(|(&logit, &dropped)| if dropped { f32::NEG_INFINITY } else { logit })
        .collect()
}

fn softmax(logits: &[f32]) -> Result<Vec<f32>, Top1RouterQatError> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_values = logits
        .iter()
        .map(|&logit| {
            if logit.is_finite() {
                (logit - max).exp()
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    let sum = exp_values.iter().sum::<f32>();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(Top1RouterQatError::InvalidSoftmaxNormalization { sum });
    }

    let probs = exp_values
        .into_iter()
        .map(|value| value / sum)
        .collect::<Vec<_>>();
    if let Some(index) = probs.iter().position(|value| !value.is_finite()) {
        return Err(Top1RouterQatError::NonFiniteRoutingProbability { index });
    }

    Ok(probs)
}

fn top1_index(logits: &[f32]) -> usize {
    logits
        .iter()
        .copied()
        .enumerate()
        .fold(
            (0, f32::NEG_INFINITY),
            |(best_index, best_value), (index, value)| {
                if value > best_value {
                    (index, value)
                } else {
                    (best_index, best_value)
                }
            },
        )
        .0
}

fn one_hot(len: usize, index: usize) -> Vec<f32> {
    let mut values = vec![0.0; len];
    values[index] = 1.0;
    values
}

fn logsumexp(logits: &[f32]) -> Result<f32, Top1RouterQatError> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum = logits.iter().map(|&logit| (logit - max).exp()).sum::<f32>();
    let value = max + exp_sum.ln();
    if !value.is_finite() {
        return Err(Top1RouterQatError::NonFiniteAuxLoss {
            name: "router logsumexp",
            value,
        });
    }

    Ok(value)
}

const DROPOUT_STREAM_KEY: u64 = 0x52f2_1c0c_e8d3_9a4b;
const JITTER_STREAM_KEY: u64 = 0xa8b9_d4e1_37c2_6f50;
const DROPOUT_KEEP_STREAM_KEY: u64 = 0x6c18_9b7f_d35a_204e;

fn sample_expert_dropout_mask(
    n_experts: usize,
    dropout_rate: f32,
    context: RouterStochasticContext,
) -> Vec<bool> {
    let stream = router_rng_stream(DROPOUT_STREAM_KEY, context);
    let mut dropped = (0..n_experts)
        .map(|expert_index| {
            let sample = sample_unit_interval(stream, expert_index as u64);
            sample < f64::from(dropout_rate)
        })
        .collect::<Vec<_>>();

    if dropped.iter().all(|&is_dropped| is_dropped) {
        let keep_stream = router_rng_stream(DROPOUT_KEEP_STREAM_KEY, context);
        let keep_index = (splitmix64(keep_stream) % n_experts as u64) as usize;
        dropped[keep_index] = false;
    }

    dropped
}

fn sample_logit_jitter(
    n_experts: usize,
    jitter_stddev: f32,
    context: RouterStochasticContext,
) -> Vec<f32> {
    let stream = router_rng_stream(JITTER_STREAM_KEY, context);
    (0..n_experts)
        .map(|expert_index| {
            let z = sample_standard_normal(stream, expert_index as u64);
            (z * f64::from(jitter_stddev)) as f32
        })
        .collect()
}

fn router_rng_stream(stream_key: u64, context: RouterStochasticContext) -> u64 {
    splitmix64(
        stream_key
            ^ context.seed()
            ^ context.step().rotate_left(17)
            ^ context.layer_id().rotate_left(41),
    )
}

fn sample_standard_normal(stream: u64, sample_index: u64) -> f64 {
    let first = sample_index.saturating_mul(2);
    let u1 = sample_unit_interval(stream, first);
    let u2 = sample_unit_interval(stream, first + 1);
    let radius = (-2.0 * u1.ln()).sqrt();
    let theta = std::f64::consts::TAU * u2;

    radius * theta.cos()
}

fn sample_unit_interval(stream: u64, sample_index: u64) -> f64 {
    let mixed = splitmix64(stream ^ sample_index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    ((mixed as f64) + 0.5) / ((u64::MAX as f64) + 1.0)
}

fn splitmix64(mut state: u64) -> u64 {
    state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod dropout {
    use super::*;

    #[test]
    fn qat_router_dropout_phase_defaults_are_config_gated() {
        assert_eq!(
            RouterStochasticConfig::for_phase(RouterStochasticPhase::DenseTeacherWarmup),
            RouterStochasticConfig::DISABLED
        );
        assert_eq!(
            RouterStochasticConfig::for_phase(RouterStochasticPhase::RouterWarmup),
            RouterStochasticConfig::new(0.1, 0.5).unwrap()
        );
        assert_eq!(
            RouterStochasticConfig::for_phase(RouterStochasticPhase::ExpertTernaryQat),
            RouterStochasticConfig::new(0.1, 0.3).unwrap()
        );
        assert_eq!(
            RouterStochasticConfig::for_phase(RouterStochasticPhase::FullNumericQat),
            RouterStochasticConfig::new(0.05, 0.1).unwrap()
        );
        assert_eq!(
            RouterStochasticConfig::for_phase(RouterStochasticPhase::HardenAndSelect),
            RouterStochasticConfig::DISABLED
        );
    }

    #[test]
    fn qat_router_dropout_rejects_invalid_stochastic_config() {
        assert_eq!(
            RouterStochasticConfig::new(1.0, 0.0),
            Err(Top1RouterQatError::InvalidExpertDropoutRate { value: 1.0 })
        );
        assert_eq!(
            RouterStochasticConfig::new(-0.25, 0.0),
            Err(Top1RouterQatError::InvalidExpertDropoutRate { value: -0.25 })
        );
        assert_eq!(
            RouterStochasticConfig::new(0.0, -0.5),
            Err(Top1RouterQatError::InvalidLogitJitterStddev { value: -0.5 })
        );
        assert_eq!(
            RouterForwardOptions::from_stochastic_config(
                0,
                RouterTrainMode::HardTop1,
                RouterStochasticConfig::DISABLED,
                RouterStochasticContext::training(7, 11, 3),
            ),
            Err(Top1RouterQatError::EmptyExpertSet)
        );
    }

    #[test]
    fn qat_router_dropout_is_reproducible_from_seed_step_and_layer() {
        let config = RouterStochasticConfig::new(0.5, 0.25).unwrap();
        let context = RouterStochasticContext::training(7, 11, 3);

        let first = RouterForwardOptions::from_stochastic_config(
            8,
            RouterTrainMode::SoftTop1,
            config,
            context,
        )
        .unwrap();
        let replay = RouterForwardOptions::from_stochastic_config(
            8,
            RouterTrainMode::SoftTop1,
            config,
            context,
        )
        .unwrap();
        let next_step = RouterForwardOptions::from_stochastic_config(
            8,
            RouterTrainMode::SoftTop1,
            config,
            RouterStochasticContext::training(7, 12, 3),
        )
        .unwrap();
        let next_layer = RouterForwardOptions::from_stochastic_config(
            8,
            RouterTrainMode::SoftTop1,
            config,
            RouterStochasticContext::training(7, 11, 4),
        )
        .unwrap();

        assert_eq!(first, replay);
        assert_ne!(first.logit_jitter(), next_step.logit_jitter());
        assert_ne!(first.logit_jitter(), next_layer.logit_jitter());
    }

    #[test]
    fn qat_router_dropout_rate_half_matches_deterministic_sampling_window() {
        let n_experts = 4;
        let trials = 512;
        let dropout_rate = 0.5_f32;
        let config = RouterStochasticConfig::new(dropout_rate, 0.0).unwrap();
        let mut dropped_counts = vec![0_usize; n_experts];

        for step in 0..trials {
            let options = RouterForwardOptions::from_stochastic_config(
                n_experts,
                RouterTrainMode::HardTop1,
                config,
                RouterStochasticContext::training(19, step as u64, 2),
            )
            .unwrap();

            for (expert, is_dropped) in options.dropped_experts().iter().enumerate() {
                dropped_counts[expert] += usize::from(*is_dropped);
            }
        }

        let all_drop_rescue_probability = f64::from(dropout_rate).powi(n_experts as i32);
        let effective_drop_probability =
            f64::from(dropout_rate) - all_drop_rescue_probability / n_experts as f64;
        let expected = trials as f64 * effective_drop_probability;
        let three_sigma = 3.0
            * (trials as f64 * effective_drop_probability * (1.0 - effective_drop_probability))
                .sqrt();

        for (expert, count) in dropped_counts.iter().enumerate() {
            let distance = (*count as f64 - expected).abs();
            assert!(
                distance <= three_sigma,
                "expert {expert} dropped {count} times, expected {expected:.2} +/- {three_sigma:.2}"
            );
        }
    }

    #[test]
    fn qat_router_dropout_eval_and_export_disable_masks_and_jitter() {
        let router = fixture_router();
        let config = RouterStochasticConfig::new(1.0 - f32::EPSILON, 2.0).unwrap();
        let expert_outputs = expert_outputs();

        for context in [
            RouterStochasticContext::eval(),
            RouterStochasticContext::export(),
        ] {
            let options = RouterForwardOptions::from_stochastic_config(
                4,
                RouterTrainMode::HardTop1,
                config,
                context,
            )
            .unwrap();
            let output = router
                .forward_stateless(&[1.0, 2.0, -1.0], None, &options)
                .unwrap();

            assert_eq!(options.dropped_experts(), &[false, false, false, false]);
            assert_eq!(options.logit_jitter(), &[0.0, 0.0, 0.0, 0.0]);
            assert_eq!(
                router
                    .apply_expert_dropout(&expert_outputs, &options)
                    .unwrap(),
                expert_outputs
            );
            assert_eq!(output.raw_router_logits(), output.effective_logits());
        }
    }

    #[test]
    fn qat_router_dropout_zeroes_dropped_experts_and_preserves_active_outputs() {
        let router = fixture_router();
        let options = RouterForwardOptions::from_stochastic_config(
            4,
            RouterTrainMode::HardTop1,
            RouterStochasticConfig::new(1.0 - f32::EPSILON, 0.0).unwrap(),
            RouterStochasticContext::training(17, 23, 5),
        )
        .unwrap();
        let expert_outputs = expert_outputs();

        assert!(options.dropped_experts().iter().any(|&dropped| dropped));
        assert!(options.dropped_experts().iter().any(|&dropped| !dropped));

        let dropped = router
            .apply_expert_dropout(&expert_outputs, &options)
            .unwrap();
        for (expert_index, (&is_dropped, (before, after))) in options
            .dropped_experts()
            .iter()
            .zip(expert_outputs.chunks_exact(3).zip(dropped.chunks_exact(3)))
            .enumerate()
        {
            if is_dropped {
                assert_eq!(after, &[0.0, 0.0, 0.0], "expert {expert_index}");
            } else {
                assert_eq!(after, before, "expert {expert_index}");
            }
        }
    }

    #[test]
    fn qat_router_dropout_jitter_affects_effective_logits_but_z_loss_uses_raw_logits() {
        let router = fixture_router();
        let options = RouterForwardOptions::from_stochastic_config(
            4,
            RouterTrainMode::HardTop1,
            RouterStochasticConfig::new(0.0, 1.0).unwrap(),
            RouterStochasticContext::training(31, 37, 2),
        )
        .unwrap();

        let output = router
            .forward_stateless(&[1.0, 2.0, -1.0], None, &options)
            .unwrap();
        let raw_z = logsumexp(output.raw_router_logits()).unwrap();

        assert_eq!(options.dropped_experts(), &[false, false, false, false]);
        assert!(
            options
                .logit_jitter()
                .iter()
                .any(|jitter| jitter.abs() > 1.0e-6)
        );
        assert_ne!(output.raw_router_logits(), output.effective_logits());
        assert!((output.aux_losses().z_loss() - raw_z * raw_z).abs() <= 1.0e-6);
    }

    fn fixture_router() -> Top1RouterQat {
        Top1RouterQat::new(
            RouterShape::new(3, 4, 2).unwrap(),
            vec![
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0,
            ],
            Some(vec![0.0, 0.0]),
            vec![
                1.0, 0.0, //
                0.0, 1.0, //
                -1.0, 0.5, //
                0.5, -1.0,
            ],
            Some(vec![0.0, 0.25, 0.0, 0.0]),
        )
        .unwrap()
    }

    fn expert_outputs() -> Vec<f32> {
        (1..=12).map(|value| value as f32).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qat_router_default_rank_is_low_rank_and_nonzero() {
        assert_eq!(default_router_rank(2), 1);
        assert_eq!(default_router_rank(4), 1);
        assert_eq!(default_router_rank(16), 4);
        assert_eq!(default_router_rank(64), 8);

        let shape = RouterShape::with_default_rank(16, 8).unwrap();
        assert_eq!(shape.rank(), 2);
    }

    #[test]
    fn qat_router_hard_top1_returns_single_expert_and_aux_losses() {
        let mut router = fixture_router();

        let output = router.forward(&[1.0, 2.0, -1.0]).unwrap();

        assert_eq!(output.expert_index(), 1);
        assert_eq!(output.dispatch_indicator(), &[0.0, 1.0, 0.0, 0.0]);
        assert_eq!(output.routing_weights(), &[0.0, 1.0, 0.0, 0.0]);
        assert!((output.routing_probs().iter().sum::<f32>() - 1.0).abs() < 1.0e-6);
        assert!(output.aux_losses().z_loss().is_finite());
        assert!(output.aux_losses().z_loss() > 0.0);
        assert!(output.aux_losses().token_balance_proxy_loss() > 0.0);
        assert_eq!(output.aux_losses().temporal_smoothness_loss(), 0.0);
        assert_eq!(router.previous_distribution(), Some(output.routing_probs()));
    }

    #[test]
    fn qat_router_soft_top1_returns_soft_distribution() {
        let router = fixture_router();
        let options = RouterForwardOptions::soft_top1(4);

        let output = router
            .forward_stateless(&[1.0, 2.0, -1.0], None, &options)
            .unwrap();

        assert_eq!(output.expert_index(), 1);
        assert!((output.routing_weights().iter().sum::<f32>() - 1.0).abs() < 1.0e-6);
        assert_eq!(output.dispatch_indicator(), &[0.0, 1.0, 0.0, 0.0]);
        assert_eq!(output.routing_weights(), output.routing_probs());
        assert!(output.routing_weights()[1] > output.routing_weights()[0]);
        assert!(output.routing_weights()[1] < 1.0);
    }

    #[test]
    fn qat_router_raw_and_effective_logits_name_jitter_boundary() {
        let router = fixture_router();
        let options = RouterForwardOptions::hard_top1(4)
            .with_dropped_experts(vec![false, true, false, false])
            .with_logit_jitter(vec![0.0, 0.0, 3.0, 0.0]);

        let output = router
            .forward_stateless(&[1.0, 2.0, -1.0], None, &options)
            .unwrap();
        let expected_raw = [1.0, 2.25, 0.0, -1.5];
        let expected_effective = [1.0, 2.25, 3.0, -1.5];
        let raw_z = logsumexp(&expected_raw).unwrap();

        assert_eq!(output.expert_index(), 2);
        assert_eq!(output.dispatch_indicator(), &[0.0, 0.0, 1.0, 0.0]);
        assert_eq!(output.routing_weights(), &[0.0, 0.0, 1.0, 0.0]);
        assert_eq!(output.raw_router_logits(), expected_raw);
        assert_eq!(output.effective_logits(), expected_effective);
        assert!(output.effective_logits()[2] > output.effective_logits()[1]);
        assert!((output.aux_losses().z_loss() - raw_z * raw_z).abs() <= 1.0e-6);
    }

    #[test]
    fn qat_router_default_eval_options_keep_raw_and_effective_logits_equal() {
        let router = fixture_router();
        let output = router
            .forward_stateless(&[1.0, 2.0, -1.0], None, &RouterForwardOptions::hard_top1(4))
            .unwrap();

        assert_eq!(output.raw_router_logits(), output.effective_logits());
    }

    #[test]
    fn qat_router_temporal_smoothness_uses_previous_distribution_at_boundary() {
        let mut router = fixture_router();
        let first = router.forward(&[1.0, 2.0, -1.0]).unwrap();
        let first_routing_probs = first.routing_probs().to_vec();
        let second = router.forward(&[-1.0, 0.5, 2.0]).unwrap();

        assert_eq!(first.expert_index(), 1);
        assert_eq!(second.expert_index(), 2);
        assert_ne!(first.routing_weights(), first.routing_probs());
        assert_ne!(
            router.previous_distribution(),
            Some(first.routing_weights())
        );
        assert!(
            (0.0..=1.0).contains(&second.aux_losses().temporal_smoothness_loss()),
            "temporal smoothness loss should stay normalized"
        );
        assert_ne!(
            router.previous_distribution(),
            Some(first_routing_probs.as_slice())
        );

        router.reset_sequence();
        assert_eq!(router.previous_distribution(), None);
        let after_reset = router.forward(&[-1.0, 0.5, 2.0]).unwrap();
        assert_eq!(after_reset.aux_losses().temporal_smoothness_loss(), 0.0);
    }

    #[test]
    fn qat_router_weighted_aux_loss_keeps_components_typed() {
        let router = fixture_router();
        let options = RouterForwardOptions::soft_top1(4);
        let output = router
            .forward_stateless(&[1.0, 2.0, -1.0], Some(&[0.25; 4]), &options)
            .unwrap();
        let weights = RouterAuxLossWeights::new(2.0, 3.0, 4.0).unwrap();
        let aux = output.aux_losses();

        assert_eq!(
            aux.weighted_sum(weights),
            aux.token_balance_proxy_loss() * 2.0
                + aux.z_loss() * 3.0
                + aux.temporal_smoothness_loss() * 4.0
        );
    }

    #[test]
    fn qat_router_rejects_non_finite_computed_logits_without_advancing_state() {
        let mut router = Top1RouterQat::new(
            RouterShape::new(1, 2, 1).unwrap(),
            vec![f32::MAX],
            None,
            vec![f32::MAX, 1.0],
            None,
        )
        .unwrap();

        let err = router.forward(&[2.0]).unwrap_err();

        assert_eq!(
            err,
            Top1RouterQatError::NonFiniteRouterComputation {
                name: "router hidden activation",
                index: 0
            }
        );
        assert_eq!(router.previous_distribution(), None);
    }

    #[test]
    fn qat_router_top1_tiebreak_uses_lowest_expert_index() {
        let router = Top1RouterQat::new(
            RouterShape::new(1, 3, 1).unwrap(),
            vec![1.0],
            None,
            vec![1.0, 1.0, 1.0],
            None,
        )
        .unwrap();

        let output = router
            .forward_stateless(&[1.0], None, &RouterForwardOptions::hard_top1(3))
            .unwrap();

        assert_eq!(output.expert_index(), 0);
        assert_eq!(output.routing_weights(), &[1.0, 0.0, 0.0]);
    }

    #[test]
    fn qat_router_rejects_invalid_contracts() {
        assert_eq!(
            RouterShape::new(0, 1, 1),
            Err(Top1RouterQatError::EmptyModelDim)
        );
        assert_eq!(
            RouterShape::new(1, 0, 1),
            Err(Top1RouterQatError::EmptyExpertSet)
        );
        assert_eq!(
            RouterShape::new(1, 1, 0),
            Err(Top1RouterQatError::EmptyRouterRank)
        );

        let err = Top1RouterQat::new(
            RouterShape::new(3, 4, 2).unwrap(),
            vec![0.0; 5],
            None,
            vec![0.0; 8],
            None,
        )
        .unwrap_err();
        assert_eq!(
            err,
            Top1RouterQatError::MatrixLenMismatch {
                name: "input_projection",
                expected: 6,
                actual: 5
            }
        );

        let router = fixture_router();
        let err = router
            .forward_stateless(
                &[1.0, 2.0, -1.0],
                None,
                &RouterForwardOptions::hard_top1(4)
                    .with_dropped_experts(vec![true, true, true, true]),
            )
            .unwrap_err();
        assert_eq!(err, Top1RouterQatError::AllExpertsDropped);
    }

    fn fixture_router() -> Top1RouterQat {
        Top1RouterQat::new(
            RouterShape::new(3, 4, 2).unwrap(),
            vec![
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0,
            ],
            Some(vec![0.0, 0.0]),
            vec![
                1.0, 0.0, //
                0.0, 1.0, //
                -1.0, 0.5, //
                0.5, -1.0,
            ],
            Some(vec![0.0, 0.25, 0.0, 0.0]),
        )
        .unwrap()
    }
}
