//! `artifact_validation.v1` Stage 0 report schema.

use std::collections::BTreeSet;

use gbf_artifact::{ArtifactFeature, GoldenVectorId};
use gbf_foundation::{Hash256, SemVer, WorkloadId};
use gbf_policy::{CompilerFeature, EvidenceRef, RuntimeMode};
use serde::{Deserialize, Serialize};

use crate::report_envelope::{ReportBody, ReportOutcome};

pub const SCHEMA_ID: &str = "artifact_validation.v1";
pub const SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactValidationReportBody {
    pub identity: ArtifactValidationIdentitySection,
    pub compatibility: ArtifactCompatibilitySection,
    pub checked_inputs: ArtifactValidationInputSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactValidationIdentitySection {
    pub artifact_source_hash: Option<Hash256>,
    pub artifact_effective_core_hash: Option<Hash256>,
    pub artifact_manifest_hash: Option<Hash256>,
    pub semantic_core_hash: Option<Hash256>,
    pub artifact_aux_hash: Option<Hash256>,
    pub lowering_manifest_hash: Option<Hash256>,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub compile_profile_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
    pub compatibility_adapter_hash: Option<Hash256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCompatibilitySection {
    pub decision: Option<ArtifactCompatibilityDecision>,
    pub failures: Vec<ArtifactCompatibilityFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactValidationInputSection {
    pub workload_refs: Vec<WorkloadId>,
    pub golden_vector_refs: Vec<GoldenVectorId>,
    pub required_artifact_features: BTreeSet<ArtifactFeature>,
    pub required_compiler_features: BTreeSet<CompilerFeature>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArtifactCompatibilityDecision {
    CurrentSchema,
    LosslessInMemoryUpgrade {
        from_schema: SemVer,
        to_schema: SemVer,
        adapter: CompatibilityAdapterId,
        adapter_hash: Hash256,
        before_semantic_core_hash: Hash256,
        after_semantic_core_hash: Hash256,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArtifactCompatibilityFailure {
    UnsupportedEpoch { observed: SemVer, supported: SemVer },
    AdapterMissing { observed: SemVer, target: SemVer },
    AdapterNotLossless { adapter: CompatibilityAdapterId },
    SemanticHashChanged { before: Hash256, after: Hash256 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompatibilityAdapterId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationDiagnosticRecord {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DiagnosticSeverity {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValidationOrigin {
    Schema,
    SemanticCore,
    Manifest,
    Lowering,
    Calibration,
    HintBundle,
    Workload,
    GoldenVector,
    CompileRequest,
    PolicyResolution,
    Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValidationCode {
    SchemaEpochUnsupported,
    SchemaCompatibilityAdapterMissing { observed: SemVer, target: SemVer },
    SchemaCompatibilityAdapterNotLossless { adapter: CompatibilityAdapterId },
    SchemaCompatibilitySemanticHashChanged { before: Hash256, after: Hash256 },
    ArtifactValidationInvariant { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValidationDetail {
    None,
    HashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    Field {
        field: String,
    },
    Text {
        value: String,
    },
}

impl ReportBody for ArtifactValidationReportBody {
    type Diagnostic = ValidationDiagnosticRecord;

    const SCHEMA_ID: &'static str = SCHEMA_ID;
    const SCHEMA_VERSION: SemVer = SCHEMA_VERSION;

    fn validate_semantics(
        &self,
        outcome: ReportOutcome,
    ) -> Result<(), Vec<ValidationDiagnosticRecord>> {
        let mut errors = Vec::new();
        let has_hard = self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Hard);

        for diagnostic in &self.diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Soft {
                errors.push(semantic_error(
                    "soft_diagnostic",
                    "F-B2 artifact validation reports reject Soft diagnostics",
                ));
            }
            if diagnostic.provenance.is_empty() {
                errors.push(semantic_error(
                    "diagnostic_provenance",
                    "diagnostics must carry at least one typed provenance reference",
                ));
            }
        }

        match outcome {
            ReportOutcome::Passed => {
                if has_hard {
                    errors.push(semantic_error(
                        "passed_with_hard_diagnostic",
                        "Passed reports must not contain Hard diagnostics",
                    ));
                }
                require_passed_identity_hashes(&self.identity, &mut errors);
            }
            ReportOutcome::Failed => {
                if !has_hard {
                    errors.push(semantic_error(
                        "failed_without_hard_diagnostic",
                        "new Failed reports must contain at least one Hard diagnostic",
                    ));
                }
            }
        }

        if let Some(ArtifactCompatibilityDecision::LosslessInMemoryUpgrade {
            before_semantic_core_hash,
            after_semantic_core_hash,
            ..
        }) = self.compatibility.decision
            && before_semantic_core_hash != after_semantic_core_hash
        {
            errors.push(semantic_error(
                "lossless_upgrade_semantic_hash_changed",
                "LosslessInMemoryUpgrade requires identical before/after semantic core hashes",
            ));
        }

        if !is_sorted(&self.checked_inputs.workload_refs) {
            errors.push(semantic_error(
                "workload_refs_sorted",
                "workload_refs must be sorted",
            ));
        }
        if !is_sorted(&self.checked_inputs.golden_vector_refs) {
            errors.push(semantic_error(
                "golden_vector_refs_sorted",
                "golden_vector_refs must be sorted",
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn require_passed_identity_hashes(
    identity: &ArtifactValidationIdentitySection,
    errors: &mut Vec<ValidationDiagnosticRecord>,
) {
    let required = [
        (
            "artifact_source_hash",
            identity.artifact_source_hash.is_some(),
        ),
        (
            "artifact_effective_core_hash",
            identity.artifact_effective_core_hash.is_some(),
        ),
        (
            "artifact_manifest_hash",
            identity.artifact_manifest_hash.is_some(),
        ),
        ("semantic_core_hash", identity.semantic_core_hash.is_some()),
        ("artifact_aux_hash", identity.artifact_aux_hash.is_some()),
        (
            "lowering_manifest_hash",
            identity.lowering_manifest_hash.is_some(),
        ),
        ("calibration_hash", identity.calibration_hash.is_some()),
    ];

    for (field, present) in required {
        if !present {
            errors.push(semantic_error(
                "passed_identity_hash_present",
                format!("Passed reports require identity.{field}"),
            ));
        }
    }
}

fn semantic_error(name: &'static str, value: impl Into<String>) -> ValidationDiagnosticRecord {
    ValidationDiagnosticRecord {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::Schema,
        code: ValidationCode::ArtifactValidationInvariant {
            name: name.to_owned(),
        },
        detail: ValidationDetail::Text {
            value: value.into(),
        },
        provenance: vec![EvidenceRef {
            kind: "semantic_validator".to_owned(),
            reference: name.to_owned(),
            hash: None,
        }],
    }
}

fn is_sorted<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] <= pair[1])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_artifact::{ArtifactFeature, HintBundle};
    use gbf_foundation::{Hash256, WorkloadId};
    use gbf_policy::{CompilerFeature, EvidenceRef, RuntimeMode};

    use super::*;
    use crate::{ReportEnvelope, compute_self_hash, round_trip_self_hash};

    #[test]
    fn f_b2_artifact_validation_v1_self_hash_round_trip() {
        let body = passing_body();
        let mut env = ReportEnvelope::new(ReportOutcome::Passed, body);
        env.report_self_hash = compute_self_hash(&env).expect("self hash");

        assert!(round_trip_self_hash(&env).is_ok());
        assert!(env.validate_semantics().is_ok());
    }

    #[test]
    fn f_b2_artifact_validation_v1_rejects_unknown_fields() {
        let mut env = ReportEnvelope::new(ReportOutcome::Passed, passing_body());
        env.report_self_hash = compute_self_hash(&env).expect("self hash");
        let mut value = serde_json::to_value(&env).expect("json");
        value
            .as_object_mut()
            .expect("object")
            .insert("surprise".to_owned(), serde_json::json!(true));

        assert!(
            serde_json::from_value::<ReportEnvelope<ArtifactValidationReportBody>>(value).is_err()
        );
    }

    #[test]
    fn f_b2_artifact_validation_v1_outcome_hard_diagnostic_invariant() {
        let mut failed_without_hard = passing_body();
        failed_without_hard.diagnostics.clear();
        assert!(
            failed_without_hard
                .validate_semantics(ReportOutcome::Failed)
                .is_err()
        );

        let mut passed_with_hard = passing_body();
        passed_with_hard.diagnostics.push(hard_diagnostic());
        assert!(
            passed_with_hard
                .validate_semantics(ReportOutcome::Passed)
                .is_err()
        );
    }

    #[test]
    fn f_b2_artifact_validation_v1_lossless_upgrade_preserves_semantic_hash() {
        let mut body = passing_body();
        body.compatibility.decision =
            Some(ArtifactCompatibilityDecision::LosslessInMemoryUpgrade {
                from_schema: SemVer::new(1, 0, 0),
                to_schema: SemVer::new(1, 1, 0),
                adapter: CompatibilityAdapterId("adapter.lossless".to_owned()),
                adapter_hash: hash(0x40),
                before_semantic_core_hash: hash(0x41),
                after_semantic_core_hash: hash(0x42),
            });

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn f_b2_artifact_validation_v1_passing_outcome_implies_all_required_hashes() {
        let mut body = passing_body();
        body.identity.calibration_hash = None;

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn f_b2_artifact_validation_v1_rejects_soft_diagnostic() {
        let mut body = passing_body();
        let mut diagnostic = hard_diagnostic();
        diagnostic.severity = DiagnosticSeverity::Soft;
        body.diagnostics.push(diagnostic);

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    #[test]
    fn f_b2_artifact_validation_v1_no_hints_uses_empty_bundle_hash() {
        let body = passing_body();

        assert_eq!(
            body.identity.hint_bundle_hash,
            HintBundle::empty().compute_canonical_hash()
        );
    }

    #[test]
    fn f_b2_artifact_validation_v1_rejects_unsorted_refs() {
        let mut body = passing_body();
        body.checked_inputs.workload_refs = vec![
            WorkloadId::from("workload.b"),
            WorkloadId::from("workload.a"),
        ];

        assert!(body.validate_semantics(ReportOutcome::Passed).is_err());
    }

    fn passing_body() -> ArtifactValidationReportBody {
        ArtifactValidationReportBody {
            identity: ArtifactValidationIdentitySection {
                artifact_source_hash: Some(hash(0x01)),
                artifact_effective_core_hash: Some(hash(0x02)),
                artifact_manifest_hash: Some(hash(0x03)),
                semantic_core_hash: Some(hash(0x04)),
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
                workload_refs: vec![WorkloadId::from("workload.a")],
                golden_vector_refs: vec![GoldenVectorId("golden.a".to_owned())],
                required_artifact_features: BTreeSet::from([ArtifactFeature::DenseI8]),
                required_compiler_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
            },
            diagnostics: Vec::new(),
        }
    }

    fn hard_diagnostic() -> ValidationDiagnosticRecord {
        ValidationDiagnosticRecord {
            severity: DiagnosticSeverity::Hard,
            origin: ValidationOrigin::Schema,
            code: ValidationCode::SchemaEpochUnsupported,
            detail: ValidationDetail::None,
            provenance: vec![EvidenceRef {
                kind: "fixture".to_owned(),
                reference: "artifact".to_owned(),
                hash: Some(hash(0xaa)),
            }],
        }
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
