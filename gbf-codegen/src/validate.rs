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
use gbf_artifact::core::{ArtifactCore, ArtifactCoreError};
use gbf_artifact::lowerings::{
    DataLoweringProfileId, LoweringManifest, LoweringShard, LoweringShardId, LoweringShardRef,
    Pack, Unpack,
};
use gbf_artifact::manifest::{
    ArtifactFeature, ArtifactSchemaVersion, ComponentKind, ManifestComponent, ManifestInvariant,
};
use gbf_artifact::sequence::SequenceSemanticsSpec;
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorPayload};
use gbf_artifact::{
    ArtifactAux, ArtifactManifest, EvidenceScope, HintBundle, HintScopeProvenance,
    TargetDataLoweringArtifact,
};
use gbf_foundation::{BlobCodec, BlobRef, Hash256, LayerId, PackerVersion, SemVer};
use gbf_hw::target::TargetProfile;
use gbf_policy::{
    CalibrationBundle, CalibrationBundleSet, CalibrationConfidenceRequirement, CalibrationLayer,
    CompatibilityAdapterId, CompileProfileSpec, CompileRequest, CompilerFeature,
    DiagnosticSeverity, EvidenceRef, FieldPath, ObjectiveRejection, RiskQuantileField, RuntimeMode,
    ServiceLevelField, Stage0Class10TargetCapabilities, TargetIncompatibilityReason, TraceProbeId,
    ValidationCode, ValidationDetail, ValidationDiagnostic as PolicyValidationDiagnostic,
    ValidationOrigin, compiler_build_supports_feature,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, serde::Deserialize)]
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
    #[cfg(test)]
    SemanticChangingProofFixture,
}

impl SchemaCompatibilityAdapter {
    fn apply(&self, source: &ImportedArtifactView) -> ImportedArtifactView {
        let mut upgraded = source.clone();
        upgraded.manifest.schema_version = self.to;
        match self.implementation {
            AdapterImplementation::SchemaVersionOnly => {}
            #[cfg(test)]
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
        .chain(test_schema_adapters())
        .find(|adapter| adapter.from == from && adapter.to == to)
}

fn builtin_schema_adapters() -> [SchemaCompatibilityAdapter; 1] {
    [SchemaCompatibilityAdapter {
        id: CompatibilityAdapterId("adapter.lossless".to_owned()),
        from: ArtifactSchemaVersion { epoch: 1, minor: 0 },
        to: CURRENT_ARTIFACT_SCHEMA_VERSION,
        lossless: true,
        implementation: AdapterImplementation::SchemaVersionOnly,
        implementation_id: "gbf-codegen.stage0.schema-v1-0-to-v1-1.lossless.v1",
    }]
}

#[cfg(test)]
fn test_schema_adapters() -> [SchemaCompatibilityAdapter; 3] {
    [
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

#[cfg(not(test))]
fn test_schema_adapters() -> std::iter::Empty<SchemaCompatibilityAdapter> {
    std::iter::empty()
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
) -> bool {
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
        return false;
    }
    true
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
    raw_forbidden_build_identity_fields(root, &value)
}

pub(crate) fn raw_forbidden_build_identity_fields(
    root: &str,
    value: &serde_json::Value,
) -> Vec<FieldPath> {
    let mut fields = Vec::new();
    collect_forbidden_build_identity_fields(value, root, &mut fields);
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

pub(crate) fn is_forbidden_build_identity_key(key: &str) -> bool {
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

fn validate_artifact_payload(
    artifact: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut rebuilt_tensors = Vec::with_capacity(artifact.core.tensors().len());
    let mut tensors_valid = true;
    for tensor in artifact.core.tensors() {
        let field = FieldPath::from(format!("core.tensors.{}.payload", tensor.id.as_str()));
        let rebuilt = gbf_artifact::tensor::CanonicalTensor::new(
            tensor.id.clone(),
            tensor.kind,
            tensor.layout.clone(),
            tensor.payload.clone(),
        );

        let Ok(rebuilt) = rebuilt else {
            tensors_valid = false;
            diagnostics.push(artifact_payload_malformed_diagnostic(
                field,
                artifact.manifest.manifest_self_hash,
            ));
            continue;
        };

        if rebuilt.content_hash != tensor.content_hash {
            diagnostics.push(ValidationDiagnostic::hard(
                ValidationOrigin::SemanticCore,
                ValidationCode::ArtifactBlobDigestMismatch {
                    blob: BlobRef {
                        hash: tensor.content_hash,
                        len: canonical_tensor_digest_material_len(tensor),
                        codec: BlobCodec::Raw,
                    },
                    expected: tensor.content_hash,
                    observed: rebuilt.content_hash,
                },
                ValidationDetail::HashMismatch {
                    expected: tensor.content_hash,
                    observed: rebuilt.content_hash,
                },
                vec![EvidenceRef {
                    kind: "artifact_tensor".to_owned(),
                    reference: tensor.id.to_string(),
                    hash: Some(artifact.manifest.manifest_self_hash),
                }],
            ));
        }
        rebuilt_tensors.push(rebuilt);
    }

    if tensors_valid {
        match ArtifactCore::new(
            rebuilt_tensors,
            artifact.core.quant().clone(),
            artifact.core.sequence_semantics(),
        ) {
            Ok(rebuilt_core) => {
                if rebuilt_core.semantic_hash() != artifact.core.semantic_hash() {
                    diagnostics.push(artifact_payload_malformed_diagnostic(
                        FieldPath::from("core"),
                        artifact.manifest.manifest_self_hash,
                    ));
                }
            }
            Err(error) => diagnostics.push(artifact_payload_malformed_diagnostic(
                artifact_core_error_field(&error),
                artifact.manifest.manifest_self_hash,
            )),
        }
    }

    validate_sequence_semantics_consistency(artifact, diagnostics);
}

fn canonical_tensor_digest_material_len(tensor: &CanonicalTensor) -> u32 {
    // Canonical tensors are embedded in ArtifactCore, so class 4 synthesizes
    // a BlobRef for diagnostics from the same digest material convention used
    // by gbf-artifact's canonical tensor content hash.
    let element_bytes = match &tensor.payload {
        CanonicalTensorPayload::F32(_) => 4_u128,
        CanonicalTensorPayload::I8(_) => 1_u128,
        CanonicalTensorPayload::U16(_) => 2_u128,
    };
    let len = 1_u128
        + 8
        + (tensor.layout.shape.dims().len() as u128 * 4)
        + 8
        + (tensor.payload.len() as u128 * element_bytes);
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn artifact_core_error_field(error: &ArtifactCoreError) -> FieldPath {
    match error {
        ArtifactCoreError::DuplicateTensor { id } => {
            FieldPath::from(format!("core.tensors.{}", id.as_str()))
        }
        ArtifactCoreError::DuplicateQuantEntry { kind, path } => FieldPath::from(format!(
            "core.quant.{}.{}",
            kind.replace(' ', "_"),
            path.as_str()
        )),
        ArtifactCoreError::MissingTensor { role, id } => FieldPath::from(format!(
            "core.quant.{}.{}",
            role.replace(' ', "_"),
            id.as_str()
        )),
        ArtifactCoreError::TensorKindMismatch { id, .. }
        | ArtifactCoreError::TensorElementTypeMismatch { id, .. }
        | ArtifactCoreError::TensorRankMismatch { id, .. }
        | ArtifactCoreError::TensorShapeMismatch { id, .. } => {
            FieldPath::from(format!("core.tensors.{}", id.as_str()))
        }
        ArtifactCoreError::InvalidActivationRange { activation } => FieldPath::from(format!(
            "core.quant.activation_quant.{}.range",
            activation.as_str()
        )),
        ArtifactCoreError::InvalidNormPlan { norm, .. } => {
            FieldPath::from(format!("core.quant.norm_plans.{}", norm.as_str()))
        }
        ArtifactCoreError::MissingNormLut { norm } => {
            FieldPath::from(format!("core.quant.norm_plans.{}.lut", norm.as_str()))
        }
        ArtifactCoreError::UnexpectedNormLut { norm, lut } => FieldPath::from(format!(
            "core.quant.norm_plans.{}.lut.{}",
            norm.as_str(),
            lut.as_str()
        )),
        ArtifactCoreError::MissingWeightQuantEntry { weight } => {
            FieldPath::from(format!("core.quant.weight_quant.{}", weight.as_str()))
        }
        ArtifactCoreError::MissingTernaryQuantEntry { weight } => FieldPath::from(format!(
            "core.quant.ternary_weight_plans.{}",
            weight.as_str()
        )),
        ArtifactCoreError::WeightQuantPlanMismatch { projection }
        | ArtifactCoreError::InvalidQuantPlan {
            path: projection, ..
        } => FieldPath::from(format!(
            "core.quant.ternary_weight_plans.{}",
            projection.as_str()
        )),
    }
}

fn validate_sequence_semantics_consistency(
    artifact: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let requires_linear = artifact
        .manifest
        .required_features
        .contains(&ArtifactFeature::LinearStateSequence);
    let requires_bounded = artifact
        .manifest
        .required_features
        .contains(&ArtifactFeature::BoundedKvSequence);

    if requires_linear && requires_bounded {
        diagnostics.push(artifact_payload_malformed_diagnostic(
            FieldPath::from("manifest.required_features.sequence_semantics"),
            artifact.manifest.manifest_self_hash,
        ));
        return;
    }

    match (
        artifact.core.sequence_semantics(),
        requires_linear,
        requires_bounded,
    ) {
        (SequenceSemanticsSpec::LinearState(_), true, false)
        | (SequenceSemanticsSpec::BoundedKv(_), false, true) => {}
        _ => {
            diagnostics.push(artifact_payload_malformed_diagnostic(
                FieldPath::from("core.sequence_semantics"),
                artifact.manifest.manifest_self_hash,
            ));
        }
    }
}

fn artifact_payload_malformed_diagnostic(
    field: FieldPath,
    provenance_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::SemanticCore,
        ValidationCode::ArtifactPayloadMalformed {
            field: field.clone(),
        },
        ValidationDetail::Field {
            field: field.clone(),
        },
        vec![EvidenceRef {
            kind: "artifact_core".to_owned(),
            reference: field.to_string(),
            hash: Some(provenance_hash),
        }],
    )
}

fn validate_artifact_aux_sidecars(
    artifact: &ImportedArtifactView,
    resolver: &dyn ArtifactResolver,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    validate_golden_vector_sidecars(
        &artifact.aux,
        artifact.manifest.manifest_self_hash,
        resolver,
        diagnostics,
    );

    if requires_checkpoint_schema(&artifact.manifest) && artifact.aux.checkpoint_schema.is_none() {
        diagnostics.push(artifact_aux_sidecar_missing_diagnostic(
            SidecarKind::SemanticCheckpointSchema,
            artifact.manifest.manifest_self_hash,
        ));
    }

    if requires_interaction_bundle(&artifact.manifest) && artifact.aux.interaction_bundle.is_none()
    {
        diagnostics.push(artifact_aux_sidecar_missing_diagnostic(
            SidecarKind::InteractionBundle,
            artifact.manifest.manifest_self_hash,
        ));
    }

    if let Some(sidecar) = &artifact.aux.checkpoint_schema {
        validate_resolved_sidecar(
            SidecarRef {
                kind: SidecarKind::SemanticCheckpointSchema,
                id: sidecar.id.0.clone(),
                hash: sidecar.hash,
            },
            artifact.manifest.manifest_self_hash,
            resolver,
            diagnostics,
        );
    }
    if let Some(sidecar) = &artifact.aux.conformance_envelope {
        validate_resolved_sidecar(
            SidecarRef {
                kind: SidecarKind::ConformanceEnvelope,
                id: sidecar.id.0.clone(),
                hash: sidecar.hash,
            },
            artifact.manifest.manifest_self_hash,
            resolver,
            diagnostics,
        );
    }
    if let Some(sidecar) = &artifact.aux.reference_observation_cache {
        validate_resolved_sidecar(
            SidecarRef {
                kind: SidecarKind::ReferenceObservationCache,
                id: sidecar.id.0.clone(),
                hash: sidecar.hash,
            },
            artifact.manifest.manifest_self_hash,
            resolver,
            diagnostics,
        );
    }
    if let Some(sidecar) = &artifact.aux.interaction_bundle {
        validate_resolved_sidecar(
            SidecarRef {
                kind: SidecarKind::InteractionBundle,
                id: sidecar.id.0.clone(),
                hash: sidecar.hash,
            },
            artifact.manifest.manifest_self_hash,
            resolver,
            diagnostics,
        );
    }
    if let Some(sidecar) = &artifact.aux.lexical_spec {
        validate_resolved_sidecar(
            SidecarRef {
                kind: SidecarKind::LexicalSpec,
                id: sidecar.id.0.clone(),
                hash: sidecar.hash,
            },
            artifact.manifest.manifest_self_hash,
            resolver,
            diagnostics,
        );
    }
}

fn validate_golden_vector_sidecars(
    aux: &ArtifactAux,
    manifest_self_hash: Hash256,
    resolver: &dyn ArtifactResolver,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut seen = BTreeSet::new();
    for vector in &aux.golden_vectors {
        if !seen.insert(vector.id.clone()) {
            diagnostics.push(artifact_aux_malformed_diagnostic(
                golden_vector_aux_field(&vector.id),
                manifest_self_hash,
            ));
            continue;
        }
        validate_golden_vector_presence(vector, manifest_self_hash, resolver, diagnostics);
    }
}

fn validate_resolved_sidecar(
    sidecar: SidecarRef,
    manifest_self_hash: Hash256,
    resolver: &dyn ArtifactResolver,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match resolver.resolve_sidecar(&sidecar) {
        Ok(resolved) => {
            let observed = sha256_hash(&resolved.bytes);
            if observed != sidecar.hash {
                diagnostics.push(ValidationDiagnostic::hard(
                    ValidationOrigin::Manifest,
                    ValidationCode::ArtifactAuxSidecarDigestMismatch {
                        kind: sidecar.kind,
                        expected: sidecar.hash,
                        observed,
                    },
                    ValidationDetail::HashMismatch {
                        expected: sidecar.hash,
                        observed,
                    },
                    vec![EvidenceRef {
                        kind: "artifact_aux_sidecar".to_owned(),
                        reference: sidecar.id,
                        hash: Some(sidecar.hash),
                    }],
                ));
            }
            // The current gbf-artifact aux schema exposes these sidecars as
            // ref placeholders only. Concrete body parsing, including
            // interaction-bundle malformed checks, belongs with the schema
            // owner bead that introduces typed sidecar payloads.
        }
        Err(ArtifactResolveError::NotFound { .. }) => {
            diagnostics.push(artifact_aux_sidecar_missing_diagnostic(
                sidecar.kind,
                manifest_self_hash,
            ));
        }
        Err(ArtifactResolveError::HashMismatch {
            expected, observed, ..
        }) => diagnostics.push(ValidationDiagnostic::hard(
            ValidationOrigin::Manifest,
            ValidationCode::ArtifactAuxSidecarDigestMismatch {
                kind: sidecar.kind,
                expected,
                observed,
            },
            ValidationDetail::HashMismatch { expected, observed },
            vec![EvidenceRef {
                kind: "artifact_aux_sidecar".to_owned(),
                reference: sidecar.id,
                hash: Some(expected),
            }],
        )),
        Err(ArtifactResolveError::Unsupported { .. }) => {
            diagnostics.push(artifact_aux_malformed_diagnostic(
                FieldPath::from(format!("aux.sidecars.{}", sidecar.id)),
                manifest_self_hash,
            ));
        }
    }
}

fn validate_golden_vector_presence(
    vector: &GoldenVectorRef,
    manifest_self_hash: Hash256,
    resolver: &dyn ArtifactResolver,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match resolver.resolve_golden_vector(vector) {
        Ok(_) => {}
        Err(ArtifactResolveError::NotFound { .. }) => diagnostics.push(
            artifact_aux_sidecar_missing_diagnostic(SidecarKind::GoldenVector, manifest_self_hash),
        ),
        Err(ArtifactResolveError::HashMismatch { .. })
        | Err(ArtifactResolveError::Unsupported { .. }) => {}
    }
}

fn requires_checkpoint_schema(manifest: &ArtifactManifest) -> bool {
    // These sequence features persist resumable state, so Stage 0 requires the
    // aux checkpoint schema sidecar before later stages can rely on the state shape.
    manifest
        .required_features
        .contains(&ArtifactFeature::LinearStateSequence)
        || manifest
            .required_features
            .contains(&ArtifactFeature::BoundedKvSequence)
}

fn requires_interaction_bundle(manifest: &ArtifactManifest) -> bool {
    // MoE routing depends on interaction metadata that is carried out-of-band
    // from the semantic core, so its aux sidecar is required when the feature is active.
    manifest
        .required_features
        .contains(&ArtifactFeature::MoeRouting)
}

fn golden_vector_aux_field(id: &GoldenVectorId) -> FieldPath {
    FieldPath::from(format!("aux.golden_vectors.{}", id.0))
}

fn artifact_aux_sidecar_missing_diagnostic(
    kind: SidecarKind,
    provenance_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Manifest,
        ValidationCode::ArtifactAuxSidecarMissing { kind },
        ValidationDetail::Field {
            field: FieldPath::from("aux"),
        },
        vec![EvidenceRef {
            kind: "artifact_aux".to_owned(),
            reference: format!("{kind:?}"),
            hash: Some(provenance_hash),
        }],
    )
}

fn artifact_aux_malformed_diagnostic(
    field: FieldPath,
    provenance_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Manifest,
        ValidationCode::ArtifactAuxMalformed {
            field: field.clone(),
        },
        ValidationDetail::Field {
            field: field.clone(),
        },
        vec![EvidenceRef {
            kind: "artifact_aux".to_owned(),
            reference: field.to_string(),
            hash: Some(provenance_hash),
        }],
    )
}

fn validate_target_data_lowering(
    inputs: &ValidateInputs<'_>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let target = inputs.target_profile.id();
    let expected_profile = expected_lowering_profile(inputs);
    let Some(lowering) = inputs.lowerings.iter().find(|lowering| {
        lowering.target.as_str() == target.as_str() && lowering.profile == expected_profile
    }) else {
        diagnostics.push(ValidationDiagnostic::hard(
            ValidationOrigin::Lowering,
            ValidationCode::LoweringMissingForTarget {
                target: target.clone(),
                lowering_profile: expected_profile,
            },
            ValidationDetail::Field {
                field: FieldPath::from("lowerings"),
            },
            vec![EvidenceRef {
                kind: "target_profile".to_owned(),
                reference: target.to_string(),
                hash: Some(input_hash(
                    "gbf-hw",
                    "TargetProfile",
                    "target_profile",
                    "1.0.0",
                    inputs.target_profile,
                )),
            }],
        ));
        return;
    };

    let runtime_version = gbf_runtime::RUNTIME_PACKER_VERSION;
    if lowering.packer_version != runtime_version {
        diagnostics.push(ValidationDiagnostic::hard(
            ValidationOrigin::Lowering,
            ValidationCode::LoweringPackerVersionMismatch {
                artifact_version: lowering.packer_version,
                runtime_version,
            },
            ValidationDetail::Field {
                field: FieldPath::from("lowerings.packer_version"),
            },
            vec![lowering_evidence(lowering, "packer_version")],
        ));
    }

    if lowering.shards.is_empty() {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            lowering_manifest_diagnostic_ref(lowering),
            ValidationDetail::Field {
                field: FieldPath::from("lowerings.shards"),
            },
            lowering_evidence(lowering, "shards"),
        ));
        return;
    }

    for shard in &lowering.shards {
        validate_lowering_shard_round_trip(lowering, shard, diagnostics);
    }
    validate_lowering_manifest_round_trip(lowering, diagnostics);
}

fn expected_lowering_profile(inputs: &ValidateInputs<'_>) -> DataLoweringProfileId {
    DataLoweringProfileId(format!(
        "{}-default",
        inputs.target_profile.family().as_str()
    ))
}

fn validate_lowering_shard_round_trip(
    lowering: &TargetDataLoweringArtifact,
    shard: &LoweringShard,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let shard_ref = lowering_shard_ref(shard);
    let Ok(packed) = shard.pack() else {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            shard_ref,
            ValidationDetail::Field {
                field: FieldPath::from(format!("lowerings.shards.{}", shard.id.0)),
            },
            lowering_evidence(lowering, shard.id.0.as_str()),
        ));
        return;
    };

    let observed_packed_hash = sha256_hash(&packed);
    if observed_packed_hash != shard.packed_bytes_hash {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            shard_ref,
            ValidationDetail::HashMismatch {
                expected: shard.packed_bytes_hash,
                observed: observed_packed_hash,
            },
            lowering_evidence(lowering, shard.id.0.as_str()),
        ));
        return;
    }

    let Ok(unpacked) = LoweringShard::unpack(&packed) else {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            shard_ref,
            ValidationDetail::Field {
                field: FieldPath::from(format!("lowerings.shards.{}", shard.id.0)),
            },
            lowering_evidence(lowering, shard.id.0.as_str()),
        ));
        return;
    };
    let Ok(repacked) = unpacked.pack() else {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            shard_ref,
            ValidationDetail::Field {
                field: FieldPath::from(format!("lowerings.shards.{}", shard.id.0)),
            },
            lowering_evidence(lowering, shard.id.0.as_str()),
        ));
        return;
    };

    if repacked != packed || unpacked != *shard {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            shard_ref,
            ValidationDetail::Field {
                field: FieldPath::from(format!("lowerings.shards.{}", shard.id.0)),
            },
            lowering_evidence(lowering, shard.id.0.as_str()),
        ));
    }
}

