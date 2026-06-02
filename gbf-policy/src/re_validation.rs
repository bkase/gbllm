//! Runtime chrome budget re-validation helpers for F-S5 D18.

use std::collections::BTreeMap;

use gbf_foundation::BudgetSlotId;
use serde::{Deserialize, Serialize};

use crate::budget::{BudgetSlotClass, RuntimeChromeBudget, RuntimeNucleusHash};

pub const D9_RUNTIME_CHROME_BUDGET_DELTA_TOLERANCE_BYTES: i64 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ReValidationOutcome {
    Pass,
    Warn,
    BlockExport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeChromeBudgetDelta {
    pub slot_id: BudgetSlotId,
    pub slot_class: BudgetSlotClass,
    pub delta_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeChromeBudgetReValidation {
    pub runtime_nucleus_hashes_match: bool,
    pub per_slot_byte_deltas: Vec<RuntimeChromeBudgetDelta>,
    pub fits_envelope: bool,
    pub outcome: ReValidationOutcome,
    pub diagnostic: String,
}

#[must_use]
pub fn revalidate_runtime_chrome_budget(
    training_time_budget: &RuntimeChromeBudget,
    current_budget: &RuntimeChromeBudget,
    fits_envelope: bool,
) -> RuntimeChromeBudgetReValidation {
    let runtime_nucleus_hashes_match =
        training_time_budget.runtime_nucleus_hash == current_budget.runtime_nucleus_hash;
    let synthetic_vs_real_mismatch = training_time_budget
        .runtime_nucleus_hash
        .is_synthetic_reference()
        != current_budget.runtime_nucleus_hash.is_synthetic_reference();
    let per_slot_byte_deltas = per_slot_byte_deltas(training_time_budget, current_budget);
    let offending_slot = per_slot_byte_deltas
        .iter()
        .find(|delta| delta.delta_bytes.abs() > D9_RUNTIME_CHROME_BUDGET_DELTA_TOLERANCE_BYTES);
    let delta_exceeds_d9_tolerance = offending_slot.is_some();

    let hash_drift_exceeds_d9_tolerance =
        !runtime_nucleus_hashes_match && delta_exceeds_d9_tolerance;

    let outcome = if !fits_envelope || synthetic_vs_real_mismatch || hash_drift_exceeds_d9_tolerance
    {
        ReValidationOutcome::BlockExport
    } else if runtime_nucleus_hashes_match {
        ReValidationOutcome::Pass
    } else {
        ReValidationOutcome::Warn
    };

    RuntimeChromeBudgetReValidation {
        runtime_nucleus_hashes_match,
        diagnostic: diagnostic(
            outcome,
            training_time_budget.runtime_nucleus_hash,
            current_budget.runtime_nucleus_hash,
            &per_slot_byte_deltas,
            fits_envelope,
            synthetic_vs_real_mismatch,
            offending_slot,
        ),
        per_slot_byte_deltas,
        fits_envelope,
        outcome,
    }
}

fn per_slot_byte_deltas(
    training_time_budget: &RuntimeChromeBudget,
    current_budget: &RuntimeChromeBudget,
) -> Vec<RuntimeChromeBudgetDelta> {
    let mut slots = BTreeMap::new();

    for slot in &training_time_budget.rom_slots {
        slots
            .entry((slot.class, slot.id))
            .or_insert((Some(slot.usable_bytes), None));
    }
    for slot in &current_budget.rom_slots {
        slots
            .entry((slot.class, slot.id))
            .and_modify(|(_, current)| *current = Some(slot.usable_bytes))
            .or_insert((None, Some(slot.usable_bytes)));
    }

    slots
        .into_iter()
        .map(
            |((slot_class, slot_id), (training_usable_bytes, current_usable_bytes))| {
                RuntimeChromeBudgetDelta {
                    slot_id,
                    slot_class,
                    delta_bytes: i64::from(current_usable_bytes.unwrap_or(0))
                        - i64::from(training_usable_bytes.unwrap_or(0)),
                }
            },
        )
        .collect()
}

fn diagnostic(
    outcome: ReValidationOutcome,
    training_hash: RuntimeNucleusHash,
    current_hash: RuntimeNucleusHash,
    deltas: &[RuntimeChromeBudgetDelta],
    fits_envelope: bool,
    synthetic_vs_real_mismatch: bool,
    offending_slot: Option<&RuntimeChromeBudgetDelta>,
) -> String {
    let delta_summary = deltas
        .iter()
        .map(|delta| {
            format!(
                "slot_id={} slot_class={:?} delta_bytes={}",
                delta.slot_id, delta.slot_class, delta.delta_bytes
            )
        })
        .collect::<Vec<_>>()
        .join("; ");

    let mut message = format!(
        "outcome={outcome:?}; training_runtime_nucleus_hash={training_hash}; current_runtime_nucleus_hash={current_hash}; fits_envelope={fits_envelope}; per_slot_byte_deltas=[{delta_summary}]"
    );

    if synthetic_vs_real_mismatch {
        message.push_str("; synthetic_vs_real_mismatch=true");
    }
    if let Some(slot) = offending_slot {
        message.push_str(&format!(
            "; offending_slot_id={} offending_slot_class={:?} offending_delta_bytes={}",
            slot.slot_id, slot.slot_class, slot.delta_bytes
        ));
    }

    message
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeSet;

    use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};

    use crate::budget::{RomBudgetSlot, RuntimeMemoryCapSection};
    use crate::compile::PlacementProfile;

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
        RuntimeNucleusHash::real(hash(byte))
    }

    fn budget(runtime_nucleus_hash: RuntimeNucleusHash, usable_bytes: u32) -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash,
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(7),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes,
                reserved_slack: 384,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x09),
            },
            wram_reserved: 128,
            sram_reserved: 512,
        }
    }

    #[test]
    fn matching_hashes_and_fit_envelope_pass() {
        let training = budget(runtime_hash(1), 1024);
        let current = budget(runtime_hash(1), 1024);

        let report = revalidate_runtime_chrome_budget(&training, &current, true);

        assert!(report.runtime_nucleus_hashes_match);
        assert_eq!(report.outcome, ReValidationOutcome::Pass);
        assert_eq!(report.per_slot_byte_deltas[0].delta_bytes, 0);
    }

    #[test]
    fn hash_drift_with_delta_at_d9_tolerance_warns() {
        let training = budget(runtime_hash(1), 1024);
        let current = budget(runtime_hash(2), 1024 + 256);

        let report = revalidate_runtime_chrome_budget(&training, &current, true);

        assert!(!report.runtime_nucleus_hashes_match);
        assert_eq!(report.outcome, ReValidationOutcome::Warn);
        assert_eq!(report.per_slot_byte_deltas[0].delta_bytes, 256);
    }

    #[test]
    fn hash_drift_with_delta_above_d9_tolerance_blocks_export() {
        let training = budget(runtime_hash(1), 1024);
        let current = budget(runtime_hash(2), 1024 + 257);

        let report = revalidate_runtime_chrome_budget(&training, &current, true);

        assert!(!report.runtime_nucleus_hashes_match);
        assert_eq!(report.outcome, ReValidationOutcome::BlockExport);
        assert_eq!(report.per_slot_byte_deltas[0].delta_bytes, 257);
        assert!(report.diagnostic.contains("offending_slot_id=7"));
        assert!(report.diagnostic.contains("offending_delta_bytes=257"));
    }

    #[test]
    fn matching_hashes_with_delta_above_d9_tolerance_passes() {
        let training = budget(runtime_hash(1), 1024);
        let current = budget(runtime_hash(1), 1024 + 257);

        let report = revalidate_runtime_chrome_budget(&training, &current, true);

        assert!(report.runtime_nucleus_hashes_match);
        assert_eq!(report.outcome, ReValidationOutcome::Pass);
        assert!(report.diagnostic.contains("offending_slot_id=7"));
    }

    #[test]
    fn failed_fit_envelope_blocks_export() {
        let training = budget(runtime_hash(1), 1024);
        let current = budget(runtime_hash(2), 1024 + 128);

        let report = revalidate_runtime_chrome_budget(&training, &current, false);

        assert_eq!(report.outcome, ReValidationOutcome::BlockExport);
        assert!(!report.fits_envelope);
        assert!(report.diagnostic.contains("fits_envelope=false"));
    }

    #[test]
    fn synthetic_vs_real_mismatch_blocks_export() {
        let training = budget(RuntimeNucleusHash::synthetic_reference(hash(1)), 1024);
        let current = budget(runtime_hash(1), 1024);

        let report = revalidate_runtime_chrome_budget(&training, &current, true);

        assert_eq!(report.outcome, ReValidationOutcome::BlockExport);
        assert!(
            report
                .diagnostic
                .contains("synthetic_vs_real_mismatch=true")
        );
    }
}
