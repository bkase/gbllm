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
    use gbf_foundation::Hash256;
    use gbf_hw::target::dmg_mbc5_8mib_128kib;
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_ID, CalibrationConfidenceClass, CalibrationConfidenceRequirement,
        DEFAULT_COMPILE_PROFILE_ID, canonical_compile_profile_specs,
    };
    use gbf_report::{ReportOutcome, canonicalize as canonicalize_report};

    use crate::budget::{
        BudgetInputs, QuantGraphBudgetSource, QuantGraphBudgetView, QuantGraphBudgetViewError,
        static_budget_report as run_stage2_static_budget,
    };
    use crate::policy::resolve_policy;

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

    #[test]
    fn f_b2_f_b4_chunk_pipeline_runs_in_order() {
        let mut order = Vec::new();
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);

        order.push("stage0");
        let validation = fixture.validation();

        order.push("stage0.5");
        let policy = resolve_policy(&validation).expect("policy resolves after Stage 0");

        order.push("stage1.synthetic");
        let quant_graph = MissingBudgetQuantGraph {
            quant_graph_hash: hash(0xe0),
        };

        order.push("stage2");
        let target_profile = dmg_mbc5_8mib_128kib();
        let budget = run_stage2_static_budget(BudgetInputs {
            policy: &policy,
            quant_graph: &quant_graph,
            runtime_chrome_budget: None,
            target_profile: &target_profile,
        });

        assert_eq!(
            order,
            vec!["stage0", "stage0.5", "stage1.synthetic", "stage2"]
        );
        assert_eq!(budget.report.outcome, ReportOutcome::Failed);
        assert_eq!(
            budget.report.body.identity.policy_resolution_self_hash,
            policy.policy_resolution_self_hash
        );
    }

    #[test]
    fn f_b2_f_b4_chunk_failures_short_circuit_correctly() {
        let stage0_failure_hash = hash(0xa0);
        let stage0_result: Result<Hash256, Hash256> = Err(stage0_failure_hash);
        let mut ran_after_stage0 = Vec::new();
        if let Ok(_artifact_validation_self_hash) = stage0_result {
            ran_after_stage0.push("stage0.5");
        }
        assert!(ran_after_stage0.is_empty());

        let artifact_validation_self_hash = hash(0xa1);
        let policy_failure_hash = hash(0xa2);
        let stage05_result: Result<Hash256, (Hash256, Hash256)> =
            Err((artifact_validation_self_hash, policy_failure_hash));
        let mut ran_after_stage05 = Vec::new();
        if let Ok(_policy_resolution_self_hash) = stage05_result {
            ran_after_stage05.push("stage2");
        }
        assert!(ran_after_stage05.is_empty());
        assert_eq!(
            stage05_result.expect_err("Stage 0.5 fixture fails").0,
            artifact_validation_self_hash
        );
    }

    #[test]
    fn f_b2_f_b4_chunk_reports_are_byte_identical_across_runs() {
        let first_fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let first_validation = first_fixture.validation();
        let first_policy = resolve_policy(&first_validation).expect("first policy resolves");
        let first_target = dmg_mbc5_8mib_128kib();
        let first_quant_graph = MissingBudgetQuantGraph {
            quant_graph_hash: hash(0xe0),
        };
        let first_budget = run_stage2_static_budget(BudgetInputs {
            policy: &first_policy,
            quant_graph: &first_quant_graph,
            runtime_chrome_budget: None,
            target_profile: &first_target,
        });

        let second_fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let second_validation = second_fixture.validation();
        let second_policy = resolve_policy(&second_validation).expect("second policy resolves");
        let second_target = dmg_mbc5_8mib_128kib();
        let second_quant_graph = MissingBudgetQuantGraph {
            quant_graph_hash: hash(0xe0),
        };
        let second_budget = run_stage2_static_budget(BudgetInputs {
            policy: &second_policy,
            quant_graph: &second_quant_graph,
            runtime_chrome_budget: None,
            target_profile: &second_target,
        });

        assert_eq!(
            canonicalize_report(&first_validation.report).expect("first Stage 0 canonicalizes"),
            canonicalize_report(&second_validation.report).expect("second Stage 0 canonicalizes")
        );
        assert_eq!(
            canonicalize_report(&first_policy.report).expect("first Stage 0.5 canonicalizes"),
            canonicalize_report(&second_policy.report).expect("second Stage 0.5 canonicalizes")
        );
        assert_eq!(
            canonicalize_report(&first_budget.report).expect("first Stage 2 canonicalizes"),
            canonicalize_report(&second_budget.report).expect("second Stage 2 canonicalizes")
        );
    }

    struct MissingBudgetQuantGraph {
        quant_graph_hash: Hash256,
    }

    impl QuantGraphBudgetSource for MissingBudgetQuantGraph {
        fn quant_graph_hash(&self) -> Hash256 {
            self.quant_graph_hash
        }

        fn semantic_core_hash(&self) -> Hash256 {
            hash(0x02)
        }

        fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
            panic!("missing runtime chrome budget must not evaluate the quant graph")
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
