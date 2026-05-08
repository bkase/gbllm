//! Shared validation diagnostic taxonomy.

use gbf_foundation::{
    BlobRef, BudgetSlotId, CompileProfileId, ExpertId, FieldPath, Hash256, LayerId, PackerVersion,
    SemVer, TargetProfileId, WorkloadId,
};
use serde::{Deserialize, Serialize};

use crate::calibration::CalibrationLayer;
use crate::compile::{
    CompileKnobBounds, CompileKnobId, CompilerFeature, ConstraintValue, EvidenceRef,
    PlacementProfile, RuntimeMode, SelectorPath,
};
use crate::risk::CalibrationConfidenceClass;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}

impl ValidationDiagnostic {
    #[must_use]
    pub fn new(
        severity: DiagnosticSeverity,
        origin: ValidationOrigin,
        code: ValidationCode,
        detail: ValidationDetail,
        provenance: Vec<EvidenceRef>,
    ) -> Self {
        Self {
            severity,
            origin,
            code,
            detail,
            provenance,
        }
    }

    #[must_use]
    pub fn hard(
        origin: ValidationOrigin,
        code: ValidationCode,
        detail: ValidationDetail,
        provenance: Vec<EvidenceRef>,
    ) -> Self {
        Self::new(DiagnosticSeverity::Hard, origin, code, detail, provenance)
    }

