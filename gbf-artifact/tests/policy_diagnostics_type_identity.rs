use gbf_artifact::{
    ArtifactFeature, ArtifactSchemaVersion, ComponentId, DataLoweringProfileId, GoldenVectorId,
    LineageId, LoweringShardId, LoweringShardRef, ManifestInvariant, SidecarKind,
};
use gbf_foundation::{FieldPath, Hash256, TargetProfileId};
use gbf_policy::diagnostics::{ValidationCode, ValidationDiagnostic, ValidationOrigin};

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[test]
fn policy_diagnostics_accept_artifact_carriers_by_type_identity() {
    let manifest_invariant = ManifestInvariant::FeatureSetEpochInconsistent {
        epoch: ArtifactSchemaVersion { epoch: 1, minor: 0 },
        feature: ArtifactFeature::MoeRouting,
    };
    let _: gbf_policy::diagnostics::ManifestInvariant = manifest_invariant.clone();
    let _ = ValidationDiagnostic::hard(
        ValidationOrigin::Manifest,
        ValidationCode::ManifestInvariantViolated {
            invariant: manifest_invariant,
        },
        gbf_policy::diagnostics::ValidationDetail::None,
        Vec::new(),
    );

    let sidecar_kind = SidecarKind::GoldenVector;
    let _: gbf_policy::diagnostics::SidecarKind = sidecar_kind;
    let _ = ValidationCode::ArtifactAuxSidecarMissing { kind: sidecar_kind };

    let lowering_profile = DataLoweringProfileId("dmg-default".to_owned());
    let _: gbf_policy::diagnostics::DataLoweringProfileId = lowering_profile.clone();
    let _ = ValidationCode::LoweringMissingForTarget {
        target: TargetProfileId::from("dmg-mbc5"),
        lowering_profile,
    };

    let shard = LoweringShardRef {
        id: LoweringShardId("weights.0".to_owned()),
        manifest_hash: hash(4),
    };
    let _: gbf_policy::diagnostics::LoweringShardRef = shard.clone();
    let _ = ValidationCode::LoweringRoundTripFailed { shard };

    let vector = GoldenVectorId("vec.smoke.001".to_owned());
    let _: gbf_policy::diagnostics::GoldenVectorId = vector.clone();
    let _ = ValidationCode::GoldenVectorMissing { vector };

    let component = ComponentId("core".to_owned());
    let lineage = LineageId(hash(5));
    let _: gbf_policy::diagnostics::ComponentId = component.clone();
    let _: gbf_policy::diagnostics::LineageId = lineage.clone();
    let _ = ManifestInvariant::ComponentDigestMismatch {
        component,
        expected: hash(1),
        observed: hash(2),
    };
    let _ = ManifestInvariant::LineageContradiction {
        derived: lineage,
        recorded: LineageId(hash(6)),
    };
    let _ = ValidationCode::ArtifactForbiddenBuildIdentityField {
        field: FieldPath::from("manifest.build_identity"),
    };
}
