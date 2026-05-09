//! `policy_resolution.v1` Stage 0.5 report schema.

use std::collections::BTreeSet;

use gbf_foundation::{
    CompileProfileId, GoldenVectorId, Hash256, LineageId, TargetProfileId, WorkloadId,
};
use gbf_hw::calibration::CalibrationSetRef;
use gbf_policy::{
    CompileKnobId, CompileKnobOverrides, CompileKnobProvenanceEntry, CompileKnobValues,
    CompileKnobs, CompileObjective, CompilerFeature, ConstraintProvenance, DiagnosticSeverity,
    EffectiveConstraints, KnobLockSet, ObservabilityMode, PolicyProvenance, PolicySource,
    RepairPolicy, ResolvedCompilePolicy, RuntimeMode, TraceBudget, ValidationCode,
    ValidationDetail, ValidationDiagnostic, ValidationOrigin,
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
    pub bounds: gbf_policy::CompileKnobBounds,
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
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferenceUse {
    pub knob: CompileKnobId,
    pub provenance: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IgnoredPreference {
    pub knob: CompileKnobId,
    pub reason: String,
    pub provenance: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintEnforcement {
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
                .all(|pair| pair[0].path <= pair[1].path)
            {
                errors.push(semantic_error("compile_knobs.provenance"));
            }

            for entry in &result.compile_knobs.provenance {
                if entry.chain.is_empty() {
                    errors.push(semantic_error("compile_knobs.provenance.chain"));
                }
                if entry.chain.iter().any(|provenance| {
                    matches!(provenance.source, PolicySource::RepairProposal { .. })
                }) {
                    errors.push(semantic_error("repair_proposal_provenance"));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
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
    use crate::ReportEnvelope;

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

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
