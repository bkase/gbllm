//! Stage 11 `ScheduleCostAnalysis` v1 surface.

use std::collections::BTreeSet;
use std::{error::Error, fmt};

use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, EvidenceRef, Hash256, KernelSpecId, SemVer,
    canonical_json_bytes_omitting_fields,
};
use gbf_policy::{
    CalibrationBundleSet, CalibrationConfidenceClass, CalibrationConfidenceRequirement,
    CalibrationSessionProfile, CompileObjective, CostBucketTotals, CostEstimate,
    DiagnosticSeverity, EstimatedCostDelta, EvidenceClass, FallbackReason, ModeCostBreakdown,
    ModeCostBreakdownEntry, ModeEstimatedCost, ObjectiveAxis, ObjectiveSatisfaction,
    ObjectiveSatisfactionMatrix, Quantile, ResolvedCompilePolicy, RuntimeChromeBudget, RuntimeMode,
    SatisfactionEntry, ScheduleCostBreakdown, ScheduleCostDiagnosticCode,
    ScheduleCostDiagnosticProvenance, ScheduleCostIdentity, ScheduleCostReport, SliceCostBreakdown,
    StaleCalibrationField, UncertaintyEnvelope, ValidationCode, ValidationDetail,
    ValidationDiagnostic, ValidationOrigin,
};
use gbf_report::{
    CanonicalJsonError as ReportCanonicalJsonError, ReportBody, ReportEnvelope,
    ReportEnvelopeError, ReportOutcome, ReportSelfHashError, canonicalize, round_trip_self_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schedule::{SchedOp, SchedulePack, YieldKind};
use crate::stage_cache::{
    CodegenStageCacheError, StoreBackedStageCacheKeys, StoreBackedStageCellKind,
    StoreBackedStageExpectedHashes, StoreBackedStageRunOutput, StoreBackedStageRunResult,
    run_store_backed_stage_with_cache, stage11_schedule_cost_store_key,
};
use gbf_store::stage_cache::StageCache as StoreStageCache;

pub const SCHEDULE_COST_SCHEMA_ID: &str = "schedule_cost.v1";
pub const SCHEDULE_COST_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const SCHEDULE_COST_PASS_VERSION: &str = "stage11/v1";
pub const SCHEDULE_COST_HEURISTIC_POLICY_VERSION: SemVer = SemVer::new(1, 0, 0);
pub const BANK_SWITCH_CYCLES: u64 = 64;
pub const SRAM_PAGE_SWITCH_CYCLES: u64 = 80;
pub const OVERLAY_INSTALL_CYCLES: u64 = 256;
pub const STATIC_SLICE_CYCLES: u64 = 32;
pub const STATIC_OP_CYCLES: u64 = 8;
pub const DEFAULT_FRAME_BUDGET_CYCLES: u64 = 17_556;

pub type ScheduleCostReportEnvelope = ReportEnvelope<ScheduleCostReportBody>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostKernelRegistry {
    pub specs: BTreeSet<KernelSpecId>,
}

impl ScheduleCostKernelRegistry {
    #[must_use]
    pub fn from_schedule_pack(schedule_pack: &SchedulePack) -> Self {
        let specs = schedule_pack
            .modes
            .iter()
            .flat_map(|mode| &mode.ir.slices)
            .flat_map(|slice| &slice.ops)
            .filter_map(|op| match op {
                SchedOp::KernelCall { spec, .. } => Some(spec.clone()),
                _ => None,
            })
            .collect();
        Self { specs }
    }

    #[must_use]
    pub fn contains(&self, spec: &KernelSpecId) -> bool {
        self.specs.contains(spec)
    }

    pub fn registry_hash(&self) -> Result<Hash256, CanonicalJsonError> {
        DomainHash::new(
            "gbf-codegen",
            "ScheduleCostKernelRegistry",
            SCHEDULE_COST_SCHEMA_ID,
            "1.0.0",
        )
        .hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCostInputs {
    pub schedule_pack: SchedulePack,
    pub policy: ResolvedCompilePolicy,
    pub calibration_bundle_set: CalibrationBundleSet,
    pub runtime_chrome_budget: RuntimeChromeBudget,
    pub target_profile_hash: Hash256,
    pub kernel_spec_registry: ScheduleCostKernelRegistry,
    pub kernel_spec_registry_hash: Hash256,
    pub active_session_profile: CalibrationSessionProfile,
    pub crate_feature_set_hash: Hash256,
    pub schedule_cost_schema_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCostOutput {
    pub outcome: ScheduleCostOutcome,
    pub report: Option<ScheduleCostReport>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleCostOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostReportBody {
    pub pass_version: String,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub report: Option<ScheduleCostReport>,
}

impl ReportBody for ScheduleCostReportBody {
    const REPORT_TYPE: &'static str = "ScheduleCostReport";
    const SCHEMA_ID: &'static str = SCHEDULE_COST_SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        validate_schedule_cost_report_body(self.report.is_some(), &self.diagnostics, outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostPolicyProjection {
    pub compile_objective: CompileObjective,
    pub risk_policy_calibration_confidence_requirement: CalibrationConfidenceRequirement,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}

impl ScheduleCostPolicyProjection {
    #[must_use]
    pub fn from_policy(policy: &ResolvedCompilePolicy) -> Self {
        Self {
            compile_objective: policy.objective.clone(),
            risk_policy_calibration_confidence_requirement: policy
                .objective
                .risk
                .calibration_confidence_requirement,
            requested_runtime_modes: policy.requested_runtime_modes.clone(),
        }
    }

    pub fn hash(&self) -> Result<Hash256, CanonicalJsonError> {
        DomainHash::new(
            "gbf-codegen",
            "ScheduleCostPolicyProjection",
            SCHEDULE_COST_SCHEMA_ID,
            "1.0.0",
        )
        .hash(self)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScheduleCostCacheKey(pub Hash256);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostCacheKeyInputs {
    pub schedule_pack_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub calibration_bundle_set_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub kernel_spec_registry_hash: Hash256,
    pub active_session_profile_hash: Hash256,
    pub schedule_cost_policy_projection_hash: Hash256,
    pub pass_version: &'static str,
    pub crate_feature_set_hash: Hash256,
    pub schedule_cost_schema_hash: Hash256,
}

impl ScheduleCostCacheKeyInputs {
    #[must_use]
    pub fn from_identity(identity: &ScheduleCostIdentity) -> Self {
        Self {
            schedule_pack_self_hash: identity.schedule_pack_self_hash,
            policy_resolution_self_hash: identity.policy_resolution_self_hash,
            calibration_bundle_set_hash: identity.calibration_bundle_set_hash,
            runtime_chrome_budget_hash: identity.runtime_chrome_budget_hash,
            target_profile_hash: identity.target_profile_hash,
            kernel_spec_registry_hash: identity.kernel_spec_registry_hash,
            active_session_profile_hash: identity.active_session_profile_hash,
            schedule_cost_policy_projection_hash: identity.schedule_cost_policy_projection_hash,
            pass_version: SCHEDULE_COST_PASS_VERSION,
            crate_feature_set_hash: identity.crate_feature_set_hash,
            schedule_cost_schema_hash: identity.schedule_cost_schema_hash,
        }
    }

    pub fn from_inputs(inputs: &ScheduleCostInputs) -> Result<Self, CanonicalJsonError> {
        let policy_projection = ScheduleCostPolicyProjection::from_policy(&inputs.policy);
        let kernel_spec_registry_hash = inputs.kernel_spec_registry.registry_hash()?;
        Ok(Self {
            schedule_pack_self_hash: inputs.schedule_pack.schedule_pack_self_hash,
            policy_resolution_self_hash: inputs.schedule_pack.identity.policy_resolution_self_hash,
            calibration_bundle_set_hash: DomainHash::new(
                "gbf-codegen",
                "CalibrationBundleSet",
                SCHEDULE_COST_SCHEMA_ID,
                "1.0.0",
            )
            .hash(&inputs.calibration_bundle_set)?,
            runtime_chrome_budget_hash: DomainHash::new(
                "gbf-codegen",
                "RuntimeChromeBudget",
                "runtime_chrome_budget.v1",
                "1.0.0",
            )
            .hash(&inputs.runtime_chrome_budget)?,
            target_profile_hash: inputs.target_profile_hash,
            kernel_spec_registry_hash,
            active_session_profile_hash: active_session_profile_hash(
                &inputs.active_session_profile,
            )?,
            schedule_cost_policy_projection_hash: policy_projection.hash()?,
            pass_version: SCHEDULE_COST_PASS_VERSION,
            crate_feature_set_hash: inputs.crate_feature_set_hash,
            schedule_cost_schema_hash: inputs.schedule_cost_schema_hash,
        })
    }

    pub fn cache_key(&self) -> Result<ScheduleCostCacheKey, CanonicalJsonError> {
        schedule_cost_cache_key(self)
    }
}

pub fn analyze_schedule_cost(inputs: &ScheduleCostInputs) -> ScheduleCostOutput {
    let mut diagnostics = Vec::new();
    let requested_modes = if inputs.policy.requested_runtime_modes.is_empty() {
        inputs
            .schedule_pack
            .identity
            .requested_runtime_modes
            .clone()
    } else {
        inputs.policy.requested_runtime_modes.clone()
    };
    let emitted_modes: BTreeSet<_> = inputs
        .schedule_pack
        .modes
        .iter()
        .map(|mode| mode.mode)
        .collect();
    for mode in requested_modes.difference(&emitted_modes) {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostPerModeMissing,
            ScheduleCostDiagnosticProvenance::Mode { mode: *mode },
        ));
    }
    for mode in emitted_modes.difference(&requested_modes) {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostPerModeUnexpected,
            ScheduleCostDiagnosticProvenance::Mode { mode: *mode },
        ));
    }
    if inputs.runtime_chrome_budget.target != inputs.policy.target {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostScheduleCostInputHashMismatch,
            ScheduleCostDiagnosticProvenance::HashMismatch {
                product: "runtime_chrome_budget.target".to_owned(),
                recorded: hash_debug_string(inputs.policy.target.as_str()),
                computed: hash_debug_string(inputs.runtime_chrome_budget.target.as_str()),
            },
        ));
    }
    let kernel_spec_registry_hash = match inputs.kernel_spec_registry.registry_hash() {
        Ok(hash) => hash,
        Err(error) => {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostScheduleCostInputHashMismatch,
                ScheduleCostDiagnosticProvenance::JsonPath {
                    json_path: "kernel_spec_registry".to_owned(),
                    field_or_tag: error.to_string(),
                },
            ));
            Hash256::ZERO
        }
    };
    if kernel_spec_registry_hash != inputs.kernel_spec_registry_hash {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostScheduleCostInputHashMismatch,
            ScheduleCostDiagnosticProvenance::HashMismatch {
                product: "kernel_spec_registry_hash".to_owned(),
                recorded: inputs.kernel_spec_registry_hash,
                computed: kernel_spec_registry_hash,
            },
        ));
    }
    diagnostics.extend(kernel_registry_diagnostics(inputs));
    let active_session_profile_hash =
        match active_session_profile_hash(&inputs.active_session_profile) {
            Ok(hash) => hash,
            Err(error) => {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostScheduleCostInputHashMismatch,
                    ScheduleCostDiagnosticProvenance::JsonPath {
                        json_path: "active_session_profile".to_owned(),
                        field_or_tag: error.to_string(),
                    },
                ));
                Hash256::ZERO
            }
        };

    let calibration = resolve_evidence(inputs, &mut diagnostics);
    let policy_projection = ScheduleCostPolicyProjection::from_policy(&inputs.policy);
    let policy_projection_hash = match policy_projection.hash() {
        Ok(hash) => hash,
        Err(error) => {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostScheduleCostInputHashMismatch,
                ScheduleCostDiagnosticProvenance::JsonPath {
                    json_path: "schedule_cost_policy_projection_hash".to_owned(),
                    field_or_tag: error.to_string(),
                },
            ));
            Hash256::ZERO
        }
    };

    let calibration_bundle_set_hash = hash_or_diagnostic(
        "CalibrationBundleSet",
        SCHEDULE_COST_SCHEMA_ID,
        &inputs.calibration_bundle_set,
        ScheduleCostDiagnosticCode::CostCalibrationBundleHashMismatch,
        "calibration_bundle_set_hash",
        &mut diagnostics,
    );
    let runtime_chrome_budget_hash = hash_or_diagnostic(
        "RuntimeChromeBudget",
        "runtime_chrome_budget.v1",
        &inputs.runtime_chrome_budget,
        ScheduleCostDiagnosticCode::CostScheduleCostInputHashMismatch,
        "runtime_chrome_budget_hash",
        &mut diagnostics,
    );
    let mut per_mode = Vec::new();
    let mut breakdown = Vec::new();
    let frame_budget_cycles = frame_budget_cycles(&inputs.schedule_pack);

    for mode_schedule in &inputs.schedule_pack.modes {
        if !requested_modes.contains(&mode_schedule.mode) {
            continue;
        }
        let mode_breakdown = analyze_mode_breakdown(&mode_schedule.ir.slices);
        let time_to_first_token_cycles = time_to_first_token_cycles(&mode_schedule.ir.slices);
        let frame_jitter_cycles = frame_jitter_cycles(&mode_breakdown);
        let estimate = estimate_mode_cost(
            &mode_breakdown,
            frame_budget_cycles,
            time_to_first_token_cycles,
            frame_jitter_cycles,
            inputs
                .policy
                .objective
                .max_sram_page_switches_per_token
                .is_some(),
            inputs
                .policy
                .objective
                .service
                .as_ref()
                .and_then(|service| service.max_ui_jitter_frames_p99),
            &calibration,
        );
        per_mode.push(ModeEstimatedCost {
            mode: mode_schedule.mode,
            delta: estimate,
        });
        breakdown.push(ModeCostBreakdownEntry {
            mode: mode_schedule.mode,
            breakdown: mode_breakdown,
        });
    }
    per_mode.sort_by_key(|entry| entry.mode);
    breakdown.sort_by_key(|entry| entry.mode);

    let mut report = ScheduleCostReport {
        objective: inputs.policy.objective.clone(),
        per_mode,
        satisfaction: ObjectiveSatisfactionMatrix {
            entries: Vec::new(),
        },
        refs: Vec::new(),
        identity: ScheduleCostIdentity {
            schedule_pack_self_hash: inputs.schedule_pack.schedule_pack_self_hash,
            policy_resolution_self_hash: inputs.schedule_pack.identity.policy_resolution_self_hash,
            calibration_bundle_set_hash,
            runtime_chrome_budget_hash,
            target_profile_hash: inputs.target_profile_hash,
            kernel_spec_registry_hash,
            active_session_profile_hash,
            schedule_cost_policy_projection_hash: policy_projection_hash,
            pass_version: SCHEDULE_COST_SCHEMA_VERSION,
            crate_feature_set_hash: inputs.crate_feature_set_hash,
            schedule_cost_schema_hash: inputs.schedule_cost_schema_hash,
            schedule_cost_report_self_hash: Hash256::ZERO,
        },
        breakdown: ScheduleCostBreakdown {
            per_mode: breakdown,
        },
    };
    report.refs = report_refs_union(&report.per_mode);
    report.satisfaction = build_satisfaction_matrix(&report.objective, &report.per_mode);
    diagnostics.extend(validate_schedule_cost_report(&report));
    diagnostics.extend(objective_violation_diagnostics(&report));
    match schedule_cost_report_self_hash(&report) {
        Ok(hash) => report.identity.schedule_cost_report_self_hash = hash,
        Err(error) => diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostScheduleCostReportRoundTripFailed,
            ScheduleCostDiagnosticProvenance::JsonPath {
                json_path: "schedule_cost_report_self_hash".to_owned(),
                field_or_tag: error.to_string(),
            },
        )),
    }

    let outcome = if diagnostics.is_empty() {
        ScheduleCostOutcome::Succeeded
    } else {
        ScheduleCostOutcome::Failed
    };
    ScheduleCostOutput {
        outcome,
        report: Some(report),
        diagnostics,
    }
}

