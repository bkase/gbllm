//! Capability registries consumed by pipeline-entry validation.

use std::collections::BTreeSet;

use gbf_foundation::{ArtifactFeature, DataLoweringProfileId, TargetFamilyId, TargetProfileId};
use gbf_hw::target::{DMG_TARGET_FAMILY_ID, TargetProfile};

use crate::compile::{CompilerFeature, RuntimeMode};
use crate::diagnostics::TargetIncompatibilityReason;

pub const STAGE0_CLASS10_TARGET_CAPABILITY_OWNER: &str =
    "gbf-policy.stage0-class10.target-capability-registry.v1";
pub const STAGE0_COMPILER_FEATURE_REGISTRY_OWNER: &str = concat!(
    "gbf-policy.stage0-class10.compiler-feature-registry.",
    env!("CARGO_PKG_VERSION"),
    ".v1"
);

pub const STAGE0_COMPILER_SUPPORTED_FEATURES: &[CompilerFeature] = &[
    CompilerFeature::ArtifactValidation,
    CompilerFeature::PolicyResolution,
];

pub const STAGE0_CLASS10_TARGET_CAPABILITY_RULES: &[Stage0Class10TargetCapabilityRule] = &[
    Stage0Class10TargetCapabilityRule {
        requirement: TargetCapabilityRequirement::Always,
        artifact_features: &[
            ArtifactFeature::DenseI8,
            ArtifactFeature::Ternary2Quant,
            ArtifactFeature::Binary1Quant,
            ArtifactFeature::SparseTernaryBitplanes,
            ArtifactFeature::LinearStateSequence,
            ArtifactFeature::BoundedKvSequence,
        ],
        runtime_modes: &[
            RuntimeMode::Interactive,
            RuntimeMode::Steady,
            RuntimeMode::Safe,
        ],
    },
    Stage0Class10TargetCapabilityRule {
        requirement: TargetCapabilityRequirement::CgbDoubleSpeedVramDma,
        artifact_features: &[ArtifactFeature::MoeRouting],
        runtime_modes: &[RuntimeMode::Trace],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage0Class10TargetCapabilities {
    pub owner: &'static str,
    pub target: TargetProfileId,
    pub target_family: TargetFamilyId,
    pub compatible_lowering_profile: DataLoweringProfileId,
    pub supported_artifact_features: BTreeSet<ArtifactFeature>,
    pub supported_runtime_modes: BTreeSet<RuntimeMode>,
}

impl Stage0Class10TargetCapabilities {
    #[must_use]
    pub fn from_target_profile(target_profile: &TargetProfile) -> Self {
        let mut supported_artifact_features = BTreeSet::new();
        let mut supported_runtime_modes = BTreeSet::new();

        for rule in STAGE0_CLASS10_TARGET_CAPABILITY_RULES {
            if !rule.requirement.is_satisfied_by(target_profile) {
                continue;
            }
            supported_artifact_features.extend(rule.artifact_features.iter().copied());
            supported_runtime_modes.extend(rule.runtime_modes.iter().copied());
        }

        Self {
            owner: STAGE0_CLASS10_TARGET_CAPABILITY_OWNER,
            target: target_profile.id().clone(),
            target_family: target_profile.family().clone(),
            compatible_lowering_profile: stage0_class10_lowering_profile_for_family(
                target_profile.family(),
            ),
            supported_artifact_features,
            supported_runtime_modes,
        }
    }

    #[must_use]
    pub fn supports_artifact_feature(&self, feature: ArtifactFeature) -> bool {
        self.supported_artifact_features.contains(&feature)
    }

    #[must_use]
    pub fn supports_runtime_mode(&self, mode: RuntimeMode) -> bool {
        self.supported_runtime_modes.contains(&mode)
    }

    pub fn target_compatibility(
        &self,
        requested: &TargetProfileId,
    ) -> Result<(), TargetIncompatibilityReason> {
        let Some(requested_family) = stage0_class10_target_family_for_profile_id(requested) else {
            return Err(TargetIncompatibilityReason::MissingLoweringProfile);
        };

        if requested_family != self.target_family {
            return Err(TargetIncompatibilityReason::TargetFamilyMismatch);
        }

        if stage0_class10_lowering_profile_for_family(&requested_family)
            != self.compatible_lowering_profile
        {
            return Err(TargetIncompatibilityReason::MissingLoweringProfile);
        }

        Ok(())
    }
}

/// Resolve canonical target profile ids onto the class-10 lowering family axis.
///
/// Stage 0 class 10 only proves compatibility with the artifact's
/// target-family lowering. Unknown profile ids therefore do not imply a family
/// mismatch; callers surface those as a missing lowering profile instead.
#[must_use]
pub fn stage0_class10_target_family_for_profile_id(
    profile_id: &TargetProfileId,
) -> Option<TargetFamilyId> {
    let profile_id = profile_id.as_str();
    if profile_id.starts_with("dmg-")
        || profile_id.starts_with("mgb-")
        || profile_id.starts_with("sgb-")
    {
        Some(TargetFamilyId::from_static(DMG_TARGET_FAMILY_ID))
    } else if profile_id.starts_with("cgb-") {
        Some(TargetFamilyId::from_static("cgb"))
    } else {
        None
    }
}

/// Return the family-default lowering profile Stage 0 class 10 can accept.
#[must_use]
pub fn stage0_class10_lowering_profile_for_family(
    family: &TargetFamilyId,
) -> DataLoweringProfileId {
    DataLoweringProfileId(format!("{}-default", family.as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stage0Class10TargetCapabilityRule {
    pub requirement: TargetCapabilityRequirement,
    pub artifact_features: &'static [ArtifactFeature],
    pub runtime_modes: &'static [RuntimeMode],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCapabilityRequirement {
    Always,
    CgbDoubleSpeedVramDma,
}

impl TargetCapabilityRequirement {
    #[must_use]
    pub fn is_satisfied_by(self, target_profile: &TargetProfile) -> bool {
        match self {
            Self::Always => true,
            Self::CgbDoubleSpeedVramDma => {
                let capabilities = target_profile.capabilities();
                capabilities.double_speed_mode && capabilities.vram_dma
            }
        }
    }
}

#[must_use]
pub fn compiler_build_supports_feature(feature: CompilerFeature) -> bool {
    STAGE0_COMPILER_SUPPORTED_FEATURES.contains(&feature)
}

#[cfg(test)]
mod tests {
    use gbf_foundation::TargetFamilyId;
    use gbf_hw::cartridge_header::{RamSize, RomSize};
    use gbf_hw::target::{
        CapabilitySet, CartridgeProfile, ConsoleModel, TargetProfile, canonical_target_profile_id,
        dmg_mbc5_8mib_128kib,
    };
    use gbf_hw::timing::{DOT_CLOCK_HZ, FRAME_DOTS, TimingProfile, VBLANK_DOTS, dmg_timing};

    use super::*;

    #[test]
    fn stage0_compiler_feature_registry_is_owner_named() {
        assert!(STAGE0_COMPILER_FEATURE_REGISTRY_OWNER.contains("gbf-policy.stage0-class10"));
        assert!(compiler_build_supports_feature(
            CompilerFeature::ArtifactValidation
        ));
        assert!(compiler_build_supports_feature(
            CompilerFeature::PolicyResolution
        ));
        assert!(!compiler_build_supports_feature(
            CompilerFeature::StaticBudgetReport
        ));
    }

    #[test]
    fn stage0_target_capabilities_are_explicit_for_dmg_bringup() {
        let contract =
            Stage0Class10TargetCapabilities::from_target_profile(&dmg_mbc5_8mib_128kib());

        assert_eq!(contract.owner, STAGE0_CLASS10_TARGET_CAPABILITY_OWNER);
        assert_eq!(contract.target_family, TargetFamilyId::from("dmg"));
        assert_eq!(
            contract.compatible_lowering_profile,
            DataLoweringProfileId("dmg-default".to_owned())
        );
        assert!(contract.supports_artifact_feature(ArtifactFeature::DenseI8));
        assert!(contract.supports_artifact_feature(ArtifactFeature::LinearStateSequence));
        assert!(!contract.supports_artifact_feature(ArtifactFeature::MoeRouting));
        assert!(contract.supports_runtime_mode(RuntimeMode::Safe));
        assert!(!contract.supports_runtime_mode(RuntimeMode::Trace));
    }

    #[test]
    fn stage0_target_capabilities_include_cgb_accelerated_rule() {
        let target = cgb_double_speed_vram_dma_target();
        let contract = Stage0Class10TargetCapabilities::from_target_profile(&target);

        assert!(contract.supports_artifact_feature(ArtifactFeature::MoeRouting));
        assert!(contract.supports_runtime_mode(RuntimeMode::Trace));
    }

    #[test]
    fn stage0_target_compatibility_uses_family_lowering_axis() {
        let contract =
            Stage0Class10TargetCapabilities::from_target_profile(&dmg_mbc5_8mib_128kib());
        let same_family_request = dmg_family_sibling_target_id(ConsoleModel::Mgb);
        let different_family_request = cgb_double_speed_vram_dma_target().id().clone();

        assert_eq!(
            stage0_class10_target_family_for_profile_id(&same_family_request),
            Some(TargetFamilyId::from("dmg"))
        );
        assert_eq!(contract.target_compatibility(&same_family_request), Ok(()));
        assert_eq!(
            contract.target_compatibility(&different_family_request),
            Err(TargetIncompatibilityReason::TargetFamilyMismatch)
        );
        assert_eq!(
            contract.target_compatibility(&TargetProfileId::from("fixture-target-without-family")),
            Err(TargetIncompatibilityReason::MissingLoweringProfile)
        );
    }

    fn cgb_double_speed_vram_dma_target() -> TargetProfile {
        let cartridge = CartridgeProfile::try_new(
            gbf_hw::cartridge_header::MbcType::Mbc5RamBattery,
            RomSize::Mib8,
            RamSize::Kib128,
            true,
            false,
        )
        .expect("fixture cartridge is valid");
        let timing =
            TimingProfile::try_new(DOT_CLOCK_HZ, 2, FRAME_DOTS, VBLANK_DOTS).expect("cgb timing");
        let capabilities = CapabilitySet {
            double_speed_mode: true,
            vram_dma: true,
            rtc_present: false,
        };
        let id = canonical_target_profile_id(ConsoleModel::Cgb, &cartridge, timing, capabilities);
        TargetProfile::try_new(
            TargetProfileId::from(id),
            TargetFamilyId::from("cgb"),
            ConsoleModel::Cgb,
            cartridge,
            timing,
            capabilities,
        )
        .expect("fixture target is valid")
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
}
