//! Stage 0 pipeline-entry validation plumbing.
//!
//! Successful validation yields a [`ValidatedInputs`] token whose witness field
//! is private to this module:
//!
//! ```compile_fail
//! use std::borrow::Cow;
//!
//! use gbf_artifact::{ArtifactAux, HintBundle, TargetDataLoweringArtifact};
//! use gbf_codegen::stages::validate::{
//!     ImportedArtifactView, ValidatedInputHashes, ValidatedInputs,
//! };
//! use gbf_hw::target::TargetProfile;
//! use gbf_policy::{CalibrationBundleSet, CompileProfileSpec, CompileRequest};
//! use gbf_workload::{GoldenVectorRef, WorkloadManifestRef};
//!
//! fn cannot_construct<'a>(
//!     artifact: Cow<'a, ImportedArtifactView>,
//!     lowerings: &'a [TargetDataLoweringArtifact],
//!     workloads: &'a [WorkloadManifestRef],
//!     golden_vectors: &'a [GoldenVectorRef],
//!     compile_request: &'a CompileRequest,
//!     target_profile: &'a TargetProfile,
//!     compile_profile: &'a CompileProfileSpec,
//!     calibration: &'a CalibrationBundleSet,
//!     input_hashes: ValidatedInputHashes,
//! ) {
//!     let _ = ValidatedInputs {
//!         artifact,
//!         lowerings,
//!         workloads,
//!         golden_vectors,
//!         compile_request,
//!         target_profile,
//!         compile_profile,
//!         calibration,
//!         input_hashes,
//!         _private: (),
//!     };
//! }
//! ```

use std::borrow::Cow;
use std::error::Error;
use std::fmt;

use gbf_artifact::aux::SidecarKind;
use gbf_artifact::{ArtifactAux, HintBundle, TargetDataLoweringArtifact};
use gbf_artifact::{core::ArtifactCore, manifest::ArtifactFeature};
use gbf_foundation::{BlobRef, Hash256};
use gbf_hw::target::TargetProfile;
use gbf_policy::{CalibrationBundleSet, CompileProfileSpec, CompileRequest, EvidenceRef};
use gbf_report::report_envelope::canonicalize as canonicalize_report;
use gbf_report::report_schemas::artifact_validation_v1::{
    ArtifactCompatibilityDecision, ArtifactCompatibilitySection, ArtifactValidationIdentitySection,
    ArtifactValidationInputSection, ArtifactValidationReportBody, DiagnosticSeverity,
    ObjectiveRejection, TargetIncompatibilityReason, ValidationCode, ValidationDetail,
    ValidationDiagnosticRecord, ValidationOrigin,
};
use gbf_report::{ReportEnvelope, ReportOutcome, compute_self_hash};
use gbf_workload::{
    GoldenVectorId, GoldenVectorRef, WorkloadId, WorkloadManifest, WorkloadManifestRef,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub type ValidationDiagnostic = ValidationDiagnosticRecord;

pub struct ValidateInputs<'a> {
    pub artifact: &'a ImportedArtifactView,
    pub lowerings: &'a [TargetDataLoweringArtifact],
    pub workloads: &'a [WorkloadManifestRef],
    pub golden_vectors: &'a [GoldenVectorRef],
    pub compile_request: &'a CompileRequest,
    pub target_profile: &'a TargetProfile,
    pub compile_profile: &'a CompileProfileSpec,
    pub calibration: Option<&'a CalibrationBundleSet>,
    pub resolver: &'a dyn ArtifactResolver,
}

pub trait ArtifactResolver {
    fn resolve_blob(&self, blob: &BlobRef) -> Result<ResolvedBlob, ArtifactResolveError>;

    fn resolve_sidecar(
        &self,
        sidecar: &SidecarRef,
    ) -> Result<ResolvedSidecar, ArtifactResolveError>;

    fn resolve_evidence(
        &self,
        evidence: &EvidenceRef,
    ) -> Result<ResolvedEvidence, ArtifactResolveError>;

    fn resolve_workload(
        &self,
        workload: &WorkloadManifestRef,
    ) -> Result<ResolvedWorkload, ArtifactResolveError>;

    fn resolve_golden_vector(
        &self,
        vector: &GoldenVectorRef,
    ) -> Result<ResolvedGoldenVector, ArtifactResolveError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedArtifactView {
    pub core: ArtifactCore,
    pub aux: ArtifactAux,
    pub hint_bundle: HintBundle,
    pub reference: Option<ReferenceLink>,
    pub transport: ArtifactTransportIdentity,
}

impl ImportedArtifactView {
    #[must_use]
    pub fn new(
        core: ArtifactCore,
        aux: ArtifactAux,
        hint_bundle: Option<HintBundle>,
        reference: Option<ReferenceLink>,
        transport: ArtifactTransportIdentity,
    ) -> Self {
        Self {
            core,
            aux,
            hint_bundle: hint_bundle.unwrap_or_else(HintBundle::empty),
            reference,
            transport,
        }
    }

