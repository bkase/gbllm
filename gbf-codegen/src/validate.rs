//! Stage 0 pipeline-entry validation plumbing.
//!
//! Successful validation yields a [`ValidatedInputs`] token whose witness field
//! is private to this module:
//!
//! ```compile_fail
//! use std::borrow::Cow;
//!
//! use gbf_artifact::{HintBundle, TargetDataLoweringArtifact};
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
//!         _private: unreachable!(),
//!     };
//! }
//! ```

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_artifact::aux::SidecarKind;
use gbf_artifact::core::ArtifactCore;
use gbf_artifact::manifest::{
    ArtifactFeature, ArtifactSchemaVersion, ComponentKind, ManifestComponent, ManifestInvariant,
};
use gbf_artifact::{ArtifactAux, ArtifactManifest, HintBundle, TargetDataLoweringArtifact};
use gbf_foundation::{BlobRef, Hash256, SemVer};
use gbf_hw::target::TargetProfile;
use gbf_policy::{
    CalibrationBundleSet, CalibrationLayer, CompatibilityAdapterId, CompileProfileSpec,
    CompileRequest, DiagnosticSeverity, EvidenceRef, FieldPath, ValidationCode, ValidationDetail,
    ValidationDiagnostic as PolicyValidationDiagnostic, ValidationOrigin,
};
use gbf_report::report_schemas::artifact_validation_v1::{
    ArtifactCompatibilityDecision, ArtifactCompatibilityFailure, ArtifactCompatibilitySection,
    ArtifactValidationIdentitySection, ArtifactValidationInputSection,
    ArtifactValidationReportBody,
};
use gbf_report::{
    ReportBody, ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report,
    canonicalize_value, compute_self_hash,
};
use gbf_workload::{
    GoldenVectorId, GoldenVectorRef, WorkloadId, WorkloadManifest, WorkloadManifestRef,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub type ValidationDiagnostic = PolicyValidationDiagnostic;

pub const CURRENT_ARTIFACT_SCHEMA_VERSION: ArtifactSchemaVersion =
    ArtifactSchemaVersion { epoch: 1, minor: 1 };

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
    pub manifest: ArtifactManifest,
    pub aux: ArtifactAux,
    pub hint_bundle: HintBundle,
    pub reference: Option<ReferenceLink>,
    pub transport: ArtifactTransportIdentity,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub forbidden_build_identity_fields: BTreeSet<FieldPath>,
}

