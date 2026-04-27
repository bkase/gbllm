//! Router-side MoE training loss contracts.
//!
//! This module owns the batch/token router terms used by the training loss
//! composer: centered router z-loss and top-1 expert load balance. The centered
//! z-loss is the `lambda_zrouter` training term and is intentionally distinct
//! from the single-token uncentered QAT router aux-loss proxy. The returned
//! values are raw diagnostic losses; callers that compose total loss apply
//! `lambda_zrouter` and `lambda_balance` explicitly.

use std::error::Error;
use std::fmt;

use crate::phase::TrainPhaseKind;

#[cfg(feature = "burn-adapter")]
use crate::adapter::burn::{
    BurnAdapterError, BurnBackend, BurnDevice, BurnFloatTensor, float_tensor_from_vec,
    float_tensor_into_vec, float_tensor_shape,
};

const ROUTING_PROBABILITY_SUM_TOLERANCE: f32 = 1.0e-4;
const ROUTING_PROBABILITY_ARGMAX_TOLERANCE: f32 = 1.0e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouterLossWeightKind {
    Balance,
    ZRouter,
}

impl fmt::Display for RouterLossWeightKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Balance => f.write_str("lambda_balance"),
            Self::ZRouter => f.write_str("lambda_zrouter"),
        }
    }
}

#[derive(Debug)]
pub enum RouterLossError {
    EmptyRouterLogits,
    EmptyRoutingProbabilities,
    ZeroExpertCount,
    RouterLogitCountNotDivisibleByExpertCount {
        logit_len: usize,
        n_experts: usize,
    },
    RoutingProbabilityCountNotDivisibleByExpertCount {
        probability_len: usize,
        n_experts: usize,
    },
    ExpertAssignmentCountMismatch {
        token_count: usize,
        assignment_count: usize,
    },
    ExpertAssignmentOutOfRange {
        token_index: usize,
        assignment: usize,
        n_experts: usize,
    },
    ExpertAssignmentNotTopProbability {
        token_index: usize,
        assignment: usize,
        assigned_probability: f32,
        max_probability: f32,
    },
    NonFiniteRouterLogit {
        index: usize,
        value: f32,
    },
    NonFiniteRoutingProbability {
        index: usize,
        value: f32,
    },
    NegativeRoutingProbability {
        index: usize,
        value: f32,
    },
    InvalidRoutingProbabilitySum {
        token_index: usize,
        sum: f32,
    },
    NonFiniteLoss {
        value: f32,
    },
    NegativeLossWeight {
        kind: RouterLossWeightKind,
        value: f32,
    },
    NonFiniteLossWeight {
        kind: RouterLossWeightKind,
        value: f32,
    },
    #[cfg(feature = "burn-adapter")]
    InvalidRouterLogitShape {
        shape: Vec<usize>,
    },
    #[cfg(feature = "burn-adapter")]
    InvalidRoutingProbabilityShape {
        shape: Vec<usize>,
    },
    #[cfg(feature = "burn-adapter")]
    BurnAdapter(BurnAdapterError),
}

