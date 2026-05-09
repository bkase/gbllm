//! Compiler pipeline from artifact import through scheduling, assembly lowering, ROM emission, and reports.

pub mod arena;
pub mod budget;
pub mod f_b1;
pub mod import;
pub mod kernel_select;
pub mod legalize;
pub mod lower_asm;
pub mod lower_infer;
pub mod lower_quant;
pub mod observe;
pub mod place;
pub mod policy;
pub mod range;
pub mod reachability;
pub mod report;
pub mod rom;
pub mod schedule;
pub mod stage_cache;
pub mod storage;
pub mod validate;
pub mod window;

pub mod stages {
    pub mod budget {
        pub use crate::budget::*;
    }

    pub mod policy {
        pub use crate::policy::*;
    }

    pub mod validate {
        pub use crate::validate::*;
    }
}

#[cfg(test)]
mod tests {
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_ID, CalibrationConfidenceClass, CalibrationConfidenceRequirement,
        DEFAULT_COMPILE_PROFILE_ID, canonical_compile_profile_specs,
    };

    #[test]
    fn f_b2_compile_profile_spec_bringup_accepts_none_confidence() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");
        let bringup = specs
            .iter()
            .find(|spec| spec.id.as_str() == BRINGUP_COMPILE_PROFILE_ID)
            .expect("bringup profile exists");

        assert_eq!(
            bringup.risk_policy.calibration_confidence_requirement,
            CalibrationConfidenceRequirement::NoMinimumConfidence
        );
        assert!(
            bringup
                .risk_policy
                .calibration_confidence_requirement
                .accepts(CalibrationConfidenceClass::None)
        );
    }

    #[test]
    fn f_b2_compile_profile_spec_bringup_no_minimum_confidence_requirement() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");
        let bringup = specs
            .iter()
            .find(|spec| spec.id.as_str() == BRINGUP_COMPILE_PROFILE_ID)
            .expect("bringup profile exists");

        assert_eq!(
            bringup.risk_policy.calibration_confidence_requirement,
            CalibrationConfidenceRequirement::NoMinimumConfidence
        );
    }

    #[test]
    fn f_b2_compile_profile_spec_default_requires_transferred_confidence() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");
        let default = specs
            .iter()
            .find(|spec| spec.id.as_str() == DEFAULT_COMPILE_PROFILE_ID)
            .expect("default profile exists");

        assert_eq!(
            default.risk_policy.calibration_confidence_requirement,
            CalibrationConfidenceRequirement::AtLeast {
                class: CalibrationConfidenceClass::Transferred,
            }
        );
        assert!(
            !default
                .risk_policy
                .calibration_confidence_requirement
                .accepts(CalibrationConfidenceClass::None)
        );
        assert!(
            default
                .risk_policy
                .calibration_confidence_requirement
                .accepts(CalibrationConfidenceClass::Transferred)
        );
    }
}
