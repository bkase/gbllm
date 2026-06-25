//! Training-side deployability preflight helpers.

use std::error::Error;
use std::fmt;

use gbf_artifact::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};
use gbf_foundation::{ByteCost, CompileProfileId};
use gbf_model::budget::{
    ExpertBudgetError, ExpertSlotFit, StaticBudgetReport, compute_expert_bytes_checked,
};
use gbf_policy::model_profile::ModelSizeProfile;
use gbf_policy::{
    BudgetSlotClass, CompileProfile, CompileRequest, RomBudgetSlot, RuntimeChromeBudget,
    RuntimeChromeBudgetValidationError, RuntimeNucleusHash, profile_by_id,
};

use crate::logging::{ExpertSlotPreflightEvent, LoggingEventError, TrainingLogEmitter};

pub fn compute_preflight_expert_bytes(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> Result<ByteCost, ExpertBudgetError> {
    compute_expert_bytes_checked(plan, d_model, d_ff)
}

pub fn compute_preflight_profile_expert_bytes(
    profile: ModelSizeProfile,
) -> Result<ByteCost, ExpertBudgetError> {
    compute_preflight_expert_bytes(
        &default_expert_weight_plan(),
        u32::from(profile.d_model()),
        u32::from(profile.d_ff()),
    )
}

fn default_expert_weight_plan() -> TernaryWeightPlan {
    TernaryWeightPlan::new(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
        ThresholdPlan::FixedQ8_8,
    )
}

/// Narrow training-side view of effective ExpertBank slot capacities.
///
/// New callers should prefer `RuntimeChromeBudget` plus
/// `preflight_runtime_chrome_budget`. This surface remains as a small fallback
/// boundary for older profile-only checks and stores slot capacity after
/// `reserved_slack` has been subtracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpertBankBudgetSurface {
    expert_bank_usable_bytes: Vec<ByteCost>,
}

impl ExpertBankBudgetSurface {
    pub fn new(expert_bank_usable_bytes: Vec<ByteCost>) -> Result<Self, ProfilePreflightError> {
        if expert_bank_usable_bytes.is_empty() {
            return Err(ProfilePreflightError::MissingExpertBanks);
        }
        if expert_bank_usable_bytes.contains(&ByteCost::ZERO) {
            return Err(ProfilePreflightError::EmptyExpertBank);
        }

        Ok(Self {
            expert_bank_usable_bytes,
        })
    }

    #[must_use]
    pub fn expert_bank_usable_bytes(&self) -> &[ByteCost] {
        &self.expert_bank_usable_bytes
    }

    #[must_use]
    pub fn smallest_expert_bank_usable_bytes(&self) -> ByteCost {
        *self
            .expert_bank_usable_bytes
            .iter()
            .min()
            .expect("constructor requires at least one ExpertBank")
    }

    pub fn from_runtime_chrome_budget(
        budget: &RuntimeChromeBudget,
    ) -> Result<Self, ProfilePreflightError> {
        if let Err(error) = budget.validate() {
            return Err(ProfilePreflightError::InvalidRuntimeChromeBudget { error });
        }

        let expert_bank_usable_bytes = budget
            .rom_slots
            .iter()
            .filter(|slot| slot.class == BudgetSlotClass::ExpertBank)
            .map(effective_slot_capacity)
            .collect();

        Self::new(expert_bank_usable_bytes)
    }
}

#[cfg(test)]
mod nucleus_self_check {
    use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};
    use gbf_policy::{
        BudgetSlotClass, PlacementProfile, RomBudgetSlot, RuntimeChromeBudget,
        RuntimeMemoryCapSection, RuntimeNucleusHash, WramReserved,
    };

    use super::{preflight_runtime_nucleus_self_check, require_runtime_nucleus_self_check};

    #[test]
    fn matching_pinned_hash_passes() {
        let pinned = runtime_hash(0xAB);
        let budget = budget_with_hash(pinned);

        let report =
            require_runtime_nucleus_self_check(pinned, &budget).expect("matching hash passes");

        assert!(report.hashes_match());
        assert_eq!(report.pinned_runtime_nucleus_hash(), pinned);
        assert_eq!(report.current_runtime_nucleus_hash(), pinned);
        assert!(report.diagnostic().contains("self-check PASS"));
    }

    #[test]
    fn drift_diagnoses_pinned_and_current_hashes() {
        let pinned = runtime_hash(0xAB);
        let current = runtime_hash(0xCD);
        let budget = budget_with_hash(current);

        let report = preflight_runtime_nucleus_self_check(pinned, &budget);
        let error =
            require_runtime_nucleus_self_check(pinned, &budget).expect_err("mismatched hash fails");

        assert!(!report.hashes_match());
        assert_eq!(error.report(), &report);
        assert!(report.diagnostic().contains("self-check FAIL"));
        assert!(
            report
                .diagnostic()
                .contains(&format!("pinned_runtime_nucleus_hash={pinned}"))
        );
        assert!(
            report
                .diagnostic()
                .contains(&format!("current_runtime_nucleus_hash={current}"))
        );
    }

    fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
        RuntimeNucleusHash::real(Hash256::from_bytes([byte; 32]))
    }

    fn budget_with_hash(runtime_nucleus_hash: RuntimeNucleusHash) -> RuntimeChromeBudget {
        RuntimeChromeBudget::new(
            TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            CompileProfileId::from("Bringup"),
            runtime_nucleus_hash,
            RuntimeChromeBudget::pinned_reference_shell_modules(),
            vec![
                RomBudgetSlot::new(
                    gbf_foundation::BudgetSlotId::new(0),
                    BudgetSlotClass::ExpertBank,
                    16 * 1024,
                    384,
                    std::collections::BTreeSet::from([PlacementProfile::Budgeted]),
                )
                .expect("valid expert slot"),
            ],
            RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: Hash256::from_bytes([0x28; 32]),
            },
            WramReserved::new(512, 4_096, 1_536).expect("valid WRAM reservation"),
            1_024,
        )
        .expect("valid RuntimeChromeBudget")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileExpertBankPreflightReport {
    profile: ModelSizeProfile,
    expert_bytes: ByteCost,
    smallest_expert_bank_usable_bytes: ByteCost,
    slack_bytes: ByteCost,
}

impl ProfileExpertBankPreflightReport {
    #[must_use]
    pub const fn profile(self) -> ModelSizeProfile {
        self.profile
    }

    #[must_use]
    pub const fn expert_bytes(self) -> ByteCost {
        self.expert_bytes
    }

    #[must_use]
    pub const fn smallest_expert_bank_usable_bytes(self) -> ByteCost {
        self.smallest_expert_bank_usable_bytes
    }

    #[must_use]
    pub const fn slack_bytes(self) -> ByteCost {
        self.slack_bytes
    }
}