impl PartialEq for RouterLossError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::EmptyRouterLogits, Self::EmptyRouterLogits)
            | (Self::EmptyRoutingProbabilities, Self::EmptyRoutingProbabilities)
            | (Self::ZeroExpertCount, Self::ZeroExpertCount) => true,
            (
                Self::RouterLogitCountNotDivisibleByExpertCount {
                    logit_len: left_logit_len,
                    n_experts: left_n_experts,
                },
                Self::RouterLogitCountNotDivisibleByExpertCount {
                    logit_len: right_logit_len,
                    n_experts: right_n_experts,
                },
            ) => left_logit_len == right_logit_len && left_n_experts == right_n_experts,
            (
                Self::RoutingProbabilityCountNotDivisibleByExpertCount {
                    probability_len: left_probability_len,
                    n_experts: left_n_experts,
                },
                Self::RoutingProbabilityCountNotDivisibleByExpertCount {
                    probability_len: right_probability_len,
                    n_experts: right_n_experts,
                },
            ) => left_probability_len == right_probability_len && left_n_experts == right_n_experts,
            (
                Self::ExpertAssignmentCountMismatch {
                    token_count: left_token_count,
                    assignment_count: left_assignment_count,
                },
                Self::ExpertAssignmentCountMismatch {
                    token_count: right_token_count,
                    assignment_count: right_assignment_count,
                },
            ) => {
                left_token_count == right_token_count
                    && left_assignment_count == right_assignment_count
            }
            (
                Self::ExpertAssignmentOutOfRange {
                    token_index: left_token_index,
                    assignment: left_assignment,
                    n_experts: left_n_experts,
                },
                Self::ExpertAssignmentOutOfRange {
                    token_index: right_token_index,
                    assignment: right_assignment,
                    n_experts: right_n_experts,
                },
            ) => {
                left_token_index == right_token_index
                    && left_assignment == right_assignment
                    && left_n_experts == right_n_experts
            }
            (
                Self::ExpertAssignmentNotTopProbability {
                    token_index: left_token_index,
                    assignment: left_assignment,
                    assigned_probability: left_assigned_probability,
                    max_probability: left_max_probability,
                },
                Self::ExpertAssignmentNotTopProbability {
                    token_index: right_token_index,
                    assignment: right_assignment,
                    assigned_probability: right_assigned_probability,
                    max_probability: right_max_probability,
                },
            ) => {
                left_token_index == right_token_index
                    && left_assignment == right_assignment
                    && float_error_value_eq(*left_assigned_probability, *right_assigned_probability)
                    && float_error_value_eq(*left_max_probability, *right_max_probability)
            }
            (
                Self::NonFiniteRouterLogit {
                    index: left_index,
                    value: left_value,
                },
                Self::NonFiniteRouterLogit {
                    index: right_index,
                    value: right_value,
                },
            )
            | (
                Self::NonFiniteRoutingProbability {
                    index: left_index,
                    value: left_value,
                },
                Self::NonFiniteRoutingProbability {
                    index: right_index,
                    value: right_value,
                },
            )
            | (
                Self::NegativeRoutingProbability {
                    index: left_index,
                    value: left_value,
                },
                Self::NegativeRoutingProbability {
                    index: right_index,
                    value: right_value,
                },
            ) => left_index == right_index && float_error_value_eq(*left_value, *right_value),
            (
                Self::InvalidRoutingProbabilitySum {
                    token_index: left_token_index,
                    sum: left_sum,
                },
                Self::InvalidRoutingProbabilitySum {
                    token_index: right_token_index,
                    sum: right_sum,
                },
            ) => {
                left_token_index == right_token_index && float_error_value_eq(*left_sum, *right_sum)
            }
            (
                Self::NonFiniteLoss { value: left_value },
                Self::NonFiniteLoss { value: right_value },
            ) => float_error_value_eq(*left_value, *right_value),
            (
                Self::NegativeLossWeight {
                    kind: left_kind,
                    value: left_value,
                },
                Self::NegativeLossWeight {
                    kind: right_kind,
                    value: right_value,
                },
            )
            | (
                Self::NonFiniteLossWeight {
                    kind: left_kind,
                    value: left_value,
                },
                Self::NonFiniteLossWeight {
                    kind: right_kind,
                    value: right_value,
                },
            ) => left_kind == right_kind && float_error_value_eq(*left_value, *right_value),
            #[cfg(feature = "burn-adapter")]
            (
                Self::InvalidRouterLogitShape { shape: left_shape },
                Self::InvalidRouterLogitShape { shape: right_shape },
            )
            | (
                Self::InvalidRoutingProbabilityShape { shape: left_shape },
                Self::InvalidRoutingProbabilityShape { shape: right_shape },
            ) => left_shape == right_shape,
            #[cfg(feature = "burn-adapter")]
            (Self::BurnAdapter(_), Self::BurnAdapter(_)) => false,
            _ => false,
        }
    }
}

impl fmt::Display for RouterLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRouterLogits => f.write_str("router logits must not be empty"),
            Self::EmptyRoutingProbabilities => {
                f.write_str("routing probabilities must not be empty")
            }
            Self::ZeroExpertCount => f.write_str("router n_experts must be greater than 0"),
            Self::RouterLogitCountNotDivisibleByExpertCount {
                logit_len,
                n_experts,
            } => write!(
                f,
                "router logits length {logit_len} is not divisible by n_experts {n_experts}"
            ),
            Self::RoutingProbabilityCountNotDivisibleByExpertCount {
                probability_len,
                n_experts,
            } => write!(
                f,
                "routing probability length {probability_len} is not divisible by n_experts {n_experts}"
            ),
            Self::ExpertAssignmentCountMismatch {
                token_count,
                assignment_count,
            } => write!(
                f,
                "expert assignments must match token rows, got {assignment_count} assignments for {token_count} tokens"
            ),
            Self::ExpertAssignmentOutOfRange {
                token_index,
                assignment,
                n_experts,
            } => write!(
                f,
                "expert assignment {assignment} at token {token_index} is outside n_experts {n_experts}"
            ),
            Self::ExpertAssignmentNotTopProbability {
                token_index,
                assignment,
                assigned_probability,
                max_probability,
            } => write!(
                f,
                "expert assignment {assignment} at token {token_index} has probability {assigned_probability}, below row max {max_probability}"
            ),
            Self::NonFiniteRouterLogit { index, value } => {
                write!(
                    f,
                    "router logit at index {index} must be finite, got {value}"
                )
            }
            Self::NonFiniteRoutingProbability { index, value } => write!(
                f,
                "routing probability at index {index} must be finite, got {value}"
            ),
            Self::NegativeRoutingProbability { index, value } => write!(
                f,
                "routing probability at index {index} must be non-negative, got {value}"
            ),
            Self::InvalidRoutingProbabilitySum { token_index, sum } => write!(
                f,
                "routing probabilities for token {token_index} must sum to 1.0, got {sum}"
            ),
            Self::NonFiniteLoss { value } => write!(f, "router loss must be finite, got {value}"),
            Self::NegativeLossWeight { kind, value } => {
                write!(f, "{kind} must be non-negative, got {value}")
            }
            Self::NonFiniteLossWeight { kind, value } => {
                write!(f, "{kind} must be finite, got {value}")
            }
            #[cfg(feature = "burn-adapter")]
            Self::InvalidRouterLogitShape { shape } => {
                write!(
                    f,
                    "router logits must be rank-2 [tokens, experts], got {shape:?}"
                )
            }
            #[cfg(feature = "burn-adapter")]
            Self::InvalidRoutingProbabilityShape { shape } => write!(
                f,
                "routing probabilities must be rank-2 [tokens, experts], got {shape:?}"
            ),
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => write!(f, "{error}"),
        }
    }
}