fn validate_lowering_manifest_round_trip(
    lowering: &TargetDataLoweringArtifact,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let manifest = assembled_lowering_manifest(lowering);
    let manifest_ref = lowering_manifest_diagnostic_ref(lowering);
    let Ok(packed) = manifest.pack() else {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            manifest_ref,
            ValidationDetail::Field {
                field: FieldPath::from("lowerings.manifest"),
            },
            lowering_evidence(lowering, "manifest"),
        ));
        return;
    };

    let observed_manifest_hash = sha256_hash(&packed);
    if observed_manifest_hash != lowering.manifest_hash {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            manifest_ref,
            ValidationDetail::HashMismatch {
                expected: lowering.manifest_hash,
                observed: observed_manifest_hash,
            },
            lowering_evidence(lowering, "manifest_hash"),
        ));
        return;
    }

    let Ok(unpacked) = LoweringManifest::unpack(&packed) else {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            manifest_ref,
            ValidationDetail::Field {
                field: FieldPath::from("lowerings.manifest"),
            },
            lowering_evidence(lowering, "manifest"),
        ));
        return;
    };
    let Ok(repacked) = unpacked.pack() else {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            manifest_ref,
            ValidationDetail::Field {
                field: FieldPath::from("lowerings.manifest"),
            },
            lowering_evidence(lowering, "manifest"),
        ));
        return;
    };

    if repacked != packed || unpacked != manifest {
        diagnostics.push(lowering_round_trip_failed_diagnostic(
            manifest_ref,
            ValidationDetail::Field {
                field: FieldPath::from("lowerings.manifest"),
            },
            lowering_evidence(lowering, "manifest"),
        ));
    }
}

#[derive(Clone, Copy)]
struct ActiveCalibrationBinding {
    target_profile_hash: Hash256,
    kernel_set_hash: Hash256,
    packer_version: PackerVersion,
    calibration_schema_hash: Hash256,
    compile_request_hash: Hash256,
    calibration_set_hash: Hash256,
}

fn validate_calibration_binding(
    inputs: &ValidateInputs<'_>,
    calibration: &CalibrationBundleSet,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let active = active_calibration_binding(inputs, calibration);
    for layer in CalibrationLayer::all() {
        let Some(requested_ref) =
            requested_calibration_ref(&inputs.compile_request.calibration_set_ref, layer)
        else {
            diagnostics.push(calibration_missing_request_ref_diagnostic(
                layer,
                active.compile_request_hash,
            ));
            continue;
        };
        if !requested_ref.is_resolved_by(&calibration.resolved_ref) {
            diagnostics.push(calibration_missing_request_ref_diagnostic(
                layer,
                active.compile_request_hash,
            ));
            continue;
        }

        if !calibration.bundles.contains_key(&layer) {
            diagnostics.push(calibration_missing_layer_diagnostic(
                layer,
                active.calibration_set_hash,
            ));
            continue;
        }
    }

    for bundle in calibration.bundles.values() {
        validate_calibration_bundle_freshness(bundle, active, diagnostics);
        validate_calibration_bundle_confidence(
            inputs,
            bundle,
            active.calibration_set_hash,
            diagnostics,
        );
    }
}

fn active_calibration_binding(
    inputs: &ValidateInputs<'_>,
    calibration: &CalibrationBundleSet,
) -> ActiveCalibrationBinding {
    ActiveCalibrationBinding {
        target_profile_hash: input_hash(
            "gbf-hw",
            "TargetProfile",
            "target_profile",
            "1.0.0",
            inputs.target_profile,
        ),
        // TODO(bd-2fj): replace this chunk-local sentinel when Stage 0 has a
        // resolved kernel-set identity input.
        kernel_set_hash: Hash256::ZERO,
        packer_version: gbf_runtime::RUNTIME_PACKER_VERSION,
        // TODO(bd-2sab): replace this sentinel when the calibration schema
        // publishes a stable schema-epoch identity hash.
        calibration_schema_hash: Hash256::ZERO,
        compile_request_hash: input_hash(
            "gbf-policy",
            "CompileRequest",
            "compile_request",
            "1.0.0",
            inputs.compile_request,
        ),
        calibration_set_hash: input_hash(
            "gbf-policy",
            "CalibrationBundleSet",
            "calibration",
            "1.0.0",
            calibration,
        ),
    }
}

fn validate_calibration_bundle_freshness(
    bundle: &CalibrationBundle,
    active: ActiveCalibrationBinding,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    push_calibration_stale_if_hash_mismatch(
        bundle,
        "target_profile_hash",
        bundle.target_profile_hash,
        active.target_profile_hash,
        active.calibration_set_hash,
        diagnostics,
    );
    push_calibration_stale_if_hash_mismatch(
        bundle,
        "kernel_set_hash",
        bundle.kernel_set_hash,
        active.kernel_set_hash,
        active.calibration_set_hash,
        diagnostics,
    );
    if bundle.packer_version != active.packer_version {
        diagnostics.push(calibration_stale_diagnostic(
            bundle.layer,
            "packer_version",
            packer_version_freshness_hash(&bundle.packer_version),
            packer_version_freshness_hash(&active.packer_version),
            active.calibration_set_hash,
        ));
    }
    push_calibration_stale_if_hash_mismatch(
        bundle,
        "calibration_schema_hash",
        bundle.calibration_schema_hash,
        active.calibration_schema_hash,
        active.calibration_set_hash,
        diagnostics,
    );
}

