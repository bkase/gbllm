use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use gbf_artifact::{ArtifactFeature, GoldenVectorId as ArtifactGoldenVectorId, HintBundle};
use gbf_foundation::{
    BudgetSlotId, CompileProfileId, ExpertId, FieldPath, GoldenVectorId, Hash256,
    KernelCalibrationId, LayerId, LineageId, PlatformCalibrationId, RuntimeCalibrationId, SemVer,
    TargetProfileId, WorkloadId,
};
use gbf_hw::calibration::CalibrationSetRef;
use gbf_policy::{
    BudgetFailure, BudgetSlotClass, CalibrationConfidenceRequirement, CompileKnobId,
    CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobValues, CompileObjective,
    CompilerFeature, ConstraintOperation, ConstraintProvenance, EffectiveConstraints, EvidenceRef,
    KnobLockSet, ObservabilityMode, ObservationKnob, ObservationProfileCaps, OverlayKnob,
    OverlayPromotion, PlacementKnob, PlacementProfile, PolicySource, ProbeCollectionLevel,
    RangeCapsSpec, RangeKnob, ReductionPlanCeiling, ReductionSiteId, RepairPolicy,
    RepairPolicyProfile, RiskPolicy, RomKernelDuplicationBias, RomKernelResidencyBias,
    RomWindowKnob, RuntimeMode, ScheduleKnob, ScheduleResourcePressure, ScheduleSliceCoarsening,
    ScheduleTileSearch, ServiceLevelObjective, SramKnob, SramPageAggression, StorageKnob,
    StorageMaterialization, SwitchProjectionSource, TraceBudget, TraceDropPolicy, ValidationCode,
    ValidationDetail, ValidationDiagnostic, ValidationOrigin, budget_failure_diagnostic,
    canonical_default_bounds_fixture,
};
use gbf_report::report_schemas::artifact_validation_v1::{
    ArtifactCompatibilityDecision, ArtifactCompatibilityFailure, ArtifactCompatibilitySection,
    ArtifactValidationIdentitySection, ArtifactValidationInputSection,
    ArtifactValidationReportBody,
};
use gbf_report::report_schemas::policy_resolution_v1::{
    ArtifactIdentitySection, CompileKnobsSection, CompileRequestSection, HintConsumptionSection,
    PolicyProvenanceSection, PolicyResolutionReportBody, PolicyResolutionSuccessSection,
    ResolvedSection,
};
use gbf_report::report_schemas::static_budget_v1::{
    AccumulatorBound, BudgetComponentRef, BudgetDecisionSection, BudgetIdentitySection,
    BudgetPolicySection, BudgetProjectionSection, CommonBankFootprintSection,
    ExpertPlacementStatus, PerBankEntry, PerExpertEntry, ProjectedSizeSection, ProjectedSizeSource,
    ProjectedSwitchCountSection, RomBudgetSlotEntry, RoutingModelSection,
    RuntimeChromeBudgetSection, RuntimeMemoryCapSection, StaticBudgetReportBody,
    StaticPlacementModel, runtime_chrome_budget_hash, static_fit_interpretation_for_fits,
};
use gbf_report::{ReportEnvelope, ReportOutcome, canonicalize, round_trip_self_hash};
use sha2::{Digest, Sha256};

const ARTIFACT_SUCCESS: &str = "artifact_validation.golden.json";
const ARTIFACT_FAILURE: &str = "artifact_validation.failure.golden.json";
const POLICY_SUCCESS: &str = "policy_resolution.golden.json";
const POLICY_FAILURE: &str = "policy_resolution.failure.golden.json";
const STATIC_BUDGET_SUCCESS: &str = "static_budget.golden.json";
const STATIC_BUDGET_FAILURE: &str = "static_budget.failure.golden.json";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "regen".to_owned());
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_artifact_dir);

    match command.as_str() {
        "regen" => regen(&out_dir),
        "verify" => verify(&out_dir),
        other => Err(format!("unknown command {other}; expected regen or verify").into()),
    }
}

fn default_artifact_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs/review/f-b2-f-b4/artifacts")
}