pub fn emit_schedule_cost_report(
    output: &ScheduleCostOutput,
) -> Result<ScheduleCostReportEnvelope, ScheduleCostEmitError> {
    let outcome = match output.outcome {
        ScheduleCostOutcome::Succeeded => ReportOutcome::Passed,
        ScheduleCostOutcome::Failed => ReportOutcome::Failed,
    };
    let body = ScheduleCostReportBody {
        pass_version: SCHEDULE_COST_PASS_VERSION.to_owned(),
        diagnostics: output.diagnostics.clone(),
        report: output.report.clone(),
    };
    let envelope = ReportEnvelope::new(outcome, body)?.with_computed_self_hash()?;
    round_trip_self_hash(&envelope)?;
    Ok(envelope)
}

pub fn emit_schedule_cost_json_bytes(
    output: &ScheduleCostOutput,
) -> Result<Vec<u8>, ScheduleCostEmitError> {
    Ok(canonicalize(&emit_schedule_cost_report(output)?)?)
}

pub fn schedule_cost_report_self_hash(
    report: &ScheduleCostReport,
) -> Result<Hash256, CanonicalJsonError> {
    let mut hash_input = report.clone();
    hash_input.identity.schedule_cost_report_self_hash = Hash256::ZERO;
    let canonical = CanonicalJson::to_vec(&hash_input)?;
    DomainHash::new(
        "gbf-codegen",
        "ScheduleCostReport",
        SCHEDULE_COST_SCHEMA_ID,
        "1.0.0",
    )
    .hash_canonical_bytes(&canonical)
}

pub fn schedule_cost_cache_key(
    inputs: &ScheduleCostCacheKeyInputs,
) -> Result<ScheduleCostCacheKey, CanonicalJsonError> {
    let canonical = canonical_json_bytes_omitting_fields(inputs, &["policy_resolution_self_hash"])?;
    DomainHash::new(
        "gbf-codegen",
        "StageCacheKey",
        SCHEDULE_COST_SCHEMA_ID,
        "1.0.0",
    )
    .hash_canonical_bytes(&canonical)
    .map(ScheduleCostCacheKey)
}

pub fn run_schedule_cost_with_cache(
    cache: &StoreStageCache<'_>,
    inputs: &ScheduleCostInputs,
    expected_hashes: StoreBackedStageExpectedHashes,
) -> Result<StoreBackedStageRunOutput<ScheduleCostReport>, CodegenStageCacheError> {
    let cache_key = ScheduleCostCacheKeyInputs::from_inputs(inputs)
        .and_then(|key_inputs| key_inputs.cache_key())
        .map_err(|error| CodegenStageCacheError::StageCacheKey {
            stage_id: "11",
            message: error.to_string(),
        })?;
    let keys = StoreBackedStageCacheKeys::new(
        "11",
        stage11_schedule_cost_store_key(cache_key, StoreBackedStageCellKind::Success),
        stage11_schedule_cost_store_key(cache_key, StoreBackedStageCellKind::FailureMemo),
    );
    run_store_backed_stage_with_cache(cache, &keys, cache_key.0, expected_hashes, || {
        let output = analyze_schedule_cost(inputs);
        let report_envelope = emit_schedule_cost_report(&output).map_err(|error| {
            CodegenStageCacheError::StageEmit {
                stage_id: "11",
                message: error.to_string(),
            }
        })?;
        let report_self_hash = report_envelope.report_self_hash;
        let report = output
            .report
            .ok_or_else(|| CodegenStageCacheError::StageOutputInvariant {
                stage_id: "11",
                message: "schedule cost output is missing ScheduleCostReport".to_owned(),
            })?;
        match output.outcome {
            ScheduleCostOutcome::Succeeded => Ok(StoreBackedStageRunResult::Success {
                product_self_hash: report.identity.schedule_cost_report_self_hash,
                product: report,
                report_self_hash,
            }),
            ScheduleCostOutcome::Failed => Ok(StoreBackedStageRunResult::FailureMemo {
                diagnostics: output.diagnostics,
                report_self_hash,
            }),
        }
    })
}

#[derive(Debug)]
pub enum ScheduleCostEmitError {
    Envelope(ReportEnvelopeError),
    SelfHash(ReportSelfHashError),
    Canonical(ReportCanonicalJsonError),
}

impl fmt::Display for ScheduleCostEmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(error) => write!(f, "schedule cost envelope failed: {error}"),
            Self::SelfHash(error) => write!(f, "schedule cost self hash failed: {error}"),
            Self::Canonical(error) => write!(f, "schedule cost canonicalization failed: {error}"),
        }
    }
}

impl Error for ScheduleCostEmitError {}

impl From<ReportEnvelopeError> for ScheduleCostEmitError {
    fn from(error: ReportEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ReportSelfHashError> for ScheduleCostEmitError {
    fn from(error: ReportSelfHashError) -> Self {
        Self::SelfHash(error)
    }
}

impl From<ReportCanonicalJsonError> for ScheduleCostEmitError {
    fn from(error: ReportCanonicalJsonError) -> Self {
        Self::Canonical(error)
    }
}

#[derive(Debug, Clone)]
struct EvidenceResolution {
    class: EvidenceClass,
    fallback_reason: Option<FallbackReason>,
    base_refs: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduleCostEvidenceAxis {
    CyclesPerToken,
    BankSwitchesPerToken,
    SramPageSwitchesPerToken,
    YieldsPerToken,
    SchedulerHeadroomUtilization,
    MaxNoProgressEstimate,
    TimeToFirstToken,
    SustainedThroughputTokensPerMegacycle,
    FrameJitter,
}

impl ScheduleCostEvidenceAxis {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CyclesPerToken => "cycles_per_token",
            Self::BankSwitchesPerToken => "bank_switches_per_token",
            Self::SramPageSwitchesPerToken => "sram_page_switches_per_token",
            Self::YieldsPerToken => "yields_per_token",
            Self::SchedulerHeadroomUtilization => "scheduler_headroom_utilization",
            Self::MaxNoProgressEstimate => "max_no_progress_estimate",
            Self::TimeToFirstToken => "time_to_first_token",
            Self::SustainedThroughputTokensPerMegacycle => {
                "sustained_throughput_tokens_per_megacycle"
            }
            Self::FrameJitter => "frame_jitter",
        }
    }

    const fn heuristic_policy_id(self) -> &'static str {
        match self {
            Self::CyclesPerToken => "CyclesPerOpDefault",
            Self::BankSwitchesPerToken => "BankSwitchesUpperBound",
            Self::SramPageSwitchesPerToken => "SramPageSwitchesUpperBound",
            Self::YieldsPerToken => "YieldsPerTokenStatic",
            Self::SchedulerHeadroomUtilization => "HeadroomUtilizationDefault",
            Self::MaxNoProgressEstimate => "MaxNoProgressEstimateDefault",
            Self::TimeToFirstToken => "TimeToFirstTokenDefault",
            Self::SustainedThroughputTokensPerMegacycle => "SustainedThroughputDefault",
            Self::FrameJitter => "FrameJitterDefault",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fb14EvidenceRef {
    CalibrationBundle {
        layer: &'static str,
        hash: Hash256,
    },
    HeuristicPolicy {
        policy_id: &'static str,
    },
    TransferPolicy {
        policy_id: &'static str,
        from_target: String,
        to_target: String,
        hash: Hash256,
    },
    AxisComposition {
        axis: ScheduleCostEvidenceAxis,
        hash: Hash256,
    },
}

impl Fb14EvidenceRef {
    fn into_ref(self) -> EvidenceRef {
        match self {
            Self::CalibrationBundle { layer, hash } => EvidenceRef {
                kind: "F-B14CalibrationBundle".to_owned(),
                reference: format!("calibration://{layer}"),
                hash: Some(hash),
            },
            Self::HeuristicPolicy { policy_id } => EvidenceRef {
                kind: "F-B14HeuristicPolicy".to_owned(),
                reference: format!(
                    "heuristic://schedule_cost/{policy_id}/{}",
                    SCHEDULE_COST_HEURISTIC_POLICY_VERSION
                ),
                hash: None,
            },
            Self::TransferPolicy {
                policy_id,
                from_target,
                to_target,
                hash,
            } => EvidenceRef {
                kind: "F-B14TransferPolicy".to_owned(),
                reference: format!(
                    "transfer://schedule_cost/{policy_id}/{from_target}/{to_target}"
                ),
                hash: Some(hash),
            },
            Self::AxisComposition { axis, hash } => EvidenceRef {
                kind: "F-B14AxisEvidence".to_owned(),
                reference: format!("schedule-cost://axis/{}", axis.as_str()),
                hash: Some(hash),
            },
        }
    }
}

fn resolve_evidence(
    inputs: &ScheduleCostInputs,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> EvidenceResolution {
    let requirement = inputs
        .policy
        .objective
        .risk
        .calibration_confidence_requirement;
    let best_bundle = inputs
        .calibration_bundle_set
        .bundles
        .values()
        .max_by_key(|bundle| bundle.confidence.rank());
    let Some(bundle) = best_bundle else {
        if !matches!(
            requirement,
            CalibrationConfidenceRequirement::NoMinimumConfidence
        ) {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostCalibrationMissingForRequirement,
                ScheduleCostDiagnosticProvenance::Estimate {
                    field: "calibration_bundle_set".to_owned(),
                    invariant: "strict calibration requirement has no bundle".to_owned(),
                },
            ));
        }
        return EvidenceResolution {
            class: EvidenceClass::Heuristic,
            fallback_reason: Some(FallbackReason::NoBundleForTarget),
            base_refs: Vec::new(),
        };
    };
    if bundle.confidence != CalibrationConfidenceClass::None
        && let Some((field, declared, observed)) = stale_calibration_field(inputs, bundle)
    {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostCalibrationBundleStale,
            ScheduleCostDiagnosticProvenance::HashMismatch {
                product: format!(
                    "calibration_bundle.{}.{}",
                    bundle.layer.as_str(),
                    field_name(field)
                ),
                recorded: declared,
                computed: observed,
            },
        ));
        return EvidenceResolution {
            class: EvidenceClass::Heuristic,
            fallback_reason: Some(FallbackReason::BundleStale {
                field,
                declared,
                observed,
            }),
            base_refs: Vec::new(),
        };
    }
    if !requirement.accepts(bundle.confidence) {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostCalibrationMissingForRequirement,
            ScheduleCostDiagnosticProvenance::Calibration {
                layer: bundle.layer,
                declared_confidence: bundle.confidence,
                required_confidence: format!("{requirement:?}"),
            },
        ));
        return EvidenceResolution {
            class: EvidenceClass::Heuristic,
            fallback_reason: Some(FallbackReason::ConfidenceBelowRequirement {
                declared: format!("{:?}", bundle.confidence),
                required: format!("{requirement:?}"),
            }),
            base_refs: Vec::new(),
        };
    }
    let class = match bundle.confidence {
        CalibrationConfidenceClass::Transferred => EvidenceClass::Transferred,
        CalibrationConfidenceClass::Weak
        | CalibrationConfidenceClass::Reasonable
        | CalibrationConfidenceClass::Strong => EvidenceClass::Calibrated,
        CalibrationConfidenceClass::None => EvidenceClass::Heuristic,
    };
    if class == EvidenceClass::Heuristic {
        return EvidenceResolution {
            class,
            fallback_reason: Some(FallbackReason::KernelSpecNotCalibrated),
            base_refs: Vec::new(),
        };
    }
    let bundle_hash = hash_or_diagnostic(
        "CalibrationBundle",
        SCHEDULE_COST_SCHEMA_ID,
        bundle,
        ScheduleCostDiagnosticCode::CostCalibrationBundleHashMismatch,
        "calibration_bundle_hash",
        diagnostics,
    );
    let mut refs = vec![
        Fb14EvidenceRef::CalibrationBundle {
            layer: bundle.layer.as_str(),
            hash: bundle_hash,
        }
        .into_ref(),
    ];
    if class == EvidenceClass::Transferred {
        refs.push(
            Fb14EvidenceRef::TransferPolicy {
                policy_id: "Identity",
                from_target: inputs.policy.target.as_str().to_owned(),
                to_target: inputs.policy.target.as_str().to_owned(),
                hash: identity_transfer_policy_hash(inputs),
            }
            .into_ref(),
        );
    }
    EvidenceResolution {
        class,
        fallback_reason: None,
        base_refs: refs,
    }
}