impl ImportedArtifactView {
    #[must_use]
    pub fn new(
        core: ArtifactCore,
        manifest: ArtifactManifest,
        aux: ArtifactAux,
        hint_bundle: Option<HintBundle>,
        reference: Option<ReferenceLink>,
        transport: ArtifactTransportIdentity,
    ) -> Self {
        Self {
            core,
            manifest,
            aux,
            hint_bundle: hint_bundle.unwrap_or_else(HintBundle::empty),
            reference,
            transport,
            forbidden_build_identity_fields: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn hint_bundle_hash(&self) -> Hash256 {
        self.hint_bundle.compute_canonical_hash()
    }

    #[must_use]
    pub fn manifest_hash(&self) -> Hash256 {
        self.manifest.manifest_self_hash
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
        artifact: Cow<'a, ImportedArtifactView>,
        calibration: &'a CalibrationBundleSet,
        input_hashes: ValidatedInputHashes,
    ) -> Self {
        Self {
            artifact,
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
    pub artifact_validation_self_hash: Hash256,
    pub artifact_validation_canonical_bytes_hash: Hash256,
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
    match validate_schema_compatibility(inputs.artifact) {
        Ok(compatibility) => compute_validated_input_hashes_for_artifact(
            inputs,
            compatibility.artifact.as_ref(),
            calibration,
            compatibility.adapter_hash,
        ),
        Err(_) => {
            compute_validated_input_hashes_for_artifact(inputs, inputs.artifact, calibration, None)
        }
    }
}

fn compute_validated_input_hashes_for_artifact(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    calibration: &CalibrationBundleSet,
    compatibility_adapter_hash: Option<Hash256>,
) -> ValidatedInputHashes {
    let artifact_effective_core_hash = artifact.core.semantic_hash();

    ValidatedInputHashes {
        artifact_source_hash: inputs.artifact.transport.transport_hash,
        artifact_effective_core_hash,
        artifact_manifest_hash: artifact.manifest_hash(),
        artifact_aux_hash: input_hash(
            "gbf-artifact",
            "ArtifactAux",
            "artifact_aux",
            "1.0.0",
            &artifact.aux,
        ),
        lowering_manifest_hash: input_hash(
            "gbf-artifact",
            "TargetDataLoweringArtifactList",
            "lowering_manifest",
            "1.0.0",
            inputs.lowerings,
        ),
        hint_bundle_hash: artifact.hint_bundle_hash(),
        compile_request_hash: input_hash(
            "gbf-policy",
            "CompileRequest",
            "compile_request",
            "1.0.0",
            inputs.compile_request,
        ),
        target_profile_hash: input_hash(
            "gbf-hw",
            "TargetProfile",
            "target_profile",
            "1.0.0",
            inputs.target_profile,
        ),
        compile_profile_hash: input_hash(
            "gbf-policy",
            "CompileProfileSpec",
            "compile_profile",
            "1.0.0",
            inputs.compile_profile,
        ),
        calibration_hash: input_hash(
            "gbf-policy",
            "CalibrationBundleSet",
            "calibration",
            "1.0.0",
            calibration,
        ),
        compatibility_adapter_hash,
    }
}

struct SchemaCompatibilityOutcome<'a> {
    artifact: Cow<'a, ImportedArtifactView>,
    decision: ArtifactCompatibilityDecision,
    adapter_hash: Option<Hash256>,
}

impl SchemaCompatibilityOutcome<'_> {
    fn compatibility_section(&self) -> ArtifactCompatibilitySection {
        ArtifactCompatibilitySection {
            decision: Some(self.decision.clone()),
            failures: Vec::new(),
        }
    }
}

struct SchemaCompatibilityFailure {
    diagnostics: Vec<ValidationDiagnostic>,
    compatibility: ArtifactCompatibilitySection,
}

#[derive(Clone)]
struct SchemaCompatibilityAdapter {
    id: CompatibilityAdapterId,
    from: ArtifactSchemaVersion,
    to: ArtifactSchemaVersion,
    lossless: bool,
    implementation: AdapterImplementation,
    implementation_id: &'static str,
}

#[derive(Clone, Copy)]
enum AdapterImplementation {
    SchemaVersionOnly,
    SemanticChangingProofFixture,
}

impl SchemaCompatibilityAdapter {
    fn apply(&self, source: &ImportedArtifactView) -> ImportedArtifactView {
        let mut upgraded = source.clone();
        upgraded.manifest.schema_version = self.to;
        match self.implementation {
            AdapterImplementation::SchemaVersionOnly => {}
            AdapterImplementation::SemanticChangingProofFixture => {
                upgraded.core = ArtifactCore::new(
                    source.core.tensors().to_vec(),
                    source.core.quant().clone(),
                    gbf_artifact::sequence::SequenceSemanticsSpec::linear_state(2)
                        .expect("fixture semantic-changing adapter state width is nonzero"),
                )
                .expect("fixture semantic-changing adapter preserves core structural validity");
                upgraded.manifest.semantic_core_hash = upgraded.core.semantic_hash();
            }
        }
        upgraded.manifest.manifest_self_hash =
            compute_artifact_manifest_self_hash(&upgraded.manifest);
        upgraded
    }

    fn hash(&self) -> Hash256 {
        #[derive(Serialize)]
        struct AdapterHashMaterial<'a> {
            id: &'a CompatibilityAdapterId,
            from: ArtifactSchemaVersion,
            to: ArtifactSchemaVersion,
            lossless: bool,
            implementation_id: &'a str,
        }

        input_hash(
            "gbf-codegen",
            "SchemaCompatibilityAdapter",
            "schema_compatibility_adapter",
            "1.0.0",
            &AdapterHashMaterial {
                id: &self.id,
                from: self.from,
                to: self.to,
                lossless: self.lossless,
                implementation_id: self.implementation_id,
            },
        )
    }
}

#[allow(clippy::result_large_err)]
fn validate_schema_compatibility<'a>(
    source: &'a ImportedArtifactView,
) -> Result<SchemaCompatibilityOutcome<'a>, SchemaCompatibilityFailure> {
    let observed = source.manifest.schema_version;
    if observed == CURRENT_ARTIFACT_SCHEMA_VERSION {
        return Ok(SchemaCompatibilityOutcome {
            artifact: Cow::Borrowed(source),
            decision: ArtifactCompatibilityDecision::CurrentSchema,
            adapter_hash: None,
        });
    }

    let observed_semver = schema_semver(observed);
    let target_semver = schema_semver(CURRENT_ARTIFACT_SCHEMA_VERSION);

    if observed.epoch != CURRENT_ARTIFACT_SCHEMA_VERSION.epoch {
        return Err(schema_compatibility_failure(
            schema_diagnostic(
                ValidationCode::SchemaEpochUnsupported,
                FieldPath::from("manifest.schema_version.epoch"),
                source.manifest.manifest_self_hash,
            ),
            vec![ArtifactCompatibilityFailure::UnsupportedEpoch {
                observed: observed_semver,
                supported: target_semver,
            }],
        ));
    }

    let Some(adapter) = registered_schema_adapter(observed, CURRENT_ARTIFACT_SCHEMA_VERSION) else {
        return Err(schema_compatibility_failure(
            schema_diagnostic(
                ValidationCode::SchemaCompatibilityAdapterMissing {
                    observed: observed_semver,
                    target: target_semver,
                },
                FieldPath::from("manifest.schema_version"),
                source.manifest.manifest_self_hash,
            ),
            vec![ArtifactCompatibilityFailure::AdapterMissing {
                observed: observed_semver,
                target: target_semver,
            }],
        ));
    };

    let adapter_hash = adapter.hash();
    if !adapter.lossless {
        return Err(schema_compatibility_failure(
            schema_diagnostic(
                ValidationCode::SchemaCompatibilityAdapterNotLossless {
                    adapter: adapter.id.clone(),
                },
                FieldPath::from("manifest.schema_version"),
                adapter_hash,
            ),
            vec![ArtifactCompatibilityFailure::AdapterNotLossless {
                adapter: adapter.id,
            }],
        ));
    }

    let before_semantic_hash = source.core.semantic_hash();
    let upgraded = adapter.apply(source);
    let after_semantic_hash = upgraded.core.semantic_hash();
    if before_semantic_hash != after_semantic_hash {
        return Err(schema_compatibility_failure(
            schema_diagnostic(
                ValidationCode::SchemaCompatibilityAdapterNotLossless {
                    adapter: adapter.id.clone(),
                },
                FieldPath::from("manifest.schema_version"),
                adapter_hash,
            ),
            vec![ArtifactCompatibilityFailure::SemanticHashChanged {
                before: before_semantic_hash,
                after: after_semantic_hash,
            }],
        ));
    }

    Ok(SchemaCompatibilityOutcome {
        artifact: Cow::Owned(upgraded),
        decision: ArtifactCompatibilityDecision::LosslessInMemoryUpgrade {
            from_schema: observed_semver,
            to_schema: target_semver,
            adapter: adapter.id,
            adapter_hash,
        },
        adapter_hash: Some(adapter_hash),
    })
}

