//! Export-time runtime chrome budget re-validation helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use gbf_foundation::BudgetSlotId;
use gbf_policy::{
    BudgetSlotClass, ReValidationOutcome, RomBudgetSlot, RuntimeChromeBudget,
    RuntimeChromeBudgetReValidation, RuntimeNucleusHash, RuntimeShellModule, WramReserved,
    revalidate_runtime_chrome_budget,
};
use serde::{Deserialize, Serialize};

use crate::preflight::{
    RuntimeChromePreflightCheck, RuntimeChromePreflightCheckKind, RuntimeChromePreflightDemand,
    RuntimeChromePreflightReport, RuntimeChromePreflightStatus, preflight_runtime_chrome_budget,
};

pub const BUDGET_DRIFT_FILE_NAME: &str = "budget_drift.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRuntimeChromeRevalidationReport {
    pub drift: BudgetDriftReport,
    pub preflight: ExportRuntimeChromePreflightSummary,
    pub policy_revalidation: RuntimeChromeBudgetReValidation,
    pub blocks_export: bool,
    pub diagnostic: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRuntimeChromeRevalidationArtifact {
    pub report: ExportRuntimeChromeRevalidationReport,
    pub budget_drift_path: PathBuf,
}

#[derive(Debug)]
pub enum ExportRuntimeChromeRevalidationError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Blocked {
        artifact: Box<ExportRuntimeChromeRevalidationArtifact>,
    },
}

impl fmt::Display for ExportRuntimeChromeRevalidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "budget drift report I/O failed: {error}"),
            Self::Json(error) => write!(f, "budget drift report JSON failed: {error}"),
            Self::Blocked { artifact } => write!(
                f,
                "final export blocked by runtime chrome revalidation: {} (report={})",
                artifact.report.diagnostic,
                artifact.budget_drift_path.display()
            ),
        }
    }
}

impl Error for ExportRuntimeChromeRevalidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Blocked { .. } => None,
        }
    }
}