impl Error for RouterLossError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "burn-adapter")]
            Self::BurnAdapter(error) => Some(error),
            Self::EmptyRouterLogits
            | Self::EmptyRoutingProbabilities
            | Self::ZeroExpertCount
            | Self::RouterLogitCountNotDivisibleByExpertCount { .. }
            | Self::RoutingProbabilityCountNotDivisibleByExpertCount { .. }
            | Self::ExpertAssignmentCountMismatch { .. }
            | Self::ExpertAssignmentOutOfRange { .. }
            | Self::ExpertAssignmentNotTopProbability { .. }
            | Self::NonFiniteRouterLogit { .. }
            | Self::NonFiniteRoutingProbability { .. }
            | Self::NegativeRoutingProbability { .. }
            | Self::InvalidRoutingProbabilitySum { .. }
            | Self::NonFiniteLoss { .. }
            | Self::NegativeLossWeight { .. }
            | Self::NonFiniteLossWeight { .. } => None,
            #[cfg(feature = "burn-adapter")]
            Self::InvalidRouterLogitShape { .. } | Self::InvalidRoutingProbabilityShape { .. } => {
                None
            }
        }
    }
}

#[cfg(feature = "burn-adapter")]
impl From<BurnAdapterError> for RouterLossError {
    fn from(error: BurnAdapterError) -> Self {
        Self::BurnAdapter(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Top1RoutingBatch<'a> {
    routing_probs: &'a [f32],
    expert_assignments: &'a [usize],
    n_experts: usize,
}

impl<'a> Top1RoutingBatch<'a> {
    pub fn new(
        routing_probs: &'a [f32],
        expert_assignments: &'a [usize],
        n_experts: usize,
    ) -> Result<Self, RouterLossError> {
        validate_routing_contract(routing_probs, expert_assignments, n_experts)?;

        Ok(Self {
            routing_probs,
            expert_assignments,
            n_experts,
        })
    }

    #[must_use]
    pub const fn routing_probs(self) -> &'a [f32] {
        self.routing_probs
    }

    #[must_use]
    pub const fn expert_assignments(self) -> &'a [usize] {
        self.expert_assignments
    }

    #[must_use]
    pub const fn n_experts(self) -> usize {
        self.n_experts
    }

    #[must_use]
    pub fn token_count(self) -> usize {
        self.expert_assignments.len()
    }
}

/// Centered router z-loss over flattened `[token, expert]` rows.
///
/// The expert axis is the innermost axis of width `n_experts`; row losses are
/// averaged over all token rows. The `ln(n_experts)` center makes a uniform
/// zero-logit router contribute zero while still penalizing large logit scale.
pub fn router_z_loss(logits: &[f32], n_experts: usize) -> Result<f32, RouterLossError> {
    validate_router_logits(logits, n_experts)?;

    let row_count = logits.len() / n_experts;
    let uniform_log_partition = (n_experts as f32).ln();
    let mut loss_sum = 0.0;
    for row in logits.chunks_exact(n_experts) {
        let centered_z = stable_logsumexp(row) - uniform_log_partition;
        loss_sum += centered_z * centered_z;
    }

    normalize_router_loss(loss_sum / row_count as f32)
}