pub fn preflight_profile_expert_bank_budget(
    budget: &ExpertBankBudgetSurface,
    profile: ModelSizeProfile,
) -> Result<ProfileExpertBankPreflightReport, ProfilePreflightError> {
    let expert_bytes = compute_preflight_profile_expert_bytes(profile)
        .map_err(|error| ProfilePreflightError::ExpertBudget { error })?;
    let smallest_expert_bank_usable_bytes = budget.smallest_expert_bank_usable_bytes();

    let Some(slack_bytes) = smallest_expert_bank_usable_bytes.checked_sub(expert_bytes) else {
        return Err(ProfilePreflightError::ExpertExceedsSmallestBank {
            profile,
            expert_bytes,
            smallest_expert_bank_usable_bytes,
            over_by: expert_bytes - smallest_expert_bank_usable_bytes,
        });
    };

    Ok(ProfileExpertBankPreflightReport {
        profile,
        expert_bytes,
        smallest_expert_bank_usable_bytes,
        slack_bytes,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeChromePreflightDemand {
    profile: ModelSizeProfile,
    common_bank_rom_bytes: ByteCost,
    bank0_resident_bytes: ByteCost,
    hot_arena_bytes: ByteCost,
    overlay_payload_bytes: ByteCost,
    sequence_state_bytes: ByteCost,
}

impl RuntimeChromePreflightDemand {
    #[must_use]
    pub const fn new(
        profile: ModelSizeProfile,
        common_bank_rom_bytes: ByteCost,
        bank0_resident_bytes: ByteCost,
        hot_arena_bytes: ByteCost,
        overlay_payload_bytes: ByteCost,
        sequence_state_bytes: ByteCost,
    ) -> Self {
        Self {
            profile,
            common_bank_rom_bytes,
            bank0_resident_bytes,
            hot_arena_bytes,
            overlay_payload_bytes,
            sequence_state_bytes,
        }
    }

    #[must_use]
    pub const fn profile(self) -> ModelSizeProfile {
        self.profile
    }

    #[must_use]
    pub const fn common_bank_rom_bytes(self) -> ByteCost {
        self.common_bank_rom_bytes
    }

    #[must_use]
    pub const fn bank0_resident_bytes(self) -> ByteCost {
        self.bank0_resident_bytes
    }

    #[must_use]
    pub const fn hot_arena_bytes(self) -> ByteCost {
        self.hot_arena_bytes
    }

    #[must_use]
    pub const fn overlay_payload_bytes(self) -> ByteCost {
        self.overlay_payload_bytes
    }

    #[must_use]
    pub const fn sequence_state_bytes(self) -> ByteCost {
        self.sequence_state_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeChromePreflightReport {
    demand: RuntimeChromePreflightDemand,
    required_expert_slots: u8,
    available_expert_bank_slots: usize,
    fits_envelope: bool,
    checks: Vec<RuntimeChromePreflightCheck>,
    hard_failures: Vec<String>,
    risk_warnings: Vec<String>,
    wram_fit_report: Option<WramFitReport>,
    switch_budget_report: Option<SwitchBudgetReport>,
}

impl RuntimeChromePreflightReport {
    #[must_use]
    pub const fn demand(&self) -> RuntimeChromePreflightDemand {
        self.demand
    }

    #[must_use]
    pub const fn required_expert_slots(&self) -> u8 {
        self.required_expert_slots
    }

    #[must_use]
    pub const fn available_expert_bank_slots(&self) -> usize {
        self.available_expert_bank_slots
    }

    #[must_use]
    pub const fn fits_envelope(&self) -> bool {
        self.fits_envelope
    }

    #[must_use]
    pub fn checks(&self) -> &[RuntimeChromePreflightCheck] {
        &self.checks
    }

    #[must_use]
    pub fn hard_failures(&self) -> &[String] {
        &self.hard_failures
    }

    #[must_use]
    pub fn risk_warnings(&self) -> &[String] {
        &self.risk_warnings
    }

    #[must_use]
    pub const fn wram_fit_report(&self) -> Option<&WramFitReport> {
        self.wram_fit_report.as_ref()
    }

    #[must_use]
    pub const fn switch_budget_report(&self) -> Option<&SwitchBudgetReport> {
        self.switch_budget_report.as_ref()
    }

    fn from_checks(
        demand: RuntimeChromePreflightDemand,
        required_expert_slots: u8,
        available_expert_bank_slots: usize,
        checks: Vec<RuntimeChromePreflightCheck>,
        mut hard_failures: Vec<String>,
        wram_fit_report: Option<WramFitReport>,
        switch_budget_report: Option<SwitchBudgetReport>,
    ) -> Self {
        hard_failures.extend(
            checks
                .iter()
                .filter(|check| check.status == RuntimeChromePreflightStatus::Fail)
                .map(|check| check.diagnostic.clone()),
        );
        let risk_warnings = checks
            .iter()
            .filter(|check| check.status == RuntimeChromePreflightStatus::Warn)
            .map(|check| check.diagnostic.clone())
            .collect();
        Self {
            demand,
            required_expert_slots,
            available_expert_bank_slots,
            fits_envelope: hard_failures.is_empty(),
            checks,
            hard_failures,
            risk_warnings,
            wram_fit_report,
            switch_budget_report,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeNucleusSelfCheckReport {
    pinned_runtime_nucleus_hash: RuntimeNucleusHash,
    current_runtime_nucleus_hash: RuntimeNucleusHash,
    hashes_match: bool,
    diagnostic: String,
}

impl RuntimeNucleusSelfCheckReport {
    #[must_use]
    pub const fn pinned_runtime_nucleus_hash(&self) -> RuntimeNucleusHash {
        self.pinned_runtime_nucleus_hash
    }

    #[must_use]
    pub const fn current_runtime_nucleus_hash(&self) -> RuntimeNucleusHash {
        self.current_runtime_nucleus_hash
    }

    #[must_use]
    pub const fn hashes_match(&self) -> bool {
        self.hashes_match
    }

    #[must_use]
    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeNucleusSelfCheckError {
    report: RuntimeNucleusSelfCheckReport,
}

impl RuntimeNucleusSelfCheckError {
    #[must_use]
    pub const fn report(&self) -> &RuntimeNucleusSelfCheckReport {
        &self.report
    }
}

impl fmt::Display for RuntimeNucleusSelfCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.report.diagnostic())
    }
}

impl Error for RuntimeNucleusSelfCheckError {}

#[must_use]
pub fn preflight_runtime_nucleus_self_check(
    pinned_runtime_nucleus_hash: RuntimeNucleusHash,
    current_budget: &RuntimeChromeBudget,
) -> RuntimeNucleusSelfCheckReport {
    let current_runtime_nucleus_hash = current_budget.runtime_nucleus_hash;
    let hashes_match = pinned_runtime_nucleus_hash == current_runtime_nucleus_hash;
    let status = if hashes_match { "PASS" } else { "FAIL" };
    let diagnostic = format!(
        "runtime_nucleus_hash self-check {status}: pinned_runtime_nucleus_hash={pinned_runtime_nucleus_hash}; current_runtime_nucleus_hash={current_runtime_nucleus_hash}"
    );

    RuntimeNucleusSelfCheckReport {
        pinned_runtime_nucleus_hash,
        current_runtime_nucleus_hash,
        hashes_match,
        diagnostic,
    }
}

pub fn require_runtime_nucleus_self_check(
    pinned_runtime_nucleus_hash: RuntimeNucleusHash,
    current_budget: &RuntimeChromeBudget,
) -> Result<RuntimeNucleusSelfCheckReport, RuntimeNucleusSelfCheckError> {
    let report = preflight_runtime_nucleus_self_check(pinned_runtime_nucleus_hash, current_budget);
    if report.hashes_match() {
        Ok(report)
    } else {
        Err(RuntimeNucleusSelfCheckError { report })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeChromePreflightCheck {
    kind: RuntimeChromePreflightCheckKind,
    status: RuntimeChromePreflightStatus,
    required_bytes: ByteCost,
    available_bytes: ByteCost,
    slack_bytes: ByteCost,
    over_by_bytes: ByteCost,
    diagnostic: String,
}

impl RuntimeChromePreflightCheck {
    #[must_use]
    pub const fn kind(&self) -> RuntimeChromePreflightCheckKind {
        self.kind
    }

    #[must_use]
    pub const fn status(&self) -> RuntimeChromePreflightStatus {
        self.status
    }

    #[must_use]
    pub const fn required_bytes(&self) -> ByteCost {
        self.required_bytes
    }

    #[must_use]
    pub const fn available_bytes(&self) -> ByteCost {
        self.available_bytes
    }

    #[must_use]
    pub const fn slack_bytes(&self) -> ByteCost {
        self.slack_bytes
    }

    #[must_use]
    pub const fn over_by_bytes(&self) -> ByteCost {
        self.over_by_bytes
    }

    #[must_use]
    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChromePreflightCheckKind {
    RuntimeBudget,
    CompileProfile,
    ExpertBank,
    CommonBank,
    Bank0Free,
    WramHotArena,
    WramOverlay,
    SwitchBudget,
    Sram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChromePreflightStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChromePreflightCadence {
    PreTraining,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChromePreflightSeverity {
    Soft,
    Hard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WramFitReport {
    pub profile_id: CompileProfileId,
    pub overlay_demand_bytes: ByteCost,
    pub overlay_capacity_bytes: ByteCost,
    pub hot_arena_demand_bytes: ByteCost,
    pub hot_arena_floor_bytes: ByteCost,
    pub overlay_fits: bool,
    pub hot_arena_fits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchBudgetReport {
    pub profile_id: CompileProfileId,
    pub cap: u16,
    pub projected_min: u16,
    pub projected_typical: u16,
    pub fits: bool,
    pub severity: RuntimeChromePreflightSeverity,
}

#[must_use]
pub fn preflight_runtime_chrome_budget(
    budget: &RuntimeChromeBudget,
    demand: RuntimeChromePreflightDemand,
) -> RuntimeChromePreflightReport {
    preflight_runtime_chrome_budget_impl(
        budget,
        None,
        RuntimeChromePreflightCadence::PreTraining,
        demand,
        Vec::new(),
    )
}

#[must_use]
pub fn preflight_compile_request_runtime_chrome_budget(
    budget: &RuntimeChromeBudget,
    compile_request: &CompileRequest,
    demand: RuntimeChromePreflightDemand,
    cadence: RuntimeChromePreflightCadence,
) -> RuntimeChromePreflightReport {
    let Some(compile_profile) = profile_by_id(&compile_request.profile) else {
        return unknown_compile_profile_report(budget, compile_request.profile.clone(), demand);
    };

    let hard_failures =
        profile_mismatch_failures("CompileRequest", &compile_request.profile, &budget.profile);

    preflight_runtime_chrome_budget_impl(
        budget,
        Some(compile_profile),
        cadence,
        demand,
        hard_failures,
    )
}

#[must_use]
pub fn preflight_runtime_chrome_budget_with_profile(
    budget: &RuntimeChromeBudget,
    compile_profile: &CompileProfile,
    demand: RuntimeChromePreflightDemand,
    cadence: RuntimeChromePreflightCadence,
) -> RuntimeChromePreflightReport {
    preflight_runtime_chrome_budget_impl(
        budget,
        Some(compile_profile),
        cadence,
        demand,
        profile_mismatch_failures("CompileProfile", &compile_profile.id, &budget.profile),
    )
}

fn profile_mismatch_failures(
    source: &str,
    requested_profile: &CompileProfileId,
    budget_profile: &CompileProfileId,
) -> Vec<String> {
    if requested_profile == budget_profile {
        Vec::new()
    } else {
        vec![format!(
            "{source} profile {requested_profile} does not match RuntimeChromeBudget profile {budget_profile}",
        )]
    }
}

fn preflight_runtime_chrome_budget_impl(
    budget: &RuntimeChromeBudget,
    compile_profile: Option<&CompileProfile>,
    cadence: RuntimeChromePreflightCadence,
    demand: RuntimeChromePreflightDemand,
    mut hard_failures: Vec<String>,
) -> RuntimeChromePreflightReport {
    let required_expert_slots = demand.profile.n_experts();
    let available_expert_bank_slots = budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == BudgetSlotClass::ExpertBank)
        .count();

    if let Some(profile) = compile_profile
        && let Err(error) = profile.validate()
    {
        hard_failures.push(format!("invalid CompileProfile {}: {error}", profile.id));
    }

    if let Err(error) = budget.validate() {
        let diagnostic = format!("invalid RuntimeChromeBudget: {error}");
        return RuntimeChromePreflightReport::from_checks(
            demand,
            required_expert_slots,
            available_expert_bank_slots,
            vec![RuntimeChromePreflightCheck {
                kind: RuntimeChromePreflightCheckKind::RuntimeBudget,
                status: RuntimeChromePreflightStatus::Fail,
                required_bytes: ByteCost::ZERO,
                available_bytes: ByteCost::ZERO,
                slack_bytes: ByteCost::ZERO,
                over_by_bytes: ByteCost::ZERO,
                diagnostic,
            }],
            hard_failures,
            None,
            None,
        );
    }

    let mut checks = Vec::new();

    if required_expert_slots > 0 {
        if available_expert_bank_slots < usize::from(required_expert_slots) {
            hard_failures.push(format!(
                "RuntimeChromeBudget has {available_expert_bank_slots} ExpertBank slots but profile {:?} requires {required_expert_slots}",
                demand.profile
            ));
        }

        checks.push(expert_bank_check(budget, demand.profile));
    }

    // CommonBank fit uses the literal shared-bank usable byte sum from the
    // RuntimeChromeBudget contract; ExpertBank and Bank0Free reserve slack.
    checks.push(byte_check(
        RuntimeChromePreflightCheckKind::CommonBank,
        demand.common_bank_rom_bytes,
        sum_usable_slot_capacity(budget, BudgetSlotClass::CommonBank),
        "CommonBank shared/runtime-adjacent ROM",
    ));
    checks.push(byte_check(
        RuntimeChromePreflightCheckKind::Bank0Free,
        demand.bank0_resident_bytes,
        sum_effective_slot_capacity(budget, BudgetSlotClass::Bank0Free),
        "Bank0Free resident hot kernels",
    ));
    let wram_fit_report = compile_profile.map(|profile| wram_fit_report(profile, budget, demand));
    let overlay_capacity = compile_profile.map_or(
        ByteCost::new(u64::from(budget.wram_reserved.overlay)),
        |profile| ByteCost::new(u64::from(profile.wram_layout.overlay_bytes)),
    );
    let overlay_label = compile_profile.map_or_else(
        || "WRAM overlay payload".to_owned(),
        |profile| format!("WRAM overlay payload (CompileProfile={})", profile.id),
    );
    let hot_arena_label = compile_profile.map_or_else(
        || "WRAM hot arena".to_owned(),
        |profile| format!("WRAM hot arena (CompileProfile={})", profile.id),
    );
    checks.push(byte_check(
        RuntimeChromePreflightCheckKind::WramHotArena,
        demand.hot_arena_bytes,
        ByteCost::new(u64::from(budget.wram_reserved.hot_arena_floor)),
        &hot_arena_label,
    ));
    checks.push(byte_check(
        RuntimeChromePreflightCheckKind::WramOverlay,
        demand.overlay_payload_bytes,
        overlay_capacity,
        &overlay_label,
    ));
    let switch_budget_report = compile_profile.map(|profile| {
        let report = switch_budget_report(profile, demand.profile, cadence);
        checks.push(switch_budget_check(&report));
        report
    });
    checks.push(byte_check(
        RuntimeChromePreflightCheckKind::Sram,
        demand.sequence_state_bytes,
        ByteCost::new(u64::from(budget.sram_reserved)),
        "SRAM sequence-state reservation",
    ));

    RuntimeChromePreflightReport::from_checks(
        demand,
        required_expert_slots,
        available_expert_bank_slots,
        checks,
        hard_failures,
        wram_fit_report,
        switch_budget_report,
    )
}

fn unknown_compile_profile_report(
    budget: &RuntimeChromeBudget,
    profile_id: CompileProfileId,
    demand: RuntimeChromePreflightDemand,
) -> RuntimeChromePreflightReport {
    let required_expert_slots = demand.profile.n_experts();
    let available_expert_bank_slots = budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == BudgetSlotClass::ExpertBank)
        .count();
    let diagnostic =
        format!("CompileRequest profile {profile_id} is not in CompileProfile registry");

    RuntimeChromePreflightReport::from_checks(
        demand,
        required_expert_slots,
        available_expert_bank_slots,
        vec![RuntimeChromePreflightCheck {
            kind: RuntimeChromePreflightCheckKind::CompileProfile,
            status: RuntimeChromePreflightStatus::Fail,
            required_bytes: ByteCost::ZERO,
            available_bytes: ByteCost::ZERO,
            slack_bytes: ByteCost::ZERO,
            over_by_bytes: ByteCost::ZERO,
            diagnostic,
        }],
        Vec::new(),
        None,
        None,
    )
}

fn wram_fit_report(
    compile_profile: &CompileProfile,
    budget: &RuntimeChromeBudget,
    demand: RuntimeChromePreflightDemand,
) -> WramFitReport {
    let overlay_capacity_bytes =
        ByteCost::new(u64::from(compile_profile.wram_layout.overlay_bytes));
    let hot_arena_floor_bytes = ByteCost::new(u64::from(budget.wram_reserved.hot_arena_floor));

    WramFitReport {
        profile_id: compile_profile.id.clone(),
        overlay_demand_bytes: demand.overlay_payload_bytes,
        overlay_capacity_bytes,
        hot_arena_demand_bytes: demand.hot_arena_bytes,
        hot_arena_floor_bytes,
        overlay_fits: demand.overlay_payload_bytes <= overlay_capacity_bytes,
        hot_arena_fits: demand.hot_arena_bytes <= hot_arena_floor_bytes,
    }
}

fn switch_budget_report(
    compile_profile: &CompileProfile,
    model_profile: ModelSizeProfile,
    cadence: RuntimeChromePreflightCadence,
) -> SwitchBudgetReport {
    // Topology-only placeholders until bd-2r4h wires schedule/export evidence:
    // projected_min = n_blocks, projected_typical = 2 * n_blocks.
    let projected_min = projected_min_bank_switches_per_token(model_profile);
    let projected_typical = projected_typical_bank_switches_per_token(model_profile);
    let cap = compile_profile.max_bank_switches_per_token;

    SwitchBudgetReport {
        profile_id: compile_profile.id.clone(),
        cap,
        projected_min,
        projected_typical,
        fits: projected_typical <= cap,
        severity: switch_budget_severity(cadence),
    }
}

fn switch_budget_check(report: &SwitchBudgetReport) -> RuntimeChromePreflightCheck {
    let status = if report.fits {
        RuntimeChromePreflightStatus::Pass
    } else {
        match report.severity {
            RuntimeChromePreflightSeverity::Soft => RuntimeChromePreflightStatus::Warn,
            RuntimeChromePreflightSeverity::Hard => RuntimeChromePreflightStatus::Fail,
        }
    };
    let over_by = report.projected_typical.saturating_sub(report.cap);
    let margin = report.cap.saturating_sub(report.projected_typical);
    let severity = match report.severity {
        RuntimeChromePreflightSeverity::Soft => "soft",
        RuntimeChromePreflightSeverity::Hard => "hard",
    };
    let outcome = match status {
        RuntimeChromePreflightStatus::Pass => {
            format!("PASS with {} margin", switch_count(margin))
        }
        RuntimeChromePreflightStatus::Warn => format!("WARN by {}", switch_count(over_by)),
        RuntimeChromePreflightStatus::Fail => format!("FAIL by {}", switch_count(over_by)),
    };

    RuntimeChromePreflightCheck {
        kind: RuntimeChromePreflightCheckKind::SwitchBudget,
        status,
        required_bytes: ByteCost::ZERO,
        available_bytes: ByteCost::ZERO,
        slack_bytes: ByteCost::ZERO,
        over_by_bytes: ByteCost::ZERO,
        diagnostic: format!(
            "Switch budget (CompileProfile={}): cap={}, projected_min={}, projected_typical={}, severity={} -> {outcome}.",
            report.profile_id, report.cap, report.projected_min, report.projected_typical, severity
        ),
    }
}

fn switch_budget_severity(
    cadence: RuntimeChromePreflightCadence,
) -> RuntimeChromePreflightSeverity {
    match cadence {
        RuntimeChromePreflightCadence::PreTraining => RuntimeChromePreflightSeverity::Soft,
        RuntimeChromePreflightCadence::Export => RuntimeChromePreflightSeverity::Hard,
    }
}

fn projected_min_bank_switches_per_token(profile: ModelSizeProfile) -> u16 {
    if profile.n_experts() == 0 {
        0
    } else {
        u16::from(profile.n_blocks())
    }
}

fn projected_typical_bank_switches_per_token(profile: ModelSizeProfile) -> u16 {
    projected_min_bank_switches_per_token(profile).saturating_mul(2)
}

fn expert_bank_check(
    budget: &RuntimeChromeBudget,
    profile: ModelSizeProfile,
) -> RuntimeChromePreflightCheck {
    let expert_bytes = match compute_preflight_profile_expert_bytes(profile) {
        Ok(expert_bytes) => expert_bytes,
        Err(error) => {
            return RuntimeChromePreflightCheck {
                kind: RuntimeChromePreflightCheckKind::ExpertBank,
                status: RuntimeChromePreflightStatus::Fail,
                required_bytes: ByteCost::ZERO,
                available_bytes: ByteCost::ZERO,
                slack_bytes: ByteCost::ZERO,
                over_by_bytes: ByteCost::ZERO,
                diagnostic: format!("expert byte budget could not be computed: {error}"),
            };
        }
    };
    let Some(slot) = smallest_effective_slot_capacity(budget, BudgetSlotClass::ExpertBank) else {
        return RuntimeChromePreflightCheck {
            kind: RuntimeChromePreflightCheckKind::ExpertBank,
            status: RuntimeChromePreflightStatus::Fail,
            required_bytes: expert_bytes,
            available_bytes: ByteCost::ZERO,
            slack_bytes: ByteCost::ZERO,
            over_by_bytes: expert_bytes,
            diagnostic: format!("RuntimeChromeBudget has no ExpertBank slot for {profile:?}"),
        };
    };

    let label = format!(
        "Expert FFN up+down for d_model={} d_ff={}",
        profile.d_model(),
        profile.d_ff()
    );
    byte_check_with_capacity(
        RuntimeChromePreflightCheckKind::ExpertBank,
        expert_bytes,
        slot,
        &label,
    )
}

fn byte_check(
    kind: RuntimeChromePreflightCheckKind,
    required_bytes: ByteCost,
    available_bytes: ByteCost,
    label: &str,
) -> RuntimeChromePreflightCheck {
    let capacity = RuntimeSlotCapacity {
        usable_bytes: available_bytes,
        reserved_slack: ByteCost::ZERO,
        available_bytes,
    };
    byte_check_with_capacity(kind, required_bytes, capacity, label)
}

fn byte_check_with_capacity(
    kind: RuntimeChromePreflightCheckKind,
    required_bytes: ByteCost,
    capacity: RuntimeSlotCapacity,
    label: &str,
) -> RuntimeChromePreflightCheck {
    if let Some(slack_bytes) = capacity.available_bytes.checked_sub(required_bytes) {
        RuntimeChromePreflightCheck {
            kind,
            status: RuntimeChromePreflightStatus::Pass,
            required_bytes,
            available_bytes: capacity.available_bytes,
            slack_bytes,
            over_by_bytes: ByteCost::ZERO,
            diagnostic: format!(
                "{label} = {}; capacity = {} usable, {} reserved -> {} available. PASS with {} margin.",
                byte_count(required_bytes),
                byte_count(capacity.usable_bytes),
                byte_count(capacity.reserved_slack),
                byte_count(capacity.available_bytes),
                byte_count(slack_bytes)
            ),
        }
    } else {
        let over_by_bytes = required_bytes - capacity.available_bytes;
        RuntimeChromePreflightCheck {
            kind,
            status: RuntimeChromePreflightStatus::Fail,
            required_bytes,
            available_bytes: capacity.available_bytes,
            slack_bytes: ByteCost::ZERO,
            over_by_bytes,
            diagnostic: format!(
                "{label} = {}; capacity = {} usable, {} reserved -> {} available. FAIL by {}.",
                byte_count(required_bytes),
                byte_count(capacity.usable_bytes),
                byte_count(capacity.reserved_slack),
                byte_count(capacity.available_bytes),
                byte_count(over_by_bytes)
            ),
        }
    }
}

fn byte_count(bytes: ByteCost) -> String {
    let bytes = bytes.as_u64();
    let unit = if bytes == 1 { "byte" } else { "bytes" };
    format!("{bytes} {unit}")
}

fn switch_count(switches: u16) -> String {
    let unit = if switches == 1 { "switch" } else { "switches" };
    format!("{switches} {unit}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeSlotCapacity {
    usable_bytes: ByteCost,
    reserved_slack: ByteCost,
    available_bytes: ByteCost,
}

fn effective_slot_capacity(slot: &RomBudgetSlot) -> ByteCost {
    ByteCost::new(u64::from(
        slot.usable_bytes
            .saturating_sub(u32::from(slot.reserved_slack)),
    ))
}

fn usable_slot_capacity(slot: &RomBudgetSlot) -> ByteCost {
    ByteCost::new(u64::from(slot.usable_bytes))
}

fn effective_slot_capacity_with_slack(slot: &RomBudgetSlot) -> RuntimeSlotCapacity {
    RuntimeSlotCapacity {
        usable_bytes: ByteCost::new(u64::from(slot.usable_bytes)),
        reserved_slack: ByteCost::new(u64::from(slot.reserved_slack)),
        available_bytes: effective_slot_capacity(slot),
    }
}

fn sum_effective_slot_capacity(budget: &RuntimeChromeBudget, class: BudgetSlotClass) -> ByteCost {
    budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == class)
        .map(effective_slot_capacity)
        .fold(ByteCost::ZERO, ByteCost::saturating_add)
}

fn sum_usable_slot_capacity(budget: &RuntimeChromeBudget, class: BudgetSlotClass) -> ByteCost {
    budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == class)
        .map(usable_slot_capacity)
        .fold(ByteCost::ZERO, ByteCost::saturating_add)
}

fn smallest_effective_slot_capacity(
    budget: &RuntimeChromeBudget,
    class: BudgetSlotClass,
) -> Option<RuntimeSlotCapacity> {
    budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == class)
        .map(effective_slot_capacity_with_slack)
        .min_by_key(|capacity| capacity.available_bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertBudgetPreflightReport {
    static_budget: StaticBudgetReport,
}

impl ExpertBudgetPreflightReport {
    pub fn check_expert_slot(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: ByteCost,
    ) -> Result<Self, ExpertBudgetError> {
        Ok(Self {
            static_budget: StaticBudgetReport::for_expert_checked(
                plan,
                d_model,
                d_ff,
                Some(expert_slot_usable_bytes),
            )?,
        })
    }

    pub fn check_expert_slot_with_logging(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: ByteCost,
        emitter: &TrainingLogEmitter,
    ) -> Result<Self, PreflightLoggingError> {
        let report = match Self::check_expert_slot(plan, d_model, d_ff, expert_slot_usable_bytes) {
            Ok(report) => report,
            Err(error) => {
                emit_budget_error(emitter, expert_slot_usable_bytes, error)?;
                return Err(PreflightLoggingError::Budget(error));
            }
        };
        report.emit_structured_log(emitter)?;
        Ok(report)
    }

    #[must_use]
    pub const fn static_budget(self) -> StaticBudgetReport {
        self.static_budget
    }

    #[must_use]
    pub fn expert_bytes(self) -> ByteCost {
        self.static_budget.expert_bytes()
    }

    #[must_use]
    pub fn expert_slot_fit(self) -> ExpertSlotFit {
        self.static_budget
            .expert_slot_fit()
            .expect("preflight report always has an expert slot budget")
    }

    #[must_use]
    pub fn fits_expert_slot(self) -> bool {
        self.expert_slot_fit().fits()
    }

    pub fn emit_structured_log(
        self,
        emitter: &TrainingLogEmitter,
    ) -> Result<(), LoggingEventError> {
        emitter.expert_slot_preflight(&self.to_preflight_event()?)
    }

    pub fn to_preflight_event(self) -> Result<ExpertSlotPreflightEvent, LoggingEventError> {
        let expert_bytes = self.expert_bytes();
        let slot_bytes = self
            .static_budget()
            .expert_slot_usable_bytes()
            .expect("preflight report always has an expert slot budget");
        let fit = self.expert_slot_fit();
        match fit {
            ExpertSlotFit::Fits { slack } => ExpertSlotPreflightEvent::fits(
                format!(
                    "expert payload fits slot with {} slack bytes",
                    slack.as_u64()
                ),
                expert_bytes.as_u64(),
                slot_bytes.as_u64(),
                slack.as_u64(),
            ),
            ExpertSlotFit::Exceeds { over_by } => ExpertSlotPreflightEvent::exceeds(
                format!("expert payload exceeds slot by {} bytes", over_by.as_u64()),
                expert_bytes.as_u64(),
                slot_bytes.as_u64(),
                over_by.as_u64(),
            ),
        }
    }
}

fn emit_budget_error(
    emitter: &TrainingLogEmitter,
    expert_slot_usable_bytes: ByteCost,
    error: ExpertBudgetError,
) -> Result<(), LoggingEventError> {
    emitter.expert_slot_preflight(&ExpertSlotPreflightEvent::invalid(
        format!("expert slot budget could not be computed: {error}"),
        expert_slot_usable_bytes.as_u64(),
    )?)
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreflightLoggingError {
    Budget(ExpertBudgetError),
    Logging(LoggingEventError),
}

impl fmt::Display for PreflightLoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl Error for PreflightLoggingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Budget(error) => Some(error),
            Self::Logging(error) => Some(error),
        }
    }
}

impl From<ExpertBudgetError> for PreflightLoggingError {
    fn from(error: ExpertBudgetError) -> Self {
        Self::Budget(error)
    }
}

impl From<LoggingEventError> for PreflightLoggingError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfilePreflightError {
    MissingExpertBanks,
    EmptyExpertBank,
    InvalidRuntimeChromeBudget {
        error: RuntimeChromeBudgetValidationError,
    },
    ExpertBudget {
        error: ExpertBudgetError,
    },
    ExpertExceedsSmallestBank {
        profile: ModelSizeProfile,
        expert_bytes: ByteCost,
        smallest_expert_bank_usable_bytes: ByteCost,
        over_by: ByteCost,
    },
}

impl fmt::Display for ProfilePreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExpertBanks => {
                f.write_str("runtime chrome budget surface has no ExpertBank slots")
            }
            Self::EmptyExpertBank => {
                f.write_str("runtime chrome budget surface has an empty ExpertBank slot")
            }
            Self::InvalidRuntimeChromeBudget { error } => {
                write!(f, "invalid RuntimeChromeBudget: {error}")
            }
            Self::ExpertBudget { error } => {
                write!(f, "expert byte budget could not be computed: {error}")
            }
            Self::ExpertExceedsSmallestBank {
                profile,
                expert_bytes,
                smallest_expert_bank_usable_bytes,
                over_by,
            } => write!(
                f,
                "{profile:?} expert payload {expert_bytes} exceeds smallest ExpertBank usable capacity {smallest_expert_bank_usable_bytes} by {over_by}"
            ),
        }
    }
}

impl Error for ProfilePreflightError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_artifact::weight_plan::{
        ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
    };
    use gbf_foundation::{Hash256, TargetProfileId};
    use gbf_hw::calibration::CalibrationSetRef;
    use gbf_model::budget::ExpertBudgetMetadata;
    use gbf_policy::{
        CalibrationConfidenceRequirement, CompileObjective, RuntimeMode, bringup_profile,
    };

    use crate::logging::{TestEventCollector, TestEventKind, TestFieldValue};

    use super::*;

    #[test]
    fn preflight_expert_budget_uses_model_compute_expert_bytes() {
        let plan = default_plan();
        let expected = compute_expert_bytes_checked(&plan, 128, 224).unwrap();

        assert_eq!(
            compute_preflight_expert_bytes(&plan, 128, 224),
            Ok(expected)
        );

        let report =
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 224, ByteCost::new(16_384))
                .unwrap();
        assert_eq!(report.expert_bytes(), expected);
        assert_eq!(report.static_budget().expert_bytes(), expected);
        assert_eq!(
            report.expert_slot_fit(),
            ExpertSlotFit::Fits {
                slack: ByteCost::new(1_294),
            }
        );
        assert!(report.fits_expert_slot());
    }

    #[test]
    fn preflight_profile_expert_budget_includes_model_metadata_overhead() {
        let profile = ModelSizeProfile::moe_tiny(4).unwrap();
        let metadata = ExpertBudgetMetadata::default().total();

        assert_eq!(
            compute_preflight_profile_expert_bytes(profile).unwrap(),
            profile.expert_byte_cost() + metadata
        );
    }

    #[test]
    fn preflight_logging_path_emits_pass_event_from_real_budget_report() {
        let plan = default_plan();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let report = ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &plan,
            128,
            224,
            ByteCost::new(16_384),
            &emitter,
        )
        .unwrap();

        assert!(report.fits_expert_slot());
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::Preflight);
        assert_eq!(
            events[0].field("check_name"),
            Some(&TestFieldValue::String("expert_slot_budget".to_owned()))
        );
        assert_eq!(
            events[0].field("status"),
            Some(&TestFieldValue::String("pass".to_owned()))
        );
        assert_eq!(
            events[0].field("budget_computed"),
            Some(&TestFieldValue::Bool(true))
        );
        assert_eq!(
            events[0].field("expert_bytes"),
            Some(&TestFieldValue::U64(15_090))
        );
        assert_eq!(
            events[0].field("expert_slot_usable_bytes"),
            Some(&TestFieldValue::U64(16_384))
        );
        assert_eq!(
            events[0].field("slack_bytes"),
            Some(&TestFieldValue::U64(1_294))
        );
    }

    #[test]
    fn preflight_reports_over_budget_experts_before_training() {
        let plan = default_plan();

        let report =
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 224, ByteCost::new(15_000))
                .unwrap();

        assert_eq!(
            report.expert_slot_fit(),
            ExpertSlotFit::Exceeds {
                over_by: ByteCost::new(90),
            }
        );
        assert!(!report.fits_expert_slot());
    }

    #[test]
    fn preflight_logging_path_emits_fail_event_for_over_budget_report() {
        let plan = default_plan();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let report = ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &plan,
            128,
            224,
            ByteCost::new(15_000),
            &emitter,
        )
        .unwrap();

        assert!(!report.fits_expert_slot());
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].field("status"),
            Some(&TestFieldValue::String("fail".to_owned()))
        );
        assert_eq!(
            events[0].field("detail"),
            Some(&TestFieldValue::String(
                "expert payload exceeds slot by 90 bytes".to_owned()
            ))
        );
        assert_eq!(
            events[0].field("expert_bytes"),
            Some(&TestFieldValue::U64(15_090))
        );
        assert_eq!(
            events[0].field("over_by_bytes"),
            Some(&TestFieldValue::U64(90))
        );
    }

    #[test]
    fn preflight_rejects_zero_expert_dimensions() {
        let plan = default_plan();

        assert_eq!(
            compute_preflight_expert_bytes(&plan, 0, 224),
            Err(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        assert_eq!(
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 0, ByteCost::new(16_384),),
            Err(ExpertBudgetError::EmptyDimension { field: "d_ff" })
        );
    }

    #[test]
    fn profile_expert_budget_passes_when_profile_fits_smallest_expert_bank() {
        let profile = ModelSizeProfile::moe_tiny(4).unwrap();
        let budget = ExpertBankBudgetSurface::new(vec![
            ByteCost::new(8_192),
            ByteCost::new(4_530),
            ByteCost::new(16_384),
        ])
        .unwrap();

        let report = preflight_profile_expert_bank_budget(&budget, profile).unwrap();

        assert_eq!(report.profile(), profile);
        assert_eq!(report.expert_bytes(), ByteCost::new(4_530));
        assert_eq!(
            report.smallest_expert_bank_usable_bytes(),
            ByteCost::new(4_530)
        );
        assert_eq!(report.slack_bytes(), ByteCost::ZERO);
    }

    #[test]
    fn profile_expert_budget_reports_positive_slack() {
        let profile = ModelSizeProfile::moe_tiny(2).unwrap();
        let budget =
            ExpertBankBudgetSurface::new(vec![ByteCost::new(16_384), ByteCost::new(8_192)])
                .unwrap();

        let report = preflight_profile_expert_bank_budget(&budget, profile).unwrap();

        assert_eq!(report.expert_bytes(), ByteCost::new(4_530));
        assert_eq!(
            report.smallest_expert_bank_usable_bytes(),
            ByteCost::new(8_192)
        );
        assert_eq!(report.slack_bytes(), ByteCost::new(3_662));
    }

    #[test]
    fn profile_expert_budget_rejects_profile_larger_than_smallest_expert_bank() {
        let profile = ModelSizeProfile::upper_bank_candidate(128, 4).unwrap();
        let budget =
            ExpertBankBudgetSurface::new(vec![ByteCost::new(16_384), ByteCost::new(12_000)])
                .unwrap();

        assert_eq!(
            preflight_profile_expert_bank_budget(&budget, profile),
            Err(ProfilePreflightError::ExpertExceedsSmallestBank {
                profile,
                expert_bytes: ByteCost::new(12_978),
                smallest_expert_bank_usable_bytes: ByteCost::new(12_000),
                over_by: ByteCost::new(978),
            })
        );
    }

    #[test]
    fn profile_expert_budget_selects_smallest_expert_bank() {
        let budget = ExpertBankBudgetSurface::new(vec![
            ByteCost::new(16_384),
            ByteCost::new(9_000),
            ByteCost::new(12_000),
        ])
        .unwrap();
        let profile = ModelSizeProfile::upper_bank_candidate(96, 4).unwrap();

        assert_eq!(
            preflight_profile_expert_bank_budget(&budget, profile),
            Err(ProfilePreflightError::ExpertExceedsSmallestBank {
                profile,
                expert_bytes: ByteCost::new(9_842),
                smallest_expert_bank_usable_bytes: ByteCost::new(9_000),
                over_by: ByteCost::new(842),
            })
        );
    }

    #[test]
    fn profile_expert_budget_surface_rejects_missing_or_empty_banks() {
        assert_eq!(
            ExpertBankBudgetSurface::new(vec![]),
            Err(ProfilePreflightError::MissingExpertBanks)
        );
        assert_eq!(
            ExpertBankBudgetSurface::new(vec![ByteCost::new(16_384), ByteCost::ZERO]),
            Err(ProfilePreflightError::EmptyExpertBank)
        );
    }

    #[test]
    fn expert_budget_surface_from_runtime_budget_uses_reserved_slack() {
        let budget = runtime_budget_fixture(12_384, 2);
        let surface = ExpertBankBudgetSurface::from_runtime_chrome_budget(&budget).unwrap();

        assert_eq!(
            surface.expert_bank_usable_bytes(),
            &[ByteCost::new(12_000), ByteCost::new(12_000)]
        );
        assert_eq!(
            surface.smallest_expert_bank_usable_bytes(),
            ByteCost::new(12_000)
        );
    }

    #[test]
    fn runtime_chrome_preflight_passes_with_structured_budget_margins() {
        let budget = runtime_budget_fixture(16_384, 4);
        let profile = ModelSizeProfile::moe_tiny(4).unwrap();
        let demand = RuntimeChromePreflightDemand::new(
            profile,
            ByteCost::new(12_000),
            ByteCost::new(2_048),
            ByteCost::new(3_712),
            ByteCost::new(380),
            ByteCost::new(768),
        );

        let report = preflight_runtime_chrome_budget(&budget, demand);

        assert!(report.fits_envelope());
        assert!(report.hard_failures().is_empty());
        assert_eq!(report.required_expert_slots(), 4);
        assert_eq!(report.available_expert_bank_slots(), 4);
        assert_eq!(report.checks().len(), 6);

        let expert = check(&report, RuntimeChromePreflightCheckKind::ExpertBank);
        assert_eq!(expert.status(), RuntimeChromePreflightStatus::Pass);
        assert_eq!(expert.required_bytes(), ByteCost::new(4_530));
        assert_eq!(expert.available_bytes(), ByteCost::new(16_000));
        assert_eq!(expert.slack_bytes(), ByteCost::new(11_470));
        assert!(
            expert
                .diagnostic()
                .contains("384 bytes reserved -> 16000 bytes available")
        );

        let common = check(&report, RuntimeChromePreflightCheckKind::CommonBank);
        assert_eq!(common.available_bytes(), ByteCost::new(16_384));
        assert_eq!(common.slack_bytes(), ByteCost::new(4_384));

        let hot_arena = check(&report, RuntimeChromePreflightCheckKind::WramHotArena);
        assert_eq!(hot_arena.available_bytes(), ByteCost::new(4_096));
        assert_eq!(hot_arena.slack_bytes(), ByteCost::new(384));

        let overlay = check(&report, RuntimeChromePreflightCheckKind::WramOverlay);
        assert_eq!(overlay.available_bytes(), ByteCost::new(512));
        assert_eq!(overlay.slack_bytes(), ByteCost::new(132));
    }

    #[test]
    fn runtime_chrome_preflight_reports_runtime_budget_failures() {
        let budget = runtime_budget_fixture(12_384, 2);
        let profile = ModelSizeProfile::upper_bank_candidate(128, 4).unwrap();
        let demand = RuntimeChromePreflightDemand::new(
            profile,
            ByteCost::new(16_500),
            ByteCost::new(9_000),
            ByteCost::new(4_097),
            ByteCost::new(513),
            ByteCost::new(1_025),
        );

        let report = preflight_runtime_chrome_budget(&budget, demand);

        assert!(!report.fits_envelope());
        assert_eq!(report.required_expert_slots(), 4);
        assert_eq!(report.available_expert_bank_slots(), 2);
        assert!(
            report
                .hard_failures()
                .iter()
                .any(|failure| failure.contains("has 2 ExpertBank slots"))
        );

        let expert = check(&report, RuntimeChromePreflightCheckKind::ExpertBank);
        assert_eq!(expert.status(), RuntimeChromePreflightStatus::Fail);
        assert_eq!(expert.required_bytes(), ByteCost::new(12_978));
        assert_eq!(expert.available_bytes(), ByteCost::new(12_000));
        assert_eq!(expert.over_by_bytes(), ByteCost::new(978));

        let common = check(&report, RuntimeChromePreflightCheckKind::CommonBank);
        assert_eq!(common.status(), RuntimeChromePreflightStatus::Fail);
        assert_eq!(common.available_bytes(), ByteCost::new(16_384));
        assert_eq!(common.over_by_bytes(), ByteCost::new(116));

        let bank0 = check(&report, RuntimeChromePreflightCheckKind::Bank0Free);
        assert_eq!(bank0.status(), RuntimeChromePreflightStatus::Fail);
        assert_eq!(bank0.available_bytes(), ByteCost::new(7_680));
        assert_eq!(bank0.over_by_bytes(), ByteCost::new(1_320));

        let hot_arena = check(&report, RuntimeChromePreflightCheckKind::WramHotArena);
        assert_eq!(hot_arena.status(), RuntimeChromePreflightStatus::Fail);
        assert_eq!(hot_arena.over_by_bytes(), ByteCost::new(1));
        assert!(hot_arena.diagnostic().contains("WRAM hot arena"));

        let overlay = check(&report, RuntimeChromePreflightCheckKind::WramOverlay);
        assert_eq!(overlay.over_by_bytes(), ByteCost::new(1));

        let sram = check(&report, RuntimeChromePreflightCheckKind::Sram);
        assert_eq!(sram.over_by_bytes(), ByteCost::new(1));
    }

    #[test]
    fn runtime_chrome_preflight_reports_invalid_runtime_budget() {
        let mut budget = runtime_budget_fixture(16_384, 4);
        budget.reference_shell_modules.clear();
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::ZERO,
            ByteCost::ZERO,
            ByteCost::ZERO,
            ByteCost::ZERO,
            ByteCost::ZERO,
        );

        let report = preflight_runtime_chrome_budget(&budget, demand);

        assert!(!report.fits_envelope());
        assert_eq!(report.checks().len(), 1);
        assert_eq!(
            report.checks()[0].kind(),
            RuntimeChromePreflightCheckKind::RuntimeBudget
        );
        assert!(
            report.hard_failures()[0].contains("must record at least one reference shell module")
        );
    }

    #[test]
    fn preflight_wram_fit_report_uses_compile_request_profile() {
        let budget = runtime_budget_fixture(16_384, 4);
        let request = compile_request_fixture("Bringup");
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::new(12_000),
            ByteCost::new(2_048),
            ByteCost::new(3_712),
            ByteCost::new(380),
            ByteCost::new(768),
        );

        let report = preflight_compile_request_runtime_chrome_budget(
            &budget,
            &request,
            demand,
            RuntimeChromePreflightCadence::PreTraining,
        );

        assert!(report.fits_envelope());
        assert!(report.hard_failures().is_empty());
        assert!(report.risk_warnings().is_empty());
        let wram = report.wram_fit_report().expect("WRAM report is present");
        assert_eq!(wram.profile_id.as_str(), "Bringup");
        assert_eq!(wram.overlay_demand_bytes, ByteCost::new(380));
        assert_eq!(wram.overlay_capacity_bytes, ByteCost::new(512));
        assert_eq!(wram.hot_arena_demand_bytes, ByteCost::new(3_712));
        assert_eq!(wram.hot_arena_floor_bytes, ByteCost::new(4_096));
        assert!(wram.overlay_fits);
        assert!(wram.hot_arena_fits);

        let overlay = check(&report, RuntimeChromePreflightCheckKind::WramOverlay);
        assert_eq!(overlay.status(), RuntimeChromePreflightStatus::Pass);
        assert_eq!(overlay.available_bytes(), ByteCost::new(512));
        assert!(overlay.diagnostic().contains("CompileProfile=Bringup"));

        let switch = report
            .switch_budget_report()
            .expect("switch budget report is present");
        assert_eq!(switch.profile_id.as_str(), "Bringup");
        assert_eq!(switch.cap, 8);
        assert_eq!(switch.projected_min, 4);
        assert_eq!(switch.projected_typical, 8);
        assert!(switch.fits);
    }

    #[test]
    fn preflight_wram_overlay_over_profile_capacity_is_hard_failure() {
        let budget = runtime_budget_fixture(16_384, 4);
        let request = compile_request_fixture("Bringup");
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::new(12_000),
            ByteCost::new(2_048),
            ByteCost::new(3_712),
            ByteCost::new(513),
            ByteCost::new(768),
        );

        let report = preflight_compile_request_runtime_chrome_budget(
            &budget,
            &request,
            demand,
            RuntimeChromePreflightCadence::PreTraining,
        );

        assert!(!report.fits_envelope());
        let wram = report.wram_fit_report().expect("WRAM report is present");
        assert!(!wram.overlay_fits);
        assert!(wram.hot_arena_fits);
        let overlay = check(&report, RuntimeChromePreflightCheckKind::WramOverlay);
        assert_eq!(overlay.status(), RuntimeChromePreflightStatus::Fail);
        assert_eq!(overlay.over_by_bytes(), ByteCost::new(1));
        assert!(overlay.diagnostic().contains("CompileProfile=Bringup"));
        assert!(overlay.diagnostic().contains("FAIL by 1 byte"));
    }

    #[test]
    fn preflight_with_profile_rejects_budget_profile_mismatch() {
        let budget = runtime_budget_fixture(16_384, 4);
        let mut profile = bringup_profile().clone();
        profile.id = CompileProfileId::from("Detached");
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::new(12_000),
            ByteCost::new(2_048),
            ByteCost::new(3_712),
            ByteCost::new(380),
            ByteCost::new(768),
        );

        let report = preflight_runtime_chrome_budget_with_profile(
            &budget,
            &profile,
            demand,
            RuntimeChromePreflightCadence::PreTraining,
        );

        assert!(!report.fits_envelope());
        assert_eq!(report.hard_failures().len(), 1);
        assert!(report.hard_failures()[0].contains(
            "CompileProfile profile Detached does not match RuntimeChromeBudget profile Bringup"
        ));
        assert_eq!(
            report
                .wram_fit_report()
                .expect("WRAM report is still populated")
                .profile_id
                .as_str(),
            "Detached"
        );
    }

    #[test]
    fn preflight_switch_budget_over_cap_is_soft_before_training() {
        let budget = runtime_budget_fixture(16_384, 4);
        let mut profile = bringup_profile().clone();
        profile.max_bank_switches_per_token = 3;
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::new(12_000),
            ByteCost::new(2_048),
            ByteCost::new(3_712),
            ByteCost::new(380),
            ByteCost::new(768),
        );

        let report = preflight_runtime_chrome_budget_with_profile(
            &budget,
            &profile,
            demand,
            RuntimeChromePreflightCadence::PreTraining,
        );

        assert!(report.fits_envelope());
        assert!(report.hard_failures().is_empty());
        assert_eq!(report.risk_warnings().len(), 1);
        let switch = report
            .switch_budget_report()
            .expect("switch budget report is present");
        assert_eq!(switch.cap, 3);
        assert_eq!(switch.projected_typical, 8);
        assert!(!switch.fits);
        assert_eq!(switch.severity, RuntimeChromePreflightSeverity::Soft);
        let check = check(&report, RuntimeChromePreflightCheckKind::SwitchBudget);
        assert_eq!(check.status(), RuntimeChromePreflightStatus::Warn);
        assert!(check.diagnostic().contains("severity=soft"));
        assert!(check.diagnostic().contains("CompileProfile=Bringup"));
    }

    #[test]
    fn preflight_switch_budget_over_cap_can_be_hard_for_export_callers() {
        let budget = runtime_budget_fixture(16_384, 4);
        let mut profile = bringup_profile().clone();
        profile.max_bank_switches_per_token = 3;
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::new(12_000),
            ByteCost::new(2_048),
            ByteCost::new(3_712),
            ByteCost::new(380),
            ByteCost::new(768),
        );

        let report = preflight_runtime_chrome_budget_with_profile(
            &budget,
            &profile,
            demand,
            RuntimeChromePreflightCadence::Export,
        );

        assert!(!report.fits_envelope());
        assert!(report.risk_warnings().is_empty());
        let switch = report
            .switch_budget_report()
            .expect("switch budget report is present");
        assert_eq!(switch.severity, RuntimeChromePreflightSeverity::Hard);
        let check = check(&report, RuntimeChromePreflightCheckKind::SwitchBudget);
        assert_eq!(check.status(), RuntimeChromePreflightStatus::Fail);
        assert!(check.diagnostic().contains("severity=hard"));
    }

    #[test]
    fn preflight_compile_request_reports_unknown_profile() {
        let budget = runtime_budget_fixture(16_384, 4);
        let request = compile_request_fixture("NotAProfile");
        let demand = RuntimeChromePreflightDemand::new(
            ModelSizeProfile::moe_tiny(4).unwrap(),
            ByteCost::ZERO,
            ByteCost::ZERO,
            ByteCost::ZERO,
            ByteCost::ZERO,
            ByteCost::ZERO,
        );

        let report = preflight_compile_request_runtime_chrome_budget(
            &budget,
            &request,
            demand,
            RuntimeChromePreflightCadence::PreTraining,
        );

        assert!(!report.fits_envelope());
        assert!(report.wram_fit_report().is_none());
        assert!(report.switch_budget_report().is_none());
        let check = check(&report, RuntimeChromePreflightCheckKind::CompileProfile);
        assert_eq!(check.status(), RuntimeChromePreflightStatus::Fail);
        assert!(check.diagnostic().contains("NotAProfile"));
    }

    #[test]
    fn preflight_logging_path_emits_fail_event_for_invalid_budget_input() {
        let plan = default_plan();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let error = ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &plan,
            0,
            224,
            ByteCost::new(16_384),
            &emitter,
        )
        .unwrap_err();

        assert_eq!(
            error,
            PreflightLoggingError::Budget(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].field("status"),
            Some(&TestFieldValue::String("fail".to_owned()))
        );
        assert_eq!(
            events[0].field("budget_computed"),
            Some(&TestFieldValue::Bool(false))
        );
        assert_eq!(
            events[0].field("detail"),
            Some(&TestFieldValue::String(
                "expert slot budget could not be computed: d_model must be nonzero".to_owned()
            ))
        );
    }

    fn default_plan() -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        )
    }

    fn check(
        report: &RuntimeChromePreflightReport,
        kind: RuntimeChromePreflightCheckKind,
    ) -> &RuntimeChromePreflightCheck {
        report
            .checks()
            .iter()
            .find(|check| check.kind() == kind)
            .expect("check kind is present")
    }

    fn runtime_budget_fixture(
        expert_slot_usable_bytes: u32,
        expert_slots: u16,
    ) -> RuntimeChromeBudget {
        let mut rom_slots = vec![
            RomBudgetSlot {
                id: gbf_foundation::BudgetSlotId::new(0),
                class: BudgetSlotClass::Bank0Free,
                usable_bytes: 8 * 1024,
                reserved_slack: 512,
                placement_caps: std::collections::BTreeSet::from([
                    gbf_policy::PlacementProfile::StrictOnePerBank,
                ]),
            },
            RomBudgetSlot {
                id: gbf_foundation::BudgetSlotId::new(1),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 512,
                placement_caps: std::collections::BTreeSet::from([
                    gbf_policy::PlacementProfile::Budgeted,
                ]),
            },
        ];

        for offset in 0..expert_slots {
            rom_slots.push(RomBudgetSlot {
                id: gbf_foundation::BudgetSlotId::new(2 + offset),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: expert_slot_usable_bytes,
                reserved_slack: 384,
                placement_caps: std::collections::BTreeSet::from([
                    gbf_policy::PlacementProfile::StrictOnePerBank,
                    gbf_policy::PlacementProfile::Budgeted,
                ]),
            });
        }

        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: gbf_policy::RuntimeNucleusHash::real(Hash256::from_bytes(
                [0x27; 32],
            )),
            reference_shell_modules: RuntimeChromeBudget::pinned_reference_shell_modules(),
            rom_slots,
            memory_caps: gbf_policy::RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: Hash256::from_bytes([0x28; 32]),
            },
            wram_reserved: gbf_policy::WramReserved::new(512, 4_096, 1_536)
                .expect("valid WRAM reservation"),
            sram_reserved: 1_024,
        }
    }

    fn compile_request_fixture(profile: &str) -> CompileRequest {
        CompileRequest {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from(profile),
            objective: CompileObjective {
                service: None,
                max_cycles_per_token: None,
                max_bank_switches_per_token: None,
                max_sram_page_switches_per_token: None,
                min_sustained_throughput_tokens_per_megacycle: None,
                min_ui_headroom_pct: 0,
                max_rom_bytes: None,
                risk: gbf_policy::RiskPolicy {
                    cycle_quantile: 90,
                    switch_quantile: 95,
                    calibration_confidence_requirement:
                        CalibrationConfidenceRequirement::NoMinimumConfidence,
                    fallback_profile: None,
                    fallback_runtime_mode: Some(RuntimeMode::Safe),
                },
            },
            calibration_set_ref: CalibrationSetRef {
                platform: None,
                kernel: None,
                runtime: None,
            },
            required_features: BTreeSet::new(),
            constraint_overrides: None,
            requested_runtime_modes: BTreeSet::new(),
        }
    }
}