fn schema_compatibility_failure(
    diagnostic: ValidationDiagnostic,
    failures: Vec<ArtifactCompatibilityFailure>,
) -> SchemaCompatibilityFailure {
    SchemaCompatibilityFailure {
        diagnostics: vec![diagnostic],
        compatibility: ArtifactCompatibilitySection {
            decision: None,
            failures,
        },
    }
}

fn registered_schema_adapter(
    from: ArtifactSchemaVersion,
    to: ArtifactSchemaVersion,
) -> Option<SchemaCompatibilityAdapter> {
    builtin_schema_adapters()
        .into_iter()
        .find(|adapter| adapter.from == from && adapter.to == to)
}

fn builtin_schema_adapters() -> [SchemaCompatibilityAdapter; 4] {
    [
        SchemaCompatibilityAdapter {
            id: CompatibilityAdapterId("adapter.lossless".to_owned()),
            from: ArtifactSchemaVersion { epoch: 1, minor: 0 },
            to: CURRENT_ARTIFACT_SCHEMA_VERSION,
            lossless: true,
            implementation: AdapterImplementation::SchemaVersionOnly,
            implementation_id: "gbf-codegen.stage0.schema-v1-0-to-v1-1.lossless.v1",
        },
        SchemaCompatibilityAdapter {
            id: CompatibilityAdapterId("adapter.lossy".to_owned()),
            from: ArtifactSchemaVersion { epoch: 1, minor: 2 },
            to: CURRENT_ARTIFACT_SCHEMA_VERSION,
            lossless: false,
            implementation: AdapterImplementation::SchemaVersionOnly,
            implementation_id: "gbf-codegen.stage0.schema-v1-2-to-v1-1.lossy.v1",
        },
        SchemaCompatibilityAdapter {
            id: CompatibilityAdapterId("adapter.semantic-changing".to_owned()),
            from: ArtifactSchemaVersion { epoch: 1, minor: 3 },
            to: CURRENT_ARTIFACT_SCHEMA_VERSION,
            lossless: true,
            implementation: AdapterImplementation::SemanticChangingProofFixture,
            implementation_id: "gbf-codegen.stage0.schema-v1-3-to-v1-1.semantic-changing.v1",
        },
        SchemaCompatibilityAdapter {
            id: CompatibilityAdapterId("adapter.cross-major".to_owned()),
            from: ArtifactSchemaVersion { epoch: 2, minor: 0 },
            to: CURRENT_ARTIFACT_SCHEMA_VERSION,
            lossless: true,
            implementation: AdapterImplementation::SchemaVersionOnly,
            implementation_id: "gbf-codegen.stage0.schema-v2-0-to-v1-1.forbidden.v1",
        },
    ]
}

fn schema_semver(version: ArtifactSchemaVersion) -> SemVer {
    SemVer::new(u64::from(version.epoch), u64::from(version.minor), 0)
}

fn schema_diagnostic(
    code: ValidationCode,
    field: FieldPath,
    provenance_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Schema,
        code,
        ValidationDetail::Field {
            field: field.clone(),
        },
        vec![EvidenceRef {
            kind: "artifact_manifest".to_owned(),
            reference: field.to_string(),
            hash: Some(provenance_hash),
        }],
    )
}

fn validate_semantic_core_hash(
    artifact: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let recomputed = artifact.core.semantic_hash();
    let recorded = artifact.manifest.semantic_core_hash;
    if recomputed != recorded {
        diagnostics.push(ValidationDiagnostic::hard(
            ValidationOrigin::SemanticCore,
            ValidationCode::SemanticCoreHashMismatch,
            ValidationDetail::HashMismatch {
                expected: recorded,
                observed: recomputed,
            },
            vec![EvidenceRef {
                kind: "artifact_manifest".to_owned(),
                reference: "semantic_core_hash".to_owned(),
                hash: Some(artifact.manifest.manifest_self_hash),
            }],
        ));
    }
}

