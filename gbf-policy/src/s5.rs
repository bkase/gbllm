//! F-S5 outcome algebra, frontier policy, and dispatch helpers.
//!
//! This module maps already-evaluated S5 hypothesis statuses and aggregate
//! frontier metrics into the RFC policy algebra. It does not run experiments
//! or emit the full `s5_frontier.v1` artifact; those integration paths are
//! owned by later beads.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

use crate::canonical::canonical_json_bytes;
use serde::{Deserialize, Serialize};

pub const S5_OUTCOME_VARIANT_COUNT: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisStatus {
    Confirmed,
    Refuted,
    NotEvaluatedDueToPriorGate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrontierRecommendation {
    #[serde(rename = "A")]
    A,
    #[serde(rename = "B")]
    B,
    #[serde(rename = "Tie")]
    Tie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S5FrontierVariantId {
    #[serde(rename = "boundedkv")]
    BoundedKv,
    #[serde(rename = "linearstate_fixed_0_5")]
    LFix1,
    #[serde(rename = "linearstate_mt4")]
    LMt4,
}

impl S5FrontierVariantId {
    pub const ALL: [Self; 3] = [Self::BoundedKv, Self::LFix1, Self::LMt4];
    pub const LINEAR_STATE: [Self; 2] = [Self::LFix1, Self::LMt4];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BoundedKv => "boundedkv",
            Self::LFix1 => "linearstate_fixed_0_5",
            Self::LMt4 => "linearstate_mt4",
        }
    }

    #[must_use]
    pub const fn is_linear_state(self) -> bool {
        matches!(self, Self::LFix1 | Self::LMt4)
    }
}

pub const S5_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];
pub const S5_PICK_SHADOW_CADENCE_STEPS: [u32; 5] = [4000, 8000, 12000, 16000, 20000];
pub const S5_FIT_SHADOW_REAL_CADENCE_STEPS: [u32; 5] = [4000, 8000, 12000, 16000, 20000];
pub const S5_FIT_FEEDBACK_PHASE_BOUNDARIES: [u32; 4] = [6000, 6001, 12000, 20000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum S5StateMachineEvent {
    PickRunning {
        variant: S5FrontierVariantId,
        seed: u64,
    },
    PickEvalDone {
        variant: S5FrontierVariantId,
        seed: u64,
    },
    PickScoreDone {
        variant: S5FrontierVariantId,
        seed: u64,
    },
    PickAttentionOracleDone {
        seed: u64,
    },
    PickShadowDone {
        variant: S5FrontierVariantId,
        seed: u64,
        cadence_step: u32,
    },
    PickFrontierInputsFrozen,
    FitPreflightDone {
        seed: u64,
    },
    FitShadowRealDone {
        seed: u64,
        cadence_step: u32,
    },
    FitFeedbackApplied {
        seed: u64,
        phase_boundary: u32,
    },
    FitExportDone {
        seed: u64,
    },
    FitRevalidationDone {
        seed: u64,
    },
    FitEncodedRomEmitted {
        seed: u64,
    },
    FitHarnessRun {
        seed: u64,
    },
    FrontierEmitted,
}

#[must_use]
pub fn s5_nominal_state_machine_events() -> Vec<S5StateMachineEvent> {
    let mut events = Vec::new();

    for variant in S5FrontierVariantId::ALL {
        for seed in S5_SEEDS {
            events.push(S5StateMachineEvent::PickRunning { variant, seed });
            events.push(S5StateMachineEvent::PickEvalDone { variant, seed });
            events.push(S5StateMachineEvent::PickScoreDone { variant, seed });
            if variant == S5FrontierVariantId::BoundedKv {
                events.push(S5StateMachineEvent::PickAttentionOracleDone { seed });
            }
            for cadence_step in S5_PICK_SHADOW_CADENCE_STEPS {
                events.push(S5StateMachineEvent::PickShadowDone {
                    variant,
                    seed,
                    cadence_step,
                });
            }
        }
    }

    events.push(S5StateMachineEvent::PickFrontierInputsFrozen);

    for seed in S5_SEEDS {
        events.push(S5StateMachineEvent::FitPreflightDone { seed });
        for cadence_step in S5_FIT_SHADOW_REAL_CADENCE_STEPS {
            events.push(S5StateMachineEvent::FitShadowRealDone { seed, cadence_step });
        }
        for phase_boundary in S5_FIT_FEEDBACK_PHASE_BOUNDARIES {
            events.push(S5StateMachineEvent::FitFeedbackApplied {
                seed,
                phase_boundary,
            });
        }
        events.push(S5StateMachineEvent::FitExportDone { seed });
        events.push(S5StateMachineEvent::FitRevalidationDone { seed });
        events.push(S5StateMachineEvent::FitEncodedRomEmitted { seed });
        if seed == 0 {
            events.push(S5StateMachineEvent::FitHarnessRun { seed });
        }
    }

    events.push(S5StateMachineEvent::FrontierEmitted);
    events
}

pub type FrontierLeaderVariant = Option<S5FrontierVariantId>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S5FrontierAxis {
    ValBpcFp,
    ValBpcTernary,
    TernaryGap,
    V0SuccessPass,
    V0SuccessScore,
    ParamCount,
    ProjectedDeployedBytes,
    ShadowCompileOkAtEnd,
    ShadowByteCostAtEnd,
    ShadowKernelCountAtEnd,
    LatencyProxyCycles,
    EncodedRomByteCost,
    FitsEnvelope,
    ReachabilityCertValid,
    ResourceStateCertValid,
}

pub const S5_FRONTIER_LOWER_IS_BETTER_AXES: [S5FrontierAxis; 9] = [
    S5FrontierAxis::ValBpcFp,
    S5FrontierAxis::ValBpcTernary,
    S5FrontierAxis::TernaryGap,
    S5FrontierAxis::ParamCount,
    S5FrontierAxis::ProjectedDeployedBytes,
    S5FrontierAxis::ShadowByteCostAtEnd,
    S5FrontierAxis::ShadowKernelCountAtEnd,
    S5FrontierAxis::LatencyProxyCycles,
    S5FrontierAxis::EncodedRomByteCost,
];

pub const S5_FRONTIER_HIGHER_IS_BETTER_AXES: [S5FrontierAxis; 1] = [S5FrontierAxis::V0SuccessScore];

pub const S5_FRONTIER_BOOLEAN_MUST_BE_TRUE_AXES: [S5FrontierAxis; 5] = [
    S5FrontierAxis::V0SuccessPass,
    S5FrontierAxis::ShadowCompileOkAtEnd,
    S5FrontierAxis::FitsEnvelope,
    S5FrontierAxis::ReachabilityCertValid,
    S5FrontierAxis::ResourceStateCertValid,
];

pub const S5_FRONTIER_DEFAULT_AXES: [S5FrontierAxis; 15] = [
    S5FrontierAxis::ValBpcFp,
    S5FrontierAxis::ValBpcTernary,
    S5FrontierAxis::TernaryGap,
    S5FrontierAxis::V0SuccessPass,
    S5FrontierAxis::V0SuccessScore,
    S5FrontierAxis::ParamCount,
    S5FrontierAxis::ProjectedDeployedBytes,
    S5FrontierAxis::ShadowCompileOkAtEnd,
    S5FrontierAxis::ShadowByteCostAtEnd,
    S5FrontierAxis::ShadowKernelCountAtEnd,
    S5FrontierAxis::LatencyProxyCycles,
    S5FrontierAxis::EncodedRomByteCost,
    S5FrontierAxis::FitsEnvelope,
    S5FrontierAxis::ReachabilityCertValid,
    S5FrontierAxis::ResourceStateCertValid,
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FrontierPointMetrics {
    pub val_bpc_fp: f64,
    pub val_bpc_ternary: f64,
    pub ternary_gap: f64,
    pub v0_success_pass: bool,
    pub v0_success_score: f64,
    pub param_count: u64,
    pub projected_deployed_bytes: u64,
    pub shadow_compile_ok_at_end: bool,
    pub shadow_byte_cost_at_end: u32,
    pub shadow_kernel_count_at_end: u32,
    pub latency_proxy_cycles: u64,
    pub encoded_rom_byte_cost: Option<u64>,
    pub fits_envelope: Option<bool>,
    pub reachability_cert_valid: Option<bool>,
    pub resource_state_cert_valid: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct S5FrontierPointCollections {
    pick_points: Vec<S5FrontierPointMetrics>,
    fit_points: Vec<S5FrontierPointMetrics>,
}

impl S5FrontierPointCollections {
    pub fn new(
        pick_points: Vec<S5FrontierPointMetrics>,
        fit_points: Vec<S5FrontierPointMetrics>,
    ) -> Result<Self, S5FrontierPointSplitError> {
        validate_pick_points_fit_fields_are_null(&pick_points)?;
        validate_fit_points_fit_fields_are_present(&fit_points)?;
        Ok(Self {
            pick_points,
            fit_points,
        })
    }

    #[must_use]
    pub fn pick_points(&self) -> &[S5FrontierPointMetrics] {
        &self.pick_points
    }

    #[must_use]
    pub fn fit_points(&self) -> &[S5FrontierPointMetrics] {
        &self.fit_points
    }

    pub fn fit_pareto_frontier_indices(
        &self,
        axes: &[S5FrontierAxis],
    ) -> Result<Vec<usize>, S5FrontierValidationError> {
        s5_pareto_frontier_indices(&self.fit_points, axes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S5FrontierPointSplitError {
    PickPointHasFitOnlyField {
        point_index: usize,
        axis: S5FrontierAxis,
    },
    FitPointMissingFitOnlyField {
        point_index: usize,
        axis: S5FrontierAxis,
    },
}

impl fmt::Display for S5FrontierPointSplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PickPointHasFitOnlyField { point_index, axis } => write!(
                f,
                "pick_points[{point_index}] must leave Fit-only axis {axis:?} null"
            ),
            Self::FitPointMissingFitOnlyField { point_index, axis } => write!(
                f,
                "fit_points[{point_index}] must populate Fit-only axis {axis:?}"
            ),
        }
    }
}

impl Error for S5FrontierPointSplitError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum S5FrontierValidationError {
    NonFiniteAxis {
        point_index: usize,
        axis: S5FrontierAxis,
        value: f64,
    },
}

impl fmt::Display for S5FrontierValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteAxis {
                point_index,
                axis,
                value,
            } => write!(
                f,
                "frontier point {point_index} has non-finite {axis:?} value {value}"
            ),
        }
    }
}