#[derive(Clone, Copy)]
enum RequestedCalibrationRef<'a> {
    Platform(&'a gbf_foundation::PlatformCalibrationId),
    Kernel(&'a gbf_foundation::KernelCalibrationId),
    Runtime(&'a gbf_foundation::RuntimeCalibrationId),
}

impl RequestedCalibrationRef<'_> {
    fn is_resolved_by(self, resolved_ref: &gbf_hw::calibration::CalibrationSetRef) -> bool {
        match self {
            Self::Platform(id) => resolved_ref.platform.as_ref() == Some(id),
            Self::Kernel(id) => resolved_ref.kernel.as_ref() == Some(id),
            Self::Runtime(id) => resolved_ref.runtime.as_ref() == Some(id),
        }
    }
}

fn requested_calibration_ref(
    set_ref: &gbf_hw::calibration::CalibrationSetRef,
    layer: CalibrationLayer,
) -> Option<RequestedCalibrationRef<'_>> {
    match layer {
        CalibrationLayer::Platform => set_ref
            .platform
            .as_ref()
            .map(RequestedCalibrationRef::Platform),
        CalibrationLayer::Kernel => set_ref.kernel.as_ref().map(RequestedCalibrationRef::Kernel),
        CalibrationLayer::Runtime => set_ref
            .runtime
            .as_ref()
            .map(RequestedCalibrationRef::Runtime),
    }
}

fn push_calibration_stale_if_hash_mismatch(
    bundle: &CalibrationBundle,
    field: &'static str,
    declared: Hash256,
    observed: Hash256,
    calibration_set_hash: Hash256,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if declared != observed {
        diagnostics.push(calibration_stale_diagnostic(
            bundle.layer,
            field,
            declared,
            observed,
            calibration_set_hash,
        ));
    }
}

fn packer_version_freshness_hash(version: &PackerVersion) -> Hash256 {
    input_hash(
        "gbf-foundation",
        "PackerVersion",
        "calibration.packer_version",
        "1.0.0",
        version,
    )
}

fn validate_calibration_bundle_confidence(
    inputs: &ValidateInputs<'_>,
    bundle: &CalibrationBundle,
    calibration_set_hash: Hash256,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let requirement = inputs
        .compile_profile
        .risk_policy
        .calibration_confidence_requirement;
    let CalibrationConfidenceRequirement::AtLeast { class: required } = requirement else {
        return;
    };
    let observed = bundle.confidence;
    if requirement.accepts(observed) {
        return;
    }

    diagnostics.push(ValidationDiagnostic::hard(
        ValidationOrigin::Calibration,
        ValidationCode::CalibrationConfidenceTooLow { required, observed },
        ValidationDetail::Field {
            field: calibration_field(bundle.layer, "confidence"),
        },
        vec![calibration_evidence(
            bundle.layer,
            "confidence",
            calibration_set_hash,
        )],
    ));
}

fn calibration_missing_layer_diagnostic(
    layer: CalibrationLayer,
    calibration_set_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Calibration,
        ValidationCode::CalibrationMissing { class: layer },
        ValidationDetail::Field {
            field: calibration_field(layer, "bundle"),
        },
        vec![calibration_evidence(layer, "bundle", calibration_set_hash)],
    )
}

fn calibration_missing_request_ref_diagnostic(
    layer: CalibrationLayer,
    compile_request_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Calibration,
        ValidationCode::CalibrationMissing { class: layer },
        ValidationDetail::Field {
            field: FieldPath::from(format!(
                "compile_request.calibration_set_ref.{}",
                calibration_request_ref_field(layer)
            )),
        },
        vec![EvidenceRef {
            kind: "compile_request".to_owned(),
            reference: format!(
                "calibration_set_ref.{}",
                calibration_request_ref_field(layer)
            ),
            hash: Some(compile_request_hash),
        }],
    )
}

const fn calibration_request_ref_field(layer: CalibrationLayer) -> &'static str {
    match layer {
        CalibrationLayer::Platform => "platform",
        CalibrationLayer::Kernel => "kernel",
        CalibrationLayer::Runtime => "runtime",
    }
}

fn calibration_stale_diagnostic(
    layer: CalibrationLayer,
    field: &'static str,
    declared: Hash256,
    observed: Hash256,
    calibration_set_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Calibration,
        ValidationCode::CalibrationStale {
            class: layer,
            declared,
            observed,
        },
        ValidationDetail::HashMismatch {
            expected: declared,
            observed,
        },
        vec![calibration_evidence(layer, field, calibration_set_hash)],
    )
}

fn calibration_evidence(
    layer: CalibrationLayer,
    reference: &'static str,
    calibration_set_hash: Hash256,
) -> EvidenceRef {
    EvidenceRef {
        kind: "calibration_bundle".to_owned(),
        reference: format!("{}.{}", layer.as_str(), reference),
        hash: Some(calibration_set_hash),
    }
}

fn calibration_field(layer: CalibrationLayer, field: &'static str) -> FieldPath {
    FieldPath::from(format!("calibration.{}.{}", layer.as_str(), field))
}

fn validate_hint_provenance(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let hint_bundle_hash = artifact.hint_bundle_hash();
    let active_layers = active_layer_ids(artifact);
    let active_lowering = active_lowering(inputs);
    validate_hint_provenance_ids_unique(artifact, hint_bundle_hash, diagnostics);

    for entry in &artifact.hint_bundle.facts.scope_provenance {
        validate_scoped_hint_entry(
            "facts",
            entry,
            inputs,
            &active_layers,
            active_lowering,
            hint_bundle_hash,
            diagnostics,
        );
    }

    for entry in artifact.hint_bundle.preferences.scope_provenance() {
        validate_scoped_hint_entry(
            "preferences",
            entry,
            inputs,
            &active_layers,
            active_lowering,
            hint_bundle_hash,
            diagnostics,
        );
    }

    for entry in &artifact.hint_bundle.constraints.entries {
        if evidence_scope_applies_to_active_build(
            &entry.scope,
            inputs,
            &active_layers,
            active_lowering,
        ) {
            continue;
        }

        diagnostics.push(hint_provenance_inconsistent_diagnostic(
            entry.provenance_id,
            FieldPath::from(format!(
                "hint_bundle.constraints.entries[provenance_id={}].scope",
                entry.provenance_id.0
            )),
            &entry.scope,
            hint_bundle_hash,
        ));
    }
}

fn validate_hint_provenance_ids_unique(
    artifact: &ImportedArtifactView,
    hint_bundle_hash: Hash256,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut seen = BTreeSet::new();

    for entry in &artifact.hint_bundle.facts.scope_provenance {
        validate_hint_provenance_id_unique(
            &mut seen,
            entry.provenance_id,
            FieldPath::from(format!("hint_bundle.facts.{}", entry.field)),
            &entry.scope,
            hint_bundle_hash,
            diagnostics,
        );
    }

    for entry in artifact.hint_bundle.preferences.scope_provenance() {
        validate_hint_provenance_id_unique(
            &mut seen,
            entry.provenance_id,
            FieldPath::from(format!("hint_bundle.preferences.{}", entry.field)),
            &entry.scope,
            hint_bundle_hash,
            diagnostics,
        );
    }

    for entry in &artifact.hint_bundle.constraints.entries {
        validate_hint_provenance_id_unique(
            &mut seen,
            entry.provenance_id,
            FieldPath::from(format!(
                "hint_bundle.constraints.entries[provenance_id={}].scope",
                entry.provenance_id.0
            )),
            &entry.scope,
            hint_bundle_hash,
            diagnostics,
        );
    }
}

fn validate_hint_provenance_id_unique(
    seen: &mut BTreeSet<TraceProbeId>,
    provenance_id: TraceProbeId,
    field: FieldPath,
    scope: &EvidenceScope,
    hint_bundle_hash: Hash256,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if seen.insert(provenance_id) {
        return;
    }

    diagnostics.push(hint_provenance_inconsistent_diagnostic(
        provenance_id,
        field,
        scope,
        hint_bundle_hash,
    ));
}

fn validate_scoped_hint_entry(
    bucket: &'static str,
    entry: &HintScopeProvenance,
    inputs: &ValidateInputs<'_>,
    active_layers: &BTreeSet<LayerId>,
    active_lowering: Option<&TargetDataLoweringArtifact>,
    hint_bundle_hash: Hash256,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if evidence_scope_applies_to_active_build(&entry.scope, inputs, active_layers, active_lowering)
    {
        return;
    }

    diagnostics.push(hint_provenance_inconsistent_diagnostic(
        entry.provenance_id,
        FieldPath::from(format!("hint_bundle.{bucket}.{}", entry.field)),
        &entry.scope,
        hint_bundle_hash,
    ));
}

fn evidence_scope_applies_to_active_build(
    scope: &EvidenceScope,
    inputs: &ValidateInputs<'_>,
    active_layers: &BTreeSet<LayerId>,
    active_lowering: Option<&TargetDataLoweringArtifact>,
) -> bool {
    match scope {
        EvidenceScope::WholeArtifact => true,
        EvidenceScope::LayerScoped { layer } => active_layers.contains(layer),
        EvidenceScope::TargetFamily { family } => family == inputs.target_profile.family(),
        EvidenceScope::WorkloadScoped { workload } => {
            inputs.workloads.iter().any(|active| &active.id == workload)
        }
        EvidenceScope::LoweringScoped { shard } => active_lowering
            .map(|lowering| lowering.shards.iter().any(|active| &active.id == shard))
            .unwrap_or(false),
    }
}

fn active_lowering<'a>(inputs: &'a ValidateInputs<'_>) -> Option<&'a TargetDataLoweringArtifact> {
    let target = inputs.target_profile.id();
    let expected_profile = expected_lowering_profile(inputs);
    inputs.lowerings.iter().find(|lowering| {
        lowering.target.as_str() == target.as_str() && lowering.profile == expected_profile
    })
}

fn active_layer_ids(artifact: &ImportedArtifactView) -> BTreeSet<LayerId> {
    artifact
        .core
        .tensors()
        .iter()
        .filter_map(|tensor| layer_id_from_artifact_path(tensor.id.as_str()))
        .collect()
}

/// Temporary Stage-0 layer inventory until artifact core exposes typed layer ids.
///
/// The accepted v1 tensor-path grammar is segment based:
/// `... . layer . <u16> . ...`, for example `model.layer.3.weight`.
/// Shorthand such as `layer3` is intentionally not parsed here.
fn layer_id_from_artifact_path(path: &str) -> Option<LayerId> {
    let mut segments = path.split('.');
    while let Some(segment) = segments.next() {
        if segment != "layer" {
            continue;
        }
        let layer = segments.next()?.parse::<u16>().ok()?;
        return Some(LayerId::new(layer));
    }
    None
}

fn hint_provenance_inconsistent_diagnostic(
    provenance_id: TraceProbeId,
    field: FieldPath,
    scope: &EvidenceScope,
    hint_bundle_hash: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::HintBundle,
        ValidationCode::HintProvenanceInconsistent {
            fact: provenance_id,
        },
        ValidationDetail::Field { field },
        vec![EvidenceRef {
            kind: "hint_bundle".to_owned(),
            reference: format!(
                "provenance_id={};scope={}",
                provenance_id.0,
                evidence_scope_reference(scope)
            ),
            hash: Some(hint_bundle_hash),
        }],
    )
}

fn evidence_scope_reference(scope: &EvidenceScope) -> String {
    match scope {
        EvidenceScope::WholeArtifact => "whole_artifact".to_owned(),
        EvidenceScope::LayerScoped { layer } => format!("layer:{layer}"),
        EvidenceScope::TargetFamily { family } => format!("target_family:{family}"),
        EvidenceScope::WorkloadScoped { workload } => format!("workload:{workload}"),
        EvidenceScope::LoweringScoped { shard } => format!("lowering_shard:{}", shard.0),
    }
}

fn validate_workload_and_golden_refs(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let mut golden_refs = ActiveGoldenVectorRefs::default();
    for vector in inputs.golden_vectors {
        golden_refs.insert(vector);
    }
    for vector in &artifact.aux.golden_vectors {
        golden_refs.insert(vector);
    }

    for workload in inputs.workloads {
        if let Some(resolved) = validate_workload_ref(inputs.resolver, workload, diagnostics) {
            for vector in &resolved.manifest.golden_vectors {
                golden_refs.insert(vector);
            }
        }
    }
    for vector in golden_refs.iter() {
        validate_golden_vector_ref(inputs.resolver, vector, diagnostics);
    }
}

fn validate_compile_request_admissibility(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let target_capabilities =
        Stage0Class10TargetCapabilities::from_target_profile(inputs.target_profile);
    for feature in &artifact.manifest.required_features {
        if !target_capabilities.supports_artifact_feature(*feature) {
            diagnostics.push(artifact_required_feature_unsupported_diagnostic(
                inputs, artifact, *feature,
            ));
        }
    }

    for feature in &inputs.compile_request.required_features {
        if !compiler_build_supports_feature(*feature) {
            diagnostics.push(compile_request_unsupported_feature_diagnostic(
                inputs, *feature,
            ));
        }
    }

    for reason in profile_objective_rejections(inputs.compile_request) {
        diagnostics.push(compile_request_profile_forbids_objective_diagnostic(
            inputs, reason,
        ));
    }

    for mode in &inputs.compile_request.requested_runtime_modes {
        if !target_capabilities.supports_runtime_mode(*mode) {
            diagnostics.push(compile_request_runtime_mode_unsupported_diagnostic(
                inputs, *mode,
            ));
        }
    }

    if let Err(reason) = target_capabilities.target_compatibility(&inputs.compile_request.target) {
        diagnostics.push(compile_request_target_incompatible_diagnostic(
            inputs, reason,
        ));
    }
}

