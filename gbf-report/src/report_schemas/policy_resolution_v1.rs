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
