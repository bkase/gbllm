//! Shadow compile policy, checkpoint frontier, and pipeline orchestration.
//!
//! This module owns the training-side F8 policy/frontier values plus the narrow
//! shadow compile orchestrator. Concrete compiler/oracle execution is injected
//! by later pipeline owners rather than hard-coded here.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gbf_artifact::ModelArtifact;
use gbf_artifact::sequence::{SequenceSemanticsKind, SequenceSemanticsSpec};
use gbf_foundation::{CheckpointId, WorkloadId};
use gbf_policy::{CostEstimate, EstimatedCostDelta};
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

use crate::ema::{EmaExportError, EmaUpdate, EmaWeights};
use crate::export::{BUDGET_DRIFT_FILE_NAME, ExportRuntimeChromeRevalidationReport};
use crate::export_visitor::{ArtifactExportModel, ExportVisitor};
use crate::logging::{LoggingEventError, ShadowCompileEvent, TrainingLogEmitter};
use crate::student::HardTernaryStudentModel;

pub const SHADOW_COMPILE_SCOPE_NOTE: &str = "policy/frontier schema plus injectable shadow pipeline; production compiler/oracle wiring remains owned by bd-1f7/bd-2am follow-ons";

pub const DEFAULT_SHADOW_COMPILE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const SEQUENCE_STATE_COMPARISON_FILE_NAME: &str = "sequence_state_comparison.json";

/// Path reference to a compile request TOML used by shadow compilation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompileRequestRef(PathBuf);

impl CompileRequestRef {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, ShadowContractError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(ShadowContractError::EmptyCompileRequestRef { index: None });
        }
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

impl Serialize for CompileRequestRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CompileRequestRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path = PathBuf::deserialize(deserializer)?;
        Self::new(path).map_err(D::Error::custom)
    }
}

/// Cadence and bounded-frontier configuration for training shadow compile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowCompilePolicy {
    pub every_n_steps: u64,
    pub requests: Vec<CompileRequestRef>,
    pub workloads: Vec<WorkloadId>,
    pub keep_frontier: usize,
    #[serde(default)]
    pub dual_sequence_state: bool,
}

impl ShadowCompilePolicy {
    pub fn new(
        every_n_steps: u64,
        requests: Vec<CompileRequestRef>,
        workloads: Vec<WorkloadId>,
        keep_frontier: usize,
    ) -> Result<Self, ShadowContractError> {
        let policy = Self {
            every_n_steps,
            requests,
            workloads,
            keep_frontier,
            dual_sequence_state: false,
        };
        policy.validate()?;
        Ok(policy)
    }

    #[must_use]
    pub const fn with_dual_sequence_state(mut self, dual_sequence_state: bool) -> Self {
        self.dual_sequence_state = dual_sequence_state;
        self
    }