fn profile_objective_rejections(request: &CompileRequest) -> Vec<ObjectiveRejection> {
    let objective = &request.objective;
    let mut rejections = Vec::new();

    if let Some(service) = &objective.service {
        for (field, value) in [
            (
                ServiceLevelField::MaxFirstTokenCyclesP95,
                service.max_first_token_cycles_p95,
            ),
            (
                ServiceLevelField::MaxCheckpointGapCyclesP95,
                service.max_checkpoint_gap_cycles_p95,
            ),
            (
                ServiceLevelField::MaxResumeLatencyCyclesP95,
                service.max_resume_latency_cycles_p95,
            ),
        ] {
            if value == Some(0) {
                rejections.push(ObjectiveRejection::ServiceLevelZero { field });
            }
        }

        if service.max_ui_jitter_frames_p99 == Some(0) {
            rejections.push(ObjectiveRejection::ServiceLevelZero {
                field: ServiceLevelField::MaxUiJitterFramesP99,
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

    let risk = &objective.risk;
    if !(1..=100).contains(&risk.cycle_quantile) {
        rejections.push(ObjectiveRejection::RiskQuantileInvalid {
            field: RiskQuantileField::CycleQuantile,
            value: risk.cycle_quantile,
        });
    }

    if !(1..=100).contains(&risk.switch_quantile) {
        rejections.push(ObjectiveRejection::RiskQuantileInvalid {
            field: RiskQuantileField::SwitchQuantile,
            value: risk.switch_quantile,
        });
    }

    rejections
}

fn artifact_required_feature_unsupported_diagnostic(
    inputs: &ValidateInputs<'_>,
    artifact: &ImportedArtifactView,
    feature: ArtifactFeature,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::CompileRequest,
        ValidationCode::ArtifactRequiredFeatureUnsupported { feature },
        ValidationDetail::Field {
            field: FieldPath::from("manifest.required_features"),
        },
        vec![
            EvidenceRef {
                kind: "artifact_manifest".to_owned(),
                reference: format!("required_features.{feature:?}"),
                hash: Some(artifact.manifest.manifest_self_hash),
            },
            target_profile_evidence(inputs),
        ],
    )
}

fn compile_request_unsupported_feature_diagnostic(
    inputs: &ValidateInputs<'_>,
    feature: CompilerFeature,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::CompileRequest,
        ValidationCode::CompileRequestUnsupportedFeature { feature },
        ValidationDetail::Field {
            field: FieldPath::from("compile_request.required_features"),
        },
        vec![compile_request_evidence(inputs, "required_features")],
    )
}

fn compile_request_profile_forbids_objective_diagnostic(
    inputs: &ValidateInputs<'_>,
    reason: ObjectiveRejection,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::CompileRequest,
        ValidationCode::CompileRequestProfileForbidsObjective {
            profile: inputs.compile_profile.id.clone(),
            reason,
        },
        ValidationDetail::Field {
            field: FieldPath::from("compile_request.objective"),
        },
        vec![
            compile_request_evidence(inputs, "objective"),
            compile_profile_evidence(inputs),
        ],
    )
}

fn compile_request_runtime_mode_unsupported_diagnostic(
    inputs: &ValidateInputs<'_>,
    mode: RuntimeMode,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::CompileRequest,
        ValidationCode::CompileRequestRuntimeModeUnsupported { mode },
        ValidationDetail::Field {
            field: FieldPath::from("compile_request.requested_runtime_modes"),
        },
        vec![
            compile_request_evidence(inputs, "requested_runtime_modes"),
            target_profile_evidence(inputs),
        ],
    )
}

fn compile_request_target_incompatible_diagnostic(
    inputs: &ValidateInputs<'_>,
    reason: TargetIncompatibilityReason,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::CompileRequest,
        ValidationCode::CompileRequestTargetIncompatible {
            target: inputs.compile_request.target.clone(),
            reason,
        },
        ValidationDetail::Field {
            field: FieldPath::from("compile_request.target"),
        },
        vec![
            compile_request_evidence(inputs, "target"),
            target_profile_evidence(inputs),
        ],
    )
}

fn compile_request_evidence(inputs: &ValidateInputs<'_>, reference: &'static str) -> EvidenceRef {
    EvidenceRef {
        kind: "compile_request".to_owned(),
        reference: reference.to_owned(),
        hash: Some(input_hash(
            "gbf-policy",
            "CompileRequest",
            "compile_request",
            "1.0.0",
            inputs.compile_request,
        )),
    }
}

fn target_profile_evidence(inputs: &ValidateInputs<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "target_profile".to_owned(),
        reference: inputs.target_profile.id().to_string(),
        hash: Some(input_hash(
            "gbf-hw",
            "TargetProfile",
            "target_profile",
            "1.0.0",
            inputs.target_profile,
        )),
    }
}

fn compile_profile_evidence(inputs: &ValidateInputs<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "compile_profile".to_owned(),
        reference: inputs.compile_profile.id.to_string(),
        hash: Some(input_hash(
            "gbf-policy",
            "CompileProfileSpec",
            "compile_profile",
            "1.0.0",
            inputs.compile_profile,
        )),
    }
}

#[derive(Default)]
struct ActiveGoldenVectorRefs {
    seen: BTreeSet<(GoldenVectorId, Hash256)>,
    refs: Vec<GoldenVectorRef>,
}

impl ActiveGoldenVectorRefs {
    fn insert(&mut self, vector: &GoldenVectorRef) {
        if self.seen.insert((vector.id.clone(), vector.manifest_hash)) {
            self.refs.push(vector.clone());
        }
    }

    fn iter(&self) -> impl Iterator<Item = &GoldenVectorRef> {
        self.refs.iter()
    }
}

fn validate_workload_ref(
    resolver: &dyn ArtifactResolver,
    workload: &WorkloadManifestRef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) -> Option<ResolvedWorkload> {
    match resolver.resolve_workload(workload) {
        Ok(resolved) => {
            if resolved.manifest.id.as_str() != workload.id.as_str() {
                diagnostics.push(workload_ref_unresolved_diagnostic(
                    workload,
                    ValidationDetail::Field {
                        field: workload_ref_field(&workload.id, "id"),
                    },
                ));
                return None;
            }
            if resolved.manifest.self_hash != workload.manifest_hash {
                diagnostics.push(workload_ref_unresolved_diagnostic(
                    workload,
                    ValidationDetail::HashMismatch {
                        expected: workload.manifest_hash,
                        observed: resolved.manifest.self_hash,
                    },
                ));
                return None;
            }
            Some(resolved)
        }
        Err(ArtifactResolveError::HashMismatch {
            expected, observed, ..
        }) => {
            diagnostics.push(workload_ref_unresolved_diagnostic(
                workload,
                ValidationDetail::HashMismatch { expected, observed },
            ));
            None
        }
        Err(ArtifactResolveError::NotFound { .. })
        | Err(ArtifactResolveError::Unsupported { .. }) => {
            diagnostics.push(workload_ref_unresolved_diagnostic(
                workload,
                ValidationDetail::Field {
                    field: FieldPath::from(format!("workloads.{}", workload.id)),
                },
            ));
            None
        }
    }
}

fn validate_golden_vector_ref(
    resolver: &dyn ArtifactResolver,
    vector: &GoldenVectorRef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match resolver.resolve_golden_vector(vector) {
        Ok(resolved) => {
            let observed = sha256_hash(&resolved.bytes);
            if observed != vector.manifest_hash {
                diagnostics.push(golden_vector_digest_mismatch_diagnostic(
                    vector,
                    vector.manifest_hash,
                    observed,
                ));
            }
        }
        Err(ArtifactResolveError::NotFound { .. })
        | Err(ArtifactResolveError::Unsupported { .. }) => {
            diagnostics.push(golden_vector_missing_diagnostic(vector));
        }
        Err(ArtifactResolveError::HashMismatch {
            expected, observed, ..
        }) => diagnostics.push(golden_vector_digest_mismatch_diagnostic(
            vector, expected, observed,
        )),
    }
}

fn workload_ref_unresolved_diagnostic(
    workload: &WorkloadManifestRef,
    detail: ValidationDetail,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Workload,
        ValidationCode::WorkloadRefUnresolved {
            workload: workload.id.clone(),
        },
        detail,
        vec![EvidenceRef {
            kind: "workload_ref".to_owned(),
            reference: workload.id.to_string(),
            hash: Some(workload.manifest_hash),
        }],
    )
}

fn workload_ref_field(workload: &WorkloadId, field: &'static str) -> FieldPath {
    FieldPath::from(format!("workloads.{workload}.{field}"))
}

fn golden_vector_missing_diagnostic(vector: &GoldenVectorRef) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::GoldenVector,
        ValidationCode::GoldenVectorMissing {
            vector: vector.id.clone(),
        },
        ValidationDetail::Field {
            field: FieldPath::from(format!("golden_vectors.{}", vector.id.0)),
        },
        vec![golden_vector_evidence(vector)],
    )
}

fn golden_vector_digest_mismatch_diagnostic(
    vector: &GoldenVectorRef,
    expected: Hash256,
    observed: Hash256,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::GoldenVector,
        ValidationCode::GoldenVectorDigestMismatch {
            vector: vector.id.clone(),
            expected,
            observed,
        },
        ValidationDetail::HashMismatch { expected, observed },
        vec![golden_vector_evidence(vector)],
    )
}

fn golden_vector_evidence(vector: &GoldenVectorRef) -> EvidenceRef {
    EvidenceRef {
        kind: "golden_vector_ref".to_owned(),
        reference: vector.id.0.clone(),
        hash: Some(vector.manifest_hash),
    }
}

fn assembled_lowering_manifest(lowering: &TargetDataLoweringArtifact) -> LoweringManifest {
    LoweringManifest {
        shard_refs: lowering.shards.iter().map(lowering_shard_ref).collect(),
        aggregate_hash: lowering.manifest_hash,
    }
}

fn lowering_shard_ref(shard: &LoweringShard) -> LoweringShardRef {
    LoweringShardRef {
        id: shard.id.clone(),
        manifest_hash: shard.packed_bytes_hash,
    }
}

fn lowering_manifest_diagnostic_ref(lowering: &TargetDataLoweringArtifact) -> LoweringShardRef {
    LoweringShardRef {
        id: LoweringShardId("lowering_manifest".to_owned()),
        manifest_hash: lowering.manifest_hash,
    }
}

fn lowering_round_trip_failed_diagnostic(
    shard: LoweringShardRef,
    detail: ValidationDetail,
    evidence: EvidenceRef,
) -> ValidationDiagnostic {
    ValidationDiagnostic::hard(
        ValidationOrigin::Lowering,
        ValidationCode::LoweringRoundTripFailed { shard },
        detail,
        vec![evidence],
    )
}

