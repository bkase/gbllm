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
use gbf_policy::{CompileKnobId, ReductionPlanCeiling};
use gbf_report::{
    ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report, canonicalize_value,
    compute_self_hash,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use gbf_policy::{
    BudgetFailure, PlacementInfeasibilityReason, StaticFitInterpretation, ValidationCode,
    budget_failure_diagnostic, budget_failure_diagnostic_with_provenance,
    budget_failure_diagnostics, budget_failure_diagnostics_with_provenance,
    budget_failure_matches_diagnostic, budget_failure_validation_code,
};
pub use gbf_report::report_schemas::static_budget_v1::{
    AccumulatorBound, BudgetComponentRef, BudgetDecisionSection, BudgetIdentitySection,
    BudgetPolicySection, BudgetProjectionSection, CommonBankFootprintSection,
    ExpertPlacementStatus, PerBankEntry, PerExpertEntry, ProjectedSize, ProjectedSizeSection,
    ProjectedSizeSource, ProjectedSwitchCount, ProjectedSwitchCountSection, RoutingModelSection,
    RuntimeChromeBudgetSection, StaticBudgetReportBody, StaticPlacementModel, UnassignedBecause,
    decision_interpretation_matches_fits,
    runtime_chrome_budget_hash as runtime_chrome_budget_section_hash,
    sort_budget_failures_canonically, static_fit_interpretation_for_fits,
};

use crate::policy::ResolvedPolicyProduct;

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
    #[serde(default)]
    pub shared_dense_ffn: Option<SharedDenseFfnProjection>,
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
    #[serde(default)]
    pub bank0_compatible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedLutProjection {
    pub id: String,
    pub bytes: u32,
    #[serde(default)]
    pub bank0_compatible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedDenseFfnProjection {
    pub bytes: u32,
    #[serde(default)]
    pub bank0_compatible: bool,
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
    pub projected_wram_source: ProjectedSizeSource,
    pub projected_sram_bytes: u32,
    pub projected_sram_source: ProjectedSizeSource,
    pub projected_hram_bytes: u32,
    pub projected_hram_source: ProjectedSizeSource,
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

        for (index, site) in self.reduction_sites.iter().enumerate() {
            let zero_terms_with_nonzero_bound = site.term_count == 0
                && (site.input_max_abs_q != 0
                    || site.weight_max_abs_q != 0
                    || site.bias_max_abs_q.unwrap_or(0) != 0);
            if zero_terms_with_nonzero_bound {
                return Err(malformed_field(format!(
                    "budget_view.reduction_sites[{index}].term_count"
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticBudgetReport {
    pub report: ReportEnvelope<StaticBudgetReportBody>,
    pub static_budget_self_hash: Hash256,
    pub static_budget_canonical_bytes_hash: Hash256,
    #[serde(default)]
    pub reduction_site_facts: Vec<ReductionSiteProjection>,
}

pub trait StaticBudgetReductionSiteFacts {
    fn reduction_site_projection(&self, site: &ReductionSiteId)
    -> Option<&ReductionSiteProjection>;
}

impl StaticBudgetReductionSiteFacts for StaticBudgetReport {
    fn reduction_site_projection(
        &self,
        site: &ReductionSiteId,
    ) -> Option<&ReductionSiteProjection> {
        self.reduction_site_facts
            .iter()
            .find(|projection| &projection.site == site)
    }
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
                    empty_projections_for_runtime_budget(runtime_chrome_budget),
                    view.reduction_sites.clone(),
                ),
            },
            Err(error) => {
                let quant_graph_hash = inputs.quant_graph.quant_graph_hash();
                budget_failure_report(
                    inputs,
                    runtime_chrome_budget,
                    quant_graph_hash,
                    vec![error.into_failure()],
                    empty_projections_for_runtime_budget(runtime_chrome_budget),
                    Vec::new(),
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
    let section = RuntimeChromeBudgetSection::from(budget);
    runtime_chrome_budget_section_hash(&section)
}

fn empty_projections_for_runtime_budget(
    runtime_chrome_budget: &RuntimeChromeBudget,
) -> BudgetProjectionSection {
    let mut per_bank_occupancy: Vec<PerBankEntry> = runtime_chrome_budget
        .rom_slots
        .iter()
        .map(|slot| PerBankEntry {
            slot: slot.id,
            class: slot.class,
            usable_bytes: slot.usable_bytes,
            reserved_slack: slot.reserved_slack,
            effective_cap_bytes: rom_slot_effective_cap_bytes(slot),
            assigned_bytes: 0,
            residual_bytes: residual_bytes_for_assignment(slot, 0),
            assigned_components: Vec::new(),
            placement_caps: slot.placement_caps.clone(),
        })
        .collect();
    per_bank_occupancy.sort_by_key(|entry| entry.slot);
    BudgetProjectionSection {
        per_bank_occupancy,
        ..BudgetProjectionSection::default()
    }
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
    finalize_static_budget_report(ReportOutcome::Failed, body, Vec::new())
}

fn evaluated_budget_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
    runtime_chrome_budget: &RuntimeChromeBudget,
    view: QuantGraphBudgetView,
) -> StaticBudgetReport {
    let reduction_site_facts = view.reduction_sites.clone();
    let (projections, failures) = project_budget(
        inputs.policy,
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
            runtime_chrome_budget: Some(RuntimeChromeBudgetSection::from(runtime_chrome_budget)),
            projections,
            decision: decision_section(
                true,
                inputs.policy.policy.knobs.global.placement.profile,
                Vec::new(),
            ),
            diagnostics: Vec::new(),
        };
        finalize_static_budget_report(ReportOutcome::Passed, body, reduction_site_facts)
    } else {
        budget_failure_report(
            inputs,
            runtime_chrome_budget,
            view.quant_graph_hash,
            failures,
            projections,
            reduction_site_facts,
        )
    }
}

fn budget_failure_report<Q: QuantGraphBudgetSource + ?Sized>(
    inputs: BudgetInputs<'_, Q>,
    runtime_chrome_budget: &RuntimeChromeBudget,
    quant_graph_hash: Hash256,
    mut failures: Vec<BudgetFailure>,
    projections: BudgetProjectionSection,
    reduction_site_facts: Vec<ReductionSiteProjection>,
) -> StaticBudgetReport {
    sort_budget_failures_canonically(&mut failures);
    let body = StaticBudgetReportBody {
        identity: identity_section(
            inputs.policy,
            quant_graph_hash,
            Some(runtime_chrome_budget_hash(runtime_chrome_budget).expect("budget hashes")),
        ),
        policy: policy_section(inputs.policy),
        runtime_chrome_budget: Some(RuntimeChromeBudgetSection::from(runtime_chrome_budget)),
        projections,
        decision: decision_section(
            false,
            inputs.policy.policy.knobs.global.placement.profile,
            failures.clone(),
        ),
        diagnostics: validation_diagnostics_for_budget_failures(&failures),
    };
    finalize_static_budget_report(ReportOutcome::Failed, body, reduction_site_facts)
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
        interpretation: static_fit_interpretation_for_fits(fits),
        placement_model: placement_model_for_profile(profile),
        failures,
    }
}

fn finalize_static_budget_report(
    outcome: ReportOutcome,
    body: StaticBudgetReportBody,
    reduction_site_facts: Vec<ReductionSiteProjection>,
) -> StaticBudgetReport {
    tracing::info!(
        site_count = reduction_site_facts.len() as u64,
        outcome = ?outcome,
        "stage2.static_budget.reduction_site_facts.bound"
    );
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
        reduction_site_facts,
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
    policy: &ResolvedPolicyProduct,
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
            residual_bytes: residual_bytes_for_assignment(slot, 0),
            assigned_components: Vec::new(),
            placement_caps: slot.placement_caps.clone(),
        })
        .collect::<Vec<_>>();
    per_bank_occupancy.sort_by_key(|entry| entry.slot);

    let mut per_expert_payload = Vec::with_capacity(view.experts.len());
    let mut expert_payloads = Vec::with_capacity(view.experts.len());
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
        let payload_index = per_expert_payload.len();
        per_expert_payload.push(PerExpertEntry {
            layer: expert.layer,
            expert: expert.expert,
            payload_bytes,
            assigned_slot: None,
            unassigned_because: Some(UnassignedBecause::NoEligibleSlots),
            placement_status: ExpertPlacementStatus::UnassignedNoEligibleSlots,
        });
        expert_payloads.push(ExpertPlacementPayload {
            payload_index,
            layer: expert.layer,
            expert: expert.expert,
            payload_bytes,
        });
    }

    failures.append(&mut place_experts_by_static_model(
        profile,
        runtime_chrome_budget,
        &mut per_bank_occupancy,
        &mut per_expert_payload,
        &expert_payloads,
    ));

    let common_bank_footprint = match common_bank_footprint(view) {
        Ok(footprint) => footprint,
        Err(failure) => {
            failures.push(failure);
            CommonBankFootprintSection::default()
        }
    };
    failures.append(&mut place_common_bank_components(
        profile,
        runtime_chrome_budget,
        &mut per_bank_occupancy,
        view,
        common_bank_footprint.aggregate_bytes,
    ));

    let (accumulator_maxima, mut accumulator_failures) =
        accumulator_maxima(&view.reduction_sites, single_i16_only_locked(policy));
    failures.append(&mut accumulator_failures);

    let projected_wram = ProjectedSize {
        peak_bytes: view.sequence_state.projected_wram_bytes,
        source: view.sequence_state.projected_wram_source,
    };
    let projected_sram = ProjectedSize {
        peak_bytes: view.sequence_state.projected_sram_bytes,
        source: view.sequence_state.projected_sram_source,
    };
    let projected_hram = ProjectedSize {
        peak_bytes: view.sequence_state.projected_hram_bytes,
        source: view.sequence_state.projected_hram_source,
    };
    failures.append(&mut memory_peak_failures(
        &projected_wram,
        &projected_sram,
        &projected_hram,
        runtime_chrome_budget,
    ));
    let projected_bank_switches_per_token = projected_bank_switches_per_token(view);
    let projected_sram_page_switches_per_token =
        projected_sram_page_switches_per_token(&view.sequence_state);
    failures.append(&mut switch_count_failures(
        &policy.policy.objective,
        &projected_bank_switches_per_token,
        &projected_sram_page_switches_per_token,
    ));

    (
        BudgetProjectionSection {
            per_expert_payload,
            per_bank_occupancy,
            common_bank_footprint,
            accumulator_maxima,
            projected_wram,
            projected_sram,
            projected_hram,
            projected_bank_switches_per_token,
            projected_sram_page_switches_per_token,
            routing_model: view.routing.model.clone(),
        },
        failures,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExpertPlacementPayload {
    payload_index: usize,
    layer: LayerId,
    expert: ExpertId,
    payload_bytes: u32,
}

fn place_experts_by_static_model(
    profile: PlacementProfile,
    runtime_chrome_budget: &RuntimeChromeBudget,
    per_bank_occupancy: &mut [PerBankEntry],
    per_expert_payload: &mut [PerExpertEntry],
    expert_payloads: &[ExpertPlacementPayload],
) -> Vec<BudgetFailure> {
    let mut failures = Vec::new();
    let model = StaticPlacementModel::for_profile(profile);
    let mut placement_order = (0..expert_payloads.len()).collect::<Vec<_>>();

    if model == StaticPlacementModel::PackedExpertsFirstFitDecreasing {
        placement_order.sort_by(|left, right| {
            let left = expert_payloads[*left];
            let right = expert_payloads[*right];
            right
                .payload_bytes
                .cmp(&left.payload_bytes)
                .then_with(|| left.layer.cmp(&right.layer))
                .then_with(|| left.expert.cmp(&right.expert))
        });
    }

    for payload_index in placement_order {
        let payload = expert_payloads[payload_index];
        match first_fit_expert_slot(
            model,
            profile,
            runtime_chrome_budget,
            per_bank_occupancy,
            payload.payload_bytes,
        ) {
            Ok(slot_index) => {
                let slot = &mut per_bank_occupancy[slot_index];
                per_expert_payload[payload.payload_index].assigned_slot = Some(slot.slot);
                per_expert_payload[payload.payload_index].unassigned_because = None;
                per_expert_payload[payload.payload_index].placement_status =
                    ExpertPlacementStatus::Assigned;
                assign_component_to_slot(
                    slot,
                    payload.payload_bytes,
                    BudgetComponentRef::Expert {
                        layer: payload.layer,
                        expert: payload.expert,
                    },
                );
            }
            Err(ExpertPlacementFailure::ExceedsEligibleSlotCap { slot_index }) => {
                let failure = expert_placement_failure(
                    ExpertPlacementFailure::ExceedsEligibleSlotCap { slot_index },
                    profile,
                    payload,
                    per_bank_occupancy,
                );
                let slot = &mut per_bank_occupancy[slot_index];
                per_expert_payload[payload.payload_index].assigned_slot = Some(slot.slot);
                per_expert_payload[payload.payload_index].unassigned_because = None;
                per_expert_payload[payload.payload_index].placement_status =
                    ExpertPlacementStatus::AssignedOverCap;
                assign_component_to_slot(
                    slot,
                    payload.payload_bytes,
                    BudgetComponentRef::Expert {
                        layer: payload.layer,
                        expert: payload.expert,
                    },
                );
                failures.push(failure);
            }
            Err(failure) => {
                per_expert_payload[payload.payload_index].placement_status =
                    expert_unassigned_status(failure);
                per_expert_payload[payload.payload_index].unassigned_because =
                    expert_unassigned_because(failure);
                failures.push(expert_placement_failure(
                    failure,
                    profile,
                    payload,
                    per_bank_occupancy,
                ));
            }
        }
    }

    failures
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpertPlacementFailure {
    NoEligibleSlots,
    StrictDistinctSlotsExhausted,
    ExceedsEligibleSlotCap { slot_index: usize },
}

fn first_fit_expert_slot(
    model: StaticPlacementModel,
    profile: PlacementProfile,
    runtime_chrome_budget: &RuntimeChromeBudget,
    per_bank_occupancy: &[PerBankEntry],
    payload_bytes: u32,
) -> Result<usize, ExpertPlacementFailure> {
    let mut saw_eligible_slot = false;
    let mut saw_empty_strict_slot = false;
    let mut first_cap_failure = None;

    for (slot_index, entry) in per_bank_occupancy.iter().enumerate() {
        if !slot_accepts_profile(runtime_chrome_budget, entry.slot, profile)
            || entry.class != BudgetSlotClass::ExpertBank
        {
            continue;
        }
        saw_eligible_slot = true;

        if model == StaticPlacementModel::StrictOnePerBank && !entry.assigned_components.is_empty()
        {
            continue;
        }
        saw_empty_strict_slot = true;

        if entry_has_residual_capacity(entry, payload_bytes) {
            return Ok(slot_index);
        }
        first_cap_failure.get_or_insert(slot_index);
    }

    if let Some(slot_index) = first_cap_failure {
        Err(ExpertPlacementFailure::ExceedsEligibleSlotCap { slot_index })
    } else if model == StaticPlacementModel::StrictOnePerBank
        && saw_eligible_slot
        && !saw_empty_strict_slot
    {
        Err(ExpertPlacementFailure::StrictDistinctSlotsExhausted)
    } else {
        Err(ExpertPlacementFailure::NoEligibleSlots)
    }
}

const fn expert_unassigned_status(failure: ExpertPlacementFailure) -> ExpertPlacementStatus {
    match failure {
        ExpertPlacementFailure::NoEligibleSlots => ExpertPlacementStatus::UnassignedNoEligibleSlots,
        ExpertPlacementFailure::StrictDistinctSlotsExhausted => {
            ExpertPlacementStatus::UnassignedStrictDistinctSlotsExhausted
        }
        ExpertPlacementFailure::ExceedsEligibleSlotCap { .. } => {
            ExpertPlacementStatus::AssignedOverCap
        }
    }
}

const fn expert_unassigned_because(failure: ExpertPlacementFailure) -> Option<UnassignedBecause> {
    match failure {
        ExpertPlacementFailure::NoEligibleSlots => Some(UnassignedBecause::NoEligibleSlots),
        ExpertPlacementFailure::StrictDistinctSlotsExhausted => {
            Some(UnassignedBecause::StrictDistinctSlotsExhausted)
        }
        ExpertPlacementFailure::ExceedsEligibleSlotCap { .. } => None,
    }
}

fn expert_placement_failure(
    failure: ExpertPlacementFailure,
    profile: PlacementProfile,
    payload: ExpertPlacementPayload,
    per_bank_occupancy: &[PerBankEntry],
) -> BudgetFailure {
    match failure {
        ExpertPlacementFailure::NoEligibleSlots => BudgetFailure::PlacementProfileInfeasible {
            profile,
            reason: PlacementInfeasibilityReason::NoSlotsForClass,
        },
        ExpertPlacementFailure::StrictDistinctSlotsExhausted => {
            BudgetFailure::PlacementProfileInfeasible {
                profile,
                reason: PlacementInfeasibilityReason::ExpertCountExceedsSlots,
            }
        }
        ExpertPlacementFailure::ExceedsEligibleSlotCap { slot_index } => {
            let slot = &per_bank_occupancy[slot_index];
            let assigned_if_placed = slot.assigned_bytes.saturating_add(payload.payload_bytes);
            let cap_bytes = u32::try_from(slot.effective_cap_bytes.max(0)).unwrap_or(u32::MAX);
            BudgetFailure::ExpertExceedsSlot {
                layer: payload.layer,
                expert: payload.expert,
                slot: slot.slot,
                payload_bytes: payload.payload_bytes,
                cap_bytes,
                excess_bytes: assigned_if_placed.saturating_sub(cap_bytes),
            }
        }
    }
}

fn place_common_bank_components(
    profile: PlacementProfile,
    runtime_chrome_budget: &RuntimeChromeBudget,
    per_bank_occupancy: &mut [PerBankEntry],
    view: &QuantGraphBudgetView,
    aggregate_bytes: u32,
) -> Vec<BudgetFailure> {
    if aggregate_bytes == 0 {
        return Vec::new();
    }

    let mut failures = Vec::new();
    for kernel in &view.shared_kernels {
        if let Some(failure) = place_common_component(
            profile,
            runtime_chrome_budget,
            per_bank_occupancy,
            kernel.bytes,
            BudgetComponentRef::SharedKernel {
                id: kernel.id.clone(),
            },
            kernel.bank0_compatible,
        ) {
            push_common_budget_failure(&mut failures, failure);
        }
    }
    for lut in &view.shared_luts {
        if let Some(failure) = place_common_component(
            profile,
            runtime_chrome_budget,
            per_bank_occupancy,
            lut.bytes,
            BudgetComponentRef::SharedLut { id: lut.id.clone() },
            lut.bank0_compatible,
        ) {
            push_common_budget_failure(&mut failures, failure);
        }
    }
    if let Some(shared_dense_ffn) = &view.shared_dense_ffn
        && let Some(failure) = place_common_component(
            profile,
            runtime_chrome_budget,
            per_bank_occupancy,
            shared_dense_ffn.bytes,
            BudgetComponentRef::SharedDenseFfn,
            shared_dense_ffn.bank0_compatible,
        )
    {
        push_common_budget_failure(&mut failures, failure);
    }

    failures
}

fn push_common_budget_failure(failures: &mut Vec<BudgetFailure>, failure: BudgetFailure) {
    if let BudgetFailure::CommonBankExceedsCap {
        assigned_bytes,
        cap_bytes,
        excess_bytes,
    } = &failure
        && let Some(existing) = failures.iter_mut().find_map(|candidate| match candidate {
            BudgetFailure::CommonBankExceedsCap {
                assigned_bytes,
                cap_bytes,
                excess_bytes,
            } => Some((assigned_bytes, cap_bytes, excess_bytes)),
            _ => None,
        })
    {
        let (existing_assigned_bytes, existing_cap_bytes, existing_excess_bytes) = existing;
        if (*excess_bytes, *assigned_bytes) > (*existing_excess_bytes, *existing_assigned_bytes) {
            *existing_assigned_bytes = *assigned_bytes;
            *existing_cap_bytes = *cap_bytes;
            *existing_excess_bytes = *excess_bytes;
        }
        return;
    }

    if !failures.contains(&failure) {
        failures.push(failure);
    }
}

fn place_common_component(
    profile: PlacementProfile,
    runtime_chrome_budget: &RuntimeChromeBudget,
    per_bank_occupancy: &mut [PerBankEntry],
    component_bytes: u32,
    component: BudgetComponentRef,
    bank0_compatible: bool,
) -> Option<BudgetFailure> {
    let mut saw_eligible_slot = false;

    for entry in per_bank_occupancy.iter() {
        if common_component_slot_eligible(runtime_chrome_budget, entry, profile, bank0_compatible) {
            saw_eligible_slot = true;
        }
    }

    if !saw_eligible_slot {
        return Some(BudgetFailure::PlacementProfileInfeasible {
            profile,
            reason: PlacementInfeasibilityReason::RequiresUnavailableSlotClass,
        });
    }

    if let Some(slot_index) = per_bank_occupancy.iter().position(|entry| {
        common_component_slot_eligible(runtime_chrome_budget, entry, profile, bank0_compatible)
            && entry_has_residual_capacity(entry, component_bytes)
    }) {
        let slot = &mut per_bank_occupancy[slot_index];
        assign_component_to_slot(slot, component_bytes, component);
        None
    } else if let Some(slot_index) = per_bank_occupancy.iter().position(|entry| {
        common_component_slot_eligible(runtime_chrome_budget, entry, profile, bank0_compatible)
    }) {
        let slot = &mut per_bank_occupancy[slot_index];
        assign_component_to_slot(slot, component_bytes, component);
        let slot_cap_bytes = u32::try_from(slot.effective_cap_bytes.max(0)).unwrap_or(u32::MAX);
        Some(BudgetFailure::CommonBankExceedsCap {
            assigned_bytes: slot.assigned_bytes,
            cap_bytes: slot_cap_bytes,
            excess_bytes: slot.assigned_bytes.saturating_sub(slot_cap_bytes),
        })
    } else {
        Some(BudgetFailure::PlacementProfileInfeasible {
            profile,
            reason: PlacementInfeasibilityReason::RequiresUnavailableSlotClass,
        })
    }
}

fn common_bank_footprint(
    view: &QuantGraphBudgetView,
) -> Result<CommonBankFootprintSection, BudgetFailure> {
    let kernel_bytes = checked_sum_byte_budget(
        view.shared_kernels
            .iter()
            .map(|kernel| ByteBudget::from(kernel.bytes)),
    )
    .and_then(u32_byte_budget)
    .map_err(|_| BudgetFailure::QuantGraphBudgetViewMalformed {
        field: FieldPath::from("budget_view.shared_kernels"),
    })?;
    let lut_bytes = checked_sum_byte_budget(
        view.shared_luts
            .iter()
            .map(|lut| ByteBudget::from(lut.bytes)),
    )
    .and_then(u32_byte_budget)
    .map_err(|_| BudgetFailure::QuantGraphBudgetViewMalformed {
        field: FieldPath::from("budget_view.shared_luts"),
    })?;
    let shared_dense_ffn_bytes = view.shared_dense_ffn.as_ref().map(|dense| dense.bytes);
    let aggregate_bytes = [Some(kernel_bytes), Some(lut_bytes), shared_dense_ffn_bytes]
        .into_iter()
        .flatten()
        .try_fold(0u32, |sum, bytes| {
            sum.checked_add(bytes).ok_or(BudgetMathError::Overflow)
        })
        .map_err(|_| BudgetFailure::QuantGraphBudgetViewMalformed {
            field: FieldPath::from("budget_view.common_bank_footprint.aggregate_bytes"),
        })?;

    Ok(CommonBankFootprintSection {
        kernel_bytes,
        lut_bytes,
        shared_dense_ffn_bytes,
        aggregate_bytes,
    })
}

fn common_component_slot_eligible(
    runtime_chrome_budget: &RuntimeChromeBudget,
    entry: &PerBankEntry,
    profile: PlacementProfile,
    bank0_compatible: bool,
) -> bool {
    let class_eligible = entry.class == BudgetSlotClass::CommonBank
        || (bank0_compatible && entry.class == BudgetSlotClass::Bank0Free);
    class_eligible && slot_accepts_profile(runtime_chrome_budget, entry.slot, profile)
}

fn slot_accepts_profile(
    runtime_chrome_budget: &RuntimeChromeBudget,
    slot: BudgetSlotId,
    profile: PlacementProfile,
) -> bool {
    runtime_chrome_budget
        .rom_slots
        .iter()
        .find(|candidate| candidate.id == slot)
        .is_some_and(|candidate| candidate.placement_caps.contains(&profile))
}

fn entry_has_residual_capacity(entry: &PerBankEntry, payload_bytes: u32) -> bool {
    i64::from(entry.assigned_bytes) + i64::from(payload_bytes) <= entry.effective_cap_bytes
}

fn assign_component_to_slot(
    slot: &mut PerBankEntry,
    component_bytes: u32,
    component: BudgetComponentRef,
) {
    slot.assigned_bytes = slot.assigned_bytes.saturating_add(component_bytes);
    slot.residual_bytes = residual_bytes_for_entry(slot);
    slot.assigned_components.push(component);
}

fn residual_bytes_for_entry(entry: &PerBankEntry) -> i32 {
    residual_i32(entry.effective_cap_bytes - i64::from(entry.assigned_bytes))
}

fn residual_bytes_for_assignment(slot: &RomBudgetSlot, assigned_bytes: u32) -> i32 {
    residual_i32(rom_slot_effective_cap_bytes(slot) - i64::from(assigned_bytes))
}

fn residual_i32(residual: i64) -> i32 {
    i32::try_from(residual).unwrap_or(if residual.is_negative() {
        i32::MIN
    } else {
        i32::MAX
    })
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

fn checked_sum_byte_budget(
    mut values: impl Iterator<Item = ByteBudget>,
) -> Result<ByteBudget, BudgetMathError> {
    values.try_fold(0, checked_add_byte_budget)
}

fn memory_peak_failures(
    projected_wram: &ProjectedSize,
    projected_sram: &ProjectedSize,
    projected_hram: &ProjectedSize,
    runtime_chrome_budget: &RuntimeChromeBudget,
) -> Vec<BudgetFailure> {
    let mut failures = Vec::new();
    let memory_caps = runtime_chrome_budget.memory_caps;

    if projected_wram.peak_bytes > memory_caps.wram_usable_bytes {
        failures.push(BudgetFailure::WramPeakExceedsCap {
            peak: projected_wram.peak_bytes,
            cap: memory_caps.wram_usable_bytes,
        });
    }
    if projected_sram.peak_bytes > memory_caps.sram_usable_bytes {
        failures.push(BudgetFailure::SramPeakExceedsCap {
            peak: projected_sram.peak_bytes,
            cap: memory_caps.sram_usable_bytes,
        });
    }
    if projected_hram.peak_bytes > memory_caps.hram_usable_bytes {
        failures.push(BudgetFailure::HramPeakExceedsCap {
            peak: projected_hram.peak_bytes,
            cap: memory_caps.hram_usable_bytes,
        });
    }

    failures
}

fn projected_bank_switches_per_token(view: &QuantGraphBudgetView) -> ProjectedSwitchCount {
    ProjectedSwitchCount {
        upper_bound: view.routing.projected_bank_switches_per_token,
        expected_q16_16: view.routing.expected_bank_switches_q16_16,
        decision_value: view.routing.projected_bank_switches_per_token,
        source: SwitchProjectionSource::ConservativeStaticUpperBound,
    }
}

fn projected_sram_page_switches_per_token(
    sequence_state: &SequenceStateProjection,
) -> ProjectedSwitchCount {
    // T-B4.7 treats the sequence-state page count as the static per-token
    // upper bound for the current access pattern. F-B9 owns the detailed SRAM
    // page plan; v1 budget decisions still use this conservative count.
    ProjectedSwitchCount {
        upper_bound: sequence_state.projected_sram_page_switches_per_token,
        expected_q16_16: None,
        decision_value: sequence_state.projected_sram_page_switches_per_token,
        source: SwitchProjectionSource::ConservativeStaticUpperBound,
    }
}

fn switch_count_failures(
    objective: &gbf_policy::CompileObjective,
    projected_bank_switches_per_token: &ProjectedSwitchCount,
    projected_sram_page_switches_per_token: &ProjectedSwitchCount,
) -> Vec<BudgetFailure> {
    let mut failures = Vec::new();

    if let Some(cap) = objective.max_bank_switches_per_token
        && projected_bank_switches_per_token.decision_value > cap
    {
        failures.push(BudgetFailure::BankSwitchesPerTokenOverCap {
            decision_value: projected_bank_switches_per_token.decision_value,
            upper_bound: projected_bank_switches_per_token.upper_bound,
            cap,
            source: projected_bank_switches_per_token.source,
        });
    }

    if let Some(cap) = objective.max_sram_page_switches_per_token
        && projected_sram_page_switches_per_token.decision_value > cap
    {
        failures.push(BudgetFailure::SramPageSwitchesPerTokenOverCap {
            decision_value: projected_sram_page_switches_per_token.decision_value,
            upper_bound: projected_sram_page_switches_per_token.upper_bound,
            cap,
            source: projected_sram_page_switches_per_token.source,
        });
    }

    failures
}

fn single_i16_only_locked(policy: &ResolvedPolicyProduct) -> bool {
    policy
        .policy
        .knobs
        .locks
        .locked
        .contains(&CompileKnobId::Range)
        && policy.policy.knobs.global.range.reduction_ceiling == ReductionPlanCeiling::ExactOnly
}

fn accumulator_maxima(
    reduction_sites: &[ReductionSiteProjection],
    single_i16_only_locked: bool,
) -> (Vec<AccumulatorBound>, Vec<BudgetFailure>) {
    let mut bounds = Vec::with_capacity(reduction_sites.len());
    let mut failures = Vec::new();

    for (index, site) in reduction_sites.iter().enumerate() {
        let bound = match accumulator_bound(site, index) {
            Ok(bound) => bound,
            Err(failure) => {
                failures.push(failure);
                continue;
            }
        };

        if !bound.i32_safe || (single_i16_only_locked && !bound.i16_safe) {
            failures.push(BudgetFailure::AccumulatorExceedsI32 {
                site: bound.site.clone(),
                projected_max_abs: bound.projected_max_abs,
            });
        }
        bounds.push(bound);
    }

    (bounds, failures)
}

fn accumulator_bound(
    site: &ReductionSiteProjection,
    index: usize,
) -> Result<AccumulatorBound, BudgetFailure> {
    let raw_product_bound = u128::from(site.input_max_abs_q)
        .checked_mul(u128::from(site.weight_max_abs_q))
        .ok_or_else(|| malformed_accumulator_projection(index))?;
    let sum_bound = u128::from(site.term_count)
        .checked_mul(raw_product_bound)
        .ok_or_else(|| malformed_accumulator_projection(index))?;
    let projected_max_abs = sum_bound
        .checked_add(u128::from(site.bias_max_abs_q.unwrap_or(0)))
        .ok_or_else(|| malformed_accumulator_projection(index))?;
    let projected_max_abs =
        u64::try_from(projected_max_abs).map_err(|_| malformed_accumulator_projection(index))?;

    Ok(AccumulatorBound {
        site: site.site.clone(),
        projected_max_abs,
        i16_safe: projected_max_abs <= i16::MAX as u64,
        i32_safe: projected_max_abs <= i32::MAX as u64,
    })
}

fn malformed_accumulator_projection(index: usize) -> BudgetFailure {
    BudgetFailure::QuantGraphBudgetViewMalformed {
        field: FieldPath::from(format!(
            "budget_view.reduction_sites[{index}].projected_max_abs"
        )),
    }
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
    use std::sync::{Arc, Mutex, OnceLock};

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
    use gbf_report::ReportBody;
    use gbf_report::ReportOutcome;
    use gbf_report::report_schemas::policy_resolution_v1::{
        ArtifactIdentitySection, CompileRequestSection, HintConsumptionSection,
        PolicyResolutionReportBody,
    };
    use tracing::field::{Field, Visit};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;

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
            range_caps: gbf_policy::RangeCapsSpec::default_v2(),
            observation_caps: gbf_policy::ObservationProfileCaps::default_v2(),
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
                compile_profile_spec_version: gbf_policy::COMPILE_PROFILE_SPEC_VERSION.to_owned(),
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
            shared_dense_ffn: None,
            reduction_sites: Vec::new(),
            sequence_state: SequenceStateProjection::default(),
            routing: RoutingProjection::default(),
        }
    }

    fn policy_with_placement(profile: PlacementProfile) -> ResolvedPolicyProduct {
        let mut policy = policy_fixture();
        policy.policy.knobs.global.placement.profile = profile;
        policy
    }

    fn runtime_budget_with_slots(rom_slots: Vec<RomBudgetSlot>) -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            rom_slots,
            ..runtime_budget_fixture()
        }
    }

    fn expert_slot(
        id: u16,
        usable_bytes: u32,
        reserved_slack: u16,
        placement_caps: impl IntoIterator<Item = PlacementProfile>,
    ) -> RomBudgetSlot {
        RomBudgetSlot {
            id: BudgetSlotId::new(id),
            class: BudgetSlotClass::ExpertBank,
            usable_bytes,
            reserved_slack,
            placement_caps: BTreeSet::from_iter(placement_caps),
        }
    }

    fn common_slot(
        id: u16,
        usable_bytes: u32,
        placement_caps: impl IntoIterator<Item = PlacementProfile>,
    ) -> RomBudgetSlot {
        RomBudgetSlot {
            id: BudgetSlotId::new(id),
            class: BudgetSlotClass::CommonBank,
            usable_bytes,
            reserved_slack: 0,
            placement_caps: BTreeSet::from_iter(placement_caps),
        }
    }

    fn bank0_slot(
        id: u16,
        usable_bytes: u32,
        placement_caps: impl IntoIterator<Item = PlacementProfile>,
    ) -> RomBudgetSlot {
        RomBudgetSlot {
            id: BudgetSlotId::new(id),
            class: BudgetSlotClass::Bank0Free,
            usable_bytes,
            reserved_slack: 0,
            placement_caps: BTreeSet::from_iter(placement_caps),
        }
    }

    fn expert_with_payload(layer: u16, expert: u16, payload_bytes: u32) -> ExpertProjection {
        assert!(payload_bytes >= 2);
        ExpertProjection {
            layer: LayerId::new(layer),
            expert: ExpertId::new(expert),
            rows: 1,
            cols: 1,
            metadata_bytes: payload_bytes - 2,
            plan: plan_with(
                WeightEncoding::Binary1,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q4_4,
            ),
        }
    }

    fn budget_view_with_experts(experts: Vec<ExpertProjection>) -> QuantGraphBudgetView {
        let layers = experts
            .iter()
            .map(|expert| expert.layer)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        QuantGraphBudgetView {
            layers,
            experts,
            ..budget_view_fixture(4, 4, 0)
        }
    }

    fn run_budget_with_policy(
        policy: &ResolvedPolicyProduct,
        runtime_chrome_budget: &RuntimeChromeBudget,
        view: QuantGraphBudgetView,
    ) -> StaticBudgetReport {
        let quant_graph = TrackingQuantGraph::new(view);
        run_static_budget(BudgetInputs {
            policy,
            quant_graph: &quant_graph,
            runtime_chrome_budget: Some(runtime_chrome_budget),
            target_profile: &dmg_mbc5_8mib_128kib(),
        })
    }

    #[test]
    fn f_b4_budget_per_expert_payload_covers_every_expert() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            expert_slot(1, 20, 0, [PlacementProfile::Budgeted]),
            common_slot(2, 64, [PlacementProfile::Budgeted]),
            expert_slot(3, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 10),
            expert_with_payload(0, 1, 11),
        ]);

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(report.report.body.projections.per_expert_payload.len(), 2);
        assert_eq!(
            report
                .report
                .body
                .projections
                .per_expert_payload
                .iter()
                .map(|entry| (entry.layer, entry.expert, entry.payload_bytes))
                .collect::<Vec<_>>(),
            vec![
                (LayerId::new(0), ExpertId::new(0), 10),
                (LayerId::new(0), ExpertId::new(1), 11),
            ]
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy.len(),
            budget.rom_slots.len()
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].placement_caps,
            BTreeSet::from([PlacementProfile::Budgeted])
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![BudgetComponentRef::Expert {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
            }]
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[2].assigned_components,
            vec![BudgetComponentRef::Expert {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
            }]
        );
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
            &policy_fixture(),
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
    fn f_b4_budget_byte_math_is_canonical_when_artifact_diagnostic_differs() {
        let per_group = plan_with(
            WeightEncoding::Ternary2,
            ScaleGranularity::per_group(16).unwrap(),
            ScaleFormat::Q8_8,
        );

        // TernaryWeightPlan::compute_byte_cost is the target-independent
        // artifact/model diagnostic helper and keeps the historical flattened
        // PerGroup scale count. Stage 2 owns canonical deployed payload bytes.
        assert_eq!(per_group.compute_byte_cost(128, 224).as_u64(), 10_752);
        assert_eq!(
            expert_payload_bytes(&expert_with_plan(128, 224, 0, per_group)),
            Ok(7_184)
        );
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
                bank0_compatible: false,
            },
            SharedKernelProjection {
                id: KernelSpecId::from("kernel.a"),
                bytes: 4,
                bank0_compatible: false,
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
                bank0_compatible: false,
            },
            SharedLutProjection {
                id: "lut.a".to_owned(),
                bytes: 4,
                bank0_compatible: false,
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
                    bank0_compatible: false,
                },
                SharedKernelProjection {
                    id: KernelSpecId::from("kernel.shared"),
                    bytes: 4,
                    bank0_compatible: false,
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
                    bank0_compatible: false,
                },
                SharedLutProjection {
                    id: "lut.shared".to_owned(),
                    bytes: 4,
                    bank0_compatible: false,
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
            bank0_compatible: false,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.affine_clip.0".to_owned(),
            bytes: 64,
            bank0_compatible: false,
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

    fn reduction_site_projection(
        site: &str,
        bias_max_abs_q: Option<u32>,
    ) -> ReductionSiteProjection {
        ReductionSiteProjection {
            site: ReductionSiteId(site.to_owned()),
            layer: Some(LayerId::new(0)),
            expert: Some(ExpertId::new(0)),
            term_count: 7,
            input_max_abs_q: 3,
            weight_max_abs_q: 5,
            bias_max_abs_q,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }
    }

    fn run_budget_with_reduction_sites(
        reduction_sites: Vec<ReductionSiteProjection>,
    ) -> StaticBudgetReport {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.reduction_sites = reduction_sites;
        let quant_graph = TrackingQuantGraph::new(view);

        run_budget_with(Some(&budget), &quant_graph)
    }

    #[test]
    fn static_budget_reduction_site_facts_trait_returns_projection_when_present() {
        let projection = reduction_site_projection("site.facts.present", None);
        let report = run_budget_with_reduction_sites(vec![projection.clone()]);

        assert_eq!(
            report.reduction_site_projection(&projection.site),
            Some(&projection)
        );
    }

    #[test]
    fn static_budget_reduction_site_facts_trait_returns_none_for_unknown_site() {
        let report = run_budget_with_reduction_sites(vec![reduction_site_projection(
            "site.facts.known",
            None,
        )]);

        assert_eq!(
            report.reduction_site_projection(&ReductionSiteId("site.facts.unknown".to_owned())),
            None
        );
    }

    #[test]
    fn static_budget_reduction_site_facts_matches_quant_graph_budget_view() {
        let mut first = reduction_site_projection("site.facts.first", Some(9));
        first.accumulator_domain = AccumulatorDomain::PostScaleQ8_8;
        let mut second = reduction_site_projection("site.facts.second", None);
        second.layer = Some(LayerId::new(1));
        second.expert = None;
        second.term_count = 11;
        second.input_max_abs_q = 13;
        second.weight_max_abs_q = 17;
        second.accumulator_domain = AccumulatorDomain::PostScaleQ16_16;
        let expected = vec![first, second];

        let report = run_budget_with_reduction_sites(expected.clone());
        let actual_value =
            serde_json::to_value(&report.reduction_site_facts).expect("facts serialize");
        let expected_value = serde_json::to_value(&expected).expect("expected facts serialize");

        assert_eq!(report.reduction_site_facts, expected);
        assert_eq!(
            canonicalize_value(&actual_value).expect("facts canonicalize"),
            canonicalize_value(&expected_value).expect("expected facts canonicalize")
        );
        for projection in &expected {
            assert_eq!(
                report.reduction_site_projection(&projection.site),
                Some(projection)
            );
        }
    }

    #[test]
    fn static_budget_report_serde_unchanged_v1_compat() {
        let projection = reduction_site_projection("site.facts.serde", Some(3));
        let report = run_budget_with_reduction_sites(vec![projection]);
        let canonical = canonicalize_report(&report.report).expect("report canonicalizes");
        let value: serde_json::Value =
            serde_json::from_slice(&canonical).expect("canonical report decodes");
        let body_value = serde_json::to_value(&report.report.body).expect("report body serializes");
        let accumulator = value["projections"]["accumulator_maxima"][0]
            .as_object()
            .expect("accumulator bound is an object");

        assert_eq!(value["schema"], serde_json::json!("static_budget.v1"));
        assert!(value.get("reduction_site_facts").is_none());
        assert!(body_value.get("reduction_site_facts").is_none());
        assert!(value["projections"].get("reduction_site_facts").is_none());
        assert!(accumulator.get("site").is_some());
        assert!(accumulator.get("projected_max_abs").is_some());
        assert!(accumulator.get("i16_safe").is_some());
        assert!(accumulator.get("i32_safe").is_some());
        assert!(accumulator.get("term_count").is_none());
        assert!(accumulator.get("input_max_abs_q").is_none());
        assert!(accumulator.get("weight_max_abs_q").is_none());
        assert!(accumulator.get("bias_max_abs_q").is_none());
        assert!(accumulator.get("accumulator_domain").is_none());
    }

    #[test]
    fn static_budget_reduction_site_facts_with_bias_none() {
        let projection = reduction_site_projection("site.facts.bias_none", None);
        let report = run_budget_with_reduction_sites(vec![projection.clone()]);

        assert_eq!(
            report
                .reduction_site_projection(&projection.site)
                .map(|projection| projection.bias_max_abs_q),
            Some(None)
        );
    }

    #[test]
    fn static_budget_reduction_site_facts_with_bias_some_zero() {
        let projection = reduction_site_projection("site.facts.bias_zero", Some(0));
        let report = run_budget_with_reduction_sites(vec![projection.clone()]);

        assert_eq!(
            report
                .reduction_site_projection(&projection.site)
                .map(|projection| projection.bias_max_abs_q),
            Some(Some(0))
        );
    }

    #[test]
    fn static_budget_reduction_site_facts_with_bias_some_nonzero() {
        let projection = reduction_site_projection("site.facts.bias_nonzero", Some(19));
        let report = run_budget_with_reduction_sites(vec![projection.clone()]);

        assert_eq!(
            report
                .reduction_site_projection(&projection.site)
                .map(|projection| projection.bias_max_abs_q),
            Some(Some(19))
        );
    }

    #[test]
    fn static_budget_reduction_site_facts_sites_match_accumulator_maxima_on_success() {
        let first = reduction_site_projection("site.consistency.first", Some(1));
        let mut second = reduction_site_projection("site.consistency.second", None);
        second.term_count = 13;
        second.input_max_abs_q = 2;
        second.weight_max_abs_q = 3;
        let report = run_budget_with_reduction_sites(vec![first.clone(), second.clone()]);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        let fact_sites = report
            .reduction_site_facts
            .iter()
            .map(|projection| projection.site.clone())
            .collect::<Vec<_>>();
        let accumulator_sites = report
            .report
            .body
            .projections
            .accumulator_maxima
            .iter()
            .map(|bound| bound.site.clone())
            .collect::<Vec<_>>();

        assert_eq!(fact_sites, accumulator_sites);
        assert_eq!(
            fact_sites,
            vec![
                ReductionSiteId("site.consistency.first".to_owned()),
                ReductionSiteId("site.consistency.second".to_owned())
            ]
        );
    }

    #[test]
    fn static_budget_validation_failure_preserves_non_empty_reduction_site_facts() {
        let budget = runtime_budget_fixture();
        let projection = reduction_site_projection("site.validation_failure.preserved", Some(5));
        let mut view = budget_view_fixture(4, 4, 0);
        view.semantic_core_hash = hash(0xee);
        view.reduction_sites = vec![projection.clone()];
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(report.reduction_site_facts, vec![projection]);
        assert!(report.report.body.projections.accumulator_maxima.is_empty());
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

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct CapturedEvent {
        message: Option<String>,
        site_count: Option<u64>,
        outcome: Option<String>,
    }

    #[derive(Clone, Default)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = EventVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("capture lock is not poisoned")
                .push(visitor.event);
        }
    }

    #[derive(Default)]
    struct EventVisitor {
        event: CapturedEvent,
    }

    impl Visit for EventVisitor {
        fn record_u64(&mut self, field: &Field, value: u64) {
            if field.name() == "site_count" {
                self.event.site_count = Some(value);
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            match field.name() {
                "outcome" => self.event.outcome = Some(format!("{value:?}")),
                "message" => {
                    self.event.message = Some(format!("{value:?}").trim_matches('"').to_owned())
                }
                _ => {}
            }
        }
    }

    static STATIC_BUDGET_EVENT_CAPTURE: OnceLock<Arc<Mutex<Vec<CapturedEvent>>>> = OnceLock::new();

    fn static_budget_event_capture() -> Arc<Mutex<Vec<CapturedEvent>>> {
        Arc::clone(STATIC_BUDGET_EVENT_CAPTURE.get_or_init(|| {
            let capture = CaptureLayer::default();
            let events = Arc::clone(&capture.events);
            let subscriber = tracing_subscriber::registry().with(capture);
            tracing::subscriber::set_global_default(subscriber)
                .expect("static budget event capture subscriber installs once");
            tracing::callsite::rebuild_interest_cache();
            events
        }))
    }

    #[test]
    fn static_budget_reduction_site_facts_bound_event_is_captured_by_subscriber() {
        let events = static_budget_event_capture();
        let projection = reduction_site_projection("site.telemetry.bound", Some(1));

        tracing::callsite::rebuild_interest_cache();
        events.lock().expect("capture lock is not poisoned").clear();

        let report = run_budget_with_reduction_sites(vec![projection]);
        assert_eq!(report.report.outcome, ReportOutcome::Passed);

        let events = events.lock().expect("capture lock is not poisoned");
        assert!(events.iter().any(|event| {
            event.message.as_deref() == Some("stage2.static_budget.reduction_site_facts.bound")
                && event.site_count == Some(1)
                && event.outcome.as_deref() == Some("Passed")
        }));
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
    fn f_b4_budget_fits_means_necessary_checks_only() {
        let budget = runtime_budget_fixture();
        let passing_graph = TrackingQuantGraph::new(budget_view_fixture(4, 4, 0));

        let passing_report = run_budget_with(Some(&budget), &passing_graph);

        assert_eq!(passing_report.report.outcome, ReportOutcome::Passed);
        assert!(passing_report.report.body.decision.fits);
        assert!(passing_report.report.body.decision.failures.is_empty());
        assert_eq!(
            passing_report.report.body.decision.interpretation,
            StaticFitInterpretation::PassesNecessaryStaticChecks
        );
        assert!(
            passing_report
                .report
                .body
                .validate_semantics(passing_report.report.outcome)
                .is_ok()
        );

        let mut failing_view = budget_view_fixture(4, 4, 0);
        failing_view.sequence_state.projected_wram_bytes = budget.memory_caps.wram_usable_bytes + 1;
        let failing_graph = TrackingQuantGraph::new(failing_view);
        let failing_report = run_budget_with(Some(&budget), &failing_graph);

        assert_eq!(failing_report.report.outcome, ReportOutcome::Failed);
        assert!(!failing_report.report.body.decision.fits);
        assert!(!failing_report.report.body.decision.failures.is_empty());
        assert_eq!(
            failing_report.report.body.decision.interpretation,
            StaticFitInterpretation::FailsNecessaryStaticChecks
        );

        let missing_budget_report = run_budget_with(None, &passing_graph);
        assert!(!missing_budget_report.report.body.decision.fits);
        assert_eq!(
            missing_budget_report.report.body.decision.failures,
            vec![BudgetFailure::MissingRuntimeChromeBudget]
        );
        assert_eq!(
            missing_budget_report.report.body.decision.interpretation,
            StaticFitInterpretation::FailsNecessaryStaticChecks
        );

        let mut mismatched = passing_report.report.body.clone();
        mismatched.decision.interpretation = StaticFitInterpretation::FailsNecessaryStaticChecks;
        assert!(
            mismatched
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );

        let mut mismatched_failure = failing_report.report.body.clone();
        mismatched_failure.decision.interpretation =
            StaticFitInterpretation::PassesNecessaryStaticChecks;
        assert!(
            mismatched_failure
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
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
    fn f_b4_budget_rejects_wram_peak_exceeds_cap() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.sequence_state.projected_wram_bytes = budget.memory_caps.wram_usable_bytes + 1;
        view.sequence_state.projected_wram_source = ProjectedSizeSource::StaticGraphProjection;
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.projected_wram,
            ProjectedSize {
                peak_bytes: 8193,
                source: ProjectedSizeSource::StaticGraphProjection,
            }
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::WramPeakExceedsCap {
                peak: 8193,
                cap: 8192,
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetWramPeakExceeds {
                peak: 8193,
                cap: 8192,
            }
        );
    }

    #[test]
    fn f_b4_budget_rejects_sram_peak_exceeds_cap() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.sequence_state.projected_sram_bytes = budget.memory_caps.sram_usable_bytes + 1;
        view.sequence_state.projected_sram_source = ProjectedSizeSource::HintBundleConstraint;
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.projected_sram,
            ProjectedSize {
                peak_bytes: 32_769,
                source: ProjectedSizeSource::HintBundleConstraint,
            }
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::SramPeakExceedsCap {
                peak: 32_769,
                cap: 32_768,
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetSramPeakExceeds {
                peak: 32_769,
                cap: 32_768,
            }
        );
    }

    #[test]
    fn f_b4_budget_rejects_hram_peak_exceeds_cap() {
        let mut budget = runtime_budget_fixture();
        budget.memory_caps.hram_usable_bytes = 64;
        let mut view = budget_view_fixture(4, 4, 0);
        view.sequence_state.projected_hram_bytes = 65;
        view.sequence_state.projected_hram_source =
            ProjectedSizeSource::CalibrationSamplingClosedForm;
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.projected_hram,
            ProjectedSize {
                peak_bytes: 65,
                source: ProjectedSizeSource::CalibrationSamplingClosedForm,
            }
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::HramPeakExceedsCap { peak: 65, cap: 64 }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetHramPeakExceeds { peak: 65, cap: 64 }
        );
    }

    #[test]
    fn f_b4_budget_records_projected_size_source() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.sequence_state.projected_wram_bytes = 128;
        view.sequence_state.projected_wram_source = ProjectedSizeSource::HintBundleConstraint;
        view.sequence_state.projected_sram_bytes = 0;
        view.sequence_state.projected_sram_source =
            ProjectedSizeSource::CalibrationSamplingClosedForm;
        view.sequence_state.projected_hram_bytes = 16;
        view.sequence_state.projected_hram_source = ProjectedSizeSource::StaticGraphProjection;
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report.report.body.projections.projected_wram,
            ProjectedSize {
                peak_bytes: 128,
                source: ProjectedSizeSource::HintBundleConstraint,
            }
        );
        assert_eq!(
            report.report.body.projections.projected_sram,
            ProjectedSize {
                peak_bytes: 0,
                source: ProjectedSizeSource::CalibrationSamplingClosedForm,
            }
        );
        assert_eq!(
            serde_json::to_value(&report.report.body.projections.projected_hram)
                .expect("projected size serializes"),
            serde_json::json!({
                "peak_bytes": 16,
                "source": { "kind": "StaticGraphProjection" }
            })
        );
    }

    #[test]
    fn f_b4_budget_routing_model_named_in_report() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.routing = RoutingProjection {
            model: RoutingModelSection {
                kind: "top-2-deterministic-once-per-token".to_owned(),
            },
            projected_bank_switches_per_token: 7,
            expected_bank_switches_q16_16: Some(3 * 65_536),
        };
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report.report.body.projections.routing_model,
            RoutingModelSection {
                kind: "top-2-deterministic-once-per-token".to_owned(),
            }
        );
        assert_eq!(
            report
                .report
                .body
                .projections
                .projected_bank_switches_per_token,
            ProjectedSwitchCount {
                upper_bound: 7,
                expected_q16_16: Some(3 * 65_536),
                decision_value: 7,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }
        );
    }

    #[test]
    fn f_b4_budget_switch_decision_uses_upper_bound_by_default() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.routing.projected_bank_switches_per_token = 9;
        view.routing.expected_bank_switches_q16_16 = Some(2 * 65_536);
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report
                .report
                .body
                .projections
                .projected_bank_switches_per_token,
            ProjectedSwitchCount {
                upper_bound: 9,
                expected_q16_16: Some(2 * 65_536),
                decision_value: 9,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }
        );
    }

    #[test]
    fn f_b4_budget_recovery_switch_decision_uses_upper_bound() {
        let mut policy = policy_fixture();
        policy.policy.profile = gbf_foundation::CompileProfileId::from("Recovery");
        policy.policy.objective.max_bank_switches_per_token = Some(8);
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.routing.projected_bank_switches_per_token = 9;
        view.routing.expected_bank_switches_q16_16 = Some(65_536);
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_static_budget(BudgetInputs {
            policy: &policy,
            quant_graph: &quant_graph,
            runtime_chrome_budget: Some(&budget),
            target_profile: &dmg_mbc5_8mib_128kib(),
        });

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report
                .report
                .body
                .projections
                .projected_bank_switches_per_token,
            ProjectedSwitchCount {
                upper_bound: 9,
                expected_q16_16: Some(65_536),
                decision_value: 9,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value: 9,
                upper_bound: 9,
                cap: 8,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }]
        );
    }

    #[test]
    fn f_b4_budget_rejects_switches_per_token_over_cap() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.routing.model = RoutingModelSection {
            kind: "top-1-deterministic-once-per-token".to_owned(),
        };
        view.routing.projected_bank_switches_per_token = 18;
        view.routing.expected_bank_switches_q16_16 = Some(4 * 65_536);
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value: 18,
                upper_bound: 18,
                cap: 17,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: 18,
                upper_bound: 18,
                cap: 17,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }
        );
    }

    #[test]
    fn f_b4_budget_rejects_sram_page_switches_per_token_over_cap() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.sequence_state.projected_sram_bytes = 8192;
        view.sequence_state.projected_sram_source = ProjectedSizeSource::StaticGraphProjection;
        view.sequence_state.projected_sram_page_switches_per_token = 4;
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report
                .report
                .body
                .projections
                .projected_sram_page_switches_per_token,
            ProjectedSwitchCount {
                upper_bound: 4,
                expected_q16_16: None,
                decision_value: 4,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::SramPageSwitchesPerTokenOverCap {
                decision_value: 4,
                upper_bound: 4,
                cap: 3,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetSramPageSwitchesPerTokenOverCap {
                decision_value: 4,
                upper_bound: 4,
                cap: 3,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            }
        );
    }

    #[test]
    fn f_b4_budget_rejects_accumulator_exceeds_i32() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.reduction_sites = vec![ReductionSiteProjection {
            site: ReductionSiteId("site.i32.overflow".to_owned()),
            layer: Some(LayerId::new(0)),
            expert: Some(ExpertId::new(0)),
            term_count: i32::MAX as u32 + 1,
            input_max_abs_q: 1,
            weight_max_abs_q: 1,
            bias_max_abs_q: None,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }];
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.accumulator_maxima,
            vec![AccumulatorBound {
                site: ReductionSiteId("site.i32.overflow".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
                i16_safe: false,
                i32_safe: false,
            }]
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::AccumulatorExceedsI32 {
                site: ReductionSiteId("site.i32.overflow".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            }]
        );
    }

    #[test]
    fn f_b4_budget_records_i16_safe_for_range_plan() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.reduction_sites = vec![ReductionSiteProjection {
            site: ReductionSiteId("site.i16.range_plan".to_owned()),
            layer: Some(LayerId::new(0)),
            expert: Some(ExpertId::new(0)),
            term_count: 40_000,
            input_max_abs_q: 1,
            weight_max_abs_q: 1,
            bias_max_abs_q: None,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }];
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert!(report.report.body.decision.failures.is_empty());
        assert_eq!(
            report.report.body.projections.accumulator_maxima,
            vec![AccumulatorBound {
                site: ReductionSiteId("site.i16.range_plan".to_owned()),
                projected_max_abs: 40_000,
                i16_safe: false,
                i32_safe: true,
            }]
        );
    }

    #[test]
    fn f_b4_budget_hard_fail_on_i16_only_lock_with_unsafe_site() {
        let budget = runtime_budget_fixture();
        let mut policy = policy_fixture();
        policy
            .policy
            .knobs
            .locks
            .locked
            .insert(CompileKnobId::Range);
        policy.policy.knobs.global.range.reduction_ceiling = ReductionPlanCeiling::ExactOnly;
        let mut view = budget_view_fixture(4, 4, 0);
        view.reduction_sites = vec![ReductionSiteProjection {
            site: ReductionSiteId("site.i16.locked".to_owned()),
            layer: Some(LayerId::new(0)),
            expert: Some(ExpertId::new(0)),
            term_count: 40_000,
            input_max_abs_q: 1,
            weight_max_abs_q: 1,
            bias_max_abs_q: None,
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }];
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_static_budget(BudgetInputs {
            policy: &policy,
            quant_graph: &quant_graph,
            runtime_chrome_budget: Some(&budget),
            target_profile: &dmg_mbc5_8mib_128kib(),
        });

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.accumulator_maxima,
            vec![AccumulatorBound {
                site: ReductionSiteId("site.i16.locked".to_owned()),
                projected_max_abs: 40_000,
                i16_safe: false,
                i32_safe: true,
            }]
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::AccumulatorExceedsI32 {
                site: ReductionSiteId("site.i16.locked".to_owned()),
                projected_max_abs: 40_000,
            }]
        );
    }

    #[test]
    fn f_b4_budget_rejects_accumulator_projection_that_cannot_fit_u64() {
        let budget = runtime_budget_fixture();
        let mut view = budget_view_fixture(4, 4, 0);
        view.reduction_sites = vec![ReductionSiteProjection {
            site: ReductionSiteId("site.u64.overflow".to_owned()),
            layer: Some(LayerId::new(0)),
            expert: Some(ExpertId::new(0)),
            term_count: u32::MAX,
            input_max_abs_q: u32::MAX,
            weight_max_abs_q: u32::MAX,
            bias_max_abs_q: Some(u32::MAX),
            accumulator_domain: AccumulatorDomain::RawIntegerProducts,
        }];
        let quant_graph = TrackingQuantGraph::new(view);

        let report = run_budget_with(Some(&budget), &quant_graph);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert!(report.report.body.projections.accumulator_maxima.is_empty());
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.reduction_sites[0].projected_max_abs")
            }]
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
            Some(RuntimeChromeBudgetSection::from(&budget))
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
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].residual_bytes,
            -14
        );
        assert!(matches!(
            report.report.body.decision.failures[0],
            BudgetFailure::ExpertExceedsSlot { .. }
        ));
        assert_eq!(budget.rom_slots[0].usable_bytes, 8);
        assert_eq!(budget.rom_slots[0].reserved_slack, 16);
    }

    #[test]
    fn f_b4_budget_rejects_expert_exceeds_slot() {
        let policy = policy_with_placement(PlacementProfile::StrictOnePerBank);
        let budget = runtime_budget_with_slots(vec![expert_slot(
            7,
            16,
            0,
            [PlacementProfile::StrictOnePerBank],
        )]);
        let view = budget_view_with_experts(vec![expert_with_payload(0, 0, 20)]);

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].residual_bytes,
            -4
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![BudgetComponentRef::Expert {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
            }]
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].assigned_slot,
            Some(BudgetSlotId::new(7))
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].placement_status,
            ExpertPlacementStatus::AssignedOverCap
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::ExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: BudgetSlotId::new(7),
                payload_bytes: 20,
                cap_bytes: 16,
                excess_bytes: 4,
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                slot: BudgetSlotId::new(7),
                payload_bytes: 20,
                cap_bytes: 16,
                excess_bytes: 4,
            }
        );
        assert_eq!(
            serde_json::to_value(&report.report.body.diagnostics[0].code)
                .expect("diagnostic code serializes"),
            serde_json::json!({
                "kind": "BudgetExpertExceedsSlot",
                "fields": {
                    "layer": 0,
                    "expert": 0,
                    "slot": 7,
                    "payload_bytes": 20,
                    "cap_bytes": 16,
                    "excess_bytes": 4
                }
            })
        );
    }

    #[test]
    fn f_b4_budget_expert_exceeds_slot_reports_expert_payload_for_cumulative_bust() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget =
            runtime_budget_with_slots(vec![expert_slot(7, 25, 0, [PlacementProfile::Budgeted])]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 20),
            expert_with_payload(0, 1, 10),
        ]);

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].placement_status,
            ExpertPlacementStatus::Assigned
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[1].assigned_slot,
            Some(BudgetSlotId::new(7))
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[1].placement_status,
            ExpertPlacementStatus::AssignedOverCap
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_bytes,
            30
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].residual_bytes,
            -5
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![
                BudgetComponentRef::Expert {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(0),
                },
                BudgetComponentRef::Expert {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(1),
                },
            ]
        );
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::ExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                slot: BudgetSlotId::new(7),
                payload_bytes: 10,
                cap_bytes: 25,
                excess_bytes: 5,
            }]
        );
        assert_eq!(
            report.report.body.diagnostics[0].code,
            ValidationCode::BudgetExpertExceedsSlot {
                layer: LayerId::new(0),
                expert: ExpertId::new(1),
                slot: BudgetSlotId::new(7),
                payload_bytes: 10,
                cap_bytes: 25,
                excess_bytes: 5,
            }
        );
        assert_eq!(
            serde_json::to_value(
                report.report.body.projections.per_expert_payload[1].placement_status
            )
            .expect("placement status serializes"),
            serde_json::json!({"kind": "AssignedOverCap"})
        );
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
    fn f_b4_budget_static_placement_model_matches_pinned_mapping() {
        for (profile, expected_model) in [
            (
                PlacementProfile::StrictOnePerBank,
                StaticPlacementModel::StrictOnePerBank,
            ),
            (
                PlacementProfile::Budgeted,
                StaticPlacementModel::BudgetedFirstFit,
            ),
            (
                PlacementProfile::PackedExperts,
                StaticPlacementModel::PackedExpertsFirstFitDecreasing,
            ),
        ] {
            let mut policy = policy_with_placement(profile);
            policy.policy.observability = ObservabilityMode::Flexible;
            assert_eq!(placement_model_for_profile(profile), expected_model);

            let budget = runtime_budget_with_slots(vec![expert_slot(
                1,
                64,
                0,
                [
                    PlacementProfile::StrictOnePerBank,
                    PlacementProfile::Budgeted,
                    PlacementProfile::PackedExperts,
                ],
            )]);
            let report = run_budget_with_policy(&policy, &budget, budget_view_fixture(1, 1, 3));

            assert_eq!(report.report.body.decision.placement_model, expected_model);
        }
    }

    #[test]
    fn f_b4_budget_strict_one_per_bank_deterministic() {
        let policy = policy_with_placement(PlacementProfile::StrictOnePerBank);
        let budget = runtime_budget_with_slots(vec![
            expert_slot(2, 32, 0, [PlacementProfile::StrictOnePerBank]),
            expert_slot(1, 32, 0, [PlacementProfile::StrictOnePerBank]),
        ]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 10),
            expert_with_payload(0, 1, 10),
        ]);

        let first = run_budget_with_policy(&policy, &budget, view.clone());
        let second = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(first.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            first.static_budget_canonical_bytes_hash,
            second.static_budget_canonical_bytes_hash
        );
        assert_eq!(
            first
                .report
                .body
                .projections
                .per_expert_payload
                .iter()
                .map(|entry| entry.assigned_slot)
                .collect::<Vec<_>>(),
            vec![Some(BudgetSlotId::new(1)), Some(BudgetSlotId::new(2))]
        );
    }

    #[test]
    fn f_b4_budget_budgeted_first_fit_is_byte_stable() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            expert_slot(1, 25, 0, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 20),
            expert_with_payload(0, 1, 10),
        ]);

        let first = run_budget_with_policy(&policy, &budget, view.clone());
        let second = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(first.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            first.static_budget_canonical_bytes_hash,
            second.static_budget_canonical_bytes_hash
        );
        assert_eq!(
            first
                .report
                .body
                .projections
                .per_expert_payload
                .iter()
                .map(|entry| entry.assigned_slot)
                .collect::<Vec<_>>(),
            vec![Some(BudgetSlotId::new(1)), Some(BudgetSlotId::new(2))]
        );
        assert_eq!(
            first.report.body.projections.per_bank_occupancy[0].assigned_bytes,
            20
        );
        assert_eq!(
            first.report.body.projections.per_bank_occupancy[1].assigned_bytes,
            10
        );
    }

    #[test]
    fn f_b4_budget_emits_canonical_json() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            expert_slot(1, 20, 0, [PlacementProfile::Budgeted]),
            common_slot(2, 64, [PlacementProfile::Budgeted]),
        ]);
        let report = run_budget_with_policy(
            &policy,
            &budget,
            budget_view_with_experts(vec![expert_with_payload(0, 0, 10)]),
        );

        let canonical = canonicalize_report(&report.report).expect("report canonicalizes");
        let decoded: ReportEnvelope<StaticBudgetReportBody> =
            serde_json::from_slice(&canonical).expect("canonical report decodes");
        let value = serde_json::to_value(&decoded).expect("decoded report serializes");

        assert_eq!(decoded.outcome, ReportOutcome::Passed);
        assert_eq!(value["schema"], serde_json::json!("static_budget.v1"));
        assert!(value["runtime_chrome_budget"].is_object());
        assert!(
            value["projections"]["per_expert_payload"][0]
                .get("assigned_slot")
                .is_some()
        );
        assert!(
            value["projections"]["per_expert_payload"][0]
                .get("unassigned_because")
                .is_none()
        );
        assert_eq!(
            Hash256::from_bytes(Sha256::digest(&canonical).into()),
            report.static_budget_canonical_bytes_hash
        );
    }

    #[test]
    fn f_b4_budget_is_deterministic_for_same_inputs() {
        let policy = policy_with_placement(PlacementProfile::PackedExperts);
        let budget = runtime_budget_with_slots(vec![
            expert_slot(1, 25, 0, [PlacementProfile::PackedExperts]),
            expert_slot(2, 25, 0, [PlacementProfile::PackedExperts]),
            common_slot(3, 64, [PlacementProfile::PackedExperts]),
        ]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 11),
            expert_with_payload(0, 1, 10),
        ]);

        let first = run_budget_with_policy(&policy, &budget, view.clone());
        let second = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(first.report, second.report);
        assert_eq!(
            first.static_budget_canonical_bytes_hash,
            second.static_budget_canonical_bytes_hash
        );
        assert_eq!(
            canonicalize_report(&first.report).expect("first canonicalizes"),
            canonicalize_report(&second.report).expect("second canonicalizes")
        );
    }

    #[test]
    fn f_b4_budget_packed_experts_descending_then_layer_expert() {
        let policy = policy_with_placement(PlacementProfile::PackedExperts);
        let budget = runtime_budget_with_slots(vec![
            expert_slot(1, 50, 0, [PlacementProfile::PackedExperts]),
            expert_slot(2, 50, 0, [PlacementProfile::PackedExperts]),
        ]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 20),
            expert_with_payload(0, 1, 30),
            expert_with_payload(1, 0, 30),
        ]);

        let first = run_budget_with_policy(&policy, &budget, view.clone());
        let second = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(first.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            first.static_budget_canonical_bytes_hash,
            second.static_budget_canonical_bytes_hash
        );
        assert_eq!(
            first.report.body.projections.per_expert_payload[0].assigned_slot,
            Some(BudgetSlotId::new(1))
        );
        assert_eq!(
            first.report.body.projections.per_expert_payload[1].assigned_slot,
            Some(BudgetSlotId::new(1))
        );
        assert_eq!(
            first.report.body.projections.per_expert_payload[2].assigned_slot,
            Some(BudgetSlotId::new(2))
        );
        assert_eq!(
            first.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![
                BudgetComponentRef::Expert {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(1),
                },
                BudgetComponentRef::Expert {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(0),
                },
            ]
        );
    }

    #[test]
    fn f_b4_budget_rejects_placement_profile_infeasible() {
        let policy = policy_with_placement(PlacementProfile::PackedExperts);
        let budget = runtime_budget_with_slots(vec![expert_slot(
            1,
            64,
            0,
            [
                PlacementProfile::StrictOnePerBank,
                PlacementProfile::Budgeted,
            ],
        )]);

        let report = run_budget_with_policy(&policy, &budget, budget_view_fixture(1, 1, 3));

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::PlacementProfileInfeasible {
                profile: PlacementProfile::PackedExperts,
                reason: PlacementInfeasibilityReason::NoSlotsForClass,
            }]
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].assigned_slot,
            None
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].placement_status,
            ExpertPlacementStatus::UnassignedNoEligibleSlots
        );
    }

    #[test]
    fn f_b4_budget_unassigned_strict_expert_records_reason() {
        let policy = policy_with_placement(PlacementProfile::StrictOnePerBank);
        let budget = runtime_budget_with_slots(vec![expert_slot(
            1,
            64,
            0,
            [PlacementProfile::StrictOnePerBank],
        )]);
        let view = budget_view_with_experts(vec![
            expert_with_payload(0, 0, 10),
            expert_with_payload(0, 1, 10),
        ]);

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::PlacementProfileInfeasible {
                profile: PlacementProfile::StrictOnePerBank,
                reason: PlacementInfeasibilityReason::ExpertCountExceedsSlots,
            }]
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[1].assigned_slot,
            None
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[1].placement_status,
            ExpertPlacementStatus::UnassignedStrictDistinctSlotsExhausted
        );
    }

    #[test]
    fn f_b4_budget_common_payload_uses_common_bank_eligible_slots() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            bank0_slot(0, 128, [PlacementProfile::Budgeted]),
            common_slot(1, 32, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.shared"),
            bytes: 12,
            bank0_compatible: false,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.shared".to_owned(),
            bytes: 8,
            bank0_compatible: false,
        }];

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert!(
            report.report.body.projections.per_bank_occupancy[0]
                .assigned_components
                .is_empty()
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[1].assigned_components,
            vec![
                BudgetComponentRef::SharedKernel {
                    id: KernelSpecId::from("kernel.shared"),
                },
                BudgetComponentRef::SharedLut {
                    id: "lut.shared".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn f_b4_budget_includes_shared_dense_ffn_in_common_bank_footprint() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            common_slot(1, 64, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.shared"),
            bytes: 12,
            bank0_compatible: false,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.shared".to_owned(),
            bytes: 8,
            bank0_compatible: false,
        }];
        view.shared_dense_ffn = Some(SharedDenseFfnProjection {
            bytes: 21,
            bank0_compatible: false,
        });

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report.report.body.projections.common_bank_footprint,
            CommonBankFootprintSection {
                kernel_bytes: 12,
                lut_bytes: 8,
                shared_dense_ffn_bytes: Some(21),
                aggregate_bytes: 41,
            }
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![
                BudgetComponentRef::SharedKernel {
                    id: KernelSpecId::from("kernel.shared"),
                },
                BudgetComponentRef::SharedLut {
                    id: "lut.shared".to_owned(),
                },
                BudgetComponentRef::SharedDenseFfn,
            ]
        );
        assert_eq!(
            report.report.body.projections.per_expert_payload[0].payload_bytes,
            6
        );

        let mut no_shared_dense_view = budget_view_fixture(1, 1, 3);
        no_shared_dense_view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.shared"),
            bytes: 12,
            bank0_compatible: false,
        }];
        no_shared_dense_view.shared_luts = vec![SharedLutProjection {
            id: "lut.shared".to_owned(),
            bytes: 8,
            bank0_compatible: false,
        }];
        let no_shared_dense_report = run_budget_with_policy(&policy, &budget, no_shared_dense_view);

        assert_eq!(no_shared_dense_report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            no_shared_dense_report
                .report
                .body
                .projections
                .common_bank_footprint,
            CommonBankFootprintSection {
                kernel_bytes: 12,
                lut_bytes: 8,
                shared_dense_ffn_bytes: None,
                aggregate_bytes: 20,
            }
        );
        assert_eq!(
            serde_json::to_value(
                &no_shared_dense_report
                    .report
                    .body
                    .projections
                    .common_bank_footprint
            )
            .expect("common footprint serializes"),
            serde_json::json!({
                "kernel_bytes": 12,
                "lut_bytes": 8,
                "shared_dense_ffn_bytes": null,
                "aggregate_bytes": 20
            })
        );
    }

    #[test]
    fn f_b4_budget_rejects_common_bank_exceeds_cap() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            common_slot(1, 10, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.shared"),
            bytes: 7,
            bank0_compatible: false,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.shared".to_owned(),
            bytes: 6,
            bank0_compatible: false,
        }];
        view.shared_dense_ffn = Some(SharedDenseFfnProjection {
            bytes: 5,
            bank0_compatible: false,
        });

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: 18,
                cap_bytes: 10,
                excess_bytes: 8,
            }]
        );
        assert_eq!(
            report
                .report
                .body
                .projections
                .common_bank_footprint
                .aggregate_bytes,
            18
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_bytes,
            18
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![
                BudgetComponentRef::SharedKernel {
                    id: KernelSpecId::from("kernel.shared"),
                },
                BudgetComponentRef::SharedLut {
                    id: "lut.shared".to_owned(),
                },
                BudgetComponentRef::SharedDenseFfn,
            ]
        );
    }

    #[test]
    fn f_b4_budget_common_fragmentation_reports_positive_excess() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            common_slot(1, 10, [PlacementProfile::Budgeted]),
            common_slot(2, 10, [PlacementProfile::Budgeted]),
            expert_slot(3, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.fragmented"),
            bytes: 15,
            bank0_compatible: false,
        }];

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: 15,
                cap_bytes: 10,
                excess_bytes: 5,
            }]
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_bytes,
            15
        );
    }

    #[test]
    fn f_b4_budget_common_failure_excludes_bank0_bytes_from_common_only_cap() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            bank0_slot(0, 8, [PlacementProfile::Budgeted]),
            common_slot(1, 10, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.bank0.ok"),
            bytes: 8,
            bank0_compatible: true,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.common.only".to_owned(),
            bytes: 8,
            bank0_compatible: false,
        }];
        view.shared_dense_ffn = Some(SharedDenseFfnProjection {
            bytes: 5,
            bank0_compatible: false,
        });

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: 13,
                cap_bytes: 10,
                excess_bytes: 3,
            }]
        );
        assert_eq!(
            report.report.body.projections.common_bank_footprint,
            CommonBankFootprintSection {
                kernel_bytes: 8,
                lut_bytes: 8,
                shared_dense_ffn_bytes: Some(5),
                aggregate_bytes: 21,
            }
        );
    }

    #[test]
    fn f_b4_budget_bank0_compatible_failure_reports_slot_domain_cap() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            bank0_slot(0, 10, [PlacementProfile::Budgeted]),
            common_slot(1, 10, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.bank0.ok"),
            bytes: 8,
            bank0_compatible: true,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.common.full".to_owned(),
            bytes: 10,
            bank0_compatible: false,
        }];
        view.shared_dense_ffn = Some(SharedDenseFfnProjection {
            bytes: 5,
            bank0_compatible: true,
        });

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: 13,
                cap_bytes: 10,
                excess_bytes: 3,
            }]
        );
        assert_eq!(
            report
                .report
                .body
                .projections
                .common_bank_footprint
                .aggregate_bytes,
            23
        );
    }

    #[test]
    fn f_b4_budget_aggregate_bytes_equals_sum_of_components() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            common_slot(1, 64, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.shared"),
            bytes: 14,
            bank0_compatible: false,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.shared".to_owned(),
            bytes: 9,
            bank0_compatible: false,
        }];
        view.shared_dense_ffn = Some(SharedDenseFfnProjection {
            bytes: 6,
            bank0_compatible: false,
        });

        let report = run_budget_with_policy(&policy, &budget, view);
        let footprint = &report.report.body.projections.common_bank_footprint;

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            *footprint,
            CommonBankFootprintSection {
                kernel_bytes: 14,
                lut_bytes: 9,
                shared_dense_ffn_bytes: Some(6),
                aggregate_bytes: 29,
            }
        );
    }

    #[test]
    fn f_b4_budget_common_footprint_overflow_is_malformed_view() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            common_slot(1, 4096, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.max"),
            bytes: u32::MAX,
            bank0_compatible: false,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.one".to_owned(),
            bytes: 1,
            bank0_compatible: false,
        }];

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            report.report.body.decision.failures,
            vec![BudgetFailure::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.common_bank_footprint.aggregate_bytes"),
            }]
        );
    }

    #[test]
    fn f_b4_budget_common_payload_respects_bank0_compatibility_flags() {
        let policy = policy_with_placement(PlacementProfile::Budgeted);
        let budget = runtime_budget_with_slots(vec![
            bank0_slot(0, 12, [PlacementProfile::Budgeted]),
            common_slot(1, 8, [PlacementProfile::Budgeted]),
            expert_slot(2, 64, 0, [PlacementProfile::Budgeted]),
        ]);
        let mut view = budget_view_fixture(1, 1, 3);
        view.shared_kernels = vec![SharedKernelProjection {
            id: KernelSpecId::from("kernel.bank0.ok"),
            bytes: 12,
            bank0_compatible: true,
        }];
        view.shared_luts = vec![SharedLutProjection {
            id: "lut.common.only".to_owned(),
            bytes: 8,
            bank0_compatible: false,
        }];

        let report = run_budget_with_policy(&policy, &budget, view);

        assert_eq!(report.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[0].assigned_components,
            vec![BudgetComponentRef::SharedKernel {
                id: KernelSpecId::from("kernel.bank0.ok"),
            }]
        );
        assert_eq!(
            report.report.body.projections.per_bank_occupancy[1].assigned_components,
            vec![BudgetComponentRef::SharedLut {
                id: "lut.common.only".to_owned(),
            }]
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
                excess_bytes: 872,
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