fn stale_calibration_field(
    inputs: &ScheduleCostInputs,
    bundle: &gbf_policy::CalibrationBundle,
) -> Option<(StaleCalibrationField, Hash256, Hash256)> {
    if bundle.target_profile_hash != inputs.target_profile_hash {
        return Some((
            StaleCalibrationField::TargetProfileHash,
            bundle.target_profile_hash,
            inputs.target_profile_hash,
        ));
    }
    let kernel_spec_registry_hash = inputs
        .kernel_spec_registry
        .registry_hash()
        .expect("kernel registry is canonical-serializable");
    if bundle.kernel_set_hash != kernel_spec_registry_hash {
        return Some((
            StaleCalibrationField::KernelSetHash,
            bundle.kernel_set_hash,
            kernel_spec_registry_hash,
        ));
    }
    if bundle.packer_version != gbf_runtime::RUNTIME_PACKER_VERSION {
        return Some((
            StaleCalibrationField::PackerVersion,
            packer_version_freshness_hash(&bundle.packer_version),
            packer_version_freshness_hash(&gbf_runtime::RUNTIME_PACKER_VERSION),
        ));
    }
    if bundle.calibration_schema_hash != Hash256::ZERO {
        return Some((
            StaleCalibrationField::CalibrationSchemaHash,
            bundle.calibration_schema_hash,
            Hash256::ZERO,
        ));
    }
    if !bundle
        .validity_envelope
        .contains_session_profile(&inputs.active_session_profile)
    {
        return Some((
            StaleCalibrationField::ValidityEnvelope,
            validity_envelope_hash(&bundle.validity_envelope),
            active_session_profile_hash(&inputs.active_session_profile)
                .expect("active session profile is canonical-serializable"),
        ));
    }
    None
}

const fn field_name(field: StaleCalibrationField) -> &'static str {
    match field {
        StaleCalibrationField::TargetProfileHash => "target_profile_hash",
        StaleCalibrationField::KernelSetHash => "kernel_set_hash",
        StaleCalibrationField::PackerVersion => "packer_version",
        StaleCalibrationField::CalibrationSchemaHash => "calibration_schema_hash",
        StaleCalibrationField::ValidityEnvelope => "validity_envelope",
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct IdentityTransferPolicyHashInput<'a> {
    policy_id: &'static str,
    policy_version: SemVer,
    from_target: &'a str,
    to_target: &'a str,
}

fn identity_transfer_policy_hash(inputs: &ScheduleCostInputs) -> Hash256 {
    DomainHash::new(
        "gbf-codegen",
        "F-B14TransferPolicy",
        SCHEDULE_COST_SCHEMA_ID,
        "1.0.0",
    )
    .hash(&IdentityTransferPolicyHashInput {
        policy_id: "Identity",
        policy_version: SCHEDULE_COST_HEURISTIC_POLICY_VERSION,
        from_target: inputs.policy.target.as_str(),
        to_target: inputs.policy.target.as_str(),
    })
    .expect("identity transfer policy hash input is canonical-serializable")
}

fn packer_version_freshness_hash(version: &gbf_foundation::PackerVersion) -> Hash256 {
    DomainHash::new(
        "gbf-foundation",
        "PackerVersion",
        "calibration.packer_version",
        "1.0.0",
    )
    .hash(version)
    .expect("packer version is canonical-serializable")
}

fn validity_envelope_hash(envelope: &gbf_policy::ValidityEnvelope) -> Hash256 {
    DomainHash::new(
        "gbf-policy",
        "ValidityEnvelope",
        "calibration.validity_envelope",
        "1.0.0",
    )
    .hash(envelope)
    .expect("validity envelope is canonical-serializable")
}

fn active_session_profile_hash(
    profile: &CalibrationSessionProfile,
) -> Result<Hash256, CanonicalJsonError> {
    DomainHash::new(
        "gbf-policy",
        "CalibrationSessionProfile",
        "calibration.session_profile",
        "1.0.0",
    )
    .hash(profile)
}

fn analyze_mode_breakdown(slices: &[crate::schedule::SchedSlice]) -> ModeCostBreakdown {
    let mut totals = CostBucketTotals::default();
    let mut slice_breakdowns = Vec::with_capacity(slices.len());
    for slice in slices {
        let breakdown = slice_cycle_breakdown(slice);
        totals.bank_switch_cycles = totals
            .bank_switch_cycles
            .saturating_add(breakdown.bank_switch_cycles);
        totals.sram_page_switch_cycles = totals
            .sram_page_switch_cycles
            .saturating_add(breakdown.sram_page_switch_cycles);
        totals.overlay_install_cycles = totals
            .overlay_install_cycles
            .saturating_add(breakdown.overlay_install_cycles);
        totals.static_slice_cycles = totals
            .static_slice_cycles
            .saturating_add(breakdown.static_slice_cycles);
        totals.total_cycles = totals.total_cycles.saturating_add(breakdown.total_cycles);
        totals.bank_switches = totals
            .bank_switches
            .saturating_add(u64::from(slice.resources.bank_switches));
        totals.sram_page_switches = totals
            .sram_page_switches
            .saturating_add(u64::from(slice.resources.sram_page_switches));
        totals.overlay_installs = totals
            .overlay_installs
            .saturating_add(u64::from(slice.resources.overlay_installs));
        totals.yields = totals.yields.saturating_add(
            slice
                .ops
                .iter()
                .filter(|op| matches!(op, SchedOp::Yield { .. }))
                .count() as u64,
        );
        slice_breakdowns.push(breakdown);
    }
    ModeCostBreakdown {
        slices: slice_breakdowns,
        totals,
    }
}

fn slice_cycle_breakdown(slice: &crate::schedule::SchedSlice) -> SliceCostBreakdown {
    let bank_switch_cycles =
        u64::from(slice.resources.bank_switches).saturating_mul(BANK_SWITCH_CYCLES);
    let sram_page_switch_cycles =
        u64::from(slice.resources.sram_page_switches).saturating_mul(SRAM_PAGE_SWITCH_CYCLES);
    let overlay_install_cycles =
        u64::from(slice.resources.overlay_installs).saturating_mul(OVERLAY_INSTALL_CYCLES);
    let static_slice_cycles = STATIC_SLICE_CYCLES
        .saturating_add((slice.ops.len() as u64).saturating_mul(STATIC_OP_CYCLES));
    let total_cycles = bank_switch_cycles
        .saturating_add(sram_page_switch_cycles)
        .saturating_add(overlay_install_cycles)
        .saturating_add(static_slice_cycles);
    SliceCostBreakdown {
        slice_id: slice.id.0,
        bank_switch_cycles,
        sram_page_switch_cycles,
        overlay_install_cycles,
        static_slice_cycles,
        total_cycles,
    }
}

fn time_to_first_token_cycles(slices: &[crate::schedule::SchedSlice]) -> u64 {
    let mut cycles = 0_u64;
    for slice in slices {
        cycles = cycles.saturating_add(slice_cycle_breakdown(slice).total_cycles);
        if slice.yield_kind == YieldKind::TokenReady
            || slice.ops.iter().any(|op| {
                matches!(
                    op,
                    SchedOp::Yield {
                        kind: YieldKind::TokenReady
                    }
                )
            })
        {
            return cycles.max(1);
        }
    }
    cycles.max(1)
}

fn frame_jitter_cycles(breakdown: &ModeCostBreakdown) -> u64 {
    let Some(min) = breakdown
        .slices
        .iter()
        .map(|slice| slice.total_cycles)
        .min()
    else {
        return 0;
    };
    let max = breakdown
        .slices
        .iter()
        .map(|slice| slice.total_cycles)
        .max()
        .unwrap_or(min);
    max.saturating_sub(min)
}

fn estimate_mode_cost(
    breakdown: &ModeCostBreakdown,
    frame_budget_cycles: u64,
    time_to_first_token_cycles: u64,
    frame_jitter_cycles: u64,
    require_sram_page_switches: bool,
    frame_jitter_target_frames: Option<u8>,
    evidence: &EvidenceResolution,
) -> EstimatedCostDelta {
    let totals = &breakdown.totals;
    let cycles = totals.total_cycles.max(1);
    let headroom_q16 = q16_ratio(cycles, frame_budget_cycles.max(1));
    let no_progress_q16 = headroom_q16;
    let throughput_q16 = (((1_000_000_u128) << 16) / u128::from(cycles)) as i64;
    let estimate = |units: i64, axis| estimate_exact(units, evidence, axis);
    EstimatedCostDelta {
        cycles_per_token: estimate(cycles as i64, ScheduleCostEvidenceAxis::CyclesPerToken),
        bank_switches_per_token: estimate(
            totals.bank_switches as i64,
            ScheduleCostEvidenceAxis::BankSwitchesPerToken,
        ),
        sram_page_switches_per_token: (require_sram_page_switches || totals.sram_page_switches > 0)
            .then(|| {
                estimate(
                    totals.sram_page_switches as i64,
                    ScheduleCostEvidenceAxis::SramPageSwitchesPerToken,
                )
            }),
        yields_per_token: estimate(
            totals.yields as i64,
            ScheduleCostEvidenceAxis::YieldsPerToken,
        ),
        scheduler_headroom_utilization: estimate_from_envelope(
            UncertaintyEnvelope::from_q16(
                headroom_q16,
                headroom_q16,
                headroom_q16,
                Some(headroom_q16),
            ),
            evidence,
            ScheduleCostEvidenceAxis::SchedulerHeadroomUtilization,
        ),
        video_commit_cost_margin: None,
        max_no_progress_estimate: estimate_from_envelope(
            UncertaintyEnvelope::from_q16(
                no_progress_q16,
                no_progress_q16,
                no_progress_q16,
                Some(no_progress_q16),
            ),
            evidence,
            ScheduleCostEvidenceAxis::MaxNoProgressEstimate,
        ),
        time_to_first_token: estimate(
            time_to_first_token_cycles as i64,
            ScheduleCostEvidenceAxis::TimeToFirstToken,
        ),
        sustained_throughput_tokens_per_megacycle: estimate_from_envelope(
            UncertaintyEnvelope::from_q16(
                throughput_q16,
                throughput_q16,
                throughput_q16,
                Some(throughput_q16),
            ),
            evidence,
            ScheduleCostEvidenceAxis::SustainedThroughputTokensPerMegacycle,
        ),
        frame_jitter: frame_jitter_target_frames.map(|_| {
            estimate(
                frame_jitter_cycles as i64,
                ScheduleCostEvidenceAxis::FrameJitter,
            )
        }),
    }
}

