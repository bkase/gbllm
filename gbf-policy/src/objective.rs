//! Compile objectives and risk policy.

use gbf_foundation::CompileProfileId;
use serde::{Deserialize, Serialize};

use crate::compile::RuntimeMode;
use crate::risk::CalibrationConfidenceRequirement;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileObjective {
    pub service: Option<ServiceLevelObjective>,
    pub max_cycles_per_token: Option<u32>,
    pub max_bank_switches_per_token: Option<u16>,
    pub max_sram_page_switches_per_token: Option<u16>,
    pub min_ui_headroom_pct: u8,
    pub max_rom_bytes: Option<u32>,
    pub risk: RiskPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskPolicy {
    pub cycle_quantile: u8,
    pub switch_quantile: u8,
    pub calibration_confidence_requirement: CalibrationConfidenceRequirement,
    pub fallback_profile: Option<CompileProfileId>,
    pub fallback_runtime_mode: Option<RuntimeMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceLevelObjective {
    pub max_first_token_cycles_p95: Option<u32>,
    pub max_checkpoint_gap_cycles_p95: Option<u32>,
    pub max_resume_latency_cycles_p95: Option<u32>,
    pub max_ui_jitter_frames_p99: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};

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
                cycle_quantile: 95,
                switch_quantile: 99,
                calibration_confidence_requirement: CalibrationConfidenceRequirement::AtLeast {
                    class: CalibrationConfidenceClass::WithinFamily,
                },
                fallback_profile: Some(CompileProfileId::from("Recovery")),
                fallback_runtime_mode: Some(RuntimeMode::Safe),
            },
        }
    }

    #[test]
    fn objective_types_round_trip() {
        let objective = objective_fixture();
        let expected_risk = serde_json::json!({
            "cycle_quantile": 95,
            "switch_quantile": 99,
            "calibration_confidence_requirement": {
                "kind": "AtLeast",
                "class": {"kind": "WithinFamily"}
            },
            "fallback_profile": "Recovery",
            "fallback_runtime_mode": {"kind": "Safe"}
        });
        let expected_objective = serde_json::json!({
            "service": {
                "max_first_token_cycles_p95": 21000,
                "max_checkpoint_gap_cycles_p95": 13000,
                "max_resume_latency_cycles_p95": 8000,
                "max_ui_jitter_frames_p99": 2
            },
            "max_cycles_per_token": 24000,
            "max_bank_switches_per_token": 17,
            "max_sram_page_switches_per_token": 3,
            "min_ui_headroom_pct": 11,
            "max_rom_bytes": 2097152,
            "risk": expected_risk.clone()
        });

        let encoded = serde_json::to_string(&objective).expect("objective serializes");
        let decoded: CompileObjective =
            serde_json::from_str(&encoded).expect("objective deserializes");

        assert_eq!(decoded, objective);
        assert_eq!(
            serde_json::to_value(&objective).expect("objective serializes"),
            expected_objective
        );
        assert_eq!(
            serde_json::to_value(&objective.risk).expect("risk policy serializes"),
            expected_risk
        );
    }

    #[test]
    fn objective_rejects_unknown_field() {
        let mut value = serde_json::to_value(objective_fixture()).expect("objective serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<CompileObjective>(value).is_err());
    }
}
