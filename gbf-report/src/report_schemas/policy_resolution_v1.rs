//! `policy_resolution.v1` Stage 0.5 report schema.

use std::collections::BTreeSet;

use gbf_foundation::{
    CompileProfileId, FieldPath, GoldenVectorId, Hash256, LineageId, TargetProfileId, WorkloadId,
};
use gbf_hw::calibration::CalibrationSetRef;
use gbf_policy::{
    CompileKnobBounds, CompileKnobId, CompileKnobOverrides, CompileKnobProvenanceEntry,
    CompileKnobValues, CompileKnobs, CompileObjective, CompilerFeature, ConstraintProvenance,
    DiagnosticSeverity, EffectiveConstraints, KnobLockSet, MonotoneKnob, ObservabilityMode,
    PolicyProvenance, PolicySource, RepairPolicy, ResolvedCompilePolicy, RuntimeMode, TraceBudget,
    ValidationCode, ValidationDetail, ValidationDiagnostic, ValidationOrigin,
};
use serde::{Deserialize, Serialize};

use crate::{ReportBody, ReportOutcome};

pub const SCHEMA_ID: &str = "policy_resolution.v1";
pub const SCHEMA_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyResolutionReportBody {
    pub artifact_identity: ArtifactIdentitySection,
    pub compile_request: CompileRequestSection,
    pub result: Option<PolicyResolutionSuccessSection>,
    pub hint_consumption: HintConsumptionSection,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyResolutionSuccessSection {
    pub resolved: ResolvedSection,
    pub compile_knobs: CompileKnobsSection,
    pub provenance: PolicyProvenanceSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentitySection {
    pub artifact_core_hash: Hash256,
    pub artifact_manifest_hash: Hash256,
    pub semantic_lineage: LineageId,
    pub lowering_manifest_hash: Hash256,
    pub hint_bundle_hash: Hash256,
    pub workload_refs: Vec<WorkloadId>,
    pub golden_vector_refs: Vec<GoldenVectorId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileRequestSection {
    pub compile_request_hash: Hash256,
    pub target: TargetProfileId,
    pub target_profile_hash: Hash256,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub required_features: BTreeSet<CompilerFeature>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub calibration_set_ref: CalibrationSetRef,
    pub calibration_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSection {
    pub effective_constraints: EffectiveConstraints,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub repair: RepairPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobsSection {
    pub global: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub locks: KnobLockSet,
    pub overrides: CompileKnobOverrides,
    pub provenance: Vec<CompileKnobProvenanceEntry>,
}

impl From<&CompileKnobs> for CompileKnobsSection {
    fn from(value: &CompileKnobs) -> Self {
        Self {
            global: value.global.clone(),
            bounds: value.bounds.clone(),
            locks: value.locks.clone(),
            overrides: value.overrides.clone(),
            provenance: value.provenance.clone(),
        }
    }
}

impl From<&ResolvedCompilePolicy> for ResolvedSection {
    fn from(value: &ResolvedCompilePolicy) -> Self {
        Self {
            effective_constraints: value.effective_constraints.clone(),
            observability: value.observability,
            trace_budget: value.trace_budget,
            repair: value.repair,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyProvenanceSection {
    pub target_defaults: Hash256,
    pub profile_defaults: Hash256,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub calibration_hash: Hash256,
}

impl PolicyProvenanceSection {
    #[must_use]
    pub fn from_policy(
        value: &PolicyProvenance,
        hint_bundle_hash: Hash256,
        calibration_hash: Hash256,
    ) -> Self {
        Self {
            target_defaults: value.target_defaults,
            profile_defaults: value.profile_defaults,
            hint_bundle_hash: value.hint_bundle_hash.unwrap_or(hint_bundle_hash),
            compile_request_hash: value.compile_request_hash,
            calibration_hash: value.calibration_hash.unwrap_or(calibration_hash),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HintConsumptionSection {
    pub facts_used: Vec<FactUse>,
    pub preferences_honored: Vec<PreferenceUse>,
    pub preferences_ignored: Vec<IgnoredPreference>,
    pub constraints_enforced: Vec<ConstraintEnforcement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactUse {
    pub fact: FieldPath,
    pub provenance: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferenceUse {
    pub preference: FieldPath,
    pub knob: CompileKnobId,
    pub provenance: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IgnoredPreference {
    pub preference: FieldPath,
    pub knob: CompileKnobId,
    pub reason: IgnoredPreferenceReason,
    pub provenance: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum IgnoredPreferenceReason {
    OutsideBounds,
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintEnforcement {
    pub constraint: FieldPath,
    pub knob: CompileKnobId,
    pub provenance: Vec<ConstraintProvenance>,
}

pub type ValidationDiagnosticRecord = ValidationDiagnostic;

impl ReportBody for PolicyResolutionReportBody {
    const REPORT_TYPE: &'static str = "PolicyResolutionReport";
    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: &'static str = SCHEMA_VERSION;

    fn validate_semantics(&self, outcome: ReportOutcome) -> Result<(), Vec<ValidationDiagnostic>> {
        let mut errors = Vec::new();
        let has_hard = self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);

        for diagnostic in &self.diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Soft {
                errors.push(semantic_error("soft_diagnostic"));
            }
        }

        match outcome {
            ReportOutcome::Passed => {
                if has_hard || self.result.is_none() {
                    errors.push(semantic_error("passed_result"));
                }
            }
            ReportOutcome::Failed => {
                if !has_hard || self.result.is_some() {
                    errors.push(semantic_error("failed_result"));
                }
            }
        }

        if let Some(result) = &self.result {
            if !result
                .compile_knobs
                .provenance
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            {
                errors.push(semantic_error("compile_knobs.provenance"));
            }

            for entry in &result.compile_knobs.provenance {
                validate_provenance_chain(
                    "compile_knobs.provenance.chain",
                    &entry.chain,
                    true,
                    &mut errors,
                );
            }

            if !bounds_are_monotone_tighter_than_target_defaults(
                &result.compile_knobs.bounds,
                &result.resolved.effective_constraints.target_caps,
            ) {
                errors.push(semantic_error("compile_knobs.bounds"));
            }
        }

        validate_hint_consumption(&self.hint_consumption, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn bounds_are_monotone_tighter_than_target_defaults(
    bounds: &CompileKnobBounds,
    target_defaults: &CompileKnobBounds,
) -> bool {
    bounds
        .placement
        .is_monotone_successor_of(&target_defaults.placement)
        && bounds
            .observation
            .is_monotone_successor_of(&target_defaults.observation)
        && bounds
            .range
            .is_monotone_successor_of(&target_defaults.range)
        && bounds
            .storage
            .is_monotone_successor_of(&target_defaults.storage)
        && bounds.sram.is_monotone_successor_of(&target_defaults.sram)
        && bounds
            .rom_window
            .is_monotone_successor_of(&target_defaults.rom_window)
        && bounds
            .overlay
            .is_monotone_successor_of(&target_defaults.overlay)
        && bounds
            .schedule
            .is_monotone_successor_of(&target_defaults.schedule)
}

fn validate_hint_consumption(
    hint_consumption: &HintConsumptionSection,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    let mut preferences = BTreeSet::new();
    for honored in &hint_consumption.preferences_honored {
        if !preferences.insert(honored.preference.clone()) {
            errors.push(semantic_error("hint_consumption.preferences"));
        }
        validate_provenance_chain(
            "hint_consumption.preferences_honored.provenance",
            &honored.provenance,
            false,
            errors,
        );
    }

    for ignored in &hint_consumption.preferences_ignored {
        if !preferences.insert(ignored.preference.clone()) {
            errors.push(semantic_error("hint_consumption.preferences"));
        }
        validate_provenance_chain(
            "hint_consumption.preferences_ignored.provenance",
            &ignored.provenance,
            false,
            errors,
        );
    }

    for fact in &hint_consumption.facts_used {
        validate_provenance_chain(
            "hint_consumption.facts_used.provenance",
            &fact.provenance,
            false,
            errors,
        );
    }

    for constraint in &hint_consumption.constraints_enforced {
        validate_provenance_chain(
            "hint_consumption.constraints_enforced.provenance",
            &constraint.provenance,
            false,
            errors,
        );
    }
}

fn validate_provenance_chain(
    field: &'static str,
    chain: &[ConstraintProvenance],
    require_non_empty: bool,
    errors: &mut Vec<ValidationDiagnostic>,
) {
    if require_non_empty && chain.is_empty() {
        errors.push(semantic_error(field));
    }

    if chain
        .iter()
        .any(|provenance| matches!(provenance.source, PolicySource::RepairProposal { .. }))
    {
        errors.push(semantic_error("repair_proposal_provenance"));
    }
}

fn semantic_error(field: &'static str) -> ValidationDiagnostic {
    let field = gbf_foundation::FieldPath::from(field);
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::Schema,
        code: ValidationCode::ReportSemanticInvariantViolated {
            field: field.clone(),
        },
        detail: ValidationDetail::Field { field },
        provenance: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_foundation::{
        CompileProfileId, FieldPath, Hash256, KernelCalibrationId, LineageId,
        PlatformCalibrationId, RuntimeCalibrationId, TargetProfileId,
    };
    use gbf_hw::calibration::CalibrationSetRef;
    use gbf_policy::{
        CalibrationConfidenceClass, CalibrationConfidenceRequirement, CompileKnobId,
        CompileKnobOverrides, CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobValues,
        CompileKnobs, CompileObjective, CompilerFeature, ConstraintOperation, ConstraintProvenance,
        EffectiveConstraints, EvidenceRef, KnobLockSet, ObservabilityMode, ObservationKnob,
        OverlayKnob, OverlayPromotion, PlacementKnob, PlacementProfile, PolicyProvenance,
        PolicySource, ProbeCollectionLevel, RangeKnob, ReductionPlanCeiling, RepairPolicy,
        RepairPolicyProfile, ResolvedCompilePolicy, RiskPolicy, RomKernelDuplicationBias,
        RomKernelResidencyBias, RomWindowKnob, RuntimeMode, ScheduleKnob, ScheduleResourcePressure,
        ScheduleSliceCoarsening, ScheduleTileSearch, ServiceLevelObjective, SramKnob,
        SramPageAggression, StorageKnob, StorageMaterialization, TraceBudget, TraceDropPolicy,
        canonical_default_bounds_fixture,
    };
    use gbf_workload::{GoldenVectorId, WorkloadId};

    use super::*;
    use crate::{ReportEnvelope, canonicalize, round_trip_self_hash};

    #[test]
    fn f_b2_policy_resolution_v1_schema_accepts_canonical_fixture() {
        let report = report_fixture();
        let value = serde_json::to_value(&report).expect("report serializes");

        assert_eq!(value["schema"], serde_json::json!("policy_resolution.v1"));
        assert_eq!(value["schema_version"], serde_json::json!("1.0.0"));
        assert_eq!(value["outcome"], serde_json::json!("Passed"));
        assert!(value["result"].is_object());
        assert_eq!(
            value["result"]["provenance"],
            serde_json::json!({
                "target_defaults": hash_value(hash(6)),
                "profile_defaults": hash_value(hash(9)),
                "hint_bundle_hash": hash_value(hash(4)),
                "compile_request_hash": hash_value(hash(5)),
                "calibration_hash": hash_value(hash(7)),
            })
        );
        assert!(value["schema_version"].is_string());
        assert!(value.get("body").is_none());
        assert!(value["result"].get("schema_version").is_none());

        serde_json::from_value::<ReportEnvelope<PolicyResolutionReportBody>>(value)
            .expect("canonical policy_resolution.v1 fixture decodes");
        canonicalize(&report).expect("canonical fixture canonicalizes");
    }

    #[test]
    fn f_b2_policy_resolution_v1_rejects_missing_required_fields() {
        let mut value = serde_json::to_value(report_fixture()).expect("report serializes");
        value["result"]["provenance"]
            .as_object_mut()
            .expect("provenance object")
            .remove("hint_bundle_hash");

        assert!(
            serde_json::from_value::<ReportEnvelope<PolicyResolutionReportBody>>(value).is_err()
        );
    }

    #[test]
    fn f_b2_policy_resolution_v1_self_hash_round_trip() {
        let report = report_fixture();

        round_trip_self_hash(&report).expect("success report self hash round-trips");
    }

    #[test]
    fn f_b2_policy_resolution_v1_failure_report_self_hash_round_trip() {
        let report = failure_report_fixture();

        round_trip_self_hash(&report).expect("failure report self hash round-trips");
        assert!(report.body.result.is_none());
    }

    #[test]
    fn f_b2_policy_resolution_v1_result_is_none_iff_outcome_failed() {
        let mut passed_without_result = report_fixture().body;
        passed_without_result.result = None;
        assert!(
            passed_without_result
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );

        let mut failed_with_result = report_fixture().body;
        failed_with_result.diagnostics.push(hard_diagnostic());
        assert!(
            failed_with_result
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );

        assert!(
            failure_report_fixture()
                .body
                .validate_semantics(ReportOutcome::Failed)
                .is_ok()
        );
    }

    #[test]
    fn f_b2_policy_resolution_v1_calibration_set_ref_is_required() {
        let mut value = serde_json::to_value(report_fixture()).expect("report serializes");
        value["compile_request"]
            .as_object_mut()
            .expect("compile request object")
            .remove("calibration_set_ref");

        assert!(
            serde_json::from_value::<ReportEnvelope<PolicyResolutionReportBody>>(value).is_err()
        );
    }

    #[test]
    fn f_b2_policy_resolution_v1_rejects_repair_proposal_provenance() {
        let mut value = serde_json::to_value(report_fixture()).expect("report serializes");
        value["result"]["compile_knobs"]["provenance"][0]["chain"][0]["source"] = serde_json::json!({
            "kind": "RepairProposal",
            "id": "future-rp-1",
        });

        assert!(
            serde_json::from_value::<ReportEnvelope<PolicyResolutionReportBody>>(value).is_err()
        );
    }

    #[test]
    fn f_b2_policy_resolution_v1_rejects_authorized_relaxation_operation() {
        let mut value = serde_json::to_value(report_fixture()).expect("report serializes");
        value["result"]["compile_knobs"]["provenance"][0]["chain"][0]["operation"] =
            serde_json::json!({"kind": "AuthorizedRelaxation"});

        assert!(
            serde_json::from_value::<ReportEnvelope<PolicyResolutionReportBody>>(value).is_err()
        );
    }

    #[test]
    fn f_b2_policy_resolution_v1_hint_consumption_invariant() {
        let mut report = report_fixture();
        let preference = FieldPath::from("hint_bundle.preferences.placement");
        let provenance = vec![preference_provenance()];
        report
            .body
            .hint_consumption
            .preferences_honored
            .push(PreferenceUse {
                preference: preference.clone(),
                knob: CompileKnobId::Placement,
                provenance: provenance.clone(),
            });
        report
            .body
            .hint_consumption
            .preferences_ignored
            .push(IgnoredPreference {
                preference: preference.clone(),
                knob: CompileKnobId::Placement,
                reason: IgnoredPreferenceReason::Locked,
                provenance,
            });

        assert!(
            report
                .body
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );

        let report = report_fixture();
        let value = serde_json::to_value(&report).expect("report serializes");
        let hint_consumption = &value["hint_consumption"];

        assert_eq!(hint_consumption["facts_used"], serde_json::json!([]));
        assert_eq!(
            hint_consumption["preferences_honored"],
            serde_json::json!([])
        );
        assert_eq!(
            hint_consumption["preferences_ignored"],
            serde_json::json!([])
        );
        assert_eq!(
            hint_consumption["constraints_enforced"],
            serde_json::json!([])
        );

        serde_json::from_value::<IgnoredPreference>(serde_json::json!({
            "preference": "hint_bundle.preferences.expert_slot_affinity.0",
            "knob": {"kind": "Placement"},
            "reason": {"kind": "OutsideBounds"},
            "provenance": []
        }))
        .expect("typed ignored preference reason is accepted");
    }

    #[test]
    fn f_b2_policy_resolution_v1_rejects_soft_diagnostic() {
        let mut body = report_fixture().body;
        let mut diagnostic = hard_diagnostic();
        diagnostic.severity = DiagnosticSeverity::Soft;
        body.diagnostics.push(diagnostic);

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn f_b2_policy_resolution_v1_rejects_loosened_compile_knob_bounds() {
        let mut body = report_fixture().body;
        let result = body.result.as_mut().expect("success result");
        result
            .resolved
            .effective_constraints
            .target_caps
            .placement
            .max_profile = PlacementProfile::Budgeted;
        result.compile_knobs.bounds.placement.max_profile = PlacementProfile::PackedExperts;

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    fn report_fixture() -> ReportEnvelope<PolicyResolutionReportBody> {
        ReportEnvelope::new(
            ReportOutcome::Passed,
            PolicyResolutionReportBody {
                artifact_identity: ArtifactIdentitySection {
                    artifact_core_hash: hash(1),
                    artifact_manifest_hash: hash(2),
                    semantic_lineage: LineageId(hash(10)),
                    lowering_manifest_hash: hash(3),
                    hint_bundle_hash: hash(4),
                    workload_refs: vec![WorkloadId::from("workload")],
                    golden_vector_refs: vec![GoldenVectorId("golden".to_owned())],
                },
                compile_request: CompileRequestSection {
                    compile_request_hash: hash(5),
                    target: TargetProfileId::from("dmg-mbc5"),
                    target_profile_hash: hash(6),
                    profile: CompileProfileId::from("Bringup"),
                    objective: objective_fixture(),
                    required_features: BTreeSet::from([
                        CompilerFeature::ArtifactValidation,
                        CompilerFeature::PolicyResolution,
                    ]),
                    requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                    calibration_set_ref: CalibrationSetRef {
                        platform: Some(PlatformCalibrationId::from("platform-calibration")),
                        kernel: Some(KernelCalibrationId::from("kernel-calibration")),
                        runtime: Some(RuntimeCalibrationId::from("runtime-calibration")),
                    },
                    calibration_hash: hash(7),
                },
                result: Some(PolicyResolutionSuccessSection {
                    resolved: ResolvedSection::from(&policy_fixture()),
                    compile_knobs: CompileKnobsSection::from(&policy_fixture().knobs),
                    provenance: PolicyProvenanceSection::from_policy(
                        &policy_fixture().provenance,
                        hash(4),
                        hash(7),
                    ),
                }),
                hint_consumption: HintConsumptionSection::default(),
                diagnostics: Vec::new(),
            },
        )
        .expect("report envelope")
        .with_computed_self_hash()
        .expect("self hash computes")
    }

    fn failure_report_fixture() -> ReportEnvelope<PolicyResolutionReportBody> {
        let mut body = report_fixture().body;
        body.result = None;
        body.diagnostics = vec![hard_diagnostic()];

        ReportEnvelope::new(ReportOutcome::Failed, body)
            .expect("report envelope")
            .with_computed_self_hash()
            .expect("self hash computes")
    }

    fn policy_fixture() -> ResolvedCompilePolicy {
        let values = CompileKnobValues {
            placement: PlacementKnob {
                profile: PlacementProfile::StrictOnePerBank,
            },
            observation: ObservationKnob {
                observability: ObservabilityMode::Invariant,
                probe_level: ProbeCollectionLevel::Operational,
            },
            range: RangeKnob {
                reduction_ceiling: ReductionPlanCeiling::Conservative,
            },
            storage: StorageKnob {
                materialization: StorageMaterialization::RecomputePureValues,
            },
            sram: SramKnob {
                page_aggression: SramPageAggression::PackCold,
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
                resource_pressure: ScheduleResourcePressure::Balanced,
            },
        };

        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: objective_fixture(),
            effective_constraints: EffectiveConstraints {
                target_caps: canonical_default_bounds_fixture(),
                required_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 4,
                max_bytes_per_frame: 128,
                drop_policy: TraceDropPolicy::HaltAndFault,
            },
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            knobs: CompileKnobs {
                global: values,
                bounds: canonical_default_bounds_fixture(),
                locks: KnobLockSet::default(),
                overrides: CompileKnobOverrides::default(),
                provenance: vec![CompileKnobProvenanceEntry {
                    path: CompileKnobPath {
                        knob: CompileKnobId::Placement,
                        selector: None,
                        field: Some(FieldPath::from("global.profile")),
                    },
                    chain: vec![ConstraintProvenance {
                        source: PolicySource::ProfileDefault,
                        operation: ConstraintOperation::SeedDefault,
                        evidence: vec![EvidenceRef {
                            kind: "ProfileFile".to_owned(),
                            reference: "Bringup.toml".to_owned(),
                            hash: Some(hash(8)),
                        }],
                    }],
                }],
            },
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            provenance: PolicyProvenance {
                target_defaults: hash(6),
                profile_defaults: hash(9),
                hint_bundle_hash: Some(hash(4)),
                compile_request_hash: hash(5),
                calibration_hash: Some(hash(7)),
            },
        }
    }

    fn objective_fixture() -> CompileObjective {
        CompileObjective {
            service: Some(ServiceLevelObjective {
                max_first_token_cycles_p95: Some(3_000),
                max_checkpoint_gap_cycles_p95: None,
                max_resume_latency_cycles_p95: Some(1_000),
                max_ui_jitter_frames_p99: Some(1),
            }),
            max_cycles_per_token: Some(8_000),
            max_bank_switches_per_token: Some(5),
            max_sram_page_switches_per_token: Some(1),
            min_ui_headroom_pct: 9,
            max_rom_bytes: Some(512 * 1024),
            risk: RiskPolicy {
                cycle_quantile: 95,
                switch_quantile: 99,
                calibration_confidence_requirement: CalibrationConfidenceRequirement::AtLeast {
                    class: CalibrationConfidenceClass::Weak,
                },
                fallback_profile: None,
                fallback_runtime_mode: Some(RuntimeMode::Safe),
            },
        }
    }

    fn hard_diagnostic() -> ValidationDiagnosticRecord {
        ValidationDiagnosticRecord {
            severity: DiagnosticSeverity::Hard,
            origin: ValidationOrigin::PolicyResolution,
            code: ValidationCode::ReportSemanticInvariantViolated {
                field: FieldPath::from("policy_resolution.fixture"),
            },
            detail: ValidationDetail::Field {
                field: FieldPath::from("policy_resolution.fixture"),
            },
            provenance: vec![EvidenceRef {
                kind: "fixture".to_owned(),
                reference: "policy_resolution".to_owned(),
                hash: Some(hash(0xfe)),
            }],
        }
    }

    fn preference_provenance() -> ConstraintProvenance {
        ConstraintProvenance {
            source: PolicySource::HintBundle,
            operation: ConstraintOperation::ApplyPreference,
            evidence: vec![EvidenceRef {
                kind: "HintBundle".to_owned(),
                reference: "preferences".to_owned(),
                hash: Some(hash(0xfd)),
            }],
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn hash_value(hash: Hash256) -> serde_json::Value {
        serde_json::to_value(hash).expect("hash serializes")
    }
}