    pub fn validate(&self) -> Result<(), ShadowContractError> {
        if self.every_n_steps == 0 {
            return Err(ShadowContractError::InvalidEveryNSteps {
                every_n_steps: self.every_n_steps,
            });
        }
        if self.keep_frontier == 0 {
            return Err(ShadowContractError::InvalidKeepFrontier {
                keep_frontier: self.keep_frontier,
            });
        }
        if self.requests.is_empty() {
            return Err(ShadowContractError::EmptyCompileRequests);
        }
        for (index, request) in self.requests.iter().enumerate() {
            if request.as_path().as_os_str().is_empty() {
                return Err(ShadowContractError::EmptyCompileRequestRef { index: Some(index) });
            }
        }
        if self.workloads.is_empty() {
            return Err(ShadowContractError::EmptyWorkloads);
        }
        for (index, workload) in self.workloads.iter().enumerate() {
            if workload.as_str().trim().is_empty() {
                return Err(ShadowContractError::EmptyWorkloadId { index });
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ShadowCompilePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = ShadowCompilePolicyUnchecked::deserialize(deserializer)?;
        Self::try_from(value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShadowCompilePolicyUnchecked {
    every_n_steps: u64,
    requests: Vec<CompileRequestRef>,
    workloads: Vec<WorkloadId>,
    keep_frontier: usize,
    #[serde(default)]
    dual_sequence_state: bool,
}

impl TryFrom<ShadowCompilePolicyUnchecked> for ShadowCompilePolicy {
    type Error = ShadowContractError;

    fn try_from(value: ShadowCompilePolicyUnchecked) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.every_n_steps,
            value.requests,
            value.workloads,
            value.keep_frontier,
        )?
        .with_dual_sequence_state(value.dual_sequence_state))
    }
}

/// Validation quality metrics for one checkpoint frontier point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QualitySummary {
    pub lm_loss: f32,
    pub perplexity: f32,
}

impl QualitySummary {
    pub fn new(lm_loss: f32, perplexity: f32) -> Result<Self, ShadowContractError> {
        let summary = Self {
            lm_loss,
            perplexity,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), ShadowContractError> {
        validate_finite("quality.lm_loss", self.lm_loss)?;
        validate_finite("quality.perplexity", self.perplexity)?;
        validate_non_negative("quality.lm_loss", self.lm_loss)?;
        validate_positive("quality.perplexity", self.perplexity)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for QualitySummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = QualitySummaryUnchecked::deserialize(deserializer)?;
        Self::new(value.lm_loss, value.perplexity).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualitySummaryUnchecked {
    lm_loss: f32,
    perplexity: f32,
}

/// Conformance result summary for shadow-compiled checkpoint evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceSummary {
    pub passes: bool,
    pub max_divergence: f32,
}

impl ConformanceSummary {
    pub fn new(passes: bool, max_divergence: f32) -> Result<Self, ShadowContractError> {
        let summary = Self {
            passes,
            max_divergence,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), ShadowContractError> {
        validate_finite("conformance.max_divergence", self.max_divergence)?;
        validate_non_negative("conformance.max_divergence", self.max_divergence)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ConformanceSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = ConformanceSummaryUnchecked::deserialize(deserializer)?;
        Self::new(value.passes, value.max_divergence).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceSummaryUnchecked {
    passes: bool,
    max_divergence: f32,
}

/// Projected static-fit summary for a shadow-compiled checkpoint.
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

/// Frontier point used by F8 checkpoint selection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointFrontierPoint {
    pub checkpoint: CheckpointId,
    pub quality: QualitySummary,
    pub conformance: ConformanceSummary,
    pub projected_fit: ProjectedFitSummary,
    pub schedule_cost: Option<EstimatedCostDelta>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceStateVariantId {
    LinearState,
    BoundedKv,
}

impl SequenceStateVariantId {
    #[must_use]
    pub const fn for_spec(spec: &SequenceSemanticsSpec) -> Self {
        match spec.kind() {
            SequenceSemanticsKind::LinearState => Self::LinearState,
            SequenceSemanticsKind::BoundedKv => Self::BoundedKv,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceVariantSummary {
    pub variant: SequenceStateVariantId,
    pub quality: QualitySummary,
    pub schedule_cost: Option<EstimatedCostDelta>,
    pub projected_bank_switches_per_token: f32,
    pub projected_state_bytes: u32,
    pub fits_envelope: bool,
}

impl SequenceVariantSummary {
    pub fn new(
        variant: SequenceStateVariantId,
        quality: QualitySummary,
        schedule_cost: Option<EstimatedCostDelta>,
        projected_bank_switches_per_token: f32,
        projected_state_bytes: u32,
        fits_envelope: bool,
    ) -> Result<Self, ShadowContractError> {
        let summary = Self {
            variant,
            quality,
            schedule_cost,
            projected_bank_switches_per_token,
            projected_state_bytes,
            fits_envelope,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn from_frontier_and_spec(
        expected: SequenceStateVariantId,
        spec: &SequenceSemanticsSpec,
        frontier: &CheckpointFrontierPoint,
    ) -> Result<Self, ShadowContractError> {
        let actual = SequenceStateVariantId::for_spec(spec);
        if actual != expected {
            return Err(ShadowContractError::SequenceVariantMismatch { expected, actual });
        }
        Self::new(
            expected,
            frontier.quality,
            frontier.schedule_cost.clone(),
            frontier
                .schedule_cost
                .as_ref()
                .map(projected_bank_switches_per_token)
                .unwrap_or(0.0),
            spec.state_size().bytes_per_layer,
            frontier.projected_fit.fits,
        )
    }

    pub fn validate(&self) -> Result<(), ShadowContractError> {
        self.quality.validate()?;
        if let Some(schedule_cost) = &self.schedule_cost {
            validate_estimated_cost_delta(schedule_cost)?;
        }
        validate_finite(
            "sequence_variant.projected_bank_switches_per_token",
            self.projected_bank_switches_per_token,
        )?;
        validate_non_negative(
            "sequence_variant.projected_bank_switches_per_token",
            self.projected_bank_switches_per_token,
        )?;
        if self.projected_state_bytes == 0 {
            return Err(ShadowContractError::ZeroProjectedStateBytes);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for SequenceVariantSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SequenceVariantSummaryUnchecked::deserialize(deserializer)?;
        Self::new(
            value.variant,
            value.quality,
            value.schedule_cost,
            value.projected_bank_switches_per_token,
            value.projected_state_bytes,
            value.fits_envelope,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceVariantSummaryUnchecked {
    variant: SequenceStateVariantId,
    quality: QualitySummary,
    schedule_cost: Option<EstimatedCostDelta>,
    projected_bank_switches_per_token: f32,
    projected_state_bytes: u32,
    fits_envelope: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceStateComparisonReport {
    pub workload: WorkloadId,
    pub linear_state: SequenceVariantSummary,
    pub bounded_kv: SequenceVariantSummary,
    pub generated_at: String,
}

impl SequenceStateComparisonReport {
    pub fn new(
        workload: WorkloadId,
        linear_state: SequenceVariantSummary,
        bounded_kv: SequenceVariantSummary,
        generated_at: impl Into<String>,
    ) -> Result<Self, ShadowContractError> {
        let report = Self {
            workload,
            linear_state,
            bounded_kv,
            generated_at: generated_at.into(),
        };
        report.validate()?;
        Ok(report)
    }

    pub fn from_shadow_reports(
        workload: WorkloadId,
        generated_at: impl Into<String>,
        linear_spec: &SequenceSemanticsSpec,
        linear_report: &ShadowCompileReport,
        bounded_spec: &SequenceSemanticsSpec,
        bounded_report: &ShadowCompileReport,
    ) -> Result<Self, ShadowContractError> {
        Self::new(
            workload,
            SequenceVariantSummary::from_frontier_and_spec(
                SequenceStateVariantId::LinearState,
                linear_spec,
                &linear_report.frontier_point,
            )?,
            SequenceVariantSummary::from_frontier_and_spec(
                SequenceStateVariantId::BoundedKv,
                bounded_spec,
                &bounded_report.frontier_point,
            )?,
            generated_at,
        )
    }

    pub fn validate(&self) -> Result<(), ShadowContractError> {
        if self.workload.as_str().trim().is_empty() {
            return Err(ShadowContractError::EmptyWorkloadId { index: 0 });
        }
        if self.generated_at.trim().is_empty() {
            return Err(ShadowContractError::EmptyGeneratedAt);
        }
        if self.linear_state.variant != SequenceStateVariantId::LinearState {
            return Err(ShadowContractError::SequenceVariantMismatch {
                expected: SequenceStateVariantId::LinearState,
                actual: self.linear_state.variant,
            });
        }
        if self.bounded_kv.variant != SequenceStateVariantId::BoundedKv {
            return Err(ShadowContractError::SequenceVariantMismatch {
                expected: SequenceStateVariantId::BoundedKv,
                actual: self.bounded_kv.variant,
            });
        }
        self.linear_state.validate()?;
        self.bounded_kv.validate()?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for SequenceStateComparisonReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SequenceStateComparisonReportUnchecked::deserialize(deserializer)?;
        Self::new(
            value.workload,
            value.linear_state,
            value.bounded_kv,
            value.generated_at,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceStateComparisonReportUnchecked {
    workload: WorkloadId,
    linear_state: SequenceVariantSummary,
    bounded_kv: SequenceVariantSummary,
    generated_at: String,
}

impl CheckpointFrontierPoint {
    pub fn new(
        checkpoint: CheckpointId,
        quality: QualitySummary,
        conformance: ConformanceSummary,
        projected_fit: ProjectedFitSummary,
        schedule_cost: Option<EstimatedCostDelta>,
    ) -> Result<Self, ShadowContractError> {
        let point = Self {
            checkpoint,
            quality,
            conformance,
            projected_fit,
            schedule_cost,
        };
        point.validate()?;
        Ok(point)
    }

    pub fn validate(&self) -> Result<(), ShadowContractError> {
        if self.checkpoint.as_str().trim().is_empty() {
            return Err(ShadowContractError::EmptyCheckpointId);
        }
        self.quality.validate()?;
        self.conformance.validate()?;
        if let Some(schedule_cost) = &self.schedule_cost {
            validate_estimated_cost_delta(schedule_cost)?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CheckpointFrontierPoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = CheckpointFrontierPointUnchecked::deserialize(deserializer)?;
        Self::try_from(value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointFrontierPointUnchecked {
    checkpoint: CheckpointId,
    quality: QualitySummary,
    conformance: ConformanceSummary,
    projected_fit: ProjectedFitSummary,
    schedule_cost: Option<EstimatedCostDelta>,
}

impl TryFrom<CheckpointFrontierPointUnchecked> for CheckpointFrontierPoint {
    type Error = ShadowContractError;

    fn try_from(value: CheckpointFrontierPointUnchecked) -> Result<Self, Self::Error> {
        Self::new(
            value.checkpoint,
            value.quality,
            value.conformance,
            value.projected_fit,
            value.schedule_cost,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowPipelineConfig {
    pub full_compile: bool,
    pub timeout: Duration,
    pub output_root: PathBuf,
    pub frontier_size: u32,
}

impl ShadowPipelineConfig {
    #[must_use]
    pub fn static_budget_only() -> Self {
        Self {
            full_compile: false,
            timeout: DEFAULT_SHADOW_COMPILE_TIMEOUT,
            output_root: PathBuf::from("shadow_results"),
            frontier_size: 0,
        }
    }

    #[must_use]
    pub fn with_full_compile(mut self, full_compile: bool) -> Self {
        self.full_compile = full_compile;
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_output_root(mut self, output_root: impl Into<PathBuf>) -> Self {
        self.output_root = output_root.into();
        self
    }

    #[must_use]
    pub const fn with_frontier_size(mut self, frontier_size: u32) -> Self {
        self.frontier_size = frontier_size;
        self
    }
}

impl Default for ShadowPipelineConfig {
    fn default() -> Self {
        Self::static_budget_only()
    }
}

pub struct ShadowPipelineInput<'a, M: EmaUpdate> {
    pub step: u64,
    pub checkpoint: CheckpointId,
    pub quality: QualitySummary,
    pub ema_weights: &'a EmaWeights<M>,
}

pub struct DualSequenceStateShadowPipelineInput<'a, ML: EmaUpdate, MB: EmaUpdate> {
    pub generated_at: String,
    pub linear_spec: &'a SequenceSemanticsSpec,
    pub linear: ShadowPipelineInput<'a, ML>,
    pub bounded_spec: &'a SequenceSemanticsSpec,
    pub bounded: ShadowPipelineInput<'a, MB>,
}

#[derive(Debug, Clone)]
pub struct ShadowPipelineOwnedInput<M: EmaUpdate> {
    pub step: u64,
    pub checkpoint: CheckpointId,
    pub quality: QualitySummary,
    pub ema_weights: EmaWeights<M>,
}

#[derive(Debug, Clone)]
pub struct DualSequenceStateShadowPipelineOwnedInput<ML: EmaUpdate, MB: EmaUpdate> {
    pub generated_at: String,
    pub linear_spec: SequenceSemanticsSpec,
    pub linear: ShadowPipelineOwnedInput<ML>,
    pub bounded_spec: SequenceSemanticsSpec,
    pub bounded: ShadowPipelineOwnedInput<MB>,
}

impl<M: EmaUpdate> ShadowPipelineOwnedInput<M> {
    fn as_borrowed(&self) -> ShadowPipelineInput<'_, M> {
        ShadowPipelineInput {
            step: self.step,
            checkpoint: self.checkpoint.clone(),
            quality: self.quality,
            ema_weights: &self.ema_weights,
        }
    }
}

impl<ML: EmaUpdate, MB: EmaUpdate> DualSequenceStateShadowPipelineOwnedInput<ML, MB> {
    fn as_borrowed(&self) -> DualSequenceStateShadowPipelineInput<'_, ML, MB> {
        DualSequenceStateShadowPipelineInput {
            generated_at: self.generated_at.clone(),
            linear_spec: &self.linear_spec,
            linear: self.linear.as_borrowed(),
            bounded_spec: &self.bounded_spec,
            bounded: self.bounded.as_borrowed(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowStaticBudgetReport {
    pub request: CompileRequestRef,
    pub fits: bool,
    pub margin_bytes: i64,
    pub diagnostic: String,
}

impl ShadowStaticBudgetReport {
    pub fn new(
        request: CompileRequestRef,
        fits: bool,
        margin_bytes: i64,
        diagnostic: impl Into<String>,
    ) -> Result<Self, ShadowPipelineError> {
        let diagnostic = diagnostic.into();
        if diagnostic.trim().is_empty() {
            return Err(ShadowPipelineError::EmptyDiagnostic {
                stage: ShadowPipelineStage::StaticBudget,
            });
        }
        Ok(Self {
            request,
            fits,
            margin_bytes,
            diagnostic,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowConformanceReport {
    pub workload: WorkloadId,
    pub passes: bool,
    pub max_divergence: f32,
    pub diagnostic: String,
}

impl ShadowConformanceReport {
    pub fn new(
        workload: WorkloadId,
        passes: bool,
        max_divergence: f32,
        diagnostic: impl Into<String>,
    ) -> Result<Self, ShadowPipelineError> {
        let diagnostic = diagnostic.into();
        if diagnostic.trim().is_empty() {
            return Err(ShadowPipelineError::EmptyDiagnostic {
                stage: ShadowPipelineStage::Conformance,
            });
        }
        ConformanceSummary::new(passes, max_divergence).map_err(ShadowPipelineError::Contract)?;
        Ok(Self {
            workload,
            passes,
            max_divergence,
            diagnostic,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowRuntimeChromeRevalidationReport {
    pub request: CompileRequestRef,
    pub report: ExportRuntimeChromeRevalidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowBudgetDriftArtifact {
    pub reports: Vec<ShadowRuntimeChromeRevalidationReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowCompileReport {
    pub step: u64,
    pub checkpoint: CheckpointId,
    pub result_dir: PathBuf,
    pub ema_update_count: u64,
    pub frontier_point: CheckpointFrontierPoint,
    pub request_reports: Vec<ShadowStaticBudgetReport>,
    pub runtime_chrome_revalidations: Vec<ShadowRuntimeChromeRevalidationReport>,
    pub budget_drift_path: Option<PathBuf>,
    pub workload_reports: Vec<ShadowConformanceReport>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DualSequenceStateShadowCompileReport {
    pub linear_state: ShadowCompileReport,
    pub bounded_kv: ShadowCompileReport,
    pub comparison_report: SequenceStateComparisonReport,
    pub comparison_path: PathBuf,
}

pub trait ShadowPipelineSteps {
    fn validate_artifact(&self, artifact: &ModelArtifact) -> Result<(), ShadowPipelineError> {
        artifact
            .validate()
            .map_err(|error| ShadowPipelineError::ArtifactValidation(error.to_string()))
    }

    fn static_budget(
        &self,
        artifact: &ModelArtifact,
        request: &CompileRequestRef,
    ) -> Result<ShadowStaticBudgetReport, ShadowPipelineError>;

    fn runtime_chrome_revalidation(
        &self,
        _artifact: &ModelArtifact,
        _request: &CompileRequestRef,
    ) -> Result<Option<ExportRuntimeChromeRevalidationReport>, ShadowPipelineError> {
        Ok(None)
    }

    fn conformance(
        &self,
        artifact: &ModelArtifact,
        workload: &WorkloadId,
    ) -> Result<ShadowConformanceReport, ShadowPipelineError>;

    fn schedule_cost(
        &self,
        _artifact: &ModelArtifact,
        _request: &CompileRequestRef,
    ) -> Result<Option<EstimatedCostDelta>, ShadowPipelineError> {
        Ok(None)
    }
}

pub fn run_shadow_compile_pipeline<M, S>(
    policy: &ShadowCompilePolicy,
    input: ShadowPipelineInput<'_, M>,
    steps: &S,
    config: &ShadowPipelineConfig,
    log_emitter: Option<&TrainingLogEmitter>,
) -> Result<ShadowCompileReport, ShadowPipelineError>
where
    M: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel,
    S: ShadowPipelineSteps,
{
    policy.validate().map_err(ShadowPipelineError::Contract)?;
    check_timeout(Instant::now(), config.timeout, ShadowPipelineStage::Start)?;

    let start = Instant::now();
    let result_dir = config.output_root.join(format!("step_{}", input.step));
    let visitor = ExportVisitor::pinned();
    let exported = input.ema_weights.export_checkpoint_artifact(&visitor)?;
    check_timeout(start, config.timeout, ShadowPipelineStage::EmaExport)?;

    steps.validate_artifact(&exported.artifact)?;
    check_timeout(
        start,
        config.timeout,
        ShadowPipelineStage::ArtifactValidation,
    )?;

    let mut request_reports = Vec::with_capacity(policy.requests.len());
    let mut runtime_chrome_revalidations = Vec::new();
    let mut fits_all_requests = true;
    let mut min_margin_bytes: Option<i64> = None;
    for request in &policy.requests {
        let report = steps.static_budget(&exported.artifact, request)?;
        fits_all_requests &= report.fits;
        min_margin_bytes = Some(match min_margin_bytes {
            Some(current) => current.min(report.margin_bytes),
            None => report.margin_bytes,
        });
        request_reports.push(report);

        if let Some(report) = steps.runtime_chrome_revalidation(&exported.artifact, request)? {
            if report.blocks_export {
                fits_all_requests = false;
                let blocking_margin = blocking_preflight_margin(&report).unwrap_or(-1);
                min_margin_bytes = Some(match min_margin_bytes {
                    Some(current) => current.min(blocking_margin),
                    None => blocking_margin,
                });
            }
            runtime_chrome_revalidations.push(ShadowRuntimeChromeRevalidationReport {
                request: request.clone(),
                report,
            });
        }
        check_timeout(start, config.timeout, ShadowPipelineStage::StaticBudget)?;
    }

    let budget_drift_path = if runtime_chrome_revalidations.is_empty() {
        None
    } else {
        Some(write_shadow_budget_drift_artifact(
            &result_dir,
            &runtime_chrome_revalidations,
        )?)
    };

    let mut workload_reports = Vec::new();
    let conformance = if fits_all_requests {
        let mut passes = true;
        let mut max_divergence = 0.0_f32;
        workload_reports.reserve(policy.workloads.len());
        for workload in &policy.workloads {
            let report = steps.conformance(&exported.artifact, workload)?;
            passes &= report.passes;
            max_divergence = max_divergence.max(report.max_divergence);
            workload_reports.push(report);
            check_timeout(start, config.timeout, ShadowPipelineStage::Conformance)?;
        }
        ConformanceSummary::new(passes, max_divergence).map_err(ShadowPipelineError::Contract)?
    } else {
        ConformanceSummary::new(false, 0.0).map_err(ShadowPipelineError::Contract)?
    };

    let mut schedule_cost = None;
    if config.full_compile && fits_all_requests {
        for request in &policy.requests {
            if schedule_cost.is_none() {
                schedule_cost = steps.schedule_cost(&exported.artifact, request)?;
            } else {
                let _ = steps.schedule_cost(&exported.artifact, request)?;
            }
            check_timeout(start, config.timeout, ShadowPipelineStage::ScheduleCost)?;
        }
    }

    let projected_fit =
        ProjectedFitSummary::new(fits_all_requests, min_margin_bytes.unwrap_or_default());
    let frontier_point = CheckpointFrontierPoint::new(
        input.checkpoint.clone(),
        input.quality,
        conformance,
        projected_fit,
        schedule_cost,
    )
    .map_err(ShadowPipelineError::Contract)?;
    let duration_ms = duration_millis(start.elapsed());

    if let Some(emitter) = log_emitter {
        emitter.shadow_compile(&shadow_compile_event(
            &input,
            policy,
            config,
            &frontier_point,
            duration_ms,
        ))?;
    }

    Ok(ShadowCompileReport {
        step: input.step,
        checkpoint: input.checkpoint,
        result_dir,
        ema_update_count: exported.update_count,
        frontier_point,
        request_reports,
        runtime_chrome_revalidations,
        budget_drift_path,
        workload_reports,
        duration_ms,
    })
}

pub fn run_shadow_compile_pipeline_owned<M, S>(
    policy: &ShadowCompilePolicy,
    input: &ShadowPipelineOwnedInput<M>,
    steps: &S,
    config: &ShadowPipelineConfig,
    log_emitter: Option<&TrainingLogEmitter>,
) -> Result<ShadowCompileReport, ShadowPipelineError>
where
    M: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel,
    S: ShadowPipelineSteps,
{
    run_shadow_compile_pipeline(policy, input.as_borrowed(), steps, config, log_emitter)
}

pub fn run_dual_sequence_state_shadow_compile_pipeline<ML, MB, S>(
    policy: &ShadowCompilePolicy,
    input: DualSequenceStateShadowPipelineInput<'_, ML, MB>,
    steps: &S,
    config: &ShadowPipelineConfig,
    log_emitter: Option<&TrainingLogEmitter>,
) -> Result<DualSequenceStateShadowCompileReport, ShadowPipelineError>
where
    ML: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel,
    MB: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel,
    S: ShadowPipelineSteps,
{
    policy.validate().map_err(ShadowPipelineError::Contract)?;
    if !policy.dual_sequence_state {
        return Err(ShadowPipelineError::Contract(
            ShadowContractError::DualSequenceStateDisabled,
        ));
    }

    let comparison_step = input.linear.step;
    let workload = policy
        .workloads
        .first()
        .cloned()
        .ok_or(ShadowPipelineError::Contract(
            ShadowContractError::EmptyWorkloads,
        ))?;
    let linear_spec = input.linear_spec;
    let bounded_spec = input.bounded_spec;
    let generated_at = input.generated_at;
    let linear_config = variant_shadow_config(config, "linear_state");
    let bounded_config = variant_shadow_config(config, "bounded_kv");
    let comparison_result_dir = config.output_root.join(format!("step_{comparison_step}"));

    let linear_state =
        run_shadow_compile_pipeline(policy, input.linear, steps, &linear_config, log_emitter)?;
    let bounded_kv =
        run_shadow_compile_pipeline(policy, input.bounded, steps, &bounded_config, log_emitter)?;
    let comparison_report = SequenceStateComparisonReport::from_shadow_reports(
        workload,
        generated_at,
        linear_spec,
        &linear_state,
        bounded_spec,
        &bounded_kv,
    )
    .map_err(ShadowPipelineError::Contract)?;
    let comparison_path =
        write_sequence_state_comparison_report(&comparison_result_dir, &comparison_report)?;

    Ok(DualSequenceStateShadowCompileReport {
        linear_state,
        bounded_kv,
        comparison_report,
        comparison_path,
    })
}

pub fn run_dual_sequence_state_shadow_compile_pipeline_owned<ML, MB, S>(
    policy: &ShadowCompilePolicy,
    input: &DualSequenceStateShadowPipelineOwnedInput<ML, MB>,
    steps: &S,
    config: &ShadowPipelineConfig,
    log_emitter: Option<&TrainingLogEmitter>,
) -> Result<DualSequenceStateShadowCompileReport, ShadowPipelineError>
where
    ML: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel,
    MB: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel,
    S: ShadowPipelineSteps,
{
    run_dual_sequence_state_shadow_compile_pipeline(
        policy,
        input.as_borrowed(),
        steps,
        config,
        log_emitter,
    )
}

pub fn spawn_shadow_compile_pipeline<M, S>(
    policy: ShadowCompilePolicy,
    input: ShadowPipelineOwnedInput<M>,
    steps: S,
    config: ShadowPipelineConfig,
    log_emitter: Option<TrainingLogEmitter>,
) -> JoinHandle<Result<ShadowCompileReport, ShadowPipelineError>>
where
    M: EmaUpdate + ArtifactExportModel + HardTernaryStudentModel + Send + 'static,
    S: ShadowPipelineSteps + Send + 'static,
{
    thread::spawn(move || {
        run_shadow_compile_pipeline_owned(&policy, &input, &steps, &config, log_emitter.as_ref())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowPipelineStage {
    Start,
    EmaExport,
    ArtifactValidation,
    StaticBudget,
    Conformance,
    ScheduleCost,
    RuntimeChromeRevalidation,
    SequenceStateComparison,
}

impl fmt::Display for ShadowPipelineStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Start => "start",
            Self::EmaExport => "ema_export",
            Self::ArtifactValidation => "artifact_validation",
            Self::StaticBudget => "static_budget",
            Self::Conformance => "conformance",
            Self::ScheduleCost => "schedule_cost",
            Self::RuntimeChromeRevalidation => "runtime_chrome_revalidation",
            Self::SequenceStateComparison => "sequence_state_comparison",
        };
        f.write_str(name)
    }
}

#[derive(Debug)]
pub enum ShadowPipelineError {
    Contract(ShadowContractError),
    Ema(EmaExportError),
    ArtifactValidation(String),
    StaticBudget(String),
    RuntimeChromeRevalidation(String),
    Conformance(String),
    ScheduleCost(String),
    SequenceStateComparison(String),
    EmptyDiagnostic {
        stage: ShadowPipelineStage,
    },
    Timeout {
        stage: ShadowPipelineStage,
        timeout: Duration,
    },
    Logging(LoggingEventError),
}

impl fmt::Display for ShadowPipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => write!(f, "{error}"),
            Self::Ema(error) => write!(f, "{error}"),
            Self::ArtifactValidation(message) => write!(f, "artifact validation failed: {message}"),
            Self::StaticBudget(message) => write!(f, "static budget failed: {message}"),
            Self::RuntimeChromeRevalidation(message) => {
                write!(f, "runtime chrome revalidation failed: {message}")
            }
            Self::Conformance(message) => write!(f, "conformance failed: {message}"),
            Self::ScheduleCost(message) => write!(f, "schedule cost failed: {message}"),
            Self::SequenceStateComparison(message) => {
                write!(f, "sequence state comparison failed: {message}")
            }
            Self::EmptyDiagnostic { stage } => {
                write!(f, "shadow pipeline {stage} diagnostic must not be empty")
            }
            Self::Timeout { stage, timeout } => {
                write!(
                    f,
                    "shadow pipeline timed out during {stage} after {timeout:?}"
                )
            }
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ShadowPipelineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            Self::Ema(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::ArtifactValidation(_)
            | Self::StaticBudget(_)
            | Self::RuntimeChromeRevalidation(_)
            | Self::Conformance(_)
            | Self::ScheduleCost(_)
            | Self::SequenceStateComparison(_)
            | Self::EmptyDiagnostic { .. }
            | Self::Timeout { .. } => None,
        }
    }
}

impl From<EmaExportError> for ShadowPipelineError {
    fn from(error: EmaExportError) -> Self {
        Self::Ema(error)
    }
}

impl From<LoggingEventError> for ShadowPipelineError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

fn check_timeout(
    start: Instant,
    timeout: Duration,
    stage: ShadowPipelineStage,
) -> Result<(), ShadowPipelineError> {
    if start.elapsed() >= timeout {
        return Err(ShadowPipelineError::Timeout { stage, timeout });
    }
    Ok(())
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn blocking_preflight_margin(report: &ExportRuntimeChromeRevalidationReport) -> Option<i64> {
    report
        .preflight
        .checks
        .iter()
        .filter(|check| check.status == "fail")
        .map(|check| {
            i64::try_from(check.over_by_bytes)
                .unwrap_or(i64::MAX)
                .saturating_neg()
        })
        .min()
}

fn write_shadow_budget_drift_artifact(
    result_dir: &Path,
    reports: &[ShadowRuntimeChromeRevalidationReport],
) -> Result<PathBuf, ShadowPipelineError> {
    fs::create_dir_all(result_dir)
        .map_err(|error| ShadowPipelineError::RuntimeChromeRevalidation(error.to_string()))?;
    let path = result_dir.join(BUDGET_DRIFT_FILE_NAME);
    let artifact = ShadowBudgetDriftArtifact {
        reports: reports.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&artifact)
        .map_err(|error| ShadowPipelineError::RuntimeChromeRevalidation(error.to_string()))?;
    fs::write(&path, bytes)
        .map_err(|error| ShadowPipelineError::RuntimeChromeRevalidation(error.to_string()))?;
    Ok(path)
}

fn write_sequence_state_comparison_report(
    result_dir: &Path,
    report: &SequenceStateComparisonReport,
) -> Result<PathBuf, ShadowPipelineError> {
    fs::create_dir_all(result_dir)
        .map_err(|error| ShadowPipelineError::SequenceStateComparison(error.to_string()))?;
    let path = result_dir.join(SEQUENCE_STATE_COMPARISON_FILE_NAME);
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| ShadowPipelineError::SequenceStateComparison(error.to_string()))?;
    fs::write(&path, bytes)
        .map_err(|error| ShadowPipelineError::SequenceStateComparison(error.to_string()))?;
    Ok(path)
}

fn variant_shadow_config(config: &ShadowPipelineConfig, variant_dir: &str) -> ShadowPipelineConfig {
    ShadowPipelineConfig {
        full_compile: config.full_compile,
        timeout: config.timeout,
        output_root: config.output_root.join(variant_dir),
        frontier_size: config.frontier_size,
    }
}

fn shadow_compile_event<M: EmaUpdate>(
    input: &ShadowPipelineInput<'_, M>,
    policy: &ShadowCompilePolicy,
    config: &ShadowPipelineConfig,
    point: &CheckpointFrontierPoint,
    duration_ms: u64,
) -> ShadowCompileEvent {
    ShadowCompileEvent {
        step: input.step,
        checkpoint_id: input.checkpoint.to_string(),
        compile_profile: policy
            .requests
            .first()
            .map(|request| request.as_path().display().to_string())
            .unwrap_or_else(|| "unknown".to_owned()),
        fit_status: if point.projected_fit.fits {
            "fits".to_owned()
        } else {
            "no_fit".to_owned()
        },
        quality_summary: format!(
            "lm_loss={:.6},perplexity={:.6}",
            point.quality.lm_loss, point.quality.perplexity
        ),
        frontier_size: config.frontier_size,
        duration_ms,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShadowContractError {
    InvalidEveryNSteps {
        every_n_steps: u64,
    },
    InvalidKeepFrontier {
        keep_frontier: usize,
    },
    EmptyCompileRequests,
    EmptyCompileRequestRef {
        index: Option<usize>,
    },
    EmptyWorkloads,
    EmptyWorkloadId {
        index: usize,
    },
    EmptyCheckpointId,
    NonFiniteMetric {
        field: &'static str,
        value: f32,
    },
    NegativeMetric {
        field: &'static str,
        value: f32,
    },
    NonPositiveMetric {
        field: &'static str,
        value: f32,
    },
    MalformedCostEnvelope {
        field: &'static str,
    },
    NegativeCostEnvelope {
        field: &'static str,
    },
    ZeroProjectedStateBytes,
    EmptyGeneratedAt,
    DualSequenceStateDisabled,
    SequenceVariantMismatch {
        expected: SequenceStateVariantId,
        actual: SequenceStateVariantId,
    },
}

impl fmt::Display for ShadowContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEveryNSteps { every_n_steps } => {
                write!(
                    f,
                    "shadow compile every_n_steps must be greater than zero, got {every_n_steps}"
                )
            }
            Self::InvalidKeepFrontier { keep_frontier } => {
                write!(
                    f,
                    "shadow compile keep_frontier must be greater than zero, got {keep_frontier}"
                )
            }
            Self::EmptyCompileRequests => {
                write!(f, "shadow compile requests must not be empty")
            }
            Self::EmptyCompileRequestRef { index } => match index {
                Some(index) => write!(f, "shadow compile request at index {index} is empty"),
                None => write!(f, "shadow compile request path is empty"),
            },
            Self::EmptyWorkloads => write!(f, "shadow compile workloads must not be empty"),
            Self::EmptyWorkloadId { index } => {
                write!(f, "shadow compile workload at index {index} is empty")
            }
            Self::EmptyCheckpointId => write!(f, "checkpoint cannot be empty"),
            Self::NonFiniteMetric { field, value } => {
                write!(f, "{field} must be finite, got {value}")
            }
            Self::NegativeMetric { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::NonPositiveMetric { field, value } => {
                write!(f, "{field} must be greater than zero, got {value}")
            }
            Self::MalformedCostEnvelope { field } => {
                write!(
                    f,
                    "{field} cost envelope must satisfy lower <= p50 <= upper <= p99"
                )
            }
            Self::NegativeCostEnvelope { field } => {
                write!(f, "{field} cost envelope must be non-negative")
            }
            Self::ZeroProjectedStateBytes => {
                write!(
                    f,
                    "sequence variant projected_state_bytes must be greater than zero"
                )
            }
            Self::EmptyGeneratedAt => {
                write!(
                    f,
                    "sequence state comparison generated_at must not be empty"
                )
            }
            Self::DualSequenceStateDisabled => {
                write!(
                    f,
                    "dual_sequence_state must be true for dual sequence-state shadow compile"
                )
            }
            Self::SequenceVariantMismatch { expected, actual } => {
                write!(
                    f,
                    "sequence variant mismatch: expected {expected:?}, got {actual:?}"
                )
            }
        }
    }
}

impl Error for ShadowContractError {}

fn validate_finite(field: &'static str, value: f32) -> Result<(), ShadowContractError> {
    if !value.is_finite() {
        return Err(ShadowContractError::NonFiniteMetric { field, value });
    }
    Ok(())
}

fn validate_non_negative(field: &'static str, value: f32) -> Result<(), ShadowContractError> {
    if value < 0.0 {
        return Err(ShadowContractError::NegativeMetric { field, value });
    }
    Ok(())
}

fn validate_positive(field: &'static str, value: f32) -> Result<(), ShadowContractError> {
    if value <= 0.0 {
        return Err(ShadowContractError::NonPositiveMetric { field, value });
    }
    Ok(())
}

fn validate_estimated_cost_delta(delta: &EstimatedCostDelta) -> Result<(), ShadowContractError> {
    for (field, estimate) in cost_estimates(delta) {
        if !estimate.envelope.is_ordered() {
            return Err(ShadowContractError::MalformedCostEnvelope { field });
        }
        if !estimate.envelope.is_non_negative() {
            return Err(ShadowContractError::NegativeCostEnvelope { field });
        }
    }
    Ok(())
}

fn projected_bank_switches_per_token(delta: &EstimatedCostDelta) -> f32 {
    delta.bank_switches_per_token.envelope.p50_q16_16 as f32
        / gbf_policy::UncertaintyEnvelope::Q16_ONE as f32
}

fn cost_estimates(delta: &EstimatedCostDelta) -> Vec<(&'static str, &CostEstimate)> {
    let mut estimates = vec![
        ("schedule_cost.cycles_per_token", &delta.cycles_per_token),
        (
            "schedule_cost.bank_switches_per_token",
            &delta.bank_switches_per_token,
        ),
        ("schedule_cost.yields_per_token", &delta.yields_per_token),
        (
            "schedule_cost.scheduler_headroom_utilization",
            &delta.scheduler_headroom_utilization,
        ),
        (
            "schedule_cost.max_no_progress_estimate",
            &delta.max_no_progress_estimate,
        ),
        (
            "schedule_cost.time_to_first_token",
            &delta.time_to_first_token,
        ),
        (
            "schedule_cost.sustained_throughput_tokens_per_megacycle",
            &delta.sustained_throughput_tokens_per_megacycle,
        ),
    ];
    if let Some(estimate) = &delta.sram_page_switches_per_token {
        estimates.push(("schedule_cost.sram_page_switches_per_token", estimate));
    }
    if let Some(estimate) = &delta.video_commit_cost_margin {
        estimates.push(("schedule_cost.video_commit_cost_margin", estimate));
    }
    if let Some(estimate) = &delta.frame_jitter {
        estimates.push(("schedule_cost.frame_jitter", estimate));
    }
    estimates
}

#[cfg(test)]
pub mod dual_path {
    use std::path::PathBuf;

    use gbf_foundation::{CheckpointId, WorkloadId};
    use gbf_model::config::SharedSequenceConfig;
    use gbf_policy::{CostEstimate, EvidenceClass, UncertaintyEnvelope};
    use serde_json::json;

    use super::*;

    #[test]
    fn shadow_dual_path_builds_sequence_state_comparison_report() {
        let workload = WorkloadId::from("tiny-smoke");
        let linear = shadow_report(
            CheckpointId::from("linear.ckpt"),
            QualitySummary::new(1.0, 2.5).unwrap(),
            Some(cost_delta(1)),
            256,
        );
        let bounded = shadow_report(
            CheckpointId::from("bounded.ckpt"),
            QualitySummary::new(0.875, 2.25).unwrap(),
            Some(cost_delta(4)),
            192,
        );

        let report = SequenceStateComparisonReport::from_shadow_reports(
            workload.clone(),
            "2026-06-07T00:00:00Z",
            &linear_state_spec(8),
            &linear,
            &bounded_kv_spec(2, 12),
            &bounded,
        )
        .unwrap();

        assert_eq!(report.workload, workload);
        assert_eq!(
            report.linear_state.variant,
            SequenceStateVariantId::LinearState
        );
        assert_eq!(report.linear_state.projected_state_bytes, 8);
        assert_eq!(report.linear_state.projected_bank_switches_per_token, 1.0);
        assert_eq!(report.bounded_kv.variant, SequenceStateVariantId::BoundedKv);
        assert_eq!(report.bounded_kv.projected_state_bytes, 24);
        assert_eq!(report.bounded_kv.projected_bank_switches_per_token, 4.0);
        assert!(report.linear_state.fits_envelope);
        assert!(report.bounded_kv.fits_envelope);

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["linear_state"]["variant"], json!("linear_state"));
        assert_eq!(value["bounded_kv"]["variant"], json!("bounded_kv"));
    }

    #[test]
    fn shadow_dual_path_rejects_swapped_sequence_specs() {
        let linear = shadow_report(
            CheckpointId::from("linear.ckpt"),
            QualitySummary::new(1.0, 2.5).unwrap(),
            None,
            256,
        );
        let bounded = shadow_report(
            CheckpointId::from("bounded.ckpt"),
            QualitySummary::new(0.875, 2.25).unwrap(),
            None,
            192,
        );

        let err = SequenceStateComparisonReport::from_shadow_reports(
            WorkloadId::from("tiny-smoke"),
            "2026-06-07T00:00:00Z",
            &bounded_kv_spec(2, 12),
            &linear,
            &linear_state_spec(8),
            &bounded,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ShadowContractError::SequenceVariantMismatch {
                expected: SequenceStateVariantId::LinearState,
                actual: SequenceStateVariantId::BoundedKv,
            }
        ));
    }

    fn shadow_report(
        checkpoint: CheckpointId,
        quality: QualitySummary,
        schedule_cost: Option<EstimatedCostDelta>,
        margin_bytes: i64,
    ) -> ShadowCompileReport {
        let frontier_point = CheckpointFrontierPoint::new(
            checkpoint.clone(),
            quality,
            ConformanceSummary::new(true, 0.0).unwrap(),
            ProjectedFitSummary::new(true, margin_bytes),
            schedule_cost,
        )
        .unwrap();

        ShadowCompileReport {
            step: 1,
            checkpoint,
            result_dir: PathBuf::from("shadow_results/step_1"),
            ema_update_count: 1,
            frontier_point,
            request_reports: Vec::new(),
            runtime_chrome_revalidations: Vec::new(),
            budget_drift_path: None,
            workload_reports: Vec::new(),
            duration_ms: 0,
        }
    }

    fn linear_state_spec(state_bytes_per_layer: u16) -> SequenceSemanticsSpec {
        SharedSequenceConfig::linear_state(
            usize::from(state_bytes_per_layer),
            state_bytes_per_layer,
        )
        .unwrap()
        .sequence_semantics()
    }

    fn bounded_kv_spec(max_context: u16, kv_bytes_per_token: u16) -> SequenceSemanticsSpec {
        SharedSequenceConfig::bounded_kv(
            usize::from(kv_bytes_per_token),
            max_context,
            kv_bytes_per_token,
        )
        .unwrap()
        .sequence_semantics()
    }

    fn cost_delta(bank_switches_per_token: i64) -> EstimatedCostDelta {
        EstimatedCostDelta {
            cycles_per_token: estimate(10),
            bank_switches_per_token: estimate(bank_switches_per_token),
            sram_page_switches_per_token: None,
            yields_per_token: estimate(1),
            scheduler_headroom_utilization: estimate(0),
            video_commit_cost_margin: None,
            max_no_progress_estimate: estimate(0),
            time_to_first_token: estimate(100),
            sustained_throughput_tokens_per_megacycle: estimate(5),
            frame_jitter: None,
        }
    }

    fn estimate(units: i64) -> CostEstimate {
        CostEstimate {
            evidence_class: EvidenceClass::Heuristic,
            envelope: UncertaintyEnvelope::exact_units(units),
            refs: Vec::new(),
            fallback_reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::*;

    use gbf_artifact::ids::ArtifactPath;
    use gbf_artifact::{
        CanonicalTensor, Dtype, PayloadRole, QuantSpec_S3, WeightQuant, canonical_payload_sha,
    };
    use gbf_foundation::{
        BudgetSlotId, ByteCost, CompileProfileId, Hash256, TargetProfileId, sha256,
    };
    use gbf_model::config::SharedSequenceConfig;
    use gbf_policy::model_profile::ModelSizeProfile;
    use gbf_policy::{
        BudgetSlotClass, CostEstimate, EvidenceClass, PlacementProfile, RomBudgetSlot,
        RuntimeChromeBudget, RuntimeMemoryCapSection, RuntimeNucleusHash, RuntimeShellModule,
        UncertaintyEnvelope, WramReserved,
    };
    use serde_json::json;

    use crate::ema::{EmaDecay, EmaWeights, update_ema_slice};
    use crate::export::revalidate_runtime_chrome_budget_for_export;
    use crate::export_visitor::{ArtifactExportModel, ExportVisitorError};
    use crate::logging::{TestEventCollector, TestEventKind, TestFieldValue, TrainingLogEmitter};
    use crate::preflight::RuntimeChromePreflightDemand;
    use crate::student::{
        HardTernaryStudentModel, StudentStorageFingerprint, StudentWeightFingerprint,
    };

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TrainingConfig {
        training: TrainingSection,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TrainingSection {
        shadow_compile: ShadowCompilePolicy,
    }

    #[test]
    fn shadow_compile_policy_toml_round_trip_pins_nested_shape() {
        let toml = r#"
[training.shadow_compile]
every_n_steps = 4000
requests = ["compile/bringup.toml", "compile/trace.toml"]
workloads = ["smoke", "quality"]
keep_frontier = 3
"#;

        let decoded: TrainingConfig = toml::from_str(toml).expect("policy TOML decodes");
        assert_eq!(decoded.training.shadow_compile.every_n_steps, 4000);
        assert_eq!(decoded.training.shadow_compile.keep_frontier, 3);
        assert_eq!(
            decoded.training.shadow_compile.requests[0].as_path(),
            Path::new("compile/bringup.toml")
        );
        assert_eq!(
            decoded.training.shadow_compile.workloads,
            vec![WorkloadId::from("smoke"), WorkloadId::from("quality")]
        );

        let reencoded = toml::to_string(&decoded).expect("policy TOML encodes");
        let round_trip: TrainingConfig =
            toml::from_str(&reencoded).expect("reencoded policy TOML decodes");
        assert_eq!(round_trip, decoded);
    }

    #[test]
    fn shadow_compile_policy_validation_rejects_unusable_values() {
        let err = ShadowCompilePolicy::new(
            0,
            vec![CompileRequestRef::new("compile/bringup.toml").unwrap()],
            vec![WorkloadId::from("smoke")],
            2,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ShadowContractError::InvalidEveryNSteps { every_n_steps: 0 }
        ));

        let err: toml::de::Error = toml::from_str::<TrainingConfig>(
            r#"
[training.shadow_compile]
every_n_steps = 10
requests = [""]
workloads = ["smoke"]
keep_frontier = 1
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("request path is empty"));

        let err: toml::de::Error = toml::from_str::<TrainingConfig>(
            r#"
[training.shadow_compile]
every_n_steps = 10
requests = ["compile/bringup.toml"]
workloads = [""]
keep_frontier = 1
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("workload at index 0 is empty"));
    }

    #[test]
    fn checkpoint_frontier_point_json_shape_is_pinned() {
        let point = fixture_point(None);
        let value = serde_json::to_value(&point).expect("frontier point serializes");

        assert_eq!(
            value,
            json!({
                "checkpoint": "checkpoint.phase-e.004",
                "quality": {
                    "lm_loss": 1.25,
                    "perplexity": 3.5
                },
                "conformance": {
                    "passes": true,
                    "max_divergence": 0.015625
                },
                "projected_fit": {
                    "fits": true,
                    "margin_bytes": 1024
                },
                "schedule_cost": null
            })
        );

        let decoded: CheckpointFrontierPoint =
            serde_json::from_value(value).expect("frontier point deserializes");
        assert_eq!(decoded, point);
    }

    #[test]
    fn checkpoint_frontier_point_validation_rejects_bad_summary_metrics() {
        let err = QualitySummary::new(f32::NAN, 3.5).unwrap_err();
        assert!(matches!(
            err,
            ShadowContractError::NonFiniteMetric {
                field: "quality.lm_loss",
                ..
            }
        ));

        let err: serde_json::Error = serde_json::from_value::<CheckpointFrontierPoint>(json!({
            "checkpoint": "checkpoint.phase-e.004",
            "quality": {
                "lm_loss": 1.25,
                "perplexity": 0.0
            },
            "conformance": {
                "passes": true,
                "max_divergence": 0.015625
            },
            "projected_fit": {
                "fits": true,
                "margin_bytes": 1024
            },
            "schedule_cost": null
        }))
        .unwrap_err();
        assert!(err.to_string().contains("quality.perplexity"));

        let err: serde_json::Error = serde_json::from_value::<CheckpointFrontierPoint>(json!({
            "checkpoint": "checkpoint.phase-e.004",
            "quality": {
                "lm_loss": 1.25,
                "perplexity": 3.5
            },
            "conformance": {
                "passes": true,
                "max_divergence": -0.01
            },
            "projected_fit": {
                "fits": true,
                "margin_bytes": 1024
            },
            "schedule_cost": null
        }))
        .unwrap_err();
        assert!(err.to_string().contains("conformance.max_divergence"));
    }

    #[test]
    fn checkpoint_frontier_point_validation_rejects_bad_schedule_cost_envelope() {
        let mut schedule_cost = fixture_cost_delta();
        schedule_cost.cycles_per_token.envelope = UncertaintyEnvelope::from_q16(
            20 * UncertaintyEnvelope::Q16_ONE,
            30 * UncertaintyEnvelope::Q16_ONE,
            40 * UncertaintyEnvelope::Q16_ONE,
            Some(50 * UncertaintyEnvelope::Q16_ONE),
        );

        let err = CheckpointFrontierPoint::new(
            CheckpointId::from("checkpoint.phase-e.004"),
            QualitySummary::new(1.25, 3.5).unwrap(),
            ConformanceSummary::new(true, 0.015625).unwrap(),
            ProjectedFitSummary::new(true, 1024),
            Some(schedule_cost),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ShadowContractError::MalformedCostEnvelope {
                field: "schedule_cost.cycles_per_token"
            }
        ));
    }

    #[test]
    fn shadow_pipeline_produces_frontier_point_and_summary_log() {
        let policy = fixture_policy();
        let input = fixture_owned_input();
        let steps = FixtureSteps::fitting(Some(fixture_cost_delta()));
        let config = ShadowPipelineConfig::default()
            .with_full_compile(true)
            .with_frontier_size(3);
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let report =
            run_shadow_compile_pipeline_owned(&policy, &input, &steps, &config, Some(&emitter))
                .expect("shadow pipeline succeeds");

        assert_eq!(report.step, 20);
        assert_eq!(
            report.checkpoint,
            CheckpointId::from("checkpoint.phase-e.020")
        );
        assert_eq!(report.result_dir, PathBuf::from("shadow_results/step_20"));
        assert_eq!(report.ema_update_count, 1);
        assert!(report.frontier_point.projected_fit.fits);
        assert_eq!(report.frontier_point.projected_fit.margin_bytes, 512);
        assert!(report.frontier_point.conformance.passes);
        assert_eq!(report.frontier_point.conformance.max_divergence, 0.015625);
        assert!(report.frontier_point.schedule_cost.is_some());
        assert_eq!(report.request_reports.len(), 1);
        assert_eq!(report.workload_reports.len(), 1);

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::ShadowCompile);
        assert_eq!(events[0].field("step"), Some(&TestFieldValue::U64(20)));
        assert_eq!(
            events[0].field("checkpoint_id"),
            Some(&TestFieldValue::String("checkpoint.phase-e.020".to_owned()))
        );
        assert_eq!(
            events[0].field("compile_profile"),
            Some(&TestFieldValue::String("compile/bringup.toml".to_owned()))
        );
        assert_eq!(
            events[0].field("fit_status"),
            Some(&TestFieldValue::String("fits".to_owned()))
        );
        assert_eq!(
            events[0].field("quality_summary"),
            Some(&TestFieldValue::String(
                "lm_loss=1.000000,perplexity=2.500000".to_owned()
            ))
        );
        assert_eq!(
            events[0].field("frontier_size"),
            Some(&TestFieldValue::U64(3))
        );
        assert!(matches!(
            events[0].field("duration_ms"),
            Some(TestFieldValue::U64(_))
        ));
    }

    #[test]
    fn shadow_pipeline_budget_failure_returns_no_fit_without_crashing_training() {
        let policy = fixture_policy();
        let input = fixture_owned_input();
        let steps = FixtureSteps::budget_failure();

        let report = run_shadow_compile_pipeline_owned(
            &policy,
            &input,
            &steps,
            &ShadowPipelineConfig::default(),
            None,
        )
        .expect("budget failure is reported, not thrown");

        assert!(!report.frontier_point.projected_fit.fits);
        assert_eq!(report.frontier_point.projected_fit.margin_bytes, -128);
        assert!(!report.frontier_point.conformance.passes);
        assert!(report.workload_reports.is_empty());
        assert_eq!(
            report.request_reports[0].diagnostic,
            "expert bank over by 128 bytes"
        );
    }

    #[test]
    fn shadow_pipeline_timeout_aborts_before_expensive_steps() {
        let policy = fixture_policy();
        let input = fixture_owned_input();
        let steps = FixtureSteps::fitting(None);
        let config = ShadowPipelineConfig::default().with_timeout(Duration::ZERO);

        let err = run_shadow_compile_pipeline_owned(&policy, &input, &steps, &config, None)
            .expect_err("zero timeout aborts");

        assert!(matches!(
            err,
            ShadowPipelineError::Timeout {
                stage: ShadowPipelineStage::Start,
                ..
            }
        ));
    }

    #[test]
    fn shadow_pipeline_can_run_on_background_thread() {
        let handle = spawn_shadow_compile_pipeline(
            fixture_policy(),
            fixture_owned_input(),
            FixtureSteps::fitting(None),
            ShadowPipelineConfig::default(),
            None,
        );

        let report = handle
            .join()
            .expect("thread does not panic")
            .expect("background shadow pipeline succeeds");

        assert!(report.frontier_point.projected_fit.fits);
    }

    #[test]
    fn shadow_dual_sequence_state_pipeline_writes_comparison_report() {
        let output_root = unique_shadow_output_root("dual_sequence");
        let policy = fixture_policy().with_dual_sequence_state(true);
        let input = dual_fixture_owned_input();
        let steps = FixtureSteps::fitting(Some(fixture_cost_delta()));
        let config = ShadowPipelineConfig::default()
            .with_full_compile(true)
            .with_output_root(&output_root);

        let report = run_dual_sequence_state_shadow_compile_pipeline_owned(
            &policy, &input, &steps, &config, None,
        )
        .expect("dual sequence-state shadow pipeline succeeds");

        let comparison_path = output_root
            .join("step_20")
            .join(SEQUENCE_STATE_COMPARISON_FILE_NAME);
        assert_eq!(report.comparison_path, comparison_path);
        assert_eq!(
            report.linear_state.result_dir,
            output_root.join("linear_state").join("step_20")
        );
        assert_eq!(
            report.bounded_kv.result_dir,
            output_root.join("bounded_kv").join("step_20")
        );
        assert!(report.linear_state.frontier_point.projected_fit.fits);
        assert!(report.bounded_kv.frontier_point.projected_fit.fits);
        assert_eq!(
            report.comparison_report.linear_state.variant,
            SequenceStateVariantId::LinearState
        );
        assert_eq!(
            report.comparison_report.bounded_kv.variant,
            SequenceStateVariantId::BoundedKv
        );
        assert_eq!(
            report
                .comparison_report
                .linear_state
                .projected_bank_switches_per_token,
            2.0
        );

        let decoded: SequenceStateComparisonReport = serde_json::from_slice(
            &std::fs::read(&comparison_path).expect("sequence comparison report readable"),
        )
        .expect("sequence comparison report decodes");
        assert_eq!(decoded, report.comparison_report);

        let _ = std::fs::remove_dir_all(output_root);
    }

    #[test]
    fn shadow_pipeline_warning_revalidation_writes_budget_drift_and_continues() {
        let output_root = unique_shadow_output_root("warning");
        let steps = FixtureSteps::fitting(None).with_revalidation(warning_revalidation_report());

        let report = run_shadow_compile_pipeline_owned(
            &fixture_policy(),
            &fixture_owned_input(),
            &steps,
            &ShadowPipelineConfig::default().with_output_root(&output_root),
            None,
        )
        .expect("warning revalidation does not block shadow compile");

        let path = output_root.join("step_20").join(BUDGET_DRIFT_FILE_NAME);
        assert!(report.frontier_point.projected_fit.fits);
        assert_eq!(report.budget_drift_path.as_ref(), Some(&path));
        assert_eq!(report.runtime_chrome_revalidations.len(), 1);
        assert!(path.exists());
        let decoded: ShadowBudgetDriftArtifact = serde_json::from_slice(
            &std::fs::read(&path).expect("shadow budget drift report readable"),
        )
        .expect("shadow budget drift report decodes");
        assert_eq!(decoded.reports, report.runtime_chrome_revalidations);
        assert!(!decoded.reports[0].report.blocks_export);

        let _ = std::fs::remove_dir_all(output_root);
    }

    #[test]
    fn shadow_pipeline_blocking_revalidation_marks_no_fit_without_conformance() {
        let output_root = unique_shadow_output_root("blocking");
        let steps = FixtureSteps::fitting(Some(fixture_cost_delta()))
            .with_revalidation(blocking_revalidation_report());

        let report = run_shadow_compile_pipeline_owned(
            &fixture_policy(),
            &fixture_owned_input(),
            &steps,
            &ShadowPipelineConfig::default()
                .with_full_compile(true)
                .with_output_root(&output_root),
            None,
        )
        .expect("blocking revalidation is reported as no-fit, not thrown");

        let path = output_root.join("step_20").join(BUDGET_DRIFT_FILE_NAME);
        assert!(!report.frontier_point.projected_fit.fits);
        assert!(report.frontier_point.projected_fit.margin_bytes < 0);
        assert!(!report.frontier_point.conformance.passes);
        assert!(report.frontier_point.schedule_cost.is_none());
        assert!(report.workload_reports.is_empty());
        assert_eq!(report.budget_drift_path.as_ref(), Some(&path));
        assert!(report.runtime_chrome_revalidations[0].report.blocks_export);
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(output_root);
    }

    fn fixture_point(schedule_cost: Option<EstimatedCostDelta>) -> CheckpointFrontierPoint {
        CheckpointFrontierPoint::new(
            CheckpointId::from("checkpoint.phase-e.004"),
            QualitySummary::new(1.25, 3.5).unwrap(),
            ConformanceSummary::new(true, 0.015625).unwrap(),
            ProjectedFitSummary::new(true, 1024),
            schedule_cost,
        )
        .expect("fixture frontier point is valid")
    }

    fn fixture_cost_delta() -> EstimatedCostDelta {
        EstimatedCostDelta {
            cycles_per_token: estimate(10),
            bank_switches_per_token: estimate(2),
            sram_page_switches_per_token: None,
            yields_per_token: estimate(1),
            scheduler_headroom_utilization: estimate(0),
            video_commit_cost_margin: None,
            max_no_progress_estimate: estimate(0),
            time_to_first_token: estimate(100),
            sustained_throughput_tokens_per_megacycle: estimate(5),
            frame_jitter: None,
        }
    }

    fn estimate(units: i64) -> CostEstimate {
        CostEstimate {
            evidence_class: EvidenceClass::Heuristic,
            envelope: UncertaintyEnvelope::exact_units(units),
            refs: Vec::new(),
            fallback_reason: None,
        }
    }

    fn fixture_policy() -> ShadowCompilePolicy {
        ShadowCompilePolicy::new(
            20,
            vec![CompileRequestRef::new("compile/bringup.toml").unwrap()],
            vec![WorkloadId::from("smoke")],
            3,
        )
        .unwrap()
    }

    fn fixture_owned_input() -> ShadowPipelineOwnedInput<ToyShadowStudent> {
        let mut ema_weights = EmaWeights::new(
            ToyShadowStudent::new(vec![1.0, 3.0], true),
            EmaDecay::new(0.5).unwrap(),
        );
        ema_weights
            .update(&ToyShadowStudent::new(vec![3.0, 7.0], true))
            .unwrap();

        ShadowPipelineOwnedInput {
            step: 20,
            checkpoint: CheckpointId::from("checkpoint.phase-e.020"),
            quality: QualitySummary::new(1.0, 2.5).unwrap(),
            ema_weights,
        }
    }

    fn dual_fixture_owned_input()
    -> DualSequenceStateShadowPipelineOwnedInput<ToyShadowStudent, ToyShadowStudent> {
        DualSequenceStateShadowPipelineOwnedInput {
            generated_at: "2026-06-07T00:00:00Z".to_owned(),
            linear_spec: SharedSequenceConfig::linear_state(8, 8)
                .unwrap()
                .sequence_semantics(),
            linear: fixture_owned_input(),
            bounded_spec: SharedSequenceConfig::bounded_kv(12, 2, 12)
                .unwrap()
                .sequence_semantics(),
            bounded: fixture_owned_input(),
        }
    }

    #[derive(Debug, Clone)]
    struct FixtureSteps {
        budget_fits: bool,
        margin_bytes: i64,
        diagnostic: &'static str,
        schedule_cost: Option<EstimatedCostDelta>,
        runtime_revalidation: Option<ExportRuntimeChromeRevalidationReport>,
    }

    impl FixtureSteps {
        fn fitting(schedule_cost: Option<EstimatedCostDelta>) -> Self {
            Self {
                budget_fits: true,
                margin_bytes: 512,
                diagnostic: "fits with 512 bytes margin",
                schedule_cost,
                runtime_revalidation: None,
            }
        }

        fn budget_failure() -> Self {
            Self {
                budget_fits: false,
                margin_bytes: -128,
                diagnostic: "expert bank over by 128 bytes",
                schedule_cost: None,
                runtime_revalidation: None,
            }
        }

        fn with_revalidation(mut self, report: ExportRuntimeChromeRevalidationReport) -> Self {
            self.runtime_revalidation = Some(report);
            self
        }
    }

    impl ShadowPipelineSteps for FixtureSteps {
        fn static_budget(
            &self,
            artifact: &gbf_artifact::ModelArtifact,
            request: &CompileRequestRef,
        ) -> Result<ShadowStaticBudgetReport, ShadowPipelineError> {
            artifact
                .validate()
                .map_err(|error| ShadowPipelineError::StaticBudget(error.to_string()))?;
            ShadowStaticBudgetReport::new(
                request.clone(),
                self.budget_fits,
                self.margin_bytes,
                self.diagnostic,
            )
        }

        fn runtime_chrome_revalidation(
            &self,
            artifact: &gbf_artifact::ModelArtifact,
            _request: &CompileRequestRef,
        ) -> Result<Option<ExportRuntimeChromeRevalidationReport>, ShadowPipelineError> {
            artifact.validate().map_err(|error| {
                ShadowPipelineError::RuntimeChromeRevalidation(error.to_string())
            })?;
            Ok(self.runtime_revalidation.clone())
        }

        fn conformance(
            &self,
            artifact: &gbf_artifact::ModelArtifact,
            workload: &WorkloadId,
        ) -> Result<ShadowConformanceReport, ShadowPipelineError> {
            artifact
                .validate()
                .map_err(|error| ShadowPipelineError::Conformance(error.to_string()))?;
            ShadowConformanceReport::new(
                workload.clone(),
                true,
                0.015625,
                "artifact oracle agreement within tolerance",
            )
        }

        fn schedule_cost(
            &self,
            artifact: &gbf_artifact::ModelArtifact,
            _request: &CompileRequestRef,
        ) -> Result<Option<EstimatedCostDelta>, ShadowPipelineError> {
            artifact
                .validate()
                .map_err(|error| ShadowPipelineError::ScheduleCost(error.to_string()))?;
            Ok(self.schedule_cost.clone())
        }
    }

    fn warning_revalidation_report() -> ExportRuntimeChromeRevalidationReport {
        let training = runtime_budget_fixture(RuntimeNucleusHash::real(hash(1)), 16_384);
        let mut current = runtime_budget_fixture(RuntimeNucleusHash::real(hash(2)), 16_256);
        current
            .reference_shell_modules
            .insert(RuntimeShellModule::Panic);
        revalidate_runtime_chrome_budget_for_export(&training, &current, pass_demand())
    }

    fn blocking_revalidation_report() -> ExportRuntimeChromeRevalidationReport {
        let training = runtime_budget_fixture(RuntimeNucleusHash::real(hash(1)), 16_384);
        let current = runtime_budget_fixture(RuntimeNucleusHash::real(hash(2)), 12_384);
        revalidate_runtime_chrome_budget_for_export(&training, &current, blocking_demand())
    }

    fn pass_demand() -> RuntimeChromePreflightDemand {
        RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::new(8_000),
            ByteCost::new(2_048),
            ByteCost::new(2_048),
            ByteCost::new(128),
            ByteCost::new(512),
        )
    }

    fn blocking_demand() -> RuntimeChromePreflightDemand {
        RuntimeChromePreflightDemand::new(
            ModelSizeProfile::upper_bank_candidate(128, 4).unwrap(),
            ByteCost::new(20_000),
            ByteCost::new(9_000),
            ByteCost::new(4_000),
            ByteCost::new(512),
            ByteCost::new(1_024),
        )
    }

    fn runtime_budget_fixture(
        runtime_nucleus_hash: RuntimeNucleusHash,
        expert_slot_usable_bytes: u32,
    ) -> RuntimeChromeBudget {
        let mut rom_slots = vec![
            RomBudgetSlot {
                id: BudgetSlotId::new(0),
                class: BudgetSlotClass::Bank0Free,
                usable_bytes: 8 * 1024,
                reserved_slack: 512,
                placement_caps: std::collections::BTreeSet::from([
                    PlacementProfile::StrictOnePerBank,
                ]),
            },
            RomBudgetSlot {
                id: BudgetSlotId::new(1),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 512,
                placement_caps: std::collections::BTreeSet::from([PlacementProfile::Budgeted]),
            },
        ];
        for slot_id in 2..6 {
            rom_slots.push(RomBudgetSlot {
                id: BudgetSlotId::new(slot_id),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: expert_slot_usable_bytes,
                reserved_slack: 384,
                placement_caps: std::collections::BTreeSet::from([
                    PlacementProfile::StrictOnePerBank,
                    PlacementProfile::Budgeted,
                ]),
            });
        }

        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash,
            reference_shell_modules: RuntimeChromeBudget::pinned_reference_shell_modules(),
            rom_slots,
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(9),
            },
            wram_reserved: WramReserved::new(512, 4_096, 1_536).expect("valid WRAM reservation"),
            sram_reserved: 1_024,
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn unique_shadow_output_root(case: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("gbf-train-shadow-{case}-{unique}"))
    }

    #[derive(Debug, Clone, PartialEq)]
    struct ToyShadowStudent {
        weights: Vec<f32>,
        requires_grad: bool,
    }

    impl ToyShadowStudent {
        fn new(weights: Vec<f32>, requires_grad: bool) -> Self {
            Self {
                weights,
                requires_grad,
            }
        }
    }

    impl crate::ema::EmaUpdate for ToyShadowStudent {
        fn ema_update_from(
            &mut self,
            current: &Self,
            decay: EmaDecay,
        ) -> Result<(), crate::ema::EmaExportError> {
            update_ema_slice(&mut self.weights, &current.weights, decay)
        }
    }

    impl HardTernaryStudentModel for ToyShadowStudent {
        fn detach_for_student(&mut self) {
            self.requires_grad = false;
        }

        fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
            StudentWeightFingerprint::new(weight_bytes(&self.weights)).unwrap()
        }

        fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
            let mut bytes = Vec::from("toy-shadow-student:f32:");
            bytes.extend_from_slice(&self.weights.len().to_le_bytes());
            bytes.extend_from_slice(&weight_bytes(&self.weights));
            StudentStorageFingerprint::new(bytes).unwrap()
        }

        fn student_storage_identity(&self) -> usize {
            self.weights.as_ptr() as usize
        }

        fn student_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    impl ArtifactExportModel for ToyShadowStudent {
        fn artifact_seed(&self) -> u64 {
            20
        }

        fn artifact_semantic_core_hash(
            &self,
            frozen: &crate::student::FrozenStudent<Self>,
        ) -> gbf_foundation::Hash256 {
            sha256(frozen.weight_fingerprint().bytes())
        }

        fn artifact_quant_spec(&self) -> QuantSpec_S3 {
            QuantSpec_S3::new(BTreeMap::from([(weight_tensor_id(), WeightQuant::Fp32)]))
        }

        fn artifact_tensors(&self) -> Result<Vec<CanonicalTensor>, ExportVisitorError> {
            let payload = weight_bytes(&self.weights);
            Ok(vec![
                CanonicalTensor::new(
                    weight_tensor_id(),
                    Dtype::Fp32,
                    vec![
                        u32::try_from(self.weights.len())
                            .map_err(|_| ExportVisitorError::model("too many weights"))?,
                    ],
                    canonical_payload_sha(&payload),
                    PayloadRole::DeployableWeight,
                )
                .map_err(|error| ExportVisitorError::model(error.to_string()))?,
            ])
        }
    }

    fn weight_tensor_id() -> ArtifactPath {
        ArtifactPath::new("shadow.ema.weight").unwrap()
    }

    fn weight_bytes(weights: &[f32]) -> Vec<u8> {
        weights
            .iter()
            .flat_map(|weight| weight.to_le_bytes())
            .collect()
    }
}
