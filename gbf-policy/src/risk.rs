//! Calibration-risk bands used by compile policy.

use serde::{Deserialize, Serialize};

/// Confidence class declared by a calibration bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CalibrationConfidenceClass {
    None,
    Transferred,
    WithinFamily,
    Onsite,
}

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
            Self::AtLeast { class } => observed as u8 >= class as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_policy_calibration_confidence_requirement_round_trip() {
        let requirement = CalibrationConfidenceRequirement::AtLeast {
            class: CalibrationConfidenceClass::WithinFamily,
        };

        let encoded = serde_json::to_string(&requirement).expect("requirement serializes");
        let decoded: CalibrationConfidenceRequirement =
            serde_json::from_str(&encoded).expect("requirement deserializes");

        assert_eq!(decoded, requirement);
        assert!(encoded.contains("AtLeast"));
        assert!(encoded.contains("WithinFamily"));
    }

    #[test]
    fn no_minimum_confidence_is_distinct_from_none_bundle_confidence() {
        let requirement = CalibrationConfidenceRequirement::NoMinimumConfidence;

        assert!(requirement.accepts(CalibrationConfidenceClass::None));
        assert_ne!(
            serde_json::to_value(requirement).expect("requirement serializes"),
            serde_json::to_value(CalibrationConfidenceClass::None).expect("class serializes")
        );
    }
}
