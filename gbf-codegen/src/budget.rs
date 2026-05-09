//! Static budget taxonomy facade for Stage 2.
//!
//! This module wires the failure taxonomy, Stage 2 invocation input, and the
//! static budget report path used by F-B4.

use std::num::NonZeroU8;

use gbf_artifact::weight_plan::{ScaleFormat, ScaleGranularity, TernaryWeightPlan, WeightEncoding};
use gbf_foundation::{BudgetSlotId, ExpertId, FieldPath, Hash256, KernelSpecId, LayerId};
use gbf_hw::target::TargetProfile;
use gbf_policy::{
    BudgetSlotClass, PlacementProfile, ReductionSiteId, RomBudgetSlot, RuntimeChromeBudget,
    SwitchProjectionSource, ValidationDiagnostic,
};
use gbf_report::{
    ReportBody, ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report,
    canonicalize_value, compute_self_hash,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use gbf_policy::{
    BudgetFailure, PlacementInfeasibilityReason, ValidationCode, budget_failure_diagnostic,
    budget_failure_diagnostic_with_provenance, budget_failure_diagnostics,
    budget_failure_diagnostics_with_provenance, budget_failure_matches_diagnostic,
    budget_failure_validation_code,
};

use crate::policy::ResolvedPolicyProduct;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StaticPlacementModel {
    StrictOnePerBank,
    BudgetedFirstFit,
    PackedExpertsFirstFitDecreasing,
}

impl StaticPlacementModel {
    #[must_use]
    pub fn for_profile(profile: PlacementProfile) -> Self {
        match profile {
            PlacementProfile::StrictOnePerBank => Self::StrictOnePerBank,
            PlacementProfile::Budgeted => Self::BudgetedFirstFit,
            PlacementProfile::PackedExperts => Self::PackedExpertsFirstFitDecreasing,
        }
    }
}

#[must_use]
pub fn placement_model_for_profile(profile: PlacementProfile) -> StaticPlacementModel {
    StaticPlacementModel::for_profile(profile)
}

pub struct BudgetInputs<'a, Q: QuantGraphBudgetSource + ?Sized> {
    pub policy: &'a ResolvedPolicyProduct,
    pub quant_graph: &'a Q,
    pub runtime_chrome_budget: Option<&'a RuntimeChromeBudget>,
    pub target_profile: &'a TargetProfile,
}