fn validate_transport_manifest(
    source: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let recomputed_source_hash = compute_imported_artifact_source_hash(source);
    if source.transport.transport_hash != recomputed_source_hash {
        diagnostics.push(ValidationDiagnostic::hard(
            ValidationOrigin::Manifest,
            ValidationCode::ArtifactTransportManifestMismatch,
            ValidationDetail::HashMismatch {
                expected: recomputed_source_hash,
                observed: source.transport.transport_hash,
            },
            vec![EvidenceRef {
                kind: "artifact_transport".to_owned(),
                reference: "transport_hash".to_owned(),
                hash: Some(source.transport.transport_hash),
            }],
        ));
    }
}

fn validate_manifest_invariants(
    source: &ImportedArtifactView,
    effective: &ImportedArtifactView,
    lowerings: &[TargetDataLoweringArtifact],
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    validate_manifest_self_hash(&source.manifest, diagnostics);
    if source.manifest != effective.manifest {
        validate_manifest_self_hash(&effective.manifest, diagnostics);
    }
    validate_feature_epoch_invariants(&effective.manifest, diagnostics);
    validate_component_digests(&effective.core, &effective.manifest, diagnostics);
    validate_forbidden_build_identity_fields(effective, lowerings, diagnostics);
}

fn validate_manifest_self_hash(
    manifest: &ArtifactManifest,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let recomputed = compute_artifact_manifest_self_hash(manifest);
    let recorded = manifest.manifest_self_hash;
    if recomputed != recorded {
        let invariant = ManifestInvariant::ManifestSelfHashMismatch {
            recomputed,
            recorded,
        };
        diagnostics.push(manifest_invariant_diagnostic(
            invariant,
            ValidationDetail::HashMismatch {
                expected: recomputed,
                observed: recorded,
            },
            recorded,
        ));
    }
}

fn validate_feature_epoch_invariants(
    manifest: &ArtifactManifest,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for feature in &manifest.required_features {
        let minimum = minimum_schema_for_feature(*feature);
        if manifest.schema_version < minimum {
            diagnostics.push(manifest_invariant_diagnostic(
                ManifestInvariant::FeatureSetEpochInconsistent {
                    epoch: manifest.schema_version,
                    feature: *feature,
                },
                ValidationDetail::Field {
                    field: FieldPath::from("manifest.required_features"),
                },
                manifest.manifest_self_hash,
            ));
        }
    }
}

fn minimum_schema_for_feature(feature: ArtifactFeature) -> ArtifactSchemaVersion {
    match feature {
        ArtifactFeature::DenseI8
        | ArtifactFeature::Ternary2Quant
        | ArtifactFeature::Binary1Quant
        | ArtifactFeature::SparseTernaryBitplanes => ArtifactSchemaVersion { epoch: 1, minor: 0 },
        ArtifactFeature::MoeRouting
        | ArtifactFeature::LinearStateSequence
        | ArtifactFeature::BoundedKvSequence => ArtifactSchemaVersion { epoch: 1, minor: 1 },
    }
}

fn validate_component_digests(
    core: &ArtifactCore,
    manifest: &ArtifactManifest,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for component in &manifest.components {
        if component.kind != ComponentKind::CanonicalTensor {
            continue;
        }
        validate_canonical_tensor_component(core, manifest, component, diagnostics);
    }
}

fn validate_canonical_tensor_component(
    core: &ArtifactCore,
    manifest: &ArtifactManifest,
    component: &ManifestComponent,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let Some(tensor) = core
        .tensors()
        .iter()
        .find(|tensor| tensor.id.as_str() == component.id.0.as_str())
    else {
        diagnostics.push(manifest_invariant_diagnostic(
            ManifestInvariant::RequiredComponentMissing {
                component: component.id.clone(),
            },
            ValidationDetail::Field {
                field: FieldPath::from(format!("manifest.components.{}", component.id.0)),
            },
            manifest.manifest_self_hash,
        ));
        return;
    };

    if tensor.content_hash != component.digest {
        diagnostics.push(manifest_invariant_diagnostic(
            ManifestInvariant::ComponentDigestMismatch {
                component: component.id.clone(),
                expected: tensor.content_hash,
                observed: component.digest,
            },
            ValidationDetail::HashMismatch {
                expected: tensor.content_hash,
                observed: component.digest,
            },
            manifest.manifest_self_hash,
        ));
    }
}

fn validate_forbidden_build_identity_fields(
    artifact: &ImportedArtifactView,
    lowerings: &[TargetDataLoweringArtifact],
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for field in serialized_forbidden_build_identity_fields("manifest", &artifact.manifest)
        .into_iter()
        .chain(serialized_forbidden_build_identity_fields(
            "aux",
            &artifact.aux,
        ))
        .chain(serialized_forbidden_build_identity_fields(
            "lowerings",
            &lowerings,
        ))
        .chain(artifact.forbidden_build_identity_fields.iter().cloned())
    {
        diagnostics.push(forbidden_build_identity_diagnostic(
            field,
            artifact.manifest.manifest_self_hash,
        ));
    }
}

