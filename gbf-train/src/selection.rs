//! Pure Pareto frontier tracking and checkpoint selection helpers.
//!
//! This module intentionally owns only the deterministic selection substrate:
//! typed checkpoint frontier points, bounded Pareto frontier computation, and
//! the `training_selection.json` payload. Producing real points from shadow
//! export/compile remains owned by the F8 shadow pipeline work. This is not
//! the canonical S7 `s7_frontier.v1` report surface.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

/// Narrow JSON schema id for the pure training-selection helper.
///
/// `training_selection.v1` is intentionally not the richer shadow-compile
/// frontier report (`s7_frontier.v1`). It records only the typed inputs needed
/// to test Pareto selection independently of the full producer pipeline.
pub const TRAINING_SELECTION_SCHEMA: &str = "training_selection.v1";
pub const TRAINING_SELECTION_SCOPE_NOTE: &str = "pure-selection-only; not canonical s7_frontier.v1; shadow compile point production is owned by bd-1f7/bd-2am";

/// Stable checkpoint identifier used by the pure selection substrate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CheckpointId(String);

impl CheckpointId {
    pub fn new(value: impl Into<String>) -> Result<Self, SelectionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SelectionError::EmptyCheckpointId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CheckpointId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Narrow quality summary for pure selection tests.
///
/// Higher `score` is better. Real producers may derive this from validation
/// loss, BPC, perplexity, or another phase-specific metric before adapting to
/// this substrate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualitySummary {
    pub score: f64,
}

impl QualitySummary {
    #[must_use]
    pub const fn new(score: f64) -> Self {
        Self { score }
    }
}

/// Conformance summary used by the pure selector.
///
/// Passing conformance dominates failing conformance. When the pass/fail bit
/// ties, lower `max_divergence` is better.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceSummary {
    pub passes: bool,
    pub max_divergence: f64,
}

impl ConformanceSummary {
    #[must_use]
    pub const fn new(passes: bool, max_divergence: f64) -> Self {
        Self {
            passes,
            max_divergence,
        }
    }
}

/// Projected runtime-fit summary used by the pure selector.
///
/// A fitting checkpoint dominates a non-fitting checkpoint. When the fit bit
/// ties, larger positive byte margin is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedFitSummary {
    pub fits: bool,
    pub margin_bytes: i64,
}

impl ProjectedFitSummary {
    #[must_use]
    pub const fn new(fits: bool, margin_bytes: i64) -> Self {
        Self { fits, margin_bytes }
    }
}

/// Two-dimensional schedule-cost summary.
///
/// Lower cycles and lower bank switches are each better. If one point has
/// fewer cycles but more bank switches than another point, schedule cost is
/// Pareto-incomparable and neither point dominates by this axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostSummary {
    pub cycles_per_token: u64,
    pub bank_switches_per_token: u64,
}

impl ScheduleCostSummary {
    #[must_use]
    pub const fn new(cycles_per_token: u64, bank_switches_per_token: u64) -> Self {
        Self {
            cycles_per_token,
            bank_switches_per_token,
        }
    }
}

/// Narrow checkpoint frontier point consumed by the pure selector.
///
/// This type is sufficient for deterministic dominance/selection tests. It is
/// not a claim that real shadow compile has emitted the canonical
/// `s7_frontier.v1` / full F8 `CheckpointFrontierPoint` surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CheckpointFrontierPointUnchecked", deny_unknown_fields)]
pub struct CheckpointFrontierPoint {
    pub checkpoint_id: CheckpointId,
    pub observed_at_step: u64,
    pub quality: QualitySummary,
    pub conformance: ConformanceSummary,
    pub projected_fit: ProjectedFitSummary,
    pub schedule_cost: ScheduleCostSummary,
}

