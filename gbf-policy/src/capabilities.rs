//! Capability registries consumed by pipeline-entry validation.

use std::collections::BTreeSet;

use gbf_foundation::{ArtifactFeature, TargetProfileId};
use gbf_hw::target::TargetProfile;

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
        if requested.as_str() == self.target.as_str() {
            Ok(())
        } else {
            Err(TargetIncompatibilityReason::TargetFamilyMismatch)
        }
    }
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
    use gbf_hw::timing::{DOT_CLOCK_HZ, FRAME_DOTS, TimingProfile, VBLANK_DOTS};

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
}