/// Top-1 Switch/standard MoE load-balance loss over flattened token rows.
///
/// `routing_probs` are flattened `[token, expert]` probabilities whose rows
/// must sum to one. `expert_assignments` are stop-gradient dispatch provenance
/// and must name the selected top-probability expert for each token. The
/// returned raw loss is `n_experts * sum_j fraction_tokens_to_j *
/// mean_probability_j`; gradients flow through `routing_probs`, not through the
/// hard assignment IDs.
pub fn load_balance_loss(
    routing_probs: &[f32],
    expert_assignments: &[usize],
    n_experts: usize,
) -> Result<f32, RouterLossError> {
    let batch = Top1RoutingBatch::new(routing_probs, expert_assignments, n_experts)?;

    load_balance_loss_for_top1_batch(batch)
}

pub fn load_balance_loss_for_top1_batch(
    batch: Top1RoutingBatch<'_>,
) -> Result<f32, RouterLossError> {
    let token_count = batch.token_count();
    let n_experts = batch.n_experts();
    let fractions = expert_assignment_fractions(batch.expert_assignments(), n_experts);
    let mut probability_means = vec![0.0; n_experts];
    for row in batch.routing_probs().chunks_exact(n_experts) {
        for (expert_index, probability) in row.iter().copied().enumerate() {
            probability_means[expert_index] += probability / token_count as f32;
        }
    }

    let loss = fractions
        .iter()
        .zip(probability_means.iter())
        .map(|(fraction, probability_mean)| fraction * probability_mean)
        .sum::<f32>()
        * n_experts as f32;

    normalize_router_loss(loss)
}

pub fn weighted_router_z_loss(
    raw_z_loss: f32,
    lambda_zrouter: f32,
) -> Result<f32, RouterLossError> {
    validate_loss_weight(RouterLossWeightKind::ZRouter, lambda_zrouter)?;
    normalize_router_loss(raw_z_loss * lambda_zrouter)
}

pub fn weighted_load_balance_loss(
    raw_balance_loss: f32,
    lambda_balance: f32,
) -> Result<f32, RouterLossError> {
    validate_loss_weight(RouterLossWeightKind::Balance, lambda_balance)?;
    normalize_router_loss(raw_balance_loss * lambda_balance)
}

/// Configuration predicate for the router-loss composer.
///
/// This helper does not prove that a training loop has adopted the loss terms;
/// the caller that composes total loss and emits metrics owns that integration.
#[must_use]
pub const fn router_losses_enabled_for_phase(phase: TrainPhaseKind) -> bool {
    matches!(
        phase,
        TrainPhaseKind::RouterWarmup
            | TrainPhaseKind::ExpertTernaryQat
            | TrainPhaseKind::FullNumericQat
            | TrainPhaseKind::HardenAndSelect
    )
}

#[must_use]
pub fn lambda_balance_for_phase(phase: TrainPhaseKind, configured_lambda_balance: f32) -> f32 {
    if router_losses_enabled_for_phase(phase) {
        configured_lambda_balance
    } else {
        0.0
    }
}

#[must_use]
pub fn lambda_zrouter_for_phase(phase: TrainPhaseKind, configured_lambda_zrouter: f32) -> f32 {
    if router_losses_enabled_for_phase(phase) {
        configured_lambda_zrouter
    } else {
        0.0
    }
}