fn serialized_forbidden_build_identity_fields<T: Serialize>(
    root: &str,
    value: &T,
) -> Vec<FieldPath> {
    let value = serde_json::to_value(value).expect("Stage 0 forbidden field scan serializes");
    let mut fields = Vec::new();
    collect_forbidden_build_identity_fields(&value, root, &mut fields);
    fields
}

fn collect_forbidden_build_identity_fields(
    value: &serde_json::Value,
    path: &str,
    fields: &mut Vec<FieldPath>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    format!("/{key}")
                } else {
                    format!("{path}/{key}")
                };
                if is_forbidden_build_identity_key(key) {
                    fields.push(FieldPath::from(child_path.clone()));
                }
                collect_forbidden_build_identity_fields(child, &child_path, fields);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_forbidden_build_identity_fields(child, &format!("{path}/{index}"), fields);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn is_forbidden_build_identity_key(key: &str) -> bool {
    matches!(
        key,
        "build_identity"
            | "build_identity_block"
            | "compatibility_envelope"
            | "encoded_rom_hash"
            | "backend_identity"
            | "stage12_identity"
    )
}

fn forbidden_build_identity_diagnostic(
    field: FieldPath,
    provenance_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Manifest,
        ValidationCode::ArtifactForbiddenBuildIdentityField {
            field: field.clone(),
        },
        ValidationDetail::Field {
            field: field.clone(),
        },
        vec![EvidenceRef {
            kind: "artifact_manifest".to_owned(),
            reference: field.to_string(),
            hash: Some(provenance_hash),
        }],
    )
}

fn manifest_invariant_diagnostic(
    invariant: ManifestInvariant,
    detail: ValidationDetail,
    provenance_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Manifest,
        ValidationCode::ManifestInvariantViolated { invariant },
        detail,
        vec![EvidenceRef {
            kind: "artifact_manifest".to_owned(),
            reference: "manifest".to_owned(),
            hash: Some(provenance_hash),
        }],
    )
}

fn stage0_failure(
    inputs: &ValidateInputs<'_>,
    artifact: Option<&ImportedArtifactView>,
    calibration: Option<&CalibrationBundleSet>,
    diagnostics: Vec<ValidationDiagnostic>,
    compatibility: ArtifactCompatibilitySection,
) -> ValidationStageFailure {
    let compatibility_adapter_hash = compatibility_section_adapter_hash(&compatibility);
    let body = ArtifactValidationReportBody {
        identity: failure_identity(inputs, artifact, calibration, compatibility_adapter_hash),
        compatibility,
        checked_inputs: checked_inputs(inputs, artifact.unwrap_or(inputs.artifact)),
        diagnostics: diagnostics.clone(),
    };
    let report = ReportEnvelope::new(ReportOutcome::Failed, body)
        .expect("artifact_validation.v1 schema constants are valid");
    let (report, artifact_validation_self_hash, artifact_validation_canonical_bytes_hash) =
        finalize_report(report);

    ValidationStageFailure {
        report,
        diagnostics,
        artifact_validation_self_hash,
        artifact_validation_canonical_bytes_hash,
    }
}

fn failure_identity(
    inputs: &ValidateInputs<'_>,
    artifact: Option<&ImportedArtifactView>,
    calibration: Option<&CalibrationBundleSet>,
    compatibility_adapter_hash: Option<Hash256>,
) -> ArtifactValidationIdentitySection {
    let compile_request_hash = input_hash(
        "gbf-policy",
        "CompileRequest",
        "compile_request",
        "1.0.0",
        inputs.compile_request,
    );
    let target_profile_hash = input_hash(
        "gbf-hw",
        "TargetProfile",
        "target_profile",
        "1.0.0",
        inputs.target_profile,
    );
    let compile_profile_hash = input_hash(
        "gbf-policy",
        "CompileProfileSpec",
        "compile_profile",
        "1.0.0",
        inputs.compile_profile,
    );

    ArtifactValidationIdentitySection {
        artifact_source_hash: Some(inputs.artifact.transport.transport_hash),
        artifact_effective_core_hash: artifact.map(|artifact| artifact.core.semantic_hash()),
        artifact_manifest_hash: artifact.map(ImportedArtifactView::manifest_hash),
        semantic_core_hash: artifact.map(|artifact| artifact.core.semantic_hash()),
        artifact_aux_hash: artifact.map(|artifact| {
            input_hash(
                "gbf-artifact",
                "ArtifactAux",
                "artifact_aux",
                "1.0.0",
                &artifact.aux,
            )
        }),
        lowering_manifest_hash: artifact.map(|_| {
            input_hash(
                "gbf-artifact",
                "TargetDataLoweringArtifactList",
                "lowering_manifest",
                "1.0.0",
                inputs.lowerings,
            )
        }),
        hint_bundle_hash: artifact.unwrap_or(inputs.artifact).hint_bundle_hash(),
        compile_request_hash,
        target_profile_hash,
        compile_profile_hash,
        calibration_hash: calibration.map(|calibration| {
            input_hash(
                "gbf-policy",
                "CalibrationBundleSet",
                "calibration",
                "1.0.0",
                calibration,
            )
        }),
        compatibility_adapter_hash,
    }
}

