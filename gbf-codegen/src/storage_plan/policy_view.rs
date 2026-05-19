//! Normalized policy view consumed by Stage 6 storage rules.

use std::collections::BTreeSet;
use std::{error::Error, fmt};

use gbf_policy::{
    ObservabilityMode, ResolvedCompilePolicy, StorageMaterialization, StoragePlanDiagnosticCode,
};
use serde::{Deserialize, Serialize};

use crate::s3::infer_ir::ValueId;
use crate::storage_plan::types::StorageClass;

pub const SOFT_PRESSURE_FRACTION: Rational = Rational {
    numerator: 85,
    denominator: 100,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rational {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyView {
    pub compile_knobs: StoragePolicyCompileKnobs,
    pub runtime_chrome_budget: StorageRuntimeChromeBudget,
    pub storage_pressure_budget: StoragePressureBudget,
    pub trace_capture_policy: TraceCapturePolicy,
    pub transcript_capture_policy: TranscriptCapturePolicy,
    pub build_profile: BuildProfile,
}

impl StoragePolicyView {
    #[must_use]
    pub fn from_resolved_policy(policy: &ResolvedCompilePolicy) -> Self {
        let budget = policy.effective_constraints.runtime_chrome_budget.as_ref();
        let wram_soft_bytes = budget
            .map(|budget| budget.memory_caps.wram_usable_bytes)
            .unwrap_or_default();
        let hram_soft_bytes = budget
            .map(|budget| budget.memory_caps.hram_usable_bytes)
            .unwrap_or_default();
        let wram_reserved_bytes = budget
            .map(|budget| u32::from(budget.wram_reserved))
            .unwrap_or_default();

        Self {
            compile_knobs: StoragePolicyCompileKnobs {
                global: StoragePolicyGlobalKnobs {
                    storage: StoragePolicyStorageKnobs {
                        recompute_promotion: policy.knobs.global.storage.materialization,
                        recompute_cycle_ceiling: policy
                            .objective
                            .max_cycles_per_token
                            .unwrap_or(u32::MAX),
                    },
                },
                bounds: StoragePolicyKnobBounds {
                    max_recompute_promotion: policy.knobs.bounds.storage.max_materialization,
                },
                overrides: StoragePolicyKnobOverrides::default(),
            },
            runtime_chrome_budget: StorageRuntimeChromeBudget {
                wram_hot: RuntimeClassChromeBudget {
                    reserved_bytes: wram_reserved_bytes,
                },
                hram_hot: RuntimeClassChromeBudget { reserved_bytes: 0 },
            },
            storage_pressure_budget: StoragePressureBudget {
                wram_hot: StorageClassPressureBudget {
                    soft_bytes: wram_soft_bytes,
                },
                hram_hot: StorageClassPressureBudget {
                    soft_bytes: hram_soft_bytes,
                },
            },
            trace_capture_policy: TraceCapturePolicy::default(),
            transcript_capture_policy: TranscriptCapturePolicy {
                enabled: matches!(policy.observability, ObservabilityMode::Flexible),
            },
            build_profile: BuildProfile {
                kind: BuildProfileKind::from_profile_id(policy.profile.as_str()),
            },
        }
    }
}

impl From<&ResolvedCompilePolicy> for StoragePolicyView {
    fn from(policy: &ResolvedCompilePolicy) -> Self {
        Self::from_resolved_policy(policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyCompileKnobs {
    pub global: StoragePolicyGlobalKnobs,
    pub bounds: StoragePolicyKnobBounds,
    pub overrides: StoragePolicyKnobOverrides,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyGlobalKnobs {
    pub storage: StoragePolicyStorageKnobs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyStorageKnobs {
    pub recompute_promotion: StorageMaterialization,
    pub recompute_cycle_ceiling: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyKnobBounds {
    pub max_recompute_promotion: StorageMaterialization,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyKnobOverrides {
    pub forced_recompute: BTreeSet<ValueId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageRuntimeChromeBudget {
    pub wram_hot: RuntimeClassChromeBudget,
    pub hram_hot: RuntimeClassChromeBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeClassChromeBudget {
    pub reserved_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePressureBudget {
    pub wram_hot: StorageClassPressureBudget,
    pub hram_hot: StorageClassPressureBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageClassPressureBudget {
    pub soft_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceCapturePolicy {
    pub enabled_probes: BTreeSet<ValueId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptCapturePolicy {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildProfile {
    pub kind: BuildProfileKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BuildProfileKind {
    Bringup,
    Default,
    Trace,
    Recovery,
    Custom { id: String },
}

impl BuildProfileKind {
    #[must_use]
    pub fn from_profile_id(profile: &str) -> Self {
        match profile {
            gbf_policy::BRINGUP_COMPILE_PROFILE_ID => Self::Bringup,
            gbf_policy::DEFAULT_COMPILE_PROFILE_ID => Self::Default,
            gbf_policy::TRACE_COMPILE_PROFILE_ID => Self::Trace,
            gbf_policy::RECOVERY_COMPILE_PROFILE_ID => Self::Recovery,
            custom => Self::Custom {
                id: custom.to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicyDiagnostic {
    pub code: StoragePlanDiagnosticCode,
    pub class: StorageClass,
    pub soft_bytes: u32,
    pub reserved_bytes: u32,
}

impl StoragePolicyDiagnostic {
    #[must_use]
    pub fn budget_underflow(class: StorageClass, soft_bytes: u32, reserved_bytes: u32) -> Self {
        Self {
            code: StoragePlanDiagnosticCode::StoragePolicyBudgetUnderflow,
            class,
            soft_bytes,
            reserved_bytes,
        }
    }
}

impl fmt::Display for StoragePolicyDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {:?} budget underflow: reserved_bytes={} exceeds soft_bytes={}",
            self.code.as_str(),
            self.class,
            self.reserved_bytes,
            self.soft_bytes
        )
    }
}

impl Error for StoragePolicyDiagnostic {}

pub type StoragePolicyDiagnosticCode = StoragePlanDiagnosticCode;

pub fn wram_hot_per_value_eligibility_ceiling(
    policy: &StoragePolicyView,
) -> Result<u32, StoragePolicyDiagnostic> {
    checked_allocatable_budget(
        StorageClass::WramHot,
        policy.storage_pressure_budget.wram_hot.soft_bytes,
        policy.runtime_chrome_budget.wram_hot.reserved_bytes,
    )
}

pub fn allocatable_hram_budget(policy: &StoragePolicyView) -> Result<u32, StoragePolicyDiagnostic> {
    checked_allocatable_budget(
        StorageClass::HramHot,
        policy.storage_pressure_budget.hram_hot.soft_bytes,
        policy.runtime_chrome_budget.hram_hot.reserved_bytes,
    )
}

pub fn soft_pressure_threshold_bytes(
    policy: &StoragePolicyView,
) -> Result<u32, StoragePolicyDiagnostic> {
    let allocatable = u64::from(wram_hot_per_value_eligibility_ceiling(policy)?);
    let threshold = (allocatable * u64::from(SOFT_PRESSURE_FRACTION.numerator))
        / u64::from(SOFT_PRESSURE_FRACTION.denominator);

    Ok(threshold as u32)
}

#[must_use]
pub fn recompute_cycle_ceiling(policy: &StoragePolicyView) -> u32 {
    policy.compile_knobs.global.storage.recompute_cycle_ceiling
}

#[must_use]
pub fn transcript_capture_enabled(policy: &StoragePolicyView) -> bool {
    policy.transcript_capture_policy.enabled
}

#[must_use]
pub fn trace_capture_admits(policy: &StoragePolicyView, value: ValueId) -> bool {
    policy.trace_capture_policy.enabled_probes.contains(&value)
}

fn checked_allocatable_budget(
    class: StorageClass,
    soft_bytes: u32,
    reserved_bytes: u32,
) -> Result<u32, StoragePolicyDiagnostic> {
    soft_bytes
        .checked_sub(reserved_bytes)
        .ok_or_else(|| StoragePolicyDiagnostic::budget_underflow(class, soft_bytes, reserved_bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_TOML, COMPILE_PROFILE_SPEC_VERSION, CompileKnobBounds,
        CompileKnobValues, CompileKnobs, CompileObjective, CompilerFeature, EffectiveConstraints,
        KnobLockSet, RepairPolicy, RepairPolicyProfile, ResolvedCompilePolicy, RuntimeChromeBudget,
        RuntimeMemoryCapSection, RuntimeMode, ServiceLevelObjective,
        canonical_default_bounds_fixture, load_compile_profile_spec,
    };

    use super::*;

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn compile_values_from_profile() -> CompileKnobValues {
        let profile =
            load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("profile parses");
        CompileKnobValues {
            placement: profile.knob_defaults.placement.expect("placement default"),
            observation: profile
                .knob_defaults
                .observation
                .expect("observation default"),
            range: profile.knob_defaults.range.expect("range default"),
            storage: profile.knob_defaults.storage.expect("storage default"),
            sram: profile.knob_defaults.sram.expect("sram default"),
            rom_window: profile
                .knob_defaults
                .rom_window
                .expect("rom window default"),
            overlay: profile.knob_defaults.overlay.expect("overlay default"),
            schedule: profile.knob_defaults.schedule.expect("schedule default"),
        }
    }

    fn compile_bounds_from_profile() -> CompileKnobBounds {
        let profile =
            load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("profile parses");
        CompileKnobBounds {
            placement: profile.knob_bounds.placement.expect("placement bound"),
            observation: profile.knob_bounds.observation.expect("observation bound"),
            range: profile.knob_bounds.range.expect("range bound"),
            storage: profile.knob_bounds.storage.expect("storage bound"),
            sram: profile.knob_bounds.sram.expect("sram bound"),
            rom_window: profile.knob_bounds.rom_window.expect("rom window bound"),
            overlay: profile.knob_bounds.overlay.expect("overlay bound"),
            schedule: profile.knob_bounds.schedule.expect("schedule bound"),
        }
    }

    fn runtime_budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            runtime_nucleus_hash: hash(0x40),
            rom_slots: vec![],
            memory_caps: RuntimeMemoryCapSection {
                wram_usable_bytes: 8192,
                sram_usable_bytes: 32768,
                hram_usable_bytes: 127,
                source_target_profile_hash: hash(0x41),
            },
            wram_reserved: 512,
            sram_reserved: 0,
        }
    }

    fn minimal_policy() -> ResolvedCompilePolicy {
        let profile =
            load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("profile parses");

        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from("Bringup"),
            objective: CompileObjective {
                service: Some(ServiceLevelObjective {
                    max_first_token_cycles_p95: Some(21_000),
                    max_checkpoint_gap_cycles_p95: None,
                    max_resume_latency_cycles_p95: Some(8_000),
                    max_ui_jitter_frames_p99: Some(2),
                }),
                max_cycles_per_token: Some(24_000),
                max_bank_switches_per_token: Some(17),
                max_sram_page_switches_per_token: Some(3),
                min_ui_headroom_pct: 11,
                max_rom_bytes: Some(2 * 1024 * 1024),
                risk: profile.risk_policy.clone(),
            },
            effective_constraints: EffectiveConstraints {
                target_caps: canonical_default_bounds_fixture(),
                required_features: BTreeSet::from([CompilerFeature::StaticBudgetReport]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                runtime_chrome_budget: Some(runtime_budget_fixture()),
            },
            observability: profile.observability,
            trace_budget: profile.trace_budget,
            range_caps: profile.range_caps,
            observation_caps: profile.observation_caps,
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            knobs: CompileKnobs {
                global: compile_values_from_profile(),
                bounds: compile_bounds_from_profile(),
                locks: KnobLockSet::default(),
                overrides: Default::default(),
                provenance: vec![],
            },
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            provenance: gbf_policy::PolicyProvenance {
                target_defaults: hash(0x01),
                profile_defaults: hash(0x02),
                compile_profile_spec_version: COMPILE_PROFILE_SPEC_VERSION.to_owned(),
                hint_bundle_hash: None,
                compile_request_hash: hash(0x03),
                calibration_hash: None,
            },
        }
    }

    fn view_fixture() -> StoragePolicyView {
        StoragePolicyView::from_resolved_policy(&minimal_policy())
    }

    #[test]
    fn wram_hot_ceiling_reports_store_034_underflow() {
        let mut view = view_fixture();
        view.storage_pressure_budget.wram_hot.soft_bytes = 10;
        view.runtime_chrome_budget.wram_hot.reserved_bytes = 11;

        let error = wram_hot_per_value_eligibility_ceiling(&view)
            .expect_err("reserved budget greater than soft budget underflows");

        assert_eq!(
            error.code,
            StoragePolicyDiagnosticCode::StoragePolicyBudgetUnderflow
        );
        assert_eq!(error.code.as_str(), "STORE-034");
        assert_eq!(error.class, StorageClass::WramHot);
        assert_eq!(error.soft_bytes, 10);
        assert_eq!(error.reserved_bytes, 11);
    }

    #[test]
    fn helpers_compile_against_minimal_resolved_policy() {
        let mut view = view_fixture();
        let admitted = ValueId::new(7);
        view.trace_capture_policy.enabled_probes.insert(admitted);

        assert_eq!(
            wram_hot_per_value_eligibility_ceiling(&view).expect("wram budget is valid"),
            7680
        );
        assert_eq!(
            allocatable_hram_budget(&view).expect("hram budget is valid"),
            127
        );
        assert_eq!(
            soft_pressure_threshold_bytes(&view).expect("soft threshold computes"),
            6528
        );
        assert_eq!(recompute_cycle_ceiling(&view), 24_000);
        assert!(!transcript_capture_enabled(&view));
        assert!(trace_capture_admits(&view, admitted));
        assert!(!trace_capture_admits(&view, ValueId::new(8)));
    }

    #[test]
    fn soft_pressure_fraction_is_integer_pair() {
        let mut view = view_fixture();
        view.storage_pressure_budget.wram_hot.soft_bytes = 1000;
        view.runtime_chrome_budget.wram_hot.reserved_bytes = 0;

        assert_eq!(SOFT_PRESSURE_FRACTION.numerator, 85);
        assert_eq!(SOFT_PRESSURE_FRACTION.denominator, 100);
        assert_eq!(
            soft_pressure_threshold_bytes(&view).expect("soft threshold computes"),
            850
        );
    }

    #[test]
    fn policy_view_static_scan_rejects_legacy_paths() {
        let source = [
            include_str!("policy_view.rs"),
            include_str!("predicates.rs"),
        ]
        .join("\n");
        let forbidden = [
            [
                "policy",
                ".",
                "compile_knobs",
                ".",
                "global",
                ".",
                "pressure",
            ]
            .concat(),
            ["storage_", "class_", "override"].concat(),
            ["overlay_", "excluded_", "set"].concat(),
            ["Overlay", "Region", "Size", "Ceiling"].concat(),
            ["kernel", ".", "staged_", "lut_", "fragments"].concat(),
            ["trace_", "demotion", ".", "level"].concat(),
        ];

        for key in forbidden {
            assert!(
                !source.contains(&key),
                "storage policy view contains unavailable policy path {key:?}"
            );
        }
    }
}
