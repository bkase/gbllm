//! Pinned reference shell composition and named future reservations.

use std::collections::{BTreeMap, BTreeSet};

use gbf_abi::RuntimeShellModule;
use serde::{Deserialize, Serialize};

/// Runtime shell module set and named slack reserved by the budget emitter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceShellSpec {
    pub included: BTreeSet<RuntimeShellModule>,
    pub future_reservations: BTreeMap<RuntimeShellModule, FutureReservation>,
}

/// Named slack for a runtime module not present in the pinned reference shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FutureReservation {
    pub rom_bytes_per_bank0: u16,
    pub wram_bytes: u16,
    pub sram_bytes: u32,
    pub justification: String,
}

impl FutureReservation {
    #[must_use]
    pub fn new(
        rom_bytes_per_bank0: u16,
        wram_bytes: u16,
        sram_bytes: u32,
        justification: impl Into<String>,
    ) -> Self {
        Self {
            rom_bytes_per_bank0,
            wram_bytes,
            sram_bytes,
            justification: justification.into(),
        }
    }
}

/// Reference-shell composition used before real runtime-budget emission exists.
///
/// The included set is the minimal+UI shell. `Panic` is intentionally recorded
/// as reserved slack rather than an included reference module: F-A5 ships a
/// minimal panic implementation as an M0 bring-up exception, while the full
/// fault/persistence budget remains a future reservation.
#[must_use]
pub fn pinned_reference_shell() -> ReferenceShellSpec {
    use RuntimeShellModule::{
        Banking, Boot, FutureHarness, FuturePersistence, FutureTrace, Interrupts, Joypad, Keyboard,
        Panic, Scheduler, Text, VideoCommit,
    };

    ReferenceShellSpec {
        included: BTreeSet::from([
            Boot,
            Interrupts,
            Scheduler,
            Banking,
            Joypad,
            Text,
            Keyboard,
            VideoCommit,
        ]),
        future_reservations: BTreeMap::from([
            (
                FuturePersistence,
                FutureReservation::new(
                    256,
                    64,
                    256,
                    "Reserve for future SRAM persistence protocol metadata and WRAM header cache.",
                ),
            ),
            (
                FutureTrace,
                FutureReservation::new(
                    512,
                    64,
                    1024,
                    "Reserve for future trace capture code and SRAM trace pages.",
                ),
            ),
            (
                FutureHarness,
                FutureReservation::new(
                    256,
                    32,
                    64,
                    "Reserve for future host harness command and result blocks.",
                ),
            ),
            (
                Panic,
                FutureReservation::new(
                    128,
                    0,
                    0,
                    "Reserve for the panic path named by T2.4; F-A5 ships only a minimal M0 panic.",
                ),
            ),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_reference_shell_includes_minimal_ui_modules() {
        let spec = pinned_reference_shell();

        assert_eq!(
            spec.included,
            BTreeSet::from([
                RuntimeShellModule::Boot,
                RuntimeShellModule::Interrupts,
                RuntimeShellModule::Scheduler,
                RuntimeShellModule::Banking,
                RuntimeShellModule::Joypad,
                RuntimeShellModule::Text,
                RuntimeShellModule::Keyboard,
                RuntimeShellModule::VideoCommit,
            ])
        );
    }

    #[test]
    fn pinned_reference_shell_reserves_only_non_included_modules() {
        let spec = pinned_reference_shell();
        let reserved: BTreeSet<_> = spec.future_reservations.keys().copied().collect();

        assert_eq!(
            reserved,
            BTreeSet::from([
                RuntimeShellModule::FuturePersistence,
                RuntimeShellModule::FutureTrace,
                RuntimeShellModule::FutureHarness,
                RuntimeShellModule::Panic,
            ])
        );
        assert!(reserved.is_disjoint(&spec.included));
        assert!(
            spec.future_reservations
                .values()
                .all(|reservation| !reservation.justification.trim().is_empty())
        );
    }

    #[test]
    fn pinned_reference_shell_reservation_bytes_match_s5_contract() {
        let spec = pinned_reference_shell();

        assert_eq!(
            spec.future_reservations[&RuntimeShellModule::FuturePersistence],
            FutureReservation::new(
                256,
                64,
                256,
                "Reserve for future SRAM persistence protocol metadata and WRAM header cache."
            )
        );
        assert_eq!(
            spec.future_reservations[&RuntimeShellModule::FutureTrace],
            FutureReservation::new(
                512,
                64,
                1024,
                "Reserve for future trace capture code and SRAM trace pages."
            )
        );
        assert_eq!(
            spec.future_reservations[&RuntimeShellModule::FutureHarness],
            FutureReservation::new(
                256,
                32,
                64,
                "Reserve for future host harness command and result blocks."
            )
        );
        assert_eq!(
            spec.future_reservations[&RuntimeShellModule::Panic],
            FutureReservation::new(
                128,
                0,
                0,
                "Reserve for the panic path named by T2.4; F-A5 ships only a minimal M0 panic."
            )
        );
    }

    #[test]
    fn reference_shell_json_shape_is_pinned() {
        let value = serde_json::to_value(pinned_reference_shell()).expect("spec serializes");

        assert_eq!(
            value,
            serde_json::json!({
                "included": [
                    "boot",
                    "interrupts",
                    "scheduler",
                    "banking",
                    "joypad",
                    "text",
                    "keyboard",
                    "video_commit"
                ],
                "future_reservations": {
                    "panic": {
                        "rom_bytes_per_bank0": 128,
                        "wram_bytes": 0,
                        "sram_bytes": 0,
                        "justification": "Reserve for the panic path named by T2.4; F-A5 ships only a minimal M0 panic."
                    },
                    "future_persistence": {
                        "rom_bytes_per_bank0": 256,
                        "wram_bytes": 64,
                        "sram_bytes": 256,
                        "justification": "Reserve for future SRAM persistence protocol metadata and WRAM header cache."
                    },
                    "future_trace": {
                        "rom_bytes_per_bank0": 512,
                        "wram_bytes": 64,
                        "sram_bytes": 1024,
                        "justification": "Reserve for future trace capture code and SRAM trace pages."
                    },
                    "future_harness": {
                        "rom_bytes_per_bank0": 256,
                        "wram_bytes": 32,
                        "sram_bytes": 64,
                        "justification": "Reserve for future host harness command and result blocks."
                    }
                }
            })
        );
    }

    #[test]
    fn reference_shell_rejects_unknown_fields() {
        let mut value = serde_json::to_value(pinned_reference_shell()).expect("spec serializes");
        value["unexpected"] = serde_json::json!("nope");

        assert!(serde_json::from_value::<ReferenceShellSpec>(value).is_err());
    }
}