fn compatibility_section_adapter_hash(
    compatibility: &ArtifactCompatibilitySection,
) -> Option<Hash256> {
    match &compatibility.decision {
        Some(ArtifactCompatibilityDecision::LosslessInMemoryUpgrade { adapter_hash, .. }) => {
            Some(*adapter_hash)
        }
        Some(ArtifactCompatibilityDecision::CurrentSchema) | None => None,
    }
}

#[must_use]
pub fn compute_artifact_manifest_self_hash(manifest: &ArtifactManifest) -> Hash256 {
    let mut normalized = manifest.clone();
    normalized.manifest_self_hash = Hash256::ZERO;
    input_hash(
        "gbf-artifact",
        "ArtifactManifest",
        "artifact_manifest_self_hash",
        "1.0.0",
        &normalized,
    )
}

fn compute_imported_artifact_source_hash(artifact: &ImportedArtifactView) -> Hash256 {
    #[derive(Serialize)]
    struct SourceHashMaterial<'a> {
        core: &'a ArtifactCore,
        manifest: &'a ArtifactManifest,
        aux: &'a ArtifactAux,
        hint_bundle: &'a HintBundle,
        reference: &'a Option<ReferenceLink>,
    }

    input_hash(
        "gbf-codegen",
        "ImportedArtifactViewSource",
        "imported_artifact_source",
        "1.0.0",
        &SourceHashMaterial {
            core: &artifact.core,
            manifest: &artifact.manifest,
            aux: &artifact.aux,
            hint_bundle: &artifact.hint_bundle,
            reference: &artifact.reference,
        },
    )
}

#[allow(clippy::result_large_err)]
pub fn validate_artifact_and_request<'a>(
    inputs: ValidateInputs<'a>,
) -> Result<ValidationProduct<'a>, ValidationStageFailure> {
    let compatibility = match validate_schema_compatibility(inputs.artifact) {
        Ok(compatibility) => compatibility,
        Err(failure) => {
            return Err(stage0_failure(
                &inputs,
                None,
                inputs.calibration,
                failure.diagnostics,
                failure.compatibility,
            ));
        }
    };
    let effective_artifact = compatibility.artifact.as_ref();

    let mut diagnostics = Vec::new();
    validate_semantic_core_hash(effective_artifact, &mut diagnostics);
    validate_transport_manifest(inputs.artifact, &mut diagnostics);
    validate_manifest_invariants(
        inputs.artifact,
        effective_artifact,
        inputs.lowerings,
        &mut diagnostics,
    );

    if !diagnostics.is_empty() {
        return Err(stage0_failure(
            &inputs,
            Some(effective_artifact),
            inputs.calibration,
            diagnostics,
            compatibility.compatibility_section(),
        ));
    }

    let Some(calibration) = inputs.calibration else {
        return Err(missing_calibration_failure(
            &inputs,
            effective_artifact,
            compatibility.compatibility_section(),
            compatibility.adapter_hash,
        ));
    };

    let input_hashes = compute_validated_input_hashes_for_artifact(
        &inputs,
        effective_artifact,
        calibration,
        compatibility.adapter_hash,
    );
    let report = success_report(
        &inputs,
        effective_artifact,
        &input_hashes,
        compatibility.decision,
    );
    let (report, artifact_validation_self_hash, artifact_validation_canonical_bytes_hash) =
        finalize_report(report);
    let validated = ValidatedInputs::new(inputs, compatibility.artifact, calibration, input_hashes);

    Ok(ValidationProduct {
        validated,
        report,
        artifact_validation_self_hash,
        artifact_validation_canonical_bytes_hash,
    })
}

fn success_report(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    input_hashes: &ValidatedInputHashes,
    decision: ArtifactCompatibilityDecision,
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
                decision: Some(decision),
                failures: Vec::new(),
            },
            checked_inputs: checked_inputs(inputs, artifact),
            diagnostics: Vec::new(),
        },
    )
    .expect("artifact_validation.v1 schema constants are valid")
}