fn regen(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if out_dir.exists() {
        fs::remove_dir_all(out_dir)?;
    }
    fs::create_dir_all(out_dir)?;
    write_report(out_dir, ARTIFACT_SUCCESS, &artifact_success_report()?)?;
    write_report(out_dir, ARTIFACT_FAILURE, &artifact_failure_report()?)?;
    write_report(out_dir, POLICY_SUCCESS, &policy_success_report()?)?;
    write_report(out_dir, POLICY_FAILURE, &policy_failure_report()?)?;
    write_report(
        out_dir,
        STATIC_BUDGET_SUCCESS,
        &static_budget_success_report()?,
    )?;
    write_report(
        out_dir,
        STATIC_BUDGET_FAILURE,
        &static_budget_failure_report()?,
    )?;
    write_fixture_tomls(out_dir)?;
    verify(out_dir)
}

fn verify(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    verify_report::<ArtifactValidationReportBody>(
        out_dir,
        ARTIFACT_SUCCESS,
        ReportOutcome::Passed,
    )?;
    verify_report::<ArtifactValidationReportBody>(
        out_dir,
        ARTIFACT_FAILURE,
        ReportOutcome::Failed,
    )?;
    verify_report::<PolicyResolutionReportBody>(out_dir, POLICY_SUCCESS, ReportOutcome::Passed)?;
    let policy_failure = verify_report::<PolicyResolutionReportBody>(
        out_dir,
        POLICY_FAILURE,
        ReportOutcome::Failed,
    )?;
    if policy_failure.body.result.is_some() {
        return Err("policy_resolution failure golden must carry result = null".into());
    }
    verify_report::<StaticBudgetReportBody>(out_dir, STATIC_BUDGET_SUCCESS, ReportOutcome::Passed)?;
    let static_budget_failure = verify_report::<StaticBudgetReportBody>(
        out_dir,
        STATIC_BUDGET_FAILURE,
        ReportOutcome::Failed,
    )?;
    if !static_budget_failure
        .body
        .diagnostics
        .iter()
        .any(|diagnostic| {
            matches!(
                diagnostic.code,
                ValidationCode::BudgetMissingRuntimeChromeBudget
            )
        })
    {
        return Err(
            "static_budget failure golden must carry BudgetMissingRuntimeChromeBudget diagnostic"
                .into(),
        );
    }
    if !static_budget_failure
        .body
        .decision
        .failures
        .contains(&BudgetFailure::MissingRuntimeChromeBudget)
    {
        return Err(
            "static_budget failure golden must carry BudgetFailure::MissingRuntimeChromeBudget"
                .into(),
        );
    }
    Ok(())
}

fn write_report<R>(
    out_dir: &Path,
    file_name: &str,
    report: &ReportEnvelope<R>,
) -> Result<(), Box<dyn std::error::Error>>
where
    R: gbf_report::ReportBody + serde::Serialize,
{
    let bytes = canonicalize(report)?;
    let path = out_dir.join(file_name);
    fs::write(&path, &bytes)?;
    fs::write(
        out_dir.join(file_name.replace(".json", ".sha256")),
        format!("sha256:{}\n", sha256_hex(&bytes)),
    )?;
    Ok(())
}

fn verify_report<R>(
    out_dir: &Path,
    file_name: &str,
    outcome: ReportOutcome,
) -> Result<ReportEnvelope<R>, Box<dyn std::error::Error>>
where
    R: gbf_report::ReportBody + serde::Serialize + serde::de::DeserializeOwned,
{
    let path = out_dir.join(file_name);
    let bytes = fs::read(&path)?;
    let expected_sidecar = format!("sha256:{}\n", sha256_hex(&bytes));
    let sidecar = fs::read_to_string(out_dir.join(file_name.replace(".json", ".sha256")))?;
    if sidecar != expected_sidecar {
        return Err(format!("{file_name} sidecar is stale").into());
    }

    let report: ReportEnvelope<R> = serde_json::from_slice(&bytes)?;
    if report.outcome != outcome {
        return Err(format!("{file_name} has wrong outcome").into());
    }
    round_trip_self_hash(&report)?;
    if canonicalize(&report)? != bytes {
        return Err(format!("{file_name} is not canonical JSON").into());
    }

    Ok(report)
}

