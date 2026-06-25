//! Compile profile registry.
//!
//! The registry intentionally contains only the BringUp profile today.
//! Production LinearState and BoundedKv profiles are deferred until the F12
//! sequence-state comparison owner provides reviewed data to choose defaults.

use std::error::Error;
use std::fmt;

use gbf_foundation::{CompileProfileId, PlatformCalibrationId};
use serde::Serialize;

use crate::compile::{BRINGUP_COMPILE_PROFILE_ID, SequenceSemanticsRef};
use crate::wram::{DMG_WRAM_SIZE_BYTES, OverlayReloadPolicy, WramLayoutPolicy};

pub const BRINGUP_SWITCH_CAP_RATIONALE: &str = "Bring-up only; tiny model, low quality bar. \
Refine after gbf-bench emits PlatformCalibrationBundle.bank_switch_cost and a target tokens/sec \
ceiling for calibration-derived bank-switch limits.";

pub const BRINGUP_MAX_BANK_SWITCHES_PER_TOKEN: u16 = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompileProfile {
    pub id: CompileProfileId,
    pub display_name: &'static str,
    pub wram_layout: WramLayoutPolicy,
    pub overlay_reload: OverlayReloadPolicy,
    pub max_bank_switches_per_token: u16,
    pub sequence_state: SequenceSemanticsRef,
    pub provenance: SwitchCapProvenance,
}

impl CompileProfile {
    pub fn validate(&self) -> Result<(), CompileProfileError> {
        if self.id.as_str().is_empty() {
            return Err(CompileProfileError::EmptyId);
        }
        if self.display_name.is_empty() {
            return Err(CompileProfileError::EmptyDisplayName {
                id: self.id.clone(),
            });
        }
        if self.wram_layout.required_wram_bytes() > DMG_WRAM_SIZE_BYTES {
            return Err(CompileProfileError::WramLayoutExceedsDmg {
                id: self.id.clone(),
                required_bytes: self.wram_layout.required_wram_bytes(),
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            });
        }
        if self.max_bank_switches_per_token == 0 {
            return Err(CompileProfileError::ZeroSwitchCap {
                id: self.id.clone(),
            });
        }
        self.provenance.validate(&self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SwitchCapProvenance {
    /// Hand-picked starting value. Replace after gbf-bench emits platform
    /// `bank_switch_cost` measurements and the profile has a target TPS.
    HandPicked { rationale: &'static str },
    /// Derived from `PlatformCalibrationBundle.bank_switch_cost` and a target
    /// tokens/sec. Carries the platform calibration id used.
    CalibrationDerived {
        calibration: PlatformCalibrationId,
        target_tps: f32,
    },
}

impl SwitchCapProvenance {
    pub fn validate(&self, profile_id: &CompileProfileId) -> Result<(), CompileProfileError> {
        match self {
            Self::HandPicked { rationale } if rationale.trim().is_empty() => {
                Err(CompileProfileError::EmptySwitchCapRationale {
                    id: profile_id.clone(),
                })
            }
            Self::HandPicked { .. } => Ok(()),
            Self::CalibrationDerived { target_tps, .. }
                if !target_tps.is_finite() || *target_tps <= 0.0 =>
            {
                Err(CompileProfileError::InvalidCalibrationTargetTps {
                    id: profile_id.clone(),
                    target_tps: *target_tps,
                })
            }
            Self::CalibrationDerived { .. } => Ok(()),
        }
    }
}

pub const BRINGUP_COMPILE_PROFILE: CompileProfile = CompileProfile {
    id: CompileProfileId::from_static(BRINGUP_COMPILE_PROFILE_ID),
    display_name: "BringUp",
    wram_layout: WramLayoutPolicy::bringup_defaults(),
    overlay_reload: OverlayReloadPolicy::PerExpertSwitch,
    max_bank_switches_per_token: BRINGUP_MAX_BANK_SWITCHES_PER_TOKEN,
    sequence_state: SequenceSemanticsRef::Unspecified,
    provenance: SwitchCapProvenance::HandPicked {
        rationale: BRINGUP_SWITCH_CAP_RATIONALE,
    },
};

pub static COMPILE_PROFILE_REGISTRY: [CompileProfile; 1] = [BRINGUP_COMPILE_PROFILE];

#[must_use]
pub fn registry() -> &'static [CompileProfile] {
    &COMPILE_PROFILE_REGISTRY
}

#[must_use]
pub fn bringup_profile() -> &'static CompileProfile {
    &COMPILE_PROFILE_REGISTRY[0]
}

#[must_use]
pub fn profile_by_id(id: &CompileProfileId) -> Option<&'static CompileProfile> {
    registry().iter().find(|profile| &profile.id == id)
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompileProfileError {
    EmptyId,
    EmptyDisplayName {
        id: CompileProfileId,
    },
    WramLayoutExceedsDmg {
        id: CompileProfileId,
        required_bytes: u32,
        wram_bytes: u32,
    },
    ZeroSwitchCap {
        id: CompileProfileId,
    },
    EmptySwitchCapRationale {
        id: CompileProfileId,
    },
    InvalidCalibrationTargetTps {
        id: CompileProfileId,
        target_tps: f32,
    },
}

impl fmt::Display for CompileProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => f.write_str("compile profile id must not be empty"),
            Self::EmptyDisplayName { id } => {
                write!(f, "compile profile {id} display_name must not be empty")
            }
            Self::WramLayoutExceedsDmg {
                id,
                required_bytes,
                wram_bytes,
            } => write!(
                f,
                "compile profile {id} WRAM layout requires {required_bytes} bytes, exceeding {wram_bytes} byte WRAM"
            ),
            Self::ZeroSwitchCap { id } => write!(
                f,
                "compile profile {id} max_bank_switches_per_token must be greater than zero"
            ),
            Self::EmptySwitchCapRationale { id } => write!(
                f,
                "compile profile {id} hand-picked switch-cap rationale must not be empty"
            ),
            Self::InvalidCalibrationTargetTps { id, target_tps } => write!(
                f,
                "compile profile {id} calibration-derived switch cap target_tps {target_tps} must be finite and positive"
            ),
        }
    }
}