fn missing_calibration_failure(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    compatibility: ArtifactCompatibilitySection,
    compatibility_adapter_hash: Option<Hash256>,
) -> ValidationStageFailure {
    let compile_request_hash = input_hash(
        "gbf-policy",
        "CompileRequest",
        "compile_request",
        "1.0.0",
        inputs.compile_request,
    );
    let diagnostics = CalibrationLayer::all()
        .into_iter()
        .map(|class| ValidationDiagnostic {
            severity: DiagnosticSeverity::Hard,
            origin: ValidationOrigin::Calibration,
            code: ValidationCode::CalibrationMissing { class },
            detail: ValidationDetail::Field {
                field: FieldPath::from(format!("calibration.{}", class.as_str())),
            },
            provenance: vec![EvidenceRef {
                kind: "compile_request".to_owned(),
                reference: "calibration_set_ref".to_owned(),
                hash: Some(compile_request_hash),
            }],
        })
        .collect::<Vec<_>>();
    let body = ArtifactValidationReportBody {
        identity: ArtifactValidationIdentitySection {
            artifact_source_hash: Some(inputs.artifact.transport.transport_hash),
            artifact_effective_core_hash: Some(artifact.core.semantic_hash()),
            artifact_manifest_hash: Some(artifact.manifest_hash()),
            semantic_core_hash: Some(artifact.core.semantic_hash()),
            artifact_aux_hash: Some(input_hash(
                "gbf-artifact",
                "ArtifactAux",
                "artifact_aux",
                "1.0.0",
                &artifact.aux,
            )),
            lowering_manifest_hash: Some(input_hash(
                "gbf-artifact",
                "TargetDataLoweringArtifactList",
                "lowering_manifest",
                "1.0.0",
                inputs.lowerings,
            )),
            hint_bundle_hash: artifact.hint_bundle_hash(),
            compile_request_hash,
            target_profile_hash: input_hash(
                "gbf-hw",
                "TargetProfile",
                "target_profile",
                "1.0.0",
                inputs.target_profile,
            ),
            compile_profile_hash: input_hash(
                "gbf-policy",
                "CompileProfileSpec",
                "compile_profile",
                "1.0.0",
                inputs.compile_profile,
            ),
            calibration_hash: None,
            compatibility_adapter_hash,
        },
        compatibility,
        checked_inputs: checked_inputs(inputs, artifact),
        diagnostics: diagnostics.clone(),
    };
    let report = ReportEnvelope::new(ReportOutcome::Failed, body)
        .expect("artifact_validation.v1 schema constants are valid");
    let (report, artifact_validation_self_hash, artifact_validation_canonical_bytes_hash) =
        finalize_report(report);

    ValidationStageFailure {
        report,
        diagnostics,
        artifact_validation_self_hash,
        artifact_validation_canonical_bytes_hash,
    }
}

fn checked_inputs(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
) -> ArtifactValidationInputSection {
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
        required_artifact_features: artifact.manifest.required_features.clone(),
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
        .body
        .validate_semantics(report.outcome)
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

fn input_hash<T: Serialize + ?Sized>(
    crate_name: &str,
    type_name: &str,
    schema_id: &str,
    schema_version: &str,
    value: &T,
) -> Hash256 {
    let encoded = canonical_input_json_bytes(value);
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}\0"
    ));
    hasher.update(encoded);
    Hash256::from_bytes(hasher.finalize().into())
}

