//! Calibration-risk bands used by compile policy.

use serde::{Deserialize, Serialize};

pub use gbf_hw::calibration::CalibrationConfidenceClass;

/// Minimum confidence required by a compile profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CalibrationConfidenceRequirement {
    NoMinimumConfidence,
    AtLeast { class: CalibrationConfidenceClass },
}

impl CalibrationConfidenceRequirement {
    #[must_use]
    pub const fn accepts(self, observed: CalibrationConfidenceClass) -> bool {
        match self {
            Self::NoMinimumConfidence => true,
            Self::AtLeast { class } => observed.rank() >= class.rank(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::RuntimeMode;
    use crate::objective::RiskPolicy;
    use gbf_foundation::CompileProfileId;

    fn risk_policy_fixture() -> RiskPolicy {
        RiskPolicy {
            cycle_quantile: 95,
            switch_quantile: 99,
            calibration_confidence_requirement: CalibrationConfidenceRequirement::AtLeast {
                class: CalibrationConfidenceClass::Reasonable,
            },
            fallback_profile: Some(CompileProfileId::from("Recovery")),
            fallback_runtime_mode: Some(RuntimeMode::Safe),
        }
    }

    #[test]
    fn risk_policy_calibration_confidence_requirement_round_trip() {
        let requirement = CalibrationConfidenceRequirement::AtLeast {
            class: CalibrationConfidenceClass::Reasonable,
        };

        let encoded = serde_json::to_string(&requirement).expect("requirement serializes");
        let decoded: CalibrationConfidenceRequirement =
            serde_json::from_str(&encoded).expect("requirement deserializes");

        assert_eq!(decoded, requirement);
        assert_eq!(
            serde_json::to_value(requirement).expect("requirement serializes"),
            serde_json::json!({
                "kind": "AtLeast",
                "class": {"kind": "Reasonable"}
            })
        );
        assert!(!requirement.accepts(CalibrationConfidenceClass::None));
        assert!(!requirement.accepts(CalibrationConfidenceClass::Weak));
        assert!(requirement.accepts(CalibrationConfidenceClass::Reasonable));
        assert!(requirement.accepts(CalibrationConfidenceClass::Strong));
    }

    #[test]
    fn calibration_confidence_requirement_rejects_unknown_field() {
        let mut value = serde_json::to_value(CalibrationConfidenceRequirement::AtLeast {
            class: CalibrationConfidenceClass::Reasonable,
        })
        .expect("requirement serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<CalibrationConfidenceRequirement>(value).is_err());
    }

    #[test]
    fn no_minimum_confidence_is_distinct_from_none_bundle_confidence() {
        let requirement = CalibrationConfidenceRequirement::NoMinimumConfidence;
        let expected_shape = serde_json::json!({
            "kind": "NoMinimumConfidence"
        });

        assert_eq!(
            serde_json::to_value(requirement).expect("requirement serializes"),
            expected_shape
        );
        let decoded: CalibrationConfidenceRequirement =
            serde_json::from_value(expected_shape).expect("requirement deserializes");
        assert_eq!(decoded, requirement);
        for observed in [
            CalibrationConfidenceClass::None,
            CalibrationConfidenceClass::Weak,
            CalibrationConfidenceClass::Reasonable,
            CalibrationConfidenceClass::Strong,
        ] {
            assert!(requirement.accepts(observed));
        }
        assert_ne!(
            serde_json::to_value(requirement).expect("requirement serializes"),
            serde_json::to_value(CalibrationConfidenceClass::None).expect("class serializes")
        );
    }

    #[test]
    fn risk_policy_rejects_unknown_field() {
        let mut value =
            serde_json::to_value(risk_policy_fixture()).expect("risk policy serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<RiskPolicy>(value).is_err());
    }
}