impl Error for S5FrontierValidationError {}

#[must_use]
pub fn s5_frontier_boolean_gates_pass(point: &S5FrontierPointMetrics) -> bool {
    point.v0_success_pass
        && point.shadow_compile_ok_at_end
        && point.fits_envelope.unwrap_or(true)
        && point.reachability_cert_valid.unwrap_or(true)
        && point.resource_state_cert_valid.unwrap_or(true)
}

#[must_use]
pub fn s5_frontier_numeric_axes_are_finite(point: &S5FrontierPointMetrics) -> bool {
    first_non_finite_frontier_axis(point).is_none()
}

#[must_use]
pub fn first_non_finite_frontier_axis(
    point: &S5FrontierPointMetrics,
) -> Option<(S5FrontierAxis, f64)> {
    [
        (S5FrontierAxis::ValBpcFp, point.val_bpc_fp),
        (S5FrontierAxis::ValBpcTernary, point.val_bpc_ternary),
        (S5FrontierAxis::TernaryGap, point.ternary_gap),
        (S5FrontierAxis::V0SuccessScore, point.v0_success_score),
    ]
    .into_iter()
    .find(|(_, value)| !value.is_finite())
}

#[must_use]
pub fn s5_frontier_dominates(
    candidate: &S5FrontierPointMetrics,
    incumbent: &S5FrontierPointMetrics,
    axes: &[S5FrontierAxis],
) -> bool {
    let mut strictly_better = false;

    for axis in axes {
        match compare_frontier_axis(*axis, candidate, incumbent) {
            AxisComparison::Worse => return false,
            AxisComparison::Invalid => return false,
            AxisComparison::Better => strictly_better = true,
            AxisComparison::Equal | AxisComparison::Skipped => {}
        }
    }

    strictly_better
}

pub fn s5_pareto_frontier_indices(
    points: &[S5FrontierPointMetrics],
    axes: &[S5FrontierAxis],
) -> Result<Vec<usize>, S5FrontierValidationError> {
    validate_frontier_points_are_finite(points)?;

    Ok(s5_pareto_frontier_indices_unchecked(points, axes))
}

fn s5_pareto_frontier_indices_unchecked(
    points: &[S5FrontierPointMetrics],
    axes: &[S5FrontierAxis],
) -> Vec<usize> {
    points
        .iter()
        .enumerate()
        .filter(|(_, point)| s5_frontier_boolean_gates_pass(point))
        .filter(|(index, point)| {
            !points.iter().enumerate().any(|(other_index, other)| {
                other_index != *index
                    && s5_frontier_boolean_gates_pass(other)
                    && s5_frontier_dominates(other, point, axes)
            })
        })
        .map(|(index, _)| index)
        .collect()
}

fn validate_frontier_points_are_finite(
    points: &[S5FrontierPointMetrics],
) -> Result<(), S5FrontierValidationError> {
    for (point_index, point) in points.iter().enumerate() {
        if let Some((axis, value)) = first_non_finite_frontier_axis(point) {
            return Err(S5FrontierValidationError::NonFiniteAxis {
                point_index,
                axis,
                value,
            });
        }
    }
    Ok(())
}