impl From<std::io::Error> for ExportRuntimeChromeRevalidationError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ExportRuntimeChromeRevalidationError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetDriftReport {
    pub runtime_nucleus_hashes_match: bool,
    pub training_hash: RuntimeNucleusHash,
    pub current_hash: RuntimeNucleusHash,
    pub training_modules: BTreeSet<RuntimeShellModule>,
    pub current_modules: BTreeSet<RuntimeShellModule>,
    /// ROM slot drift keyed by `(BudgetSlotClass, BudgetSlotId)`.
    ///
    /// If a slot keeps the same id but changes class, the report emits a
    /// removed entry for the old class and an added entry for the new class.
    pub slot_deltas: Vec<RomBudgetSlotDelta>,
    pub wram_delta: WramReservedDelta,
    pub sram_delta: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomBudgetSlotDelta {
    pub slot_id: BudgetSlotId,
    pub slot_class: BudgetSlotClass,
    pub training_usable_bytes: Option<u32>,
    pub current_usable_bytes: Option<u32>,
    pub training_reserved_slack: Option<u16>,
    pub current_reserved_slack: Option<u16>,
    pub training_available_bytes: Option<u32>,
    pub current_available_bytes: Option<u32>,
    pub usable_delta_bytes: i64,
    pub available_delta_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WramReservedDelta {
    pub training_overlay_bytes: u16,
    pub current_overlay_bytes: u16,
    pub overlay_delta_bytes: i64,
    pub training_hot_arena_floor_bytes: u16,
    pub current_hot_arena_floor_bytes: u16,
    pub hot_arena_floor_delta_bytes: i64,
    pub training_total_reserved_bytes: u16,
    pub current_total_reserved_bytes: u16,
    pub total_reserved_delta_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRuntimeChromePreflightSummary {
    pub fits_envelope: bool,
    pub hard_failures: Vec<String>,
    pub checks: Vec<ExportRuntimeChromePreflightCheckSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRuntimeChromePreflightCheckSummary {
    pub check_kind: String,
    pub status: String,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub slack_bytes: u64,
    pub over_by_bytes: u64,
    pub diagnostic: String,
}

#[must_use]
pub fn revalidate_runtime_chrome_budget_for_export(
    training_time_budget: &RuntimeChromeBudget,
    current_budget: &RuntimeChromeBudget,
    demand: RuntimeChromePreflightDemand,
) -> ExportRuntimeChromeRevalidationReport {
    let drift = budget_drift_report(training_time_budget, current_budget);
    let preflight_report = preflight_runtime_chrome_budget(current_budget, demand);
    let preflight = ExportRuntimeChromePreflightSummary::from_report(&preflight_report);
    let policy_revalidation = revalidate_runtime_chrome_budget(
        training_time_budget,
        current_budget,
        preflight_report.fits_envelope(),
    );
    let blocks_export = policy_revalidation.outcome == ReValidationOutcome::BlockExport;
    let diagnostic = export_revalidation_diagnostic(&drift, &preflight, &policy_revalidation);

    ExportRuntimeChromeRevalidationReport {
        drift,
        preflight,
        policy_revalidation,
        blocks_export,
        diagnostic,
    }
}

/// Phase E / HardenAndSelect final-export gate for RuntimeChromeBudget drift.
///
/// The report is always written before the decision is returned. A blocking
/// current-budget fit failure is represented as an error so final-export CLIs
/// can map it directly to a non-zero process exit.
pub fn run_final_export_runtime_chrome_revalidation(
    out_dir: impl AsRef<Path>,
    training_time_budget: &RuntimeChromeBudget,
    current_budget: &RuntimeChromeBudget,
    demand: RuntimeChromePreflightDemand,
) -> Result<ExportRuntimeChromeRevalidationArtifact, ExportRuntimeChromeRevalidationError> {
    let report =
        revalidate_runtime_chrome_budget_for_export(training_time_budget, current_budget, demand);
    let budget_drift_path = write_budget_drift_report_json(out_dir, &report)?;
    let artifact = ExportRuntimeChromeRevalidationArtifact {
        report,
        budget_drift_path,
    };
    if artifact.report.blocks_export {
        Err(ExportRuntimeChromeRevalidationError::Blocked {
            artifact: Box::new(artifact),
        })
    } else {
        Ok(artifact)
    }
}

pub fn write_budget_drift_report_json(
    out_dir: impl AsRef<Path>,
    report: &ExportRuntimeChromeRevalidationReport,
) -> Result<PathBuf, ExportRuntimeChromeRevalidationError> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir)?;
    let path = out_dir.join(BUDGET_DRIFT_FILE_NAME);
    fs::write(&path, serde_json::to_vec_pretty(report)?)?;
    Ok(path)
}

#[must_use]
pub fn budget_drift_report(
    training_time_budget: &RuntimeChromeBudget,
    current_budget: &RuntimeChromeBudget,
) -> BudgetDriftReport {
    BudgetDriftReport {
        runtime_nucleus_hashes_match: training_time_budget.runtime_nucleus_hash
            == current_budget.runtime_nucleus_hash,
        training_hash: training_time_budget.runtime_nucleus_hash,
        current_hash: current_budget.runtime_nucleus_hash,
        training_modules: training_time_budget.reference_shell_modules.clone(),
        current_modules: current_budget.reference_shell_modules.clone(),
        slot_deltas: rom_slot_deltas(training_time_budget, current_budget),
        wram_delta: WramReservedDelta::new(
            training_time_budget.wram_reserved,
            current_budget.wram_reserved,
        ),
        sram_delta: i64::from(current_budget.sram_reserved)
            - i64::from(training_time_budget.sram_reserved),
    }
}

impl WramReservedDelta {
    #[must_use]
    pub const fn new(training: WramReserved, current: WramReserved) -> Self {
        Self {
            training_overlay_bytes: training.overlay,
            current_overlay_bytes: current.overlay,
            overlay_delta_bytes: current.overlay as i64 - training.overlay as i64,
            training_hot_arena_floor_bytes: training.hot_arena_floor,
            current_hot_arena_floor_bytes: current.hot_arena_floor,
            hot_arena_floor_delta_bytes: current.hot_arena_floor as i64
                - training.hot_arena_floor as i64,
            training_total_reserved_bytes: training.total,
            current_total_reserved_bytes: current.total,
            total_reserved_delta_bytes: current.total as i64 - training.total as i64,
        }
    }
}

impl ExportRuntimeChromePreflightSummary {
    #[must_use]
    pub fn from_report(report: &RuntimeChromePreflightReport) -> Self {
        Self {
            fits_envelope: report.fits_envelope(),
            hard_failures: report.hard_failures().to_vec(),
            checks: report
                .checks()
                .iter()
                .map(ExportRuntimeChromePreflightCheckSummary::from_check)
                .collect(),
        }
    }
}

impl ExportRuntimeChromePreflightCheckSummary {
    #[must_use]
    pub fn from_check(check: &RuntimeChromePreflightCheck) -> Self {
        Self {
            check_kind: check_kind_name(check.kind()).to_owned(),
            status: preflight_status_name(check.status()).to_owned(),
            required_bytes: check.required_bytes().as_u64(),
            available_bytes: check.available_bytes().as_u64(),
            slack_bytes: check.slack_bytes().as_u64(),
            over_by_bytes: check.over_by_bytes().as_u64(),
            diagnostic: check.diagnostic().to_owned(),
        }
    }
}

fn rom_slot_deltas(
    training_time_budget: &RuntimeChromeBudget,
    current_budget: &RuntimeChromeBudget,
) -> Vec<RomBudgetSlotDelta> {
    let mut slots: BTreeMap<
        (BudgetSlotClass, BudgetSlotId),
        (Option<SlotSnapshot>, Option<SlotSnapshot>),
    > = BTreeMap::new();

    for slot in &training_time_budget.rom_slots {
        slots
            .entry((slot.class, slot.id))
            .or_insert((Some(SlotSnapshot::from_slot(slot)), None));
    }
    for slot in &current_budget.rom_slots {
        slots
            .entry((slot.class, slot.id))
            .and_modify(|(_, current)| *current = Some(SlotSnapshot::from_slot(slot)))
            .or_insert((None, Some(SlotSnapshot::from_slot(slot))));
    }

    slots
        .into_iter()
        .map(
            |((slot_class, slot_id), (training, current))| RomBudgetSlotDelta {
                slot_id,
                slot_class,
                training_usable_bytes: training.map(|slot| slot.usable_bytes),
                current_usable_bytes: current.map(|slot| slot.usable_bytes),
                training_reserved_slack: training.map(|slot| slot.reserved_slack),
                current_reserved_slack: current.map(|slot| slot.reserved_slack),
                training_available_bytes: training.map(|slot| slot.available_bytes),
                current_available_bytes: current.map(|slot| slot.available_bytes),
                usable_delta_bytes: optional_delta(
                    training.map(|slot| slot.usable_bytes),
                    current.map(|slot| slot.usable_bytes),
                ),
                available_delta_bytes: optional_delta(
                    training.map(|slot| slot.available_bytes),
                    current.map(|slot| slot.available_bytes),
                ),
            },
        )
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct SlotSnapshot {
    usable_bytes: u32,
    reserved_slack: u16,
    available_bytes: u32,
}

impl SlotSnapshot {
    fn from_slot(slot: &RomBudgetSlot) -> Self {
        Self {
            usable_bytes: slot.usable_bytes,
            reserved_slack: slot.reserved_slack,
            available_bytes: slot
                .usable_bytes
                .saturating_sub(u32::from(slot.reserved_slack)),
        }
    }
}

fn optional_delta(training: Option<u32>, current: Option<u32>) -> i64 {
    i64::from(current.unwrap_or(0)) - i64::from(training.unwrap_or(0))
}

fn export_revalidation_diagnostic(
    drift: &BudgetDriftReport,
    preflight: &ExportRuntimeChromePreflightSummary,
    policy_revalidation: &RuntimeChromeBudgetReValidation,
) -> String {
    let drift_prefix = if drift.runtime_nucleus_hashes_match {
        "Budget drift not detected"
    } else {
        "Budget drift detected"
    };
    let first_failure = preflight
        .hard_failures
        .first()
        .map_or("none", String::as_str);

    format!(
        "{drift_prefix}: training_runtime_nucleus_hash={}; current_runtime_nucleus_hash={}; \
         training_modules={:?}; current_modules={:?}; fits_envelope={}; outcome={:?}; \
         first_hard_failure={first_failure}; wram_overlay_delta_bytes={}; \
         wram_hot_arena_floor_delta_bytes={}; sram_delta_bytes={}",
        drift.training_hash,
        drift.current_hash,
        drift.training_modules,
        drift.current_modules,
        preflight.fits_envelope,
        policy_revalidation.outcome,
        drift.wram_delta.overlay_delta_bytes,
        drift.wram_delta.hot_arena_floor_delta_bytes,
        drift.sram_delta
    )
}

fn check_kind_name(kind: RuntimeChromePreflightCheckKind) -> &'static str {
    match kind {
        RuntimeChromePreflightCheckKind::RuntimeBudget => "runtime_budget",
        RuntimeChromePreflightCheckKind::CompileProfile => "compile_profile",
        RuntimeChromePreflightCheckKind::ExpertBank => "expert_bank",
        RuntimeChromePreflightCheckKind::CommonBank => "common_bank",
        RuntimeChromePreflightCheckKind::Bank0Free => "bank0_free",
        RuntimeChromePreflightCheckKind::WramHotArena => "wram_hot_arena",
        RuntimeChromePreflightCheckKind::WramOverlay => "wram_overlay",
        RuntimeChromePreflightCheckKind::SwitchBudget => "switch_budget",
        RuntimeChromePreflightCheckKind::Sram => "sram",
    }
}

fn preflight_status_name(status: RuntimeChromePreflightStatus) -> &'static str {
    match status {
        RuntimeChromePreflightStatus::Pass => "pass",
        RuntimeChromePreflightStatus::Warn => "warn",
        RuntimeChromePreflightStatus::Fail => "fail",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};
    use gbf_policy::{
        PlacementProfile, RomBudgetSlot, RuntimeMemoryCapSection, RuntimeNucleusHash,
        RuntimeShellModule, WramReserved,
    };

    use super::*;
    use crate::preflight::RuntimeChromePreflightDemand;
    use gbf_foundation::ByteCost;
    use gbf_policy::model_profile::ModelSizeProfile;

    #[test]
    fn revalidation_no_drift_passes_and_runs_current_preflight() {
        let training = budget(RuntimeNucleusHash::real(hash(1)), 16_384, 512, 4_096, 1_024);
        let current = training.clone();
        let report =
            revalidate_runtime_chrome_budget_for_export(&training, &current, pass_demand());

        assert!(!report.blocks_export);
        assert_eq!(
            report.policy_revalidation.outcome,
            ReValidationOutcome::Pass
        );
        assert!(report.drift.runtime_nucleus_hashes_match);
        assert!(report.preflight.fits_envelope);
        assert!(
            report
                .preflight
                .checks
                .iter()
                .any(|check| check.check_kind == "expert_bank")
        );
        let common = summary_check(&report.preflight, "common_bank");
        let bank0 = summary_check(&report.preflight, "bank0_free");
        assert_eq!(common.available_bytes, 16 * 1024);
        assert_eq!(bank0.available_bytes, (8 * 1024) - 512);
        assert!(report.diagnostic.contains("Budget drift not detected"));
    }

    #[test]
    fn revalidation_drift_passing_warns_and_exports_json_shape() {
        let training = budget(RuntimeNucleusHash::real(hash(1)), 16_384, 512, 4_096, 1_024);
        let mut current = budget(RuntimeNucleusHash::real(hash(2)), 16_256, 384, 4_000, 960);
        current
            .reference_shell_modules
            .insert(RuntimeShellModule::Panic);

        let report =
            revalidate_runtime_chrome_budget_for_export(&training, &current, pass_demand());

        assert!(!report.blocks_export);
        assert_eq!(
            report.policy_revalidation.outcome,
            ReValidationOutcome::Warn
        );
        assert!(!report.drift.runtime_nucleus_hashes_match);
        assert_eq!(report.drift.wram_delta.overlay_delta_bytes, -128);
        assert_eq!(report.drift.wram_delta.hot_arena_floor_delta_bytes, -96);
        assert_eq!(report.drift.sram_delta, -64);
        assert!(
            report
                .drift
                .current_modules
                .contains(&RuntimeShellModule::Panic)
        );
        let expert_delta = report
            .drift
            .slot_deltas
            .iter()
            .find(|delta| delta.slot_class == BudgetSlotClass::ExpertBank)
            .expect("expert delta is present");
        assert_eq!(expert_delta.usable_delta_bytes, -128);
        assert_eq!(expert_delta.available_delta_bytes, -128);

        let json = serde_json::to_value(&report.drift).expect("drift report serializes");
        assert!(json.get("training_hash").is_some());
        assert!(json.get("current_hash").is_some());
        assert!(json.get("slot_deltas").is_some());
        assert!(json.get("wram_delta").is_some());
        assert!(json.get("sram_delta").is_some());
    }

    #[test]
    fn revalidation_drift_blocking_blocks_export_on_current_preflight_failure() {
        let training = budget(RuntimeNucleusHash::real(hash(1)), 16_384, 512, 4_096, 1_024);
        let current = budget(RuntimeNucleusHash::real(hash(2)), 12_384, 256, 3_000, 512);

        let report =
            revalidate_runtime_chrome_budget_for_export(&training, &current, blocking_demand());

        assert!(report.blocks_export);
        assert_eq!(
            report.policy_revalidation.outcome,
            ReValidationOutcome::BlockExport
        );
        assert!(!report.preflight.fits_envelope);
        assert!(
            report
                .preflight
                .hard_failures
                .iter()
                .any(|failure| failure.contains("Expert FFN up+down"))
        );
        assert!(report.diagnostic.contains("fits_envelope=false"));
    }

    #[test]
    fn final_export_revalidation_writes_budget_drift_json_for_warning_drift() {
        let training = budget(RuntimeNucleusHash::real(hash(1)), 16_384, 512, 4_096, 1_024);
        let current = budget(RuntimeNucleusHash::real(hash(2)), 16_256, 384, 4_000, 960);
        let out_dir = unique_output_dir("warning");

        let artifact = run_final_export_runtime_chrome_revalidation(
            &out_dir,
            &training,
            &current,
            pass_demand(),
        )
        .expect("warning drift writes report and permits export");

        assert_eq!(
            artifact.budget_drift_path,
            out_dir.join(BUDGET_DRIFT_FILE_NAME)
        );
        assert!(artifact.budget_drift_path.exists());
        assert!(!artifact.report.blocks_export);
        assert_eq!(
            artifact.report.policy_revalidation.outcome,
            ReValidationOutcome::Warn
        );
        let decoded: ExportRuntimeChromeRevalidationReport = serde_json::from_slice(
            &std::fs::read(&artifact.budget_drift_path).expect("budget drift report readable"),
        )
        .expect("budget drift report decodes");
        assert_eq!(decoded, artifact.report);

        let _ = std::fs::remove_dir_all(out_dir);
    }

    #[test]
    fn final_export_revalidation_writes_budget_drift_json_before_blocking() {
        let training = budget(RuntimeNucleusHash::real(hash(1)), 16_384, 512, 4_096, 1_024);
        let current = budget(RuntimeNucleusHash::real(hash(2)), 12_384, 256, 3_000, 512);
        let out_dir = unique_output_dir("blocking");

        let error = run_final_export_runtime_chrome_revalidation(
            &out_dir,
            &training,
            &current,
            blocking_demand(),
        )
        .expect_err("current-budget hard failure blocks final export");

        let ExportRuntimeChromeRevalidationError::Blocked { artifact } = error else {
            panic!("expected blocked final export");
        };
        assert_eq!(
            artifact.budget_drift_path,
            out_dir.join(BUDGET_DRIFT_FILE_NAME)
        );
        assert!(artifact.budget_drift_path.exists());
        assert!(artifact.report.blocks_export);
        assert_eq!(
            artifact.report.policy_revalidation.outcome,
            ReValidationOutcome::BlockExport
        );

        let _ = std::fs::remove_dir_all(out_dir);
    }

    #[test]
    fn budget_drift_report_tracks_removed_and_added_slots() {
        let mut training = budget(RuntimeNucleusHash::real(hash(1)), 16_384, 512, 4_096, 1_024);
        let mut current = budget(RuntimeNucleusHash::real(hash(2)), 16_384, 512, 4_096, 1_024);
        training
            .rom_slots
            .retain(|slot| slot.id != BudgetSlotId::new(3));
        current
            .rom_slots
            .retain(|slot| slot.id != BudgetSlotId::new(2));

        let drift = budget_drift_report(&training, &current);

        let removed = drift
            .slot_deltas
            .iter()
            .find(|delta| delta.slot_id == BudgetSlotId::new(2))
            .expect("removed slot is present");
        assert_eq!(removed.current_usable_bytes, None);
        assert_eq!(removed.available_delta_bytes, -16_000);

        let added = drift
            .slot_deltas
            .iter()
            .find(|delta| delta.slot_id == BudgetSlotId::new(3))
            .expect("added slot is present");
        assert_eq!(added.training_usable_bytes, None);
        assert_eq!(added.available_delta_bytes, 16_000);
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

    fn budget(
        runtime_nucleus_hash: RuntimeNucleusHash,
        expert_slot_usable_bytes: u32,
        overlay_bytes: u16,
        hot_arena_floor_bytes: u16,
        sram_reserved: u32,
    ) -> RuntimeChromeBudget {
        let mut rom_slots = vec![
            RomBudgetSlot {
                id: BudgetSlotId::new(0),
                class: BudgetSlotClass::Bank0Free,
                usable_bytes: 8 * 1024,
                reserved_slack: 512,
                placement_caps: BTreeSet::from([PlacementProfile::StrictOnePerBank]),
            },
            RomBudgetSlot {
                id: BudgetSlotId::new(1),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 16 * 1024,
                reserved_slack: 512,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            },
        ];
        for slot_id in 2..6 {
            rom_slots.push(RomBudgetSlot {
                id: BudgetSlotId::new(slot_id),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: expert_slot_usable_bytes,
                reserved_slack: 384,
                placement_caps: BTreeSet::from([
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
            wram_reserved: wram_reserved_fixture(overlay_bytes, hot_arena_floor_bytes),
            sram_reserved,
        }
    }

    fn summary_check<'a>(
        summary: &'a ExportRuntimeChromePreflightSummary,
        check_kind: &str,
    ) -> &'a ExportRuntimeChromePreflightCheckSummary {
        summary
            .checks
            .iter()
            .find(|check| check.check_kind == check_kind)
            .expect("check kind is present")
    }

    fn wram_reserved_fixture(overlay_bytes: u16, hot_arena_floor_bytes: u16) -> WramReserved {
        let total = overlay_bytes
            .checked_add(1_024)
            .expect("fixture WRAM total fits u16");
        WramReserved::new(overlay_bytes, hot_arena_floor_bytes, total)
            .expect("valid WRAM reservation")
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn unique_output_dir(case: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("gbf-train-export-{case}-{unique}"))
    }
}