fn build_satisfaction_matrix(
    objective: &CompileObjective,
    per_mode: &[ModeEstimatedCost],
) -> ObjectiveSatisfactionMatrix {
    let mut entries = Vec::new();
    for mode_estimate in per_mode {
        let mode = mode_estimate.mode;
        let estimate = &mode_estimate.delta;
        if let Some(target) = objective.max_cycles_per_token {
            insert_satisfaction_for_all_quantiles(
                &mut entries,
                mode,
                ObjectiveAxis::CyclesPerToken,
                &estimate.cycles_per_token,
                i64::from(target) * UncertaintyEnvelope::Q16_ONE,
            );
        }
        if let Some(target) = objective.max_bank_switches_per_token {
            insert_satisfaction_for_all_quantiles(
                &mut entries,
                mode,
                ObjectiveAxis::BankSwitchesPerToken,
                &estimate.bank_switches_per_token,
                i64::from(target) * UncertaintyEnvelope::Q16_ONE,
            );
        }
        if let (Some(target), Some(sram_estimate)) = (
            objective.max_sram_page_switches_per_token,
            estimate.sram_page_switches_per_token.as_ref(),
        ) {
            insert_satisfaction_for_all_quantiles(
                &mut entries,
                mode,
                ObjectiveAxis::SramPageSwitchesPerToken,
                sram_estimate,
                i64::from(target) * UncertaintyEnvelope::Q16_ONE,
            );
        }
        if let Some(target) = objective.min_sustained_throughput_tokens_per_megacycle {
            insert_min_satisfaction_for_all_quantiles(
                &mut entries,
                mode,
                ObjectiveAxis::SustainedThroughputTokensPerMegacycle,
                &estimate.sustained_throughput_tokens_per_megacycle,
                i64::from(target) * UncertaintyEnvelope::Q16_ONE,
            );
        }
        let max_utilization_pct = 100_u8.saturating_sub(objective.min_ui_headroom_pct);
        insert_satisfaction_for_all_quantiles(
            &mut entries,
            mode,
            ObjectiveAxis::SchedulerHeadroomUtilization,
            &estimate.scheduler_headroom_utilization,
            i64::from(max_utilization_pct) * UncertaintyEnvelope::Q16_ONE / 100,
        );
        if let Some(service) = &objective.service {
            if let Some(target) = service.max_first_token_cycles_p95 {
                insert_satisfaction_for_all_quantiles(
                    &mut entries,
                    mode,
                    ObjectiveAxis::TimeToFirstToken,
                    &estimate.time_to_first_token,
                    i64::from(target) * UncertaintyEnvelope::Q16_ONE,
                );
            }
            if let (Some(target_frames), Some(frame_jitter)) = (
                service.max_ui_jitter_frames_p99,
                estimate.frame_jitter.as_ref(),
            ) {
                insert_satisfaction_for_all_quantiles(
                    &mut entries,
                    mode,
                    ObjectiveAxis::FrameJitter,
                    frame_jitter,
                    i64::from(target_frames)
                        * DEFAULT_FRAME_BUDGET_CYCLES as i64
                        * UncertaintyEnvelope::Q16_ONE,
                );
            }
        }
    }
    entries.sort_by_key(|entry| entry.key);
    ObjectiveSatisfactionMatrix { entries }
}

fn insert_satisfaction_for_all_quantiles(
    entries: &mut Vec<SatisfactionEntry>,
    mode: RuntimeMode,
    axis: ObjectiveAxis,
    estimate: &CostEstimate,
    target_q16_16: i64,
) {
    for quantile in [Quantile::P50, Quantile::P95, Quantile::P99] {
        insert_satisfaction(entries, mode, axis, quantile, estimate, target_q16_16);
    }
}

fn insert_min_satisfaction_for_all_quantiles(
    entries: &mut Vec<SatisfactionEntry>,
    mode: RuntimeMode,
    axis: ObjectiveAxis,
    estimate: &CostEstimate,
    target_q16_16: i64,
) {
    for quantile in [Quantile::P50, Quantile::P95, Quantile::P99] {
        insert_min_satisfaction(entries, mode, axis, quantile, estimate, target_q16_16);
    }
}

fn insert_satisfaction(
    entries: &mut Vec<SatisfactionEntry>,
    mode: RuntimeMode,
    axis: ObjectiveAxis,
    quantile: Quantile,
    estimate: &CostEstimate,
    target_q16_16: i64,
) {
    let upper = estimate.envelope.upper_for(quantile);
    let lower = estimate.envelope.lower_for(quantile);
    let satisfaction = if upper <= target_q16_16 {
        ObjectiveSatisfaction::Satisfied
    } else if lower <= target_q16_16 {
        ObjectiveSatisfaction::Borderline
    } else {
        ObjectiveSatisfaction::Violated
    };
    entries.push(SatisfactionEntry {
        key: gbf_policy::SatisfactionKey {
            mode,
            axis,
            quantile,
        },
        satisfaction,
    });
}

fn insert_min_satisfaction(
    entries: &mut Vec<SatisfactionEntry>,
    mode: RuntimeMode,
    axis: ObjectiveAxis,
    quantile: Quantile,
    estimate: &CostEstimate,
    target_q16_16: i64,
) {
    let upper = estimate.envelope.upper_for(quantile);
    let lower = estimate.envelope.lower_for(quantile);
    let satisfaction = if lower >= target_q16_16 {
        ObjectiveSatisfaction::Satisfied
    } else if upper >= target_q16_16 {
        ObjectiveSatisfaction::Borderline
    } else {
        ObjectiveSatisfaction::Violated
    };
    entries.push(SatisfactionEntry {
        key: gbf_policy::SatisfactionKey {
            mode,
            axis,
            quantile,
        },
        satisfaction,
    });
}

fn objective_violation_diagnostics(report: &ScheduleCostReport) -> Vec<ValidationDiagnostic> {
    report
        .satisfaction
        .entries
        .iter()
        .filter_map(|entry| {
            (entry.satisfaction == ObjectiveSatisfaction::Violated).then(|| {
                let key = &entry.key;
                let estimate =
                    estimate_for_axis(mode_delta(&report.per_mode, key.mode)?, key.axis)?;
                Some(diagnostic(
                    ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixInconsistent,
                    ScheduleCostDiagnosticProvenance::Objective {
                        mode: key.mode,
                        axis: format!("{:?}", key.axis),
                        quantile: format!("{:?}", key.quantile),
                        target_q16_16: objective_target(report, key).unwrap_or_default(),
                        observed_q16_16: observed_for_objective(estimate, key),
                    },
                ))
            })?
        })
        .collect()
}

fn objective_target(report: &ScheduleCostReport, key: &gbf_policy::SatisfactionKey) -> Option<i64> {
    match key.axis {
        ObjectiveAxis::CyclesPerToken => report
            .objective
            .max_cycles_per_token
            .map(|target| i64::from(target) * UncertaintyEnvelope::Q16_ONE),
        ObjectiveAxis::BankSwitchesPerToken => report
            .objective
            .max_bank_switches_per_token
            .map(|target| i64::from(target) * UncertaintyEnvelope::Q16_ONE),
        ObjectiveAxis::SramPageSwitchesPerToken => report
            .objective
            .max_sram_page_switches_per_token
            .map(|target| i64::from(target) * UncertaintyEnvelope::Q16_ONE),
        ObjectiveAxis::SchedulerHeadroomUtilization => Some(
            i64::from(100_u8.saturating_sub(report.objective.min_ui_headroom_pct))
                * UncertaintyEnvelope::Q16_ONE
                / 100,
        ),
        ObjectiveAxis::TimeToFirstToken => report
            .objective
            .service
            .as_ref()?
            .max_first_token_cycles_p95
            .map(|target| i64::from(target) * UncertaintyEnvelope::Q16_ONE),
        ObjectiveAxis::FrameJitter => report
            .objective
            .service
            .as_ref()?
            .max_ui_jitter_frames_p99
            .map(|frames| {
                i64::from(frames)
                    * DEFAULT_FRAME_BUDGET_CYCLES as i64
                    * UncertaintyEnvelope::Q16_ONE
            }),
        ObjectiveAxis::SustainedThroughputTokensPerMegacycle => report
            .objective
            .min_sustained_throughput_tokens_per_megacycle
            .map(|target| i64::from(target) * UncertaintyEnvelope::Q16_ONE),
    }
}

fn observed_for_objective(estimate: &CostEstimate, key: &gbf_policy::SatisfactionKey) -> i64 {
    if key.axis == ObjectiveAxis::SustainedThroughputTokensPerMegacycle {
        estimate.envelope.lower_for(key.quantile)
    } else {
        estimate.envelope.upper_for(key.quantile)
    }
}

fn estimate_for_axis(delta: &EstimatedCostDelta, axis: ObjectiveAxis) -> Option<&CostEstimate> {
    match axis {
        ObjectiveAxis::CyclesPerToken => Some(&delta.cycles_per_token),
        ObjectiveAxis::BankSwitchesPerToken => Some(&delta.bank_switches_per_token),
        ObjectiveAxis::SramPageSwitchesPerToken => delta.sram_page_switches_per_token.as_ref(),
        ObjectiveAxis::SchedulerHeadroomUtilization => Some(&delta.scheduler_headroom_utilization),
        ObjectiveAxis::TimeToFirstToken => Some(&delta.time_to_first_token),
        ObjectiveAxis::SustainedThroughputTokensPerMegacycle => {
            Some(&delta.sustained_throughput_tokens_per_megacycle)
        }
        ObjectiveAxis::FrameJitter => delta.frame_jitter.as_ref(),
    }
}

fn mode_delta(per_mode: &[ModeEstimatedCost], mode: RuntimeMode) -> Option<&EstimatedCostDelta> {
    per_mode
        .iter()
        .find(|entry| entry.mode == mode)
        .map(|entry| &entry.delta)
}

fn mode_breakdown(
    per_mode: &[ModeCostBreakdownEntry],
    mode: RuntimeMode,
) -> Option<&ModeCostBreakdown> {
    per_mode
        .iter()
        .find(|entry| entry.mode == mode)
        .map(|entry| &entry.breakdown)
}