fn canonical_input_json_bytes<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    let value = serde_json::to_value(value).expect("Stage 0 input identity serializes");
    canonicalize_value(&value).expect("Stage 0 input identity canonicalizes")
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
    use gbf_artifact::manifest::{
        ArtifactFeature, ArtifactSchemaVersion, ComponentId, ComponentKind, LineageId,
        ManifestComponent, ManifestTimestamp,
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
    fn f_b2_validate_validated_inputs_privacy_proof_lives_in_compile_fail_doctest() {
        // The module-level compile_fail doctest is the outside-module privacy proof.
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
    fn f_b2_validate_canonical_input_hash_has_known_byte_fixture() {
        let value = serde_json::json!({
            "zeta": 2,
            "alpha": {
                "b": 2,
                "a": 1,
            },
        });

        let canonical = canonical_input_json_bytes(&value);

        assert_eq!(canonical.as_slice(), br#"{"alpha":{"a":1,"b":2},"zeta":2}"#);
        assert_eq!(
            input_hash(
                "fixture-crate",
                "FixtureInput",
                "fixture_input",
                "1.0.0",
                &value,
            )
            .to_string(),
            "4dac1a04bf8464cc8239fa0a1feb1fa1dfa7b112599272d999ebc8634fcf6962"
        );
    }

    #[test]
    fn f_b2_validate_imported_artifact_view_normalizes_missing_hints() {
        let view = ImportedArtifactView::new(
            artifact_core(),
            artifact_manifest(),
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
    fn f_b2_validate_rejects_schema_epoch_unsupported() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 2, minor: 0 });

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(code, ValidationCode::SchemaEpochUnsupported)
        });
        assert!(matches!(
            failure.report.body.compatibility.failures.as_slice(),
            [ArtifactCompatibilityFailure::UnsupportedEpoch { .. }]
        ));
    }

    #[test]
    fn f_b2_validate_accepts_lossless_in_memory_schema_adapter() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 1, minor: 0 });

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert!(matches!(product.validated.artifact, Cow::Owned(_)));
        assert_eq!(
            product.validated.artifact.manifest.schema_version,
            CURRENT_ARTIFACT_SCHEMA_VERSION
        );
        assert!(matches!(
            product.report.body.compatibility.decision,
            Some(ArtifactCompatibilityDecision::LosslessInMemoryUpgrade { .. })
        ));
        assert!(
            product
                .validated
                .input_hashes
                .compatibility_adapter_hash
                .is_some()
        );
    }

    #[test]
    fn f_b2_validate_lossless_adapter_preserves_semantic_hash() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 1, minor: 0 });
        let before = fixture.artifact.core.semantic_hash();

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(product.validated.artifact.core.semantic_hash(), before);
        assert_eq!(
            product.report.body.identity.semantic_core_hash,
            Some(before)
        );
    }

    #[test]
    fn f_b2_validate_lossless_adapter_records_source_and_effective_hashes() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 1, minor: 0 });
        let source_hash = fixture.artifact.transport.transport_hash;

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(
            product.validated.input_hashes.artifact_source_hash,
            source_hash
        );
        assert_eq!(
            product.report.body.identity.artifact_source_hash,
            Some(source_hash)
        );
        assert_eq!(
            product.report.body.identity.artifact_effective_core_hash,
            Some(product.validated.artifact.core.semantic_hash())
        );
        assert_eq!(
            product.report.body.identity.compatibility_adapter_hash,
            product.validated.input_hashes.compatibility_adapter_hash
        );
    }

    #[test]
    fn f_b2_validate_rejects_lossy_schema_adapter() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 1, minor: 2 });

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::SchemaCompatibilityAdapterNotLossless { .. }
            )
        });
        assert!(matches!(
            failure.report.body.compatibility.failures.as_slice(),
            [ArtifactCompatibilityFailure::AdapterNotLossless { .. }]
        ));
    }

    #[test]
    fn f_b2_validate_rejects_lossless_adapter_that_changes_semantic_hash() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 1, minor: 3 });

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::SchemaCompatibilityAdapterNotLossless { .. }
            )
        });
        assert!(matches!(
            failure.report.body.compatibility.failures.as_slice(),
            [ArtifactCompatibilityFailure::SemanticHashChanged { .. }]
        ));
    }

    #[test]
    fn f_b2_validate_rejects_unregistered_schema_adapter() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 1, minor: 4 });

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::SchemaCompatibilityAdapterMissing { .. }
            )
        });
        assert!(matches!(
            failure.report.body.compatibility.failures.as_slice(),
            [ArtifactCompatibilityFailure::AdapterMissing { .. }]
        ));
    }

    #[test]
    fn f_b2_validate_rejects_cross_major_schema_adapter() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.set_schema_version(ArtifactSchemaVersion { epoch: 2, minor: 0 });

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(code, ValidationCode::SchemaEpochUnsupported)
        });
    }

    #[test]
    fn f_b2_validate_rejects_semantic_core_hash_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.manifest.semantic_core_hash = hash(0xee);
        fixture.refresh_manifest_self_hash();
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(code, ValidationCode::SemanticCoreHashMismatch)
        });
    }

    #[test]
    fn f_b2_validate_rejects_manifest_invariant_violated() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .artifact
            .manifest
            .components
            .push(ManifestComponent {
                digest: hash(0x44),
                id: ComponentId("tensor.missing".to_owned()),
                kind: ComponentKind::CanonicalTensor,
            });
        fixture.refresh_manifest_self_hash();
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ManifestInvariantViolated {
                    invariant: ManifestInvariant::RequiredComponentMissing { .. }
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_manifest_self_hash_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.manifest.manifest_self_hash = hash(0xef);
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ManifestInvariantViolated {
                    invariant: ManifestInvariant::ManifestSelfHashMismatch { .. }
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_transport_manifest_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.transport.transport_hash = hash(0xfa);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(code, ValidationCode::ArtifactTransportManifestMismatch)
        });
    }

    #[test]
    fn f_b2_validate_rejects_artifact_forbidden_build_identity_field() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .artifact
            .forbidden_build_identity_fields
            .insert(FieldPath::from("/build_identity"));
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactForbiddenBuildIdentityField { field }
                    if field == &FieldPath::from("/build_identity")
            )
        });
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
            let mut artifact = ImportedArtifactView::new(
                artifact_core(),
                artifact_manifest(),
                artifact_aux(),
                hint_bundle,
                None,
                transport_identity(),
            );
            artifact.transport.transport_hash = compute_imported_artifact_source_hash(&artifact);

            Self {
                artifact,
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

        fn set_schema_version(&mut self, version: ArtifactSchemaVersion) {
            self.artifact.manifest.schema_version = version;
            self.refresh_manifest_self_hash();
            self.refresh_transport_hash();
        }

        fn refresh_manifest_self_hash(&mut self) {
            self.artifact.manifest.manifest_self_hash =
                compute_artifact_manifest_self_hash(&self.artifact.manifest);
        }

        fn refresh_transport_hash(&mut self) {
            self.artifact.transport.transport_hash =
                compute_imported_artifact_source_hash(&self.artifact);
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

    fn artifact_manifest() -> ArtifactManifest {
        let mut manifest = ArtifactManifest {
            components: Vec::new(),
            created_at: ManifestTimestamp(0),
            lineage: LineageId(hash(0x08)),
            manifest_self_hash: Hash256::ZERO,
            required_features: BTreeSet::from([ArtifactFeature::DenseI8]),
            schema_version: CURRENT_ARTIFACT_SCHEMA_VERSION,
            semantic_core_hash: artifact_core().semantic_hash(),
        };
        manifest.manifest_self_hash = compute_artifact_manifest_self_hash(&manifest);
        manifest
    }

    fn transport_identity() -> ArtifactTransportIdentity {
        ArtifactTransportIdentity {
            source_uri: Some("fixture://artifact".to_owned()),
            transport_hash: Hash256::ZERO,
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

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn assert_failure_code(
        failure: &ValidationStageFailure,
        matches_code: impl Fn(&ValidationCode) -> bool,
    ) {
        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| matches_code(&diagnostic.code)),
            "diagnostics were {:#?}",
            failure.diagnostics
        );
    }
}