    #[must_use]
    pub fn soft(
        origin: ValidationOrigin,
        code: ValidationCode,
        detail: ValidationDetail,
        provenance: Vec<EvidenceRef>,
    ) -> Self {
        Self::new(DiagnosticSeverity::Soft, origin, code, detail, provenance)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DiagnosticSeverity {
    Hard,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[serde(tag = "kind", content = "fields", deny_unknown_fields)]
pub enum ValidationCode {
    SchemaEpochUnsupported,
    SchemaCompatibilityAdapterMissing {
        observed: SemVer,
        target: SemVer,
    },
    SchemaCompatibilityAdapterNotLossless {
        adapter: CompatibilityAdapterId,
    },
    SemanticCoreHashMismatch,
    ArtifactTransportManifestMismatch,
    ManifestInvariantViolated {
        invariant: ManifestInvariant,
    },
    ArtifactPayloadMalformed {
        field: FieldPath,
    },
    ArtifactBlobDigestMismatch {
        blob: BlobRef,
        expected: Hash256,
        observed: Hash256,
    },
    ArtifactAuxMalformed {
        field: FieldPath,
    },
    ArtifactAuxSidecarMissing {
        kind: SidecarKind,
    },
    ArtifactAuxSidecarDigestMismatch {
        kind: SidecarKind,
        expected: Hash256,
        observed: Hash256,
    },
    ArtifactForbiddenBuildIdentityField {
        field: FieldPath,
    },
    ArtifactRequiredFeatureUnsupported {
        feature: ArtifactFeature,
    },
    LoweringMissingForTarget {
        target: TargetProfileId,
        lowering_profile: DataLoweringProfileId,
    },
    LoweringRoundTripFailed {
        shard: LoweringShardRef,
    },
    LoweringPackerVersionMismatch {
        artifact_version: PackerVersion,
        runtime_version: PackerVersion,
    },
    CalibrationMissing {
        class: CalibrationLayer,
    },
    CalibrationStale {
        class: CalibrationLayer,
        declared: Hash256,
        observed: Hash256,
    },
    CalibrationConfidenceTooLow {
        required: CalibrationConfidenceClass,
        observed: CalibrationConfidenceClass,
    },
    HintProvenanceInconsistent {
        fact: TraceProbeId,
    },
    WorkloadRefUnresolved {
        workload: WorkloadId,
    },
    GoldenVectorMissing {
        vector: GoldenVectorId,
    },
    GoldenVectorDigestMismatch {
        vector: GoldenVectorId,
        expected: Hash256,
        observed: Hash256,
    },
    CompileRequestUnsupportedFeature {
        feature: CompilerFeature,
    },
    CompileRequestProfileForbidsObjective {
        profile: CompileProfileId,
        reason: ObjectiveRejection,
    },
    CompileRequestRuntimeModeUnsupported {
        mode: RuntimeMode,
    },
    CompileRequestTargetIncompatible {
        target: TargetProfileId,
        reason: TargetIncompatibilityReason,
    },
    PolicyKnobOutOfBounds {
        knob: CompileKnobId,
        requested: KnobValueDescriptor,
        bounds: CompileKnobBounds,
    },
    PolicyConstraintUnsatisfiable {
        knob: CompileKnobId,
        left: CompileKnobBounds,
        right: CompileKnobBounds,
    },
    PolicyKnobLockedAndOverridden {
        knob: CompileKnobId,
    },
    BudgetMissingRuntimeChromeBudget,
    BudgetQuantGraphViewMalformed {
        field: FieldPath,
    },
    BudgetExpertExceedsSlot {
        layer: LayerId,
        expert: ExpertId,
        slot: BudgetSlotId,
        payload_bytes: u32,
        cap_bytes: u32,
    },
    BudgetCommonBankExceedsCap {
        assigned_bytes: u32,
        cap_bytes: u32,
    },
    BudgetWramPeakExceeds {
        peak: u32,
        cap: u32,
    },
    BudgetSramPeakExceeds {
        peak: u32,
        cap: u32,
    },
    BudgetHramPeakExceeds {
        peak: u32,
        cap: u32,
    },
    BudgetAccumulatorOverflow {
        site: ReductionSiteId,
        projected_max_abs: u64,
    },
    BudgetSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    BudgetSramPageSwitchesPerTokenOverCap {
        decision_value: u16,
        upper_bound: u16,
        cap: u16,
        source: SwitchProjectionSource,
    },
    BudgetPlacementProfileInfeasible {
        profile: PlacementProfile,
        reason: PlacementInfeasibilityReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValidationDetail {
    None,
    HashMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    Bytes {
        observed: u32,
        cap: u32,
    },
    Range {
        observed_lo: i64,
        observed_hi: i64,
        cap_lo: i64,
        cap_hi: i64,
    },
    Selector {
        selector: SelectorPath,
    },
    Field {
        field: FieldPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CompatibilityAdapterId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ManifestInvariant {
    FeatureSetEpochInconsistent {
        epoch: ArtifactSchemaVersion,
        feature: ArtifactFeature,
    },
    RequiredComponentMissing {
        component: ComponentId,
    },
    ComponentDigestMismatch {
        component: ComponentId,
        expected: Hash256,
        observed: Hash256,
    },
    LineageContradiction {
        derived: LineageId,
        recorded: LineageId,
    },
    ManifestSelfHashMismatch {
        recomputed: Hash256,
        recorded: Hash256,
    },
    ForbiddenBuildIdentityField {
        field: FieldPath,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSchemaVersion {
    pub epoch: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ArtifactFeature {
    DenseI8,
    Ternary2Quant,
    Binary1Quant,
    SparseTernaryBitplanes,
    MoeRouting,
    LinearStateSequence,
    BoundedKvSequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SidecarKind {
    GoldenVector,
    SemanticCheckpointSchema,
    ConformanceEnvelope,
    ReferenceObservationCache,
    InteractionBundle,
    LexicalSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComponentId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LineageId(pub Hash256);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataLoweringProfileId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoweringShardRef {
    pub id: LoweringShardId,
    pub manifest_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoweringShardId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TraceProbeId(pub u16);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoldenVectorId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ObjectiveRejection {
    ServiceLevelTooStrict,
    RomBudgetTooStrict,
    RuntimeSwitchBudgetTooStrict,
    RiskPolicyNotSupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TargetIncompatibilityReason {
    TargetFamilyMismatch,
    MissingLoweringProfile,
    UnsupportedRuntimeMode,
    UnsupportedCompilerFeature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobValueDescriptor {
    pub value: ConstraintValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReductionSiteId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SwitchProjectionSource {
    ConservativeStaticUpperBound,
    HintWeightedExpectedWithStaticCap,
    CalibrationClosedFormWithStaticCap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PlacementInfeasibilityReason {
    RequiresUnavailableSlotClass,
    ExceedsCommonBankCap,
    ExceedsExpertBankCap,
    ViolatesTargetLayout,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::canonical_default_bounds_fixture;
    use gbf_foundation::BlobCodec;

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn provenance() -> Vec<EvidenceRef> {
        vec![EvidenceRef {
            kind: "Fixture".to_owned(),
            reference: "diagnostics".to_owned(),
            hash: Some(hash(9)),
        }]
    }

    fn diagnostic(code: ValidationCode) -> ValidationDiagnostic {
        ValidationDiagnostic::hard(
            ValidationOrigin::PolicyResolution,
            code,
            ValidationDetail::None,
            provenance(),
        )
    }

    fn assert_diagnostic_round_trip(diagnostic: ValidationDiagnostic) {
        let encoded = serde_json::to_string(&diagnostic).expect("diagnostic serializes");
        let decoded: ValidationDiagnostic =
            serde_json::from_str(&encoded).expect("diagnostic deserializes");

        assert_eq!(decoded, diagnostic);
    }

    fn assert_code_round_trip(code: ValidationCode) {
        assert_diagnostic_round_trip(diagnostic(code));
    }

    #[test]
    fn validation_diagnostic_round_trips_through_serde() {
        assert_diagnostic_round_trip(ValidationDiagnostic::new(
            DiagnosticSeverity::Soft,
            ValidationOrigin::Schema,
            ValidationCode::SchemaEpochUnsupported,
            ValidationDetail::Field {
                field: FieldPath::from("schema.epoch"),
            },
            provenance(),
        ));
    }

    #[test]
    fn validation_detail_round_trips_through_serde() {
        for detail in [
            ValidationDetail::None,
            ValidationDetail::HashMismatch {
                expected: hash(1),
                observed: hash(2),
            },
            ValidationDetail::Bytes {
                observed: 17,
                cap: 11,
            },
            ValidationDetail::Range {
                observed_lo: -3,
                observed_hi: 14,
                cap_lo: 0,
                cap_hi: 10,
            },
            ValidationDetail::Selector {
                selector: SelectorPath("experts[0]".to_owned()),
            },
            ValidationDetail::Field {
                field: FieldPath::from("manifest.lineage"),
            },
        ] {
            let encoded = serde_json::to_string(&detail).expect("detail serializes");
            let decoded: ValidationDetail =
                serde_json::from_str(&encoded).expect("detail deserializes");

            assert_eq!(decoded, detail);
        }
    }

    #[test]
    fn validation_code_round_trips_every_variant() {
        let versions = (SemVer::new(1, 2, 3), SemVer::new(2, 0, 0));
        let blob = BlobRef {
            hash: hash(3),
            len: 32,
            codec: BlobCodec::Raw,
        };
        let shard = LoweringShardRef {
            id: LoweringShardId("weights.0".to_owned()),
            manifest_hash: hash(4),
        };
        let bounds = canonical_default_bounds_fixture();

        for code in [
            ValidationCode::SchemaEpochUnsupported,
            ValidationCode::SchemaCompatibilityAdapterMissing {
                observed: versions.0,
                target: versions.1,
            },
            ValidationCode::SchemaCompatibilityAdapterNotLossless {
                adapter: CompatibilityAdapterId("adapter.v1".to_owned()),
            },
            ValidationCode::SemanticCoreHashMismatch,
            ValidationCode::ArtifactTransportManifestMismatch,
            ValidationCode::ManifestInvariantViolated {
                invariant: ManifestInvariant::ForbiddenBuildIdentityField {
                    field: FieldPath::from("build.host"),
                },
            },
            ValidationCode::ArtifactPayloadMalformed {
                field: FieldPath::from("core.tensors"),
            },
            ValidationCode::ArtifactBlobDigestMismatch {
                blob,
                expected: hash(1),
                observed: hash(2),
            },
            ValidationCode::ArtifactAuxMalformed {
                field: FieldPath::from("aux.golden_vectors"),
            },
            ValidationCode::ArtifactAuxSidecarMissing {
                kind: SidecarKind::GoldenVector,
            },
            ValidationCode::ArtifactAuxSidecarDigestMismatch {
                kind: SidecarKind::SemanticCheckpointSchema,
                expected: hash(5),
                observed: hash(6),
            },
            ValidationCode::ArtifactForbiddenBuildIdentityField {
                field: FieldPath::from("manifest.build_identity"),
            },
            ValidationCode::ArtifactRequiredFeatureUnsupported {
                feature: ArtifactFeature::MoeRouting,
            },
            ValidationCode::LoweringMissingForTarget {
                target: TargetProfileId::from("dmg-mbc5"),
                lowering_profile: DataLoweringProfileId("dmg-default".to_owned()),
            },
            ValidationCode::LoweringRoundTripFailed {
                shard: shard.clone(),
            },
            ValidationCode::LoweringPackerVersionMismatch {
                artifact_version: PackerVersion::new(1, 0, 0),
                runtime_version: PackerVersion::new(2, 0, 0),
            },
            ValidationCode::CalibrationMissing {
                class: CalibrationLayer::Kernel,
            },
            ValidationCode::CalibrationStale {
                class: CalibrationLayer::Platform,
                declared: hash(7),
                observed: hash(8),
            },
            ValidationCode::CalibrationConfidenceTooLow {
                required: CalibrationConfidenceClass::Reasonable,
                observed: CalibrationConfidenceClass::Weak,
            },
            ValidationCode::HintProvenanceInconsistent {
                fact: TraceProbeId(2),
            },
            ValidationCode::WorkloadRefUnresolved {
                workload: WorkloadId::from("smoke"),
            },
            ValidationCode::GoldenVectorMissing {
                vector: GoldenVectorId("vec.smoke.001".to_owned()),
            },
            ValidationCode::GoldenVectorDigestMismatch {
                vector: GoldenVectorId("vec.smoke.002".to_owned()),
                expected: hash(10),
                observed: hash(11),
            },
            ValidationCode::CompileRequestUnsupportedFeature {
                feature: CompilerFeature::StaticBudgetReport,
            },
            ValidationCode::CompileRequestProfileForbidsObjective {
                profile: CompileProfileId::from("Bringup"),
                reason: ObjectiveRejection::ServiceLevelTooStrict,
            },
            ValidationCode::CompileRequestRuntimeModeUnsupported {
                mode: RuntimeMode::Trace,
            },
            ValidationCode::CompileRequestTargetIncompatible {
                target: TargetProfileId::from("gbc-mbc5"),
                reason: TargetIncompatibilityReason::MissingLoweringProfile,
            },
            ValidationCode::PolicyKnobOutOfBounds {
                knob: CompileKnobId::Placement,
                requested: KnobValueDescriptor {
                    value: ConstraintValue::PlacementProfile {
                        value: PlacementProfile::PackedExperts,
                    },
                },
                bounds: bounds.clone(),
            },
            ValidationCode::PolicyConstraintUnsatisfiable {
                knob: CompileKnobId::Schedule,
                left: bounds.clone(),
                right: bounds.clone(),
            },
            ValidationCode::PolicyKnobLockedAndOverridden {
                knob: CompileKnobId::RomWindow,
            },
            ValidationCode::BudgetMissingRuntimeChromeBudget,
            ValidationCode::BudgetQuantGraphViewMalformed {
                field: FieldPath::from("quant_graph.layers[0]"),
            },
            ValidationCode::BudgetExpertExceedsSlot {
                layer: LayerId::new(1),
                expert: ExpertId::new(2),
                slot: BudgetSlotId::new(3),
                payload_bytes: 9000,
                cap_bytes: 8192,
            },
            ValidationCode::BudgetCommonBankExceedsCap {
                assigned_bytes: 20_000,
                cap_bytes: 16_384,
            },
            ValidationCode::BudgetWramPeakExceeds {
                peak: 5000,
                cap: 4096,
            },
            ValidationCode::BudgetSramPeakExceeds {
                peak: 9000,
                cap: 8192,
            },
            ValidationCode::BudgetHramPeakExceeds {
                peak: 256,
                cap: 127,
            },
            ValidationCode::BudgetAccumulatorOverflow {
                site: ReductionSiteId("ffn.0.acc".to_owned()),
                projected_max_abs: i32::MAX as u64 + 1,
            },
            ValidationCode::BudgetSwitchesPerTokenOverCap {
                decision_value: 7,
                upper_bound: 9,
                cap: 5,
                source: SwitchProjectionSource::ConservativeStaticUpperBound,
            },
            ValidationCode::BudgetSramPageSwitchesPerTokenOverCap {
                decision_value: 3,
                upper_bound: 4,
                cap: 2,
                source: SwitchProjectionSource::CalibrationClosedFormWithStaticCap,
            },
            ValidationCode::BudgetPlacementProfileInfeasible {
                profile: PlacementProfile::PackedExperts,
                reason: PlacementInfeasibilityReason::ExceedsExpertBankCap,
            },
        ] {
            assert_code_round_trip(code);
        }
    }

    #[test]
    fn policy_constraint_unsatisfiable_round_trip() {
        assert_code_round_trip(ValidationCode::PolicyConstraintUnsatisfiable {
            knob: CompileKnobId::Placement,
            left: canonical_default_bounds_fixture(),
            right: CompileKnobBounds {
                placement: crate::compile::PlacementKnobBounds {
                    max_profile: PlacementProfile::StrictOnePerBank,
                },
                ..canonical_default_bounds_fixture()
            },
        });
    }

    #[test]
    fn budget_quant_graph_view_malformed_round_trip() {
        assert_code_round_trip(ValidationCode::BudgetQuantGraphViewMalformed {
            field: FieldPath::from("budget_view.per_expert_payload"),
        });
    }

    #[test]
    fn artifact_forbidden_build_identity_field_round_trip() {
        assert_code_round_trip(ValidationCode::ArtifactForbiddenBuildIdentityField {
            field: FieldPath::from("aux.build_identity.git_sha"),
        });
    }

    #[test]
    fn validation_diagnostic_rejects_unknown_fields() {
        let mut value = serde_json::to_value(diagnostic(ValidationCode::SchemaEpochUnsupported))
            .expect("diagnostic serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<ValidationDiagnostic>(value).is_err());
    }

    #[test]
    fn validation_code_rejects_unknown_fields() {
        let mut value = serde_json::to_value(ValidationCode::BudgetQuantGraphViewMalformed {
            field: FieldPath::from("budget_view"),
        })
        .expect("code serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<ValidationCode>(value).is_err());
    }

    #[test]
    fn manifest_invariant_carrier_values_round_trip() {
        for invariant in [
            ManifestInvariant::FeatureSetEpochInconsistent {
                epoch: ArtifactSchemaVersion { epoch: 1, minor: 0 },
                feature: ArtifactFeature::DenseI8,
            },
            ManifestInvariant::RequiredComponentMissing {
                component: ComponentId("core".to_owned()),
            },
            ManifestInvariant::ComponentDigestMismatch {
                component: ComponentId("core".to_owned()),
                expected: hash(1),
                observed: hash(2),
            },
            ManifestInvariant::LineageContradiction {
                derived: LineageId(hash(3)),
                recorded: LineageId(hash(4)),
            },
            ManifestInvariant::ManifestSelfHashMismatch {
                recomputed: hash(5),
                recorded: hash(6),
            },
            ManifestInvariant::ForbiddenBuildIdentityField {
                field: FieldPath::from("manifest.created_by"),
            },
        ] {
            let encoded = serde_json::to_string(&invariant).expect("invariant serializes");
            let decoded: ManifestInvariant =
                serde_json::from_str(&encoded).expect("invariant deserializes");

            assert_eq!(decoded, invariant);
        }
    }
}