pub trait QuantGraphBudgetSource {
    fn quant_graph_hash(&self) -> Hash256;
    fn semantic_core_hash(&self) -> Hash256;
    fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantGraphBudgetView {
    pub semantic_core_hash: Hash256,
    pub quant_graph_hash: Hash256,
    pub layers: Vec<LayerId>,
    pub experts: Vec<ExpertProjection>,
    pub shared_kernels: Vec<SharedKernelProjection>,
    pub shared_luts: Vec<SharedLutProjection>,
    pub reduction_sites: Vec<ReductionSiteProjection>,
    pub sequence_state: SequenceStateProjection,
    pub routing: RoutingProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertProjection {
    pub layer: LayerId,
    pub expert: ExpertId,
    pub rows: u32,
    pub cols: u32,
    pub metadata_bytes: u32,
    pub plan: TernaryWeightPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedKernelProjection {
    pub id: KernelSpecId,
    pub bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedLutProjection {
    pub id: String,
    pub bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReductionSiteProjection {
    pub site: ReductionSiteId,
    pub layer: Option<LayerId>,
    pub expert: Option<ExpertId>,
    pub term_count: u32,
    pub input_max_abs_q: u32,
    pub weight_max_abs_q: u32,
    pub bias_max_abs_q: Option<u32>,
    pub accumulator_domain: AccumulatorDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum AccumulatorDomain {
    RawIntegerProducts,
    PostScaleQ8_8,
    PostScaleQ16_16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceStateProjection {
    pub projected_wram_bytes: u32,
    pub projected_sram_bytes: u32,
    pub projected_hram_bytes: u32,
    pub projected_sram_page_switches_per_token: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingProjection {
    pub model: RoutingModelSection,
    pub projected_bank_switches_per_token: u16,
    pub expected_bank_switches_q16_16: Option<u32>,
}

impl Default for RoutingProjection {
    fn default() -> Self {
        Self {
            model: RoutingModelSection {
                kind: "synthetic-top1".to_owned(),
            },
            projected_bank_switches_per_token: 0,
            expected_bank_switches_q16_16: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantGraphBudgetViewError {
    Malformed { field: FieldPath },
}

pub type ByteBudget = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleFormatByteWidths {
    pow2_scale_bytes: NonZeroU8,
}

impl ScaleFormatByteWidths {
    #[must_use]
    pub const fn new(pow2_scale_bytes: NonZeroU8) -> Self {
        Self { pow2_scale_bytes }
    }

    #[must_use]
    pub fn for_target_profile(_target_profile: &TargetProfile) -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn pow2_scale_bytes(self) -> NonZeroU8 {
        self.pow2_scale_bytes
    }
}

impl Default for ScaleFormatByteWidths {
    fn default() -> Self {
        Self {
            pow2_scale_bytes: NonZeroU8::new(1).expect("default Pow2 scale width is non-zero"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetMathError {
    Overflow,
}

impl QuantGraphBudgetViewError {
    #[must_use]
    pub fn field_path(&self) -> &FieldPath {
        match self {
            Self::Malformed { field } => field,
        }
    }

    fn into_failure(self) -> BudgetFailure {
        match self {
            Self::Malformed { field } => BudgetFailure::QuantGraphBudgetViewMalformed { field },
        }
    }
}

impl std::fmt::Display for QuantGraphBudgetViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed { field } => write!(f, "malformed QuantGraph budget view at {field}"),
        }
    }
}

impl std::error::Error for QuantGraphBudgetViewError {}

impl QuantGraphBudgetView {
    pub fn validate_semantics(&self) -> Result<(), QuantGraphBudgetViewError> {
        ensure_sorted_by(
            self.layers.windows(2).all(|pair| pair[0] <= pair[1]),
            "budget_view.layers",
        )?;
        ensure_sorted_by(
            self.experts
                .windows(2)
                .all(|pair| (pair[0].layer, pair[0].expert) < (pair[1].layer, pair[1].expert)),
            "budget_view.experts",
        )?;
        ensure_sorted_by(
            self.shared_kernels
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id),
            "budget_view.shared_kernels",
        )?;
        ensure_sorted_by(
            self.shared_luts
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id),
            "budget_view.shared_luts",
        )?;
        ensure_sorted_by(
            self.reduction_sites
                .windows(2)
                .all(|pair| pair[0].site < pair[1].site),
            "budget_view.reduction_sites",
        )?;

        for (index, expert) in self.experts.iter().enumerate() {
            if !self.layers.contains(&expert.layer) {
                return Err(malformed_field(format!(
                    "budget_view.experts[{index}].layer"
                )));
            }
            if expert.rows == 0 {
                return Err(malformed_field(format!(
                    "budget_view.experts[{index}].rows"
                )));
            }
            if expert.cols == 0 {
                return Err(malformed_field(format!(
                    "budget_view.experts[{index}].cols"
                )));
            }
        }

        Ok(())
    }
}

fn ensure_sorted_by(
    is_sorted: bool,
    field: impl Into<String>,
) -> Result<(), QuantGraphBudgetViewError> {
    if is_sorted {
        Ok(())
    } else {
        Err(malformed_field(field))
    }
}

fn malformed_field(field: impl Into<String>) -> QuantGraphBudgetViewError {
    QuantGraphBudgetViewError::Malformed {
        field: FieldPath::from(field.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticBudgetReport {
    pub report: ReportEnvelope<StaticBudgetReportBody>,
    pub static_budget_self_hash: Hash256,
    pub static_budget_canonical_bytes_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticBudgetReportBody {
    pub identity: BudgetIdentitySection,
    pub policy: BudgetPolicySection,
    pub runtime_chrome_budget: Option<RuntimeChromeBudget>,
    pub projections: BudgetProjectionSection,
    pub decision: BudgetDecisionSection,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ReportBody for StaticBudgetReportBody {
    const REPORT_TYPE: &'static str = "StaticBudgetReport";
    const SCHEMA_ID: &'static str = "static_budget.v1";
    const SCHEMA_VERSION: &'static str = "1.0.0";

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let missing_runtime_chrome_budget_failure = BudgetFailure::MissingRuntimeChromeBudget;
        let has_exact_missing_budget_diagnostic = self.diagnostics.len() == 1
            && budget_failure_matches_diagnostic(
                &missing_runtime_chrome_budget_failure,
                &self.diagnostics[0],
            );
        let has_any_missing_budget_diagnostic = self.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code,
                ValidationCode::BudgetMissingRuntimeChromeBudget
            )
        });
        let has_exact_missing_budget_failure =
            self.decision.failures.as_slice() == [BudgetFailure::MissingRuntimeChromeBudget];
        let has_any_missing_budget_failure = self
            .decision
            .failures
            .iter()
            .any(|failure| matches!(failure, BudgetFailure::MissingRuntimeChromeBudget));
        let is_missing_budget_shape = self.identity.runtime_chrome_budget_hash.is_none()
            && self.runtime_chrome_budget.is_none();

        if is_missing_budget_shape {
            if outcome != ReportOutcome::Failed
                || !has_exact_missing_budget_diagnostic
                || !has_exact_missing_budget_failure
            {
                return Err(self.diagnostics.clone());
            }
        } else if has_any_missing_budget_diagnostic
            || has_any_missing_budget_failure
            || self.identity.runtime_chrome_budget_hash.is_none()
            || self.runtime_chrome_budget.is_none()
        {
            return Err(self.diagnostics.clone());
        }

        if let (Some(runtime_chrome_budget), Some(expected_runtime_chrome_budget_hash)) = (
            self.runtime_chrome_budget.as_ref(),
            self.identity.runtime_chrome_budget_hash,
        ) {
            let observed_hash = match runtime_chrome_budget_hash(runtime_chrome_budget) {
                Ok(hash) => hash,
                Err(_) => return Err(self.diagnostics.clone()),
            };
            if observed_hash != expected_runtime_chrome_budget_hash {
                return Err(self.diagnostics.clone());
            }
        }

        if self.decision.fits != self.decision.failures.is_empty() {
            return Err(self.diagnostics.clone());
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetIdentitySection {
    pub artifact_core_hash: Hash256,
    pub quant_graph_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Option<Hash256>,
    pub target_profile_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetPolicySection {
    pub placement_profile: PlacementProfile,
    pub objective_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetProjectionSection {
    pub per_expert_payload: Vec<PerExpertEntry>,
    pub per_bank_occupancy: Vec<PerBankEntry>,
    pub common_bank_footprint: CommonBankFootprintSection,
    pub accumulator_maxima: Vec<AccumulatorEntry>,
    pub projected_wram: ProjectedSizeSection,
    pub projected_sram: ProjectedSizeSection,
    pub projected_hram: ProjectedSizeSection,
    pub projected_bank_switches_per_token: ProjectedSwitchCountSection,
    pub projected_sram_page_switches_per_token: ProjectedSwitchCountSection,
    pub routing_model: RoutingModelSection,
}

impl Default for BudgetProjectionSection {
    fn default() -> Self {
        Self {
            per_expert_payload: Vec::new(),
            per_bank_occupancy: Vec::new(),
            common_bank_footprint: CommonBankFootprintSection::default(),
            accumulator_maxima: Vec::new(),
            projected_wram: ProjectedSizeSection::default(),
            projected_sram: ProjectedSizeSection::default(),
            projected_hram: ProjectedSizeSection::default(),
            projected_bank_switches_per_token: ProjectedSwitchCountSection::default(),
            projected_sram_page_switches_per_token: ProjectedSwitchCountSection::default(),
            routing_model: RoutingModelSection {
                kind: "not_evaluated_missing_runtime_chrome_budget".to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerExpertEntry {
    pub layer: LayerId,
    pub expert: ExpertId,
    pub payload_bytes: u32,
    pub assigned_slot: Option<BudgetSlotId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerBankEntry {
    pub slot: BudgetSlotId,
    pub class: BudgetSlotClass,
    pub usable_bytes: u32,
    pub reserved_slack: u16,
    pub effective_cap_bytes: i64,
    pub assigned_bytes: u32,
    pub assigned_components: Vec<BudgetComponentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BudgetComponentRef {
    Expert { layer: LayerId, expert: ExpertId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonBankFootprintSection {
    pub shared_kernel_bytes: u32,
    pub shared_lut_bytes: u32,
    pub shared_dense_ffn_bytes: Option<u32>,
    pub total_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccumulatorEntry {
    pub site: ReductionSiteId,
    pub projected_max_abs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedSizeSection {
    pub bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedSwitchCountSection {
    pub upper_bound: u16,
    pub expected_q16_16: Option<u32>,
    pub decision_value: u16,
    pub source: SwitchProjectionSource,
}

impl Default for ProjectedSwitchCountSection {
    fn default() -> Self {
        Self {
            upper_bound: 0,
            expected_q16_16: None,
            decision_value: 0,
            source: SwitchProjectionSource::ConservativeStaticUpperBound,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingModelSection {
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetDecisionSection {
    pub fits: bool,
    pub interpretation: StaticFitInterpretation,
    pub placement_model: StaticPlacementModel,
    pub failures: Vec<BudgetFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StaticFitInterpretation {
    PassesNecessaryStaticChecks,
    FailsNecessaryStaticChecks,
}

pub fn run_static_budget<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
) -> StaticBudgetReport {
    match inputs.runtime_chrome_budget {
        None => missing_runtime_chrome_budget_report(inputs),
        Some(runtime_chrome_budget) => match inputs.quant_graph.to_budget_view() {
            Ok(view) => match validate_budget_view(&inputs, &view) {
                Ok(()) => evaluated_budget_report(inputs, runtime_chrome_budget, view),
                Err(failure) => budget_failure_report(
                    inputs,
                    runtime_chrome_budget,
                    view.quant_graph_hash,
                    vec![failure],
                    BudgetProjectionSection::default(),
                ),
            },
            Err(error) => {
                let quant_graph_hash = inputs.quant_graph.quant_graph_hash();
                budget_failure_report(
                    inputs,
                    runtime_chrome_budget,
                    quant_graph_hash,
                    vec![error.into_failure()],
                    BudgetProjectionSection::default(),
                )
            }
        },
    }
}

pub fn static_budget_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
) -> StaticBudgetReport {
    run_static_budget(inputs)
}

pub fn produce_static_budget_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
) -> StaticBudgetReport {
    run_static_budget(inputs)
}

pub fn runtime_chrome_budget_hash(
    budget: &RuntimeChromeBudget,
) -> Result<Hash256, serde_json::Error> {
    let value = serde_json::to_value(budget)?;
    let canonical = canonicalize_value(&value).expect("runtime chrome budget canonicalizes");
    Ok(Hash256::from_bytes(Sha256::digest(canonical).into()))
}

#[must_use]
pub fn rom_slot_effective_cap_bytes(slot: &RomBudgetSlot) -> i64 {
    i64::from(slot.usable_bytes) - i64::from(slot.reserved_slack)
}

#[must_use]
pub fn validation_diagnostic_for_budget_failure(failure: &BudgetFailure) -> ValidationDiagnostic {
    budget_failure_diagnostic(failure)
}

#[must_use]
pub fn validation_diagnostics_for_budget_failures(
    failures: &[BudgetFailure],
) -> Vec<ValidationDiagnostic> {
    budget_failure_diagnostics(failures)
}

fn missing_runtime_chrome_budget_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
) -> StaticBudgetReport {
    let failures = vec![BudgetFailure::MissingRuntimeChromeBudget];
    let mut projections = BudgetProjectionSection::default();
    projections.routing_model.kind = "not_evaluated_missing_runtime_chrome_budget".to_owned();
    let body = StaticBudgetReportBody {
        identity: identity_section(inputs.policy, inputs.quant_graph.quant_graph_hash(), None),
        policy: policy_section(inputs.policy),
        runtime_chrome_budget: None,
        projections,
        decision: decision_section(
            false,
            inputs.policy.policy.knobs.global.placement.profile,
            failures.clone(),
        ),
        diagnostics: validation_diagnostics_for_budget_failures(&failures),
    };
    finalize_static_budget_report(ReportOutcome::Failed, body)
}

fn evaluated_budget_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
    runtime_chrome_budget: &RuntimeChromeBudget,
    view: QuantGraphBudgetView,
) -> StaticBudgetReport {
    let (projections, failures) = project_budget(
        inputs.policy.policy.knobs.global.placement.profile,
        runtime_chrome_budget,
        &view,
        ScaleFormatByteWidths::for_target_profile(inputs.target_profile),
    );
    if failures.is_empty() {
        let body = StaticBudgetReportBody {
            identity: identity_section(
                inputs.policy,
                view.quant_graph_hash,
                Some(runtime_chrome_budget_hash(runtime_chrome_budget).expect("budget hashes")),
            ),
            policy: policy_section(inputs.policy),
            runtime_chrome_budget: Some(runtime_chrome_budget.clone()),
            projections,
            decision: decision_section(
                true,
                inputs.policy.policy.knobs.global.placement.profile,
                Vec::new(),
            ),
            diagnostics: Vec::new(),
        };
        finalize_static_budget_report(ReportOutcome::Passed, body)
    } else {
        budget_failure_report(
            inputs,
            runtime_chrome_budget,
            view.quant_graph_hash,
            failures,
            projections,
        )
    }
}

fn budget_failure_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
    runtime_chrome_budget: &RuntimeChromeBudget,
    quant_graph_hash: Hash256,
    failures: Vec<BudgetFailure>,
    projections: BudgetProjectionSection,
) -> StaticBudgetReport {
    let body = StaticBudgetReportBody {
        identity: identity_section(
            inputs.policy,
            quant_graph_hash,
            Some(runtime_chrome_budget_hash(runtime_chrome_budget).expect("budget hashes")),
        ),
        policy: policy_section(inputs.policy),
        runtime_chrome_budget: Some(runtime_chrome_budget.clone()),
        projections,
        decision: decision_section(
            false,
            inputs.policy.policy.knobs.global.placement.profile,
            failures.clone(),
        ),
        diagnostics: validation_diagnostics_for_budget_failures(&failures),
    };
    finalize_static_budget_report(ReportOutcome::Failed, body)
}

fn identity_section(
    policy: &ResolvedPolicyProduct,
    quant_graph_hash: Hash256,
    runtime_chrome_budget_hash: Option<Hash256>,
) -> BudgetIdentitySection {
    BudgetIdentitySection {
        artifact_core_hash: policy.input_hashes.artifact_effective_core_hash,
        quant_graph_hash,
        policy_resolution_self_hash: policy.policy_resolution_self_hash,
        runtime_chrome_budget_hash,
        target_profile_hash: policy.input_hashes.target_profile_hash,
    }
}

fn policy_section(policy: &ResolvedPolicyProduct) -> BudgetPolicySection {
    BudgetPolicySection {
        placement_profile: policy.policy.knobs.global.placement.profile,
        objective_hash: hash_json_value(&policy.policy.objective).expect("objective hashes"),
    }
}

fn decision_section(
    fits: bool,
    profile: PlacementProfile,
    failures: Vec<BudgetFailure>,
) -> BudgetDecisionSection {
    BudgetDecisionSection {
        fits,
        interpretation: if fits {
            StaticFitInterpretation::PassesNecessaryStaticChecks
        } else {
            StaticFitInterpretation::FailsNecessaryStaticChecks
        },
        placement_model: placement_model_for_profile(profile),
        failures,
    }
}

fn finalize_static_budget_report(
    outcome: ReportOutcome,
    body: StaticBudgetReportBody,
) -> StaticBudgetReport {
    let mut report =
        ReportEnvelope::new(outcome, body).expect("static_budget.v1 schema constants are valid");
    report.report_self_hash =
        compute_self_hash(&report).expect("static budget report self-hash is computable");
    let canonical_bytes = canonicalize_report(&report).expect("static budget report canonicalizes");
    let static_budget_canonical_bytes_hash =
        Hash256::from_bytes(Sha256::digest(&canonical_bytes).into());
    StaticBudgetReport {
        static_budget_self_hash: report.report_self_hash,
        static_budget_canonical_bytes_hash,
        report,
    }
}

fn validate_budget_view<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: &BudgetInputs<'_, Q>,
    view: &QuantGraphBudgetView,
) -> Result<(), BudgetFailure> {
    let expected_semantic_core_hash = inputs.policy.input_hashes.artifact_effective_core_hash;
    if inputs.quant_graph.semantic_core_hash() != expected_semantic_core_hash {
        return Err(BudgetFailure::QuantGraphBudgetViewMalformed {
            field: FieldPath::from("quant_graph.semantic_core_hash"),
        });
    }
    if view.semantic_core_hash != expected_semantic_core_hash {
        return Err(BudgetFailure::QuantGraphBudgetViewMalformed {
            field: FieldPath::from("budget_view.semantic_core_hash"),
        });
    }

    view.validate_semantics()
        .map_err(QuantGraphBudgetViewError::into_failure)
}

fn project_budget(
    profile: PlacementProfile,
    runtime_chrome_budget: &RuntimeChromeBudget,
    view: &QuantGraphBudgetView,
    scale_format_byte_widths: ScaleFormatByteWidths,
) -> (BudgetProjectionSection, Vec<BudgetFailure>) {
    let mut per_bank_occupancy = runtime_chrome_budget
        .rom_slots
        .iter()
        .map(|slot| PerBankEntry {
            slot: slot.id,
            class: slot.class,
            usable_bytes: slot.usable_bytes,
            reserved_slack: slot.reserved_slack,
            effective_cap_bytes: rom_slot_effective_cap_bytes(slot),
            assigned_bytes: 0,
            assigned_components: Vec::new(),
        })
        .collect::<Vec<_>>();
    per_bank_occupancy.sort_by_key(|entry| entry.slot);

    let mut per_expert_payload = Vec::with_capacity(view.experts.len());
    let mut failures = Vec::new();

    for (expert_index, expert) in view.experts.iter().enumerate() {
        let payload_bytes = match expert_payload_bytes_with_widths(expert, scale_format_byte_widths)
            .and_then(u32_byte_budget)
        {
            Ok(payload_bytes) => payload_bytes,
            Err(error) => {
                failures.push(budget_math_error_failure(error, expert_index));
                continue;
            }
        };
        let slot_index = per_bank_occupancy.iter().position(|entry| {
            entry.class == BudgetSlotClass::ExpertBank
                && runtime_chrome_budget
                    .rom_slots
                    .iter()
                    .find(|slot| slot.id == entry.slot)
                    .is_some_and(|slot| slot.placement_caps.contains(&profile))
                && (profile != PlacementProfile::StrictOnePerBank
                    || entry.assigned_components.is_empty())
        });

        if let Some(slot_index) = slot_index {
            let slot = &mut per_bank_occupancy[slot_index];
            per_expert_payload.push(PerExpertEntry {
                layer: expert.layer,
                expert: expert.expert,
                payload_bytes,
                assigned_slot: Some(slot.slot),
            });
            slot.assigned_bytes = slot.assigned_bytes.saturating_add(payload_bytes);
            slot.assigned_components.push(BudgetComponentRef::Expert {
                layer: expert.layer,
                expert: expert.expert,
            });
            if i64::from(slot.assigned_bytes) > slot.effective_cap_bytes {
                let cap_bytes = u32::try_from(slot.effective_cap_bytes.max(0)).unwrap_or(u32::MAX);
                failures.push(BudgetFailure::ExpertExceedsSlot {
                    layer: expert.layer,
                    expert: expert.expert,
                    slot: slot.slot,
                    payload_bytes: slot.assigned_bytes,
                    cap_bytes,
                    excess_bytes: slot.assigned_bytes.saturating_sub(cap_bytes),
                });
            }
        } else {
            per_expert_payload.push(PerExpertEntry {
                layer: expert.layer,
                expert: expert.expert,
                payload_bytes,
                assigned_slot: None,
            });
            failures.push(BudgetFailure::PlacementProfileInfeasible {
                profile,
                reason: PlacementInfeasibilityReason::NoSlotsForClass,
            });
        }
    }

    let shared_kernel_bytes =
        checked_sum_u32(view.shared_kernels.iter().map(|kernel| kernel.bytes));
    let shared_lut_bytes = checked_sum_u32(view.shared_luts.iter().map(|lut| lut.bytes));
    let common_total = shared_kernel_bytes.saturating_add(shared_lut_bytes);

    (
        BudgetProjectionSection {
            per_expert_payload,
            per_bank_occupancy,
            common_bank_footprint: CommonBankFootprintSection {
                shared_kernel_bytes,
                shared_lut_bytes,
                shared_dense_ffn_bytes: None,
                total_bytes: common_total,
            },
            accumulator_maxima: accumulator_maxima(&view.reduction_sites),
            projected_wram: ProjectedSizeSection {
                bytes: view.sequence_state.projected_wram_bytes,
            },
            projected_sram: ProjectedSizeSection {
                bytes: view.sequence_state.projected_sram_bytes,
            },
            projected_hram: ProjectedSizeSection {
                bytes: view.sequence_state.projected_hram_bytes,
            },
            projected_bank_switches_per_token: ProjectedSwitchCountSection {
                upper_bound: view.routing.projected_bank_switches_per_token,
                expected_q16_16: view.routing.expected_bank_switches_q16_16,
                decision_value: view.routing.projected_bank_switches_per_token,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            projected_sram_page_switches_per_token: ProjectedSwitchCountSection {
                upper_bound: view.sequence_state.projected_sram_page_switches_per_token,
                expected_q16_16: None,
                decision_value: view.sequence_state.projected_sram_page_switches_per_token,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            routing_model: view.routing.model.clone(),
        },
        failures,
    )
}

pub fn expert_payload_bytes(expert: &ExpertProjection) -> Result<ByteBudget, BudgetMathError> {
    expert_payload_bytes_with_widths(expert, ScaleFormatByteWidths::default())
}

pub fn expert_payload_bytes_for_target(
    expert: &ExpertProjection,
    target_profile: &TargetProfile,
) -> Result<ByteBudget, BudgetMathError> {
    expert_payload_bytes_with_widths(
        expert,
        ScaleFormatByteWidths::for_target_profile(target_profile),
    )
}

fn expert_payload_bytes_with_widths(
    expert: &ExpertProjection,
    scale_format_byte_widths: ScaleFormatByteWidths,
) -> Result<ByteBudget, BudgetMathError> {
    checked_add_byte_budget(
        checked_add_byte_budget(
            weight_bytes(expert.rows, expert.cols, expert.plan.encoding)?,
            scale_bytes(
                expert.rows,
                expert.plan.scale_granularity,
                expert.plan.scale_format,
                scale_format_byte_widths,
            )?,
        )?,
        ByteBudget::from(expert.metadata_bytes),
    )
}

fn weight_bytes(
    rows: u32,
    cols: u32,
    encoding: WeightEncoding,
) -> Result<ByteBudget, BudgetMathError> {
    let weight_count = ByteBudget::from(rows)
        .checked_mul(ByteBudget::from(cols))
        .ok_or(BudgetMathError::Overflow)?;
    match encoding {
        WeightEncoding::Ternary2 => ceil_div_byte_budget(weight_count, 4),
        WeightEncoding::Binary1 => ceil_div_byte_budget(weight_count, 8),
        WeightEncoding::SparseTernaryBitplanes => {
            let bitplane_bytes = ceil_div_byte_budget(weight_count, 8)?;
            checked_add_byte_budget(bitplane_bytes, bitplane_bytes)
        }
    }
}

fn scale_bytes(
    rows: u32,
    granularity: ScaleGranularity,
    format: ScaleFormat,
    scale_format_byte_widths: ScaleFormatByteWidths,
) -> Result<ByteBudget, BudgetMathError> {
    let scale_count = match granularity {
        ScaleGranularity::PerTensor => 1,
        ScaleGranularity::PerOutputRow => ByteBudget::from(rows),
        ScaleGranularity::PerGroup(group_size) => {
            ceil_div_byte_budget(ByteBudget::from(rows), ByteBudget::from(group_size.get()))?
        }
    };
    scale_count
        .checked_mul(ByteBudget::from(scale_format_bytes(
            format,
            scale_format_byte_widths,
        )))
        .ok_or(BudgetMathError::Overflow)
}

const fn scale_format_bytes(
    format: ScaleFormat,
    scale_format_byte_widths: ScaleFormatByteWidths,
) -> u8 {
    match format {
        ScaleFormat::Q8_8 => 2,
        ScaleFormat::Q4_4 => 1,
        ScaleFormat::Pow2 => scale_format_byte_widths.pow2_scale_bytes().get(),
    }
}

fn ceil_div_byte_budget(
    numerator: ByteBudget,
    denominator: ByteBudget,
) -> Result<ByteBudget, BudgetMathError> {
    Ok(numerator.div_ceil(denominator))
}

fn checked_add_byte_budget(
    left: ByteBudget,
    right: ByteBudget,
) -> Result<ByteBudget, BudgetMathError> {
    left.checked_add(right).ok_or(BudgetMathError::Overflow)
}

fn u32_byte_budget(value: ByteBudget) -> Result<u32, BudgetMathError> {
    u32::try_from(value).map_err(|_| BudgetMathError::Overflow)
}

fn budget_math_error_failure(error: BudgetMathError, expert_index: usize) -> BudgetFailure {
    match error {
        BudgetMathError::Overflow => BudgetFailure::QuantGraphBudgetViewMalformed {
            field: FieldPath::from(format!("budget_view.experts[{expert_index}].payload_bytes")),
        },
    }
}

fn checked_sum_u32(values: impl Iterator<Item = u32>) -> u32 {
    values.fold(0u32, u32::saturating_add)
}

fn accumulator_maxima(reduction_sites: &[ReductionSiteProjection]) -> Vec<AccumulatorEntry> {
    reduction_sites
        .iter()
        .map(|site| {
            let bias = u64::from(site.bias_max_abs_q.unwrap_or(0));
            let product = u64::from(site.term_count)
                .saturating_mul(u64::from(site.input_max_abs_q))
                .saturating_mul(u64::from(site.weight_max_abs_q));
            AccumulatorEntry {
                site: site.site.clone(),
                projected_max_abs: product.saturating_add(bias),
            }
        })
        .collect()
}

fn hash_json_value<T: Serialize + ?Sized>(value: &T) -> Result<Hash256, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    let canonical = canonicalize_value(&value).expect("value canonicalizes");
    Ok(Hash256::from_bytes(Sha256::digest(canonical).into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeSet;

    use gbf_artifact::weight_plan::{ScaleFormat, ScaleGranularity, ThresholdPlan, WeightEncoding};
    use gbf_foundation::{LineageId, TargetProfileId};
    use gbf_hw::calibration::CalibrationSetRef;
    use gbf_hw::target::dmg_mbc5_8mib_128kib;
    use gbf_policy::DiagnosticSeverity;
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_TOML, CompileKnobOverrides, CompileKnobPartialBounds,
        CompileKnobPartialValues, CompileKnobProvenanceEntry, CompileKnobValues, CompilerFeature,
        ConstraintOperation, ConstraintProvenance, EffectiveConstraints, EvidenceRef, KnobLockSet,
        ObservabilityMode, PolicyProvenance, PolicySource, RepairPolicy, RepairPolicyProfile,
        RiskPolicy, RuntimeMemoryCapSection, RuntimeMode, TraceBudget, TraceDropPolicy,
        ValidationDetail, ValidationOrigin, canonical_default_bounds_fixture,
        load_compile_profile_spec,
    };
    use gbf_policy::{CalibrationConfidenceRequirement, CompileObjective, ServiceLevelObjective};
    use gbf_report::ReportOutcome;
    use gbf_report::report_schemas::policy_resolution_v1::{
        ArtifactIdentitySection, CompileRequestSection, HintConsumptionSection,
        PolicyResolutionReportBody,
    };

    use crate::policy::ResolvedPolicyProduct;
    use crate::validate::ValidatedInputHashes;

    fn round_trip_failure(failure: BudgetFailure) {
        let encoded = serde_json::to_string(&failure).expect("budget failure serializes");
        let decoded: BudgetFailure =
            serde_json::from_str(&encoded).expect("budget failure deserializes");

        assert_eq!(decoded, failure);
    }

    fn all_failure_variants() -> Vec<BudgetFailure> {
        vec![
            BudgetFailure::MissingRuntimeChromeBudget,
            BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.routing"),
            },
            BudgetFailure::ExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                slot: BudgetSlotId::new(2),
                payload_bytes: 17_000,
                cap_bytes: 16_128,
                excess_bytes: 872,
            },
            BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
                excess_bytes: 3_616,
            },
            BudgetFailure::WramPeakExceedsCap {
                peak: 8_300,
                cap: 8_192,
            },
            BudgetFailure::SramPeakExceedsCap {
                peak: 33_000,
                cap: 32_768,
            },
            BudgetFailure::HramPeakExceedsCap {
                peak: 144,
                cap: 127,
            },
            BudgetFailure::AccumulatorExceedsI32 {
                site: ReductionSiteId("ffn.0.acc".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            },
            BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value: 9,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            BudgetFailure::SramPageSwitchesPerTokenOverCap {
                decision_value: 4,
                upper_bound: 4,
                cap: 2,
                source: SwitchProjectionSource::HintWeightedExpectedWithStaticCap,
            },
            BudgetFailure::PlacementProfileInfeasible {
                profile: PlacementProfile::Budgeted,
                reason: PlacementInfeasibilityReason::NoSlotsForClass,
            },
        ]
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn objective_fixture() -> CompileObjective {
        CompileObjective {
            service: Some(ServiceLevelObjective {
                max_first_token_cycles_p95: Some(21_000),
                max_checkpoint_gap_cycles_p95: Some(13_000),
                max_resume_latency_cycles_p95: Some(8_000),
                max_ui_jitter_frames_p99: Some(2),
            }),
            max_cycles_per_token: Some(24_000),
            max_bank_switches_per_token: Some(17),
            max_sram_page_switches_per_token: Some(3),
            min_ui_headroom_pct: 11,
            max_rom_bytes: Some(2 * 1024 * 1024),
            risk: RiskPolicy {
                cycle_quantile: 90,
                switch_quantile: 95,
                calibration_confidence_requirement:
                    CalibrationConfidenceRequirement::NoMinimumConfidence,
                fallback_profile: None,
                fallback_runtime_mode: None,
            },
        }
    }

    fn compile_values_from_bringup_profile() -> CompileKnobValues {
        let profile = load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML)
            .expect("bringup profile parses");
        CompileKnobValues {
            placement: profile.knob_defaults.placement.expect("placement default"),
            observation: profile
                .knob_defaults
                .observation
                .expect("observation default"),
            range: profile.knob_defaults.range.expect("range default"),
            storage: profile.knob_defaults.storage.expect("storage default"),
            sram: profile.knob_defaults.sram.expect("sram default"),
            rom_window: profile
                .knob_defaults
                .rom_window
                .expect("rom window default"),
            overlay: profile.knob_defaults.overlay.expect("overlay default"),
            schedule: profile.knob_defaults.schedule.expect("schedule default"),
        }
    }

    fn policy_fixture() -> ResolvedPolicyProduct {
        let objective = objective_fixture();
        let values = compile_values_from_bringup_profile();
        let input_hashes = ValidatedInputHashes {
            artifact_source_hash: hash(0x01),
            artifact_effective_core_hash: hash(0x02),
            artifact_manifest_hash: hash(0x03),
            artifact_aux_hash: hash(0x04),
            lowering_manifest_hash: hash(0x05),
            hint_bundle_hash: hash(0x06),
            compile_request_hash: hash(0x07),
            target_profile_hash: hash(0x08),
            compile_profile_hash: hash(0x09),
            calibration_hash: hash(0x0a),
            compatibility_adapter_hash: None,
        };
        let policy = gbf_policy::ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: gbf_foundation::CompileProfileId::from("Bringup"),
            objective: objective.clone(),
            effective_constraints: EffectiveConstraints {
                target_caps: canonical_default_bounds_fixture(),
                required_features: BTreeSet::from([CompilerFeature::StaticBudgetReport]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 64,
                max_bytes_per_frame: 2048,
                drop_policy: TraceDropPolicy::DropOldest,
            },
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            knobs: gbf_policy::CompileKnobs {
                global: values,
                bounds: canonical_default_bounds_fixture(),
                locks: KnobLockSet::default(),
                overrides: CompileKnobOverrides {
                    values: CompileKnobPartialValues::default(),
                    bounds: CompileKnobPartialBounds::default(),
                },
                provenance: vec![CompileKnobProvenanceEntry {
                    path: gbf_policy::CompileKnobPath {
                        knob: gbf_policy::CompileKnobId::Placement,
                        selector: None,
                        field: Some(FieldPath::from("profile")),
                    },
                    chain: vec![ConstraintProvenance {
                        source: PolicySource::ProfileDefault,
                        operation: ConstraintOperation::SeedDefault,
                        evidence: Vec::new(),
                    }],
                }],
            },
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            provenance: PolicyProvenance {
                target_defaults: input_hashes.target_profile_hash,
                profile_defaults: input_hashes.compile_profile_hash,
                hint_bundle_hash: Some(input_hashes.hint_bundle_hash),
                compile_request_hash: input_hashes.compile_request_hash,
                calibration_hash: Some(input_hashes.calibration_hash),
            },
        };
        let report = ReportEnvelope::new(
            ReportOutcome::Failed,
            PolicyResolutionReportBody {
                artifact_identity: ArtifactIdentitySection {
                    artifact_core_hash: input_hashes.artifact_effective_core_hash,
                    artifact_manifest_hash: input_hashes.artifact_manifest_hash,
                    semantic_lineage: LineageId(hash(0x30)),
                    lowering_manifest_hash: input_hashes.lowering_manifest_hash,
                    hint_bundle_hash: input_hashes.hint_bundle_hash,
                    workload_refs: Vec::new(),
                    golden_vector_refs: Vec::new(),
                },
                compile_request: CompileRequestSection {
                    compile_request_hash: input_hashes.compile_request_hash,
                    target: policy.target.clone(),
                    target_profile_hash: input_hashes.target_profile_hash,
                    profile: policy.profile.clone(),
                    objective,
                    required_features: BTreeSet::from([CompilerFeature::StaticBudgetReport]),
                    requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                    calibration_set_ref: CalibrationSetRef {
                        platform: None,
                        kernel: None,
                        runtime: None,
                    },
                    calibration_hash: input_hashes.calibration_hash,
                },
                result: None,
                hint_consumption: HintConsumptionSection::default(),
                diagnostics: vec![budget_failure_diagnostic(
                    &BudgetFailure::MissingRuntimeChromeBudget,
                )],
            },
        )
        .expect("policy report envelope");
        ResolvedPolicyProduct {
            policy,
            input_hashes,
            artifact_validation_self_hash: hash(0x0b),
            report,
            policy_resolution_self_hash: hash(0x0c),
            policy_resolution_canonical_bytes_hash: hash(0x0d),
        }
    }

    fn runtime_budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: gbf_foundation::CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(0x40),
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(1),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 1024,
                reserved_slack: 128,
                placement_caps: BTreeSet::from([PlacementProfile::StrictOnePerBank]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x08),
            },
            wram_reserved: 0,
            sram_reserved: 0,
        }
    }

    fn ternary_plan() -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        )
    }

    fn plan_with(
        encoding: WeightEncoding,
        scale_granularity: ScaleGranularity,
        scale_format: ScaleFormat,
    ) -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            encoding,
            scale_granularity,
            scale_format,
            ThresholdPlan::FixedQ8_8,
        )
    }

    fn expert_with_plan(
        rows: u32,
        cols: u32,
        metadata_bytes: u32,
        plan: TernaryWeightPlan,
    ) -> ExpertProjection {
        ExpertProjection {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            rows,
            cols,
            metadata_bytes,
            plan,
        }
    }

    fn budget_view_fixture(rows: u32, cols: u32, metadata_bytes: u32) -> QuantGraphBudgetView {
        QuantGraphBudgetView {
            semantic_core_hash: hash(0x02),
            quant_graph_hash: hash(0x23),
            layers: vec![LayerId::new(0)],
            experts: vec![ExpertProjection {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                rows,
                cols,
                metadata_bytes,
                plan: ternary_plan(),
            }],
            shared_kernels: Vec::new(),
            shared_luts: Vec::new(),
            reduction_sites: Vec::new(),
            sequence_state: SequenceStateProjection::default(),
            routing: RoutingProjection::default(),
        }
    }

    #[test]
    fn f_b4_budget_computes_ternary2_payload_bytes_from_shape() {
        let expert = expert_with_plan(
            3,
            5,
            7,
            plan_with(
                WeightEncoding::Ternary2,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
        );

        assert_eq!(expert_payload_bytes(&expert), Ok(13));
    }

    #[test]
    fn f_b4_budget_computes_scale_bytes_by_granularity() {
        let per_tensor = expert_with_plan(
            10,
            7,
            0,
            plan_with(
                WeightEncoding::Binary1,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
        );
        let per_output_row = expert_with_plan(
            10,
            7,
            0,
            plan_with(
                WeightEncoding::Binary1,
                ScaleGranularity::PerOutputRow,
                ScaleFormat::Q4_4,
            ),
        );
        let per_group = expert_with_plan(
            10,
            7,
            0,
            plan_with(
                WeightEncoding::Binary1,
                ScaleGranularity::per_group(4).unwrap(),
                ScaleFormat::Pow2,
            ),
        );

        assert_eq!(expert_payload_bytes(&per_tensor), Ok(11));
        assert_eq!(expert_payload_bytes(&per_output_row), Ok(19));
        assert_eq!(expert_payload_bytes(&per_group), Ok(12));
    }

    #[test]
    fn f_b4_budget_applies_non_default_pow2_scale_width_from_project_budget_path() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(10, 7, 0);
        view.experts[0].plan = plan_with(
            WeightEncoding::Binary1,
            ScaleGranularity::per_group(4).unwrap(),
            ScaleFormat::Pow2,
        );

        let (projections, failures) = project_budget(
            PlacementProfile::StrictOnePerBank,
            &budget,
            &view,
            ScaleFormatByteWidths::new(NonZeroU8::new(3).unwrap()),
        );

        assert!(failures.is_empty());
        assert_eq!(projections.per_expert_payload[0].payload_bytes, 18);
        assert_eq!(
            expert_payload_bytes_with_widths(
                &view.experts[0],
                ScaleFormatByteWidths::new(NonZeroU8::new(3).unwrap())
            ),
            Ok(18)
        );
    }

    #[test]
    fn f_b4_budget_expert_payload_bytes_pins_bd_w80_matrix_orientation() {
        let per_tensor = plan_with(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        );
        let per_output_row = plan_with(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
        );
        let per_group = plan_with(
            WeightEncoding::Ternary2,
            ScaleGranularity::per_group(16).unwrap(),
            ScaleFormat::Q8_8,
        );

        assert_eq!(
            expert_payload_bytes(&expert_with_plan(128, 224, 0, per_tensor)),
            Ok(7_170)
        );
        assert_eq!(
            expert_payload_bytes(&expert_with_plan(128, 224, 0, per_output_row)),
            Ok(7_424)
        );
        assert_eq!(
            expert_payload_bytes(&expert_with_plan(224, 128, 0, per_output_row)),
            Ok(7_616)
        );
        assert_eq!(
            expert_payload_bytes(&expert_with_plan(128, 224, 0, per_group)),
            Ok(7_184)
        );

        let full_two_matrix_expert =
            expert_payload_bytes(&expert_with_plan(224, 128, 0, per_output_row)).unwrap()
                + expert_payload_bytes(&expert_with_plan(128, 224, 0, per_output_row)).unwrap();
        assert_eq!(full_two_matrix_expert, 15_040);
    }

    #[test]
    fn f_b4_budget_computes_sparse_bitplane_payload_without_plan_metadata_layout() {
        let expert = expert_with_plan(
            3,
            5,
            3,
            plan_with(
                WeightEncoding::SparseTernaryBitplanes,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q4_4,
            ),
        );

        // Current TernaryWeightPlan has no sparse metadata layout field; bd-nyen
        // owns richer non-Ternary2 artifact/export layout support.
        assert_eq!(expert_payload_bytes(&expert), Ok(8));
    }

    #[test]
    fn f_b4_budget_rejects_overflow_in_byte_math() {
        assert_eq!(
            checked_add_byte_budget(ByteBudget::MAX, 1),
            Err(BudgetMathError::Overflow)
        );

        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(u32::MAX, u32::MAX, u32::MAX);
        view.experts[0].plan = plan_with(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        );
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.experts[0].payload_bytes")
            }]
        );
        assert!(report.report.body.projections.per_expert_payload.is_empty());
    }

    #[test]
    fn f_b4_budget_validates_quant_graph_budget_view_ordering() {
        let mut view = budget_view_fixture(4, 4, 0);
        view.layers = vec![LayerId::new(1), LayerId::new(0)];
        assert_eq!(
            view.validate_semantics().expect_err("layers are unsorted"),
            QuantGraphBudgetViewError::Malformed {
                field: FieldPath::from("budget_view.layers")
            }
        );

        let mut view = budget_view_fixture(4, 4, 0);
        view.layers = vec![LayerId::new(0), LayerId::new(1)];
        view.experts[0].layer = LayerId::new(1);
        view.experts.push(ExpertProjection {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            rows: 4,
            cols: 4,
            metadata_bytes: 0,
            plan: ternary_plan(),
        });
        assert_eq!(
            view.validate_semantics().expect_err("experts are unsorted"),
            QuantGraphBudgetViewError::Malformed {
                field: FieldPath::from("budget_view.experts")
            }
        );

        let mut view = budget_view_fixture(4, 4, 0);
        view.shared_kernels = vec![
            SharedKernelProjection {
                id: KernelSpecId::from("kernel.z"),
                bytes: 8,
            },
            SharedKernelProjection {
                id: KernelSpecId::from("kernel.a"),
                bytes: 4,
            },
        ];
        assert_eq!(
            view.validate_semantics()
                .expect_err("shared kernels are unsorted"),
            QuantGraphBudgetViewError::Malformed {
                field: FieldPath::from("budget_view.shared_kernels")
            }
        );

        let mut view = budget_view_fixture(4, 4, 0);
        view.shared_luts = vec![
            SharedLutProjection {
                id: "lut.z".to_owned(),
                bytes: 8,
            },
            SharedLutProjection {
                id: "lut.a".to_owned(),
                bytes: 4,
            },
        ];
        assert_eq!(
            view.validate_semantics()
                .expect_err("shared LUTs are unsorted"),
            QuantGraphBudgetViewError::Malformed {
                field: FieldPath::from("budget_view.shared_luts")
            }
        );

        let mut view = budget_view_fixture(4, 4, 0);
        view.reduction_sites = vec![
            ReductionSiteProjection {
                site: ReductionSiteId("site.z".to_owned()),
                layer: Some(LayerId::new(0)),
                expert: Some(ExpertId::new(0)),
                term_count: 4,
                input_max_abs_q: 2,
                weight_max_abs_q: 3,
                bias_max_abs_q: Some(1),
                accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            },
            ReductionSiteProjection {
                site: ReductionSiteId("site.a".to_owned()),
                layer: Some(LayerId::new(0)),
                expert: Some(ExpertId::new(0)),
                term_count: 4,
                input_max_abs_q: 2,
                weight_max_abs_q: 3,
                bias_max_abs_q: Some(1),
                accumulator_domain: AccumulatorDomain::RawIntegerProducts,
            },
        ];
        assert_eq!(
            view.validate_semantics()
                .expect_err("reduction sites are unsorted"),
            QuantGraphBudgetViewError::Malformed {
                field: FieldPath::from("budget_view.reduction_sites")
            }
        );
    }

    #[test]
    fn f_b4_budget_rejects_duplicate_quant_graph_budget_view_keys() {
        let duplicate_expert = {
            let mut view = budget_view_fixture(4, 4, 0);
            view.experts.push(view.experts[0].clone());
            view
        };
        let duplicate_shared_kernel = {
            let mut view = budget_view_fixture(4, 4, 0);
            view.shared_kernels = vec![
                SharedKernelProjection {
                    id: KernelSpecId::from("kernel.shared"),
                    bytes: 8,
                },
                SharedKernelProjection {
                    id: KernelSpecId::from("kernel.shared"),
                    bytes: 4,
                },
            ];
            view
        };
        let duplicate_shared_lut = {
            let mut view = budget_view_fixture(4, 4, 0);
            view.shared_luts = vec![
                SharedLutProjection {
                    id: "lut.shared".to_owned(),
                    bytes: 8,
                },
                SharedLutProjection {
                    id: "lut.shared".to_owned(),
                    bytes: 4,
                },
            ];
            view
        };
        let duplicate_reduction_site = {
            let mut view = budget_view_fixture(4, 4, 0);
            view.reduction_sites = vec![
                ReductionSiteProjection {
                    site: ReductionSiteId("site.shared".to_owned()),
                    layer: Some(LayerId::new(0)),
                    expert: Some(ExpertId::new(0)),
                    term_count: 4,
                    input_max_abs_q: 2,
                    weight_max_abs_q: 3,
                    bias_max_abs_q: Some(1),
                    accumulator_domain: AccumulatorDomain::RawIntegerProducts,
                },
                ReductionSiteProjection {
                    site: ReductionSiteId("site.shared".to_owned()),
                    layer: Some(LayerId::new(0)),
                    expert: Some(ExpertId::new(0)),
                    term_count: 8,
                    input_max_abs_q: 2,
                    weight_max_abs_q: 3,
                    bias_max_abs_q: Some(1),
                    accumulator_domain: AccumulatorDomain::RawIntegerProducts,
                },
            ];
            view
        };

        for (view, expected_field) in [
            (duplicate_expert, "budget_view.experts"),
            (duplicate_shared_kernel, "budget_view.shared_kernels"),
            (duplicate_shared_lut, "budget_view.shared_luts"),
            (duplicate_reduction_site, "budget_view.reduction_sites"),
        ] {
            assert_eq!(
                view.validate_semantics()
                    .expect_err("duplicate keyed projection is malformed"),
                QuantGraphBudgetViewError::Malformed {
                    field: FieldPath::from(expected_field)
                }
            );
        }
    }

    #[test]
    fn quant_graph_budget_view_round_trips_canonically() {
        let mut view = budget_view_fixture(4, 8, 12);
        view.layers = vec![LayerId::new(0), LayerId::new(1)];
        view.experts.push(ExpertProjection {
            layer: LayerId::new(1),
            expert: ExpertId::new(0),
            rows: 8,
            cols: 4,
            metadata_bytes: 16,
            plan: ternary_plan(),
        });
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.shared.ffn"),
            bytes: 128,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.affine_clip.0".to_owned(),
            bytes: 64,
        }];
        view.reduction_sites = vec![ReductionSiteProjection {
            site: ReductionSiteId("ffn.0.acc".to_owned()),
            layer: Some(LayerId::new(0)),
            expert: Some(ExpertId::new(0)),
            term_count: 16,
            input_max_abs_q: 7,
            weight_max_abs_q: 5,
            bias_max_abs_q: Some(3),
            accumulator_domain: AccumulatorDomain::PostScaleQ8_8,
        }];

        view.validate_semantics().expect("fixture view is valid");
        let first_value = serde_json::to_value(&view).expect("view serializes");
        let first_canonical =
            canonicalize_value(&first_value).expect("view canonicalizes before round trip");
        let decoded: QuantGraphBudgetView =
            serde_json::from_slice(&first_canonical).expect("canonical view deserializes");
        decoded
            .validate_semantics()
            .expect("decoded view remains valid");
        let second_value = serde_json::to_value(&decoded).expect("decoded view serializes");
        let second_canonical =
            canonicalize_value(&second_value).expect("view canonicalizes after round trip");

        assert_eq!(decoded, view);
        assert_eq!(second_canonical, first_canonical);
    }

    struct TrackingQuantGraph {
        view: QuantGraphBudgetView,
        calls: Cell<u32>,
    }

    impl TrackingQuantGraph {
        fn new(view: QuantGraphBudgetView) -> Self {
            Self {
                view,
                calls: Cell::new(0),
            }
        }
    }

    impl QuantGraphBudgetSource for TrackingQuantGraph {
        fn quant_graph_hash(&self) -> Hash256 {
            self.view.quant_graph_hash
        }

        fn semantic_core_hash(&self) -> Hash256 {
            self.view.semantic_core_hash
        }

        fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.view.clone())
        }
    }

    struct ErrorQuantGraph {
        quant_graph_hash: Hash256,
        semantic_core_hash: Hash256,
        error: QuantGraphBudgetViewError,
    }

    impl QuantGraphBudgetSource for ErrorQuantGraph {
        fn quant_graph_hash(&self) -> Hash256 {
            self.quant_graph_hash
        }

        fn semantic_core_hash(&self) -> Hash256 {
            self.semantic_core_hash
        }

        fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
            Err(self.error.clone())
        }
    }

    fn run_budget_with(
        runtime_chrome_budget: Option<&RuntimeChromeBudget>,
        quant_graph: &TrackingQuantGraph,
    ) -> StaticBudgetReport {
        let policy = policy_fixture();
        run_static_budget(BudgetInputs {
            policy: &policy,
            quant_graph,
            runtime_chrome_budget,
            target_profile: &dmg_mbc5_8mib_128kib(),
        })
    }

    #[test]
    fn f_b4_budget_rejects_missing_runtime_chrome_budget() {
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(None, &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert!(!report.report.body.decision.fits);
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetMissingRuntimeChromeBudget
        );
    }

    #[test]
    fn f_b4_budget_missing_runtime_chrome_budget_records_budget_failure() {
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(None, &quant_graph);

        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::MissingRuntimeChromeBudget]
        );
        assert_eq!(
            report.report.body.decision.fits,
            report.report.body.decision.failures.is_empty()
        );
    }

    #[test]
    fn f_b4_budget_missing_runtime_chrome_budget_emits_failure_report_without_budget_hash() {
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(None, &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(report.report.body.identity.runtime_chrome_budget_hash, None);
        assert_eq!(report.report.body.runtime_chrome_budget, None);
        assert!(report.report.body.projections.per_bank_occupancy.is_empty());
        assert_eq!(report.report.body.diagnostics.len(), 1);
        assert_eq!(report.report.body.decision.failures.len(), 1);
        assert!(
            report
                .report
                .body
                .validate_semantics(report.report.outcome)
                .is_ok()
        );
    }

    #[test]
    fn f_b4_budget_missing_runtime_chrome_budget_semantics_reject_extra_shape() {
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));
        let report = run_budget_with(None, &quant_graph);

        let mut extra_diagnostic = report.report.body.clone();
        extra_diagnostic.diagnostics.push(budget_failure_diagnostic(
            &BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.experts"),
            },
        ));
        assert!(
            extra_diagnostic
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );

        let mut extra_failure = report.report.body.clone();
        extra_failure
            .decision
            .failures
            .push(BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.experts"),
            });
        assert!(
            extra_failure
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );

        let mut soft_diagnostic = report.report.body.clone();
        soft_diagnostic.diagnostics[0].severity = DiagnosticSeverity::Soft;
        assert!(
            soft_diagnostic
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );

        let mut wrong_origin = report.report.body.clone();
        wrong_origin.diagnostics[0].origin = ValidationOrigin::SemanticCore;
        assert!(
            wrong_origin
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );
    }

    #[test]
    fn f_b4_budget_missing_runtime_chrome_budget_does_not_call_to_budget_view() {
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let _report = run_budget_with(None, &quant_graph);

        assert_eq!(quant_graph.calls.get(), 0);
    }

    #[test]
    fn f_b4_budget_uses_quant_graph_budget_source_trait_stub_until_f_b3() {
        let budget = runtime_budget_fixture();
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(quant_graph.calls.get(), 1);
        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(report.report.body.identity.quant_graph_hash, hash(0x23));
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].layer,
            LayerId::new(0)
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].expert,
            ExpertId::new(0)
        );
    }

    #[test]
    fn f_b4_budget_runtime_chrome_budget_excerpt_hash_matches_input() {
        let budget = runtime_budget_fixture();
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(quant_graph.calls.get(), 1);
        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report.report.body.runtime_chrome_budget,
            Some(budget.clone())
        );
        assert_eq!(
            report.report.body.identity.runtime_chrome_budget_hash,
            Some(runtime_chrome_budget_hash(&budget).expect("budget hashes"))
        );
        assert_eq!(report.report.body.identity.artifact_core_hash, hash(0x02));
        assert_eq!(budget.rom_slots[0].reserved_slack, 128);
    }

    #[test]
    fn f_b4_budget_runtime_chrome_budget_excerpt_hash_mismatch_rejected() {
        let budget = runtime_budget_fixture();
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(Some(&budget), &quant_graph);
        let mut body = report.report.body.clone();
        body.identity.runtime_chrome_budget_hash = Some(hash(0xee));

        assert!(body.validate_semantics(report.report.outcome).is_err());
    }

    #[test]
    fn f_b4_budget_missing_runtime_chrome_budget_markers_rejected_with_populated_budget() {
        let budget = runtime_budget_fixture();
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));
        let report = run_budget_with(Some(&budget), &quant_graph);

        let mut missing_diagnostic = report.report.body.clone();
        missing_diagnostic.diagnostics = vec![budget_failure_diagnostic(
            &BudgetFailure::MissingRuntimeChromeBudget,
        )];
        assert!(
            missing_diagnostic
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );

        let mut missing_failure = report.report.body.clone();
        missing_failure.decision.fits = false;
        missing_failure.decision.interpretation =
            StaticFitInterpretation::FailsNecessaryStaticChecks;
        missing_failure.decision.failures = vec![BudgetFailure::MissingRuntimeChromeBudget];
        assert!(
            missing_failure
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );
    }