fn artifact_success_report()
-> Result<ReportEnvelope<ArtifactValidationReportBody>, Box<dyn std::error::Error>> {
    Ok(ReportEnvelope::new(
        ReportOutcome::Passed,
        ArtifactValidationReportBody {
            identity: ArtifactValidationIdentitySection {
                artifact_source_hash: Some(hash(0x01)),
                artifact_effective_core_hash: Some(hash(0x02)),
                artifact_manifest_hash: Some(hash(0x03)),
                semantic_core_hash: Some(hash(0x02)),
                artifact_aux_hash: Some(hash(0x05)),
                lowering_manifest_hash: Some(hash(0x06)),
                hint_bundle_hash: HintBundle::empty().compute_canonical_hash(),
                compile_request_hash: hash(0x08),
                target_profile_hash: hash(0x09),
                compile_profile_hash: hash(0x0a),
                calibration_hash: Some(hash(0x0b)),
                compatibility_adapter_hash: None,
            },
            compatibility: ArtifactCompatibilitySection {
                decision: Some(ArtifactCompatibilityDecision::CurrentSchema),
                failures: Vec::new(),
            },
            checked_inputs: ArtifactValidationInputSection {
                workload_refs: vec![WorkloadId::from("workload.f-b2.review")],
                golden_vector_refs: vec![ArtifactGoldenVectorId("golden.f-b2.review".to_owned())],
                required_artifact_features: BTreeSet::from([ArtifactFeature::DenseI8]),
                required_compiler_features: BTreeSet::from([
                    CompilerFeature::ArtifactValidation,
                    CompilerFeature::PolicyResolution,
                ]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
            },
            diagnostics: Vec::new(),
        },
    )?
    .with_computed_self_hash()?)
}

fn artifact_failure_report()
-> Result<ReportEnvelope<ArtifactValidationReportBody>, Box<dyn std::error::Error>> {
    Ok(ReportEnvelope::new(
        ReportOutcome::Failed,
        ArtifactValidationReportBody {
            identity: ArtifactValidationIdentitySection {
                artifact_source_hash: Some(hash(0x01)),
                artifact_effective_core_hash: None,
                artifact_manifest_hash: None,
                semantic_core_hash: None,
                artifact_aux_hash: None,
                lowering_manifest_hash: None,
                hint_bundle_hash: HintBundle::empty().compute_canonical_hash(),
                compile_request_hash: hash(0x08),
                target_profile_hash: hash(0x09),
                compile_profile_hash: hash(0x0a),
                calibration_hash: None,
                compatibility_adapter_hash: None,
            },
            compatibility: ArtifactCompatibilitySection {
                decision: None,
                failures: vec![ArtifactCompatibilityFailure::UnsupportedEpoch {
                    observed: SemVer::new(2, 0, 0),
                    supported: SemVer::new(1, 1, 0),
                }],
            },
            checked_inputs: ArtifactValidationInputSection {
                workload_refs: Vec::new(),
                golden_vector_refs: Vec::new(),
                required_artifact_features: BTreeSet::new(),
                required_compiler_features: BTreeSet::new(),
                requested_runtime_modes: BTreeSet::new(),
            },
            diagnostics: vec![artifact_hard_diagnostic()],
        },
    )?
    .with_computed_self_hash()?)
}

fn policy_success_report()
-> Result<ReportEnvelope<PolicyResolutionReportBody>, Box<dyn std::error::Error>> {
    let policy = policy_fixture();
    Ok(ReportEnvelope::new(
        ReportOutcome::Passed,
        PolicyResolutionReportBody {
            artifact_identity: ArtifactIdentitySection {
                artifact_core_hash: hash(0x02),
                artifact_manifest_hash: hash(0x03),
                semantic_lineage: LineageId(hash(0x0c)),
                lowering_manifest_hash: hash(0x06),
                hint_bundle_hash: empty_hint_bundle_hash(),
                workload_refs: vec![WorkloadId::from("workload.f-b2.review")],
                golden_vector_refs: vec![GoldenVectorId("golden.f-b2.review".to_owned())],
            },
            compile_request: compile_request_section(),
            result: Some(PolicyResolutionSuccessSection {
                resolved: ResolvedSection::from(&policy),
                compile_knobs: CompileKnobsSection::from(&policy.knobs),
                provenance: PolicyProvenanceSection::from_policy(
                    &policy.provenance,
                    empty_hint_bundle_hash(),
                    hash(0x0b),
                ),
            }),
            hint_consumption: HintConsumptionSection::default(),
            diagnostics: Vec::new(),
        },
    )?
    .with_computed_self_hash()?)
}

fn policy_failure_report()
-> Result<ReportEnvelope<PolicyResolutionReportBody>, Box<dyn std::error::Error>> {
    let mut report = policy_success_report()?;
    report.outcome = ReportOutcome::Failed;
    report.report_self_hash = Hash256::ZERO;
    report.body.result = None;
    report.body.diagnostics = vec![policy_hard_diagnostic()];
    Ok(report.with_computed_self_hash()?)
}

fn static_budget_success_report()
-> Result<ReportEnvelope<StaticBudgetReportBody>, Box<dyn std::error::Error>> {
    let budget = static_runtime_budget_section();
    let budget_hash = runtime_chrome_budget_hash(&budget)?;
    Ok(ReportEnvelope::new(
        ReportOutcome::Passed,
        StaticBudgetReportBody {
            identity: static_budget_identity(Some(budget_hash)),
            policy: static_budget_policy_section(),
            runtime_chrome_budget: Some(budget),
            projections: static_budget_projection_section(),
            decision: static_budget_decision(true, Vec::new()),
            diagnostics: Vec::new(),
        },
    )?
    .with_computed_self_hash()?)
}

fn static_budget_failure_report()
-> Result<ReportEnvelope<StaticBudgetReportBody>, Box<dyn std::error::Error>> {
    let failure = BudgetFailure::MissingRuntimeChromeBudget;
    Ok(ReportEnvelope::new(
        ReportOutcome::Failed,
        StaticBudgetReportBody {
            identity: static_budget_identity(None),
            policy: static_budget_policy_section(),
            runtime_chrome_budget: None,
            projections: BudgetProjectionSection::default(),
            decision: static_budget_decision(false, vec![failure.clone()]),
            diagnostics: vec![budget_failure_diagnostic(&failure)],
        },
    )?
    .with_computed_self_hash()?)
}

fn static_budget_identity(runtime_chrome_budget_hash: Option<Hash256>) -> BudgetIdentitySection {
    BudgetIdentitySection {
        artifact_core_hash: hash(0x02),
        quant_graph_hash: hash(0x23),
        policy_resolution_self_hash: policy_success_report()
            .expect("policy report builds")
            .report_self_hash,
        runtime_chrome_budget_hash,
        target_profile_hash: hash(0x09),
    }
}

fn static_budget_policy_section() -> BudgetPolicySection {
    BudgetPolicySection {
        placement_profile: PlacementProfile::Budgeted,
        objective_hash: hash(0x24),
    }
}

fn static_runtime_budget_section() -> RuntimeChromeBudgetSection {
    RuntimeChromeBudgetSection {
        target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        profile: CompileProfileId::from("Bringup"),
        runtime_nucleus_hash: hash(0x40),
        rom_slots: vec![
            RomBudgetSlotEntry {
                id: BudgetSlotId::new(1),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 1024,
                reserved_slack: 128,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            },
            RomBudgetSlotEntry {
                id: BudgetSlotId::new(2),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 2048,
                reserved_slack: 0,
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            },
        ],
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

fn static_budget_projection_section() -> BudgetProjectionSection {
    BudgetProjectionSection {
        per_expert_payload: vec![PerExpertEntry {
            layer: LayerId::new(0),
            expert: ExpertId::new(0),
            payload_bytes: 64,
            assigned_slot: Some(BudgetSlotId::new(1)),
            unassigned_because: None,
            placement_status: ExpertPlacementStatus::Assigned,
        }],
        per_bank_occupancy: vec![
            PerBankEntry {
                slot: BudgetSlotId::new(1),
                class: BudgetSlotClass::ExpertBank,
                usable_bytes: 1024,
                reserved_slack: 128,
                effective_cap_bytes: 896,
                assigned_bytes: 64,
                residual_bytes: 832,
                assigned_components: vec![BudgetComponentRef::Expert {
                    layer: LayerId::new(0),
                    expert: ExpertId::new(0),
                }],
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            },
            PerBankEntry {
                slot: BudgetSlotId::new(2),
                class: BudgetSlotClass::CommonBank,
                usable_bytes: 2048,
                reserved_slack: 0,
                effective_cap_bytes: 2048,
                assigned_bytes: 0,
                residual_bytes: 2048,
                assigned_components: Vec::new(),
                placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
            },
        ],
        common_bank_footprint: CommonBankFootprintSection {
            kernel_bytes: 0,
            lut_bytes: 0,
            shared_dense_ffn_bytes: None,
            aggregate_bytes: 0,
        },
        accumulator_maxima: vec![AccumulatorBound {
            site: ReductionSiteId("review.accumulator.0".to_owned()),
            projected_max_abs: 127,
            i16_safe: true,
            i32_safe: true,
        }],
        projected_wram: ProjectedSizeSection {
            peak_bytes: 256,
            source: ProjectedSizeSource::StaticGraphProjection,
        },
        projected_sram: ProjectedSizeSection {
            peak_bytes: 0,
            source: ProjectedSizeSource::StaticGraphProjection,
        },
        projected_hram: ProjectedSizeSection {
            peak_bytes: 8,
            source: ProjectedSizeSource::StaticGraphProjection,
        },
        projected_bank_switches_per_token: ProjectedSwitchCountSection {
            upper_bound: 1,
            expected_q16_16: None,
            decision_value: 1,
            source: SwitchProjectionSource::ConservativeStaticUpperBound,
        },
        projected_sram_page_switches_per_token: ProjectedSwitchCountSection::default(),
        routing_model: RoutingModelSection {
            kind: "Top1DeterministicReviewFixture".to_owned(),
        },
    }
}

fn static_budget_decision(fits: bool, failures: Vec<BudgetFailure>) -> BudgetDecisionSection {
    BudgetDecisionSection {
        fits,
        interpretation: static_fit_interpretation_for_fits(fits),
        placement_model: StaticPlacementModel::BudgetedFirstFit,
        failures,
    }
}

fn compile_request_section() -> CompileRequestSection {
    CompileRequestSection {
        compile_request_hash: hash(0x08),
        target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        target_profile_hash: hash(0x09),
        profile: CompileProfileId::from("Bringup"),
        objective: objective_fixture(),
        required_features: BTreeSet::from([
            CompilerFeature::ArtifactValidation,
            CompilerFeature::PolicyResolution,
        ]),
        requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
        calibration_set_ref: CalibrationSetRef {
            platform: Some(PlatformCalibrationId::from("bootstrap-dmg-platform")),
            kernel: Some(KernelCalibrationId::from("bootstrap-dmg-kernel")),
            runtime: Some(RuntimeCalibrationId::from("bootstrap-dmg-runtime")),
        },
        calibration_hash: hash(0x0b),
    }
}

fn policy_fixture() -> gbf_policy::ResolvedCompilePolicy {
    let values = CompileKnobValues {
        placement: PlacementKnob {
            profile: PlacementProfile::StrictOnePerBank,
        },
        observation: ObservationKnob {
            observability: ObservabilityMode::Invariant,
            trace_demotion: gbf_policy::TraceDemotionLevel::None,
            probe_level: ProbeCollectionLevel::RequiredOnly,
        },
        range: RangeKnob {
            reduction_ceiling: ReductionPlanCeiling::Conservative,
        },
        storage: StorageKnob {
            materialization: StorageMaterialization::RecomputePureValues,
        },
        sram: SramKnob {
            page_aggression: SramPageAggression::PackCold,
            spill_policy: gbf_policy::SramSpillPolicy::SpillOnPressure,
        },
        rom_window: RomWindowKnob {
            kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
            kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
        },
        overlay: OverlayKnob {
            promotion: OverlayPromotion::TinyLuts,
        },
        schedule: ScheduleKnob {
            tile_search: ScheduleTileSearch::Local,
            slice_coarsening: ScheduleSliceCoarsening::Balanced,
            resource_pressure: ScheduleResourcePressure::Conservative,
            pressure_thresholds: gbf_policy::ResourcePressureThresholds::default(),
            stage_iteration_ceilings: gbf_policy::StageIterationLimits::uniform(4),
        },
    };
    let bounds = canonical_default_bounds_fixture();

    gbf_policy::ResolvedCompilePolicy {
        target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        profile: CompileProfileId::from("Bringup"),
        objective: objective_fixture(),
        effective_constraints: EffectiveConstraints {
            target_caps: bounds.clone(),
            required_features: BTreeSet::from([
                CompilerFeature::ArtifactValidation,
                CompilerFeature::PolicyResolution,
            ]),
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
            runtime_chrome_budget: None,
        },
        observability: ObservabilityMode::Invariant,
        trace_budget: TraceBudget {
            max_events_per_slice: 4,
            max_bytes_per_frame: 128,
            drop_policy: TraceDropPolicy::HaltAndFault,
        },
        range_caps: RangeCapsSpec::default_v2(),
        observation_caps: ObservationProfileCaps::default_v2(),
        requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
        knobs: gbf_policy::CompileKnobs {
            global: values,
            bounds,
            locks: KnobLockSet::default(),
            overrides: gbf_policy::CompileKnobOverrides::default(),
            provenance: knob_provenance_entries(),
        },
        repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
        provenance: gbf_policy::PolicyProvenance {
            target_defaults: hash(0x09),
            profile_defaults: hash(0x0a),
            compile_profile_spec_version: "2.0.0".to_owned(),
            hint_bundle_hash: Some(empty_hint_bundle_hash()),
            compile_request_hash: hash(0x08),
            calibration_hash: Some(hash(0x0b)),
        },
    }
}

fn knob_provenance_entries() -> Vec<CompileKnobProvenanceEntry> {
    let mut entries = vec![
        knob_entry(
            CompileKnobId::Placement,
            "bounds.max_profile",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Placement,
            "global.profile",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Observation,
            "bounds.max_probe_level",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Observation,
            "global.observability",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Observation,
            "global.probe_level",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Range,
            "bounds.max_reduction_ceiling",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Range,
            "global.reduction_ceiling",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Storage,
            "bounds.max_materialization",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Storage,
            "global.materialization",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Sram,
            "bounds.max_page_aggression",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Sram,
            "global.page_aggression",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::RomWindow,
            "bounds.max_kernel_duplication_bias",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::RomWindow,
            "bounds.max_kernel_residency_bias",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::RomWindow,
            "global.kernel_duplication_bias",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::RomWindow,
            "global.kernel_residency_bias",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Overlay,
            "bounds.max_promotion",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Overlay,
            "global.promotion",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Schedule,
            "bounds.max_resource_pressure",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Schedule,
            "bounds.max_slice_coarsening",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Schedule,
            "bounds.max_tile_search",
            PolicySource::TargetDefault,
        ),
        knob_entry(
            CompileKnobId::Schedule,
            "global.resource_pressure",
            PolicySource::Calibration,
        ),
        knob_entry(
            CompileKnobId::Schedule,
            "global.slice_coarsening",
            PolicySource::ProfileDefault,
        ),
        knob_entry(
            CompileKnobId::Schedule,
            "global.tile_search",
            PolicySource::ProfileDefault,
        ),
    ];
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

fn knob_entry(
    knob: CompileKnobId,
    field: &'static str,
    source: PolicySource,
) -> CompileKnobProvenanceEntry {
    CompileKnobProvenanceEntry {
        path: CompileKnobPath {
            knob,
            selector: None,
            field: Some(FieldPath::from(field)),
        },
        chain: vec![ConstraintProvenance {
            source: source.clone(),
            operation: match source {
                PolicySource::TargetDefault | PolicySource::ProfileDefault => {
                    ConstraintOperation::SeedDefault
                }
                PolicySource::Calibration => ConstraintOperation::ApplyCalibration,
                PolicySource::CompileRequestOverride => ConstraintOperation::ApplyOverride,
                PolicySource::HintBundle => ConstraintOperation::ApplyHardConstraint,
                PolicySource::RepairProposal { .. } => unreachable!("F-B2 forbids RepairProposal"),
            },
            evidence: vec![EvidenceRef {
                kind: match source {
                    PolicySource::TargetDefault => "TargetProfile",
                    PolicySource::ProfileDefault => "CompileProfileSpec",
                    PolicySource::CompileRequestOverride => "CompileRequest",
                    PolicySource::HintBundle => "HintBundle",
                    PolicySource::Calibration => "BootstrapCalibrationBundle",
                    PolicySource::RepairProposal { .. } => {
                        unreachable!("F-B2 forbids RepairProposal")
                    }
                }
                .to_owned(),
                reference: field.to_owned(),
                hash: Some(match source {
                    PolicySource::TargetDefault => hash(0x09),
                    PolicySource::ProfileDefault => hash(0x0a),
                    PolicySource::CompileRequestOverride => hash(0x08),
                    PolicySource::HintBundle => empty_hint_bundle_hash(),
                    PolicySource::Calibration => hash(0x0b),
                    PolicySource::RepairProposal { .. } => {
                        unreachable!("F-B2 forbids RepairProposal")
                    }
                }),
            }],
        }],
    }
}

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
        min_sustained_throughput_tokens_per_megacycle: None,
        min_ui_headroom_pct: 11,
        max_rom_bytes: Some(8 * 1024 * 1024),
        risk: RiskPolicy {
            cycle_quantile: 95,
            switch_quantile: 99,
            calibration_confidence_requirement:
                CalibrationConfidenceRequirement::NoMinimumConfidence,
            fallback_profile: None,
            fallback_runtime_mode: Some(RuntimeMode::Safe),
        },
    }
}

fn artifact_hard_diagnostic() -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Schema,
        ValidationCode::SchemaEpochUnsupported,
        ValidationDetail::Field {
            field: FieldPath::from("manifest.schema_version.epoch"),
        },
        vec![EvidenceRef {
            kind: "artifact_manifest".to_owned(),
            reference: "manifest.schema_version.epoch".to_owned(),
            hash: Some(hash(0xaa)),
        }],
    )
}