pub fn validate_schedule_cost_report(report: &ScheduleCostReport) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    diagnostics.extend(validate_satisfaction_matrix(report));
    diagnostics.extend(validate_option_fields(report));
    for mode_estimate in &report.per_mode {
        let mode = mode_estimate.mode;
        let delta = &mode_estimate.delta;
        let estimates = [
            ("cycles_per_token", Some(&delta.cycles_per_token)),
            (
                "bank_switches_per_token",
                Some(&delta.bank_switches_per_token),
            ),
            (
                "sram_page_switches_per_token",
                delta.sram_page_switches_per_token.as_ref(),
            ),
            ("yields_per_token", Some(&delta.yields_per_token)),
            (
                "scheduler_headroom_utilization",
                Some(&delta.scheduler_headroom_utilization),
            ),
            (
                "video_commit_cost_margin",
                delta.video_commit_cost_margin.as_ref(),
            ),
            (
                "max_no_progress_estimate",
                Some(&delta.max_no_progress_estimate),
            ),
            ("time_to_first_token", Some(&delta.time_to_first_token)),
            (
                "sustained_throughput_tokens_per_megacycle",
                Some(&delta.sustained_throughput_tokens_per_megacycle),
            ),
            ("frame_jitter", delta.frame_jitter.as_ref()),
        ];
        for (field, estimate) in estimates
            .into_iter()
            .filter_map(|(field, estimate)| estimate.map(|estimate| (field, estimate)))
        {
            if !estimate.envelope.is_ordered() {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostUncertaintyEnvelopeMalformed,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: "p95_lower <= p50 <= p95_upper <= p99_upper".to_owned(),
                    },
                ));
            }
            if !estimate.envelope.is_non_negative() {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostUncertaintyEnvelopeNegative,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: "cost envelopes must be non-negative".to_owned(),
                    },
                ));
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostFinalNonNegativityViolation,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: "final schedule-cost quantities must be non-negative".to_owned(),
                    },
                ));
            }
            if matches!(
                estimate.evidence_class,
                EvidenceClass::Calibrated | EvidenceClass::Transferred
            ) && estimate.refs.is_empty()
            {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: "calibrated/transferred estimates require refs".to_owned(),
                    },
                ));
            }
            let has_calibration_ref = estimate
                .refs
                .iter()
                .any(|reference| reference.kind == "F-B14CalibrationBundle");
            let has_transfer_ref = estimate
                .refs
                .iter()
                .any(|reference| reference.kind == "F-B14TransferPolicy");
            let has_heuristic_ref = estimate
                .refs
                .iter()
                .any(|reference| reference.kind == "F-B14HeuristicPolicy");
            if let Some(reference) = estimate
                .refs
                .iter()
                .find(|reference| reference.kind == "F-B14HeuristicPolicy")
                && !is_known_heuristic_policy_ref(reference)
            {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostHeuristicPolicyUnknown,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: reference.reference.clone(),
                    },
                ));
            }
            if let Some(reference) = estimate
                .refs
                .iter()
                .find(|reference| reference.kind == "F-B14TransferPolicy")
                && !is_known_transfer_policy_ref(reference)
            {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostTransferPolicyUnknown,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: reference.reference.clone(),
                    },
                ));
            }
            match estimate.evidence_class {
                EvidenceClass::Calibrated if !has_calibration_ref || has_transfer_ref => {
                    diagnostics.push(diagnostic(
                        ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent,
                        ScheduleCostDiagnosticProvenance::Estimate {
                            field: format!("{mode:?}.{field}"),
                            invariant:
                                "calibrated estimates require calibration refs and forbid transfer refs"
                                    .to_owned(),
                        },
                    ));
                }
                EvidenceClass::Transferred if !has_calibration_ref || !has_transfer_ref => {
                    diagnostics.push(diagnostic(
                        ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent,
                        ScheduleCostDiagnosticProvenance::Estimate {
                            field: format!("{mode:?}.{field}"),
                            invariant:
                                "transferred estimates require calibration and transfer refs"
                                    .to_owned(),
                        },
                    ));
                }
                EvidenceClass::Heuristic if !has_heuristic_ref => {
                    diagnostics.push(diagnostic(
                        ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent,
                        ScheduleCostDiagnosticProvenance::Estimate {
                            field: format!("{mode:?}.{field}"),
                            invariant: "heuristic estimates require heuristic policy refs"
                                .to_owned(),
                        },
                    ));
                }
                EvidenceClass::Fallback if !has_heuristic_ref => {
                    diagnostics.push(diagnostic(
                        ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent,
                        ScheduleCostDiagnosticProvenance::Estimate {
                            field: format!("{mode:?}.{field}"),
                            invariant: "fallback estimates require heuristic policy refs"
                                .to_owned(),
                        },
                    ));
                }
                _ => {}
            }
            if matches!(
                estimate.evidence_class,
                EvidenceClass::Heuristic | EvidenceClass::Fallback
            ) && estimate.fallback_reason.is_none()
            {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostFallbackReasonMissing,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant: "heuristic/fallback estimates require fallback_reason"
                            .to_owned(),
                    },
                ));
            }
            if matches!(
                estimate.evidence_class,
                EvidenceClass::Calibrated | EvidenceClass::Transferred
            ) && estimate.fallback_reason.is_some()
            {
                diagnostics.push(diagnostic(
                    ScheduleCostDiagnosticCode::CostFallbackReasonPresentForCalibrated,
                    ScheduleCostDiagnosticProvenance::Estimate {
                        field: format!("{mode:?}.{field}"),
                        invariant:
                            "calibrated/transferred estimates must not carry fallback_reason"
                                .to_owned(),
                    },
                ));
            }
        }
    }
    let expected_refs = report_refs_union(&report.per_mode);
    if report.refs != expected_refs {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostRefsUnionInconsistent,
            ScheduleCostDiagnosticProvenance::Estimate {
                field: "refs".to_owned(),
                invariant: "report refs must equal the deduplicated estimate refs union".to_owned(),
            },
        ));
    }
    diagnostics
}

fn validate_satisfaction_matrix(report: &ScheduleCostReport) -> Vec<ValidationDiagnostic> {
    let expected = build_satisfaction_matrix(&report.objective, &report.per_mode);
    let mut diagnostics = Vec::new();
    let expected_keys: BTreeSet<_> = expected.entries.iter().map(|entry| entry.key).collect();
    let actual_keys: BTreeSet<_> = report
        .satisfaction
        .entries
        .iter()
        .map(|entry| entry.key)
        .collect();
    for key in expected_keys.difference(&actual_keys) {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixIncomplete,
            ScheduleCostDiagnosticProvenance::Objective {
                mode: key.mode,
                axis: format!("{:?}", key.axis),
                quantile: format!("{:?}", key.quantile),
                target_q16_16: objective_target(report, key).unwrap_or_default(),
                observed_q16_16: estimate_for_axis(
                    mode_delta(&report.per_mode, key.mode).expect("expected mode exists"),
                    key.axis,
                )
                .map(|estimate| observed_for_objective(estimate, key))
                .unwrap_or_default(),
            },
        ));
    }
    for entry in &report.satisfaction.entries {
        let Some(expected_entry) = expected
            .entries
            .iter()
            .find(|expected_entry| expected_entry.key == entry.key)
        else {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixInconsistent,
                ScheduleCostDiagnosticProvenance::Objective {
                    mode: entry.key.mode,
                    axis: format!("{:?}", entry.key.axis),
                    quantile: format!("{:?}", entry.key.quantile),
                    target_q16_16: objective_target(report, &entry.key).unwrap_or_default(),
                    observed_q16_16: mode_delta(&report.per_mode, entry.key.mode)
                        .and_then(|delta| estimate_for_axis(delta, entry.key.axis))
                        .map(|estimate| observed_for_objective(estimate, &entry.key))
                        .unwrap_or_default(),
                },
            ));
            continue;
        };
        if expected_entry.satisfaction != entry.satisfaction {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixInconsistent,
                ScheduleCostDiagnosticProvenance::Objective {
                    mode: entry.key.mode,
                    axis: format!("{:?}", entry.key.axis),
                    quantile: format!("{:?}", entry.key.quantile),
                    target_q16_16: objective_target(report, &entry.key).unwrap_or_default(),
                    observed_q16_16: mode_delta(&report.per_mode, entry.key.mode)
                        .and_then(|delta| estimate_for_axis(delta, entry.key.axis))
                        .map(|estimate| observed_for_objective(estimate, &entry.key))
                        .unwrap_or_default(),
                },
            ));
        }
    }
    diagnostics
}

fn validate_option_fields(report: &ScheduleCostReport) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    for mode_estimate in &report.per_mode {
        let mode = mode_estimate.mode;
        let delta = &mode_estimate.delta;
        if report.objective.max_sram_page_switches_per_token.is_some()
            && delta.sram_page_switches_per_token.is_none()
        {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostOptionFieldMissing,
                ScheduleCostDiagnosticProvenance::Estimate {
                    field: format!("{mode:?}.sram_page_switches_per_token"),
                    invariant: "sram objective target requires an estimate".to_owned(),
                },
            ));
        }
        if report.objective.max_sram_page_switches_per_token.is_none()
            && delta.sram_page_switches_per_token.is_some()
            && mode_breakdown(&report.breakdown.per_mode, mode)
                .map(|breakdown| breakdown.totals.sram_page_switches == 0)
                .unwrap_or(false)
        {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostOptionFieldPresentUnexpectedly,
                ScheduleCostDiagnosticProvenance::Estimate {
                    field: format!("{mode:?}.sram_page_switches_per_token"),
                    invariant: "sram estimate is only emitted when targeted or observed".to_owned(),
                },
            ));
        }
        let frame_target = report
            .objective
            .service
            .as_ref()
            .and_then(|service| service.max_ui_jitter_frames_p99);
        if frame_target.is_some() && delta.frame_jitter.is_none() {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostOptionFieldMissing,
                ScheduleCostDiagnosticProvenance::Estimate {
                    field: format!("{mode:?}.frame_jitter"),
                    invariant: "frame jitter objective target requires an estimate".to_owned(),
                },
            ));
        }
        if frame_target.is_none() && delta.frame_jitter.is_some() {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostOptionFieldPresentUnexpectedly,
                ScheduleCostDiagnosticProvenance::Estimate {
                    field: format!("{mode:?}.frame_jitter"),
                    invariant: "frame jitter estimate is only emitted when targeted".to_owned(),
                },
            ));
        }
        if delta.video_commit_cost_margin.is_some() {
            diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostOptionFieldPresentUnexpectedly,
                ScheduleCostDiagnosticProvenance::Estimate {
                    field: format!("{mode:?}.video_commit_cost_margin"),
                    invariant: "video commit margin is reserved for a later producer".to_owned(),
                },
            ));
        }
    }
    diagnostics
}

fn is_known_heuristic_policy_ref(reference: &EvidenceRef) -> bool {
    const KNOWN: [&str; 9] = [
        "CyclesPerOpDefault",
        "BankSwitchesUpperBound",
        "SramPageSwitchesUpperBound",
        "YieldsPerTokenStatic",
        "HeadroomUtilizationDefault",
        "MaxNoProgressEstimateDefault",
        "TimeToFirstTokenDefault",
        "SustainedThroughputDefault",
        "FrameJitterDefault",
    ];
    KNOWN.iter().any(|policy| {
        reference.reference
            == format!(
                "heuristic://schedule_cost/{policy}/{}",
                SCHEDULE_COST_HEURISTIC_POLICY_VERSION
            )
    })
}

fn is_known_transfer_policy_ref(reference: &EvidenceRef) -> bool {
    reference
        .reference
        .starts_with("transfer://schedule_cost/Identity/")
}

pub fn validate_schedule_cost_json_bytes(bytes: &[u8]) -> Vec<ValidationDiagnostic> {
    let value = match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => value,
        Err(error) => {
            return vec![diagnostic(
                ScheduleCostDiagnosticCode::CostScheduleCostSchemaUnknown,
                ScheduleCostDiagnosticProvenance::JsonPath {
                    json_path: "$".to_owned(),
                    field_or_tag: error.to_string(),
                },
            )];
        }
    };
    validate_schedule_cost_json_value(&value)
}

pub fn validate_schedule_cost_json_value(value: &Value) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    detect_floating_point_fields(value, "$", &mut diagnostics);
    if value.get("schema").and_then(Value::as_str) != Some(SCHEDULE_COST_SCHEMA_ID) {
        diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostScheduleCostSchemaUnknown,
            ScheduleCostDiagnosticProvenance::JsonPath {
                json_path: "$.schema".to_owned(),
                field_or_tag: value
                    .get("schema")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing-or-non-string>")
                    .to_owned(),
            },
        ));
    }
    if diagnostics.is_empty()
        && let Some(report_value) = value.get("report")
        && !report_value.is_null()
    {
        match serde_json::from_value::<ScheduleCostReport>(report_value.clone()) {
            Ok(report) => diagnostics.extend(validate_schedule_cost_report(&report)),
            Err(error) => diagnostics.push(diagnostic(
                ScheduleCostDiagnosticCode::CostScheduleCostSchemaUnknown,
                ScheduleCostDiagnosticProvenance::JsonPath {
                    json_path: "$.report".to_owned(),
                    field_or_tag: error.to_string(),
                },
            )),
        }
    }
    diagnostics
}

fn detect_floating_point_fields(
    value: &Value,
    path: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match value {
        Value::Number(number) if number.is_f64() => diagnostics.push(diagnostic(
            ScheduleCostDiagnosticCode::CostFloatingPointFieldDetected,
            ScheduleCostDiagnosticProvenance::JsonPath {
                json_path: path.to_owned(),
                field_or_tag: number.to_string(),
            },
        )),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                detect_floating_point_fields(value, &format!("{path}[{index}]"), diagnostics);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                detect_floating_point_fields(value, &format!("{path}.{key}"), diagnostics);
            }
        }
        _ => {}
    }
}

fn report_refs_union(per_mode: &[ModeEstimatedCost]) -> Vec<EvidenceRef> {
    let mut refs = BTreeSet::new();
    for mode_estimate in per_mode {
        for estimate in cost_estimates(&mode_estimate.delta) {
            refs.extend(estimate.refs.iter().cloned());
        }
    }
    refs.into_iter().collect()
}