impl Error for CompileProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_exactly_bringup() {
        let profiles = registry();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id.as_str(), BRINGUP_COMPILE_PROFILE_ID);
        assert_eq!(bringup_profile(), &profiles[0]);
        assert_eq!(
            profile_by_id(&CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID)),
            Some(&profiles[0])
        );
        assert_eq!(profile_by_id(&CompileProfileId::from("Default")), None);
    }

    #[test]
    fn bringup_invariants_are_pinned() {
        let bringup = bringup_profile();

        assert_eq!(bringup.display_name, "BringUp");
        assert_eq!(bringup.wram_layout.overlay_bytes, 512);
        assert_eq!(bringup.wram_layout.continuation_bytes, 256);
        assert_eq!(bringup.wram_layout.stack_bytes, 256);
        assert_eq!(bringup.wram_layout.hot_arena_bytes_min, 4096);
        assert!(bringup.wram_layout.required_wram_bytes() <= DMG_WRAM_SIZE_BYTES);
        assert_eq!(bringup.overlay_reload, OverlayReloadPolicy::PerExpertSwitch);
        assert_eq!(bringup.max_bank_switches_per_token, 8);
        assert!(bringup.max_bank_switches_per_token > 0);
        assert_eq!(bringup.sequence_state, SequenceSemanticsRef::Unspecified);
        assert!(bringup.validate().is_ok());
    }

    #[test]
    fn bringup_switch_cap_provenance_names_calibration_path() {
        let SwitchCapProvenance::HandPicked { rationale } = &bringup_profile().provenance else {
            panic!(
                "BringUp switch cap must be hand-picked until calibration-derived ceilings land"
            );
        };

        assert!(!rationale.trim().is_empty());
        assert!(rationale.contains("gbf-bench"));
        assert!(rationale.contains("PlatformCalibrationBundle.bank_switch_cost"));
        assert!(rationale.contains("target tokens/sec"));
    }

    #[test]
    fn profile_json_shape_is_stable() {
        let json = serde_json::to_value(bringup_profile()).expect("profile serializes");

        assert_eq!(json["id"], "Bringup");
        assert_eq!(json["display_name"], "BringUp");
        assert_eq!(
            json["wram_layout"],
            serde_json::json!({
                "overlay_bytes": 512,
                "continuation_bytes": 256,
                "stack_bytes": 256,
                "hot_arena_bytes_min": 4096
            })
        );
        assert_eq!(
            json["overlay_reload"],
            serde_json::json!({ "kind": "PerExpertSwitch" })
        );
        assert_eq!(json["max_bank_switches_per_token"], 8);
        assert_eq!(
            json["sequence_state"],
            serde_json::json!({ "kind": "Unspecified" })
        );
        assert_eq!(json["provenance"]["kind"], "HandPicked");
        assert!(json["provenance"]["rationale"].is_string());
    }

    #[test]
    fn validation_rejects_bad_switch_cap_provenance() {
        let mut profile = BRINGUP_COMPILE_PROFILE.clone();
        profile.max_bank_switches_per_token = 0;
        assert_eq!(
            profile.validate(),
            Err(CompileProfileError::ZeroSwitchCap {
                id: CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID)
            })
        );

        profile = BRINGUP_COMPILE_PROFILE.clone();
        profile.provenance = SwitchCapProvenance::HandPicked { rationale: " " };
        assert_eq!(
            profile.validate(),
            Err(CompileProfileError::EmptySwitchCapRationale {
                id: CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID)
            })
        );

        profile.provenance = SwitchCapProvenance::CalibrationDerived {
            calibration: PlatformCalibrationId::from("gbf-bench-platform-dmg"),
            target_tps: f32::INFINITY,
        };
        assert_eq!(
            profile.validate(),
            Err(CompileProfileError::InvalidCalibrationTargetTps {
                id: CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID),
                target_tps: f32::INFINITY
            })
        );
    }
}