fn policy_hard_diagnostic() -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::PolicyResolution,
        ValidationCode::PolicyKnobLockedAndOverridden {
            knob: CompileKnobId::RomWindow,
        },
        ValidationDetail::Field {
            field: FieldPath::from("compile_knobs.overrides.values.rom_window"),
        },
        vec![EvidenceRef {
            kind: "CompileRequest".to_owned(),
            reference: "constraint_overrides.values.rom_window".to_owned(),
            hash: Some(hash(0x08)),
        }],
    )
}

fn write_fixture_tomls(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(
        out_dir.join("artifact_validation.fixture.toml"),
        r#"# F-B2 Stage 0 review fixture.
#
# This tiny fixture describes the generated success and failure reports in this
# directory. These fields intentionally mirror the F-B2 Stage 0 report identity
# sections so reviewers can read the whole fixture in one screen.

[success]
profile = "Bringup"
target = "dmg-mbc5-8mib-128kib"
artifact_schema = "1.1.0"
artifact_feature = "DenseI8"
workload_ref = "workload.f-b2.review"
golden_vector_ref = "golden.f-b2.review"
diagnostics = []

[failure]
case = "unsupported_schema_epoch"
artifact_schema = "2.0.0"
expected_outcome = "Failed"
expected_hard_diagnostic = "SchemaEpochUnsupported"
"#,
    )?;
    fs::write(
        out_dir.join("policy_resolution.fixture.toml"),
        r#"# F-B2 Stage 0.5 review fixture.
#
# The success case proves compile_knobs provenance is populated without
# RepairProposal sources. The failure case preserves artifact identity but sets
# result = null, matching RFC §7.5.

[success]
profile = "Bringup"
target = "dmg-mbc5-8mib-128kib"
allowed_policy_sources = [
  "TargetDefault",
  "ProfileDefault",
  "CompileRequestOverride",
  "HintBundle",
  "Calibration",
]
forbidden_policy_source = "RepairProposal"

[failure]
case = "locked_knob_override"
expected_outcome = "Failed"
expected_result = "null"
expected_hard_diagnostic = "PolicyKnobLockedAndOverridden"
"#,
    )?;
    fs::write(
        out_dir.join("static_budget.fixture.toml"),
        r#"# F-B4 Stage 2 review fixture.
#
# The success case is a tiny synthetic QuantGraph-budget projection that fits
# a runtime chrome budget. The failure case models the RFC §7.7 missing-budget
# short circuit: Stage 2 records no view-derived payload and emits both the
# Hard diagnostic and BudgetFailure marker.

[success]
profile = "Bringup"
target = "dmg-mbc5-8mib-128kib"
placement_profile = "Budgeted"
quant_graph_hash = "2323232323232323232323232323232323232323232323232323232323232323"
runtime_chrome_budget = "present"
expert_payload_bytes = 64
assigned_slot = 1
diagnostics = []

[failure]
case = "missing_runtime_chrome_budget"
expected_outcome = "Failed"
expected_hard_diagnostic = "BudgetMissingRuntimeChromeBudget"
expected_budget_failure = "BudgetFailure::MissingRuntimeChromeBudget"
runtime_chrome_budget = "null"
"#,
    )?;
    Ok(())
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn empty_hint_bundle_hash() -> Hash256 {
    HintBundle::empty().compute_canonical_hash()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