fn cost_estimates(delta: &EstimatedCostDelta) -> Vec<&CostEstimate> {
    let mut estimates = vec![
        &delta.cycles_per_token,
        &delta.bank_switches_per_token,
        &delta.yields_per_token,
        &delta.scheduler_headroom_utilization,
        &delta.max_no_progress_estimate,
        &delta.time_to_first_token,
        &delta.sustained_throughput_tokens_per_megacycle,
    ];
    if let Some(estimate) = &delta.sram_page_switches_per_token {
        estimates.push(estimate);
    }
    if let Some(estimate) = &delta.video_commit_cost_margin {
        estimates.push(estimate);
    }
    if let Some(estimate) = &delta.frame_jitter {
        estimates.push(estimate);
    }
    estimates
}

fn estimate_exact(
    units: i64,
    evidence: &EvidenceResolution,
    axis: ScheduleCostEvidenceAxis,
) -> CostEstimate {
    estimate_from_envelope(UncertaintyEnvelope::exact_units(units), evidence, axis)
}

fn estimate_from_envelope(
    envelope: UncertaintyEnvelope,
    evidence: &EvidenceResolution,
    axis: ScheduleCostEvidenceAxis,
) -> CostEstimate {
    CostEstimate {
        evidence_class: evidence.class,
        envelope,
        refs: compose_evidence_refs(evidence, axis, envelope),
        fallback_reason: evidence.fallback_reason.clone(),
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct AxisCompositionHashInput {
    pass_version: &'static str,
    axis: &'static str,
    envelope: UncertaintyEnvelope,
}

fn compose_evidence_refs(
    evidence: &EvidenceResolution,
    axis: ScheduleCostEvidenceAxis,
    envelope: UncertaintyEnvelope,
) -> Vec<EvidenceRef> {
    let axis_hash = DomainHash::new(
        "gbf-codegen",
        "F-B14AxisEvidence",
        SCHEDULE_COST_SCHEMA_ID,
        "1.0.0",
    )
    .hash(&AxisCompositionHashInput {
        pass_version: SCHEDULE_COST_PASS_VERSION,
        axis: axis.as_str(),
        envelope,
    })
    .expect("axis evidence hash input is canonical-serializable");
    let mut refs: BTreeSet<_> = evidence.base_refs.iter().cloned().collect();
    if matches!(
        evidence.class,
        EvidenceClass::Heuristic | EvidenceClass::Fallback
    ) {
        refs.insert(heuristic_ref(axis.heuristic_policy_id()));
    }
    refs.insert(
        Fb14EvidenceRef::AxisComposition {
            axis,
            hash: axis_hash,
        }
        .into_ref(),
    );
    refs.into_iter().collect()
}

fn q16_ratio(numerator: u64, denominator: u64) -> i64 {
    (((u128::from(numerator)) << 16) / u128::from(denominator.max(1))) as i64
}

fn frame_budget_cycles(pack: &SchedulePack) -> u64 {
    pack.modes
        .iter()
        .flat_map(|mode| mode.ir.slices.iter())
        .map(|slice| u64::from(slice.soft_target_cycles.max(1)))
        .min()
        .unwrap_or(DEFAULT_FRAME_BUDGET_CYCLES)
}

fn kernel_registry_diagnostics(inputs: &ScheduleCostInputs) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    for (mode_index, mode_schedule) in inputs.schedule_pack.modes.iter().enumerate() {
        for (slice_index, slice) in mode_schedule.ir.slices.iter().enumerate() {
            for (op_index, op) in slice.ops.iter().enumerate() {
                let SchedOp::KernelCall { spec, .. } = op else {
                    continue;
                };
                if !inputs.kernel_spec_registry.contains(spec) {
                    diagnostics.push(diagnostic(
                        ScheduleCostDiagnosticCode::CostKernelSpecNotInRegistry,
                        ScheduleCostDiagnosticProvenance::JsonPath {
                            json_path: format!(
                                "schedule_pack.modes[{mode_index}].ir.slices[{slice_index}].ops[{op_index}].spec"
                            ),
                            field_or_tag: spec.as_str().to_owned(),
                        },
                    ));
                }
            }
        }
    }
    diagnostics
}

fn hash_or_diagnostic<T: Serialize>(
    type_name: &'static str,
    schema_id: &'static str,
    value: &T,
    code: ScheduleCostDiagnosticCode,
    field: &'static str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> Hash256 {
    match DomainHash::new("gbf-codegen", type_name, schema_id, "1.0.0").hash(value) {
        Ok(hash) => hash,
        Err(error) => {
            diagnostics.push(diagnostic(
                code,
                ScheduleCostDiagnosticProvenance::JsonPath {
                    json_path: field.to_owned(),
                    field_or_tag: error.to_string(),
                },
            ));
            Hash256::ZERO
        }
    }
}

fn hash_debug_string(value: &str) -> Hash256 {
    DomainHash::new(
        "gbf-codegen",
        "DebugString",
        SCHEDULE_COST_SCHEMA_ID,
        "1.0.0",
    )
    .hash(&value)
    .expect("debug string is canonical-serializable")
}

fn heuristic_ref(policy_id: &'static str) -> EvidenceRef {
    Fb14EvidenceRef::HeuristicPolicy { policy_id }.into_ref()
}

fn validate_schedule_cost_report_body(
    has_report: bool,
    diagnostics: &[ValidationDiagnostic],
    outcome: ReportOutcome,
) -> Result<(), Vec<ValidationDiagnostic>> {
    let mut errors = Vec::new();
    match outcome {
        ReportOutcome::Passed => {
            if !has_report || !diagnostics.is_empty() {
                errors.push(report_semantic_diagnostic("outcome"));
            }
        }
        ReportOutcome::Failed => {
            if diagnostics.is_empty() {
                errors.push(report_semantic_diagnostic("diagnostics"));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn report_semantic_diagnostic(field: &str) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Schema,
        ValidationCode::ReportSemanticInvariantViolated {
            field: gbf_foundation::FieldPath::from(field),
        },
        ValidationDetail::Field {
            field: gbf_foundation::FieldPath::from(field),
        },
        Vec::new(),
    )
}

fn diagnostic(
    code: ScheduleCostDiagnosticCode,
    provenance: ScheduleCostDiagnosticProvenance,
) -> ValidationDiagnostic {
    ValidationDiagnostic::new(
        DiagnosticSeverity::Hard,
        ValidationOrigin::ScheduleCostAnalysis,
        ValidationCode::ScheduleCost { code, provenance },
        ValidationDetail::None,
        vec![EvidenceRef {
            kind: "ScheduleCostAnalysis".to_owned(),
            reference: format!("diagnostic://schedule_cost/{}", code.as_str()),
            hash: None,
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};
    use gbf_policy::{
        BootstrapCalibrationBundle, CompileKnobOverrides, CompileKnobPartialBounds,
        CompileKnobPartialValues, CompileKnobValues, CompileKnobs, EffectiveConstraints,
        KnobLockSet, ObservabilityMode, ObservationKnob, ObservationProfileCaps, OverlayKnob,
        OverlayPromotion, PlacementKnob, PlacementProfile, PolicyProvenance, ProbeCollectionLevel,
        RangeCapsSpec, RangeKnob, ReductionPlanCeiling, RepairPolicy, RepairPolicyProfile,
        RiskPolicy, RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob, ScheduleKnob,
        ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch,
        ServiceLevelObjective, SramKnob, SramPageAggression, StorageKnob, StorageMaterialization,
        TraceBudget, TraceDropPolicy, canonical_default_bounds_fixture,
    };

    use crate::s1::quant_graph::DeterminismClass;
    use crate::schedule::{
        DriftEnvelope, EntryResidency, ExitKind, GbSchedIR, InterruptPolicy, ModeResidencyEpochs,
        ModeSchedule, ModeSwitchPolicy, ObservedDriftEnvelope, ResourceVector, RuntimeDriftMonitor,
        SafeModeTrigger, SchedSlice, SchedulePack, SchedulePackInputIdentity, SliceId,
        UiPressureThreshold, YieldCheckClass, YieldKind,
    };

    #[test]
    fn schedule_cost_passes_for_static_schedule() {
        let output = analyze_schedule_cost(&inputs_with_objective(|objective| {
            objective.max_cycles_per_token = Some(10_000);
            objective.max_bank_switches_per_token = Some(2);
        }));

        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let report = output.report.as_ref().expect("report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");
        assert_eq!(
            delta.cycles_per_token.envelope.p50_q16_16,
            112 * UncertaintyEnvelope::Q16_ONE
        );
        assert_eq!(
            mode_breakdown(&report.breakdown.per_mode, RuntimeMode::Interactive)
                .expect("interactive breakdown")
                .totals
                .bank_switch_cycles,
            BANK_SWITCH_CYCLES
        );
    }

    #[test]
    fn schedule_cost_rejects_over_budget_objective() {
        let output = analyze_schedule_cost(&inputs_with_objective(|objective| {
            objective.max_cycles_per_token = Some(100);
        }));

        assert_eq!(output.outcome, ScheduleCostOutcome::Failed);
        assert!(output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ScheduleCost {
                code: ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixInconsistent,
                ..
            }
        )));
    }

    #[test]
    fn schedule_cost_cache_key_is_deterministic_and_sensitive() {
        let first = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let identity = first.report.as_ref().expect("report").identity;
        let key_inputs = ScheduleCostCacheKeyInputs::from_identity(&identity);
        let key_a = key_inputs.cache_key().expect("cache key");
        let key_b = key_inputs.cache_key().expect("cache key");
        assert_eq!(key_a, key_b);

        let mut changed = key_inputs;
        changed.kernel_spec_registry_hash = hash(0xee);
        let key_c = changed.cache_key().expect("cache key");
        assert_ne!(key_a, key_c);
    }

    #[test]
    fn schedule_cost_cache_key_excludes_policy_resolution_audit_parent() {
        let first = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let identity = first.report.as_ref().expect("report").identity;
        let key_inputs = ScheduleCostCacheKeyInputs::from_identity(&identity);
        let key_a = key_inputs.cache_key().expect("cache key");

        let mut audit_only_changed = key_inputs;
        audit_only_changed.policy_resolution_self_hash = hash(0xee);
        assert_eq!(
            key_a,
            audit_only_changed.cache_key().expect("cache key"),
            "K14 must be isolated from full policy-resolution audit-parent drift"
        );

        let mut projection_changed = key_inputs;
        projection_changed.schedule_cost_policy_projection_hash = hash(0xef);
        assert_ne!(
            key_a,
            projection_changed.cache_key().expect("cache key"),
            "K14 must still miss when the F-B14 policy projection changes"
        );
    }

    #[test]
    fn schedule_cost_report_round_trips() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let envelope = emit_schedule_cost_report(&output).expect("schedule cost report");
        round_trip_self_hash(&envelope).expect("round trip");
        let bytes = emit_schedule_cost_json_bytes(&output).expect("json bytes");
        let decoded: ScheduleCostReportEnvelope =
            serde_json::from_slice(&bytes).expect("canonical json decodes");
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn schedule_cost_report_self_hash_recomputes_from_populated_report() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let report = output.report.as_ref().expect("schedule cost report");

        assert_ne!(
            report.identity.schedule_cost_report_self_hash,
            Hash256::ZERO
        );
        assert_eq!(
            schedule_cost_report_self_hash(report).expect("self hash recomputes"),
            report.identity.schedule_cost_report_self_hash
        );
    }

    #[test]
    fn schedule_cost_heuristic_refs_and_report_refs_are_honest() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let report = output.report.as_ref().expect("schedule cost report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");

        assert_eq!(
            delta.cycles_per_token.evidence_class,
            EvidenceClass::Heuristic
        );
        assert!(matches!(
            delta.cycles_per_token.fallback_reason,
            Some(FallbackReason::KernelSpecNotCalibrated)
        ));
        assert!(
            delta
                .cycles_per_token
                .refs
                .iter()
                .any(|reference| reference.kind == "F-B14HeuristicPolicy")
        );
        assert_eq!(report.refs, report_refs_union(&report.per_mode));
    }

    #[test]
    fn schedule_cost_composes_per_axis_evidence_refs() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let report = output.report.as_ref().expect("schedule cost report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");

        let cycles_axis = axis_ref(&delta.cycles_per_token).expect("cycles axis ref");
        let bank_axis = axis_ref(&delta.bank_switches_per_token).expect("bank axis ref");
        let throughput_axis =
            axis_ref(&delta.sustained_throughput_tokens_per_megacycle).expect("throughput ref");

        assert_eq!(
            cycles_axis.reference,
            "schedule-cost://axis/cycles_per_token"
        );
        assert_eq!(
            bank_axis.reference,
            "schedule-cost://axis/bank_switches_per_token"
        );
        assert_eq!(
            throughput_axis.reference,
            "schedule-cost://axis/sustained_throughput_tokens_per_megacycle"
        );
        assert_ne!(cycles_axis.hash, bank_axis.hash);
        assert!(
            delta
                .cycles_per_token
                .refs
                .iter()
                .any(|reference| reference.kind == "F-B14HeuristicPolicy")
        );
        assert_eq!(report.refs, report_refs_union(&report.per_mode));
    }

    #[test]
    fn schedule_cost_rejects_strict_missing_calibration() {
        let output = analyze_schedule_cost(&inputs_with_objective(|objective| {
            objective.risk.calibration_confidence_requirement =
                CalibrationConfidenceRequirement::AtLeast {
                    class: CalibrationConfidenceClass::Transferred,
                };
        }));

        assert_eq!(output.outcome, ScheduleCostOutcome::Failed);
        assert!(output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.code,
            ValidationCode::ScheduleCost {
                code: ScheduleCostDiagnosticCode::CostCalibrationMissingForRequirement,
                ..
            }
        )));
    }

    #[test]
    fn schedule_cost_rejects_stale_calibration_with_bundle_stale_fallback() {
        let mut inputs = inputs_with_calibration(CalibrationConfidenceClass::Reasonable);
        for bundle in inputs.calibration_bundle_set.bundles.values_mut() {
            bundle.target_profile_hash = hash(0xde);
        }

        let output = analyze_schedule_cost(&inputs);

        assert_eq!(output.outcome, ScheduleCostOutcome::Failed);
        assert!(has_schedule_cost_code(
            &output.diagnostics,
            ScheduleCostDiagnosticCode::CostCalibrationBundleStale
        ));
        let report = output.report.as_ref().expect("schedule cost report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");
        assert_eq!(
            delta.cycles_per_token.evidence_class,
            EvidenceClass::Heuristic
        );
        assert!(matches!(
            delta.cycles_per_token.fallback_reason,
            Some(FallbackReason::BundleStale {
                field: StaleCalibrationField::TargetProfileHash,
                ..
            })
        ));
        assert!(has_ref_kind(
            &delta.cycles_per_token,
            "F-B14HeuristicPolicy"
        ));
        assert!(!has_ref_kind(
            &delta.cycles_per_token,
            "F-B14CalibrationBundle"
        ));
    }

    #[test]
    fn schedule_cost_rejects_kernel_call_not_in_typed_registry() {
        let mut inputs = inputs_with_objective(|_| {});
        inputs.schedule_pack.modes[0].ir.slices[0].ops.insert(
            0,
            SchedOp::KernelCall {
                spec: KernelSpecId::from("kernel.unregistered"),
                tile_index: 0,
            },
        );

        let output = analyze_schedule_cost(&inputs);

        assert_eq!(output.outcome, ScheduleCostOutcome::Failed);
        assert!(has_schedule_cost_code(
            &output.diagnostics,
            ScheduleCostDiagnosticCode::CostKernelSpecNotInRegistry
        ));
    }

    #[test]
    fn schedule_cost_rejects_stale_session_profile_freshness() {
        let mut inputs = inputs_with_calibration(CalibrationConfidenceClass::Reasonable);
        inputs.active_session_profile.compile_profile = CompileProfileId::from("Trace");

        let output = analyze_schedule_cost(&inputs);

        assert_eq!(output.outcome, ScheduleCostOutcome::Failed);
        assert!(has_schedule_cost_code(
            &output.diagnostics,
            ScheduleCostDiagnosticCode::CostCalibrationBundleStale
        ));
        let report = output.report.as_ref().expect("schedule cost report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");
        assert!(matches!(
            delta.cycles_per_token.fallback_reason,
            Some(FallbackReason::BundleStale {
                field: StaleCalibrationField::ValidityEnvelope,
                ..
            })
        ));
    }

    #[test]
    fn schedule_cost_transferred_estimates_carry_bundle_and_transfer_refs() {
        let output = analyze_schedule_cost(&inputs_with_calibration(
            CalibrationConfidenceClass::Transferred,
        ));

        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let report = output.report.as_ref().expect("schedule cost report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");

        assert_eq!(
            delta.cycles_per_token.evidence_class,
            EvidenceClass::Transferred
        );
        assert_eq!(delta.cycles_per_token.fallback_reason, None);
        assert!(has_ref_kind(
            &delta.cycles_per_token,
            "F-B14CalibrationBundle"
        ));
        assert!(has_ref_kind(&delta.cycles_per_token, "F-B14TransferPolicy"));
    }

    #[test]
    fn schedule_cost_validator_rejects_transferred_without_transfer_ref() {
        let output = analyze_schedule_cost(&inputs_with_calibration(
            CalibrationConfidenceClass::Transferred,
        ));
        let mut report = output.report.expect("schedule cost report");
        report.per_mode[0]
            .delta
            .cycles_per_token
            .refs
            .retain(|reference| reference.kind != "F-B14TransferPolicy");
        report.refs = report_refs_union(&report.per_mode);

        let diagnostics = validate_schedule_cost_report(&report);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent
        ));
    }

    #[test]
    fn schedule_cost_validator_rejects_calibrated_with_transfer_ref() {
        let output = analyze_schedule_cost(&inputs_with_calibration(
            CalibrationConfidenceClass::Reasonable,
        ));
        let mut report = output.report.expect("schedule cost report");
        report.per_mode[0].delta.cycles_per_token.refs.push(
            Fb14EvidenceRef::TransferPolicy {
                policy_id: "Identity",
                from_target: "dmg-mbc5".to_owned(),
                to_target: "dmg-mbc5".to_owned(),
                hash: hash(0xab),
            }
            .into_ref(),
        );
        report.refs = report_refs_union(&report.per_mode);

        let diagnostics = validate_schedule_cost_report(&report);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostEvidenceClassRefsInconsistent
        ));
    }

    #[test]
    fn schedule_cost_service_metrics_use_schedule_shape() {
        let mut inputs = inputs_with_objective(|objective| {
            objective.service = Some(ServiceLevelObjective {
                max_first_token_cycles_p95: Some(200),
                max_checkpoint_gap_cycles_p95: None,
                max_resume_latency_cycles_p95: None,
                max_ui_jitter_frames_p99: Some(1),
            });
        });
        let first_slice = inputs.schedule_pack.modes[0].ir.slices[0].clone();
        let mut second_slice = first_slice;
        second_slice.id = SliceId(1);
        second_slice.ops.clear();
        second_slice.resources.bank_switches = 0;
        second_slice.yield_kind = YieldKind::Micro;
        inputs.schedule_pack.modes[0].ir.slices.push(second_slice);

        let output = analyze_schedule_cost(&inputs);

        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let report = output.report.as_ref().expect("schedule cost report");
        let delta =
            mode_delta(&report.per_mode, RuntimeMode::Interactive).expect("interactive estimate");
        assert_eq!(
            delta.time_to_first_token.envelope.p50_q16_16,
            112 * UncertaintyEnvelope::Q16_ONE
        );
        assert_eq!(
            delta
                .frame_jitter
                .as_ref()
                .expect("frame jitter")
                .envelope
                .p50_q16_16,
            80 * UncertaintyEnvelope::Q16_ONE
        );
        for axis in [ObjectiveAxis::TimeToFirstToken, ObjectiveAxis::FrameJitter] {
            for quantile in [Quantile::P50, Quantile::P95, Quantile::P99] {
                assert!(report.satisfaction.entries.iter().any(|entry| {
                    entry.key.mode == RuntimeMode::Interactive
                        && entry.key.axis == axis
                        && entry.key.quantile == quantile
                }));
            }
        }
    }

    #[test]
    fn schedule_cost_projects_min_throughput_objective() {
        let output = analyze_schedule_cost(&inputs_with_objective(|objective| {
            objective.min_sustained_throughput_tokens_per_megacycle = Some(8_000);
        }));

        assert_eq!(
            output.outcome,
            ScheduleCostOutcome::Succeeded,
            "{:?}",
            output.diagnostics
        );
        let report = output.report.as_ref().expect("schedule cost report");
        for quantile in [Quantile::P50, Quantile::P95, Quantile::P99] {
            assert!(report.satisfaction.entries.iter().any(|entry| {
                entry.key.mode == RuntimeMode::Interactive
                    && entry.key.axis == ObjectiveAxis::SustainedThroughputTokensPerMegacycle
                    && entry.key.quantile == quantile
                    && entry.satisfaction == ObjectiveSatisfaction::Satisfied
            }));
        }

        let violated = analyze_schedule_cost(&inputs_with_objective(|objective| {
            objective.min_sustained_throughput_tokens_per_megacycle = Some(9_000);
        }));

        assert_eq!(violated.outcome, ScheduleCostOutcome::Failed);
        assert!(has_schedule_cost_code(
            &violated.diagnostics,
            ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixInconsistent
        ));
    }

    #[test]
    fn schedule_cost_validator_rejects_incomplete_satisfaction_matrix() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let mut report = output.report.expect("schedule cost report");
        report.satisfaction.entries.pop();

        let diagnostics = validate_schedule_cost_report(&report);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixIncomplete
        ));
    }

    #[test]
    fn schedule_cost_validator_rejects_unknown_heuristic_policy() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let mut report = output.report.expect("schedule cost report");
        let reference = report.per_mode[0]
            .delta
            .cycles_per_token
            .refs
            .iter_mut()
            .find(|reference| reference.kind == "F-B14HeuristicPolicy")
            .expect("heuristic ref");
        reference.reference = "heuristic://schedule_cost/Unknown/1.0.0".to_owned();

        let diagnostics = validate_schedule_cost_report(&report);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostHeuristicPolicyUnknown
        ));
    }

    #[test]
    fn schedule_cost_validator_rejects_unknown_transfer_policy() {
        let output = analyze_schedule_cost(&inputs_with_calibration(
            CalibrationConfidenceClass::Transferred,
        ));
        let mut report = output.report.expect("schedule cost report");
        let reference = report.per_mode[0]
            .delta
            .cycles_per_token
            .refs
            .iter_mut()
            .find(|reference| reference.kind == "F-B14TransferPolicy")
            .expect("transfer ref");
        reference.reference = "transfer://schedule_cost/Unknown/dmg-mbc5/dmg-mbc5".to_owned();

        let diagnostics = validate_schedule_cost_report(&report);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostTransferPolicyUnknown
        ));
    }

    #[test]
    fn schedule_cost_json_validator_rejects_float_and_unknown_schema() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let bytes = emit_schedule_cost_json_bytes(&output).expect("json bytes");
        let mut value: Value = serde_json::from_slice(&bytes).expect("json value");
        value["body"]["pass_version"] = serde_json::json!(1.5);
        value["schema"] = serde_json::json!("schedule_cost.v2");

        let diagnostics = validate_schedule_cost_json_value(&value);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostFloatingPointFieldDetected
        ));
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostScheduleCostSchemaUnknown
        ));
    }

    #[test]
    fn schedule_cost_json_validator_rejects_mutated_policy_and_matrix_fields() {
        let diagnostics = validate_mutated_schedule_cost_json(
            analyze_schedule_cost(&inputs_with_objective(|_| {})),
            |value| {
                value["report"]["satisfaction"]["entries"]
                    .as_array_mut()
                    .expect("satisfaction entries")
                    .pop();
            },
        );
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostObjectiveSatisfactionMatrixIncomplete
        ));

        let diagnostics = validate_mutated_schedule_cost_json(
            analyze_schedule_cost(&inputs_with_objective(|_| {})),
            |value| {
                let refs = value["report"]["per_mode"][0]["delta"]["cycles_per_token"]["refs"]
                    .as_array_mut()
                    .expect("cycles refs");
                let reference = refs
                    .iter_mut()
                    .find(|reference| reference["kind"] == "F-B14HeuristicPolicy")
                    .expect("heuristic ref");
                reference["reference"] =
                    serde_json::json!("heuristic://schedule_cost/Unknown/1.0.0");
            },
        );
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostHeuristicPolicyUnknown
        ));

        let diagnostics = validate_mutated_schedule_cost_json(
            analyze_schedule_cost(&inputs_with_calibration(
                CalibrationConfidenceClass::Transferred,
            )),
            |value| {
                let refs = value["report"]["per_mode"][0]["delta"]["cycles_per_token"]["refs"]
                    .as_array_mut()
                    .expect("cycles refs");
                let reference = refs
                    .iter_mut()
                    .find(|reference| reference["kind"] == "F-B14TransferPolicy")
                    .expect("transfer ref");
                reference["reference"] =
                    serde_json::json!("transfer://schedule_cost/Unknown/dmg-mbc5/dmg-mbc5");
            },
        );
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostTransferPolicyUnknown
        ));
    }

    #[test]
    fn schedule_cost_json_validator_rejects_mutated_option_and_final_fields() {
        let diagnostics = validate_mutated_schedule_cost_json(
            analyze_schedule_cost(&inputs_with_objective(|_| {})),
            |value| {
                value["report"]["per_mode"][0]["delta"]["sram_page_switches_per_token"] =
                    Value::Null;
            },
        );
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostOptionFieldMissing
        ));

        let diagnostics = validate_mutated_schedule_cost_json(
            analyze_schedule_cost(&inputs_with_objective(|_| {})),
            |value| {
                value["report"]["per_mode"][0]["delta"]["video_commit_cost_margin"] =
                    value["report"]["per_mode"][0]["delta"]["cycles_per_token"].clone();
            },
        );
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostOptionFieldPresentUnexpectedly
        ));

        let diagnostics = validate_mutated_schedule_cost_json(
            analyze_schedule_cost(&inputs_with_objective(|_| {})),
            |value| {
                value["report"]["per_mode"][0]["delta"]["scheduler_headroom_utilization"]["envelope"]
                    ["p50_q16_16"] = serde_json::json!(-1);
            },
        );
        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostFinalNonNegativityViolation
        ));
    }

    #[test]
    fn schedule_cost_validator_rejects_option_field_contract_drift() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let mut missing = output.report.clone().expect("schedule cost report");
        missing.per_mode[0].delta.sram_page_switches_per_token = None;

        let missing_diagnostics = validate_schedule_cost_report(&missing);

        assert!(has_schedule_cost_code(
            &missing_diagnostics,
            ScheduleCostDiagnosticCode::CostOptionFieldMissing
        ));

        let mut unexpected = output.report.expect("schedule cost report");
        unexpected.per_mode[0].delta.video_commit_cost_margin =
            Some(unexpected.per_mode[0].delta.cycles_per_token.clone());

        let unexpected_diagnostics = validate_schedule_cost_report(&unexpected);

        assert!(has_schedule_cost_code(
            &unexpected_diagnostics,
            ScheduleCostDiagnosticCode::CostOptionFieldPresentUnexpectedly
        ));
    }

    #[test]
    fn schedule_cost_validator_rejects_final_negative_quantities() {
        let output = analyze_schedule_cost(&inputs_with_objective(|_| {}));
        let mut report = output.report.expect("schedule cost report");
        report.per_mode[0]
            .delta
            .scheduler_headroom_utilization
            .envelope
            .p50_q16_16 = -1;

        let diagnostics = validate_schedule_cost_report(&report);

        assert!(has_schedule_cost_code(
            &diagnostics,
            ScheduleCostDiagnosticCode::CostFinalNonNegativityViolation
        ));
    }

    fn inputs_with_objective(mut edit: impl FnMut(&mut CompileObjective)) -> ScheduleCostInputs {
        let mut policy = policy_fixture();
        edit(&mut policy.objective);
        let schedule_pack = schedule_pack_fixture();
        let kernel_spec_registry = ScheduleCostKernelRegistry::from_schedule_pack(&schedule_pack);
        let kernel_spec_registry_hash =
            kernel_spec_registry.registry_hash().expect("registry hash");
        let active_session_profile = active_session_profile_fixture(&policy, hash(0x40));
        ScheduleCostInputs {
            schedule_pack,
            policy,
            calibration_bundle_set: BootstrapCalibrationBundle::new(hash(0x40)),
            runtime_chrome_budget: runtime_budget_fixture(),
            target_profile_hash: hash(0x40),
            kernel_spec_registry,
            kernel_spec_registry_hash,
            active_session_profile,
            crate_feature_set_hash: hash(0x42),
            schedule_cost_schema_hash: hash(0x43),
        }
    }

    fn inputs_with_calibration(confidence: CalibrationConfidenceClass) -> ScheduleCostInputs {
        let mut inputs = inputs_with_objective(|_| {});
        for bundle in inputs.calibration_bundle_set.bundles.values_mut() {
            bundle.target_profile_hash = inputs.target_profile_hash;
            bundle.kernel_set_hash = inputs.kernel_spec_registry_hash;
            bundle.packer_version = gbf_runtime::RUNTIME_PACKER_VERSION;
            bundle.calibration_schema_hash = Hash256::ZERO;
            bundle
                .validity_envelope
                .session_profiles
                .insert(inputs.active_session_profile.clone());
            bundle.confidence = confidence;
        }
        inputs
    }

    fn active_session_profile_fixture(
        policy: &ResolvedCompilePolicy,
        target_profile_hash: Hash256,
    ) -> CalibrationSessionProfile {
        CalibrationSessionProfile {
            target_profile_hash,
            compile_profile: policy.profile.clone(),
            runtime_modes: policy.requested_runtime_modes.clone(),
        }
    }

    fn schedule_pack_fixture() -> SchedulePack {
        SchedulePack {
            identity: SchedulePackInputIdentity {
                infer_ir_self_hash: hash(0x01),
                observation_plan_self_hash: hash(0x02),
                range_plan_self_hash: hash(0x03),
                storage_plan_self_hash: hash(0x04),
                sram_page_plan_self_hash: hash(0x05),
                rom_window_plan_self_hash: hash(0x06),
                overlay_plan_self_hash: hash(0x07),
                arena_plan_self_hash: hash(0x08),
                policy_resolution_self_hash: hash(0x09),
                runtime_chrome_budget_self_hash: hash(0x0a),
                feature_set_hash: hash(0x0b),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                determinism: DeterminismClass::Deterministic,
                target_profile_id: TargetProfileId::from("dmg-mbc5"),
                schema_version: SemVer::new(1, 0, 0),
            },
            modes: vec![ModeSchedule {
                mode: RuntimeMode::Interactive,
                ir: GbSchedIR {
                    mode: RuntimeMode::Interactive,
                    entry_slice: SliceId(0),
                    slices: vec![SchedSlice {
                        id: SliceId(0),
                        ops: vec![
                            SchedOp::BankSwitch {
                                binding: crate::window::RomWindowBindingId(0),
                                bank: crate::window::RomBankIndex(1),
                            },
                            SchedOp::Yield {
                                kind: YieldKind::TokenReady,
                            },
                        ],
                        hard_cycles_to_safe_point: 17_556,
                        soft_target_cycles: 14_000,
                        max_interrupt_latency: 40,
                        resources: ResourceVector {
                            bank_switches: 1,
                            sram_page_switches: 0,
                            trace_bytes: 0,
                            persist_bytes: 0,
                            overlay_installs: 0,
                        },
                        live_wram: Vec::new(),
                        live_sram: Vec::new(),
                        yield_kind: YieldKind::TokenReady,
                        yield_check: YieldCheckClass::OnceAtEnd,
                        entry_residency: EntryResidency::Bank0,
                        interrupt_policy: InterruptPolicy::Enabled,
                        required_leases: Vec::new(),
                        exit_kind: ExitKind::SaveContinuationAndYield,
                        semantic_checkpoint_pins: Vec::new(),
                        trace_probe_pins: Vec::new(),
                        successors: Vec::new(),
                    }],
                },
            }],
            epochs: vec![ModeResidencyEpochs {
                mode: RuntimeMode::Interactive,
                epochs: Vec::new(),
            }],
            leases: Vec::new(),
            checkpoint_schema_hash: hash(0x0c),
            continuation_abi_hash: hash(0x0d),
            switch_policy: ModeSwitchPolicy {
                legal_switch_points: Vec::new(),
                legal_epoch_boundaries: Vec::new(),
                ui_pressure_thresholds: vec![UiPressureThreshold {
                    max_pending_frames: 1,
                }],
                safe_mode_triggers: vec![SafeModeTrigger::Fault],
                drift_triggers: Vec::new(),
            },
            drift_monitor: RuntimeDriftMonitor {
                expected: DriftEnvelope {
                    slice_cycles_p95: Some(17_556),
                    ui_commit_cycles_p95: None,
                    trace_drop_rate_pct: Some(0),
                    persist_overrun_rate_pct: Some(0),
                },
                observed: ObservedDriftEnvelope::all_none(),
                consecutive_violations: 0,
                window_frames: 60,
            },
            schedule_pack_self_hash: hash(0x0e),
        }
    }

    fn policy_fixture() -> ResolvedCompilePolicy {
        let bounds = canonical_default_bounds_fixture();
        let requested_runtime_modes = BTreeSet::from([RuntimeMode::Interactive]);
        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: CompileObjective {
                service: None,
                max_cycles_per_token: Some(10_000),
                max_bank_switches_per_token: Some(4),
                max_sram_page_switches_per_token: Some(2),
                min_sustained_throughput_tokens_per_megacycle: None,
                min_ui_headroom_pct: 10,
                max_rom_bytes: Some(2 * 1024 * 1024),
                risk: RiskPolicy {
                    cycle_quantile: 95,
                    switch_quantile: 99,
                    calibration_confidence_requirement:
                        CalibrationConfidenceRequirement::NoMinimumConfidence,
                    fallback_profile: None,
                    fallback_runtime_mode: Some(RuntimeMode::Safe),
                },
            },
            effective_constraints: EffectiveConstraints {
                target_caps: bounds.clone(),
                required_features: BTreeSet::new(),
                requested_runtime_modes: requested_runtime_modes.clone(),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Flexible,
            trace_budget: TraceBudget {
                max_events_per_slice: 8,
                max_bytes_per_frame: 512,
                drop_policy: TraceDropPolicy::HaltAndFault,
            },
            range_caps: RangeCapsSpec::default_v2(),
            observation_caps: ObservationProfileCaps::default_v2(),
            requested_runtime_modes,
            knobs: CompileKnobs {
                global: CompileKnobValues {
                    placement: PlacementKnob {
                        profile: PlacementProfile::Budgeted,
                    },
                    observation: ObservationKnob {
                        observability: ObservabilityMode::Flexible,
                        trace_demotion: gbf_policy::TraceDemotionLevel::None,
                        probe_level: ProbeCollectionLevel::Operational,
                    },
                    range: RangeKnob {
                        reduction_ceiling: ReductionPlanCeiling::Conservative,
                    },
                    storage: StorageKnob {
                        materialization: StorageMaterialization::RecomputePureValues,
                    },
                    sram: SramKnob {
                        page_aggression: SramPageAggression::PackCold,
                        spill_policy: gbf_policy::SramSpillPolicy::SpillOnPressure,
                    },
                    rom_window: RomWindowKnob {
                        kernel_residency_bias: RomKernelResidencyBias::PreferCommonBank,
                        kernel_duplication_bias: RomKernelDuplicationBias::Share,
                    },
                    overlay: OverlayKnob {
                        promotion: OverlayPromotion::TinyLuts,
                    },
                    schedule: ScheduleKnob {
                        tile_search: ScheduleTileSearch::Local,
                        slice_coarsening: ScheduleSliceCoarsening::Balanced,
                        resource_pressure: ScheduleResourcePressure::Balanced,
                        pressure_thresholds: gbf_policy::ResourcePressureThresholds::default(),
                        stage_iteration_ceilings: gbf_policy::StageIterationLimits::uniform(4),
                    },
                },
                bounds,
                locks: KnobLockSet::default(),
                overrides: CompileKnobOverrides {
                    values: CompileKnobPartialValues::default(),
                    bounds: CompileKnobPartialBounds::default(),
                    ..CompileKnobOverrides::default()
                },
                provenance: Vec::new(),
            },
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Default),
            provenance: PolicyProvenance {
                target_defaults: hash(0x31),
                profile_defaults: hash(0x32),
                compile_profile_spec_version: "2.0.0".to_owned(),
                hint_bundle_hash: None,
                compile_request_hash: hash(0x33),
                calibration_hash: None,
            },
        }
    }

    fn runtime_budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(0x21),
            rom_slots: Vec::new(),
            memory_caps: gbf_policy::RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x40),
            },
            wram_reserved: 128,
            sram_reserved: 512,
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn validate_mutated_schedule_cost_json(
        output: ScheduleCostOutput,
        mutate: impl FnOnce(&mut Value),
    ) -> Vec<ValidationDiagnostic> {
        let bytes = emit_schedule_cost_json_bytes(&output).expect("json bytes");
        let mut value: Value = serde_json::from_slice(&bytes).expect("json value");
        mutate(&mut value);
        let bytes = serde_json::to_vec(&value).expect("mutated json bytes");
        validate_schedule_cost_json_bytes(&bytes)
    }

    fn has_schedule_cost_code(
        diagnostics: &[ValidationDiagnostic],
        expected: ScheduleCostDiagnosticCode,
    ) -> bool {
        diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code,
                ValidationCode::ScheduleCost { code, .. } if code == expected
            )
        })
    }

    fn has_ref_kind(estimate: &CostEstimate, kind: &str) -> bool {
        estimate.refs.iter().any(|reference| reference.kind == kind)
    }

    fn axis_ref(estimate: &CostEstimate) -> Option<&EvidenceRef> {
        estimate
            .refs
            .iter()
            .find(|reference| reference.kind == "F-B14AxisEvidence")
    }
}