    #[test]
    fn f_b4_budget_rejects_quant_graph_semantic_hash_mismatch() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.semantic_core_hash = hash(0xee);
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(quant_graph.calls.get(), 1);
        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(report.report.body.identity.artifact_core_hash, hash(0x02));
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("quant_graph.semantic_core_hash")
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("quant_graph.semantic_core_hash")
            }
        );
    }

    #[test]
    fn f_b4_budget_quant_graph_view_error_maps_to_typed_field_path() {
        let budget = runtime_budget_fixture();
        let error = QuantGraphBudgetViewError::Malformed {
            field: FieldPath::from("budget_view.shared_kernels"),
        };
        assert_eq!(
            error.field_path(),
            &FieldPath::from("budget_view.shared_kernels")
        );
        let quant_graph = ErrorQuantGraph {
            quant_graph_hash: hash(0x23),
            semantic_core_hash: hash(0x02),
            error,
        };
        let policy = policy_fixture();

        let report = run_static_budget(BudgetInputs {
            policy: &policy,
            quant_graph: &quant_graph,
            runtime_chrome_budget: Some(&budget),
            target_profile: &dmg_mbc5_8mib_128kib(),
        });

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.shared_kernels")
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("budget_view.shared_kernels")
            }
        );

        let mut invalid_view = budget_view_fixture(4, 4, 0);
        invalid_view.experts[0].layer = LayerId::new(9);
        let quant_graph = TrackingQuantGraph::new(invalid_view);
        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("budget_view.experts[0].layer")
            }
        );
    }

    #[test]
    fn f_b4_budget_uses_reserved_slack_in_effective_cap() {
        let mut budget = runtime_budget_fixture();
        budget.rom_slots[0].usable_bytes = 8;
        budget.rom_slots[0].reserved_slack = 16;
        let quant_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].effective_cap_bytes,
            -8
        );
        assert!(matches!(
            report.report.body.decision.failures[0],
            BudgetFailure::ExpertExceedsSlot { .. }
        ));
        assert_eq!(budget.rom_slots[0].usable_bytes, 8);
        assert_eq!(budget.rom_slots[0].reserved_slack, 16);
    }

    #[test]
    fn f_b4_budget_records_static_placement_model() {
        assert_eq!(
            placement_model_for_profile(PlacementProfile::StrictOnePerBank),
            StaticPlacementModel::StrictOnePerBank
        );
        assert_eq!(
            placement_model_for_profile(PlacementProfile::Budgeted),
            StaticPlacementModel::BudgetedFirstFit
        );
        assert_eq!(
            placement_model_for_profile(PlacementProfile::PackedExperts),
            StaticPlacementModel::PackedExpertsFirstFitDecreasing
        );

        assert_eq!(
            serde_json::to_value(StaticPlacementModel::PackedExpertsFirstFitDecreasing)
                .expect("placement model serializes"),
            serde_json::json!({"kind": "PackedExpertsFirstFitDecreasing"})
        );
    }

    #[test]
    fn f_b4_budget_failure_records_concrete_byte_counts() {
        let expert = BudgetFailure::ExpertExceedsSlot {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            slot: BudgetSlotId::new(7),
            payload_bytes: 17_000,
            cap_bytes: 16_128,
            excess_bytes: 872,
        };
        let common = BudgetFailure::CommonBankExceedsCap {
            assigned_bytes: 20_000,
            cap_bytes: 16_384,
            excess_bytes: 3_616,
        };

        assert_eq!(
            budget_failure_validation_code(&expert),
            ValidationCode::BudgetExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: BudgetSlotId::new(7),
                payload_bytes: 17_000,
                cap_bytes: 16_128,
            }
        );
        assert_eq!(
            budget_failure_validation_code(&common),
            ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
            }
        );

        assert_eq!(
            serde_json::to_value(&expert).expect("expert failure serializes"),
            serde_json::json!({
                "kind": "ExpertExceedsSlot",
                "fields": {
                    "layer": 0,
                    "expert": 0,
                    "slot": 7,
                    "payload_bytes": 17000,
                    "cap_bytes": 16128,
                    "excess_bytes": 872
                }
            })
        );
        assert_eq!(
            serde_json::to_value(&common).expect("common failure serializes"),
            serde_json::json!({
                "kind": "CommonBankExceedsCap",
                "fields": {
                    "assigned_bytes": 20000,
                    "cap_bytes": 16384,
                    "excess_bytes": 3616
                }
            })
        );
    }

    #[test]
    fn f_b4_budget_failure_taxonomy_one_to_one_with_validation_code() {
        let failures = all_failure_variants();
        let diagnostics = validation_diagnostics_for_budget_failures(&failures);

        assert_eq!(diagnostics.len(), failures.len());
        for (failure, diagnostic) in failures.iter().zip(&diagnostics) {
            assert_eq!(diagnostic.origin, ValidationOrigin::Budget);
            assert_eq!(diagnostic.code, failure.validation_code());
            assert!(budget_failure_matches_diagnostic(failure, diagnostic));

            match failure {
                BudgetFailure::MissingRuntimeChromeBudget
                | BudgetFailure::QuantGraphBudgetViewMalformed { .. } => {
                    assert!(matches!(&diagnostic.detail, ValidationDetail::Field { .. }));
                }
                _ => {
                    assert!(matches!(
                        &diagnostic.detail,
                        ValidationDetail::Selector { .. }
                    ));
                }
            }
        }
    }

    #[test]
    fn f_b4_budget_failure_pins_selector_detail_strings() {
        let cases = [
            (
                BudgetFailure::ExpertExceedsSlot {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(1),
                    slot: BudgetSlotId::new(2),
                    payload_bytes: 17_000,
                    cap_bytes: 16_128,
                    excess_bytes: 872,
                },
                "budget.expert[layer=0,expert=1,slot=2]",
            ),
            (
                BudgetFailure::CommonBankExceedsCap {
                    assigned_bytes: 20_000,
                    cap_bytes: 16_384,
                    excess_bytes: 3_616,
                },
                "budget.common_bank",
            ),
            (
                BudgetFailure::AccumulatorExceedsI32 {
                    site: ReductionSiteId("ffn.0.acc".to_owned()),
                    projected_max_abs: i32::MAX as u64 + 1,
                },
                "budget.accumulator[site=ffn.0.acc]",
            ),
            (
                BudgetFailure::BankSwitchesPerTokenOverCap {
                    decision_value: 9,
                    upper_bound: 9,
                    cap: 5,
                    source: SwitchProjectionSource::ConservativeStaticUpperBound,
                },
                "budget.switches.bank_per_token",
            ),
            (
                BudgetFailure::PlacementProfileInfeasible {
                    profile: PlacementProfile::PackedExperts,
                    reason: PlacementInfeasibilityReason::ExpertCountExceedsSlots,
                },
                "budget.placement[profile=packed_experts,reason=expert_count_exceeds_slots]",
            ),
        ];

        for (failure, selector) in cases {
            assert_eq!(
                serde_json::to_value(failure.diagnostic_detail())
                    .expect("selector detail serializes"),
                serde_json::json!({
                    "kind": "Selector",
                    "selector": selector
                })
            );
        }
    }

    #[test]
    fn f_b4_budget_failure_diagnostic_accepts_provenance() {
        let failure = BudgetFailure::CommonBankExceedsCap {
            assigned_bytes: 20_000,
            cap_bytes: 16_384,
            excess_bytes: 3_616,
        };
        let provenance = vec![EvidenceRef {
            kind: "Fixture".to_owned(),
            reference: "static-budget-input".to_owned(),
            hash: Some(Hash256::from_bytes([7; 32])),
        }];

        let diagnostic = budget_failure_diagnostic_with_provenance(&failure, provenance.clone());

        assert_eq!(diagnostic.provenance, provenance);
        assert!(budget_failure_matches_diagnostic(&failure, &diagnostic));
    }

    #[test]
    fn f_b4_budget_failure_missing_runtime_chrome_budget_round_trip() {
        round_trip_failure(BudgetFailure::MissingRuntimeChromeBudget);

        assert_eq!(
            serde_json::to_value(BudgetFailure::MissingRuntimeChromeBudget)
                .expect("missing budget failure serializes"),
            serde_json::json!({"kind": "MissingRuntimeChromeBudget"})
        );
    }

    #[test]
    fn f_b4_budget_failure_quant_graph_view_malformed_round_trip() {
        let failure = BudgetFailure::QuantGraphBudgetViewMalformed {
            field: FieldPath::from("budget_view.per_expert_payload"),
        };

        round_trip_failure(failure.clone());
        assert_eq!(
            budget_failure_validation_code(&failure),
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("budget_view.per_expert_payload"),
            }
        );
        assert_eq!(
            serde_json::to_value(failure).expect("budget view malformed failure serializes"),
            serde_json::json!({
                "kind": "QuantGraphBudgetViewMalformed",
                "fields": {
                    "field": "budget_view.per_expert_payload"
                }
            })
        );
    }
}