impl CheckpointFrontierPoint {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        checkpoint_id: impl Into<String>,
        observed_at_step: u64,
        quality_score: f64,
        conformance_passes: bool,
        max_divergence: f64,
        projected_fit: bool,
        margin_bytes: i64,
        cycles_per_token: u64,
        bank_switches_per_token: u64,
    ) -> Result<Self, SelectionError> {
        let point = Self {
            checkpoint_id: CheckpointId::new(checkpoint_id)?,
            observed_at_step,
            quality: QualitySummary::new(quality_score),
            conformance: ConformanceSummary::new(conformance_passes, max_divergence),
            projected_fit: ProjectedFitSummary::new(projected_fit, margin_bytes),
            schedule_cost: ScheduleCostSummary::new(cycles_per_token, bank_switches_per_token),
        };
        point.validate()?;
        Ok(point)
    }

    pub fn validate(&self) -> Result<(), SelectionError> {
        validate_finite("quality.score", self.quality.score)?;
        validate_finite(
            "conformance.max_divergence",
            self.conformance.max_divergence,
        )?;
        if self.conformance.max_divergence < 0.0 {
            return Err(SelectionError::NegativeMetric {
                field: "conformance.max_divergence",
                value: self.conformance.max_divergence,
            });
        }
        Ok(())
    }

    #[must_use]
    pub const fn is_hard_failure(&self) -> bool {
        !self.conformance.passes || !self.projected_fit.fits
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointFrontierPointUnchecked {
    checkpoint_id: CheckpointId,
    observed_at_step: u64,
    quality: QualitySummary,
    conformance: ConformanceSummary,
    projected_fit: ProjectedFitSummary,
    schedule_cost: ScheduleCostSummary,
}

impl TryFrom<CheckpointFrontierPointUnchecked> for CheckpointFrontierPoint {
    type Error = SelectionError;

    fn try_from(value: CheckpointFrontierPointUnchecked) -> Result<Self, Self::Error> {
        let point = Self {
            checkpoint_id: value.checkpoint_id,
            observed_at_step: value.observed_at_step,
            quality: value.quality,
            conformance: value.conformance,
            projected_fit: value.projected_fit,
            schedule_cost: value.schedule_cost,
        };
        point.validate()?;
        Ok(point)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParetoAxis {
    Quality,
    Conformance,
    ProjectedFit,
    ScheduleCost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxisOrdering {
    Better,
    Equal,
    Worse,
    Incomparable,
}

/// Machine-readable explanation for one dominance relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DominanceReason {
    pub dominant_checkpoint_id: CheckpointId,
    pub dominated_checkpoint_id: CheckpointId,
    /// Axes considered by the dominance rule.
    ///
    /// For a valid dominance relationship this is always the full axis set,
    /// because domination requires being no worse on every axis. The separate
    /// `strictly_better_axes` field is the informative subset that explains
    /// why the two points were not merely equal.
    pub better_or_equal_axes: Vec<ParetoAxis>,
    pub strictly_better_axes: Vec<ParetoAxis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityEviction {
    pub checkpoint_id: CheckpointId,
    pub reason: CapacityEvictionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityEvictionReason {
    OldestAfterDominancePruning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardFailureReason {
    pub checkpoint_id: CheckpointId,
    pub conformance_passes: bool,
    pub projected_fit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStatus {
    Selected,
    EmptyFrontier,
    AllCandidatesHardFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionSummary {
    pub status: SelectionStatus,
    pub selected_checkpoint_id: Option<CheckpointId>,
    pub reason: String,
}

/// Pure `training_selection.json` payload.
///
/// The `frontier`, `selected`, `domination_reasoning`, and
/// `capacity_evictions` fields describe the bounded frontier after dominance
/// pruning. `hard_failure_filter` is frontier-scoped: it lists only hard
/// failures that survived onto the bounded frontier and were therefore
/// excluded from selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingSelectionReport {
    pub schema: String,
    pub scope_note: String,
    pub generated_at_unix_seconds: u64,
    pub keep_frontier: usize,
    pub input_points_count: usize,
    pub frontier: Vec<CheckpointFrontierPoint>,
    pub selected: Option<CheckpointFrontierPoint>,
    pub selection: SelectionSummary,
    pub domination_reasoning: Vec<DominanceReason>,
    pub capacity_evictions: Vec<CapacityEviction>,
    /// Hard-failing points present on the bounded frontier.
    ///
    /// This is not a global ledger of every rejected input candidate. Dominated
    /// or capacity-evicted hard failures are reported through domination or
    /// eviction fields instead.
    pub hard_failure_filter: Vec<HardFailureReason>,
}

impl TrainingSelectionReport {
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrontierUpdate {
    pub accepted: bool,
    pub frontier: Vec<CheckpointFrontierPoint>,
    pub rejected_as_dominated_by: Vec<CheckpointId>,
    pub evicted_for_capacity: Vec<CapacityEviction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParetoFrontier {
    keep_frontier: usize,
    points: Vec<CheckpointFrontierPoint>,
}

impl ParetoFrontier {
    pub fn new(keep_frontier: usize) -> Result<Self, SelectionError> {
        validate_keep_frontier(keep_frontier)?;
        Ok(Self {
            keep_frontier,
            points: Vec::new(),
        })
    }

    pub fn add_point(
        &mut self,
        point: CheckpointFrontierPoint,
    ) -> Result<FrontierUpdate, SelectionError> {
        point.validate()?;
        let mut candidates = self.points.clone();
        candidates.push(point.clone());
        let bounded = compute_bounded_frontier(&candidates, self.keep_frontier)?;
        let retained_ids: BTreeSet<CheckpointId> = bounded
            .frontier
            .iter()
            .map(|point| point.checkpoint_id.clone())
            .collect();

        let rejected_as_dominated_by = bounded
            .domination_reasoning
            .iter()
            .filter(|reason| reason.dominated_checkpoint_id == point.checkpoint_id)
            .map(|reason| reason.dominant_checkpoint_id.clone())
            .collect::<Vec<_>>();

        let accepted = retained_ids.contains(&point.checkpoint_id);
        self.points = bounded.frontier.clone();

        Ok(FrontierUpdate {
            accepted,
            frontier: self.points.clone(),
            rejected_as_dominated_by,
            evicted_for_capacity: bounded.capacity_evictions,
        })
    }

    #[must_use]
    pub const fn keep_frontier(&self) -> usize {
        self.keep_frontier
    }

    #[must_use]
    pub fn points(&self) -> &[CheckpointFrontierPoint] {
        &self.points
    }

    pub fn selection_report(
        &self,
        generated_at_unix_seconds: u64,
    ) -> Result<TrainingSelectionReport, SelectionError> {
        training_selection_report(
            self.points.clone(),
            self.keep_frontier,
            generated_at_unix_seconds,
        )
    }
}

#[must_use]
pub fn dominates(left: &CheckpointFrontierPoint, right: &CheckpointFrontierPoint) -> bool {
    dominance_reason(left, right).is_some()
}

pub fn training_selection_json(
    points: impl IntoIterator<Item = CheckpointFrontierPoint>,
    keep_frontier: usize,
    generated_at_unix_seconds: u64,
) -> Result<String, SelectionError> {
    Ok(
        training_selection_report(points, keep_frontier, generated_at_unix_seconds)?
            .to_json_string()?,
    )
}

pub fn training_selection_report(
    points: impl IntoIterator<Item = CheckpointFrontierPoint>,
    keep_frontier: usize,
    generated_at_unix_seconds: u64,
) -> Result<TrainingSelectionReport, SelectionError> {
    validate_keep_frontier(keep_frontier)?;
    let points = points.into_iter().collect::<Vec<_>>();
    let bounded = compute_bounded_frontier(&points, keep_frontier)?;
    let hard_failure_filter = bounded
        .frontier
        .iter()
        .filter(|point| point.is_hard_failure())
        .map(|point| HardFailureReason {
            checkpoint_id: point.checkpoint_id.clone(),
            conformance_passes: point.conformance.passes,
            projected_fit: point.projected_fit.fits,
        })
        .collect::<Vec<_>>();

    let selected = select_from_frontier(&bounded.frontier).cloned();
    let selection = selection_summary(&bounded.frontier, selected.as_ref());

    Ok(TrainingSelectionReport {
        schema: TRAINING_SELECTION_SCHEMA.to_owned(),
        scope_note: TRAINING_SELECTION_SCOPE_NOTE.to_owned(),
        generated_at_unix_seconds,
        keep_frontier,
        input_points_count: points.len(),
        frontier: bounded.frontier,
        selected,
        selection,
        domination_reasoning: bounded.domination_reasoning,
        capacity_evictions: bounded.capacity_evictions,
        hard_failure_filter,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct BoundedFrontier {
    frontier: Vec<CheckpointFrontierPoint>,
    domination_reasoning: Vec<DominanceReason>,
    capacity_evictions: Vec<CapacityEviction>,
}

fn compute_bounded_frontier(
    points: &[CheckpointFrontierPoint],
    keep_frontier: usize,
) -> Result<BoundedFrontier, SelectionError> {
    validate_keep_frontier(keep_frontier)?;
    for point in points {
        point.validate()?;
    }
    let mut seen_checkpoint_ids = BTreeSet::new();
    for point in points {
        if !seen_checkpoint_ids.insert(point.checkpoint_id.clone()) {
            return Err(SelectionError::DuplicateCheckpointId {
                checkpoint_id: point.checkpoint_id.clone(),
            });
        }
    }

    let domination_reasoning = dominance_relationships(points);
    let dominated_ids = domination_reasoning
        .iter()
        .map(|reason| reason.dominated_checkpoint_id.clone())
        .collect::<BTreeSet<_>>();

    let mut frontier = points
        .iter()
        .filter(|point| !dominated_ids.contains(&point.checkpoint_id))
        .cloned()
        .collect::<Vec<_>>();
    frontier.sort_by(oldest_point_order);

    let mut capacity_evictions = Vec::new();
    while frontier.len() > keep_frontier {
        let evicted = frontier.remove(0);
        capacity_evictions.push(CapacityEviction {
            checkpoint_id: evicted.checkpoint_id,
            reason: CapacityEvictionReason::OldestAfterDominancePruning,
        });
    }

    frontier.sort_by(report_point_order);

    Ok(BoundedFrontier {
        frontier,
        domination_reasoning,
        capacity_evictions,
    })
}

fn dominance_relationships(points: &[CheckpointFrontierPoint]) -> Vec<DominanceReason> {
    let mut reasons = Vec::new();
    for left in points {
        for right in points {
            if left.checkpoint_id == right.checkpoint_id {
                continue;
            }
            if let Some(reason) = dominance_reason(left, right) {
                reasons.push(reason);
            }
        }
    }
    reasons.sort_by(|left, right| {
        left.dominant_checkpoint_id
            .cmp(&right.dominant_checkpoint_id)
            .then_with(|| {
                left.dominated_checkpoint_id
                    .cmp(&right.dominated_checkpoint_id)
            })
    });
    reasons
}

fn dominance_reason(
    left: &CheckpointFrontierPoint,
    right: &CheckpointFrontierPoint,
) -> Option<DominanceReason> {
    let comparisons = [
        (ParetoAxis::Quality, compare_quality(left, right)),
        (ParetoAxis::Conformance, compare_conformance(left, right)),
        (ParetoAxis::ProjectedFit, compare_projected_fit(left, right)),
        (ParetoAxis::ScheduleCost, compare_schedule_cost(left, right)),
    ];

    if comparisons
        .iter()
        .any(|(_, ordering)| matches!(ordering, AxisOrdering::Worse | AxisOrdering::Incomparable))
    {
        return None;
    }

    let strictly_better_axes = comparisons
        .iter()
        .filter_map(|(axis, ordering)| matches!(ordering, AxisOrdering::Better).then_some(*axis))
        .collect::<Vec<_>>();
    if strictly_better_axes.is_empty() {
        return None;
    }

    Some(DominanceReason {
        dominant_checkpoint_id: left.checkpoint_id.clone(),
        dominated_checkpoint_id: right.checkpoint_id.clone(),
        better_or_equal_axes: comparisons.iter().map(|(axis, _)| *axis).collect(),
        strictly_better_axes,
    })
}

fn compare_quality(
    left: &CheckpointFrontierPoint,
    right: &CheckpointFrontierPoint,
) -> AxisOrdering {
    higher_is_better(left.quality.score, right.quality.score)
}

fn compare_conformance(
    left: &CheckpointFrontierPoint,
    right: &CheckpointFrontierPoint,
) -> AxisOrdering {
    match left.conformance.passes.cmp(&right.conformance.passes) {
        Ordering::Greater => AxisOrdering::Better,
        Ordering::Less => AxisOrdering::Worse,
        Ordering::Equal => lower_is_better(
            left.conformance.max_divergence,
            right.conformance.max_divergence,
        ),
    }
}

fn compare_projected_fit(
    left: &CheckpointFrontierPoint,
    right: &CheckpointFrontierPoint,
) -> AxisOrdering {
    match left.projected_fit.fits.cmp(&right.projected_fit.fits) {
        Ordering::Greater => AxisOrdering::Better,
        Ordering::Less => AxisOrdering::Worse,
        Ordering::Equal => match left
            .projected_fit
            .margin_bytes
            .cmp(&right.projected_fit.margin_bytes)
        {
            Ordering::Greater => AxisOrdering::Better,
            Ordering::Equal => AxisOrdering::Equal,
            Ordering::Less => AxisOrdering::Worse,
        },
    }
}

fn compare_schedule_cost(
    left: &CheckpointFrontierPoint,
    right: &CheckpointFrontierPoint,
) -> AxisOrdering {
    let cycles = left
        .schedule_cost
        .cycles_per_token
        .cmp(&right.schedule_cost.cycles_per_token);
    let switches = left
        .schedule_cost
        .bank_switches_per_token
        .cmp(&right.schedule_cost.bank_switches_per_token);

    match (cycles, switches) {
        (Ordering::Equal, Ordering::Equal) => AxisOrdering::Equal,
        (Ordering::Less | Ordering::Equal, Ordering::Less | Ordering::Equal) => {
            AxisOrdering::Better
        }
        (Ordering::Greater | Ordering::Equal, Ordering::Greater | Ordering::Equal) => {
            AxisOrdering::Worse
        }
        _ => AxisOrdering::Incomparable,
    }
}

fn higher_is_better(left: f64, right: f64) -> AxisOrdering {
    match left.total_cmp(&right) {
        Ordering::Greater => AxisOrdering::Better,
        Ordering::Equal => AxisOrdering::Equal,
        Ordering::Less => AxisOrdering::Worse,
    }
}

fn lower_is_better(left: f64, right: f64) -> AxisOrdering {
    match left.total_cmp(&right) {
        Ordering::Less => AxisOrdering::Better,
        Ordering::Equal => AxisOrdering::Equal,
        Ordering::Greater => AxisOrdering::Worse,
    }
}

fn select_from_frontier(frontier: &[CheckpointFrontierPoint]) -> Option<&CheckpointFrontierPoint> {
    frontier
        .iter()
        .filter(|point| !point.is_hard_failure())
        .max_by(|left, right| selection_order(left, right))
}

fn selection_summary(
    frontier: &[CheckpointFrontierPoint],
    selected: Option<&CheckpointFrontierPoint>,
) -> SelectionSummary {
    match selected {
        Some(point) => SelectionSummary {
            status: SelectionStatus::Selected,
            selected_checkpoint_id: Some(point.checkpoint_id.clone()),
            reason:
                "selected highest-quality hard-pass frontier point with deterministic tie-break"
                    .to_owned(),
        },
        None if frontier.is_empty() => SelectionSummary {
            status: SelectionStatus::EmptyFrontier,
            selected_checkpoint_id: None,
            reason: "frontier is empty".to_owned(),
        },
        None => SelectionSummary {
            status: SelectionStatus::AllCandidatesHardFailed,
            selected_checkpoint_id: None,
            reason: "all frontier candidates failed conformance or projected-fit hard gates"
                .to_owned(),
        },
    }
}

fn selection_order(left: &CheckpointFrontierPoint, right: &CheckpointFrontierPoint) -> Ordering {
    left.quality
        .score
        .total_cmp(&right.quality.score)
        .then_with(|| {
            right
                .conformance
                .max_divergence
                .total_cmp(&left.conformance.max_divergence)
        })
        .then_with(|| {
            left.projected_fit
                .margin_bytes
                .cmp(&right.projected_fit.margin_bytes)
        })
        .then_with(|| {
            right
                .schedule_cost
                .cycles_per_token
                .cmp(&left.schedule_cost.cycles_per_token)
        })
        .then_with(|| {
            right
                .schedule_cost
                .bank_switches_per_token
                .cmp(&left.schedule_cost.bank_switches_per_token)
        })
        .then_with(|| right.checkpoint_id.cmp(&left.checkpoint_id))
        .then_with(|| right.observed_at_step.cmp(&left.observed_at_step))
}

fn report_point_order(left: &CheckpointFrontierPoint, right: &CheckpointFrontierPoint) -> Ordering {
    left.checkpoint_id
        .cmp(&right.checkpoint_id)
        .then_with(|| left.observed_at_step.cmp(&right.observed_at_step))
}

fn oldest_point_order(left: &CheckpointFrontierPoint, right: &CheckpointFrontierPoint) -> Ordering {
    left.observed_at_step
        .cmp(&right.observed_at_step)
        .then_with(|| left.checkpoint_id.cmp(&right.checkpoint_id))
}

fn validate_keep_frontier(keep_frontier: usize) -> Result<(), SelectionError> {
    if keep_frontier == 0 {
        return Err(SelectionError::InvalidKeepFrontier { keep_frontier });
    }
    Ok(())
}

fn validate_finite(field: &'static str, value: f64) -> Result<(), SelectionError> {
    if !value.is_finite() {
        return Err(SelectionError::NonFiniteMetric { field, value });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectionError {
    EmptyCheckpointId,
    DuplicateCheckpointId { checkpoint_id: CheckpointId },
    InvalidKeepFrontier { keep_frontier: usize },
    NonFiniteMetric { field: &'static str, value: f64 },
    NegativeMetric { field: &'static str, value: f64 },
    Json { detail: String },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCheckpointId => write!(f, "checkpoint_id cannot be empty"),
            Self::DuplicateCheckpointId { checkpoint_id } => {
                write!(
                    f,
                    "duplicate checkpoint_id in frontier input: {checkpoint_id}"
                )
            }
            Self::InvalidKeepFrontier { keep_frontier } => {
                write!(
                    f,
                    "keep_frontier must be greater than zero, got {keep_frontier}"
                )
            }
            Self::NonFiniteMetric { field, value } => {
                write!(f, "{field} must be finite, got {value}")
            }
            Self::NegativeMetric { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::Json { detail } => write!(f, "training_selection JSON error: {detail}"),
        }
    }
}

impl Error for SelectionError {}

impl From<serde_json::Error> for SelectionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json {
            detail: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn point(
        checkpoint_id: &str,
        observed_at_step: u64,
        quality_score: f64,
        passes: bool,
        fits: bool,
        cycles_per_token: u64,
    ) -> CheckpointFrontierPoint {
        point_with(
            checkpoint_id,
            observed_at_step,
            quality_score,
            passes,
            0.001,
            fits,
            if fits { 256 } else { -128 },
            cycles_per_token,
            4,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn point_with(
        checkpoint_id: &str,
        observed_at_step: u64,
        quality_score: f64,
        passes: bool,
        max_divergence: f64,
        fits: bool,
        margin_bytes: i64,
        cycles_per_token: u64,
        bank_switches_per_token: u64,
    ) -> CheckpointFrontierPoint {
        CheckpointFrontierPoint::new(
            checkpoint_id,
            observed_at_step,
            quality_score,
            passes,
            max_divergence,
            fits,
            margin_bytes,
            cycles_per_token,
            bank_switches_per_token,
        )
        .expect("fixture point is valid")
    }

    #[test]
    fn selection_dominance_requires_no_worse_axis_and_one_better_axis() {
        let dominant = point("ckpt-a", 10, 0.90, true, true, 100);
        let dominated = point("ckpt-b", 20, 0.80, true, true, 150);
        let cheaper_tradeoff = point("ckpt-c", 30, 0.85, true, true, 80);

        assert!(dominates(&dominant, &dominated));
        assert!(!dominates(&dominant, &cheaper_tradeoff));
        assert!(!dominates(&cheaper_tradeoff, &dominant));
    }

    #[test]
    fn selection_schedule_cost_tradeoffs_are_pareto_incomparable() {
        let fewer_cycles = point_with("ckpt-cycles", 10, 0.90, true, 0.001, true, 256, 80, 8);
        let fewer_switches = point_with("ckpt-switches", 20, 0.90, true, 0.001, true, 256, 100, 4);

        assert!(!dominates(&fewer_cycles, &fewer_switches));
        assert!(!dominates(&fewer_switches, &fewer_cycles));

        let report = training_selection_report(
            [fewer_cycles.clone(), fewer_switches.clone()],
            4,
            1_700_000_000,
        )
        .expect("schedule tradeoff remains reportable");
        assert_eq!(report.frontier.len(), 2);
        assert!(report.domination_reasoning.is_empty());
    }

    #[test]
    fn selection_frontier_tracks_non_dominated_points_and_bounds_capacity() {
        let mut frontier = ParetoFrontier::new(2).expect("valid capacity");

        frontier
            .add_point(point("ckpt-old", 1, 0.80, true, true, 80))
            .expect("adds old");
        frontier
            .add_point(point("ckpt-middle", 2, 0.85, true, true, 90))
            .expect("adds middle");
        let update = frontier
            .add_point(point("ckpt-new", 3, 0.90, true, true, 100))
            .expect("adds new and evicts oldest after cap");

        assert!(update.accepted);
        assert_eq!(frontier.points().len(), 2);
        assert_eq!(
            update.evicted_for_capacity,
            vec![CapacityEviction {
                checkpoint_id: CheckpointId::new("ckpt-old").unwrap(),
                reason: CapacityEvictionReason::OldestAfterDominancePruning,
            }]
        );
        assert_eq!(
            frontier
                .points()
                .iter()
                .map(|point| point.checkpoint_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ckpt-middle", "ckpt-new"]
        );
    }

    #[test]
    fn selection_capacity_can_evict_an_older_incoming_non_dominated_point() {
        let mut frontier = ParetoFrontier::new(2).expect("valid capacity");
        frontier
            .add_point(point("ckpt-quality", 10, 0.90, true, true, 100))
            .expect("adds quality point");
        frontier
            .add_point(point("ckpt-cost", 20, 0.80, true, true, 50))
            .expect("adds cost point");

        let update = frontier
            .add_point(point("ckpt-too-old", 1, 0.85, true, true, 75))
            .expect("incoming point is non-dominated but oldest");

        assert!(!update.accepted);
        assert!(update.rejected_as_dominated_by.is_empty());
        assert_eq!(
            update.evicted_for_capacity,
            vec![CapacityEviction {
                checkpoint_id: CheckpointId::new("ckpt-too-old").unwrap(),
                reason: CapacityEvictionReason::OldestAfterDominancePruning,
            }]
        );
        assert_eq!(
            frontier
                .points()
                .iter()
                .map(|point| point.checkpoint_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ckpt-cost", "ckpt-quality"]
        );
    }

    #[test]
    fn selection_rejects_dominated_additions() {
        let mut frontier = ParetoFrontier::new(4).expect("valid capacity");
        frontier
            .add_point(point("ckpt-good", 10, 0.90, true, true, 100))
            .expect("adds good");

        let update = frontier
            .add_point(point("ckpt-bad", 20, 0.80, true, true, 150))
            .expect("rejects dominated");

        assert!(!update.accepted);
        assert_eq!(
            update.rejected_as_dominated_by,
            vec![CheckpointId::new("ckpt-good").unwrap()]
        );
        assert_eq!(frontier.points().len(), 1);
        assert_eq!(frontier.points()[0].checkpoint_id.as_str(), "ckpt-good");
    }

    #[test]
    fn selection_hard_failure_filter_is_frontier_scoped() {
        let report = training_selection_report(
            [
                point("ckpt-good", 10, 0.90, true, true, 100),
                point_with(
                    "ckpt-hard-frontier",
                    20,
                    0.99,
                    false,
                    0.001,
                    true,
                    256,
                    50,
                    1,
                ),
                point_with(
                    "ckpt-hard-dominated",
                    30,
                    0.50,
                    false,
                    0.001,
                    false,
                    -128,
                    200,
                    8,
                ),
            ],
            8,
            1_700_000_000,
        )
        .expect("selection succeeds");

        assert_eq!(
            report
                .frontier
                .iter()
                .map(|point| point.checkpoint_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ckpt-good", "ckpt-hard-frontier"]
        );
        assert_eq!(
            report
                .hard_failure_filter
                .iter()
                .map(|reason| reason.checkpoint_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ckpt-hard-frontier"]
        );
        assert!(report.domination_reasoning.iter().any(|reason| {
            reason.dominated_checkpoint_id == CheckpointId::new("ckpt-hard-dominated").unwrap()
        }));
    }

    #[test]
    fn selection_filters_hard_failures_before_quality_pick() {
        let report = training_selection_report(
            [
                point("ckpt-fit", 10, 0.90, true, true, 120),
                point("ckpt-better-but-no-fit", 20, 0.99, true, false, 80),
                point("ckpt-better-but-no-conformance", 30, 0.98, false, true, 70),
            ],
            8,
            1_700_000_000,
        )
        .expect("selection succeeds");

        assert_eq!(report.selection.status, SelectionStatus::Selected);
        assert_eq!(
            report.selection.selected_checkpoint_id,
            Some(CheckpointId::new("ckpt-fit").unwrap())
        );
        assert_eq!(
            report
                .hard_failure_filter
                .iter()
                .map(|reason| reason.checkpoint_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ckpt-better-but-no-conformance", "ckpt-better-but-no-fit"]
        );
    }

    #[test]
    fn selection_tie_break_is_deterministic() {
        let report = training_selection_report(
            [
                point("ckpt-z", 20, 0.90, true, true, 100),
                point("ckpt-a", 30, 0.90, true, true, 100),
            ],
            4,
            1_700_000_000,
        )
        .expect("selection succeeds");

        assert_eq!(
            report.selection.selected_checkpoint_id,
            Some(CheckpointId::new("ckpt-a").unwrap())
        );
    }

    #[test]
    fn selection_empty_frontier_is_total_and_selects_none() {
        let report = training_selection_report([], 3, 1_700_000_000).expect("empty is reportable");

        assert!(report.frontier.is_empty());
        assert!(report.selected.is_none());
        assert_eq!(report.selection.status, SelectionStatus::EmptyFrontier);
    }

    #[test]
    fn selection_all_hard_failures_selects_none() {
        let report = training_selection_report(
            [
                point("ckpt-no-fit", 10, 0.90, true, false, 100),
                point("ckpt-no-conformance", 20, 0.91, false, true, 90),
            ],
            3,
            1_700_000_000,
        )
        .expect("hard failures are reportable");

        assert!(report.selected.is_none());
        assert_eq!(
            report.selection.status,
            SelectionStatus::AllCandidatesHardFailed
        );
        assert_eq!(report.hard_failure_filter.len(), 2);
    }

    #[test]
    fn selection_rejects_invalid_frontier_inputs() {
        assert!(matches!(
            ParetoFrontier::new(0),
            Err(SelectionError::InvalidKeepFrontier { keep_frontier: 0 })
        ));
        assert!(matches!(
            training_selection_report([point("ckpt-a", 10, 0.90, true, true, 100)], 0, 0),
            Err(SelectionError::InvalidKeepFrontier { keep_frontier: 0 })
        ));

        let duplicate_error = training_selection_report(
            [
                point("ckpt-duplicate", 10, 0.90, true, true, 100),
                point("ckpt-duplicate", 20, 0.80, true, true, 50),
            ],
            4,
            0,
        )
        .expect_err("duplicate checkpoint ids are rejected");
        assert!(matches!(
            duplicate_error,
            SelectionError::DuplicateCheckpointId { ref checkpoint_id }
                if checkpoint_id.as_str() == "ckpt-duplicate"
        ));

        let non_finite =
            CheckpointFrontierPoint::new("ckpt-nan", 10, f64::NAN, true, 0.001, true, 256, 100, 4)
                .expect_err("NaN quality score is rejected");
        assert!(matches!(
            non_finite,
            SelectionError::NonFiniteMetric { field: "quality.score", value }
                if value.is_nan()
        ));

        assert!(matches!(
            CheckpointFrontierPoint::new(
                "ckpt-negative-divergence",
                10,
                0.90,
                true,
                -0.001,
                true,
                256,
                100,
                4,
            ),
            Err(SelectionError::NegativeMetric {
                field: "conformance.max_divergence",
                value
            }) if value == -0.001
        ));
    }

    #[test]
    fn selection_training_selection_json_shape_is_pinned() {
        let json_string = training_selection_json(
            [
                point("ckpt-a", 10, 0.90, true, true, 100),
                point("ckpt-b", 20, 0.80, true, true, 150),
            ],
            3,
            1_700_000_000,
        )
        .expect("json serializes");
        let value: serde_json::Value = serde_json::from_str(&json_string).expect("json parses");

        assert_eq!(
            value,
            json!({
                "schema": "training_selection.v1",
                "scope_note": "pure-selection-only; not canonical s7_frontier.v1; shadow compile point production is owned by bd-1f7/bd-2am",
                "generated_at_unix_seconds": 1_700_000_000_u64,
                "keep_frontier": 3,
                "input_points_count": 2,
                "frontier": [
                    {
                        "checkpoint_id": "ckpt-a",
                        "observed_at_step": 10,
                        "quality": { "score": 0.9 },
                        "conformance": { "passes": true, "max_divergence": 0.001 },
                        "projected_fit": { "fits": true, "margin_bytes": 256 },
                        "schedule_cost": {
                            "cycles_per_token": 100,
                            "bank_switches_per_token": 4
                        }
                    }
                ],
                "selected": {
                    "checkpoint_id": "ckpt-a",
                    "observed_at_step": 10,
                    "quality": { "score": 0.9 },
                    "conformance": { "passes": true, "max_divergence": 0.001 },
                    "projected_fit": { "fits": true, "margin_bytes": 256 },
                    "schedule_cost": {
                        "cycles_per_token": 100,
                        "bank_switches_per_token": 4
                    }
                },
                "selection": {
                    "status": "selected",
                    "selected_checkpoint_id": "ckpt-a",
                    "reason": "selected highest-quality hard-pass frontier point with deterministic tie-break"
                },
                "domination_reasoning": [
                    {
                        "dominant_checkpoint_id": "ckpt-a",
                        "dominated_checkpoint_id": "ckpt-b",
                        "better_or_equal_axes": [
                            "quality",
                            "conformance",
                            "projected_fit",
                            "schedule_cost"
                        ],
                        "strictly_better_axes": [
                            "quality",
                            "schedule_cost"
                        ]
                    }
                ],
                "capacity_evictions": [],
                "hard_failure_filter": []
            })
        );
    }
}