pub const S5_FRONTIER_RECOMMENDATION_MARGIN_BPC: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FrontierVariantRecord {
    pub variant: S5FrontierVariantId,
    pub aggregate: S5FrontierPointMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FrontierRecommendationResult {
    pub frontier_recommendation: FrontierRecommendation,
    pub frontier_leader_variant: FrontierLeaderVariant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FrontierRecommendationReport {
    pub variant_records: Vec<S5FrontierVariantRecord>,
    pub frontier_recommendation: FrontierRecommendation,
    pub frontier_leader_variant: FrontierLeaderVariant,
}

impl S5FrontierRecommendationReport {
    pub fn from_variant_records(
        variant_records: Vec<S5FrontierVariantRecord>,
    ) -> Result<Self, S5FrontierRecommendationError> {
        let result = s5_frontier_recommendation(&variant_records)?;
        Ok(Self {
            variant_records,
            frontier_recommendation: result.frontier_recommendation,
            frontier_leader_variant: result.frontier_leader_variant,
        })
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        canonical_json_bytes(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S5FrontierRecommendationError {
    MissingVariantRecord { variant: S5FrontierVariantId },
    DuplicateVariantRecord { variant: S5FrontierVariantId },
    NoRecommendationBecauseVariantGateFailed,
}

impl fmt::Display for S5FrontierRecommendationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVariantRecord { variant } => {
                write!(
                    f,
                    "missing S5 frontier variant record for {}",
                    variant.as_str()
                )
            }
            Self::DuplicateVariantRecord { variant } => {
                write!(
                    f,
                    "duplicate S5 frontier variant record for {}",
                    variant.as_str()
                )
            }
            Self::NoRecommendationBecauseVariantGateFailed => write!(
                f,
                "neither A nor B holds and at least one variant failed the Tie gates"
            ),
        }
    }
}

impl Error for S5FrontierRecommendationError {}

pub fn s5_frontier_recommendation(
    records: &[S5FrontierVariantRecord],
) -> Result<S5FrontierRecommendationResult, S5FrontierRecommendationError> {
    let bounded_kv = unique_variant_record(records, S5FrontierVariantId::BoundedKv)?;
    let l_fix1 = unique_variant_record(records, S5FrontierVariantId::LFix1)?;
    let l_mt4 = unique_variant_record(records, S5FrontierVariantId::LMt4)?;

    let linear_records = [l_fix1, l_mt4];
    if recommendation_gates_pass(&bounded_kv.aggregate)
        && linear_records
            .iter()
            .all(|linear| beats_by_recommendation_margin(&bounded_kv.aggregate, &linear.aggregate))
    {
        return Ok(S5FrontierRecommendationResult {
            frontier_recommendation: FrontierRecommendation::A,
            frontier_leader_variant: None,
        });
    }

    if let Some(leader) = linear_records
        .into_iter()
        .filter(|linear| {
            recommendation_gates_pass(&linear.aggregate)
                && beats_by_recommendation_margin(&linear.aggregate, &bounded_kv.aggregate)
        })
        .min_by(|left, right| compare_linear_frontier_leaders(left, right))
    {
        return Ok(S5FrontierRecommendationResult {
            frontier_recommendation: FrontierRecommendation::B,
            frontier_leader_variant: Some(leader.variant),
        });
    }

    if [bounded_kv, l_fix1, l_mt4]
        .iter()
        .all(|record| recommendation_gates_pass(&record.aggregate))
    {
        return Ok(S5FrontierRecommendationResult {
            frontier_recommendation: FrontierRecommendation::Tie,
            frontier_leader_variant: None,
        });
    }

    Err(S5FrontierRecommendationError::NoRecommendationBecauseVariantGateFailed)
}

fn unique_variant_record(
    records: &[S5FrontierVariantRecord],
    variant: S5FrontierVariantId,
) -> Result<&S5FrontierVariantRecord, S5FrontierRecommendationError> {
    let mut matches = records.iter().filter(|record| record.variant == variant);
    let first = matches
        .next()
        .ok_or(S5FrontierRecommendationError::MissingVariantRecord { variant })?;
    if matches.next().is_some() {
        return Err(S5FrontierRecommendationError::DuplicateVariantRecord { variant });
    }
    Ok(first)
}

fn recommendation_gates_pass(point: &S5FrontierPointMetrics) -> bool {
    point.v0_success_pass && point.shadow_compile_ok_at_end
}

fn beats_by_recommendation_margin(
    candidate: &S5FrontierPointMetrics,
    incumbent: &S5FrontierPointMetrics,
) -> bool {
    candidate.val_bpc_ternary + S5_FRONTIER_RECOMMENDATION_MARGIN_BPC <= incumbent.val_bpc_ternary
}

fn compare_linear_frontier_leaders(
    left: &S5FrontierVariantRecord,
    right: &S5FrontierVariantRecord,
) -> Ordering {
    left.aggregate
        .val_bpc_ternary
        .total_cmp(&right.aggregate.val_bpc_ternary)
        .then_with(|| {
            match (
                left.aggregate.encoded_rom_byte_cost,
                right.aggregate.encoded_rom_byte_cost,
            ) {
                (Some(left_cost), Some(right_cost)) => left_cost.cmp(&right_cost),
                _ => Ordering::Equal,
            }
        })
        .then_with(|| left.variant.as_str().cmp(right.variant.as_str()))
}

#[must_use]
pub fn s5_argmax_token(logits: &[f64]) -> Option<usize> {
    if logits.is_empty() || logits.iter().any(|logit| !logit.is_finite()) {
        return None;
    }

    logits
        .iter()
        .enumerate()
        .max_by(|(left_index, left), (right_index, right)| {
            left.total_cmp(right)
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(index, _)| index)
}

pub fn s5_f13_bias_non_oracle_token_above_predicted(
    oracle_logits: &[f64],
    non_oracle_token: usize,
) -> Option<Vec<f64>> {
    let oracle_token = s5_argmax_token(oracle_logits)?;
    if non_oracle_token >= oracle_logits.len() || non_oracle_token == oracle_token {
        return None;
    }

    let mut biased = oracle_logits.to_vec();
    biased[non_oracle_token] = oracle_logits[oracle_token] + 0.5;
    Some(biased)
}

pub fn s5_f13_bias_predicted_token_below_runner_up(oracle_logits: &[f64]) -> Option<Vec<f64>> {
    let (predicted_token, runner_up_token) = s5_top_two_tokens(oracle_logits)?;
    let mut biased = oracle_logits.to_vec();
    biased[predicted_token] = oracle_logits[runner_up_token] - 0.5;
    Some(biased)
}

#[must_use]
pub const fn s5_h15_oracle_token_agreement(
    oracle_token: usize,
    harness_token: usize,
) -> HypothesisStatus {
    if oracle_token == harness_token {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S5SelectionAuthority {
    #[serde(rename = "automated")]
    Automated,
    #[serde(rename = "manual-override")]
    ManualOverride,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5ParetoEmissionVerification {
    pub expected_frontier_indices: Vec<usize>,
    pub observed_frontier_indices: Vec<usize>,
    pub expected_selected_index: Option<usize>,
    pub observed_selected_index: Option<usize>,
    pub h14: HypothesisStatus,
    pub selection_authority: S5SelectionAuthority,
    pub closure_allowed: bool,
}

pub fn verify_s5_h14_pareto_emission(
    points: &[S5FrontierPointMetrics],
    axes: &[S5FrontierAxis],
    observed_frontier_indices: Vec<usize>,
    observed_selected_index: Option<usize>,
    selection_authority: S5SelectionAuthority,
) -> Result<S5ParetoEmissionVerification, S5FrontierValidationError> {
    let expected_frontier_indices = s5_pareto_frontier_indices(points, axes)?;
    let expected_selected_index = expected_frontier_indices.first().copied();
    let observed_indices_in_range = observed_frontier_indices
        .iter()
        .all(|index| *index < points.len())
        && observed_selected_index
            .map(|index| index < points.len())
            .unwrap_or(true);
    let h14 = if observed_indices_in_range
        && observed_frontier_indices == expected_frontier_indices
        && observed_selected_index == expected_selected_index
    {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };
    let closure_allowed = s5_h14_selection_authority_allows_closure(h14, selection_authority);

    Ok(S5ParetoEmissionVerification {
        expected_frontier_indices,
        observed_frontier_indices,
        expected_selected_index,
        observed_selected_index,
        h14,
        selection_authority,
        closure_allowed,
    })
}

#[must_use]
pub const fn s5_h14_selection_authority_allows_closure(
    h14: HypothesisStatus,
    selection_authority: S5SelectionAuthority,
) -> bool {
    matches!(
        (h14, selection_authority),
        (HypothesisStatus::Confirmed, S5SelectionAuthority::Automated)
            | (
                HypothesisStatus::Refuted,
                S5SelectionAuthority::ManualOverride
            )
    )
}

pub const S5_FEEDBACK_SAFE_BOUND_MIN: f64 = 0.5;
pub const S5_FEEDBACK_SAFE_BOUND_MAX: f64 = 16.0;
pub const S5_FEEDBACK_GROW_ALPHA: f64 = 0.10;
pub const S5_FEEDBACK_SHRINK_ALPHA: f64 = 0.95;
pub const S5_FEEDBACK_SHRINK_THRESHOLD: f64 = 0.5;
pub const S5_FEEDBACK_EXPECTED_EPSILON: f64 = 1.0e-9;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FeedbackApplyConfig {
    pub safe_bound_min: f64,
    pub safe_bound_max: f64,
    pub grow_alpha: f64,
    pub shrink_alpha: f64,
    pub shrink_threshold: f64,
}

impl S5FeedbackApplyConfig {
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            safe_bound_min: S5_FEEDBACK_SAFE_BOUND_MIN,
            safe_bound_max: S5_FEEDBACK_SAFE_BOUND_MAX,
            grow_alpha: S5_FEEDBACK_GROW_ALPHA,
            shrink_alpha: S5_FEEDBACK_SHRINK_ALPHA,
            shrink_threshold: S5_FEEDBACK_SHRINK_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FeedbackSafeBoundCase {
    pub current: f64,
    pub max_abs: f64,
    pub expected: f64,
}

pub const S5_FEEDBACK_FIXTURE_V1_SAFE_BOUND_CASES: [S5FeedbackSafeBoundCase; 4] = [
    S5FeedbackSafeBoundCase {
        current: 4.0,
        max_abs: 5.0,
        expected: 4.4,
    },
    S5FeedbackSafeBoundCase {
        current: 8.0,
        max_abs: 4.0,
        expected: 7.6,
    },
    S5FeedbackSafeBoundCase {
        current: 16.0,
        max_abs: 32.0,
        expected: 16.0,
    },
    S5FeedbackSafeBoundCase {
        current: 0.5,
        max_abs: 0.0,
        expected: 0.5,
    },
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5FeedbackFixtureVerification {
    pub safe_bound_outputs: Vec<f64>,
    pub router_state_byte_identical: bool,
    pub h16: HypothesisStatus,
}

#[must_use]
pub fn s5_apply_feedback_safe_bound(
    current: f64,
    max_abs: f64,
    config: S5FeedbackApplyConfig,
) -> Option<f64> {
    if !current.is_finite()
        || !max_abs.is_finite()
        || !config.safe_bound_min.is_finite()
        || !config.safe_bound_max.is_finite()
        || !config.grow_alpha.is_finite()
        || !config.shrink_alpha.is_finite()
        || !config.shrink_threshold.is_finite()
        || config.safe_bound_min > config.safe_bound_max
    {
        return None;
    }

    let next = if max_abs > current {
        current + (config.grow_alpha * current).min(0.5 * (max_abs - current))
    } else if max_abs <= config.shrink_threshold * current {
        current * config.shrink_alpha
    } else {
        current
    };

    Some(next.clamp(config.safe_bound_min, config.safe_bound_max))
}

#[must_use]
pub fn verify_s5_h16_feedback_fixture(
    cases: &[S5FeedbackSafeBoundCase],
    router_state_before: &[u8],
    router_state_after_empty_affinity: &[u8],
    config: S5FeedbackApplyConfig,
) -> S5FeedbackFixtureVerification {
    let mut safe_bound_outputs = Vec::with_capacity(cases.len());
    let mut safe_bounds_match = true;

    for case in cases {
        match s5_apply_feedback_safe_bound(case.current, case.max_abs, config) {
            Some(output) => {
                safe_bounds_match &= (output - case.expected).abs() <= S5_FEEDBACK_EXPECTED_EPSILON;
                safe_bound_outputs.push(output);
            }
            None => safe_bounds_match = false,
        }
    }

    let router_state_byte_identical = router_state_before == router_state_after_empty_affinity;
    let h16 = if safe_bounds_match && router_state_byte_identical {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Refuted
    };

    S5FeedbackFixtureVerification {
        safe_bound_outputs,
        router_state_byte_identical,
        h16,
    }
}

fn s5_top_two_tokens(logits: &[f64]) -> Option<(usize, usize)> {
    if logits.len() < 2 || logits.iter().any(|logit| !logit.is_finite()) {
        return None;
    }

    let mut indices = (0..logits.len()).collect::<Vec<_>>();
    indices.sort_by(|left, right| {
        logits[*right]
            .total_cmp(&logits[*left])
            .then_with(|| left.cmp(right))
    });
    Some((indices[0], indices[1]))
}

fn validate_pick_points_fit_fields_are_null(
    points: &[S5FrontierPointMetrics],
) -> Result<(), S5FrontierPointSplitError> {
    for (point_index, point) in points.iter().enumerate() {
        if let Some(axis) = first_present_fit_only_axis(point) {
            return Err(S5FrontierPointSplitError::PickPointHasFitOnlyField { point_index, axis });
        }
    }
    Ok(())
}

fn validate_fit_points_fit_fields_are_present(
    points: &[S5FrontierPointMetrics],
) -> Result<(), S5FrontierPointSplitError> {
    for (point_index, point) in points.iter().enumerate() {
        if let Some(axis) = first_missing_fit_only_axis(point) {
            return Err(S5FrontierPointSplitError::FitPointMissingFitOnlyField {
                point_index,
                axis,
            });
        }
    }
    Ok(())
}

fn first_present_fit_only_axis(point: &S5FrontierPointMetrics) -> Option<S5FrontierAxis> {
    if point.encoded_rom_byte_cost.is_some() {
        Some(S5FrontierAxis::EncodedRomByteCost)
    } else if point.fits_envelope.is_some() {
        Some(S5FrontierAxis::FitsEnvelope)
    } else if point.reachability_cert_valid.is_some() {
        Some(S5FrontierAxis::ReachabilityCertValid)
    } else if point.resource_state_cert_valid.is_some() {
        Some(S5FrontierAxis::ResourceStateCertValid)
    } else {
        None
    }
}

fn first_missing_fit_only_axis(point: &S5FrontierPointMetrics) -> Option<S5FrontierAxis> {
    if point.encoded_rom_byte_cost.is_none() {
        Some(S5FrontierAxis::EncodedRomByteCost)
    } else if point.fits_envelope.is_none() {
        Some(S5FrontierAxis::FitsEnvelope)
    } else if point.reachability_cert_valid.is_none() {
        Some(S5FrontierAxis::ReachabilityCertValid)
    } else if point.resource_state_cert_valid.is_none() {
        Some(S5FrontierAxis::ResourceStateCertValid)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxisComparison {
    Better,
    Equal,
    Worse,
    Skipped,
    Invalid,
}

fn compare_frontier_axis(
    axis: S5FrontierAxis,
    candidate: &S5FrontierPointMetrics,
    incumbent: &S5FrontierPointMetrics,
) -> AxisComparison {
    match axis {
        S5FrontierAxis::ValBpcFp => {
            compare_finite_lower(candidate.val_bpc_fp, incumbent.val_bpc_fp)
        }
        S5FrontierAxis::ValBpcTernary => {
            compare_finite_lower(candidate.val_bpc_ternary, incumbent.val_bpc_ternary)
        }
        S5FrontierAxis::TernaryGap => {
            compare_finite_lower(candidate.ternary_gap, incumbent.ternary_gap)
        }
        S5FrontierAxis::V0SuccessPass => {
            compare_bool(candidate.v0_success_pass, incumbent.v0_success_pass)
        }
        S5FrontierAxis::V0SuccessScore => {
            compare_finite_higher(candidate.v0_success_score, incumbent.v0_success_score)
        }
        S5FrontierAxis::ParamCount => compare_lower(candidate.param_count, incumbent.param_count),
        S5FrontierAxis::ProjectedDeployedBytes => compare_lower(
            candidate.projected_deployed_bytes,
            incumbent.projected_deployed_bytes,
        ),
        S5FrontierAxis::ShadowCompileOkAtEnd => compare_bool(
            candidate.shadow_compile_ok_at_end,
            incumbent.shadow_compile_ok_at_end,
        ),
        S5FrontierAxis::ShadowByteCostAtEnd => compare_lower(
            candidate.shadow_byte_cost_at_end,
            incumbent.shadow_byte_cost_at_end,
        ),
        S5FrontierAxis::ShadowKernelCountAtEnd => compare_lower(
            candidate.shadow_kernel_count_at_end,
            incumbent.shadow_kernel_count_at_end,
        ),
        S5FrontierAxis::LatencyProxyCycles => compare_lower(
            candidate.latency_proxy_cycles,
            incumbent.latency_proxy_cycles,
        ),
        S5FrontierAxis::EncodedRomByteCost => compare_optional_lower(
            candidate.encoded_rom_byte_cost,
            incumbent.encoded_rom_byte_cost,
        ),
        S5FrontierAxis::FitsEnvelope => {
            compare_optional_bool(candidate.fits_envelope, incumbent.fits_envelope)
        }
        S5FrontierAxis::ReachabilityCertValid => compare_optional_bool(
            candidate.reachability_cert_valid,
            incumbent.reachability_cert_valid,
        ),
        S5FrontierAxis::ResourceStateCertValid => compare_optional_bool(
            candidate.resource_state_cert_valid,
            incumbent.resource_state_cert_valid,
        ),
    }
}

fn compare_lower<T: PartialOrd>(candidate: T, incumbent: T) -> AxisComparison {
    if candidate < incumbent {
        AxisComparison::Better
    } else if candidate > incumbent {
        AxisComparison::Worse
    } else {
        AxisComparison::Equal
    }
}

fn compare_higher<T: PartialOrd>(candidate: T, incumbent: T) -> AxisComparison {
    if candidate > incumbent {
        AxisComparison::Better
    } else if candidate < incumbent {
        AxisComparison::Worse
    } else {
        AxisComparison::Equal
    }
}

fn compare_finite_lower(candidate: f64, incumbent: f64) -> AxisComparison {
    if !candidate.is_finite() || !incumbent.is_finite() {
        return AxisComparison::Invalid;
    }
    compare_lower(candidate, incumbent)
}

fn compare_finite_higher(candidate: f64, incumbent: f64) -> AxisComparison {
    if !candidate.is_finite() || !incumbent.is_finite() {
        return AxisComparison::Invalid;
    }
    compare_higher(candidate, incumbent)
}

fn compare_bool(candidate: bool, incumbent: bool) -> AxisComparison {
    match (candidate, incumbent) {
        (true, false) => AxisComparison::Better,
        (false, true) => AxisComparison::Worse,
        _ => AxisComparison::Equal,
    }
}

fn compare_optional_lower<T: PartialOrd>(
    candidate: Option<T>,
    incumbent: Option<T>,
) -> AxisComparison {
    match (candidate, incumbent) {
        (Some(candidate), Some(incumbent)) => compare_lower(candidate, incumbent),
        _ => AxisComparison::Skipped,
    }
}

fn compare_optional_bool(candidate: Option<bool>, incumbent: Option<bool>) -> AxisComparison {
    match (candidate, incumbent) {
        (Some(candidate), Some(incumbent)) => compare_bool(candidate, incumbent),
        _ => AxisComparison::Skipped,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Substrate,
    Capacity,
    Suspicious,
    Phase,
    Metric,
    AttentionOracle,
    FrontierIncomplete,
    ShadowCompileWiring,
    RuntimeBudget,
    CompileProfile,
    EncodedRom,
    EmulatorHarness,
    FeedbackLoop,
    LoggingOverhead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum S5Outcome {
    #[serde(rename = "Pass-clean")]
    PassClean,
    #[serde(rename = "Pass-with-A-frontier")]
    PassWithAFrontier,
    #[serde(rename = "Pass-with-B-frontier")]
    PassWithBFrontier,
    #[serde(rename = "Pass-with-tie")]
    PassWithTie,
    #[serde(rename = "Pass-with-frontier-warning")]
    PassWithFrontierWarning,
    #[serde(rename = "Pass-with-shadow-gap-warning")]
    PassWithShadowGapWarning,
    #[serde(rename = "Fail-frontier-incomplete")]
    FailFrontierIncomplete,
    #[serde(rename = "Fail-attention-oracle")]
    FailAttentionOracle,
    #[serde(rename = "Fail-bounded-kv-grad")]
    FailBoundedKvGrad,
    #[serde(rename = "Fail-linearstate-grad")]
    FailLinearstateGrad,
    #[serde(rename = "Fail-runtime-budget")]
    FailRuntimeBudget,
    #[serde(rename = "Fail-compile-profile")]
    FailCompileProfile,
    #[serde(rename = "Fail-shadow-compile")]
    FailShadowCompile,
    #[serde(rename = "Fail-encoded-rom")]
    FailEncodedRom,
    #[serde(rename = "Fail-emulator-harness")]
    FailEmulatorHarness,
    #[serde(rename = "Fail-feedback-loop")]
    FailFeedbackLoop,
    #[serde(rename = "Fail-logging-overhead")]
    FailLoggingOverhead,
    #[serde(rename = "Fail-substrate")]
    FailSubstrate { failure_kind: FailureKind },
}

impl S5Outcome {
    #[must_use]
    pub const fn variant_name(self) -> &'static str {
        match self {
            Self::PassClean => "Pass-clean",
            Self::PassWithAFrontier => "Pass-with-A-frontier",
            Self::PassWithBFrontier => "Pass-with-B-frontier",
            Self::PassWithTie => "Pass-with-tie",
            Self::PassWithFrontierWarning => "Pass-with-frontier-warning",
            Self::PassWithShadowGapWarning => "Pass-with-shadow-gap-warning",
            Self::FailFrontierIncomplete => "Fail-frontier-incomplete",
            Self::FailAttentionOracle => "Fail-attention-oracle",
            Self::FailBoundedKvGrad => "Fail-bounded-kv-grad",
            Self::FailLinearstateGrad => "Fail-linearstate-grad",
            Self::FailRuntimeBudget => "Fail-runtime-budget",
            Self::FailCompileProfile => "Fail-compile-profile",
            Self::FailShadowCompile => "Fail-shadow-compile",
            Self::FailEncodedRom => "Fail-encoded-rom",
            Self::FailEmulatorHarness => "Fail-emulator-harness",
            Self::FailFeedbackLoop => "Fail-feedback-loop",
            Self::FailLoggingOverhead => "Fail-logging-overhead",
            Self::FailSubstrate { .. } => "Fail-substrate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S5OutcomeDispatchInput {
    pub substrate_divergence: bool,
    pub h1: HypothesisStatus,
    pub h2: HypothesisStatus,
    pub h3: HypothesisStatus,
    pub h4: HypothesisStatus,
    pub h5: HypothesisStatus,
    pub h6: HypothesisStatus,
    pub h7: HypothesisStatus,
    pub h8: HypothesisStatus,
    pub h9: HypothesisStatus,
    pub h10: HypothesisStatus,
    pub h11: HypothesisStatus,
    pub h12: HypothesisStatus,
    pub h13: HypothesisStatus,
    pub h13_shadow_gap_warning_band: bool,
    pub h14: HypothesisStatus,
    pub h14_manual_override: bool,
    pub h15: HypothesisStatus,
    pub h16: HypothesisStatus,
    pub h17: HypothesisStatus,
    pub d18_block_export: bool,
    pub runtime_self_check_fired: bool,
    pub encoded_rom_cert_invalid: bool,
    pub reference_shell_bind_count_zero: bool,
    pub frontier_recommendation: FrontierRecommendation,
}

impl S5OutcomeDispatchInput {
    #[must_use]
    pub const fn all_confirmed(frontier_recommendation: FrontierRecommendation) -> Self {
        Self {
            substrate_divergence: false,
            h1: HypothesisStatus::Confirmed,
            h2: HypothesisStatus::Confirmed,
            h3: HypothesisStatus::Confirmed,
            h4: HypothesisStatus::Confirmed,
            h5: HypothesisStatus::Confirmed,
            h6: HypothesisStatus::Confirmed,
            h7: HypothesisStatus::Confirmed,
            h8: HypothesisStatus::Confirmed,
            h9: HypothesisStatus::Confirmed,
            h10: HypothesisStatus::Confirmed,
            h11: HypothesisStatus::Confirmed,
            h12: HypothesisStatus::Confirmed,
            h13: HypothesisStatus::Confirmed,
            h13_shadow_gap_warning_band: false,
            h14: HypothesisStatus::Confirmed,
            h14_manual_override: false,
            h15: HypothesisStatus::Confirmed,
            h16: HypothesisStatus::Confirmed,
            h17: HypothesisStatus::Confirmed,
            d18_block_export: false,
            runtime_self_check_fired: false,
            encoded_rom_cert_invalid: false,
            reference_shell_bind_count_zero: false,
            frontier_recommendation,
        }
    }
}

#[must_use]
pub const fn dispatch_s5_outcome(input: &S5OutcomeDispatchInput) -> S5Outcome {
    if input.substrate_divergence {
        return S5Outcome::FailSubstrate {
            failure_kind: FailureKind::Substrate,
        };
    }
    if matches!(input.h1, HypothesisStatus::Refuted) {
        return S5Outcome::FailAttentionOracle;
    }
    if matches!(input.h2, HypothesisStatus::Refuted) {
        return S5Outcome::FailBoundedKvGrad;
    }
    if matches!(input.h3, HypothesisStatus::Refuted) {
        return S5Outcome::FailLinearstateGrad;
    }
    if matches!(input.h7, HypothesisStatus::Refuted) {
        return S5Outcome::FailFrontierIncomplete;
    }
    if matches!(input.h4, HypothesisStatus::Refuted) {
        return S5Outcome::FailSubstrate {
            failure_kind: FailureKind::Capacity,
        };
    }
    if matches!(input.h6, HypothesisStatus::Refuted) {
        return S5Outcome::FailShadowCompile;
    }
    if matches!(input.h9, HypothesisStatus::Refuted)
        || matches!(input.h10, HypothesisStatus::Refuted)
    {
        return S5Outcome::FailSubstrate {
            failure_kind: FailureKind::Substrate,
        };
    }
    if matches!(input.h11, HypothesisStatus::Refuted)
        || input.d18_block_export
        || input.runtime_self_check_fired
    {
        return S5Outcome::FailRuntimeBudget;
    }
    if matches!(input.h12, HypothesisStatus::Refuted) {
        return S5Outcome::FailCompileProfile;
    }
    if matches!(input.h17, HypothesisStatus::Refuted) {
        return S5Outcome::FailLoggingOverhead;
    }
    if matches!(input.h16, HypothesisStatus::Refuted) {
        return S5Outcome::FailFeedbackLoop;
    }
    if input.encoded_rom_cert_invalid || input.reference_shell_bind_count_zero {
        return S5Outcome::FailEncodedRom;
    }
    if matches!(input.h13, HypothesisStatus::Refuted) {
        return S5Outcome::FailShadowCompile;
    }
    if matches!(input.h15, HypothesisStatus::Refuted) {
        return S5Outcome::FailEmulatorHarness;
    }
    if matches!(input.h14, HypothesisStatus::Refuted) && !input.h14_manual_override {
        return S5Outcome::FailFrontierIncomplete;
    }
    if input.h13_shadow_gap_warning_band {
        return S5Outcome::PassWithShadowGapWarning;
    }
    if matches!(input.h14, HypothesisStatus::Refuted) && input.h14_manual_override {
        return S5Outcome::PassWithFrontierWarning;
    }
    if s5_pass_clean_predicate(input) {
        return S5Outcome::PassClean;
    }
    match input.frontier_recommendation {
        FrontierRecommendation::A => S5Outcome::PassWithAFrontier,
        FrontierRecommendation::B => S5Outcome::PassWithBFrontier,
        FrontierRecommendation::Tie => S5Outcome::PassWithTie,
    }
}

#[must_use]
pub const fn s5_pass_clean_predicate(input: &S5OutcomeDispatchInput) -> bool {
    matches!(input.frontier_recommendation, FrontierRecommendation::A)
        && matches!(input.h5, HypothesisStatus::Confirmed)
        && matches!(input.h8, HypothesisStatus::Confirmed)
        && matches!(input.h14, HypothesisStatus::Confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_has_single_frontier_emitted_event() {
        let events = s5_nominal_state_machine_events();

        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, S5StateMachineEvent::FrontierEmitted))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(S5StateMachineEvent::FrontierEmitted)
        ));
    }

    #[test]
    fn state_machine_uses_inputs_frozen_name_not_pick_frontier_emitted() {
        let events = s5_nominal_state_machine_events();
        let encoded = serde_json::to_string(&events).expect("state events serialize");

        assert!(
            events
                .iter()
                .any(|event| matches!(event, S5StateMachineEvent::PickFrontierInputsFrozen))
        );
        assert!(encoded.contains("PickFrontierInputsFrozen"));
        assert!(!encoded.contains("PickFrontierEmitted"));
        assert!(!encoded.contains("FitFrontierEmitted"));
    }

    #[test]
    fn state_machine_runs_seed_zero_harness_once() {
        let harness_events = s5_nominal_state_machine_events()
            .into_iter()
            .filter_map(|event| match event {
                S5StateMachineEvent::FitHarnessRun { seed } => Some(seed),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(harness_events, vec![0]);
    }

    #[test]
    fn state_machine_fit_loop_covers_seeds_zero_through_four_before_frontier() {
        let events = s5_nominal_state_machine_events();
        let emitted_index = events
            .iter()
            .position(|event| matches!(event, S5StateMachineEvent::FrontierEmitted))
            .expect("frontier is emitted");

        for seed in S5_SEEDS {
            let encoded_rom_index = events
                .iter()
                .position(|event| {
                    matches!(event, S5StateMachineEvent::FitEncodedRomEmitted { seed: found } if *found == seed)
                })
                .expect("seed has encoded ROM event");
            assert!(encoded_rom_index < emitted_index);
        }
    }

    fn frontier_point() -> S5FrontierPointMetrics {
        S5FrontierPointMetrics {
            val_bpc_fp: 1.5,
            val_bpc_ternary: 1.7,
            ternary_gap: 0.2,
            v0_success_pass: true,
            v0_success_score: 0.8,
            param_count: 10_000,
            projected_deployed_bytes: 20_000,
            shadow_compile_ok_at_end: true,
            shadow_byte_cost_at_end: 18_000,
            shadow_kernel_count_at_end: 12,
            latency_proxy_cycles: 50_000,
            encoded_rom_byte_cost: Some(22_000),
            fits_envelope: Some(true),
            reachability_cert_valid: Some(true),
            resource_state_cert_valid: Some(true),
        }
    }

    fn pick_frontier_point() -> S5FrontierPointMetrics {
        S5FrontierPointMetrics {
            encoded_rom_byte_cost: None,
            fits_envelope: None,
            reachability_cert_valid: None,
            resource_state_cert_valid: None,
            ..frontier_point()
        }
    }

    fn frontier_record(
        variant: S5FrontierVariantId,
        val_bpc_ternary: f64,
        encoded_rom_byte_cost: Option<u64>,
    ) -> S5FrontierVariantRecord {
        let mut aggregate = frontier_point();
        aggregate.val_bpc_ternary = val_bpc_ternary;
        aggregate.encoded_rom_byte_cost = encoded_rom_byte_cost;
        S5FrontierVariantRecord { variant, aggregate }
    }

    fn frontier_records(
        bounded_kv_bpc: f64,
        l_fix1_bpc: f64,
        l_mt4_bpc: f64,
    ) -> Vec<S5FrontierVariantRecord> {
        vec![
            frontier_record(S5FrontierVariantId::BoundedKv, bounded_kv_bpc, Some(21_000)),
            frontier_record(S5FrontierVariantId::LFix1, l_fix1_bpc, Some(22_000)),
            frontier_record(S5FrontierVariantId::LMt4, l_mt4_bpc, Some(20_000)),
        ]
    }

    #[test]
    fn s5_outcome_algebra_has_eighteen_variants_and_no_fail_capacity() {
        let outcomes = [
            S5Outcome::PassClean,
            S5Outcome::PassWithAFrontier,
            S5Outcome::PassWithBFrontier,
            S5Outcome::PassWithTie,
            S5Outcome::PassWithFrontierWarning,
            S5Outcome::PassWithShadowGapWarning,
            S5Outcome::FailFrontierIncomplete,
            S5Outcome::FailAttentionOracle,
            S5Outcome::FailBoundedKvGrad,
            S5Outcome::FailLinearstateGrad,
            S5Outcome::FailRuntimeBudget,
            S5Outcome::FailCompileProfile,
            S5Outcome::FailShadowCompile,
            S5Outcome::FailEncodedRom,
            S5Outcome::FailEmulatorHarness,
            S5Outcome::FailFeedbackLoop,
            S5Outcome::FailLoggingOverhead,
            S5Outcome::FailSubstrate {
                failure_kind: FailureKind::Capacity,
            },
        ];

        assert_eq!(outcomes.len(), S5_OUTCOME_VARIANT_COUNT);
        assert!(
            !outcomes
                .iter()
                .any(|outcome| outcome.variant_name() == "Fail-capacity")
        );
    }

    #[test]
    fn frontier_dominance_uses_axis_names_not_axis_order() {
        let mut better = frontier_point();
        better.val_bpc_ternary = 1.6;
        better.latency_proxy_cycles = 40_000;
        let worse = frontier_point();
        let mut reversed_axes = S5_FRONTIER_DEFAULT_AXES;
        reversed_axes.reverse();

        assert!(s5_frontier_dominates(
            &better,
            &worse,
            &S5_FRONTIER_DEFAULT_AXES
        ));
        assert!(s5_frontier_dominates(&better, &worse, &reversed_axes));
        assert!(!s5_frontier_dominates(
            &worse,
            &better,
            &S5_FRONTIER_DEFAULT_AXES
        ));
    }

    #[test]
    fn frontier_dominance_treats_v0_success_score_as_higher_is_better() {
        let mut better = frontier_point();
        better.v0_success_score = 0.95;
        let mut worse = frontier_point();
        worse.v0_success_score = 0.70;

        assert!(s5_frontier_dominates(
            &better,
            &worse,
            &S5_FRONTIER_HIGHER_IS_BETTER_AXES
        ));
        assert!(!s5_frontier_dominates(
            &worse,
            &better,
            &S5_FRONTIER_HIGHER_IS_BETTER_AXES
        ));
    }

    #[test]
    fn frontier_boolean_false_points_are_filtered() {
        let pass = frontier_point();
        let mut v0_fail = frontier_point();
        v0_fail.v0_success_pass = false;
        let mut fit_fail = frontier_point();
        fit_fail.fits_envelope = Some(false);
        let points = [v0_fail, pass, fit_fail];

        assert_eq!(
            s5_pareto_frontier_indices(&points, &S5_FRONTIER_DEFAULT_AXES).unwrap(),
            vec![1]
        );
    }

    #[test]
    fn frontier_dominance_rejects_non_finite_float_axes() {
        let finite = frontier_point();
        let mut non_finite_lower = frontier_point();
        non_finite_lower.val_bpc_ternary = f64::NAN;
        let mut non_finite_higher = frontier_point();
        non_finite_higher.v0_success_score = f64::INFINITY;

        assert!(!s5_frontier_numeric_axes_are_finite(&non_finite_lower));
        assert!(!s5_frontier_numeric_axes_are_finite(&non_finite_higher));
        assert!(!s5_frontier_dominates(
            &non_finite_lower,
            &finite,
            &[S5FrontierAxis::ValBpcTernary]
        ));
        assert!(!s5_frontier_dominates(
            &finite,
            &non_finite_lower,
            &[S5FrontierAxis::ValBpcTernary]
        ));
        assert!(!s5_frontier_dominates(
            &non_finite_higher,
            &finite,
            &[S5FrontierAxis::V0SuccessScore]
        ));
    }

    #[test]
    fn frontier_pareto_errors_on_non_finite_float_axes() {
        let pass = frontier_point();
        let mut nan_bpc = frontier_point();
        nan_bpc.val_bpc_ternary = f64::NAN;
        let mut infinite_score = frontier_point();
        infinite_score.v0_success_score = f64::INFINITY;
        let points = [nan_bpc, pass, infinite_score];

        assert!(matches!(
            s5_pareto_frontier_indices(&points, &S5_FRONTIER_DEFAULT_AXES),
            Err(S5FrontierValidationError::NonFiniteAxis {
                point_index: 0,
                axis: S5FrontierAxis::ValBpcTernary,
                value,
            }) if value.is_nan()
        ));

        let points = [pass, infinite_score];
        assert_eq!(
            s5_pareto_frontier_indices(&points, &S5_FRONTIER_DEFAULT_AXES).unwrap_err(),
            S5FrontierValidationError::NonFiniteAxis {
                point_index: 1,
                axis: S5FrontierAxis::V0SuccessScore,
                value: f64::INFINITY,
            }
        );
    }

    #[test]
    fn frontier_split_allows_fit_nulls_only_in_pick_points() {
        let collection =
            S5FrontierPointCollections::new(vec![pick_frontier_point()], vec![frontier_point()])
                .unwrap();

        assert_eq!(collection.pick_points().len(), 1);
        assert_eq!(collection.fit_points().len(), 1);

        let pick_with_fit_field = S5FrontierPointMetrics {
            encoded_rom_byte_cost: Some(22_000),
            ..pick_frontier_point()
        };
        assert_eq!(
            S5FrontierPointCollections::new(vec![pick_with_fit_field], vec![]).unwrap_err(),
            S5FrontierPointSplitError::PickPointHasFitOnlyField {
                point_index: 0,
                axis: S5FrontierAxis::EncodedRomByteCost,
            }
        );
    }

    #[test]
    fn frontier_split_rejects_fit_points_with_null_fit_only_fields() {
        let fit_missing_cert = S5FrontierPointMetrics {
            reachability_cert_valid: None,
            ..frontier_point()
        };

        assert_eq!(
            S5FrontierPointCollections::new(vec![], vec![fit_missing_cert]).unwrap_err(),
            S5FrontierPointSplitError::FitPointMissingFitOnlyField {
                point_index: 0,
                axis: S5FrontierAxis::ReachabilityCertValid,
            }
        );
    }

    #[test]
    fn frontier_split_runs_pareto_selection_over_fit_points_only() {
        let mut pick_best_metrics = pick_frontier_point();
        pick_best_metrics.val_bpc_ternary = 1.0;
        pick_best_metrics.latency_proxy_cycles = 10_000;
        let fit_kept = frontier_point();
        let mut fit_dominated = frontier_point();
        fit_dominated.val_bpc_ternary = 1.8;
        fit_dominated.latency_proxy_cycles = 60_000;
        let collection =
            S5FrontierPointCollections::new(vec![pick_best_metrics], vec![fit_kept, fit_dominated])
                .unwrap();

        assert_eq!(
            collection
                .fit_pareto_frontier_indices(&S5_FRONTIER_DEFAULT_AXES)
                .unwrap(),
            vec![0]
        );
    }

    #[test]
    fn frontier_recommendation_a_has_null_leader_when_bounded_kv_leads() {
        let records = frontier_records(1.20, 1.25, 1.30);

        let result = s5_frontier_recommendation(&records).unwrap();

        assert_eq!(result.frontier_recommendation, FrontierRecommendation::A);
        assert_eq!(result.frontier_leader_variant, None);
    }

    #[test]
    fn frontier_recommendation_b_can_select_l_mt4_leader() {
        let records = frontier_records(1.30, 1.24, 1.20);

        let result = s5_frontier_recommendation(&records).unwrap();

        assert_eq!(result.frontier_recommendation, FrontierRecommendation::B);
        assert_eq!(
            result.frontier_leader_variant,
            Some(S5FrontierVariantId::LMt4)
        );
    }

    #[test]
    fn frontier_recommendation_b_can_select_l_fix1_leader() {
        let records = frontier_records(1.30, 1.20, 1.24);

        let result = s5_frontier_recommendation(&records).unwrap();

        assert_eq!(result.frontier_recommendation, FrontierRecommendation::B);
        assert_eq!(
            result.frontier_leader_variant,
            Some(S5FrontierVariantId::LFix1)
        );
    }

    #[test]
    fn frontier_recommendation_b_tiebreaks_by_encoded_rom_then_variant_id() {
        let mut records = frontier_records(1.30, 1.20, 1.20);
        records[1].aggregate.encoded_rom_byte_cost = Some(20_001);
        records[2].aggregate.encoded_rom_byte_cost = Some(20_000);

        let result = s5_frontier_recommendation(&records).unwrap();
        assert_eq!(
            result.frontier_leader_variant,
            Some(S5FrontierVariantId::LMt4)
        );

        records[1].aggregate.encoded_rom_byte_cost = None;
        records[2].aggregate.encoded_rom_byte_cost = Some(20_000);
        let result = s5_frontier_recommendation(&records).unwrap();
        assert_eq!(
            result.frontier_leader_variant,
            Some(S5FrontierVariantId::LFix1)
        );
    }

    #[test]
    fn frontier_recommendation_tie_has_null_leader() {
        let records = frontier_records(1.20, 1.21, 1.22);

        let result = s5_frontier_recommendation(&records).unwrap();

        assert_eq!(result.frontier_recommendation, FrontierRecommendation::Tie);
        assert_eq!(result.frontier_leader_variant, None);
    }

    #[test]
    fn frontier_recommendation_report_json_pins_leader_shape() {
        let records = frontier_records(1.30, 1.24, 1.20);
        let report = S5FrontierRecommendationReport::from_variant_records(records).unwrap();

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["frontier_recommendation"], serde_json::json!("B"));
        assert_eq!(
            value["frontier_leader_variant"],
            serde_json::json!("linearstate_mt4")
        );

        let decoded: S5FrontierRecommendationReport = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, report);
        assert_eq!(
            decoded.canonical_json_bytes().unwrap(),
            report.canonical_json_bytes().unwrap()
        );
    }

    #[test]
    fn f13_bias_non_oracle_token_above_predicted_changes_argmax() {
        let oracle_logits = [2.0, 1.5, -0.25];

        let biased = s5_f13_bias_non_oracle_token_above_predicted(&oracle_logits, 1).unwrap();

        assert_eq!(s5_argmax_token(&oracle_logits), Some(0));
        assert_eq!(s5_argmax_token(&biased), Some(1));
    }

    #[test]
    fn f13_bias_predicted_token_below_runner_up_changes_argmax() {
        let oracle_logits = [2.0, 1.5, -0.25];

        let biased = s5_f13_bias_predicted_token_below_runner_up(&oracle_logits).unwrap();

        assert_eq!(s5_argmax_token(&oracle_logits), Some(0));
        assert_eq!(s5_argmax_token(&biased), Some(1));
    }

    #[test]
    fn h15_verifier_rejects_under_f13_oracle_disagreement() {
        let oracle_logits = [2.0, 1.5, -0.25];
        let biased = s5_f13_bias_non_oracle_token_above_predicted(&oracle_logits, 1).unwrap();
        let oracle_token = s5_argmax_token(&oracle_logits).unwrap();
        let harness_token = s5_argmax_token(&biased).unwrap();

        assert_eq!(
            s5_h15_oracle_token_agreement(oracle_token, harness_token),
            HypothesisStatus::Refuted
        );
    }

    #[test]
    fn f14_stub_pareto_emitter_picking_first_point_refutes_h14() {
        let mut dominated_first = frontier_point();
        dominated_first.val_bpc_ternary = 1.8;
        dominated_first.latency_proxy_cycles = 60_000;
        let mut expected_selected = frontier_point();
        expected_selected.val_bpc_ternary = 1.6;
        expected_selected.latency_proxy_cycles = 40_000;
        let mut failed_gate = frontier_point();
        failed_gate.v0_success_pass = false;
        let points = [dominated_first, expected_selected, failed_gate];

        let report = verify_s5_h14_pareto_emission(
            &points,
            &S5_FRONTIER_DEFAULT_AXES,
            vec![0],
            Some(0),
            S5SelectionAuthority::Automated,
        )
        .unwrap();

        assert_eq!(report.expected_frontier_indices, vec![1]);
        assert_eq!(report.h14, HypothesisStatus::Refuted);
        assert!(!report.closure_allowed);
    }

    #[test]
    fn f14_manual_override_required_for_closure_under_refuted_h14() {
        assert!(!s5_h14_selection_authority_allows_closure(
            HypothesisStatus::Refuted,
            S5SelectionAuthority::Automated
        ));
        assert!(s5_h14_selection_authority_allows_closure(
            HypothesisStatus::Refuted,
            S5SelectionAuthority::ManualOverride
        ));

        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::B);
        input.h14 = HypothesisStatus::Refuted;
        assert_eq!(
            dispatch_s5_outcome(&input),
            S5Outcome::FailFrontierIncomplete
        );
        input.h14_manual_override = true;
        assert_eq!(
            dispatch_s5_outcome(&input),
            S5Outcome::PassWithFrontierWarning
        );
    }

    #[test]
    fn f15_feedback_fixture_safe_bounds_confirm_h16() {
        let router_state = [0xaa, 0xbb, 0xcc];

        let report = verify_s5_h16_feedback_fixture(
            &S5_FEEDBACK_FIXTURE_V1_SAFE_BOUND_CASES,
            &router_state,
            &router_state,
            S5FeedbackApplyConfig::pinned(),
        );

        assert_eq!(report.safe_bound_outputs, vec![4.4, 7.6, 16.0, 0.5]);
        assert_eq!(report.h16, HypothesisStatus::Confirmed);
    }

    #[test]
    fn f15_mutated_router_state_breaks_safe_bound_invariant() {
        let router_state_before = [0xaa, 0xbb, 0xcc];
        let router_state_after = [0xaa, 0x00, 0xcc];

        let report = verify_s5_h16_feedback_fixture(
            &S5_FEEDBACK_FIXTURE_V1_SAFE_BOUND_CASES,
            &router_state_before,
            &router_state_after,
            S5FeedbackApplyConfig::pinned(),
        );

        assert!(!report.router_state_byte_identical);
        assert_eq!(report.h16, HypothesisStatus::Refuted);
    }

    #[test]
    fn f15_changed_safe_bound_constant_refutes_h16() {
        let router_state = [0xaa, 0xbb, 0xcc];
        let mut broken_config = S5FeedbackApplyConfig::pinned();
        broken_config.grow_alpha = 0.20;

        let report = verify_s5_h16_feedback_fixture(
            &S5_FEEDBACK_FIXTURE_V1_SAFE_BOUND_CASES,
            &router_state,
            &router_state,
            broken_config,
        );

        assert_eq!(report.safe_bound_outputs[0], 4.5);
        assert_eq!(report.h16, HypothesisStatus::Refuted);
    }

    #[test]
    fn h4_refuted_dispatches_to_fail_substrate_capacity() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h4 = HypothesisStatus::Refuted;

        assert_eq!(
            dispatch_s5_outcome(&input),
            S5Outcome::FailSubstrate {
                failure_kind: FailureKind::Capacity,
            }
        );
    }

    #[test]
    fn h3_refuted_blocks_even_when_frontier_would_pass_clean() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h3 = HypothesisStatus::Refuted;

        assert!(s5_pass_clean_predicate(&input));
        assert_eq!(dispatch_s5_outcome(&input), S5Outcome::FailLinearstateGrad);
    }

    #[test]
    fn pass_clean_is_reachable_for_all_green_a_frontier() {
        let input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);

        assert!(s5_pass_clean_predicate(&input));
        assert_eq!(dispatch_s5_outcome(&input), S5Outcome::PassClean);
    }

    #[test]
    fn pass_with_a_frontier_remains_reachable_when_nonblocking_h5_is_refuted() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h5 = HypothesisStatus::Refuted;

        assert!(!s5_pass_clean_predicate(&input));
        assert_eq!(dispatch_s5_outcome(&input), S5Outcome::PassWithAFrontier);
    }

    #[test]
    fn shadow_gap_warning_takes_precedence_over_pass_clean() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h13_shadow_gap_warning_band = true;

        assert!(s5_pass_clean_predicate(&input));
        assert_eq!(
            dispatch_s5_outcome(&input),
            S5Outcome::PassWithShadowGapWarning
        );
    }

    #[test]
    fn h13_refuted_takes_precedence_over_shadow_gap_warning() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h13 = HypothesisStatus::Refuted;
        input.h13_shadow_gap_warning_band = true;

        assert_eq!(dispatch_s5_outcome(&input), S5Outcome::FailShadowCompile);
    }

    #[test]
    fn h14_refuted_takes_precedence_over_shadow_gap_warning() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h14 = HypothesisStatus::Refuted;
        input.h13_shadow_gap_warning_band = true;

        assert_eq!(
            dispatch_s5_outcome(&input),
            S5Outcome::FailFrontierIncomplete
        );
    }

    #[test]
    fn h15_refuted_takes_precedence_over_shadow_gap_warning() {
        let mut input = S5OutcomeDispatchInput::all_confirmed(FrontierRecommendation::A);
        input.h15 = HypothesisStatus::Refuted;
        input.h13_shadow_gap_warning_band = true;

        assert_eq!(dispatch_s5_outcome(&input), S5Outcome::FailEmulatorHarness);
    }
}