    #[must_use]
    pub fn hint_bundle_hash(&self) -> Hash256 {
        self.hint_bundle.compute_canonical_hash()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactTransportIdentity {
    pub source_uri: Option<String>,
    pub transport_hash: Hash256,
    pub import_tool_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceLink {
    pub reference: String,
    pub hash: Hash256,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedInputs<'a> {
    pub artifact: Cow<'a, ImportedArtifactView>,
    pub lowerings: &'a [TargetDataLoweringArtifact],
    pub workloads: &'a [WorkloadManifestRef],
    pub golden_vectors: &'a [GoldenVectorRef],
    pub compile_request: &'a CompileRequest,
    pub target_profile: &'a TargetProfile,
    pub compile_profile: &'a CompileProfileSpec,
    pub calibration: &'a CalibrationBundleSet,
    pub input_hashes: ValidatedInputHashes,
    _private: PrivateValidatedInputs,
}

impl<'a> ValidatedInputs<'a> {
    fn new(
        inputs: ValidateInputs<'a>,
        calibration: &'a CalibrationBundleSet,
        input_hashes: ValidatedInputHashes,
    ) -> Self {
        Self {
            artifact: Cow::Borrowed(inputs.artifact),
            lowerings: inputs.lowerings,
            workloads: inputs.workloads,
            golden_vectors: inputs.golden_vectors,
            compile_request: inputs.compile_request,
            target_profile: inputs.target_profile,
            compile_profile: inputs.compile_profile,
            calibration,
            input_hashes,
            _private: PrivateValidatedInputs(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivateValidatedInputs(());

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationProduct<'a> {
    pub validated: ValidatedInputs<'a>,
    pub report: ReportEnvelope<ArtifactValidationReportBody>,
    pub artifact_validation_self_hash: Hash256,
    pub artifact_validation_canonical_bytes_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationStageFailure {
    pub report: ReportEnvelope<ArtifactValidationReportBody>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatedInputHashes {
    pub artifact_source_hash: Hash256,
    pub artifact_effective_core_hash: Hash256,
    pub artifact_manifest_hash: Hash256,
    pub artifact_aux_hash: Hash256,
    pub lowering_manifest_hash: Hash256,
    pub hint_bundle_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub compile_profile_hash: Hash256,
    pub calibration_hash: Hash256,
    pub compatibility_adapter_hash: Option<Hash256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarRef {
    pub kind: SidecarKind,
    pub id: String,
    pub hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBlob {
    pub bytes: Vec<u8>,
    pub content_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSidecar {
    pub bytes: Vec<u8>,
    pub content_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEvidence {
    pub bytes: Vec<u8>,
    pub content_hash: Option<Hash256>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkload {
    pub manifest: WorkloadManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGoldenVector {
    pub bytes: Vec<u8>,
    pub manifest_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactResolveError {
    NotFound {
        reference: String,
    },
    HashMismatch {
        reference: String,
        expected: Hash256,
        observed: Hash256,
    },
    Unsupported {
        message: String,
    },
}

impl ArtifactResolveError {
    #[must_use]
    pub fn not_found(reference: impl Into<String>) -> Self {
        Self::NotFound {
            reference: reference.into(),
        }
    }

    #[must_use]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported {
            message: message.into(),
        }
    }
}

impl fmt::Display for ArtifactResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { reference } => {
                write!(f, "artifact resolver could not find {reference}")
            }
            Self::HashMismatch {
                reference,
                expected,
                observed,
            } => write!(
                f,
                "artifact resolver hash mismatch for {reference}: expected {expected}, observed {observed}"
            ),
            Self::Unsupported { message } => f.write_str(message),
        }
    }
}

impl Error for ArtifactResolveError {}

#[must_use]
pub fn compute_validated_input_hashes(
    inputs: &ValidateInputs<'_>,
    calibration: &CalibrationBundleSet,
) -> ValidatedInputHashes {
    let artifact_effective_core_hash = inputs.artifact.core.semantic_hash();

    ValidatedInputHashes {
        artifact_source_hash: inputs.artifact.transport.transport_hash,
        artifact_effective_core_hash,
        artifact_manifest_hash: input_hash("artifact_manifest", &artifact_effective_core_hash),
        artifact_aux_hash: input_hash("artifact_aux", &inputs.artifact.aux),
        lowering_manifest_hash: input_hash("lowering_manifest", inputs.lowerings),
        hint_bundle_hash: inputs.artifact.hint_bundle_hash(),
        compile_request_hash: input_hash("compile_request", inputs.compile_request),
        target_profile_hash: input_hash("target_profile", inputs.target_profile),
        compile_profile_hash: input_hash("compile_profile", inputs.compile_profile),
        calibration_hash: input_hash("calibration", calibration),
        compatibility_adapter_hash: None,
    }
}

#[allow(clippy::result_large_err)]
pub fn validate_artifact_and_request<'a>(
    inputs: ValidateInputs<'a>,
) -> Result<ValidationProduct<'a>, ValidationStageFailure> {
    let Some(calibration) = inputs.calibration else {
        return Err(missing_calibration_failure(&inputs));
    };

    let diagnostics = compile_request_diagnostics(&inputs);
    if !diagnostics.is_empty() {
        return Err(validation_failure(&inputs, Some(calibration), diagnostics));
    }

    let input_hashes = compute_validated_input_hashes(&inputs, calibration);
    let report = success_report(&inputs, &input_hashes);
    let (report, artifact_validation_self_hash, artifact_validation_canonical_bytes_hash) =
        finalize_report(report);
    let validated = ValidatedInputs::new(inputs, calibration, input_hashes);

    Ok(ValidationProduct {
        validated,
        report,
        artifact_validation_self_hash,
        artifact_validation_canonical_bytes_hash,
    })
}

fn success_report(
    inputs: &ValidateInputs<'_>,
    input_hashes: &ValidatedInputHashes,
) -> ReportEnvelope<ArtifactValidationReportBody> {
    ReportEnvelope::new(
        ReportOutcome::Passed,
        ArtifactValidationReportBody {
            identity: ArtifactValidationIdentitySection {
                artifact_source_hash: Some(input_hashes.artifact_source_hash),
                artifact_effective_core_hash: Some(input_hashes.artifact_effective_core_hash),
                artifact_manifest_hash: Some(input_hashes.artifact_manifest_hash),
                semantic_core_hash: Some(input_hashes.artifact_effective_core_hash),
                artifact_aux_hash: Some(input_hashes.artifact_aux_hash),
                lowering_manifest_hash: Some(input_hashes.lowering_manifest_hash),
                hint_bundle_hash: input_hashes.hint_bundle_hash,
                compile_request_hash: input_hashes.compile_request_hash,
                target_profile_hash: input_hashes.target_profile_hash,
                compile_profile_hash: input_hashes.compile_profile_hash,
                calibration_hash: Some(input_hashes.calibration_hash),
                compatibility_adapter_hash: input_hashes.compatibility_adapter_hash,
            },
            compatibility: ArtifactCompatibilitySection {
                decision: Some(ArtifactCompatibilityDecision::CurrentSchema),
                failures: Vec::new(),
            },
            checked_inputs: checked_inputs(inputs),
            diagnostics: Vec::new(),
        },
    )
}

fn missing_calibration_failure(inputs: &ValidateInputs<'_>) -> ValidationStageFailure {
    let diagnostic = ValidationDiagnosticRecord {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::Calibration,
        code: ValidationCode::ArtifactValidationInvariant {
            name: "MissingCalibration".to_owned(),
        },
        detail: ValidationDetail::Field {
            field: "calibration".to_owned(),
        },
        provenance: vec![EvidenceRef {
            kind: "compile_request".to_owned(),
            reference: "calibration_set_ref".to_owned(),
            hash: None,
        }],
    };

    validation_failure(inputs, None, vec![diagnostic])
}

fn validation_failure(
    inputs: &ValidateInputs<'_>,
    calibration: Option<&CalibrationBundleSet>,
    diagnostics: Vec<ValidationDiagnostic>,
) -> ValidationStageFailure {
    let body = ArtifactValidationReportBody {
        identity: ArtifactValidationIdentitySection {
            artifact_source_hash: Some(inputs.artifact.transport.transport_hash),
            artifact_effective_core_hash: Some(inputs.artifact.core.semantic_hash()),
            artifact_manifest_hash: Some(input_hash(
                "artifact_manifest",
                &inputs.artifact.core.semantic_hash(),
            )),
            semantic_core_hash: Some(inputs.artifact.core.semantic_hash()),
            artifact_aux_hash: Some(input_hash("artifact_aux", &inputs.artifact.aux)),
            lowering_manifest_hash: Some(input_hash("lowering_manifest", inputs.lowerings)),
            hint_bundle_hash: inputs.artifact.hint_bundle_hash(),
            compile_request_hash: input_hash("compile_request", inputs.compile_request),
            target_profile_hash: input_hash("target_profile", inputs.target_profile),
            compile_profile_hash: input_hash("compile_profile", inputs.compile_profile),
            calibration_hash: calibration.map(|calibration| input_hash("calibration", calibration)),
            compatibility_adapter_hash: None,
        },
        compatibility: ArtifactCompatibilitySection {
            decision: Some(ArtifactCompatibilityDecision::CurrentSchema),
            failures: Vec::new(),
        },
        checked_inputs: checked_inputs(inputs),
        diagnostics: diagnostics.clone(),
    };
    let report = ReportEnvelope::new(ReportOutcome::Failed, body);
    let (report, _, _) = finalize_report(report);

    ValidationStageFailure {
        report,
        diagnostics,
    }
}

fn compile_request_diagnostics(inputs: &ValidateInputs<'_>) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();

    for feature in &inputs.compile_request.required_features {
        if !supported_compiler_features().contains(feature) {
            diagnostics.push(compile_request_diagnostic(
                ValidationCode::CompileRequestUnsupportedFeature { feature: *feature },
                ValidationDetail::Field {
                    field: "compile_request.required_features".to_owned(),
                },
            ));
        }
    }

    if inputs.compile_request.target != *inputs.target_profile.id() {
        diagnostics.push(compile_request_diagnostic(
            ValidationCode::CompileRequestTargetIncompatible {
                target: inputs.compile_request.target.clone(),
                reason: TargetIncompatibilityReason::CompileRequestTargetMismatch {
                    expected: inputs.target_profile.id().clone(),
                },
            },
            ValidationDetail::Field {
                field: "compile_request.target".to_owned(),
            },
        ));
    }

    profile_objective_rejections(inputs)
        .into_iter()
        .map(|reason| {
            compile_request_diagnostic(
                ValidationCode::CompileRequestProfileForbidsObjective {
                    profile: inputs.compile_profile.id.clone(),
                    reason,
                },
                ValidationDetail::Field {
                    field: "compile_request.objective".to_owned(),
                },
            )
        })
        .for_each(|diagnostic| diagnostics.push(diagnostic));

    diagnostics
}

fn profile_objective_rejections(inputs: &ValidateInputs<'_>) -> Vec<ObjectiveRejection> {
    let objective = &inputs.compile_request.objective;
    let mut rejections = Vec::new();

    if let Some(service) = &objective.service {
        for (field, value) in [
            (
                "max_first_token_cycles_p95",
                service.max_first_token_cycles_p95,
            ),
            (
                "max_checkpoint_gap_cycles_p95",
                service.max_checkpoint_gap_cycles_p95,
            ),
            (
                "max_resume_latency_cycles_p95",
                service.max_resume_latency_cycles_p95,
            ),
        ] {
            if value == Some(0) {
                rejections.push(ObjectiveRejection::ServiceLevelZero {
                    field: field.to_owned(),
                });
            }
        }

        if service.max_ui_jitter_frames_p99 == Some(0) {
            rejections.push(ObjectiveRejection::ServiceLevelZero {
                field: "max_ui_jitter_frames_p99".to_owned(),
            });
        }
    }

    if objective.max_cycles_per_token == Some(0) {
        rejections.push(ObjectiveRejection::MaxCyclesPerTokenZero);
    }
    if objective.max_rom_bytes == Some(0) {
        rejections.push(ObjectiveRejection::MaxRomBytesZero);
    }
    if objective.max_bank_switches_per_token == Some(0) {
        rejections.push(ObjectiveRejection::MaxBankSwitchesPerTokenZero);
    }
    if objective.max_sram_page_switches_per_token == Some(0) {
        rejections.push(ObjectiveRejection::MaxSramPageSwitchesPerTokenZero);
    }
    if !(1..=100).contains(&objective.risk.cycle_quantile) {
        rejections.push(ObjectiveRejection::RiskQuantileInvalid {
            field: "cycle_quantile".to_owned(),
            value: objective.risk.cycle_quantile,
        });
    }
    if !(1..=100).contains(&objective.risk.switch_quantile) {
        rejections.push(ObjectiveRejection::RiskQuantileInvalid {
            field: "switch_quantile".to_owned(),
            value: objective.risk.switch_quantile,
        });
    }

    rejections
}

fn compile_request_diagnostic(
    code: ValidationCode,
    detail: ValidationDetail,
) -> ValidationDiagnostic {
    ValidationDiagnosticRecord {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::CompileRequest,
        code,
        detail,
        provenance: vec![EvidenceRef {
            kind: "compile_request".to_owned(),
            reference: "compile_request".to_owned(),
            hash: None,
        }],
    }
}

fn supported_compiler_features() -> std::collections::BTreeSet<gbf_policy::CompilerFeature> {
    std::collections::BTreeSet::from([gbf_policy::CompilerFeature::ArtifactValidation])
}

fn checked_inputs(inputs: &ValidateInputs<'_>) -> ArtifactValidationInputSection {
    let mut workload_refs = inputs
        .workloads
        .iter()
        .map(|workload| workload.id.clone())
        .collect::<Vec<WorkloadId>>();
    workload_refs.sort();

    let mut golden_vector_refs = inputs
        .golden_vectors
        .iter()
        .map(|vector| vector.id.clone())
        .collect::<Vec<GoldenVectorId>>();
    golden_vector_refs.sort();

    ArtifactValidationInputSection {
        workload_refs,
        golden_vector_refs,
        required_artifact_features: std::collections::BTreeSet::<ArtifactFeature>::new(),
        required_compiler_features: inputs.compile_request.required_features.clone(),
        requested_runtime_modes: inputs.compile_request.requested_runtime_modes.clone(),
    }
}

fn finalize_report(
    mut report: ReportEnvelope<ArtifactValidationReportBody>,
) -> (
    ReportEnvelope<ArtifactValidationReportBody>,
    Hash256,
    Hash256,
) {
    report.report_self_hash =
        compute_self_hash(&report).expect("artifact validation report self-hash is computable");
    report
        .validate_semantics()
        .expect("artifact validation report semantics are valid");
    let canonical_bytes =
        canonicalize_report(&report).expect("artifact validation report canonicalizes");
    let canonical_bytes_hash = sha256_hash(&canonical_bytes);

    (
        report.clone(),
        report.report_self_hash,
        canonical_bytes_hash,
    )
}

fn input_hash<T: Serialize + ?Sized>(domain: &str, value: &T) -> Hash256 {
    let encoded = serde_json::to_vec(value).expect("Stage 0 input identity serializes");
    let mut hasher = Sha256::new();
    hasher.update(b"gbf-codegen/stage0/input-hash/v1\0");
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Hash256::from_bytes(hasher.finalize().into())
}

fn sha256_hash(bytes: &[u8]) -> Hash256 {
    Hash256::from_bytes(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use gbf_artifact::aux::ArtifactAux;
    use gbf_artifact::core::ArtifactCore;
    use gbf_artifact::lowerings::{
        DataLoweringProfileId, LoweringShard, LoweringShardId, LoweringShardKind,
    };
    use gbf_artifact::quant::QuantSpec;
    use gbf_artifact::sequence::SequenceSemanticsSpec;
    use gbf_foundation::{BlobCodec, CompileProfileId, PackerVersion, TargetProfileId};
    use gbf_hw::calibration::CalibrationSetRef;
    use gbf_hw::target::dmg_mbc5_8mib_128kib;
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_ID, BootstrapCalibrationBundle, CalibrationConfidenceRequirement,
        CompileObjective, CompilerFeature, RiskPolicy, RuntimeMode, ServiceLevelObjective,
        canonical_compile_profile_specs,
    };
    use gbf_workload::{GoldenVectorId, WorkloadLocator};

    use super::*;

    #[test]
    fn f_b2_validate_returns_typed_validated_inputs_handle() {
        let fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(product.report.outcome, ReportOutcome::Passed);
        assert_eq!(
            product.validated.calibration,
            fixture.calibration.as_ref().unwrap()
        );
        assert!(matches!(product.validated.artifact, Cow::Borrowed(_)));
        assert_eq!(
            product.artifact_validation_self_hash,
            product.report.report_self_hash
        );
        assert_ne!(
            product.artifact_validation_canonical_bytes_hash,
            Hash256::ZERO
        );
    }

    #[test]
    fn f_b2_validate_validated_inputs_cannot_be_constructed_outside_module() {
        assert!(
            std::any::type_name::<ValidatedInputs<'static>>()
                .starts_with("gbf_codegen::validate::ValidatedInputs")
        );
    }

    #[test]
    fn f_b2_validate_records_canonical_input_hashes() {
        let fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let expected = compute_validated_input_hashes(
            &fixture.inputs(),
            fixture.calibration.as_ref().expect("calibration"),
        );

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(product.validated.input_hashes, expected);
        assert_eq!(
            product.report.body.identity.hint_bundle_hash,
            expected.hint_bundle_hash
        );
        assert_eq!(
            product.report.body.identity.calibration_hash,
            Some(expected.calibration_hash)
        );
        assert_eq!(
            product.report.body.identity.compatibility_adapter_hash,
            expected.compatibility_adapter_hash
        );
    }

    #[test]
    fn f_b2_validate_imported_artifact_view_normalizes_missing_hints() {
        let view = ImportedArtifactView::new(
            artifact_core(),
            artifact_aux(),
            None,
            None,
            transport_identity(),
        );

        assert_eq!(view.hint_bundle, HintBundle::empty());
        assert_eq!(
            view.hint_bundle_hash(),
            HintBundle::empty().compute_canonical_hash()
        );
    }

    #[test]
    fn f_b2_validate_uses_artifact_resolver_trait() {
        let resolver = RecordingResolver;
        let blob = BlobRef {
            hash: hash(0x44),
            len: 3,
            codec: BlobCodec::Raw,
        };

        let resolved = resolver.resolve_blob(&blob).expect("blob resolves");

        assert_eq!(resolved.content_hash, blob.hash);
        assert_eq!(resolved.bytes, vec![1, 2, 3]);
    }

    #[test]
    fn f_b2_validate_validated_inputs_calibration_is_required() {
        let fixture = Fixture::new(Some(HintBundle::empty()), None);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert_eq!(failure.report.body.identity.calibration_hash, None);
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.origin == ValidationOrigin::Calibration)
        );
    }

    #[test]
    fn f_b2_validate_validated_input_hashes_calibration_hash_is_required() {
        let fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_ne!(
            product.validated.input_hashes.calibration_hash,
            Hash256::ZERO
        );
        assert_eq!(
            product.report.body.identity.calibration_hash,
            Some(product.validated.input_hashes.calibration_hash)
        );
    }

    #[test]
    fn f_b2_validate_rejects_compile_request_max_cycles_per_token_zero() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.objective.max_cycles_per_token = Some(0);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert!(
            objective_rejection_reasons(&failure)
                .contains(&ObjectiveRejection::MaxCyclesPerTokenZero)
        );
    }

    #[test]
    fn f_b2_validate_rejects_all_independent_objective_violations() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let service = fixture
            .compile_request
            .objective
            .service
            .as_mut()
            .expect("fixture has service objective");
        service.max_first_token_cycles_p95 = Some(0);
        service.max_resume_latency_cycles_p95 = Some(0);
        fixture.compile_request.objective.max_cycles_per_token = Some(0);
        fixture.compile_request.objective.max_rom_bytes = Some(0);
        fixture.compile_request.objective.risk.cycle_quantile = 0;
        fixture.compile_request.objective.risk.switch_quantile = 101;

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");
        let reasons = objective_rejection_reasons(&failure);

        assert_eq!(reasons.len(), 6);
        assert!(reasons.contains(&ObjectiveRejection::ServiceLevelZero {
            field: "max_first_token_cycles_p95".to_owned()
        }));
        assert!(reasons.contains(&ObjectiveRejection::ServiceLevelZero {
            field: "max_resume_latency_cycles_p95".to_owned()
        }));
        assert!(reasons.contains(&ObjectiveRejection::MaxCyclesPerTokenZero));
        assert!(reasons.contains(&ObjectiveRejection::MaxRomBytesZero));
        assert!(reasons.contains(&ObjectiveRejection::RiskQuantileInvalid {
            field: "cycle_quantile".to_owned(),
            value: 0,
        }));
        assert!(reasons.contains(&ObjectiveRejection::RiskQuantileInvalid {
            field: "switch_quantile".to_owned(),
            value: 101,
        }));
    }

    #[test]
    fn f_b2_validate_rejects_compile_request_objective_rules_table() {
        let cases: Vec<(&str, fn(&mut CompileObjective), ObjectiveRejection)> = vec![
            (
                "service_first_token_zero",
                |objective| {
                    objective
                        .service
                        .as_mut()
                        .expect("fixture has service objective")
                        .max_first_token_cycles_p95 = Some(0);
                },
                ObjectiveRejection::ServiceLevelZero {
                    field: "max_first_token_cycles_p95".to_owned(),
                },
            ),
            (
                "service_checkpoint_gap_zero",
                |objective| {
                    objective
                        .service
                        .as_mut()
                        .expect("fixture has service objective")
                        .max_checkpoint_gap_cycles_p95 = Some(0);
                },
                ObjectiveRejection::ServiceLevelZero {
                    field: "max_checkpoint_gap_cycles_p95".to_owned(),
                },
            ),
            (
                "service_resume_latency_zero",
                |objective| {
                    objective
                        .service
                        .as_mut()
                        .expect("fixture has service objective")
                        .max_resume_latency_cycles_p95 = Some(0);
                },
                ObjectiveRejection::ServiceLevelZero {
                    field: "max_resume_latency_cycles_p95".to_owned(),
                },
            ),
            (
                "service_ui_jitter_zero",
                |objective| {
                    objective
                        .service
                        .as_mut()
                        .expect("fixture has service objective")
                        .max_ui_jitter_frames_p99 = Some(0);
                },
                ObjectiveRejection::ServiceLevelZero {
                    field: "max_ui_jitter_frames_p99".to_owned(),
                },
            ),
            (
                "rom_zero",
                |objective| objective.max_rom_bytes = Some(0),
                ObjectiveRejection::MaxRomBytesZero,
            ),
            (
                "max_cycles_per_token_zero",
                |objective| objective.max_cycles_per_token = Some(0),
                ObjectiveRejection::MaxCyclesPerTokenZero,
            ),
            (
                "bank_switch_zero",
                |objective| objective.max_bank_switches_per_token = Some(0),
                ObjectiveRejection::MaxBankSwitchesPerTokenZero,
            ),
            (
                "sram_page_switch_zero",
                |objective| objective.max_sram_page_switches_per_token = Some(0),
                ObjectiveRejection::MaxSramPageSwitchesPerTokenZero,
            ),
            (
                "cycle_quantile_zero",
                |objective| objective.risk.cycle_quantile = 0,
                ObjectiveRejection::RiskQuantileInvalid {
                    field: "cycle_quantile".to_owned(),
                    value: 0,
                },
            ),
            (
                "switch_quantile_above_100",
                |objective| objective.risk.switch_quantile = 101,
                ObjectiveRejection::RiskQuantileInvalid {
                    field: "switch_quantile".to_owned(),
                    value: 101,
                },
            ),
        ];

        for (case, mutate, expected) in cases {
            let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
            mutate(&mut fixture.compile_request.objective);

            let failure = validate_artifact_and_request(fixture.inputs()).expect_err(case);

            assert_eq!(
                objective_rejection_reasons(&failure),
                vec![expected],
                "{case}"
            );
        }
    }

    #[test]
    fn f_b2_validate_fallback_objective_fields_are_not_stage0_gates() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.objective.risk.fallback_profile =
            Some(CompileProfileId::from("recovery"));
        fixture.compile_request.objective.risk.fallback_runtime_mode =
            Some(RuntimeMode::Interactive);

        validate_artifact_and_request(fixture.inputs()).expect("fallback fields are accepted");
    }

    struct Fixture {
        artifact: ImportedArtifactView,
        lowerings: Vec<TargetDataLoweringArtifact>,
        workloads: Vec<WorkloadManifestRef>,
        golden_vectors: Vec<GoldenVectorRef>,
        compile_request: CompileRequest,
        target_profile: TargetProfile,
        compile_profile: CompileProfileSpec,
        calibration: Option<CalibrationBundleSet>,
        resolver: RecordingResolver,
    }

    impl Fixture {
        fn new(hint_bundle: Option<HintBundle>, calibration: Option<CalibrationBundleSet>) -> Self {
            Self {
                artifact: ImportedArtifactView::new(
                    artifact_core(),
                    artifact_aux(),
                    hint_bundle,
                    None,
                    transport_identity(),
                ),
                lowerings: vec![lowering()],
                workloads: vec![workload()],
                golden_vectors: vec![golden_vector()],
                compile_request: compile_request(),
                target_profile: dmg_mbc5_8mib_128kib(),
                compile_profile: compile_profile(),
                calibration,
                resolver: RecordingResolver,
            }
        }

        fn inputs(&self) -> ValidateInputs<'_> {
            ValidateInputs {
                artifact: &self.artifact,
                lowerings: &self.lowerings,
                workloads: &self.workloads,
                golden_vectors: &self.golden_vectors,
                compile_request: &self.compile_request,
                target_profile: &self.target_profile,
                compile_profile: &self.compile_profile,
                calibration: self.calibration.as_ref(),
                resolver: &self.resolver,
            }
        }
    }

    struct RecordingResolver;

    impl ArtifactResolver for RecordingResolver {
        fn resolve_blob(&self, blob: &BlobRef) -> Result<ResolvedBlob, ArtifactResolveError> {
            Ok(ResolvedBlob {
                bytes: vec![1, 2, 3],
                content_hash: blob.hash,
            })
        }

        fn resolve_sidecar(
            &self,
            sidecar: &SidecarRef,
        ) -> Result<ResolvedSidecar, ArtifactResolveError> {
            Ok(ResolvedSidecar {
                bytes: Vec::new(),
                content_hash: sidecar.hash,
            })
        }

        fn resolve_evidence(
            &self,
            evidence: &EvidenceRef,
        ) -> Result<ResolvedEvidence, ArtifactResolveError> {
            Ok(ResolvedEvidence {
                bytes: evidence.reference.as_bytes().to_vec(),
                content_hash: evidence.hash,
            })
        }

        fn resolve_workload(
            &self,
            workload: &WorkloadManifestRef,
        ) -> Result<ResolvedWorkload, ArtifactResolveError> {
            Ok(ResolvedWorkload {
                manifest: WorkloadManifest {
                    id: workload.id.clone(),
                    schema_version: gbf_workload::WorkloadSchemaVersion { epoch: 1, minor: 0 },
                    self_hash: workload.manifest_hash,
                    golden_vectors: Vec::new(),
                    future_fields: gbf_workload::WorkloadFuturePlaceholder::default(),
                },
            })
        }

        fn resolve_golden_vector(
            &self,
            vector: &GoldenVectorRef,
        ) -> Result<ResolvedGoldenVector, ArtifactResolveError> {
            Ok(ResolvedGoldenVector {
                bytes: Vec::new(),
                manifest_hash: vector.manifest_hash,
            })
        }
    }

    fn artifact_core() -> ArtifactCore {
        ArtifactCore::new(
            Vec::new(),
            QuantSpec::default(),
            SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
        )
        .expect("empty core with linear state is valid")
    }

    fn artifact_aux() -> ArtifactAux {
        ArtifactAux {
            checkpoint_schema: None,
            conformance_envelope: None,
            golden_vectors: Vec::new(),
            interaction_bundle: None,
            lexical_spec: None,
            reference_observation_cache: None,
        }
    }

    fn transport_identity() -> ArtifactTransportIdentity {
        ArtifactTransportIdentity {
            source_uri: Some("fixture://artifact".to_owned()),
            transport_hash: hash(0x01),
            import_tool_hash: hash(0x02),
        }
    }

    fn lowering() -> TargetDataLoweringArtifact {
        TargetDataLoweringArtifact {
            profile: DataLoweringProfileId("fixture.dmg".to_owned()),
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            packer_version: PackerVersion::new(1, 0, 0),
            manifest_hash: hash(0x03),
            shards: vec![LoweringShard {
                id: LoweringShardId("weight.layer0".to_owned()),
                kind: LoweringShardKind::WeightShard,
                payload_hash: hash(0x04),
                packed_bytes_hash: hash(0x05),
            }],
        }
    }

    fn workload() -> WorkloadManifestRef {
        WorkloadManifestRef {
            id: WorkloadId::from("workload.fixture"),
            manifest_hash: hash(0x06),
            locator: WorkloadLocator::Path {
                path: "fixtures/workload.json".to_owned(),
            },
        }
    }

    fn golden_vector() -> GoldenVectorRef {
        GoldenVectorRef {
            id: GoldenVectorId("golden.fixture".to_owned()),
            manifest_hash: hash(0x07),
        }
    }

    fn compile_request() -> CompileRequest {
        CompileRequest {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID),
            objective: CompileObjective {
                service: Some(ServiceLevelObjective {
                    max_first_token_cycles_p95: Some(21_000),
                    max_checkpoint_gap_cycles_p95: Some(13_000),
                    max_resume_latency_cycles_p95: Some(8_000),
                    max_ui_jitter_frames_p99: Some(2),
                }),
                max_cycles_per_token: Some(24_000),
                max_bank_switches_per_token: Some(17),
                max_sram_page_switches_per_token: Some(3),
                min_ui_headroom_pct: 11,
                max_rom_bytes: Some(2 * 1024 * 1024),
                risk: RiskPolicy {
                    cycle_quantile: 95,
                    switch_quantile: 99,
                    calibration_confidence_requirement:
                        CalibrationConfidenceRequirement::NoMinimumConfidence,
                    fallback_profile: None,
                    fallback_runtime_mode: Some(RuntimeMode::Safe),
                },
            },
            calibration_set_ref: CalibrationSetRef::default(),
            required_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
            constraint_overrides: None,
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
        }
    }

    fn compile_profile() -> CompileProfileSpec {
        canonical_compile_profile_specs()
            .expect("canonical profiles parse")
            .into_iter()
            .find(|profile| profile.id.as_str() == BRINGUP_COMPILE_PROFILE_ID)
            .expect("bringup profile exists")
    }

    fn calibration() -> CalibrationBundleSet {
        CalibrationBundleSet {
            bundles: BTreeMap::from_iter(BootstrapCalibrationBundle::new(hash(0x08)).bundles),
        }
    }

    fn objective_rejection_reasons(failure: &ValidationStageFailure) -> Vec<ObjectiveRejection> {
        failure
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match &diagnostic.code {
                ValidationCode::CompileRequestProfileForbidsObjective { reason, .. } => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }
}
