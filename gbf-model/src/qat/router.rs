//! Backend-independent top-1 router QAT core.

use std::error::Error;
use std::fmt;

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

pub fn default_router_rank(n_experts: usize) -> usize {
    (n_experts / 4).clamp(1, 8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterTrainMode {
    SoftTop1,
    HardTop1,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterAuxLossWeights {
    balance: f32,
    z_loss: f32,
    temporal_smoothness: f32,
}

impl RouterAuxLossWeights {
    pub fn new(
        balance: f32,
        z_loss: f32,
        temporal_smoothness: f32,
    ) -> Result<Self, Top1RouterQatError> {
        validate_nonnegative_finite("balance loss weight", balance)?;
        validate_nonnegative_finite("z-loss weight", z_loss)?;
        validate_nonnegative_finite("temporal smoothness loss weight", temporal_smoothness)?;

        Ok(Self {
            balance,
            z_loss,
            temporal_smoothness,
        })
    }

    pub fn balance(self) -> f32 {
        self.balance
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
            balance: 1.0,
            z_loss: 1.0,
            temporal_smoothness: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouterAuxLosses {
    balance_loss: f32,
    z_loss: f32,
    temporal_smoothness_loss: f32,
}

impl RouterAuxLosses {
    pub fn balance_loss(self) -> f32 {
        self.balance_loss
    }

    pub fn z_loss(self) -> f32 {
        self.z_loss
    }

    pub fn temporal_smoothness_loss(self) -> f32 {
        self.temporal_smoothness_loss
    }

    pub fn weighted_sum(self, weights: RouterAuxLossWeights) -> f32 {
        self.balance_loss * weights.balance()
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterForwardOutput {
    expert_index: usize,
    routing_weights: Vec<f32>,
    aux_losses: RouterAuxLosses,
    logits: Vec<f32>,
}

impl RouterForwardOutput {
    pub fn expert_index(&self) -> usize {
        self.expert_index
    }

    pub fn routing_weights(&self) -> &[f32] {
        &self.routing_weights
    }

    pub fn aux_losses(&self) -> RouterAuxLosses {
        self.aux_losses
    }

    pub fn logits(&self) -> &[f32] {
        &self.logits
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
        self.previous_distribution = Some(output.routing_weights.clone());
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
        let base_logits = matvec(
            self.shape.n_experts(),
            self.shape.rank(),
            &self.expert_projection,
            &hidden,
            self.expert_bias.as_deref(),
        );
        let logits = apply_router_training_noise(&base_logits, options);
        let masked_logits = mask_dropped_experts(&logits, options);
        let soft_probs = softmax(&masked_logits);
        let expert_index = top1_index(&masked_logits);
        let routing_weights = match options.mode() {
            RouterTrainMode::SoftTop1 => soft_probs.clone(),
            RouterTrainMode::HardTop1 => one_hot(self.shape.n_experts(), expert_index),
        };
        let aux_losses =
            self.compute_aux_losses(&logits, &soft_probs, expert_index, previous_distribution);

        Ok(RouterForwardOutput {
            expert_index,
            routing_weights,
            aux_losses,
            logits,
        })
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
        logits: &[f32],
        soft_probs: &[f32],
        expert_index: usize,
        previous_distribution: Option<&[f32]>,
    ) -> RouterAuxLosses {
        let z = logsumexp(logits);
        let z_loss = z * z;
        let balance_loss = soft_probs[expert_index] * self.shape.n_experts() as f32;
        let temporal_smoothness_loss = previous_distribution.map_or(0.0, |previous| {
            let dot = soft_probs
                .iter()
                .zip(previous)
                .map(|(&current, &previous)| current * previous)
                .sum::<f32>();
            (1.0 - dot).clamp(0.0, 1.0)
        });

        RouterAuxLosses {
            balance_loss,
            z_loss,
            temporal_smoothness_loss,
        }
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

fn softmax(logits: &[f32]) -> Vec<f32> {
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

    exp_values.into_iter().map(|value| value / sum).collect()
}

fn top1_index(logits: &[f32]) -> usize {
    logits
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, lhs), (_, rhs)| lhs.total_cmp(rhs))
        .map(|(index, _)| index)
        .expect("validated non-empty expert logits")
}

fn one_hot(len: usize, index: usize) -> Vec<f32> {
    let mut values = vec![0.0; len];
    values[index] = 1.0;
    values
}

fn logsumexp(logits: &[f32]) -> f32 {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum = logits.iter().map(|&logit| (logit - max).exp()).sum::<f32>();
    max + exp_sum.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qat_router_default_rank_is_low_rank_and_nonzero() {
        assert_eq!(default_router_rank(1), 1);
        assert_eq!(default_router_rank(4), 1);
        assert_eq!(default_router_rank(32), 8);
        assert_eq!(default_router_rank(128), 8);

        let shape = RouterShape::with_default_rank(16, 8).unwrap();
        assert_eq!(shape.rank(), 2);
    }

    #[test]
    fn qat_router_hard_top1_returns_single_expert_and_aux_losses() {
        let mut router = fixture_router();

        let output = router.forward(&[1.0, 2.0, -1.0]).unwrap();

        assert_eq!(output.expert_index(), 1);
        assert_eq!(output.routing_weights(), &[0.0, 1.0, 0.0, 0.0]);
        assert!(output.aux_losses().z_loss().is_finite());
        assert!(output.aux_losses().z_loss() > 0.0);
        assert!(output.aux_losses().balance_loss() > 0.0);
        assert_eq!(output.aux_losses().temporal_smoothness_loss(), 0.0);
        assert_eq!(
            router.previous_distribution(),
            Some([0.0, 1.0, 0.0, 0.0].as_slice())
        );
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
        assert!(output.routing_weights()[1] > output.routing_weights()[0]);
        assert!(output.routing_weights()[1] < 1.0);
    }

    #[test]
    fn qat_router_dropout_and_jitter_are_explicit_forward_inputs() {
        let router = fixture_router();
        let options = RouterForwardOptions::hard_top1(4)
            .with_dropped_experts(vec![false, true, false, false])
            .with_logit_jitter(vec![0.0, 0.0, 3.0, 0.0]);

        let output = router
            .forward_stateless(&[1.0, 2.0, -1.0], None, &options)
            .unwrap();

        assert_eq!(output.expert_index(), 2);
        assert_eq!(output.routing_weights(), &[0.0, 0.0, 1.0, 0.0]);
        assert!(output.logits()[2] > output.logits()[1]);
    }

    #[test]
    fn qat_router_temporal_smoothness_uses_previous_distribution_at_boundary() {
        let mut router = fixture_router();
        let first = router.forward(&[1.0, 2.0, -1.0]).unwrap();
        let second = router.forward(&[-1.0, 0.5, 2.0]).unwrap();

        assert_eq!(first.expert_index(), 1);
        assert_eq!(second.expert_index(), 2);
        assert!(
            (0.0..=1.0).contains(&second.aux_losses().temporal_smoothness_loss()),
            "temporal smoothness loss should stay normalized"
        );

        router.reset_sequence();
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
            aux.balance_loss() * 2.0 + aux.z_loss() * 3.0 + aux.temporal_smoothness_loss() * 4.0
        );
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
