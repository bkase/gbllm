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
pub mod s1;
pub mod s3;
pub mod s4;
pub mod s5;
pub mod schedule;
pub mod stage_cache;
pub mod storage;
pub mod storage_plan;
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
    use std::collections::BTreeSet;

    use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
    use gbf_hw::target::dmg_mbc5_8mib_128kib;
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_ID, BudgetSlotClass, CalibrationConfidenceClass,
        CalibrationConfidenceRequirement, DEFAULT_COMPILE_PROFILE_ID, PlacementProfile,
        RomBudgetSlot, RuntimeChromeBudget, RuntimeMemoryCapSection,
        canonical_compile_profile_specs,
    };
    use gbf_report::{ReportOutcome, canonicalize as canonicalize_report};

    use crate::budget::{
        BudgetInputs, QuantGraphBudgetSource, QuantGraphBudgetView, QuantGraphBudgetViewError,
        RoutingProjection, SequenceStateProjection,
        static_budget_report as run_stage2_static_budget,
    };
    use crate::policy::{PolicyResolutionStageFailure, ResolvedPolicyProduct, resolve_policy};
    use crate::validate::{ValidationProduct, ValidationStageFailure};

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
        let fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let mut harness = ChunkDispatchHarness::default();
        let budget = harness
            .run_success_path(&fixture)
            .expect("Stage 0 -> Stage 0.5 -> synthetic Stage 1 -> Stage 2 succeeds");

        assert_eq!(
            harness.order,
            vec!["stage0", "stage0.5", "stage1.synthetic", "stage2"]
        );
        assert_eq!(budget.report.outcome, ReportOutcome::Passed);
        assert!(budget.report.body.decision.fits);
    }

    #[test]
    fn f_b2_f_b4_chunk_failures_short_circuit_correctly() {
        let mut stage0_fixture = crate::policy::tests::Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        stage0_fixture.require_unsupported_stage0_compiler_feature();
        let mut stage0_harness = ChunkDispatchHarness::default();
        let stage0_failure = stage0_harness
            .run_until_failure(&stage0_fixture)
            .expect_err("unsupported compiler feature fails in real Stage 0")
            .expect_stage0("unsupported compiler feature fails in real Stage 0");
        assert_eq!(stage0_harness.order, vec!["stage0"]);
        assert_eq!(stage0_failure.report.outcome, ReportOutcome::Failed);

        let mut stage05_fixture = crate::policy::tests::Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        stage05_fixture.force_stage05_locked_placement_override();
        let upstream_validation = stage05_fixture
            .stage0_result()
            .expect("Stage 0 succeeds before policy failure");
        let upstream_report = upstream_validation.report.clone();
        let upstream_self_hash = upstream_validation.artifact_validation_self_hash;
        let mut stage05_harness = ChunkDispatchHarness::default();
        let stage05_failure = stage05_harness
            .run_until_failure(&stage05_fixture)
            .expect_err("locked placement override fails in real Stage 0.5")
            .expect_stage05("locked placement override fails in real Stage 0.5");
        assert_eq!(stage05_harness.order, vec!["stage0", "stage0.5"]);
        assert_eq!(
            stage05_harness.upstream_stage0_self_hash,
            Some(upstream_self_hash)
        );
        assert_eq!(stage05_failure.report.outcome, ReportOutcome::Failed);
        assert_eq!(upstream_validation.report, upstream_report);
        assert_eq!(
            upstream_validation.artifact_validation_self_hash,
            upstream_self_hash
        );
        assert_eq!(
            upstream_validation.report.report_self_hash,
            upstream_self_hash
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

    #[derive(Default)]
    struct ChunkDispatchHarness {
        order: Vec<&'static str>,
        upstream_stage0_self_hash: Option<Hash256>,
    }

    #[derive(Debug)]
    enum ChunkDispatchFailure {
        Stage0(ValidationStageFailure),
        Stage05(PolicyResolutionStageFailure),
    }

    impl ChunkDispatchFailure {
        fn expect_stage0(self, message: &str) -> ValidationStageFailure {
            match self {
                Self::Stage0(failure) => failure,
                Self::Stage05(_) => panic!("{message}: reached Stage 0.5 instead"),
            }
        }

        fn expect_stage05(self, message: &str) -> PolicyResolutionStageFailure {
            match self {
                Self::Stage05(failure) => failure,
                Self::Stage0(_) => panic!("{message}: failed in Stage 0 instead"),
            }
        }
    }

    impl ChunkDispatchHarness {
        fn run_success_path(
            &mut self,
            fixture: &crate::policy::tests::Fixture,
        ) -> Result<crate::budget::StaticBudgetReport, ChunkDispatchFailure> {
            let validation = self.stage0(fixture).map_err(ChunkDispatchFailure::Stage0)?;
            let policy = self
                .stage05(&validation)
                .map_err(ChunkDispatchFailure::Stage05)?;
            let quant_graph = self.stage1_synthetic(&policy);
            let runtime_budget = runtime_budget_fixture();

            Ok(self.stage2(&policy, &quant_graph, Some(&runtime_budget)))
        }

        fn run_until_failure(
            &mut self,
            fixture: &crate::policy::tests::Fixture,
        ) -> Result<crate::budget::StaticBudgetReport, ChunkDispatchFailure> {
            let validation = self.stage0(fixture).map_err(ChunkDispatchFailure::Stage0)?;
            let policy = self
                .stage05(&validation)
                .map_err(ChunkDispatchFailure::Stage05)?;
            let quant_graph = self.stage1_synthetic(&policy);

            Ok(self.stage2(&policy, &quant_graph, None))
        }

        fn stage0<'a>(
            &mut self,
            fixture: &'a crate::policy::tests::Fixture,
        ) -> Result<ValidationProduct<'a>, ValidationStageFailure> {
            self.order.push("stage0");
            fixture.stage0_result()
        }

        fn stage05(
            &mut self,
            validation: &ValidationProduct<'_>,
        ) -> Result<ResolvedPolicyProduct, PolicyResolutionStageFailure> {
            self.order.push("stage0.5");
            let upstream_report = validation.report.clone();
            let upstream_self_hash = validation.artifact_validation_self_hash;
            let result = resolve_policy(validation);
            assert_eq!(validation.report, upstream_report);
            assert_eq!(validation.artifact_validation_self_hash, upstream_self_hash);
            assert_eq!(validation.report.report_self_hash, upstream_self_hash);
            self.upstream_stage0_self_hash = Some(upstream_self_hash);
            result
        }

        fn stage1_synthetic(&mut self, policy: &ResolvedPolicyProduct) -> BudgetViewQuantGraph {
            self.order.push("stage1.synthetic");
            BudgetViewQuantGraph {
                view: empty_budget_view(policy.input_hashes.artifact_effective_core_hash),
            }
        }

        fn stage2(
            &mut self,
            policy: &ResolvedPolicyProduct,
            quant_graph: &BudgetViewQuantGraph,
            runtime_chrome_budget: Option<&RuntimeChromeBudget>,
        ) -> crate::budget::StaticBudgetReport {
            self.order.push("stage2");
            let target_profile = dmg_mbc5_8mib_128kib();
            run_stage2_static_budget(BudgetInputs {
                policy,
                quant_graph,
                runtime_chrome_budget,
                target_profile: &target_profile,
            })
        }
    }

    struct BudgetViewQuantGraph {
        view: QuantGraphBudgetView,
    }

    impl QuantGraphBudgetSource for BudgetViewQuantGraph {
        fn quant_graph_hash(&self) -> Hash256 {
            self.view.quant_graph_hash
        }

        fn semantic_core_hash(&self) -> Hash256 {
            self.view.semantic_core_hash
        }

        fn to_budget_view(&self) -> Result<QuantGraphBudgetView, QuantGraphBudgetViewError> {
            Ok(self.view.clone())
        }
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

    fn empty_budget_view(semantic_core_hash: Hash256) -> QuantGraphBudgetView {
        QuantGraphBudgetView {
            semantic_core_hash,
            quant_graph_hash: hash(0xe0),
            layers: Vec::new(),
            experts: Vec::new(),
            shared_kernels: Vec::new(),
            shared_luts: Vec::new(),
            shared_dense_ffn: None,
            reduction_sites: Vec::new(),
            sequence_state: SequenceStateProjection::default(),
            routing: RoutingProjection::default(),
        }
    }

    fn runtime_budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(0x40),
            rom_slots: vec![RomBudgetSlot {
                id: BudgetSlotId::new(1),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 1024,
                reserved_slack: 128,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            }],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x09),
            },
            wram_reserved: 0,
            sram_reserved: 0,
        }
    }
}