fn lowering_evidence(lowering: &TargetDataLoweringArtifact, reference: &str) -> EvidenceRef {
    EvidenceRef {
        kind: "target_data_lowering".to_owned(),
        reference: format!("{}:{}:{reference}", lowering.target, lowering.profile.0),
        hash: Some(lowering.manifest_hash),
    }
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
    let semantic_core_hash_matches =
        validate_semantic_core_hash(effective_artifact, &mut diagnostics);
    validate_transport_manifest(inputs.artifact, &mut diagnostics);
    if !semantic_core_hash_matches {
        return Err(stage0_failure(
            &inputs,
            Some(effective_artifact),
            inputs.calibration,
            diagnostics,
            compatibility.compatibility_section(),
        ));
    }
    validate_manifest_invariants(
        inputs.artifact,
        effective_artifact,
        inputs.lowerings,
        &mut diagnostics,
    );
    validate_artifact_payload(effective_artifact, &mut diagnostics);
    validate_artifact_aux_sidecars(effective_artifact, inputs.resolver, &mut diagnostics);
    validate_target_data_lowering(&inputs, &mut diagnostics);

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
    validate_calibration_binding(&inputs, calibration, &mut diagnostics);
    if !diagnostics.is_empty() {
        // TODO(bd-26zc): keep collecting classes 8-10 after class 7 once the
        // full Stage 0 diagnostic collector replaces these fail-fast slices.
        return Err(stage0_failure(
            &inputs,
            Some(effective_artifact),
            Some(calibration),
            diagnostics,
            compatibility.compatibility_section(),
        ));
    }

    validate_hint_provenance(&inputs, effective_artifact, &mut diagnostics);
    validate_workload_and_golden_refs(&inputs, effective_artifact, &mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(stage0_failure(
            &inputs,
            Some(effective_artifact),
            Some(calibration),
            diagnostics,
            compatibility.compatibility_section(),
        ));
    }

    validate_compile_request_admissibility(&inputs, effective_artifact, &mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(stage0_failure(
            &inputs,
            Some(effective_artifact),
            Some(calibration),
            diagnostics,
            compatibility.compatibility_section(),
        ));
    }

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
    use std::cell::Cell;
    use std::collections::{BTreeMap, BTreeSet};

    use gbf_artifact::BuildConstraintEntry;
    use gbf_artifact::aux::{
        ArtifactAux, ConformanceEnvelopeId, ConformanceEnvelopeRef, InteractionBundleId,
        InteractionBundleRef, LexicalSpecId, LexicalSpecRef, ReferenceObservationCacheId,
        ReferenceObservationCacheRef, SemanticCheckpointSchemaId, SemanticCheckpointSchemaRef,
    };
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
    use gbf_artifact::tensor::{
        CanonicalTensor, CanonicalTensorId, CanonicalTensorKind, CanonicalTensorLayout,
        CanonicalTensorPayload, CanonicalTensorShape, TensorElementType,
    };
    use gbf_foundation::{
        BlobCodec, CompileProfileId, KernelCalibrationId, PackerVersion, TargetFamilyId,
        TargetProfileId,
    };
    use gbf_hw::calibration::CalibrationSetRef;
    use gbf_hw::target::{
        CapabilitySet, CartridgeProfile, ConsoleModel, canonical_target_profile_id,
        dmg_mbc5_8mib_128kib,
    };
    use gbf_hw::timing::dmg_timing;
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_ID, BootstrapCalibrationBundle, CalibrationConfidenceClass,
        CalibrationConfidenceRequirement, CompileKnobId, CompileObjective, CompilerFeature,
        ConstraintValue, DEFAULT_COMPILE_PROFILE_ID, ObjectiveRejection, PlacementProfile,
        RiskPolicy, RiskQuantileField, RuntimeMode, ServiceLevelField, ServiceLevelObjective,
        TargetIncompatibilityReason, TraceProbeId, canonical_compile_profile_specs,
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
        let resolver = RecordingResolver::default();
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
    fn f_b2_validate_rejects_missing_calibration() {
        let fixture = Fixture::new(Some(HintBundle::empty()), None);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert_eq!(failure.report.body.identity.calibration_hash, None);
        assert_eq!(
            failure
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    matches!(diagnostic.code, ValidationCode::CalibrationMissing { .. })
                })
                .count(),
            CalibrationLayer::all().len()
        );
    }

    #[test]
    fn f_b2_validate_rejects_explicit_calibration_unresolved() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let mut set_ref = calibration_set_ref();
        set_ref.kernel = Some(KernelCalibrationId::from("kernel.missing"));
        fixture.compile_request.calibration_set_ref = set_ref;

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_eq!(
            failure.report.body.identity.calibration_hash,
            fixture.calibration.as_ref().map(|calibration| {
                input_hash(
                    "gbf-policy",
                    "CalibrationBundleSet",
                    "calibration",
                    "1.0.0",
                    calibration,
                )
            })
        );
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationMissing {
                    class: CalibrationLayer::Kernel
                }
            )
        });
        let diagnostic = failure
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.code,
                    ValidationCode::CalibrationMissing {
                        class: CalibrationLayer::Kernel
                    }
                )
            })
            .expect("kernel calibration missing diagnostic is present");
        assert_eq!(
            diagnostic.detail,
            ValidationDetail::Field {
                field: FieldPath::from("compile_request.calibration_set_ref.kernel"),
            }
        );
        assert!(
            diagnostic.provenance.iter().any(|evidence| {
                evidence.kind == "compile_request"
                    && evidence.reference == "calibration_set_ref.kernel"
                    && evidence.hash.is_some()
            }),
            "unresolved requested ID points at the request field"
        );
    }

    #[test]
    fn f_b2_validate_rejects_partial_calibration_set_ref() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.calibration_set_ref = CalibrationSetRef {
            platform: calibration_set_ref().platform,
            kernel: None,
            runtime: None,
        };

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_eq!(
            failure.report.body.identity.calibration_hash,
            fixture.calibration.as_ref().map(|calibration| {
                input_hash(
                    "gbf-policy",
                    "CalibrationBundleSet",
                    "calibration",
                    "1.0.0",
                    calibration,
                )
            })
        );
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationMissing {
                    class: CalibrationLayer::Kernel
                }
            )
        });
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationMissing {
                    class: CalibrationLayer::Runtime
                }
            )
        });
        let kernel_diagnostic = failure
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.code,
                    ValidationCode::CalibrationMissing {
                        class: CalibrationLayer::Kernel
                    }
                )
            })
            .expect("kernel calibration missing diagnostic is present");
        assert_eq!(
            kernel_diagnostic.detail,
            ValidationDetail::Field {
                field: FieldPath::from("compile_request.calibration_set_ref.kernel"),
            }
        );
        assert!(
            !failure.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic.code,
                ValidationCode::CalibrationMissing {
                    class: CalibrationLayer::Platform
                }
            )),
            "referenced platform layer is not rejected"
        );
    }

    #[test]
    fn f_b2_validate_rejects_calibration_stale() {
        struct StaleCase {
            field: &'static str,
            mutate: fn(&mut CalibrationBundle),
            declared: Hash256,
            observed: Hash256,
        }

        let stale_packer_version = PackerVersion::new(2, 0, 0);
        let cases = [
            StaleCase {
                field: "target_profile_hash",
                mutate: |bundle| bundle.target_profile_hash = hash(0xbb),
                declared: hash(0xbb),
                observed: active_target_profile_hash(),
            },
            StaleCase {
                field: "kernel_set_hash",
                mutate: |bundle| bundle.kernel_set_hash = hash(0xcc),
                declared: hash(0xcc),
                observed: Hash256::ZERO,
            },
            StaleCase {
                field: "packer_version",
                mutate: |bundle| bundle.packer_version = PackerVersion::new(2, 0, 0),
                declared: packer_version_freshness_hash(&stale_packer_version),
                observed: packer_version_freshness_hash(&gbf_runtime::RUNTIME_PACKER_VERSION),
            },
            StaleCase {
                field: "calibration_schema_hash",
                mutate: |bundle| bundle.calibration_schema_hash = hash(0xdd),
                declared: hash(0xdd),
                observed: Hash256::ZERO,
            },
        ];

        for case in cases {
            let mut calibration = calibration();
            let bundle = calibration
                .bundles
                .get_mut(&CalibrationLayer::Platform)
                .expect("platform calibration exists");
            (case.mutate)(bundle);
            let fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration));

            let failure =
                validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");
            let diagnostic = failure
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    matches!(
                        &diagnostic.code,
                        ValidationCode::CalibrationStale {
                            class: CalibrationLayer::Platform,
                            declared,
                            observed,
                        } if declared == &case.declared && observed == &case.observed
                    )
                })
                .unwrap_or_else(|| panic!("{} stale diagnostic is present", case.field));

            assert_eq!(
                diagnostic.detail,
                ValidationDetail::HashMismatch {
                    expected: case.declared,
                    observed: case.observed,
                }
            );
            assert!(
                diagnostic.provenance.iter().any(|evidence| {
                    evidence.kind == "calibration_bundle"
                        && evidence.reference == format!("Platform.{}", case.field)
                        && evidence.hash == failure.report.body.identity.calibration_hash
                }),
                "{} stale diagnostic carries bundle-field evidence",
                case.field
            );
        }
    }

    #[test]
    fn f_b2_validate_rejects_stale_unrequested_calibration_bundle() {
        let mut calibration = calibration();
        calibration
            .bundles
            .get_mut(&CalibrationLayer::Kernel)
            .expect("kernel calibration exists")
            .target_profile_hash = hash(0xbb);
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration));
        fixture.compile_request.calibration_set_ref = CalibrationSetRef {
            platform: calibration_set_ref().platform,
            kernel: None,
            runtime: None,
        };

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationStale {
                    class: CalibrationLayer::Kernel,
                    declared,
                    observed,
                } if *declared == hash(0xbb) && *observed == active_target_profile_hash()
            )
        });
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationMissing {
                    class: CalibrationLayer::Kernel
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_low_confidence_unrequested_calibration_bundle() {
        let mut calibration = calibration();
        calibration
            .bundles
            .get_mut(&CalibrationLayer::Platform)
            .expect("platform calibration exists")
            .confidence = CalibrationConfidenceClass::Transferred;
        calibration
            .bundles
            .get_mut(&CalibrationLayer::Runtime)
            .expect("runtime calibration exists")
            .confidence = CalibrationConfidenceClass::Transferred;
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration));
        fixture.compile_request.calibration_set_ref = CalibrationSetRef {
            platform: calibration_set_ref().platform,
            kernel: None,
            runtime: None,
        };
        fixture.compile_request.profile = CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID);
        fixture.compile_profile = compile_profile_by_id(DEFAULT_COMPILE_PROFILE_ID);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        let diagnostic = failure
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.code,
                    ValidationCode::CalibrationConfidenceTooLow {
                        required: CalibrationConfidenceClass::Transferred,
                        observed: CalibrationConfidenceClass::None,
                    }
                ) && diagnostic.detail
                    == (ValidationDetail::Field {
                        field: FieldPath::from("calibration.Kernel.confidence"),
                    })
            })
            .expect("unrequested kernel low-confidence diagnostic is present");
        assert!(
            diagnostic.provenance.iter().any(|evidence| {
                evidence.kind == "calibration_bundle"
                    && evidence.reference == "Kernel.confidence"
                    && evidence.hash == failure.report.body.identity.calibration_hash
            }),
            "low-confidence diagnostic points at the downstream calibration bundle"
        );
    }

    #[test]
    fn f_b2_validate_accepts_checked_in_bootstrap_calibration_when_profile_requires_none() {
        let fixture = Fixture::new(
            Some(HintBundle::empty()),
            Some(checked_in_bootstrap_calibration()),
        );

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(product.report.outcome, ReportOutcome::Passed);
        for bundle in product.validated.calibration.bundles.values() {
            assert_eq!(bundle.target_profile_hash, active_target_profile_hash());
        }
    }

    #[test]
    fn f_b2_validate_rejects_empty_calibration_set_ref() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.calibration_set_ref = CalibrationSetRef::default();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_eq!(
            failure
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    matches!(diagnostic.code, ValidationCode::CalibrationMissing { .. })
                })
                .count(),
            CalibrationLayer::all().len()
        );
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationMissing {
                    class: CalibrationLayer::Platform,
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_accepts_bootstrap_calibration_when_profile_requires_none() {
        let fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));

        let product = validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(product.report.outcome, ReportOutcome::Passed);
        assert!(
            product
                .validated
                .compile_profile
                .risk_policy
                .calibration_confidence_requirement
                .accepts(CalibrationConfidenceClass::None)
        );
    }

    #[test]
    fn f_b2_validate_rejects_bootstrap_calibration_under_default_profile() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.profile = CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID);
        fixture.compile_profile = compile_profile_by_id(DEFAULT_COMPILE_PROFILE_ID);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CalibrationConfidenceTooLow {
                    required: CalibrationConfidenceClass::Transferred,
                    observed: CalibrationConfidenceClass::None,
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_hint_provenance_inconsistent() {
        let mut hint_bundle = HintBundle::empty();
        hint_bundle
            .constraints
            .entries
            .push(build_constraint_with_scope(
                TraceProbeId(401),
                EvidenceScope::TargetFamily {
                    family: TargetFamilyId::from("cgb"),
                },
            ));
        let fixture = Fixture::new(Some(hint_bundle), Some(calibration()));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::HintProvenanceInconsistent {
                    fact: TraceProbeId(401)
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_accepts_hint_scope_matrix_for_active_and_broader_scopes() {
        let mut hint_bundle = HintBundle::empty();
        for (index, scope) in [
            EvidenceScope::WholeArtifact,
            EvidenceScope::TargetFamily {
                family: TargetFamilyId::from("dmg"),
            },
            EvidenceScope::WorkloadScoped {
                workload: WorkloadId::from("workload.fixture"),
            },
            EvidenceScope::LoweringScoped {
                shard: LoweringShardId("weight.layer0".to_owned()),
            },
            EvidenceScope::LayerScoped {
                layer: LayerId::new(0),
            },
        ]
        .into_iter()
        .enumerate()
        {
            hint_bundle
                .constraints
                .entries
                .push(build_constraint_with_scope(
                    TraceProbeId(410 + index as u16),
                    scope,
                ));
        }
        hint_bundle
            .facts
            .scope_provenance
            .push(HintScopeProvenance {
                provenance_id: TraceProbeId(420),
                field: FieldPath::from("activation_ranges.0"),
                scope: EvidenceScope::LayerScoped {
                    layer: LayerId::new(0),
                },
            });
        hint_bundle.preferences =
            hint_bundle
                .preferences
                .with_scope_provenance(vec![HintScopeProvenance {
                    provenance_id: TraceProbeId(421),
                    field: FieldPath::from("expert_slot_affinity.0"),
                    scope: EvidenceScope::WorkloadScoped {
                        workload: WorkloadId::from("workload.fixture"),
                    },
                }]);
        let mut fixture = Fixture::new(Some(hint_bundle), Some(calibration()));
        fixture.set_single_layer_tensor(0);

        validate_artifact_and_request(fixture.inputs()).expect("validation passes");
    }

    #[test]
    fn f_b2_validate_rejects_duplicate_scope_provenance_id_across_facts_and_preferences() {
        let duplicate_id = TraceProbeId(812);
        let mut hint_bundle = HintBundle::empty();
        hint_bundle
            .facts
            .scope_provenance
            .push(HintScopeProvenance {
                provenance_id: duplicate_id,
                field: FieldPath::from("activation_ranges.0"),
                scope: EvidenceScope::WholeArtifact,
            });
        hint_bundle.preferences =
            hint_bundle
                .preferences
                .with_scope_provenance(vec![HintScopeProvenance {
                    provenance_id: duplicate_id,
                    field: FieldPath::from("expert_slot_affinity.0"),
                    scope: EvidenceScope::WholeArtifact,
                }]);
        let fixture = Fixture::new(Some(hint_bundle), Some(calibration()));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::HintProvenanceInconsistent {
                    fact
                } if *fact == duplicate_id
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_duplicate_scope_provenance_id_across_preferences_and_constraints() {
        let duplicate_id = TraceProbeId(813);
        let mut hint_bundle = HintBundle::empty();
        hint_bundle
            .facts
            .scope_provenance
            .push(HintScopeProvenance {
                provenance_id: TraceProbeId(814),
                field: FieldPath::from("activation_ranges.0"),
                scope: EvidenceScope::WholeArtifact,
            });
        hint_bundle.preferences =
            hint_bundle
                .preferences
                .with_scope_provenance(vec![HintScopeProvenance {
                    provenance_id: duplicate_id,
                    field: FieldPath::from("expert_slot_affinity.0"),
                    scope: EvidenceScope::WholeArtifact,
                }]);
        hint_bundle
            .constraints
            .entries
            .push(build_constraint_with_scope(
                duplicate_id,
                EvidenceScope::WholeArtifact,
            ));
        let fixture = Fixture::new(Some(hint_bundle), Some(calibration()));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::HintProvenanceInconsistent {
                    fact
                } if *fact == duplicate_id
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_hint_scope_matrix_for_inactive_narrower_scopes() {
        let mut hint_bundle = HintBundle::empty();
        for (index, scope) in [
            EvidenceScope::TargetFamily {
                family: TargetFamilyId::from("cgb"),
            },
            EvidenceScope::WorkloadScoped {
                workload: WorkloadId::from("workload.other"),
            },
            EvidenceScope::LoweringScoped {
                shard: LoweringShardId("weight.layer99".to_owned()),
            },
            EvidenceScope::LayerScoped {
                layer: LayerId::new(9),
            },
        ]
        .into_iter()
        .enumerate()
        {
            hint_bundle
                .constraints
                .entries
                .push(build_constraint_with_scope(
                    TraceProbeId(430 + index as u16),
                    scope,
                ));
        }
        hint_bundle
            .facts
            .scope_provenance
            .push(HintScopeProvenance {
                provenance_id: TraceProbeId(440),
                field: FieldPath::from("temporal_switch.0"),
                scope: EvidenceScope::TargetFamily {
                    family: TargetFamilyId::from("cgb"),
                },
            });
        hint_bundle.preferences =
            hint_bundle
                .preferences
                .with_scope_provenance(vec![HintScopeProvenance {
                    provenance_id: TraceProbeId(441),
                    field: FieldPath::from("expert_slot_affinity.0"),
                    scope: EvidenceScope::WorkloadScoped {
                        workload: WorkloadId::from("workload.other"),
                    },
                }]);
        let mut fixture = Fixture::new(Some(hint_bundle), Some(calibration()));
        fixture.set_single_layer_tensor(0);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        let provenance_failures = failure
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.code,
                    ValidationCode::HintProvenanceInconsistent { .. }
                )
            })
            .count();
        assert_eq!(
            provenance_failures, 6,
            "diagnostics were {:#?}",
            failure.diagnostics
        );
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::HintProvenanceInconsistent {
                    fact: TraceProbeId(440)
                }
            )
        });
        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::HintProvenanceInconsistent {
                    fact: TraceProbeId(441)
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_active_layer_ids_use_segmented_layer_path_grammar() {
        assert_eq!(
            layer_id_from_artifact_path("model.layer.12.weight"),
            Some(LayerId::new(12))
        );
        assert_eq!(layer_id_from_artifact_path("model.layer12.weight"), None);
        assert_eq!(
            layer_id_from_artifact_path("model.layer.not_u16.weight"),
            None
        );
    }

    #[test]
    fn f_b2_validate_rejects_workload_ref_unresolved() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .resolver
            .missing_workloads
            .insert(WorkloadId::from("workload.fixture"));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::WorkloadRefUnresolved { workload }
                    if workload == &WorkloadId::from("workload.fixture")
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_workload_ref_id_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.resolver.workload_id_mismatches.insert(
            WorkloadId::from("workload.fixture"),
            WorkloadId::from("workload.other"),
        );

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::WorkloadRefUnresolved { workload }
                    if workload == &WorkloadId::from("workload.fixture")
            )
        });
        assert!(
            failure.diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.detail,
                ValidationDetail::Field { field }
                    if field == &FieldPath::from("workloads.workload.fixture.id")
            )),
            "diagnostics were {:#?}",
            failure.diagnostics
        );
    }

    #[test]
    fn f_b2_validate_rejects_workload_ref_manifest_hash_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.resolver.workload_hash_mismatches.insert(
            WorkloadId::from("workload.fixture"),
            (hash(0x06), hash(0x60)),
        );

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::WorkloadRefUnresolved { workload }
                    if workload == &WorkloadId::from("workload.fixture")
            )
        });
        assert!(
            failure.diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.detail,
                ValidationDetail::HashMismatch {
                    expected,
                    observed,
                } if *expected == hash(0x06) && *observed == hash(0x60)
            )),
            "diagnostics were {:#?}",
            failure.diagnostics
        );
    }

    #[test]
    fn f_b2_validate_rejects_golden_vector_missing() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .resolver
            .missing_golden_vectors
            .insert(GoldenVectorId("golden.fixture".to_owned()));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::GoldenVectorMissing { vector }
                    if vector == &GoldenVectorId("golden.fixture".to_owned())
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_golden_vector_digest_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.resolver.golden_vector_bytes.insert(
            GoldenVectorId("golden.fixture".to_owned()),
            b"mutated golden vector".to_vec(),
        );
        let expected = golden_vector_hash();
        let observed = sha256_hash(b"mutated golden vector");

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::GoldenVectorDigestMismatch {
                    vector,
                    expected: actual_expected,
                    observed: actual_observed,
                } if vector == &GoldenVectorId("golden.fixture".to_owned())
                    && actual_expected == &expected
                    && actual_observed == &observed
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_artifact_required_feature_unsupported_by_target() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .artifact
            .manifest
            .required_features
            .insert(ArtifactFeature::MoeRouting);
        fixture.artifact.aux.interaction_bundle = Some(InteractionBundleRef {
            id: InteractionBundleId("interaction.fixture".to_owned()),
            hash: sha256_hash(&[]),
        });
        fixture.refresh_manifest_self_hash();
        fixture.refresh_transport_hash();

        let target_capabilities =
            Stage0Class10TargetCapabilities::from_target_profile(&fixture.target_profile);
        assert!(!target_capabilities.supports_artifact_feature(ArtifactFeature::MoeRouting));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactRequiredFeatureUnsupported {
                    feature: ArtifactFeature::MoeRouting
                }
            )
        });
        assert_no_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CompileRequestUnsupportedFeature { .. }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_compile_request_compiler_feature_unsupported() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .compile_request
            .required_features
            .insert(CompilerFeature::StaticBudgetReport);

        assert!(!compiler_build_supports_feature(
            CompilerFeature::StaticBudgetReport
        ));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CompileRequestUnsupportedFeature {
                    feature: CompilerFeature::StaticBudgetReport
                }
            )
        });
        assert_no_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactRequiredFeatureUnsupported { .. }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_compile_request_profile_forbids_objective() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.objective.risk.cycle_quantile = 0;

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CompileRequestProfileForbidsObjective {
                    profile,
                    reason: ObjectiveRejection::RiskQuantileInvalid {
                        field,
                        value: 0,
                    },
                } if profile == &CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID)
                    && field == &RiskQuantileField::CycleQuantile
            )
        });
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
            field: ServiceLevelField::MaxFirstTokenCyclesP95
        }));
        assert!(reasons.contains(&ObjectiveRejection::ServiceLevelZero {
            field: ServiceLevelField::MaxResumeLatencyCyclesP95
        }));
        assert!(reasons.contains(&ObjectiveRejection::MaxCyclesPerTokenZero));
        assert!(reasons.contains(&ObjectiveRejection::MaxRomBytesZero));
        assert!(reasons.contains(&ObjectiveRejection::RiskQuantileInvalid {
            field: RiskQuantileField::CycleQuantile,
            value: 0,
        }));
        assert!(reasons.contains(&ObjectiveRejection::RiskQuantileInvalid {
            field: RiskQuantileField::SwitchQuantile,
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
                    field: ServiceLevelField::MaxFirstTokenCyclesP95,
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
                    field: ServiceLevelField::MaxCheckpointGapCyclesP95,
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
                    field: ServiceLevelField::MaxResumeLatencyCyclesP95,
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
                    field: ServiceLevelField::MaxUiJitterFramesP99,
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
                    field: RiskQuantileField::CycleQuantile,
                    value: 0,
                },
            ),
            (
                "cycle_quantile_above_100",
                |objective| objective.risk.cycle_quantile = 101,
                ObjectiveRejection::RiskQuantileInvalid {
                    field: RiskQuantileField::CycleQuantile,
                    value: 101,
                },
            ),
            (
                "switch_quantile_zero",
                |objective| objective.risk.switch_quantile = 0,
                ObjectiveRejection::RiskQuantileInvalid {
                    field: RiskQuantileField::SwitchQuantile,
                    value: 0,
                },
            ),
            (
                "switch_quantile_above_100",
                |objective| objective.risk.switch_quantile = 101,
                ObjectiveRejection::RiskQuantileInvalid {
                    field: RiskQuantileField::SwitchQuantile,
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
            Some(CompileProfileId::from(DEFAULT_COMPILE_PROFILE_ID));
        fixture.compile_request.objective.risk.fallback_runtime_mode = Some(RuntimeMode::Trace);

        validate_artifact_and_request(fixture.inputs()).expect("fallback fields are accepted");
    }

    #[test]
    fn f_b2_validate_rejects_compile_request_runtime_mode_unsupported() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .compile_request
            .requested_runtime_modes
            .insert(RuntimeMode::Trace);

        let target_capabilities =
            Stage0Class10TargetCapabilities::from_target_profile(&fixture.target_profile);
        assert!(!target_capabilities.supports_runtime_mode(RuntimeMode::Trace));

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CompileRequestRuntimeModeUnsupported {
                    mode: RuntimeMode::Trace
                }
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_compile_request_target_incompatible() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.target = TargetProfileId::from("cgb-mbc5-8mib-128kib");

        let target_capabilities =
            Stage0Class10TargetCapabilities::from_target_profile(&fixture.target_profile);
        assert_eq!(
            target_capabilities.target_compatibility(&fixture.compile_request.target),
            Err(TargetIncompatibilityReason::TargetFamilyMismatch)
        );

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::CompileRequestTargetIncompatible {
                    target,
                    reason: TargetIncompatibilityReason::TargetFamilyMismatch,
                } if target == &TargetProfileId::from("cgb-mbc5-8mib-128kib")
            )
        });
    }

    #[test]
    fn f_b2_validate_allows_compile_request_target_with_same_family_lowering() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.compile_request.target = dmg_family_sibling_target_id(ConsoleModel::Mgb);

        let target_capabilities =
            Stage0Class10TargetCapabilities::from_target_profile(&fixture.target_profile);
        assert_eq!(
            target_capabilities.target_compatibility(&fixture.compile_request.target),
            Ok(())
        );

        validate_artifact_and_request(fixture.inputs()).expect("validation passes");
    }

    #[test]
    fn f_b2_validate_rechecks_aux_golden_vectors_in_class9() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let vector = golden_vector_with_id("golden.aux");
        fixture.golden_vectors.clear();
        fixture.artifact.aux.golden_vectors = vec![vector.clone()];
        fixture
            .resolver
            .golden_vector_bytes
            .insert(vector.id.clone(), b"mutated aux golden vector".to_vec());
        fixture.refresh_transport_hash();
        let observed = sha256_hash(b"mutated aux golden vector");

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::GoldenVectorDigestMismatch {
                    vector: actual_vector,
                    expected,
                    observed: actual_observed,
                } if actual_vector == &vector.id
                    && expected == &golden_vector_hash()
                    && actual_observed == &observed
            )
        });
    }

    #[test]
    fn f_b2_validate_rechecks_workload_manifest_golden_vectors_in_class9() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let vector = golden_vector_with_id("golden.workload");
        fixture.golden_vectors.clear();
        fixture
            .resolver
            .workload_golden_vectors
            .insert(WorkloadId::from("workload.fixture"), vec![vector.clone()]);
        fixture.resolver.golden_vector_bytes.insert(
            vector.id.clone(),
            b"mutated workload golden vector".to_vec(),
        );
        let observed = sha256_hash(b"mutated workload golden vector");

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::GoldenVectorDigestMismatch {
                    vector: actual_vector,
                    expected,
                    observed: actual_observed,
                } if actual_vector == &vector.id
                    && expected == &golden_vector_hash()
                    && actual_observed == &observed
            )
        });
    }

    #[test]
    fn f_b2_validate_workload_ref_resolution_uses_artifact_resolver_trait() {
        let fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));

        validate_artifact_and_request(fixture.inputs()).expect("validation passes");

        assert_eq!(fixture.resolver.workload_resolve_calls.get(), 1);
        assert_eq!(fixture.resolver.golden_vector_resolve_calls.get(), 1);
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
    fn f_b2_validate_semantic_core_hash_mismatch_short_circuits_manifest_invariants() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.manifest.semantic_core_hash = hash(0xee);
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
            matches!(code, ValidationCode::SemanticCoreHashMismatch)
        });
        assert_no_failure_code(&failure, |code| {
            matches!(code, ValidationCode::ManifestInvariantViolated { .. })
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

    #[test]
    fn f_b2_import_raw_forbidden_build_identity_field_reaches_stage0() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let mut manifest_json =
            serde_json::to_value(&fixture.artifact.manifest).expect("manifest serializes");
        manifest_json["build_identity"] = serde_json::json!({
            "backend": "must not be part of frozen inputs",
        });
        assert!(
            serde_json::from_value::<ArtifactManifest>(manifest_json.clone()).is_err(),
            "raw forbidden manifest fields must be captured before typed serde rejects them",
        );

        let mut aux_json = serde_json::to_value(&fixture.artifact.aux).expect("aux serializes");
        aux_json["backend_identity"] = serde_json::json!("late-stage-only");

        let mut lowerings_json =
            serde_json::to_value(&fixture.lowerings).expect("lowerings serialize");
        lowerings_json[0]["shards"][0]["stage12_identity"] =
            serde_json::json!({ "rom": "post-input identity" });

        let imported = crate::import::import_artifact_view_from_raw_json(
            crate::import::RawArtifactJsonImport {
                core: fixture.artifact.core.clone(),
                manifest_json,
                aux_json,
                lowerings_json,
                hint_bundle: Some(fixture.artifact.hint_bundle.clone()),
                reference: fixture.artifact.reference.clone(),
                transport: fixture.artifact.transport.clone(),
            },
        )
        .expect("raw artifact imports after forbidden fields are side-channeled");

        fixture.artifact = imported.artifact;
        fixture.lowerings = imported.lowerings;
        fixture.refresh_transport_hash();

        assert_eq!(
            fixture.artifact.forbidden_build_identity_fields,
            BTreeSet::from([
                FieldPath::from("aux/backend_identity"),
                FieldPath::from("lowerings/0/shards/0/stage12_identity"),
                FieldPath::from("manifest/build_identity"),
            ])
        );

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        for expected in [
            FieldPath::from("aux/backend_identity"),
            FieldPath::from("lowerings/0/shards/0/stage12_identity"),
            FieldPath::from("manifest/build_identity"),
        ] {
            assert_failure_code(&failure, |code| {
                matches!(
                    code,
                    ValidationCode::ArtifactForbiddenBuildIdentityField { field }
                        if field == &expected
                )
            });
        }
    }

    #[test]
    fn f_b2_validate_forbidden_build_identity_walker_finds_nested_serialized_keys() {
        let value = serde_json::json!({
            "metadata": {
                "compatibility_envelope": {
                    "ignored": true
                },
                "items": [
                    {
                        "stage12_identity": "late-stage-only"
                    }
                ]
            },
            "safe": {
                "nested": "ok"
            }
        });

        let fields = serialized_forbidden_build_identity_fields("artifact", &value);

        assert_eq!(
            fields,
            vec![
                FieldPath::from("artifact/metadata/compatibility_envelope"),
                FieldPath::from("artifact/metadata/items/0/stage12_identity"),
            ]
        );
    }

    #[test]
    fn f_b2_validate_rejects_artifact_payload_blob_digest_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let mut tensor = CanonicalTensor::new(
            CanonicalTensorId::new("tensor.bias").expect("tensor id"),
            CanonicalTensorKind::Bias,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[2]).expect("shape"),
                TensorElementType::Float32,
            ),
            CanonicalTensorPayload::F32(vec![1.0, 2.0]),
        )
        .expect("valid tensor");
        tensor.content_hash = hash(0xaa);
        fixture.artifact.core = ArtifactCore::new(
            vec![tensor.clone()],
            QuantSpec::default(),
            SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
        )
        .expect("core accepts non-deployable bias tensor");
        fixture.artifact.manifest.components = vec![ManifestComponent {
            digest: tensor.content_hash,
            id: ComponentId("tensor.bias".to_owned()),
            kind: ComponentKind::CanonicalTensor,
        }];
        fixture.artifact.manifest.semantic_core_hash = fixture.artifact.core.semantic_hash();
        fixture.refresh_manifest_self_hash();
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactBlobDigestMismatch { blob, .. } if blob.len == 29
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_artifact_payload_malformed_tensor_payload() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let tensor = CanonicalTensor {
            id: CanonicalTensorId::new("tensor.bad").expect("tensor id"),
            kind: CanonicalTensorKind::Bias,
            layout: CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[2]).expect("shape"),
                TensorElementType::Float32,
            ),
            payload: CanonicalTensorPayload::F32(vec![1.0]),
            content_hash: hash(0x52),
        };
        fixture.artifact.core = unchecked_artifact_core(
            vec![tensor.clone()],
            QuantSpec::default(),
            SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
        );
        fixture.artifact.manifest.components = vec![ManifestComponent {
            digest: tensor.content_hash,
            id: ComponentId("tensor.bad".to_owned()),
            kind: ComponentKind::CanonicalTensor,
        }];
        fixture.refresh_core_identity();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactPayloadMalformed { field }
                    if field == &FieldPath::from("core.tensors.tensor.bad.payload")
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_artifact_payload_malformed_quant_spec() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let tensor = CanonicalTensor::new(
            CanonicalTensorId::new("tensor.dense").expect("tensor id"),
            CanonicalTensorKind::DenseWeight,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[1, 1]).expect("shape"),
                TensorElementType::Float32,
            ),
            CanonicalTensorPayload::F32(vec![1.0]),
        )
        .expect("valid dense tensor");
        fixture.artifact.core = unchecked_artifact_core(
            vec![tensor.clone()],
            QuantSpec::default(),
            SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
        );
        fixture.artifact.manifest.components = vec![ManifestComponent {
            digest: tensor.content_hash,
            id: ComponentId("tensor.dense".to_owned()),
            kind: ComponentKind::CanonicalTensor,
        }];
        fixture.refresh_core_identity();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactPayloadMalformed { field }
                    if field == &FieldPath::from("core.quant.weight_quant.tensor.dense")
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_sequence_feature_semantics_mismatch() {
        let cases = [
            (
                SequenceSemanticsSpec::linear_state(1).expect("linear fixture"),
                BTreeSet::from([ArtifactFeature::DenseI8]),
            ),
            (
                SequenceSemanticsSpec::bounded_kv(16, 4).expect("bounded fixture"),
                BTreeSet::from([ArtifactFeature::DenseI8]),
            ),
            (
                SequenceSemanticsSpec::linear_state(1).expect("linear fixture"),
                BTreeSet::from([ArtifactFeature::DenseI8, ArtifactFeature::BoundedKvSequence]),
            ),
            (
                SequenceSemanticsSpec::bounded_kv(16, 4).expect("bounded fixture"),
                BTreeSet::from([
                    ArtifactFeature::DenseI8,
                    ArtifactFeature::LinearStateSequence,
                ]),
            ),
        ];

        for (semantics, required_features) in cases {
            let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
            fixture.artifact.core = ArtifactCore::new(Vec::new(), QuantSpec::default(), semantics)
                .expect("fixture core is valid");
            fixture.artifact.manifest.required_features = required_features;
            fixture.refresh_core_identity();

            let failure =
                validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

            assert_failure_code(&failure, |code| {
                matches!(
                    code,
                    ValidationCode::ArtifactPayloadMalformed { field }
                        if field == &FieldPath::from("core.sequence_semantics")
                )
            });
        }
    }

    #[test]
    fn f_b2_validate_rejects_both_sequence_required_features() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture
            .artifact
            .manifest
            .required_features
            .insert(ArtifactFeature::BoundedKvSequence);
        fixture.refresh_manifest_self_hash();
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactPayloadMalformed { field }
                    if field == &FieldPath::from("manifest.required_features.sequence_semantics")
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_artifact_aux_sidecar_missing() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.checkpoint_schema = Some(SemanticCheckpointSchemaRef {
            id: SemanticCheckpointSchemaId("checkpoint.fixture".to_owned()),
            hash: hash(0x40),
        });
        fixture.resolver.missing_sidecars.insert(SidecarRef {
            kind: SidecarKind::SemanticCheckpointSchema,
            id: "checkpoint.fixture".to_owned(),
            hash: hash(0x40),
        });
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactAuxSidecarMissing {
                    kind: SidecarKind::SemanticCheckpointSchema
                }
            )
        });
        let diagnostic = failure
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.code,
                    ValidationCode::ArtifactAuxSidecarMissing {
                        kind: SidecarKind::SemanticCheckpointSchema
                    }
                )
            })
            .expect("missing sidecar diagnostic");
        assert_eq!(
            diagnostic
                .provenance
                .first()
                .and_then(|evidence| evidence.hash),
            Some(fixture.artifact.manifest.manifest_self_hash)
        );
    }

    #[test]
    fn f_b2_validate_rejects_artifact_aux_sidecar_digest_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.conformance_envelope = Some(ConformanceEnvelopeRef {
            id: ConformanceEnvelopeId("conformance.fixture".to_owned()),
            hash: hash(0x41),
        });
        fixture.resolver.sidecar_bytes.insert(
            SidecarRef {
                kind: SidecarKind::ConformanceEnvelope,
                id: "conformance.fixture".to_owned(),
                hash: hash(0x41),
            },
            b"conformance sidecar".to_vec(),
        );
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactAuxSidecarDigestMismatch {
                    kind: SidecarKind::ConformanceEnvelope,
                    expected,
                    observed,
                } if expected == &hash(0x41) && observed == &sha256_hash(b"conformance sidecar")
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_resolver_reported_aux_sidecar_hash_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        let sidecar = SidecarRef {
            kind: SidecarKind::ReferenceObservationCache,
            id: "reference-cache.fixture".to_owned(),
            hash: hash(0x42),
        };
        fixture.artifact.aux.reference_observation_cache = Some(ReferenceObservationCacheRef {
            id: ReferenceObservationCacheId(sidecar.id.clone()),
            hash: sidecar.hash,
        });
        fixture
            .resolver
            .sidecar_hash_mismatches
            .insert(sidecar, (hash(0x42), hash(0x43)));
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactAuxSidecarDigestMismatch {
                    kind: SidecarKind::ReferenceObservationCache,
                    expected,
                    observed,
                } if expected == &hash(0x42) && observed == &hash(0x43)
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_lexical_spec_sidecar_digest_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.lexical_spec = Some(LexicalSpecRef {
            id: LexicalSpecId("lexical.fixture".to_owned()),
            hash: hash(0x51),
        });
        fixture.resolver.sidecar_bytes.insert(
            SidecarRef {
                kind: SidecarKind::LexicalSpec,
                id: "lexical.fixture".to_owned(),
                hash: hash(0x51),
            },
            b"lexical spec sidecar".to_vec(),
        );
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactAuxSidecarDigestMismatch {
                    kind: SidecarKind::LexicalSpec,
                    expected,
                    observed,
                } if expected == &hash(0x51)
                    && observed == &sha256_hash(b"lexical spec sidecar")
            )
        });
    }

    #[test]
    fn f_b2_validate_feature_gated_sidecar_mapping_is_pinned() {
        let cases = [
            (
                ArtifactFeature::LinearStateSequence,
                SidecarKind::SemanticCheckpointSchema,
                SequenceSemanticsSpec::linear_state(1).expect("linear fixture"),
            ),
            (
                ArtifactFeature::BoundedKvSequence,
                SidecarKind::SemanticCheckpointSchema,
                SequenceSemanticsSpec::bounded_kv(16, 4).expect("bounded fixture"),
            ),
            (
                ArtifactFeature::MoeRouting,
                SidecarKind::InteractionBundle,
                SequenceSemanticsSpec::linear_state(1).expect("linear fixture"),
            ),
        ];

        for (feature, expected_kind, semantics) in cases {
            let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
            fixture.artifact.core = ArtifactCore::new(Vec::new(), QuantSpec::default(), semantics)
                .expect("fixture core is valid");
            fixture.artifact.manifest.required_features =
                BTreeSet::from([ArtifactFeature::DenseI8, feature]);
            if matches!(feature, ArtifactFeature::MoeRouting) {
                fixture
                    .artifact
                    .manifest
                    .required_features
                    .insert(ArtifactFeature::LinearStateSequence);
            } else {
                fixture.artifact.aux.checkpoint_schema = None;
            }
            fixture.refresh_core_identity();

            let failure =
                validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

            assert_failure_code(&failure, |code| {
                matches!(
                    code,
                    ValidationCode::ArtifactAuxSidecarMissing { kind } if kind == &expected_kind
                )
            });
        }
    }

    #[test]
    fn f_b2_validate_golden_vector_sidecar_presence_does_not_hash_manifest_ref() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.golden_vectors = vec![golden_vector()];
        fixture.refresh_transport_hash();

        validate_artifact_and_request(fixture.inputs()).expect("validation passes");
    }

    #[test]
    fn f_b2_validate_resolver_reported_golden_vector_hash_mismatch_is_class9() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.golden_vectors = vec![golden_vector()];
        fixture.resolver.golden_vector_hash_mismatches.insert(
            GoldenVectorId("golden.fixture".to_owned()),
            (golden_vector_hash(), hash(0x70)),
        );
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::GoldenVectorDigestMismatch {
                    vector,
                    expected,
                    observed,
                } if vector == &GoldenVectorId("golden.fixture".to_owned())
                    && expected == &golden_vector_hash()
                    && observed == &hash(0x70)
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_missing_golden_vector_sidecar_ref() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.golden_vectors = vec![golden_vector()];
        fixture
            .resolver
            .missing_golden_vectors
            .insert(GoldenVectorId("golden.fixture".to_owned()));
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactAuxSidecarMissing {
                    kind: SidecarKind::GoldenVector
                }
            )
        });
        let diagnostic = failure
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.code,
                    ValidationCode::ArtifactAuxSidecarMissing {
                        kind: SidecarKind::GoldenVector
                    }
                )
            })
            .expect("missing golden vector diagnostic");
        assert_eq!(
            diagnostic
                .provenance
                .first()
                .and_then(|evidence| evidence.hash),
            Some(fixture.artifact.manifest.manifest_self_hash)
        );
    }

    #[test]
    fn f_b2_validate_rejects_duplicate_golden_vector_sidecar_refs() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.golden_vectors = vec![golden_vector(), golden_vector()];
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::ArtifactAuxMalformed { field }
                    if field == &FieldPath::from("aux.golden_vectors.golden.fixture")
            )
        });
    }

    #[test]
    fn f_b2_validate_unsupported_golden_vector_sidecar_ref_is_class9_missing() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.artifact.aux.golden_vectors = vec![golden_vector()];
        fixture
            .resolver
            .unsupported_golden_vectors
            .insert(GoldenVectorId("golden.fixture".to_owned()));
        fixture.refresh_transport_hash();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::GoldenVectorMissing { vector }
                    if vector == &GoldenVectorId("golden.fixture".to_owned())
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_lowering_round_trip_failure() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.lowerings[0].shards[0].packed_bytes_hash = hash(0xfa);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(code, ValidationCode::LoweringRoundTripFailed { .. })
        });
    }

    #[test]
    fn f_b2_validate_rejects_lowering_missing_for_target() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.lowerings.clear();

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::LoweringMissingForTarget {
                    target,
                    lowering_profile,
                } if target == fixture.target_profile.id()
                    && lowering_profile == &DataLoweringProfileId("dmg-default".to_owned())
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_lowering_wrong_profile_for_target() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.lowerings[0].profile = DataLoweringProfileId("wrong-profile".to_owned());

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::LoweringMissingForTarget {
                    target,
                    lowering_profile,
                } if target == fixture.target_profile.id()
                    && lowering_profile == &DataLoweringProfileId("dmg-default".to_owned())
            )
        });
    }

    #[test]
    fn f_b2_validate_rejects_lowering_packer_version_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.lowerings[0].packer_version = PackerVersion::new(2, 0, 0);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::LoweringPackerVersionMismatch {
                    artifact_version,
                    runtime_version,
                } if artifact_version == &PackerVersion::new(2, 0, 0)
                    && runtime_version == &gbf_runtime::RUNTIME_PACKER_VERSION
            )
        });
        assert_no_failure_code(&failure, |code| {
            matches!(code, ValidationCode::LoweringRoundTripFailed { .. })
        });
    }

    #[test]
    fn f_b2_validate_rejects_lowering_manifest_hash_mismatch() {
        let mut fixture = Fixture::new(Some(HintBundle::empty()), Some(calibration()));
        fixture.lowerings[0].manifest_hash = hash(0xfb);

        let failure =
            validate_artifact_and_request(fixture.inputs()).expect_err("validation fails");

        assert_failure_code(&failure, |code| {
            matches!(
                code,
                ValidationCode::LoweringRoundTripFailed { shard }
                    if shard.id == LoweringShardId("lowering_manifest".to_owned())
                        && shard.manifest_hash == hash(0xfb)
            )
        });
    }

    #[test]
    fn f_b2_validate_builtin_schema_adapters_excludes_test_only_proof_adapters() {
        let adapters = builtin_schema_adapters();

        assert_eq!(adapters.len(), 1);
        assert_eq!(
            adapters[0].id,
            CompatibilityAdapterId("adapter.lossless".to_owned())
        );
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
                resolver: RecordingResolver::default(),
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

        fn refresh_core_identity(&mut self) {
            self.artifact.manifest.semantic_core_hash = self.artifact.core.semantic_hash();
            self.refresh_manifest_self_hash();
            self.refresh_transport_hash();
        }

        fn refresh_transport_hash(&mut self) {
            self.artifact.transport.transport_hash =
                compute_imported_artifact_source_hash(&self.artifact);
        }

        fn set_single_layer_tensor(&mut self, layer: u16) {
            self.artifact.core = ArtifactCore::new(
                vec![layer_tensor(layer)],
                QuantSpec::default(),
                SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
            )
            .expect("layer tensor core is valid");
            self.refresh_core_identity();
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

    #[derive(Default)]
    struct RecordingResolver {
        missing_sidecars: BTreeSet<SidecarRef>,
        sidecar_bytes: BTreeMap<SidecarRef, Vec<u8>>,
        sidecar_hash_mismatches: BTreeMap<SidecarRef, (Hash256, Hash256)>,
        missing_workloads: BTreeSet<WorkloadId>,
        workload_id_mismatches: BTreeMap<WorkloadId, WorkloadId>,
        workload_hash_mismatches: BTreeMap<WorkloadId, (Hash256, Hash256)>,
        workload_golden_vectors: BTreeMap<WorkloadId, Vec<GoldenVectorRef>>,
        workload_resolve_calls: Cell<usize>,
        golden_vector_bytes: BTreeMap<GoldenVectorId, Vec<u8>>,
        missing_golden_vectors: BTreeSet<GoldenVectorId>,
        golden_vector_hash_mismatches: BTreeMap<GoldenVectorId, (Hash256, Hash256)>,
        unsupported_golden_vectors: BTreeSet<GoldenVectorId>,
        golden_vector_resolve_calls: Cell<usize>,
    }

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
            if let Some((expected, observed)) = self.sidecar_hash_mismatches.get(sidecar) {
                return Err(ArtifactResolveError::HashMismatch {
                    reference: format!("{:?}:{}", sidecar.kind, sidecar.id),
                    expected: *expected,
                    observed: *observed,
                });
            }
            if self.missing_sidecars.contains(sidecar) {
                return Err(ArtifactResolveError::not_found(format!(
                    "{:?}:{}",
                    sidecar.kind, sidecar.id
                )));
            }
            let bytes = self.sidecar_bytes.get(sidecar).cloned().unwrap_or_default();
            let content_hash = sha256_hash(&bytes);
            Ok(ResolvedSidecar {
                bytes,
                content_hash,
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
            self.workload_resolve_calls
                .set(self.workload_resolve_calls.get() + 1);
            if self.missing_workloads.contains(&workload.id) {
                return Err(ArtifactResolveError::not_found(format!(
                    "workload:{}",
                    workload.id
                )));
            }
            let self_hash = if let Some((expected, observed)) =
                self.workload_hash_mismatches.get(&workload.id)
            {
                if expected != &workload.manifest_hash {
                    return Err(ArtifactResolveError::HashMismatch {
                        reference: format!("workload:{}", workload.id),
                        expected: *expected,
                        observed: *observed,
                    });
                }
                *observed
            } else {
                workload.manifest_hash
            };
            let id = self
                .workload_id_mismatches
                .get(&workload.id)
                .cloned()
                .unwrap_or_else(|| workload.id.clone());
            let golden_vectors = self
                .workload_golden_vectors
                .get(&workload.id)
                .cloned()
                .unwrap_or_default();
            Ok(ResolvedWorkload {
                manifest: WorkloadManifest {
                    id,
                    schema_version: gbf_workload::WorkloadSchemaVersion { epoch: 1, minor: 0 },
                    self_hash,
                    golden_vectors,
                    future_fields: gbf_workload::WorkloadFuturePlaceholder::default(),
                },
            })
        }

        fn resolve_golden_vector(
            &self,
            vector: &GoldenVectorRef,
        ) -> Result<ResolvedGoldenVector, ArtifactResolveError> {
            self.golden_vector_resolve_calls
                .set(self.golden_vector_resolve_calls.get() + 1);
            if self.missing_golden_vectors.contains(&vector.id) {
                return Err(ArtifactResolveError::not_found(format!(
                    "golden_vector:{}",
                    vector.id.0
                )));
            }
            if let Some((expected, observed)) = self.golden_vector_hash_mismatches.get(&vector.id) {
                return Err(ArtifactResolveError::HashMismatch {
                    reference: format!("golden_vector:{}", vector.id.0),
                    expected: *expected,
                    observed: *observed,
                });
            }
            if self.unsupported_golden_vectors.contains(&vector.id) {
                return Err(ArtifactResolveError::unsupported(format!(
                    "unsupported golden vector {}",
                    vector.id.0
                )));
            }
            let bytes = self
                .golden_vector_bytes
                .get(&vector.id)
                .cloned()
                .unwrap_or_else(|| golden_vector_bytes().to_vec());
            let manifest_hash = sha256_hash(&bytes);
            Ok(ResolvedGoldenVector {
                bytes,
                manifest_hash,
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

    fn unchecked_artifact_core(
        tensors: Vec<CanonicalTensor>,
        quant: QuantSpec,
        sequence_semantics: SequenceSemanticsSpec,
    ) -> ArtifactCore {
        serde_json::from_value(serde_json::json!({
            "sequence_semantics": sequence_semantics,
            "tensors": tensors,
            "quant": quant,
        }))
        .expect("unchecked ArtifactCore fixture deserializes")
    }

    fn artifact_aux() -> ArtifactAux {
        ArtifactAux {
            checkpoint_schema: Some(SemanticCheckpointSchemaRef {
                id: SemanticCheckpointSchemaId("checkpoint.fixture".to_owned()),
                hash: sha256_hash(&[]),
            }),
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
            required_features: BTreeSet::from([
                ArtifactFeature::DenseI8,
                ArtifactFeature::LinearStateSequence,
            ]),
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
        let shards = vec![lowering_shard(
            "weight.layer0",
            LoweringShardKind::WeightShard,
            hash(0x04),
        )];
        TargetDataLoweringArtifact {
            profile: DataLoweringProfileId("dmg-default".to_owned()),
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            packer_version: PackerVersion::new(1, 0, 0),
            manifest_hash: lowering_manifest_hash(&shards),
            shards,
        }
    }

    fn lowering_shard(id: &str, kind: LoweringShardKind, payload_hash: Hash256) -> LoweringShard {
        let mut shard = LoweringShard {
            id: LoweringShardId(id.to_owned()),
            kind,
            payload_hash,
            packed_bytes_hash: Hash256::ZERO,
        };
        shard.packed_bytes_hash = sha256_hash(&shard.pack().expect("fixture lowering shard packs"));
        shard
    }

    fn lowering_manifest_hash(shards: &[LoweringShard]) -> Hash256 {
        let manifest = LoweringManifest {
            shard_refs: shards.iter().map(lowering_shard_ref).collect(),
            aggregate_hash: Hash256::ZERO,
        };
        sha256_hash(&manifest.pack().expect("fixture lowering manifest packs"))
    }

    fn dmg_family_sibling_target_id(console: ConsoleModel) -> TargetProfileId {
        let cartridge = CartridgeProfile::dmg_mbc5_8mib_128kib_battery();
        let timing = dmg_timing();
        let capabilities = CapabilitySet::default();
        TargetProfileId::from(canonical_target_profile_id(
            console,
            &cartridge,
            timing,
            capabilities,
        ))
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
        golden_vector_with_id("golden.fixture")
    }

    fn golden_vector_with_id(id: &str) -> GoldenVectorRef {
        GoldenVectorRef {
            id: GoldenVectorId(id.to_owned()),
            manifest_hash: golden_vector_hash(),
        }
    }

    fn golden_vector_bytes() -> &'static [u8] {
        b"golden vector fixture"
    }

    fn golden_vector_hash() -> Hash256 {
        sha256_hash(golden_vector_bytes())
    }

    fn layer_tensor(layer: u16) -> CanonicalTensor {
        CanonicalTensor::new(
            CanonicalTensorId::new(format!("model.layer.{layer}.bias")).expect("tensor id"),
            CanonicalTensorKind::Bias,
            CanonicalTensorLayout::new(
                CanonicalTensorShape::from_usize_dims(&[1, 1]).expect("shape"),
                TensorElementType::Float32,
            ),
            CanonicalTensorPayload::F32(vec![1.0]),
        )
        .expect("layer tensor is valid")
    }

    fn build_constraint_with_scope(
        provenance_id: TraceProbeId,
        scope: EvidenceScope,
    ) -> BuildConstraintEntry {
        BuildConstraintEntry {
            provenance_id,
            knob: CompileKnobId::Placement,
            path: None,
            value: ConstraintValue::PlacementProfile {
                value: PlacementProfile::Budgeted,
            },
            evidence: Vec::new(),
            scope,
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
            calibration_set_ref: calibration_set_ref(),
            required_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
            constraint_overrides: None,
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
        }
    }

    fn calibration_set_ref() -> CalibrationSetRef {
        BootstrapCalibrationBundle::dmg_mbc5_ref()
    }

    fn compile_profile() -> CompileProfileSpec {
        compile_profile_by_id(BRINGUP_COMPILE_PROFILE_ID)
    }

    fn compile_profile_by_id(id: &str) -> CompileProfileSpec {
        canonical_compile_profile_specs()
            .expect("canonical profiles parse")
            .into_iter()
            .find(|profile| profile.id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} profile exists"))
    }

    fn calibration() -> CalibrationBundleSet {
        BootstrapCalibrationBundle::new(active_target_profile_hash())
    }

    fn checked_in_bootstrap_calibration() -> CalibrationBundleSet {
        serde_json::from_str(include_str!(
            "../../fixtures/calibration/bootstrap-dmg-mbc5.calibration.json"
        ))
        .expect("checked-in bootstrap calibration fixture deserializes")
    }

    fn active_target_profile_hash() -> Hash256 {
        input_hash(
            "gbf-hw",
            "TargetProfile",
            "target_profile",
            "1.0.0",
            &dmg_mbc5_8mib_128kib(),
        )
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

    fn assert_no_failure_code(
        failure: &ValidationStageFailure,
        matches_code: impl Fn(&ValidationCode) -> bool,
    ) {
        assert!(
            failure
                .diagnostics
                .iter()
                .all(|diagnostic| !matches_code(&diagnostic.code)),
            "diagnostics were {:#?}",
            failure.diagnostics
        );
    }
}
