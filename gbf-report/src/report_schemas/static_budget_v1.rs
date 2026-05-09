//! `static_budget.v1` Stage 2 report schema.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use gbf_foundation::{
    BudgetSlotId, CompileProfileId, ExpertId, FieldPath, Hash256, KernelSpecId, LayerId,
    TargetProfileId,
};
pub use gbf_policy::StaticFitInterpretation;
use gbf_policy::{
    BudgetFailure, BudgetSlotClass, DiagnosticSeverity, EvidenceRef, PlacementProfile,
    ReductionSiteId, RomBudgetSlot, RuntimeChromeBudget, SwitchProjectionSource, ValidationCode,
    ValidationDetail, ValidationDiagnostic, ValidationOrigin, budget_failure_diagnostic,
    budget_failure_diagnostic_with_provenance, budget_failure_matches_diagnostic,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ReportBody, ReportOutcome, canonicalize_value};

pub const SCHEMA_ID: &str = "static_budget.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

pub type BudgetFailureRecord = BudgetFailure;
pub type ValidationDiagnosticRecord = ValidationDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaticBudgetReportBody {
    pub identity: BudgetIdentitySection,
    pub policy: BudgetPolicySection,
    pub runtime_chrome_budget: Option<RuntimeChromeBudgetSection>,
    pub projections: BudgetProjectionSection,
    pub decision: BudgetDecisionSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

impl ReportBody for StaticBudgetReportBody {
    const REPORT_TYPE: &'static str = "StaticBudgetReport";
    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let mut errors = Vec::new();

        validate_missing_budget_shape(self, outcome, &mut errors);
        validate_decision(self, outcome, &mut errors);
        validate_diagnostics(self, &mut errors);
        validate_projection_order(&self.projections, &mut errors);
        validate_projection_arithmetic(&self.projections, &mut errors);

        if let (Some(section), Some(expected_hash)) = (
            self.runtime_chrome_budget.as_ref(),
            self.identity.runtime_chrome_budget_hash,
        ) {
            match runtime_chrome_budget_hash(section) {
                Ok(observed_hash) if observed_hash == expected_hash => {}
                _ => errors.push(semantic_error("identity.runtime_chrome_budget_hash")),
            }
            validate_runtime_chrome_budget(section, &self.projections, &mut errors);
        }

        if contains_missing_runtime_chrome_budget_failure(&self.decision.failures) {
            validate_missing_budget_has_no_view_payload(&self.projections, &mut errors);
        } else {
            validate_expert_assignment_invariants(
                &self.projections,
                self.decision.fits,
                &self.decision.failures,
                &mut errors,
            );
        }

        validate_switch_failures(&self.projections, &self.decision.failures, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
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
pub struct RuntimeChromeBudgetSection {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub runtime_nucleus_hash: Hash256,
    pub rom_slots: Vec<RomBudgetSlotEntry>,
    pub memory_caps: RuntimeMemoryCapSection,
    pub wram_reserved: u16,
    pub sram_reserved: u32,
}

impl From<&RuntimeChromeBudget> for RuntimeChromeBudgetSection {
    fn from(value: &RuntimeChromeBudget) -> Self {
        let mut rom_slots = value
            .rom_slots
            .iter()
            .map(RomBudgetSlotEntry::from)
            .collect::<Vec<_>>();
        rom_slots.sort_by_key(|slot| slot.id);
        Self {
            target: value.target.clone(),
            profile: value.profile.clone(),
            runtime_nucleus_hash: value.runtime_nucleus_hash,
            rom_slots,
            memory_caps: RuntimeMemoryCapSection::from(&value.memory_caps),
            wram_reserved: value.wram_reserved,
            sram_reserved: value.sram_reserved,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomBudgetSlotEntry {
    pub id: BudgetSlotId,
    pub class: BudgetSlotClass,
    pub usable_bytes: u32,
    pub reserved_slack: u16,
    pub placement_caps: BTreeSet<PlacementProfile>,
}

impl From<&RomBudgetSlot> for RomBudgetSlotEntry {
    fn from(value: &RomBudgetSlot) -> Self {
        Self {
            id: value.id,
            class: value.class,
            usable_bytes: value.usable_bytes,
            reserved_slack: value.reserved_slack,
            placement_caps: value.placement_caps.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMemoryCapSection {
    pub wram_usable_bytes: u32,
    pub sram_usable_bytes: u32,
    pub hram_usable_bytes: u32,
    pub source_target_profile_hash: Hash256,
}

impl From<&gbf_policy::RuntimeMemoryCapSection> for RuntimeMemoryCapSection {
    fn from(value: &gbf_policy::RuntimeMemoryCapSection) -> Self {
        Self {
            wram_usable_bytes: value.wram_usable_bytes,
            sram_usable_bytes: value.sram_usable_bytes,
            hram_usable_bytes: value.hram_usable_bytes,
            source_target_profile_hash: value.source_target_profile_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetProjectionSection {
    pub per_expert_payload: Vec<PerExpertEntry>,
    pub per_bank_occupancy: Vec<PerBankEntry>,
    pub common_bank_footprint: CommonBankFootprintSection,
    pub accumulator_maxima: Vec<AccumulatorBound>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_slot: Option<BudgetSlotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unassigned_because: Option<UnassignedBecause>,
    pub placement_status: ExpertPlacementStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ExpertPlacementStatus {
    Assigned,
    AssignedOverCap,
    UnassignedNoEligibleSlots,
    UnassignedStrictDistinctSlotsExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum UnassignedBecause {
    NoEligibleSlots,
    StrictDistinctSlotsExhausted,
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
    pub residual_bytes: i32,
    pub assigned_components: Vec<BudgetComponentRef>,
    pub placement_caps: BTreeSet<PlacementProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BudgetComponentRef {
    Expert { layer: LayerId, expert: ExpertId },
    SharedKernel { id: KernelSpecId },
    SharedLut { id: String },
    SharedDenseFfn,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonBankFootprintSection {
    pub kernel_bytes: u32,
    pub lut_bytes: u32,
    pub shared_dense_ffn_bytes: Option<u32>,
    pub aggregate_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccumulatorBound {
    pub site: ReductionSiteId,
    pub projected_max_abs: u64,
    pub i16_safe: bool,
    pub i32_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedSize {
    pub peak_bytes: u32,
    pub source: ProjectedSizeSource,
}

pub type ProjectedSizeSection = ProjectedSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProjectedSizeSource {
    #[default]
    StaticGraphProjection,
    HintBundleConstraint,
    CalibrationSamplingClosedForm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedSwitchCount {
    pub upper_bound: u16,
    pub expected_q16_16: Option<u32>,
    pub decision_value: u16,
    pub source: SwitchProjectionSource,
}

pub type ProjectedSwitchCountSection = ProjectedSwitchCount;

impl Default for ProjectedSwitchCount {
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
    pub failures: Vec<BudgetFailureRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StaticPlacementModel {
    StrictOnePerBank,
    BudgetedFirstFit,
    PackedExpertsFirstFitDecreasing,
}

impl StaticPlacementModel {
    #[must_use]
    pub const fn for_profile(profile: PlacementProfile) -> Self {
        match profile {
            PlacementProfile::StrictOnePerBank => Self::StrictOnePerBank,
            PlacementProfile::Budgeted => Self::BudgetedFirstFit,
            PlacementProfile::PackedExperts => Self::PackedExpertsFirstFitDecreasing,
        }
    }
}

#[must_use]
pub const fn static_fit_interpretation_for_fits(fits: bool) -> StaticFitInterpretation {
    if fits {
        StaticFitInterpretation::PassesNecessaryStaticChecks
    } else {
        StaticFitInterpretation::FailsNecessaryStaticChecks
    }
}

#[must_use]
pub const fn decision_interpretation_matches_fits(
    fits: bool,
    interpretation: StaticFitInterpretation,
) -> bool {
    matches!(
        (fits, interpretation),
        (true, StaticFitInterpretation::PassesNecessaryStaticChecks)
            | (false, StaticFitInterpretation::FailsNecessaryStaticChecks)
    )
}

#[must_use]
pub fn diagnostics_for_budget_failures(
    failures: &[BudgetFailureRecord],
) -> Vec<ValidationDiagnosticRecord> {
    failures.iter().map(budget_failure_diagnostic).collect()
}

#[must_use]
pub fn diagnostics_for_budget_failures_with_provenance(
    failures: &[BudgetFailureRecord],
    provenance: Vec<EvidenceRef>,
) -> Vec<ValidationDiagnosticRecord> {
    failures
        .iter()
        .map(|failure| budget_failure_diagnostic_with_provenance(failure, provenance.clone()))
        .collect()
}

#[must_use]
pub fn failure_diagnostics_are_one_to_one(
    failures: &[BudgetFailureRecord],
    diagnostics: &[ValidationDiagnosticRecord],
) -> bool {
    if failures.len() != diagnostics.len() {
        return false;
    }

    let mut matched_diagnostics = vec![false; diagnostics.len()];
    failures.iter().all(|failure| {
        let Some((index, _)) = diagnostics.iter().enumerate().find(|(index, diagnostic)| {
            !matched_diagnostics[*index] && budget_failure_matches_diagnostic(failure, diagnostic)
        }) else {
            return false;
        };
        matched_diagnostics[index] = true;
        true
    })
}

pub fn sort_budget_failures_canonically(failures: &mut [BudgetFailureRecord]) {
    failures.sort_by(cmp_budget_failures);
}

fn is_canonically_ordered_by<T>(values: &[T], mut cmp: impl FnMut(&T, &T) -> Ordering) -> bool {
    values
        .windows(2)
        .all(|pair| cmp(&pair[0], &pair[1]) != Ordering::Greater)
}

fn cmp_budget_failures(left: &BudgetFailure, right: &BudgetFailure) -> Ordering {
    budget_failure_rank(left)
        .cmp(&budget_failure_rank(right))
        .then_with(|| cmp_budget_failure_fields(left, right))
}

fn budget_failure_rank(failure: &BudgetFailure) -> u8 {
    match failure {
        BudgetFailure::MissingRuntimeChromeBudget => 0,
        BudgetFailure::QuantGraphBudgetViewMalformed { .. } => 1,
        BudgetFailure::ExpertExceedsSlot { .. } => 2,
        BudgetFailure::CommonBankExceedsCap { .. } => 3,
        BudgetFailure::WramPeakExceedsCap { .. } => 4,
        BudgetFailure::SramPeakExceedsCap { .. } => 5,
        BudgetFailure::HramPeakExceedsCap { .. } => 6,
        BudgetFailure::AccumulatorExceedsI32 { .. } => 7,
        BudgetFailure::BankSwitchesPerTokenOverCap { .. } => 8,
        BudgetFailure::SramPageSwitchesPerTokenOverCap { .. } => 9,
        BudgetFailure::PlacementProfileInfeasible { .. } => 10,
    }
}

fn cmp_budget_failure_fields(left: &BudgetFailure, right: &BudgetFailure) -> Ordering {
    match (left, right) {
        (
            BudgetFailure::QuantGraphBudgetViewMalformed { field: left },
            BudgetFailure::QuantGraphBudgetViewMalformed { field: right },
        ) => left.cmp(right),
        (
            BudgetFailure::ExpertExceedsSlot {
                layer: left_layer,
                expert: left_expert,
                slot: left_slot,
                payload_bytes: left_payload,
                cap_bytes: left_cap,
                excess_bytes: left_excess,
            },
            BudgetFailure::ExpertExceedsSlot {
                layer: right_layer,
                expert: right_expert,
                slot: right_slot,
                payload_bytes: right_payload,
                cap_bytes: right_cap,
                excess_bytes: right_excess,
            },
        ) => (
            left_layer,
            left_expert,
            left_slot,
            left_payload,
            left_cap,
            left_excess,
        )
            .cmp(&(
                right_layer,
                right_expert,
                right_slot,
                right_payload,
                right_cap,
                right_excess,
            )),
        (
            BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: left_assigned,
                cap_bytes: left_cap,
                excess_bytes: left_excess,
            },
            BudgetFailure::CommonBankExceedsCap {
                assigned_bytes: right_assigned,
                cap_bytes: right_cap,
                excess_bytes: right_excess,
            },
        ) => (left_assigned, left_cap, left_excess).cmp(&(right_assigned, right_cap, right_excess)),
        (
            BudgetFailure::WramPeakExceedsCap {
                peak: left_peak,
                cap: left_cap,
            },
            BudgetFailure::WramPeakExceedsCap {
                peak: right_peak,
                cap: right_cap,
            },
        )
        | (
            BudgetFailure::SramPeakExceedsCap {
                peak: left_peak,
                cap: left_cap,
            },
            BudgetFailure::SramPeakExceedsCap {
                peak: right_peak,
                cap: right_cap,
            },
        )
        | (
            BudgetFailure::HramPeakExceedsCap {
                peak: left_peak,
                cap: left_cap,
            },
            BudgetFailure::HramPeakExceedsCap {
                peak: right_peak,
                cap: right_cap,
            },
        ) => (left_peak, left_cap).cmp(&(right_peak, right_cap)),
        (
            BudgetFailure::AccumulatorExceedsI32 {
                site: left_site,
                projected_max_abs: left_projected,
            },
            BudgetFailure::AccumulatorExceedsI32 {
                site: right_site,
                projected_max_abs: right_projected,
            },
        ) => (left_site, left_projected).cmp(&(right_site, right_projected)),
        (
            BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value: left_decision,
                upper_bound: left_upper,
                cap: left_cap,
                source: left_source,
            },
            BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value: right_decision,
                upper_bound: right_upper,
                cap: right_cap,
                source: right_source,
            },
        )
        | (
            BudgetFailure::SramPageSwitchesPerTokenOverCap {
                decision_value: left_decision,
                upper_bound: left_upper,
                cap: left_cap,
                source: left_source,
            },
            BudgetFailure::SramPageSwitchesPerTokenOverCap {
                decision_value: right_decision,
                upper_bound: right_upper,
                cap: right_cap,
                source: right_source,
            },
        ) => (left_decision, left_upper, left_cap, left_source).cmp(&(
            right_decision,
            right_upper,
            right_cap,
            right_source,
        )),
        (
            BudgetFailure::PlacementProfileInfeasible {
                profile: left_profile,
                reason: left_reason,
            },
            BudgetFailure::PlacementProfileInfeasible {
                profile: right_profile,
                reason: right_reason,
            },
        ) => (left_profile, placement_reason_rank(left_reason))
            .cmp(&(right_profile, placement_reason_rank(right_reason))),
        _ => Ordering::Equal,
    }
}

fn placement_reason_rank(reason: &gbf_policy::PlacementInfeasibilityReason) -> u8 {
    match reason {
        gbf_policy::PlacementInfeasibilityReason::NoSlotsForClass => 0,
        gbf_policy::PlacementInfeasibilityReason::ExpertCountExceedsSlots => 1,
        gbf_policy::PlacementInfeasibilityReason::RequiresUnavailableSlotClass => 2,
        gbf_policy::PlacementInfeasibilityReason::ExceedsCommonBankCap => 3,
        gbf_policy::PlacementInfeasibilityReason::ExceedsExpertBankCap => 4,
        gbf_policy::PlacementInfeasibilityReason::ViolatesTargetLayout => 5,
    }
}

fn cmp_diagnostics(left: &ValidationDiagnostic, right: &ValidationDiagnostic) -> Ordering {
    validation_code_rank(&left.code)
        .cmp(&validation_code_rank(&right.code))
        .then_with(|| {
            validation_origin_rank(left.origin).cmp(&validation_origin_rank(right.origin))
        })
        .then_with(|| cmp_validation_codes(&left.code, &right.code))
        .then_with(|| {
            diagnostic_detail_key(&left.detail).cmp(&diagnostic_detail_key(&right.detail))
        })
        .then_with(|| left.provenance.cmp(&right.provenance))
}

fn validation_origin_rank(origin: ValidationOrigin) -> u8 {
    match origin {
        ValidationOrigin::Schema => 0,
        ValidationOrigin::SemanticCore => 1,
        ValidationOrigin::Manifest => 2,
        ValidationOrigin::Lowering => 3,
        ValidationOrigin::Calibration => 4,
        ValidationOrigin::HintBundle => 5,
        ValidationOrigin::Workload => 6,
        ValidationOrigin::GoldenVector => 7,
        ValidationOrigin::CompileRequest => 8,
        ValidationOrigin::PolicyResolution => 9,
        ValidationOrigin::Budget => 10,
    }
}

fn validation_code_rank(code: &ValidationCode) -> u8 {
    match code {
        ValidationCode::BudgetMissingRuntimeChromeBudget => 0,
        ValidationCode::BudgetQuantGraphViewMalformed { .. } => 1,
        ValidationCode::BudgetExpertExceedsSlot { .. } => 2,
        ValidationCode::BudgetCommonBankExceedsCap { .. } => 3,
        ValidationCode::BudgetWramPeakExceeds { .. } => 4,
        ValidationCode::BudgetSramPeakExceeds { .. } => 5,
        ValidationCode::BudgetHramPeakExceeds { .. } => 6,
        ValidationCode::BudgetAccumulatorOverflow { .. } => 7,
        ValidationCode::BudgetSwitchesPerTokenOverCap { .. } => 8,
        ValidationCode::BudgetSramPageSwitchesPerTokenOverCap { .. } => 9,
        ValidationCode::BudgetPlacementProfileInfeasible { .. } => 10,
        _ => 100,
    }
}

fn cmp_validation_codes(left: &ValidationCode, right: &ValidationCode) -> Ordering {
    match (left, right) {
        (
            ValidationCode::BudgetQuantGraphViewMalformed { field: left },
            ValidationCode::BudgetQuantGraphViewMalformed { field: right },
        ) => left.cmp(right),
        (
            ValidationCode::BudgetExpertExceedsSlot {
                layer: left_layer,
                expert: left_expert,
                slot: left_slot,
                payload_bytes: left_payload,
                cap_bytes: left_cap,
                excess_bytes: left_excess,
            },
            ValidationCode::BudgetExpertExceedsSlot {
                layer: right_layer,
                expert: right_expert,
                slot: right_slot,
                payload_bytes: right_payload,
                cap_bytes: right_cap,
                excess_bytes: right_excess,
            },
        ) => (
            left_layer,
            left_expert,
            left_slot,
            left_payload,
            left_cap,
            left_excess,
        )
            .cmp(&(
                right_layer,
                right_expert,
                right_slot,
                right_payload,
                right_cap,
                right_excess,
            )),
        (
            ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: left_assigned,
                cap_bytes: left_cap,
            },
            ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: right_assigned,
                cap_bytes: right_cap,
            },
        ) => (left_assigned, left_cap).cmp(&(right_assigned, right_cap)),
        (
            ValidationCode::BudgetWramPeakExceeds {
                peak: left_peak,
                cap: left_cap,
            },
            ValidationCode::BudgetWramPeakExceeds {
                peak: right_peak,
                cap: right_cap,
            },
        )
        | (
            ValidationCode::BudgetSramPeakExceeds {
                peak: left_peak,
                cap: left_cap,
            },
            ValidationCode::BudgetSramPeakExceeds {
                peak: right_peak,
                cap: right_cap,
            },
        )
        | (
            ValidationCode::BudgetHramPeakExceeds {
                peak: left_peak,
                cap: left_cap,
            },
            ValidationCode::BudgetHramPeakExceeds {
                peak: right_peak,
                cap: right_cap,
            },
        ) => (left_peak, left_cap).cmp(&(right_peak, right_cap)),
        (
            ValidationCode::BudgetAccumulatorOverflow {
                site: left_site,
                projected_max_abs: left_projected,
            },
            ValidationCode::BudgetAccumulatorOverflow {
                site: right_site,
                projected_max_abs: right_projected,
            },
        ) => (left_site, left_projected).cmp(&(right_site, right_projected)),
        (
            ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: left_decision,
                upper_bound: left_upper,
                cap: left_cap,
                source: left_source,
            },
            ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: right_decision,
                upper_bound: right_upper,
                cap: right_cap,
                source: right_source,
            },
        )
        | (
            ValidationCode::BudgetSramPageSwitchesPerTokenOverCap {
                decision_value: left_decision,
                upper_bound: left_upper,
                cap: left_cap,
                source: left_source,
            },
            ValidationCode::BudgetSramPageSwitchesPerTokenOverCap {
                decision_value: right_decision,
                upper_bound: right_upper,
                cap: right_cap,
                source: right_source,
            },
        ) => (left_decision, left_upper, left_cap, left_source).cmp(&(
            right_decision,
            right_upper,
            right_cap,
            right_source,
        )),
        (
            ValidationCode::BudgetPlacementProfileInfeasible {
                profile: left_profile,
                reason: left_reason,
            },
            ValidationCode::BudgetPlacementProfileInfeasible {
                profile: right_profile,
                reason: right_reason,
            },
        ) => (left_profile, placement_reason_rank(left_reason))
            .cmp(&(right_profile, placement_reason_rank(right_reason))),
        _ => format!("{left:?}").cmp(&format!("{right:?}")),
    }
}

fn diagnostic_detail_key(detail: &ValidationDetail) -> String {
    match detail {
        ValidationDetail::Selector { selector } => selector.0.clone(),
        ValidationDetail::Field { field } => field.to_string(),
        _ => format!("{detail:?}"),
    }
}

pub fn runtime_chrome_budget_hash(
    budget: &RuntimeChromeBudgetSection,
) -> Result<Hash256, serde_json::Error> {
    let value = serde_json::to_value(budget)?;
    let canonical = canonicalize_value(&value).expect("runtime chrome budget canonicalizes");
    Ok(Hash256::from_bytes(Sha256::digest(canonical).into()))
}

fn validate_missing_budget_shape(
    report: &StaticBudgetReportBody,
    outcome: ReportOutcome,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    let missing_hash = report.identity.runtime_chrome_budget_hash.is_none();
    let missing_budget = report.runtime_chrome_budget.is_none();
    let missing_diagnostics = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.code,
                ValidationCode::BudgetMissingRuntimeChromeBudget
            )
        })
        .count();
    let missing_failures = report
        .decision
        .failures
        .iter()
        .filter(|failure| matches!(failure, BudgetFailure::MissingRuntimeChromeBudget))
        .count();

    let any_missing_marker =
        missing_hash || missing_budget || missing_diagnostics > 0 || missing_failures > 0;
    let exact_missing_shape = missing_hash
        && missing_budget
        && outcome == ReportOutcome::Failed
        && missing_diagnostics == 1
        && missing_failures == 1
        && report.diagnostics.len() == 1
        && report.decision.failures.len() == 1;

    if any_missing_marker && !exact_missing_shape {
        errors.push(semantic_error("runtime_chrome_budget.missing_shape"));
    }
}

fn validate_decision(
    report: &StaticBudgetReportBody,
    outcome: ReportOutcome,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    if report.decision.fits != report.decision.failures.is_empty() {
        errors.push(semantic_error("decision.fits"));
    }
    if !decision_interpretation_matches_fits(report.decision.fits, report.decision.interpretation) {
        errors.push(semantic_error("decision.interpretation"));
    }
    if (report.decision.fits && outcome != ReportOutcome::Passed)
        || (!report.decision.fits && outcome != ReportOutcome::Failed)
    {
        errors.push(semantic_error("outcome"));
    }
}

fn validate_diagnostics(report: &StaticBudgetReportBody, errors: &mut Vec<ValidationDiagnostic>) {
    for diagnostic in &report.diagnostics {
        if diagnostic.severity == DiagnosticSeverity::Soft {
            errors.push(semantic_error("diagnostics.severity"));
        }
    }
    if !failure_diagnostics_are_one_to_one(&report.decision.failures, &report.diagnostics) {
        errors.push(semantic_error("diagnostics.budget_failure_one_to_one"));
    }
    if !is_canonically_ordered_by(&report.decision.failures, cmp_budget_failures) {
        errors.push(semantic_error("decision.failures"));
    }
    if !is_canonically_ordered_by(&report.diagnostics, cmp_diagnostics) {
        errors.push(semantic_error("diagnostics"));
    }
}

fn validate_projection_order(
    projections: &BudgetProjectionSection,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    if !projections
        .per_expert_payload
        .windows(2)
        .all(|pair| (pair[0].layer, pair[0].expert) < (pair[1].layer, pair[1].expert))
    {
        errors.push(semantic_error("projections.per_expert_payload"));
    }
    if !projections
        .per_bank_occupancy
        .windows(2)
        .all(|pair| pair[0].slot < pair[1].slot)
    {
        errors.push(semantic_error("projections.per_bank_occupancy"));
    }
    if !projections
        .accumulator_maxima
        .windows(2)
        .all(|pair| pair[0].site < pair[1].site)
    {
        errors.push(semantic_error("projections.accumulator_maxima"));
    }
    if projections.routing_model.kind.is_empty() {
        errors.push(semantic_error("projections.routing_model.kind"));
    }
}

fn validate_projection_arithmetic(
    projections: &BudgetProjectionSection,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    let expected_aggregate = projections
        .common_bank_footprint
        .kernel_bytes
        .checked_add(projections.common_bank_footprint.lut_bytes)
        .and_then(|sum| {
            sum.checked_add(
                projections
                    .common_bank_footprint
                    .shared_dense_ffn_bytes
                    .unwrap_or(0),
            )
        });
    if expected_aggregate != Some(projections.common_bank_footprint.aggregate_bytes) {
        errors.push(semantic_error(
            "projections.common_bank_footprint.aggregate_bytes",
        ));
    }
    for switch in [
        &projections.projected_bank_switches_per_token,
        &projections.projected_sram_page_switches_per_token,
    ] {
        if switch.decision_value > switch.upper_bound {
            errors.push(semantic_error(
                "projections.projected_switches.decision_value",
            ));
        }
    }
}

fn validate_runtime_chrome_budget(
    budget: &RuntimeChromeBudgetSection,
    projections: &BudgetProjectionSection,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    if !budget
        .rom_slots
        .windows(2)
        .all(|pair| pair[0].id < pair[1].id)
    {
        errors.push(semantic_error("runtime_chrome_budget.rom_slots"));
    }

    let budget_slots = budget
        .rom_slots
        .iter()
        .map(|slot| slot.id)
        .collect::<Vec<_>>();
    let occupancy_slots = projections
        .per_bank_occupancy
        .iter()
        .map(|entry| entry.slot)
        .collect::<Vec<_>>();
    if budget_slots != occupancy_slots {
        errors.push(semantic_error("projections.per_bank_occupancy.coverage"));
        return;
    }

    for (slot, entry) in budget.rom_slots.iter().zip(&projections.per_bank_occupancy) {
        let effective_cap = i64::from(slot.usable_bytes) - i64::from(slot.reserved_slack);
        if entry.class != slot.class
            || entry.usable_bytes != slot.usable_bytes
            || entry.reserved_slack != slot.reserved_slack
            || entry.placement_caps != slot.placement_caps
            || entry.effective_cap_bytes != effective_cap
        {
            errors.push(semantic_error(
                "projections.per_bank_occupancy.slot_excerpt",
            ));
        }
        let residual = effective_cap - i64::from(entry.assigned_bytes);
        match i32::try_from(residual) {
            Ok(residual) if entry.residual_bytes == residual => {}
            _ => {
                errors.push(semantic_error(
                    "projections.per_bank_occupancy.residual_bytes",
                ));
            }
        }
    }
}

fn validate_missing_budget_has_no_view_payload(
    projections: &BudgetProjectionSection,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    if !projections.per_expert_payload.is_empty()
        || !projections.per_bank_occupancy.is_empty()
        || !projections.accumulator_maxima.is_empty()
        || projections.common_bank_footprint != CommonBankFootprintSection::default()
        || projections.projected_wram != ProjectedSizeSection::default()
        || projections.projected_sram != ProjectedSizeSection::default()
        || projections.projected_hram != ProjectedSizeSection::default()
        || projections.projected_bank_switches_per_token != ProjectedSwitchCountSection::default()
        || projections.projected_sram_page_switches_per_token
            != ProjectedSwitchCountSection::default()
        || projections.routing_model.kind != "not_evaluated_missing_runtime_chrome_budget"
    {
        errors.push(semantic_error("projections.missing_runtime_chrome_budget"));
    }
}

fn validate_expert_assignment_invariants(
    projections: &BudgetProjectionSection,
    fits: bool,
    failures: &[BudgetFailure],
    errors: &mut Vec<ValidationDiagnostic>,
) {
    let assigned_experts = projections
        .per_bank_occupancy
        .iter()
        .flat_map(|bank| {
            bank.assigned_components
                .iter()
                .filter_map(|component| match component {
                    BudgetComponentRef::Expert { layer, expert } => {
                        Some((*layer, *expert, bank.slot))
                    }
                    _ => None,
                })
        })
        .collect::<BTreeSet<_>>();

    for expert in &projections.per_expert_payload {
        let assigned_component = expert
            .assigned_slot
            .is_some_and(|slot| assigned_experts.contains(&(expert.layer, expert.expert, slot)));
        if expert.assigned_slot.is_some() && !assigned_component {
            errors.push(semantic_error(
                "projections.per_expert_payload.assigned_slot",
            ));
        }
        if fits && !assigned_component {
            errors.push(semantic_error(
                "projections.per_expert_payload.assigned_component",
            ));
        }
        if !fits
            && expert.assigned_slot.is_none()
            && expert.unassigned_because.is_none()
            && !unassigned_expert_has_matching_failure(expert, failures)
        {
            errors.push(semantic_error(
                "projections.per_expert_payload.unassigned_because",
            ));
        }
    }
}

fn unassigned_expert_has_matching_failure(
    expert: &PerExpertEntry,
    failures: &[BudgetFailure],
) -> bool {
    failures.iter().any(|failure| match failure {
        BudgetFailure::ExpertExceedsSlot {
            layer,
            expert: failure_expert,
            ..
        } => (*layer, *failure_expert) == (expert.layer, expert.expert),
        BudgetFailure::PlacementProfileInfeasible { reason, .. } => matches!(
            (&expert.placement_status, reason),
            (
                ExpertPlacementStatus::UnassignedNoEligibleSlots,
                gbf_policy::PlacementInfeasibilityReason::NoSlotsForClass
                    | gbf_policy::PlacementInfeasibilityReason::RequiresUnavailableSlotClass
            ) | (
                ExpertPlacementStatus::UnassignedStrictDistinctSlotsExhausted,
                gbf_policy::PlacementInfeasibilityReason::ExpertCountExceedsSlots
            )
        ),
        _ => false,
    })
}

fn validate_switch_failures(
    projections: &BudgetProjectionSection,
    failures: &[BudgetFailure],
    errors: &mut Vec<ValidationDiagnostic>,
) {
    for failure in failures {
        match failure {
            BudgetFailure::BankSwitchesPerTokenOverCap {
                decision_value,
                upper_bound,
                cap,
                source,
            } => {
                let projection = &projections.projected_bank_switches_per_token;
                if *decision_value != projection.decision_value
                    || *upper_bound != projection.upper_bound
                    || *source != projection.source
                    || decision_value <= cap
                {
                    errors.push(semantic_error("decision.failures.bank_switches_per_token"));
                }
            }
            BudgetFailure::SramPageSwitchesPerTokenOverCap {
                decision_value,
                upper_bound,
                cap,
                source,
            } => {
                let projection = &projections.projected_sram_page_switches_per_token;
                if *decision_value != projection.decision_value
                    || *upper_bound != projection.upper_bound
                    || *source != projection.source
                    || decision_value <= cap
                {
                    errors.push(semantic_error(
                        "decision.failures.sram_page_switches_per_token",
                    ));
                }
            }
            _ => {}
        }
    }
}

fn contains_missing_runtime_chrome_budget_failure(failures: &[BudgetFailure]) -> bool {
    failures
        .iter()
        .any(|failure| matches!(failure, BudgetFailure::MissingRuntimeChromeBudget))
}

fn semantic_error(field: &'static str) -> ValidationDiagnostic {
    let field = FieldPath::from(field);
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::Schema,
        code: ValidationCode::ReportSemanticInvariantViolated {
            field: field.clone(),
        },
        detail: ValidationDetail::Field {
            field: field.clone(),
        },
        provenance: vec![EvidenceRef {
            kind: "semantic_validator".to_owned(),
            reference: field.to_string(),
            hash: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use crate::{CanonicalJsonError, ReportEnvelope, canonicalize, round_trip_self_hash};

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn semantic_error_fields(body: &StaticBudgetReportBody) -> Vec<String> {
        body.validate_semantics(ReportOutcome::Failed)
            .expect_err("body should fail semantic validation")
            .into_iter()
            .filter_map(|diagnostic| match diagnostic.code {
                ValidationCode::ReportSemanticInvariantViolated { field } => {
                    Some(field.to_string())
                }
                _ => None,
            })
            .collect()
    }

    fn identity(runtime_chrome_budget_hash: Option<Hash256>) -> BudgetIdentitySection {
        BudgetIdentitySection {
            artifact_core_hash: hash(1),
            quant_graph_hash: hash(2),
            policy_resolution_self_hash: hash(3),
            runtime_chrome_budget_hash,
            target_profile_hash: hash(4),
        }
    }

    fn runtime_budget() -> RuntimeChromeBudgetSection {
        RuntimeChromeBudgetSection {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(5),
            rom_slots: vec![
                RomBudgetSlotEntry {
                    id: BudgetSlotId::new(1),
                    class: BudgetSlotClass::ExpertBank,
                    usable_bytes: 1024,
                    reserved_slack: 128,
                    placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
                },
                RomBudgetSlotEntry {
                    id: BudgetSlotId::new(2),
                    class: BudgetSlotClass::CommonBank,
                    usable_bytes: 2048,
                    reserved_slack: 0,
                    placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
                },
            ],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(4),
            },
            wram_reserved: 0,
            sram_reserved: 0,
        }
    }

    fn projections() -> BudgetProjectionSection {
        BudgetProjectionSection {
            per_expert_payload: vec![PerExpertEntry {
                layer: LayerId::new(0),
                expert: ExpertId::new(0),
                payload_bytes: 64,
                assigned_slot: Some(BudgetSlotId::new(1)),
                unassigned_because: None,
                placement_status: ExpertPlacementStatus::Assigned,
            }],
            per_bank_occupancy: vec![
                PerBankEntry {
                    slot: BudgetSlotId::new(1),
                    class: BudgetSlotClass::ExpertBank,
                    usable_bytes: 1024,
                    reserved_slack: 128,
                    effective_cap_bytes: 896,
                    assigned_bytes: 64,
                    residual_bytes: 832,
                    assigned_components: vec![BudgetComponentRef::Expert {
                        layer: LayerId::new(0),
                        expert: ExpertId::new(0),
                    }],
                    placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
                },
                PerBankEntry {
                    slot: BudgetSlotId::new(2),
                    class: BudgetSlotClass::CommonBank,
                    usable_bytes: 2048,
                    reserved_slack: 0,
                    effective_cap_bytes: 2048,
                    assigned_bytes: 0,
                    residual_bytes: 2048,
                    assigned_components: Vec::new(),
                    placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
                },
            ],
            common_bank_footprint: CommonBankFootprintSection {
                kernel_bytes: 0,
                lut_bytes: 0,
                shared_dense_ffn_bytes: None,
                aggregate_bytes: 0,
            },
            accumulator_maxima: vec![AccumulatorBound {
                site: ReductionSiteId("site.0".to_owned()),
                projected_max_abs: 127,
                i16_safe: true,
                i32_safe: true,
            }],
            projected_wram: ProjectedSizeSection {
                peak_bytes: 256,
                source: ProjectedSizeSource::StaticGraphProjection,
            },
            projected_sram: ProjectedSizeSection {
                peak_bytes: 0,
                source: ProjectedSizeSource::StaticGraphProjection,
            },
            projected_hram: ProjectedSizeSection {
                peak_bytes: 8,
                source: ProjectedSizeSource::StaticGraphProjection,
            },
            projected_bank_switches_per_token: ProjectedSwitchCountSection {
                upper_bound: 1,
                expected_q16_16: None,
                decision_value: 1,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            projected_sram_page_switches_per_token: ProjectedSwitchCountSection::default(),
            routing_model: RoutingModelSection {
                kind: "Top1Deterministic".to_owned(),
            },
        }
    }

    fn decision(fits: bool, failures: Vec<BudgetFailure>) -> BudgetDecisionSection {
        BudgetDecisionSection {
            fits,
            interpretation: static_fit_interpretation_for_fits(fits),
            placement_model: StaticPlacementModel::BudgetedFirstFit,
            failures,
        }
    }

    fn report_fixture() -> ReportEnvelope<StaticBudgetReportBody> {
        let budget = runtime_budget();
        let budget_hash = runtime_chrome_budget_hash(&budget).expect("budget hashes");
        ReportEnvelope::new(
            ReportOutcome::Passed,
            StaticBudgetReportBody {
                identity: identity(Some(budget_hash)),
                policy: BudgetPolicySection {
                    placement_profile: PlacementProfile::Budgeted,
                    objective_hash: hash(6),
                },
                runtime_chrome_budget: Some(budget),
                projections: projections(),
                decision: decision(true, Vec::new()),
                diagnostics: Vec::new(),
            },
        )
        .expect("envelope")
        .with_computed_self_hash()
        .expect("self hash")
    }

    fn failure_report_fixture() -> ReportEnvelope<StaticBudgetReportBody> {
        let budget = runtime_budget();
        let budget_hash = runtime_chrome_budget_hash(&budget).expect("budget hashes");
        let failure = BudgetFailure::ExpertExceedsSlot {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            slot: BudgetSlotId::new(1),
            payload_bytes: 1024,
            cap_bytes: 896,
            excess_bytes: 128,
        };
        let mut projections = projections();
        projections.per_expert_payload[0].placement_status = ExpertPlacementStatus::AssignedOverCap;
        projections.per_bank_occupancy[0].assigned_bytes = 1024;
        projections.per_bank_occupancy[0].residual_bytes = -128;
        ReportEnvelope::new(
            ReportOutcome::Failed,
            StaticBudgetReportBody {
                identity: identity(Some(budget_hash)),
                policy: BudgetPolicySection {
                    placement_profile: PlacementProfile::Budgeted,
                    objective_hash: hash(6),
                },
                runtime_chrome_budget: Some(budget),
                projections,
                decision: decision(false, vec![failure.clone()]),
                diagnostics: vec![budget_failure_diagnostic(&failure)],
            },
        )
        .expect("envelope")
        .with_computed_self_hash()
        .expect("self hash")
    }

    fn missing_budget_report_fixture() -> ReportEnvelope<StaticBudgetReportBody> {
        let failure = BudgetFailure::MissingRuntimeChromeBudget;
        ReportEnvelope::new(
            ReportOutcome::Failed,
            StaticBudgetReportBody {
                identity: identity(None),
                policy: BudgetPolicySection {
                    placement_profile: PlacementProfile::Budgeted,
                    objective_hash: hash(6),
                },
                runtime_chrome_budget: None,
                projections: BudgetProjectionSection::default(),
                decision: decision(false, vec![failure.clone()]),
                diagnostics: vec![budget_failure_diagnostic(&failure)],
            },
        )
        .expect("envelope")
        .with_computed_self_hash()
        .expect("self hash")
    }

    #[derive(Debug, Clone, Serialize)]
    struct StaticBudgetFloatProbe {
        projections: serde_json::Value,
        diagnostics: Vec<ValidationDiagnosticRecord>,
    }

    impl ReportBody for StaticBudgetFloatProbe {
        const REPORT_TYPE: &'static str = "StaticBudgetReport";
        const SCHEMA_ID: &'static str = SCHEMA_ID;
        const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

        fn validate_semantics(
            &self,
            _outcome: ReportOutcome,
        ) -> Result<(), Vec<ValidationDiagnostic>> {
            Ok(())
        }
    }

    #[test]
    fn f_b4_static_budget_v1_schema_accepts_canonical_fixture() {
        let report = report_fixture();
        let value = serde_json::to_value(&report).expect("report serializes");

        assert_eq!(value["schema"], serde_json::json!("static_budget.v1"));
        assert_eq!(value["schema_version"], serde_json::json!("1.0.0"));
        assert_eq!(value["outcome"], serde_json::json!("Passed"));
        assert_eq!(
            value["identity"]["runtime_chrome_budget_hash"],
            hash_value_from_report(&report)
        );
        assert!(value["runtime_chrome_budget"].is_object());
        assert!(value["body"].is_null());
        assert!(
            value["projections"]["per_expert_payload"][0]
                .get("unassigned_because")
                .is_none()
        );
        assert!(
            value["projections"]["per_expert_payload"][0]
                .get("assigned_slot")
                .is_some()
        );

        serde_json::from_value::<ReportEnvelope<StaticBudgetReportBody>>(value)
            .expect("canonical static_budget.v1 fixture decodes");
        canonicalize(&report).expect("canonical fixture canonicalizes");
    }

    fn hash_value_from_report(
        report: &ReportEnvelope<StaticBudgetReportBody>,
    ) -> serde_json::Value {
        serde_json::to_value(report.body.identity.runtime_chrome_budget_hash.unwrap())
            .expect("hash serializes")
    }

    #[test]
    fn f_b4_static_budget_v1_rejects_missing_required_fields() {
        let mut value = serde_json::to_value(report_fixture()).expect("report serializes");
        value["identity"]
            .as_object_mut()
            .expect("identity object")
            .remove("quant_graph_hash");

        assert!(serde_json::from_value::<ReportEnvelope<StaticBudgetReportBody>>(value).is_err());
    }

    #[test]
    fn f_b4_static_budget_v1_self_hash_round_trip() {
        round_trip_self_hash(&report_fixture()).expect("success report self hash round-trips");
    }

    #[test]
    fn f_b4_static_budget_v1_failure_report_self_hash_round_trip() {
        round_trip_self_hash(&failure_report_fixture())
            .expect("failure report self hash round-trips");
    }

    #[test]
    fn f_b4_static_budget_v1_rejects_float_values() {
        let report = ReportEnvelope::new(
            ReportOutcome::Passed,
            StaticBudgetFloatProbe {
                projections: serde_json::json!({
                    "projected_wram": {
                        "peak_bytes": 1.25
                    }
                }),
                diagnostics: Vec::new(),
            },
        )
        .expect("float probe envelope");

        assert_eq!(
            canonicalize(&report),
            Err(CanonicalJsonError::FloatingPointValue {
                path: "projections.projected_wram.peak_bytes".to_owned()
            })
        );
    }

    #[test]
    fn f_b4_static_budget_v1_missing_budget_includes_failure() {
        let report = missing_budget_report_fixture();
        round_trip_self_hash(&report).expect("missing-budget report self hash round-trips");
        assert_eq!(
            report.body.decision.failures,
            vec![BudgetFailure::MissingRuntimeChromeBudget]
        );
        assert!(matches!(
            report.body.diagnostics[0].code,
            ValidationCode::BudgetMissingRuntimeChromeBudget
        ));

        let mut missing_failure = report.body.clone();
        missing_failure.decision.failures.clear();
        assert!(
            missing_failure
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );

        let extra_failure = BudgetFailure::WramPeakExceedsCap { peak: 2, cap: 1 };
        let mut extra = report.body.clone();
        extra.decision.failures.push(extra_failure.clone());
        extra
            .diagnostics
            .push(budget_failure_diagnostic(&extra_failure));

        assert!(
            semantic_error_fields(&extra)
                .contains(&"runtime_chrome_budget.missing_shape".to_owned())
        );
    }

    #[test]
    fn f_b4_static_budget_v1_runtime_chrome_budget_excerpt_hash_matches() {
        let report = report_fixture();
        let mut mismatched = report.body.clone();
        mismatched.identity.runtime_chrome_budget_hash = Some(hash(99));

        assert!(
            mismatched
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );
    }

    #[test]
    fn f_b4_static_budget_v1_missing_budget_has_no_view_derived_fields() {
        let report = missing_budget_report_fixture();
        let mut body = report.body.clone();
        body.projections.per_expert_payload.push(PerExpertEntry {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            payload_bytes: 64,
            assigned_slot: None,
            unassigned_because: Some(UnassignedBecause::NoEligibleSlots),
            placement_status: ExpertPlacementStatus::UnassignedNoEligibleSlots,
        });

        assert!(body.validate_semantics(ReportOutcome::Failed).is_err());
    }

    #[test]
    fn f_b4_static_budget_v1_rejects_soft_diagnostic() {
        let mut report = failure_report_fixture();
        report.body.diagnostics[0].severity = DiagnosticSeverity::Soft;

        assert!(canonicalize(&report).is_err());
    }

    #[test]
    fn f_b4_static_budget_v1_rejects_unlisted_null_fields() {
        let mut value = serde_json::to_value(report_fixture()).expect("report serializes");
        value["projections"]["per_expert_payload"][0]["assigned_slot"] = serde_json::Value::Null;
        let result = serde_json::from_value::<ReportEnvelope<StaticBudgetReportBody>>(value);

        assert!(matches!(
            result,
            Err(error) if error.to_string().contains("assigned_slot")
        ));
    }

    #[test]
    fn f_b4_static_budget_v1_failure_diagnostic_one_to_one() {
        let failures = all_failure_variants();
        let diagnostics = diagnostics_for_budget_failures(&failures);

        assert_eq!(diagnostics.len(), failures.len());
        assert!(failure_diagnostics_are_one_to_one(&failures, &diagnostics));

        for (failure, diagnostic) in failures.iter().zip(&diagnostics) {
            assert_eq!(diagnostic.origin, ValidationOrigin::Budget);
            assert_eq!(diagnostic.code, failure.validation_code());
        }

        let mut missing = diagnostics.clone();
        missing.pop();
        assert!(!failure_diagnostics_are_one_to_one(&failures, &missing));

        let mut mismatched = diagnostics.clone();
        mismatched.swap(0, 1);
        assert!(failure_diagnostics_are_one_to_one(&failures, &mismatched));

        mismatched[0] =
            budget_failure_diagnostic(&BudgetFailure::WramPeakExceedsCap { peak: 9, cap: 8 });
        assert!(!failure_diagnostics_are_one_to_one(&failures, &mismatched));
    }

    #[test]
    fn f_b4_static_budget_v1_enforces_projection_and_failure_ordering() {
        let mut unordered_accumulators = report_fixture().body;
        unordered_accumulators.projections.accumulator_maxima = vec![
            AccumulatorBound {
                site: ReductionSiteId("site.1".to_owned()),
                projected_max_abs: 127,
                i16_safe: true,
                i32_safe: true,
            },
            AccumulatorBound {
                site: ReductionSiteId("site.0".to_owned()),
                projected_max_abs: 127,
                i16_safe: true,
                i32_safe: true,
            },
        ];

        assert!(
            semantic_error_fields(&unordered_accumulators)
                .contains(&"projections.accumulator_maxima".to_owned())
        );

        let expert_failure = BudgetFailure::ExpertExceedsSlot {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            slot: BudgetSlotId::new(1),
            payload_bytes: 1024,
            cap_bytes: 896,
            excess_bytes: 128,
        };
        let wram_failure = BudgetFailure::WramPeakExceedsCap {
            peak: 8193,
            cap: 8192,
        };
        let mut unordered_failures = failure_report_fixture().body;
        unordered_failures.decision.failures = vec![wram_failure.clone(), expert_failure.clone()];
        unordered_failures.diagnostics =
            diagnostics_for_budget_failures(&[expert_failure.clone(), wram_failure.clone()]);

        assert!(
            semantic_error_fields(&unordered_failures).contains(&"decision.failures".to_owned())
        );

        let mut unordered_diagnostics = failure_report_fixture().body;
        unordered_diagnostics.decision.failures =
            vec![expert_failure.clone(), wram_failure.clone()];
        unordered_diagnostics.diagnostics =
            diagnostics_for_budget_failures(&[wram_failure, expert_failure]);

        assert!(failure_diagnostics_are_one_to_one(
            &unordered_diagnostics.decision.failures,
            &unordered_diagnostics.diagnostics
        ));
        assert!(semantic_error_fields(&unordered_diagnostics).contains(&"diagnostics".to_owned()));
    }

    #[test]
    fn f_b4_static_budget_v1_rejects_residual_overflow() {
        let mut body = report_fixture().body;
        let slot = &mut body
            .runtime_chrome_budget
            .as_mut()
            .expect("runtime budget present")
            .rom_slots[0];
        slot.usable_bytes = u32::MAX;
        slot.reserved_slack = 0;

        body.projections.per_bank_occupancy[0].usable_bytes = u32::MAX;
        body.projections.per_bank_occupancy[0].reserved_slack = 0;
        body.projections.per_bank_occupancy[0].effective_cap_bytes = i64::from(u32::MAX);
        body.projections.per_bank_occupancy[0].residual_bytes = i32::MAX;
        body.identity.runtime_chrome_budget_hash =
            Some(runtime_chrome_budget_hash(body.runtime_chrome_budget.as_ref().unwrap()).unwrap());

        assert!(
            semantic_error_fields(&body)
                .contains(&"projections.per_bank_occupancy.residual_bytes".to_owned())
        );
    }

    #[test]
    fn f_b4_static_budget_v1_unassigned_experts_may_be_explained_by_failures() {
        let mut body = report_fixture().body;
        let failure = BudgetFailure::PlacementProfileInfeasible {
            profile: PlacementProfile::Budgeted,
            reason: gbf_policy::PlacementInfeasibilityReason::NoSlotsForClass,
        };
        body.decision = decision(false, vec![failure.clone()]);
        body.diagnostics = vec![budget_failure_diagnostic(&failure)];
        body.projections.per_expert_payload[0].assigned_slot = None;
        body.projections.per_expert_payload[0].unassigned_because = None;
        body.projections.per_expert_payload[0].placement_status =
            ExpertPlacementStatus::UnassignedNoEligibleSlots;
        body.projections.per_bank_occupancy[0].assigned_bytes = 0;
        body.projections.per_bank_occupancy[0].residual_bytes = 896;
        body.projections.per_bank_occupancy[0]
            .assigned_components
            .clear();

        body.validate_semantics(ReportOutcome::Failed)
            .expect("matching placement failure explains unassigned expert");

        body.decision.failures.clear();
        body.diagnostics.clear();
        assert!(
            semantic_error_fields(&body)
                .contains(&"projections.per_expert_payload.unassigned_because".to_owned())
        );
    }

    #[test]
    fn f_b4_static_budget_v1_provenance_helper_preserves_one_to_one_mapping() {
        let failures = all_failure_variants();
        let provenance = vec![EvidenceRef {
            kind: "Fixture".to_owned(),
            reference: "static-budget-report".to_owned(),
            hash: Some(hash(4)),
        }];
        let diagnostics =
            diagnostics_for_budget_failures_with_provenance(&failures, provenance.clone());

        assert!(failure_diagnostics_are_one_to_one(&failures, &diagnostics));
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.provenance == provenance)
        );
    }

    #[test]
    fn f_b4_static_budget_v1_decision_interpretation_invariant() {
        assert_eq!(
            static_fit_interpretation_for_fits(true),
            StaticFitInterpretation::PassesNecessaryStaticChecks
        );
        assert_eq!(
            static_fit_interpretation_for_fits(false),
            StaticFitInterpretation::FailsNecessaryStaticChecks
        );

        assert!(decision_interpretation_matches_fits(
            true,
            StaticFitInterpretation::PassesNecessaryStaticChecks
        ));
        assert!(decision_interpretation_matches_fits(
            false,
            StaticFitInterpretation::FailsNecessaryStaticChecks
        ));
        assert!(!decision_interpretation_matches_fits(
            true,
            StaticFitInterpretation::FailsNecessaryStaticChecks
        ));
        assert!(!decision_interpretation_matches_fits(
            false,
            StaticFitInterpretation::PassesNecessaryStaticChecks
        ));
    }

    fn all_failure_variants() -> Vec<BudgetFailureRecord> {
        vec![
            BudgetFailureRecord::MissingRuntimeChromeBudget,
            BudgetFailureRecord::QuantGraphBudgetViewMalformed {
                field: FieldPath::from("budget_view.experts[0].rows"),
            },
            BudgetFailureRecord::ExpertExceedsSlot {
                layer: LayerId::new(1),
                expert: ExpertId::new(2),
                slot: BudgetSlotId::new(3),
                payload_bytes: 17_000,
                cap_bytes: 16_128,
                excess_bytes: 872,
            },
            BudgetFailureRecord::CommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
                excess_bytes: 3_616,
            },
            BudgetFailureRecord::WramPeakExceedsCap {
                peak: 8_300,
                cap: 8_192,
            },
            BudgetFailureRecord::SramPeakExceedsCap {
                peak: 33_000,
                cap: 32_768,
            },
            BudgetFailureRecord::HramPeakExceedsCap {
                peak: 144,
                cap: 127,
            },
            BudgetFailureRecord::AccumulatorExceedsI32 {
                site: ReductionSiteId("ffn.0.acc".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            },
            BudgetFailureRecord::BankSwitchesPerTokenOverCap {
                decision_value: 9,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            BudgetFailureRecord::SramPageSwitchesPerTokenOverCap {
                decision_value: 4,
                upper_bound: 4,
                cap: 2,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            BudgetFailureRecord::PlacementProfileInfeasible {
                profile: PlacementProfile::PackedExperts,
                reason: gbf_policy::PlacementInfeasibilityReason::ExpertCountExceedsSlots,
            },
        ]
    }

    #[test]
    fn runtime_budget_section_hash_is_canonical_sha256() {
        let hash = runtime_chrome_budget_hash(&runtime_budget()).expect("budget hashes");
        let text = serde_json::to_value(hash)
            .expect("hash serializes")
            .as_str()
            .expect("hash is a JSON string")
            .to_owned();
        assert!(Hash256::from_str(&text).is_ok());
        assert!(text.starts_with("sha256:"));
    }
}