#[cfg(feature = "burn-adapter")]
pub fn burn_router_z_loss<B>(
    logits: BurnFloatTensor<B, 2>,
) -> Result<BurnFloatTensor<B, 1>, RouterLossError>
where
    B: BurnBackend,
{
    let shape = float_tensor_shape(&logits);
    validate_burn_router_logits_shape(shape)?;
    validate_burn_router_logits(&logits)?;

    let n_experts = shape[1];
    let max_per_row = logits.clone().max_dim(1).detach();
    let shifted = logits - max_per_row.clone().repeat_dim(1, n_experts);
    let z = max_per_row + shifted.exp().sum_dim(1).log();
    let centered_z = z - (n_experts as f32).ln();
    let loss = (centered_z.clone() * centered_z).mean();
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_load_balance_loss<B>(
    routing_probs: BurnFloatTensor<B, 2>,
    expert_assignments: &[usize],
    device: &BurnDevice<B>,
) -> Result<BurnFloatTensor<B, 1>, RouterLossError>
where
    B: BurnBackend,
{
    let shape = float_tensor_shape(&routing_probs);
    validate_burn_routing_probability_shape(shape)?;
    let n_experts = shape[1];
    validate_burn_routing_probabilities(&routing_probs, expert_assignments, n_experts)?;

    let fractions = expert_assignment_fractions(expert_assignments, n_experts);
    let fractions_tensor = float_tensor_from_vec::<B, 1>(fractions, [n_experts], device)?;
    let mean_probabilities = routing_probs.mean_dim(0).reshape([n_experts]);
    let loss = (mean_probabilities * fractions_tensor).sum() * n_experts as f32;
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_weighted_router_z_loss<B, const D: usize>(
    raw_z_loss: BurnFloatTensor<B, D>,
    lambda_zrouter: f32,
) -> Result<BurnFloatTensor<B, D>, RouterLossError>
where
    B: BurnBackend,
{
    validate_loss_weight(RouterLossWeightKind::ZRouter, lambda_zrouter)?;
    let loss = raw_z_loss * lambda_zrouter;
    validate_burn_loss(&loss)?;

    Ok(loss)
}

#[cfg(feature = "burn-adapter")]
pub fn burn_weighted_load_balance_loss<B, const D: usize>(
    raw_balance_loss: BurnFloatTensor<B, D>,
    lambda_balance: f32,
) -> Result<BurnFloatTensor<B, D>, RouterLossError>
where
    B: BurnBackend,
{
    validate_loss_weight(RouterLossWeightKind::Balance, lambda_balance)?;
    let loss = raw_balance_loss * lambda_balance;
    validate_burn_loss(&loss)?;

    Ok(loss)
}

fn validate_router_logits(logits: &[f32], n_experts: usize) -> Result<(), RouterLossError> {
    if logits.is_empty() {
        return Err(RouterLossError::EmptyRouterLogits);
    }

    validate_expert_count(n_experts)?;

    if !logits.len().is_multiple_of(n_experts) {
        return Err(RouterLossError::RouterLogitCountNotDivisibleByExpertCount {
            logit_len: logits.len(),
            n_experts,
        });
    }

    for (index, &value) in logits.iter().enumerate() {
        if !value.is_finite() {
            return Err(RouterLossError::NonFiniteRouterLogit { index, value });
        }
    }

    Ok(())
}

fn validate_routing_contract(
    routing_probs: &[f32],
    expert_assignments: &[usize],
    n_experts: usize,
) -> Result<(), RouterLossError> {
    if routing_probs.is_empty() {
        return Err(RouterLossError::EmptyRoutingProbabilities);
    }

    validate_expert_count(n_experts)?;

    if !routing_probs.len().is_multiple_of(n_experts) {
        return Err(
            RouterLossError::RoutingProbabilityCountNotDivisibleByExpertCount {
                probability_len: routing_probs.len(),
                n_experts,
            },
        );
    }

    let token_count = routing_probs.len() / n_experts;
    if expert_assignments.len() != token_count {
        return Err(RouterLossError::ExpertAssignmentCountMismatch {
            token_count,
            assignment_count: expert_assignments.len(),
        });
    }

    validate_expert_assignments(expert_assignments, n_experts)?;
    validate_routing_probabilities(routing_probs, expert_assignments, n_experts)
}

fn validate_expert_count(n_experts: usize) -> Result<(), RouterLossError> {
    if n_experts == 0 {
        return Err(RouterLossError::ZeroExpertCount);
    }

    Ok(())
}

fn validate_expert_assignments(
    expert_assignments: &[usize],
    n_experts: usize,
) -> Result<(), RouterLossError> {
    for (token_index, &assignment) in expert_assignments.iter().enumerate() {
        if assignment >= n_experts {
            return Err(RouterLossError::ExpertAssignmentOutOfRange {
                token_index,
                assignment,
                n_experts,
            });
        }
    }

    Ok(())
}

fn validate_routing_probabilities(
    routing_probs: &[f32],
    expert_assignments: &[usize],
    n_experts: usize,
) -> Result<(), RouterLossError> {
    for (index, &value) in routing_probs.iter().enumerate() {
        if !value.is_finite() {
            return Err(RouterLossError::NonFiniteRoutingProbability { index, value });
        }
        if value < 0.0 {
            return Err(RouterLossError::NegativeRoutingProbability { index, value });
        }
    }

    for (token_index, row) in routing_probs.chunks_exact(n_experts).enumerate() {
        let sum = row.iter().sum::<f32>();
        if (sum - 1.0).abs() > ROUTING_PROBABILITY_SUM_TOLERANCE {
            return Err(RouterLossError::InvalidRoutingProbabilitySum { token_index, sum });
        }

        let assignment = expert_assignments[token_index];
        let assigned_probability = row[assignment];
        let max_probability = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if assigned_probability + ROUTING_PROBABILITY_ARGMAX_TOLERANCE < max_probability {
            return Err(RouterLossError::ExpertAssignmentNotTopProbability {
                token_index,
                assignment,
                assigned_probability,
                max_probability,
            });
        }
    }

    Ok(())
}

fn validate_loss_weight(kind: RouterLossWeightKind, value: f32) -> Result<(), RouterLossError> {
    if !value.is_finite() {
        return Err(RouterLossError::NonFiniteLossWeight { kind, value });
    }

    if value < 0.0 {
        return Err(RouterLossError::NegativeLossWeight { kind, value });
    }

    Ok(())
}

fn stable_logsumexp(values: &[f32]) -> f32 {
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum = values.iter().map(|value| (value - max).exp()).sum::<f32>();

    max + exp_sum.ln()
}

fn expert_assignment_fractions(expert_assignments: &[usize], n_experts: usize) -> Vec<f32> {
    let mut counts = vec![0.0; n_experts];
    for &assignment in expert_assignments {
        counts[assignment] += 1.0;
    }

    let token_count = expert_assignments.len() as f32;
    counts
        .into_iter()
        .map(|count| count / token_count)
        .collect()
}

fn normalize_router_loss(value: f32) -> Result<f32, RouterLossError> {
    if !value.is_finite() {
        return Err(RouterLossError::NonFiniteLoss { value });
    }

    Ok(value)
}

fn float_error_value_eq(left: f32, right: f32) -> bool {
    left == right || left.is_nan() && right.is_nan()
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_router_logits_shape(shape: [usize; 2]) -> Result<(), RouterLossError> {
    if shape[0] == 0 || shape[1] == 0 {
        return Err(RouterLossError::InvalidRouterLogitShape {
            shape: shape.to_vec(),
        });
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_routing_probability_shape(shape: [usize; 2]) -> Result<(), RouterLossError> {
    if shape[0] == 0 || shape[1] == 0 {
        return Err(RouterLossError::InvalidRoutingProbabilityShape {
            shape: shape.to_vec(),
        });
    }

    Ok(())
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_router_logits<B>(logits: &BurnFloatTensor<B, 2>) -> Result<(), RouterLossError>
where
    B: BurnBackend,
{
    let shape = float_tensor_shape(logits);
    let values = float_tensor_into_vec(logits.clone().detach())?;
    validate_router_logits(&values, shape[1])
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_routing_probabilities<B>(
    routing_probs: &BurnFloatTensor<B, 2>,
    expert_assignments: &[usize],
    n_experts: usize,
) -> Result<(), RouterLossError>
where
    B: BurnBackend,
{
    let values = float_tensor_into_vec(routing_probs.clone().detach())?;
    validate_routing_contract(&values, expert_assignments, n_experts)
}

#[cfg(feature = "burn-adapter")]
fn validate_burn_loss<B, const D: usize>(
    loss: &BurnFloatTensor<B, D>,
) -> Result<(), RouterLossError>
where
    B: BurnBackend,
{
    for value in float_tensor_into_vec(loss.clone().detach())? {
        normalize_router_loss(value)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {actual} to be within {tolerance} of {expected}"
        );
    }

    #[test]
    fn router_z_loss_is_zero_for_all_zero_logits() {
        let loss = router_z_loss(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3).unwrap();

        assert_close(loss, 0.0, 1.0e-7);
    }

    #[test]
    fn router_z_loss_matches_centered_logsumexp_oracle() {
        let logits = [1.0, 0.0, -1.0, 2.0, 1.0, 0.0];

        let loss = router_z_loss(&logits, 3).unwrap();

        assert_close(loss, 0.904_470_56, 1.0e-6);
    }

    #[test]
    fn router_z_loss_increases_with_logit_magnitude() {
        let small = router_z_loss(&[0.25, 0.0, 0.0], 3).unwrap();
        let large = router_z_loss(&[2.0, 0.0, 0.0], 3).unwrap();

        assert!(small > 0.0);
        assert!(large > small);
    }

    #[test]
    fn router_z_loss_reduces_as_mean_over_token_rows() {
        let row_a = router_z_loss(&[1.0, 0.0, -1.0], 3).unwrap();
        let row_b = router_z_loss(&[2.0, 1.0, 0.0], 3).unwrap();
        let batched = router_z_loss(&[1.0, 0.0, -1.0, 2.0, 1.0, 0.0], 3).unwrap();

        assert_close(batched, (row_a + row_b) / 2.0, 1.0e-6);
    }

    #[test]
    fn load_balance_loss_is_minimized_at_uniform_routing() {
        let uniform = load_balance_loss(&[0.5, 0.5, 0.5, 0.5], &[0, 1], 2).unwrap();
        let collapsed = load_balance_loss(&[0.9, 0.1, 0.8, 0.2], &[0, 0], 2).unwrap();

        assert_close(uniform, 1.0, 1.0e-6);
        assert_close(collapsed, 1.7, 1.0e-6);
        assert!(uniform < collapsed);
    }

    #[test]
    fn load_balance_loss_uses_fraction_times_mean_probability() {
        let routing_probs = [
            0.8, 0.2, 0.0, //
            0.6, 0.4, 0.0, //
            0.1, 0.7, 0.2, //
            0.2, 0.3, 0.5, //
        ];
        let assignments = [0, 0, 1, 2];
        let batch = Top1RoutingBatch::new(&routing_probs, &assignments, 3).unwrap();

        let loss = load_balance_loss_for_top1_batch(batch).unwrap();

        assert_close(loss, 1.068_749_9, 1.0e-6);
    }

    #[test]
    fn weighted_router_losses_apply_explicit_lambdas() {
        assert_close(weighted_router_z_loss(2.0, 0.001).unwrap(), 0.002, 1.0e-9);
        assert_close(
            weighted_load_balance_loss(1.25, 0.01).unwrap(),
            0.0125,
            1.0e-9,
        );
        assert_eq!(
            weighted_router_z_loss(f32::MAX, 2.0).unwrap_err(),
            RouterLossError::NonFiniteLoss {
                value: f32::INFINITY,
            }
        );
    }

    #[test]
    fn router_losses_are_enabled_from_router_warmup_onward() {
        assert!(!router_losses_enabled_for_phase(
            TrainPhaseKind::DenseTeacherWarmup
        ));
        assert!(router_losses_enabled_for_phase(
            TrainPhaseKind::RouterWarmup
        ));
        assert!(router_losses_enabled_for_phase(
            TrainPhaseKind::ExpertTernaryQat
        ));
        assert!(router_losses_enabled_for_phase(
            TrainPhaseKind::FullNumericQat
        ));
        assert!(router_losses_enabled_for_phase(
            TrainPhaseKind::HardenAndSelect
        ));
        assert_close(
            lambda_balance_for_phase(TrainPhaseKind::DenseTeacherWarmup, 0.01),
            0.0,
            0.0,
        );
        assert_close(
            lambda_zrouter_for_phase(TrainPhaseKind::RouterWarmup, 0.001),
            0.001,
            0.0,
        );
        assert_close(
            lambda_balance_for_phase(TrainPhaseKind::HardenAndSelect, 0.01),
            0.01,
            0.0,
        );
    }

    #[test]
    fn router_loss_validation_rejects_invalid_contracts() {
        assert_eq!(
            router_z_loss(&[], 2).unwrap_err(),
            RouterLossError::EmptyRouterLogits
        );
        assert_eq!(
            router_z_loss(&[1.0, 2.0, 3.0], 2).unwrap_err(),
            RouterLossError::RouterLogitCountNotDivisibleByExpertCount {
                logit_len: 3,
                n_experts: 2,
            }
        );
        assert_eq!(
            router_z_loss(&[f32::NAN], 1).unwrap_err(),
            RouterLossError::NonFiniteRouterLogit {
                index: 0,
                value: f32::NAN,
            }
        );
        assert_eq!(
            load_balance_loss(&[], &[], 2).unwrap_err(),
            RouterLossError::EmptyRoutingProbabilities
        );
        assert_eq!(
            load_balance_loss(&[0.5, 0.5], &[], 2).unwrap_err(),
            RouterLossError::ExpertAssignmentCountMismatch {
                token_count: 1,
                assignment_count: 0,
            }
        );
        assert_eq!(
            load_balance_loss(&[0.5, 0.5], &[2], 2).unwrap_err(),
            RouterLossError::ExpertAssignmentOutOfRange {
                token_index: 0,
                assignment: 2,
                n_experts: 2,
            }
        );
        assert_eq!(
            load_balance_loss(&[0.5, f32::NAN], &[0], 2).unwrap_err(),
            RouterLossError::NonFiniteRoutingProbability {
                index: 1,
                value: f32::NAN,
            }
        );
        assert_eq!(
            load_balance_loss(&[0.5, -0.1], &[0], 2).unwrap_err(),
            RouterLossError::NegativeRoutingProbability {
                index: 1,
                value: -0.1,
            }
        );
        assert_eq!(
            load_balance_loss(&[0.7, 0.2], &[0], 2).unwrap_err(),
            RouterLossError::InvalidRoutingProbabilitySum {
                token_index: 0,
                sum: 0.9,
            }
        );
        assert_eq!(
            load_balance_loss(&[0.4, 0.6], &[0], 2).unwrap_err(),
            RouterLossError::ExpertAssignmentNotTopProbability {
                token_index: 0,
                assignment: 0,
                assigned_probability: 0.4,
                max_probability: 0.6,
            }
        );
        assert_eq!(
            weighted_router_z_loss(1.0, -0.1).unwrap_err(),
            RouterLossError::NegativeLossWeight {
                kind: RouterLossWeightKind::ZRouter,
                value: -0.1,
            }
        );
    }

    #[cfg(feature = "burn-adapter")]
    mod burn_tests {
        use super::*;
        use crate::adapter::burn::{
            BurnDevice, BurnNdArrayAutodiffBackend, burn_softmax, float_tensor_from_vec,
            float_tensor_into_vec,
        };

        type B = BurnNdArrayAutodiffBackend;

        #[test]
        fn burn_router_z_loss_matches_scalar_oracle_for_batched_logits() {
            let device = BurnDevice::<B>::default();
            let values = vec![1.0, 0.0, -1.0, 2.0, 1.0, 0.0];
            let logits = float_tensor_from_vec::<B, 2>(values.clone(), [2, 3], &device).unwrap();

            let burn_loss = burn_router_z_loss(logits).unwrap();
            let scalar_loss = router_z_loss(&values, 3).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-5,
            );
        }

        #[test]
        fn burn_load_balance_loss_matches_scalar_oracle_for_batched_probs() {
            let device = BurnDevice::<B>::default();
            let values = vec![
                0.8, 0.2, 0.0, //
                0.6, 0.4, 0.0, //
                0.1, 0.7, 0.2, //
                0.2, 0.3, 0.5, //
            ];
            let probs = float_tensor_from_vec::<B, 2>(values.clone(), [4, 3], &device).unwrap();

            let burn_loss = burn_load_balance_loss(probs, &[0, 0, 1, 2], &device).unwrap();
            let scalar_loss = load_balance_loss(&values, &[0, 0, 1, 2], 3).unwrap();

            assert_close(
                float_tensor_into_vec(burn_loss).unwrap()[0],
                scalar_loss,
                1.0e-5,
            );
        }

        #[test]
        fn burn_router_z_loss_flows_gradient_to_logits() {
            let device = BurnDevice::<B>::default();
            let logits = float_tensor_from_vec::<B, 2>(
                vec![1.0, 0.0, -1.0, 0.5, 0.25, -0.25],
                [2, 3],
                &device,
            )
            .unwrap()
            .require_grad();

            let loss = burn_router_z_loss(logits.clone()).unwrap();
            let gradients = loss.backward();
            let grad = logits
                .grad(&gradients)
                .expect("router logits should receive gradients");

            assert!(
                float_tensor_into_vec(grad)
                    .unwrap()
                    .iter()
                    .any(|value| value.abs() > 0.0)
            );
        }

        #[test]
        fn burn_load_balance_loss_flows_gradient_to_routing_probs() {
            let device = BurnDevice::<B>::default();
            let probs = float_tensor_from_vec::<B, 2>(vec![0.8, 0.2, 0.25, 0.75], [2, 2], &device)
                .unwrap()
                .require_grad();

            let loss = burn_load_balance_loss(probs.clone(), &[0, 1], &device).unwrap();
            let gradients = loss.backward();
            let grad = probs
                .grad(&gradients)
                .expect("routing probabilities should receive gradients");

            assert!(
                float_tensor_into_vec(grad)
                    .unwrap()
                    .iter()
                    .any(|value| value.abs() > 0.0)
            );
        }

        #[test]
        fn burn_load_balance_loss_flows_gradient_through_softmax_logits() {
            let device = BurnDevice::<B>::default();
            let logits = float_tensor_from_vec::<B, 2>(vec![2.0, 0.0, 1.5, 0.0], [2, 2], &device)
                .unwrap()
                .require_grad();
            let probs = burn_softmax(logits.clone(), 1);

            let loss = burn_load_balance_loss(probs, &[0, 0], &device).unwrap();
            let gradients = loss.backward();
            let grad = logits
                .grad(&gradients)
                .expect("router logits should receive load-balance gradients through softmax");

            assert!(
                float_tensor_into_vec(grad)
                    .unwrap()
                    .iter()
                    .any(|value| value.abs() > 0.0)
            );
        }

        #[test]
        fn burn_router_z_loss_rejects_non_finite_result() {
            let device = BurnDevice::<B>::default();
            let logits =
                float_tensor_from_vec::<B, 2>(vec![f32::MAX, 0.0], [1, 2], &device).unwrap();

            assert_eq!(
                burn_router_z_loss(logits).unwrap_err(),
                RouterLossError::NonFiniteLoss {
                    value: f32::INFINITY,
                }
            );
        }

        #[test]
        fn burn_weighted_losses_reject_non_finite_results() {
            let device = BurnDevice::<B>::default();
            let raw_loss = float_tensor_from_vec::<B, 1>(vec![f32::MAX], [1], &device).unwrap();

            assert_eq!(
                burn_weighted_router_z_loss(raw_loss, 2.0).unwrap_err(),
                RouterLossError::NonFiniteLoss {
                    value: f32::INFINITY,
                }
            );
        }
    }
}
